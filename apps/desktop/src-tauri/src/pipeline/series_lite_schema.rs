//! Strict response schemas for the frozen lite 0.4.1 desktop adapter.
//! Schemas expose creative fields, never source/settings/registry rewrites.
//! Domain gates still prove exact source, reference relations, patches and review.
use crate::series_lite_edit::{canonical, parse, sha256, Json};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;
pub(crate) const VERSION: &str = "desktop-lite-response-schema/v1";
pub(crate) const AUTHOR_RESPONSE_RULE: &str = "只输出plan/selection/draft三个字段。plan在同次完成事件、情绪合同、entryFacts和fantasyCandidates；不要写workPackages/provisionalUnits，程序由实际成稿反算。draft一次写完整声画，source/settings/scenes/assets由冻结基线保留，不回传或改写。所有已提供的源行、对白、场和资产引用必须使用schema内的实际ID；新事件/单元/镜头/候选ID在本次响应内一致关联。原声逐字、声道和源定顺序仍由原合同校验，不能通过删句、改源或空数组消除问题。无修订/未决/视觉主体时对应数组为空；普通小物留画面与状态，不另造正式PR键。segments仅在确有完整分段时使用非空完整分区。";
pub(crate) const FINISHER_RESPONSE_RULE: &str = "只输出baseSha256和operations。baseSha256使用当前完整基稿原字节摘要；每项op=set，path只能选提供的现有创作路径，expected必须逐值等于该路径当前旧值，value是本次局部修订。无具体改善返回operations:[]并沿用原稿。禁止修改source/settings/scenes/assets，禁止重交独立整集创作版本。替换units或连续单元时保留未改内容、原声和实际接续。程序索引workPackages/provisionalUnits不由模型改写；镜长/事件变化后由controller依据原派生证据重新编译索引。不能只删unresolved标签关闭正文问题。";
pub(crate) const REVIEWER_RESPONSE_RULE: &str = "只输出绑定本次bundleSha256的独审Report；sourceRowIds/unitIds必须按当前稿原顺序完整列出，逐行读源和逐单元读全稿，不能把ID列齐冒充实际核查。issues只写真实问题、位置、源依据和影响；priorIssues逐项保留或以当前版回查依据关闭，不代改正文。跨集边界以主控实际供给的前稿原字节、尾单元和当前首单元为准；未供前稿则pending，只有明确首集范围才not_applicable。报告pass不代替实际独立上下文证据或正式采用。";
pub(crate) fn response_rule(role: &str) -> Result<&'static str> {
    match role {
        "author" => Ok(AUTHOR_RESPONSE_RULE),
        "finisher" => Ok(FINISHER_RESPONSE_RULE),
        "reviewer" => Ok(REVIEWER_RESPONSE_RULE),
        _ => Err("LITE_SCHEMA:UNKNOWN_ROLE".into()),
    }
}
fn need(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("LITE_SCHEMA:{code}"))
    }
}
fn object(properties: Value) -> Value {
    json!({"type":"object","additionalProperties":false,
        "required":properties.as_object().expect("schema properties").keys().collect::<Vec<_>>(),"properties":properties})
}
fn text() -> Value {
    json!({"type":"string"})
}
fn nonempty() -> Value {
    json!({"type":"string","minLength":1})
}
fn number() -> Value {
    json!({"type":"number"})
}
fn array(items: Value) -> Value {
    json!({"type":"array","items":items})
}
fn nonempty_array(items: Value) -> Value {
    let mut v = array(items);
    v["minItems"] = json!(1);
    v
}
fn choices(values: &[String]) -> Value {
    if values.is_empty() {
        text()
    } else {
        json!({"type":"string","enum":values})
    }
}
fn fixed(value: &str) -> Value {
    json!({"type":"string","enum":[value]})
}
fn tags(values: &[&str]) -> Value {
    choices(&values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>())
}
fn refs(values: &[String]) -> Value {
    let mut v = array(choices(values));
    if values.is_empty() {
        v["maxItems"] = json!(0);
    }
    v
}
fn nullable(schema: Value) -> Value {
    json!({"anyOf":[schema,{"type":"null"}]})
}
fn nullable_choices(values: &[String]) -> Value {
    if values.is_empty() {
        json!({"type":"null"})
    } else {
        nullable(choices(values))
    }
}
fn reference(name: &str) -> Value {
    json!({"$ref":format!("#/$defs/{name}")})
}
fn range() -> Value {
    json!({"type":"array","items":{"type":"number","minimum":0},"minItems":2,"maxItems":2})
}
fn strings(value: &Json, key: &str) -> Result<Vec<String>> {
    value
        .array()?
        .iter()
        .map(|v| v.get(key)?.string().map(str::to_owned))
        .collect()
}
fn unique(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|v| seen.insert(v.clone()))
        .collect()
}

struct Pins {
    raw: Json,
    source_ids: Vec<String>,
    context_ids: Vec<String>,
    dialogue_ids: Vec<String>,
    scene_ids: Vec<String>,
    asset_keys: Vec<String>,
    image_keys: Vec<String>,
    character_keys: Vec<String>,
    character_names: Vec<String>,
    prop_names: Vec<String>,
    scene_names: Vec<String>,
    candidate_ids: Vec<String>,
    off: bool,
    cap: i128,
    spans: bool,
}
impl Pins {
    fn load(raw: &[u8]) -> Result<Self> {
        let base = parse(raw)?;
        need(
            base.get("version")?.string()? == "series-storyboard-bundle-v1",
            "BUNDLE_VERSION",
        )?;
        let episodes = base.get("episodes")?.array()?;
        need(episodes.len() == 1, "SINGLE_EPISODE_REQUIRED")?;
        let ep = &episodes[0];
        // Only the source is normalized to obtain ACTUAL compiled dialogue IDs.
        // Creative decimals stay in the exact parser and are never written back.
        let source: Value = serde_json::from_slice(&canonical(ep.get("source")?))
            .map_err(|_| "LITE_SCHEMA:SOURCE_JSON")?;
        let source = crate::series_lite_bundle_standard::normalize_source(&source)?;
        let normalized =
            parse(&serde_json::to_vec(&source).map_err(|_| "LITE_SCHEMA:SOURCE_JSON")?)?;
        let source_ids = strings(normalized.get("sourceRows")?, "id")?;
        need(!source_ids.is_empty(), "SOURCE_ROWS_REQUIRED")?;
        let context_ids = unique(
            source_ids
                .iter()
                .cloned()
                .chain(strings(normalized.get("contextRows")?, "id")?)
                .collect(),
        );
        let dialogue_ids = strings(normalized.get("dialogue")?, "id")?;
        let scene_ids = strings(ep.get("scenes")?, "id")?;
        need(!scene_ids.is_empty(), "SCENES_REQUIRED")?;
        let mut asset_keys = Vec::new();
        let mut image_keys = Vec::new();
        let mut character_keys = Vec::new();
        let mut character_names = Vec::new();
        let mut prop_names = Vec::new();
        let mut scene_names = Vec::new();
        for a in base.get("assets")?.array()? {
            let key = a.get("key")?.string()?.to_owned();
            let name = a.get("name")?.string()?.to_owned();
            match a.get("type")?.string()? {
                "CH" => {
                    character_keys.push(key.clone());
                    character_names.push(name);
                }
                "SC" => scene_names.push(name),
                "PR" => prop_names.push(name),
                _ => return Err("LITE_SCHEMA:ASSET_TYPE".into()),
            }
            if a.object()?.get("imageRequired") != Some(&Json::Bool(false)) {
                image_keys.push(key.clone());
            }
            asset_keys.push(key);
        }
        need(!asset_keys.is_empty(), "ASSETS_REQUIRED")?;
        need(!scene_names.is_empty(), "SCENE_ASSET_REQUIRED")?;
        let settings = ep.get("settings")?;
        let off = settings.get("fantasyMode")?.string()? == "off";
        let cap = settings
            .get("maxUnitSeconds")?
            .integer()
            .ok_or("LITE_SCHEMA:UNIT_CAP")?;
        need(matches!(cap, 15 | 30), "UNIT_CAP")?;
        let candidate_ids = ep
            .object()?
            .get("plan")
            .and_then(|p| p.object().ok())
            .and_then(|p| p.get("fantasyCandidates"))
            .and_then(|a| strings(a, "id").ok())
            .unwrap_or_default();
        let spans = ep
            .object()?
            .get("fantasyTimingVersion")
            .or_else(|| base.object().ok()?.get("fantasyTimingVersion"))
            .is_some_and(|v| v.string().ok() == Some("fantasy-spans-v1"))
            || ep
                .object()?
                .get("draft")
                .and_then(|d| d.object().ok())
                .and_then(|d| d.get("fantasyRealization"))
                .and_then(|r| r.array().ok())
                .is_some_and(|records| {
                    records.iter().any(|r| {
                        r.object().ok().is_some_and(|r| {
                            r.get("screenSpans").is_some()
                                || r.get("timingVersion")
                                    .is_some_and(|v| v.string().ok() == Some("fantasy-spans-v1"))
                        })
                    })
                });
        Ok(Self {
            raw: base,
            source_ids,
            context_ids,
            dialogue_ids,
            scene_ids,
            asset_keys: unique(asset_keys),
            image_keys: unique(image_keys),
            character_keys: unique(character_keys),
            character_names: unique(character_names),
            prop_names: unique(prop_names),
            scene_names: unique(scene_names),
            candidate_ids: unique(candidate_ids),
            off,
            cap,
            spans,
        })
    }
    fn episode(&self) -> Result<&Json> {
        Ok(&self.raw.get("episodes")?.array()?[0])
    }
}
fn boundary(unit: bool) -> Value {
    object(
        json!({"kind":if unit {tags(&["episode_start","continuous","capacity","new_event","time_space_jump"])}
        else {tags(&["episode_start","continuous","new_event","time_space_jump"])},"reason":nonempty()}),
    )
}
fn emotion() -> Value {
    object(
        json!({"subject":nonempty(),"contextTone":nonempty(),"audienceEffect":nonempty(),
        "change":nonempty(),"landing":nonempty(),"retain":nonempty_array(nonempty())}),
    )
}
fn day_interior() -> Value {
    let values: Vec<_> = ["日", "夜", "晨", "午", "黄昏", "待确认"]
        .iter()
        .flat_map(|day| {
            ["内", "外", "待确认"]
                .iter()
                .map(move |inside| format!("{day} {inside}"))
        })
        .collect();
    choices(&values)
}
fn definitions(p: &Pins, authoring: bool) -> Value {
    let selected = if authoring {
        text()
    } else {
        choices(&p.candidate_ids)
    };
    let event = object(
        json!({"id":nonempty(),"sourceRefs":nonempty_array(choices(&p.source_ids)),"sceneKey":nonempty(),
        "boundaryBefore":boundary(false),"emotionContract":emotion()}),
    );
    let entry = object(
        json!({"id":nonempty(),"subject":nonempty(),"statement":nonempty(),
        "sourceRefs":nonempty_array(choices(&p.context_ids)),"evidenceMode":fixed("source_rows")}),
    );
    let fantasy = object(
        json!({"id":nonempty(),"anchorRefs":nonempty_array(choices(&p.source_ids)),"subject":nonempty(),
        "emotionPurpose":nonempty(),"recommended":{"type":"boolean"},"proposal":nonempty(),
        "screenSecondsRange":range(),"extraSecondsRange":range()}),
    );
    let mut fantasies = array(reference("fantasyCandidate"));
    if p.off {
        fantasies["maxItems"] = json!(0);
    }
    let plan = object(
        json!({"schemaVersion":fixed("performance-core-v4-plan"),"unitPackingPolicy":fixed("event-first-v1"),
        "emotionArc":nonempty(),"events":nonempty_array(reference("event")),"entryFacts":array(reference("entryFact")),"fantasyCandidates":fantasies}),
    );
    let mut selected_ids = array(selected.clone());
    if p.off || (!authoring && p.candidate_ids.is_empty()) {
        selected_ids["maxItems"] = json!(0);
    }
    let selection = object(json!({"selectedIds":selected_ids}));
    let dialogue = object(json!({"lineId":choices(&p.dialogue_ids),"text":text(),
        "presentation":tags(&["画内","画外","VO","OS","系统音","画外转画内","画内转画外"]),
        "delivery":nonempty(),"voiceStart":{"type":"number","minimum":0},"voiceEnd":{"type":"number","minimum":0}}));
    let mut dialogue_parts = array(reference("dialoguePart"));
    if p.dialogue_ids.is_empty() {
        dialogue_parts["maxItems"] = json!(0);
    }
    let shot = object(
        json!({"id":nonempty(),"duration":{"type":"number","exclusiveMinimum":0,"maximum":p.cap as i64},
        "camera":nonempty(),"picture":nonempty(),"lighting":nonempty(),"sound":text(),"transition":text(),
        "sourceRefs":nonempty_array(choices(&p.source_ids)),"contextRefs":refs(&p.context_ids),"dialogueParts":dialogue_parts}),
    );
    let visual = object(json!({"name":nonempty(),"domain":tags(&["subjective","prospective"])}));
    let visual_binding = object(
        json!({"name":nonempty(),"domain":tags(&["subjective","prospective"]),"assetKey":nullable_choices(&p.character_keys)}),
    );
    let voice_binding = object(
        json!({"lineId":choices(&p.dialogue_ids),"assetKey":nullable_choices(&p.character_keys)}),
    );
    let media = object(
        json!({"name":nonempty(),"medium":tags(&["screen","photo","flashback"]),"sourceRefs":nonempty_array(choices(&p.source_ids))}),
    );
    let intent = object(
        json!({"subject":nonempty(),"change":nonempty(),"landing":nonempty(),"retain":nonempty_array(nonempty())}),
    );
    let segment = object(
        json!({"title":nonempty(),"unitDirection":nonempty(),"lighting":nonempty(),"initialState":nonempty(),"exitState":nonempty(),"shotIds":nonempty_array(nonempty())}),
    );
    let mut images = refs(&p.image_keys);
    images["maxItems"] = json!(if p.image_keys.is_empty() { 0 } else { 30 });
    let mut voices = array(reference("voiceBinding"));
    if p.dialogue_ids.is_empty() {
        voices["maxItems"] = json!(0);
    }
    let unit = object(
        json!({"id":nonempty(),"eventId":nonempty(),"boundaryBefore":boundary(true),"title":nonempty(),
        "sceneId":choices(&p.scene_ids),"scene":choices(&p.scene_names),"dayInterior":day_interior(),
        "bindings":nonempty_array(choices(&p.asset_keys)),"imageBindings":images,"voiceBindings":voices,
        "participants":refs(&p.character_names),"props":refs(&p.prop_names),"visualSubjects":array(reference("visualSubject")),
        "visualSubjectBindings":array(reference("visualSubjectBinding")),"mediaSubjects":array(reference("mediaSubject")),
        "unitDirection":nonempty(),"intent":reference("intent"),"lighting":nonempty(),"initialState":nonempty(),"exitState":nonempty(),
        "shots":nonempty_array(reference("shot"))}),
    );
    // Missing segments and nonempty complete segments are separate closed
    // variants; requiring []/null would conflict with the native SEGMENTS gate.
    let mut segmented = unit.clone();
    segmented["properties"]["segments"] = nonempty_array(reference("segment"));
    segmented = object(segmented["properties"].clone());
    let realization = object(
        json!({"candidateId":selected,"status":fixed("realized"),"unitIds":nonempty_array(nonempty()),
        "shotIds":nonempty_array(nonempty()),"screenSeconds":{"type":"number","exclusiveMinimum":0},
        "entry":nonempty(),"exit":nonempty(),"domain":nonempty(),"preservedFunction":nonempty()}),
    );
    let mut spans = realization.clone();
    for key in ["shotIds", "screenSeconds"] {
        spans["properties"].as_object_mut().unwrap().remove(key);
        spans["required"]
            .as_array_mut()
            .unwrap()
            .retain(|v| v != key);
    }
    spans["properties"]["screenSpans"] = nonempty_array(object(
        json!({"shotId":nonempty(),"range":nullable(range())}),
    ));
    spans["properties"]["timingVersion"] = fixed("fantasy-spans-v1");
    spans = object(spans["properties"].clone());
    let mut spans_with_indices = spans.clone();
    for field in ["shotIds", "screenSeconds"] {
        spans_with_indices["properties"][field] = realization["properties"][field].clone();
    }
    spans_with_indices = object(spans_with_indices["properties"].clone());
    let mut realizations = array(reference(if p.spans {
        "fantasySpans"
    } else {
        "fantasyRealization"
    }));
    if p.off || (!authoring && p.candidate_ids.is_empty()) {
        realizations["maxItems"] = json!(0);
    }
    let revision = object(
        json!({"coordinate":nonempty(),"sourceRefs":refs(&p.source_ids),"change":nonempty(),"preservedFunction":nonempty()}),
    );
    let unresolved = object(json!({"coordinate":nonempty(),"problem":nonempty()}));
    let draft = object(
        json!({"schemaVersion":fixed("performance-core-v4"),"units":nonempty_array(reference("unit")),
        "fantasyRealization":realizations,"revisionNotes":array(reference("revisionNote")),"unresolved":array(reference("unresolved"))}),
    );
    json!({"event":event,"entryFact":entry,"fantasyCandidate":fantasy,"plan":plan,"selection":selection,
        "dialoguePart":dialogue,"shot":shot,"visualSubject":visual,"visualSubjectBinding":visual_binding,"voiceBinding":voice_binding,
        "mediaSubject":media,"intent":intent,"segment":segment,"unitPlain":unit,"unitSegmented":segmented,
        "unit":{"anyOf":[reference("unitPlain"),reference("unitSegmented")]},"fantasyRealization":realization,
        "fantasySpansOnly":spans,"fantasySpansWithIndices":spans_with_indices,
        "fantasySpans":{"anyOf":[reference("fantasySpansOnly"),reference("fantasySpansWithIndices")]},
        "revisionNote":revision,"unresolved":unresolved,"draft":draft})
}

pub(crate) fn author(base_raw: &[u8]) -> Result<Value> {
    let p = Pins::load(base_raw)?;
    let mut result = object(
        json!({"plan":reference("plan"),"selection":reference("selection"),"draft":reference("draft")}),
    );
    result["$defs"] = definitions(&p, true);
    Ok(result)
}

// Expected-value schemas preserve the CURRENT old tree's real keys and scalar
// kinds, including unknown creative metadata. They are shape constraints only;
// patch_bytes independently proves old-value equality and the raw base hash.
fn shape(value: &Json) -> Value {
    match value {
        Json::Null => json!({"type":"null"}),
        Json::Bool(_) => json!({"type":"boolean"}),
        Json::Number { .. } => number(),
        Json::String(_) => text(),
        Json::Object(o) => object(Value::Object(
            o.iter().map(|(k, v)| (k.clone(), shape(v))).collect(),
        )),
        Json::Array(a) => {
            let mut variants = BTreeMap::new();
            for item in a {
                let v = shape(item);
                variants.insert(v.to_string(), v);
            }
            let mut out = array(match variants.len() {
                0 => text(),
                1 => variants.into_values().next().unwrap(),
                _ => json!({"anyOf":variants.into_values().collect::<Vec<_>>()}),
            });
            out["minItems"] = json!(a.len());
            out["maxItems"] = json!(a.len());
            out
        }
    }
}
fn replacement(p: &Pins, path: &[String], old: &Json, defs: &Value) -> Value {
    let last = path.last().map(String::as_str).unwrap_or("");
    if path.len() == 3 {
        return reference(last);
    }
    let named = match last {
        "events" => Some(("event", true)),
        "entryFacts" => Some(("entryFact", false)),
        "fantasyCandidates" => Some(("fantasyCandidate", false)),
        "shots" => Some(("shot", true)),
        "units" => Some(("unit", true)),
        "dialogueParts" => Some(("dialoguePart", false)),
        "voiceBindings" => Some(("voiceBinding", false)),
        "visualSubjects" => Some(("visualSubject", false)),
        "visualSubjectBindings" => Some(("visualSubjectBinding", false)),
        "mediaSubjects" => Some(("mediaSubject", false)),
        "segments" => Some(("segment", true)),
        "revisionNotes" => Some(("revisionNote", false)),
        "unresolved" => Some(("unresolved", false)),
        _ => None,
    };
    if let Some((name, required)) = named {
        let mut result = if required {
            nonempty_array(reference(name))
        } else {
            array(reference(name))
        };
        if (last == "fantasyCandidates" && p.off)
            || (last == "dialogueParts" && p.dialogue_ids.is_empty())
            || (last == "voiceBindings" && p.dialogue_ids.is_empty())
        {
            result["maxItems"] = json!(0);
        }
        return result;
    }
    match last {
        "sourceRefs" | "anchorRefs" => return refs(&p.source_ids),
        "contextRefs" => return refs(&p.context_ids),
        "bindings" => return nonempty_array(choices(&p.asset_keys)),
        "imageBindings" => {
            let mut a = refs(&p.image_keys);
            a["maxItems"] = json!(if p.image_keys.is_empty() { 0 } else { 30 });
            return a;
        }
        "participants" => return refs(&p.character_names),
        "props" => return refs(&p.prop_names),
        "selectedIds" => {
            let mut a = refs(&p.candidate_ids);
            if p.off {
                a["maxItems"] = json!(0);
            }
            return a;
        }
        "lineId" => return choices(&p.dialogue_ids),
        "assetKey" => return nullable_choices(&p.character_keys),
        "schemaVersion" => {
            return fixed(if path[2] == "plan" {
                "performance-core-v4-plan"
            } else {
                "performance-core-v4"
            });
        }
        "unitPackingPolicy" => return fixed("event-first-v1"),
        "evidenceMode" => return fixed("source_rows"),
        "medium" => return tags(&["screen", "photo", "flashback"]),
        "presentation" => {
            return tags(&[
                "画内",
                "画外",
                "VO",
                "OS",
                "系统音",
                "画外转画内",
                "画内转画外",
            ])
        }
        "sceneId" => return choices(&p.scene_ids),
        "scene" => return choices(&p.scene_names),
        "dayInterior" => return day_interior(),
        "emotionContract" => return emotion(),
        "intent" => return reference("intent"),
        "boundaryBefore" => return boundary(path.iter().any(|s| s == "units")),
        "duration" => return json!({"type":"number","exclusiveMinimum":0,"maximum":p.cap as i64}),
        "voiceStart" | "voiceEnd" => return json!({"type":"number","minimum":0}),
        "fantasyRealization" => return defs["draft"]["properties"]["fantasyRealization"].clone(),
        _ => {}
    }
    if path.len() == 5 && path[3] == "units" {
        return reference("unit");
    }
    if path.len() == 7 && path[5] == "shots" {
        return reference("shot");
    }
    shape(old)
}
pub(crate) fn finisher(base_raw: &[u8]) -> Result<Value> {
    let p = Pins::load(base_raw)?;
    let mut defs = definitions(&p, false);
    let mut groups = BTreeMap::<String, (Value, Value, Vec<String>)>::new();
    fn collect(
        p: &Pins,
        value: &Json,
        path: &mut Vec<String>,
        defs: &Value,
        groups: &mut BTreeMap<String, (Value, Value, Vec<String>)>,
    ) -> Result<()> {
        if path.len() >= 4
            && path[2] == "plan"
            && ["workPackages", "provisionalUnits"].contains(&path[3].as_str())
        {
            return Ok(());
        }
        // A default finishing pass cannot replace all creative roots as a
        // disguised new full episode. Existing fields/units remain patchable.
        if path.len() > 3 {
            let expected = shape(value);
            let replacement = replacement(p, path, value, defs);
            let key = json!([expected, replacement]).to_string();
            let pointer = format!(
                "/{}",
                path.iter()
                    .map(|s| s.replace('~', "~0").replace('/', "~1"))
                    .collect::<Vec<_>>()
                    .join("/")
            );
            groups
                .entry(key)
                .or_insert_with(|| (expected, replacement, vec![]))
                .2
                .push(pointer);
        }
        match value {
            Json::Object(o) => {
                for (k, v) in o {
                    path.push(k.clone());
                    collect(p, v, path, defs, groups)?;
                    path.pop();
                }
            }
            Json::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    path.push(i.to_string());
                    collect(p, v, path, defs, groups)?;
                    path.pop();
                }
            }
            _ => {}
        }
        Ok(())
    }
    for field in ["plan", "selection", "draft"] {
        let current = p.episode()?.get(field)?;
        need(
            !current.object()?.is_empty(),
            "FINISHER_NEEDS_COMPLETE_CANDIDATE",
        )?;
        collect(
            &p,
            current,
            &mut vec!["episodes".into(), "0".into(), field.into()],
            &defs,
            &mut groups,
        )?;
    }
    let mut operations = Vec::new();
    for (index, (_, (expected, replacement, paths))) in groups.into_iter().enumerate() {
        let name = format!("expected{index}");
        defs[&name] = expected;
        operations.push(object(json!({"op":fixed("set"),"path":choices(&paths),"expected":reference(&name),"value":replacement})));
    }
    need(!operations.is_empty(), "PATCH_TARGETS_REQUIRED")?;
    let mut out = object(
        json!({"baseSha256":fixed(&sha256(base_raw)),"operations":array(json!({"anyOf":operations}))}),
    );
    out["$defs"] = defs;
    Ok(out)
}

pub(crate) fn reviewer(bundle_raw: &[u8]) -> Result<Value> {
    let p = Pins::load(bundle_raw)?;
    let units = strings(p.episode()?.get("draft")?.get("units")?, "id")?;
    need(!units.is_empty(), "REVIEW_NEEDS_COMPLETE_CANDIDATE")?;
    let mut schema = crate::series_lite_review::schema();
    schema["properties"]["bundleSha256"] = fixed(&sha256(bundle_raw));
    for (field, ids) in [("sourceRowIds", &p.source_ids), ("unitIds", &units)] {
        let mut exact_scope = refs(ids);
        exact_scope["minItems"] = json!(ids.len());
        exact_scope["maxItems"] = json!(ids.len());
        schema["properties"][field] = exact_scope;
    }
    for field in ["issues", "closedIssues"] {
        schema["properties"][field]["items"]["properties"]["unitIds"] = refs(&units);
        schema["properties"][field]["items"]["properties"]["sourceRefs"] = refs(&p.source_ids);
    }
    let boundary = &mut schema["properties"]["boundary"]["properties"];
    boundary["previousBundleSha256"] =
        nullable(json!({"type":"string","pattern":"^[0-9a-f]{64}$"}));
    boundary["currentUnitId"] = nullable(fixed(&units[0]));
    // previousUnitId/hash cannot be fabricated from this bundle. The controller
    // pins them from BoundaryInput and review::validate proves exact scope/order.
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series_lite_bundle::{export_lingmo_v7, verify_lingmo_v7, BundleGate};
    use crate::series_lite_bundle_gate::NativeLegacyBundleGate;
    use crate::series_lite_edit::{compose_derived_bytes, patch_bytes};

    fn corpus() -> Value {
        serde_json::from_str(include_str!("../series_lite_execution_golden.json")).unwrap()
    }
    fn fixture() -> Value {
        let mut b: Value =
            serde_json::from_str(corpus()["files"]["bundle.json"].as_str().unwrap()).unwrap();
        b["generationBoundaryVersion"] = json!("video-prompt-v1");
        b["episodes"][0]["settings"]["fantasyMode"] = json!("off");
        b
    }
    fn no_empty_open_objects(v: &Value) {
        if v["type"] == "object" {
            assert_eq!(v["additionalProperties"], false);
            assert!(
                v["properties"].as_object().is_some_and(|p| !p.is_empty()),
                "empty object erases creative fields"
            );
            let expected: BTreeSet<_> = v["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            let actual: BTreeSet<_> = v["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect();
            assert_eq!(actual, expected);
        }
        if let Some(o) = v.as_object() {
            for (key, item) in o {
                if !["enum", "default"].contains(&key.as_str()) {
                    no_empty_open_objects(item);
                }
            }
        }
        if let Some(a) = v.as_array() {
            for item in a {
                no_empty_open_objects(item);
            }
        }
    }
    #[test]
    fn lite041_schemas_pin_actual_normalized_ids_and_survive_real_transport_schema_conversion() {
        let base = serde_json::to_vec(&fixture()).unwrap();
        for schema in [
            author(&base).unwrap(),
            finisher(&base).unwrap(),
            reviewer(&base).unwrap(),
        ] {
            no_empty_open_objects(&schema);
            assert_eq!(
                sha256(
                    crate::text_executor::structured_schema_for_budget(&schema)
                        .to_string()
                        .as_bytes()
                ),
                sha256(schema.to_string().as_bytes())
            );
            assert_eq!(crate::text_executor::sdk_strict_schema(&schema), schema);
        }
        let schema = author(&base).unwrap();
        assert_eq!(
            schema["$defs"]["dialoguePart"]["properties"]["lineId"]["enum"],
            json!(["E01:R2:D1", "E01:R4:D2"])
        );
        assert_eq!(
            schema["$defs"]["shot"]["properties"]["sourceRefs"]["items"]["enum"],
            json!(["E01:R1", "E01:R2", "E01:R3", "E01:R4"])
        );
        assert_eq!(
            schema["$defs"]["unitPlain"]["properties"]["sceneId"]["enum"],
            json!(["1-1", "1-2"])
        );
        assert_eq!(
            schema["$defs"]["unitPlain"]["properties"]["bindings"]["items"]["enum"],
            json!(["值守者·面罩CH", "门厅SC", "走廊SC", "铜钥匙PR"])
        );
        assert!(schema["$defs"]["plan"]["properties"]
            .get("workPackages")
            .is_none());
        assert!(schema["$defs"]["plan"]["properties"]
            .get("provisionalUnits")
            .is_none());
        assert!(schema["properties"].get("source").is_none());
        assert_eq!(
            schema["$defs"]["plan"]["properties"]["fantasyCandidates"]["maxItems"],
            0
        );
        assert!(AUTHOR_RESPONSE_RULE.contains("不要写workPackages/provisionalUnits"));
        assert!(response_rule("unknown").is_err());
    }
    #[test]
    fn lite041_schema_uses_verified_standard_dialogue_ids_instead_of_legacy_id_guessing() {
        let standard: Value = serde_json::from_str(include_str!(
            "../series_lite_bundle_standard_delivery_golden.json"
        ))
        .unwrap();
        let mut base: Value = serde_json::from_str(standard[0]["raw"].as_str().unwrap()).unwrap();
        base["episodes"][0]["settings"]["fantasyMode"] = json!("off");
        let raw = serde_json::to_vec(&base).unwrap();
        let schema = author(&raw).unwrap();
        // This standard source assigns D1 to the system line. Legacy extraction
        // in the other fixture assigns D2; neither spelling is guessed here.
        assert_eq!(
            schema["$defs"]["dialoguePart"]["properties"]["lineId"]["enum"],
            json!(["E01:R2:D1", "E01:R4:D1"])
        );
        assert_eq!(
            schema["$defs"]["shot"]["properties"]["sourceRefs"]["items"]["enum"],
            standard[0]["sources"][0]["sourceRows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["id"].clone())
                .collect::<Value>()
        );
        let mut invalid = base;
        invalid["episodes"][0]["source"]["standardPackage"]["sourceHash"] = json!("a".repeat(64));
        assert!(author(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    #[test]
    fn lite041_patch_schema_binds_base_allows_noop_and_exposes_only_existing_creative_paths() {
        let base = serde_json::to_vec(&fixture()).unwrap();
        let schema = finisher(&base).unwrap();
        assert_eq!(
            schema["properties"]["baseSha256"]["enum"],
            json!([sha256(&base)])
        );
        let no_change = json!({"baseSha256":sha256(&base),"operations":[]});
        let r = patch_bytes(&base, &serde_json::to_vec(&no_change).unwrap()).unwrap();
        assert!(!r.changed);
        assert_eq!(r.bytes, base);
        let branches = schema["properties"]["operations"]["items"]["anyOf"]
            .as_array()
            .unwrap();
        let paths: Vec<_> = branches
            .iter()
            .flat_map(|b| b["properties"]["path"]["enum"].as_array().unwrap())
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(paths.contains(&"/episodes/0/draft/units/0/shots/0/picture"));
        assert!(paths.contains(&"/episodes/0/draft/units"));
        assert!(paths.iter().all(|p| [
            "/episodes/0/plan",
            "/episodes/0/selection",
            "/episodes/0/draft"
        ]
        .iter()
        .any(|prefix| p.starts_with(prefix))));
        assert!(!paths.iter().any(|p| p.contains("workPackages")
            || p.contains("provisionalUnits")
            || p.starts_with("/episodes/0/source")));
        let old = fixture()["episodes"][0]["draft"]["units"][0]["shots"][0]["picture"].clone();
        let patch = json!({"baseSha256":sha256(&base),"operations":[{"op":"set","path":"/episodes/0/draft/units/0/shots/0/picture","expected":old,"value":"局部合成测试画面"}]});
        assert!(patch_bytes(&base, &serde_json::to_vec(&patch).unwrap()).is_ok());
        let mut drift = patch;
        drift["operations"][0]["expected"] = json!("错误基稿值");
        assert!(patch_bytes(&base, &serde_json::to_vec(&drift).unwrap()).is_err());
    }
    #[test]
    fn lite041_review_schema_and_real_verifier_bind_current_candidate_full_scope_and_boundary() {
        let bundle = serde_json::to_vec(&fixture()).unwrap();
        let schema = reviewer(&bundle).unwrap();
        assert_eq!(
            schema["properties"]["bundleSha256"]["enum"],
            json!([sha256(&bundle)])
        );
        assert_eq!(schema["properties"]["sourceRowIds"]["minItems"], 4);
        assert_eq!(schema["properties"]["unitIds"]["minItems"], 2);
        let mut report = json!({"version":crate::series_lite_review::VERSION,"bundleSha256":sha256(&bundle),"sourceRowIds":["E01:R1","E01:R2","E01:R3","E01:R4"],"unitIds":["U1","U2"],"issues":[],"unresolved":[],"closedIssues":[],"boundary":{"status":"not_applicable","previousBundleSha256":null,"previousUnitId":null,"currentUnitId":null,"basis":"合成首集，无前集边界"},"verdict":"pass","summary":"合成测试已读全部源与两单元，不证明真实模型独审。"});
        assert!(
            crate::series_lite_review::validate(
                &serde_json::to_vec(&report).unwrap(),
                &bundle,
                &sha256(&bundle),
                &[],
                crate::series_lite_review::BoundaryInput::FirstInScope
            )
            .unwrap()
            .declared_content_pass
        );
        report["sourceRowIds"] = json!(["E01:R2", "E01:R1", "E01:R3", "E01:R4"]);
        assert!(crate::series_lite_review::validate(
            &serde_json::to_vec(&report).unwrap(),
            &bundle,
            &sha256(&bundle),
            &[],
            crate::series_lite_review::BoundaryInput::FirstInScope
        )
        .is_err());
    }
    #[test]
    fn lite041_author_shape_keeps_real_creative_fields_for_derived_compose_and_native_delivery_gates(
    ) {
        let mut b = fixture();
        let e = &mut b["episodes"][0];
        let events = vec![
            json!({"id":"EV1","sourceRefs":["E01:R1","E01:R2"],"sceneKey":"1-1","boundaryBefore":{"kind":"episode_start","reason":"源首场"},"emotionContract":{"subject":"值守者","contextTone":"戒备","audienceEffect":"克制提醒","change":"等待到提醒","landing":"继续等门","retain":["面罩与持物保持"]}}),
            json!({"id":"EV2","sourceRefs":["E01:R3","E01:R4"],"sceneKey":"1-2","boundaryBefore":{"kind":"new_event","reason":"原源转入空走廊"},"emotionContract":{"subject":"系统声","contextTone":"等待","audienceEffect":"收到确认","change":"等待到确认","landing":"无人走廊保持","retain":["系统声不具象化"]}}),
        ];
        e["plan"] = json!({"schemaVersion":"performance-core-v4-plan","unitPackingPolicy":"event-first-v1","emotionArc":"等待至确认","events":events,"entryFacts":[],"fantasyCandidates":[]});
        e["draft"]["revisionNotes"] = json!([]);
        for i in 0..2 {
            e["draft"]["units"][i]["eventId"] = events[i]["id"].clone();
            e["draft"]["units"][i]["boundaryBefore"] = events[i]["boundaryBefore"].clone();
            e["draft"]["units"][i]["dayInterior"] = json!("夜 内");
            e["draft"]["units"][i]["mediaSubjects"] = json!([]);
            e["draft"]["units"][i]["visualSubjectBindings"] = json!([]);
        }
        let creative = json!({"plan":e["plan"],"selection":e["selection"],"draft":e["draft"]});
        for field in ["plan", "selection", "draft"] {
            e[field] = json!({});
        }
        let base = serde_json::to_vec(&b).unwrap();
        let schema = author(&base).unwrap();
        no_empty_open_objects(&schema);
        let result = compose_derived_bytes(
            &base,
            &sha256(&base),
            &serde_json::to_vec(&creative).unwrap(),
        )
        .unwrap();
        assert!(result.derived_bookkeeping);
        let registry: Value =
            serde_json::from_str(corpus()["files"]["registry/v000001.json"].as_str().unwrap())
                .unwrap();
        let lock = corpus()["files"]["source-lock.json"]
            .as_str()
            .unwrap()
            .as_bytes()
            .to_vec();
        let mut gate = NativeLegacyBundleGate {
            registry: Some(&registry),
            source_lock: Some(&lock),
            season_scope: None,
        };
        gate.check(&result.edit.bytes).unwrap();
        let delivery = export_lingmo_v7(&result.edit.bytes, &mut gate).unwrap();
        verify_lingmo_v7(
            &result.edit.bytes,
            &delivery.markdown,
            &delivery.manifest_json,
            &mut gate,
        )
        .unwrap();
    }
}
