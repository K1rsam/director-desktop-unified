//! Rust counterpart of series v6.7 job_packet.py's effective role view.
//! Original files stay byte-for-byte frozen; adaptations fail closed on drift.
use super::super::{digest, RuleSnapshot};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub const VERSION: &str = "20261025-v6.7.2-readable-image-bindings";
pub(super) const CONTENT_ONCE: &str = "series-rule-content-once/v1";
const CONTENT_READ: &str = "完整规则正文均在 bodies 中，按 roleSets.repair 的 primary 再 methods 原顺序读取；roleSets.origin 保留原岗位的 primary/methods 顺序与职责关联。每个描述符的 contentRef 与 sha256 指向同一正文，正文只存一份但两组规则全部有效；不能以引用、哈希或短卡代替读取正文。";
const COMMON: &str = "references/creative/references/common-contract.md";
const SOURCE: &str = "references/creative/skills/source-context-preparer/SKILL.md";
const AUTHOR: &str = "references/creative/skills/unit-performance-director/SKILL.md";
const FINISHER: &str = "references/creative/skills/episode-finishing-editor/SKILL.md";
const REVIEWER: &str = "stages/review-delivery/SKILL.md";
macro_rules! rule {
    ($path:literal) => {
        (
            $path,
            include_str!(concat!(
                "../../../../../rules/series/20261025-v6.7.2/",
                $path
            )),
        )
    };
}
fn files() -> Vec<(&'static str, &'static str)> {
    vec![
        rule!("references/creative/references/common-contract.md"),
        rule!("references/production-contract.md"),
        rule!("references/data-contract.md"),
        rule!("references/lingmo-v7-contract.md"),
        rule!("references/expression-and-screen-text.md"),
        rule!("references/creative/skills/unit-performance-director/SKILL.md"),
        rule!("references/creative/skills/episode-finishing-editor/SKILL.md"),
        rule!("stages/review-delivery/SKILL.md"),
        rule!("references/creative/skills/source-context-preparer/SKILL.md"),
        rule!("references/creative/skills/episode-emotion-planner/SKILL.md"),
        rule!("references/creative/skills/acting-blocking-methods/SKILL.md"),
        rule!("references/creative/skills/camera-emotion-methods/SKILL.md"),
        rule!("references/creative/skills/sound-rhythm-methods/SKILL.md"),
        rule!("references/creative/skills/lighting-space-methods/SKILL.md"),
        rule!("references/creative/skills/fantasy-expression-methods/SKILL.md"),
        rule!("references/creative/skills/director-refinement/SKILL.md"),
        rule!("references/shot-handoffs.md"),
        rule!("references/system-presentation.md"),
        rule!("references/asset-registry-contract.md"),
        rule!("stages/asset-design/SKILL.md"),
        rule!("references/preproduction-contract.md"),
        rule!("references/asset-handoff.md"),
        rule!("references/lingmo-field-budget.md"),
    ]
}
pub fn snapshots() -> Result<Vec<RuleSnapshot>, String> {
    let installed = include_str!("../../../../../rules/series/20261025-v6.7.2/VERSION");
    if installed.trim() != VERSION {
        return Err("SERIES_RULE_VERSION_MISMATCH".into());
    }
    Ok(files()
        .into_iter()
        .map(|(path, body)| RuleSnapshot {
            id: format!("series/{path}"),
            version: VERSION.into(),
            sha256: digest(body),
            content: body.into(),
        })
        .collect())
}
pub fn pack_hash() -> Result<String, String> {
    let manifest: Vec<_> = snapshots()?
        .into_iter()
        .map(|r| json!({"id":r.id,"sha256":r.sha256,"version":r.version}))
        .collect();
    Ok(digest(&json!(manifest).to_string()))
}
fn replace_once(
    body: &mut String,
    before: &str,
    after: &str,
    id: &str,
    changes: &mut Vec<String>,
) -> Result<(), String> {
    if body.matches(before).count() != 1 {
        return Err(format!("SERIES_RULE_ADAPTER_DRIFT:{id}"));
    }
    *body = body.replacen(before, after, 1);
    changes.push(id.into());
    Ok(())
}
fn sections(body: &str, wanted: &[&str]) -> Result<String, String> {
    let mut headings = std::collections::BTreeMap::<String, String>::new();
    let mut current: Option<String> = None;
    for line in body.split_inclusive('\n') {
        if let Some(heading) = line.strip_prefix("## ") {
            let name = heading.trim().to_owned();
            if headings.insert(name.clone(), String::new()).is_some() {
                return Err("SERIES_DUPLICATE_RULE_SECTION".into());
            }
            current = Some(name);
        }
        if let Some(name) = &current {
            headings.get_mut(name).unwrap().push_str(line);
        }
    }
    wanted
        .iter()
        .map(|name| {
            headings
                .get(*name)
                .cloned()
                .ok_or_else(|| format!("SERIES_RULE_SECTION_MISSING:{name}"))
        })
        .collect()
}
fn effective(path: &str, raw: &str) -> Result<(String, Vec<String>), String> {
    let mut body = raw.to_owned();
    let mut changes = Vec::new();
    if path == SOURCE {
        replace_once(&mut body,
            "上游标准结构、全剧方向、连续性、目录与已采用美术是可选辅助，有则核对来源版本、对象和适用范围后择需使用；没有则依据本集原文和实际提供的前情自行准备必要事实与最小一致文字设计，不等待资料齐套。",
            "本岗位包已提供本集受核源、有效方向、时点事实、完整必要采用设计与实际入口，核对来源版本、对象和适用范围后使用；发现真正缺失的必要依赖时标明具体缺口交主控补齐，不自行改造已采用设计。无关辅助资料无需等待。",
            "required-input-context", &mut changes)?;
    }
    if path == COMMON {
        replace_once(&mut body,
            "每个单元的镜长合计落在整数秒，单镜仍可小数；用成立的切点和相位排定，不四舍五入显示掩盖实际小数，也不催读、拖动作或补空镜凑整。",
            "v7 内部镜长与单元实际总长保留精确小数；外层容器取 max(4,ceil(实际总长))，内部实际总长和外层容器均不得超过项目15/30秒上限。不改成立的切点与相位，不催读、拖动作或补空镜凑整。",
            "common-unit-end", &mut changes)?;
    }
    if path == FINISHER {
        replace_once(&mut body,
            "整数终点只在相关单元内选成立的切点/相位，单镜可小数；不改已好的邻段凑整，不以新的暂停或付费返修处理小数格式。若整数终点仍与真实声画冲突，明确保留待核坐标，不能假称整数交付。",
            "v7 内部保留精确小数终点，外层容器取 max(4,ceil(实际总长))；内部与容器均守项目15/30秒上限。不改已好的邻段凑整，不以新的暂停或付费返修处理合法小数。",
            "finisher-unit-end", &mut changes)?;
        replace_once(
            &mut body,
            "最终按成立的相位排整数终点，已收束则不填到26秒",
            "最终保留成立相位的精确小数终点，由外层容器取整，已收束则不填到26秒",
            "finisher-example-end",
            &mut changes,
        )?;
    }
    if path == "references/creative/skills/director-refinement/SKILL.md" {
        replace_once(&mut body, "单元整数终点由真实动作/声画调整，内镜允许小数秒，不靠等待/吞字凑整。", "v7 保留内部镜长和单元实际总长的精确小数，外层容器取 max(4,ceil(实际总长))；二者均守项目15/30秒上限，不靠等待/吞字凑整。", "refinement-unit-end", &mut changes)?;
    }
    let wanted: &[&str] = match path {
        "references/production-contract.md" => &[
            "新任务的完整计划",
            "幻想候选、模式与选择",
            "整编、精修与采用",
        ],
        "references/data-contract.md" => {
            &["Bundle 根与原文", "后期文字交接", "场、单元、镜头与人声"]
        }
        "references/lingmo-v7-contract.md" => &[
            "1. 创作范围与兼容边界",
            "2. 内部身份、图片引用与安全名",
            "3. 正文结构与字段保留",
            "4. 精确时间与外层容器",
        ],
        REVIEWER => &["内容双向复核"],
        _ => &[],
    };
    if !wanted.is_empty() {
        body = sections(&body, wanted)?;
        changes.extend(wanted.iter().map(|s| format!("section:{s}")));
    }
    Ok((body, changes))
}
pub fn role(role: &str, fantasy: bool) -> Result<Value, String> {
    let mut paths = vec![
        COMMON,
        "references/production-contract.md",
        "references/data-contract.md",
        "references/lingmo-v7-contract.md",
        "references/expression-and-screen-text.md",
    ];
    let primary = match role {
        "author" => AUTHOR,
        "finisher" | "repair" => FINISHER,
        "reviewer" => REVIEWER,
        "design" | "design-review" => "stages/asset-design/SKILL.md",
        _ => return Err("SERIES_ROLE_UNKNOWN".into()),
    };
    if matches!(role, "design" | "design-review") {
        paths = vec![
            primary,
            "references/asset-registry-contract.md",
            "references/preproduction-contract.md",
            "references/system-presentation.md",
            "references/asset-handoff.md",
        ];
    } else {
        paths.push(primary);
        if role == "author" {
            paths.extend([
                SOURCE,
                "references/creative/skills/episode-emotion-planner/SKILL.md",
            ]);
        }
        if matches!(role, "finisher" | "repair" | "reviewer") {
            paths.push("references/shot-handoffs.md");
        }
        paths.extend([
            "references/creative/skills/acting-blocking-methods/SKILL.md",
            "references/creative/skills/camera-emotion-methods/SKILL.md",
            "references/creative/skills/sound-rhythm-methods/SKILL.md",
            "references/creative/skills/lighting-space-methods/SKILL.md",
            "references/system-presentation.md",
            "references/asset-registry-contract.md",
            "references/lingmo-field-budget.md",
        ]);
        if fantasy {
            paths.push("references/creative/skills/fantasy-expression-methods/SKILL.md");
        }
        if role == "repair" {
            paths.push("references/creative/skills/director-refinement/SKILL.md");
        }
    }
    let all = files();
    let mut seen = HashSet::new();
    let mut rules = Vec::new();
    for path in paths {
        if !seen.insert(path) {
            continue;
        }
        let raw = all
            .iter()
            .find(|(p, _)| *p == path)
            .ok_or("SERIES_RULE_MISSING")?
            .1;
        let (content, adjustments) = effective(path, raw)?;
        rules.push(json!({"id":format!("series/{path}"),"path":path,"version":VERSION,"sourceSha256":digest(raw),"sha256":digest(&content),"adjustments":adjustments,"content":content}));
    }
    let p = rules
        .iter()
        .position(|r| r["path"] == primary)
        .ok_or("SERIES_PRIMARY_MISSING")?;
    let primary_rule = rules.remove(p);
    Ok(
        json!({"primary":primary_rule,"methods":rules,"creativePack":{"version":"desktop-series-role-rules/v1","skillVersion":VERSION,"packSha256":pack_hash()?,"role":role,"adapterVersion":"series672-rust-role-rules/v1"}}),
    )
}

/// Only a newly frozen workflow opts into this representation. Every complete
/// rule body remains present; role descriptions and method order stay separate.
pub(super) fn materialize_repair(original: &Value) -> Result<Value, String> {
    fn one(rule: &Value, bodies: &mut Map<String, Value>) -> Result<Value, String> {
        let mut descriptor = rule
            .as_object()
            .cloned()
            .ok_or("SERIES_RULE_DESCRIPTOR_REQUIRED")?;
        let body = descriptor
            .remove("content")
            .ok_or("SERIES_RULE_BODY_REQUIRED")?;
        let sha = descriptor
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or("SERIES_RULE_DIGEST_REQUIRED")?
            .to_owned();
        if descriptor.contains_key("contentRef")
            || body.as_str().is_none_or(|text| digest(text) != sha)
        {
            return Err("SERIES_RULE_BODY_DIGEST_MISMATCH".into());
        }
        if bodies.get(&sha).is_some_and(|old| old != &body) {
            return Err("SERIES_RULE_BODY_COLLISION".into());
        }
        bodies.insert(sha.clone(), body);
        descriptor.insert("contentRef".into(), json!(sha));
        Ok(Value::Object(descriptor))
    }
    fn role_set(value: &Value, bodies: &mut Map<String, Value>) -> Result<Value, String> {
        let mut value = value.clone();
        value["primary"] = one(&value["primary"], bodies)?;
        let methods = value["methods"]
            .as_array()
            .ok_or("SERIES_RULE_METHODS_REQUIRED")?
            .iter()
            .map(|r| one(r, bodies))
            .collect::<Result<Vec<_>, _>>()?;
        value["methods"] = json!(methods);
        Ok(value)
    }
    let mut base = original.clone();
    let origin = base
        .as_object_mut()
        .ok_or("SERIES_RULE_SET_REQUIRED")?
        .remove("originRules");
    let mut bodies = Map::new();
    let mut role_sets = Map::new();
    role_sets.insert("repair".into(), role_set(&base, &mut bodies)?);
    if let Some(origin) = origin {
        role_sets.insert("origin".into(), role_set(&origin, &mut bodies)?);
    }
    let materialized = json!({"version":CONTENT_ONCE,"readContract":CONTENT_READ,"bodies":bodies,"roleSets":role_sets});
    if restore_repair(&materialized)? != *original {
        return Err("SERIES_RULE_MATERIALIZATION_ROUNDTRIP".into());
    }
    Ok(materialized)
}
pub(super) fn restore_repair(value: &Value) -> Result<Value, String> {
    if value["version"] != CONTENT_ONCE {
        if value.get("bodies").is_some() || value.get("roleSets").is_some() {
            return Err("SERIES_RULE_MATERIALIZATION_VERSION".into());
        }
        return Ok(value.clone());
    }
    if value.as_object().is_none_or(|v| v.len() != 4) || value["readContract"] != CONTENT_READ {
        return Err("SERIES_RULE_MATERIALIZATION_SHAPE".into());
    }
    let bodies = value["bodies"]
        .as_object()
        .ok_or("SERIES_RULE_BODIES_REQUIRED")?;
    let sets = value["roleSets"]
        .as_object()
        .ok_or("SERIES_RULE_SETS_REQUIRED")?;
    if !sets.contains_key("repair")
        || sets
            .keys()
            .any(|k| !matches!(k.as_str(), "repair" | "origin"))
    {
        return Err("SERIES_RULE_ROLE_SET_CHANGED".into());
    }
    fn one(
        rule: &Value,
        bodies: &Map<String, Value>,
        used: &mut HashSet<String>,
    ) -> Result<Value, String> {
        let mut descriptor = rule
            .as_object()
            .cloned()
            .ok_or("SERIES_RULE_DESCRIPTOR_REQUIRED")?;
        let reference = descriptor
            .remove("contentRef")
            .ok_or("SERIES_RULE_REFERENCE_REQUIRED")?;
        let sha = reference.as_str().ok_or("SERIES_RULE_REFERENCE_INVALID")?;
        let body = bodies.get(sha).ok_or("SERIES_RULE_BODY_MISSING")?;
        if descriptor.contains_key("content")
            || descriptor.get("sha256") != Some(&reference)
            || body.as_str().is_none_or(|text| digest(text) != sha)
        {
            return Err("SERIES_RULE_BODY_DIGEST_MISMATCH".into());
        }
        used.insert(sha.to_owned());
        descriptor.insert("content".into(), body.clone());
        Ok(Value::Object(descriptor))
    }
    fn role_set(
        value: &Value,
        bodies: &Map<String, Value>,
        used: &mut HashSet<String>,
    ) -> Result<Value, String> {
        let mut value = value.clone();
        value["primary"] = one(&value["primary"], bodies, used)?;
        value["methods"] = json!(value["methods"]
            .as_array()
            .ok_or("SERIES_RULE_METHODS_REQUIRED")?
            .iter()
            .map(|r| one(r, bodies, used))
            .collect::<Result<Vec<_>, _>>()?);
        Ok(value)
    }
    let mut used = HashSet::new();
    let mut restored = role_set(&sets["repair"], bodies, &mut used)?;
    if let Some(origin) = sets.get("origin") {
        restored["originRules"] = role_set(origin, bodies, &mut used)?;
    }
    if used.len() != bodies.len() {
        return Err("SERIES_RULE_UNREFERENCED_BODY".into());
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effective_roles_preserve_full_rules_and_replace_only_known_v7_clauses() {
        for name in [
            "author",
            "finisher",
            "reviewer",
            "design",
            "design-review",
            "repair",
        ] {
            let loaded = role(name, true).unwrap();
            assert_eq!(loaded["creativePack"]["skillVersion"], VERSION);
            let all = loaded.to_string();
            assert!(!all.contains("mac-3008-compatible-rust"));
            assert!(!all.contains("每个单元的镜长合计落在整数秒"));
        }
        let a = role("author", true).unwrap();
        assert!(a.to_string().contains("完整必要采用设计"));
        assert!(a.to_string().contains("episode-emotion-planner"));
        let reviewer = role("reviewer", false).unwrap();
        assert!(reviewer["primary"]["content"]
            .as_str()
            .unwrap()
            .starts_with("## 内容双向复核"));
        assert!(snapshots().unwrap().len() > 20);
    }
    #[test]
    fn source_adapter_drift_fails_closed() {
        assert!(effective(SOURCE, "changed upstream clause").is_err());
    }
    #[test]
    fn pooled_rules_preserve_every_descriptor_role_order_creative_pack_and_full_body() {
        for origin in ["author", "finisher", "reviewer", "design", "design-review"] {
            for fantasy in [false, true] {
                let original = super::super::repair_rules(origin, fantasy).unwrap();
                let pooled = materialize_repair(&original).unwrap();
                assert_eq!(restore_repair(&pooled).unwrap(), original);
                let mut expected = Map::new();
                let mut occurrences = 0;
                for set in [&original, &original["originRules"]] {
                    for rule in
                        std::iter::once(&set["primary"]).chain(set["methods"].as_array().unwrap())
                    {
                        occurrences += 1;
                        expected.insert(
                            rule["sha256"].as_str().unwrap().to_owned(),
                            rule["content"].clone(),
                        );
                    }
                }
                assert_eq!(pooled["bodies"], Value::Object(expected));
                assert!(pooled["bodies"].as_object().unwrap().len() < occurrences);
                assert_eq!(
                    pooled["roleSets"]["repair"]["creativePack"],
                    original["creativePack"]
                );
                assert_eq!(
                    pooled["roleSets"]["origin"]["creativePack"],
                    original["originRules"]["creativePack"]
                );
                if origin == "finisher" {
                    assert!(pooled.to_string().len() < original.to_string().len());
                }
            }
        }
    }
    #[test]
    fn pooled_rules_reject_missing_changed_ambiguous_or_unused_bodies() {
        let original = super::super::repair_rules("finisher", false).unwrap();
        let pooled = materialize_repair(&original).unwrap();
        let primary = pooled["roleSets"]["repair"]["primary"]["contentRef"]
            .as_str()
            .unwrap()
            .to_owned();
        for kind in [
            "missing",
            "changed",
            "reference",
            "inline",
            "unused",
            "role",
            "version",
        ] {
            let mut bad = pooled.clone();
            match kind {
                "missing" => {
                    bad["bodies"].as_object_mut().unwrap().remove(&primary);
                }
                "changed" => bad["bodies"][&primary] = json!("不能删减规则正文"),
                "reference" => {
                    bad["roleSets"]["repair"]["primary"]["contentRef"] = json!("0".repeat(64))
                }
                "inline" => {
                    bad["roleSets"]["repair"]["primary"]["content"] = json!("第二份不受绑定的正文")
                }
                "unused" => {
                    let text = "unused complete rule";
                    bad["bodies"][digest(text)] = json!(text);
                }
                "role" => bad["roleSets"]["unknown"] = original.clone(),
                "version" => bad["version"] = json!("series-rule-content-once/future"),
                _ => unreachable!(),
            }
            assert!(restore_repair(&bad).is_err(), "{kind}");
        }
        assert_eq!(restore_repair(&original).unwrap(), original);
    }
}
