//! Immutable workflow artifacts. Explicit installation is a write migration;
//! every read works against an old database without creating tables. Publishing
//! adapters retain responsibility for domain validation and real review traces.
use crate::pipeline::digest;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;
pub(crate) type Inputs = BTreeMap<String, String>;
pub(crate) const CONTRACT: &str = "workflow-artifacts/v1";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Raw,
    Candidate,
    ManualDraft,
    ReviewRaw,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Capability {
    Structure,
    SourceCoverage,
    ContentReview,
    VisualReview,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Artifact {
    pub id: String,
    pub identity: String,
    pub kind: Kind,
    pub contract_version: String,
    pub source_sha256: String,
    pub inputs: Inputs,
    pub producer: String,
    pub author_attempt: String,
    pub parents: Vec<String>,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredArtifact {
    pub artifact: Artifact,
    pub payload_sha256: String,
    pub input_sha256: String,
}

/// Implemented by trusted Rust publishing adapters, not deserialized model JSON.
/// `verify` must actually run its mechanical check or verify a persisted human /
/// independent-review receipt and native visual-read trace. A model saying PASS
/// is not an implementation. The core additionally proves exact artifact binding.
pub(crate) trait Verifier {
    fn id(&self) -> &str;
    fn capability(&self) -> Capability;
    fn verify(&self, candidate: &StoredArtifact, evidence: &StoredArtifact) -> Result<Verdict>;
}
pub(crate) struct Verdict {
    pub accepted: bool,
    pub findings: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Validation {
    pub id: String,
    pub candidate_id: String,
    pub candidate_sha256: String,
    pub evidence_id: String,
    pub evidence_sha256: String,
    pub input_sha256: String,
    pub capability: Capability,
    pub verifier: String,
    pub accepted: bool,
    pub findings: Value,
}

pub(crate) struct Adopt<'a> {
    pub id: &'a str,
    pub candidate_id: &'a str,
    pub identity: &'a str,
    pub expected_previous: u64,
    pub source_sha256: &'a str,
    pub contract_version: &'a str,
    pub current_inputs: &'a Inputs,
    pub required: &'a [Capability],
    pub validation_ids: &'a [String],
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Adoption {
    pub id: String,
    pub identity: String,
    pub version: u64,
    pub previous_version: u64,
    pub candidate_id: String,
    pub candidate_sha256: String,
    pub source_sha256: String,
    pub input_sha256: String,
    pub capabilities: Vec<Capability>,
    pub validation_ids: Vec<String>,
    pub contract_version: String,
}
pub(crate) struct Consume<'a> {
    pub id: &'a str,
    pub consumer: &'a str,
    pub identity: &'a str,
    pub adopted_version: u64,
    pub expected_payload_sha256: &'a str,
    pub source_sha256: &'a str,
    pub contract_version: &'a str,
    pub current_inputs: &'a Inputs,
    pub required: &'a [Capability],
}

fn error(message: &str) -> String {
    format!("WORKFLOW_ARTIFACT: {message}")
}
fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(digest(&serde_json::to_string(value).map_err(|e| e.to_string())?))
}
fn sha(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn inputs_valid(inputs: &Inputs) -> bool {
    !inputs.is_empty() && inputs.iter().all(|(k, v)| !k.trim().is_empty() && sha(v))
}
fn atomic<T>(tx: &Transaction<'_>, action: impl FnOnce() -> Result<T>) -> Result<T> {
    tx.execute_batch("SAVEPOINT workflow_artifact_write").map_err(|e| e.to_string())?;
    match action() {
        Ok(value) => match tx.execute_batch("RELEASE workflow_artifact_write") {
            Ok(()) => Ok(value),
            Err(e) => {
                let _ = tx.execute_batch("ROLLBACK TO workflow_artifact_write; RELEASE workflow_artifact_write");
                Err(e.to_string())
            }
        },
        Err(e) => {
            tx.execute_batch("ROLLBACK TO workflow_artifact_write; RELEASE workflow_artifact_write")
                .map_err(|rollback| format!("{e}; rollback: {rollback}"))?;
            Err(e)
        }
    }
}

/// Additive only: deliberately does not touch schema_version or legacy tables.
pub(crate) fn install(tx: &Transaction<'_>) -> Result<()> {
    atomic(tx, || {
        tx.execute_batch("CREATE TABLE IF NOT EXISTS workflow_artifacts_v1(id TEXT PRIMARY KEY,identity TEXT NOT NULL,kind TEXT NOT NULL,source_sha256 TEXT NOT NULL,contract_version TEXT NOT NULL,payload_sha256 TEXT NOT NULL,input_sha256 TEXT NOT NULL,data TEXT NOT NULL,data_sha256 TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS workflow_validations_v1(id TEXT PRIMARY KEY,candidate_id TEXT NOT NULL REFERENCES workflow_artifacts_v1(id),evidence_id TEXT NOT NULL REFERENCES workflow_artifacts_v1(id),data TEXT NOT NULL,data_sha256 TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS workflow_adoptions_v1(id TEXT PRIMARY KEY,identity TEXT NOT NULL,version INTEGER NOT NULL,candidate_id TEXT NOT NULL REFERENCES workflow_artifacts_v1(id),data TEXT NOT NULL,data_sha256 TEXT NOT NULL,UNIQUE(identity,version));
CREATE TABLE IF NOT EXISTS workflow_adoption_heads_v1(identity TEXT PRIMARY KEY,version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS workflow_consumptions_v1(id TEXT PRIMARY KEY,adoption_id TEXT NOT NULL REFERENCES workflow_adoptions_v1(id),data TEXT NOT NULL,data_sha256 TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS workflow_uses_v1(id TEXT PRIMARY KEY,adoption_id TEXT NOT NULL REFERENCES workflow_adoptions_v1(id),data TEXT NOT NULL,data_sha256 TEXT NOT NULL);").map_err(|e|e.to_string())?;
        for table in [
            "workflow_artifacts_v1",
            "workflow_validations_v1",
            "workflow_adoptions_v1",
            "workflow_consumptions_v1",
            "workflow_uses_v1",
        ] {
            for action in ["UPDATE", "DELETE"] {
                tx.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS {table}_no_{action} BEFORE {action} ON {table} BEGIN SELECT RAISE(ABORT,'immutable workflow artifact'); END;")).map_err(|e|e.to_string())?;
            }
        }
        Ok(())
    })
}
fn exists(c: &Connection, table: &str) -> Result<bool> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [table], |r| r.get(0))
        .map_err(|e| e.to_string())
}
fn require_installed(c: &Connection) -> Result<()> {
    if !exists(c, "workflow_artifacts_v1")? {
        Err(error("需显式安装成果扩展，读取不会自动建表"))
    } else {
        Ok(())
    }
}

pub(crate) fn artifact(c: &Connection, id: &str) -> Result<Option<StoredArtifact>> {
    if !exists(c, "workflow_artifacts_v1")? {
        return Ok(None);
    }
    let row:Option<(String,String,String,String,String,String,String,String)>=c.query_row("SELECT identity,kind,source_sha256,contract_version,payload_sha256,input_sha256,data,data_sha256 FROM workflow_artifacts_v1 WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|e|e.to_string())?;
    row.map(|(identity, kind, source, contract, payload_hash, input_hash, data, data_hash)| {
        if digest(&data) != data_hash {
            return Err(error("成果完整来源描述摘要损坏"));
        }
        let a: Artifact = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        if a.id != id
            || a.identity != identity
            || serde_json::to_string(&a.kind).unwrap() != kind
            || a.source_sha256 != source
            || a.contract_version != contract
            || hash(&a.payload)? != payload_hash
            || hash(&a.inputs)? != input_hash
        {
            return Err(error("成果身份或负载摘要损坏"));
        }
        Ok(StoredArtifact { artifact: a, payload_sha256: payload_hash, input_sha256: input_hash })
    })
    .transpose()
}

pub(crate) fn put(tx: &Transaction<'_>, a: &Artifact) -> Result<StoredArtifact> {
    require_installed(tx)?;
    if [&a.id, &a.identity, &a.contract_version, &a.producer, &a.author_attempt].iter().any(|s| s.trim().is_empty())
        || !sha(&a.source_sha256)
        || !inputs_valid(&a.inputs)
    {
        return Err(error("成果缺稳定身份、来源、合同或必要输入摘要"));
    }
    if let Some(old) = artifact(tx, &a.id)? {
        if old.artifact == *a {
            return Ok(old);
        }
        return Err(error("成果ID已绑定其他内容，禁止覆盖"));
    }
    let mut parents = BTreeSet::new();
    let mut has_origin = false;
    for id in &a.parents {
        if id == &a.id || !parents.insert(id) {
            return Err(error("成果上游引用重复或自循环"));
        }
        let parent = artifact(tx, id)?.ok_or_else(|| error("成果上游不存在"))?;
        if parent.artifact.source_sha256 != a.source_sha256 {
            return Err(error("成果上游来源不一致"));
        }
        has_origin |= matches!(parent.artifact.kind, Kind::Raw | Kind::ManualDraft);
    }
    if a.kind == Kind::Candidate && !has_origin {
        return Err(error("候选缺原回包或明确人工草稿来源"));
    }
    let stored =
        StoredArtifact { artifact: a.clone(), payload_sha256: hash(&a.payload)?, input_sha256: hash(&a.inputs)? };
    atomic(tx, || {
        tx.execute(
            "INSERT INTO workflow_artifacts_v1 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                a.id,
                a.identity,
                serde_json::to_string(&a.kind).unwrap(),
                a.source_sha256,
                a.contract_version,
                stored.payload_sha256,
                stored.input_sha256,
                serde_json::to_string(a).map_err(|e| e.to_string())?,
                hash(a)?
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(stored)
    })
}

fn record<T: Serialize>(tx: &Transaction<'_>, table: &str, id: &str, link: (&str, &str), value: &T) -> Result<()> {
    if id.trim().is_empty() {
        return Err(error("记录缺幂等ID"));
    }
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    let digest = hash(value)?;
    let old: Option<(String, String)> = tx
        .query_row(&format!("SELECT data,data_sha256 FROM {table} WHERE id=?1"), [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((data, sha)) = old {
        if data == text && sha == digest {
            return Ok(());
        }
        return Err(error("幂等记录ID已绑定其他事实"));
    }
    tx.execute(
        &format!("INSERT INTO {table}(id,{},data,data_sha256) VALUES(?1,?2,?3,?4)", link.0),
        params![id, link.1, text, digest],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
fn read_record<T: for<'de> Deserialize<'de>>(c: &Connection, table: &str, id: &str) -> Result<Option<T>> {
    if !exists(c, table)? {
        return Ok(None);
    }
    let row: Option<(String, String)> = c
        .query_row(&format!("SELECT data,data_sha256 FROM {table} WHERE id=?1"), [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(|e| e.to_string())?;
    row.map(|(text, sha)| {
        if digest(&text) != sha {
            return Err(error("证据记录摘要损坏"));
        }
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if value["id"] != id {
            return Err(error("证据记录身份不匹配"));
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    })
    .transpose()
}

fn review_binding(candidate: &StoredArtifact, evidence: &StoredArtifact, capability: Capability) -> Result<()> {
    if candidate.artifact.kind != Kind::Candidate {
        return Err(error("只有明确候选可进入验证，人工草稿不是候选"));
    }
    if evidence.artifact.source_sha256 != candidate.artifact.source_sha256
        || evidence.input_sha256 != candidate.input_sha256
    {
        return Err(error("验证证据来源或必要输入不匹配"));
    }
    if matches!(capability, Capability::ContentReview | Capability::VisualReview) {
        let payload = &evidence.artifact.payload;
        if evidence.artifact.kind != Kind::ReviewRaw
            || evidence.artifact.author_attempt == candidate.artifact.author_attempt
            || payload["candidateSha256"] != candidate.payload_sha256
            || payload["inputSha256"] != candidate.input_sha256
            || payload["authorAttempt"] != candidate.artifact.author_attempt
            || payload["reviewerAttempt"] != evidence.artifact.author_attempt
        {
            return Err(error("内容审阅必须绑定当前候选及独立审阅原回包"));
        }
        if capability == Capability::VisualReview {
            let media = payload["readMediaSha256"]
                .as_array()
                .filter(|m| !m.is_empty())
                .ok_or_else(|| error("视觉审阅缺实际读图证据"))?;
            if media.iter().any(|v| {
                v.as_str().is_none_or(|s| {
                    !candidate.artifact.inputs.iter().any(|(key, hash)| key.starts_with("media:") && hash == s)
                })
            }) {
                return Err(error("读图证据未绑定当前媒体输入"));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate(
    tx: &Transaction<'_>,
    id: &str,
    candidate_id: &str,
    evidence_id: &str,
    verifier: &dyn Verifier,
) -> Result<Validation> {
    require_installed(tx)?;
    let candidate = artifact(tx, candidate_id)?.ok_or_else(|| error("候选不存在"))?;
    origin_bound(tx, &candidate)?;
    let evidence = artifact(tx, evidence_id)?.ok_or_else(|| error("验证原始证据不存在"))?;
    if verifier.id().trim().is_empty() {
        return Err(error("验证适配器没有版本身份"));
    }
    review_binding(&candidate, &evidence, verifier.capability())?;
    if let Some(old) = read_record::<Validation>(tx, "workflow_validations_v1", id)? {
        if old.candidate_id == candidate_id
            && old.candidate_sha256 == candidate.payload_sha256
            && old.input_sha256 == candidate.input_sha256
            && old.evidence_id == evidence_id
            && old.evidence_sha256 == evidence.payload_sha256
            && old.verifier == verifier.id()
            && old.capability == verifier.capability()
        {
            return Ok(old);
        }
        return Err(error("验证ID已被另一组证据使用"));
    }
    // Adapter errors create no validation record; a completed negative verdict
    // is retained and can never satisfy an adoption capability.
    let verdict = verifier.verify(&candidate, &evidence)?;
    let value = Validation {
        id: id.into(),
        candidate_id: candidate_id.into(),
        candidate_sha256: candidate.payload_sha256,
        evidence_id: evidence_id.into(),
        evidence_sha256: evidence.payload_sha256,
        input_sha256: candidate.input_sha256,
        capability: verifier.capability(),
        verifier: verifier.id().into(),
        accepted: verdict.accepted,
        findings: verdict.findings,
    };
    atomic(tx, || {
        tx.execute(
            "INSERT INTO workflow_validations_v1 VALUES(?1,?2,?3,?4,?5)",
            params![
                id,
                candidate_id,
                evidence_id,
                serde_json::to_string(&value).map_err(|e| e.to_string())?,
                hash(&value)?
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(value)
    })
}

fn current_inputs(candidate: &StoredArtifact, source: &str, inputs: &Inputs) -> Result<()> {
    if !inputs_valid(inputs) || candidate.artifact.source_sha256 != source || candidate.artifact.inputs != *inputs {
        return Err(error("必要输入或来源已改变，不能沿用原验证/采用"));
    }
    Ok(())
}
fn capabilities(values: &[Capability]) -> BTreeSet<Capability> {
    values.iter().copied().collect()
}
fn ensure_candidate(candidate: &StoredArtifact) -> Result<()> {
    if candidate.artifact.kind != Kind::Candidate {
        Err(error("原回包或人工草稿不能直接采用"))
    } else {
        Ok(())
    }
}
fn origin_bound(c: &Connection, candidate: &StoredArtifact) -> Result<()> {
    ensure_candidate(candidate)?;
    let mut found = false;
    for id in &candidate.artifact.parents {
        let parent = artifact(c, id)?.ok_or_else(|| error("候选的原始来源记录丢失"))?;
        if parent.artifact.source_sha256 != candidate.artifact.source_sha256 {
            return Err(error("候选原始来源发生漂移"));
        }
        found |= matches!(parent.artifact.kind, Kind::Raw | Kind::ManualDraft);
    }
    if !found {
        return Err(error("候选没有可验证的原始来源"));
    }
    Ok(())
}

pub(crate) fn adopt(tx: &Transaction<'_>, request: Adopt<'_>) -> Result<Adoption> {
    require_installed(tx)?;
    let candidate = artifact(tx, request.candidate_id)?.ok_or_else(|| error("候选不存在"))?;
    origin_bound(tx, &candidate)?;
    current_inputs(&candidate, request.source_sha256, request.current_inputs)?;
    if request.id.trim().is_empty()
        || request.identity != candidate.artifact.identity
        || request.contract_version != candidate.artifact.contract_version
        || !request.required.contains(&Capability::Structure)
    {
        return Err(error("采用身份/合同不匹配或缺最低结构校验要求"));
    }
    let mut provided = BTreeSet::new();
    let mut ids = request.validation_ids.to_vec();
    ids.sort();
    ids.dedup();
    if ids.len() != request.validation_ids.len() {
        return Err(error("验证证据重复"));
    }
    for id in &ids {
        let validation =
            read_record::<Validation>(tx, "workflow_validations_v1", id)?.ok_or_else(|| error("验证记录不存在"))?;
        if !validation.accepted
            || validation.candidate_id != candidate.artifact.id
            || validation.candidate_sha256 != candidate.payload_sha256
            || validation.input_sha256 != candidate.input_sha256
        {
            return Err(error("验证未通过或绑定其他候选"));
        }
        let evidence = artifact(tx, &validation.evidence_id)?.ok_or_else(|| error("验证原始证据丢失"))?;
        if evidence.payload_sha256 != validation.evidence_sha256 {
            return Err(error("验证证据摘要改变"));
        }
        review_binding(&candidate, &evidence, validation.capability)?;
        provided.insert(validation.capability);
    }
    if !capabilities(request.required).is_subset(&provided) {
        return Err(error("采用缺所需验证能力；机械通过不等于内容/视觉通过"));
    }
    let value = Adoption {
        id: request.id.into(),
        identity: request.identity.into(),
        version: request
            .expected_previous
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| error("采用版本溢出"))?,
        previous_version: request.expected_previous,
        candidate_id: request.candidate_id.into(),
        candidate_sha256: candidate.payload_sha256,
        source_sha256: request.source_sha256.into(),
        input_sha256: candidate.input_sha256,
        capabilities: provided.into_iter().collect(),
        validation_ids: ids,
        contract_version: request.contract_version.into(),
    };
    if let Some(old) = read_record::<Adoption>(tx, "workflow_adoptions_v1", request.id)? {
        if old == value {
            return Ok(old);
        }
        return Err(error("采用幂等ID冲突"));
    }
    atomic(tx, || {
        let current: u64 = tx
            .query_row("SELECT version FROM workflow_adoption_heads_v1 WHERE identity=?1", [request.identity], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        if current != request.expected_previous {
            return Err(error("采用前版CAS不匹配，未覆盖当前成果"));
        }
        tx.execute(
            "INSERT INTO workflow_adoptions_v1 VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                value.id,
                value.identity,
                value.version,
                value.candidate_id,
                serde_json::to_string(&value).map_err(|e| e.to_string())?,
                hash(&value)?
            ],
        )
        .map_err(|e| e.to_string())?;
        let changed = if current == 0 {
            tx.execute("INSERT INTO workflow_adoption_heads_v1 VALUES(?1,?2)", params![value.identity, value.version])
        } else {
            tx.execute(
                "UPDATE workflow_adoption_heads_v1 SET version=?1 WHERE identity=?2 AND version=?3",
                params![value.version, value.identity, current],
            )
        }
        .map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err(error("采用指针CAS失败"));
        }
        Ok(value)
    })
}

pub(crate) fn adoption(c: &Connection, identity: &str, version: u64) -> Result<Option<Adoption>> {
    if !exists(c, "workflow_adoptions_v1")? {
        return Ok(None);
    }
    let id: Option<String> = c
        .query_row(
            "SELECT id FROM workflow_adoptions_v1 WHERE identity=?1 AND version=?2",
            params![identity, version],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    id.map(|id| {
        let a: Adoption = read_record(c, "workflow_adoptions_v1", &id)?.ok_or_else(|| error("采用记录丢失"))?;
        if a.identity != identity || a.version != version {
            return Err(error("采用版本或身份不匹配"));
        }
        Ok(a)
    })
    .transpose()
}
fn adopted_candidate(c: &Connection, a: &Adoption) -> Result<StoredArtifact> {
    let candidate = artifact(c, &a.candidate_id)?.ok_or_else(|| error("已采用候选丢失"))?;
    if candidate.payload_sha256 != a.candidate_sha256
        || candidate.input_sha256 != a.input_sha256
        || candidate.artifact.identity != a.identity
        || candidate.artifact.source_sha256 != a.source_sha256
        || candidate.artifact.contract_version != a.contract_version
    {
        return Err(error("采用与候选不匹配"));
    }
    origin_bound(c, &candidate)?;
    Ok(candidate)
}

pub(crate) fn consume(tx: &Transaction<'_>, request: Consume<'_>) -> Result<Value> {
    require_installed(tx)?;
    let adopted = adoption(tx, request.identity, request.adopted_version)?
        .ok_or_else(|| error("指定采用版本不存在，completed不能替代采用"))?;
    let candidate = adopted_candidate(tx, &adopted)?;
    current_inputs(&candidate, request.source_sha256, request.current_inputs)?;
    if request.consumer.trim().is_empty()
        || request.required.is_empty()
        || request.contract_version != adopted.contract_version
        || request.expected_payload_sha256 != adopted.candidate_sha256
        || !capabilities(request.required).is_subset(&capabilities(&adopted.capabilities))
    {
        return Err(error("下游缺明确成果摘要、合同或所需验证能力"));
    }
    // Verify bound receipts again; completed jobs and a cached capability label
    // cannot stand in for the original evidence at consumption time.
    let mut proven = BTreeSet::new();
    for id in &adopted.validation_ids {
        let v =
            read_record::<Validation>(tx, "workflow_validations_v1", id)?.ok_or_else(|| error("采用验证记录丢失"))?;
        let evidence = artifact(tx, &v.evidence_id)?.ok_or_else(|| error("采用审阅证据丢失"))?;
        if !v.accepted
            || v.candidate_id != adopted.candidate_id
            || v.candidate_sha256 != adopted.candidate_sha256
            || v.input_sha256 != adopted.input_sha256
            || v.evidence_sha256 != evidence.payload_sha256
        {
            return Err(error("采用验证与当前必要输入不一致"));
        }
        review_binding(&candidate, &evidence, v.capability)?;
        proven.insert(v.capability);
    }
    if proven != capabilities(&adopted.capabilities) {
        return Err(error("采用能力标签与原验证记录不一致"));
    }
    let value = json!({"version":CONTRACT,"id":request.id,"consumer":request.consumer,"adoptionId":adopted.id,"identity":adopted.identity,"adoptedVersion":adopted.version,"candidateSha256":adopted.candidate_sha256,"sourceSha256":request.source_sha256,"inputSha256":hash(request.current_inputs)?,"requiredCapabilities":capabilities(request.required)});
    atomic(tx, || {
        record(tx, "workflow_consumptions_v1", request.id, ("adoption_id", &adopted.id), &value)?;
        Ok(value)
    })
}

/// A domain adapter validates scene/episode scope before calling this function.
/// Uses are separate immutable facts, not a new design or silent re-adoption.
pub(crate) fn record_use(
    tx: &Transaction<'_>,
    id: &str,
    identity: &str,
    adopted_version: u64,
    source_sha256: &str,
    scope: &Value,
    supersedes: Option<&str>,
) -> Result<Value> {
    require_installed(tx)?;
    let adopted = adoption(tx, identity, adopted_version)?.ok_or_else(|| error("用途缺明确采用版本"))?;
    adopted_candidate(tx, &adopted)?;
    if source_sha256 != adopted.source_sha256 || !scope.is_object() || scope.as_object().is_none_or(|s| s.is_empty()) {
        return Err(error("用途来源或范围不合法"));
    }
    if let Some(previous) = supersedes {
        let old = read_record::<Value>(tx, "workflow_uses_v1", previous)?.ok_or_else(|| error("前用途记录不存在"))?;
        if old["identity"] != identity
            || old["adoptionId"] != adopted.id
            || old["sourceSha256"] != source_sha256
            || previous == id
        {
            return Err(error("用途接续不能改换设计或来源"));
        }
    }
    let value = json!({"version":CONTRACT,"id":id,"identity":identity,"adoptionId":adopted.id,"adoptedVersion":adopted.version,"candidateSha256":adopted.candidate_sha256,"sourceSha256":source_sha256,"scope":scope,"supersedes":supersedes});
    atomic(tx, || {
        record(tx, "workflow_uses_v1", id, ("adoption_id", &adopted.id), &value)?;
        Ok(value)
    })
}

#[cfg(test)]
#[path = "workflow_artifacts_tests.rs"]
mod tests;
