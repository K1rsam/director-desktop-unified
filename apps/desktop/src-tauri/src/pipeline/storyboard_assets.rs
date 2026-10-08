use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub(super) const PROFILE: &str = "asset-production-card/v1";
pub(super) const SCOPED_PROFILE: &str = "asset-production-card/v2";

/// The marker is frozen only on new workflows. Historical inputs must keep the
/// exact asset envelope that produced their already-paid author/plan caches.
pub(super) fn validate_profile(input: &Value) -> Result<bool, String> {
    match input.get("storyboardAssetInputProfile") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::String(profile)) if [PROFILE, SCOPED_PROFILE].contains(&profile.as_str()) => {
            Ok(true)
        }
        _ => Err("分镜资产输入运输版本未知；原快照和缓存已保留，未派发模型请求".into()),
    }
}

fn scoped_profile(input: &Value) -> bool {
    input["storyboardAssetInputProfile"] == SCOPED_PROFILE
}

/// Alias evidence is copied from a registered identity, never inferred from a
/// display name. This read-only copy also supplies the existing historical
/// retrieval, while saved assets and historical request inputs stay untouched.
pub(super) fn inherit_identity_aliases(input: &Value, assets: &mut [Value]) -> Result<(), String> {
    validate_profile(input)?;
    if !scoped_profile(input) {
        return Ok(());
    }
    for asset in assets {
        let identity = asset["identityId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .or_else(|| asset["baseAssetId"].as_str().filter(|id| !id.is_empty()));
        let Some(identity) = identity else { continue };
        if asset["identityId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .is_some_and(|id| {
                asset["baseAssetId"]
                    .as_str()
                    .filter(|base| !base.is_empty())
                    .is_some_and(|base| base != id)
            })
        {
            return Err("资产的正式身份键不一致，未按姓名继承别名".into());
        }
        let candidates: Vec<_> = input["foundation"]["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|registered| registered["id"] == identity)
            .collect();
        let registered = match candidates.as_slice() {
            [] => continue, // A separately imported/manual identity keeps its own aliases.
            [one] => *one,
            _ => return Err("正式制作基础含重复身份键，未猜测别名来源".into()),
        };
        let variant = registered["variants"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|variant| {
                variant["id"] == asset["variantId"]
                    && variant["assetRowId"]
                        .as_str()
                        .map(|row| asset["id"] == row)
                        .unwrap_or_else(|| {
                            asset["id"]
                                == format!(
                                    "{}:{}",
                                    identity,
                                    asset["variantId"].as_str().unwrap_or("")
                                )
                        })
            });
        let mut aliases = Vec::new();
        for value in [
            asset.get("aliases"),
            registered.get("aliases"),
            variant.and_then(|v| v.get("aliases")),
        ]
        .into_iter()
        .flatten()
        {
            for alias in value.as_array().ok_or("正式资产别名须为字符串数组")? {
                let alias = alias.as_str().ok_or("正式资产别名不是字符串")?;
                if !aliases.iter().any(|known| known == alias) {
                    aliases.push(alias.to_string());
                }
            }
        }
        if !aliases.is_empty() {
            asset["aliases"] = json!(aliases);
        }
    }
    Ok(())
}

fn string_set(value: &Value) -> Option<BTreeSet<&str>> {
    let items = value.as_array()?;
    let values: BTreeSet<_> = items.iter().map(Value::as_str).collect::<Option<_>>()?;
    (values.len() == items.len()).then_some(values)
}

/// Only the typed, program-owned scope is reduced. Unknown imported shapes
/// remain literal; validators always receive the original full scope.
fn project_scope(card: &mut Value, episode_id: &str) {
    let Some(scope) = card["scope"].as_object() else {
        return;
    };
    if scope
        .keys()
        .any(|key| !["kind", "reason", "scenes"].contains(&key.as_str()))
        || !["SCENES", "GLOBAL"].contains(&scope.get("kind").and_then(Value::as_str).unwrap_or(""))
        || scope
            .get("reason")
            .is_some_and(|v| !v.is_null() && !v.is_string())
    {
        return;
    }
    let Some(pairs) = scope.get("scenes").and_then(Value::as_array) else {
        return;
    };
    if pairs.is_empty()
        || pairs.iter().any(|pair| {
            pair.as_object().is_none_or(|p| {
                p.len() != 2
                    || !p.contains_key("episodeId")
                    || !p.contains_key("sceneId")
                    || !pair["episodeId"].is_string()
                    || !pair["sceneId"].is_string()
            })
        })
    {
        return;
    }
    let episodes: BTreeSet<_> = pairs
        .iter()
        .filter_map(|p| p["episodeId"].as_str())
        .collect();
    let scenes: BTreeSet<_> = pairs.iter().filter_map(|p| p["sceneId"].as_str()).collect();
    let equivalent = card["applicability"].as_object().is_some_and(|a| {
        a.len() == 2
            && a.contains_key("episodes")
            && a.contains_key("scenes")
            && string_set(&a["episodes"]).as_ref() == Some(&episodes)
            && string_set(&a["scenes"]).as_ref() == Some(&scenes)
    });
    let current: Vec<_> = pairs
        .iter()
        .filter(|pair| pair["episodeId"] == episode_id)
        .cloned()
        .collect();
    card["scope"]["scenes"] = json!(current);
    if equivalent {
        card.as_object_mut().unwrap().remove("applicability");
    }
}

pub(in crate::pipeline) fn project_for_episode(
    input: &Value,
    assets: &[Value],
    episode_id: &str,
) -> Result<Vec<Value>, String> {
    let mut cards = project(input, assets)?;
    if !scoped_profile(input) {
        return Ok(cards);
    }
    inherit_identity_aliases(input, &mut cards)?;
    for card in &mut cards {
        project_scope(card, episode_id);
        let shared = card["sourceScope"].as_object().is_some_and(|scope| {
            scope.len() == 2
                && scope.get("kind").and_then(Value::as_str) == Some("PROGRAM_SCOPE")
                && scope.contains_key("sourceSha256")
                && scope["sourceSha256"] == input["source"]["sha256"]
                && input["source"]["sha256"]
                    .as_str()
                    .is_some_and(|sha| !sha.is_empty())
        });
        if shared {
            card.as_object_mut().unwrap().remove("sourceScope");
            card["sourceScopeKind"] = json!("PROGRAM_SCOPE");
        }
    }
    Ok(cards)
}

/// New Mac-compatible requests select at the role's source boundary before
/// serialization. Unknown imported scope shapes stay literal; only explicit
/// episode/scene identities authorize pruning. Full validators keep receiving
/// the original catalogue, so a context/material card never grants appearance.
pub(in crate::pipeline) fn project_for_scope(
    input: &Value,
    assets: &[Value],
    episode_id: &str,
    rows: &[crate::pipeline::SourceRow],
    context: &[crate::pipeline::SourceRow],
) -> Result<(Vec<Value>, Value), String> {
    let cards = project_for_episode(input, assets, episode_id)?;
    if !crate::pipeline::stage3_pack::uses_profile(input)? {
        return Ok((cards, Value::Null));
    }
    let scenes: BTreeSet<_> = rows
        .iter()
        .filter_map(|row| row.scene_id.as_deref())
        .collect();
    let context_refs: BTreeSet<_> = context.iter().map(|row| row.id.as_str()).collect();
    let mut selected = Vec::new();
    let mut purposes = Vec::new();
    for (asset, card) in assets.iter().zip(cards) {
        // This is an exact identity predicate, never a name/description search.
        let applicable = if asset["scope"]["kind"] == "GLOBAL" {
            Some(true)
        } else if asset["scope"]["kind"] == "SCENES" {
            asset["scope"]["scenes"]
                .as_array()
                .filter(|pairs| !pairs.is_empty())
                .filter(|pairs| {
                    pairs
                        .iter()
                        .all(|pair| pair["episodeId"].is_string() && pair["sceneId"].is_string())
                })
                .map(|pairs| {
                    pairs.iter().any(|pair| {
                        pair["episodeId"] == episode_id
                            && pair["sceneId"]
                                .as_str()
                                .is_some_and(|scene| scenes.contains(scene))
                    })
                })
        } else if let Some(scope) = asset["applicability"].as_object() {
            scope
                .get("scenes")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
                .filter(|items| items.iter().all(Value::is_string))
                .map(|items| {
                    items
                        .iter()
                        .any(|scene| scenes.contains(scene.as_str().unwrap()))
                })
        } else if let Some(pairs) = asset["applicability"]
            .as_array()
            .filter(|items| !items.is_empty())
        {
            if pairs
                .iter()
                .all(|pair| pair["episodeId"].is_string() && pair["sceneId"].is_string())
            {
                Some(pairs.iter().any(|pair| {
                    pair["episodeId"] == episode_id
                        && pair["sceneId"]
                            .as_str()
                            .is_some_and(|scene| scenes.contains(scene))
                }))
            } else {
                None
            }
        } else {
            None
        };
        let context_evidence = asset["sourceRefs"].as_array().is_some_and(|refs| {
            refs.iter()
                .filter_map(Value::as_str)
                .any(|id| context_refs.contains(id))
        });
        if applicable == Some(false) && !context_evidence {
            continue;
        }
        let mut card = card;
        let purpose = if applicable == Some(false) {
            card["materialRole"] = json!("CONTEXT_EVIDENCE_ONLY");
            "CONTEXT_EVIDENCE_ONLY"
        } else if asset["materialRole"] == "MATERIAL_SOURCE_ONLY" {
            "MATERIAL_SOURCE_ONLY"
        } else if applicable.is_none() {
            "LITERAL_IMPORTED_SCOPE"
        } else {
            "PRODUCTION_SCOPE"
        };
        purposes
            .push(json!({"assetId":asset["id"],"variantId":asset["variantId"],"purpose":purpose}));
        selected.push(card);
    }
    // Keep image lineage even if its original appearance is outside this role's
    // source range. It is explicitly a material parent, not a future shape.
    let mut cursor = 0;
    while cursor < selected.len() {
        if let Some(parent) = crate::pipeline::resolve_asset_dependency(assets, &selected[cursor])?
        {
            if !selected.iter().any(|card| card["id"] == parent) {
                let original = assets
                    .iter()
                    .find(|asset| asset["id"] == parent)
                    .ok_or("分镜来源父资产不在已绑定目录，未猜测形象来源")?;
                let mut card =
                    project_for_episode(input, &[original.clone()], episode_id)?.remove(0);
                card["materialRole"] = json!("MATERIAL_SOURCE_ONLY");
                purposes.push(json!({"assetId":card["id"],"variantId":card["variantId"],"purpose":"MATERIAL_SOURCE_ONLY"}));
                selected.push(card);
            }
        }
        cursor += 1;
    }
    let receipt = json!({"version":crate::pipeline::stage3_pack::SELECTOR_VERSION,"episodeId":episode_id,
        "sourceRefs":rows.iter().map(|row|&row.id).collect::<Vec<_>>(),"contextRefs":context.iter().map(|row|&row.id).collect::<Vec<_>>(),
        "productionSceneIds":scenes,"selectedAssets":purposes,
        "policy":"按正式集场和源锚取料；上下文与图片来源不授予当前入画或源事实。未知导入范围保留正文，不按姓名猜剪。"});
    Ok((selected, receipt))
}

/// The shared source binding is production metadata, not a claim of appearance
/// or a new source fact. Finishing inherits this same episode packet.
pub(super) fn attach_catalog_scope(target: &mut Value, input: &Value, episode_id: &str) {
    if scoped_profile(input) {
        target["assetCatalogScope"] = json!({
            "version":SCOPED_PROFILE,"episodeId":episode_id,"sourceSha256":input["source"]["sha256"],
            "policy":"资产scope仅投影本集适用场；MATERIAL_SOURCE_ONLY是图片来源，不证明入画。sourceScopeKind=PROGRAM_SCOPE绑定本包sourceSha256，仍是程序集场定位；sourceRefsPurpose=SOURCE_SCOPE不升格为SOURCE事实。别名只来自正式身份登记，不能按别名合并身份或回填当前状态。完整身份/形象适用范围由执行核验证。"
        });
    }
}

/// Mechanical projection of the old episode-upstream-material handoff:
/// canonical prompt bodies, production histories, URLs and local media paths
/// are not writing inputs. Source-bound visual facts continue through the
/// existing scoped facts/context packet; catalogue text never upgrades to facts.
pub(super) fn project(input: &Value, assets: &[Value]) -> Result<Vec<Value>, String> {
    if !validate_profile(input)? {
        return Ok(assets.to_vec());
    }
    assets
        .iter()
        .map(|asset| {
            let object = asset.as_object().ok_or("分镜资产目录必须为对象")?;
            let mut card: Map<String, Value> = [
                "id",
                "variantId",
                "identityId",
                "baseAssetId",
                "parentAssetId",
                "version",
                "name",
                "aliases",
                "catalogName",
                "kind",
                "type",
                "description",
                "role",
                "relationNote",
                "note",
                "preparation",
                "applicability",
                "scope",
                "sourceRefs",
                "sourceRefsPurpose",
                "sourceScope",
                "sourceAssetId",
                "sourceVariantId",
                "materialSourceAssetId",
                "materialSourceVariantId",
                "materialRole",
                // Some imported handoffs already carry explicit evidence cards.
                // Retain their source/claim semantics, never derive them from a prompt.
                "sourceFacts",
                "visualFacts",
            ]
            .iter()
            .filter_map(|key| object.get(*key).map(|value| ((*key).into(), value.clone())))
            .collect();
            if let Some(design) = adopted_design(input, asset)? {
                card.insert("adoptedDesign".into(), design);
            }
            Ok(Value::Object(card))
        })
        .collect()
}

fn adopted_design(input: &Value, asset: &Value) -> Result<Option<Value>, String> {
    let Some(design) = asset.get("adoptedDesign").and_then(Value::as_object) else {
        return Ok(None);
    };
    // A candidate prompt, successful image download, or an adopted media ID
    // alone is not an adopted text brief. No new summarization/adoption call.
    if design.get("provenance").and_then(Value::as_str) != Some("ADOPTED_DESIGN_NOT_SOURCE_FACT")
        || design.get("status").and_then(Value::as_str) == Some("STALE")
    {
        return Ok(None);
    }
    if design
        .get("sourceSha256")
        .is_some_and(|sha| sha != &input["source"]["sha256"])
        || design
            .get("assetVersion")
            .is_some_and(|version| version != &asset["version"])
    {
        return Ok(None);
    }
    if design.get("assetId").is_some_and(|id| id != &asset["id"])
        || design
            .get("variantId")
            .is_some_and(|id| id != &asset["variantId"])
    {
        return Err("已采用分镜设计简述的资产/形象身份不一致；未按姓名匹配".into());
    }
    let brief = design
        .get("brief")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());
    let Some(brief) = brief else { return Ok(None) };
    let mut projected = Map::new();
    projected.insert("brief".into(), Value::String(brief.into()));
    projected.insert(
        "provenance".into(),
        Value::String("ADOPTED_DESIGN_NOT_SOURCE_FACT".into()),
    );
    for field in ["version", "freeDesignScope"] {
        if let Some(value) = design.get(field) {
            projected.insert(field.into(), value.clone());
        }
    }
    Ok(Some(Value::Object(projected)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mac_role_selection_keeps_exact_material_parents_and_excludes_future_appearance() {
        let source = crate::pipeline::prepare_source(
            "第一集\n场景：门口\n△小王推门。\n场景：院内\n△小王换衣。\n",
        )
        .unwrap();
        let episode = &source.episodes[0].id;
        let now = &source.scenes[0].id;
        let future = &source.scenes[1].id;
        let parent = json!({"id":"formal-parent","variantId":"base","scope":{"kind":"SCENES","scenes":[{"episodeId":episode,"sceneId":future}]},"description":"正式父资产形象仅作材质来源"});
        let current = json!({"id":"formal-current","variantId":"current","sourceAssetId":"formal-parent","scope":{"kind":"SCENES","scenes":[{"episodeId":episode,"sceneId":now}]},"description":"当前适用形象"});
        let later = json!({"id":"formal-later","variantId":"future","scope":{"kind":"SCENES","scenes":[{"episodeId":episode,"sceneId":future}]},"description":"未来形象的细节不供当前主创"});
        let literal = json!({"id":"literal-import","variantId":"raw","scope":{"kind":"IMPORTED","rawCoordinates":["unknown"]}});
        let assets = vec![parent, current, later, literal];
        let mut input = json!({"source":source,"storyboardAssetInputProfile":SCOPED_PROFILE});
        crate::pipeline::stage3_pack::mark_input(&mut input).unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body" && row.scene_id.as_ref() == Some(now))
            .cloned()
            .collect();
        let (cards, receipt) = project_for_scope(&input, &assets, episode, &rows, &[]).unwrap();
        assert!(!cards.iter().any(|card| card["id"] == "formal-later"));
        assert!(cards.iter().any(|card| card["id"] == "formal-current"));
        assert!(cards
            .iter()
            .any(|card| card["id"] == "formal-parent"
                && card["materialRole"] == "MATERIAL_SOURCE_ONLY"));
        assert_eq!(
            cards
                .iter()
                .find(|card| card["id"] == "literal-import")
                .unwrap()["scope"],
            assets[3]["scope"]
        );
        assert_eq!(receipt["productionSceneIds"], json!([now]));
        assert_eq!(receipt["selectedAssets"].as_array().unwrap().len(), 3);
        input
            .as_object_mut()
            .unwrap()
            .remove(crate::pipeline::stage3_pack::PROFILE_FIELD);
        let (legacy, no_receipt) = project_for_scope(&input, &assets, episode, &rows, &[]).unwrap();
        assert_eq!(
            legacy,
            project_for_episode(&input, &assets, episode).unwrap()
        );
        assert!(no_receipt.is_null());
    }
    use serde_json::json;

    fn input() -> Value {
        json!({"storyboardAssetInputProfile":PROFILE,"source":{"sha256":"source-now"}})
    }

    #[test]
    fn card_preserves_canonical_scope_facts_and_dependencies_without_runtime_payloads() {
        let asset = json!({"id":"same:blue","variantId":"blue","identityId":"same","baseAssetId":"same","version":7,"name":"同名角色蓝衣","aliases":["同名角色"],"kind":"character","description":"目录说明","role":"REFERENCE","relationNote":"换装形象","preparation":{"mode":"INLINE","fromKey":"same:base","note":"本场沿基线参考"},"applicability":{"episodes":["e2"],"scenes":["s2"]},"scope":{"kind":"SCENES","episodes":["e2"],"scenes":[["e2","s2"]]},"sourceRefs":["r2"],"sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":{"kind":"PROGRAM_SCOPE","sourceRefs":["r2"]},"sourceAssetId":"same:base","sourceVariantId":"base","materialRole":"ACTUAL_APPEARANCE","visualFacts":[{"id":"f2","attribute":"衣服","value":"蓝衣","claim":"CHARACTER_CLAIM","sourceRefs":["r2"]}],"prompt":"PRIVATE_IMAGE_PROMPT","design":{"prompt":"PRIVATE_IMAGE_PROMPT","inputFingerprint":"PRIVATE_HISTORY"},"history":["PRIVATE_HISTORY"],"referenceImages":[{"url":"PRIVATE_URL","path":"PRIVATE_PATH"}],"adoptedMediaId":"downloaded-only","authorDecisions":["UNSCOPED_DECISION"]});
        let original = asset.clone();
        let card = project(&input(), &[asset]).unwrap().remove(0);
        for field in [
            "id",
            "variantId",
            "identityId",
            "baseAssetId",
            "version",
            "preparation",
            "scope",
            "applicability",
            "sourceRefs",
            "sourceRefsPurpose",
            "sourceScope",
            "sourceAssetId",
            "sourceVariantId",
            "visualFacts",
        ] {
            assert_eq!(
                card[field], original[field],
                "canonical field {field} must remain exact"
            );
        }
        let encoded = card.to_string();
        for private in [
            "PRIVATE_IMAGE_PROMPT",
            "PRIVATE_HISTORY",
            "PRIVATE_URL",
            "PRIVATE_PATH",
            "UNSCOPED_DECISION",
        ] {
            assert!(!encoded.contains(private));
        }
        assert!(
            card.get("adoptedDesign").is_none(),
            "downloaded media is not an adopted text brief"
        );
    }

    #[test]
    fn only_explicit_current_adopted_brief_is_production_design_not_source() {
        let mut asset = json!({"id":"a:base","variantId":"base","version":2,"designStatus":"candidate","design":{"prompt":"CANDIDATE","freeDesignScope":"CANDIDATE_SCOPE"},"adoptedDesign":{"assetId":"a:base","variantId":"base","assetVersion":2,"sourceSha256":"source-now","version":3,"brief":"已采用的黑发白衣简述。","freeDesignScope":"未锁定细节允许合理设计。","provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT","prompt":"PRIVATE_PROMPT","history":["PRIVATE_HISTORY"]}});
        let card = project(&input(), &[asset.clone()]).unwrap().remove(0);
        assert_eq!(
            card["adoptedDesign"],
            json!({"version":3,"brief":"已采用的黑发白衣简述。","freeDesignScope":"未锁定细节允许合理设计。","provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT"})
        );
        asset["adoptedDesign"]["sourceSha256"] = json!("source-old");
        assert!(project(&input(), &[asset.clone()]).unwrap()[0]
            .get("adoptedDesign")
            .is_none());
        asset["adoptedDesign"]["sourceSha256"] = json!("source-now");
        asset["adoptedDesign"]["assetVersion"] = json!(1);
        assert!(project(&input(), &[asset.clone()]).unwrap()[0]
            .get("adoptedDesign")
            .is_none());
        asset["adoptedDesign"]["assetVersion"] = json!(2);
        asset["adoptedDesign"]["variantId"] = json!("another-identity");
        assert!(project(&input(), &[asset]).is_err());
    }

    #[test]
    fn unmarked_and_null_historical_inputs_keep_the_exact_asset_json() {
        let assets = vec![
            json!({"id":"a","variantId":"base","prompt":"原冻结完整图片设计","design":{"prompt":"原冻结完整图片设计","history":[{"version":1}]},"unknownLegacyField":{"value":"历史材料"}}),
        ];
        let frozen = serde_json::to_string(&assets).unwrap();
        for input in [json!({}), json!({"storyboardAssetInputProfile":null})] {
            assert_eq!(
                serde_json::to_string(&project(&input, &assets).unwrap()).unwrap(),
                frozen
            );
        }
    }

    #[test]
    fn unknown_transport_profile_fails_before_a_model_can_be_called() {
        for input in [
            json!({"storyboardAssetInputProfile":"unknown/v9"}),
            json!({"storyboardAssetInputProfile":true}),
            json!({"storyboardAssetInputProfile":{}}),
        ] {
            assert!(validate_profile(&input).is_err());
            assert!(project(&input, &[]).is_err());
        }
    }

    fn scoped_input() -> Value {
        json!({"storyboardAssetInputProfile":SCOPED_PROFILE,"source":{"sha256":"source-now"},
            "foundation":{"assets":[{"id":"person","name":"正式身份","aliases":["旧称","正式身份"],
                "variants":[{"id":"base","assetRowId":"person:base","aliases":["基础别名"]}]}]}})
    }

    fn scoped_asset() -> Value {
        json!({"id":"person:base","identityId":"person","baseAssetId":"person","variantId":"base","version":7,
            "name":"正式身份｜基础","aliases":["已有别名","旧称"],"kind":"CH","sourceRefs":["r1","r2"],
            "scope":{"kind":"SCENES","reason":null,"scenes":[{"episodeId":"e1","sceneId":"s1"},{"episodeId":"e2","sceneId":"s2"}]},
            "applicability":{"episodes":["e1","e2"],"scenes":["s1","s2"]},
            "sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":{"kind":"PROGRAM_SCOPE","sourceSha256":"source-now"},
            "sourceAssetId":"parent:base","sourceVariantId":"base","preparation":{"mode":"INLINE","fromKey":"parent:base"},
            "visualFacts":[{"id":"f","claim":"CHARACTER_CLAIM","sourceRefs":["r1"],"value":"原有视觉事实"}]})
    }

    #[test]
    fn scoped_card_keeps_registered_identity_facts_and_material_parent_without_global_scope_repetition(
    ) {
        let input = scoped_input();
        let asset = scoped_asset();
        let original = asset.clone();
        let card = project_for_episode(&input, &[asset], "e1")
            .unwrap()
            .remove(0);
        assert_eq!(
            card["scope"],
            json!({"kind":"SCENES","reason":null,"scenes":[{"episodeId":"e1","sceneId":"s1"}]})
        );
        assert!(card.get("applicability").is_none());
        assert!(card.get("sourceScope").is_none());
        assert_eq!(card["sourceScopeKind"], "PROGRAM_SCOPE");
        assert_eq!(
            card["aliases"],
            json!(["已有别名", "旧称", "正式身份", "基础别名"])
        );
        for field in [
            "id",
            "variantId",
            "identityId",
            "baseAssetId",
            "version",
            "name",
            "sourceRefs",
            "sourceRefsPurpose",
            "sourceAssetId",
            "sourceVariantId",
            "preparation",
            "visualFacts",
        ] {
            assert_eq!(
                card[field], original[field],
                "{field} retains its canonical meaning"
            );
        }
        let mut request = json!({"sourceRows":[{"id":"r1","text":"原行逐字不变"}],"facts":original["visualFacts"],"assets":[card]});
        let body = request["sourceRows"].clone();
        let facts = request["facts"].clone();
        attach_catalog_scope(&mut request, &input, "e1");
        assert_eq!(request["assetCatalogScope"]["sourceSha256"], "source-now");
        assert_eq!(request["assetCatalogScope"]["episodeId"], "e1");
        assert_eq!(request["sourceRows"], body);
        assert_eq!(request["facts"], facts);
        assert_eq!(original, scoped_asset(), "saved asset is never reduced");
    }

    #[test]
    fn scoped_card_keeps_unknown_or_nonequivalent_scope_and_source_binding_literal() {
        let mut asset = scoped_asset();
        asset["scope"]["customEligibility"] = json!({"notCurrentState":true});
        asset["applicability"]["scenes"] = json!(["different-scene"]);
        asset["sourceScope"]["additionalEvidence"] = json!(["prior-r"]);
        let card = project_for_episode(&scoped_input(), &[asset.clone()], "e1")
            .unwrap()
            .remove(0);
        for field in ["scope", "applicability", "sourceScope"] {
            assert_eq!(card[field], asset[field]);
        }
        assert!(card.get("sourceScopeKind").is_none());
        asset["scope"]
            .as_object_mut()
            .unwrap()
            .remove("customEligibility");
        let card = project_for_episode(&scoped_input(), &[asset.clone()], "e1")
            .unwrap()
            .remove(0);
        assert_eq!(
            card["applicability"], asset["applicability"],
            "non-equivalent eligibility cannot be silently discarded"
        );
        asset["sourceScope"] = json!({"kind":"PROGRAM_SCOPE","sourceSha256":"another-source"});
        let card = project_for_episode(&scoped_input(), &[asset.clone()], "e1")
            .unwrap()
            .remove(0);
        assert_eq!(card["sourceScope"], asset["sourceScope"]);
        asset["scope"] = json!({"scenes":[],"reason":null});
        assert!(
            project_for_episode(&scoped_input(), &[asset], "e1").is_ok(),
            "unknown keys stay literal without panic"
        );
    }

    #[test]
    fn aliases_require_formal_identity_not_same_display_name_and_only_new_profile_inherits() {
        let input = scoped_input();
        let mut asset = scoped_asset();
        asset["identityId"] = json!("another");
        asset["baseAssetId"] = json!("another");
        let mut items = vec![asset.clone()];
        inherit_identity_aliases(&input, &mut items).unwrap();
        assert_eq!(
            items[0], asset,
            "same display name grants no alias inheritance"
        );
        asset["baseAssetId"] = json!("person");
        assert!(inherit_identity_aliases(&input, &mut [asset]).is_err());
        let mut old = input.clone();
        old["storyboardAssetInputProfile"] = json!(PROFILE);
        let asset = scoped_asset();
        let mut items = vec![asset.clone()];
        inherit_identity_aliases(&old, &mut items).unwrap();
        assert_eq!(items[0], asset);
        let mut duplicate = input.clone();
        let second = duplicate["foundation"]["assets"][0].clone();
        duplicate["foundation"]["assets"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(inherit_identity_aliases(&duplicate, &mut [scoped_asset()]).is_err());
    }

    #[test]
    fn v1_and_unmarked_requests_keep_exact_cards_and_have_no_new_scope_packet() {
        let asset = scoped_asset();
        for mut input in [
            json!({}),
            json!({"storyboardAssetInputProfile":null}),
            scoped_input(),
        ] {
            if input["storyboardAssetInputProfile"] == SCOPED_PROFILE {
                input["storyboardAssetInputProfile"] = json!(PROFILE);
            }
            let frozen = project(&input, &[asset.clone()]).unwrap();
            assert_eq!(
                project_for_episode(&input, &[asset.clone()], "e1").unwrap(),
                frozen
            );
            let mut request = json!({"assets":frozen,"sourceRows":[{"text":"原行"}]});
            let exact = request.to_string();
            attach_catalog_scope(&mut request, &input, "e1");
            assert_eq!(request.to_string(), exact);
        }
    }
}
