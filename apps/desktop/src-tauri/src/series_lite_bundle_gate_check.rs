//! Complete raw-row + Lingmo-v7 BundleGate adapter. Structured standard-source
//! packages are explicitly refused until their full node proof verifier exists.
use super::*;
use crate::series_lite_bundle::{check_mechanical, verify_source_lock, BundleGate, GateEvidence};
use crate::series_lite_registry::{check_asset_bindings, check_season_bundle};
use regex::Regex;
use sha2::{Digest, Sha256};
fn has(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim().is_empty() && !s.contains(['\r', '\0']))
}
fn arr<'a>(v: &'a Value, code: &str) -> Result<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| format!("{code}: expected array"))
}
fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn set_eq(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len()
        && a.iter()
            .enumerate()
            .all(|(i, v)| !a[..i].contains(v) && b.contains(v))
}
fn names(keys: &[Value], assets: &BTreeMap<&str, &Value>, kind: &str) -> Vec<Value> {
    keys.iter()
        .filter_map(|k| k.as_str().and_then(|k| assets.get(k)))
        .filter(|a| a["type"] == kind)
        .map(|a| a["name"].clone())
        .collect()
}
fn day(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        Regex::new(r"^(日|夜|晨|午|黄昏|待确认) (内|外|待确认)$")
            .unwrap()
            .is_match(s)
    })
}
pub struct NativeLegacyBundleGate<'a> {
    pub registry: Option<&'a Value>,
    pub source_lock: Option<&'a [u8]>,
    pub season_scope: Option<&'a Value>,
}
impl BundleGate for NativeLegacyBundleGate<'_> {
    fn check(&mut self, raw: &[u8]) -> Result<GateEvidence> {
        let bundle: Value = serde_json::from_slice(raw).map_err(|e| format!("BUNDLE_JSON: {e}"))?;
        need(
            bundle["version"] == "series-storyboard-bundle-v1",
            "BUNDLE_VERSION",
            "unknown bundle version",
        )?;
        let scope = self
            .season_scope
            .or_else(|| self.registry.and_then(|r| r.get("productionScope")));
        let season = check_season_bundle(&bundle, scope, false)?;
        need(
            bundle["exportProfile"] == "lingmo-v7",
            "EXPORT_PROFILE",
            "native adapter currently supports Lingmo v7 only",
        )?;
        need(
            bundle.get("generationTextVersion").is_none()
                || bundle["generationTextVersion"] == "voice-delivery-v1",
            "GENERATION_TEXT_VERSION",
            "unknown generation text strategy",
        )?;
        if bundle.get("sourceLockHash").is_some() {
            need(
                bundle["generationTextVersion"] == "voice-delivery-v1",
                "GENERATION_TEXT_VERSION",
                "locked source requires voice delivery policy",
            )?;
        }
        let episodes = arr(&bundle["episodes"], "SCOPE")?;
        let sources = episodes
            .iter()
            .map(|e| normalize_legacy_source(&e["source"]))
            .collect::<Result<Vec<_>>>()?;
        check_mechanical(&bundle, &sources)?;
        for a in bundle["assets"].as_array().unwrap() {
            need(
                a["name"].as_str().is_some_and(|n| !n.contains('\n')),
                "ASSET",
                "asset name must be single line",
            )?;
        }
        for scope in bundle["scope"].as_array().unwrap() {
            let ids = arr(&scope["sceneIds"], "SCOPE")?;
            need(
                !ids.is_empty()
                    && ids.iter().enumerate().all(|(i, id)| {
                        has(id) && !id.as_str().unwrap().contains('\n') && !ids[..i].contains(id)
                    }),
                "SCOPE",
                "invalid original scene labels",
            )?;
        }
        let source_fidelity = if bundle.get("sourceLockHash").is_none()
            && self.source_lock.is_none()
        {
            json!({"mode":"bundle-only","proof":"仅核bundle内部原文与分镜；未与创作前冻结源独立比对"})
        } else {
            let raw_lock = self
                .source_lock
                .ok_or("SOURCE_LOCK_REQUIRED: frozen source lock missing")?;
            let lock: Value =
                serde_json::from_slice(raw_lock).map_err(|e| format!("SOURCE_LOCK_HASH: {e}"))?;
            let lock_sources = arr(&lock["episodes"], "SOURCE_LOCK_SCOPE")?
                .iter()
                .map(|e| normalize_legacy_source(&e["source"]))
                .collect::<Result<Vec<_>>>()?;
            verify_source_lock(raw, raw_lock, &sources, &lock_sources)?
        };
        let visual_baseline = if let Some(b) = episodes
            .first()
            .and_then(|e| e["settings"].get("visualBaseline"))
        {
            json!({"mode":"adopted-baseline-v1","id":b["id"],"revision":b["revision"],"proof":"采用基准原文及覆盖依据一致；不证明美术质量、镜表无冲突或成片效果"})
        } else {
            json!({"mode":"legacy-unverified","proof":"旧稿保真兼容；没有采用视觉基准，未验证美术充分性及跨段共同基准"})
        };
        let assets: BTreeMap<_, _> = bundle["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| (a["key"].as_str().unwrap(), a))
            .collect();
        let mut stats = vec![];
        for (ei, e) in episodes.iter().enumerate() {
            let source = &sources[ei];
            let settings = &e["settings"];
            let plan = &e["plan"];
            let selection = &e["selection"];
            need(
                has(&settings["visualStyle"]) && has(&settings["prohibitions"]),
                "SETTINGS",
                "explicit visual style/prohibitions missing",
            )?;
            let candidates = arr(&plan["fantasyCandidates"], "PLAN")?;
            let selected = arr(&selection["selectedIds"], "PLAN")?;
            let mode = settings
                .get("fantasyMode")
                .unwrap_or(&json!("recommended"))
                .as_str()
                .unwrap_or("")
                .to_string();
            need(
                matches!(mode.as_str(), "off" | "recommended" | "all" | "review"),
                "FANTASY_MODE",
                "unknown fantasy mode",
            )?;
            if let Some(enabled) = settings.get("fantasyEnabled") {
                need(
                    enabled.as_bool() == Some(mode != "off"),
                    "FANTASY_MODE",
                    "legacy fantasy flag differs",
                )?;
            }
            let candidate_ids: Vec<_> = candidates.iter().map(|f| f["id"].clone()).collect();
            need(
                candidate_ids.iter().all(has)
                    && candidate_ids
                        .iter()
                        .enumerate()
                        .all(|(i, v)| !candidate_ids[..i].contains(v)),
                "FANTASY_CANDIDATE",
                "candidate ID missing/duplicate",
            )?;
            need(
                selected
                    .iter()
                    .enumerate()
                    .all(|(i, v)| !selected[..i].contains(v) && candidate_ids.contains(v)),
                "FANTASY_SELECTION",
                "selected candidate unknown/duplicate",
            )?;
            if mode == "off" {
                need(
                    selected.is_empty() && list(&e["draft"]["fantasyRealization"]).is_empty(),
                    "FANTASY_MODE",
                    "off cannot realize extra fantasy",
                )?;
            }
            if mode == "all" {
                need(
                    set_eq(selected, &candidate_ids),
                    "FANTASY_MODE",
                    "all must adopt every proposed candidate",
                )?;
            }
            for q in source["dialogue"].as_array().unwrap() {
                need(
                    matches!(
                        q["channel"].as_str(),
                        Some("现场对白" | "VO" | "OS" | "系统音")
                    ),
                    "CHANNEL",
                    "unsupported source channel",
                )?;
                need(
                    matches!(
                        q["kind"].as_str(),
                        Some("speech" | "roar" | "scream" | "vocalization")
                    ),
                    "VOICE_KIND",
                    "unsupported source voice kind",
                )?;
            }
            if e["source"].get("dialogue").is_some() {
                let mut raw_source = e["source"].clone();
                raw_source.as_object_mut().unwrap().remove("dialogue");
                let inferred = normalize_legacy_source(&raw_source)?;
                let sig = |s: &Value, fields: &[&str]| {
                    Value::Array(
                        s["dialogue"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|q| Value::Array(fields.iter().map(|k| q[*k].clone()).collect()))
                            .collect(),
                    )
                };
                need(
                    same(
                        &sig(source, &["speaker", "quote", "channel", "sourceIds"]),
                        &sig(&inferred, &["speaker", "quote", "channel", "sourceIds"]),
                    ),
                    "SOURCE_DIALOGUE_COVERAGE",
                    "explicit dialogue catalog omits/invents source voice",
                )?;
                need(
                    same(
                        &sig(
                            source,
                            &["cue", "kind", "originalChannel", "performanceCues"],
                        ),
                        &sig(
                            &inferred,
                            &["cue", "kind", "originalChannel", "performanceCues"],
                        ),
                    ),
                    "SOURCE_DIALOGUE_BINDING",
                    "explicit source voice cues differ",
                )?;
            }
            need(
                list(&source["diagnostics"]).is_empty(),
                "SOURCE_DIAGNOSTICS",
                "source has unresolved voice/action ambiguity",
            )?;
            let events = checked_events(plan, source)?;
            let mut used: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            for scene in e["scenes"].as_array().unwrap() {
                need(
                    day(&scene["dayInterior"]),
                    "DAY_INTERIOR",
                    "invalid day/interior",
                )?;
                need(
                    !arr(&scene["sourceRefs"], "SCENE_SOURCE")?.is_empty(),
                    "SCENE_SOURCE",
                    "scene source mapping cannot be empty",
                )?;
                let bindings = arr(&scene["bindings"], "SCENE_BINDINGS")?;
                need(
                    bindings.iter().enumerate().all(|(i, k)| {
                        !bindings[..i].contains(k)
                            && k.as_str().is_some_and(|k| assets.contains_key(k))
                    }),
                    "SCENE_BINDINGS",
                    "invalid scene binding",
                )?;
                used.insert(scene["id"].as_str().unwrap().into(), vec![]);
            }
            for u in e["draft"]["units"].as_array().unwrap() {
                let scene = e["scenes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| s["id"] == u["sceneId"])
                    .unwrap();
                if let Some(di) = u.get("dayInterior") {
                    need(day(di), "DAY_INTERIOR", "invalid unit day/interior")?;
                }
                let bindings = u["bindings"].as_array().unwrap();
                let collected = used.get_mut(u["sceneId"].as_str().unwrap()).unwrap();
                for k in bindings {
                    if !collected.contains(k) {
                        collected.push(k.clone());
                    }
                }
                need(
                    set_eq(&names(bindings, &assets, "SC"), &[u["scene"].clone()]),
                    "UNIT_BINDINGS",
                    "SC binding/name not closed",
                )?;
                need(
                    set_eq(&names(bindings, &assets, "PR"), list(&u["props"])),
                    "UNIT_BINDINGS",
                    "PR bindings/props not closed",
                )?;
                let subjects = list(&u["visualSubjects"]);
                if let Some(v) = u.get("visualSubjects") {
                    need(
                        valid_visual_subjects(v),
                        "VISUAL_SUBJECTS",
                        "invalid visual subjects",
                    )?;
                }
                let visual = u
                    .get("visualSubjectBindings")
                    .map(|v| arr(v, "VISUAL_BINDINGS"))
                    .transpose()?
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                need(
                    visual.len() == subjects.len(),
                    "VISUAL_BINDINGS",
                    "one explicit mapping per visual subject required",
                )?;
                let mut visual_ids = HashSet::new();
                for v in visual {
                    need(
                        v.as_object().is_some_and(|o| {
                            o.keys()
                                .all(|k| matches!(k.as_str(), "name" | "domain" | "assetKey"))
                        }) && subjects
                            .iter()
                            .any(|s| s["name"] == v["name"] && s["domain"] == v["domain"])
                            && visual_ids.insert((
                                v["name"].as_str().unwrap_or(""),
                                v["domain"].as_str().unwrap_or(""),
                            )),
                        "VISUAL_BINDINGS",
                        "unknown/duplicate visual mapping",
                    )?;
                    need(
                        v.get("assetKey") == Some(&Value::Null)
                            || v["assetKey"].as_str().is_some_and(|key| {
                                assets.get(key).is_some_and(|a| a["type"] == "CH")
                                    && bindings.contains(&v["assetKey"])
                            }),
                        "VISUAL_BINDINGS",
                        "visual target must be explicit bound CH or null",
                    )?;
                }
                let participants = list(&u["participants"]);
                need(
                    !subjects.iter().any(|s| participants.contains(&s["name"])),
                    "VISUAL_PRESENCE",
                    "visual-only subject cannot claim actual presence",
                )?;
                let characters = names(bindings, &assets, "CH");
                let media = u
                    .get("mediaSubjects")
                    .map(|v| arr(v, "MEDIA_SUBJECT"))
                    .transpose()?
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let mut media_ids = HashSet::new();
                for s in media {
                    need(
                        s.as_object().is_some_and(|o| {
                            o.keys()
                                .all(|k| matches!(k.as_str(), "name" | "medium" | "sourceRefs"))
                        }) && has(&s["name"])
                            && matches!(
                                s["medium"].as_str(),
                                Some("screen" | "photo" | "flashback")
                            ),
                        "MEDIA_SUBJECT",
                        "invalid media subject",
                    )?;
                    let refs = arr(&s["sourceRefs"], "MEDIA_SUBJECT")?;
                    need(
                        media_ids
                            .insert((s["name"].as_str().unwrap(), s["medium"].as_str().unwrap()))
                            && characters.contains(&s["name"])
                            && !refs.is_empty()
                            && refs.iter().enumerate().all(|(i, r)| {
                                !refs[..i].contains(r)
                                    && scene["sourceRefs"].as_array().unwrap().contains(r)
                            }),
                        "MEDIA_SUBJECT",
                        "media subject source/binding mismatch",
                    )?;
                }
                let mut voice = BTreeMap::new();
                for v in arr(&u["voiceBindings"], "VOICE_BINDING")? {
                    let a = v["assetKey"].as_str().and_then(|k| assets.get(k));
                    need(
                        a.is_some_and(|a| a["type"] == "CH") && bindings.contains(&v["assetKey"]),
                        "VOICE_BINDING",
                        "voice asset missing/non-CH/unbound",
                    )?;
                    let key = v["lineId"]
                        .as_str()
                        .ok_or("VOICE_BINDING: line ID missing")?;
                    need(
                        voice.insert(key, a.unwrap()["name"].clone()).is_none(),
                        "VOICE_BINDING",
                        "duplicate voice line",
                    )?;
                }
                let mut speakers = vec![];
                let mut voice_ids = HashSet::new();
                for shot in u["shots"].as_array().unwrap() {
                    for p in shot["dialogueParts"].as_array().unwrap() {
                        let q = source["dialogue"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|q| q["id"] == p["lineId"])
                            .unwrap();
                        if let Some(events) = events.as_ref().and_then(Value::as_array) {
                            if let Some(event) = events.iter().find(|e| e["id"] == u["eventId"]) {
                                need(
                                    q["sourceIds"].as_array().unwrap().iter().all(|r| {
                                        event["sourceRefs"].as_array().unwrap().contains(r)
                                    }),
                                    "DIALOGUE_EVENT",
                                    "quote source outside unit event",
                                )?;
                            }
                        }
                        let presentation = p["presentation"].as_str().unwrap_or("");
                        let allowed = match q["channel"].as_str().unwrap() {
                            "现场对白" => matches!(
                                presentation,
                                "画内" | "画外" | "画外转画内" | "画内转画外"
                            ),
                            "系统音" => matches!(presentation, "系统音" | "画外"),
                            channel => channel == presentation,
                        };
                        need(allowed, "CHANNEL", "source channel presentation changed")?;
                        if q["channel"] != "系统音" {
                            let qid = q["id"].as_str().unwrap();
                            let name = voice
                                .get(qid)
                                .ok_or("VOICE_BINDING: original line lacks voice asset")?;
                            if !speakers.contains(name) {
                                speakers.push(name.clone());
                            }
                            voice_ids.insert(qid);
                        }
                    }
                }
                need(
                    voice.keys().all(|id| voice_ids.contains(id)) && voice.len() == voice_ids.len(),
                    "VOICE_BINDING",
                    "voice bindings must exactly match consumed lines",
                )?;
                let visuals: Vec<_> = visual
                    .iter()
                    .filter_map(|v| {
                        v["assetKey"]
                            .as_str()
                            .map(|key| assets[key]["name"].clone())
                    })
                    .collect();
                let mut expected = vec![];
                for n in participants
                    .iter()
                    .chain(visuals.iter())
                    .chain(media.iter().map(|s| &s["name"]))
                    .chain(speakers.iter())
                {
                    if !expected.contains(n) {
                        expected.push(n.clone());
                    }
                }
                need(
                    set_eq(&characters, &expected),
                    "UNIT_BINDINGS",
                    "CH bindings do not close presence/media/voice/visual subjects",
                )?;
                for key in u["imageBindings"].as_array().unwrap() {
                    let a = assets[key.as_str().unwrap()];
                    need(
                        a["type"] != "CH"
                            || participants.contains(&a["name"])
                            || visuals.contains(&a["name"])
                            || media.iter().any(|s| s["name"] == a["name"]),
                        "IMAGE_BINDINGS",
                        "voice-only character cannot become image reference",
                    )?;
                }
                if let Some(segments) = u.get("segments") {
                    let segments = arr(segments, "SEGMENTS")?;
                    need(!segments.is_empty(), "SEGMENTS", "empty segments")?;
                    let mut shots = vec![];
                    for s in segments {
                        need(
                            ["title", "lighting", "initialState", "exitState"]
                                .iter()
                                .all(|k| has(&s[k]))
                                && s["shotIds"].as_array().is_some_and(|s| !s.is_empty()),
                            "SEGMENTS",
                            "missing segment fields/shots",
                        )?;
                        shots.extend(s["shotIds"].as_array().unwrap().iter().cloned());
                    }
                    need(
                        shots
                            == u["shots"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|s| s["id"].clone())
                                .collect::<Vec<_>>(),
                        "SEGMENTS",
                        "segment shots must cover once in order",
                    )?;
                }
            }
            for scene in e["scenes"].as_array().unwrap() {
                let actual = &used[scene["id"].as_str().unwrap()];
                need(
                    !actual.is_empty() && set_eq(scene["bindings"].as_array().unwrap(), actual),
                    "SCENE_BINDINGS",
                    "scene binding set differs from actual units",
                )?;
            }
            let report = validate_draft(
                &e["draft"],
                source,
                settings,
                Some(selection),
                plan,
                e["unitDirectionVersion"].as_str(),
            );
            if let Some(error) = report["errors"].as_array().unwrap().first() {
                return Err(format!(
                    "{}: {}",
                    error["code"].as_str().unwrap(),
                    report["errors"]
                ));
            }
            let mut row = report["stats"].clone();
            row["number"] = e["number"].clone();
            row["warnings"] = report["warnings"].clone();
            stats.push(row);
        }
        let production = bundle.get("sourceHash").is_some()
            || bundle.get("assetRegistry").is_some()
            || bundle["assets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a.get("assetId").is_some() || a.get("assetRevision").is_some());
        let registry_report =
            if bundle["testFixture"] != true || production || self.registry.is_some() {
                Some(check_asset_bindings(
                    &bundle,
                    self.registry
                        .ok_or("ASSET_REGISTRY_REQUIRED: current registry missing")?,
                )?)
            } else {
                None
            };
        let mut report = json!({"ok":true,"testFixture":bundle["testFixture"],"exportProfile":"lingmo-v7","episodes":stats,"sourceFidelity":source_fidelity,"visualBaseline":visual_baseline,"proof":"结构及源引用检查；冻结源证据见sourceFidelity；不证明画面语义、素材图片、平台导入或视频效果"});
        if season["legacy"] != true {
            report["season"] = season;
        }
        if let Some(r) = registry_report {
            report["assetRegistry"] = r;
        }
        Ok(GateEvidence {
            bundle_sha256: format!("{:x}", Sha256::digest(raw)),
            normalized_sources: sources,
            report,
            validator_id: "native-lite012-raw-rows-lingmo-v7".into(),
        })
    }
}
