//! Explicit, evidence-bound v10 reparsing. This is not the legacy metadata-only
//! migration: a changed dialogue makes its episode and later continuity unsafe
//! to carry automatically. Historical requests and receipts are never rewritten.
use crate::pipeline::{self, LockedSource, SourceRow};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const VERSION: &str = "explicit-source-reparse-migration/v1";
type Result<T> = std::result::Result<T, String>;

pub(crate) struct Plan {
    pub old: LockedSource,
    pub next: LockedSource,
    pub evidence: Value,
    changed: BTreeSet<String>,
    first_affected: usize,
}

fn value<T: serde::Serialize>(item: T) -> Result<Value> {
    serde_json::to_value(item).map_err(|e| e.to_string())
}

pub(crate) fn plan(previous: &Value, candidate: &Value) -> Result<Plan> {
    let old: LockedSource = serde_json::from_value(previous.clone()).map_err(|e| e.to_string())?;
    let next: LockedSource = serde_json::from_value(candidate.clone()).map_err(|e| e.to_string())?;
    pipeline::validate_source(&old)?;
    pipeline::validate_source(&next)?;
    if next.parser_version != "source-parser/v10" || old.parser_version == next.parser_version {
        return Err("SOURCE_MIGRATION_DIALECT: 仅处理显式升级至 v10 的来源".into());
    }
    if value(pipeline::reparse_source(&old)?)? != value(&next)? {
        return Err("SOURCE_MIGRATION_CANDIDATE: 候选不是原来源的完整程序重解析".into());
    }
    let mut before = value(&old)?;
    let mut after = value(&next)?;
    for key in ["parserVersion", "rows", "episodes", "scenes"] {
        before.as_object_mut().unwrap().remove(key);
        after.as_object_mut().unwrap().remove(key);
    }
    if before != after || next.diagnostics.iter().any(|d| d.blocking) {
        return Err("SOURCE_MIGRATION_BOUNDARY: 原字节、坐标、决定或诊断发生额外变化".into());
    }
    let mut mapping = BTreeMap::<String, Vec<String>>::new();
    let mut changed = BTreeSet::new();
    let mut affected = BTreeSet::new();
    let mut mapped = BTreeSet::new();
    let mut row_evidence = Vec::new();
    for row in &old.rows {
        let parts: Vec<&SourceRow> = next.rows.iter().filter(|r|
            r.range.start >= row.range.start && r.range.end <= row.range.end).collect();
        let exact = !parts.is_empty()
            && parts.first().unwrap().range.start == row.range.start
            && parts.last().unwrap().range.end == row.range.end
            && parts.windows(2).all(|p| p[0].range.end == p[1].range.start)
            && parts.iter().map(|r| r.text.as_str()).collect::<String>() == row.text
            && parts.iter().all(|r| r.number == row.number && r.kind == row.kind
                && r.episode_id == row.episode_id && r.scene_id == row.scene_id);
        if !exact || parts.iter().any(|r| !mapped.insert(r.id.clone())) {
            return Err("SOURCE_MIGRATION_ROW: 原行不能以同坐标同字节连续映射".into());
        }
        let identical = parts.len() == 1 && value(parts[0])? == value(row)?;
        let ids: Vec<_> = parts.iter().map(|r| r.id.clone()).collect();
        mapping.insert(row.id.clone(), ids.clone());
        if !identical {
            changed.insert(row.id.clone());
            changed.extend(ids.clone());
            if let Some(id) = &row.episode_id { affected.insert(id.clone()); }
            else { return Err("SOURCE_MIGRATION_CONTEXT: 公共上下文解释变化，不能自动沿用制作资料".into()); }
        }
        row_evidence.push(json!({"oldRowId":row.id,"newRowIds":ids,"range":row.range,
            "textSha256":pipeline::digest(&row.text),"interpretationUnchanged":identical}));
    }
    if mapped.len() != next.rows.len() || changed.is_empty() {
        return Err("SOURCE_MIGRATION_ROW: 新行覆盖不完整或没有 v10 解释变更".into());
    }
    let project_membership = |items: Value| -> Result<Value> {
        let mut items = items;
        for item in items.as_array_mut().ok_or("来源目录不是数组")? {
            let mut refs = Vec::new();
            for id in item["rowIds"].as_array().ok_or("来源目录缺少原行")? {
                let id = id.as_str().ok_or("来源原行 ID 无效")?;
                refs.extend(mapping.get(id).ok_or("来源目录引用未知旧行")?.iter().map(|id|json!(id)));
            }
            item["rowIds"] = json!(refs);
        }
        Ok(items)
    };
    if project_membership(value(&old.episodes)?)? != value(&next.episodes)?
        || project_membership(value(&old.scenes)?)? != value(&next.scenes)? {
        return Err("SOURCE_MIGRATION_MEMBERSHIP: 集场身份或成员顺序发生变化".into());
    }
    let first_affected = old.episodes.iter().position(|e| affected.contains(&e.id))
        .ok_or("SOURCE_MIGRATION_SCOPE: 解释变更没有明确集归属")?;
    let evidence = json!({"version":VERSION,"fromParser":old.parser_version,"toParser":next.parser_version,
        "sourceSha256":old.sha256,"oldSourceVersion":previous["version"],
        "oldSourceSha256":pipeline::digest(&previous.to_string()),"rowMapping":row_evidence,
        "affectedEpisodeIds":affected,"continuityReviewEpisodeIds":old.episodes[first_affected..].iter().map(|e|&e.id).collect::<Vec<_>>(),
        "rawTextAndCoordinatesUnchanged":true,"newModelCalls":0});
    Ok(Plan { old, next, evidence, changed, first_affected })
}

impl Plan {
    pub fn episode_eligible(&self, id: &str) -> bool {
        self.old.episodes.iter().position(|e| e.id == id).is_some_and(|p|p < self.first_affected)
    }

    /// Inspection only. Recurse to detect consumption, never to rewrite frozen
    /// material. Known row objects must remain byte/interpretation equal.
    pub fn require_unconsumed(&self, input: &Value) -> Result<()> {
        fn visit(plan: &Plan, input: &Value) -> Result<()> {
            match input {
                Value::String(id) => {
                    let row = id.strip_prefix("dialogue:").unwrap_or(id);
                    if plan.changed.contains(row) {
                        return Err("SOURCE_MIGRATION_CONSUMED: 材料消费了解释变化的原行".into());
                    }
                }
                Value::Array(items) => for item in items { visit(plan, item)?; },
                Value::Object(object) => {
                    if let Some(id) = object.get("id").and_then(Value::as_str) {
                        if let Some(old) = plan.old.rows.iter().find(|r|r.id == id) {
                            if object.contains_key("text") && object.contains_key("range") {
                                let next = plan.next.rows.iter().find(|r|r.id == id).ok_or("来源行已经拆分")?;
                                if input != &value(old)? || value(old)? != value(next)? {
                                    return Err("SOURCE_MIGRATION_ROW_INPUT: 已消费原行与冻结来源不等".into());
                                }
                            }
                        }
                    }
                    for (key, item) in object { visit(plan, &json!(key))?; visit(plan, item)?; }
                }
                _ => {}
            }
            Ok(())
        }
        visit(self, input)
    }
}

/// Exact current-vs-published comparison; these are only fields owned by the
/// existing publication/storage boundary, not an arbitrary ignored-field list.
pub(crate) fn adopted_body(unit: &Value) -> Value {
    let mut value = unit.clone();
    if let Some(object) = value.as_object_mut() {
        for key in ["continuityStale", "foundationVersion", "planningProfile", "retired", "sourceVersion", "version"] {
            object.remove(key);
        }
    }
    value
}

#[cfg(test)]
#[path = "source_migration_tests.rs"]
pub(crate) mod tests;
