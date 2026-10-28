//! Image-only design transitions from frozen lite 0.4.1 (f855227).
//! No model, media, filesystem publication, or narrative asset mutation.
//! A caller-owned full registry/locked-scope validator is required for recording.
use crate::series_lite_asset_uses::RegistryValidator;
use crate::series_lite_registry::canonical;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

type Result<T> = std::result::Result<T, String>;
const MAX_SAFE: i64 = 9_007_199_254_740_991;
fn need(ok: bool, code: &str, detail: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("{code}: {detail}"))
    }
}
fn js_space(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
fn text(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim_matches(js_space).is_empty())
}
fn integer(v: &Value) -> Option<i64> {
    let n = v.as_f64()?;
    (n.is_finite() && n.fract() == 0.0 && n.abs() <= MAX_SAFE as f64).then_some(n as i64)
}
fn positive(v: &Value) -> bool {
    integer(v).is_some_and(|n| n > 0)
}
fn digest(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}
fn fields(v: &Value, allowed: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|o| o.keys().all(|k| allowed.contains(&k.as_str())))
}
fn adopted(design: &Value) -> bool {
    matches!(
        design["state"].as_str(),
        Some("TEXT_ADOPTED" | "REFERENCE_ADOPTED")
    )
}
fn array<'a>(v: &'a Value, code: &str) -> Result<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| format!("{code}: expected array"))
}
fn nullable_list(v: &Value) -> Result<&[Value]> {
    if v.is_null() {
        Ok(&[])
    } else {
        Ok(array(v, "ASSET_IMAGE_DESIGN")?.as_slice())
    }
}
fn entries(registry: &Value) -> Result<Vec<&Value>> {
    let mut out = vec![];
    for b in nullable_list(&registry["imageDesigns"])? {
        out.extend(array(&b["entries"], "ASSET_IMAGE_DESIGN")?);
    }
    Ok(out)
}
fn ids(registry: &Value) -> Result<BTreeMap<&str, &Value>> {
    array(&registry["assets"], "ASSET_IMAGE_DESIGN")?
        .iter()
        .map(|a| {
            Ok((
                a["id"]
                    .as_str()
                    .ok_or("ASSET_IMAGE_DESIGN: asset ID missing")?,
                a,
            ))
        })
        .collect()
}
fn deps(design: &Value) -> Result<&[Value]> {
    nullable_list(&design["dependencies"])
}
fn equal(a: &Value, b: &Value) -> Result<bool> {
    Ok(canonical(a)? == canonical(b)?)
}
fn numeric_equal(a: &Value, b: &Value) -> bool {
    a.as_f64().is_some_and(|n| b.as_f64() == Some(n))
}
pub(crate) fn image_hash(value: &Value) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(canonical(value)?.as_bytes())
    ))
}

pub(crate) fn validate_design_level(asset: &Value) -> Result<()> {
    let d = &asset["design"];
    need(
        d.get("detailLevel").is_none()
            || matches!(d["detailLevel"].as_str(), Some("storyboard" | "image")),
        "ASSET_DESIGN_LEVEL",
        "detailLevel须为storyboard或image",
    )?;
    if d.get("detailLevel").is_some() && adopted(d) {
        need(
            text(&d["full"]["body"]),
            "ASSET_DESIGN_FULL",
            "须有必要完整设计full.body，不能只给名称或短卡",
        )?;
    }
    Ok(())
}

pub(crate) fn validate_image_designs(registry: &Value) -> Result<()> {
    need(
        registry.get("imageDesigns").is_none() || registry["imageDesigns"].is_array(),
        "ASSET_IMAGE_DESIGN",
        "imageDesigns须为批次数组",
    )?;
    let assets = ids(registry)?;
    let mut seen = HashSet::new();
    let mut versions = BTreeMap::<&str, i64>::new();
    let mut history = BTreeMap::<(&str, i64), &Value>::new();
    for batch in nullable_list(&registry["imageDesigns"])? {
        need(
            fields(
                batch,
                &["id", "reason", "recordedAtRevision", "inputHash", "entries"],
            ) && text(&batch["id"])
                && !seen.contains(batch["id"].as_str().unwrap_or(""))
                && text(&batch["reason"])
                && positive(&batch["recordedAtRevision"])
                && batch["recordedAtRevision"].as_f64() <= registry["revision"].as_f64()
                && digest(&batch["inputHash"])
                && batch["entries"].as_array().is_some_and(|e| !e.is_empty()),
            "ASSET_IMAGE_DESIGN",
            "制图批次缺少唯一标识、版本或实际条目",
        )?;
        for key in ["requests", "changes", "usageAnnotations"] {
            need(
                !nullable_list(&registry[key])?
                    .iter()
                    .any(|r| r["id"] == batch["id"]),
                "ASSET_IMAGE_DESIGN",
                "制图requestId与其它修改冲突",
            )?;
        }
        seen.insert(batch["id"].as_str().unwrap());
        let mut in_batch = HashSet::new();
        for e in batch["entries"].as_array().unwrap() {
            let id = e["assetId"].as_str().unwrap_or("");
            let asset = assets.get(id);
            need(
                fields(
                    e,
                    &[
                        "assetId",
                        "assetRevision",
                        "imageDesignRevision",
                        "design",
                        "dependencyImages",
                        "compatibilityBasis",
                    ],
                ) && asset.is_some()
                    && !in_batch.contains(id)
                    && positive(&e["assetRevision"])
                    && e["assetRevision"].as_f64()
                        <= asset.map(|a| a["revision"].as_f64()).flatten()
                    && integer(&e["imageDesignRevision"])
                        == Some(versions.get(id).copied().unwrap_or(0) + 1)
                    && text(&e["compatibilityBasis"]),
                "ASSET_IMAGE_DESIGN",
                "制图条目未知、重复、版本断裂或缺少保留分镜约束的依据",
            )?;
            need(
                adopted(&e["design"])
                    && e["design"]["detailLevel"] == "image"
                    && text(&e["design"]["text"])
                    && text(&e["design"]["full"]["body"]),
                "ASSET_IMAGE_DESIGN",
                "制图记录须为已采用的完整制图设计",
            )?;
            let parents = array(&e["dependencyImages"], "ASSET_IMAGE_DESIGN")?;
            let mut parent_ids = HashSet::new();
            for p in parents {
                let parent_id = p["assetId"].as_str().unwrap_or("");
                need(
                    fields(
                        p,
                        &[
                            "assetId",
                            "assetRevision",
                            "imageDesignRevision",
                            "designHash",
                        ],
                    ) && assets.contains_key(parent_id)
                        && parent_id != id
                        && !parent_ids.contains(parent_id)
                        && positive(&p["assetRevision"])
                        && p["assetRevision"].as_f64()
                            <= assets.get(parent_id).and_then(|a| a["revision"].as_f64())
                        && integer(&p["imageDesignRevision"]).is_some_and(|n| n >= 0)
                        && digest(&p["designHash"]),
                    "ASSET_IMAGE_DESIGN",
                    "父制图绑定无效",
                )?;
                parent_ids.insert(parent_id);
            }
            in_batch.insert(id);
            let version = integer(&e["imageDesignRevision"]).unwrap();
            versions.insert(id, version);
            history.insert((id, version), e);
        }
    }
    for e in entries(registry)? {
        for p in array(&e["dependencyImages"], "ASSET_IMAGE_DESIGN")? {
            let revision = integer(&p["imageDesignRevision"]).unwrap();
            if revision > 0 {
                let parent = history.get(&(p["assetId"].as_str().unwrap(), revision));
                need(
                    parent.is_some_and(|v| numeric_equal(&v["assetRevision"], &p["assetRevision"]))
                        && parent
                            .map(|v| image_hash(&v["design"]))
                            .transpose()?
                            .as_deref()
                            == p["designHash"].as_str(),
                    "ASSET_IMAGE_DESIGN",
                    "父制图记录不存在或hash不匹配",
                )?;
            }
        }
    }
    Ok(())
}

/// Resolve current art and only its real direct design parents. A result is a
/// design binding, never evidence of actual generated/approved pixels.
pub(crate) fn resolve_image_design(registry: &Value, asset_id: &str) -> Result<Value> {
    resolve(
        registry,
        asset_id,
        &mut BTreeMap::new(),
        &mut HashSet::new(),
    )
}
fn resolve(
    registry: &Value,
    asset_id: &str,
    cache: &mut BTreeMap<String, Value>,
    active: &mut HashSet<String>,
) -> Result<Value> {
    if let Some(old) = cache.get(asset_id) {
        return Ok(old.clone());
    }
    need(
        active.insert(asset_id.into()),
        "ASSET_IMAGE_CYCLE",
        "制图依赖成环",
    )?;
    let assets = ids(registry)?;
    let asset = assets
        .get(asset_id)
        .copied()
        .filter(|a| adopted(&a["design"]))
        .ok_or_else(|| format!("ASSET_DESIGN_UNADOPTED: {asset_id} 分镜必要设计未采用"))?;
    let records = entries(registry)?;
    let record = records.into_iter().rev().find(|e| e["assetId"] == asset_id);
    if let Some(record) = record {
        need(
            numeric_equal(&record["assetRevision"], &asset["revision"]),
            "ASSET_IMAGE_STALE",
            "分镜设计已变；只复核该项制图资料",
        )?;
    }
    let design = record.map(|r| &r["design"]).unwrap_or(&asset["design"]);
    need(
        record.is_some() || design["detailLevel"] != "storyboard",
        "ASSET_IMAGE_NOT_READY",
        "仅有分镜必要设计，尚无可生图完整设计；分镜仍可推进",
    )?;
    let declared = deps(&asset["design"])?;
    if let Some(record) = record {
        let mut expected: Vec<_> = declared.iter().map(|d| d["assetId"].clone()).collect();
        let mut actual: Vec<_> = array(&record["dependencyImages"], "ASSET_IMAGE_DEPENDENCY")?
            .iter()
            .map(|p| p["assetId"].clone())
            .collect();
        expected.sort_by(|a, b| {
            a.as_str()
                .unwrap_or("")
                .encode_utf16()
                .cmp(b.as_str().unwrap_or("").encode_utf16())
        });
        actual.sort_by(|a, b| {
            a.as_str()
                .unwrap_or("")
                .encode_utf16()
                .cmp(b.as_str().unwrap_or("").encode_utf16())
        });
        need(
            equal(&json!(deps(design)?), &json!(declared))?
                && equal(&json!(actual), &json!(expected))?,
            "ASSET_IMAGE_DEPENDENCY",
            "制图记录须绑定实际直接父设计",
        )?;
    }
    let mut parents = vec![];
    for d in declared {
        let id = d["assetId"]
            .as_str()
            .ok_or("ASSET_IMAGE_DEPENDENCY: missing parent ID")?;
        let parent = resolve(registry, id, cache, active)?;
        need(
            numeric_equal(&parent["assetRevision"], &d["assetRevision"]),
            "ASSET_DESIGN_DEPENDENCY_STALE",
            "父分镜设计已变",
        )?;
        let fallback = json!({"assetRevision":d["assetRevision"],"imageDesignRevision":0,"designHash":image_hash(&assets[id]["design"])?});
        let expected = record
            .and_then(|r| r["dependencyImages"].as_array())
            .and_then(|p| p.iter().find(|p| p["assetId"] == d["assetId"]))
            .unwrap_or(&fallback);
        need(
            numeric_equal(&expected["assetRevision"], &parent["assetRevision"])
                && numeric_equal(
                    &expected["imageDesignRevision"],
                    &parent["imageDesignRevision"],
                )
                && expected["designHash"] == parent["designHash"],
            "ASSET_IMAGE_DEPENDENCY_STALE",
            "父制图已变；仅复核真实派生制图，不回退分镜",
        )?;
        parents.push(json!({"assetId":id,"assetRevision":parent["assetRevision"],"imageDesignRevision":parent["imageDesignRevision"],"designHash":parent["designHash"]}));
    }
    active.remove(asset_id);
    let result = json!({"assetId":asset_id,"assetRevision":asset["revision"],"imageDesignRevision":record.map(|r|r["imageDesignRevision"].clone()).unwrap_or(json!(0)),"design":design,"designHash":image_hash(design)?,"dependencyImages":parents});
    cache.insert(asset_id.into(), result.clone());
    Ok(result)
}

pub(crate) fn prepare_image_design_batch(registry: &Value, input: &Value) -> Result<Value> {
    need(
        fields(
            input,
            &["version", "baseRevision", "requestId", "reason", "entries"],
        ) && input["version"] == "series-image-design-v1"
            && positive(&input["baseRevision"])
            && text(&input["requestId"])
            && text(&input["reason"])
            && input["entries"].as_array().is_some_and(|e| !e.is_empty()),
        "ASSET_IMAGE_DESIGN",
        "制图补充须有批次版本、原因与条目",
    )?;
    let assets = ids(registry)?;
    let mut pending = BTreeMap::new();
    let mut input_order = vec![];
    for e in input["entries"].as_array().unwrap() {
        let id = e["assetId"].as_str().unwrap_or("");
        let asset = assets.get(id);
        need(
            fields(
                e,
                &["assetId", "assetRevision", "design", "compatibilityBasis"],
            ) && asset.is_some()
                && !pending.contains_key(id)
                && asset.is_some_and(|a| {
                    numeric_equal(&e["assetRevision"], &a["revision"]) && adopted(&a["design"])
                })
                && text(&e["compatibilityBasis"]),
            "ASSET_IMAGE_DESIGN",
            "制图补充须绑定当前采用资产，不接受资产身份、版本或范围修改",
        )?;
        need(
            adopted(&e["design"])
                && text(&e["design"]["text"])
                && text(&e["design"]["full"]["body"])
                && (e["design"].get("detailLevel").is_none()
                    || e["design"]["detailLevel"] == "image"),
            "ASSET_IMAGE_DESIGN",
            "须提供完整已采用制图文字与full.body",
        )?;
        let declared = json!(deps(&asset.unwrap()["design"])?);
        need(
            e["design"].get("dependencies").is_none()
                || equal(&e["design"]["dependencies"], &declared)?,
            "ASSET_IMAGE_DEPENDENCY",
            "制图不能另改分镜父设计链",
        )?;
        pending.insert(id.to_string(), e.clone());
        input_order.push(id.to_string());
    }
    let revision =
        integer(&registry["revision"]).ok_or("ASSET_IMAGE_DESIGN: invalid registry revision")?;
    need(
        revision > 0 && revision < MAX_SAFE,
        "ASSET_IMAGE_DESIGN",
        "registry revision exceeds safe integer",
    )?;
    let mut batch = json!({"id":input["requestId"],"reason":input["reason"],"recordedAtRevision":revision+1,"inputHash":image_hash(&json!({"reason":input["reason"],"entries":input["entries"]}))?,"entries":[]});
    let mut trial = registry.clone();
    let mut batches = nullable_list(&registry["imageDesigns"])?.to_vec();
    batches.push(batch.clone());
    trial["imageDesigns"] = json!(batches);
    let prior = entries(registry)?;
    let mut done = HashSet::new();
    let mut active = HashSet::new();
    fn add(
        id: &str,
        registry: &Value,
        trial: &mut Value,
        batch: &mut Value,
        pending: &BTreeMap<String, Value>,
        prior: &[&Value],
        done: &mut HashSet<String>,
        active: &mut HashSet<String>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        need(
            active.insert(id.into()),
            "ASSET_IMAGE_CYCLE",
            "制图依赖成环",
        )?;
        let assets = ids(registry)?;
        let asset = assets.get(id).ok_or("ASSET_IMAGE_DESIGN: unknown asset")?;
        let mut parents = vec![];
        for d in deps(&asset["design"])? {
            let parent_id = d["assetId"]
                .as_str()
                .ok_or("ASSET_IMAGE_DEPENDENCY: missing parent")?;
            if pending.contains_key(parent_id) {
                add(
                    parent_id, registry, trial, batch, pending, prior, done, active,
                )?;
            }
            let parent = resolve_image_design(trial, parent_id)?;
            need(
                numeric_equal(&parent["assetRevision"], &d["assetRevision"]),
                "ASSET_DESIGN_DEPENDENCY_STALE",
                "父分镜设计已变，先修真实约束",
            )?;
            parents.push(json!({"assetId":parent_id,"assetRevision":parent["assetRevision"],"imageDesignRevision":parent["imageDesignRevision"],"designHash":parent["designHash"]}));
        }
        let e = &pending[id];
        let mut design = e["design"].clone();
        design["detailLevel"] = json!("image");
        design["dependencies"] = json!(deps(&asset["design"])?);
        let old = prior
            .iter()
            .rev()
            .find(|e| e["assetId"] == id)
            .map(|e| {
                integer(&e["imageDesignRevision"])
                    .ok_or("ASSET_IMAGE_DESIGN: invalid prior revision")
            })
            .transpose()?
            .unwrap_or(0);
        need(
            old < MAX_SAFE,
            "ASSET_IMAGE_DESIGN",
            "image design revision exceeds safe integer",
        )?;
        batch["entries"].as_array_mut().unwrap().push(json!({"assetId":id,"assetRevision":asset["revision"],"imageDesignRevision":old+1,"design":design,"dependencyImages":parents,"compatibilityBasis":e["compatibilityBasis"]}));
        *trial["imageDesigns"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap() = batch.clone();
        active.remove(id);
        done.insert(id.into());
        Ok(())
    }
    for id in input_order {
        add(
            &id,
            registry,
            &mut trial,
            &mut batch,
            &pending,
            &prior,
            &mut done,
            &mut active,
        )?;
    }
    Ok(batch)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageDesignResult {
    pub registry: Value,
    pub changed: bool,
    pub asset_revisions_changed: bool,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<Value>>,
}
/// The caller holds the existing registry transaction/CAS and publishes the
/// returned complete snapshot. Validation grants no content or pixel approval.
pub(crate) fn record_image_design(
    registry: &Value,
    input: &Value,
    validator: &mut impl RegistryValidator,
) -> Result<ImageDesignResult> {
    validator.validate(registry, true)?;
    validate_image_designs(registry)?;
    let batch = prepare_image_design_batch(registry, input)?;
    let previous = nullable_list(&registry["imageDesigns"])?
        .iter()
        .find(|r| r["id"] == input["requestId"]);
    if let Some(previous) = previous {
        need(
            previous["inputHash"] == batch["inputHash"],
            "ASSET_IMAGE_DESIGN",
            "制图requestId已用于不同内容",
        )?;
        for entry in array(&previous["entries"], "ASSET_IMAGE_DESIGN")? {
            let id = entry["assetId"]
                .as_str()
                .ok_or("ASSET_IMAGE_DESIGN: assetId missing")?;
            let current = resolve_image_design(registry, id)?;
            need(
                numeric_equal(
                    &current["imageDesignRevision"],
                    &entry["imageDesignRevision"],
                ),
                "ASSET_IMAGE_STALE",
                "该请求已有后续制图版本，不能当作当前采用重试",
            )?;
        }
        return Ok(ImageDesignResult {
            registry: registry.clone(),
            changed: false,
            asset_revisions_changed: false,
            request_id: input["requestId"].as_str().unwrap().into(),
            entries: None,
        });
    }
    need(
        numeric_equal(&input["baseRevision"], &registry["revision"]),
        "ASSET_BASE_REVISION",
        "制图补充基于旧总库；读取现版重试，不重做分镜",
    )?;
    for key in ["requests", "changes", "usageAnnotations"] {
        need(
            !nullable_list(&registry[key])?
                .iter()
                .any(|r| r["id"] == input["requestId"]),
            "ASSET_IMAGE_DESIGN",
            "requestId已被其它修改使用",
        )?;
    }
    let mut result = registry.clone();
    let mut batches = nullable_list(&registry["imageDesigns"])?.to_vec();
    batches.push(batch.clone());
    result["imageDesigns"] = json!(batches);
    result["revision"] = batch["recordedAtRevision"].clone();
    validate_image_designs(&result)?;
    validator.validate(&result, true)?;
    Ok(ImageDesignResult { registry: result, changed: true, asset_revisions_changed: false, request_id: input["requestId"].as_str().unwrap().into(), entries: Some(batch["entries"].as_array().unwrap().iter().map(|e| json!({"assetId":e["assetId"],"imageDesignRevision":e["imageDesignRevision"]})).collect()) })
}

#[cfg(test)]
#[path = "series_lite_image_design_tests.rs"]
mod tests;
