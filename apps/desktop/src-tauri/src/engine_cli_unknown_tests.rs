//! Real engine entry points, synthetic stores, zero model/provider calls.
use super::*;

#[test]
fn cli_unknown_real_engine_new_production_never_reserves_or_bypasses_via_backend() {
    for state in ["submission_unknown", "failed", "interrupted", "cancelled"] {
        for provider in ["codex", "codex_sdk", "api"] {
            let temp = tempfile::tempdir().unwrap();
            let engine = Engine::new_offline_verification(temp.path().join("isolated-app")).unwrap();
            let store = Store::open(temp.path().join("synthetic-project")).unwrap();
            store.create("synthetic CLI unknown", "synthetic-cli-unknown").unwrap();
            let source = store.save_source(json!(pipeline::prepare_source("第一集\n场景：门口\n甲：你好。\n").unwrap())).unwrap();
            let mut settings = store.settings().unwrap(); settings["textProvider"] = json!(provider);
            store.save_settings(&settings).unwrap();
            let origin = store.add_job("text", "original-dispatch", "codex", "synthetic", &json!({"originalRequest":"immutable"}), state).unwrap();
            store.update_job(&origin, state, &json!({"prepared":{"nativeBudget":{"reserved":true}},"error":"original interruption"})).unwrap();
            store.event(&origin,"codex_event",&json!({"type":"turn.started"})).unwrap();
            let original = store.job(&origin).unwrap();
            let count = || store.conn().unwrap().query_row("SELECT (SELECT COUNT(*) FROM jobs),(SELECT COUNT(*) FROM events)",[],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?))).unwrap();
            let before = count();
            for stage in ["foundation.run", "storyboard.run", "storyboard.refine", "images.design", "images.produce"] {
                let error = engine.spawn_stage(store.clone(),json!({"action":stage,"episodeIds":[source["episodes"][0]["id"]],"episodeId":source["episodes"][0]["id"]})).unwrap_err();
                assert!(error.contains("CLI_SUBMISSION_UNKNOWN"), "{provider}/{stage}/{state}: {error}");
                assert_eq!(count(),before,"no new workflow, text attempt, preflight event or reserved budget");
                assert_eq!(store.job(&origin).unwrap(),original);
                assert!(engine.active.lock().unwrap().is_empty());
            }
            assert!(engine.spawn_foundation_choices(store.clone(),&json!({})).unwrap_err().contains("CLI_SUBMISSION_UNKNOWN"));
            assert_eq!(count(),before);
        }
    }
}
