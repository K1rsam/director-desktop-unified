//! All stores, model deltas and receipts in this module are synthetic.
use super::*;
use crate::pipeline::{self, asset_use_binding as binding, TextExecutor, TextRequest};

struct Model {
    inner: pipeline::storyboard_refinement::tests::Model,
    store: Store,
    run: String,
}
impl Model {
    fn new(store: &Store, run: &str, mode: &'static str) -> Self {
        let mut inner = pipeline::storyboard_refinement::tests::Model::default();
        inner.mode = mode;
        Self {
            inner,
            store: store.clone(),
            run: run.into(),
        }
    }
}
#[async_trait::async_trait]
impl TextExecutor for Model {
    async fn execute(&self, request: TextRequest) -> Result<Value> {
        let key = pipeline::digest(&serde_json::to_value(&request).unwrap().to_string());
        if let Some(value) = self.store.accepted_step(&self.run, &key)? {
            return Ok(value);
        }
        let mut raw = self.inner.execute(request.clone()).await?;
        if request.stage == "storyboardCreate" {
            for unit in raw["units"].as_array_mut().unwrap() {
                unit["assetRefs"] = json!([{"assetId":"actor","variantId":"base","use":"character","order":1,"useShotIds":[unit["shots"][0]["id"]]}]);
            }
        }
        for group in raw.get_mut("replacements").and_then(Value::as_array_mut).into_iter().flatten() {
            for unit in group["units"].as_array_mut().into_iter().flatten() {
                unit.as_object_mut().unwrap().remove(binding::FIELD);
                for reference in unit["assetRefs"].as_array_mut().into_iter().flatten() {
                    for key in ["sourceRefs", "sourceRefsPurpose", "identityEvidence"] {
                        reference.as_object_mut().unwrap().remove(key);
                    }
                }
            }
        }
        let job = self.store.add_job(
            "text",
            &request.stage,
            "codex",
            "offline-fixture",
            &serde_json::to_value(&request).unwrap(),
            "received",
        )?;
        self.store.event(
            &job,
            "text_received_raw",
            &json!({"requestHash":key,"value":raw}),
        )?;
        self.store.checkpoint(
            &self.run,
            &key,
            &request.stage,
            &key,
            "received",
            Some(&job),
            Some(&raw),
            None,
        )?;
        Ok(raw)
    }
    async fn accept(&self, request: &TextRequest, value: &Value) -> Result<()> {
        let key = pipeline::digest(&serde_json::to_value(request).unwrap().to_string());
        let job: String = self
            .store
            .conn()?
            .query_row(
                "SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1 AND request_hash=?2",
                rusqlite::params![self.run, key],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        self.store.update_job(&job,"completed",&json!({"result":value,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}}))?;
        self.store.checkpoint(
            &self.run,
            &key,
            &request.stage,
            &key,
            "accepted",
            Some(&job),
            Some(value),
            None,
        )
    }
}

async fn fixture() -> (tempfile::TempDir, Store, Value, String) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("synthetic-project")).unwrap();
    store.create("identity rebind fixture", "offline").unwrap();
    store.ensure_workflow_storage().unwrap();
    store.save_settings(&json!({"textProvider":"codex_sdk","visualStyle":"16:9真人","fantasyMode":"off","reasoning":"medium","textModel":"offline-fixture"})).unwrap();
    let source = store
        .save_source(json!(pipeline::prepare_source(
            "第一集\n场景：门口\n小王OS：“先走，别回头😀。”\n△小王推门。\n△小王走入。\n"
        )
        .unwrap()))
        .unwrap();
    let rows: Vec<_> = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "body")
        .map(|r| r["id"].clone())
        .collect();
    let mut asset=store.save_asset(json!({"id":"actor","variantId":"base","kind":"character","name":"小王","sourceRefs":rows,"sourceRefsPurpose":"SOURCE_SCOPE","prompt":"初始"})).unwrap();
    asset["prompt"] = json!("旧美术");
    store.save_asset(asset).unwrap();
    let capabilities = json!({"minDuration":0,"maxDuration":3,"imageLimit":1});
    let mut input = super::super::production_input_with_capabilities(
        &store,
        &json!({"action":"storyboard.run","episodeIds":[source["episodes"][0]["id"]]}),
        Some(&capabilities),
    )
    .unwrap();
    binding::freeze_new(&mut input).unwrap();
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":input,"guard":crate::workflow::guard_from_input(&input)});
    let origin = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "offline-fixture",
            &snapshot,
            "running",
        )
        .unwrap();
    let output = pipeline::run_storyboard(
        &input,
        &Model::new(&store, &origin, "changed"),
        "offline-fixture",
        "medium",
    )
    .await
    .unwrap();
    assert_eq!(output["status"], "completed", "{}", output["blocked"]);
    store
        .publish_storyboard(
            &crate::workflow::guard_from_input(&input),
            &output,
            &input,
            None,
        )
        .unwrap();
    store
        .update_job(&origin, "completed", &json!({"result":output}))
        .unwrap();
    let mut asset = store.get("assets", "actor").unwrap();
    asset["prompt"] = json!("三视图新美术");
    asset["design"] = json!({"description":"新设计"});
    store.save_asset(asset).unwrap();
    let mut current=super::super::refinement_input_with_capabilities(&store,&json!({"episodeId":source["episodes"][0]["id"],"reviewNotes":"只精修真实声画缺口","preserveNotes":"原话及OS保持"}),Some(&capabilities)).unwrap();
    binding::freeze_new(&mut current).unwrap();
    assert!(current.get(identity::FIELD).is_some());
    (directory, store, current, origin)
}
fn failed(store: &Store, input: &Value) -> String {
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.refine","input":input,"guard":crate::workflow::guard_from_input(input),"savedSettingsSha256":pipeline::digest(&store.settings().unwrap().to_string())});
    let run = store
        .add_job(
            "workflow",
            "storyboard.refine",
            "local",
            "offline-fixture",
            &snapshot,
            "running",
        )
        .unwrap();
    store.update_job(&run,"failed",&json!({"error":"素材使用锚或登记身份依据变化，未采用","prepared":{"retainedBudget":1}})).unwrap();
    run
}

#[tokio::test]
async fn zero_dispatch_recovery_preserves_snapshot_budget_and_same_id_after_reopen() {
    let (directory, store, mut input, _origin) = fixture().await;
    let packet = input
        .as_object_mut()
        .unwrap()
        .remove(identity::FIELD)
        .unwrap();
    let run = failed(&store, &input);
    let before = store.job(&run).unwrap();
    let units = store.values("units").unwrap();
    let count = store.jobs().unwrap().len();
    prepare_resume(&store, &run).unwrap();
    let reopened = Store::open(directory.path().join("synthetic-project")).unwrap();
    let recovered = reopened.job(&run).unwrap();
    for field in ["id", "snapshot", "result", "receipt", "error", "status"] {
        assert_eq!(recovered[field], before[field], "{field}");
    }
    assert_eq!(recovered["prepared"]["retainedBudget"], 1);
    assert_eq!(recovered["prepared"][identity::RECOVERY]["packet"], packet);
    assert_eq!(reopened.jobs().unwrap().len(), count);
    assert_eq!(reopened.values("units").unwrap(), units);
    prepare_resume(&reopened, &run).unwrap();
    assert_eq!(
        reopened.job(&run).unwrap(),
        recovered,
        "idempotent, same original workflow"
    );
    let mut effective = input.clone();
    identity::attach_recovery(
        &mut effective,
        &recovered["snapshot"],
        &recovered["prepared"],
    )
    .unwrap();
    verify(&reopened, &effective).unwrap();
    let candidate = pipeline::run_storyboard_refinement(
        &effective,
        &Model::new(&reopened, &run, "changed"),
        "offline-fixture",
        "medium",
    )
    .await
    .unwrap();
    reopened
        .update_job(&run, "completed", &json!({"result":candidate}))
        .unwrap();
    let stored_candidate:String=reopened.conn().unwrap().query_row("SELECT result FROM jobs WHERE id=?1",[&run],|r|r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&stored_candidate).unwrap(),candidate);
    reopened
        .publish_storyboard_refinement(&run, s(&candidate, "candidateSha256"), &effective)
        .unwrap();
    let after = reopened.job(&run).unwrap();
    assert_eq!(after["snapshot"], before["snapshot"]);
    let saved=Store::open(reopened.root.clone()).unwrap();
    let stored_after:String=saved.conn().unwrap().query_row("SELECT result FROM jobs WHERE id=?1",[&run],|r|r.get(0)).unwrap();
    assert_eq!(stored_after,stored_candidate,"raw database candidate remains byte-exact; UI mediaIds projection is separate");
    assert_eq!(
        after["prepared"]["refinementAdoption"][identity::FIELD],
        packet
    );
    let retained = &units[1];
    assert_eq!(reopened.get("units", s(retained, "id")).unwrap(), *retained);
    assert_eq!(
        retained["assetRefs"][0]["identityEvidence"]["assetVersion"],
        2
    );
    let mut next = effective.clone();
    next["existingUnits"] = after["prepared"]["refinementAdoption"]["adoptedUnits"].clone();
    let ids: Vec<_> = next["existingUnits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["id"].clone())
        .collect();
    let contexts =
        super::super::accepted_storyboard_revision_contexts(&reopened, &next, &ids).unwrap();
    assert_eq!(
        contexts[0]["unitBindings"].as_array().unwrap().len(),
        3,
        "historical lineage reopens with explicit receipt"
    );
}

#[tokio::test]
async fn origin_snapshot_drift_and_non_art_asset_changes_fail_before_any_model_dispatch() {
    let (_directory, store, input, origin) = fixture().await;
    verify(&store, &input).unwrap();
    let mut origin_snapshot = store.job(&origin).unwrap()["snapshot"].clone();
    origin_snapshot["changedOutsideInput"] = json!(true);
    store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET snapshot=?1 WHERE id=?2",
            rusqlite::params![origin_snapshot.to_string(), origin],
        )
        .unwrap();
    assert!(
        verify(&store, &input).is_err(),
        "authoritative original snapshot SHA is freshly checked"
    );
    let mut asset = store.get("assets", "actor").unwrap();
    asset["name"] = json!("另一个人");
    store.save_asset(asset).unwrap();
    assert!(super::super::refinement_input_with_capabilities(
        &store,
        &json!({"episodeId":input["episodeId"]}),
        Some(&input["capabilities"])
    )
    .is_err());
}

#[tokio::test]
async fn existing_text_attempt_or_fact_drift_cannot_upgrade_an_old_failed_request() {
    for mode in ["step", "text-owner", "attempt", "asset-drift", "unit-drift"] {
        let (_directory, store, mut input, _origin) = fixture().await;
        input.as_object_mut().unwrap().remove(identity::FIELD);
        let run = failed(&store, &input);
        match mode {
            "step" => {
                store
                    .checkpoint(
                        &run,
                        "old-node",
                        "storyboardRefine",
                        "old-hash",
                        "rejected",
                        None,
                        None,
                        Some("old rejection"),
                    )
                    .unwrap();
            }
            "text-owner" => {
                store
                    .add_job(
                        "text",
                        "storyboardRefine",
                        "codex_sdk",
                        "offline",
                        &json!({"workflowId":run}),
                        "submission_unknown",
                    )
                    .unwrap();
            }
            "attempt" => {
                let request = TextRequest {
                    stage: "storyboardRefine".into(),
                    skill: "director-refinement".into(),
                    model: "offline".into(),
                    reasoning: "medium".into(),
                    input: json!({}),
                    schema: json!({}),
                    rule_snapshot: json!({}),
                };
                let attempt = crate::text_transport::FrozenAttempt::prepare(
                    "attempt",
                    &run,
                    "node",
                    crate::text_transport::Backend::CodexSdk,
                    request,
                    &json!({}),
                )
                .unwrap();
                let path = store
                    .root
                    .join("text-runs")
                    .join("attempt")
                    .join("sdk-attempt.json");
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                attempt.persist(&path).unwrap();
            }
            "asset-drift" => {
                let mut a = store.get("assets", "actor").unwrap();
                a["prompt"] = json!("later art edit");
                store.save_asset(a).unwrap();
            }
            _ => {
                let mut u = store.values("units").unwrap()[0].clone();
                u["title"] = json!("later adopted edit");
                store.save_unit(u).unwrap();
            }
        }
        let before = store.job(&run).unwrap();
        assert!(prepare_resume(&store, &run).is_err(), "{mode}");
        assert_eq!(store.job(&run).unwrap(), before, "{mode}");
    }
}
