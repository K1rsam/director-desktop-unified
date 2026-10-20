//! New-workflow-only asset use transport. Identity evidence stays program-owned;
//! the model selects local shot IDs, never invents an identity's source row.
use super::{digest, TextRequest};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const FIELD: &str = "assetUseBindingProfile";
pub const PROFILE: &str = "asset-use-binding/v1";
pub(crate) const ORDER_REJECTION: &str = "素材使用顺序须从1连续且不得重复";
const USE_PURPOSE: &str = "UNIT_USE_ANCHOR";
const RULE: &str = "【素材身份与使用位置】本次 asset-use-binding/v1：资产身份、形象及其来源由冻结目录提供，assets.sourceRefs/sourceRefsPurpose 仅为登记身份/适用范围依据，不是当前镜头的新增动作事实。每个实际 assetRefs 只选正式 assetId、variantId、use、order 和 useShotIds；useShotIds 是该资产在本单元实际入画或声音使用的镜号，必须非空、无重复、全部属于本单元 shots，不写另一个单元镜号。不要输出资产 sourceRefs、identityEvidence、sourceRefsPurpose 或程序绑定标记；Rust 从已核对的所选镜头制作 sourceRefs 生成本地使用锚，并原样保留登记身份依据。输入旧稿里的这些派生字段仅供审阅，替换单元按本次 schema 重交实际镜号。连续人物/道具不需原文在每个容量单元重复点名，但仍依据原文、入口事实与实际连续态判断当前是否在场及持物；本场目录、身份出处、父图或 useShotIds 均不授权提前入场、未来受伤/换装、源外动作、画外人物实体化。CONTEXT_EVIDENCE_ONLY 与 MATERIAL_SOURCE_ONLY 不可选作当前实体。unit/shot.sourceRefs 的制作覆盖、contextRefs 的前情边界、原话及动作时序全部保持原规则，整编仍检查实际画面。";

pub fn enabled(input: &Value) -> Result<bool, String> {
    match input.get(FIELD) {
        None => Ok(false),
        Some(Value::String(value)) if value == PROFILE => Ok(true),
        _ => Err("素材使用绑定版本未知；保留原请求，未派发模型".into()),
    }
}
pub fn freeze_new(input: &mut Value) -> Result<(), String> {
    enabled(input)?;
    input[FIELD] = json!(PROFILE);
    Ok(())
}
pub fn inherit(target: &mut Value, frozen: &Value) -> Result<(), String> {
    if enabled(frozen)? {
        target[FIELD] = json!(PROFILE);
    } else if target.get(FIELD).is_some() {
        return Err("不能向旧冻结任务补入素材使用绑定版本".into());
    }
    Ok(())
}

/// Runs after the existing creative adapter; both CLI and SDK compile this
/// final schema. Never changes an unmarked request, even its serialized bytes.
pub(super) fn prepare_request(request: &mut TextRequest) -> Result<(), String> {
    if !enabled(&request.input)? { return Ok(()); }
    if !super::stage3_pack::uses_profile(&request.input)?
        || !matches!(request.stage.as_str(), "storyboardPlan" | "storyboardCreate" | "storyboardFinish" | "storyboardRefine") {
        return Err("素材使用绑定未绑定第三阶段冻结规则或岗位".into());
    }
    fn visit(schema: &mut Value) -> Result<(), String> {
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            if ["assetId", "variantId", "use", "order"].iter().all(|key| properties.contains_key(*key)) {
                match (properties.contains_key("sourceRefs"), properties.contains_key("useShotIds")) {
                    (true, false) => {
                        properties.remove("sourceRefs");
                        // Provider structured output supports minItems; uniqueness
                        // is enforced by ids(), not an unsupported wire keyword.
                        properties.insert("useShotIds".into(), json!({"type":"array","items":{"type":"string"},"minItems":1}));
                        let required = schema["required"].as_array_mut().ok_or("素材绑定输出schema缺少required")?;
                        let index = required.iter().position(|key| key == "sourceRefs").ok_or("素材绑定输出schema缺少来源字段")?;
                        required[index] = json!("useShotIds");
                    }
                    (false, true) => (),
                    _ => return Err("素材绑定输出schema形状冲突".into()),
                }
            }
        }
        match schema {
            Value::Object(map) => for child in map.values_mut() { visit(child)?; },
            Value::Array(items) => for child in items { visit(child)?; },
            _ => (),
        }
        Ok(())
    }
    visit(&mut request.schema)?;
    let runtime = request.rule_snapshot.get_mut("runtimeContract").ok_or("素材绑定缺少可信运行合同")?;
    if runtime.get(FIELD).is_none() {
        let content = runtime["content"].as_str().ok_or("素材绑定运行合同缺少正文")?;
        let content = format!("{content}\n\n{RULE}");
        runtime["content"] = json!(content);
        runtime["sha256"] = json!(digest(&content));
        runtime[FIELD] = json!(PROFILE);
    } else if runtime[FIELD] != PROFILE || !runtime["content"].as_str().is_some_and(|text| text.ends_with(RULE)) {
        return Err("素材绑定可信合同不一致".into());
    }
    Ok(())
}

fn ids(value: &Value, key: &str, nonempty: bool) -> Result<Vec<String>, String> {
    let items = value[key].as_array().ok_or_else(|| format!("素材使用缺少 {key}"))?;
    let mut seen = HashSet::new();
    let result = items.iter().map(|item| {
        let id = item.as_str().filter(|id| !id.is_empty()).ok_or("素材使用坐标为空或非字符串")?;
        if !seen.insert(id) { return Err("素材使用坐标重复"); }
        Ok(id.to_string())
    }).collect::<Result<Vec<_>, _>>().map_err(str::to_string)?;
    if nonempty && result.is_empty() { return Err(format!("素材使用 {key} 不能为空")); }
    Ok(result)
}

fn identity_evidence(asset: &Value) -> Value {
    json!({"purpose":"REGISTERED_IDENTITY_NOT_ACTION","assetId":asset["id"],"variantId":asset["variantId"],
        "assetVersion":asset["version"],"sourceRefsSha256":digest(&asset["sourceRefs"].to_string()),"sourceRefsPurpose":asset["sourceRefsPurpose"]})
}

fn usage_refs(unit: &Value, reference: &Value) -> Result<Value, String> {
    let owned = ids(unit, "sourceRefs", true)?;
    let uses = ids(reference, "useShotIds", true)?;
    let shots = unit["shots"].as_array().ok_or("素材使用缺少本单元镜表")?;
    let mut local = HashMap::new();
    for shot in shots {
        let id = shot["id"].as_str().filter(|id| !id.is_empty()).ok_or("素材使用镜号无效")?;
        if local.insert(id, shot).is_some() { return Err("素材使用镜号在本单元重复".into()); }
    }
    let mut selected = Vec::new();
    for id in uses {
        let shot = local.get(id.as_str()).ok_or("素材使用引用未知或其它单元镜号")?;
        for row in ids(shot, "sourceRefs", true)? {
            if !selected.contains(&row) { selected.push(row); }
        }
    }
    // Keep invalid same-package citations visible for the existing bounded
    // diagnostic path; never filter them into a seemingly valid projection.
    let mut refs: Vec<_> = owned.into_iter().filter(|id| selected.contains(id)).collect();
    for row in selected { if !refs.contains(&row) { refs.push(row); } }
    Ok(json!(refs))
}

/// Input and raw are separate. The caller retains raw before deriving a copy;
/// no old failed raw is mechanically repaired or promoted into an adopted job.
pub(super) fn materialize(input: &Value, raw: &Value) -> Result<Value, String> {
    materialize_with_order_gaps(input, raw, &HashSet::new())
}

// Diagnostic projection only: the original Finisher must return a fresh valid
// delta. No reference is reordered, removed, renumbered or accepted here.
pub(super) fn materialize_order_gap_diagnostic(input: &Value, raw: &Value, issues: &[Value]) -> Result<Value, String> {
    if issues.is_empty() || issues.iter().any(|issue| issue["code"] != "ASSET_USE_ORDER") {
        return Err(ORDER_REJECTION.into());
    }
    let units=issues.iter().filter_map(|issue|issue["unitId"].as_str().map(str::to_owned)).collect::<HashSet<_>>();
    if units.len()!=issues.len() { return Err(ORDER_REJECTION.into()); }
    for issue in issues {
        let matches=raw["replacements"].as_array().into_iter().flatten()
            .flat_map(|group|group["units"].as_array().into_iter().flatten())
            .filter(|unit|unit["id"]==issue["unitId"]).collect::<Vec<_>>();
        let [unit]=matches.as_slice() else {return Err(ORDER_REJECTION.into());};
        let refs=unit["assetRefs"].as_array().ok_or(ORDER_REJECTION)?;
        if issue["actualOrders"]!=json!(refs.iter().map(|r|r["order"].clone()).collect::<Vec<_>>())
            || issue["referenceSelectionSha256"]!=selection_hash(refs) {return Err(ORDER_REJECTION.into());}
    }
    materialize_with_order_gaps(input, raw, &units)
}

fn selection_hash(refs:&[Value])->String {
    digest(&json!(refs.iter().map(|r|json!({"assetId":r["assetId"],"variantId":r["variantId"],"use":r["use"],"useShotIds":r["useShotIds"]})).collect::<Vec<_>>()).to_string())
}

pub(super) fn order_gap_issues(input:&Value,raw:&Value,base:&Value,prior:&[Value])->Result<Vec<Value>,String>{
    let fail=||ORDER_REJECTION.to_string();
    if !enabled(input)? || raw["schemaVersion"]!="storyboard-delta/v2" || !input["finishRepair"].is_object(){return Err(fail());}
    let old_units=base["units"].as_array().ok_or_else(fail)?;
    let mut issues=Vec::new();
    let wire=|r:&Value,with_order:bool|{let mut v=json!({"assetId":r["assetId"],"variantId":r["variantId"],"use":r["use"],"useShotIds":r["useShotIds"]});if with_order{v["order"]=r["order"].clone();}v};
    for group in raw["replacements"].as_array().ok_or_else(fail)? {
        for unit in group["units"].as_array().ok_or_else(fail)? {
            let refs=unit["assetRefs"].as_array().ok_or_else(fail)?;
            let orders=refs.iter().map(|r|r["order"].as_u64().filter(|n|*n>0).ok_or_else(fail)).collect::<Result<Vec<_>,_>>()?;
            if orders.windows(2).any(|pair|pair[0]>=pair[1]){return Err(fail());}
            if orders.iter().enumerate().all(|(i,n)|*n==i as u64+1){continue;}
            let old=old_units.iter().find(|old|old["id"]==unit["id"]).ok_or_else(fail)?;
            let old_refs=old["assetRefs"].as_array().ok_or_else(fail)?;
            let previous_order=prior.iter().find(|issue|issue["code"]=="ASSET_USE_ORDER"&&issue["unitId"]==unit["id"]);
            if let Some(previous)=previous_order {
                if previous["actualOrders"]!=json!(old_refs.iter().map(|r|r["order"].clone()).collect::<Vec<_>>())
                    || old_refs.iter().map(|r|wire(r,false)).collect::<Vec<_>>()!=refs.iter().map(|r|wire(r,false)).collect::<Vec<_>>(){return Err(fail());}
            } else {
                if !old_refs.iter().enumerate().all(|(i,r)|r["order"].as_u64()==Some(i as u64+1)){return Err(fail());}
                let retained=old_refs.iter().filter(|old|refs.iter().any(|r|wire(old,true)==wire(r,true))).collect::<Vec<_>>();
                if retained.len()!=refs.len() || retained.iter().map(|r|wire(r,true)).collect::<Vec<_>>()!=refs.iter().map(|r|wire(r,true)).collect::<Vec<_>>(){return Err(fail());}
                let removed=old_refs.iter().filter(|old|!refs.iter().any(|r|wire(old,true)==wire(r,true))).collect::<Vec<_>>();
                if removed.is_empty() || removed.iter().any(|removed|{
                    removed["use"]!="audio" || !prior.iter().any(|issue|issue["code"]=="ASSET_USE_IDENTITY"&&issue["unitId"]==unit["id"]&&issue["assetId"]==removed["assetId"]&&issue["variantId"]==removed["variantId"])
                    || !refs.iter().any(|r|r["assetId"]==removed["assetId"]&&r["variantId"]==removed["variantId"]&&r["use"]=="character"
                        && r["useShotIds"].as_array().is_some_and(|ids|!ids.is_empty()&&removed["useShotIds"].as_array().is_some_and(|audio|!audio.is_empty()&&ids.iter().all(|id|!audio.contains(id)))))
                }){return Err(fail());}
            }
            issues.push(json!({"code":"ASSET_USE_ORDER","unitId":unit["id"],"coordinate":format!("{}.assetRefs.order",unit["id"].as_str().ok_or_else(fail)?),
                "sourceRefs":unit["sourceRefs"],"actualOrders":orders,"expectedOrders":(1..=refs.len()).collect::<Vec<_>>(),
                "referenceSelectionSha256":selection_hash(refs),"problem":ORDER_REJECTION}));
        }
    }
    if issues.is_empty(){return Err(fail());}Ok(issues)
}

fn materialize_with_order_gaps(input: &Value, raw: &Value, gaps: &HashSet<String>) -> Result<Value, String> {
    if !enabled(input)? { return Ok(raw.clone()); }
    let assets = input["assets"].as_array().ok_or("素材使用缺少冻结目录")?;
    let rows = input["sourceRows"].as_array().ok_or("素材使用缺少制作原行")?;
    let allowed: HashSet<_> = rows.iter().filter_map(|row| row["id"].as_str()).collect();
    let mut result = raw.clone();
    fn units(value: &mut Value, assets: &[Value], allowed: &HashSet<&str>, gaps: &HashSet<String>) -> Result<(), String> {
        let Some(units) = value.get_mut("units") else { return Ok(()); };
        for unit in units.as_array_mut().ok_or("素材使用单元列表无效")? {
            if unit.get(FIELD).is_some() { return Err("模型回包不得伪造程序素材绑定标记".into()); }
            let owned = ids(unit, "sourceRefs", true)?;
            if owned.iter().any(|id| !allowed.contains(id.as_str())) { return Err("素材使用单元越出冻结制作范围".into()); }
            let refs = unit["assetRefs"].as_array().ok_or("素材使用缺少资产选择")?;
            let diagnostic=gaps.contains(unit["id"].as_str().unwrap_or(""));
            if diagnostic {
                let orders=refs.iter().map(|r|r["order"].as_u64().filter(|n|*n>0).ok_or(ORDER_REJECTION)).collect::<Result<Vec<_>,_>>()?;
                if orders.windows(2).any(|p|p[0]>=p[1]){return Err(ORDER_REJECTION.into());}
            }
            let mut bindings = Vec::new();
            for (index, reference) in refs.iter().enumerate() {
                if !diagnostic && reference["order"].as_u64() != Some(index as u64 + 1) {
                    return Err(ORDER_REJECTION.into());
                }
                if ["sourceRefs", "sourceRefsPurpose", "identityEvidence"].iter().any(|key| reference.get(*key).is_some()) {
                    return Err("模型回包不得重写素材身份证据或程序使用锚".into());
                }
                let found: Vec<_> = assets.iter().filter(|asset| asset["id"] == reference["assetId"] && asset["variantId"] == reference["variantId"]).collect();
                let [asset] = found.as_slice() else { return Err("素材使用不对应唯一正式身份与形象".into()); };
                if matches!(asset["materialRole"].as_str(), Some("CONTEXT_EVIDENCE_ONLY" | "MATERIAL_SOURCE_ONLY")) {
                    return Err("背景证据或父图不授权当前实体入画".into());
                }
                let mut bound = reference.clone();
                bound["sourceRefs"] = usage_refs(unit, reference)?;
                if ids(&bound,"sourceRefs",true)?.iter().any(|id| !allowed.contains(id.as_str())) {
                    return Err("素材使用镜头越出冻结制作范围；不采用上下文或未知来源".into());
                }
                bound["sourceRefsPurpose"] = json!(USE_PURPOSE);
                bound["identityEvidence"] = identity_evidence(asset);
                bindings.push(bound);
            }
            unit["assetRefs"] = json!(bindings);
            unit[FIELD] = json!(PROFILE);
        }
        Ok(())
    }
    units(&mut result, assets, &allowed, gaps)?;
    if let Some(groups) = result.get_mut("replacements") {
        for group in groups.as_array_mut().ok_or("素材使用替换组无效")? { units(group, assets, &allowed, gaps)?; }
    }
    Ok(result)
}

/// Recheck the adopted projection too (including reopen, prefixing and packs).
pub(super) fn validate_unit(unit: &Value, assets: &[Value]) -> Result<(), String> {
    validate_unit_for_repair(unit,assets,false)
}

pub(super) fn validate_unit_for_repair(unit: &Value, assets: &[Value], defer_shot_source: bool) -> Result<(), String> {
    if !enabled(unit)? { return Ok(()); }
    for reference in unit["assetRefs"].as_array().into_iter().flatten().chain(unit["soundSources"].as_array().into_iter().flatten()) {
        let asset = assets.iter().find(|asset| asset["id"] == reference["assetId"] && asset["variantId"] == reference["variantId"]).ok_or("素材使用绑定身份失效")?;
        if reference["sourceRefsPurpose"] != USE_PURPOSE || reference["identityEvidence"] != identity_evidence(asset)
            || reference["sourceRefs"] != usage_refs(unit, reference)? {
            return Err("素材使用锚或登记身份依据变化，未采用".into());
        }
        if !defer_shot_source && ids(reference,"sourceRefs",true)?.iter().any(|id| !ids(unit,"sourceRefs",true).unwrap_or_default().contains(id)) {
            return Err(crate::text_executor::AUTHOR_SHOT_SOURCE_REJECTION.into());
        }
    }
    Ok(())
}

pub(super) fn remap_unit(unit: &mut Value, old_owner: &str, shots: &HashMap<(String, String), String>) -> Result<(), String> {
    if !enabled(unit)? { return Ok(()); }
    for key in ["assetRefs", "soundSources"] {
        for reference in unit.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten() {
            let old = ids(reference, "useShotIds", true)?;
            reference["useShotIds"] = json!(old.iter().map(|id| shots.get(&(old_owner.to_string(), id.clone())).cloned().ok_or("素材使用镜号无法按本单元重定位")).collect::<Result<Vec<_>, _>>()?);
        }
    }
    Ok(())
}

pub(super) fn merge_reference(left: &mut Value, right: &Value) -> Result<(), String> {
    if left.get("useShotIds").is_none() && right.get("useShotIds").is_none() { return Ok(()); }
    if left["sourceRefsPurpose"] != USE_PURPOSE || right["sourceRefsPurpose"] != USE_PURPOSE || left["identityEvidence"] != right["identityEvidence"] {
        return Err("联排素材使用绑定版本或登记身份不一致".into());
    }
    let mut ids = ids(left, "useShotIds", true)?;
    for id in self::ids(right, "useShotIds", true)? { if !ids.contains(&id) { ids.push(id); } }
    left["useShotIds"] = json!(ids);
    Ok(())
}

pub(super) fn copy_sound_fields(sound: &mut Value, reference: &Value) {
    for field in ["useShotIds", "sourceRefsPurpose", "identityEvidence"] {
        if let Some(value) = reference.get(field) { sound[field] = value.clone(); }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn fixture() -> (Value, Value) {
        let input = json!({FIELD:PROFILE,"assets":[{"id":"child","variantId":"base","version":3,
            "sourceRefs":["identity-before"],"sourceRefsPurpose":"SOURCE_SCOPE","materialRole":"ACTUAL_APPEARANCE"}],
            "sourceRows":[{"id":"r1"},{"id":"r2"}],"contextRows":[{"id":"identity-before"}]});
        let raw = json!({"schemaVersion":"storyboard-draft/v2","units":[{"id":"U1","sourceRefs":["r1","r2"],
            "assetRefs":[{"assetId":"child","variantId":"base","use":"character","order":1,"useShotIds":["S2"]}],
            "shots":[{"id":"S1","sourceRefs":["r1"],"contextRefs":["identity-before"]},{"id":"S2","sourceRefs":["r2"]}]}],"fantasyRealization":[],"revisionNotes":[]});
        (input, raw)
    }

    #[test]
    fn local_shot_use_is_distinct_from_identity_and_raw_is_unchanged() {
        let (input, raw) = fixture();
        let before = serde_json::to_vec(&raw).unwrap();
        let draft = materialize(&input, &raw).unwrap();
        let bound = &draft["units"][0]["assetRefs"][0];
        assert_eq!(bound["sourceRefs"], json!(["r2"]));
        assert_eq!(bound["identityEvidence"]["sourceRefsSha256"], digest(&json!(["identity-before"]).to_string()));
        assert!(bound["identityEvidence"].get("sourceRefs").is_none(),"long identity ranges stay once in the frozen catalogue");
        assert_eq!(bound["identityEvidence"]["purpose"], "REGISTERED_IDENTITY_NOT_ACTION");
        validate_unit(&draft["units"][0], input["assets"].as_array().unwrap()).unwrap();
        assert_eq!(serde_json::to_vec(&raw).unwrap(), before);
        let reopened: Value = serde_json::from_slice(&serde_json::to_vec(&draft).unwrap()).unwrap();
        validate_unit(&reopened["units"][0], input["assets"].as_array().unwrap()).unwrap();
        assert_eq!(materialize(&input, &raw).unwrap(), reopened);
        let mut changed = reopened;
        changed["units"][0]["assetRefs"][0]["sourceRefs"] = json!(["identity-before"]);
        assert!(validate_unit(&changed["units"][0], input["assets"].as_array().unwrap()).is_err());
    }

    #[test]
    fn post_audio_removal_gap_is_diagnostic_only_and_requires_exact_evidence(){
        let (mut input,mut base)=fixture();
        input["finishRepair"]=json!({"attempt":1});
        let mut prop=input["assets"][0].clone();prop["id"]=json!("prop");
        input["assets"].as_array_mut().unwrap().push(prop);
        base["units"][0]["assetRefs"]=json!([
            {"assetId":"child","variantId":"base","use":"character","order":1,"useShotIds":["S1"]},
            {"assetId":"child","variantId":"base","use":"audio","order":2,"useShotIds":["S2"]},
            {"assetId":"prop","variantId":"base","use":"prop","order":3,"useShotIds":["S1"]}]);
        let mut unit=base["units"][0].clone();unit["assetRefs"].as_array_mut().unwrap().remove(1);
        let raw=json!({"schemaVersion":"storyboard-delta/v2","replacements":[{"units":[unit]}]});
        let before=raw.clone();
        let prior=vec![json!({"code":"ASSET_USE_IDENTITY","unitId":"U1","assetId":"child","variantId":"base"})];
        let issues=order_gap_issues(&input,&raw,&base,&prior).unwrap();
        assert_eq!(issues[0]["actualOrders"],json!([1,3]));
        assert_eq!(materialize(&input,&raw).unwrap_err(),ORDER_REJECTION);
        let diagnostic=materialize_order_gap_diagnostic(&input,&raw,&issues).unwrap();
        assert_eq!(diagnostic["replacements"][0]["units"][0]["assetRefs"][1]["order"],3);
        assert_eq!(raw,before);
        for mode in ["legacy","no-prior","wrong-identity","duplicate","zero","descending","selection-change","forged-proof","forged-anchor"] {
            let mut input=input.clone();let mut raw=raw.clone();let mut prior=prior.clone();let mut issues=issues.clone();
            match mode {
                "legacy"=>{input.as_object_mut().unwrap().remove(FIELD);},
                "no-prior"=>prior.clear(),
                "wrong-identity"=>prior[0]["assetId"]=json!("other"),
                "duplicate"=>raw["replacements"][0]["units"][0]["assetRefs"][1]["order"]=json!(1),
                "zero"=>raw["replacements"][0]["units"][0]["assetRefs"][0]["order"]=json!(0),
                "descending"=>raw["replacements"][0]["units"][0]["assetRefs"][0]["order"]=json!(4),
                "selection-change"=>raw["replacements"][0]["units"][0]["assetRefs"][1]["assetId"]=json!("unknown"),
                "forged-proof"=>issues[0]["referenceSelectionSha256"]=json!("wrong"),
                _=>raw["replacements"][0]["units"][0]["assetRefs"][1]["sourceRefs"]=json!(["r1"]),
            }
            if matches!(mode,"forged-proof"|"forged-anchor") {
                assert!(materialize_order_gap_diagnostic(&input,&raw,&issues).is_err(),"{mode}");
            } else {assert!(order_gap_issues(&input,&raw,&base,&prior).is_err(),"{mode}");}
        }
    }

    #[test]
    fn rejects_unknown_cross_unit_duplicate_context_and_forged_use() {
        let (input, raw) = fixture();
        for uses in [json!([]),json!(["other-unit:S1"]),json!(["S2","S2"]),json!([null])] {
            let mut bad = raw.clone();
            bad["units"][0]["assetRefs"][0]["useShotIds"] = uses;
            assert!(materialize(&input,&bad).is_err());
        }
        for key in ["sourceRefs","sourceRefsPurpose","identityEvidence"] {
            let mut bad = raw.clone(); bad["units"][0]["assetRefs"][0][key]=json!(null);
            assert!(materialize(&input,&bad).is_err(),"{key}");
        }
        let mut bad = raw.clone(); bad["units"][0][FIELD]=json!(PROFILE);
        assert!(materialize(&input,&bad).is_err());
        let mut bad = raw.clone(); bad["units"][0]["shots"][1]["sourceRefs"]=json!(["identity-before"]);
        assert!(materialize(&input,&bad).is_err());
        let mut bad = raw.clone(); bad["units"][0]["shots"][1]["sourceRefs"]=json!([]);
        assert!(materialize(&input,&bad).is_err());
        let mut bad = raw.clone(); bad["units"][0]["assetRefs"][0]["order"]=json!(2);
        assert!(materialize(&input,&bad).is_err());
        for role in ["MATERIAL_SOURCE_ONLY","CONTEXT_EVIDENCE_ONLY"] {
            let mut bad = input.clone(); bad["assets"][0]["materialRole"]=json!(role);
            assert!(materialize(&bad,&raw).is_err());
        }
        let mut bad = raw.clone(); bad["units"][0]["assetRefs"][0]["variantId"]=json!("future-injury");
        assert!(materialize(&input,&bad).is_err());
        let mut bad = input.clone(); bad["assets"].as_array_mut().unwrap().push(input["assets"][0].clone());
        assert!(materialize(&bad,&raw).is_err());
    }

    #[test]
    fn unmarked_bytes_and_profile_freezing_are_exact() {
        let (mut input, raw)=fixture(); input.as_object_mut().unwrap().remove(FIELD);
        let old_input=serde_json::to_vec(&input).unwrap(); let old_raw=serde_json::to_vec(&raw).unwrap();
        assert_eq!(serde_json::to_vec(&materialize(&input,&raw).unwrap()).unwrap(),old_raw);
        let mut target=json!({"existing":"unchanged"});let target_bytes=serde_json::to_vec(&target).unwrap();
        inherit(&mut target,&input).unwrap();assert_eq!(serde_json::to_vec(&target).unwrap(),target_bytes);
        assert_eq!(serde_json::to_vec(&input).unwrap(),old_input);
        freeze_new(&mut input).unwrap(); inherit(&mut target,&input).unwrap(); assert_eq!(target[FIELD],PROFILE);
        assert!(inherit(&mut target,&json!({})).is_err());
        for bad in [Value::Null,json!("asset-use-binding/v999"),json!(true)] {
            input[FIELD]=bad; assert!(enabled(&input).is_err()); assert!(freeze_new(&mut input).is_err());
        }
    }

    #[test]
    fn delta_derives_only_replacements_and_keeps_retained_and_raw_bytes() {
        let (input,raw)=fixture();
        let mut base=materialize(&input,&raw).unwrap();
        let retained=serde_json::to_vec(&base["units"][0]).unwrap();
        let mut replacement=raw["units"][0].clone(); replacement["id"]=json!("NEW");
        base["units"].as_array_mut().unwrap().push(json!({"id":"REPLACE","opaqueOldField":"do not rewrite"}));
        let delta=json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":super::super::storyboard::finish_base_hash(&base),"retainedUnitIds":["U1"],
            "replacements":[{"replaces":["REPLACE"],"units":[replacement],"fantasyRealization":[]}],"revisionNotes":[],"unresolved":[]});
        let bytes=serde_json::to_vec(&delta).unwrap();
        let projected=materialize(&input,&delta).unwrap();
        let next=super::super::storyboard::apply_finish_delta(&base,&projected).unwrap();
        assert_eq!(serde_json::to_vec(&next["units"][0]).unwrap(),retained);
        assert_eq!(next["units"][1][FIELD],PROFILE);
        assert_eq!(serde_json::to_vec(&delta).unwrap(),bytes);
        assert!(delta["replacements"][0]["units"][0]["assetRefs"][0].get("sourceRefs").is_none());
    }

    #[test]
    fn all_four_final_role_schemas_and_contracts_are_idempotent_and_old_bytes_stay_old() {
        use super::super::{stage3_pack,storyboard::creative};
        for (stage,skill) in [("storyboardPlan","episode-emotion-planner"),("storyboardCreate","unit-performance-director"),("storyboardFinish","episode-finishing-editor"),("storyboardRefine","director-refinement")] {
            let mut input=json!({"sourceRows":[],"assets":[],"settings":{},"capabilities":{"maxDuration":15},"capacityBrief":{"preferredRange":[13,15]}});
            stage3_pack::mark_input(&mut input).unwrap();
            let asset=super::super::obj(json!({"assetId":{"type":"string"},"variantId":{"type":"string"},"use":{"type":"string"},"order":{"type":"integer"},"sourceRefs":{"type":"array","items":{"type":"string"}}}));
            let request=TextRequest{stage:stage.into(),skill:skill.into(),model:"offline".into(),reasoning:"medium".into(),input,schema:json!({"type":"object","properties":{"replacements":{"type":"array","items":{"type":"object","properties":{"assetRefs":{"type":"array","items":asset}}}}}}),rule_snapshot:stage3_pack::snapshot(skill,&["storyboard-common"]).unwrap()};
            let mut old=creative::prepare_request(request.clone()).unwrap();
            let old_bytes=serde_json::to_vec(&old).unwrap();prepare_request(&mut old).unwrap();assert_eq!(serde_json::to_vec(&old).unwrap(),old_bytes);
            let mut fresh=request;freeze_new(&mut fresh.input).unwrap();let fresh=creative::prepare_request(fresh).unwrap();
            let properties=&fresh.schema["properties"]["replacements"]["items"]["properties"]["assetRefs"]["items"]["properties"];
            assert!(properties.get("sourceRefs").is_none());assert!(properties.get("useShotIds").is_some());assert!(properties.get("identityEvidence").is_none());
            assert_eq!(fresh.rule_snapshot["runtimeContract"][FIELD],PROFILE);
            assert_eq!(fresh.rule_snapshot["runtimeContract"]["content"].as_str().unwrap().matches(RULE).count(),1);
            assert!(serde_json::to_vec(&creative::prepare_request(fresh.clone()).unwrap()).unwrap()==serde_json::to_vec(&fresh).unwrap(),"{stage} must be stable on repeated prepare");
            let cli=crate::text_executor::compile_codex_input(&fresh).unwrap();
            let attempt=crate::text_transport::FrozenAttempt::prepare_sdk_rules_first("attempt","workflow","node",fresh.clone(),&json!({"fixture":true})).unwrap();
            let sdk=crate::text_executor::compile_sdk_attempt_input(&attempt).unwrap();
            assert_eq!(sdk.schema,cli.schema);
            let wire:Value=serde_json::from_str(&sdk.schema).unwrap();
            fn check_schema(value:&Value)->usize {
                let mut count=0;
                if let Value::Object(map)=value { assert!(!map.contains_key("uniqueItems"),"CLI/SDK wire must use supported structured-output keywords; Rust rejects duplicates"); }
                if value["properties"].get("assetId").is_some() { let fields=&value["properties"];assert!(fields.get("sourceRefs").is_none());assert!(fields.get("useShotIds").is_some());count+=1; }
                match value {Value::Object(map)=>count+=map.values().map(check_schema).sum::<usize>(),Value::Array(list)=>count+=list.iter().map(check_schema).sum::<usize>(),_=>()};count
            }
            assert_eq!(check_schema(&wire),1);
            assert_ne!(old_bytes,serde_json::to_vec(&fresh).unwrap());
        }
    }
}
