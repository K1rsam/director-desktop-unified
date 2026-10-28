//! Offline route and compatibility checks. No channel or real account is used.
use super::*;

fn fixture() -> (tempfile::TempDir, Store) {
    let dir=tempfile::tempdir().unwrap();
    let engine=Engine::with_channels(dir.path().join("home"),Arc::new(crate::execution_policy::OfflineChannels)).unwrap();
    let project=engine.dispatch(json!({"action":"projects.create","title":"lite041 route synthetic"})).unwrap();
    let store=engine.store(s(&project,"id")).unwrap();
    (dir,store)
}
fn verify(store:&Store,guard:&Value)->Result<()> {
    let mut c=store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
    crate::workflow::verify_guard(&tx,guard)
}

#[tokio::test]
async fn explicit_lite_route_avoids_legacy_full_catalogue_gate_and_preserves_source_scope() {
    let (_dir,store)=fixture();
    let seeded=pipeline::series::seed_engine_series_fixture(&store,1).await;
    // An unrelated incomplete old foundation must not prevent lite preparing
    // only the selected episode. This does not claim its designs are adopted.
    store.save_foundation(json!({"sourceSha256":seeded["source"]["sha256"],"productionDirection":"保存的方向"})).unwrap();
    let source=store.source().unwrap();let mut settings=store.settings().unwrap();
    settings["videoProvider"]=json!("missing-video-component");settings["videoModel"]=json!("unavailable-video-catalog");
    store.save_settings(&settings).unwrap();let settings=store.settings().unwrap();
    let before=store.jobs().unwrap();let assets=store.values("assets").unwrap();
    let req=json!({"action":"storyboard.run","workflowProfile":pipeline::series_lite::PROFILE,"episodeIds":[source["episodes"][1]["id"]]});
    let input=production_input_for_new_workflow(&store,&req).unwrap();
    assert!(pipeline::series_lite::uses_profile(&input));
    assert!(!pipeline::series::uses_profile(&input).unwrap());
    assert_eq!(input["source"]["sha256"],source["sha256"]);
    assert_eq!(input["source"]["version"],source["version"]);
    assert_eq!(input["selection"]["episodeIds"],req["episodeIds"]);
    assert_eq!(input["settings"],settings);
    assert_eq!(input["capabilities"]["profileId"],"series-lite/storyboard-planning/v1");
    assert_eq!(input["capabilities"]["planningOnly"],true);
    for marker in ["storyboardRulePackProfile","storyboardCreativeProfile","storyboardCarrierProfile","storyboardSceneInputProfile","storyboardSoundEventProfile","assetCoverageProfile","seriesDesignSourceUses","seriesAssetSupplementVersion"] {
        assert!(input.get(marker).is_none(),"legacy marker {marker}");
    }
    assert_eq!(store.jobs().unwrap(),before);
    assert_eq!(store.values("assets").unwrap(),assets);
    let mut partial=req;partial["unitIds"]=json!(["legacy-unit"]);
    assert!(production_input_for_new_workflow(&store,&partial).unwrap_err().contains("LITE_WHOLE_EPISODE_REQUIRED"));
}

#[test]
fn frozen_rule_dispatch_is_version_bound_and_never_resigns_old_tasks() {
    let mut lite=json!({});pipeline::series_lite::mark_input(&mut lite).unwrap();
    let rules=json!(pipeline::series_lite::rule_snapshots_for(&lite).unwrap());
    let snapshot=json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":lite,"rules":rules});
    let job=json!({"kind":"workflow","entityId":"storyboard.run","snapshot":snapshot});
    let before=job.clone();
    assert_eq!(production_rules_for_snapshot(&job["snapshot"]).unwrap(),rules);
    assert!(workflow_resume_rule_audit(&job,&rules).unwrap().is_none());
    assert_eq!(job,before);
    let mut invalid=job["snapshot"].clone();invalid["input"]["seriesRulePackSha256"]=json!("0".repeat(64));
    assert!(production_rules_for_snapshot(&invalid).is_err());
    let mut old=json!({});pipeline::series::mark_legacy_input(&mut old).unwrap();
    let original=json!({"stage":"storyboard.run","input":old});
    assert_eq!(production_rules_for_snapshot(&original).unwrap(),json!(pipeline::series::rule_snapshots_for(&original["input"]).unwrap()));
}

#[tokio::test]
async fn explicit_lite_start_recovers_original_old_snapshot_and_holds_unknown() {
    let (_dir,store)=fixture();
    let seeded=pipeline::series::seed_engine_series_fixture(&store,1).await;
    let episode=seeded["productionScope"]["episodeIds"][1].clone();
    let original=production_input(&store,&json!({"action":"storyboard.run","episodeIds":[episode]})).unwrap();
    let snapshot=json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","sourceVersion":original["source"]["version"],"input":original});
    let id=store.add_job("workflow","storyboard.run","local","local",&snapshot,"interrupted").unwrap();
    let input=production_input_for_new_workflow(&store,&json!({"action":"storyboard.run","workflowProfile":pipeline::series_lite::PROFILE,"episodeIds":[episode]})).unwrap();
    assert_eq!(existing_storyboard_start(&store,&input).unwrap(),StoryboardStartBoundary::Resume(id.clone()));
    assert_eq!(store.job(&id).unwrap()["snapshot"],snapshot);
    store.update_job(&id,"submission_unknown",&json!({})).unwrap();
    assert!(matches!(existing_storyboard_start(&store,&input).unwrap(),StoryboardStartBoundary::Hold(_)));
    assert_eq!(store.job(&id).unwrap()["snapshot"],snapshot);
}

#[tokio::test]
async fn lite_guard_ignores_unconsumed_storage_changes_but_keeps_real_text_cas() {
    let (_dir,store)=fixture();
    let seeded=pipeline::series::seed_engine_series_fixture(&store,1).await;
    let input=production_input_for_new_workflow(&store,&json!({"action":"storyboard.run","workflowProfile":pipeline::series_lite::PROFILE,"episodeIds":[seeded["productionScope"]["episodeIds"][1]]})).unwrap();
    let asset=store.values("assets").unwrap()[0].clone();
    let guard=crate::series_lite_guard::with_designs(crate::workflow::guard_from_input(&input),&[asset.clone()]).unwrap();
    let mut legacy=input.clone();legacy.as_object_mut().unwrap().remove("workflowProfile");
    let old_guard=crate::workflow::guard_from_input(&legacy);
    verify(&store,&guard).unwrap();verify(&store,&old_guard).unwrap();
    let mut media=asset.clone();media["imagePrompt"]=json!("仅改图片提示词");media["adoptedMediaId"]=json!("new-picture");
    store.save_asset(media.clone()).unwrap();
    store.save_asset(json!({"id":"unrelated","name":"无关物件","kind":"PROP","variantId":"base"})).unwrap();
    let mut foundation=store.foundation().unwrap();foundation["visualFactBatches"]=json!([{"audit":"new"}]);store.save_foundation(foundation).unwrap();
    verify(&store,&guard).unwrap();
    assert!(verify(&store,&old_guard).is_err(),"old snapshots keep strict original storage guards");
    let snapshot=json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":input,"guard":guard});
    let id=store.add_job("workflow","storyboard.run","local","local",&snapshot,"interrupted").unwrap();
    store.refresh_storyboard_audit_guard(&id,&snapshot).unwrap();
    assert_eq!(store.job(&id).unwrap()["snapshot"],snapshot);
    assert_eq!(store.job(&id).unwrap()["prepared"]["publicationGuard"],guard);
    media=store.get("assets",s(&asset,"id")).unwrap();media["adoptedDesign"]=json!({"full":"真实设计语义改变"});store.save_asset(media).unwrap();
    assert!(verify(&store,&guard).unwrap_err().contains("消费设计已变化"));
    let parent=crate::workflow::guard_from_input(&input);
    let mut boundary=store.get("units",s(&input["previousBoundary"],"id")).unwrap();
    boundary["prompt"]=json!("用户改动了真实前情");store.save_unit(boundary).unwrap();
    assert!(verify(&store,&parent).unwrap_err().contains("人工编辑变化"));
    let mut unknown=parent;unknown["version"]=json!("future-guard/v99");
    assert!(verify(&store,&unknown).unwrap_err().contains("WORKFLOW_GUARD_VERSION_UNSUPPORTED"));
}
