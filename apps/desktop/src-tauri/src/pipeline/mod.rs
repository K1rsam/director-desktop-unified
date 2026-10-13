//! Source-bound production pipeline. Models author decisions; Rust copies and validates evidence.
pub mod asset_visual_facts;
pub(crate) mod asset_coverage;
pub mod asset_design_contract;
pub mod asset_production_context;
mod assets;
pub mod foundation_delivery;
pub(crate) mod foundation_wire;
mod scene_edits;
mod source;
mod storyboard;
pub(crate) mod storyboard_sound_events;
pub mod stage3_pack;
pub(crate) mod storyboard_refinement;
pub use storyboard_refinement::{run_storyboard_refinement, adopt_storyboard_refinement};
pub(crate) mod storyboard_dependencies;
mod storyboard_transport;
pub mod asset_use_binding;

pub use assets::{
    automatic_design_required, image_production_policy, resolve_asset_dependency, run_asset_design, validate_asset_visual_gate,
};
pub(crate) use assets::{freeze_design_evidence_profile, design_evidence_binding, DESIGN_EVIDENCE_PROFILE_FIELD};
use async_trait::async_trait;
pub use scene_edits::execute_foundation_scene_edits;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use source::*;
use std::collections::{HashMap, HashSet};
pub(crate) use storyboard::{finish_base_hash, finish_base_input_projection, validate_episode_delivery, CONTEXT_INPUT_PROFILE_FIELD, CONTEXT_INPUT_PROFILE};
pub(crate) use storyboard::planner_source_order_replay_result;
#[cfg(test)]
pub(crate) fn scope_repair_delta_schema_for_test(full: &Value, base: &Value) -> Value {
    storyboard::finish_delta_schema(full, base)
}
pub use storyboard::{
    compile_reference_template, freeze_scene_input_profile, render_legacy_unit, run_storyboard, validate_storyboard,
};
pub(crate) use storyboard_transport::{
    compile_legacy_finish_grouping_transport, compile_storyboard_dispatch_transport,
    compile_storyboard_transport, storyboard_transport_rule, CompiledStoryboardTransport,
    STORYBOARD_LOSSLESS_INPUT_RULE, STORYBOARD_TRANSPORT_LIMIT,
    attach_repetition_transport_profile, REPETITION_TRANSPORT_PROFILE_FIELD, REPETITION_TRANSPORT_PROFILE,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRequest {
    pub stage: String,
    pub skill: String,
    pub model: String,
    pub reasoning: String,
    pub input: Value,
    pub schema: Value,
    pub rule_snapshot: Value,
}

#[async_trait]
pub trait TextExecutor: Send + Sync {
    async fn execute(&self, request: TextRequest) -> Result<Value, String>;
    /// Recover only a proven stopped text transport. Default executors never retry.
    async fn prepare_transport_request(&self, request: TextRequest) -> Result<TextRequest, String> {
        Ok(request)
    }
    fn input_transport_characters(&self, request: &TextRequest) -> Result<usize, String> {
        let wire = compile_storyboard_dispatch_transport(request)?;
        let (input, schema) = wire
            .as_ref()
            .map(|wire| (&wire.input, &wire.schema))
            .unwrap_or((&request.input, &request.schema));
        Ok(crate::text_executor::transport_payload_characters(
            request, input, schema,
        ))
    }
    /// A new editor may consume the parent's frozen production cards. A saved
    /// executor must preserve an already-dispatched editor's original request.
    async fn prepare_finish_asset_projection(
        &self,
        _legacy: TextRequest,
        projected: TextRequest,
    ) -> Result<TextRequest, String> {
        Ok(projected)
    }
    /// Read-only, source-bound evidence for the one legacy A15 attempt whose
    /// accepted result omitted already-adopted production constraints.
    async fn visual_fact_repair_context(
        &self,
        _source_sha: &str,
        _packet_hash: &str,
        _original_domain_hash: &str,
        _original_raw_hash: &str,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }
    /// Recover the original identity selection of a legacy accepted A15 packet.
    /// Read-only evidence; the domain must still rebuild its hash and facts.
    async fn visual_fact_packet_identities(&self, _source_sha: &str, _packet_hash: &str) -> Result<Option<Vec<Value>>, String> {
        Ok(None)
    }
    async fn accept(&self, _request: &TextRequest, _value: &Value) -> Result<(), String> {
        Ok(())
    }
    async fn reject(
        &self,
        _request: &TextRequest,
        _value: &Value,
        _reason: &str,
    ) -> Result<(), String> {
        Ok(())
    }
    /// Program-only checkpoint after a whole episode has passed the original
    /// source/dialogue/identity contracts. This is never a model request.
    async fn publish_episode(&self, _output: &Value, _input: &Value) -> Result<(), String> {
        Ok(())
    }
    async fn continuity_changed(&self, _evidence: &Value) -> Result<(), String> {
        Ok(())
    }
    /// Select a new node only when an adopted upstream repair changed the
    /// continuity of an existing author. Identical requests retain their cache.
    async fn author_chain_revision(&self, _request: &TextRequest, _repair: &Value) -> Result<Option<Value>, String> {
        Ok(None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleSnapshot {
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub content: String,
}

pub fn rule_snapshots() -> Vec<RuleSnapshot> {
    [
        (
            "screenplay-fidelity",
            include_str!("../../../rules/screenplay-fidelity/SKILL.md"),
        ),
        (
            "story-foundation",
            include_str!("../../../rules/story-foundation/SKILL.md"),
        ),
        (
            "foundation-asset-catalog",
            include_str!("../../../rules/foundation-asset-catalog/SKILL.md"),
        ),
        (
            "foundation-finishing-editor",
            include_str!("../../../rules/foundation-finishing-editor/SKILL.md"),
        ),
        (
            "asset-art-director",
            include_str!("../../../rules/asset-art-director/SKILL.md"),
        ),
        (
            "asset-visual-facts-extractor",
            include_str!("../../../rules/asset-visual-facts-extractor/SKILL.md"),
        ),
        (
            "asset-image-production",
            include_str!("../../../rules/asset-image-production/SKILL.md"),
        ),
        (
            "episode-emotion-planner",
            include_str!("../../../rules/episode-emotion-planner/SKILL.md"),
        ),
        (
            "unit-performance-director",
            include_str!("../../../rules/unit-performance-director/SKILL.md"),
        ),
        (
            "episode-finishing-editor",
            include_str!("../../../rules/episode-finishing-editor/SKILL.md"),
        ),
        (
            "unit-video-production",
            include_str!("../../../rules/unit-video-production/SKILL.md"),
        ),
        (
            "acting-blocking",
            include_str!("../../../rules/methods/acting-blocking.md"),
        ),
        (
            "camera-emotion",
            include_str!("../../../rules/methods/camera-emotion.md"),
        ),
        (
            "lighting-space",
            include_str!("../../../rules/methods/lighting-space.md"),
        ),
        (
            "sound-rhythm",
            include_str!("../../../rules/methods/sound-rhythm.md"),
        ),
        (
            "fantasy-expression",
            include_str!("../../../rules/methods/fantasy-expression.md"),
        ),
        (
            "storyboard-common",
            include_str!("../../../rules/methods/storyboard-common.md"),
        ),
        (
            "storyboard-source-context",
            include_str!("../../../rules/methods/storyboard-source-context.md"),
        ),
        (
            "storyboard-card-camera-path-repair",
            include_str!("../../../rules/methods/cards/camera-path-repair.md"),
        ),
        (
            "storyboard-card-comic-contrast",
            include_str!("../../../rules/methods/cards/comic-contrast.md"),
        ),
        (
            "storyboard-card-danger-rescue",
            include_str!("../../../rules/methods/cards/danger-rescue.md"),
        ),
        (
            "storyboard-card-editing-integrity",
            include_str!("../../../rules/methods/cards/editing-integrity.md"),
        ),
        (
            "storyboard-card-expressive-emphasis",
            include_str!("../../../rules/methods/cards/expressive-emphasis.md"),
        ),
        (
            "storyboard-card-joy-acceptance",
            include_str!("../../../rules/methods/cards/joy-acceptance.md"),
        ),
        (
            "storyboard-card-long-voice",
            include_str!("../../../rules/methods/cards/long-voice.md"),
        ),
        (
            "storyboard-card-quiet-nonhuman",
            include_str!("../../../rules/methods/cards/quiet-nonhuman.md"),
        ),
        (
            "storyboard-card-scale-silence",
            include_str!("../../../rules/methods/cards/scale-silence.md"),
        ),
        (
            "storyboard-card-spatial-contact",
            include_str!("../../../rules/methods/cards/spatial-contact.md"),
        ),
        (
            "storyboard-card-status-conflict",
            include_str!("../../../rules/methods/cards/status-conflict.md"),
        ),
    ]
    .into_iter()
    .map(|(id, content)| RuleSnapshot {
        id: id.into(),
        version: content
            .lines()
            .find_map(|line| line.trim().strip_prefix("version:").map(str::trim))
            .filter(|version| !version.is_empty())
            .unwrap_or("1.0.0")
            .into(),
        sha256: digest(content),
        content: content.into(),
    })
    .collect()
}

pub fn rule_snapshot(name: &str) -> Result<Value, String> {
    rule_snapshots()
        .into_iter()
        .find(|r| r.id == name)
        .map(|r| json!(r))
        .ok_or_else(|| format!("规则包不存在：{name}"))
}

pub fn list_rules() -> Value {
    json!(rule_snapshots())
}

pub(crate) fn snapshot(skill: &str, methods: &[&str]) -> Result<Value, String> {
    let registry = rule_snapshots();
    let primary = registry
        .iter()
        .find(|r| r.id == skill)
        .ok_or_else(|| format!("规则包不存在：{skill}"))?;
    let cards: Vec<_> = methods
        .iter()
        .map(|name| {
            registry
                .iter()
                .find(|r| r.id == *name)
                .ok_or_else(|| format!("方法卡不存在：{name}"))
        })
        .collect::<Result<_, _>>()?;
    Ok(json!({"primary":primary,"methods":cards}))
}

pub(crate) async fn call_validated(
    executor: &dyn TextExecutor,
    stage: &str,
    skill: &str,
    model: &str,
    reasoning: &str,
    input: Value,
    schema: Value,
    methods: &[&str],
    validate: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<Value, String> {
    if model.trim().is_empty()
        || !["low", "medium", "high", "xhigh", "max", "ultra"].contains(&reasoning)
    {
        return Err("请选择有效文本模型与实际支持的 reasoning 参数".into());
    }
    let request = TextRequest {
        stage: stage.into(),
        skill: skill.into(),
        model: model.into(),
        reasoning: reasoning.into(),
        input,
        schema,
        rule_snapshot: snapshot(skill, methods)?,
    };
    let mut request = executor.prepare_transport_request(request).await?;
    let mut value = match executor.execute(request.clone()).await {
        Ok(value) => value,
        Err(error) => {
            let next = executor.prepare_transport_request(request.clone()).await?;
            if serde_json::to_value(&next).map_err(|e| e.to_string())?
                == serde_json::to_value(&request).map_err(|e| e.to_string())? {
                return Err(error);
            }
            // One additional execution at most. A domain rejection is handled
            // below and can never enter this transport recovery path.
            request = next;
            executor.execute(request.clone()).await?
        }
    };
    let original = value.clone();
    if let Err(reason) = validate(&mut value) {
        executor.reject(&request, &original, &reason).await?;
        return Err(reason);
    }
    executor.accept(&request, &value).await?;
    Ok(value)
}

pub(crate) fn obj(properties: Value) -> Value {
    let keys: Vec<_> = properties.as_object().unwrap().keys().cloned().collect();
    json!({"type":"object","properties":properties,"required":keys,"additionalProperties":false})
}

#[cfg(test)]
mod transport_recovery_runner_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize,Ordering};
    struct Executor { calls:AtomicUsize, accepted:AtomicUsize, rejected:AtomicUsize, mode:u8 }
    #[async_trait]
    impl TextExecutor for Executor {
        async fn prepare_transport_request(&self, mut request:TextRequest)->Result<TextRequest,String> {
            if self.calls.load(Ordering::SeqCst)>0 { request.input["textTransportRetry"]=json!({"offlineBoundProof":true}); }
            Ok(request)
        }
        async fn execute(&self, request:TextRequest)->Result<Value,String> {
            let count=self.calls.fetch_add(1,Ordering::SeqCst);
            if self.mode==2 || count==0 && self.mode==1 {return Err("offline stopped transport".into())}
            if count>0 {assert_eq!(request.input["textTransportRetry"]["offlineBoundProof"],true)}
            Ok(json!({"value":"exact original domain output"}))
        }
        async fn accept(&self,_:&TextRequest,_:&Value)->Result<(),String>{self.accepted.fetch_add(1,Ordering::SeqCst);Ok(())}
        async fn reject(&self,_:&TextRequest,_:&Value,_:&str)->Result<(),String>{self.rejected.fetch_add(1,Ordering::SeqCst);Ok(())}
    }
    #[tokio::test]
    async fn transport_recovery_is_bounded_and_domain_rejection_never_reexecutes() {
        for mode in [0,1,2,3] {
            let executor=Executor{calls:AtomicUsize::new(0),accepted:AtomicUsize::new(0),rejected:AtomicUsize::new(0),mode};
            let result=call_validated(&executor,"storyboardPlan","episode-emotion-planner","offline","medium",json!({}),json!({"type":"object"}),&[],|_|if mode==3{Err("invalid domain".into())}else{Ok(())}).await;
            assert_eq!(executor.calls.load(Ordering::SeqCst),if [1,2].contains(&mode){2}else{1});
            assert_eq!(result.is_ok(),[0,1].contains(&mode));
            assert_eq!(executor.accepted.load(Ordering::SeqCst),usize::from([0,1].contains(&mode)));
            assert_eq!(executor.rejected.load(Ordering::SeqCst),usize::from(mode==3));
        }
    }
}
pub(crate) fn array(items: Value) -> Value {
    json!({"type":"array","items":items})
}
pub(crate) fn string() -> Value {
    json!({"type":"string"})
}
pub(crate) fn choices(values: &[&str]) -> Value {
    json!({"type":"string","enum":values})
}
pub(crate) fn refs(ids: &[String]) -> Value {
    if ids.is_empty() {
        json!({"type":"array","items":{"type":"string"},"maxItems":0})
    } else {
        json!({"type":"array","items":{"type":"string","enum":ids}})
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Applicability {
    pub episode_id: String,
    pub scene_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationScope {
    pub kind: String,
    #[serde(default)]
    pub scenes: Vec<Applicability>,
    #[serde(default)]
    pub reason: Option<String>,
}
impl Default for FoundationScope {
    fn default() -> Self {
        Self {
            kind: "UNRESOLVED".into(),
            scenes: vec![],
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationVariant {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_variant_id: Option<String>,
    pub source_refs: Vec<String>,
    pub applicability: Vec<Applicability>,
    #[serde(default)]
    pub scope: FoundationScope,
    #[serde(default)]
    pub asset_row_id: Option<String>,
    #[serde(default, flatten)]
    pub metadata: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationAsset {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub source_refs: Vec<String>,
    pub variants: Vec<FoundationVariant>,
    #[serde(default, flatten)]
    pub metadata: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationFact {
    pub id: String,
    pub asset_id: String,
    pub variant_id: Option<String>,
    pub attribute: String,
    pub value: String,
    pub scope: String,
    pub claim: String,
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub quotes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationEpisode {
    pub episode_id: String,
    pub summary: String,
    pub context: String,
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationIssue {
    pub coordinate: String,
    pub problem: String,
    pub source_refs: Vec<String>,
    pub blocking: bool,
    #[serde(default)]
    pub asset_id: Option<String>,
    #[serde(default)]
    pub variant_id: Option<String>,
    #[serde(default)]
    pub episode_ids: Vec<String>,
    #[serde(default)]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationProposal {
    pub schema_version: String,
    pub source_sha256: String,
    pub overview: String,
    pub direction: String,
    pub episodes: Vec<FoundationEpisode>,
    pub assets: Vec<FoundationAsset>,
    pub facts: Vec<FoundationFact>,
    pub issues: Vec<FoundationIssue>,
    #[serde(default)]
    pub rule_snapshots: Vec<RuleSnapshot>,
    #[serde(default)]
    pub updated_episode_ids: Vec<String>,
}

fn foundation_schema(source: &LockedSource, row_ids: &[String]) -> Value {
    let nullable_id = json!({"type":["string","null"]});
    let scene_ids: Vec<_> = source.scenes.iter().map(|s| s.id.clone()).collect();
    let episode_ids: Vec<_> = source.episodes.iter().map(|e| e.id.clone()).collect();
    let place = obj(
        json!({"episodeId":{"type":"string","enum":episode_ids},"sceneId":{"type":"string","enum":scene_ids}}),
    );
    let scope = obj(
        json!({"kind":choices(&["UNRESOLVED","GLOBAL","SCENES"]),"scenes":array(place.clone()),"reason":{"type":["string","null"]}}),
    );
    obj(json!({
        "schemaVersion":choices(&["foundation/v1"]),"sourceSha256":{"type":"string","enum":[source.sha256]},
        "overview":string(),"direction":string(),
        "episodes":array(obj(json!({"episodeId":{"type":"string","enum":episode_ids},"summary":string(),"context":string(),"sourceRefs":refs(row_ids)}))),
        "assets":array(obj(json!({"id":string(),"name":string(),"kind":choices(&["CH","GROUP","BEAST","LOC","PROP"]),"aliases":array(string()),"description":string(),"sourceRefs":refs(row_ids),"variants":array(obj(json!({"id":string(),"name":string(),"description":string(),"sourceVariantId":nullable_id,"sourceRefs":refs(row_ids),"applicability":array(place),"scope":scope})))}))),
        "facts":array(obj(json!({"id":string(),"assetId":string(),"variantId":nullable_id,"attribute":string(),"value":string(),"scope":choices(&["STABLE","STATE","TRANSITION"]),"claim":choices(&["SOURCE","CHARACTER_CLAIM"]),"sourceRefs":refs(row_ids)}))),
        "issues":array(obj(json!({"coordinate":string(),"problem":string(),"sourceRefs":refs(row_ids),"blocking":{"type":"boolean"},"assetId":nullable_id,"variantId":nullable_id,"episodeIds":refs(&episode_ids)})))
    }))
}

fn effective_scope(variant: &FoundationVariant) -> FoundationScope {
    if variant.scope.kind == "UNRESOLVED" && !variant.applicability.is_empty() {
        FoundationScope {
            kind: "SCENES".into(),
            scenes: variant.applicability.clone(),
            reason: None,
        }
    } else {
        variant.scope.clone()
    }
}

fn check_refs(list: &[String], known: &HashSet<&str>, allow_empty: bool) -> Result<(), String> {
    if !allow_empty && list.is_empty() {
        return Err("源证据引用不能为空".into());
    }
    let mut seen = HashSet::new();
    if list
        .iter()
        .any(|id| !known.contains(id.as_str()) || !seen.insert(id))
    {
        return Err("源引用不存在、越出本次发送范围或重复".into());
    }
    Ok(())
}

// A joint source label denotes several explicitly named speakers, never a new
// canonical identity. This guard is a no-op for v1-v7 and all non-joint rows.
fn validate_joint_speaker_asset_identities(source: &LockedSource, proposal: &FoundationProposal) -> Result<(), String> {
    for q in source.rows.iter().filter_map(|r| r.dialogue.as_ref()).filter(|q| !q.speakers.is_empty()) {
        for asset in &proposal.assets {
            let name = asset.name.split(['|', '｜']).next().unwrap_or("").trim();
            if name == q.speaker || asset.aliases.iter().any(|a| a == &q.speaker) {
                return Err(format!("共同说者「{}」是成员标签，不能作为一个资产或别名；请分别沿用成员身份", q.speaker));
            }
            if q.speakers.iter().filter(|member| name == member.text || asset.aliases.contains(&member.text)).count() > 1 {
                return Err(format!("共同说者「{}」的多个成员被归并为同一资产别名，请分别保留身份", q.speaker));
            }
        }
    }
    Ok(())
}

pub fn validate_foundation(
    source: &LockedSource,
    proposal: &FoundationProposal,
) -> Result<(), String> {
    validate_source(source)?;
    if proposal.schema_version != "foundation/v1" || proposal.source_sha256 != source.sha256 {
        return Err("制作基础版本与锁定源不一致".into());
    }
    validate_joint_speaker_asset_identities(source, proposal)?;
    let known: HashSet<_> = source.rows.iter().map(|r| r.id.as_str()).collect();
    let mut ids = HashSet::new();
    let mut variants = HashMap::new();
    for asset in &proposal.assets {
        if asset.id.trim().is_empty() {
            return Err(format!("资产「{}」缺少稳定身份 ID", asset.name));
        }
        if asset.name.trim().is_empty() {
            return Err(format!("资产 {} 缺少显示名称", asset.id));
        }
        if !["CH", "GROUP", "BEAST", "LOC", "PROP"].contains(&asset.kind.as_str()) {
            return Err(format!(
                "资产「{}」的类型「{}」不受支持",
                asset.name, asset.kind
            ));
        }
        if !ids.insert(asset.id.as_str()) {
            return Err(format!(
                "资产「{}」重复使用身份 ID {}，需明确归属",
                asset.name, asset.id
            ));
        }
        if asset.variants.is_empty() {
            return Err("资产缺少基础形象版本，不能进入图片生产目录".into());
        }
        let catalog_scope = asset.metadata.get("sourceRefsPurpose") == Some(&json!("SOURCE_SCOPE"))
            && asset
                .metadata
                .get("sourceScope")
                .and_then(|v| v.get("sourceSha256"))
                .and_then(Value::as_str)
                == Some(source.sha256.as_str());
        check_refs(&asset.source_refs, &known, catalog_scope)
            .map_err(|reason| format!("资产「{}」的原文引用无效：{reason}", asset.name))?;
        for variant in &asset.variants {
            if variant.id.trim().is_empty()
                || variants.contains_key(&(asset.id.as_str(), variant.id.as_str()))
            {
                return Err("形象身份为空或重复".into());
            }
            variants.insert((asset.id.as_str(), variant.id.as_str()), variant);
            let unlocated = variant.scope.kind == "UNRESOLVED"
                && variant.scope.scenes.is_empty()
                && variant.metadata.get("sourceRefsPurpose") == Some(&json!("SOURCE_SCOPE"))
                && variant
                    .metadata
                    .get("sourceScope")
                    .and_then(|v| v.get("sourceSha256"))
                    .and_then(Value::as_str)
                    == Some(source.sha256.as_str());
            check_refs(&variant.source_refs, &known, unlocated).map_err(|reason| {
                format!(
                    "资产「{}」形象「{}」的原文引用无效：{reason}",
                    asset.name, variant.name
                )
            })?;
            let scope = effective_scope(variant);
            if !["UNRESOLVED", "GLOBAL", "SCENES"].contains(&scope.kind.as_str())
                || scope.kind == "GLOBAL"
                    && (scope.reason.as_ref().is_none_or(|r| r.trim().is_empty())
                        || !scope.scenes.is_empty()
                        || !variant.applicability.is_empty())
                || scope.kind == "SCENES" && scope.scenes.is_empty()
                || scope.kind == "UNRESOLVED" && !scope.scenes.is_empty()
                || !variant.applicability.is_empty() && variant.applicability != scope.scenes
            {
                return Err("形象作用范围无效；未定位不得当作全剧适用".into());
            }
            let mut scopes = HashSet::new();
            for scope in &scope.scenes {
                if !scopes.insert((&scope.episode_id, &scope.scene_id)) {
                    return Err("形象集场范围重复".into());
                }
                if !source
                    .scenes
                    .iter()
                    .any(|s| s.id == scope.scene_id && s.episode_id == scope.episode_id)
                {
                    return Err("形象适用集场未在原文中成立".into());
                }
                if !variant.source_refs.iter().any(|id| {
                    source
                        .rows
                        .iter()
                        .any(|r| &r.id == id && r.scene_id.as_ref() == Some(&scope.scene_id))
                }) {
                    return Err("形象适用场缺少本场有效来源，不能用主体出场回推衣服".into());
                }
            }
        }
    }
    for ((owner, id), variant) in &variants {
        let mut chain = HashSet::new();
        chain.insert((*owner, *id));
        let mut parent_owner = *owner;
        let mut parent = variant.source_variant_id.as_deref();
        while let Some(key) = parent {
            let (next_owner, next) =
                resolve_foundation_variant(&proposal.assets, parent_owner, key)?;
            if !chain.insert((next_owner, next.id.as_str())) {
                return Err("派生形象来源存在循环".into());
            }
            parent_owner = next_owner;
            parent = next.source_variant_id.as_deref();
        }
    }
    let mut fact_ids = HashSet::new();
    for fact in &proposal.facts {
        if !fact_ids.insert(&fact.id)
            || !proposal.assets.iter().any(|a| a.id == fact.asset_id)
            || fact
                .variant_id
                .as_ref()
                .map(|id| !variants.contains_key(&(fact.asset_id.as_str(), id.as_str())))
                .unwrap_or(false)
        {
            return Err("视觉事实身份或形象归属不一致".into());
        }
        if !["STABLE", "STATE", "TRANSITION"].contains(&fact.scope.as_str())
            || !["SOURCE", "CHARACTER_CLAIM"].contains(&fact.claim.as_str())
            || ["STATE", "TRANSITION"].contains(&fact.scope.as_str()) && fact.variant_id.is_none()
        {
            return Err("视觉事实状态/来源分型不合法".into());
        }
        check_refs(&fact.source_refs, &known, false)?;
    }
    let mut episode_ids = HashSet::new();
    let source_rows: HashMap<_, _> = source
        .rows
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();
    for episode in &proposal.episodes {
        if !episode_ids.insert(&episode.episode_id)
            || !source.episodes.iter().any(|e| e.id == episode.episode_id)
        {
            return Err("制作集引用未知或重复".into());
        }
        check_refs(&episode.source_refs, &known, false)?;
        for id in &episode.source_refs {
            let row = source_rows[id.as_str()];
            match row.episode_id.as_deref() {
                Some(owner) if owner == episode.episode_id => {}
                // The first foundation batch includes locked public background.
                // It is evidence for episode context, not another episode's body.
                None if row.kind == "context" => {}
                Some(owner) => {
                    let current = source
                        .episodes
                        .iter()
                        .find(|e| e.id == episode.episode_id)
                        .unwrap();
                    let other = source.episodes.iter().find(|e| e.id == owner);
                    return Err(format!(
                        "分集资料引用跨入未声明的别集：当前「{}」，引用「{}」第 {} 行（{}）",
                        current.title,
                        other.map(|e| e.title.as_str()).unwrap_or(owner),
                        row.number,
                        row.id
                    ));
                }
                None => {
                    return Err(format!(
                        "分集资料引用未定位且非公共背景的原文：第 {} 行（{}）",
                        row.number, row.id
                    ));
                }
            }
        }
    }
    for issue in &proposal.issues {
        check_refs(&issue.source_refs, &known, true)?;
        if issue
            .asset_id
            .as_ref()
            .is_some_and(|id| !proposal.assets.iter().any(|a| &a.id == id))
            || issue.variant_id.as_ref().is_some_and(|id| {
                issue
                    .asset_id
                    .as_ref()
                    .map(|asset| !variants.contains_key(&(asset.as_str(), id.as_str())))
                    .unwrap_or_else(|| !variants.keys().any(|(_, v)| *v == id))
            })
            || issue
                .episode_ids
                .iter()
                .any(|id| !source.episodes.iter().any(|e| &e.id == id))
        {
            return Err("局部问题的资产、形象或集数身份无效".into());
        }
    }
    Ok(())
}

fn resolve_foundation_variant<'a>(
    assets: &'a [FoundationAsset],
    owner: &str,
    id: &str,
) -> Result<(&'a str, &'a FoundationVariant), String> {
    if let Some(asset) = assets.iter().find(|a| a.id == owner) {
        if let Some(variant) = asset.variants.iter().find(|v| v.id == id) {
            return Ok((&asset.id, variant));
        }
    }
    let matches: Vec<_> = assets
        .iter()
        .flat_map(|a| {
            a.variants
                .iter()
                .filter(move |v| v.id == id)
                .map(move |v| (a.id.as_str(), v))
        })
        .collect();
    if matches.len() == 1 {
        Ok(matches[0])
    } else {
        Err("派生形象来源不存在或跨身份不唯一".into())
    }
}

fn empty_foundation(source: &LockedSource) -> FoundationProposal {
    FoundationProposal {
        schema_version: "foundation/v1".into(),
        source_sha256: source.sha256.clone(),
        overview: String::new(),
        direction: String::new(),
        episodes: vec![],
        assets: vec![],
        facts: vec![],
        issues: vec![],
        rule_snapshots: vec![],
        updated_episode_ids: vec![],
    }
}
fn materialize_foundation(
    source: &LockedSource,
    proposal: &mut FoundationProposal,
) -> Result<(), String> {
    for asset in &mut proposal.assets {
        for variant in &mut asset.variants {
            variant.scope = effective_scope(variant);
            variant.applicability = variant.scope.scenes.clone();
        }
    }
    let mut safe = Vec::new();
    for mut fact in std::mem::take(&mut proposal.facts) {
        if ["STATE", "TRANSITION"].contains(&fact.scope.as_str()) && fact.variant_id.is_none() {
            let episodes = source
                .rows
                .iter()
                .filter(|r| fact.source_refs.contains(&r.id))
                .filter_map(|r| r.episode_id.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            proposal.issues.push(FoundationIssue {
                coordinate: format!("asset:{}/fact:{}", fact.asset_id, fact.id),
                problem: "状态或变化事实尚未绑定形象，不会广播到其它形象".into(),
                source_refs: fact.source_refs.clone(),
                blocking: true,
                asset_id: Some(fact.asset_id.clone()),
                variant_id: None,
                episode_ids: episodes,
                details: Some(json!({"unresolvedFact":fact})),
            });
            continue;
        }
        fact.quotes = fact
            .source_refs
            .iter()
            .map(|id| {
                source
                    .rows
                    .iter()
                    .find(|r| &r.id == id)
                    .map(|r| r.text.clone())
                    .ok_or("事实回读引用不存在".into())
            })
            .collect::<Result<_, String>>()?;
        safe.push(fact);
    }
    proposal.facts = safe;
    Ok(())
}
fn existing_foundation(
    source: &LockedSource,
    context: &Value,
) -> Result<FoundationProposal, String> {
    let mut p = if context.get("foundation").is_some_and(Value::is_object) {
        serde_json::from_value::<FoundationProposal>(context["foundation"].clone())
            .map_err(|e| format!("既有制作基础须先按来源契约导入：{e}"))?
    } else {
        empty_foundation(source)
    };
    if p.source_sha256 != source.sha256 {
        return Err("既有基础不属于当前原文，需先按锁定来源重定位；不会静默沿用旧证据".into());
    }
    for row in context
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| !crate::store::asset_deleted(row))
    {
        let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or("");
        let owner = if !text("identityId").is_empty() {
            text("identityId")
        } else if !text("baseAssetId").is_empty() {
            text("baseAssetId")
        } else {
            text("id")
        };
        let vid = text("variantId");
        let source_refs: Vec<String> =
            serde_json::from_value(row.get("sourceRefs").cloned().unwrap_or(json!([])))
                .map_err(|_| format!("资产「{}」的原文引用格式无效", text("name")))?;
        let registered = p
            .assets
            .iter()
            .any(|a| a.id == owner && a.variants.iter().any(|v| v.id == vid));
        // Image assets may be standalone drafts. A saved picture/name is not source evidence
        // and must not become a first-stage identity merely because it is in this project.
        if source_refs.is_empty() && !registered {
            continue;
        }
        if owner.is_empty() || vid.is_empty() {
            return Err(format!("资产「{}」缺少稳定身份或形象 ID", text("name")));
        }
        if !p.assets.iter().any(|a| a.id == owner) {
            let kind = match text("kind") {
                "character" | "CH" => "CH",
                "group" | "GROUP" => "GROUP",
                "beast" | "BEAST" => "BEAST",
                "scene" | "LOC" => "LOC",
                "prop" | "PROP" => "PROP",
                kind => return Err(format!("资产「{}」的类型「{kind}」不受支持", text("name"))),
            };
            p.assets.push(FoundationAsset {
                id: owner.into(),
                name: text("name").split('｜').next().unwrap_or(owner).into(),
                kind: kind.into(),
                aliases: vec![],
                description: text("description").into(),
                source_refs: source_refs.clone(),
                variants: vec![],
                metadata: production_metadata(row),
            });
        }
        let asset = p.assets.iter_mut().find(|a| a.id == owner).unwrap();
        if let Some(v) = asset.variants.iter_mut().find(|v| v.id == vid) {
            if v.asset_row_id.as_ref().is_some_and(|id| id != text("id")) {
                return Err("同一身份/形象对应多条既有资产行，需明确映射".into());
            }
            v.asset_row_id = Some(text("id").into());
        } else {
            let scenes: Vec<_> = source
                .scenes
                .iter()
                .filter(|scene| {
                    row.pointer("/applicability/scenes")
                        .and_then(Value::as_array)
                        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(&scene.id)))
                })
                .map(|s| Applicability {
                    episode_id: s.episode_id.clone(),
                    scene_id: s.id.clone(),
                })
                .collect();
            let scope = row
                .get("scope")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| "既有形象范围无效")?
                .unwrap_or_else(|| {
                    if scenes.is_empty() {
                        FoundationScope::default()
                    } else {
                        FoundationScope {
                            kind: "SCENES".into(),
                            scenes: scenes.clone(),
                            reason: None,
                        }
                    }
                });
            asset.variants.push(FoundationVariant {
                id: vid.into(),
                name: text("name").into(),
                description: text("description").into(),
                source_variant_id: row
                    .get("sourceVariantId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                source_refs,
                applicability: scenes,
                scope,
                asset_row_id: Some(text("id").into()),
                metadata: production_metadata(row),
            });
        }
    }
    materialize_foundation(source, &mut p)?;
    if !p.assets.is_empty() {
        validate_foundation(source, &p)?;
    }
    Ok(p)
}
fn merge_foundation(
    target: &mut FoundationProposal,
    candidate: FoundationProposal,
    confirmed: Option<&str>,
    resolved_at_start: &HashSet<(String, String)>,
) -> Result<(), String> {
    if !candidate.overview.trim().is_empty() {
        target.overview = candidate.overview;
    }
    if let Some(direction) = confirmed {
        target.direction = direction.into();
    } else if !candidate.direction.trim().is_empty() {
        target.direction = candidate.direction;
    }
    for e in candidate.episodes {
        if let Some(old) = target
            .episodes
            .iter_mut()
            .find(|old| old.episode_id == e.episode_id)
        {
            if !old.summary.is_empty() {
                old.summary.push('\n');
            }
            old.summary.push_str(&e.summary);
            if !old.context.is_empty() {
                old.context.push('\n');
            }
            old.context.push_str(&e.context);
            for id in e.source_refs {
                if !old.source_refs.contains(&id) {
                    old.source_refs.push(id);
                }
            }
        } else {
            target.episodes.push(e);
        }
    }
    for a in candidate.assets {
        if let Some(old) = target.assets.iter_mut().find(|old| old.id == a.id) {
            if old.name != a.name || old.kind != a.kind {
                return Err("已登记身份被改名或改类型，请局部核对".into());
            }
            for (key, value) in a.metadata {
                old.metadata.insert(key, value);
            }
            for alias in a.aliases {
                if !old.aliases.contains(&alias) {
                    old.aliases.push(alias);
                }
            }
            for id in a.source_refs {
                if !old.source_refs.contains(&id) {
                    old.source_refs.push(id);
                }
            }
            for v in a.variants {
                if let Some(prior) = old.variants.iter_mut().find(|old| old.id == v.id) {
                    if prior.source_variant_id != v.source_variant_id {
                        return Err(format!(
                            "形象「{}｜{}」的派生来源发生变化（{} → {}），请明确调整来源关系",
                            old.name,
                            prior.name,
                            prior.source_variant_id.as_deref().unwrap_or("无"),
                            v.source_variant_id.as_deref().unwrap_or("无")
                        ));
                    }
                    for (key, value) in v.metadata {
                        prior.metadata.insert(key, value);
                    }
                    // IDs and parent relationships identify the appearance. A display label
                    // cannot break a batch. Only a genuinely unlocated appearance may acquire
                    // its first concrete label; pruning a rerun's scenes does not unlock it.
                    if prior.scope.kind == "UNRESOLVED"
                        && v.scope.kind == "SCENES"
                        && prior.name != v.name
                        && !v.name.trim().is_empty()
                        && !resolved_at_start.contains(&(old.id.clone(), prior.id.clone()))
                    {
                        prior.name = v.name.clone();
                        prior.scope.reason = v.scope.reason.clone();
                    }
                    for id in v.source_refs {
                        if !prior.source_refs.contains(&id) {
                            prior.source_refs.push(id);
                        }
                    }
                    if prior.scope.kind == "GLOBAL" || v.scope.kind == "GLOBAL" {
                        if prior.scope.kind != "GLOBAL" {
                            prior.scope = v.scope;
                            prior.applicability.clear();
                        }
                    } else {
                        for place in v.scope.scenes {
                            if !prior.scope.scenes.contains(&place) {
                                prior.scope.scenes.push(place);
                            }
                        }
                        prior.scope.kind = if prior.scope.scenes.is_empty() {
                            "UNRESOLVED"
                        } else {
                            "SCENES"
                        }
                        .into();
                        prior.applicability = prior.scope.scenes.clone();
                    }
                    if !v.description.trim().is_empty() {
                        prior.description = v.description;
                    }
                } else {
                    old.variants.push(v);
                }
            }
        } else {
            target.assets.push(a);
        }
    }
    for f in candidate.facts {
        if let Some(prior) = target.facts.iter_mut().find(|old| old.id == f.id) {
            if prior.asset_id != f.asset_id
                || prior.variant_id != f.variant_id
                || prior.attribute != f.attribute
                || prior.value != f.value
                || prior.scope != f.scope
                || prior.claim != f.claim
            {
                return Err("事实ID被复用为不同内容".into());
            }
            for id in f.source_refs {
                if !prior.source_refs.contains(&id) {
                    prior.source_refs.push(id);
                }
            }
        } else {
            target.facts.push(f);
        }
    }
    for issue in candidate.issues {
        if !target.issues.iter().any(|old| {
            old.coordinate == issue.coordinate
                && old.problem == issue.problem
                && old.source_refs == issue.source_refs
        }) {
            target.issues.push(issue);
        }
    }
    Ok(())
}
pub async fn run_foundation(
    source: &LockedSource,
    context: &Value,
    executor: &dyn TextExecutor,
    model: &str,
    reasoning: &str,
) -> Result<FoundationProposal, String> {
    let source = upgrade_source_dialogues(source)?;
    let selected: Vec<String> = if let Some(ids) = context.pointer("/selection/episodeIds") {
        let ids = ids
            .as_array()
            .filter(|ids| !ids.is_empty())
            .ok_or("请选择非空制作集范围")?;
        let mut seen = HashSet::new();
        ids.iter()
            .map(|id| {
                let id = id.as_str().ok_or("制作集ID无效")?;
                if !source.episodes.iter().any(|e| e.id == id) || !seen.insert(id) {
                    return Err("制作集未知或重复".into());
                }
                Ok(id.into())
            })
            .collect::<Result<_, String>>()?
    } else {
        source.episodes.iter().map(|e| e.id.clone()).collect()
    };
    if selected.is_empty() {
        return Err("当前原文没有已确认的制作集".into());
    }
    if source.diagnostics.iter().any(|d| {
        d.blocking
            && (d.row_ids.is_empty()
                || d.row_ids.iter().any(|id| {
                    source.rows.iter().find(|r| &r.id == id).is_some_and(|r| {
                        r.episode_id.as_ref().is_none_or(|ep| selected.contains(ep))
                    })
                }))
    }) {
        return Err("所选范围仍有待确认集场/对白，请先保存明确决定".into());
    }
    let mut merged = existing_foundation(&source, context)?;
    let resolved_at_start: HashSet<_> = merged
        .assets
        .iter()
        .flat_map(|asset| {
            asset.variants.iter().filter_map(|variant| {
                (effective_scope(variant).kind != "UNRESOLVED")
                    .then(|| (asset.id.clone(), variant.id.clone()))
            })
        })
        .collect();
    let confirmed = context
        .pointer("/settings/confirmedDirection")
        .and_then(|v| {
            v.as_str()
                .or_else(|| v.get("content").and_then(Value::as_str))
        })
        .filter(|s| !s.trim().is_empty());
    if let Some(direction) = confirmed {
        merged.direction = direction.into();
    }
    merged
        .episodes
        .retain(|e| !selected.contains(&e.episode_id));
    for a in &mut merged.assets {
        for v in &mut a.variants {
            if v.scope.kind == "SCENES" {
                v.scope.scenes.retain(|s| !selected.contains(&s.episode_id));
                v.applicability = v.scope.scenes.clone();
                if v.scope.scenes.is_empty() {
                    v.scope.kind = "UNRESOLVED".into();
                }
            }
        }
    }
    merged.facts.retain_mut(|f| {
        f.source_refs.retain(|id| {
            source.rows.iter().find(|r| &r.id == id).is_none_or(|r| {
                r.episode_id
                    .as_ref()
                    .is_none_or(|ep| !selected.contains(ep))
            })
        });
        !f.source_refs.is_empty()
    });
    merged
        .issues
        .retain(|issue| !issue.episode_ids.iter().any(|id| selected.contains(id)));
    let author: Vec<_> = source.rows.iter().filter(|r| r.kind == "context").collect();
    let mut first = true;
    let episodes: Vec<_> = source
        .episodes
        .iter()
        .filter(|e| selected.contains(&e.id))
        .collect();
    for (episode_index, episode) in episodes.iter().enumerate() {
        let mut batches: Vec<Vec<&SourceRow>> = vec![vec![]];
        let mut size = 0;
        for row in source
            .rows
            .iter()
            .filter(|r| r.episode_id.as_ref() == Some(&episode.id))
        {
            if size > 0 && size + row.text.chars().count() > 16000 {
                batches.push(vec![]);
                size = 0;
            }
            batches.last_mut().unwrap().push(row);
            size += row.text.chars().count();
        }
        for (batch_index, batch) in batches.iter().enumerate() {
            let provided: Vec<_> = if first {
                author
                    .iter()
                    .copied()
                    .chain(batch.iter().copied())
                    .collect()
            } else {
                batch.clone()
            };
            first = false;
            let row_ids: Vec<_> = provided.iter().map(|r| r.id.clone()).collect();
            let known: HashSet<_> = row_ids.iter().map(String::as_str).collect();
            let catalog:Vec<_>=merged.assets.iter().map(|a|json!({"id":a.id,"name":a.name,"kind":a.kind,"aliases":a.aliases,"variants":a.variants.iter().map(|v|json!({"id":v.id,"name":v.name,"sourceVariantId":v.source_variant_id,"scope":v.scope})).collect::<Vec<_>>()})).collect();
            let input = json!({"sourceSha256":source.sha256,"episode":episode,"selection":{"episodeIds":selected},"batchIndex":batch_index,"batchCount":batches.len(),"rows":provided,"knownAssets":catalog,"confirmedDirection":confirmed,"priorDirection":merged.direction,"finalSelectedBatch":episode_index+1==episodes.len()&&batch_index+1==batches.len(),"instruction":"只输出本批有据新增/更新；沿用已登记identity/variant ID，同名不是合并依据；只申报当前episode。保留已确认方向；未确认方向逐批累计，最后批次交范围匹配候选。源句由程序回读；资料只在首次提供不重复提取。UNRESOLVED不是GLOBAL；STATE/TRANSITION指向具体形象，否则交局部issue。"});
            let result = call_validated(
                executor,
                "foundation",
                "story-foundation",
                model,
                reasoning,
                input,
                foundation_schema(&source, &row_ids),
                &[],
                |value| {
                    let mut c: FoundationProposal = serde_json::from_value(value.clone())
                        .map_err(|e| format!("制作基础结构无效：{e}"))?;
                    if c.schema_version != "foundation/v1" || c.source_sha256 != source.sha256 {
                        return Err("本批源版本不匹配".into());
                    }
                    if c.episodes.len() != 1 {
                        return Err("本批必须完整申报当前制作集，不得漏集或重复".into());
                    }
                    for e in &c.episodes {
                        if e.episode_id != episode.id {
                            return Err("本批输出了未提供的制作集".into());
                        }
                        check_refs(&e.source_refs, &known, false)?;
                    }
                    for a in &c.assets {
                        check_refs(&a.source_refs, &known, false)?;
                        for v in &a.variants {
                            check_refs(&v.source_refs, &known, false)?;
                            if effective_scope(v)
                                .scenes
                                .iter()
                                .any(|s| s.episode_id != episode.id)
                            {
                                return Err("本批形象范围越出所选制作集".into());
                            }
                            // Validate this batch's own evidence before old references are
                            // unioned. Earlier appearance evidence cannot prove a new scene.
                            for place in &effective_scope(v).scenes {
                                if !v.source_refs.iter().any(|id| {
                                    source.rows.iter().any(|row| {
                                        &row.id == id
                                            && row.scene_id.as_ref() == Some(&place.scene_id)
                                            && row.episode_id.as_ref() == Some(&place.episode_id)
                                    })
                                }) {
                                    return Err(format!(
                                        "形象「{}｜{}」的本批适用场缺少本场来源，不能沿用旧引用补足",
                                        a.name, v.name
                                    ));
                                }
                            }
                        }
                    }
                    for f in &c.facts {
                        check_refs(&f.source_refs, &known, false)?;
                    }
                    for issue in &c.issues {
                        check_refs(&issue.source_refs, &known, true)?;
                        if issue.episode_ids.iter().any(|id| id != &episode.id) {
                            return Err("本批问题范围越出所选制作集".into());
                        }
                    }
                    for a in &mut c.assets {
                        for v in &mut a.variants {
                            v.asset_row_id = None;
                        }
                    }
                    materialize_foundation(&source, &mut c)?;
                    let mut next = merged.clone();
                    merge_foundation(&mut next, c.clone(), confirmed, &resolved_at_start)?;
                    materialize_foundation(&source, &mut next)?;
                    validate_foundation(&source, &next)?;
                    *value = serde_json::to_value(c).map_err(|e| e.to_string())?;
                    Ok(())
                },
            )
            .await?;
            merge_foundation(
                &mut merged,
                serde_json::from_value(result).map_err(|e| e.to_string())?,
                confirmed,
                &resolved_at_start,
            )?;
        }
    }
    materialize_foundation(&source, &mut merged)?;
    validate_foundation(&source, &merged)?;
    if selected
        .iter()
        .any(|id| !merged.episodes.iter().any(|e| &e.episode_id == id))
    {
        return Err("本批遗漏所选制作集，已接受批次保留，可局部继续".into());
    }
    merged.updated_episode_ids = selected;
    merged.rule_snapshots = rule_snapshots()
        .into_iter()
        .filter(|r| ["screenplay-fidelity", "story-foundation"].contains(&r.id.as_str()))
        .collect();
    Ok(merged)
}

pub fn flatten_foundation_assets(proposal: &FoundationProposal) -> Vec<Value> {
    proposal.assets.iter().flat_map(|asset|asset.variants.iter().map(move |variant|{
        let scope=effective_scope(variant);
        let mut row=json!({"id":variant.asset_row_id.clone().unwrap_or_else(||format!("{}:{}",asset.id,variant.id)),"identityId":asset.id,"baseAssetId":asset.id,"variantId":variant.id,"name":format!("{}｜{}",asset.name,variant.name),"kind":asset.kind,"description":variant.description,"sourceRefs":variant.source_refs,"scope":scope,"applicability":{"episodes":scope.scenes.iter().map(|a|a.episode_id.clone()).collect::<std::collections::BTreeSet<_>>(),"scenes":scope.scenes.iter().map(|a|a.scene_id.clone()).collect::<std::collections::BTreeSet<_>>()},"sourceVariantId":variant.source_variant_id});
        if !asset.aliases.is_empty() {row["aliases"]=json!(asset.aliases);}
        for metadata in [&asset.metadata,&variant.metadata] {
            for key in ["role","relationNote","preparation","note","prompt","catalogName","usageEpisodes","sourceRefsPurpose","sourceScope"] {
                if let Some(value)=metadata.get(key) { row[key]=value.clone(); }
            }
        }
        row
    })).collect()
}

fn production_metadata(row: &Value) -> serde_json::Map<String, Value> {
    [
        "role",
        "relationNote",
        "preparation",
        "note",
        "prompt",
        "catalogName",
        "usageEpisodes",
        "sourceRefsPurpose",
        "sourceScope",
    ]
    .into_iter()
    .filter_map(|key| row.get(key).map(|value| (key.to_owned(), value.clone())))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScopeHandoffDesigner {
        requests: std::sync::Mutex<Vec<Value>>,
    }
    #[async_trait]
    impl TextExecutor for ScopeHandoffDesigner {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            assert_eq!(request.stage, "assetDesign", "有效事实交接不应追加模型岗位");
            self.requests.lock().unwrap().push(request.input.clone());
            let asset = &request.input["asset"];
            Ok(
                json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":"依据既有视觉记录的基础设计","freeDesignScope":"未限定背景","sourceRefs":asset["sourceRefs"],"referenceVariantIds":[]}),
            )
        }
    }

    #[tokio::test]
    async fn program_scope_metadata_survives_asset_handoff_without_locating_candidates_or_creating_facts(
    ) {
        let source = prepare_source("第一集\n场景：门口\n△小王黑发白衣出现。\n").unwrap();
        let row = source
            .rows
            .iter()
            .find(|row| row.text.contains("黑发"))
            .unwrap();
        let place = Applicability {
            episode_id: source.episodes[0].id.clone(),
            scene_id: row.scene_id.clone().unwrap(),
        };
        let source_scope = json!({"sourceSha256":source.sha256,"kind":"PROGRAM_SCOPE"});
        let metadata: serde_json::Map<String, Value> = serde_json::from_value(json!({
            "sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":source_scope,"usageEpisodes":[place.episode_id]
        })).unwrap();
        let base = FoundationVariant {
            id: "base".into(),
            name: "基础".into(),
            description: String::new(),
            source_variant_id: None,
            source_refs: vec![row.id.clone()],
            applicability: vec![place.clone()],
            scope: FoundationScope {
                kind: "SCENES".into(),
                scenes: vec![place],
                reason: None,
            },
            asset_row_id: Some("base-row".into()),
            metadata: metadata.clone(),
        };
        let mut candidate = FoundationVariant {
            id: "candidate".into(),
            name: "待定位形象".into(),
            description: String::new(),
            source_variant_id: Some("base".into()),
            source_refs: vec![],
            applicability: vec![],
            scope: FoundationScope::default(),
            asset_row_id: Some("candidate-row".into()),
            metadata: metadata.clone(),
        };
        candidate.metadata.insert("usageEpisodes".into(), json!([]));
        let mut foundation = empty_foundation(&source);
        foundation.assets.push(FoundationAsset {
            id: "person".into(),
            name: "小王".into(),
            kind: "CH".into(),
            aliases: vec!["王少".into(), "原登记旧称".into()],
            description: String::new(),
            source_refs: vec![row.id.clone()],
            variants: vec![base, candidate],
            metadata,
        });
        foundation.facts.push(FoundationFact {
            id: "hair".into(),
            asset_id: "person".into(),
            variant_id: Some("base".into()),
            attribute: "hair.color".into(),
            value: "黑发".into(),
            scope: "STABLE".into(),
            claim: "SOURCE".into(),
            source_refs: vec![row.id.clone()],
            quotes: vec![row.text.clone()],
        });
        validate_foundation(&source, &foundation).unwrap();
        let assets = flatten_foundation_assets(&foundation);
        for asset in &assets {
            assert_eq!(
                asset["aliases"],
                json!(["王少", "原登记旧称"]),
                "every formal variant carries the exact registered identity aliases"
            );
            assert_eq!(asset["sourceRefsPurpose"], "SOURCE_SCOPE");
            assert_eq!(asset["sourceScope"], source_scope);
            let imported_metadata = production_metadata(asset);
            assert_eq!(imported_metadata["sourceRefsPurpose"], "SOURCE_SCOPE");
            assert_eq!(imported_metadata["sourceScope"], source_scope);
        }
        let restored =
            existing_foundation(&source, &json!({"foundation":foundation,"assets":assets}))
                .unwrap();
        assert_eq!(
            serde_json::to_value(&restored.facts).unwrap(),
            serde_json::to_value(&foundation.facts).unwrap()
        );
        let pending = &restored.assets[0].variants[1];
        assert_eq!(pending.scope.kind, "UNRESOLVED");
        assert!(
            pending.scope.scenes.is_empty()
                && pending.applicability.is_empty()
                && pending.source_refs.is_empty()
        );
        assert_eq!(
            assets[1]["applicability"],
            json!({"episodes":[],"scenes":[]})
        );
        assert_eq!(assets[1]["usageEpisodes"], json!([]));
        let executor = ScopeHandoffDesigner {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let input = json!({"source":source,"assets":assets,"selection":{"assetIds":["base-row"]},"foundation":restored});
        let designed = assets::run_asset_design(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert!(
            designed["blocked"].as_array().unwrap().is_empty(),
            "{designed}"
        );
        assert_eq!(
            designed["facts"],
            serde_json::to_value(&foundation.facts).unwrap()
        );
        let calls = executor.requests.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["asset"]["sourceRefsPurpose"], "SOURCE_SCOPE");
        assert_eq!(calls[0]["asset"]["sourceScope"], source_scope);
        assert!(calls[0]["sourceEvidencePolicy"].is_string());
        assert_eq!(calls[0]["facts"][0]["assetId"], "person");
        assert_eq!(calls[0]["facts"][0]["variantId"], "base");
        assert_eq!(calls[0]["facts"][0]["claim"], "SOURCE");
    }

    #[test]
    fn original_production_rules_and_selected_cards_have_versioned_bound_hashes() {
        let rules = rule_snapshots();
        let identities: HashSet<_> = rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(identities.len(), rules.len());
        for id in [
            "foundation-asset-catalog",
            "foundation-finishing-editor",
            "asset-visual-facts-extractor",
            "storyboard-common",
            "storyboard-source-context",
        ] {
            assert!(
                identities.contains(id),
                "missing original production contract {id}"
            );
        }
        assert!(rules.iter().all(|r| r.sha256 == digest(&r.content)));
        for rule in &rules {
            let expected = if ["foundation-asset-catalog", "foundation-finishing-editor"]
                .contains(&rule.id.as_str())
            {
                "2.0.1"
            } else if [
                "episode-emotion-planner",
                "unit-performance-director",
                "episode-finishing-editor",
            ]
            .contains(&rule.id.as_str())
                || [
                    "acting-blocking",
                    "camera-emotion",
                    "lighting-space",
                    "sound-rhythm",
                    "fantasy-expression",
                ]
                .contains(&rule.id.as_str())
            {
                "2.0.0"
            } else if rule.id == "asset-art-director" {
                "2.0.0"
            } else if rule.id == "asset-image-production" {
                "1.2.0"
            } else if [
                "screenplay-fidelity",
                "story-foundation",
                "asset-image-production",
                "unit-video-production",
                "asset-visual-facts-extractor",
            ]
            .contains(&rule.id.as_str())
            {
                "1.1.0"
            } else {
                "1.0.0"
            };
            assert_eq!(
                rule.version, expected,
                "Frozen rule identity must reflect the versioned production contract"
            );
        }
    }
    #[test]
    fn cross_scene_appearance_and_derived_cycle_are_rejected() {
        let s =
            prepare_source("第一集\n场景：大厅\n小王：走。\n场景：门口\n△小王换白衣。\n").unwrap();
        let variant = FoundationVariant {
            id: "look-a".into(),
            name: "白衣".into(),
            description: "白衣".into(),
            source_variant_id: None,
            source_refs: vec![s.rows[4].id.clone()],
            applicability: vec![Applicability {
                episode_id: s.episodes[0].id.clone(),
                scene_id: s.scenes[0].id.clone(),
            }],
            scope: FoundationScope::default(),
            asset_row_id: None,
            metadata: Default::default(),
        };
        let mut p = FoundationProposal {
            schema_version: "foundation/v1".into(),
            source_sha256: s.sha256.clone(),
            overview: "".into(),
            direction: "".into(),
            episodes: vec![],
            assets: vec![FoundationAsset {
                id: "a".into(),
                name: "小王".into(),
                kind: "CH".into(),
                aliases: vec![],
                description: "人物".into(),
                source_refs: vec![s.rows[2].id.clone()],
                variants: vec![variant],
                metadata: Default::default(),
            }],
            facts: vec![],
            issues: vec![],
            rule_snapshots: vec![],
            updated_episode_ids: vec![],
        };
        assert!(validate_foundation(&s, &p).is_err());
        p.assets[0].variants[0].applicability[0].scene_id = s.scenes[1].id.clone();
        p.assets[0].variants[0].source_variant_id = Some("look-a".into());
        assert!(validate_foundation(&s, &p).is_err());
    }
}
