//! Narrow bridge for newly dispatched foundationConflictResolution callbacks.
//! Invoke inside sdk_commit's existing transaction, after BOTH job/node CASes
//! and before commit. Any error must abort that transaction. No old node is
//! back-signed; this grants Structure only, never review/adoption/consumption.
use crate::pipeline::{digest, TextRequest};
use crate::store::{s, Result};
use crate::workflow_artifacts as artifacts;
use rusqlite::{params, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

const VERSION: &str = "foundation-conflict-resolution/v1";
fn fail(message: &str) -> String {
    format!("FOUNDATION_CONFLICT_ARTIFACT: {message}")
}

/// Implements only the schema subset emitted by the conflict domain module.
/// Unknown validation keywords fail closed instead of pretending validation.
fn shape(value: &Value, schema: &Value, root: &Value) -> Result<()> {
    let object = schema.as_object().ok_or_else(|| fail("schema不是对象"))?;
    for key in object.keys() {
        if ![
            "$ref",
            "$defs",
            "type",
            "enum",
            "anyOf",
            "properties",
            "required",
            "additionalProperties",
            "items",
            "minItems",
            "maxItems",
            "uniqueItems",
            "minLength",
        ]
        .contains(&key.as_str())
        {
            return Err(fail("冲突schema出现未支持的校验关键字"));
        }
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let pointer = reference.strip_prefix('#').ok_or_else(|| fail("不支持外部schema引用"))?;
        return shape(value, root.pointer(pointer).ok_or_else(|| fail("schema引用缺失"))?, root);
    }
    if let Some(options) = schema["anyOf"].as_array() {
        if options.iter().any(|option| shape(value, option, root).is_ok()) {
            return Ok(());
        }
        return Err(fail("回包不符合任何允许的决议结构"));
    }
    if let Some(values) = schema["enum"].as_array() {
        if !values.contains(value) {
            return Err(fail("决议字段越过冻结枚举"));
        }
    }
    let matches = |kind: &str| match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        _ => false,
    };
    if let Some(kind) = schema["type"].as_str() {
        if !matches(kind) {
            return Err(fail("决议字段类型不符"));
        }
    }
    if let Some(kinds) = schema["type"].as_array() {
        if !kinds.iter().filter_map(Value::as_str).any(matches) {
            return Err(fail("决议可空字段类型不符"));
        }
    }
    if let Some(map) = value.as_object() {
        for key in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !map.contains_key(key) {
                return Err(fail("决议缺必填字段"));
            }
        }
        for (key, value) in map {
            if let Some(property) = schema["properties"].get(key) {
                shape(value, property, root)?;
            } else if schema["additionalProperties"] == false {
                return Err(fail("决议含额外字段"));
            }
        }
    }
    if let Some(array) = value.as_array() {
        if schema["minItems"].as_u64().is_some_and(|n| array.len() < n as usize)
            || schema["maxItems"].as_u64().is_some_and(|n| array.len() > n as usize)
        {
            return Err(fail("决议数组数量不符"));
        }
        if schema["uniqueItems"] == true {
            let mut unique = HashSet::new();
            if array.iter().any(|v| !unique.insert(v.to_string())) {
                return Err(fail("决议数组含重复项"));
            }
        }
        if let Some(item) = schema.get("items") {
            for value in array {
                shape(value, item, root)?;
            }
        }
    }
    if value.as_str().is_some_and(|v| schema["minLength"].as_u64().is_some_and(|n| v.chars().count() < n as usize)) {
        return Err(fail("决议文本长度不足"));
    }
    Ok(())
}

struct CallbackVerifier<'a> {
    request: &'a TextRequest,
    reason: Option<&'a str>,
}
impl artifacts::Verifier for CallbackVerifier<'_> {
    fn id(&self) -> &str {
        "foundation-conflict-structure-and-domain-callback/v1"
    }
    fn capability(&self) -> artifacts::Capability {
        artifacts::Capability::Structure
    }
    fn verify(
        &self,
        candidate: &artifacts::StoredArtifact,
        evidence: &artifacts::StoredArtifact,
    ) -> Result<artifacts::Verdict> {
        if evidence.artifact.payload["received"] != candidate.artifact.payload {
            return Err(fail("候选不是原收到决议"));
        }
        let checked = shape(&candidate.artifact.payload, &self.request.schema, &self.request.schema);
        if self.reason.is_none() && checked.is_err() {
            return Err(fail("接受回调未满足真实结构校验，事务必须回滚"));
        }
        Ok(artifacts::Verdict {
            accepted: self.reason.is_none() && checked.is_ok(),
            findings: json!({"mechanicalStructurePassed":checked.is_ok(),"mechanicalError":checked.err(),"domainCallbackAccepted":self.reason.is_none(),"domainError":self.reason,"contentReviewPerformed":false,"visualReviewPerformed":false,"adoptionGranted":false}),
        })
    }
}

pub(crate) fn record_callback(
    tx: &Transaction<'_>,
    request: &TextRequest,
    snapshot: &Value,
    original_job: &Value,
    workflow: Option<&str>,
    node_key: &str,
    raw: &Value,
    value: &Value,
    reason: Option<&str>,
) -> Result<Option<Value>> {
    let Some(marker) = request.input.get("foundationConflictResolution") else {
        return Ok(None);
    };
    if marker["version"] != VERSION
        || s(original_job, "status") != "received"
        || s(original_job, "provider") != "codex_sdk"
    {
        return Err(fail("只允许新SDK决议的原received回调，不补签旧节点"));
    }
    let run = workflow.filter(|s| !s.is_empty()).ok_or_else(|| fail("缺正式工作流"))?;
    if reason.is_some_and(|s| s.trim().is_empty()) || raw != value {
        return Err(fail("新决议回调不得改写原回包或提供空拒绝原因"));
    }
    let decoded: TextRequest = serde_json::from_value(snapshot.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_value(decoded).map_err(|e| e.to_string())?
        != serde_json::to_value(request).map_err(|e| e.to_string())?
    {
        return Err(fail("回调与冻结请求不匹配"));
    }
    let manifest = &request.input["foundationConflictManifest"];
    let mut unsigned = manifest.clone();
    unsigned.as_object_mut().ok_or_else(|| fail("缺冲突清单"))?.remove("manifestSha256");
    if manifest["version"] != VERSION
        || manifest["manifestSha256"] != digest(&unsigned.to_string())
        || request.stage != format!("foundation:{}", s(manifest, "stage"))
        || request.input["scope"] != manifest["scope"]
        || request.input["sourceSha256"] != manifest["sourceSha256"]
    {
        return Err(fail("清单摘要、阶段或源范围不匹配"));
    }
    for key in [
        "sourceSha256",
        "manifestSha256",
        "causeSha256",
        "issueSetSha256",
        "originalInputSha256",
        "originalResultSha256",
        "currentBasisSha256",
    ] {
        if marker[key] != manifest[key] {
            return Err(fail("尝试marker与清单不一致"));
        }
    }
    let job_id = s(original_job, "id");
    let snapshot_text = snapshot.to_string();
    let request_hash = digest(&snapshot_text);
    let (state,saved_snapshot,saved_raw,prepared,error,receipt):(String,String,String,String,Option<String>,String)=tx.query_row("SELECT state,snapshot,result,prepared,error,receipt FROM jobs WHERE id=?1 AND kind='text' AND provider='codex_sdk'",[job_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|e|e.to_string())?;
    let expected_state = if reason.is_some() { "rejected" } else { "completed" };
    let node_state = if reason.is_some() { "rejected" } else { "accepted" };
    let prepared: Value = serde_json::from_str(&prepared).map_err(|e| e.to_string())?;
    let receipt: Value = serde_json::from_str(&receipt).map_err(|e| e.to_string())?;
    if state != expected_state
        || saved_snapshot != snapshot_text
        || saved_raw != raw.to_string()
        || error.as_deref() != reason
        || receipt["contract"] != "desktop-runner/v1"
        || receipt["validation"] != node_state
    {
        return Err(fail("原子job CAS后像不匹配"));
    }
    if reason.is_none()
        && (prepared["sdkDomainResult"] != *value || prepared["sdkDomainResultSha256"] != digest(&value.to_string()))
    {
        return Err(fail("领域结果后像不匹配"));
    }
    let checkpoint:(String,String,String,String,Option<String>)=tx.query_row("SELECT state,request_hash,text_job_id,result,error FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2 AND stage=?3",params![run,node_key,request.stage],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
    if checkpoint
        != (node_state.into(), request_hash.clone(), job_id.into(), value.to_string(), reason.map(str::to_owned))
    {
        return Err(fail("原子checkpoint CAS后像不匹配"));
    }
    let received: String = tx
        .query_row(
            "SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",
            [job_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if received != json!({"requestHash":request_hash,"value":raw}).to_string() {
        return Err(fail("原接收事件不匹配"));
    }
    let (event_id, native): (i64, String) = tx
        .query_row(
            "SELECT id,data FROM events WHERE job_id=?1 AND type='sdk_raw_result' ORDER BY id DESC LIMIT 1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let native: Value = serde_json::from_str(&native).map_err(|e| e.to_string())?;
    let native_text = native["rawText"].as_str().ok_or_else(|| fail("缺SDK原始文本"))?;
    if original_job["prepared"]["sdkRawTextSha256"] != digest(native_text)
        || prepared["sdkRawTextSha256"] != digest(native_text)
    {
        return Err(fail("SDK原始文本摘要不匹配"));
    }
    let mut inputs: BTreeMap<String, String> = [
        ("request", request_hash.clone()),
        ("rules", digest(&request.rule_snapshot.to_string())),
        ("schema", digest(&request.schema.to_string())),
        ("nativeRaw", digest(native_text)),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v))
    .collect();
    for key in ["manifestSha256", "originalInputSha256", "originalResultSha256", "currentBasisSha256"] {
        inputs.insert(key.into(), s(manifest, key).into());
    }
    let identity = format!("{run}/{node_key}");
    let prefix = format!("foundation-conflict:{job_id}");
    let raw_id = format!("{prefix}:raw");
    let candidate_id = format!("{prefix}:candidate");
    let validation_id = format!("{prefix}:validation");
    artifacts::install(tx)?;
    artifacts::put(
        tx,
        &artifacts::Artifact {
            id: raw_id.clone(),
            identity: identity.clone(),
            kind: artifacts::Kind::Raw,
            contract_version: VERSION.into(),
            source_sha256: s(manifest, "sourceSha256").into(),
            inputs: inputs.clone(),
            producer: "codex_sdk".into(),
            author_attempt: job_id.into(),
            parents: vec![],
            payload: json!({"sdkRawResultEventId":event_id,"sdkRawResult":native,"received":raw,"requestSnapshot":snapshot}),
        },
    )?;
    let candidate = artifacts::put(
        tx,
        &artifacts::Artifact {
            id: candidate_id.clone(),
            identity,
            kind: artifacts::Kind::Candidate,
            contract_version: VERSION.into(),
            source_sha256: s(manifest, "sourceSha256").into(),
            inputs,
            producer: "foundation-conflict-domain".into(),
            author_attempt: job_id.into(),
            parents: vec![raw_id.clone()],
            payload: value.clone(),
        },
    )?;
    let validation =
        artifacts::validate(tx, &validation_id, &candidate_id, &raw_id, &CallbackVerifier { request, reason })?;
    Ok(Some(
        json!({"rawArtifactId":raw_id,"candidateArtifactId":candidate_id,"validationId":validation_id,"candidateSha256":candidate.payload_sha256,"capability":"structure","accepted":validation.accepted,"adopted":false}),
    ))
}

#[cfg(test)]
#[path = "workflow_conflict_artifacts_tests.rs"]
mod tests;
