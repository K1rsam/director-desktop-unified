use super::*;
use rusqlite::Connection;

struct Fixture {
    connection: Connection,
    request: TextRequest,
    snapshot: Value,
    job: Value,
    raw: Value,
}
fn fixture() -> Fixture {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE jobs(id TEXT PRIMARY KEY,kind TEXT,provider TEXT,state TEXT,snapshot TEXT,result TEXT,prepared TEXT,error TEXT,receipt TEXT);CREATE TABLE workflow_steps(workflow_id TEXT,node_key TEXT,stage TEXT,request_hash TEXT,state TEXT,text_job_id TEXT,result TEXT,error TEXT);CREATE TABLE events(id INTEGER PRIMARY KEY AUTOINCREMENT,job_id TEXT,type TEXT,data TEXT);").unwrap();
    let mut manifest = json!({"version":VERSION,"stage":"F2","scope":["episode-1"],"sourceSha256":"a".repeat(64),"originalInputSha256":"b".repeat(64),"originalResultSha256":"c".repeat(64),"currentBasisSha256":"d".repeat(64),"causeSha256":"e".repeat(64),"issueSetSha256":"f".repeat(64),"conflicts":["/assets/shield/description"]});
    manifest["manifestSha256"] = json!(digest(&manifest.to_string()));
    let mut marker = json!({"version":VERSION,"attempt":2,"maximumAttempts":2});
    for key in [
        "sourceSha256",
        "originalInputSha256",
        "originalResultSha256",
        "currentBasisSha256",
        "causeSha256",
        "issueSetSha256",
        "manifestSha256",
    ] {
        marker[key] = manifest[key].clone();
    }
    let raw = json!({"version":VERSION,"manifestSha256":manifest["manifestSha256"],"decisions":[{"conflictId":"conflict-1","action":"adopt_current"}]});
    let schema = json!({"type":"object","additionalProperties":false,"required":["version","manifestSha256","decisions"],"properties":{"version":{"type":"string","enum":[VERSION]},"manifestSha256":{"type":"string","enum":[manifest["manifestSha256"]]},"decisions":{"type":"array","minItems":1,"maxItems":1,"items":{"type":"object","additionalProperties":false,"required":["conflictId","action"],"properties":{"conflictId":{"type":"string","enum":["conflict-1"]},"action":{"type":"string","enum":["adopt_current"]}}}}}});
    let request = TextRequest {
        stage: "foundation:F2".into(),
        skill: "foundation-asset-catalog".into(),
        model: "test-model".into(),
        reasoning: "medium".into(),
        input: json!({"scope":["episode-1"],"sourceSha256":"a".repeat(64),"foundationConflictManifest":manifest,"foundationConflictResolution":marker}),
        schema,
        rule_snapshot: json!({"id":"complete-frozen-rule-fixture","sha256":"1".repeat(64)}),
    };
    let snapshot = json!(request);
    let raw_text = format!("  {}\n", raw);
    let prepared = json!({"sdkRawTextSha256":digest(&raw_text)});
    connection
        .execute(
            "INSERT INTO jobs VALUES('job','text','codex_sdk','received',?1,?2,?3,NULL,NULL)",
            params![snapshot.to_string(), raw.to_string(), prepared.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflow_steps VALUES('run','node','foundation:F2',?1,'running','job',NULL,NULL)",
            [digest(&snapshot.to_string())],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO events(job_id,type,data)VALUES('job','sdk_raw_result',?1)",
            [json!({"rawText":raw_text,"parsedJson":raw,"parseError":null}).to_string()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO events(job_id,type,data)VALUES('job','text_received_raw',?1)",
            [json!({"requestHash":digest(&snapshot.to_string()),"value":raw}).to_string()],
        )
        .unwrap();
    let job = json!({"id":"job","provider":"codex_sdk","status":"received","prepared":prepared});
    Fixture { connection, request, snapshot, job, raw }
}
fn transition(tx: &Transaction<'_>, job: &Value, value: &Value, reason: Option<&str>) {
    let mut prepared = job["prepared"].clone();
    if reason.is_none() {
        prepared["sdkDomainResult"] = value.clone();
        prepared["sdkDomainResultSha256"] = json!(digest(&value.to_string()));
    }
    tx.execute(
        "UPDATE jobs SET state=?1,error=?2,receipt=?3,prepared=?4 WHERE id='job'",
        params![
            if reason.is_some() { "rejected" } else { "completed" },
            reason,
            json!({"contract":"desktop-runner/v1","validation":if reason.is_some(){"rejected"}else{"accepted"}})
                .to_string(),
            prepared.to_string()
        ],
    )
    .unwrap();
    tx.execute(
        "UPDATE workflow_steps SET state=?1,result=?2,error=?3 WHERE text_job_id='job'",
        params![if reason.is_some() { "rejected" } else { "accepted" }, value.to_string(), reason],
    )
    .unwrap();
}
fn count(c: &Connection, table: &str) -> i64 {
    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

#[test]
fn new_accepted_callback_archives_exact_native_raw_candidate_and_only_mechanical_validation() {
    let mut f = fixture();
    let tx = f.connection.transaction().unwrap();
    transition(&tx, &f.job, &f.raw, None);
    let receipt = record_callback(&tx, &f.request, &f.snapshot, &f.job, Some("run"), "node", &f.raw, &f.raw, None)
        .unwrap()
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(receipt["accepted"], true);
    assert_eq!(receipt["adopted"], false);
    assert_eq!(count(&f.connection, "workflow_artifacts_v1"), 2);
    assert_eq!(count(&f.connection, "workflow_validations_v1"), 1);
    assert_eq!(count(&f.connection, "workflow_adoptions_v1"), 0);
    assert_eq!(count(&f.connection, "workflow_consumptions_v1"), 0);
    let raw = artifacts::artifact(&f.connection, s(&receipt, "rawArtifactId")).unwrap().unwrap();
    assert_eq!(raw.artifact.payload["sdkRawResult"]["rawText"], format!("  {}\n", f.raw));
    assert_eq!(raw.artifact.payload["received"], f.raw);
    let validation: String =
        f.connection.query_row("SELECT data FROM workflow_validations_v1", [], |r| r.get(0)).unwrap();
    let validation: Value = serde_json::from_str(&validation).unwrap();
    assert_eq!(validation["capability"], "structure");
    assert_eq!(validation["findings"]["contentReviewPerformed"], false);
    assert_eq!(validation["findings"]["visualReviewPerformed"], false);
}

#[test]
fn new_rejected_callback_retains_negative_validation_and_does_not_adopt() {
    let mut f = fixture();
    let tx = f.connection.transaction().unwrap();
    transition(&tx, &f.job, &f.raw, Some("实际领域校验拒绝"));
    let receipt = record_callback(
        &tx,
        &f.request,
        &f.snapshot,
        &f.job,
        Some("run"),
        "node",
        &f.raw,
        &f.raw,
        Some("实际领域校验拒绝"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(receipt["accepted"], false);
    tx.commit().unwrap();
    assert_eq!(count(&f.connection, "workflow_artifacts_v1"), 2);
    assert_eq!(count(&f.connection, "workflow_adoptions_v1"), 0);
}

#[test]
fn artifact_failure_rolls_back_job_checkpoint_and_partial_artifacts_together() {
    let mut f = fixture();
    {
        let tx = f.connection.transaction().unwrap();
        artifacts::install(&tx).unwrap();
        tx.commit().unwrap();
    }
    f.connection.execute_batch("CREATE TEMP TRIGGER fail_new_validation BEFORE INSERT ON workflow_validations_v1 BEGIN SELECT RAISE(ABORT,'injected artifact failure'); END;").unwrap();
    {
        let tx = f.connection.transaction().unwrap();
        transition(&tx, &f.job, &f.raw, None);
        assert!(
            record_callback(&tx, &f.request, &f.snapshot, &f.job, Some("run"), "node", &f.raw, &f.raw, None).is_err()
        );
    }
    assert_eq!(
        f.connection.query_row("SELECT state FROM jobs WHERE id='job'", [], |r| r.get::<_, String>(0)).unwrap(),
        "received"
    );
    assert_eq!(
        f.connection
            .query_row("SELECT state FROM workflow_steps WHERE text_job_id='job'", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "running"
    );
    assert_eq!(count(&f.connection, "workflow_artifacts_v1"), 0);
    assert_eq!(count(&f.connection, "workflow_validations_v1"), 0);
}

#[test]
fn old_requests_and_old_completed_nodes_are_never_back_signed() {
    let mut f = fixture();
    let tx = f.connection.transaction().unwrap();
    let mut old = f.request.clone();
    old.input.as_object_mut().unwrap().remove("foundationConflictResolution");
    assert!(record_callback(&tx, &old, &f.snapshot, &f.job, Some("run"), "node", &f.raw, &f.raw, None)
        .unwrap()
        .is_none());
    let mut old_job = f.job.clone();
    old_job["status"] = json!("completed");
    assert!(record_callback(&tx, &f.request, &f.snapshot, &old_job, Some("run"), "node", &f.raw, &f.raw, None).is_err());
    assert_eq!(
        tx.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='workflow_artifacts_v1'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn wrong_native_hash_or_checkpoint_aborts_before_artifact_install() {
    for change in ["native", "node"] {
        let mut f = fixture();
        let tx = f.connection.transaction().unwrap();
        transition(&tx, &f.job, &f.raw, None);
        if change == "native" {
            tx.execute("UPDATE events SET data=json_set(data,'$.rawText','changed') WHERE type='sdk_raw_result'", [])
                .unwrap();
        } else {
            tx.execute("UPDATE workflow_steps SET request_hash='changed'", []).unwrap();
        }
        assert!(
            record_callback(&tx, &f.request, &f.snapshot, &f.job, Some("run"), "node", &f.raw, &f.raw, None).is_err()
        );
        assert_eq!(
            tx.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='workflow_artifacts_v1'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn structure_verifier_checks_actual_frozen_schema_not_model_pass_word() {
    let f = fixture();
    assert!(shape(&f.raw, &f.request.schema, &f.request.schema).is_ok());
    assert!(shape(&json!({"PASS":true}), &f.request.schema, &f.request.schema).is_err());
    let mut duplicate = f.raw.clone();
    duplicate["decisions"].as_array_mut().unwrap().push(f.raw["decisions"][0].clone());
    assert!(shape(&duplicate, &f.request.schema, &f.request.schema).is_err());
    let mut wrong = f.raw.clone();
    wrong["decisions"][0]["action"] = json!("change_anything");
    assert!(shape(&wrong, &f.request.schema, &f.request.schema).is_err());
}
