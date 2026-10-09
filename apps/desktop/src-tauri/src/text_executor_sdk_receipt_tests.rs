//! Historical readback uses the existing sdk_raw proof and never a live bridge.
use super::*;
use crate::text_transport::{AttemptState, Backend, FrozenAttempt};

#[tokio::test]
async fn sdk_passive_completed_and_received_replay_old_bridge_with_exact_raw_proof() {
    for mode in ["received", "completed", "unknown"] {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("project")).unwrap();
        store.create("synthetic historical receipt", "synthetic").unwrap();
        store.ensure_workflow_storage().unwrap();
        // No installed Node/runtime is needed to read existing evidence.
        let path = temp.path().join("missing-node.exe").to_string_lossy().into_owned();
        let settings = json!({"textProvider":"codex_sdk","textModel":"gpt-6.1-sol","reasoning":"medium",
            "textSdkIdentity":{"version":"official-sdk-identity/v1","provider":"codex_sdk",
            "sdkPackage":"@openai/codex-sdk","sdkVersion":crate::text_sdk::SDK_VERSION,
            "sdkImplementationSha256":crate::text_sdk::SDK_IMPLEMENTATION_SHA,
            "bridgeVersion":"0.1.0","bridgeSha256":"a".repeat(64),"protocolVersion":crate::text_sdk::PROTOCOL,
            "nodeExecutable":path,"nodeSha256":"b".repeat(64),"nodeVersion":"v22.0.0",
            "runtimePackageVersion":crate::text_sdk::SDK_VERSION,"runtimeIdentity":"bundled-synthetic-fixture",
            "permissionProfile":crate::text_sdk::POLICY,"accountFingerprint":"synthetic-no-account"}});
        let run = store.add_job("workflow", "foundation.run", "local", "local",
            &json!({"runnerVersion":"desktop-runner/v1","input":{"settings":settings}}), "running").unwrap();
        let executor = SavedTextExecutor { store, settings, workflow_id:Some(run.clone()) };
        let request = TextRequest { stage:"foundation:SDK_CONTRACT".into(),skill:"asset-art-director".into(),
            model:"gpt-6.1-sol".into(),reasoning:"medium".into(),input:json!({"text":"合成回包"}),
            schema:json!({"type":"object"}),rule_snapshot:crate::pipeline::snapshot("asset-art-director", &[]).unwrap() };
        let snapshot = executor.request_snapshot(&request).unwrap();
        assert!(crate::text_sdk::identity(&executor.settings).is_err());
        let hash = crate::pipeline::digest(&snapshot.to_string());
        let job = executor.store.add_job("text", "synthetic", "codex_sdk", "synthetic-no-account", &snapshot, "submitting").unwrap();
        let dir=executor.store.root.join("text-runs").join(&job);
        std::fs::create_dir_all(&dir).unwrap();
        let compiled=compile_codex_input(&request).unwrap();
        crate::store::atomic_json(&dir.join("sdk-compiled.json"), &json!({"prompt":compiled.prompt,"schema":compiled.schema,
            "promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)})).unwrap();
        executor.store.update_job(&job,"submitting",&json!({"prepared":{"sdkRequestHash":hash,"sdkAttemptId":job,
            "sdkDispatchStarted":true,"sdkOriginWorkflowId":run,"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),
            "compiledSchemaSha256":crate::pipeline::digest(&compiled.schema)}})).unwrap();
        executor.store.checkpoint(&run,&step_key(&request),&request.stage,&hash,"running",Some(&job),None,None).unwrap();
        let mut attempt=FrozenAttempt::prepare(&job,&run,&step_key(&request),Backend::CodexSdk,request.clone(),&snapshot["executorIdentity"]).unwrap();
        let prepared_attempt=json!(attempt);
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        if mode=="unknown" {
            attempt.state=AttemptState::SubmissionUnknown; attempt.thread_id=Some("old-thread".into());attempt.last_sequence=3;
            attempt.persist(&dir.join("sdk-attempt.json")).unwrap();
            let events=format!("{}\n",json!({"protocolVersion":crate::text_sdk::PROTOCOL,"attemptId":job,
                "sequence":1,"threadId":"old-thread","turnId":null,"rawEvent":{"type":"thread.started","thread_id":"old-thread"}}));
            let record=json!({"protocolVersion":crate::text_sdk::PROTOCOL,"attempt":prepared_attempt,
                "frozenAttemptSha256":crate::pipeline::digest(&prepared_attempt.to_string()),
                "requestSnapshotSha256":crate::pipeline::digest(&prepared_attempt["request"].to_string()),
                "journalSha256":crate::pipeline::digest(&events),"lastSequence":1,"threadId":"old-thread","turnId":null,
                "domainAccepted":false,"state":"submission_unknown"});
            let text=record.to_string();
            let sealed=format!("{},\"receiptEnvelopeSha256\":\"{}\"}}",&text[..text.len()-1],crate::pipeline::digest(&text));
            let worker_dir=executor.store.root.join("sdk-worker").join(&job);
            std::fs::create_dir_all(&worker_dir).unwrap();
            std::fs::write(worker_dir.join("receipt.json"),&sealed).unwrap();
            std::fs::write(worker_dir.join("events.ndjson"),&events).unwrap();
            executor.store.update_job(&job,"submission_unknown",&json!({"error":"original error"})).unwrap();
            let jobs_before=executor.store.jobs().unwrap();
            let attempt_before=std::fs::read(dir.join("sdk-attempt.json")).unwrap();
            assert!(executor.execute(request.clone()).await.unwrap_err().contains("SDK_EXISTING_ATTEMPT_UNRESOLVED"));
            assert_eq!(executor.store.jobs().unwrap(),jobs_before);
            assert_eq!(std::fs::read(dir.join("sdk-attempt.json")).unwrap(),attempt_before);
            assert_eq!(std::fs::read_to_string(worker_dir.join("receipt.json")).unwrap(),sealed);
            assert_eq!(std::fs::read_to_string(worker_dir.join("events.ndjson")).unwrap(),events);
            assert!(!executor.store.root.join("sdk-worker/worker.lock").exists());
            continue;
        }
        attempt.state=AttemptState::Completed;attempt.last_sequence=3;attempt.thread_id=Some("original-thread".into());
        let raw=json!({"ok":true,"text":"合成回包"});
        attempt.raw_result=Some(json!({"rawText":raw.to_string(),"parsedJson":raw,"parseError":null,"domainAccepted":false}));
        attempt.persist(&dir.join("sdk-attempt.json")).unwrap();
        assert_eq!(executor.sdk_receive(&request,&snapshot,&job,&attempt).unwrap(),raw);
        if mode=="completed" { executor.accept(&request,&raw).await.unwrap(); }
        let jobs_before=executor.store.jobs().unwrap();
        let original_attempt=std::fs::read(dir.join("sdk-attempt.json")).unwrap();
        assert_eq!(executor.execute(request.clone()).await.unwrap(),raw);
        assert_eq!(executor.store.jobs().unwrap(),jobs_before);
        assert_eq!(std::fs::read(dir.join("sdk-attempt.json")).unwrap(),original_attempt);
        assert!(!executor.store.root.join("sdk-worker").exists());
        // Existing raw proof still fails closed after a bridge upgrade.
        std::fs::write(dir.join("result.json"),"{\"tampered\":true}").unwrap();
        assert!(executor.execute(request.clone()).await.unwrap_err().contains("SDK_RAW_BINDING_MISMATCH"));
        assert_eq!(executor.store.jobs().unwrap(),jobs_before);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }
}
