use super::*;

fn imported_assets(f:&Fixture)->(Value,Value) {
    f.import_md();
    let assets=f.store.values("assets").unwrap();
    (assets.iter().find(|a|a["lingmoType"]=="CH").unwrap().clone(),
     assets.iter().find(|a|a["lingmoType"]=="SC").unwrap().clone())
}
fn save_words(f:&Fixture,asset:&Value,text:&str,review:bool)->Value {
    f.store.save_image_prompt(&json!({"assetId":asset["id"],"expectedVersion":asset["version"],
        "text":text,"ratio":"1:1","reviewCurrentBasis":review}),&json!({})).unwrap()
}

#[test]
fn saving_draft_when_current_source_context_is_unavailable_does_not_restore_old_ready_projection() {
    let f=Fixture::new();
    let (actor,_)=imported_assets(&f);
    let ready=save_words(&f,&actor,"原来已经确认的手工正文。",false);
    let mut missing_filename=ready;
    missing_filename["filename"]=json!("invalid/nested.png");
    f.store.save_asset(missing_filename).unwrap();
    let current=f.store.active_asset(s(&actor,"id")).unwrap();
    assert_eq!(current["promptStatus"],"ready");
    let draft=save_words(&f,&current,"先存下新草稿，待恢复合法目录再核对。",false);
    assert_eq!(draft["promptStatus"],"needs_review");
    assert_eq!(draft["imagePrompt"]["status"],"needs_review");
    assert_eq!(draft["imagePrompt"]["text"],"先存下新草稿，待恢复合法目录再核对。");
    assert!(f.store.save_image_prompt(&json!({"assetId":draft["id"],"expectedVersion":draft["version"],"text":draft["imagePrompt"]["text"],"reviewCurrentBasis":true}),&json!({})).is_err());
    assert!(f.store.jobs().unwrap().is_empty());
}

#[test]
fn manual_source_review_preserves_words_and_unknown_while_only_real_consumers_are_invalidated() {
    let f=Fixture::new();
    let (actor,scene)=imported_assets(&f);
    let actor=save_words(&f,&actor,"手工原稿：灰衣人物，不加新剧情。",false);
    let scene=save_words(&f,&scene,"手工场景：无人门厅，门左窗右。",false);
    assert_eq!(actor["promptStatus"],"ready");
    let before=actor["imagePrompt"].clone();
    assert_eq!(before["sourceBindingContract"],asset_workflow::SOURCE_BINDING_V1);
    let old_snapshot=json!({"kind":"image","workflowProfile":asset_graph::PROFILE,"assetId":actor["id"],
        "variantId":actor["variantId"],"assetVersion":actor["version"],"referenceRequirements":actor["referenceRequirements"],
        "references":[],"sourceVersion":null,"promptSource":before});
    crate::binding::validate_generation_snapshot(&f.store,&old_snapshot).unwrap();
    let unknown=f.store.add_job("image",s(&actor,"id"),"offline","original-account",&old_snapshot,"submission_unknown").unwrap();
    f.store.update_job(&unknown,"submission_unknown",&json!({"receipt":{"raw":"immutable-receipt"},"error":"original unknown"})).unwrap();
    let original_unknown=f.store.job(&unknown).unwrap();
    let mut unit=f.store.get("units",s(&before["sourceUnitBindings"][0],"unitId")).unwrap();
    unit["prompt"]=json!(format!("{}\n桌面保存编辑：保持原人物身份，灰衣改为深灰衣。",s(&unit,"prompt")));
    f.store.save_unit(unit).unwrap();
    let affected=f.store.active_asset(s(&actor,"id")).unwrap();
    assert_eq!(affected["version"],actor["version"],"dependency projection does not invent design versions");
    assert_eq!(affected["promptStatus"],"needs_review");
    assert_eq!(affected["imagePrompt"],before,"all original words and input proof remain inspectable");
    assert_eq!(f.store.active_asset(s(&scene,"id")).unwrap(),scene,"an unrelated unit/asset is untouched");
    assert!(validate_prompt_source(&f.store,&before).is_err());
    assert!(crate::binding::validate_generation_snapshot(&f.store,&old_snapshot).unwrap_err().contains("来源单元"));
    // A status-only edit cannot claim that changed evidence was checked.
    let mut status_only=affected.clone();
    status_only["promptStatus"]=json!("ready");
    status_only["imagePrompt"]["status"]=json!("ready");
    f.store.save_asset(status_only).unwrap();
    let current=f.store.active_asset(s(&actor,"id")).unwrap();
    assert_eq!(current["promptStatus"],"needs_review");
    let draft=save_words(&f,&current,"我保留的手写修订正文。",false);
    assert_eq!(draft["promptStatus"],"needs_review");
    assert_eq!(draft["imagePrompt"]["text"],"我保留的手写修订正文。");
    let reviewed=save_words(&f,&draft,s(&draft["imagePrompt"],"text"),true);
    assert_eq!(reviewed["promptStatus"],"ready");
    assert_eq!(reviewed["imagePrompt"]["text"],draft["imagePrompt"]["text"]);
    assert_eq!(reviewed["imagePrompt"]["manualReview"]["kind"],"explicit-current-basis-confirmation");
    validate_prompt_source(&f.store,&reviewed["imagePrompt"]).unwrap();
    assert_ne!(reviewed["imagePrompt"]["sourceUnitBindings"],before["sourceUnitBindings"]);
    assert_eq!(f.store.job(&unknown).unwrap(),original_unknown);
    assert_eq!(f.store.jobs().unwrap().len(),1,"local editing/review does not create generation work");
    assert!(f.store.source().unwrap().is_null());
}

#[test]
fn storage_only_unit_edit_and_legacy_prompt_contract_do_not_invalidate_manual_words() {
    let f=Fixture::new();
    let (actor,_)=imported_assets(&f);
    let actor=save_words(&f,&actor,"保留这份手工正文。",false);
    let mut unit=f.store.get("units",s(&actor["imagePrompt"]["sourceUnitBindings"][0],"unitId")).unwrap();
    unit["title"]=json!("只改界面标题");
    let unit=f.store.save_unit(unit).unwrap();
    assert_eq!(f.store.active_asset(s(&actor,"id")).unwrap(),actor);
    validate_prompt_source(&f.store,&actor["imagePrompt"]).unwrap();
    let mut legacy=actor.clone();
    legacy["imagePrompt"].as_object_mut().unwrap().remove("sourceBindingContract");
    legacy["imagePrompt"].as_object_mut().unwrap().remove("sourceUnitBindings");
    legacy=f.store.save_asset(legacy).unwrap();
    let before=f.store.active_asset(s(&actor,"id")).unwrap();
    let mut edited=unit;
    edited["prompt"]=json!(format!("{}\n新的分镜正文。",s(&edited,"prompt")));
    f.store.save_unit(edited).unwrap();
    assert_eq!(f.store.active_asset(s(&actor,"id")).unwrap(),before,"no silent retrofit into a historic contract");
    validate_prompt_source(&f.store,&legacy["imagePrompt"]).unwrap();
}

#[test]
fn manual_review_rejects_unadopted_design_and_stale_asset_version_but_can_keep_a_draft() {
    let f=Fixture::new();
    let (actor,_)=imported_assets(&f);
    let actor=save_words(&f,&actor,"保留手工正文。",false);
    let mut invalid=actor.clone();
    invalid["designStatus"]=json!("needs_review");
    invalid["adoptedDesign"]=json!({"full":"尚未重新采用的新身份设计","designRevision":2,"status":"CANDIDATE"});
    f.store.save_asset(invalid).unwrap();
    let current=f.store.active_asset(s(&actor,"id")).unwrap();
    let request=json!({"assetId":current["id"],"expectedVersion":current["version"],"text":"手工改字保留。","ratio":"1:1","reviewCurrentBasis":true});
    assert!(f.store.save_image_prompt(&request,&json!({})).is_err());
    let draft=save_words(&f,&current,"手工改字保留。",false);
    assert_eq!(draft["promptStatus"],"needs_review");
    assert_eq!(draft["imagePrompt"]["text"],"手工改字保留。");
    assert!(f.store.save_image_prompt(&json!({"assetId":draft["id"],"expectedVersion":actor["version"],"text":"过时覆盖","reviewCurrentBasis":true}),&json!({})).is_err());
    assert_eq!(f.store.active_asset(s(&actor,"id")).unwrap()["imagePrompt"]["text"],"手工改字保留。");
    assert!(f.store.jobs().unwrap().is_empty());
}

#[test]
fn adopted_storyboard_supplies_current_exact_unit_words_and_keeps_original_delivery() {
    let f=Fixture::new();
    let asset=f.asset("visible");
    let unit=f.store.save_unit(json!({"id":"u1","episodeId":"e1","sceneId":"s1","prompt":"桌面当前镜头：门左窗右，人物停在门外。",
        "imageBindings":[{"assetId":asset["id"],"variantId":"v1","purpose":"opening"}]})).unwrap();
    let delivery=json!({"markdown":"采用时的原分镜全文。","assetIndex":{"assets":[asset]},"bundle":{"draft":{"units":[unit]}}});
    f.store.conn().unwrap().execute("INSERT INTO metadata VALUES('series.delivery:e1',?1)",[delivery.to_string()]).unwrap();
    let input=f.store.image_workflow_input(&json!({}),&json!({})).unwrap();
    assert_eq!(input["imageSource"]["sourceKind"],"adopted-storyboard");
    assert_eq!(input["imageSource"]["evidence"][0]["text"],unit["prompt"]);
    assert_eq!(input["imageSource"]["evidence"][0]["assetIds"],json!(["visible"]));
    assert_eq!(input["sourceUnitBindings"][0],source_unit_binding(&unit));
    assert_eq!(f.store.series_deliveries().unwrap(),vec![delivery]);
    assert!(s(&input["imageSource"],"text").contains(s(&unit,"prompt")));
    asset_workflow::validate_source(&input["imageSource"]).unwrap();
    let basis=asset_workflow::manual_prompt_basis(&input,&asset).unwrap();
    assert_eq!(basis["sourceUnitBindings"],input["sourceUnitBindings"]);
}

#[tokio::test]
async fn confirmed_manual_prompt_survives_an_unrelated_md_unit_change_without_model_call() {
    let f=Fixture::new();
    let (actor,scene)=imported_assets(&f);
    let actor=save_words(&f,&actor,"已确认手工人物词。",false);
    let mut unrelated=f.store.values("units").unwrap().into_iter().find(|u|unit_asset_ids(u).contains(&scene["id"])).unwrap();
    unrelated["prompt"]=json!(format!("{}\n只调整场景灯光，人物单元没有变化。",s(&unrelated,"prompt")));
    f.store.save_unit(unrelated).unwrap();
    let input=f.store.image_workflow_input(&json!({"assetIds":[actor["id"]]}),&json!({})).unwrap();
    let exec=OfflineText{calls:AtomicUsize::new(0)};
    let output=asset_workflow::run_asset_prompts(&input,&exec,"offline","medium").await.unwrap();
    assert_eq!(output["blocked"],json!([]));
    assert_eq!(output["assets"][0]["imagePrompt"]["text"],actor["imagePrompt"]["text"]);
    assert_eq!(output["assets"][0]["promptStatus"],"ready");
    assert_eq!(exec.calls.load(Ordering::SeqCst),0,"manual source basis ignores unrelated global MD hash changes");
}
