use super::*;
use crate::pipeline::{asset_use_binding as binding, stage3_pack, storyboard_refinement::tests::Model};

struct Record { saved: SavedTextExecutor, model: Model }
#[async_trait::async_trait]
impl TextExecutor for Record {
    async fn execute(&self, request: TextRequest) -> Result<Value> {
        let raw=self.model.execute(request.clone()).await?;
        let snapshot=self.saved.request_snapshot(&request)?;
        let hash=crate::pipeline::digest(&snapshot.to_string());
        let job=self.saved.store.add_job("text",&request.stage,"api","offline-fixture",&snapshot,"received")?;
        self.saved.store.event(&job,"text_received_raw",&json!({"requestHash":hash,"value":raw}))?;
        self.saved.store.checkpoint(self.saved.workflow_id.as_deref().unwrap(),&step_key(&request),&request.stage,&hash,"received",Some(&job),Some(&raw),None)?;
        Ok(raw)
    }
    async fn accept(&self,request:&TextRequest,value:&Value)->Result<()> {self.saved.accept(request,value).await}
}
struct Replay(SavedTextExecutor);
#[async_trait::async_trait]
impl TextExecutor for Replay {
    async fn execute(&self,request:TextRequest)->Result<Value>{self.0.execute(request).await}
    async fn accept(&self,request:&TextRequest,value:&Value)->Result<()>{self.0.accept(request,value).await}
    async fn reject(&self,request:&TextRequest,value:&Value,reason:&str)->Result<()>{self.0.reject(request,value,reason).await}
}

#[tokio::test]
async fn asset_use_binding_saved_api_run_reopens_raw_and_preserves_accepted_nodes() {
    let directory=tempfile::tempdir().unwrap();let path=directory.path().join("project");
    let store=Store::open(path.clone()).unwrap();store.create("离线素材绑定验收","fixture").unwrap();store.ensure_workflow_storage().unwrap();
    let source=store.save_source(json!(crate::pipeline::prepare_source("第一集\n场景：门口\n△小王推门。\n").unwrap())).unwrap();
    let settings=json!({"textProvider":"api","textEndpoint":"http://127.0.0.1:9","textApiToken":"offline-fixture-placeholder"});
    let mut input=json!({"source":source,"assets":[],"foundation":{"facts":[]},"settings":{"fantasyMode":"off","visualStyle":"16:9真人"},"capabilities":{"minDuration":0,"maxDuration":15,"imageLimit":0},"selection":{"episodeIds":[source["episodes"][0]["id"]]}});
    stage3_pack::mark_input(&mut input).unwrap();binding::freeze_new(&mut input).unwrap();
    let run=store.add_job("workflow","storyboard.run","local","offline-fixture",&json!({"input":input}),"running").unwrap();
    let recorder=Record{saved:SavedTextExecutor{store:store.clone(),settings:settings.clone(),workflow_id:Some(run.clone())},model:Model::default()};
    let first=crate::pipeline::run_storyboard(&input,&recorder,"offline-fixture","medium").await.unwrap();
    assert_eq!(first["status"],"completed","{}",first["blocked"]);
    let requests=recorder.model.calls.lock().await.clone();assert_eq!(requests.len(),3);
    let evidence=|store:&Store|->Value {
        let c=store.conn().unwrap();let mut q=c.prepare("SELECT id,state,snapshot,result,receipt FROM jobs WHERE kind='text' ORDER BY id").unwrap();
        let jobs=q.query_map([],|r|Ok(json!([r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?]))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        let mut q=c.prepare("SELECT job_id,data FROM events WHERE type='text_received_raw' ORDER BY id").unwrap();let raw=q.query_map([],|r|Ok(json!([r.get::<_,String>(0)?,r.get::<_,String>(1)?]))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();json!({"jobs":jobs,"raw":raw})
    };
    let before=evidence(&store);let reopened=Store::open(path).unwrap();
    let replay=Replay(SavedTextExecutor{store:reopened.clone(),settings,workflow_id:Some(run)});
    let second=crate::pipeline::run_storyboard(&input,&replay,"offline-fixture","medium").await.unwrap();
    assert_eq!(second["status"],"completed","{}",second["blocked"]);assert_eq!(second,first);assert_eq!(evidence(&reopened),before);
    let author=requests.iter().find(|r|r.stage=="storyboardCreate").unwrap();let snapshot=replay.0.request_snapshot(author).unwrap();
    let raw=replay.0.asset_use_accepted_raw(author,&snapshot).unwrap().unwrap();assert!(raw["units"][0].get(binding::FIELD).is_none());
    let mut changed=first.clone();changed["units"][0]["title"]=json!("未经采用的替换");
    assert!(replay.0.accept(author,&changed).await.is_err());assert!(replay.0.reject(author,&raw,"强制重验失败").await.is_err());assert_eq!(evidence(&reopened),before);
    let mut legacy=author.clone();legacy.input.as_object_mut().unwrap().remove(binding::FIELD);
    assert!(replay.0.asset_use_accepted_raw(&legacy,&snapshot).unwrap().is_none(),"unmarked request never enters new cache contract");
    let job:String=reopened.conn().unwrap().query_row("SELECT id FROM jobs WHERE kind='text' AND snapshot=?1",[snapshot.to_string()],|r|r.get(0)).unwrap();
    reopened.conn().unwrap().execute("DELETE FROM events WHERE job_id=?1 AND type='text_received_raw'",[job]).unwrap();
    let before=evidence(&reopened);let error=replay.0.execute(author.clone()).await.unwrap_err();assert!(error.contains("ASSET_USE_ACCEPTED_RAW_HELD"),"{error}");assert_eq!(evidence(&reopened),before,"missing raw never dispatches or mutates accepted checkpoint");
}

#[test]
fn asset_use_binding_unmanaged_cli_raw_replay_checks_exact_bindings_without_launch() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path().join("project")).unwrap();store.create("离线CLI读取","fixture").unwrap();store.ensure_workflow_storage().unwrap();
    let run=store.add_job("workflow","storyboard.run","local","offline",&json!({}),"running").unwrap();
    let request=TextRequest{stage:"storyboardCreate".into(),skill:"unit-performance-director".into(),model:"offline".into(),reasoning:"medium".into(),input:json!({binding::FIELD:binding::PROFILE}),schema:json!({}),rule_snapshot:json!({})};
    let snapshot=json!({"frozen":"synthetic-unmanaged-cli"});let hash=crate::pipeline::digest(&snapshot.to_string());let raw=json!({"units":[{"id":"U","assetRefs":[]}]});let accepted=json!({"units":[{"id":"U",binding::FIELD:binding::PROFILE,"assetRefs":[]}]});
    let job=store.add_job("text",&request.stage,"codex","offline",&snapshot,"completed").unwrap();
    store.update_job(&job,"completed",&json!({"result":accepted,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}})).unwrap();
    store.event(&job,"text_received_raw",&json!({"requestHash":hash,"value":raw})).unwrap();
    store.checkpoint(&run,&step_key(&request),&request.stage,&hash,"accepted",Some(&job),Some(&accepted),None).unwrap();
    let executor=SavedTextExecutor{store:store.clone(),settings:json!({"textProvider":"codex"}),workflow_id:Some(run)};
    let before=store.job(&job).unwrap()["result"].clone();
    assert_eq!(executor.asset_use_accepted_raw(&request,&snapshot).unwrap(),Some(raw));
    store.event(&job,"text_received_raw",&json!({"requestHash":"tampered","value":{"bad":true}})).unwrap();
    assert!(executor.asset_use_accepted_raw(&request,&snapshot).unwrap_err().contains("ASSET_USE_ACCEPTED_RAW_HELD"));
    assert_eq!(store.job(&job).unwrap()["result"],before);
}

#[tokio::test]
async fn asset_use_binding_saved_repairs_commit_reopen_without_dispatch_and_reject_bad_audits() {
    for (batch,field,version,policy,audit_key) in [
        ("carrier-repair/v1","carrierRepair","accepted-finisher-carrier-repair/v1","carrier-received-validation/v1","carrierReceivedValidation"),
        ("author-domain-repair/v1","authorRepair","rejected-author-scope-repair/v1","author-scope-received-validation/v1","authorScopeReceivedValidation"),
        ("finish-domain-repair/v1","finishRepair","rejected-finish-scope-repair/v1","author-scope-received-validation/v1","authorScopeReceivedValidation"),
    ] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("project");
        let store=Store::open(path.clone()).unwrap();store.create("离线原修复恢复","fixture").unwrap();store.ensure_workflow_storage().unwrap();
        let run=store.add_job("workflow","storyboard.run","local","offline",&json!({}),"running").unwrap();
        let settings=json!({"textProvider":"api","textEndpoint":"http://127.0.0.1:9","textApiToken":"offline-fixture-placeholder"});
        let executor=SavedTextExecutor{store:store.clone(),settings:settings.clone(),workflow_id:Some(run.clone())};
        let request=TextRequest{stage:"storyboardFinish".into(),skill:"episode-finishing-editor".into(),model:"offline".into(),reasoning:"medium".into(),
            input:json!({binding::FIELD:binding::PROFILE,"episodeId":"ep","sourceRows":[{"id":"r1","text":"甲：原话。"}],"executionBatch":{"version":batch,"id":"repair-1"},field:{"version":version,"attempt":1,"maximumAttempts":2,"originDomainRequestSha256":"frozen-origin","baseDraftHash":"frozen-base"}}),schema:json!({"type":"object"}),rule_snapshot:json!({})};
        let snapshot=executor.request_snapshot(&request).unwrap();let hash=crate::pipeline::digest(&snapshot.to_string());
        let raw=json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":"frozen-base","retainedUnitIds":[],"replacements":[]});
        let mut accepted=raw.clone();accepted["materializedDraft"]=json!({"units":[{"id":"U",binding::FIELD:binding::PROFILE,"assetRefs":[]}]});
        let job=store.add_job("text",&request.stage,"api","offline",&snapshot,"received").unwrap();
        store.update_job(&job,"received",&json!({"result":raw})).unwrap();
        store.event(&job,"text_received_raw",&json!({"requestHash":hash,"value":raw})).unwrap();
        store.checkpoint(&run,&step_key(&request),&request.stage,&hash,"running",Some(&job),None,None).unwrap();
        executor.accept(&request,&accepted).await.unwrap();
        let proof:Value=store.conn().unwrap().query_row("SELECT receipt FROM jobs WHERE id=?1",[&job],|r|r.get::<_,String>(0)).map(|s|serde_json::from_str(&s).unwrap()).unwrap();
        assert_eq!(proof["migrationPolicy"],policy,"real existing commit emitted the policy");
        let evidence=|store:&Store|->Value {let c=store.conn().unwrap();json!({"job":c.query_row("SELECT snapshot||state||result||receipt FROM jobs WHERE id=?1",[&job],|r|r.get::<_,String>(0)).unwrap(),"step":c.query_row("SELECT state||result FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2",params![run,step_key(&request)],|r|r.get::<_,String>(0)).unwrap(),"jobs":store.jobs().unwrap().len(),"events":c.query_row("SELECT COUNT(*) FROM events",[],|r|r.get::<_,i64>(0)).unwrap()})};
        let before=evidence(&store);let resumed=SavedTextExecutor{store:Store::open(path).unwrap(),settings,workflow_id:Some(run.clone())};
        for _ in 0..2 {assert_eq!(resumed.execute(request.clone()).await.unwrap(),raw);resumed.accept(&request,&accepted).await.unwrap();assert_eq!(evidence(&resumed.store),before);}
        // The same proof contract serves unmanaged CLI without any process launch.
        let cli=SavedTextExecutor{store:resumed.store.clone(),settings:json!({"textProvider":"codex"}),workflow_id:Some(run.clone())};
        cli.store.conn().unwrap().execute("UPDATE jobs SET provider='codex' WHERE id=?1",[&job]).unwrap();
        assert_eq!(cli.asset_use_accepted_raw(&request,&snapshot).unwrap(),Some(raw.clone()));
        cli.store.conn().unwrap().execute("UPDATE jobs SET provider='api' WHERE id=?1",[&job]).unwrap();
        for key in ["originalRawEventId","originalResultSha256","originalSnapshotSha256","workflowId","nodeKey","generationRequestsSent","acceptedAutomatically"] {
            let mut bad=proof.clone();bad[audit_key][key]=json!("tampered");resumed.store.conn().unwrap().execute("UPDATE jobs SET receipt=?1 WHERE id=?2",params![bad.to_string(),job]).unwrap();
            let before=evidence(&resumed.store);assert!(resumed.execute(request.clone()).await.is_err(),"{key}");assert_eq!(evidence(&resumed.store),before);
        }
        let mut bad=proof;bad["migrationPolicy"]=json!("unrecognized/v1");resumed.store.conn().unwrap().execute("UPDATE jobs SET receipt=?1 WHERE id=?2",params![bad.to_string(),job]).unwrap();
        let before=evidence(&resumed.store);assert!(resumed.execute(request.clone()).await.is_err());assert_eq!(evidence(&resumed.store),before);
    }
}
