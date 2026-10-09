//! Mechanical, reversible transport for oversized original three-role requests.
//! The canonical request and every accepted draft remain unchanged.
use super::{digest, TextRequest, Value};
use serde_json::json;
use std::collections::HashMap;
#[path = "storyboard_schema.rs"]
mod schema_projection;

pub(crate) const STORYBOARD_TRANSPORT_LIMIT: usize = 200_000;
// Frozen only on new workflows. Missing markers preserve the historical wire.
pub(crate) const REPETITION_TRANSPORT_PROFILE_FIELD: &str = "storyboardRepetitionTransportProfile";
pub(crate) const REPETITION_TRANSPORT_PROFILE: &str = "gain-gated-finish-repetitions/v1";

fn uses_repetition_transport_profile(input: &Value) -> Result<bool, String> {
    match input.get(REPETITION_TRANSPORT_PROFILE_FIELD) {
        None => Ok(false),
        Some(Value::String(value)) if value == REPETITION_TRANSPORT_PROFILE => Ok(true),
        _ => Err("分镜重复资料运输版本未知；保留冻结请求，未调用模型".into()),
    }
}

pub(crate) fn attach_repetition_transport_profile(input: &mut Value, frozen: &Value) -> Result<(), String> {
    if uses_repetition_transport_profile(frozen)? {
        input[REPETITION_TRANSPORT_PROFILE_FIELD] = json!(REPETITION_TRANSPORT_PROFILE);
    }
    Ok(())
}
const PROFILE: &str = "storyboard-lossless-finish/v1";
const THREE_ROLE_PROFILE: &str = "storyboard-lossless-input/v1";
// Frozen deployed v1 envelope, used only to preserve an unmarked workflow's
// greedy finishing partition. Actual dispatch can still use the current wire.
const LEGACY_FINISH_INPUT_RULE: &str = "【资料单跳等值引用】本次输入中的{\"$same\":\"#/路径\"}表示本JSON对应路径处的完整原值，数字为0基数组下标；目标保留完整且不含引用。{\"$rowIds\":[[\"sourceRows\",起,止],...]}表示按列举顺序取本输入sourceRows或contextRows对应行数组的id，0基且含末项；不变更行顺序或制作覆盖。plan.sourceFingerprints的{\"$sourceFingerprints\":\"SHA256_UTF8_ROWS\"}仅代表执行核已逐项验证的程序核验元数据：两原行目录各id对应其text UTF-8字节SHA256；若有literal字典，该部分原样保留本输入未展示原行的既有核验值，不是新增原行或本批事实权限。模型不要猜测或重算hash，不由此派生创作内容、覆盖或事实权限。这些只替代已由程序逐值核同的重复资料，不是摘要、删减或新的身份；引用不改变制作范围、前情边界或事实权限。按目标完整资料照常审查，输出schema及原始完整id不变。previousDraft完整现稿、sourceRows/contextRows原行、声道与前后连续态保持直接可读，原岗位和方法规则全部生效。";
pub(crate) const STORYBOARD_LOSSLESS_INPUT_RULE: &str = "【资料单跳等值引用】本次输入中的{\"$same\":\"#/路径\"}表示本JSON对应路径处的完整原值，数字为0基数组下标；目标保留完整且不含引用。{\"$rowIds\":[[\"sourceRows\",起,止],...]}表示按列举顺序取本输入sourceRows或contextRows对应行目录的id，0基且含末项；不变更行顺序或制作覆盖。若原行目录为{\"$sourceRows\":{\"columns\":...,\"values\":...}}，values逐行与columns.row列名一一对应；其中range数组对应columns.range，dialogue数组对应columns.dialogue，spokenSegments/performanceSegments每项数组对应columns.segment，其range同样对应columns.range。null仍为原null，未压缩的对象仍是完整原行。所有原行正文、quote、说者、声道、分段正文及绝对UTF16半开区间均直接存在，不须猜测、摘要或补全；原id完整不变。plan.sourceFingerprints的{\"$sourceFingerprints\":\"SHA256_UTF8_ROWS\"}仅代表执行核已逐项验证的程序核验元数据：两原行目录各id对应其text UTF-8字节SHA256；若有literal字典，该部分原样保留本输入未展示原行的既有核验值，不是新增原行或本批事实权限。模型不要猜测或重算hash，不由此派生创作内容、覆盖或事实权限。这些只替代已由程序逐值核同的重复资料，不是摘要、删减或新的身份；引用不改变制作范围、前情边界或事实权限。按目标完整资料照常审查，输出schema及原始完整id不变。previousDraft完整现稿、sourceRows/contextRows原行、声道与前后连续态保持直接可读，原岗位和方法规则全部生效。";
const SHARED_ROW_RULE: &str = "【原行共享列补充】shared.version=source-row-shared/v1时，tuple行的episodeId/sceneId若为整数，分别取shared.episodeIds/shared.sceneIds中0基位置的完整字符串；null仍为原null。dialogue.id若为null，完整值是字符串dialogue:加该行完整id；分段text若为null，完整值就是同一dialogue的完整quote。只有程序已逐值验证相等的重复字段使用上述写法；行text、quote、说者、声道、分段坐标均保留。输出始终使用完整原id，不输出共享列索引。";
fn shared_row_rule() -> String { format!("{STORYBOARD_LOSSLESS_INPUT_RULE}\n{SHARED_ROW_RULE}") }

fn supported_request(request: &TextRequest) -> bool {
    matches!(
        request.stage.as_str(),
        "storyboardPlan" | "storyboardCreate"
    ) || (request.stage == "storyboardFinish" && request.input["finishMode"] == "delta")
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledStoryboardTransport {
    pub profile: &'static str,
    pub input: Value,
    pub schema: Value,
}

/// The trusted executor can explain the mechanical notation in its outer
/// instruction. A user-supplied field/marker is never promoted to authority.
pub(crate) fn storyboard_transport_rule(
    request: &TextRequest,
    input: &Value,
) -> Option<&'static str> {
    let rule = match input.get("losslessInputRule").and_then(Value::as_str) {
        Some(rule) if rule == STORYBOARD_LOSSLESS_INPUT_RULE => STORYBOARD_LOSSLESS_INPUT_RULE,
        Some(rule) if rule == shared_row_rule() => {
            static RULE:std::sync::OnceLock<String>=std::sync::OnceLock::new();
            RULE.get_or_init(shared_row_rule).as_str()
        },
        Some(rule)
            if rule == LEGACY_FINISH_INPUT_RULE
                && request.stage == "storyboardFinish"
                && request.input["finishMode"] == "delta" =>
        {
            LEGACY_FINISH_INPUT_RULE
        }
        _ => return None,
    };
    let candidate = supported_request(request)
        && request.input.get("losslessInputRule").is_none()
        && !has_notation(&request.input)
        && input != &request.input;
    if !candidate {
        return None;
    }
    let mut projected = input.clone();
    projected.as_object_mut()?.remove("losslessInputRule");
    (expand(&projected, &projected).ok().as_ref() == Some(&request.input)).then_some(rule)
}

fn has_notation(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key("$same")
                || map.contains_key("$rowIds")
                || map.contains_key("$sourceFingerprints")
                || map.contains_key("$sourceRows")
                || map.values().any(has_notation)
        }
        Value::Array(items) => items.iter().any(has_notation),
        _ => false,
    }
}

fn reference_if_shorter(original: &Value, pointer: &str) -> Value {
    let reference = json!({"$same":format!("#{pointer}")});
    if reference.to_string().encode_utf16().count() < original.to_string().encode_utf16().count() {
        reference
    } else {
        original.clone()
    }
}

fn same_list(input: &Value, local: &Value, target: &str) -> Value {
    let Some(canonical) = input.pointer(target) else {
        return local.clone();
    };
    if canonical == local {
        return reference_if_shorter(local, target);
    }
    let (Some(local), Some(canonical)) = (local.as_array(), canonical.as_array()) else {
        return local.clone();
    };
    json!(local
        .iter()
        .map(|item| {
            canonical
                .iter()
                .position(|candidate| candidate == item)
                .map(|index| reference_if_shorter(item, &format!("{target}/{index}")))
                .unwrap_or_else(|| item.clone())
        })
        .collect::<Vec<_>>())
}

fn row_columns() -> Value {
    json!({
        "row":["id","number","range","text","kind","episodeId","sceneId","dialogue"],
        "range":["start","end"],
        "dialogue":["id","speaker","channel","quote","range","spokenSegments","performanceSegments"],
        "segment":["text","range"]
    })
}

// Serde's roundtrip rejects any unknown, omitted or differently typed field.
// Every copied quote/segment must also agree with its explicit locked interval.
// A row that cannot prove that contract remains a literal full object.
fn known_locked_row(value: &Value) -> bool {
    let Ok(row) = serde_json::from_value::<super::SourceRow>(value.clone()) else {
        return false;
    };
    if serde_json::to_value(&row).ok().as_ref() != Some(value)
        || row.range.end.checked_sub(row.range.start) != Some(row.text.encode_utf16().count())
    {
        return false;
    }
    let matches = |text: &str, range: &super::SourceRange| {
        let (Some(start), Some(end)) = (
            range.start.checked_sub(row.range.start),
            range.end.checked_sub(row.range.start),
        ) else {
            return false;
        };
        super::slice_utf16(&row.text, &super::SourceRange { start, end })
            .is_ok_and(|actual| actual == text)
    };
    row.dialogue.as_ref().is_none_or(|dialogue| {
        matches(&dialogue.quote, &dialogue.range)
            && dialogue
                .spoken_segments
                .iter()
                .chain(&dialogue.performance_segments)
                .all(|segment| matches(&segment.text, &segment.range))
    })
}

fn encode_row(value: &Value) -> Value {
    // Joint members carry exact source-label intervals. Preserve the full row
    // instead of extending a frozen seven-column historical dialogue tuple.
    if !known_locked_row(value) || value.pointer("/dialogue/speakers")
        .and_then(Value::as_array).is_some_and(|members| !members.is_empty()) {
        return value.clone();
    }
    let range = |value: &Value| json!([value["start"], value["end"]]);
    let segments = |value: &Value| {
        json!(value
            .as_array()
            .unwrap()
            .iter()
            .map(|segment| { json!([segment["text"], range(&segment["range"])]) })
            .collect::<Vec<_>>())
    };
    let dialogue = if value["dialogue"].is_null() {
        Value::Null
    } else {
        let d = &value["dialogue"];
        json!([
            d["id"],
            d["speaker"],
            d["channel"],
            d["quote"],
            range(&d["range"]),
            segments(&d["spokenSegments"]),
            segments(&d["performanceSegments"])
        ])
    };
    let encoded = json!([
        value["id"],
        value["number"],
        range(&value["range"]),
        value["text"],
        value["kind"],
        value["episodeId"],
        value["sceneId"],
        dialogue
    ]);
    if encoded.to_string().encode_utf16().count() < value.to_string().encode_utf16().count() {
        encoded
    } else {
        value.clone()
    }
}

fn decode_row(value: &Value) -> Result<Value, String> {
    if value.is_object() && !has_notation(value) {
        return Ok(value.clone());
    }
    fn tuple(value: &Value, count: usize) -> Result<&[Value], String> {
        value
            .as_array()
            .filter(|items| items.len() == count)
            .map(Vec::as_slice)
            .ok_or("无损原行表列数不匹配".into())
    }
    fn range(value: &Value) -> Result<Value, String> {
        let r = tuple(value, 2)?;
        Ok(json!({"start":r[0],"end":r[1]}))
    }
    fn segments(value: &Value) -> Result<Value, String> {
        let items = value.as_array().ok_or("无损原行分段目录不是数组")?;
        Ok(Value::Array(
            items
                .iter()
                .map(|segment| {
                    let s = tuple(segment, 2)?;
                    Ok(json!({"text":s[0],"range":range(&s[1])?}))
                })
                .collect::<Result<_, String>>()?,
        ))
    }
    let row = tuple(value, 8)?;
    let dialogue = if row[7].is_null() {
        Value::Null
    } else {
        let d = tuple(&row[7], 7)?;
        json!({"id":d[0],"speaker":d[1],"channel":d[2],"quote":d[3],"range":range(&d[4])?,"spokenSegments":segments(&d[5])?,"performanceSegments":segments(&d[6])?})
    };
    let decoded = json!({"id":row[0],"number":row[1],"range":range(&row[2])?,"text":row[3],"kind":row[4],"episodeId":row[5],"sceneId":row[6],"dialogue":dialogue});
    if !known_locked_row(&decoded) {
        return Err("无损原行表字段或UTF16原文区间不匹配".into());
    }
    Ok(decoded)
}

fn decode_row_table(value: &Value) -> Result<Value, String> {
    let wrapper = value
        .as_object()
        .filter(|map| map.len() == 1)
        .ok_or("无损原行表含额外字段")?;
    let table = wrapper
        .get("$sourceRows")
        .and_then(Value::as_object)
        .filter(|map| (map.len()==2 || map.len()==3 && map.contains_key("shared")) && map.get("columns") == Some(&row_columns()))
        .ok_or("无损原行表列定义无效")?;
    let values = table
        .get("values")
        .and_then(Value::as_array)
        .ok_or("无损原行表values缺失")?;
    let shared=table.get("shared");
    if shared.is_some_and(|v|v["version"]!="source-row-shared/v1" || !v.as_object().is_some_and(|m|m.len()==3)
        || !["episodeIds","sceneIds"].iter().all(|k|v[*k].as_array().is_some_and(|a|a.iter().all(Value::is_string)))) {
        return Err("无损共享原行列定义无效".into());
    }
    let mut decoded=Vec::new();
    for value in values {
        let mut value=value.clone();
        if let (Some(shared),Some(row))=(shared,value.as_array_mut()) {
            if row.len()!=8 {return Err("无损共享原行列数不匹配".into())}
            for (field,catalog) in [(5,"episodeIds"),(6,"sceneIds")] {
                if let Some(index)=row[field].as_u64() {
                    row[field]=shared[catalog].as_array().and_then(|a|a.get(index as usize)).cloned().ok_or("无损共享原行身份索引越界")?;
                }
            }
            let row_id=row[0].as_str().ok_or("共享原行缺少完整id")?.to_string();
            if let Some(dialogue)=row[7].as_array_mut() {
                if dialogue.len()!=7 {return Err("共享对白列数不匹配".into())}
                if dialogue[0].is_null(){dialogue[0]=json!(format!("dialogue:{row_id}"));}
                let quote=dialogue[3].as_str().ok_or("共享对白缺少完整quote")?.to_string();
                for field in [5,6] {
                    for segment in dialogue[field].as_array_mut().ok_or("共享对白分段不是数组")? {
                        let segment=segment.as_array_mut().filter(|a|a.len()==2).ok_or("共享对白分段列数不匹配")?;
                        if segment[0].is_null(){segment[0]=json!(quote);}
                    }
                }
            }
        }
        decoded.push(decode_row(&value)?);
    }
    Ok(Value::Array(decoded))
}

fn catalog_rows(input: &Value, catalog: &str) -> Result<Value, String> {
    let rows = &input[catalog];
    if rows.is_array() {
        Ok(rows.clone())
    } else {
        decode_row_table(rows)
    }
}

fn compact_source_rows(input: &mut Value, canonical: &Value) {
    for catalog in ["sourceRows", "contextRows"] {
        let Some(rows) = canonical[catalog].as_array() else {
            continue;
        };
        let projected = json!({"$sourceRows":{"columns":row_columns(),"values":rows.iter().map(encode_row).collect::<Vec<_>>()}});
        // Leave unknown schemas and small lists unchanged; no shape guessing.
        if projected.to_string().encode_utf16().count()
            < canonical[catalog].to_string().encode_utf16().count()
            && decode_row_table(&projected).ok().as_ref() == Some(&canonical[catalog])
        {
            input[catalog] = projected;
        }
    }
}

fn compact_shared_source_rows(input:&mut Value,canonical:&Value) {
    for catalog in ["sourceRows","contextRows"] {
        let Some(rows)=canonical[catalog].as_array() else{continue};
        let mut episodes=Vec::new();let mut scenes=Vec::new();
        let values:Vec<_>=rows.iter().map(|original| {
            let mut encoded=encode_row(original);
            if let Some(row)=encoded.as_array_mut() {
                for (field,ids) in [(5,&mut episodes),(6,&mut scenes)] {
                    if row[field].is_string() {
                        let index=ids.iter().position(|id|id==&row[field]).unwrap_or_else(||{ids.push(row[field].clone());ids.len()-1});
                        row[field]=json!(index);
                    }
                }
                let id=row[0].as_str().unwrap().to_string();
                if let Some(dialogue)=row[7].as_array_mut() {
                    if dialogue[0]==format!("dialogue:{id}"){dialogue[0]=Value::Null;}
                    let quote=dialogue[3].clone();
                    for field in [5,6] { for segment in dialogue[field].as_array_mut().unwrap() {
                        if segment[0]==quote {segment[0]=Value::Null;}
                    }}
                }
            }
            encoded
        }).collect();
        let projected=json!({"$sourceRows":{"columns":row_columns(),"values":values,"shared":{"version":"source-row-shared/v1","episodeIds":episodes,"sceneIds":scenes}}});
        if projected.to_string().encode_utf16().count()<input[catalog].to_string().encode_utf16().count()
            && decode_row_table(&projected).ok().as_ref()==Some(&canonical[catalog]) {input[catalog]=projected;}
    }
}

fn compact_direct_copies(input: &mut Value, canonical: &Value) {
    for (local, target) in [
        ("workPackages", "/plan/workPackages"),
        ("entryFacts", "/plan/entryFacts"),
        ("selectedFantasy", "/plan/fantasyCandidates"),
        ("dependencyOutputs", "/plan/dependencyExits"),
    ] {
        if let Some(value) = canonical.get(local) {
            input[local] = same_list(canonical, value, target);
        }
    }
    if let Some(value) = canonical.get("workPackage") {
        if let Some(index) = canonical
            .pointer("/plan/workPackages")
            .and_then(Value::as_array)
            .and_then(|packages| packages.iter().position(|package| package == value))
        {
            input["workPackage"] =
                reference_if_shorter(value, &format!("/plan/workPackages/{index}"));
        }
    }
    if let (Some(top), Some(copy)) = (
        canonical.get("acceptedContinuity"),
        canonical.pointer("/plan/acceptedContinuity"),
    ) {
        if top == copy {
            input["plan"]["acceptedContinuity"] = reference_if_shorter(copy, "/acceptedContinuity");
        }
    }
}

/// Expand only the documented single-hop notation. Targets must stay directly
/// readable; neither a missing target nor reference chaining can be accepted.
fn expand(value: &Value, input: &Value) -> Result<Value, String> {
    match value {
        Value::Object(map) if map.contains_key("$sourceRows") => decode_row_table(value),
        Value::Object(map) if map.contains_key("$sourceFingerprints") => {
            if map["$sourceFingerprints"] != "SHA256_UTF8_ROWS"
                || map
                    .keys()
                    .any(|key| !["$sourceFingerprints", "literal"].contains(&key.as_str()))
            {
                return Err("整编派生核验元数据运输形状无效".into());
            }
            let mut fingerprints = source_fingerprints(input)?;
            if let Some(literal) = map.get("literal") {
                let literal = literal.as_object().ok_or("整编保留核验元数据必须为字典")?;
                for (id, value) in literal {
                    if fingerprints
                        .as_object_mut()
                        .unwrap()
                        .insert(id.clone(), value.clone())
                        .is_some()
                    {
                        return Err("整编保留核验元数据与可见原行重复".into());
                    }
                }
            }
            Ok(fingerprints)
        }
        Value::Object(map) if map.contains_key("$rowIds") => {
            if map.len() != 1 {
                return Err("整编无损行引用含额外字段".into());
            }
            let spans = map["$rowIds"]
                .as_array()
                .ok_or("整编无损行引用必须为区间数组")?;
            let mut ids = Vec::new();
            for span in spans {
                let span = span
                    .as_array()
                    .filter(|s| s.len() == 3)
                    .ok_or("整编无损行区间形状无效")?;
                let catalog = span[0]
                    .as_str()
                    .filter(|s| ["sourceRows", "contextRows"].contains(s))
                    .ok_or("整编无损行引用越出原行目录")?;
                let first = span[1].as_u64().ok_or("整编无损行引用起点无效")? as usize;
                let last = span[2].as_u64().ok_or("整编无损行引用终点无效")? as usize;
                let decoded = catalog_rows(input, catalog)?;
                let rows = decoded.as_array().ok_or("整编无损行目录缺失")?;
                if first > last || last >= rows.len() {
                    return Err("整编无损行引用越界或倒序".into());
                }
                for row in &rows[first..=last] {
                    ids.push(Value::String(
                        row["id"].as_str().ok_or("整编无损行目录缺ID")?.into(),
                    ));
                }
            }
            Ok(Value::Array(ids))
        }
        Value::Object(map) if map.contains_key("$same") => {
            if map.len() != 1 {
                return Err("整编无损运输引用含额外字段".into());
            }
            let pointer = map["$same"]
                .as_str()
                .and_then(|s| s.strip_prefix('#'))
                .ok_or("整编无损运输引用不是本输入JSON Pointer")?;
            let asset_prompt = pointer.strip_prefix("/assets/").is_some_and(|tail| {
                tail.split_once('/').is_some_and(|(index, field)| {
                    index
                        .parse::<usize>()
                        .is_ok_and(|parsed| parsed.to_string() == index)
                        && field == "design/prompt"
                })
            });
            let metadata_kind = [("/facts/", "sourceSha256"), ("/assets/", "scope")].into_iter().find_map(|(prefix, field)| {
                let (index, tail) = pointer.strip_prefix(prefix)?.split_once('/')?;
                (index.parse::<usize>().is_ok_and(|n| n.to_string() == index) && tail == field).then_some(field)
            });
            let continuity_unit = pointer == "/acceptedContinuity/unit";
            let continuity_field = pointer.starts_with("/acceptedContinuity/unit/");
            if !pointer.starts_with("/plan/") && pointer != "/acceptedContinuity"
                && !continuity_unit && !continuity_field && !asset_prompt && metadata_kind.is_none() {
                return Err("整编无损运输引用越出完整规划目录".into());
            }
            let target = input.pointer(pointer).ok_or("整编无损运输引用目标不存在")?;
            if metadata_kind == Some("sourceSha256") && !target.as_str().is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())) {
                return Err("共享源身份必须为完整 SHA256".into());
            }
            if metadata_kind == Some("scope") && !target.is_object() { return Err("共享适用范围必须为完整对象".into()); }
            if continuity_field && input.pointer("/acceptedContinuity/unit")
                .is_none_or(|unit| !unit.is_object() || has_notation(unit)) {
                return Err("整编前稿子字段必须来自完整无引用单元".into());
            }
            if continuity_unit && !target.is_object() {
                return Err("整编前稿单元等值目标必须为完整对象".into());
            }
            if asset_prompt && !target.is_string() {
                return Err("整编资产提示词等值目标必须为完整字符串".into());
            }
            if has_notation(target) {
                return Err("整编无损运输引用不允许链式或循环引用".into());
            }
            Ok(target.clone())
        }
        Value::Object(map) => Ok(Value::Object(
            map.iter()
                .map(|(key, value)| Ok((key.clone(), expand(value, input)?)))
                .collect::<Result<_, String>>()?,
        )),
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|v| expand(v, input))
                .collect::<Result<_, _>>()?,
        )),
        _ => Ok(value.clone()),
    }
}

fn source_fingerprints(input: &Value) -> Result<Value, String> {
    let mut fingerprints = serde_json::Map::new();
    for catalog in ["sourceRows", "contextRows"] {
        let decoded = catalog_rows(input, catalog)?;
        let rows = decoded.as_array().ok_or("整编派生核验元数据缺原行目录")?;
        for row in rows {
            let id = row["id"].as_str().ok_or("整编派生核验元数据缺原行ID")?;
            let text = row["text"]
                .as_str()
                .ok_or("整编派生核验元数据缺准确原行正文")?;
            if fingerprints
                .insert(id.into(), json!(digest(text)))
                .is_some()
            {
                return Err("整编派生核验元数据原行ID重复".into());
            }
        }
    }
    Ok(Value::Object(fingerprints))
}

fn fingerprint_projection(canonical: &Value) -> Option<Value> {
    let original = canonical.pointer("/plan/sourceFingerprints")?.as_object()?;
    let derived = source_fingerprints(canonical).ok()?;
    let mut literal = original.clone();
    for (id, hash) in derived.as_object()? {
        // Missing or changed visible IDs cannot become an implicit new value.
        // Non-visible entries retain their exact original keys and values.
        if literal.remove(id).as_ref() != Some(hash) {
            return None;
        }
    }
    let mut projected = json!({"$sourceFingerprints":"SHA256_UTF8_ROWS"});
    if !literal.is_empty() {
        projected["literal"] = Value::Object(literal);
    }
    (projected.to_string().encode_utf16().count()
        < Value::Object(original.clone())
            .to_string()
            .encode_utf16()
            .count())
    .then_some(projected)
}

fn compact_plan_and_rows(input: &mut Value, canonical: &Value) {
    // Full alias targets stay complete; no target may contain another reference.
    let target_catalogs = ["workPackages", "entryFacts", "fantasyCandidates"];
    let mut emotions: Vec<(Value, String)> = Vec::new();
    fn collect(value: &Value, path: &str, emotions: &mut Vec<(Value, String)>) {
        if let Some(map) = value.as_object() {
            for (key, item) in map {
                let next = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                if key == "emotionContract" && item.is_object() {
                    if !emotions.iter().any(|(v, _)| v == item) {
                        emotions.push((item.clone(), next));
                    }
                } else {
                    collect(item, &next, emotions);
                }
            }
        } else if let Some(items) = value.as_array() {
            for (i, item) in items.iter().enumerate() {
                collect(item, &format!("{path}/{i}"), emotions);
            }
        }
    }
    for key in target_catalogs {
        if let Some(value) = canonical["plan"].get(key) {
            collect(value, &format!("/plan/{key}"), &mut emotions);
        }
    }
    fn alias_targets(value: &Value, targets: &mut Vec<String>) {
        if let Some(pointer) = value
            .get("$same")
            .and_then(Value::as_str)
            .and_then(|s| s.strip_prefix('#'))
        {
            targets.push(pointer.into());
        }
        match value {
            Value::Object(map) => map.values().for_each(|value| alias_targets(value, targets)),
            Value::Array(items) => items.iter().for_each(|value| alias_targets(value, targets)),
            _ => {}
        }
    }
    let mut protected = Vec::new();
    alias_targets(input, &mut protected);
    protected.extend(emotions.iter().map(|(_, path)| path.clone()));
    fn is_protected(path: &str, targets: &[String]) -> bool {
        targets.iter().any(|target| {
            path == target
                || path
                    .strip_prefix(target)
                    .is_some_and(|tail| tail.starts_with('/'))
        })
    }
    // A future author's IDs can be absent from this batch's row catalog. They
    // cannot use row spans, but exact copies inside the complete plan can share
    // one directly readable original array without granting source coverage.
    fn collect_refs(
        value: &Value,
        path: &str,
        refs: &mut Vec<(Value, String, usize)>,
        protected: &[String],
    ) {
        match value {
            Value::Object(map) => {
                for (key, item) in map {
                    let next = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    if ["sourceRefs", "contextRefs", "anchorRefs"].contains(&key.as_str())
                        && item
                            .as_array()
                            .is_some_and(|ids| !ids.is_empty() && ids.iter().all(Value::is_string))
                    {
                        if let Some((_, target, count)) =
                            refs.iter_mut().find(|(old, _, _)| old == item)
                        {
                            if is_protected(&next, protected) && !is_protected(target, protected) {
                                *target = next.clone();
                            }
                            *count += 1;
                        } else {
                            refs.push((item.clone(), next.clone(), 1));
                        }
                    }
                    collect_refs(item, &next, refs, protected);
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    collect_refs(item, &format!("{path}/{index}"), refs, protected);
                }
            }
            _ => {}
        }
    }
    let mut repeated_refs = Vec::new();
    collect_refs(&canonical["plan"], "/plan", &mut repeated_refs, &protected);
    repeated_refs
        .retain(|(value, path, count)| *count > 1 && reference_if_shorter(value, path) != *value);
    protected.extend(repeated_refs.iter().map(|(_, path, _)| path.clone()));
    let mut positions: HashMap<String, (&str, usize)> = HashMap::new();
    for catalog in ["sourceRows", "contextRows"] {
        for (index, row) in canonical[catalog]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(id) = row["id"].as_str() else {
                return;
            };
            if positions.insert(id.into(), (catalog, index)).is_some() {
                return;
            }
        }
    }
    fn row_ids(value: &Value, positions: &HashMap<String, (&str, usize)>) -> Value {
        let Some(ids) = value
            .as_array()
            .filter(|v| !v.is_empty() && v.iter().all(Value::is_string))
        else {
            return value.clone();
        };
        let mut spans: Vec<(&str, usize, usize)> = Vec::new();
        for id in ids {
            let Some(&(catalog, index)) = positions.get(id.as_str().unwrap()) else {
                return value.clone();
            };
            if let Some(last) = spans
                .last_mut()
                .filter(|last| last.0 == catalog && last.2 + 1 == index)
            {
                last.2 = index;
            } else {
                spans.push((catalog, index, index));
            }
        }
        let replacement = json!({"$rowIds":spans});
        if replacement.to_string().encode_utf16().count() < value.to_string().encode_utf16().count()
        {
            replacement
        } else {
            value.clone()
        }
    }
    fn visit(
        value: &Value,
        path: &str,
        positions: &HashMap<String, (&str, usize)>,
        emotions: &[(Value, String)],
        repeated_refs: &[(Value, String, usize)],
        protected: &[String],
    ) -> Value {
        if is_protected(path, protected) {
            return value.clone();
        }
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, item)| {
                        let next = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                        let value = if is_protected(&next, protected) {
                            item.clone()
                        } else if ["sourceRefs", "contextRefs", "anchorRefs"]
                            .contains(&key.as_str())
                        {
                            let rows = row_ids(item, positions);
                            let alias = repeated_refs
                                .iter()
                                .find(|(original, _, _)| original == item)
                                .map(|(_, target, _)| reference_if_shorter(item, target))
                                .unwrap_or_else(|| item.clone());
                            if alias.to_string().encode_utf16().count()
                                < rows.to_string().encode_utf16().count()
                            {
                                alias
                            } else {
                                rows
                            }
                        } else if key == "emotionContract" {
                            emotions
                                .iter()
                                .find(|(v, _)| v == item)
                                .map(|(_, p)| reference_if_shorter(item, p))
                                .unwrap_or_else(|| item.clone())
                        } else {
                            visit(item, &next, positions, emotions, repeated_refs, protected)
                        };
                        (key.clone(), value)
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        visit(
                            item,
                            &format!("{path}/{index}"),
                            positions,
                            emotions,
                            repeated_refs,
                            protected,
                        )
                    })
                    .collect(),
            ),
            _ => value.clone(),
        }
    }
    if canonical["plan"].is_object() {
        input["plan"] = visit(
            &canonical["plan"],
            "/plan",
            &positions,
            &emotions,
            &repeated_refs,
            &protected,
        );
    }
    for field in ["executionBatch", "facts"] {
        if let Some(value) = canonical.get(field) {
            input[field] = visit(
                value,
                &format!("/{field}"),
                &positions,
                &[],
                &repeated_refs,
                &protected,
            );
        }
    }
}

fn compact_asset_prompt_copies(input: &mut Value, canonical: &Value) {
    for (index, asset) in canonical["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let (Some(prompt), Some(design)) = (asset.get("prompt"), asset.pointer("/design/prompt"))
        {
            if prompt.is_string() && prompt == design {
                input["assets"][index]["prompt"] =
                    reference_if_shorter(prompt, &format!("/assets/{index}/design/prompt"));
            }
        }
    }
}

pub(super) fn expand_storyboard_schema(schema: &Value) -> Result<Value, String> {
    schema_projection::expand(schema)
}

pub(super) fn expand_storyboard_input(input: &Value) -> Result<Value, String> {
    if !input
        .get("losslessInputRule")
        .and_then(Value::as_str)
        .is_some_and(|rule| {
            [STORYBOARD_LOSSLESS_INPUT_RULE, LEGACY_FINISH_INPUT_RULE].contains(&rule) || rule==shared_row_rule()
        })
    {
        return Err("无损资料缺少已知运输声明".into());
    }
    let mut projected = input.clone();
    projected
        .as_object_mut()
        .ok_or("无损资料必须为对象")?
        .remove("losslessInputRule");
    expand(&projected, &projected)
}

/// Model-input transport only: all cache identities and the output schema use
/// the original TextRequest. This is also the budget planner's exact compiler.
pub(crate) fn compile_storyboard_transport(
    request: &TextRequest,
) -> Result<Option<CompiledStoryboardTransport>, String> {
    compile_transport(request, false)
}

/// Select only the actual outgoing wire. Capacity planning and finishing
/// partitions keep their existing compiler, even when this wire is smaller.
pub(crate) fn compile_storyboard_dispatch_transport(
    request: &TextRequest,
) -> Result<Option<CompiledStoryboardTransport>, String> {
    let original = compile_dispatch_before_metadata(request)?;
    let (old_input, old_schema) = original.as_ref().map(|wire| (&wire.input, &wire.schema)).unwrap_or((&request.input, &request.schema));
    // Every already dispatchable historical wire, including partial continuity,
    // remains byte-exact. Compose lossless candidates only after that boundary.
    if request.stage != "storyboardFinish" || !supported_request(request)
        || crate::text_executor::codex_wire_characters(request, old_input, old_schema) <= STORYBOARD_TRANSPORT_LIMIT
        || has_notation(&request.input) || request.input.get("losslessInputRule").is_some() {
        return Ok(original);
    }
    let mut input = old_input.clone();
    input.as_object_mut().ok_or("整编输入不是对象")?.remove("losslessInputRule");
    compact_finish_repetitions(&mut input, &request.input);
    compact_partial_continuity(&mut input, &request.input);
    compact_shared_source_rows(&mut input, &request.input);
    compact_metadata_copies(&mut input, &request.input);
    if expand(&input, &input)? != request.input { return Err("整编共享元数据未完整还原原输入；未派发请求".into()); }
    input["losslessInputRule"] = json!(shared_row_rule());
    let before = crate::text_executor::transport_payload_characters(request, old_input, old_schema);
    let after = crate::text_executor::transport_payload_characters(request, &input, old_schema);
    if before.checked_sub(after).is_none_or(|saving| saving < 2_048)
        || crate::text_executor::codex_wire_characters(request, &input, old_schema) > STORYBOARD_TRANSPORT_LIMIT {
        return Ok(original);
    }
    Ok(Some(CompiledStoryboardTransport { profile: PROFILE, input, schema: old_schema.clone() }))
}

pub(super) fn compile_dispatch_before_metadata(
    request: &TextRequest,
) -> Result<Option<CompiledStoryboardTransport>, String> {
    let original = compile_dispatch_transport(request, true)?;
    let (old_input, old_schema) = original.as_ref().map(|wire|(&wire.input,&wire.schema)).unwrap_or((&request.input,&request.schema));
    // Only recover a previously undispatchable editor. Every fitting historical
    // wire, its cache seal and grouping remain byte-for-byte unchanged.
    if request.stage != "storyboardFinish" || !supported_request(request)
        || crate::text_executor::codex_wire_characters(request,old_input,old_schema) <= STORYBOARD_TRANSPORT_LIMIT
        || has_notation(&request.input) || request.input.get("losslessInputRule").is_some() {
        return Ok(original);
    }
    let mut input=old_input.clone();
    input.as_object_mut().ok_or("整编输入不是对象")?.remove("losslessInputRule");
    compact_partial_continuity(&mut input,&request.input);
    if expand(&input,&input)? != request.input {return Err("整编连续态重复资料反解未完整还原；未派发请求".into());}
    input["losslessInputRule"]=old_input.get("losslessInputRule").cloned().unwrap_or_else(||json!(shared_row_rule()));
    let before=crate::text_executor::transport_payload_characters(request,old_input,old_schema);
    let after=crate::text_executor::transport_payload_characters(request,&input,old_schema);
    if before.checked_sub(after).is_none_or(|saving|saving<2_048)
        || crate::text_executor::codex_wire_characters(request,&input,old_schema)>STORYBOARD_TRANSPORT_LIMIT {
        return Ok(original);
    }
    Ok(Some(CompiledStoryboardTransport{profile:PROFILE,input,schema:old_schema.clone()}))
}

#[cfg(test)]
pub(super) fn compile_dispatch_before_partial_continuity(request:&TextRequest)->Result<Option<CompiledStoryboardTransport>,String> {
    compile_dispatch_transport(request,true)
}

#[cfg(test)]
pub(super) fn compile_dispatch_before_finish_repetitions(
    request: &TextRequest,
) -> Result<Option<CompiledStoryboardTransport>, String> {
    compile_dispatch_transport(request, false)
}

// Hash and applicability metadata only. Both the source and every replacement
// must equal the canonical field. Earlier aliases protect their literal targets.
fn compact_metadata_copies(input: &mut Value, canonical: &Value) {
    fn targets(value: &Value, out: &mut Vec<String>) {
        if let Some(path) = value.get("$same").and_then(Value::as_str).and_then(|p| p.strip_prefix('#')) { out.push(path.into()); }
        match value { Value::Object(map) => map.values().for_each(|v| targets(v, out)), Value::Array(items) => items.iter().for_each(|v| targets(v, out)), _ => {} }
    }
    fn within(path: &str, ancestor: &str) -> bool { path == ancestor || path.strip_prefix(ancestor).is_some_and(|suffix| suffix.starts_with('/')) }
    let mut protected = Vec::new(); targets(input, &mut protected);
    for (catalog, field) in [("facts", "sourceSha256"), ("assets", "scope")] {
        let mut literals: Vec<(Value, String)> = Vec::new();
        for (index, item) in canonical[catalog].as_array().into_iter().flatten().enumerate() {
            let Some(value) = item.get(field) else { continue; };
            if has_notation(value) || (field == "sourceSha256" && !value.as_str().is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())))
                || (field == "scope" && !value.is_object()) { continue; }
            let local = format!("/{catalog}/{index}/{field}");
            if input.pointer(&local) != Some(value) { continue; }
            if let Some((_, target)) = literals.iter().find(|(old, _)| old == value) {
                if !protected.iter().any(|path| within(&local, path) || within(path, &local)) {
                    *input.pointer_mut(&local).unwrap() = reference_if_shorter(value, target);
                }
            } else { literals.push((value.clone(), local)); }
        }
    }
}

/// Share only unchanged parts of two independently preserved continuity units.
/// The current accepted unit stays literal. Changed fields keep the plan's exact
/// old values; incoming aliases protect their complete targets from new chains.
fn compact_partial_continuity(input:&mut Value, canonical:&Value) {
    fn targets(value:&Value,out:&mut Vec<String>) {
        if let Some(path)=value.get("$same").and_then(Value::as_str).and_then(|v|v.strip_prefix('#')) {out.push(path.into());}
        match value {Value::Object(map)=>map.values().for_each(|v|targets(v,out)),Value::Array(items)=>items.iter().for_each(|v|targets(v,out)),_=>{}}
    }
    fn within(path:&str,ancestor:&str)->bool {path==ancestor || path.strip_prefix(ancestor).is_some_and(|tail|tail.starts_with('/'))}
    fn visit(wire:&mut Value,old:&Value,accepted:&Value,local:&str,target:&str,protected:&[String]) {
        if protected.iter().any(|p|within(local,p)) {return;}
        let incoming_child=protected.iter().any(|p|within(p,local));
        if old==accepted && !has_notation(accepted) && !incoming_child {
            let reference=reference_if_shorter(accepted,target);
            if reference.to_string().encode_utf16().count()<wire.to_string().encode_utf16().count() {*wire=reference;return;}
        }
        match (wire,old,accepted) {
            (Value::Object(w),Value::Object(a),Value::Object(b))=>for (key,value) in w {
                if let (Some(old),Some(accepted))=(a.get(key),b.get(key)) {
                    let key=key.replace('~',"~0").replace('/',"~1");
                    visit(value,old,accepted,&format!("{local}/{key}"),&format!("{target}/{key}"),protected);
                }
            },
            (Value::Array(w),Value::Array(a),Value::Array(b))=>for (i,value) in w.iter_mut().enumerate() {
                if let (Some(old),Some(accepted))=(a.get(i),b.get(i)) {visit(value,old,accepted,&format!("{local}/{i}"),&format!("{target}/{i}"),protected);}
            },_=>{}
        }
    }
    let mut protected=Vec::new();targets(input,&mut protected);
    if let (Some(old),Some(accepted),Some(literal))=(canonical.pointer("/plan/acceptedContinuity/unit"),canonical.pointer("/acceptedContinuity/unit"),input.pointer("/acceptedContinuity/unit")) {
        if accepted==literal && !has_notation(literal) {
            if let Some(wire)=input.pointer_mut("/plan/acceptedContinuity/unit") {visit(wire,old,accepted,"/plan/acceptedContinuity/unit","/acceptedContinuity/unit",&protected);}
        }
    }
    let mut copies=vec![("/acceptedContinuity/sourceRefs".to_string(),"/acceptedContinuity/unit/sourceRefs".to_string()),("/plan/acceptedContinuity/sourceRefs".to_string(),"/acceptedContinuity/unit/sourceRefs".to_string())];
    for index in 0..canonical["plan"]["workPackages"].as_array().map_or(0,Vec::len) {
        copies.push(("/scopedAuthorDecisions/productionSourceRefs".into(),format!("/plan/workPackages/{index}/sourceRefs")));
    }
    for (local,target) in copies {
        if protected.iter().any(|p|within(&local,p)||within(p,&local)) {continue;}
        let (Some(old),Some(accepted),Some(literal))=(canonical.pointer(&local),canonical.pointer(&target),input.pointer(&target)) else {continue;};
        if old!=accepted || accepted!=literal || has_notation(literal) {continue;}
        let reference=reference_if_shorter(accepted,&target);
        if let Some(wire)=input.pointer_mut(&local) {
            if reference.to_string().encode_utf16().count()<wire.to_string().encode_utf16().count() {*wire=reference;}
        }
    }
}

/// The oversized editor, or an explicitly opted-in new editor with a proven
/// net saving, may share exact copies. Canonical hashes and grouping stay fixed.
fn compact_finish_repetitions(input: &mut Value, canonical: &Value) {
    fn references_plan_unit(value: &Value) -> bool {
        match value {
            Value::Object(map) => map.get("$same").and_then(Value::as_str).is_some_and(|target| {
                target == "#/plan/acceptedContinuity/unit"
                    || target.starts_with("#/plan/acceptedContinuity/unit/")
            }) || map.values().any(references_plan_unit),
            Value::Array(items) => items.iter().any(references_plan_unit),
            _ => false,
        }
    }
    if let (Some(top), Some(copy)) = (
        canonical.pointer("/acceptedContinuity/unit"),
        canonical.pointer("/plan/acceptedContinuity/unit"),
    ) {
        if top.is_object() && top == copy && !has_notation(top)
            && input.pointer("/acceptedContinuity/unit") == Some(top)
            // A whole-continuity alias has no writable unit. Existing single-hop
            // aliases may also depend on this complete unit or one of its fields.
            && input.pointer("/plan/acceptedContinuity/unit").is_some()
            && !references_plan_unit(input)
        {
            let reference = reference_if_shorter(copy, "/acceptedContinuity/unit");
            input["plan"]["acceptedContinuity"]["unit"] = reference;
        }
    }
    // Selector IDs are coverage metadata, not new rows. Every ID must resolve
    // to the directly preserved row catalogue, in exactly the original order.
    let mut positions = HashMap::new();
    for catalog in ["sourceRows", "contextRows"] {
        for (index, row) in canonical[catalog].as_array().into_iter().flatten().enumerate() {
            let Some(id) = row["id"].as_str() else { return; };
            if positions.insert(id, (catalog, index)).is_some() { return; }
        }
    }
    for field in ["sourceRefs", "contextRefs"] {
        let Some(ids) = input.pointer(&format!("/assetSelection/{field}")).and_then(Value::as_array)
            .filter(|ids| !ids.is_empty() && ids.iter().all(Value::is_string)) else { continue; };
        let mut spans: Vec<(&str, usize, usize)> = Vec::new();
        let mut known = true;
        for id in ids {
            let Some(&(catalog, index)) = positions.get(id.as_str().unwrap()) else { known = false; break; };
            if let Some(last) = spans.last_mut().filter(|span| span.0 == catalog && span.2 + 1 == index) {
                last.2 = index;
            } else { spans.push((catalog, index, index)); }
        }
        if known {
            let original = json!(ids);
            let replacement = json!({"$rowIds":spans});
            if replacement.to_string().encode_utf16().count() < original.to_string().encode_utf16().count() {
                input["assetSelection"][field] = replacement;
            }
        }
    }
}

fn compile_dispatch_transport(
    request: &TextRequest,
    finish_repetitions: bool,
) -> Result<Option<CompiledStoryboardTransport>, String> {
    let gain_gated = uses_repetition_transport_profile(&request.input)?;
    if finish_repetitions && request.stage == "storyboardFinish" {
        // Measure the old compiler's final selected wire, not an intermediate
        // candidate with an extra instruction. Unmarked frozen fitting bytes stay exact;
        // only the versioned new-workflow marker removes the whole-wire size gate.
        let original = compile_dispatch_transport(request, false)?;
        let (old_input, old_schema) = original.as_ref()
            .map(|wire| (&wire.input, &wire.schema)).unwrap_or((&request.input, &request.schema));
        if (!gain_gated && crate::text_executor::codex_wire_characters(request, old_input, old_schema) <= STORYBOARD_TRANSPORT_LIMIT)
            || !supported_request(request) || has_notation(&request.input)
            || request.input.get("losslessInputRule").is_some()
        { return Ok(original); }
        let mut input = old_input.clone();
        input.as_object_mut().ok_or("整编输入不是对象")?.remove("losslessInputRule");
        compact_finish_repetitions(&mut input, &request.input);
        if expand(&input, &input)? != request.input {
            return Err("整编重复资料反解未完整还原原输入；未派发请求".into());
        }
        input["losslessInputRule"] = old_input.get("losslessInputRule").cloned().unwrap_or_else(|| json!(shared_row_rule()));
        let old_chars = crate::text_executor::transport_payload_characters(request, old_input, old_schema);
        let new_chars = crate::text_executor::transport_payload_characters(request, &input, old_schema);
        if old_chars.checked_sub(new_chars).is_none_or(|gain| gain < 2_048) { return Ok(original); }
        return Ok(Some(CompiledStoryboardTransport {profile:PROFILE,input,schema:old_schema.clone()}));
    }
    let baseline = compile_storyboard_transport(request)?;
    if !supported_request(request)
        || has_notation(&request.input)
        || request.input.get("losslessInputRule").is_some()
    {
        return Ok(baseline);
    }
    let mut input = baseline.as_ref().map(|wire|wire.input.clone()).unwrap_or_else(||request.input.clone());
    input.as_object_mut().unwrap().remove("losslessInputRule");
    compact_direct_copies(&mut input, &request.input);
    compact_shared_source_rows(&mut input, &request.input);
    if input == request.input {
        return Ok(baseline);
    }
    if expand(&input, &input)? != request.input {
        return Err("分镜发送运输反解未逐值还原原资料；未派发请求".into());
    }
    input["losslessInputRule"] = json!(shared_row_rule());
    let (baseline_input, baseline_schema) = baseline
        .as_ref()
        .map(|wire| (&wire.input, &wire.schema))
        .unwrap_or((&request.input, &request.schema));
    let baseline_characters = crate::text_executor::transport_payload_characters(
        request,
        baseline_input,
        baseline_schema,
    );
    let candidate_characters =
        crate::text_executor::transport_payload_characters(request, &input, baseline_schema);
    // This includes both copies of the trusted explanation, strict schema and
    // JSON message escaping. A smaller input alone is not sufficient.
    if baseline_characters
        .checked_sub(candidate_characters)
        .is_none_or(|gain| gain < 2_048)
    {
        return Ok(baseline);
    }
    Ok(Some(CompiledStoryboardTransport {
        profile: if request.stage == "storyboardFinish" {
            PROFILE
        } else {
            THREE_ROLE_PROFILE
        },
        input,
        schema: baseline_schema.clone(),
    }))
}

/// Measurement compatibility, not a second creative or dispatch pipeline.
/// Never let a newly smaller wire change an already accepted editor scope.
pub(crate) fn compile_legacy_finish_grouping_transport(
    request: &TextRequest,
) -> Result<Option<CompiledStoryboardTransport>, String> {
    compile_transport(request, true)
}

fn compile_transport(
    request: &TextRequest,
    legacy_grouping: bool,
) -> Result<Option<CompiledStoryboardTransport>, String> {
    if !supported_request(request) || (legacy_grouping && request.stage != "storyboardFinish") {
        return Ok(None);
    }
    let canonical_characters = crate::text_executor::transport_payload_characters(
        request,
        &request.input,
        &request.schema,
    );
    if canonical_characters <= STORYBOARD_TRANSPORT_LIMIT {
        return Ok(None);
    }
    // A source-supplied notation marker remains ordinary data. Never guess that
    // it is an instruction or overwrite an existing independent rule field.
    if has_notation(&request.input) || request.input.get("losslessInputRule").is_some() {
        return Ok(None);
    }
    let mut input = request.input.clone();
    if legacy_grouping {
        for (local, target) in [
            ("workPackages", "/plan/workPackages"),
            ("entryFacts", "/plan/entryFacts"),
            ("selectedFantasy", "/plan/fantasyCandidates"),
        ] {
            if let Some(value) = request.input.get(local) {
                input[local] = same_list(&request.input, value, target);
            }
        }
    } else {
        compact_direct_copies(&mut input, &request.input);
    }
    let rule = if legacy_grouping {
        LEGACY_FINISH_INPUT_RULE
    } else {
        STORYBOARD_LOSSLESS_INPUT_RULE
    };
    let mut schema = request.schema.clone();
    input["losslessInputRule"] = json!(rule);
    if crate::text_executor::transport_payload_characters(request, &input, &schema)
        > STORYBOARD_TRANSPORT_LIMIT
    {
        schema = schema_projection::project(&schema);
        if schema != request.schema && schema_projection::expand(&schema)? != request.schema {
            return Err("整编共享schema反解未还原全部原约束；未派发请求".into());
        }
        if crate::text_executor::transport_payload_characters(request, &input, &schema)
            > STORYBOARD_TRANSPORT_LIMIT
        {
            compact_plan_and_rows(&mut input, &request.input);
            // The plan visitor reads canonical values. Reapply the exact
            // duplicate of the direct top-level continuity, never its reverse.
            if !legacy_grouping {
                compact_direct_copies(&mut input, &request.input);
            }
        }
        if crate::text_executor::transport_payload_characters(request, &input, &schema)
            > STORYBOARD_TRANSPORT_LIMIT
        {
            if let Some(projected) = fingerprint_projection(&request.input) {
                input["plan"]["sourceFingerprints"] = projected;
            }
        }
        if crate::text_executor::transport_payload_characters(request, &input, &schema)
            > STORYBOARD_TRANSPORT_LIMIT
        {
            compact_asset_prompt_copies(&mut input, &request.input);
        }
        if !legacy_grouping
            && crate::text_executor::transport_payload_characters(request, &input, &schema)
                > STORYBOARD_TRANSPORT_LIMIT
        {
            compact_source_rows(&mut input, &request.input);
        }
    }
    input.as_object_mut().unwrap().remove("losslessInputRule");
    if input == request.input && schema == request.schema {
        return Ok(None);
    }
    // Prove exact restoration before attaching the explanation. Nothing in the
    // existing draft, quotes, speakers, IDs, claims or applicability is removed.
    if expand(&input, &input)? != request.input {
        return Err("整编运输反解未逐值还原原资料；未派发请求".into());
    }
    input["losslessInputRule"] = json!(rule);
    if crate::text_executor::transport_payload_characters(request, &input, &schema)
        >= canonical_characters
    {
        return Ok(None);
    }
    Ok(Some(CompiledStoryboardTransport {
        profile: if request.stage == "storyboardFinish" {
            PROFILE
        } else {
            THREE_ROLE_PROFILE
        },
        input,
        schema,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_speaker_rows_stay_literal_and_all_metadata_roundtrips() {
        let source=super::super::prepare_source("第一集\n场景：码头\n人物：林川、周航\n林川&周航：真的来了！\n林川：听见了。\n").unwrap();
        let joint=serde_json::to_value(&source.rows[3]).unwrap();
        let single=serde_json::to_value(&source.rows[4]).unwrap();
        assert!(joint["dialogue"]["speakers"].as_array().unwrap().len()==2);
        assert!(single["dialogue"].get("speakers").is_none());
        assert!(known_locked_row(&joint));
        assert_eq!(encode_row(&joint),joint);
        assert_eq!(decode_row(&encode_row(&single)).unwrap(),single);
        let canonical=json!({"sourceRows":[joint.clone(),single],"contextRows":[]});
        let mut projected=canonical.clone(); compact_source_rows(&mut projected,&canonical); compact_shared_source_rows(&mut projected,&canonical);
        assert_eq!(catalog_rows(&projected,"sourceRows").unwrap(),canonical["sourceRows"]);
        assert_eq!(expand(&projected,&projected).unwrap(),canonical);
        let mut request=fixture(); request.input["sourceRows"]=canonical["sourceRows"].clone();
        let frozen=serde_json::to_vec(&request).unwrap();
        let wire=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        let mut normalized=wire.input.clone(); normalized.as_object_mut().unwrap().remove("losslessInputRule");
        assert_eq!(expand(&normalized,&normalized).unwrap(),request.input);
        assert_eq!(serde_json::to_vec(&request).unwrap(),frozen);
    }


    fn finish_repetition_fixture(padding: usize) -> TextRequest {
        let mut request = dispatch_fixture();
        let unit = json!({"id":"accepted-original-tail","shots":[{"picture":"完整已采用表演🙂".repeat(2_000)}],"exitState":"原连续态"});
        request.input["acceptedContinuity"] = json!({"version":"accepted-tail-v1","unit":unit,"evidenceRole":"ACCEPTED_CREATIVE_STATE_ONLY"});
        request.input["plan"]["acceptedContinuity"] = json!({"version":"accepted-tail-v1","executionBatchId":"original-author-batch","unit":unit,"boundaryBefore":{"kind":"continuous"},"evidenceRole":"ACCEPTED_CREATIVE_STATE_ONLY"});
        request.input["assetSelection"] = json!({"sourceRefs":request.input["sourceRows"].as_array().unwrap().iter().map(|row|row["id"].clone()).collect::<Vec<_>>(),
            "contextRefs":request.input["contextRows"].as_array().unwrap().iter().map(|row|row["id"].clone()).collect::<Vec<_>>(),"productionSceneIds":["original-scene"],"selectedAssets":[{"assetId":"person:state","purpose":"PRODUCTION_SCOPE"}]});
        request.input["previousDraft"]["units"][0]["unitDirection"] = json!("全".repeat(padding));
        request
    }

    fn dispatch_chars(request: &TextRequest, extra: bool) -> usize {
        let wire = compile_dispatch_transport(request, extra).unwrap();
        let (input,schema) = wire.as_ref().map(|wire|(&wire.input,&wire.schema)).unwrap_or((&request.input,&request.schema));
        crate::text_executor::codex_wire_characters(request,input,schema)
    }


    fn metadata_fixture(target:usize)->TextRequest {
        let mut request=finish_repetition_fixture(150_000);
        request.input["acceptedContinuity"]=Value::Null;request.input["plan"]["acceptedContinuity"]=Value::Null;
        request.input["facts"]=json!((0..39).map(|i|json!({"id":format!("fact-{i}"),"sourceSha256":"ab".repeat(32),"literal":"source fact untouched"})).collect::<Vec<_>>());
        request.input["assets"]=json!((0..13).map(|i|json!({"id":format!("asset-{i}"),"scope":{"episodes":["episode-a"],"scenes":[{"episodeId":"episode-a","sceneId":"scene-1","literalUnknownMetadata":"exact metadata🙂".repeat(12)}]}})).collect::<Vec<_>>());
        for _ in 0..3 {let wire=compile_dispatch_before_metadata(&request).unwrap().unwrap();let chars=crate::text_executor::codex_wire_characters(&request,&wire.input,&wire.schema);let n=request.input["previousDraft"]["units"][0]["unitDirection"].as_str().unwrap().encode_utf16().count();request.input["previousDraft"]["units"][0]["unitDirection"]=json!("全".repeat((n as isize+target as isize-chars as isize) as usize));}
        request
    }

    #[test]
    fn overflow_metadata_composes_exact_candidates_and_keeps_creative_fields_literal() {
        let request=metadata_fixture(203_826);let frozen=serde_json::to_vec(&request).unwrap();
        let old=compile_dispatch_before_metadata(&request).unwrap().unwrap();assert_eq!(crate::text_executor::codex_wire_characters(&request,&old.input,&old.schema),203_826);
        let new=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();assert!(crate::text_executor::codex_wire_characters(&request,&new.input,&new.schema)<=200_000);
        assert_eq!(new.input["facts"][1]["sourceSha256"],json!({"$same":"#/facts/0/sourceSha256"}));assert_eq!(new.input["assets"][1]["scope"],json!({"$same":"#/assets/0/scope"}));
        for field in ["previousDraft","acceptedContinuity","nextBoundary","settings","productionDirection"] {assert_eq!(new.input[field],old.input[field]);}
        assert_eq!(new.schema,old.schema);assert_eq!(expand_storyboard_schema(&new.schema).unwrap(),request.schema);assert_eq!(expand_storyboard_input(&new.input).unwrap(),request.input);assert_eq!(serde_json::to_vec(&request).unwrap(),frozen);
    }

    #[test]
    fn overflow_metadata_preserves_all_fitting_old_wires_and_fails_closed_when_still_large() {
        for size in [190_000,199_999,200_000,260_000] {
            let request=metadata_fixture(size);let old=compile_dispatch_before_metadata(&request).unwrap().unwrap();let new=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
            assert_eq!(serde_json::to_vec(&json!([old.profile,old.input,old.schema])).unwrap(),serde_json::to_vec(&json!([new.profile,new.input,new.schema])).unwrap());
        }
        let request=partial_continuity_fixture(208_543);let old=compile_dispatch_before_metadata(&request).unwrap().unwrap();assert!(crate::text_executor::codex_wire_characters(&request,&old.input,&old.schema)<=200_000);let new=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(serde_json::to_vec(&json!([old.input,old.schema])).unwrap(),serde_json::to_vec(&json!([new.input,new.schema])).unwrap());
    }

    #[test]
    fn metadata_duplicates_keep_incoming_targets_and_different_values() {
        let scope=json!({"episodes":["episode-a"],"scenes":[],"future":"exact unknown extension🙂".repeat(10)});
        let canonical=json!({"facts":[{"sourceSha256":"ab".repeat(32)},{"sourceSha256":"ab".repeat(32)},{"sourceSha256":"cd".repeat(32)}],"assets":[{"scope":scope},{"scope":scope},{"scope":{"episodes":["other"],"scenes":[]}}],"keepHash":"ab".repeat(32),"keepScope":scope});
        let mut input=canonical.clone();input["keepHash"]=json!({"$same":"#/facts/1/sourceSha256"});input["keepScope"]=json!({"$same":"#/assets/1/scope"});compact_metadata_copies(&mut input,&canonical);
        assert_eq!(input["facts"][1],canonical["facts"][1]);assert_eq!(input["assets"][1],canonical["assets"][1]);assert_eq!(input["facts"][2],canonical["facts"][2]);assert_eq!(input["assets"][2],canonical["assets"][2]);assert_eq!(expand(&input,&input).unwrap(),canonical);
    }

    #[test]
    fn metadata_decoder_rejects_wrong_shapes_indices_fields_and_chained_targets() {
        for target in [json!(null),json!(false),json!(0),json!("ab".repeat(31)),json!("xz".repeat(32)),json!({"$same":"#/facts/1/sourceSha256"})] {
            let input=json!({"facts":[{"sourceSha256":target}],"local":{"$same":"#/facts/0/sourceSha256"}});assert!(expand(&input,&input).is_err());
        }
        for target in [json!(null),json!(false),json!([]),json!("scope"),json!({"nested":{"$same":"#/plan/value"}}),json!({"nested":{"$rowIds":[]}})] {
            let input=json!({"assets":[{"scope":target}],"local":{"$same":"#/assets/0/scope"}});assert!(expand(&input,&input).is_err());
        }
        for pointer in ["#/facts/00/sourceSha256","#/facts/1/sourceSha256","#/facts/0/sourceSha256/","#/facts/0/id","#/assets/00/scope","#/assets/1/scope","#/assets/0/scope/episodes"] {
            let input=json!({"facts":[{"sourceSha256":"ab".repeat(32),"id":"never authorized"}],"assets":[{"scope":{"episodes":[]}}],"local":{"$same":pointer}});assert!(expand(&input,&input).is_err(),"invalid metadata pointer: {pointer}");
        }
    }

    fn partial_continuity_fixture(target:usize)->TextRequest {
        let mut request=finish_repetition_fixture(150_000);
        request.input["plan"]["acceptedContinuity"]["unit"]["exitState"]=json!("必须保留的旧状态，与已整编前稿不同");
        for _ in 0..3 {
            let old=compile_dispatch_before_partial_continuity(&request).unwrap().unwrap();
            let chars=crate::text_executor::codex_wire_characters(&request,&old.input,&old.schema);
            let old_padding=request.input["previousDraft"]["units"][0]["unitDirection"].as_str().unwrap().encode_utf16().count();
            let padding=(old_padding as isize+target as isize-chars as isize) as usize;
            request.input["previousDraft"]["units"][0]["unitDirection"]=json!("全".repeat(padding));
        }
        request
    }

    #[test]
    fn overflow_partial_continuity_restores_changed_old_fields_and_full_frozen_input() {
        let request=partial_continuity_fixture(208_543);
        let before=serde_json::to_vec(&request).unwrap();let old=compile_dispatch_before_partial_continuity(&request).unwrap().unwrap();
        assert_eq!(crate::text_executor::codex_wire_characters(&request,&old.input,&old.schema),208_543);
        let wire=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert!(crate::text_executor::codex_wire_characters(&request,&wire.input,&wire.schema)<=STORYBOARD_TRANSPORT_LIMIT);
        assert_eq!(wire.input["plan"]["acceptedContinuity"]["unit"]["exitState"],request.input["plan"]["acceptedContinuity"]["unit"]["exitState"]);
        assert_eq!(wire.input["plan"]["acceptedContinuity"]["unit"]["shots"],json!({"$same":"#/acceptedContinuity/unit/shots"}));
        for field in ["acceptedContinuity","previousDraft","settings"] {assert_eq!(wire.input[field],old.input[field]);}
        assert_eq!(expand_storyboard_input(&wire.input).unwrap(),request.input);assert_eq!(expand_storyboard_schema(&wire.schema).unwrap(),request.schema);
        assert_eq!(serde_json::to_vec(&request).unwrap(),before);
    }

    #[test]
    fn overflow_partial_continuity_fitting_wires_remain_byte_exact() {
        for size in [190_000,199_999,200_000] {
            let request=partial_continuity_fixture(size);let bytes=|w:Option<CompiledStoryboardTransport>|serde_json::to_vec(&w.map(|w|json!([w.profile,w.input,w.schema]))).unwrap();
            assert_eq!(bytes(compile_dispatch_before_partial_continuity(&request).unwrap()),bytes(compile_storyboard_dispatch_transport(&request).unwrap()));
        }
    }

    #[test]
    fn overflow_partial_continuity_preserves_incoming_alias_targets_and_escapes_paths() {
        let same=json!({"quoted/key~":"必须原样保留的完整值🙂".repeat(1_000)});
        let mut canonical=json!({"acceptedContinuity":{"unit":{"same":same,"changed":"new"}},"plan":{"acceptedContinuity":{"unit":{"same":same,"changed":"old"}}},"kept":same});
        let mut wire=canonical.clone();wire["kept"]=json!({"$same":"#/plan/acceptedContinuity/unit/same"});
        compact_partial_continuity(&mut wire,&canonical);
        assert_eq!(wire["plan"]["acceptedContinuity"]["unit"]["same"],canonical["plan"]["acceptedContinuity"]["unit"]["same"]);
        assert_eq!(expand(&wire,&wire).unwrap(),canonical);
        canonical.as_object_mut().unwrap().remove("kept");
        canonical["plan"]["acceptedContinuity"]["unit"]["same"]["different"]=json!(false);
        wire=canonical.clone();compact_partial_continuity(&mut wire,&canonical);
        assert_eq!(wire["plan"]["acceptedContinuity"]["unit"]["same"]["quoted/key~"],json!({"$same":"#/acceptedContinuity/unit/same/quoted~1key~0"}));
        assert_eq!(expand(&wire,&wire).unwrap(),canonical);
    }

    #[test]
    fn overflow_partial_continuity_does_not_publish_a_still_oversized_candidate() {
        let request=partial_continuity_fixture(250_000);
        let old=compile_dispatch_before_partial_continuity(&request).unwrap().unwrap();let new=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(serde_json::to_vec(&json!([old.input,old.schema])).unwrap(),serde_json::to_vec(&json!([new.input,new.schema])).unwrap());
    }

    #[test]
    fn gain_gated_fitting_finish_restores_every_value_and_keeps_frozen_grouping() {
        let mut request = finish_repetition_fixture(0);
        request.input[REPETITION_TRANSPORT_PROFILE_FIELD] = json!(REPETITION_TRANSPORT_PROFILE);
        let frozen = serde_json::to_vec(&request).unwrap();
        let before = compile_dispatch_before_finish_repetitions(&request).unwrap().unwrap();
        assert!(crate::text_executor::codex_wire_characters(&request,&before.input,&before.schema) < STORYBOARD_TRANSPORT_LIMIT);
        let grouping = compile_storyboard_transport(&request).unwrap().map(|wire|json!([wire.input,wire.schema]));
        let legacy_grouping = compile_legacy_finish_grouping_transport(&request).unwrap().map(|wire|json!([wire.input,wire.schema]));
        let after = compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(after.input["plan"]["acceptedContinuity"]["unit"],json!({"$same":"#/acceptedContinuity/unit"}));
        assert_eq!(after.input["acceptedContinuity"],request.input["acceptedContinuity"]);
        assert_eq!(after.input["previousDraft"],request.input["previousDraft"]);
        assert_eq!(expand_storyboard_input(&after.input).unwrap(),request.input);
        assert_eq!(expand_storyboard_schema(&after.schema).unwrap(),request.schema);
        let before_chars=crate::text_executor::transport_payload_characters(&request,&before.input,&before.schema);
        let after_chars=crate::text_executor::transport_payload_characters(&request,&after.input,&after.schema);
        assert!(before_chars-after_chars>=2_048);
        assert_eq!(serde_json::to_vec(&request).unwrap(),frozen);
        assert_eq!(compile_storyboard_transport(&request).unwrap().map(|wire|json!([wire.input,wire.schema])),grouping);
        assert_eq!(compile_legacy_finish_grouping_transport(&request).unwrap().map(|wire|json!([wire.input,wire.schema])),legacy_grouping);
    }

    #[test]
    fn gain_gated_small_saving_or_different_value_keeps_the_original_wire() {
        for picture in ["小量重复🙂".repeat(50),"完整长稿🙂".repeat(2_000)] {
            let unit=json!({"id":"tail","picture":picture});
            let mut request=TextRequest {stage:"storyboardFinish".into(),skill:"episode-finishing-editor".into(),model:"offline".into(),reasoning:"medium".into(),
                input:json!({"finishMode":"delta","acceptedContinuity":{"unit":unit},"plan":{"acceptedContinuity":{"unit":unit,"executionBatchId":"original"}},"previousDraft":{"units":[]}}),
                schema:json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),rule_snapshot:json!({"primary":{"content":"完整原规则"}})};
            request.input[REPETITION_TRANSPORT_PROFILE_FIELD]=json!(REPETITION_TRANSPORT_PROFILE);
            if picture.len()>2_048 { request.input["plan"]["acceptedContinuity"]["unit"]["distinct"]=json!(true); }
            let wire_bytes=|wire:Option<CompiledStoryboardTransport>| serde_json::to_vec(&wire.map(|wire|json!([wire.input,wire.schema]))).unwrap();
            assert_eq!(wire_bytes(compile_storyboard_dispatch_transport(&request).unwrap()),wire_bytes(compile_dispatch_before_finish_repetitions(&request).unwrap()));
        }
    }

    #[test]
    fn gain_gated_keeps_existing_alias_targets_complete() {
        let mut request=finish_repetition_fixture(180_000);
        request.input[REPETITION_TRANSPORT_PROFILE_FIELD]=json!(REPETITION_TRANSPORT_PROFILE);
        let refs=json!((0..16).map(|index|format!("prior-row-{index}-{}","x".repeat(32))).collect::<Vec<_>>());
        request.input["acceptedContinuity"]["unit"]["sourceRefs"]=refs.clone();
        request.input["plan"]["acceptedContinuity"]["unit"]["sourceRefs"]=refs.clone();
        request.input["plan"]["events"]=json!([{"sourceRefs":refs}]);
        let old=compile_dispatch_before_finish_repetitions(&request).unwrap().unwrap();
        let new=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(old.input["plan"]["events"][0]["sourceRefs"],json!({"$same":"#/plan/acceptedContinuity/unit/sourceRefs"}));
        assert_eq!(new.input["plan"]["acceptedContinuity"]["unit"],old.input["plan"]["acceptedContinuity"]["unit"]);
        assert_eq!(expand_storyboard_input(&new.input).unwrap(),request.input);
    }

    #[test]
    fn gain_gated_profile_is_explicit_inherited_and_unknown_versions_fail_closed() {
        let empty=json!({}); let mut target=json!({"original":"unchanged"}); let bytes=target.to_string();
        attach_repetition_transport_profile(&mut target,&empty).unwrap(); assert_eq!(target.to_string(),bytes);
        let mut frozen=json!({}); frozen[REPETITION_TRANSPORT_PROFILE_FIELD]=json!(REPETITION_TRANSPORT_PROFILE);
        attach_repetition_transport_profile(&mut target,&frozen).unwrap();
        assert_eq!(target[REPETITION_TRANSPORT_PROFILE_FIELD],REPETITION_TRANSPORT_PROFILE);
        for invalid in [Value::Null,json!(false),json!(0),json!(""),json!("unknown/v99")] {
            frozen[REPETITION_TRANSPORT_PROFILE_FIELD]=invalid.clone(); let before=target.to_string();
            assert!(attach_repetition_transport_profile(&mut target,&frozen).is_err()); assert_eq!(target.to_string(),before);
            for stage in ["storyboardPlan","storyboardCreate","storyboardFinish"] {
                let mut request=finish_repetition_fixture(0); request.stage=stage.into(); request.input[REPETITION_TRANSPORT_PROFILE_FIELD]=invalid.clone();
                assert!(compile_storyboard_dispatch_transport(&request).is_err());
            }
        }
    }

    #[test]
    fn finish_repetition_preserves_existing_whole_continuity_alias() {
        let mut request = finish_repetition_fixture(180_000);
        request.input["plan"]["acceptedContinuity"] = request.input["acceptedContinuity"].clone();
        let old = compile_dispatch_before_finish_repetitions(&request).unwrap().unwrap();
        assert!(crate::text_executor::codex_wire_characters(&request, &old.input, &old.schema) > STORYBOARD_TRANSPORT_LIMIT);
        assert_eq!(old.input["plan"]["acceptedContinuity"], json!({"$same":"#/acceptedContinuity"}));
        let new = compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(new.input["plan"]["acceptedContinuity"], old.input["plan"]["acceptedContinuity"]);
        assert_eq!(expand_storyboard_input(&new.input).unwrap(), request.input);
    }

    #[test]
    fn finish_repetition_keeps_original_plan_unit_when_existing_aliases_target_its_children() {
        let mut request = finish_repetition_fixture(180_000);
        let refs = json!((0..16).map(|index| format!("prior-row-{index}-{}", "x".repeat(32))).collect::<Vec<_>>());
        request.input["acceptedContinuity"]["unit"]["sourceRefs"] = refs.clone();
        request.input["plan"]["acceptedContinuity"]["unit"]["sourceRefs"] = refs.clone();
        request.input["plan"]["events"] = json!([{"sourceRefs":refs}]);
        let old = compile_dispatch_before_finish_repetitions(&request).unwrap().unwrap();
        assert!(crate::text_executor::codex_wire_characters(&request, &old.input, &old.schema) > STORYBOARD_TRANSPORT_LIMIT);
        assert_eq!(old.input["plan"]["events"][0]["sourceRefs"], json!({"$same":"#/plan/acceptedContinuity/unit/sourceRefs"}));
        assert_eq!(expand_storyboard_input(&old.input).unwrap(), request.input);
        let new = compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(new.input["plan"]["acceptedContinuity"]["unit"], old.input["plan"]["acceptedContinuity"]["unit"]);
        assert_eq!(new.input["plan"]["events"], old.input["plan"]["events"]);
        assert_eq!(expand_storyboard_input(&new.input).unwrap(), request.input);
    }

    #[test]
    fn finish_repetition_fitting_canonical_boundary_never_adds_a_transport_instruction() {
        let unit=json!({"id":"tail","picture":"完整🙂".repeat(2_000)});
        let mut request=TextRequest {stage:"storyboardFinish".into(),skill:"episode-finishing-editor".into(),model:"offline".into(),reasoning:"medium".into(),
            input:json!({"finishMode":"delta","acceptedContinuity":{"unit":unit},"plan":{"acceptedContinuity":{"unit":unit,"executionBatchId":"old-batch"}},"previousDraft":{"units":[{"id":"current","unitDirection":""}]}}),
            schema:json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),rule_snapshot:json!({"primary":{"content":"原规则"}})};
        let base=crate::text_executor::codex_wire_characters(&request,&request.input,&request.schema);
        request.input["previousDraft"]["units"][0]["unitDirection"]=json!("全".repeat(199_800-base));
        assert_eq!(crate::text_executor::codex_wire_characters(&request,&request.input,&request.schema),199_800);
        assert!(compile_dispatch_before_finish_repetitions(&request).unwrap().is_none());
        assert!(compile_storyboard_dispatch_transport(&request).unwrap().is_none());
        let compiled=crate::text_executor::compile_codex_input(&request).unwrap();
        assert_eq!(compiled.prompt.encode_utf16().count()+compiled.schema.encode_utf16().count(),199_800);
        assert!(!compiled.prompt.contains("losslessInputRule"));
    }

    #[test]
    fn finish_repetition_uses_canonical_equality_when_the_plan_copy_is_already_projected() {
        let mut request=finish_repetition_fixture(0);
        let ids=request.input["assetSelection"]["sourceRefs"].clone();
        request.input["acceptedContinuity"]["unit"]["sourceRefs"]=ids.clone();
        request.input["plan"]["acceptedContinuity"]["unit"]["sourceRefs"]=ids;
        let mut wire=request.input.clone();
        wire["plan"]["acceptedContinuity"]["unit"]["sourceRefs"]=json!({"$rowIds":[["sourceRows",0,request.input["sourceRows"].as_array().unwrap().len()-1]]});
        assert_eq!(expand(&wire,&wire).unwrap(),request.input);
        compact_finish_repetitions(&mut wire,&request.input);
        assert_eq!(wire["plan"]["acceptedContinuity"]["unit"],json!({"$same":"#/acceptedContinuity/unit"}));
        assert_eq!(expand(&wire,&wire).unwrap(),request.input);
    }

    #[test]
    fn finish_repetition_wire_roundtrips_complete_units_and_selector_rows_with_real_wrapper_capacity() {
        let mut padding = 150_000;
        let mut request = finish_repetition_fixture(padding);
        for _ in 0..3 {
            let old = dispatch_chars(&request, false);
            padding = (padding as isize + 215_000 - old as isize).try_into().unwrap();
            request.input["previousDraft"]["units"][0]["unitDirection"] = json!("全".repeat(padding));
        }
        let canonical = serde_json::to_vec(&request).unwrap();
        let grouping = compile_storyboard_transport(&request).unwrap();
        let legacy = compile_legacy_finish_grouping_transport(&request).unwrap();
        assert_eq!(dispatch_chars(&request, false), 215_000);
        assert!(dispatch_chars(&request, true) <= STORYBOARD_TRANSPORT_LIMIT);
        let wire = compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(wire.input["plan"]["acceptedContinuity"]["unit"], json!({"$same":"#/acceptedContinuity/unit"}));
        assert!(wire.input["assetSelection"]["sourceRefs"].get("$rowIds").is_some());
        assert!(wire.input["assetSelection"]["contextRefs"].get("$rowIds").is_some());
        assert_eq!(wire.input["acceptedContinuity"], request.input["acceptedContinuity"]);
        assert_eq!(wire.input["previousDraft"], request.input["previousDraft"]);
        assert_eq!(expand_storyboard_input(&wire.input).unwrap(), request.input);
        assert_eq!(expand_storyboard_schema(&wire.schema).unwrap(), request.schema);
        assert_eq!(serde_json::to_vec(&request).unwrap(), canonical);
        let grouping_after = compile_storyboard_transport(&request).unwrap();
        let legacy_after = compile_legacy_finish_grouping_transport(&request).unwrap();
        assert_eq!(grouping.map(|wire|json!([wire.input,wire.schema])), grouping_after.map(|wire|json!([wire.input,wire.schema])));
        assert_eq!(legacy.map(|wire|json!([wire.input,wire.schema])), legacy_after.map(|wire|json!([wire.input,wire.schema])));
    }

    #[test]
    fn finish_repetition_fitting_and_nonfinish_compiled_wires_keep_exact_bytes() {
        let fitting = finish_repetition_fixture(0);
        assert!(dispatch_chars(&fitting,false) <= STORYBOARD_TRANSPORT_LIMIT);
        for request in [fitting, {let mut request=finish_repetition_fixture(180_000);request.stage="storyboardCreate".into();request.skill="unit-performance-director".into();request}] {
            let old = compile_dispatch_before_finish_repetitions(&request).unwrap();
            let new = compile_storyboard_dispatch_transport(&request).unwrap();
            let bytes = |wire:Option<CompiledStoryboardTransport>| {
                let (input,schema)=wire.map(|wire|(wire.input,wire.schema)).unwrap_or((request.input.clone(),request.schema.clone()));
                serde_json::to_vec(&json!([input,schema])).unwrap()
            };
            assert_eq!(bytes(old),bytes(new),"fitting or already-frozen other-role transport is unaffected");
        }
    }

    #[test]
    fn finish_repetition_never_aliases_different_units_or_unknown_ambiguous_selector_ids() {
        let mut request=finish_repetition_fixture(180_000);
        request.input["plan"]["acceptedContinuity"]["unit"]["exitState"]=json!("真正不同的前稿末态");
        request.input["assetSelection"]["contextRefs"][0]=json!("unprovided-source-id");
        let wire=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(wire.input["plan"]["acceptedContinuity"]["unit"],request.input["plan"]["acceptedContinuity"]["unit"]);
        assert_eq!(wire.input["assetSelection"]["contextRefs"],request.input["assetSelection"]["contextRefs"]);
        assert_eq!(expand_storyboard_input(&wire.input).unwrap(),request.input);
        let mut ambiguous=finish_repetition_fixture(0).input;
        let original=ambiguous.clone();
        let row=ambiguous["sourceRows"][0].clone();
        ambiguous["contextRows"][0]=row;
        let canonical=ambiguous.clone();
        compact_finish_repetitions(&mut ambiguous,&canonical);
        assert_eq!(ambiguous["assetSelection"],original["assetSelection"],"ambiguous catalogue does not grant coverage from a guessed index");
    }

    #[test]
    fn finish_repetition_decoder_accepts_only_the_literal_unit_and_its_exact_fields() {
        let unit=json!({"id":"literal-original-unit","shots":[{"picture":"完整原镜头🙂","sourceRefs":["original-row"]}],"exitState":"保留原末态","futureField":{"literal":"完整未知扩展也原样保留"}});
        let reference=json!({"$same":"#/acceptedContinuity/unit"});
        let original=json!({"acceptedContinuity":{"unit":unit},"plan":{"acceptedContinuity":{"unit":reference}}});
        assert_eq!(expand(&original,&original).unwrap()["plan"]["acceptedContinuity"]["unit"],unit);
        for path in ["/shots","/shots/0","/exitState","/futureField"] {
            let input=json!({"acceptedContinuity":{"unit":unit},"local":{"$same":format!("#/acceptedContinuity/unit{path}")}});
            assert_eq!(expand(&input,&input).unwrap()["local"],*unit.pointer(path).unwrap());
        }
        for pointer in ["#/acceptedContinuity/executionBatchId","#/acceptedContinuity/unit/","#/acceptedContinuity/unit/missing","#/acceptedContinuity/unit/shots/01","#/acceptedContinuity//unit","#/acceptedContinuity/unit~1shots"] {
            let input=json!({"acceptedContinuity":{"unit":unit,"executionBatchId":"batch"},"local":{"$same":pointer}});
            assert!(expand(&input,&input).is_err(),"missing or out-of-unit path is not authorized: {pointer}");
        }
        for target in [Value::Null,json!("not a unit"),json!([]),json!(1),json!(true),
            json!({"$same":"#/plan/original"}),
            json!({"id":"chained-unit","nested":{"$same":"#/plan/original"}}),
            json!({"id":"row-unit","nested":{"$rowIds":[["sourceRows",0,0]]}}),
            json!({"id":"table-unit","nested":{"$sourceRows":{}}}),
            json!({"id":"fingerprint-unit","nested":{"$sourceFingerprints":"SHA256_UTF8_ROWS"}})] {
            let input=json!({"acceptedContinuity":{"unit":target},"plan":{"original":unit},"local":reference});
            assert!(expand(&input,&input).is_err(),"target must be a complete literal object without any nested notation");
        }
        let extra=json!({"acceptedContinuity":{"unit":unit},"local":{"$same":"#/acceptedContinuity/unit","extra":true}});
        assert!(expand(&extra,&extra).is_err());
        let missing=json!({"acceptedContinuity":{},"local":reference});
        assert!(expand(&missing,&missing).is_err());
        for marker in [json!({"$same":"#/plan/value"}),json!({"$rowIds":[]}),json!({"$sourceRows":{}}),json!({"$sourceFingerprints":"SHA256_UTF8_ROWS"})] {
            let nested=json!({"acceptedContinuity":{"unit":{"literal":"retained","nested":marker}},"plan":{"value":"retained"},"local":{"$same":"#/acceptedContinuity/unit/literal"}});
            assert!(expand(&nested,&nested).is_err(),"even sibling notation makes the continuity catalogue non-literal");
        }
    }

    #[test]
    fn finish_repetition_selector_decoder_preserves_mixed_order_duplicates_and_rejects_bad_spans() {
        let mut request=finish_repetition_fixture(0);
        let refs=json!([request.input["contextRows"][3]["id"],request.input["contextRows"][2]["id"],request.input["sourceRows"][1]["id"],request.input["sourceRows"][1]["id"],request.input["sourceRows"][2]["id"]]);
        request.input["assetSelection"]["sourceRefs"]=refs.clone();
        let mut wire=request.input.clone();
        compact_finish_repetitions(&mut wire,&request.input);
        compact_shared_source_rows(&mut wire,&request.input);
        assert!(wire["assetSelection"]["sourceRefs"].get("$rowIds").is_some());
        assert_eq!(expand(&wire,&wire).unwrap()["assetSelection"]["sourceRefs"],refs);
        for spans in [json!([["assets",0,0]]),json!([["sourceRows",3,2]]),json!([["contextRows",0,999999]]),json!([["sourceRows",-1,0]]),json!([["sourceRows",0,"1"]]),json!([["sourceRows",0,0,0]])] {
            let mut bad=wire.clone();bad["assetSelection"]["sourceRefs"]=json!({"$rowIds":spans});
            assert!(expand(&bad,&bad).is_err(),"selector transport cannot invent, reorder or exceed catalogue spans");
        }
        let mut extra=wire.clone();extra["assetSelection"]["sourceRefs"]=json!({"$rowIds":[["sourceRows",0,0]],"extra":true});
        assert!(expand(&extra,&extra).is_err());
    }

    #[test]
    fn shared_source_columns_roundtrip_full_ids_speech_and_unicode_with_no_canonical_changes() {
        let mut request=dispatch_fixture();
        let original=serde_json::to_vec(&request).unwrap();
        let baseline=compile_storyboard_transport(&request).unwrap();
        let legacy=compile_legacy_finish_grouping_transport(&request).unwrap();
        let wire=compile_storyboard_dispatch_transport(&request).unwrap().unwrap();
        assert_eq!(expand_storyboard_input(&wire.input).unwrap(),request.input);
        assert_eq!(serde_json::to_vec(&request).unwrap(),original);
        assert_eq!(baseline.is_some(),compile_storyboard_transport(&request).unwrap().is_some());
        assert_eq!(legacy.is_some(),compile_legacy_finish_grouping_transport(&request).unwrap().is_some());
        assert_eq!(wire.input["contextRows"]["$sourceRows"]["shared"]["version"],"source-row-shared/v1");
        let mut corrupt=wire.input.clone();corrupt["contextRows"]["$sourceRows"]["values"][0][5]=json!(9999);
        assert!(expand_storyboard_input(&corrupt).is_err());
        request.input["contextRows"][0]["dialogue"]["spokenSegments"][0]["range"]["start"]=json!(1);
        let mut projected=request.input.clone();compact_shared_source_rows(&mut projected,&request.input);
        assert_eq!(expand(&projected,&projected).unwrap(),request.input);
        assert_eq!(projected["contextRows"]["$sourceRows"]["values"][0],request.input["contextRows"][0]);
    }

    fn fixture() -> TextRequest {
        let package = json!({"id":"canonical-package","sourceRefs":["canonical-row"],"emotionContract":{"subject":"同名角色","change":"必须保留的完整变化。".repeat(2000)},"state":{"claim":"CHARACTER_CLAIM","sourceRefs":["canonical-row"],"currentApplicability":"UNKNOWN"}});
        let fact = json!({"id":"canonical-fact","evidence":"准确原行证据。".repeat(1000),"sourceRefs":["canonical-row"]});
        let fantasy = json!({"id":"canonical-fantasy","description":"完整进入、退出和现实功能。".repeat(600)});
        TextRequest {
            stage: "storyboardFinish".into(),
            skill: "episode-finishing-editor".into(),
            model: "offline".into(),
            reasoning: "medium".into(),
            input: json!({"finishMode":"delta","sourceRows":[{"id":"canonical-row","text":"甲OS：不可改写的原话。","speaker":"甲","speechMode":"OS"}],"contextRows":[{"id":"historical-row","text":"历史证据不可丢失。","currentApplicability":"UNKNOWN"}],"plan":{"workPackages":[package.clone()],"entryFacts":[fact.clone()],"fantasyCandidates":[fantasy.clone()]},"workPackages":[package],"entryFacts":[fact],"selectedFantasy":[fantasy],"previousDraft":{"units":[{"id":"paid-unit","prompt":"完整现稿不可删。".repeat(18000),"shots":[{"dialogueParts":[{"lineId":"canonical-row","speaker":"甲","channel":"OS","text":"不可改写的原话。"}]}]}]},"acceptedContinuity":{"unit":{"id":"paid-predecessor","description":"已采用退出态保持完整。"}},"baseDraftHash":"canonical-paid-draft-sha"}),
            schema: json!({"type":"object","properties":{"baseDraftHash":{"type":"string","enum":["canonical-paid-draft-sha"]}}}),
            rule_snapshot: json!({"primary":{"id":"original-editor","content":"原规则保持。"}}),
        }
    }

    #[test]
    fn oversized_finish_roundtrips_every_value_and_keeps_draft_source_and_rules_direct() {
        let request = fixture();
        let before = serde_json::to_string(&request).unwrap();
        let wire = compile_storyboard_transport(&request).unwrap().unwrap();
        assert_eq!(
            storyboard_transport_rule(&request, &wire.input),
            Some(STORYBOARD_LOSSLESS_INPUT_RULE)
        );
        assert!(storyboard_transport_rule(&request, &request.input).is_none());
        let mut tampered = wire.input.clone();
        tampered["previousDraft"]["units"][0]["id"] = json!("different-paid-unit");
        assert!(storyboard_transport_rule(&request, &tampered).is_none());
        assert_eq!(wire.profile, PROFILE);
        assert_eq!(wire.schema, request.schema);
        for key in [
            "previousDraft",
            "sourceRows",
            "contextRows",
            "acceptedContinuity",
            "baseDraftHash",
        ] {
            assert_eq!(
                wire.input[key], request.input[key],
                "{key} must stay directly readable"
            );
        }
        assert_eq!(
            wire.input["workPackages"],
            json!({"$same":"#/plan/workPackages"})
        );
        assert!(wire.input["losslessInputRule"]
            .as_str()
            .unwrap()
            .contains("原始完整id"));
        let mut projected = wire.input.clone();
        projected
            .as_object_mut()
            .unwrap()
            .remove("losslessInputRule");
        assert_eq!(expand(&projected, &projected).unwrap(), request.input);
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            before,
            "cache snapshot/hash must not drift"
        );
        assert!(
            crate::text_executor::transport_payload_characters(&request, &wire.input, &wire.schema)
                < crate::text_executor::transport_payload_characters(
                    &request,
                    &request.input,
                    &request.schema
                )
        );
    }

    #[test]
    fn differing_scopes_and_claims_are_never_coalesced_and_existing_markers_stay_data() {
        let mut request = fixture();
        request.input["entryFacts"][0]["claim"] = json!("SOURCE");
        request.input["entryFacts"][0]["sourceRefs"] = json!(["different-row"]);
        let wire = compile_storyboard_transport(&request).unwrap().unwrap();
        assert_eq!(wire.input["entryFacts"], request.input["entryFacts"]);
        request.input["contextRows"][0]["literal"] = json!({"$same":"#/ordinary-source-data"});
        assert!(compile_storyboard_transport(&request).unwrap().is_none());
        let mut request = fixture();
        request.input["losslessInputRule"] = json!("原输入独有说明不可覆盖");
        assert!(compile_storyboard_transport(&request).unwrap().is_none());
        let mut arbitrary = request.input.clone();
        arbitrary["losslessInputRule"] = json!(STORYBOARD_LOSSLESS_INPUT_RULE);
        assert!(storyboard_transport_rule(&request, &arbitrary).is_none());
    }

    #[test]
    fn short_requests_and_non_delta_editor_inputs_keep_original_transport() {
        let mut request = fixture();
        request.input["previousDraft"] = json!({"units":[]});
        request.input["plan"] = json!({});
        request.input["workPackages"] = json!([]);
        request.input["entryFacts"] = json!([]);
        request.input["selectedFantasy"] = json!([]);
        assert!(compile_storyboard_transport(&request).unwrap().is_none());
        let mut request = fixture();
        request.stage = "storyboardCreate".into();
        assert!(compile_storyboard_transport(&request).unwrap().is_some());
        request.stage = "storyboardFinish".into();
        request.input["finishMode"] = json!("full");
        assert!(compile_storyboard_transport(&request).unwrap().is_none());
    }

    #[test]
    fn decoder_rejects_unknown_target_chaining_and_extra_fields() {
        for input in [
            json!({"plan":{},"local":{"$same":"#/plan/missing"}}),
            json!({"plan":{"a":{"$same":"#/plan/b"},"b":"value"},"local":{"$same":"#/plan/a"}}),
            json!({"plan":{"a":"value"},"local":{"$same":"#/plan/a","extra":"not-an-alias"}}),
            json!({"sourceRows":[],"local":{"$same":"#/sourceRows"}}),
        ] {
            assert!(expand(&input, &input).is_err());
        }
    }

    #[test]
    fn row_spans_preserve_catalog_order_repeated_ids_unknown_refs_and_complete_targets() {
        let source:Vec<_>=(0..12).map(|i|json!({"id":format!("canonical-long-source-coordinate-{i}"),"text":format!("原行{i}。"),"speaker":"甲","speechMode":"OS"})).collect();
        let context:Vec<_>=(0..8).map(|i|json!({"id":format!("canonical-long-history-coordinate-{i}"),"text":format!("历史{i}。"),"currentApplicability":"UNKNOWN"})).collect();
        let mut refs: Vec<_> = source.iter().map(|r| r["id"].clone()).collect();
        refs.extend(context.iter().rev().map(|r| r["id"].clone()));
        refs.push(source[2]["id"].clone());
        refs.push(source[2]["id"].clone());
        let emotion = json!({"subject":"甲","change":"必要情绪变化不能摘要。".repeat(40),"retain":["原话与OS声道"]});
        let mut canonical = json!({"sourceRows":source,"contextRows":context,"previousDraft":{"unique":"完整现稿。"},"acceptedContinuity":{"unit":{"unique":"退出态。"}},"nextBoundary":{"unique":"下一范围。"},"plan":{"workPackages":[{"id":"stable-owner","sourceRefs":refs,"emotionContract":emotion.clone()}],"entryFacts":[],"fantasyCandidates":[],"events":[{"id":"stable-event","sourceRefs":refs,"emotionContract":emotion}],"unknownExtension":{"sourceRefs":["unavailable-source-row"]}},"executionBatch":{"sourceRefs":refs},"facts":[{"claim":"CHARACTER_CLAIM","currentApplicability":"UNKNOWN","sourceRefs":refs}]});
        let mut span_only: Vec<_> = canonical["sourceRows"].as_array().unwrap()[..6]
            .iter()
            .map(|row| row["id"].clone())
            .collect();
        span_only.extend([
            canonical["sourceRows"][2]["id"].clone(),
            canonical["sourceRows"][2]["id"].clone(),
        ]);
        canonical["executionBatch"]["sourceRefs"] = json!(span_only);
        canonical["workPackages"] = canonical["plan"]["workPackages"].clone();
        let mut projected = canonical.clone();
        projected["workPackages"] = json!({"$same":"#/plan/workPackages"});
        compact_plan_and_rows(&mut projected, &canonical);
        assert!(projected["executionBatch"]["sourceRefs"]
            .get("$rowIds")
            .is_some());
        assert_eq!(
            projected["plan"]["events"][0]["sourceRefs"],
            json!({"$same":"#/plan/workPackages/0/sourceRefs"})
        );
        assert_eq!(
            projected["plan"]["events"][0]["emotionContract"],
            json!({"$same":"#/plan/workPackages/0/emotionContract"})
        );
        for field in [
            "sourceRows",
            "contextRows",
            "previousDraft",
            "acceptedContinuity",
            "nextBoundary",
        ] {
            assert_eq!(projected[field], canonical[field]);
        }
        assert_eq!(
            projected["plan"]["workPackages"],
            canonical["plan"]["workPackages"]
        );
        assert_eq!(
            projected["plan"]["unknownExtension"],
            canonical["plan"]["unknownExtension"]
        );
        assert_eq!(
            expand(&projected, &projected).unwrap(),
            canonical,
            "ordered, reversed and repeated IDs roundtrip exactly"
        );
        assert_eq!(projected["facts"][0]["claim"], "CHARACTER_CLAIM");
    }

    #[test]
    fn only_actual_alias_targets_stay_full_while_other_packages_compact_exactly() {
        let rows:Vec<_>=(0..16).map(|i|json!({"id":format!("long-source-coordinate-{i}"),"text":format!("准确原句{i}。"),"speaker":"甲"})).collect();
        let refs: Vec<_> = rows.iter().map(|row| row["id"].clone()).collect();
        let first =
            json!({"id":"owner-one","sourceRefs":refs,"emotionContract":{"change":"第一完整情绪"}});
        let second = json!({"id":"owner-two","sourceRefs":refs,"contextRefs":refs,"emotionContract":{"change":"第二完整情绪"}});
        let canonical = json!({"sourceRows":rows,"contextRows":[],"plan":{"workPackages":[first.clone(),second],"entryFacts":[],"fantasyCandidates":[]},"workPackages":[first]});
        let mut projected = canonical.clone();
        projected["workPackages"] = json!([{"$same":"#/plan/workPackages/0"}]);
        compact_plan_and_rows(&mut projected, &canonical);
        assert_eq!(
            projected["plan"]["workPackages"][0],
            canonical["plan"]["workPackages"][0]
        );
        assert!(projected["plan"]["workPackages"][1]["sourceRefs"]
            .get("$rowIds")
            .is_some());
        assert_eq!(
            projected["plan"]["workPackages"][1]["emotionContract"],
            canonical["plan"]["workPackages"][1]["emotionContract"]
        );
        assert_eq!(expand(&projected, &projected).unwrap(), canonical);
    }

    #[test]
    fn future_plan_refs_and_exact_asset_prompt_copies_remain_single_hop_full_values() {
        let future: Vec<_> = (0..18)
            .map(|i| json!(format!("future-source-coordinate-{i}")))
            .collect();
        let canonical = json!({"sourceRows":[{"id":"present-row","text":"本批原文不变。"}],"contextRows":[],"plan":{"events":[{"sourceRefs":future}],"workPackages":[{"sourceRefs":future,"beatContracts":[{"sourceRefs":future}]}],"entryFacts":[],"fantasyCandidates":[]},"assets":[{"id":"canonical-asset","variantId":"canonical-variant","prompt":"已采用设计内容保持。".repeat(100),"design":{"prompt":"已采用设计内容保持。".repeat(100)},"scope":{"mode":"SCENES","scenes":["future-scene"]}}]});
        let mut projected = canonical.clone();
        compact_plan_and_rows(&mut projected, &canonical);
        compact_asset_prompt_copies(&mut projected, &canonical);
        assert_eq!(
            projected["plan"]["events"][0]["sourceRefs"],
            canonical["plan"]["events"][0]["sourceRefs"]
        );
        assert_eq!(
            projected["plan"]["workPackages"][0]["sourceRefs"],
            json!({"$same":"#/plan/events/0/sourceRefs"})
        );
        assert_eq!(
            projected["assets"][0]["prompt"],
            json!({"$same":"#/assets/0/design/prompt"})
        );
        assert_eq!(
            projected["assets"][0]["design"],
            canonical["assets"][0]["design"]
        );
        assert_eq!(expand(&projected, &projected).unwrap(), canonical);
        assert!(expand(
            &json!({"assets":[{"prompt":"literal"}],"copy":{"$same":"#/assets/0/prompt"}}),
            &json!({"assets":[{"prompt":"literal"}]})
        )
        .is_err());
        let mut changed = canonical.clone();
        changed["assets"][0]["prompt"] = json!("独立人工修改的提示词");
        let frozen = changed.clone();
        compact_asset_prompt_copies(&mut changed, &frozen);
        assert_eq!(
            changed, frozen,
            "only exactly equal duplicated prompts can share"
        );
    }

    #[test]
    fn invalid_row_spans_and_literal_row_notation_cannot_acquire_authority() {
        for input in [
            json!({"sourceRows":[{"id":"r"}],"local":{"$rowIds":[["sourceRows",1,1]]}}),
            json!({"sourceRows":[{"id":"r"}],"local":{"$rowIds":[["sourceRows",0,1]]}}),
            json!({"sourceRows":[{"id":"r"}],"local":{"$rowIds":[["assets",0,0]]}}),
            json!({"sourceRows":[{"id":"r"}],"local":{"$rowIds":[["sourceRows",1,0]]}}),
            json!({"sourceRows":[{"id":"r"}],"local":{"$rowIds":[["sourceRows",0,0]],"extra":"source-data"}}),
        ] {
            assert!(expand(&input, &input).is_err());
        }
        let mut request = fixture();
        request.input["contextRows"][0]["literal"] = json!({"$rowIds":[["sourceRows",0,0]]});
        assert!(compile_storyboard_transport(&request).unwrap().is_none());
        let mut wire = request.input.clone();
        wire["losslessInputRule"] = json!(STORYBOARD_LOSSLESS_INPUT_RULE);
        assert!(storyboard_transport_rule(&request, &wire).is_none());
    }

    #[test]
    fn program_fingerprints_only_project_after_every_visible_id_and_utf8_hash_matches() {
        let mut request = fixture();
        // Keep this first branch deliberately below the real envelope limit
        // after exact aliases, including the expanded three-role explanation.
        request.input["previousDraft"]["units"][0]["prompt"] =
            json!("完整现稿不可删。".repeat(16_000));
        request.input["sourceRows"]=json!((0..82).map(|i|json!({"id":format!("canonical-source-coordinate-{i}"),"text":format!("原行{i}：非ASCII台词与表情🙂。"),"speaker":"甲","speechMode":"OS"})).collect::<Vec<_>>());
        request.input["plan"]["sourceFingerprints"] = source_fingerprints(&request.input).unwrap();
        let smaller_wire = compile_storyboard_transport(&request).unwrap().unwrap();
        assert_eq!(
            smaller_wire.input["plan"]["sourceFingerprints"],
            request.input["plan"]["sourceFingerprints"],
            "do not project the dictionary when earlier lossless aliases already fit"
        );
        assert!(
            crate::text_executor::transport_payload_characters(
                &request,
                &smaller_wire.input,
                &smaller_wire.schema
            ) <= STORYBOARD_TRANSPORT_LIMIT
        );
        // Retain a full draft above the limit even after all duplicate fields
        // have been projected, so this fixture actually exercises the final
        // program-metadata branch rather than expecting eager compression.
        request.input["previousDraft"]["units"][0]["prompt"] =
            json!("完整现稿不可删。".repeat(30_000));
        let frozen = request.clone();
        let wire = compile_storyboard_transport(&request).unwrap().unwrap();
        assert_eq!(
            wire.input["plan"]["sourceFingerprints"],
            json!({"$sourceFingerprints":"SHA256_UTF8_ROWS"})
        );
        for field in [
            "sourceRows",
            "contextRows",
            "previousDraft",
            "acceptedContinuity",
        ] {
            assert_eq!(wire.input[field], request.input[field]);
        }
        let mut projected = wire.input.clone();
        projected
            .as_object_mut()
            .unwrap()
            .remove("losslessInputRule");
        assert_eq!(expand(&projected, &projected).unwrap(), request.input);
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            serde_json::to_string(&frozen).unwrap()
        );
        request.input["plan"]["sourceFingerprints"]["canonical-source-coordinate-1"] =
            json!("incorrect-saved-hash");
        let wire = compile_storyboard_transport(&request).unwrap().unwrap();
        assert_eq!(
            wire.input["plan"]["sourceFingerprints"],
            request.input["plan"]["sourceFingerprints"]
        );
        request.input["sourceRows"][1]["id"] = request.input["sourceRows"][0]["id"].clone();
        assert!(source_fingerprints(&request.input).is_err());
    }

    #[test]
    fn mixed_fingerprints_retain_unseen_values_and_require_every_visible_identity_and_hash() {
        let mut canonical = json!({"sourceRows":(0..28).map(|i|json!({"id":format!("visible-source-coordinate-{i}"),"text":format!("准确原句{i}🙂。")})).collect::<Vec<_>>(),"contextRows":[],"plan":{}});
        canonical["plan"]["sourceFingerprints"] = source_fingerprints(&canonical).unwrap();
        let retained = json!({"unseen-later-author-row":digest("本批未携带的后续原行。")});
        canonical["plan"]["sourceFingerprints"]
            .as_object_mut()
            .unwrap()
            .extend(retained.as_object().unwrap().clone());
        let projected = fingerprint_projection(&canonical).unwrap();
        assert_eq!(projected["literal"], retained);
        assert_eq!(
            expand(&projected, &canonical).unwrap(),
            canonical["plan"]["sourceFingerprints"]
        );
        let frozen = canonical.clone();
        canonical["plan"]["sourceFingerprints"]["visible-source-coordinate-0"] =
            json!("wrong-existing-hash");
        assert!(fingerprint_projection(&canonical).is_none());
        canonical = frozen.clone();
        canonical["plan"]["sourceFingerprints"]
            .as_object_mut()
            .unwrap()
            .remove("visible-source-coordinate-0");
        assert!(
            fingerprint_projection(&canonical).is_none(),
            "unknown visible identity must not be derived into the dictionary"
        );
        canonical = frozen.clone();
        canonical["sourceRows"][1]["id"] = canonical["sourceRows"][0]["id"].clone();
        assert!(fingerprint_projection(&canonical).is_none());
        let overlap = json!({"$sourceFingerprints":"SHA256_UTF8_ROWS","literal":{"visible-source-coordinate-0":"override"}});
        assert!(expand(&overlap, &frozen).is_err());
    }

    #[test]
    fn exact_author_duplicates_keep_full_targets_scope_and_continuity() {
        let mut request = fixture();
        request.stage = "storyboardCreate".into();
        request.input["workPackage"] = request.input["plan"]["workPackages"][0].clone();
        let dependency = json!({"previous-package":{"units":[{"id":"previous-unit","sourceRefs":["historical-row"],"exitState":"原完整退出态。".repeat(4000)}]}});
        request.input["dependencyOutputs"] = dependency.clone();
        request.input["plan"]["dependencyExits"] = dependency.clone();
        request.input["plan"]["acceptedContinuity"] = request.input["acceptedContinuity"].clone();
        let frozen = serde_json::to_vec(&request).unwrap();
        let wire = compile_storyboard_transport(&request).unwrap().unwrap();
        assert_eq!(wire.profile, THREE_ROLE_PROFILE);
        assert_eq!(
            wire.input["workPackage"],
            json!({"$same":"#/plan/workPackages/0"})
        );
        assert_eq!(
            wire.input["dependencyOutputs"],
            json!({"$same":"#/plan/dependencyExits"})
        );
        assert_eq!(wire.input["plan"]["dependencyExits"], dependency);
        assert_eq!(
            wire.input["acceptedContinuity"],
            request.input["acceptedContinuity"]
        );
        assert_eq!(
            wire.input["plan"]["acceptedContinuity"],
            json!({"$same":"#/acceptedContinuity"})
        );
        let mut projected = wire.input.clone();
        projected
            .as_object_mut()
            .unwrap()
            .remove("losslessInputRule");
        assert_eq!(expand(&projected, &projected).unwrap(), request.input);
        assert_eq!(serde_json::to_vec(&request).unwrap(), frozen);
        request.input["dependencyOutputs"]["previous-package"]["units"][0]["exitState"] =
            json!("独立的真实变化，不能合并");
        let wire = compile_storyboard_transport(&request).unwrap().unwrap();
        assert_eq!(
            wire.input["dependencyOutputs"],
            request.input["dependencyOutputs"]
        );
    }

    #[test]
    fn legacy_grouping_keeps_frozen_v1_aliases_and_rule_without_new_author_aliases() {
        let mut request = fixture();
        request.input["workPackage"] = request.input["plan"]["workPackages"][0].clone();
        let dependency = json!({"previous-package":{"units":[{"id":"previous-unit","exitState":"完整依赖稿。".repeat(3000)}]}});
        request.input["dependencyOutputs"] = dependency.clone();
        request.input["plan"]["dependencyExits"] = dependency;
        request.input["plan"]["acceptedContinuity"] = request.input["acceptedContinuity"].clone();
        let frozen = serde_json::to_vec(&request).unwrap();
        let wire = compile_legacy_finish_grouping_transport(&request)
            .unwrap()
            .unwrap();
        assert_eq!(wire.profile, PROFILE);
        assert_eq!(wire.input["losslessInputRule"], LEGACY_FINISH_INPUT_RULE);
        assert_eq!(
            storyboard_transport_rule(&request, &wire.input),
            Some(LEGACY_FINISH_INPUT_RULE)
        );
        assert_eq!(
            wire.input["workPackages"],
            json!({"$same":"#/plan/workPackages"})
        );
        for key in [
            "workPackage",
            "dependencyOutputs",
            "acceptedContinuity",
            "sourceRows",
            "contextRows",
        ] {
            assert_eq!(wire.input[key], request.input[key]);
        }
        assert_eq!(
            wire.input["plan"]["acceptedContinuity"],
            request.input["plan"]["acceptedContinuity"]
        );
        assert_eq!(expand_storyboard_input(&wire.input).unwrap(), request.input);
        assert_eq!(serde_json::to_vec(&request).unwrap(), frozen);
        request.stage = "storyboardCreate".into();
        assert!(compile_legacy_finish_grouping_transport(&request)
            .unwrap()
            .is_none());
        assert!(
            storyboard_transport_rule(&request, &wire.input).is_none(),
            "v1 Finish instruction cannot acquire Author authority"
        );
        let mut arbitrary = fixture().input;
        arbitrary["losslessInputRule"] = json!("unrecognized source text");
        assert!(expand_storyboard_input(&arbitrary).is_err());
    }

    #[test]
    fn large_locked_row_tables_roundtrip_for_all_roles_without_losing_quotes_or_metadata() {
        let mut script = "第一集\n场景：同一处\n".to_string();
        for index in 0..600 {
            script.push_str(&format!(
                "甲OS：第{index}句原话🙂（压低声音）还要逐字保留。\n"
            ));
        }
        let source = super::super::prepare_source(&script).unwrap();
        let rows: Vec<Value> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .map(|row| serde_json::to_value(row).unwrap())
            .collect();
        assert!(rows.iter().all(known_locked_row));
        let mut input = json!({"sourceRows":rows[..30],"contextRows":rows[30..],"assets":[{"id":"person:state","variantId":"state","version":9,"applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]},"sourceRefs":[rows[0]["id"]]}],"previousDraft":{"units":[{"id":"accepted-unit","prompt":"完整已验收现稿保持原值"}]},"acceptedContinuity":{"unit":{"id":"accepted-predecessor","exitState":"保留完整连续态"}},"nextBoundary":{"unit":"后续边界不可截"},"facts":[{"claim":"CHARACTER_CLAIM","currentApplicability":"UNKNOWN","sourceRefs":[rows[31]["id"]]}]});
        input["contextRows"][0]["unknownExtension"] =
            json!({"currentApplicability":"UNKNOWN","scope":"HISTORICAL_EVIDENCE_ONLY"});
        for stage in ["storyboardPlan", "storyboardCreate", "storyboardFinish"] {
            let mut request = fixture();
            request.stage = stage.into();
            request.input = input.clone();
            request.input["finishMode"] = json!("delta");
            let frozen = serde_json::to_vec(&request).unwrap();
            let canonical = crate::text_executor::transport_payload_characters(
                &request,
                &request.input,
                &request.schema,
            );
            assert!(canonical > STORYBOARD_TRANSPORT_LIMIT);
            let wire = compile_storyboard_transport(&request).unwrap().unwrap();
            if stage == "storyboardFinish" {
                let legacy = compile_legacy_finish_grouping_transport(&request).unwrap();
                let legacy_characters = legacy
                    .as_ref()
                    .map(|legacy| {
                        assert_eq!(legacy.input["sourceRows"], request.input["sourceRows"]);
                        assert_eq!(legacy.input["contextRows"], request.input["contextRows"]);
                        crate::text_executor::transport_payload_characters(
                            &request,
                            &legacy.input,
                            &legacy.schema,
                        )
                    })
                    .unwrap_or(canonical);
                assert!(
                    legacy_characters > STORYBOARD_TRANSPORT_LIMIT,
                    "legacy grouping cannot gain the new row-table capacity"
                );
            } else {
                assert!(compile_legacy_finish_grouping_transport(&request)
                    .unwrap()
                    .is_none());
            }
            assert!(wire.input["sourceRows"].get("$sourceRows").is_some());
            assert!(wire.input["contextRows"].get("$sourceRows").is_some());
            assert_eq!(
                wire.input["contextRows"]["$sourceRows"]["values"][0],
                request.input["contextRows"][0],
                "unknown metadata remains a complete literal row"
            );
            assert_eq!(
                wire.input["sourceRows"]["$sourceRows"]["values"][0][3],
                rows[0]["text"]
            );
            assert_eq!(
                wire.input["sourceRows"]["$sourceRows"]["values"][0][7][1],
                rows[0]["dialogue"]["speaker"]
            );
            assert_eq!(
                wire.input["sourceRows"]["$sourceRows"]["values"][0][7][2],
                rows[0]["dialogue"]["channel"]
            );
            for field in [
                "previousDraft",
                "acceptedContinuity",
                "nextBoundary",
                "assets",
            ] {
                assert_eq!(wire.input[field], request.input[field]);
            }
            let mut projected = wire.input.clone();
            projected
                .as_object_mut()
                .unwrap()
                .remove("losslessInputRule");
            assert_eq!(expand(&projected, &projected).unwrap(), request.input);
            assert_eq!(
                expand_storyboard_schema(&wire.schema).unwrap(),
                request.schema
            );
            assert_eq!(
                storyboard_transport_rule(&request, &wire.input),
                Some(STORYBOARD_LOSSLESS_INPUT_RULE)
            );
            assert_eq!(serde_json::to_vec(&request).unwrap(), frozen);
            assert!(
                crate::text_executor::transport_payload_characters(
                    &request,
                    &wire.input,
                    &wire.schema
                ) < STORYBOARD_TRANSPORT_LIMIT
            );
        }
    }

    #[test]
    fn row_table_invalid_columns_ranges_and_unknown_fields_never_get_guessed() {
        let source =
            super::super::prepare_source("第一集\n场景：门口\n甲：原话🙂（抬手）后半句。\n")
                .unwrap();
        let row = serde_json::to_value(source.rows.iter().find(|r| r.dialogue.is_some()).unwrap())
            .unwrap();
        let mut table =
            json!({"$sourceRows":{"columns":row_columns(),"values":[encode_row(&row)]}});
        assert_eq!(decode_row_table(&table).unwrap(), json!([row]));
        table["$sourceRows"]["columns"]["row"][0] = json!("wrong-id-field");
        assert!(decode_row_table(&table).is_err());
        let mut invalid = row.clone();
        invalid["dialogue"]["spokenSegments"][0]["text"] = json!("修改原话");
        assert_eq!(encode_row(&invalid), invalid);
        let mut unknown = row.clone();
        unknown["dialogue"]["extra"] = json!("未知但必要的字段保留");
        assert_eq!(encode_row(&unknown), unknown);
        let mut table =
            json!({"$sourceRows":{"columns":row_columns(),"values":[encode_row(&row)]}});
        table["$sourceRows"]["values"][0][7][5][0][1][0] = json!(999999);
        assert!(decode_row_table(&table).is_err());
        for stage in ["storyboardPlan", "storyboardCreate"] {
            let mut request = fixture();
            request.stage = stage.into();
            request.input["ordinaryData"] =
                json!({"$sourceRows":{"columns":"not-a-transport","values":[]}});
            assert!(compile_storyboard_transport(&request).unwrap().is_none());
            let mut wire = request.input.clone();
            wire["losslessInputRule"] = json!(STORYBOARD_LOSSLESS_INPUT_RULE);
            assert!(storyboard_transport_rule(&request, &wire).is_none());
        }
    }

    #[test]
    fn row_span_and_hash_recipes_read_exact_decoded_catalogs_without_scope_expansion() {
        let script = format!(
            "第一集\n场景：门口\n{}",
            (0..20)
                .map(|i| format!("甲：原句{i}🙂。\n"))
                .collect::<String>()
        );
        let source = super::super::prepare_source(&script).unwrap();
        let rows: Vec<Value> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body")
            .map(|r| serde_json::to_value(r).unwrap())
            .collect();
        let mut canonical = json!({"sourceRows":rows,"contextRows":[],"plan":{}});
        canonical["plan"]["sourceFingerprints"] = source_fingerprints(&canonical).unwrap();
        let mut projected = canonical.clone();
        projected["plan"]["sourceFingerprints"] = fingerprint_projection(&canonical).unwrap();
        compact_source_rows(&mut projected, &canonical);
        assert_eq!(expand(&projected, &projected).unwrap(), canonical);
        let recipe = json!({"$rowIds":[["sourceRows",5,6],["sourceRows",5,5]]});
        assert_eq!(
            expand(&recipe, &projected).unwrap(),
            json!([rows[5]["id"], rows[6]["id"], rows[5]["id"]])
        );
        let table = projected["sourceRows"].clone();
        projected["sourceRows"]["$sourceRows"]["values"][0][0] = rows[1]["id"].clone();
        assert!(source_fingerprints(&projected).is_err());
        projected["sourceRows"] = table;
        assert_eq!(
            source_fingerprints(&projected).unwrap(),
            canonical["plan"]["sourceFingerprints"]
        );
    }

    fn dispatch_fixture() -> TextRequest {
        let mut script = "第一集\n场景：同一处\n".to_string();
        for index in 0..64 {
            script.push_str(&format!(
                "甲OS：第{index}句原话🙂（压低声音）还要逐字保留。\n"
            ));
        }
        let source = super::super::prepare_source(&script).unwrap();
        let rows: Vec<Value> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .map(|row| serde_json::to_value(row).unwrap())
            .collect();
        let package = json!({"id":"original-package","sourceRefs":[rows[0]["id"]],"emotionContract":{"change":"原情绪变化完整保留。".repeat(30)}});
        let fact = json!({"sourceRefs":[rows[0]["id"]],"evidence":"原事实与适用范围不得改变。".repeat(20)});
        let continuity = json!({"unit":{"id":"accepted-predecessor","exitState":"已采用连续态完整保留。".repeat(30)}});
        let mut request = fixture();
        request.input = json!({"finishMode":"delta","sourceRows":rows[..8],"contextRows":rows[8..],"plan":{"workPackages":[package.clone()],"entryFacts":[fact.clone()],"fantasyCandidates":[],"acceptedContinuity":continuity.clone()},"workPackages":[package],"entryFacts":[fact.clone()],"selectedFantasy":[],"facts":[fact],"acceptedContinuity":continuity,"previousDraft":{"units":[{"id":"accepted-unit","prompt":"完整已采用现稿不改。"}]},"nextBoundary":{"unit":"后续边界保持原值"},"assets":[{"id":"person:state","variantId":"state","version":9,"sourceRefs":[rows[0]["id"]]}]});
        request
    }

    #[test]
    fn dispatch_selector_compacts_short_three_roles_without_changing_capacity_or_canonical() {
        for (stage, skill) in [
            ("storyboardPlan", "episode-emotion-planner"),
            ("storyboardCreate", "unit-performance-director"),
            ("storyboardFinish", "episode-finishing-editor"),
        ] {
            let mut request = dispatch_fixture();
            request.stage = stage.into();
            request.skill = skill.into();
            let frozen = serde_json::to_vec(&request).unwrap();
            let hash = digest(&request.input.to_string());
            let baseline = crate::text_executor::transport_payload_characters(
                &request,
                &request.input,
                &request.schema,
            );
            assert!(baseline < STORYBOARD_TRANSPORT_LIMIT);
            assert!(compile_storyboard_transport(&request).unwrap().is_none());
            assert!(compile_legacy_finish_grouping_transport(&request)
                .unwrap()
                .is_none());
            let wire = compile_storyboard_dispatch_transport(&request)
                .unwrap()
                .unwrap();
            let actual = crate::text_executor::transport_payload_characters(
                &request,
                &wire.input,
                &wire.schema,
            );
            assert!(baseline - actual >= 2_048);
            assert!(wire.input["sourceRows"].get("$sourceRows").is_some());
            assert!(wire.input["contextRows"].get("$sourceRows").is_some());
            assert_eq!(
                wire.input["workPackages"],
                json!({"$same":"#/plan/workPackages"})
            );
            assert_eq!(wire.schema, request.schema);
            assert_eq!(expand_storyboard_input(&wire.input).unwrap(), request.input);
            assert_eq!(
                storyboard_transport_rule(&request, &wire.input),
                Some(shared_row_rule().as_str())
            );
            for field in [
                "previousDraft",
                "acceptedContinuity",
                "nextBoundary",
                "assets",
                "facts",
            ] {
                assert_eq!(wire.input[field], request.input[field]);
            }
            assert_eq!(serde_json::to_vec(&request).unwrap(), frozen);
            assert_eq!(digest(&request.input.to_string()), hash);
            assert!(compile_storyboard_transport(&request).unwrap().is_none());
            assert!(compile_legacy_finish_grouping_transport(&request)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn dispatch_selector_requires_2048_utf16_net_gain_in_complete_envelopes() {
        let request_for = |padding: usize| {
            let continuity = json!({"tail":"a".repeat(padding)});
            let mut request = fixture();
            request.stage = "storyboardPlan".into();
            request.input = json!({"acceptedContinuity":continuity.clone(),"plan":{"acceptedContinuity":continuity},"sourceRows":[],"contextRows":[]});
            request
        };
        let measure_gain = |request: &TextRequest| {
            let mut input = request.input.clone();
            input["plan"]["acceptedContinuity"] = json!({"$same":"#/acceptedContinuity"});
            input["losslessInputRule"] = json!(shared_row_rule());
            crate::text_executor::transport_payload_characters(
                request,
                &request.input,
                &request.schema,
            ) as i64
                - crate::text_executor::transport_payload_characters(
                    request,
                    &input,
                    &request.schema,
                ) as i64
        };
        let gain_at_1000 = measure_gain(&request_for(1_000));
        for expected_gain in [2_047_i64, 2_048] {
            let request = request_for((1_000 + expected_gain - gain_at_1000) as usize);
            assert_eq!(measure_gain(&request), expected_gain);
            let frozen = request.clone();
            let wire = compile_storyboard_dispatch_transport(&request).unwrap();
            assert_eq!(wire.is_some(), expected_gain == 2_048);
            if let Some(wire) = wire {
                assert_eq!(expand_storyboard_input(&wire.input).unwrap(), request.input);
                assert_eq!(wire.schema, request.schema);
            }
            assert_eq!(
                serde_json::to_value(request).unwrap(),
                serde_json::to_value(frozen).unwrap()
            );
        }
    }

    #[test]
    fn dispatch_selector_rejects_source_notation_and_preserves_old_wire_without_net_gain() {
        for marker in [
            json!({"$same":"#/plan/workPackages"}),
            json!({"$rowIds":[["sourceRows",0,0]]}),
            json!({"$sourceFingerprints":"SHA256_UTF8_ROWS"}),
            json!({"$sourceRows":{"columns":[],"values":[]}}),
        ] {
            let mut request = dispatch_fixture();
            request.input["untrustedEvidence"] = marker;
            let frozen = serde_json::to_value(&request).unwrap();
            assert!(compile_storyboard_dispatch_transport(&request)
                .unwrap()
                .is_none());
            assert_eq!(serde_json::to_value(&request).unwrap(), frozen);
        }
        let mut request = dispatch_fixture();
        request.input["losslessInputRule"] = json!(STORYBOARD_LOSSLESS_INPUT_RULE);
        assert!(compile_storyboard_dispatch_transport(&request)
            .unwrap()
            .is_none());
        request
            .input
            .as_object_mut()
            .unwrap()
            .remove("losslessInputRule");
        request.input["finishMode"] = json!("full");
        assert!(compile_storyboard_dispatch_transport(&request)
            .unwrap()
            .is_none());
        let request = fixture();
        let baseline = compile_storyboard_transport(&request).unwrap().unwrap();
        let wire = compile_storyboard_dispatch_transport(&request)
            .unwrap()
            .unwrap();
        assert_eq!(wire.profile, baseline.profile);
        assert_eq!(wire.input, baseline.input);
        assert_eq!(wire.schema, baseline.schema);
    }

    #[test]
    fn dispatch_selector_keeps_unknown_rows_literal_and_rejects_tampered_authority() {
        let mut request = dispatch_fixture();
        request.input["contextRows"][0]["unknownExtension"] =
            json!({"scope":"HISTORICAL_EVIDENCE_ONLY"});
        request.input["contextRows"][1]["dialogue"]["quote"] = json!("与锁定区间不符的资料原值");
        let wire = compile_storyboard_dispatch_transport(&request)
            .unwrap()
            .unwrap();
        for index in [0, 1] {
            assert_eq!(
                wire.input["contextRows"]["$sourceRows"]["values"][index],
                request.input["contextRows"][index]
            );
        }
        assert_eq!(expand_storyboard_input(&wire.input).unwrap(), request.input);
        let mut tampered = wire.input;
        tampered["sourceRows"]["$sourceRows"]["values"][0][3] = json!("擅改原文");
        assert!(storyboard_transport_rule(&request, &tampered).is_none());
        assert!(expand_storyboard_input(&tampered).is_err());
    }

    #[test]
    #[ignore = "explicit saved latest three-role requests; read-only, no channels or production writes"]
    fn saved_latest_three_role_dispatch_selector_measures_actual_rust_envelope_gain() {
        let path = std::env::var("DIRECTOR_STAGE3_READONLY_DB")
            .expect("explicit readonly DB fixture required");
        let workflow = std::env::var("DIRECTOR_STAGE3_READONLY_WORKFLOW")
            .expect("explicit workflow fixture required");
        let conn =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        conn.execute_batch("PRAGMA query_only=ON; PRAGMA busy_timeout=1500;")
            .unwrap();
        for stage in ["storyboardPlan", "storyboardCreate", "storyboardFinish"] {
            let raw: String = conn.query_row("SELECT j.snapshot FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.stage=?2 AND w.state='accepted' AND j.state='completed' AND json_extract(j.receipt,'$.validation')='accepted' ORDER BY w.updated_at DESC LIMIT 1", rusqlite::params![workflow, stage], |row| row.get(0)).unwrap();
            let request: TextRequest = serde_json::from_str(&raw).unwrap();
            let frozen = serde_json::to_vec(&request).unwrap();
            let baseline = compile_storyboard_transport(&request).unwrap();
            let (input, schema) = baseline
                .as_ref()
                .map(|wire| (&wire.input, &wire.schema))
                .unwrap_or((&request.input, &request.schema));
            let canonical = crate::text_executor::transport_payload_characters(
                &request,
                &request.input,
                &request.schema,
            );
            let current =
                crate::text_executor::transport_payload_characters(&request, input, schema);
            let wire = compile_storyboard_dispatch_transport(&request)
                .unwrap()
                .unwrap();
            let actual = crate::text_executor::transport_payload_characters(
                &request,
                &wire.input,
                &wire.schema,
            );
            assert_eq!(expand_storyboard_input(&wire.input).unwrap(), request.input);
            assert_eq!(wire.schema, request.schema);
            assert_eq!(serde_json::to_vec(&request).unwrap(), frozen);
            assert!(current
                .checked_sub(actual)
                .is_some_and(|gain| gain >= 2_048));
            assert!(actual <= STORYBOARD_TRANSPORT_LIMIT);
            println!("{stage} canonicalCharacters={canonical} currentCapacityWireCharacters={current} dispatchWireCharacters={actual} netGain={} canonicalEqual=true schemaEqual=true modelCalls=0", current - actual);
        }
    }

    #[test]
    #[ignore = "explicit env-bound saved requests; read-only, no model or production writes"]
    fn saved_three_role_requests_prove_exact_transport_and_actual_envelope_budget() {
        let path = std::env::var("DIRECTOR_STAGE3_READONLY_DB")
            .expect("explicit readonly DB fixture required");
        let workflow = std::env::var("DIRECTOR_STAGE3_READONLY_WORKFLOW")
            .expect("explicit workflow fixture required");
        let conn =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        conn.execute_batch("PRAGMA query_only=ON;").unwrap();
        let mut statement = conn.prepare("SELECT DISTINCT j.snapshot FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE w.workflow_id=?1 AND j.kind='text' AND j.state='completed' AND json_extract(j.receipt,'$.validation')='accepted' ORDER BY j.created_at").unwrap();
        let snapshots: Vec<String> = statement
            .query_map([workflow], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for snapshot in snapshots {
            let request: TextRequest = serde_json::from_str(&snapshot).unwrap();
            if !supported_request(&request) {
                continue;
            }
            let before = serde_json::to_vec(&request).unwrap();
            let canonical = crate::text_executor::transport_payload_characters(
                &request,
                &request.input,
                &request.schema,
            );
            let wire = compile_storyboard_transport(&request).unwrap();
            let actual = if let Some(wire) = wire {
                let mut input = wire.input.clone();
                input.as_object_mut().unwrap().remove("losslessInputRule");
                assert_eq!(expand(&input, &input).unwrap(), request.input);
                assert_eq!(
                    expand_storyboard_schema(&wire.schema).unwrap(),
                    request.schema
                );
                assert_eq!(
                    storyboard_transport_rule(&request, &wire.input),
                    Some(STORYBOARD_LOSSLESS_INPUT_RULE)
                );
                crate::text_executor::transport_payload_characters(
                    &request,
                    &wire.input,
                    &wire.schema,
                )
            } else {
                canonical
            };
            assert_eq!(serde_json::to_vec(&request).unwrap(), before);
            assert!(
                actual <= STORYBOARD_TRANSPORT_LIMIT,
                "{}: canonical={canonical} actual={actual}",
                request.stage
            );
            *counts.entry(request.stage.clone()).or_default() += 1;
            println!("{} canonicalCharacters={canonical} wireCharacters={actual} canonicalEqual=true newModelCalls=0",request.stage);
        }
        assert!(counts.get("storyboardPlan").copied().unwrap_or(0) >= 2);
        assert!(counts.get("storyboardCreate").copied().unwrap_or(0) >= 5);
        println!(
            "savedAcceptedRoleCounts={counts:?}; readonly=true; futureEpisodesNotGenerated=true"
        );
    }
}
