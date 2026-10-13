//! Engineering-only bridge for an explicitly adopted source reparse. The old
//! visual packet is always verified in its original source; only reading
//! coverage is projected. This is never model input or a new fact adoption.
use super::{digest, LockedSource};
use crate::source_migration::{self, Plan};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) const FIELD: &str = "visualFactSourceReparse";
const VERSION: &str = "visual-fact-source-reparse/v1";

pub(crate) struct Context<'a> {
    current: &'a LockedSource,
    plan: Option<Arc<Plan>>,
    profile: Option<&'a Value>,
    packets: BTreeMap<String, String>,
}

fn ids(cache: &Value) -> Result<Vec<String>, String> {
    cache["rowIds"].as_array().ok_or("源行适配缺少缓存原行")?.iter()
        .map(|id| id.as_str().map(str::to_owned).ok_or("源行适配原行 ID 无效".into())).collect()
}

fn packet_bindings(input: &Value, plan: &Plan) -> Vec<Value> {
    let split_ids: BTreeSet<_> = plan.evidence["rowMapping"].as_array().unwrap().iter()
        .filter(|row|row["newRowIds"].as_array().is_none_or(|ids|ids.len() != 1 || ids[0] != row["oldRowId"]))
        .filter_map(|row|row["oldRowId"].as_str()).collect();
    input["foundation"]["visualFactBatches"].as_array().into_iter().flatten()
        .filter(|cache| ids(cache).is_ok_and(|ids|ids.iter().any(|id|split_ids.contains(id.as_str()))))
        .map(|cache|json!({"inputFingerprint":cache["inputFingerprint"],"cacheSha256":digest(&cache.to_string())})).collect()
}

/// Receipt/dependency evaluation visits the same source many times per asset.
/// Retain at most one successfully verified source pair, never failed proofs.
/// The key hashes actual complete values, not caller-supplied digest fields.
fn verified_plan(old: &Value, next: &Value) -> Result<Arc<Plan>, String> {
    static LAST: OnceLock<Mutex<Option<(String, Arc<Plan>)>>> = OnceLock::new();
    let key = format!("{}:{}", digest(&old.to_string()), digest(&next.to_string()));
    let cache = LAST.get_or_init(||Mutex::new(None));
    if let Some((found, plan)) = cache.lock().map_err(|_|"源行验证缓存不可用")?.as_ref() {
        if *found == key { return Ok(Arc::clone(plan)); }
    }
    let plan = Arc::new(source_migration::plan(old, next)?);
    *cache.lock().map_err(|_|"源行验证缓存不可用")? = Some((key, Arc::clone(&plan)));
    Ok(plan)
}

impl<'a> Context<'a> {
    pub(crate) fn new(input: &'a Value, current: &'a LockedSource) -> Result<Self, String> {
        let Some(profile) = input.get(FIELD) else {
            return Ok(Self { current, plan: None, profile: None, packets: BTreeMap::new() });
        };
        let proof = &profile["migrationProof"];
        let mut payload = proof.clone();
        let object = payload.as_object_mut().ok_or("视觉缓存源行适配缺少独立迁移证明")?;
        object.remove("proofSha256");
        if profile["version"] != VERSION || proof["version"] != source_migration::VERSION
            || proof["proofSha256"] != digest(&payload.to_string())
            || proof["newSourceSha256"] != digest(&input["source"].to_string())
            || proof["newSourceVersion"] != input["source"]["version"] {
            return Err("视觉缓存源行适配版本或当前来源绑定不符".into());
        }
        let plan = verified_plan(&profile["originalSource"], &input["source"])?;
        if plan.evidence != proof["source"] || serde_json::to_value(&plan.next).unwrap() != serde_json::to_value(current).unwrap() {
            return Err("视觉缓存源行适配不是已验证的逐字唯一映射".into());
        }
        let mut packets = BTreeMap::new();
        for packet in profile["packets"].as_array().ok_or("视觉缓存源行适配缺少原包绑定")? {
            let fingerprint = packet["inputFingerprint"].as_str().filter(|s|!s.is_empty()).ok_or("源行适配原包身份无效")?;
            let sha = packet["cacheSha256"].as_str().filter(|s|!s.is_empty()).ok_or("源行适配原包摘要无效")?;
            if packets.insert(fingerprint.to_owned(), sha.to_owned()).is_some() {
                return Err("视觉缓存源行适配的原包身份重复".into());
            }
        }
        if packets.is_empty() { return Err("视觉缓存源行适配没有绑定原包".into()); }
        let expected = packet_bindings(input, &plan);
        if expected.len() != packets.len() || expected.iter().any(|packet|
            packet["inputFingerprint"].as_str().and_then(|id|packets.get(id)).map(String::as_str) != packet["cacheSha256"].as_str()) {
            return Err("视觉缓存源行适配的完整原包集合缺失或变化，未重发提取".into());
        }
        Ok(Self { current, plan: Some(plan), profile: Some(profile), packets })
    }

    fn bound(&self, cache: &Value) -> bool {
        cache["inputFingerprint"].as_str().is_some_and(|id|self.packets.contains_key(id))
    }

    /// Coverage lookup does not approve facts. Invalid bound packets still
    /// select their existing recovery path, rather than silently reextracting.
    pub(crate) fn coverage_ids(&self, cache: &Value) -> Result<Vec<String>, String> {
        if !self.bound(cache) { return Ok(ids(cache).unwrap_or_default()); }
        let original = ids(cache)?;
        let mapping = self.plan.as_ref().unwrap().evidence["rowMapping"].as_array().unwrap();
        let mut expanded = Vec::new();
        for id in original {
            let row = mapping.iter().find(|row|row["oldRowId"] == id).ok_or("视觉缓存原行未被唯一迁移映射覆盖")?;
            expanded.extend(row["newRowIds"].as_array().unwrap().iter().map(|id|id.as_str().unwrap().to_owned()));
        }
        Ok(expanded)
    }

    pub(crate) fn source(&self, cache: &Value) -> Result<&LockedSource, String> {
        if !self.bound(cache) { return Ok(self.current); }
        let fingerprint = cache["inputFingerprint"].as_str().unwrap();
        if self.packets[fingerprint] != digest(&cache.to_string()) {
            return Err("视觉缓存原包已变化，不能沿用冻结源行适配".into());
        }
        // The reading directory may split. Every fact, decision, identity,
        // raw-response binding and budget must remain interpretation-identical.
        let mut consumed = cache.clone();
        consumed.as_object_mut().ok_or("视觉缓存不是对象")?.remove("rowIds");
        let plan = self.plan.as_ref().unwrap();
        plan.require_unconsumed(&consumed)?;
        if cache.get("unassignedFacts").is_some_and(|v|v.as_array().is_none_or(|v|!v.is_empty())) {
            return Err("视觉缓存仍有未决事实，不能以源行适配重置原修订预算".into());
        }
        Ok(&plan.old)
    }

    pub(crate) fn requested_ids(&self, cache: &Value, requested: &[String]) -> Result<Vec<String>, String> {
        if !self.bound(cache) { return Ok(requested.to_vec()); }
        let expanded = self.coverage_ids(cache)?;
        let mut cursor = 0;
        for id in requested {
            let index = expanded[cursor..].iter().position(|row|row == id).ok_or("视觉缓存迁移范围不覆盖当前取料")?;
            cursor += index + 1;
        }
        // Full original validation, not a relabeled/rehashed current packet.
        ids(cache)
    }

    pub(crate) fn proof(&self, cache: &Value, requested: &[String]) -> Option<Value> {
        self.bound(cache).then(||json!({"kind":VERSION,"migrationProofSha256":self.profile.unwrap()["migrationProof"]["proofSha256"],
            "originalInputFingerprint":cache["inputFingerprint"],"originalCacheSha256":digest(&cache.to_string()),
            "requestedRowIds":requested,"originalPacketUnchanged":true,"factInterpretationsUnchanged":true,
            "coverageExpandedByVerifiedMapping":true,"newModelCalls":0}))
    }
}

/// Called only after Store independently verifies the persisted migration.
pub(crate) fn freeze(input: &mut Value, old: Value, proof: Value) -> Result<(), String> {
    if input.get(FIELD).is_some() { return Err("不覆盖既有视觉缓存源行适配合同".into()); }
    let plan = source_migration::plan(&old, &input["source"])?;
    let packets = packet_bindings(input, &plan);
    if packets.is_empty() { return Ok(()); }
    input[FIELD] = json!({"version":VERSION,"originalSource":old,"migrationProof":proof,"packets":packets});
    let source: LockedSource = serde_json::from_value(input["source"].clone()).map_err(|e|e.to_string())?;
    Context::new(input, &source)?;
    Ok(())
}
