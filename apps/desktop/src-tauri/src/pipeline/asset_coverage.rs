//! Source/asset coverage advice for new workflows only. Never guesses identities
//! or mutates a catalogue, adopted draft, frozen request, or retry budget.
use super::{digest, TextRequest};
use serde_json::{json, Value};

pub(crate) const FIELD: &str = "assetCoverageProfile";
pub(crate) const PROFILE: &str = "source-bound-asset-coverage/v1";

const FOUNDATION: &str = "必要素材闭环核对：在本岗原有生成/终编及有界返修内，按本批真实原文逐场核对稳定个体、持续交互物与当前目录，不另设审查岗或要求用户逐项确认。GROUP只承载真实群体；不能因为角色没有姓名、是路人或同属顾客，就把有独立连续询价/交接/冲突动作、需跨镜辨认的个体并成群像别名。一次背景呼喊或集体应声本身不要求拆人物；依据完整连续事件判断个体身份，原文未证明的不同称呼不能自动合并。工具/物件明确被取起、操作、传递或在后续连续动作中再次使用，且需要稳定辨认/交互一致性时，核对是否已有准确可承载的基线；不能因常见或无专名而漏掉，也不能凭活动关键词补造装备。已登记身份沿原key；缺少必要目录项时仅按当前schema和授权以原文及准确集场补齐，Q1保留正确成果只交真实差异。只有已验证的角色/场景配套能连续承载时才并入，不假设现有图片已包含。明确画外、台词提及、背景装饰、临时动作不强行登记；不把原文100%保留误当成制作目录已齐。APPLY_DECISION/AUTO_INTERPRET/REPAIR仍服从各自变更范围，不借核对扩大修改、虚构身份或关闭未修问题。";

const STORYBOARD: &str = "实际入画素材核对：主创与整编在本岗原有完整创作/有界修订内，把逐镜可见人物、场景和需要稳定辨认或独立交互一致性的道具，与当前提供的正式资产目录逐一核对。已登记且真实入画的实体使用准确assetId+variantId及本单元源依据；不能只在participants/props/镜头正文写名称而漏掉必要assetRefs。目录不是必入画清单：明确画外收持、纯声音、台词提及或前情状态不因出现名称而强绑图片，不靠关键词或同名猜身份。独立连续交易/交接/冲突的个体不能用GROUP群像或群像别名冒充独立基线；只在原文确为群体呈现且目录相符时引用GROUP。目录确实缺少必要实体/形象时，不虚构ID、不借不相关现成资产兜底、不删掉源动作来掩盖缺口，按已有unresolved记录准确单元/源行/实体及缺少的目录条件，保留原稿与已有修订预算。上下文和MATERIAL_SOURCE_ONLY仍不授权当前入画；跨单元持物连续性可核对已有边界，但不能把前单元源行冒充本单元制作覆盖。";

pub(crate) fn enabled(input: &Value) -> Result<bool, String> {
    match input.get(FIELD) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::String(value)) if value == PROFILE => Ok(true),
        _ => Err("素材核对合同版本未知；保留冻结请求，未派发新请求".into()),
    }
}

/// Only the engine's genuinely new workflow branch calls this. Recovery copies
/// the saved marker and never opts old workflows in based on installed code.
pub(crate) fn freeze_new(input: &mut Value) -> Result<(), String> {
    enabled(input)?;
    input[FIELD] = json!(PROFILE);
    Ok(())
}

pub(crate) fn inherit(target: &mut Value, frozen: &Value) -> Result<(), String> {
    if enabled(frozen)? {
        target[FIELD] = json!(PROFILE);
    }
    Ok(())
}

// input_for is an infallible projection; prepare_capacity_batch validates the
// frozen marker before calling it. Absence leaves legacy input byte-equivalent.
pub(crate) fn foundation_input(stage: &str, target: &mut Value, frozen: &Value) {
    if frozen.get(FIELD).and_then(Value::as_str) != Some(PROFILE) {
        return;
    }
    target[FIELD] = json!(PROFILE);
    if !["F2", "Q1"].contains(&stage)
        || ["AUTO_INTERPRET", "APPLY_DECISION", "REPAIR"]
            .contains(&target["delivery"]["phase"].as_str().unwrap_or(""))
    {
        return;
    }
    let prior = target["transportInstructions"].as_str().unwrap_or("");
    if !prior.contains(FOUNDATION) {
        target["transportInstructions"] = json!(format!("{prior}\n\n{FOUNDATION}"));
    }
}

pub(crate) fn storyboard_request(request: &mut TextRequest) -> Result<(), String> {
    if !enabled(&request.input)?
        || !["storyboardCreate", "storyboardFinish", "storyboardRefine"]
            .contains(&request.stage.as_str())
    {
        return Ok(());
    }
    let runtime = request.rule_snapshot.get_mut("runtimeContract")
        .filter(|value| value.is_object())
        .ok_or("素材核对缺少可信运输合同；未派发新请求")?;
    let prior = runtime["content"].as_str()
        .ok_or("素材核对运输合同正文无效；未派发新请求")?;
    let content = if prior.contains(STORYBOARD) { prior.to_owned() }
        else { format!("{prior}\n\n{STORYBOARD}") };
    runtime[FIELD] = json!(PROFILE);
    runtime["sha256"] = json!(digest(&content));
    runtime["content"] = json!(content);
    Ok(())
}

#[cfg(test)]
mod asset_coverage_tests {
    use super::*;

    fn request(stage: &str, profile: bool) -> TextRequest {
        let skill = match stage {
            "storyboardPlan" => "episode-emotion-planner",
            "storyboardCreate" => "unit-performance-director",
            "storyboardFinish" => "episode-finishing-editor",
            _ => "director-refinement",
        };
        let mut input = json!({"sourceRows":[],"assets":[{"id":"known","variantId":"base"}],
            "draft":{"units":[{"id":"u1","props":["画外手机"],"assetRefs":[]}]}});
        super::super::stage3_pack::mark_input(&mut input).unwrap();
        if profile { freeze_new(&mut input).unwrap(); }
        let mut rules = super::super::stage3_pack::snapshot(skill, &["storyboard-common"]).unwrap();
        rules["runtimeContract"] = super::super::stage3_pack::runtime_contract(stage, false).unwrap();
        TextRequest { stage:stage.into(),skill:skill.into(),model:"offline".into(),reasoning:"medium".into(),
            input,schema:json!({"type":"object"}),rule_snapshot:rules }
    }

    #[test]
    fn asset_coverage_old_frozen_requests_are_exactly_unchanged() {
        for stage in ["storyboardPlan","storyboardCreate","storyboardFinish","storyboardRefine"] {
            let mut original = request(stage, false);
            // The existing rhythm adapter has its own frozen opt-in. This test
            // compares the complete request before/after our added contract.
            original = super::super::stage3_pack::prepare_request(original).unwrap();
            let bytes = serde_json::to_vec(&original).unwrap();
            storyboard_request(&mut original).unwrap();
            assert_eq!(serde_json::to_vec(&original).unwrap(),bytes);
            let mut target = json!({"alreadyFrozen":true});
            inherit(&mut target,&original.input).unwrap();
            assert_eq!(target,json!({"alreadyFrozen":true}));
        }
    }

    #[test]
    fn asset_coverage_new_saved_profile_reopens_and_all_roles_inherit_without_asset_mutation() {
        for stage in ["storyboardPlan","storyboardCreate","storyboardFinish","storyboardRefine"] {
            let original = request(stage,true);
            let frozen: Value=serde_json::from_slice(&serde_json::to_vec(&original.input).unwrap()).unwrap();
            let mut selected = original.input.clone();
            selected.as_object_mut().unwrap().remove(FIELD);
            super::super::stage3_pack::attach_profile(&mut selected,&frozen).unwrap();
            assert_eq!(selected[FIELD],PROFILE);
            let mut selected_request = original.clone();
            selected_request.input=selected;
            let prepared=super::super::stage3_pack::prepare_request(selected_request).unwrap();
            assert_eq!(prepared.input["assets"],original.input["assets"]);
            assert_eq!(prepared.input["draft"],original.input["draft"]);
            assert_eq!(prepared.input["sourceRows"],original.input["sourceRows"]);
            let runtime=&prepared.rule_snapshot["runtimeContract"];
            assert_eq!(runtime["sha256"],digest(runtime["content"].as_str().unwrap()));
            assert_eq!(runtime["content"].as_str().unwrap().matches(STORYBOARD).count(),usize::from(stage!="storyboardPlan"));
            let once=serde_json::to_vec(&prepared).unwrap();
            let twice=super::super::stage3_pack::prepare_request(prepared).unwrap();
            assert_eq!(serde_json::to_vec(&twice).unwrap(),once,"reopen must not duplicate adapters");
        }
    }

    #[test]
    fn asset_coverage_unknown_or_unbound_profile_is_rejected_before_request() {
        let mut request=request("storyboardCreate",false);
        request.input[FIELD]=json!("future-unknown");
        let frozen=request.input.clone();
        assert!(super::super::stage3_pack::prepare_request(request.clone()).is_err());
        assert!(freeze_new(&mut request.input).is_err());
        assert_eq!(request.input,frozen);
        request.input[FIELD]=json!(PROFILE);
        request.input.as_object_mut().unwrap().remove(super::super::stage3_pack::PROFILE_FIELD);
        assert!(super::super::stage3_pack::prepare_request(request).is_err());
    }
}
