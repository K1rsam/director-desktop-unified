use director_core::Engine;
use serde_json::json;

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn standard_desktop_opens_only_compiled_capabilities_and_keeps_offline_probe_isolated() {
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::new_standard(root.path().join("standard")).unwrap();
    let policy = engine.dispatch(json!({"action":"getExecutionPolicy"})).unwrap();
    assert_eq!(policy["mode"], "standard_native");
    assert_eq!(policy["liveExecutionAllowed"], true);
    assert_eq!(policy["readiness"]["enabled"], true);
    let actions = policy["allowedActions"].as_array().unwrap();
    for action in ["foundation.run", "images.design", "providers.refresh", "textCli.login", "videos.prepare"] {
        assert!(actions.contains(&json!(action)), "{action}");
    }
    let ready=director_core::execution_policy::creative_runtime_ready();
    for action in ["storyboard.run", "units.regenerate", "storyboard.decision.resolve", "storyboard.refine", "storyboard.refinement.adopt"] {
        assert_eq!(actions.contains(&json!(action)),ready);
        let error=engine.dispatch(json!({"action":action,"projectId":"missing"})).unwrap_err();
        assert_eq!(error.contains("STAGE3_ADAPTER_PENDING"),!ready);
    }
    let p = engine.dispatch(json!({"action":"projects.create","title":"synthetic standard"})).unwrap();
    let store = engine.store(p["id"].as_str().unwrap()).unwrap();
    let old = store.add_job("workflow", "storyboard.run", "codex", "synthetic", &json!({}), "blocked").unwrap();
    let before = store.job(&old).unwrap();
    let held=engine.dispatch(json!({"action":"workflows.resume","projectId":p["id"],"workflowId":old})).unwrap_err();
    assert!(!held.is_empty(),"unproven historical workflow cannot execute");
    assert_eq!(store.job(&old).unwrap(), before);
    // An empty source fails domain preflight, rather than the old master switch.
    let error = engine.dispatch(json!({"action":"foundation.run","projectId":p["id"]})).unwrap_err();
    assert!(!error.contains("OFFLINE_EXECUTION_BLOCKED"));
    assert_eq!(store.jobs().unwrap().len(), 1, "no external worker or new workflow");
    let isolated = Engine::new_offline_verification(root.path().join("probe")).unwrap();
    let policy = isolated.dispatch(json!({"action":"getExecutionPolicy"})).unwrap();
    assert_eq!(policy["liveExecutionAllowed"], false);
    assert_eq!(policy["readiness"]["enabled"], false);
}

#[test]
fn standard_allowlist_preserves_unknown_action_and_stage3_boundaries() {
    use director_core::execution_policy::check_standard;
    assert!(check_standard(&json!({"action":"settings.save","settings":{"textApiKey":"synthetic"}})).is_ok());
    assert!(check_standard(&json!({"action":"unknown.external.action"})).unwrap_err().contains("STANDARD_ACTION_UNSUPPORTED"));
    assert_eq!(check_standard(&json!({"action":"storyboard.run","settings":{"active":true}})).is_ok(),director_core::execution_policy::creative_runtime_ready(),"settings cannot activate a different pack");
}
