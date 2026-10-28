//! Initial lite 0.4.1 three-role validation and artifact adoption, never a runner.
//! The caller owns the project lease and narrow source/design/unit CAS, invokes
//! this inside that same transaction, then projects units in the transaction.
//! No planned context ID, model PASS, receipt seal, or successful download alone
//! grants ContentReview. Actual original provider contexts are checked below.
use crate::pipeline::{digest, TextRequest};
use crate::series_lite_bundle::{verify_lingmo_v7, BundleGate};
use crate::series_lite_bundle_gate::NativeLegacyBundleGate;
use crate::series_lite_controller_store::{self as controller, RoleRecord};
use crate::series_lite_edit::{parse, sha256, Json};
use crate::series_lite_review::{self as review, BoundaryInput};
use crate::series_lite_transport::Role;
use crate::store::{Result, Store};
use crate::text_transport::{AttemptState, Backend, FrozenAttempt};
use crate::workflow_artifacts::{
    self as artifacts, Artifact, Capability, Inputs, Kind, StoredArtifact, Verdict, Verifier,
};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub(crate) const VERSION: &str = "desktop-lite041-publish/v1";
fn need(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("LITE_PUBLISH:{code}"))
    }
}
fn encoded<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| e.to_string())
}
fn raw_json(bytes: &[u8]) -> Result<Value> {
    parse(bytes)?; // duplicate-key/precision/nesting gate, before metadata parsing
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    need(
        meta.is_file() && !meta.file_type().is_symlink() && !reparse && meta.len() <= limit,
        "FILE_TYPE_OR_LIMIT",
    )?;
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    need(raw.len() as u64 <= limit, "FILE_LIMIT")?;
    Ok(raw)
}
fn inside(root: &Path, relative: &str) -> Result<PathBuf> {
    need(
        !relative.is_empty()
            && !relative.contains(['\\', ':', '\0'])
            && Path::new(relative)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "PATH_OUTSIDE_WORKSPACE",
    )?;
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let path = std::fs::canonicalize(root.join(relative)).map_err(|e| e.to_string())?;
    need(
        path != root && path.starts_with(&root),
        "PATH_OUTSIDE_WORKSPACE",
    )?;
    Ok(path)
}
fn event(c: &Connection, job: &str, kind: &str) -> Result<Value> {
    let text: String = c
        .query_row(
            "SELECT data FROM events WHERE job_id=?1 AND type=?2 ORDER BY id DESC LIMIT 1",
            params![job, kind],
            |r| r.get(0),
        )
        .map_err(|_| format!("LITE_PUBLISH:MISSING_{kind}"))?;
    raw_json(text.as_bytes())
}
fn node(record: &RoleRecord) -> String {
    let f = &record.frozen;
    format!("{}:{}",f.request.stage,digest(&json!({"episode":f.context.episode,"iteration":f.context.iteration_id,"context":f.context.context_id}).to_string()))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActualContext {
    pub role: Role,
    pub job_id: String,
    pub provider: String,
    /// CLI and SDK use the same Codex provider conversation namespace.
    pub provider_thread_id: String,
    pub runtime_evidence_sha256: String,
}
#[derive(Clone)]
struct NativeJob {
    id: String,
    provider: String,
    snapshot: Value,
    prepared: Value,
    raw: Vec<u8>,
}
fn original_job(c: &Connection, root: &Path, record: &RoleRecord, invalid: Option<&str>) -> Result<NativeJob> {
    let result = record
        .result
        .as_ref()
        .ok_or("LITE_PUBLISH:ROLE_RESULT_MISSING")?;
    need(
        record.transport_state.as_deref() == Some(if invalid.is_some(){"rejected"}else{"completed"}),
        "ROLE_NOT_ACCEPTED_OR_UNKNOWN",
    )?;
    need(
        uuid::Uuid::parse_str(&result.job_id).is_ok(),
        "INVALID_JOB_ID",
    )?;
    let (provider,state,snapshot,prepared,raw,receipt,error):(String,String,String,Option<String>,Option<String>,Option<String>,Option<String>)=c.query_row("SELECT provider,state,snapshot,prepared,result,receipt,error FROM jobs WHERE id=?1 AND kind='text'",[&result.job_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|e|e.to_string())?;
    let snapshot_value = raw_json(snapshot.as_bytes())?;
    let request: TextRequest =
        serde_json::from_value(snapshot_value.clone()).map_err(|e| e.to_string())?;
    let envelope = raw_json(raw.as_deref().ok_or("LITE_PUBLISH:RAW_MISSING")?.as_bytes())?;
    let text = envelope["rawText"]
        .as_str()
        .ok_or("LITE_PUBLISH:RAW_NOT_EXACT")?;
    let receipt = raw_json(
        receipt
            .as_deref()
            .ok_or("LITE_PUBLISH:RECEIPT_MISSING")?
            .as_bytes(),
    )?;
    need(
        state == (if invalid.is_some(){"rejected"}else{"completed"})
            && error.as_deref() == invalid
            && receipt == json!({"contract":"desktop-runner/v1","validation":if invalid.is_some(){"rejected"}else{"accepted"}})
            && matches!(provider.as_str(), "codex" | "codex_sdk")
            && encoded(&request)? == encoded(&record.frozen.request)?
            && digest(&snapshot) == result.request_sha256
            && sha256(text.as_bytes()) == result.native_response_sha256
            && envelope
                == json!({"version":"model-response-utf8/v1","rawText":text,"nativeTextSha256":sha256(text.as_bytes()),"byteLength":text.len()}),
        "ORIGINAL_JOB_BINDING",
    )?;
    let received = event(c, &result.job_id, "text_received_raw")?;
    need(
        received == json!({"requestHash":result.request_sha256,"value":envelope}),
        "ORIGINAL_RECEIVE_EVENT",
    )?;
    need(
        event(c, &result.job_id, "lite_exact_received")?
            == json!({"version":"series-lite-exact-text/v1","requestHash":result.request_sha256,"nativeTextSha256":sha256(text.as_bytes()),"byteLength":text.len()}),
        "EXACT_RECEIVE_EVENT",
    )?;
    let raw = bounded(
        &root
            .join("text-runs")
            .join(&result.job_id)
            .join("result.json"),
        1_048_576,
    )?;
    need(raw == text.as_bytes(), "RESULT_FILE_DRIFT")?;
    Ok(NativeJob {
        id: result.job_id.clone(),
        provider,
        snapshot: snapshot_value,
        prepared: prepared
            .as_deref()
            .map(|s| raw_json(s.as_bytes()))
            .transpose()?
            .unwrap_or(Value::Null),
        raw,
    })
}
fn observed_thread(c: &Connection, job: &str, sdk: bool) -> Result<(String, Value)> {
    let kind = if sdk { "sdk_event" } else { "codex_event" };
    let mut q = c
        .prepare("SELECT id,data FROM events WHERE job_id=?1 AND type=?2 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map(params![job, kind], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    need(
        !rows.is_empty() && rows.len() <= 2048,
        "ACTUAL_CONTEXT_EVENTS_MISSING_OR_LIMIT",
    )?;
    let mut thread = None;
    let mut request_id: Option<String> = None;
    let mut finished = false;
    let mut turns = 0;
    let mut evidence = vec![];
    for (i, (id, text)) in rows.iter().enumerate() {
        need(text.len() <= 1_048_576, "CONTEXT_EVENT_LIMIT")?;
        let frame = raw_json(text.as_bytes())?;
        let raw = if sdk {
            let id = frame["requestId"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or("LITE_PUBLISH:SDK_CONTEXT_REQUEST_ID")?;
            if let Some(previous) = &request_id {
                need(previous == id, "SDK_CONTEXT_REQUEST_DRIFT")?;
            } else {
                request_id = Some(id.into());
            }
            need(
                frame["type"] == "sdk_event"
                    && frame["protocolVersion"] == crate::text_sdk::PROTOCOL
                    && frame["attemptId"] == job
                    && frame["sequence"].as_u64() == Some(i as u64 + 1)
                    && frame["turnId"].is_null(),
                "SDK_CONTEXT_SEQUENCE_OR_ATTEMPT",
            )?;
            &frame["rawEvent"]
        } else {
            &frame
        };
        match raw["type"].as_str() {
            Some("thread.started") => {
                let value = raw["thread_id"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("LITE_PUBLISH:ACTUAL_THREAD_MISSING")?;
                need(
                    thread.is_none() && !finished,
                    "MULTIPLE_OR_LATE_CONTEXT_STARTS",
                )?;
                thread = Some(value.to_string());
            }
            Some("turn.started") => {
                need(thread.is_some() && !finished, "TURN_WITHOUT_CONTEXT")?;
                turns += 1;
            }
            Some("turn.completed") => {
                need(
                    thread.is_some() && turns == 1 && !finished,
                    "INVALID_CONTEXT_COMPLETION",
                )?;
                finished = true;
            }
            Some("turn.failed" | "error") => {
                return Err("LITE_PUBLISH:FAILED_CONTEXT_NOT_CONTENT_REVIEW".into())
            }
            _ => {}
        }
        if sdk {
            if let Some(thread) = &thread {
                need(frame["threadId"] == *thread, "SDK_CONTEXT_DRIFT")?;
            }
        }
        evidence.push(json!({"eventId":id,"sha256":digest(text)}));
    }
    need(finished && turns == 1, "ACTUAL_CONTEXT_NOT_COMPLETED")?;
    Ok((
        thread.unwrap(),
        json!({"events":evidence,"requestId":request_id,"lastSequence":rows.len()}),
    ))
}
fn actual_context(c: &Connection, root: &Path, record: &RoleRecord) -> Result<ActualContext> {
    actual_context_with_invalid(c,root,record,None)
}
fn actual_context_with_invalid(c: &Connection, root: &Path, record: &RoleRecord, invalid:Option<&str>) -> Result<ActualContext> {
    let job = original_job(c, root, record, invalid)?;
    let dir = root.join("text-runs").join(&job.id);
    let (thread, events) = observed_thread(c, &job.id, job.provider == "codex_sdk")?;
    let extra = if job.provider == "codex_sdk" {
        let attempt_raw = bounded(&dir.join("sdk-attempt.json"), 4_194_304)?;
        let attempt = FrozenAttempt::load(&dir.join("sdk-attempt.json"))?;
        let sdk_identity: crate::text_sdk::SdkIdentity =
            serde_json::from_value(attempt.runtime_identity.clone()).map_err(|e| e.to_string())?;
        sdk_identity.validate_receipt()?;
        let compiled = crate::text_executor::compile_sdk_attempt_input(&attempt)?;
        let compiled_raw = bounded(&dir.join("sdk-compiled.json"), 2_097_152)?;
        let compiled_file = raw_json(&compiled_raw)?;
        let raw = attempt
            .raw_result
            .as_ref()
            .ok_or("LITE_PUBLISH:SDK_NATIVE_RESULT_MISSING")?;
        let raw_text = std::str::from_utf8(&job.raw).map_err(|e| e.to_string())?;
        let domain = json!({"version":"model-response-utf8/v1","rawText":raw_text,"nativeTextSha256":sha256(&job.raw),"byteLength":job.raw.len()});
        need(
            attempt.backend == Backend::CodexSdk
                && attempt.state == AttemptState::Completed
                && attempt.dispatch_started
                && attempt.attempt_id == job.id
                && attempt.workflow_id == record.frozen.workflow_id
                && attempt.logical_node_id == node(record)
                && attempt.thread_id.as_deref() == Some(&thread)
                && attempt.last_sequence == events["lastSequence"].as_u64().unwrap() * 2 + 1
                && attempt.turn_id.is_none()
                && encoded(&attempt.request)? == encoded(&record.frozen.request)?
                && attempt.runtime_identity == job.snapshot["executorIdentity"]
                && raw["rawText"].as_str().map(str::as_bytes) == Some(job.raw.as_slice())
                && compiled_file
                    == json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":digest(&compiled.prompt),"schemaSha256":digest(&compiled.schema)})
                && job.prepared["compiledPromptSha256"] == compiled_file["promptSha256"]
                && job.prepared["compiledSchemaSha256"] == compiled_file["schemaSha256"]
                && job.prepared["sdkRequestHash"] == record.result.as_ref().unwrap().request_sha256
                && job.prepared["sdkAttemptId"] == job.id
                && job.prepared["sdkDispatchStarted"] == true
                && job.prepared["sdkRawTextSha256"] == sha256(&job.raw)
                && (if invalid.is_some() {
                    job.prepared["sdkDomainResult"].is_null() && job.prepared["sdkDomainResultSha256"].is_null()
                }else{
                    job.prepared["sdkDomainResult"] == domain && job.prepared["sdkDomainResultSha256"] == digest(&domain.to_string())
                }),
            "SDK_ORIGINAL_RUNTIME_BINDING",
        )?;
        let ack = event(c, &job.id, "sdk_dispatch_receipt")?;
        let terminal = event(c, &job.id, "sdk_observation")?;
        need(
            ack["type"] == "response"
                && ack["protocolVersion"] == crate::text_sdk::PROTOCOL
                && ack["ok"] == true
                && ack["attemptId"] == job.id
                && ack["requestId"] == events["requestId"]
                && terminal["type"] == "observation"
                && terminal["protocolVersion"] == crate::text_sdk::PROTOCOL
                && terminal["attemptId"] == job.id
                && terminal["requestId"] == ack["requestId"]
                && terminal["state"] == "transport_completed"
                && terminal["backend"] == "codex_sdk"
                && terminal["lastSequence"] == events["lastSequence"]
                && terminal["processTreeStopProven"] == false
                && terminal["threadId"] == thread
                && terminal["turnId"].is_null()
                && terminal["domainAccepted"] == false
                && terminal["semanticRequestHash"] == attempt.semantic_request_hash
                && terminal["executionFingerprint"] == attempt.execution_fingerprint
                && terminal["attemptEnvelopeHash"] == attempt.attempt_envelope_hash
                && terminal["compiledPromptSha256"] == digest(&compiled.prompt)
                && terminal["compiledSchemaSha256"] == digest(&compiled.schema)
                && terminal["rawResult"] == *raw,
            "SDK_TERMINAL_RECEIPT_BINDING",
        )?;
        need(
            bounded(&dir.join("sdk-attempt.json"), 4_194_304)? == attempt_raw
                && bounded(&dir.join("sdk-compiled.json"), 2_097_152)? == compiled_raw,
            "SDK_RUNTIME_FILE_CHANGED_DURING_READ",
        )?;
        json!({"attemptFileSha256":sha256(&attempt_raw),"compiledFileSha256":sha256(&compiled_raw),"ack":ack,"terminal":terminal})
    } else {
        let metadata_raw = bounded(&dir.join("cli-dispatch.json"), 2_097_152)?;
        let metadata = raw_json(&metadata_raw)?;
        let prompt = bounded(&dir.join("cli-prompt.txt"), 2_097_152)?;
        let schema = bounded(&dir.join("schema.json"), 2_097_152)?;
        let compiled = crate::text_executor::compile_codex_input(&record.frozen.request)?;
        let prompt_value = raw_json(&prompt)?;
        need(
            matches!(
                metadata["format"].as_str(),
                Some("legacy-json/v1" | "rules-first-json/v1")
            ) && prompt_value == raw_json(compiled.prompt.as_bytes())?
                && metadata_raw == metadata.to_string().as_bytes()
                && schema == compiled.schema.as_bytes()
                && metadata["version"] == "cli-dispatch-format/v1"
                && metadata["jobId"] == job.id
                && metadata["workflowId"] == record.frozen.workflow_id
                && metadata["nodeKey"] == node(record)
                && metadata["requestHash"] == record.result.as_ref().unwrap().request_sha256
                && metadata["freshSemanticNode"] == true
                && metadata["retryOrigin"].is_null()
                && metadata["promptSha256"] == sha256(&prompt)
                && metadata["schemaSha256"] == sha256(&schema)
                && metadata["promptBytes"].as_u64() == Some(prompt.len() as u64)
                && metadata["promptUtf16"].as_u64()
                    == Some(
                        std::str::from_utf8(&prompt)
                            .map_err(|e| e.to_string())?
                            .encode_utf16()
                            .count() as u64,
                    )
                && event(c, &job.id, "cli_dispatch_frozen")? == metadata,
            "CLI_ORIGINAL_DISPATCH_BINDING",
        )?;
        need(
            bounded(&dir.join("cli-dispatch.json"), 2_097_152)? == metadata_raw
                && bounded(&dir.join("cli-prompt.txt"), 2_097_152)? == prompt
                && bounded(&dir.join("schema.json"), 2_097_152)? == schema,
            "CLI_RUNTIME_FILE_CHANGED_DURING_READ",
        )?;
        json!({"dispatchFileSha256":sha256(&metadata_raw),"promptSha256":sha256(&prompt),"schemaSha256":sha256(&schema)})
    };
    need(
        bounded(&dir.join("result.json"), 1_048_576)? == job.raw,
        "RESULT_CHANGED_DURING_RUNTIME_VERIFICATION",
    )?;
    Ok(ActualContext {
        role: record.frozen.context.role,
        job_id: job.id,
        provider: job.provider,
        provider_thread_id: thread,
        runtime_evidence_sha256: digest(&json!({"events":events,"original":extra}).to_string()),
    })
}
fn distinct_contexts(contexts: &[ActualContext]) -> Result<()> {
    need(
        contexts.len() >= 3
            && contexts[0].role == Role::Author
            && contexts.iter().any(|c|c.role == Role::Finisher)
            && contexts.last().is_some_and(|c|c.role == Role::Reviewer),
        "THREE_ROLES_REQUIRED",
    )?;
    let mut jobs = BTreeSet::new();
    let mut threads = BTreeSet::new();
    for context in contexts {
        need(
            jobs.insert(&context.job_id) && threads.insert(&context.provider_thread_id),
            "ACTUAL_REVIEWER_OR_FINISHER_REUSED_CREATIVE_CONTEXT",
        )?;
    }
    Ok(())
}

pub(crate) struct AdoptionRequest<'a> {
    pub run: &'a str,
    pub episode_id: &'a str,
    pub episode_number: i64,
    pub workspace_root: &'a Path,
    pub execution_receipt: &'a str,
    /// Frozen separately by the caller's existing run ledger after local run.
    pub execution_receipt_sha256: &'a str,
    pub expected_previous: u64,
    /// Rebuilt by the caller's source/design/visual/boundary narrow CAS, never AI.
    pub current_inputs: &'a Inputs,
}
#[derive(Debug)]
pub(crate) struct AdoptionOutcome {
    pub adoption: Option<artifacts::Adoption>,
    pub candidate_artifact_id: String,
    pub candidate_native_sha256: String,
    pub final_delivery_ready: bool,
    pub boundary_pending: bool,
    pub review: Value,
    pub actual_contexts: Vec<ActualContext>,
}
struct CheckedEvidence {
    candidate: StoredArtifact,
    review: StoredArtifact,
    report: review::Checked,
    mechanical: Value,
    contexts: Vec<ActualContext>,
}
fn exact_contains(container: &Json, expected: &Json) -> bool {
    fn contained(container: &Json, expected: &Json, depth: usize) -> bool {
        if depth > 128 {
            return false;
        }
        container == expected
            || match container {
                Json::Array(a) => a.iter().any(|v| contained(v, expected, depth + 1)),
                Json::Object(o) => o.values().any(|v| contained(v, expected, depth + 1)),
                Json::String(s) => {
                    parse(s.as_bytes()).is_ok_and(|v| contained(&v, expected, depth + 1))
                }
                _ => false,
            }
    }
    contained(container, expected, 0)
}
fn boundary<'a>(
    c: &Connection,
    record: &'a Value,
    source: &Value,
    request: &AdoptionRequest<'_>,
    reviewer: &RoleRecord,
    previous: &'a mut Option<Vec<u8>>,
) -> Result<BoundaryInput<'a>> {
    let late=crate::series_lite_late_boundary::load(c,request.run,request.episode_id)?;
    let frozen_hash=reviewer.frozen.common_inputs.get("boundary");
    let late_bound=late.as_ref().is_some_and(|late|late["boundary"]==*record && late["sourceSha256"]==reviewer.frozen.source_sha256 && late["originalBoundarySha256"].as_str()==frozen_hash.map(String::as_str));
    need(
        !record.is_null()
            && (frozen_hash == Some(&digest(&record.to_string())) || late_bound),
        "BOUNDARY_NOT_FROZEN",
    )?;
    let first = source["episodes"]
        .as_array()
        .and_then(|a| a.first())
        .is_some_and(|e| e["id"] == request.episode_id);
    match record["kind"].as_str() {
        Some("firstInScope") => {
            need(first, "SELECTION_START_IS_NOT_SOURCE_START")?;
            Ok(BoundaryInput::FirstInScope)
        }
        Some("missingPrevious") => {
            need(!first, "SOURCE_START_HAS_NO_PREVIOUS")?;
            Ok(BoundaryInput::MissingPrevious)
        }
        Some("supplied") => {
            need(!first, "SOURCE_START_CANNOT_CLAIM_PREVIOUS")?;
            let identity = record["adoptionIdentity"]
                .as_str()
                .ok_or("LITE_PUBLISH:BOUNDARY_ADOPTION_IDENTITY")?;
            let version = record["version"]
                .as_u64()
                .ok_or("LITE_PUBLISH:BOUNDARY_ADOPTION_VERSION")?;
            let adopted = artifacts::adoption(c, identity, version)?
                .ok_or("LITE_PUBLISH:PREVIOUS_ADOPTION_MISSING")?;
            let artifact = artifacts::artifact(c, &adopted.candidate_id)?
                .ok_or("LITE_PUBLISH:PREVIOUS_CANDIDATE_MISSING")?;
            let raw = artifact.artifact.payload["text"]
                .as_str()
                .ok_or("LITE_PUBLISH:PREVIOUS_CANDIDATE_NOT_EXACT")?
                .as_bytes()
                .to_vec();
            need(
                sha256(&raw) == record["nativeSha256"]
                    && adopted.source_sha256 == reviewer.frozen.source_sha256
                    && artifact.payload_sha256 == adopted.candidate_sha256,
                "PREVIOUS_ADOPTION_DRIFT",
            )?;
            let previous_json = parse(&raw)?;
            let episodes = source["episodes"]
                .as_array()
                .ok_or("LITE_PUBLISH:SOURCE_EPISODES_MISSING")?;
            let index = episodes
                .iter()
                .position(|e| e["id"] == request.episode_id)
                .ok_or("LITE_PUBLISH:CURRENT_EPISODE_MISSING")?;
            let actual_previous = episodes
                .get(
                    index
                        .checked_sub(1)
                        .ok_or("LITE_PUBLISH:NO_PREVIOUS_EPISODE")?,
                )
                .ok_or("LITE_PUBLISH:NO_PREVIOUS_EPISODE")?;
            let previous_number = crate::pipeline::number_value(
                actual_previous["sourceNumber"].as_str().unwrap_or(""),
            )
            .ok_or("LITE_PUBLISH:PREVIOUS_SOURCE_NUMBER_UNKNOWN")?;
            let previous_episodes = previous_json.get("episodes")?.array()?;
            need(
                previous_episodes.len() == 1
                    && previous_episodes[0].get("number")?.integer()
                        == Some(previous_number as i128),
                "SUPPLIED_BOUNDARY_NOT_DIRECT_SOURCE_PREDECESSOR",
            )?;
            let tail = previous_json
                .get("episodes")?
                .array()?
                .first()
                .ok_or("LITE_PUBLISH:PREVIOUS_EPISODE_MISSING")?
                .get("draft")?
                .get("units")?
                .array()?
                .last()
                .ok_or("LITE_PUBLISH:PREVIOUS_TAIL_MISSING")?;
            let packet = parse(
                reviewer.frozen.request.input["packetUtf8"]
                    .as_str()
                    .ok_or("LITE_PUBLISH:REVIEW_PACKET_MISSING")?
                    .as_bytes(),
            )?;
            let supplied = packet.get("bindings")?.array()?.iter().any(|b| {
                matches!(b.get("kind").and_then(Json::string), Ok("state" | "entry"))
                    && b.get("blob")
                        .and_then(Json::string)
                        .ok()
                        .and_then(|key| packet.get("blobs").ok()?.object().ok()?.get(key))
                        .and_then(|body| body.get("value").ok())
                        .is_some_and(|v| exact_contains(v, tail))
            });
            need(supplied, "ACTUAL_PREVIOUS_TAIL_NOT_IN_REVIEW_PACKET")?;
            *previous = Some(raw);
            Ok(BoundaryInput::Supplied {
                previous_bytes: previous.as_deref().unwrap(),
                previous_sha256: record["nativeSha256"]
                    .as_str()
                    .ok_or("LITE_PUBLISH:PREVIOUS_SHA_MISSING")?,
            })
        }
        _ => Err("LITE_PUBLISH:BOUNDARY_KIND_UNSUPPORTED".into()),
    }
}
fn validate(
    c: &Connection,
    store: &Store,
    request: &AdoptionRequest<'_>,
) -> Result<CheckedEvidence> {
    need(
        request.episode_number > 0 && !request.episode_id.is_empty(),
        "EPISODE_IDENTITY",
    )?;
    let parent_raw:String=c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run' AND state IN ('running','partially_blocked','completed')",[request.run],|r|r.get(0)).map_err(|_|"LITE_PUBLISH:PARENT_NOT_ACTIVE")?;
    let parent = raw_json(parent_raw.as_bytes())?;
    need(
        parent["input"]["liteControllerContract"] == controller::VERSION,
        "PARENT_CONTRACT",
    )?;
    let source_raw: String = c
        .query_row(
            "SELECT data FROM sources ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "LITE_PUBLISH:CURRENT_SOURCE_MISSING")?;
    let source = raw_json(source_raw.as_bytes())?;
    let source_episode = source["episodes"]
        .as_array()
        .and_then(|episodes| episodes.iter().find(|e| e["id"] == request.episode_id))
        .ok_or("LITE_PUBLISH:EPISODE_OUTSIDE_SOURCE")?;
    let original_number =
        crate::pipeline::number_value(source_episode["sourceNumber"].as_str().unwrap_or(""))
            .ok_or("LITE_PUBLISH:ORIGINAL_EPISODE_NUMBER_UNKNOWN")?;
    need(
        Some(original_number) == u32::try_from(request.episode_number).ok()
            && parent["input"]["selection"]["episodeIds"]
                .as_array()
                .is_some_and(|a| a.iter().any(|e| e == request.episode_id)),
        "ORIGINAL_EPISODE_OR_SELECTION_MISMATCH",
    )?;
    let v2 = parent["input"]["liteRuntimeContract"] == crate::series_lite_controller_v2::VERSION;
    let mut invalids = Vec::new();
    let mut prior_issues = Vec::new();
    let mut roles = vec![];
    if v2 {
        let slots = crate::series_lite_controller_v2::history(c, request.run, &request.episode_number.to_string())?;
        need(slots.len()>=3, "ROLE_MISSING")?;
        for slot in slots {
            prior_issues = slot.prior_issues;
            invalids.push(slot.invalid);
            roles.push(slot.record);
        }
        need(invalids.last().is_some_and(Option::is_none), "FINAL_REVIEW_INVALID")?;
    } else {
        for role in [Role::Author, Role::Finisher, Role::Reviewer] {
            roles.push(controller::read_role(c, request.run, &request.episode_number.to_string(), role)?.ok_or("LITE_PUBLISH:ROLE_MISSING")?);
            invalids.push(None);
        }
    }
    let author = &roles[0];
    let reviewer = roles.last().ok_or("LITE_PUBLISH:REVIEWER_MISSING")?;
    need(author.frozen.context.role==Role::Author && reviewer.frozen.context.role==Role::Reviewer,"AUTHOR_OR_REVIEWER_ROLE")?;
    let finisher = roles.iter().rev().find(|r|r.frozen.context.role!=Role::Reviewer && r.result.as_ref().is_some_and(|v|v.candidate.is_some())).ok_or("LITE_PUBLISH:CREATIVE_CANDIDATE_MISSING")?;
    let known_creative: BTreeSet<_> = roles.iter().filter(|r|r.frozen.context.role!=Role::Reviewer)
        .filter_map(|r| r.result.as_ref().map(|v| v.job_id.as_str()))
        .collect();
    let mut creative_query=c.prepare("SELECT j.id FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND json_extract(j.snapshot,'$.input.roleContext.episode')=?2 AND (w.stage='series-lite:author' OR w.stage='series-lite:finisher' OR w.stage='series-lite:repair')").map_err(|e|e.to_string())?;
    let creative_ids = creative_query
        .query_map(
            params![request.run, request.episode_number.to_string()],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    need(
        creative_ids.len() == known_creative.len()
            && creative_ids
                .iter()
                .all(|id| known_creative.contains(id.as_str())),
        "CREATIVE_CONTEXT_LEDGER_INCOMPLETE",
    )?;
    for role in &roles {
        let packet = raw_json(
            role.frozen.request.input["packetUtf8"]
                .as_str()
                .ok_or("LITE_PUBLISH:PACKET_MISSING")?
                .as_bytes(),
        )?;
        need(
            packet["skill"]["version"] == crate::series_lite_contract::CURRENT_RELEASE
                && role.frozen.common_inputs == *request.current_inputs
                && role.frozen.source_sha256 == source["sha256"]
                && role.frozen.source_sha256 == parent["input"]["source"]["sha256"],
            "CURRENT_SOURCE_OR_RULE_OR_INPUT_DRIFT",
        )?;
    }
    let contexts = roles.iter().zip(&invalids)
        .map(|(r,invalid)| actual_context_with_invalid(c, &store.root, r, invalid.as_deref()))
        .collect::<Result<Vec<_>>>()?;
    distinct_contexts(&contexts)?;
    let pin = finisher
        .result
        .as_ref()
        .and_then(|r| r.candidate.as_ref())
        .ok_or("LITE_PUBLISH:FINISHER_CANDIDATE_MISSING")?;
    let candidate = artifacts::artifact(c, &pin.artifact_id)?
        .ok_or("LITE_PUBLISH:CANDIDATE_ARTIFACT_MISSING")?;
    let candidate_text = candidate.artifact.payload["text"]
        .as_str()
        .ok_or("LITE_PUBLISH:CANDIDATE_NOT_EXACT")?;
    need(
        candidate.payload_sha256 == pin.payload_sha256
            && sha256(candidate_text.as_bytes()) == pin.native_sha256
            && reviewer.frozen.base_utf8 == candidate_text
            && reviewer.frozen.previous_candidate.as_ref() == Some(pin)
            && author.frozen.common_inputs == finisher.frozen.common_inputs,
        "CURRENT_REVIEW_BASE_MISMATCH",
    )?;
    let review_result = reviewer
        .result
        .as_ref()
        .ok_or("LITE_PUBLISH:REVIEW_RESULT_MISSING")?;
    let review_artifact = artifacts::artifact(c, &review_result.raw_artifact_id)?
        .ok_or("LITE_PUBLISH:REVIEW_ARTIFACT_MISSING")?;
    let raw_review = review_artifact.artifact.payload["text"]
        .as_str()
        .ok_or("LITE_PUBLISH:REVIEW_NOT_EXACT")?;
    need(
        sha256(raw_review.as_bytes()) == review_result.native_response_sha256,
        "REVIEW_ARTIFACT_NOT_ORIGINAL_RESPONSE",
    )?;
    let mut previous = None;
    let boundary_record = if v2 {crate::series_lite_late_boundary::load(c,request.run,request.episode_id)?.map(|v|v["boundary"].clone())}else{None}
        .or(crate::series_lite_desktop_prepare::frozen_boundary(c, request.run, request.episode_id)?
            .filter(|v| !v.is_null())
        ).unwrap_or_else(|| {
                parent["input"]["liteBoundaryByEpisode"][request.episode_number.to_string()].clone()
            });
    let input = boundary(
        c,
        &boundary_record,
        &source,
        request,
        reviewer,
        &mut previous,
    )?;
    if v2 && boundary_record["kind"]=="supplied" {
        crate::series_lite_consumers::verify_boundary(c,&boundary_record,source["sha256"].as_str().ok_or("LITE_PUBLISH:SOURCE_SHA")?)?;
    }
    let checked = review::validate(
        raw_review.as_bytes(),
        candidate_text.as_bytes(),
        &pin.native_sha256,
        &prior_issues,
        input,
    )?;
    let workroot = std::fs::canonicalize(request.workspace_root).map_err(|e| e.to_string())?;
    let project = std::fs::canonicalize(&store.root).map_err(|e| e.to_string())?;
    need(
        workroot != project && workroot.starts_with(&project),
        "WORKSPACE_NOT_PROJECT_OWNED",
    )?;
    let receipt = crate::series_lite_execution::verify_receipt(
        &workroot,
        request.execution_receipt,
        request.execution_receipt_sha256,
        Some(request.episode_number),
    )?;
    let receipt_value = raw_json(&receipt.bytes)?;
    let manifest_path = receipt_value["manifest"]
        .as_str()
        .ok_or("LITE_PUBLISH:MANIFEST_PATH")?;
    let manifest = raw_json(&bounded(&inside(&workroot, manifest_path)?, 2_097_152)?)?;
    let bundle_path = manifest["bundle"]
        .as_str()
        .ok_or("LITE_PUBLISH:BUNDLE_PATH")?;
    need(
        bounded(&inside(&workroot, bundle_path)?, 4_194_304)? == candidate_text.as_bytes(),
        "MECHANICAL_RECEIPT_NOT_CURRENT_CANDIDATE",
    )?;
    let lock = bounded(
        &inside(
            &workroot,
            manifest["sourceLock"]
                .as_str()
                .ok_or("LITE_PUBLISH:SOURCE_LOCK_PATH")?,
        )?,
        4_194_304,
    )?;
    let registry = raw_json(&bounded(
        &inside(
            &workroot,
            receipt_value["registrySnapshot"]["path"]
                .as_str()
                .ok_or("LITE_PUBLISH:REGISTRY_SNAPSHOT_PATH")?,
        )?,
        16_777_216,
    )?)?;
    crate::series_lite_registry::validate_registry_current(&registry, false)?;
    let bundle = raw_json(candidate_text.as_bytes())?;
    need(
        bundle["testFixture"] == false
            && bundle["generationBoundaryVersion"] == "video-prompt-v1"
            && bundle["generationTextVersion"] == "voice-delivery-v1"
            && bundle["sourceHash"] == reviewer.frozen.source_sha256
            && bundle["sourceLockHash"]
                .as_str()
                .is_some_and(|s| s.len() == 64),
        "PRODUCTION_SOURCE_GATE_REQUIRED",
    )?;
    let scope = manifest
        .get("seasonScope")
        .and_then(Value::as_str)
        .map(|p| {
            inside(&workroot, p)
                .and_then(|path| bounded(&path, 4_194_304))
                .and_then(|raw| raw_json(&raw))
        })
        .transpose()?;
    let mut gate = NativeLegacyBundleGate {
        registry: Some(&registry),
        source_lock: Some(&lock),
        season_scope: scope.as_ref(),
    };
    let proof = gate.check(candidate_text.as_bytes())?;
    let preview = bounded(&inside(&workroot, &receipt.preview_path)?, 16_777_216)?;
    let catalog = bounded(
        &inside(&workroot, &format!("{}.assets.json", receipt.preview_path))?,
        16_777_216,
    )?;
    verify_lingmo_v7(
        candidate_text.as_bytes(),
        std::str::from_utf8(&preview).map_err(|e| e.to_string())?,
        std::str::from_utf8(&catalog).map_err(|e| e.to_string())?,
        &mut gate,
    )?;
    let mechanical = json!({"version":VERSION,"receiptPath":request.execution_receipt,"receiptSha256":receipt.file_sha256,"executionFingerprint":receipt.execution_fingerprint,"candidateNativeSha256":pin.native_sha256,"gate":proof.report,"validatorId":proof.validator_id,"previewSha256":sha256(&preview),"catalogSha256":sha256(&catalog)});
    Ok(CheckedEvidence {
        candidate,
        review: review_artifact,
        report: checked,
        mechanical,
        contexts,
    })
}
struct BoundVerifier<'a> {
    id: &'static str,
    capability: Capability,
    evidence: &'a CheckedEvidence,
}
impl Verifier for BoundVerifier<'_> {
    fn id(&self) -> &str {
        self.id
    }
    fn capability(&self) -> Capability {
        self.capability
    }
    fn verify(&self, candidate: &StoredArtifact, evidence: &StoredArtifact) -> Result<Verdict> {
        need(
            candidate.payload_sha256 == self.evidence.candidate.payload_sha256
                && candidate.artifact == self.evidence.candidate.artifact,
            "VERIFIER_CANDIDATE_DRIFT",
        )?;
        let (accepted, findings) = if self.capability == Capability::ContentReview {
            need(evidence == &self.evidence.review, "VERIFIER_REVIEW_DRIFT")?;
            distinct_contexts(&self.evidence.contexts)?;
            (
                self.evidence.report.declared_content_pass
                    && !self.evidence.report.boundary_pending,
                json!({"actualContextIndependence":"VERIFIED","contexts":self.evidence.contexts,"boundaryPending":self.evidence.report.boundary_pending,"review":self.evidence.report.report}),
            )
        } else {
            need(
                evidence.artifact.payload == self.evidence.mechanical,
                "VERIFIER_MECHANICAL_DRIFT",
            )?;
            (true, self.evidence.mechanical.clone())
        };
        Ok(Verdict { accepted, findings })
    }
}
/// No commit and no unit projection here: caller verifies its narrow CAS before
/// this call and projects the returned adoption inside the same transaction.
/// A negative/pending review remains a saved candidate, never final delivery.
pub(crate) fn validate_and_adopt(
    tx: &Transaction<'_>,
    store: &Store,
    request: &AdoptionRequest<'_>,
) -> Result<AdoptionOutcome> {
    let checked = validate(tx, store, request)?;
    artifacts::install(tx)?;
    let candidate = &checked.candidate;
    let base = format!(
        "{}:publish:{}",
        candidate.artifact.id,
        checked.mechanical["receiptSha256"].as_str().unwrap()
    );
    let mechanical = artifacts::put(
        tx,
        &Artifact {
            id: format!("{base}:mechanical"),
            identity: candidate.artifact.identity.clone(),
            kind: Kind::Raw,
            contract_version: candidate.artifact.contract_version.clone(),
            source_sha256: candidate.artifact.source_sha256.clone(),
            inputs: candidate.artifact.inputs.clone(),
            producer: VERSION.into(),
            author_attempt: candidate.artifact.author_attempt.clone(),
            parents: vec![],
            payload: checked.mechanical.clone(),
        },
    )?;
    let mut validations = vec![];
    for capability in [
        Capability::Structure,
        Capability::SourceCoverage,
        Capability::ContentReview,
    ] {
        let (suffix, id, evidence) = match capability {
            Capability::Structure => ("structure", "desktop-lite041-structure/v1", &mechanical),
            Capability::SourceCoverage => ("source", "desktop-lite041-source-lock/v1", &mechanical),
            _ => (
                "content",
                "desktop-lite041-independent-content/v1",
                &checked.review,
            ),
        };
        let v = artifacts::validate(
            tx,
            &format!("{base}:{suffix}"),
            &candidate.artifact.id,
            &evidence.artifact.id,
            &BoundVerifier {
                id,
                capability,
                evidence: &checked,
            },
        )?;
        if v.accepted {
            validations.push(v.id);
        }
    }
    let final_ready = checked.report.declared_content_pass && !checked.report.boundary_pending;
    let adoption = if checked.report.declared_content_pass {
        let required = if final_ready {
            vec![
                Capability::Structure,
                Capability::SourceCoverage,
                Capability::ContentReview,
            ]
        } else {
            vec![Capability::Structure, Capability::SourceCoverage]
        };
        let adoption = artifacts::adopt(
            tx,
            artifacts::Adopt {
                id: &format!("{base}:adoption"),
                candidate_id: &candidate.artifact.id,
                identity: &candidate.artifact.identity,
                expected_previous: request.expected_previous,
                source_sha256: &candidate.artifact.source_sha256,
                contract_version: &candidate.artifact.contract_version,
                current_inputs: request.current_inputs,
                required: &required,
                validation_ids: &validations,
            },
        )?;
        artifacts::consume(
            tx,
            artifacts::Consume {
                id: &format!("{base}:consume"),
                consumer: if final_ready {
                    "stage4:validated-text"
                } else {
                    "storyboard:provisional-text-boundary-pending"
                },
                identity: &adoption.identity,
                adopted_version: adoption.version,
                expected_payload_sha256: &candidate.payload_sha256,
                source_sha256: &candidate.artifact.source_sha256,
                contract_version: &candidate.artifact.contract_version,
                current_inputs: request.current_inputs,
                required: &required,
            },
        )?;
        Some(adoption)
    } else {
        None
    };
    Ok(AdoptionOutcome {
        adoption,
        candidate_artifact_id: candidate.artifact.id.clone(),
        candidate_native_sha256: candidate.artifact.payload["nativeSha256"]
            .as_str()
            .unwrap()
            .into(),
        final_delivery_ready: final_ready,
        boundary_pending: checked.report.boundary_pending,
        review: serde_json::to_value(&checked.report.report).map_err(|e| e.to_string())?,
        actual_contexts: checked.contexts,
    })
}

#[cfg(test)]
#[path = "series_lite_desktop_publish_tests.rs"]
mod tests;
