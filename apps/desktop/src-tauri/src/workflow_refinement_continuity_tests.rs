use super::*;
use crate::pipeline::{self, storyboard_refinement::tests::Model, TextExecutor, TextRequest};

struct DurableReviewModel {
    inner: Model,
    store: Store,
    run: String,
}
#[async_trait::async_trait]
impl TextExecutor for DurableReviewModel {
    async fn execute(&self, request: TextRequest) -> Result<Value> {
        let mut raw = self.inner.execute(request.clone()).await?;
        raw["replacements"][0]["units"][0]["exitState"] = json!(format!(
            "原文动作已收束，{}",
            s(&raw["replacements"][0]["units"][0], "exitState")
        ));
        let snapshot = serde_json::to_value(&request).unwrap();
        let key = sha(&snapshot);
        let job = self.store.add_job(
            "text",
            "storyboardRefine",
            "codex",
            "offline",
            &snapshot,
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
        let key = sha(&serde_json::to_value(request).unwrap());
        let job: String = self
            .store
            .conn()?
            .query_row(
                "SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1 AND request_hash=?2",
                params![self.run, key],
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

pub(crate) async fn fixture() -> (tempfile::TempDir, Store, Value, String, Value) {
    let (dir, store, input, _origin) = pipeline::storyboard_refinement::tests::fixture().await;
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.refine","input":input,"savedSettingsSha256":pipeline::digest(&store.settings().unwrap().to_string()),"guard":guard_from_input(&input)});
    let run = store
        .add_job(
            "workflow",
            "storyboard.refine",
            "local",
            "offline",
            &snapshot,
            "running",
        )
        .unwrap();
    let mut inner = Model::default();
    inner.mode = "changed";
    let model = DurableReviewModel {
        inner,
        store: store.clone(),
        run: run.clone(),
    };
    let candidate = pipeline::run_storyboard_refinement(&input, &model, "offline", "medium")
        .await
        .unwrap();
    store
        .update_job(&run, "completed", &json!({"result":candidate}))
        .unwrap();
    store
        .publish_storyboard_refinement(&run, s(&candidate, "candidateSha256"), &input)
        .unwrap();
    assert_eq!(
        store
            .values("units")
            .unwrap()
            .iter()
            .filter(|u| u["continuityStale"] == true)
            .count(),
        2
    );
    (dir, store, input, run, candidate)
}
pub(crate) fn review(plan: &Value) -> Value {
    let frontier = &plan["frontiers"][0];
    json!({"version":REVIEW,"planSha256":plan["sha256"],"reviewer":"authorized_director_review","frontierReviews":[{"fromUnitId":frontier["fromUnitId"],"nextUnitId":frontier["nextUnitId"],"decision":"compatible","reason":"已对照原文及采用前后出口，下一单元仍可按原入口推进；后续出口逐字保留。","sourceRefs":[frontier["fromSourceRefs"][0],frontier["nextSourceRefs"][0]]}]})
}
fn raw_job(store: &Store, run: &str) -> (String, String, String) {
    store
        .conn()
        .unwrap()
        .query_row(
            "SELECT snapshot,result,prepared FROM jobs WHERE id=?1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

#[tokio::test]
async fn formal_review_closes_only_exclusive_metadata_fallout_and_reopens_with_original_creative_receipts(
) {
    let (dir, store, input, run, candidate) = fixture().await;
    let before = raw_job(&store, &run);
    let old_adoption = parse(before.2.clone()).unwrap()["refinementAdoption"].clone();
    let units = store.values("units").unwrap();
    let jobs = store.jobs().unwrap().len();
    let plan = store
        .preview_refinement_continuity(&run, s(&candidate, "candidateSha256"))
        .unwrap();
    let stale = units.iter().find(|u| u["continuityStale"] == true).unwrap();
    let bound = crate::binding::bind(&store, stale, "jimeng", "seedance2.5").unwrap();
    assert!(bound["problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["code"] == "CONTINUITY_REVISION_REQUIRED"));
    assert_eq!(plan["automaticCompatibility"], false);
    assert_eq!(plan["affectedUnitBindings"].as_array().unwrap().len(), 2);
    assert_eq!(raw_job(&store, &run), before, "preview is read-only");
    let proof = store
        .adopt_refinement_continuity(&run, s(&candidate, "candidateSha256"), &review(&plan))
        .unwrap();
    let reopened = Store::open(dir.path().join("isolated-source")).unwrap();
    let after = raw_job(&reopened, &run);
    assert_eq!(after.0, before.0);
    assert_eq!(after.1, before.1);
    assert_eq!(parse(after.2).unwrap()["refinementAdoption"], old_adoption);
    let current = reopened.values("units").unwrap();
    for unit in &current {
        let bound = crate::binding::bind(&reopened, unit, "jimeng", "seedance2.5").unwrap();
        assert!(bound["problems"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["code"] != "CONTINUITY_REVISION_REQUIRED"));
    }
    for old in &units {
        let new = current.iter().find(|u| u["id"] == old["id"]).unwrap();
        assert_eq!(no_derived_metadata(old), no_derived_metadata(new));
        if old["continuityStale"] == true {
            assert_eq!(new["continuityStale"], false);
            assert_eq!(
                new["version"].as_u64(),
                old["version"].as_u64().map(|v| v + 1)
            );
        } else {
            assert_eq!(old, new);
        }
    }
    let mut next = input;
    next["existingUnits"] = json!(current);
    verify_history_for_input(&reopened.conn().unwrap(), &next).unwrap();
    assert_eq!(reopened.jobs().unwrap().len(), jobs);
    assert_eq!(proof["modelCalls"], 0);
    let stored = reopened.job(&run).unwrap();
    let event_count:i64=reopened.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='storyboard_refinement_continuity_review_adopted'",[&run],|r|r.get(0)).unwrap();
    assert_eq!(
        reopened
            .adopt_refinement_continuity(&run, s(&candidate, "candidateSha256"), &review(&plan))
            .unwrap(),
        proof
    );
    assert_eq!(reopened.job(&run).unwrap(), stored);
    assert_eq!(event_count, 1);
}

#[tokio::test]
async fn missing_negative_or_stale_review_never_clears_flags_or_mutates_the_old_record() {
    for mode in [
        "none",
        "needs_revision",
        "reason",
        "unknown-source",
        "wrong-plan",
        "graph",
        "body",
        "other-cause",
        "raw",
        "adoption",
        "source",
        "asset",
    ] {
        let (_dir, store, _input, run, candidate) = fixture().await;
        let plan = store
            .preview_refinement_continuity(&run, s(&candidate, "candidateSha256"))
            .unwrap();
        let mut decision = review(&plan);
        match mode {
            "none" => decision = json!({}),
            "needs_revision" => {
                decision["frontierReviews"][0]["decision"] = json!("needs_revision")
            }
            "reason" => decision["frontierReviews"][0]["reason"] = json!(""),
            "unknown-source" => decision["frontierReviews"][0]["sourceRefs"] = json!(["fake"]),
            "wrong-plan" => decision["planSha256"] = json!("wrong"),
            "graph" => {
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "DELETE FROM unit_dependencies WHERE unit_id=?1",
                        [s(&plan["frontiers"][0], "nextUnitId")],
                    )
                    .unwrap();
            }
            "body" => {
                let mut u = store
                    .get("units", s(&plan["affectedUnitBindings"][0], "unitId"))
                    .unwrap();
                u["prompt"] = json!("独立人工修改");
                store.save_unit(u).unwrap();
            }
            "other-cause" => {
                let mut u = store
                    .get("units", s(&plan["frontiers"][0], "fromUnitId"))
                    .unwrap();
                u["exitState"] = json!("另一修改原因");
                store.save_unit(u).unwrap();
                store
                    .invalidate_unit_dependents(s(&plan["frontiers"][0], "fromUnitId"))
                    .unwrap();
            }
            "raw" => {
                let mut v = candidate.clone();
                v["rawDelta"]["extra"] = json!(true);
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE jobs SET result=?1 WHERE id=?2",
                        params![v.to_string(), run],
                    )
                    .unwrap();
            }
            "source" => {
                store
                    .save_source(json!(pipeline::prepare_source(
                        "第一集\n场景：门口\n△小王停下。\n"
                    )
                    .unwrap()))
                    .unwrap();
            }
            "asset" => {
                store
                    .save_asset(
                        json!({"id":"new-asset","kind":"prop","name":"新物件","variantId":"base"}),
                    )
                    .unwrap();
            }
            _ => {
                let mut p = parse(raw_job(&store, &run).2).unwrap();
                p["refinementAdoption"]["extra"] = json!(true);
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE jobs SET prepared=?1 WHERE id=?2",
                        params![p.to_string(), run],
                    )
                    .unwrap();
            }
        }
        let before = raw_job(&store, &run);
        let units = store.values("units").unwrap();
        let count = store.jobs().unwrap().len();
        assert!(
            store
                .adopt_refinement_continuity(&run, s(&candidate, "candidateSha256"), &decision)
                .is_err(),
            "{mode}"
        );
        assert_eq!(raw_job(&store, &run), before, "{mode}");
        assert_eq!(store.values("units").unwrap(), units, "{mode}");
        assert_eq!(store.jobs().unwrap().len(), count);
    }
}

#[tokio::test]
async fn metadata_lineage_rejects_receipt_event_and_historical_version_tampering() {
    for mode in [
        "receipt",
        "missing-receipt",
        "event",
        "duplicate-event",
        "version",
    ] {
        let (_dir, store, mut input, run, candidate) = fixture().await;
        let plan = store
            .preview_refinement_continuity(&run, s(&candidate, "candidateSha256"))
            .unwrap();
        store
            .adopt_refinement_continuity(&run, s(&candidate, "candidateSha256"), &review(&plan))
            .unwrap();
        match mode {
            "missing-receipt" => {
                let mut p = parse(raw_job(&store, &run).2).unwrap();
                p.as_object_mut().unwrap().remove(FIELD);
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE jobs SET prepared=?1 WHERE id=?2",
                        params![p.to_string(), run],
                    )
                    .unwrap();
            }
            "duplicate-event" => {
                store.conn().unwrap().execute("INSERT INTO events(job_id,type,data,created_at) SELECT job_id,type,data,created_at FROM events WHERE job_id=?1 AND type='storyboard_refinement_continuity_review_adopted'", [&run]).unwrap();
            }
            "receipt" => {
                let mut p = parse(raw_job(&store, &run).2).unwrap();
                p[FIELD]["review"]["frontierReviews"][0]["reason"] = json!("tampered");
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE jobs SET prepared=?1 WHERE id=?2",
                        params![p.to_string(), run],
                    )
                    .unwrap();
            }
            "event" => {
                store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='storyboard_refinement_continuity_review_adopted'",[&run]).unwrap();
            }
            _ => {
                let id = s(&plan["affectedUnitBindings"][0], "unitId");
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE unit_versions SET data='{}' WHERE unit_id=?1 AND version=?2",
                        params![
                            id,
                            plan["affectedUnitBindings"][0]["unitVersion"]
                                .as_i64()
                                .unwrap()
                        ],
                    )
                    .unwrap();
            }
        }
        input["existingUnits"] = json!(store.values("units").unwrap());
        assert!(
            verify_history_for_input(&store.conn().unwrap(), &input).is_err(),
            "{mode}"
        );
    }
}

#[test]
#[ignore = "explicit read-only captured project preflight; no DB write or generation"]
fn readonly_captured_refinement_continuity_preflight() {
    let path =
        std::env::var("DIRECTOR_CONTINUITY_READONLY_CAPTURE").expect("captured project.sqlite");
    let path = std::fs::canonicalize(path).unwrap();
    let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));
    assert!(
        !wal.exists() || std::fs::metadata(wal).unwrap().len() == 0,
        "immutable preflight requires a standalone checkpointed capture"
    );
    let mut uri = reqwest::Url::from_file_path(&path).unwrap();
    uri.query_pairs_mut().append_pair("immutable", "1");
    let c = Connection::open_with_flags(
        uri.as_str(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    let run = std::env::var("DIRECTOR_CONTINUITY_WORKFLOW").unwrap();
    let candidate = std::env::var("DIRECTOR_CONTINUITY_CANDIDATE").unwrap();
    let plan = preview(&c, &run, &candidate).unwrap();
    assert_eq!(plan["affectedUnitBindings"].as_array().unwrap().len(), 10);
    if let Ok(path) = std::env::var("DIRECTOR_CONTINUITY_PREVIEW_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    }
    println!(
        "{}",
        json!({"planSha256":plan["sha256"],"affected":10,"automaticCompatibility":false,"databaseReadOnly":true,"modelCalls":0})
    );
}
