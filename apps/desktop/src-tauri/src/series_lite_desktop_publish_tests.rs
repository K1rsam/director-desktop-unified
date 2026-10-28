use super::*;
use crate::series_lite_controller_store::{FrozenRole, RoleResult};
use crate::series_lite_transport::{RoleContext, TransportIdentity};

struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
    record: RoleRecord,
}
fn fixture(provider: &str, role: Role, thread: &str) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store
        .create("synthetic original contexts", "synthetic")
        .unwrap();
    store.ensure_workflow_storage().unwrap();
    let context = RoleContext {
        episode: "1".into(),
        role,
        iteration_id: format!("initial-{}", role.as_str()),
        context_id: format!("planned-{}", role.as_str()),
        prior_contributors: vec![],
    };
    let request = TextRequest {
        stage: role.stage().into(),
        skill: format!("series-lite-{}", role.as_str()),
        model: "gpt-6.1-sol".into(),
        reasoning: "medium".into(),
        input: json!({"synthetic":true}),
        schema: json!({"type":"object"}),
        rule_snapshot: json!({"syntheticFrozenRules":true}),
    };
    let identity = if provider == "codex_sdk" {
        crate::text_sdk::offline_settings("success")["textSdkIdentity"].clone()
    } else {
        json!({"provider":"codex","executable":"C:\\synthetic-never-launch.exe"})
    };
    let mut snapshot = serde_json::to_value(&request).unwrap();
    snapshot["executorIdentity"] = identity.clone();
    snapshot["executorPolicy"] = json!(if provider == "codex_sdk" {
        "sdk-text-only-offline/v1"
    } else {
        "codex-isolated/v1"
    });
    let request_sha = digest(&snapshot.to_string());
    let job = store
        .add_job(
            "text",
            "synthetic-original",
            provider,
            "synthetic-account",
            &snapshot,
            "completed",
        )
        .unwrap();
    let raw = "{\"review\":\"synthetic exact response\"}";
    let envelope = json!({"version":"model-response-utf8/v1","rawText":raw,"nativeTextSha256":digest(raw),"byteLength":raw.len()});
    store.update_job(&job,"completed",&json!({"result":envelope,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}})).unwrap();
    store
        .event(
            &job,
            "text_received_raw",
            &json!({"requestHash":request_sha,"value":envelope}),
        )
        .unwrap();
    store.event(&job,"lite_exact_received",&json!({"version":"series-lite-exact-text/v1","requestHash":request_sha,"nativeTextSha256":digest(raw),"byteLength":raw.len()})).unwrap();
    let f = FrozenRole {
        version: controller::VERSION.into(),
        workflow_id: "synthetic-run".into(),
        source_sha256: "a".repeat(64),
        request,
        identity: TransportIdentity {
            profile: "synthetic".into(),
            packet_file_sha256: "b".repeat(64),
            packet_bytes: 2,
            input_fingerprint: "c".repeat(64),
            episode: "1".into(),
            role,
            rule_identity_sha256: "d".repeat(64),
            role_context_sha256: "e".repeat(64),
        },
        context,
        base_utf8: "{}".into(),
        base_sha256: digest("{}"),
        common_inputs: Inputs::new(),
        rule_snapshot_sha256: "f".repeat(64),
        previous_candidate: None,
    };
    let record = RoleRecord {
        slot_id: format!("synthetic-{}", role.as_str()),
        ordinal: if role == Role::Author {
            1
        } else if role == Role::Finisher {
            2
        } else {
            3
        },
        frozen: f,
        result: Some(RoleResult {
            slot_id: format!("synthetic-{}", role.as_str()),
            job_id: job.clone(),
            request_sha256: request_sha.clone(),
            native_response_sha256: digest(raw),
            raw_artifact_id: "unused".into(),
            raw_artifact_payload_sha256: "a".repeat(64),
            candidate: None,
        }),
        transport_state: Some("completed".into()),
    };
    let dir = store.root.join("text-runs").join(&job);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("result.json"), raw).unwrap();
    let events = [
        json!({"type":"thread.started","thread_id":thread}),
        json!({"type":"turn.started"}),
        json!({"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}),
    ];
    if provider == "codex_sdk" {
        let mut attempt = FrozenAttempt::prepare(
            &job,
            &record.frozen.workflow_id,
            &node(&record),
            Backend::CodexSdk,
            record.frozen.request.clone(),
            &identity,
        )
        .unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        attempt.state = AttemptState::Completed;
        attempt.thread_id = Some(thread.into());
        attempt.last_sequence = 7;
        attempt.raw_result = Some(
            json!({"rawText":raw,"parsedJson":{"review":"synthetic exact response"},"parseError":null,"domainAccepted":false}),
        );
        attempt.persist(&dir.join("sdk-attempt.json")).unwrap();
        let compiled = crate::text_executor::compile_sdk_attempt_input(&attempt).unwrap();
        std::fs::write(dir.join("sdk-compiled.json"),json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":digest(&compiled.prompt),"schemaSha256":digest(&compiled.schema)}).to_string()).unwrap();
        store.update_job(&job,"completed",&json!({"prepared":{"compiledPromptSha256":digest(&compiled.prompt),"compiledSchemaSha256":digest(&compiled.schema),"sdkRequestHash":request_sha,"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkRawTextSha256":digest(raw),"sdkDomainResult":envelope,"sdkDomainResultSha256":digest(&envelope.to_string())}})).unwrap();
        for (i, event) in events.iter().enumerate() {
            store.event(&job,"sdk_event",&json!({"type":"sdk_event","protocolVersion":crate::text_sdk::PROTOCOL,"requestId":"execute-1","attemptId":job,"sequence":i+1,"threadId":thread,"turnId":null,"rawEvent":event})).unwrap();
        }
        store.event(&job,"sdk_dispatch_receipt",&json!({"type":"response","protocolVersion":crate::text_sdk::PROTOCOL,"requestId":"execute-1","attemptId":job,"ok":true})).unwrap();
        store.event(&job,"sdk_observation",&json!({"type":"observation","protocolVersion":crate::text_sdk::PROTOCOL,"requestId":"execute-1","attemptId":job,"state":"transport_completed","backend":"codex_sdk","lastSequence":3,"processTreeStopProven":false,"threadId":thread,"turnId":null,"domainAccepted":false,"semanticRequestHash":attempt.semantic_request_hash,"executionFingerprint":attempt.execution_fingerprint,"attemptEnvelopeHash":attempt.attempt_envelope_hash,"compiledPromptSha256":digest(&compiled.prompt),"compiledSchemaSha256":digest(&compiled.schema),"rawResult":attempt.raw_result})).unwrap();
    } else {
        let compiled = crate::text_executor::compile_codex_input(&record.frozen.request).unwrap();
        let metadata = json!({"version":"cli-dispatch-format/v1","format":"legacy-json/v1","jobId":job,"workflowId":record.frozen.workflow_id,"nodeKey":node(&record),"requestHash":request_sha,"freshSemanticNode":true,"retryOrigin":null,"promptSha256":digest(&compiled.prompt),"schemaSha256":digest(&compiled.schema),"promptBytes":compiled.prompt.len(),"promptUtf16":compiled.prompt.encode_utf16().count()});
        std::fs::write(dir.join("cli-dispatch.json"), metadata.to_string()).unwrap();
        std::fs::write(dir.join("cli-prompt.txt"), compiled.prompt).unwrap();
        std::fs::write(dir.join("schema.json"), compiled.schema).unwrap();
        store.event(&job, "cli_dispatch_frozen", &metadata).unwrap();
        for event in events {
            store.event(&job, "codex_event", &event).unwrap();
        }
    }
    Fixture {
        _temp: temp,
        store,
        record,
    }
}
#[test]
fn cli_and_sdk_read_original_accepted_runtime_contexts_without_dispatch() {
    for provider in ["codex", "codex_sdk"] {
        let f = fixture(provider, Role::Reviewer, "real-provider-thread");
        let c = f.store.conn().unwrap();
        let before: i64 = c
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        let context = actual_context(&c, &f.store.root, &f.record).unwrap();
        assert_eq!(context.provider_thread_id, "real-provider-thread");
        assert_ne!(
            context.provider_thread_id,
            f.record.frozen.context.context_id
        );
        assert_eq!(
            before,
            c.query_row::<i64, _, _>("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .unwrap()
        );
    }
}
#[test]
fn planned_contexts_and_account_switch_do_not_prove_provider_independence() {
    let a = ActualContext {
        role: Role::Author,
        job_id: "author".into(),
        provider: "codex".into(),
        provider_thread_id: "same-real-thread".into(),
        runtime_evidence_sha256: "a".repeat(64),
    };
    let mut f = a.clone();
    f.role = Role::Finisher;
    f.job_id = "finisher".into();
    f.provider_thread_id = "finisher-real-thread".into();
    let mut r = a.clone();
    r.role = Role::Reviewer;
    r.job_id = "reviewer".into();
    r.provider = "codex_sdk".into();
    assert!(distinct_contexts(&[a.clone(), f.clone(), r.clone()])
        .unwrap_err()
        .contains("REUSED_CREATIVE_CONTEXT"));
    r.provider_thread_id = "reviewer-new-thread".into();
    assert!(distinct_contexts(&[a, f, r]).is_ok());
}
#[test]
fn unknown_and_missing_runtime_events_never_grant_content_review() {
    let mut f = fixture("codex", Role::Reviewer, "real-thread");
    f.record.transport_state = Some("submission_unknown".into());
    assert!(
        actual_context(&f.store.conn().unwrap(), &f.store.root, &f.record)
            .unwrap_err()
            .contains("ROLE_NOT_ACCEPTED_OR_UNKNOWN")
    );
    f.record.transport_state = Some("completed".into());
    f.store.conn().unwrap().execute("DELETE FROM events WHERE type='codex_event' AND json_extract(data,'$.type')='thread.started'",[]).unwrap();
    assert!(actual_context(&f.store.conn().unwrap(), &f.store.root, &f.record).is_err());
}
#[test]
fn result_file_dispatch_and_sdk_terminal_drift_are_rejected() {
    for provider in ["codex", "codex_sdk"] {
        let f = fixture(provider, Role::Reviewer, "real-thread");
        let id = &f.record.result.as_ref().unwrap().job_id;
        std::fs::write(
            f.store.root.join("text-runs").join(id).join("result.json"),
            "{\"different\":true}",
        )
        .unwrap();
        assert!(
            actual_context(&f.store.conn().unwrap(), &f.store.root, &f.record)
                .unwrap_err()
                .contains("RESULT_FILE_DRIFT")
        );
    }
    let f = fixture("codex_sdk", Role::Reviewer, "real-thread");
    let id = &f.record.result.as_ref().unwrap().job_id;
    f.store.conn().unwrap().execute("UPDATE events SET data=json_set(data,'$.threadId','forged-planned-context') WHERE job_id=?1 AND type='sdk_observation'",[id]).unwrap();
    assert!(
        actual_context(&f.store.conn().unwrap(), &f.store.root, &f.record)
            .unwrap_err()
            .contains("SDK_TERMINAL_RECEIPT_BINDING")
    );
}
#[test]
fn sdk_event_sequence_or_actual_thread_changes_are_not_hidden_by_a_good_result() {
    for change in ["sequence", "threadId"] {
        let f = fixture("codex_sdk", Role::Reviewer, "real-thread");
        let id = &f.record.result.as_ref().unwrap().job_id;
        let sql = if change == "sequence" {
            "UPDATE events SET data=json_set(data,'$.sequence',99) WHERE job_id=?1 AND type='sdk_event' AND json_extract(data,'$.sequence')=2"
        } else {
            "UPDATE events SET data=json_set(data,'$.threadId','different-thread') WHERE job_id=?1 AND type='sdk_event' AND json_extract(data,'$.sequence')=2"
        };
        f.store.conn().unwrap().execute(sql, [id]).unwrap();
        assert!(actual_context(&f.store.conn().unwrap(), &f.store.root, &f.record).is_err());
    }
}

#[test]
fn sdk_rejected_creative_context_retains_native_history_without_accepted_domain_or_review_capability() {
    for role in [Role::Author, Role::Finisher] {
        let mut f=fixture("codex_sdk",role,"rejected-creative-provider-thread");
        let id=f.record.result.as_ref().unwrap().job_id.clone();
        let original=f.store.job(&id).unwrap();
        let original_result=original["result"].clone();
        let accepted_prepared=original["prepared"].clone();
        let invalid="LITE_NATIVE_INVALID: synthetic creative fields are incomplete";
        let mut rejected_prepared=accepted_prepared.clone();
        for field in ["sdkDomainResult","sdkDomainResultSha256"] {rejected_prepared.as_object_mut().unwrap().remove(field);}
        {
            let mut c=f.store.conn().unwrap();let tx=c.transaction().unwrap();
            artifacts::install(&tx).unwrap();
            tx.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,created_at,updated_at) VALUES(?1,'workflow','storyboard.run','local','running','local','{}',?2,?2)",params![f.record.frozen.workflow_id,crate::store::now()]).unwrap();
            tx.execute("UPDATE jobs SET state='rejected',receipt=?1,error=?2,prepared=?3 WHERE id=?4",params![json!({"contract":"desktop-runner/v1","validation":"rejected"}).to_string(),invalid,rejected_prepared.to_string(),id]).unwrap();
            tx.execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,result,error,updated_at) VALUES(?1,?2,?3,?4,'rejected',?5,?6,?7,?8)",params![f.record.frozen.workflow_id,node(&f.record),role.stage(),f.record.result.as_ref().unwrap().request_sha256,id,original_result.to_string(),invalid,crate::store::now()]).unwrap();
            tx.commit().unwrap();
        }
        f.record.transport_state=Some("rejected".into());
        let before=f.store.job(&id).unwrap();
        let c=f.store.conn().unwrap();
        let events:i64=c.query_row("SELECT COUNT(*) FROM events",[],|r|r.get(0)).unwrap();
        let step:(String,String)=c.query_row("SELECT state,error FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2",params![f.record.frozen.workflow_id,node(&f.record)],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(step,("rejected".into(),invalid.into()));
        assert_eq!(before["result"],original_result);
        assert!(before["prepared"]["sdkDomainResult"].is_null());
        assert!(before["prepared"]["sdkDomainResultSha256"].is_null());
        let context=actual_context_with_invalid(&c,&f.store.root,&f.record,Some(invalid)).unwrap();
        assert_eq!(context.role,role);
        assert_eq!(context.provider_thread_id,"rejected-creative-provider-thread");
        assert_eq!(context.job_id,id);
        assert!(actual_context(&c,&f.store.root,&f.record).unwrap_err().contains("ROLE_NOT_ACCEPTED_OR_UNKNOWN"));
        assert!(actual_context_with_invalid(&c,&f.store.root,&f.record,Some("forged invalid explanation")).unwrap_err().contains("ORIGINAL_JOB_BINDING"));
        assert_eq!(f.store.job(&id).unwrap(),before,"reading a rejected contributor must preserve its original record");
        assert_eq!(c.query_row::<i64,_,_>("SELECT COUNT(*) FROM events",[],|r|r.get(0)).unwrap(),events);
        for table in ["workflow_validations_v1","workflow_adoptions_v1"] {
            assert_eq!(c.query_row::<i64,_,_>(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap(),0,"creative-context history is not an adoption or ContentReview grant");
        }
        c.execute("UPDATE jobs SET error='tampered stored invalid error' WHERE id=?1",[&id]).unwrap();
        assert!(actual_context_with_invalid(&c,&f.store.root,&f.record,Some(invalid)).unwrap_err().contains("ORIGINAL_JOB_BINDING"));
        c.execute("UPDATE jobs SET error=?1 WHERE id=?2",params![invalid,id]).unwrap();
        for field in ["sdkDomainResult","sdkDomainResultSha256"] {
            let mut tampered=rejected_prepared.clone();tampered[field]=accepted_prepared[field].clone();
            c.execute("UPDATE jobs SET prepared=?1 WHERE id=?2",params![tampered.to_string(),id]).unwrap();
            assert!(actual_context_with_invalid(&c,&f.store.root,&f.record,Some(invalid)).unwrap_err().contains("SDK_ORIGINAL_RUNTIME_BINDING"),"rejected {field} must remain absent");
        }
        c.execute("UPDATE jobs SET prepared=?1 WHERE id=?2",params![rejected_prepared.to_string(),id]).unwrap();
        assert_eq!(f.store.job(&id).unwrap(),before);
    }
}
