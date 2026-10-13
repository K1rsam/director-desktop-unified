//! Versioned performance interpretation; immutable source metadata is never reclassified.
use super::{digest, TextRequest};
use serde_json::{json, Value};

pub const PROFILE_FIELD: &str = "storyboardSoundEventProfile";
pub const PROFILE: &str = "source-bound-sound-events/v1";
pub const PRESENTATION: &str = "声音事件";
const PRESENTATIONS: &[&str] = &["画内", "画外", "VO", "OS", "系统音", "画外转画内", "画内转画外", PRESENTATION];
const CONTRACT: &str = "声音事件合同 source-bound-sound-events/v1：原文的vo/VO等标签不自动等于音效，原句中提及响声也不等于声音事件。主创和整编依据完整源句及当拍声画语义，只有无需人声朗读的实际响声、环境声或声音变化才选择dialogueParts.presentation=声音事件；真实对白、旁白、OS、系统语音仍使用原有呈现方式。保留全部lineId、绝对UTF16 quoteStart/quoteEnd及时间窗；Rust仍逐字保全text/speaker/channel和完整来源覆盖，不能把人物台词或旁白改成声音事件来删词、缩时或绕过保真。原源字段是追溯信息，声音事件交付为非人声演绎，不读出描述文字。整编须逐项核对类型、实际sound、源事实和声音发生时刻。新字段是本次冻结演绎合同，不改旧请求或旧采用稿；无此合同的旧稿保持原解释。声画定位单列明确事件，不将声音描述字数当人声语速；无对白不等于无声或空等。";

pub fn enabled(value: &Value) -> Result<bool, String> {
    match value.get(PROFILE_FIELD) {
        None => Ok(false),
        Some(Value::String(version)) if version == PROFILE => Ok(true),
        _ => Err("声音事件合同版本未知；保留原请求与回包，未改变解释".into()),
    }
}

/// Only called after the engine's existing-workflow resume boundary.
pub fn freeze_new(input: &mut Value) -> Result<(), String> {
    enabled(input)?;
    if !input.is_object() || !super::stage3_pack::uses_profile(input)? {
        return Err("声音事件合同须绑定冻结第三阶段生产规则".into());
    }
    input[PROFILE_FIELD] = json!(PROFILE);
    Ok(())
}

pub(super) fn inherit(target: &mut Value, frozen: &Value) -> Result<(), String> {
    let target_enabled = enabled(target)?;
    let frozen_enabled = enabled(frozen)?;
    if target_enabled && !frozen_enabled {
        return Err("声音事件合同不能添加到旧冻结请求".into());
    }
    if frozen_enabled { target[PROFILE_FIELD] = json!(PROFILE); }
    Ok(())
}

fn extend_schema(schema: &mut Value) {
    if let Some(parts) = schema.pointer_mut("/properties/dialogueParts/items/properties/presentation/enum")
        .and_then(Value::as_array_mut)
    {
        if !parts.contains(&json!(PRESENTATION)) { parts.push(json!(PRESENTATION)); }
    }
    match schema {
        Value::Object(map) => { for item in map.values_mut() { extend_schema(item); } }
        Value::Array(items) => { for item in items { extend_schema(item); } }
        _ => (),
    }
}

pub(super) fn prepare_request(request: &mut TextRequest) -> Result<(), String> {
    if !enabled(&request.input)? { return Ok(()); }
    if !super::stage3_pack::uses_profile(&request.input)?
        || !["storyboardPlan", "storyboardCreate", "storyboardFinish", "storyboardRefine"].contains(&request.stage.as_str())
    { return Err("声音事件请求缺少匹配的冻结第三阶段岗位".into()); }
    extend_schema(&mut request.schema);
    let runtime = request.rule_snapshot.get_mut("runtimeContract")
        .filter(|value| value.is_object()).ok_or("声音事件缺少可信执行合同")?;
    match runtime.get(PROFILE_FIELD) {
        Some(version) if version == PROFILE => {
            if !runtime["content"].as_str().is_some_and(|value| value.contains(CONTRACT)) {
                return Err("声音事件执行合同正文与版本不匹配".into());
            }
        }
        Some(_) => return Err("声音事件执行合同版本不匹配".into()),
        None => {
            let content = format!("{}\n\n{CONTRACT}", runtime["content"].as_str().ok_or("缺少执行合同正文")?);
            runtime["content"] = json!(content);
            runtime[PROFILE_FIELD] = json!(PROFILE);
            runtime["sha256"] = json!(digest(&content));
        }
    }
    Ok(())
}

pub(super) fn is_event(unit: &Value, part: &Value) -> bool {
    unit.get(PROFILE_FIELD).and_then(Value::as_str) == Some(PROFILE)
        && part["presentation"].as_str() == Some(PRESENTATION)
}

pub(super) fn validate_unit(unit: &Value) -> Result<(), String> {
    let active = enabled(unit)?;
    for part in unit["shots"].as_array().into_iter().flatten()
        .flat_map(|shot| shot["dialogueParts"].as_array().into_iter().flatten())
    {
        let presentation = part["presentation"].as_str().unwrap_or("");
        if presentation == PRESENTATION && !active {
            return Err("声音事件呈现缺少明确冻结合同，不能静默重解释旧稿".into());
        }
        if active && !PRESENTATIONS.contains(&presentation) {
            return Err("声音事件合同包含未知呈现枚举".into());
        }
    }
    Ok(())
}

/// Pure projection of this role's own output, before source/coverage validation.
/// Does not edit raw receipts or stamp retained units copied from an older base.
pub(super) fn materialize(input: &Value, raw: &Value) -> Result<Value, String> {
    let active = enabled(input)?;
    let mut result = raw.clone();
    fn units(value: &mut Value, active: bool) -> Result<(), String> {
        for unit in value.as_array_mut().ok_or("声音事件投影缺少单元数组")? {
            let supplied = enabled(unit)?;
            if supplied && !active { return Err("回包不能升级旧冻结声音合同".into()); }
            if active { unit[PROFILE_FIELD] = json!(PROFILE); }
            validate_unit(unit)?;
        }
        Ok(())
    }
    if let Some(value) = result.get_mut("units") { units(value, active)?; }
    if let Some(groups) = result.get_mut("replacements").and_then(Value::as_array_mut) {
        for group in groups {
            if let Some(value) = group.get_mut("units") { units(value, active)?; }
        }
    }
    Ok(result)
}

fn seconds(value: f64) -> String {
    let value = format!("{value:.3}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Both exports share the same explicit non-speech label. Caller checks is_event.
pub(super) fn render_event(part: &Value, offset: f64) -> String {
    format!("声音事件：{}–{}秒｜{}（按原文执行声音变化，不作人声朗读）",
        seconds(offset + part["voiceStart"].as_f64().unwrap_or_default()),
        seconds(offset + part["voiceEnd"].as_f64().unwrap_or_default()),
        part["text"].as_str().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(active: bool) -> TextRequest {
        let mut input = json!({});
        super::super::stage3_pack::mark_input(&mut input).unwrap();
        if active { freeze_new(&mut input).unwrap(); }
        TextRequest {stage:"storyboardCreate".into(),skill:"unit-performance-director".into(),model:"test".into(),reasoning:"medium".into(),input,
            schema:json!({"properties":{"units":{"items":{"properties":{"shots":{"items":{"properties":{"dialogueParts":{"items":{"properties":{"presentation":{"enum":["画内","画外","VO"]}}}}}}}}}}}}),
            rule_snapshot:json!({"runtimeContract":{"content":"original contract","sha256":digest("original contract")},"primary":{"content":"original skill"}})}
    }
    fn unit(presentation: &str) -> Value {
        json!({"id":"U","title":"声画","scene":"公园","initialState":"笑声未止","exitState":"笑声停止","unitDirection":"保留原声音事件","lighting":"日光","participants":[],"props":[],"assetRefs":[],"shots":[{"id":"S","duration":2.0,"camera":"固定","picture":"众人看坑","lighting":"日光","sound":"笑声骤停","transition":"切出","dialogueParts":[{"lineId":"L","quoteStart":10,"quoteEnd":19,"speaker":"Vo","channel":"现场","text":"群众笑声戛然而止。","presentation":presentation,"delivery":"声音事件这个自由文本本身无权改分类","voiceStart":0.0,"voiceEnd":0.3}]}]})
    }
    #[test]
    fn old_request_preparation_is_byte_exact_and_new_contract_idempotent() {
        let mut old=request(false); let before=serde_json::to_vec(&old).unwrap();
        prepare_request(&mut old).unwrap(); assert_eq!(serde_json::to_vec(&old).unwrap(),before);
        let mut new=request(true); let primary=new.rule_snapshot["primary"].clone();
        prepare_request(&mut new).unwrap(); let first=serde_json::to_vec(&new).unwrap();
        prepare_request(&mut new).unwrap(); assert_eq!(serde_json::to_vec(&new).unwrap(),first);
        assert_eq!(new.rule_snapshot["primary"],primary);
        assert!(new.schema.to_string().contains(PRESENTATION));
        new.input[PROFILE_FIELD]=json!("unknown"); assert!(prepare_request(&mut new).is_err());
        assert!(inherit(&mut json!({PROFILE_FIELD:PROFILE}),&json!({})).is_err());
    }
    #[test]
    fn typed_output_preserves_source_fields_and_raw_and_rejects_unbound_events() {
        let raw=json!({"units":[unit(PRESENTATION)]}); let saved=raw.clone();
        assert!(materialize(&json!({}),&raw).is_err());
        let projected=materialize(&json!({PROFILE_FIELD:PROFILE}),&raw).unwrap();
        assert_eq!(raw,saved);
        assert_eq!(projected["units"][0]["shots"],raw["units"][0]["shots"]);
        assert_eq!(projected["units"][0][PROFILE_FIELD],PROFILE);
        let delta=json!({"replacements":[{"units":[unit(PRESENTATION)]}],"retainedUnitIds":["old"]});
        let projected=materialize(&json!({PROFILE_FIELD:PROFILE}),&delta).unwrap();
        assert_eq!(projected["retainedUnitIds"],delta["retainedUnitIds"]);
        assert_eq!(projected["replacements"][0]["units"][0][PROFILE_FIELD],PROFILE);
        let mut unknown=unit("这是声音事件"); unknown[PROFILE_FIELD]=json!(PROFILE);
        assert!(validate_unit(&unknown).is_err());
    }
    #[test]
    fn both_exports_distinguish_explicit_event_from_identical_spoken_words() {
        for presentation in ["画内","VO",PRESENTATION] {
            let projected=materialize(&json!({PROFILE_FIELD:PROFILE}),&json!({"units":[unit(presentation)]})).unwrap();
            let new=super::super::storyboard::creative::render_units(&projected,None,"").unwrap();
            let legacy=super::super::storyboard::render_legacy_unit(&projected["units"][0]);
            for output in [&new[0],&legacy] {
                assert_eq!(output.contains("声音事件：0–0.3秒"),presentation==PRESENTATION);
                assert!(output.contains("群众笑声戛然而止。"));
                if presentation==PRESENTATION { assert!(!output.contains("｜Vo（")); }
                else { assert!(output.contains("Vo")); }
            }
        }
        let old=unit("画外"); let a=super::super::storyboard::render_legacy_unit(&old);
        assert!(!a.contains("声音事件："),"delivery wording does not classify speech");
    }
}
