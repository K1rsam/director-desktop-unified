//! Exact-byte carrier for already verified lite job packets. Not a production
//! route, workspace verifier, reviewer attestation or dispatch authorization.
//! The only creative payload stored in serde_json::Value is a UTF-8 STRING;
//! precision-sensitive packet JSON is never converted to Value and reserialized.
use crate::series_lite_contract::VerifiedPack;
use crate::series_lite_edit::{canonical, fields, parse, sha256, Json};
use crate::series_lite_job_packet::{CompiledPacket, PACKET_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const PROFILE: &str = "series-lite-packet-utf8/v1";
pub const MODEL_RULE: &str = "packetUtf8是本岗位完整冻结JSON文档的UTF-8原文字符串，按其中task、bindings、rules和blobs阅读；不另行概括规则或丢弃完整设计。所有引用及精确数字以原字符串为准。carrier只运输供料，不代表独审通过、正式采用或生成授权。输出仍服从本岗位另外冻结的响应schema，不回传载体或包。";
const INPUT_KEYS: &[&str] = &[
    "episode",
    "role",
    "requestOrigin",
    "bindings",
    "review",
    "rules",
    "scope",
    "skill",
    "compiler",
];
type Result<T> = std::result::Result<T, String>;
fn need(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("SERIES_LITE_TRANSPORT: {code}"))
    }
}
fn generic<T>(r: std::result::Result<T, String>) -> Result<T> {
    r.map_err(|_| "SERIES_LITE_TRANSPORT: invalid packet structure".into())
}
fn hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn json_value<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v)
        .map_err(|_| "SERIES_LITE_TRANSPORT: metadata serialization failed".into())
}
fn context_hash(context: &RoleContext) -> Result<String> {
    let encoded = serde_json::to_vec(context)
        .map_err(|_| "SERIES_LITE_TRANSPORT: metadata serialization failed")?;
    // Independent of serde_json's preserve_order feature and saved key order.
    Ok(sha256(&canonical(&generic(parse(&encoded))?)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Author,
    Finisher,
    Reviewer,
}
impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Finisher => "finisher",
            Self::Reviewer => "reviewer",
        }
    }
    pub fn stage(self) -> &'static str {
        match self {
            Self::Author => "series-lite:author",
            Self::Finisher => "series-lite:finisher",
            Self::Reviewer => "series-lite:reviewer",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        match s {
            "author" => Ok(Self::Author),
            "finisher" => Ok(Self::Finisher),
            "reviewer" => Ok(Self::Reviewer),
            _ => Err("SERIES_LITE_TRANSPORT: unsupported role".into()),
        }
    }
}

/// Supplied from the controller's frozen execution-context ledger, NOT from a
/// model assertion. This is a distinct conversation/context identity, not an
/// account identity; switching the user's SDK account is not forbidden here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contributor {
    pub role: Role,
    pub context_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleContext {
    /// Exact canonical integer spelling, including values larger than f64/i128.
    pub episode: String,
    pub role: Role,
    /// Controller-allocated unique role iteration/attempt slot. Uniqueness is
    /// ledger-owned; this layer only freezes/binds the supplied ID. It must not
    /// be inferred from a provider account or reused for a different revision.
    pub iteration_id: String,
    pub context_id: String,
    /// Complete creative-context history from the controller, including all
    /// actual finisher/repair contexts. This carrier checks consistency, not
    /// completeness. The upstream reviewer contract does not require that a
    /// finisher must have run, but any one that did run must appear here.
    pub prior_contributors: Vec<Contributor>,
}
impl RoleContext {
    fn validate(&self) -> Result<()> {
        need(
            !self.context_id.trim().is_empty(),
            "missing execution context",
        )?;
        need(
            !self.iteration_id.trim().is_empty(),
            "missing role iteration",
        )?;
        let n = generic(parse(self.episode.as_bytes()))?;
        need(
            n.is_integer() && canonical(&n) == self.episode.as_bytes(),
            "episode must be exact canonical integer",
        )?;
        let mut seen = BTreeSet::new();
        for c in &self.prior_contributors {
            need(
                c.role != Role::Reviewer
                    && !c.context_id.trim().is_empty()
                    && seen.insert(&c.context_id),
                "invalid contributor ledger",
            )?;
        }
        if self.role != Role::Author {
            need(
                self.prior_contributors
                    .iter()
                    .any(|c| c.role == Role::Author),
                "missing authoring context ledger",
            )?;
            need(
                !self
                    .prior_contributors
                    .iter()
                    .any(|c| c.context_id == self.context_id),
                "finisher or independent reviewer reused authoring context",
            )?;
        }
        Ok(())
    }
}

/// Store this in the immutable request/attempt identity separately from the
/// model-facing payload. restore() NEVER derives its expected pin from payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransportIdentity {
    pub profile: String,
    pub packet_file_sha256: String,
    pub packet_bytes: usize,
    pub input_fingerprint: String,
    pub episode: String,
    pub role: Role,
    pub rule_identity_sha256: String,
    pub role_context_sha256: String,
}
#[derive(Clone, Debug)]
pub struct FrozenCarrier {
    pub input: Value,
    pub identity: TransportIdentity,
}
/// Returned only after exact file/hash/fingerprint/rules/role/context binding.
/// It carries supplied evidence, not an independent content-review verdict.
#[derive(Clone, Debug)]
pub struct RestoredPacket {
    pub bytes: Vec<u8>,
    pub identity: TransportIdentity,
}

struct Checked {
    packet: Json,
    role: Role,
    episode: String,
    fingerprint: String,
    rule_hash: String,
    inputs: usize,
    rules: usize,
    blobs: usize,
}
fn checked(raw: &[u8]) -> Result<Checked> {
    generic(check_structure(raw))
}
fn check_structure(raw: &[u8]) -> Result<Checked> {
    std::str::from_utf8(raw).map_err(|_| "packet must be UTF-8")?;
    let packet = parse(raw)?;
    let mut keys = INPUT_KEYS.to_vec();
    keys.extend([
        "version",
        "maxBytes",
        "blobs",
        "inputFingerprint",
        "task",
        "evidenceBoundary",
        "packetSha256",
    ]);
    fields(&packet, &keys, &[])?;
    need(
        packet.get("version")?.string()? == PACKET_VERSION,
        "unsupported packet contract",
    )?;
    need(canonical(&packet) == raw, "noncanonical packet bytes")?;
    let max = packet
        .get("maxBytes")?
        .integer()
        .and_then(|n| usize::try_from(n).ok());
    need(
        max.is_some_and(|n| n > 0 && raw.len() <= n),
        "packet budget invalid",
    )?;
    let file_self = packet.get("packetSha256")?.string()?;
    need(hex(file_self), "invalid packet self hash")?;
    let mut body = packet.object()?.clone();
    body.remove("packetSha256");
    need(
        sha256(&canonical(&Json::Object(body))) == file_self,
        "packet self hash mismatch",
    )?;
    let fingerprint = packet.get("inputFingerprint")?.string()?.to_string();
    need(hex(&fingerprint), "invalid input fingerprint")?;
    let inputs = Json::Object(
        INPUT_KEYS
            .iter()
            .map(|k| Ok(((*k).to_owned(), packet.get(k)?.clone())))
            .collect::<Result<BTreeMap<_, _>>>()?,
    );
    need(
        sha256(&canonical(&inputs)) == fingerprint,
        "input fingerprint mismatch",
    )?;
    let role = Role::parse(packet.get("role")?.string()?)?;
    let episode = packet.get("episode")?;
    need(episode.is_integer(), "episode is not integer")?;
    let episode = String::from_utf8(canonical(episode)).map_err(|_| "invalid episode")?;
    for key in ["skill", "compiler"] {
        fields(packet.get(key)?, &["version", "sha256"], &[])?;
        need(
            !packet.get(key)?.get("version")?.string()?.trim().is_empty()
                && hex(packet.get(key)?.get("sha256")?.string()?),
            "invalid rule identity",
        )?;
    }
    fields(packet.get("task")?, &["role", "contract"], &[])?;
    for value in packet.get("task")?.object()?.values() {
        need(!value.string()?.trim().is_empty(), "missing task contract")?;
    }
    need(
        !packet.get("evidenceBoundary")?.string()?.trim().is_empty(),
        "missing evidence boundary",
    )?;
    fields(packet.get("requestOrigin")?, &["path", "sha256"], &[])?;
    need(
        hex(packet.get("requestOrigin")?.get("sha256")?.string()?),
        "invalid origin fingerprint",
    )?;
    let blobs = packet.get("blobs")?.object()?;
    need(!blobs.is_empty(), "empty blob pool")?;
    for (key, body) in blobs {
        fields(body, &["format", "value"], &[])?;
        need(
            hex(key) && sha256(&canonical(body)) == *key,
            "invalid blob checksum",
        )?;
        let f = body.get("format")?.string()?;
        need(matches!(f, "text" | "json"), "unsupported blob format")?;
        if f == "text" {
            body.get("value")?.string()?;
        }
    }
    let mut used = BTreeSet::new();
    let bindings = packet.get("bindings")?.array()?;
    let mut kinds = BTreeSet::new();
    let review = packet.get("review")?;
    let current = packet.get("compiler")?.get("version")?.string()? == "lingmo-lite-role-rules-v2";
    // v2 explicitly permits no preparatory review. Old v1 retains its contract.
    need(current || *review != Json::Null, "legacy packet requires original review")?;
    for record in bindings
        .iter()
        .chain(((*review != Json::Null).then_some(review)).into_iter())
    {
        fields(
            record,
            &["kind", "path", "sha256", "originBytes", "blob"],
            &["pointer"],
        )?;
        let key = record.get("blob")?.string()?;
        need(
            hex(key) && blobs.contains_key(key) && hex(record.get("sha256")?.string()?),
            "input binding missing or corrupt",
        )?;
        need(
            record.get("originBytes")?.integer().is_some_and(|n| n >= 0),
            "invalid origin size",
        )?;
        used.insert(key.to_owned());
        kinds.insert(record.get("kind")?.string()?);
    }
    let mut required = vec!["source", "direction", "assets", "visual-baseline"];
    if !current { required.extend(["state", "entry"]); }
    if role != Role::Author {
        required.extend(["draft", "plan", "selection"])
    }
    need(
        required.iter().all(|kind| kinds.contains(kind)),
        "missing full role dependencies",
    )?;
    let rules = packet.get("rules")?.array()?;
    let mut paths = BTreeSet::new();
    for rule in rules {
        fields(
            rule,
            &[
                "kind",
                "path",
                "sha256",
                "originBytes",
                "adjustments",
                "blob",
            ],
            &[],
        )?;
        let key = rule.get("blob")?.string()?;
        let path = rule.get("path")?.string()?;
        need(
            rule.get("kind")?.string()? == "rules"
                && paths.insert(path)
                && hex(rule.get("sha256")?.string()?)
                && hex(key),
            "invalid rule binding",
        )?;
        let body = blobs.get(key).ok_or("missing rule blob")?;
        need(
            body.get("format")?.string()? == "text",
            "rule blob not text",
        )?;
        body.get("value")?.string()?;
        need(
            rule.get("originBytes")?.integer().is_some_and(|n| n >= 0),
            "invalid rule size",
        )?;
        for adjustment in rule.get("adjustments")?.array()? {
            need(
                !adjustment.string()?.trim().is_empty(),
                "invalid rule adjustment",
            )?;
        }
        used.insert(key.to_owned());
    }
    need(
        ["references/creative-core.md", "references/schema.md"]
            .iter()
            .all(|p| paths.contains(p))
            && paths.contains(format!("references/{}.md", role.as_str()).as_str()),
        "missing complete role rules",
    )?;
    need(
        used == blobs.keys().cloned().collect(),
        "unreferenced or missing complete blob",
    )?;
    let identity = Json::Object(
        ["skill", "compiler", "rules"]
            .into_iter()
            .map(|k| Ok((k.to_owned(), packet.get(k)?.clone())))
            .collect::<Result<_>>()?,
    );
    let result = Checked {
        role,
        episode,
        fingerprint,
        rule_hash: sha256(&canonical(&identity)),
        inputs: bindings.len(),
        rules: rules.len(),
        blobs: blobs.len(),
        packet,
    };
    Ok(result)
}
fn check_current_rules(packet: &Json, role: Role, pack: &VerifiedPack) -> Result<()> {
    let all = pack.selected_role_rules(
        role.as_str(),
        &["fantasy-expression-methods"],
        &["system-presentation"],
    )?;
    let blobs = packet.get("blobs")?.object()?;
    for rule in packet.get("rules")?.array()? {
        let raw = all
            .get(rule.get("path")?.string()?)
            .ok_or("SERIES_LITE_TRANSPORT: rule not in pinned role pack")?;
        let body = blobs
            .get(rule.get("blob")?.string()?)
            .ok_or("SERIES_LITE_TRANSPORT: missing complete rule body")?;
        need(
            body.get("value")?.string()?.as_bytes() == raw
                && rule.get("sha256")?.string()? == sha256(raw)
                && rule.get("originBytes")?.integer() == Some(raw.len() as i128)
                && rule.get("adjustments")?.array()?.is_empty(),
            "actual rule bytes differ from pinned pack",
        )?;
    }
    let version_bytes = pack.file_bytes("VERSION")?;
    let compiler_bytes = pack.file_bytes("scripts/job_packet.py")?;
    need(
        packet.get("skill")?.get("version")?.string()?
            == std::str::from_utf8(version_bytes).map_err(|_| "invalid pack version")?.trim()
            && packet.get("skill")?.get("sha256")?.string()? == sha256(version_bytes),
        "skill version identity mismatch",
    )?;
    need(
        packet.get("compiler")?.get("version")?.string()? == pack.compiler_version()
            && packet.get("compiler")?.get("sha256")?.string()? == sha256(compiler_bytes),
        "compiler identity mismatch",
    )
}
fn check_context(c: &Checked, context: &RoleContext) -> Result<()> {
    context.validate()?;
    need(
        c.role == context.role && c.episode == context.episode,
        "slot role or episode mismatch",
    )
}
fn identity(raw: &[u8], c: &Checked, context: &RoleContext) -> Result<TransportIdentity> {
    Ok(TransportIdentity {
        profile: PROFILE.into(),
        packet_file_sha256: sha256(raw),
        packet_bytes: raw.len(),
        input_fingerprint: c.fingerprint.clone(),
        episode: c.episode.clone(),
        role: c.role,
        rule_identity_sha256: c.rule_hash.clone(),
        role_context_sha256: context_hash(context)?,
    })
}

/// Caller supplies CompiledPacket returned by assemble/verify (not a Value copy).
/// Filesystem/source-scope and authoring-history completeness remain caller-owned
/// trust boundaries. This layer rechecks packet integrity and current rule bytes.
pub fn freeze_verified(
    packet: &CompiledPacket,
    context: &RoleContext,
    pack: &VerifiedPack,
) -> Result<FrozenCarrier> {
    need(
        hex(&packet.sha256) && sha256(&packet.bytes) == packet.sha256,
        "compiled packet file hash mismatch",
    )?;
    let c = checked(&packet.bytes)?;
    check_context(&c, context)?;
    need(
        c.fingerprint == packet.input_fingerprint
            && c.inputs == packet.inputs
            && c.rules == packet.rules
            && c.blobs == packet.unique_bodies,
        "compiled metadata mismatch",
    )?;
    generic(check_current_rules(&c.packet, c.role, pack))?;
    let identity = identity(&packet.bytes, &c, context)?;
    let string = std::str::from_utf8(&packet.bytes)
        .map_err(|_| "SERIES_LITE_TRANSPORT: packet is not UTF-8")?;
    let input = json!({"transportProfile":PROFILE,"packetUtf8":string,"identity":json_value(&identity)?,"roleContext":json_value(context)?,"transportRule":MODEL_RULE});
    Ok(FrozenCarrier { input, identity })
}

/// Read the original frozen payload using a separately pinned attempt identity.
/// Today's Skill/workspace is deliberately not consulted: historical byte and
/// rule pins are checked, not silently migrated to current rules. The caller
/// must obtain expected/context from its immutable attempt ledger, not input.
pub fn restore(
    input: &Value,
    expected: &TransportIdentity,
    context: &RoleContext,
) -> Result<RestoredPacket> {
    let object = input
        .as_object()
        .ok_or("SERIES_LITE_TRANSPORT: carrier must be object")?;
    need(
        object.len() == 5
            && [
                "transportProfile",
                "packetUtf8",
                "identity",
                "roleContext",
                "transportRule",
            ]
            .iter()
            .all(|k| object.contains_key(*k)),
        "unknown or missing carrier fields",
    )?;
    need(
        input["transportProfile"] == PROFILE
            && input["transportRule"] == MODEL_RULE
            && expected.profile == PROFILE,
        "transport contract mismatch",
    )?;
    let actual: TransportIdentity = serde_json::from_value(input["identity"].clone())
        .map_err(|_| "SERIES_LITE_TRANSPORT: invalid transport identity")?;
    need(&actual == expected, "frozen attempt identity mismatch")?;
    let frozen_context: RoleContext = serde_json::from_value(input["roleContext"].clone())
        .map_err(|_| "SERIES_LITE_TRANSPORT: invalid role context")?;
    need(
        &frozen_context == context,
        "frozen execution context mismatch",
    )?;
    let raw = input["packetUtf8"]
        .as_str()
        .ok_or("SERIES_LITE_TRANSPORT: packetUtf8 must be string, never parsed Value")?
        .as_bytes();
    need(
        raw.len() == expected.packet_bytes
            && hex(&expected.packet_file_sha256)
            && sha256(raw) == expected.packet_file_sha256,
        "frozen exact bytes mismatch",
    )?;
    let c = checked(raw)?;
    check_context(&c, context)?;
    need(
        identity(raw, &c, context)? == *expected,
        "recomputed packet identity mismatch",
    )?;
    Ok(RestoredPacket {
        bytes: raw.to_vec(),
        identity: actual,
    })
}

#[cfg(test)]
#[path = "series_lite_transport_tests.rs"]
mod tests;
