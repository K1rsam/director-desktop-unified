use super::*;
use crate::series_lite_contract::VerifiedPack;
use crate::series_lite_job_packet::{assemble, RejectUnverifiedWorkspace};
use crate::series_lite_transport::{freeze_verified, Role};
use crate::text_transport::{AttemptState, Backend, FrozenAttempt};

fn request() -> (TextRequest, TransportIdentity, RoleContext) {
    let corpus: Value =
        serde_json::from_str(include_str!("series_lite_job_packet_golden.json")).unwrap();
    let case = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "author_complete")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in case["files"].as_object().unwrap() {
        let target = dir.path().join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text.as_str().unwrap()).unwrap();
    }
    let pack = VerifiedPack::embedded().unwrap();
    let packet = assemble(
        dir.path(),
        "request.json",
        case["maxBytes"].as_u64().unwrap() as usize,
        &pack,
        &RejectUnverifiedWorkspace,
    )
    .unwrap();
    let context = RoleContext {
        episode: "1".into(),
        role: Role::Author,
        iteration_id: "iteration-1".into(),
        context_id: "author-context-1".into(),
        prior_contributors: vec![],
    };
    let carrier = freeze_verified(&packet, &context, &pack).unwrap();
    let mut input = carrier.input;
    input["textExact"] = json!({"version":PROFILE,"baseSha256":"a".repeat(64)});
    (
        TextRequest {
            stage: Role::Author.stage().into(),
            skill: "series-lite".into(),
            model: "gpt-6.1-sol".into(),
            reasoning: "medium".into(),
            input,
            schema: json!({"type":"object"}),
            rule_snapshot: json!({"version":"synthetic-frozen-rules/v1"}),
        },
        carrier.identity,
        context,
    )
}
const TEXT: &str =
    "{\r\n\"精确\":1.250000001123456789123456789,\"huge\":1e309,\"negativeZero\":-0.0000000}\r\n";

fn fixture(
    provider: &str,
    text: &str,
) -> (
    tempfile::TempDir,
    SavedTextExecutor,
    TextRequest,
    TransportIdentity,
    RoleContext,
    String,
) {
    let (request, identity, context) = request();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store.create("exact synthetic", "synthetic").unwrap();
    store.ensure_workflow_storage().unwrap();
    let path = temp
        .path()
        .join("never-launch.exe")
        .to_string_lossy()
        .into_owned();
    let settings = json!({"textProvider":provider,"textModel":"gpt-6.1-sol","reasoning":"medium","textCliPath":path,
        "textSdkIdentity":{"version":"official-sdk-identity/v1","provider":"codex_sdk","sdkPackage":"@openai/codex-sdk","sdkVersion":crate::text_sdk::SDK_VERSION,
        "sdkImplementationSha256":crate::text_sdk::SDK_IMPLEMENTATION_SHA,"bridgeVersion":"0.1.0","bridgeSha256":"a".repeat(64),"protocolVersion":crate::text_sdk::PROTOCOL,
        "nodeExecutable":path,"nodeSha256":"b".repeat(64),"nodeVersion":"v22.0.0","runtimePackageVersion":crate::text_sdk::SDK_VERSION,"runtimeIdentity":"bundled-synthetic-fixture",
        "permissionProfile":crate::text_sdk::POLICY,"accountFingerprint":"synthetic-no-account"}});
    let run = store
        .add_job(
            "workflow",
            "series.run",
            "local",
            "local",
            &json!({"runnerVersion":"desktop-runner/v1","input":{"settings":settings}}),
            "running",
        )
        .unwrap();
    let executor = SavedTextExecutor {
        store,
        settings,
        workflow_id: Some(run.clone()),
    };
    // CLI evidence does not need an installed executable: historical reads use
    // its saved dispatch files. SDK identity matches the established fixture.
    let snapshot = if provider == "codex_sdk" {
        executor.request_snapshot(&request).unwrap()
    } else {
        let mut value = executor.request_domain_snapshot(&request).unwrap();
        value["executorIdentity"] =
            json!({"provider":"codex","executable":path,"version":"codex-cli synthetic"});
        value
    };
    let hash = crate::pipeline::digest(&snapshot.to_string());
    let job = executor
        .store
        .add_job(
            "text",
            "synthetic exact",
            provider,
            "synthetic",
            &snapshot,
            "submitting",
        )
        .unwrap();
    executor
        .store
        .checkpoint(
            &run,
            &step_key(&request),
            &request.stage,
            &hash,
            "running",
            Some(&job),
            None,
            None,
        )
        .unwrap();
    let dir = executor.store.root.join("text-runs").join(&job);
    std::fs::create_dir_all(&dir).unwrap();
    let compiled = compile_codex_input(&request).unwrap();
    if provider == "codex_sdk" {
        crate::store::atomic_json(&dir.join("sdk-compiled.json"),&json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)})).unwrap();
        executor.store.update_job(&job,"submitting",&json!({"prepared":{"sdkRequestHash":hash,"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":run,"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema)}})).unwrap();
        let mut attempt = FrozenAttempt::prepare(
            &job,
            &run,
            &step_key(&request),
            Backend::CodexSdk,
            request.clone(),
            &snapshot["executorIdentity"],
        )
        .unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        attempt.state = AttemptState::Completed;
        attempt.last_sequence = 3;
        attempt.thread_id = Some("synthetic-thread".into());
        // Deliberately inaccurate JS projection must never become authoritative.
        attempt.raw_result = Some(
            json!({"rawText":text,"parsedJson":{"huge":null,"精确":1.25},"parseError":null,"domainAccepted":false}),
        );
        attempt.persist(&dir.join("sdk-attempt.json")).unwrap();
        executor
            .sdk_receive(&request, &snapshot, &job, &attempt)
            .unwrap();
    } else {
        let prompt =
            rules_first_cli_prompt(&serde_json::from_str(&compiled.prompt).unwrap()).unwrap();
        let allocation = CliDispatchAllocation {
            job: job.clone(),
            request_hash: hash.clone(),
            workflow: Some(run.clone()),
            node: step_key(&request),
            retry_origin: None,
        };
        let metadata = cli_dispatch_metadata(
            &allocation,
            CLI_RULES_FIRST_FORMAT,
            &prompt,
            &compiled.schema,
        );
        std::fs::write(dir.join("cli-prompt.txt"), prompt).unwrap();
        std::fs::write(dir.join("schema.json"), compiled.schema).unwrap();
        std::fs::write(dir.join("cli-dispatch.json"), metadata.to_string()).unwrap();
        executor
            .store
            .event(&job, "cli_dispatch_frozen", &metadata)
            .unwrap();
        std::fs::write(dir.join("result.json"), text).unwrap();
        let envelope = wrap(text.as_bytes()).unwrap();
        executor
            .store
            .event(
                &job,
                "lite_exact_received",
                &receive_evidence(&envelope, &hash),
            )
            .unwrap();
        executor
            .store
            .event(
                &job,
                "text_received_raw",
                &json!({"requestHash":hash,"value":envelope}),
            )
            .unwrap();
        executor
            .store
            .update_job(&job, "received", &json!({"result":envelope}))
            .unwrap();
    }
    (temp, executor, request, identity, context, job)
}

#[test]
fn exact_utf8_envelope_preserves_decimal_spellings_and_duplicate_keys_for_domain_rejection() {
    for text in [TEXT, "{\"x\":1,\"x\":2}", "not JSON"] {
        let wrapped = wrap(text.as_bytes()).unwrap();
        let saved = serde_json::to_vec(&wrapped).unwrap();
        assert_eq!(
            unwrap(&serde_json::from_slice(&saved).unwrap()).unwrap(),
            text.as_bytes()
        );
    }
    assert!(serde_json::from_str::<Value>(TEXT).is_err());
    assert!(crate::series_lite_edit::parse(b"{\"x\":1,\"x\":2}").is_err());
    assert!(wrap(&[0xff]).is_err());
    let mut value = wrap(TEXT.as_bytes()).unwrap();
    value["nativeTextSha256"] = json!("bad");
    assert!(unwrap(&value).is_err());
}

#[tokio::test]
async fn exact_cli_and_sdk_replay_native_bytes_without_runtime_or_current_account() {
    for provider in ["codex", "codex_sdk"] {
        let (_temp, mut executor, request, identity, context, job) = fixture(provider, TEXT);
        let jobs = executor.store.jobs().unwrap();
        executor.settings["textSdkIdentity"]["accountFingerprint"] =
            json!("not-inspected-no-account");
        let raw = executor
            .execute_exact(request.clone(), &identity, &context)
            .await
            .unwrap();
        assert_eq!(raw.bytes(), TEXT.as_bytes());
        assert_eq!(raw.sha256(), crate::pipeline::digest(TEXT));
        assert_eq!(raw.job_id(), job);
        assert_eq!(executor.store.jobs().unwrap(), jobs);
        assert!(!executor.store.root.join("sdk-worker").exists());
        // Exact immutable packet pins are distinct from the self-described input.
        let mut wrong = identity.clone();
        wrong.packet_file_sha256 = "b".repeat(64);
        assert!(executor
            .execute_exact(request.clone(), &wrong, &context)
            .await
            .is_err());
        let mut wrong_context = context.clone();
        wrong_context.iteration_id = "different".into();
        assert!(executor
            .execute_exact(request.clone(), &identity, &wrong_context)
            .await
            .is_err());
        assert_eq!(executor.store.jobs().unwrap(), jobs);
    }
}

#[tokio::test]
async fn exact_raw_and_receive_pollution_are_held_without_replacement_jobs() {
    for provider in ["codex", "codex_sdk"] {
        for mutation in ["file", "native-event", "value-only", "prompt"] {
            let (_temp, executor, request, identity, context, job) = fixture(provider, TEXT);
            let dir = executor.store.root.join("text-runs").join(&job);
            match mutation {
                "file" => std::fs::write(dir.join("result.json"), "{}").unwrap(),
                "native-event" => {
                    executor
                        .store
                        .conn()
                        .unwrap()
                        .execute(
                            "DELETE FROM events WHERE job_id=?1 AND type='lite_exact_received'",
                            [&job],
                        )
                        .unwrap();
                }
                "value-only" => {
                    executor
                        .store
                        .update_job(&job, "received", &json!({"result":{"x":1.25}}))
                        .unwrap();
                }
                "prompt" => std::fs::write(
                    dir.join(if provider == "codex" {
                        "cli-prompt.txt"
                    } else {
                        "sdk-compiled.json"
                    }),
                    "{}",
                )
                .unwrap(),
                _ => unreachable!(),
            }
            let before = executor.store.jobs().unwrap();
            assert!(
                executor
                    .execute_exact(request, &identity, &context)
                    .await
                    .is_err(),
                "{provider} {mutation}"
            );
            assert_eq!(executor.store.jobs().unwrap(), before);
        }
    }
}

#[tokio::test]
async fn exact_cli_callback_is_cas_bound_and_never_stores_parsed_candidate_as_raw() {
    let (_temp, executor, request, identity, context, _) = fixture("codex", TEXT);
    let raw = executor
        .execute_exact(request.clone(), &identity, &context)
        .await
        .unwrap();
    assert!(executor
        .accept(&request, &json!({"normalized":"candidate"}))
        .await
        .is_err());
    executor
        .accept(&request, raw.transport_envelope())
        .await
        .unwrap();
    executor
        .accept(&request, raw.transport_envelope())
        .await
        .unwrap();
    assert!(executor
        .reject(&request, raw.transport_envelope(), "new failure")
        .await
        .is_err());
    assert_eq!(
        executor
            .execute_exact(request, &identity, &context)
            .await
            .unwrap()
            .bytes(),
        TEXT.as_bytes()
    );
}

#[test]
fn exact_slot_content_drift_does_not_allocate_a_new_node_and_old_requests_stay_unmarked() {
    let (request, _, _) = request();
    let key = step_key(&request);
    let mut changed = request.clone();
    changed.input["textExact"]["baseSha256"] = json!("b".repeat(64));
    assert_eq!(step_key(&changed), key);
    changed.input["identity"]["packetFileSha256"] = json!("c".repeat(64));
    assert_eq!(step_key(&changed), key);
    changed = request.clone();
    changed.input["roleContext"]["iterationId"] = json!("iteration-2");
    assert_ne!(step_key(&changed), key);
    let mut old = request;
    old.stage = "storyboardCreate".into();
    old.input = json!({"episodeId":"original"});
    assert!(!enabled(&old));
    assert!(validate_request(&old).is_ok());
    assert_eq!(base_step_key(&old), "storyboardCreate:original:");
}

#[tokio::test]
async fn exact_sdk_unknown_preserves_original_attempt_and_never_executes_a_replacement() {
    let (_temp, executor, request, identity, context, job) = fixture("codex_sdk", TEXT);
    let dir = executor.store.root.join("text-runs").join(&job);
    let path = dir.join("sdk-attempt.json");
    let mut attempt = FrozenAttempt::load(&path).unwrap();
    attempt.state = AttemptState::SubmissionUnknown;
    attempt.raw_result = None;
    attempt.persist(&path).unwrap();
    executor.store.conn().unwrap().execute("UPDATE jobs SET state='submission_unknown',result=NULL,error='synthetic unknown' WHERE id=?1",[&job]).unwrap();
    let before = executor.store.jobs().unwrap();
    let frozen = std::fs::read(&path).unwrap();
    // The fixture has no executable or authoritative passive receipt. Existing
    // recovery must stop; it cannot buy a replacement under the current account.
    assert!(executor
        .execute_exact(request, &identity, &context)
        .await
        .is_err());
    assert_eq!(executor.store.jobs().unwrap(), before);
    assert_eq!(std::fs::read(&path).unwrap(), frozen);
    assert!(!executor.store.root.join("sdk-worker").exists());
}

#[tokio::test]
async fn exact_sdk_accept_preserves_native_text_and_uses_existing_commit() {
    let (_temp, executor, request, identity, context, job) = fixture("codex_sdk", TEXT);
    let raw = executor
        .execute_exact(request.clone(), &identity, &context)
        .await
        .unwrap();
    executor
        .accept(&request, raw.transport_envelope())
        .await
        .unwrap();
    let after = executor
        .execute_exact(request, &identity, &context)
        .await
        .unwrap();
    assert_eq!(after.bytes(), TEXT.as_bytes());
    assert_eq!(executor.store.job(&job).unwrap()["status"], "completed");
    assert_eq!(
        std::fs::read(
            executor
                .store
                .root
                .join("text-runs")
                .join(&job)
                .join("result.json")
        )
        .unwrap(),
        TEXT.as_bytes()
    );
}
