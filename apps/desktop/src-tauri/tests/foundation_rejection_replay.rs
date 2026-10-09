//! A narrowly bound replay of the known public-context validation regression.
//! All normal tests use isolated stores and a native offline counter fixture.
use director_core::{
    pipeline::{
        prepare_source, run_foundation, validate_foundation, FoundationProposal, LockedSource,
        TextExecutor, TextRequest,
    },
    store::Store,
    text_executor::SavedTextExecutor,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

const OLD_ERROR: &str = "分集资料引用跨入未声明的别集";
const OLD_VARIANT_ERROR: &str = "已登记形象的名称/来源关系被静默改写";
const STOP: &str = "isolated replay intentionally stops after the first accepted batch";

struct Fixture {
    directory: tempfile::TempDir,
    executable: PathBuf,
    counter: PathBuf,
    generation_counter: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join(if cfg!(windows) {
            "offline.exe"
        } else {
            "offline"
        });
        let counter = directory.path().join("invocations.txt");
        let generation_counter = directory.path().join("generation-invocations.txt");
        let source = directory.path().join("offline.rs");
        let code = r#"
use std::io::{BufRead, Read, Write};
fn main() {
  std::fs::OpenOptions::new().create(true).append(true).open(__COUNTER__).unwrap().write_all(b"called\n").unwrap();
  let args:Vec<String>=std::env::args().collect();
  if args.iter().any(|a|a=="--version") { println!("codex-cli offline-fixture"); return; }
  if args.iter().any(|a|a=="--help") { println!("--json --output-schema --output-last-message --ignore-user-config --ignore-rules --ephemeral"); return; }
  if args.get(1).map(|s|s.as_str())==Some("app-server") {
    for line in std::io::stdin().lock().lines() {
      let line=line.unwrap();
      if line.contains("\"initialize\"") { println!("{{\"id\":1,\"result\":{{}}}}"); }
      else if line.contains("account/read") { println!("{{\"id\":2,\"result\":{{\"account\":{{\"type\":\"chatgpt\",\"planType\":\"offline\"}}}}}}"); }
      else if line.contains("model/list") { println!("{{\"id\":10,\"result\":{{\"data\":[{{\"model\":\"offline-fixture-model\",\"isDefault\":true,\"defaultReasoningEffort\":\"medium\",\"supportedReasoningEfforts\":[{{\"reasoningEffort\":\"medium\"}},{{\"reasoningEffort\":\"high\"}}]}}],\"nextCursor\":null}}}}"); }
      else if line.contains("account/rateLimits/read") { println!("{{\"id\":30,\"result\":{{\"rateLimitsByLimitId\":{{}}}}}}"); }
      std::io::stdout().flush().unwrap();
    }
    return;
  }
  std::fs::OpenOptions::new().create(true).append(true).open(__GENERATIONS__).unwrap().write_all(b"generated\n").unwrap();
  let path=args.windows(2).find(|a|a[0]=="--output-last-message").unwrap();
  let mut prompt=String::new(); std::io::stdin().read_to_string(&mut prompt).unwrap();
  std::fs::write(&path[1], b"{\"draft\":\"fresh-offline-result\"}").unwrap();
}
"#.replace("__COUNTER__", &json!(counter).to_string())
  .replace("__GENERATIONS__", &json!(generation_counter).to_string());
        std::fs::write(&source, code).unwrap();
        let compiler = std::env::var_os("RUSTC")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                            .map(|p| PathBuf::from(p).join(".cargo"))
                    })
                    .map(|p| {
                        p.join("bin")
                            .join(if cfg!(windows) { "rustc.exe" } else { "rustc" })
                    })
                    .filter(|p| p.is_file())
                    .unwrap_or_else(|| PathBuf::from("rustc"))
            });
        let mut command = Command::new(compiler);
        command
            .args(["--edition=2021", "--crate-name", "offline_replay_fixture"])
            .arg(source)
            .arg("-o")
            .arg(&executable);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            directory,
            executable,
            counter,
            generation_counter,
        }
    }
    fn store(&self) -> Store {
        make_store(self.directory.path().join(uuid::Uuid::new_v4().to_string()))
    }
    fn calls(&self) -> usize {
        std::fs::read_to_string(&self.counter)
            .unwrap_or_default()
            .lines()
            .count()
    }
    fn generation_calls(&self) -> usize {
        std::fs::read_to_string(&self.generation_counter)
            .unwrap_or_default()
            .lines()
            .count()
    }
    fn executor(&self, store: &Store, workflow_id: Option<String>) -> SavedTextExecutor {
        SavedTextExecutor {
            store: store.clone(),
            settings: json!({"textProvider":"codex","codexPath":self.executable}),
            workflow_id,
        }
    }
}
fn make_store(root: PathBuf) -> Store {
    let store = Store::open(root).unwrap();
    store
        .create(
            "isolated-rejection-replay",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    store
}
fn job_evidence(store: &Store, job: &str) -> String {
    store
        .conn()
        .unwrap()
        .query_row(
            "SELECT json_array(snapshot,result,receipt,error,state) FROM jobs WHERE id=?1",
            [job],
            |row| row.get(0),
        )
        .unwrap()
}
fn request() -> TextRequest {
    TextRequest {
        stage: "foundation".into(),
        skill: "story-foundation".into(),
        model: "offline-fixture-model".into(),
        reasoning: "medium".into(),
        input: json!({"sourceSha256":"frozen","episode":{"id":"episode-1"},"batchIndex":0}),
        schema: json!({"type":"object"}),
        rule_snapshot: json!({"id":"story-foundation","version":"1.1.0","content":"isolated rule"}),
    }
}
fn snapshot(request: &TextRequest, executable: &std::path::Path) -> Value {
    let mut value = serde_json::to_value(request).unwrap();
    value["executorPolicy"] = json!("codex-isolated/v1");
    value["executorIdentity"] =
        json!({"provider":"codex","executable":executable.to_string_lossy()});
    value
}
fn node_key(request: &TextRequest) -> String {
    let episode = request
        .input
        .get("episodeId")
        .or_else(|| request.input.get("episode").and_then(|v| v.get("id")))
        .and_then(Value::as_str)
        .unwrap_or("project");
    let part = request
        .input
        .get("workPackage")
        .and_then(|v| v.get("id"))
        .or_else(|| request.input.get("batch"))
        .or_else(|| request.input.get("batchIndex"))
        .map(Value::to_string)
        .unwrap_or_default();
    format!("{}:{episode}:{part}", request.stage)
}
fn workflow(store: &Store) -> String {
    store
        .add_job(
            "workflow",
            "foundation.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run"}),
            "running",
        )
        .unwrap()
}
fn seed(
    store: &Store,
    run: &str,
    request: &TextRequest,
    snapshot: &Value,
    result: &Value,
    error: &str,
) -> String {
    let job = store
        .add_job(
            "text",
            "foundation:isolated-seed",
            "codex",
            "local-text-account",
            snapshot,
            "received",
        )
        .unwrap();
    store.update_job(&job,"rejected",&json!({"error":error,"result":result,"receipt":{"validation":"rejected","contract":"desktop-runner/v1"}})).unwrap();
    store
        .checkpoint(
            run,
            &node_key(request),
            &request.stage,
            &director_core::pipeline::digest(&snapshot.to_string()),
            "rejected",
            Some(&job),
            Some(result),
            Some(error),
        )
        .unwrap();
    job
}
fn source() -> LockedSource {
    prepare_source("【故事简介】\n公共人物背景资料。\n第一集\n场景：校园\n甲：走吧。\n第二集\n场景：家中\n甲：回来了。\n").unwrap()
}
fn proposal(source: &LockedSource, cross_episode: bool) -> Value {
    let background = source
        .rows
        .iter()
        .find(|r| r.kind == "context" && r.text.contains("公共"))
        .unwrap();
    let body = source
        .rows
        .iter()
        .find(|r| {
            r.kind == "body"
                && r.episode_id.as_deref() == Some(&source.episodes[usize::from(cross_episode)].id)
        })
        .unwrap();
    json!({"schemaVersion":"foundation/v1","sourceSha256":source.sha256,"overview":"isolated","direction":"3D","episodes":[{"episodeId":source.episodes[0].id,"summary":"资料","context":"公共背景","sourceRefs":[background.id,body.id]}],"assets":[],"facts":[],"issues":[]})
}

#[tokio::test]
async fn known_context_rejection_revalidates_same_job_without_any_cli_and_keeps_audit() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let run = workflow(&store);
    let request = request();
    let snapshot = snapshot(&request, &fixture.executable);
    let source = source();
    let raw = proposal(&source, false);
    let job = seed(&store, &run, &request, &snapshot, &raw, OLD_ERROR);
    // A newer identical snapshot belongs to a different workflow: never rewrite it.
    let other_run = workflow(&store);
    let decoy = seed(
        &store,
        &other_run,
        &request,
        &snapshot,
        &json!({"draft":"other workflow"}),
        OLD_ERROR,
    );
    let executor = fixture.executor(&store, Some(run.clone()));
    let value = executor.execute(request.clone()).await.unwrap();
    assert_eq!(value, raw);
    let proposal: FoundationProposal = serde_json::from_value(value.clone()).unwrap();
    validate_foundation(&source, &proposal).unwrap();
    let mut accepted = value;
    accepted["materialized"] = json!(true);
    executor.accept(&request, &accepted).await.unwrap();
    assert_eq!(fixture.calls(), 0);
    assert_eq!(store.job(&job).unwrap()["status"], "completed");
    assert_eq!(store.job(&decoy).unwrap()["status"], "rejected");
    assert_eq!(
        store
            .accepted_step(
                &run,
                &director_core::pipeline::digest(&snapshot.to_string())
            )
            .unwrap(),
        Some(accepted.clone())
    );
    assert_eq!(executor.execute(request.clone()).await.unwrap(), accepted);
    assert_eq!(fixture.calls(), 0);
    let errors: i64 = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE job_id=?1 AND json_extract(data,'$.error')=?2",
            rusqlite::params![job, OLD_ERROR],
            |r| r.get(0),
        )
        .unwrap();
    assert!(errors >= 1, "previous rejection event must remain");
}

#[tokio::test]
async fn true_cross_episode_result_remains_rejected_without_cli_or_a_new_text_job() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let run = workflow(&store);
    let request = request();
    let snapshot = snapshot(&request, &fixture.executable);
    let source = source();
    let raw = proposal(&source, true);
    let job = seed(&store, &run, &request, &snapshot, &raw, OLD_ERROR);
    let executor = fixture.executor(&store, Some(run));
    let value = executor.execute(request.clone()).await.unwrap();
    let proposal: FoundationProposal = serde_json::from_value(value.clone()).unwrap();
    let error = validate_foundation(&source, &proposal).unwrap_err();
    assert!(error.starts_with(OLD_ERROR));
    executor.reject(&request, &value, &error).await.unwrap();
    assert_eq!(fixture.calls(), 0);
    let saved = store.job(&job).unwrap();
    assert_eq!(saved["status"], "rejected");
    assert_eq!(saved["error"], error);
    let count: i64 = store
        .conn()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM jobs WHERE kind='text'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn replay_requires_exact_workflow_step_snapshot_identity_and_known_error() {
    let fixture = Fixture::new();
    for change in [
        "workflow",
        "no_workflow",
        "stage",
        "skill",
        "input",
        "source",
        "model",
        "reasoning",
        "rules",
        "schema",
        "executor",
        "node",
        "hash",
        "error",
        "receipt",
        "checkpoint_result",
    ] {
        let store = fixture.store();
        let run = workflow(&store);
        let mut request = request();
        let saved_snapshot = snapshot(&request, &fixture.executable);
        let raw = json!({"draft":"must not be reused"});
        let job = seed(&store, &run, &request, &saved_snapshot, &raw, OLD_ERROR);
        let mut selected_run = Some(run.clone());
        let mut executor_path = fixture.executable.clone();
        match change {
            "workflow" => selected_run = Some(workflow(&store)),
            "no_workflow" => selected_run = None,
            "stage" => request.stage = "storyboardPlan".into(),
            "skill" => request.skill = "other-rule".into(),
            "input" => request.input["batchIndex"] = json!(1),
            "source" => request.input["sourceSha256"] = json!("different"),
            "model" => request.model = "unavailable-fixture-model".into(),
            "reasoning" => request.reasoning = "high".into(),
            "rules" => request.rule_snapshot["version"] = json!("different"),
            "schema" => request.schema["description"] = json!("different"),
            "executor" => {
                executor_path = fixture.directory.path().join(if cfg!(windows) {
                    "alternative.exe"
                } else {
                    "alternative"
                });
                std::fs::copy(&fixture.executable, &executor_path).unwrap();
            }
            "node" => {
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE workflow_steps SET node_key='different' WHERE workflow_id=?1",
                        [&run],
                    )
                    .unwrap();
            }
            "hash" => {
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE workflow_steps SET request_hash='different' WHERE workflow_id=?1",
                        [&run],
                    )
                    .unwrap();
            }
            "error" => {
                store
                    .update_job(&job, "rejected", &json!({"error":"other rejection"}))
                    .unwrap();
            }
            "receipt" => {
                store.update_job(&job,"rejected",&json!({"error":OLD_ERROR,"receipt":{"validation":"rejected","contract":"unknown"}})).unwrap();
            }
            "checkpoint_result" => {
                store
                    .conn()
                    .unwrap()
                    .execute(
                        "UPDATE workflow_steps SET result='{}' WHERE workflow_id=?1",
                        [&run],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let executor = SavedTextExecutor {
            store: store.clone(),
            settings: json!({"textProvider":"codex","codexPath":executor_path}),
            workflow_id: selected_run,
        };
        let old_evidence = job_evidence(&store, &job);
        let external_calls = fixture.calls();
        let generation_calls = fixture.generation_calls();
        let original_steps: String = store.conn().unwrap().query_row(
            "SELECT COALESCE(json_group_array(json_array(node_key,stage,request_hash,state,text_job_id,result,error)), '[]') FROM workflow_steps WHERE workflow_id=?1",
            [&run], |row| row.get(0),
        ).unwrap();
        let result = executor.execute(request.clone()).await;
        assert_ne!(
            result.as_ref().ok(),
            Some(&raw),
            "unmatched {change} reused rejected raw result"
        );
        assert_eq!(
            store
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM jobs WHERE kind='text'", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1,
            "unfrozen {change} cannot create a fresh text job"
        );
        assert_eq!(
            fixture.generation_calls(),
            generation_calls,
            "unfrozen {change} cannot dispatch even the synthetic runtime"
        );
        assert_eq!(
            job_evidence(&store, &job),
            old_evidence,
            "unfrozen {change} must retain original snapshot/raw/receipt/error/state"
        );

        let error = result.unwrap_err();
        let existing_node_held = !matches!(change, "workflow" | "stage" | "input" | "node");
        if existing_node_held {
            assert!(
                error.starts_with("CODEX_EXISTING_ATTEMPT_UNRESOLVED:"),
                "{change}: {error}"
            );
            assert_eq!(
                fixture.calls(), external_calls,
                "{change}: an old node or unowned original snapshot is held before all runtime I/O"
            );
        } else {
            assert!(
                error.contains("CODEX_UNMANAGED_IDENTITY_CHANGED"),
                "{change}: {error}"
            );
        }
        if change == "model" {
            let preflight = director_core::text_cli::resolve_settings(&json!({
                "textProvider":"codex", "codexPath":executor_path,
                "textModel":request.model, "reasoning":request.reasoning
            }))
            .await
            .unwrap_err();
            assert_eq!(
                preflight,
                "所选文本模型不在 CLI 模型目录中，请刷新并选择模型"
            );
            assert_eq!(
                fixture.generation_calls(),
                generation_calls,
                "an unavailable model stays blocked through explicit preflight"
            );
        } else {
            let settings = director_core::text_cli::resolve_settings(&json!({
                "textProvider":"codex", "codexPath":executor_path,
                "textModel":request.model, "reasoning":request.reasoning
            }))
            .await
            .unwrap();
            assert_eq!(settings["textCliVersion"], "codex-cli offline-fixture");
            assert_eq!(settings["textCliExecutable"], settings["codexPath"]);
            assert_eq!(
                fixture.generation_calls(),
                generation_calls,
                "explicit preflight only reads the synthetic runtime contract"
            );
            // Never relabel or overwrite the historical two-field workflow to
            // authorize a fresh request. Even dropping the workflow identifier
            // cannot turn an original request into a fresh standalone attempt.
            // A separate, explicitly frozen parent owns the new attempt.
            let fresh_run = store.add_job("workflow", "foundation.run", "local", "local",
                &json!({"runnerVersion":"desktop-runner/v1", "stage":"foundation.run", "input":{"settings":settings}}),
                "running").unwrap();
            assert_ne!(fresh_run, run);
            assert_eq!(store.job(&fresh_run).unwrap()["snapshot"]["input"]["settings"], settings);
            let fresh = SavedTextExecutor {
                store: store.clone(),
                settings: settings.clone(),
                workflow_id: Some(fresh_run),
            };
            assert_eq!(
                fresh.execute(request.clone()).await.unwrap(),
                json!({"draft":"fresh-offline-result"}),
                "preflighted {change} returns only its fresh synthetic result"
            );
            assert_eq!(
                store
                    .conn()
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM jobs WHERE kind='text'", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                2,
                "preflighted {change} creates exactly one fresh job"
            );
            assert_eq!(
                fixture.generation_calls(),
                generation_calls + 1,
                "preflighted {change} executes the synthetic fixture exactly once"
            );
            let new_snapshot: String = store
                .conn()
                .unwrap()
                .query_row(
                    "SELECT snapshot FROM jobs WHERE kind='text' AND id!=?1",
                    [&job],
                    |r| r.get(0),
                )
                .unwrap();
            let new_snapshot: Value = serde_json::from_str(&new_snapshot).unwrap();
            assert_eq!(
                new_snapshot["executorIdentity"],
                json!({"provider":"codex","executable":settings["textCliExecutable"],"version":settings["textCliVersion"]}),
                "new dispatch records the actually inspected three-field identity"
            );
        }
        let current_steps: String = store.conn().unwrap().query_row(
            "SELECT COALESCE(json_group_array(json_array(node_key,stage,request_hash,state,text_job_id,result,error)), '[]') FROM workflow_steps WHERE workflow_id=?1",
            [&run], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            current_steps, original_steps,
            "{change} must not replace the original legacy workflow checkpoint"
        );
        assert_eq!(
            job_evidence(&store, &job),
            old_evidence,
            "{change} must not change the original snapshot/raw/receipt/error/state"
        );
        assert_eq!(
            store.job(&job).unwrap()["status"],
            "rejected",
            "{change} must preserve old job"
        );
    }
}

#[tokio::test]
async fn unsupported_reasoning_rejects_before_a_new_job_or_generation_and_holds_original_raw() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let run = workflow(&store);
    let mut request = request();
    let saved_snapshot = snapshot(&request, &fixture.executable);
    let raw = json!({"draft":"rejected raw cannot become a readiness result"});
    let job = seed(&store, &run, &request, &saved_snapshot, &raw, OLD_ERROR);
    let evidence = job_evidence(&store, &job);
    let original_step: String = store.conn().unwrap().query_row(
        "SELECT json_array(node_key,stage,request_hash,state,text_job_id,result,error,updated_at) FROM workflow_steps WHERE workflow_id=?1",
        [&run], |row| row.get(0),
    ).unwrap();
    request.reasoning = "unsupported-strength".into();
    let external_calls = fixture.calls();
    let error = fixture
        .executor(&store, Some(run.clone()))
        .execute(request.clone())
        .await
        .unwrap_err();
    assert!(error.starts_with("CODEX_EXISTING_ATTEMPT_UNRESOLVED:"), "{error}");
    assert_eq!(fixture.calls(), external_calls, "the original node is held before readiness or generation I/O");
    assert_eq!(fixture.generation_calls(), 0);
    assert_eq!(job_evidence(&store, &job), evidence);
    let preflight = director_core::text_cli::resolve_settings(&json!({
        "textProvider":"codex", "codexPath":fixture.executable,
        "textModel":request.model, "reasoning":request.reasoning
    })).await.unwrap_err();
    assert_eq!(preflight, "所选文本模型不支持此推理强度，请调整后再生成");
    assert_eq!(fixture.generation_calls(), 0, "explicit read-only preflight does not authorize a replacement request");
    assert_eq!(job_evidence(&store, &job), evidence);
    let jobs: i64 = store
        .conn()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM jobs WHERE kind='text'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(jobs, 1);
    let replays: i64 = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='validation_replay'",
            [&job],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        replays, 0,
        "an unavailable reasoning choice cannot reuse rejected raw"
    );
    let current_step: String = store.conn().unwrap().query_row(
        "SELECT json_array(node_key,stage,request_hash,state,text_job_id,result,error,updated_at) FROM workflow_steps WHERE workflow_id=?1",
        [&run], |row| row.get(0),
    ).unwrap();
    assert_eq!(current_step, original_step, "read-only preflight cannot change the original node or its frozen budget");
}

#[tokio::test]
async fn variant_label_regression_replays_only_the_original_job_and_rechecks_parent_changes() {
    let fixture = Fixture::new();
    for accepted in [true, false] {
        let store = fixture.store();
        let run = workflow(&store);
        let request = request();
        let snapshot = snapshot(&request, &fixture.executable);
        let raw = json!({"appearance":"original received result"});
        let job = seed(&store, &run, &request, &snapshot, &raw, OLD_VARIANT_ERROR);
        let executor = fixture.executor(&store, Some(run.clone()));
        let value = executor.execute(request.clone()).await.unwrap();
        assert_eq!(value, raw);
        if accepted {
            executor.accept(&request, &value).await.unwrap();
            assert_eq!(store.job(&job).unwrap()["status"], "completed");
            assert!(store
                .accepted_step(
                    &run,
                    &director_core::pipeline::digest(&snapshot.to_string())
                )
                .unwrap()
                .is_some());
        } else {
            executor
                .reject(&request, &value, "派生来源发生变化")
                .await
                .unwrap();
            assert_eq!(store.job(&job).unwrap()["status"], "rejected");
            assert!(store
                .accepted_step(
                    &run,
                    &director_core::pipeline::digest(&snapshot.to_string())
                )
                .unwrap()
                .is_none());
        }
        assert_eq!(
            store.job(&job).unwrap()["receipt"]["previousValidationError"],
            OLD_VARIANT_ERROR
        );
        assert_eq!(
            store
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM jobs WHERE kind='text'", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(fixture.calls(), 0, "revalidation cannot launch the CLI");
    }
}

struct FirstBatchOnly<'a> {
    executor: &'a SavedTextExecutor,
    expected: TextRequest,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl TextExecutor for FirstBatchOnly<'_> {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            return Err(STOP.into());
        }
        if serde_json::to_value(&request).unwrap() != serde_json::to_value(&self.expected).unwrap()
        {
            return Err(
                "private replay rebuilt a different request; no executor was called".into(),
            );
        }
        self.executor.execute(request).await
    }
    async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
        self.executor.accept(request, value).await
    }
    async fn reject(
        &self,
        request: &TextRequest,
        value: &Value,
        reason: &str,
    ) -> Result<(), String> {
        self.executor.reject(request, value, reason).await
    }
}

#[tokio::test]
#[ignore = "requires DIRECTOR_FOUNDATION_REPLAY_FIXTURE pointing to a private saved request; no generation is permitted"]
async fn private_saved_first_batch_is_accepted_and_next_batch_stops_without_generation() {
    let path = std::env::var_os("DIRECTOR_FOUNDATION_REPLAY_FIXTURE")
        .expect("private fixture env is required");
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let request: TextRequest = serde_json::from_value(fixture["request"].clone()).unwrap();
    let source: LockedSource = serde_json::from_value(fixture["source"].clone()).unwrap();
    let original_text = source.text.clone();
    let original_sha = source.sha256.clone();
    let offline = Fixture::new();
    let store = offline.store();
    // Keep the actual source/input/model/rules/schema exactly as saved. Replace
    // only the executor identity with a non-networked native counter fixture, so
    // even a replay-cache regression cannot invoke a real account or generation.
    let mut isolated_snapshot = fixture["request"].clone();
    isolated_snapshot["executorIdentity"] =
        json!({"provider":"codex","executable":offline.executable.to_string_lossy()});
    let run = workflow(&store);
    let job = seed(
        &store,
        &run,
        &request,
        &isolated_snapshot,
        &fixture["result"],
        OLD_ERROR,
    );
    let executor = offline.executor(&store, Some(run.clone()));
    let wrapper = FirstBatchOnly {
        executor: &executor,
        expected: request.clone(),
        calls: AtomicUsize::new(0),
    };
    let error = run_foundation(
        &source,
        &fixture["context"],
        &wrapper,
        &request.model,
        &request.reasoning,
    )
    .await
    .unwrap_err();
    assert_eq!(error, STOP);
    assert_eq!(store.job(&job).unwrap()["status"], "completed");
    assert_eq!(store.job(&job).unwrap()["error"], Value::Null);
    assert!(store
        .accepted_step(
            &run,
            &director_core::pipeline::digest(&isolated_snapshot.to_string())
        )
        .unwrap()
        .is_some());
    assert_eq!(wrapper.calls.load(Ordering::SeqCst), 2);
    assert_eq!(source.text, original_text);
    assert_eq!(source.sha256, original_sha);
    let count: i64 = store
        .conn()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM jobs WHERE kind='text'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        !store.root.join("text-runs").exists(),
        "CLI output directory must not be created"
    );
    assert_eq!(
        offline.calls(),
        0,
        "not even the offline CLI may be invoked"
    );
    println!("private first batch accepted; next batch intentionally stopped; zero CLI invocations/new generation; original source text/hash and TextRequest unchanged; executor identity replaced by isolated offline fixture");
}
