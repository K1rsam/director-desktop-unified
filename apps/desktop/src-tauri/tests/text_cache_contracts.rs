//! Durable text acceptance/cache boundaries. Never launches Codex or calls an API.
use director_core::{
    pipeline::{TextExecutor, TextRequest},
    store::Store,
    text_executor::SavedTextExecutor,
};
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command};

struct Fixture {
    directory: tempfile::TempDir,
    executable: PathBuf,
    counter: PathBuf,
    settings: Value,
}
impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("director-text-cache-中文 ")
            .tempdir()
            .unwrap();
        let counter = directory.path().join("calls.txt");
        let executable = directory.path().join(if cfg!(windows) {
            "fixture-cli.exe"
        } else {
            "fixture-cli"
        });
        let source = directory.path().join("fixture.rs");
        // Compile a tiny native, non-networked executable. A shell wrapper would not
        // exercise the desktop's direct executable/argument/output-file boundary.
        let code = r#"
use std::io::{BufRead, Read, Write};
fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.iter().any(|a| a=="--version") { println!("codex-cli offline-fixture"); return; }
    if arguments.iter().any(|a| a=="--help") {
        println!("--json --output-schema --output-last-message --ignore-user-config --ignore-rules --ephemeral"); return;
    }
    if arguments.get(1).map(|a|a.as_str())==Some("app-server") {
        for line in std::io::stdin().lock().lines() {
            let line=line.unwrap();
            // This is a fixed offline RPC fixture, never a model implementation.
            // Match only the methods produced by the actual readiness contract.
            if line.contains("\"initialize\"") {
                println!("{{\"id\":1,\"result\":{{}}}}");
            } else if line.contains("account/read") {
                println!("{{\"id\":2,\"result\":{{\"account\":{{\"type\":\"chatgpt\",\"planType\":\"offline\"}}}}}}");
            } else if line.contains("model/list") {
                println!("{{\"id\":10,\"result\":{{\"data\":[{{\"model\":\"offline-fixture-model\",\"isDefault\":true,\"defaultReasoningEffort\":\"medium\",\"supportedReasoningEfforts\":[{{\"reasoningEffort\":\"medium\"}}]}}],\"nextCursor\":null}}}}");
            } else if line.contains("account/rateLimits/read") {
                println!("{{\"id\":30,\"result\":{{\"rateLimitsByLimitId\":{{}}}}}}");
            }
            std::io::stdout().flush().unwrap();
        }
        return;
    }
    let output = arguments.windows(2).find(|a| a[0] == "--output-last-message")
        .map(|a| &a[1]).expect("missing output path");
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    assert!(!input.is_empty());
    let mut calls = std::fs::OpenOptions::new().create(true).append(true)
        .open(__COUNTER_PATH__).unwrap();
    calls.write_all(b"called\n").unwrap();
    std::fs::write(output, b"{\"schemaVersion\":\"fixture/v1\",\"draft\":\"unvalidated\"}").unwrap();
    println!("{{\"type\":\"fixture.completed\"}}");
}
"#
        .replace("__COUNTER_PATH__", &json!(counter).to_string());
        std::fs::write(&source, code).unwrap();
        let compiler = std::env::var_os("RUSTC")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let cargo_home = std::env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                            .map(|p| PathBuf::from(p).join(".cargo"))
                    });
                cargo_home
                    .map(|p| {
                        p.join("bin")
                            .join(if cfg!(windows) { "rustc.exe" } else { "rustc" })
                    })
                    .filter(|p| p.is_file())
                    .unwrap_or_else(|| PathBuf::from("rustc"))
            });
        let mut command = Command::new(compiler);
        command.args(["--edition=2021", "--crate-name", "saved_text_fixture"]);
        command.arg(&source).arg("-o").arg(&executable);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let compiled = command
            .output()
            .expect("Rust compiler required by offline fixture");
        assert!(
            compiled.status.success(),
            "fixture compilation failed: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        // Fresh dispatch uses the same real preflight/freeze path as the app.
        // This probes only our just-compiled, non-networked synthetic runtime.
        let settings = director_core::text_cli::resolve_settings(&json!({
            "textProvider":"codex", "codexPath":executable,
            "textModel":"offline-fixture-model", "reasoning":"medium"
        }))
        .await
        .unwrap();
        assert_eq!(settings["textCliVersion"], "codex-cli offline-fixture");
        assert_eq!(settings["textCliExecutable"], settings["codexPath"]);
        assert!(PathBuf::from(settings["textCliExecutable"].as_str().unwrap()).is_file());
        assert!(
            !counter.exists(),
            "read-only synthetic preflight never generates"
        );
        Self {
            directory,
            executable,
            counter,
            settings,
        }
    }
    fn store(&self) -> Store {
        let store = Store::open(self.directory.path().join("project")).unwrap();
        store
            .create("offline-cache-test", &uuid::Uuid::new_v4().to_string())
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        store
    }
    fn calls(&self) -> usize {
        std::fs::read_to_string(&self.counter)
            .unwrap_or_default()
            .lines()
            .count()
    }
    fn executor(&self, store: &Store, workflow_id: Option<String>) -> SavedTextExecutor {
        SavedTextExecutor {
            store: store.clone(),
            settings: self.settings.clone(),
            workflow_id,
        }
    }
}
fn request() -> TextRequest {
    TextRequest {
        stage: "foundationExtract".into(),
        skill: "story-foundation".into(),
        model: "offline-fixture-model".into(),
        reasoning: "medium".into(),
        input: json!({"episodeId":"episode-1", "batch":1, "rows":[{"id":"row-1","text":"原文"}]}),
        schema: json!({"type":"object"}),
        rule_snapshot: json!({"id":"story-foundation","version":"1.1.0","content":"offline fixture rule"}),
    }
}
fn standalone_design_request() -> TextRequest {
    let mut value = request();
    value.stage = "assetDesign".into();
    value.skill = "asset-art-director".into();
    value.input = json!({"asset":{"id":"standalone-offline-asset","variantId":"base"},"sourceSha256":"a".repeat(64)});
    value.rule_snapshot = json!({"primary":director_core::pipeline::rule_snapshot("asset-art-director").unwrap(),"methods":[]});
    value
}
fn workflow(store: &Store) -> String {
    store
        .add_job(
            "workflow",
            "foundation.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1"}),
            "running",
        )
        .unwrap()
}
fn text_jobs(store: &Store) -> Vec<Value> {
    let connection = store.conn().unwrap();
    let mut query = connection
        .prepare("SELECT id,state,result,receipt FROM jobs WHERE kind='text' ORDER BY rowid")
        .unwrap();
    query.query_map([], |row| {
        let result: Option<String> = row.get(2)?;
        let receipt: Option<String> = row.get(3)?;
        Ok(json!({"id":row.get::<_,String>(0)?,"state":row.get::<_,String>(1)?,"result":result.and_then(|v|serde_json::from_str::<Value>(&v).ok()),"receipt":receipt.and_then(|v|serde_json::from_str::<Value>(&v).ok())}))
    }).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
}
fn step_state(store: &Store, run: &str) -> String {
    store
        .conn()
        .unwrap()
        .query_row(
            "SELECT state FROM workflow_steps WHERE workflow_id=?1",
            [run],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn standalone_design_without_current_frozen_native_identity_never_dispatches() {
    let fixture = Fixture::new().await;
    let store = fixture.store();
    let request = standalone_design_request();
    for recorded_version in [None, Some("codex-cli stale-offline-version")] {
        let mut executor = fixture.executor(&store, None);
        if let Some(version) = recorded_version {
            executor.settings["textCliVersion"] = json!(version);
        } else {
            executor
                .settings
                .as_object_mut()
                .unwrap()
                .remove("textCliVersion");
        }
        let error = executor.execute(request.clone()).await.unwrap_err();
        assert!(
            error.contains("CODEX_UNMANAGED_IDENTITY_CHANGED"),
            "{error}"
        );
        assert_eq!(
            fixture.calls(),
            0,
            "readiness cannot upgrade an unfrozen standalone request into generation"
        );
        assert!(
            text_jobs(&store).is_empty(),
            "no attempt is allocated before identity is frozen"
        );
    }
}

#[tokio::test]
async fn frozen_standalone_design_reopens_received_and_accepted_raw_without_extra_dispatch() {
    let fixture = Fixture::new().await;
    let store = fixture.store();
    let executor = fixture.executor(&store, None);
    let request = standalone_design_request();
    let raw = executor.execute(request.clone()).await.unwrap();
    assert_eq!(fixture.calls(), 1);
    let job = text_jobs(&store)[0]["id"].as_str().unwrap().to_owned();
    let frozen = store.job(&job).unwrap()["snapshot"].clone();
    assert_eq!(
        frozen["executorIdentity"]["version"],
        fixture.settings["textCliVersion"]
    );
    assert_eq!(
        frozen["executorIdentity"]["executable"],
        fixture.settings["textCliExecutable"]
    );
    let original_event: String = store.conn().unwrap().query_row("SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1", [&job], |row| row.get(0)).unwrap();
    let original_file =
        std::fs::read(store.root.join("text-runs").join(&job).join("result.json")).unwrap();
    let root = store.root.clone();
    drop(executor);
    drop(store);
    let reopened = Store::open(root).unwrap();
    let resumed = fixture.executor(&reopened, None);
    assert_eq!(resumed.execute(request.clone()).await.unwrap(), raw);
    let normalized = json!({"schemaVersion":"fixture/v1","draft":"trusted offline materialized design","validated":true});
    resumed.accept(&request, &normalized).await.unwrap();
    let accepted = text_jobs(&reopened);
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["result"], normalized);
    assert_eq!(
        resumed.execute(request.clone()).await.unwrap(),
        raw,
        "standalone validation reuses original raw rather than the normalized cache value"
    );
    resumed.accept(&request, &normalized).await.unwrap();
    assert_eq!(text_jobs(&reopened), accepted);
    assert_eq!(fixture.calls(), 1);
    assert_eq!(reopened.job(&job).unwrap()["snapshot"], frozen);
    let event: String = reopened.conn().unwrap().query_row("SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1", [&job], |row| row.get(0)).unwrap();
    assert_eq!(event, original_event);
    assert_eq!(
        std::fs::read(
            reopened
                .root
                .join("text-runs")
                .join(&job)
                .join("result.json")
        )
        .unwrap(),
        original_file
    );
}

#[tokio::test]
async fn changed_standalone_identity_or_unknown_attempt_preserves_old_request_and_holds_dispatch() {
    let fixture = Fixture::new().await;
    let store = fixture.store();
    let executor = fixture.executor(&store, None);
    let request = standalone_design_request();
    executor.execute(request.clone()).await.unwrap();
    let job = text_jobs(&store)[0]["id"].as_str().unwrap().to_owned();
    let frozen = store.job(&job).unwrap();
    let mut drift = fixture.executor(&store, None);
    drift.settings["textCliVersion"] = json!("codex-cli changed-offline-version");
    let error = drift.execute(request.clone()).await.unwrap_err();
    assert!(
        error.contains("CODEX_STANDALONE_ATTEMPT_UNRESOLVED"),
        "{error}"
    );
    assert_eq!(
        store.job(&job).unwrap(),
        frozen,
        "identity drift leaves the existing attempt intact"
    );
    store
        .update_job(
            &job,
            "submission_unknown",
            &json!({"error":"synthetic unresolved carrier"}),
        )
        .unwrap();
    let unknown = store.job(&job).unwrap();
    let error = executor.execute(request).await.unwrap_err();
    assert!(
        error.contains("CODEX_STANDALONE_ATTEMPT_UNRESOLVED"),
        "{error}"
    );
    assert_eq!(store.job(&job).unwrap(), unknown);
    assert_eq!(text_jobs(&store).len(), 1);
    assert_eq!(
        fixture.calls(),
        1,
        "neither changed identity nor unknown submission authorizes another request"
    );
}

#[tokio::test]
async fn accepted_materialized_result_survives_reopen_and_reuses_without_cli() {
    let fixture = Fixture::new().await;
    let store = fixture.store();
    let run = workflow(&store);
    let executor = fixture.executor(&store, Some(run.clone()));
    let request = request();
    let received = executor.execute(request.clone()).await.unwrap();
    assert_eq!(text_jobs(&store)[0]["state"], "received");
    assert_eq!(fixture.calls(), 1);
    let accepted = json!({"schemaVersion":"fixture/v1","draft":"program materialized exact source","validated":true});
    assert_ne!(
        accepted, received,
        "accept must persist the validated/materialized value"
    );
    executor.accept(&request, &accepted).await.unwrap();
    let original = text_jobs(&store);
    assert_eq!(original[0]["state"], "completed");
    assert_eq!(original[0]["receipt"]["validation"], "accepted");
    assert_eq!(step_state(&store, &run), "accepted");
    let root = store.root.clone();
    drop(executor);
    drop(store);

    let reopened = Store::open(root).unwrap();
    let same_run = fixture.executor(&reopened, Some(run));
    assert_eq!(same_run.execute(request.clone()).await.unwrap(), accepted);
    assert_eq!(
        fixture.calls(),
        1,
        "accepted workflow checkpoint must avoid subprocesses"
    );
    assert_eq!(text_jobs(&reopened), original);

    let new_run = workflow(&reopened);
    let another = fixture.executor(&reopened, Some(new_run.clone()));
    assert_eq!(another.execute(request).await.unwrap(), accepted);
    assert_eq!(
        fixture.calls(),
        1,
        "global accepted cache must avoid subprocesses too"
    );
    assert_eq!(text_jobs(&reopened), original);
    assert_eq!(step_state(&reopened, &new_run), "accepted");
}

#[tokio::test]
async fn received_and_rejected_outputs_are_never_completed_cache_after_reopen() {
    for reject in [false, true] {
        let fixture = Fixture::new().await;
        let store = fixture.store();
        let run = workflow(&store);
        let executor = fixture.executor(&store, Some(run.clone()));
        let request = request();
        let value = executor.execute(request.clone()).await.unwrap();
        if reject {
            executor
                .reject(&request, &value, "source coverage failed")
                .await
                .unwrap();
        }
        let first = text_jobs(&store)[0].clone();
        assert_eq!(first["state"], if reject { "rejected" } else { "received" });
        assert_eq!(
            step_state(&store, &run),
            if reject { "rejected" } else { "running" }
        );
        let root = store.root.clone();
        drop(executor);
        drop(store);

        let reopened = Store::open(root).unwrap();
        fixture
            .executor(&reopened, Some(run))
            .execute(request)
            .await
            .unwrap();
        assert_eq!(fixture.calls(), 2, "unaccepted response must execute again");
        let jobs = text_jobs(&reopened);
        assert_eq!(jobs.len(), 2);
        assert_eq!(
            jobs[0], first,
            "original rejected/received evidence must remain intact"
        );
        assert_eq!(jobs[1]["state"], "received");
    }
}

#[tokio::test]
async fn different_native_executor_identity_cannot_reuse_accepted_output() {
    let fixture = Fixture::new().await;
    let store = fixture.store();
    let run = workflow(&store);
    let first = fixture.executor(&store, Some(run.clone()));
    let request = request();
    let value = first.execute(request.clone()).await.unwrap();
    let accepted = json!({"validated":true,"from":"first executor"});
    first.accept(&request, &accepted).await.unwrap();
    let alternative = fixture.directory.path().join(if cfg!(windows) {
        "alternative-cli.exe"
    } else {
        "alternative-cli"
    });
    std::fs::copy(&fixture.executable, &alternative).unwrap();
    let changed = SavedTextExecutor {
        store: Store::open(store.root.clone()).unwrap(),
        settings: director_core::text_cli::resolve_settings(&json!({
            "textProvider":"codex", "codexPath":alternative,
            "textModel":"offline-fixture-model", "reasoning":"medium"
        }))
        .await
        .unwrap(),
        workflow_id: Some(run),
    };
    assert_eq!(changed.execute(request).await.unwrap(), value);
    assert_eq!(
        fixture.calls(),
        2,
        "changed executor must launch the offline fixture"
    );
    let jobs = text_jobs(&store);
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0]["result"], accepted);
    assert_eq!(jobs[0]["state"], "completed");
    assert_eq!(jobs[1]["state"], "received");
}

#[tokio::test]
async fn changed_api_endpoint_cannot_reuse_accepted_workflow_or_global_cache() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("project")).unwrap();
    store
        .create("offline-api-cache-test", &uuid::Uuid::new_v4().to_string())
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    let run = workflow(&store);
    let request = request();
    // Seed an accepted API result without touching a credential store or network.
    // The following is the persisted executor identity contract, not an API response.
    let mut snapshot = serde_json::to_value(&request).unwrap();
    snapshot["executorPolicy"] = json!("codex-isolated/v1");
    snapshot["executorIdentity"] =
        json!({"provider":"api","endpoint":"https://offline.invalid/v1"});
    let job = store
        .add_job(
            "text",
            "foundationExtract:offline-seed",
            "api",
            "local-text-account",
            &snapshot,
            "received",
        )
        .unwrap();
    let seeded = SavedTextExecutor {
        store: store.clone(),
        settings: json!({"textProvider":"api","textEndpoint":"https://offline.invalid/v1"}),
        workflow_id: Some(run.clone()),
    };
    let accepted = json!({"validated":true,"from":"previous endpoint"});
    seeded.accept(&request, &accepted).await.unwrap();
    assert_eq!(seeded.execute(request.clone()).await.unwrap(), accepted);
    assert_eq!(text_jobs(&store).len(), 1);

    // Empty is deliberately a different endpoint: api() rejects it before keyring
    // lookup and HTTP. A stale cache hit would incorrectly return the old value.
    for selected_run in [Some(run), None] {
        let changed = SavedTextExecutor {
            store: Store::open(store.root.clone()).unwrap(),
            settings: json!({"textProvider":"api","textEndpoint":""}),
            workflow_id: selected_run,
        };
        let error = changed.execute(request.clone()).await.unwrap_err();
        assert!(
            error.contains("文本API地址"),
            "unexpected offline failure: {error}"
        );
    }
    let jobs = text_jobs(&store);
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[0]["id"], job);
    assert_eq!(jobs[0]["state"], "completed");
    assert_eq!(jobs[0]["result"], accepted);
    assert_eq!(jobs[1]["state"], "failed");
    assert_eq!(jobs[2]["state"], "failed");
}
