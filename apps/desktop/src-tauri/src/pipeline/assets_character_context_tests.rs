use super::*;

fn fixture() -> Value {
    let source = prepare_source(concat!(
        "第一集\n场景：门口\n△小林拿起杯子。\n△一辆车经过。\n",
        "场景：桥边\n△小林被飞石惊动。\n△同伴挡在她身前。\n△小林向桥下望去，她握紧栏杆。\n",
        "第二集\n场景：桥边\n△另一个同名小林是男性，穿着未来场次红衣。\n"
    )).unwrap();
    let first = &source.scenes[0];
    let second = &source.scenes[1];
    let heading = source.rows.iter().find(|row| row.scene_id.as_ref() == Some(&first.id) && row.kind == "format").unwrap();
    let body = source.rows.iter().find(|row| row.text.contains("杯子")).unwrap();
    json!({"source":source,"assets":[{
        "id":"person:base","identityId":"person","baseAssetId":"person","variantId":"base",
        "kind":"CH","name":"小林｜默认","sourceRefs":[heading.id,body.id],
        "sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":{"kind":"PROGRAM_SCOPE","sourceSha256":source.sha256},
        "scope":{"kind":"SCENES","scenes":[{"episodeId":first.episode_id,"sceneId":first.id},{"episodeId":second.episode_id,"sceneId":second.id}]},
        "preparation":{"mode":"REFERENCE","note":"原文未指定衣装，不新增事实。"}
    }],"foundation":{"sourceSha256":source.sha256,"facts":[],"assets":[{"id":"person","name":"小林","aliases":[]}]}})
}

fn context(input:&Value)->Vec<SourceRow> {
    let source:LockedSource=serde_json::from_value(input["source"].clone()).unwrap();
    let refs:HashSet<_>=input["assets"][0]["sourceRefs"].as_array().unwrap().iter().map(|id|id.as_str().unwrap()).collect();
    scene_context(input,&source,&input["assets"][0],&[],&refs).unwrap().into_iter().cloned().collect()
}

#[test]
fn character_named_action_and_following_pronoun_survive_sparse_body_reference() {
    let mut input=fixture();
    assert!(context(&input).is_empty(),"the one unrelated body anchor reproduces the legacy short circuit");
    freeze_design_evidence_profile(&mut input);
    let original=input.clone();
    let rows=context(&input);
    assert!(rows.iter().any(|row|row.text.contains("挡在她身前")));
    assert!(rows.iter().any(|row|row.text.contains("小林向桥下望去，她")));
    assert!(!rows.iter().any(|row|row.text.contains("未来场次红衣")),"same scene title cannot join another episode");
    assert_eq!(input,original);
    let source:LockedSource=serde_json::from_value(input["source"].clone()).unwrap();
    assert!(rows.iter().all(|row|source.rows.iter().any(|original|
        serde_json::to_value(original).unwrap()==serde_json::to_value(row).unwrap())),"raw row bytes and ranges are unchanged");
    let binding=design_evidence_binding(&input,&input["assets"][0]).unwrap().unwrap();
    assert_eq!(binding["version"],DESIGN_EVIDENCE_PROFILE);
    assert!(binding["evidenceRowIds"].as_array().unwrap().iter().any(|id|
        rows.iter().any(|row|row.text.contains("挡在她身前")&&id==&json!(row.id))));
}

#[test]
fn character_legacy_v1_and_absent_profiles_keep_original_frozen_evidence() {
    let mut input=fixture();
    let absent=context(&input);
    input[DESIGN_EVIDENCE_PROFILE_FIELD]=json!({"version":LEGACY_DESIGN_EVIDENCE_PROFILE,
        "instructions":INSTRUCTIONS,"instructionsSha256":digest(INSTRUCTIONS)});
    assert_eq!(serde_json::to_value(context(&input)).unwrap(),serde_json::to_value(absent).unwrap());
    let binding=design_evidence_binding(&input,&input["assets"][0]).unwrap().unwrap();
    assert_eq!(binding["version"],LEGACY_DESIGN_EVIDENCE_PROFILE);
    assert_eq!(binding["instructionsSha256"],digest(INSTRUCTIONS));
    assert_eq!(binding["evidenceRowIds"],input["assets"][0]["sourceRefs"]);
}

#[test]
fn character_context_has_fixed_raw_row_budget_and_does_not_invent_attribution() {
    let mut script="第一集\n场景：门口\n".to_owned();
    for i in 0..200 {script.push_str(&format!("△小林看向同伴，她挥手，第{i}次。\n"));}
    let source=prepare_source(&script).unwrap();
    let mut input=fixture();
    input["source"]=json!(source);
    input["foundation"]["sourceSha256"]=json!(source.sha256);
    let heading=source.rows.iter().find(|r|r.kind=="format"&&r.scene_id.is_some()).unwrap();
    input["assets"][0]["sourceRefs"]=json!([heading.id]);
    input["assets"][0]["sourceScope"]["sourceSha256"]=json!(source.sha256);
    input["assets"][0]["scope"]=json!({"kind":"SCENES","scenes":[{"episodeId":heading.episode_id,"sceneId":heading.scene_id}]});
    freeze_design_evidence_profile(&mut input);
    let rows=context(&input);
    assert!(!rows.is_empty()&&rows.len()<=CHARACTER_CONTEXT_ROWS);
    assert!(rows.iter().map(|r|r.text.encode_utf16().count()).sum::<usize>()<=CHARACTER_CONTEXT_UTF16);
    assert!(rows.len()<source.rows.len());
    assert_eq!(input["foundation"]["facts"],json!([]));
    let binding=design_evidence_binding(&input,&input["assets"][0]).unwrap().unwrap();
    assert!(binding.get("gender").is_none(),"raw pronoun ranking never creates a fact");
    assert!(profile_instructions(DESIGN_EVIDENCE_PROFILE).unwrap().contains("代词可能指同场其他人"));
}

#[test]
fn character_context_receipt_binds_only_consumed_rows_and_rejects_scope_drift() {
    let mut input=fixture();freeze_design_evidence_profile(&mut input);
    let original=design_evidence_binding(&input,&input["assets"][0]).unwrap();
    let mut changed=input.clone();
    changed["source"]["rows"].as_array_mut().unwrap().iter_mut().find(|r|r["text"].as_str().unwrap().contains("挡在她身前")).unwrap()["text"]=json!("△同伴挡在他身前。\n");
    assert_ne!(original,design_evidence_binding(&changed,&changed["assets"][0]).unwrap());
    let mut unread=input.clone();
    unread["source"]["rows"].as_array_mut().unwrap().iter_mut().find(|r|r["text"].as_str().unwrap().contains("未来场次红衣")).unwrap()["text"]=json!("△未来内容变化。\n");
    assert_eq!(original,design_evidence_binding(&unread,&unread["assets"][0]).unwrap());
    input["assets"][0]["scope"]["scenes"][0]["episodeId"]=json!("other-episode");
    assert!(design_evidence_binding(&input,&input["assets"][0]).unwrap_err().contains("集场坐标"));
}

#[derive(Default)]
struct Capture(std::sync::Mutex<Vec<Value>>);
#[async_trait]
impl TextExecutor for Capture {
    async fn execute(&self,request:TextRequest)->Result<Value,String>{
        if request.stage=="assetVisualFacts" {return Ok(json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":[],"issues":[]}));}
        assert_eq!(request.stage,"assetDesign");self.0.lock().unwrap().push(request.input.clone());
        let asset=&request.input["asset"];
        Ok(json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":"仅合成美术测试","freeDesignScope":"合成测试","sourceRefs":asset["sourceRefs"],"referenceVariantIds":[]}))
    }
}

#[tokio::test]
async fn character_context_reaches_cd_wire_as_raw_evidence_without_modifying_facts_or_assets(){
    let mut input=fixture();freeze_design_evidence_profile(&mut input);let original=input.clone();
    let capture=Capture::default();let result=run_asset_design(&input,&capture,"offline","medium").await.unwrap();
    assert!(result["blocked"].as_array().unwrap().is_empty(),"{result}");
    let requests=capture.0.lock().unwrap();assert_eq!(requests.len(),1);
    let wire=&requests[0];assert_eq!(wire["facts"],json!([]));assert_eq!(wire["deltaFacts"],json!([]));
    assert!(wire["evidence"].as_array().unwrap().iter().any(|r|r["text"].as_str().unwrap().contains("挡在她身前")&&r["evidenceUses"].as_array().unwrap().contains(&json!("SOURCE_CONTEXT"))));
    assert!(!wire["evidence"].to_string().contains("未来场次红衣"));
    assert_eq!(wire["designEvidenceContext"]["version"],DESIGN_EVIDENCE_PROFILE);
    assert_eq!(input,original);
}
