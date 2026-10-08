use director_core::{
    native_acceptance::{AcceptanceGrant, MODEL, REASONING, VERSION},
    platform, Engine,
};
use serde_json::json;

#[test]
fn safe_sdk_status_is_readable_without_runtime_account_or_job() {
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path().join("home")).unwrap();
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"Synthetic SDK status"}))
        .unwrap();
    let status = engine
        .dispatch(json!({"action":"textSdk.status","projectId":project["id"]}))
        .unwrap();
    assert_eq!(status["backend"], "codex_sdk");
    assert_eq!(status["liveCallsAllowed"], false);
    assert_eq!(status["sharedRustExecutor"], true);
    assert_eq!(status["nativeVerified"], false);
    assert!(engine
        .store(project["id"].as_str().unwrap())
        .unwrap()
        .jobs()
        .unwrap()
        .is_empty());
}
#[test]
fn sdk_unknown_job_cannot_be_closed_to_release_ownership() {
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path().join("home")).unwrap();
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"Synthetic unknown SDK"}))
        .unwrap();
    let store = engine.store(project["id"].as_str().unwrap()).unwrap();
    let job = store
        .add_job(
            "text",
            "synthetic",
            "codex_sdk",
            "synthetic",
            &json!({}),
            "submission_unknown",
        )
        .unwrap();
    let before = store.job(&job).unwrap();
    let error=engine.dispatch(json!({"action":"jobs.close_unknown","projectId":project["id"],"jobId":job,"confirmedNotAccepted":true,"reason":"no proof"})).unwrap_err();
    assert!(error.contains("SDK_UNRESOLVED_HELD"));
    assert_eq!(before, store.job(&job).unwrap());
}
#[test]
fn stale_independent_episode_confirmation_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new(root.path().join("home")).unwrap();
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"Synthetic duplicate"}))
        .unwrap();
    let workspace=engine.dispatch(json!({"action":"source.import","projectId":project["id"],"text":"第1集\n场景：门口\n甲：原句。\n第1集\n场景：院中\n乙：另句。\n"})).unwrap();
    let source = workspace["source"].clone();
    let version = source["version"].as_i64().unwrap();
    let duplicate = source["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DUPLICATE_EPISODE_NUMBER")
        .unwrap();
    let row = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == duplicate["rowIds"][0])
        .unwrap();
    let error=engine.dispatch(json!({"action":"source.resolve","projectId":project["id"],"expectedSourceVersion":version,"decisions":[{"start":row["range"]["start"],"end":row["range"]["end"],"kind":"independent_episode","episodeNumber":null,"sceneTitle":null,"reason":"synthetic explicit decision","decisionVersion":"boundary-decision/v2","confirmation":{"sourceVersion":version-1,"sourceSha256":source["sha256"],"parserVersion":source["parserVersion"],"headingRowId":row["id"],"diagnosticCode":"DUPLICATE_EPISODE_NUMBER"}}]})).unwrap_err();
    assert!(error.contains("版本已过期"));
    assert_eq!(
        engine
            .store(project["id"].as_str().unwrap())
            .unwrap()
            .source()
            .unwrap(),
        source
    );
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[test]
fn linux_cannot_enable_native_acceptance_or_fake_persistent_keyring() {
    let root = tempfile::tempdir().unwrap();
    let grant = AcceptanceGrant {
        version: VERSION.into(),
        grant_id: "synthetic".into(),
        approved_document_sha256: "a".repeat(64),
        locked_source_sha256: "b".repeat(64),
        episode_count: 10,
        storyboard_episode_numbers: vec![1, 2],
        backend: "codex".into(),
        model: MODEL.into(),
        reasoning: REASONING.into(),
    };
    assert!(Engine::with_native_acceptance(root.path().join("native"), grant).is_err());
    assert_eq!(platform::credential_backend_status()["supported"], false);
    assert!(platform::write_credential("synthetic", "synthetic")
        .unwrap_err()
        .contains("CREDENTIAL_BACKEND_UNSUPPORTED"));
}

#[test]
fn headless_native_async_command_rejects_before_home_or_worker_creation() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    for action in [
        "foundation.run",
        "images.design",
        "storyboard.run",
        "storyboard.refine",
        "workflows.resume",
    ] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("never-created");
        let mut child = Command::new(env!("CARGO_BIN_EXE_director-next-core"))
            .arg(&home)
            .arg("--native-cli-acceptance")
            .arg(root.path().join("unused-grant.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                json!({"action":action,"projectId":"synthetic"})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains("HEADLESS_ASYNC_ACCEPTANCE_BLOCKED"),
            "{action}: {error}"
        );
        assert!(
            !home.exists(),
            "rejected action created runtime state: {action}"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn headless_saved_grant_cannot_start_an_async_workflow() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("native-cli-acceptance.json"), b"{}").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_director-next-core"))
        .arg(root.path()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(br#"{"action":"foundation.run"}"#).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("HEADLESS_ASYNC_ACCEPTANCE_BLOCKED"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}
