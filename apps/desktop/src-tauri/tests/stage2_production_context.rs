//! Synthetic C/D input, ordering and replay proof. No provider/model dispatch.
use async_trait::async_trait;
use director_core::pipeline::{
    self, asset_design_contract as contract, asset_production_context as production, TextExecutor,
    TextRequest,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

fn fixture(new_profile: bool) -> Value {
    let source = pipeline::prepare_source("第一集\n场景：工坊\n△甲腕部佩戴标识物件。\n△乙拎起工具。\n场景：院外\n△甲走入共用院落。\n第二集\n场景：院外\n△后续院落出现积水。\n").unwrap();
    let scope = |indices: &[usize]| json!({"kind":"SCENES","scenes":indices.iter().map(|index| json!({"episodeId":source.scenes[*index].episode_id,"sceneId":source.scenes[*index].id})).collect::<Vec<_>>(),"reason":"合成准确形象范围"});
    let row = |needle: &str| {
        source
            .rows
            .iter()
            .find(|row| row.text.contains(needle))
            .unwrap()
            .id
            .clone()
    };
    let make = |id: &str, identity: &str, kind: &str, refs: Vec<String>, indices: &[usize]| json!({"id":id,"identityId":identity,"baseAssetId":identity,"variantId":"base","version":1,"name":id,"kind":kind,"description":"","sourceVariantId":null,"sourceRefs":refs,"scope":scope(indices),"applicability":{"episodes":indices.iter().map(|index|source.scenes[*index].episode_id.clone()).collect::<Vec<_>>(),"scenes":indices.iter().map(|index|source.scenes[*index].id.clone()).collect::<Vec<_>>()},"preparation":{"mode":"REFERENCE"}});
    let mut assets = vec![
        make("a-person", "person", "CH", vec![row("佩戴")], &[0]),
        make("b-other", "other", "CH", vec![row("拎起")], &[0]),
        make(
            "place",
            "yard",
            "LOC",
            vec![row("走入"), row("积水")],
            &[1, 2],
        ),
        make("z-prop", "object", "PROP", vec![row("佩戴")], &[0]),
    ];
    for id in ["independent-1", "independent-2", "independent-3"] {
        assets.push(make(id, id, "CH", vec![row("拎起")], &[0]));
    }
    let mut input = json!({"source":source,"assets":assets,"foundation":{"sourceSha256":source.sha256,"facts":[]},"settings":{"textProvider":"codex_cli"}});
    if new_profile {
        production::freeze(&mut input);
    }
    input
}

#[derive(Default)]
struct Designer {
    mode: &'static str,
    requests: Mutex<Vec<TextRequest>>,
    events: Mutex<Vec<String>>,
    accepted: Mutex<HashSet<String>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    calls: AtomicUsize,
    cache_hits: AtomicUsize,
    raw_cache: Mutex<BTreeMap<String, Value>>,
    replay: bool,
}

#[async_trait]
impl TextExecutor for Designer {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let key = serde_json::to_string(&request).unwrap();
        self.requests.lock().unwrap().push(request.clone());
        if self.replay {
            let result = self
                .raw_cache
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .ok_or("replay request bytes changed")?;
            self.cache_hits.fetch_add(1, Ordering::SeqCst);
            return Ok(result);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = if request.stage == "assetVisualFacts" {
            let rows = request.input["rows"].as_array().unwrap();
            let row = |needle: &str| {
                rows.iter()
                    .find(|row| row["text"].as_str().unwrap().contains(needle))
                    .map(|row| row["id"].clone())
            };
            let title = rows
                .iter()
                .find(|row| row["kind"] == "format" && row["sceneId"].is_string())
                .map(|row| row["id"].clone());
            let mut facts = Vec::new();
            for identity in request.input["identities"].as_array().unwrap() {
                let id = identity["assetId"].as_str().unwrap();
                let (attribute, value, scope, reference) = match id {
                    "person" => (
                        "accessories.item",
                        "腕部佩戴物件",
                        "STATE",
                        if self.mode == "title" {
                            title.clone()
                        } else {
                            row("佩戴")
                        },
                    ),
                    "object" => (
                        "form",
                        "佩戴在腕部的独立物件",
                        "STABLE",
                        if self.mode == "title" {
                            title.clone()
                        } else if self.mode == "scene-only" {
                            row("拎起")
                        } else {
                            row("佩戴")
                        },
                    ),
                    "other" => ("accessories.tool", "拎起工具", "STATE", row("拎起")),
                    "yard" => ("surface.state", "后续出现积水", "STATE", row("积水")),
                    _ => continue,
                };
                if let Some(reference) = reference {
                    facts.push(json!({"assetId":id,"variantId":if scope == "STABLE" {Value::Null} else {identity["versions"][0]["variantId"].clone()},"attribute":attribute,"value":value,"scope":scope,"claim":if self.mode == "claim" && id == "person" {"CHARACTER_CLAIM"} else {"SOURCE"},"sourceRefs":[reference]}));
                }
            }
            json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":facts,"issues":[]})
        } else {
            assert_eq!(request.stage, "assetDesign");
            let asset = &request.input["asset"];
            let id = asset["id"].as_str().unwrap();
            self.events.lock().unwrap().push(format!("start:{id}"));
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(if id == "z-prop" {
                45
            } else {
                8
            }))
            .await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.events.lock().unwrap().push(format!("finish:{id}"));
            let references = request.input["parentAsset"]["id"]
                .as_str()
                .into_iter()
                .collect::<Vec<_>>();
            json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":if id == "z-prop" {"独立物件固定形制与配色"} else {"当前主体完整制作设计"},"freeDesignScope":"普通美术取舍不变成原文事实","sourceRefs":[asset["sourceRefs"][0]],"referenceVariantIds":references})
        };
        self.raw_cache.lock().unwrap().insert(key, result.clone());
        Ok(result)
    }
    async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
        if request.stage == "assetDesign" {
            let id = value["assetId"].as_str().unwrap();
            self.accepted.lock().unwrap().insert(id.to_owned());
            self.events.lock().unwrap().push(format!("accept:{id}"));
        }
        Ok(())
    }
}

fn accepted_basis(mut input: Value, output: &Value) -> Value {
    for proposed in output["assets"].as_array().unwrap() {
        let row = input["assets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["id"] == proposed["id"])
            .unwrap();
        *row = proposed.clone();
    }
    input["foundation"]["facts"] = output["facts"].clone();
    input["foundation"]["visualFactBatches"] = output["visualFactBatches"].clone();
    input
}

#[tokio::test]
async fn accepted_non_title_shared_source_waits_only_related_prop_and_keeps_four_lanes_and_image_parent_graph(
) {
    let input = fixture(true);
    let frozen = input.to_string();
    let executor = Designer::default();
    let output = pipeline::run_asset_design(&input, &executor, "synthetic", "medium")
        .await
        .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    assert_eq!(output["designs"].as_array().unwrap().len(), 7);
    let requests = executor.requests.lock().unwrap();
    let request = |id: &str| {
        requests
            .iter()
            .find(|request| request.input["asset"]["id"] == id)
            .unwrap()
    };
    let context = &request("a-person").input["assetProductionContext"];
    assert_eq!(context["version"], production::PROFILE);
    assert_eq!(
        context["readSceneIds"],
        json!([]),
        "CH gets proven shared rows, not whole applicable scenes"
    );
    assert_eq!(
        request("z-prop").input["assetProductionContext"]["readSceneIds"],
        json!([]),
        "props keep existing evidence without expanding scenes"
    );
    assert_eq!(
        context["instructionsSha256"],
        pipeline::digest(production::INSTRUCTIONS)
    );
    assert_eq!(context["relatedProps"].as_array().unwrap().len(), 1);
    assert_eq!(context["relatedProps"][0]["assetId"], "z-prop");
    assert_eq!(
        context["relatedProps"][0]["acceptedDesign"]["prompt"],
        "独立物件固定形制与配色"
    );
    assert_eq!(
        context["relatedProps"][0]["relation"],
        "SOURCE_ROW_CANDIDATE_NOT_OWNER_FACT"
    );
    assert!(
        request("b-other").input["assetProductionContext"]["relatedProps"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let place = &request("place").input;
    assert_eq!(
        place["assetProductionContext"]["applicableScenes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        place["assetProductionContext"]["factEvidenceScenes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        place["facts"][0]["value"], "后续出现积水",
        "later state fact is preserved, not deleted"
    );
    assert!(
        place["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["text"].as_str().unwrap().contains("走入")),
        "C/D receives the initial applicable scene as well as later fact evidence"
    );
    let events = executor.events.lock().unwrap();
    let pos = |event: &str| events.iter().position(|item| item == event).unwrap();
    assert!(pos("accept:z-prop") < pos("start:a-person"));
    assert!(pos("start:b-other") < pos("accept:z-prop"));
    assert_eq!(executor.peak.load(Ordering::SeqCst), 4);
    assert!(
        output["imagePlan"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["dependencies"] == json!([])),
        "co-props never enter image references"
    );
    let saved = accepted_basis(input.clone(), &output);
    for asset in saved["assets"].as_array().unwrap() {
        assert!(
            contract::design_receipt_current(&saved, asset),
            "{}",
            asset["id"]
        );
        production::validate_consumed_prop_designs(
            &asset["design"],
            saved["assets"].as_array().unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        input.to_string(),
        frozen,
        "source/input/old history remain immutable"
    );
}

#[tokio::test]
async fn same_scene_titles_and_character_claims_do_not_become_prop_ownership_candidates() {
    for mode in ["scene-only", "title", "claim"] {
        let input = fixture(true);
        let executor = Designer {
            mode,
            ..Default::default()
        };
        let output = pipeline::run_asset_design(&input, &executor, "synthetic", "medium")
            .await
            .unwrap();
        assert!(
            output["blocked"].as_array().unwrap().is_empty(),
            "{mode}: {output}"
        );
        let requests = executor.requests.lock().unwrap();
        let person = requests
            .iter()
            .find(|request| request.input["asset"]["id"] == "a-person")
            .unwrap();
        assert_eq!(
            person.input["assetProductionContext"]["relatedProps"],
            json!([]),
            "{mode}"
        );
    }
}

#[tokio::test]
async fn prop_appearances_keep_explicit_scene_scope_and_exclude_future_derived_designs() {
    for same_scene in [false, true] {
        let mut input = fixture(true);
        let source: pipeline::LockedSource =
            serde_json::from_value(input["source"].clone()).unwrap();
        let scene = &source.scenes[if same_scene { 0 } else { 1 }];
        let mut damaged = input["assets"][3].clone();
        damaged["id"] = json!("zz-prop-damaged");
        damaged["variantId"] = json!("damaged");
        damaged["name"] = json!("同一物件破损形象");
        damaged["sourceAssetId"] = json!("z-prop");
        damaged["scope"] = json!({"kind":"SCENES","scenes":[{"episodeId":scene.episode_id,"sceneId":scene.id}],"reason":"准确子形象适用场"});
        damaged["applicability"] = json!({"episodes":[scene.episode_id],"scenes":[scene.id]});
        input["assets"]
            .as_array_mut()
            .unwrap()
            .push(damaged.clone());
        let frozen = input.to_string();
        let executor = Designer::default();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            pipeline::run_asset_design(&input, &executor, "synthetic", "medium"),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
        assert_eq!(output["designs"].as_array().unwrap().len(), 8);
        let requests = executor.requests.lock().unwrap();
        let character = requests
            .iter()
            .find(|request| request.input["asset"]["id"] == "a-person")
            .unwrap();
        let related = character.input["assetProductionContext"]["relatedProps"]
            .as_array()
            .unwrap();
        assert_eq!(related.len(), if same_scene { 2 } else { 1 });
        assert_eq!(related[0]["assetId"], "z-prop");
        assert_eq!(related[0]["scope"], input["assets"][3]["scope"]);
        assert_eq!(
            related[0]["applicability"],
            input["assets"][3]["applicability"]
        );
        if same_scene {
            assert_eq!(related[1]["assetId"], damaged["id"]);
            assert_eq!(related[1]["variantId"], "damaged");
            assert_eq!(related[1]["scope"], damaged["scope"]);
            assert_eq!(related[1]["applicability"], damaged["applicability"]);
            assert!(!related[1]["acceptedDesign"].is_null());
            let events = executor.events.lock().unwrap();
            let pos = |event: &str| events.iter().position(|item| item == event).unwrap();
            assert!(pos("accept:zz-prop-damaged") < pos("start:a-person"));
        } else {
            assert!(related.iter().all(|prop| prop["assetId"] != damaged["id"]));
        }
        let image_plan = output["imagePlan"].as_array().unwrap();
        assert_eq!(
            image_plan
                .iter()
                .find(|item| item["assetId"] == "a-person")
                .unwrap()["dependencies"],
            json!([])
        );
        assert_eq!(
            image_plan
                .iter()
                .find(|item| item["assetId"] == damaged["id"])
                .unwrap()["dependencies"],
            json!(["z-prop"]),
            "scene filtering never changes the existing parent image DAG"
        );
        assert_eq!(input.to_string(), frozen);
    }
}

#[tokio::test]
async fn prop_parent_cycle_omits_candidate_without_blocking_existing_parent_order() {
    let mut input = fixture(true);
    input["assets"][3]["sourceAssetId"] = json!("a-person");
    let executor = Designer::default();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        pipeline::run_asset_design(&input, &executor, "synthetic", "medium"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(), "{output}");
    let requests = executor.requests.lock().unwrap();
    let person = requests
        .iter()
        .find(|request| request.input["asset"]["id"] == "a-person")
        .unwrap();
    assert_eq!(
        person.input["assetProductionContext"]["relatedProps"],
        json!([])
    );
    let events = executor.events.lock().unwrap();
    let pos = |event: &str| events.iter().position(|item| item == event).unwrap();
    assert!(pos("accept:a-person") < pos("start:z-prop"));
    let prop = output["imagePlan"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["assetId"] == "z-prop")
        .unwrap();
    assert_eq!(
        prop["dependencies"],
        json!(["a-person"]),
        "original parent DAG wins"
    );
}

#[tokio::test]
async fn out_of_scope_prop_requires_current_source_receipt_and_never_adds_model_targets() {
    let input = fixture(true);
    let first = pipeline::run_asset_design(&input, &Designer::default(), "synthetic", "medium")
        .await
        .unwrap();
    let accepted = accepted_basis(input, &first);
    let legacy_input = fixture(false);
    let legacy_output =
        pipeline::run_asset_design(&legacy_input, &Designer::default(), "synthetic", "medium")
            .await
            .unwrap();
    let legacy_accepted = accepted_basis(legacy_input, &legacy_output);
    for mode in ["current", "legacy", "edited", "manual", "no-receipt"] {
        let mut selected = if mode == "legacy" {
            legacy_accepted.clone()
        } else {
            accepted.clone()
        };
        production::freeze(&mut selected);
        selected["selection"] = json!({"assetIds":["a-person"]});
        let prop = selected["assets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|prop| prop["id"] == "z-prop")
            .unwrap();
        match mode {
            "edited" => prop["prompt"] = json!("用户改变的物件设计"),
            "manual" => prop["promptOrigin"] = json!("MANUAL"),
            "no-receipt" => {
                prop["design"]
                    .as_object_mut()
                    .unwrap()
                    .remove("designReceipt");
            }
            _ => {}
        }
        let frozen_prop = selected["assets"][3].clone();
        let executor = Designer::default();
        let output = pipeline::run_asset_design(&selected, &executor, "synthetic", "medium")
            .await
            .unwrap();
        assert!(
            output["blocked"].as_array().unwrap().is_empty(),
            "{mode}: {output}"
        );
        assert_eq!(output["assets"].as_array().unwrap().len(), 1);
        let requests = executor.requests.lock().unwrap();
        let designs: Vec<_> = requests
            .iter()
            .filter(|request| request.stage == "assetDesign")
            .collect();
        assert_eq!(designs.len(), 1, "external prop is never auto-redesigned");
        let candidate = &designs[0].input["assetProductionContext"]["relatedProps"][0];
        assert_eq!(
            !candidate["acceptedDesign"].is_null(),
            ["current", "legacy"].contains(&mode),
            "{mode}"
        );
        assert_eq!(selected["assets"][3], frozen_prop);
        assert_eq!(output["imagePlan"][0]["dependencies"], json!([]));
        if mode == "current" {
            let design = &output["designs"][0];
            production::validate_consumed_prop_designs(
                design,
                selected["assets"].as_array().unwrap(),
            )
            .unwrap();
            let mut changed = selected["assets"].clone();
            changed[3]["prompt"] = json!("并行编辑了物件稿");
            assert!(production::validate_consumed_prop_designs(
                design,
                changed.as_array().unwrap()
            )
            .is_err());
            assert!(contract::validate_resume_facts(&selected, &output, &accepted).is_ok());
            let mut edited = accepted.clone();
            edited["assets"] = changed;
            assert!(contract::validate_resume_facts(&selected, &output, &edited).is_err());
        }
    }
}

#[tokio::test]
async fn old_and_new_frozen_requests_replay_exact_bytes_without_additional_synthetic_dispatch() {
    for new_profile in [false, true] {
        let input = fixture(new_profile);
        let frozen = input.to_string();
        let original = Designer::default();
        let first = pipeline::run_asset_design(&input, &original, "synthetic", "medium")
            .await
            .unwrap();
        assert!(first["blocked"].as_array().unwrap().is_empty());
        let raw_cache = original.raw_cache.into_inner().unwrap();
        let replay = Designer {
            replay: true,
            raw_cache: Mutex::new(raw_cache),
            ..Default::default()
        };
        let second = pipeline::run_asset_design(&input, &replay, "synthetic", "medium")
            .await
            .unwrap();
        assert!(second["blocked"].as_array().unwrap().is_empty(), "{second}");
        assert_eq!(first, second);
        assert_eq!(replay.calls.load(Ordering::SeqCst), 0);
        assert!(replay.cache_hits.load(Ordering::SeqCst) > 0);
        assert_eq!(input.to_string(), frozen);
        if !new_profile {
            assert!(replay
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|request| request.input.get("assetProductionContext").is_none()));
            assert!(first["designs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|design| design["designReceipt"]["basis"]
                    .get("assetProductionContext")
                    .is_none()));
        }
    }
}
