use super::*;

const POLICY: &str = "finisher-asset-input-projection/v1";
const API_EXISTING_NODE_HELD: &str = "API_FINISH_EXISTING_NODE_HELD: 原整编API节点尚未处理完，或原采用缓存证据不完整；保留原请求、回包和预算，不创建或重发任务";
fn held() -> String {
    "FINISH_ASSET_PROJECTION_UNBOUND: 整编资产卡与原流程或已存在请求不一致；保留原任务，未提交新请求".into()
}

impl SavedTextExecutor {
    /// API has no passive reconciliation for an interrupted request. An existing
    /// editor can only replay its proven adopted cache, never allocate a replacement.
    pub(super) fn check_existing_api_finish_node(&self, request: &TextRequest) -> Result<()> {
        if s(&self.settings, "textProvider") != "api" || request.stage != "storyboardFinish" {
            return Ok(());
        }
        let Some(run) = self.workflow_id.as_deref() else {
            return Ok(());
        };
        let c = self.store.conn()?;
        let saved: Option<Value> = c.query_row(
            "SELECT w.stage,w.request_hash,w.state,w.result,w.error,j.id,j.kind,j.provider,j.state,j.snapshot,j.result,j.receipt,j.error FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",
            params![run,step_key(request)], |row| Ok(json!({
                "stage":row.get::<_,String>(0)?,"requestHash":row.get::<_,String>(1)?,
                "stepState":row.get::<_,String>(2)?,"accepted":row.get::<_,Option<String>>(3)?,
                "stepError":row.get::<_,Option<String>>(4)?,"jobId":row.get::<_,Option<String>>(5)?,
                "kind":row.get::<_,Option<String>>(6)?,"provider":row.get::<_,Option<String>>(7)?,
                "jobState":row.get::<_,Option<String>>(8)?,"snapshot":row.get::<_,Option<String>>(9)?,
                "result":row.get::<_,Option<String>>(10)?,"receipt":row.get::<_,Option<String>>(11)?,
                "jobError":row.get::<_,Option<String>>(12)?
            })),
        ).optional().map_err(|e|e.to_string())?;
        let Some(saved) = saved else {
            return Ok(());
        };
        let held = || API_EXISTING_NODE_HELD.to_owned();
        if saved["stepState"] != "accepted" || saved["jobState"] != "completed"
            || saved["stage"] != request.stage || saved["kind"] != "text"
            || saved["provider"] != "api" || !saved["stepError"].is_null()
            || !saved["jobError"].is_null()
        {
            return Err(held());
        }
        let snapshot_text = saved["snapshot"].as_str().ok_or_else(held)?;
        let snapshot: Value = serde_json::from_str(snapshot_text).map_err(|_|held())?;
        let expected = self.request_snapshot(request).map_err(|_|held())?;
        let hash = crate::pipeline::digest(snapshot_text);
        let receipt: Value = serde_json::from_str(s(&saved,"receipt")).map_err(|_|held())?;
        let accepted: Value = serde_json::from_str(s(&saved,"accepted")).map_err(|_|held())?;
        let result: Value = serde_json::from_str(s(&saved,"result")).map_err(|_|held())?;
        let event: Option<String> = c.query_row(
            "SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",
            [s(&saved,"jobId")],|row|row.get(0),
        ).optional().map_err(|e|e.to_string())?;
        let event: Value = serde_json::from_str(event.as_deref().ok_or_else(held)?).map_err(|_|held())?;
        if snapshot.to_string() != snapshot_text || snapshot != expected
            || saved["requestHash"] != hash
            || receipt["contract"] != "desktop-runner/v1" || receipt["validation"] != "accepted"
            || !receipt["migrationPolicy"].is_null()
            || receipt.get("acceptedRequestHash").is_some_and(|value|value != &json!(hash))
            || event["requestHash"] != hash || event["value"].is_null()
            || accepted.is_null() || accepted != result
            || self.store.accepted_step(run,&hash)?.as_ref() != Some(&accepted)
        {
            return Err(held());
        }
        Ok(())
    }

    /// Select only the exact old/new domain at the same persisted node. This
    /// never creates a task, spends a repair attempt, or rewrites a frozen input.
    pub(super) fn saved_finish_asset_projection(
        &self,
        legacy: TextRequest,
        projected: TextRequest,
    ) -> Result<TextRequest> {
        if serde_json::to_value(&legacy).map_err(|_| held())?
            == serde_json::to_value(&projected).map_err(|_| held())?
        {
            return Ok(legacy);
        }
        let run = self.workflow_id.as_deref().ok_or_else(held)?;
        let c = self.store.conn()?;
        let parent_text: String = c.query_row(
            "SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",
            [run], |r| r.get(0),
        ).map_err(|_| held())?;
        let parent: Value = serde_json::from_str(&parent_text).map_err(|_| held())?;
        let proof = &projected.input["finisherAssetInputProjection"];
        let sha = crate::pipeline::digest;
        let original = serde_json::to_value(&legacy).map_err(|_| held())?;
        let mut restored = serde_json::to_value(&projected).map_err(|_| held())?;
        let expected = json!({
            "version":POLICY,"assetInputProfile":"asset-production-card/v2",
            "sourceSha256":legacy.input["assetCatalogScope"]["sourceSha256"],
            "episodeId":legacy.input["episodeId"],
            "frozenWorkflowInputSha256":sha(&parent["input"].to_string()),
            "legacyDomainRequestSha256":sha(&serde_json::to_string(&legacy).map_err(|_| held())?),
            "projectedAssetsSha256":sha(&projected.input["assets"].to_string()),
            "projectedAssetSelectionSha256":sha(&projected.input["assetSelection"].to_string()),
            "authorInputPreserved":true
        });
        if legacy.stage != "storyboardFinish"
            || legacy.skill != "episode-finishing-editor"
            || legacy.input.get("finisherAssetInputProjection").is_some()
            || projected.input["storyboardAssetInputProfile"] != "asset-production-card/v2"
            || parent["input"]["storyboardAssetInputProfile"] != "asset-production-card/v2"
            || parent["input"]["settings"] != self.settings
            || parent["input"]["source"]["sha256"]
                != legacy.input["assetCatalogScope"]["sourceSha256"]
            || legacy.input["assetCatalogScope"]["sourceSha256"]
                .as_str()
                .is_none_or(str::is_empty)
            || legacy.input["assetCatalogScope"]["version"] != "asset-production-card/v2"
            || legacy.input["assetCatalogScope"]["episodeId"] != legacy.input["episodeId"]
            || proof != &expected
            || step_key(&legacy) != step_key(&projected)
        {
            return Err(held());
        }
        let input = restored["input"].as_object_mut().ok_or_else(held)?;
        for field in [
            "assets",
            "assetSelection",
            "storyboardAssetInputProfile",
            "finisherAssetInputProjection",
        ] {
            if let Some(value) = legacy.input.get(field) {
                input.insert(field.into(), value.clone());
            } else {
                input.remove(field);
            }
        }
        if restored != original {
            return Err(held());
        }
        let old_domain = self.request_domain_snapshot(&legacy)?;
        let new_domain = self.request_domain_snapshot(&projected)?;
        let saved: Option<(String, String, String, String, String)> = c.query_row(
            "SELECT w.request_hash,j.snapshot,j.kind,j.provider,w.stage FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",
            params![run,step_key(&legacy)], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional().map_err(|_| held())?;
        if let Some((hash, text, kind, provider, stage)) = saved {
            let snapshot: Value = serde_json::from_str(&text).map_err(|_| held())?;
            let mut domain = snapshot.clone();
            if domain
                .as_object_mut()
                .ok_or_else(held)?
                .remove("executorIdentity")
                .is_none()
                || snapshot.to_string() != text
                || sha(&text) != hash
                || kind != "text"
                || provider != s(&self.settings, "textProvider")
                || stage != legacy.stage
            {
                return Err(held());
            }
            // A supervisor handoff is executor evidence, not creative input.
            // Only its own complete persisted proof may authorize stripping it.
            if snapshot.get("sdkSupervisorHandoff").is_some() {
                domain = self
                    .sdk_handoff_existing_domain(&snapshot)?
                    .ok_or_else(held)?;
            }
            // Unknown/received/rejected/accepted states all retain the original
            // request. The ordinary execute/recovery gates own their disposition.
            let selected = if domain == old_domain {
                legacy
            } else if domain == new_domain {
                projected
            } else {
                return Err(held());
            };
            self.check_existing_api_finish_node(&selected)?;
            return Ok(selected);
        }
        // An interrupted local insertion must not become a new request simply
        // because its workflow step is missing. Match full domains, not names.
        let mut q = c.prepare("SELECT j.snapshot FROM jobs j WHERE j.kind='text' AND json_extract(j.snapshot,'$.stage')='storyboardFinish' AND NOT EXISTS(SELECT 1 FROM workflow_steps w WHERE w.text_job_id=j.id)").map_err(|_| held())?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| held())?;
        for row in rows {
            let mut domain: Value =
                serde_json::from_str(&row.map_err(|_| held())?).map_err(|_| held())?;
            domain
                .as_object_mut()
                .ok_or_else(held)?
                .remove("executorIdentity");
            if domain == old_domain || domain == new_domain {
                return Err(held());
            }
        }
        Ok(projected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn fixture() -> (
        tempfile::TempDir,
        SavedTextExecutor,
        TextRequest,
        TextRequest,
    ) {
        fixture_at("https://invalid.example")
    }
    fn fixture_at(endpoint: &str) -> (
        tempfile::TempDir,
        SavedTextExecutor,
        TextRequest,
        TextRequest,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store
            .create("offline finisher projection", "fixture")
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        let settings = json!({"textProvider":"api","textEndpoint":endpoint,"textApiKey":"SYNTHETIC_TEST_KEY_NOT_A_CREDENTIAL"});
        let source_sha = crate::pipeline::digest("甲。乙。");
        let input = json!({"storyboardAssetInputProfile":"asset-production-card/v2","settings":settings,"source":{"sha256":source_sha}});
        let run = store
            .add_job(
                "workflow",
                "storyboard.run",
                "local",
                "local",
                &json!({"input":input}),
                "partially_blocked",
            )
            .unwrap();
        let executor = SavedTextExecutor {
            store,
            settings,
            workflow_id: Some(run),
        };
        let legacy = TextRequest {
            stage: "storyboardFinish".into(),
            skill: "episode-finishing-editor".into(),
            model: "offline".into(),
            reasoning: "medium".into(),
            input: json!({"episodeId":"episode-1","sourceSha256":source_sha,"assets":[{"id":"asset-a","design":{"large":"old design"}}],"assetSelection":{"unchanged":true},"previousDraft":{"units":[{"id":"u-1"}]}}),
            schema: json!({"type":"object"}),
            rule_snapshot: json!({"primary":{"content":"unchanged rules"}}),
        };
        let mut legacy = legacy;
        legacy.input.as_object_mut().unwrap().remove("sourceSha256");
        legacy.input["assetCatalogScope"] = json!({"version":"asset-production-card/v2","sourceSha256":source_sha,"episodeId":"episode-1"});
        let mut projected = legacy.clone();
        projected.input["assets"] = json!([{"id":"asset-a","kind":"CH","subject":"甲"}]);
        projected.input["storyboardAssetInputProfile"] = json!("asset-production-card/v2");
        projected.input["finisherAssetInputProjection"] = json!({"version":POLICY,"assetInputProfile":"asset-production-card/v2","sourceSha256":source_sha,"episodeId":"episode-1","frozenWorkflowInputSha256":crate::pipeline::digest(&input.to_string()),"legacyDomainRequestSha256":crate::pipeline::digest(&serde_json::to_string(&legacy).unwrap()),"projectedAssetsSha256":crate::pipeline::digest(&projected.input["assets"].to_string()),"projectedAssetSelectionSha256":crate::pipeline::digest(&projected.input["assetSelection"].to_string()),"authorInputPreserved":true});
        (dir, executor, legacy, projected)
    }
    fn save(
        executor: &SavedTextExecutor,
        request: &TextRequest,
        state: &str,
        step: bool,
    ) -> String {
        let mut snapshot = executor.request_domain_snapshot(request).unwrap();
        snapshot["executorIdentity"] =
            json!({"provider":"api","endpoint":s(&executor.settings,"textEndpoint").trim_end_matches('/')});
        let job = executor
            .store
            .add_job("text", "fixture", "api", "local", &snapshot, state)
            .unwrap();
        if step {
            executor
                .store
                .checkpoint(
                    executor.workflow_id.as_deref().unwrap(),
                    &step_key(request),
                    &request.stage,
                    &crate::pipeline::digest(&snapshot.to_string()),
                    state,
                    Some(&job),
                    None,
                    None,
                )
                .unwrap();
        }
        job
    }

    fn complete(executor: &SavedTextExecutor, request: &TextRequest, job: &str) -> Value {
        let value = json!({"savedAcceptedValue":"SYNTHETIC_ORIGINAL_EDITOR_RESULT"});
        let snapshot = executor.request_snapshot(request).unwrap();
        let hash = crate::pipeline::digest(&snapshot.to_string());
        executor.store.update_job(job,"completed",&json!({"result":value,
            "receipt":{"contract":"desktop-runner/v1","validation":"accepted"},"error":null})).unwrap();
        executor.store.checkpoint(executor.workflow_id.as_deref().unwrap(),&step_key(request),
            &request.stage,&hash,"accepted",Some(job),Some(&value),None).unwrap();
        executor.store.event(job,"text_received_raw",&json!({"requestHash":hash,"value":value})).unwrap();
        value
    }

    fn evidence(executor: &SavedTextExecutor) -> Value {
        let c = executor.store.conn().unwrap();
        let mut steps = c.prepare("SELECT workflow_id,node_key,stage,request_hash,state,text_job_id,result,error,updated_at FROM workflow_steps ORDER BY rowid").unwrap();
        let steps: Vec<_> = steps.query_map([],|row|Ok(json!([
            row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,
            row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,Option<String>>(5)?,
            row.get::<_,Option<String>>(6)?,row.get::<_,Option<String>>(7)?,row.get::<_,String>(8)?
        ]))).unwrap().collect::<std::result::Result<_,_>>().unwrap();
        let mut events = c.prepare("SELECT id,job_id,type,data,created_at FROM events ORDER BY id").unwrap();
        let events: Vec<_> = events.query_map([],|row|Ok(json!([
            row.get::<_,i64>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,String>(2)?,
            row.get::<_,String>(3)?,row.get::<_,String>(4)?
        ]))).unwrap().collect::<std::result::Result<_,_>>().unwrap();
        json!({"jobs":executor.store.jobs().unwrap(),"steps":steps,"events":events})
    }

    /// Loopback only; every attempted request is counted and rejected. The guard
    /// must return before even this synthetic API transport is contacted.
    struct RefusingApi {
        endpoint: String,
        requests: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl RefusingApi {
        fn start() -> Self {
            let listener = std::net::TcpListener::bind(("127.0.0.1",0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = format!("http://{}",listener.local_addr().unwrap());
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let counted = requests.clone();let stopping = stop.clone();
            let worker = std::thread::spawn(move|| {
                while !stopping.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream,_)) => {
                            counted.fetch_add(1,Ordering::SeqCst);
                            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(50)));
                            let mut buffer = [0u8;4096];let _ = stream.read(&mut buffer);
                            let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                        }
                        Err(error) if error.kind()==std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self{endpoint,requests,stop,worker:Some(worker)}
        }
    }
    impl Drop for RefusingApi {
        fn drop(&mut self) {
            self.stop.store(true,Ordering::SeqCst);
            if let Some(worker)=self.worker.take(){let _=worker.join();}
        }
    }
    #[test]
    fn new_finisher_uses_bound_cards_without_creating_jobs_or_changing_parent() {
        let (_dir, e, old, new) = fixture();
        let before = e.store.job(e.workflow_id.as_deref().unwrap()).unwrap();
        let selected = e.saved_finish_asset_projection(old, new.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(selected).unwrap(),
            serde_json::to_value(new).unwrap()
        );
        assert_eq!(
            e.store.job(e.workflow_id.as_deref().unwrap()).unwrap(),
            before
        );
        assert_eq!(
            e.store
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn saved_legacy_and_projected_nodes_keep_exact_requests_including_unknown() {
        for projected in [false, true] {
            for state in ["submission_unknown", "received", "rejected", "completed"] {
                let (_dir, e, old, new) = fixture();
                let expected = if projected { &new } else { &old };
                let job = save(&e, expected, state, true);
                if state == "completed" { complete(&e,expected,&job); }
                let before = e.store.job(&job).unwrap();
                let selected = e.saved_finish_asset_projection(old.clone(), new.clone());
                if state == "completed" {
                    assert_eq!(serde_json::to_value(selected.unwrap()).unwrap(),serde_json::to_value(expected).unwrap());
                } else {
                    assert_eq!(selected.unwrap_err(),API_EXISTING_NODE_HELD,"{state} remains a held original API node");
                }
                assert_eq!(e.store.job(&job).unwrap(), before);
            }
        }
    }
    #[test]
    fn changed_rules_source_previous_draft_and_proof_cannot_change_frozen_nodes() {
        for field in ["rules", "source", "draft", "proof", "parent"] {
            let (_dir, e, old, mut new) = fixture();
            save(&e, &old, "submission_unknown", true);
            match field {
                "rules" => new.rule_snapshot = json!({"changed":true}),
                "source" => new.input["sourceSha256"] = json!("changed"),
                "draft" => new.input["previousDraft"] = json!({"changed":true}),
                "proof" => {
                    new.input["finisherAssetInputProjection"]["projectedAssetsSha256"] =
                        json!("forged")
                }
                _ => {
                    e.store.conn().unwrap().execute("UPDATE jobs SET snapshot=json_set(snapshot,'$.input.settings.textEndpoint','changed') WHERE id=?1",[e.workflow_id.as_deref().unwrap()]).unwrap();
                }
            }
            assert!(
                e.saved_finish_asset_projection(old, new).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn orphan_request_and_changed_existing_request_hold_without_new_dispatch() {
        let (_dir, e, old, new) = fixture();
        save(&e, &old, "submission_unknown", false);
        assert!(e.saved_finish_asset_projection(old, new).is_err());
        let (_dir, e, old, new) = fixture();
        let mut changed = old.clone();
        changed.model = "changed".into();
        save(&e, &changed, "failed", true);
        assert!(e.saved_finish_asset_projection(old, new).is_err());
    }

    #[tokio::test]
    async fn api_existing_unprocessed_finisher_execute_holds_without_dispatch_or_mutation() {
        let server = RefusingApi::start();
        for projected in [false,true] {
            for state in ["queued","running","submitting","submission_unknown","received","interrupted","rejected","failed"] {
                let (_dir,e,old,new)=fixture_at(&server.endpoint);
                let request=if projected {&new}else{&old};
                let job=save(&e,request,state,true);
                e.store.update_job(&job,state,&json!({"result":{"originalRaw":"SYNTHETIC_RAW_DO_NOT_REPLACE"},"prepared":{"originalPrepared":"SYNTHETIC_PREPARED_DO_NOT_REPLACE"}})).unwrap();
                let before=evidence(&e);
                assert_eq!(e.saved_finish_asset_projection(old.clone(),new.clone()).unwrap_err(),API_EXISTING_NODE_HELD,"selector {state}");
                assert_eq!(e.execute(request.clone()).await.unwrap_err(),API_EXISTING_NODE_HELD,"execute {state}");
                assert_eq!(evidence(&e),before,"{state}: jobs/checkpoint/prepared/raw/events unchanged");
                assert_eq!(server.requests.load(Ordering::SeqCst),0,"{state}: no loopback API dispatch");
                assert!(!e.store.root.join("text-runs").exists());
            }
        }
    }

    #[tokio::test]
    async fn api_completed_finisher_reuses_only_original_accepted_cache_without_dispatch() {
        let server=RefusingApi::start();
        for projected in [false,true] {
            let (_dir,e,old,new)=fixture_at(&server.endpoint);
            let request=if projected {&new}else{&old};
            let job=save(&e,request,"received",true);
            let value=complete(&e,request,&job);
            let before=evidence(&e);
            let selected=e.saved_finish_asset_projection(old.clone(),new.clone()).unwrap();
            assert_eq!(serde_json::to_value(&selected).unwrap(),serde_json::to_value(request).unwrap());
            for _ in 0..2 {assert_eq!(e.execute(selected.clone()).await.unwrap(),value);}
            assert_eq!(evidence(&e),before);
            assert_eq!(server.requests.load(Ordering::SeqCst),0);
            assert!(!e.store.root.join("text-runs").exists());
        }
    }

    #[tokio::test]
    async fn api_completed_finisher_unproven_cache_holds_without_dispatch_or_mutation() {
        let server=RefusingApi::start();
        for changed in ["missing-receipt","missing-event","changed-event-hash","changed-checkpoint-hash","changed-result","changed-identity","unaccepted-step","changed-domain"] {
            let (_dir,e,mut request,_new)=fixture_at(&server.endpoint);
            let job=save(&e,&request,"received",true);complete(&e,&request,&job);
            let c=e.store.conn().unwrap();
            match changed {
                "missing-receipt" => {c.execute("UPDATE jobs SET receipt=NULL WHERE id=?1",[&job]).unwrap();}
                "missing-event" => {c.execute("DELETE FROM events WHERE job_id=?1 AND type='text_received_raw'",[&job]).unwrap();}
                "changed-event-hash" => {c.execute("UPDATE events SET data=json_set(data,'$.requestHash','changed') WHERE job_id=?1 AND type='text_received_raw'",[&job]).unwrap();}
                "changed-checkpoint-hash" => {c.execute("UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",[&job]).unwrap();}
                "changed-result" => {c.execute("UPDATE jobs SET result='{}' WHERE id=?1",[&job]).unwrap();}
                "changed-identity" => {c.execute("UPDATE jobs SET snapshot=json_set(snapshot,'$.executorIdentity.endpoint','http://invalid.example') WHERE id=?1",[&job]).unwrap();}
                "unaccepted-step" => {c.execute("UPDATE workflow_steps SET state='running' WHERE text_job_id=?1",[&job]).unwrap();}
                _ => {request.input["previousDraft"]=json!({"changed":"not original accepted draft"});}
            }
            drop(c);
            let before=evidence(&e);
            assert_eq!(e.execute(request).await.unwrap_err(),API_EXISTING_NODE_HELD,"{changed}");
            assert_eq!(evidence(&e),before,"{changed}: no mutation");
            assert_eq!(server.requests.load(Ordering::SeqCst),0,"{changed}: no dispatch");
        }
    }

    #[test]
    fn api_finisher_hold_does_not_apply_to_other_backends_or_stages() {
        let (_dir,mut e,mut request,_new)=fixture();
        save(&e,&request,"submission_unknown",true);
        for provider in ["codex","codex_sdk"] {
            e.settings["textProvider"]=json!(provider);
            assert!(e.check_existing_api_finish_node(&request).is_ok());
        }
        e.settings["textProvider"]=json!("api");request.stage="storyboardCreate".into();
        assert!(e.check_existing_api_finish_node(&request).is_ok());
        request.stage="storyboardFinish".into();e.workflow_id=None;
        assert!(e.check_existing_api_finish_node(&request).is_ok());
    }
}
