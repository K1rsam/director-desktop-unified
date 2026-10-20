use super::*;

fn fixture() -> Value {
    let source = prepare_source("第一集\n场景：门口\n△门口有台阶。\n△一盆绿植被飞石劈开。\n△甲从门缝向外看。\n第二集\n场景：门口\n△远处另一栋楼的门口堆着木箱。\n").unwrap();
    let marker = source
        .rows
        .iter()
        .find(|row| row.kind == "format" && row.scene_id.is_some())
        .unwrap();
    let fact = source
        .rows
        .iter()
        .find(|row| row.text.contains("台阶"))
        .unwrap();
    json!({
        "source":source,"assets":[{"id":"door:base","baseAssetId":"door","variantId":"base","kind":"LOC","name":"门口","sourceRefs":[marker.id],"sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":{"kind":"PROGRAM_SCOPE","sourceSha256":source.sha256},"scope":{"kind":"SCENES","scenes":[{"episodeId":marker.episode_id,"sceneId":marker.scene_id}]},"preparation":{"mode":"GENERATE","note":"盆栽是配套环境；破裂仅本场变化。"}}],
        "foundation":{"sourceSha256":source.sha256,"facts":[{"id":"stairs","assetId":"door","variantId":"base","attribute":"结构","value":"台阶","scope":"STABLE","claim":"SOURCE","sourceRefs":[fact.id]}]}
    })
}

#[test]
fn partial_fact_scene_keeps_exact_scene_and_source_bytes() {
    let mut input = fixture();
    let original = input.clone();
    let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
    let asset = &original["assets"][0];
    let facts = original["foundation"]["facts"].as_array().unwrap();
    let ids: HashSet<_> = [
        asset["sourceRefs"][0].as_str().unwrap(),
        facts[0]["sourceRefs"][0].as_str().unwrap(),
    ]
    .into_iter()
    .collect();
    assert!(scene_context(&input, &source, asset, facts, &ids)
        .unwrap()
        .is_empty());
    freeze_design_evidence_profile(&mut input);
    let rows = scene_context(&input, &source, &input["assets"][0], facts, &ids).unwrap();
    assert!(rows.iter().any(|row| row.text.contains("绿植")));
    assert!(rows.iter().any(|row| row.text.contains("门缝")));
    assert!(
        !rows.iter().any(|row| row.text.contains("木箱")),
        "same display scene name must not join a different episode"
    );
    assert!(rows.iter().all(
        |row| source
            .rows
            .iter()
            .any(|original| serde_json::to_value(original).unwrap()
                == serde_json::to_value(row).unwrap())
    ));
    assert_eq!(input["source"], original["source"]);
    assert_eq!(input["foundation"], original["foundation"]);
    assert_eq!(input["assets"], original["assets"]);
}

#[test]
fn legacy_absent_null_and_bad_profile_never_silently_upgrade() {
    let input = fixture();
    assert_eq!(
        design_evidence_binding(&input, &input["assets"][0]).unwrap(),
        None
    );
    let mut absent = input.clone();
    absent[DESIGN_EVIDENCE_PROFILE_FIELD] = Value::Null;
    assert_eq!(
        design_evidence_binding(&absent, &absent["assets"][0]).unwrap(),
        None
    );
    let mut frozen = input.clone();
    freeze_design_evidence_profile(&mut frozen);
    for key in ["version", "instructions", "instructionsSha256"] {
        let mut changed = frozen.clone();
        changed[DESIGN_EVIDENCE_PROFILE_FIELD][key] = json!("changed");
        assert!(design_evidence_binding(&changed, &changed["assets"][0]).is_err());
    }
    frozen["assets"][0]["scope"]["scenes"][0]["episodeId"] = json!("wrong-episode");
    assert!(design_evidence_binding(&frozen, &frozen["assets"][0])
        .unwrap_err()
        .contains("集场坐标"));
}

#[test]
fn global_location_and_many_scene_character_do_not_read_whole_script() {
    let mut input = fixture();
    freeze_design_evidence_profile(&mut input);
    input["assets"][0]["scope"] = json!({"kind":"GLOBAL"});
    let global = design_evidence_binding(&input, &input["assets"][0])
        .unwrap()
        .unwrap();
    assert_eq!(global["readScenes"].as_array().unwrap().len(), 1);
    input["assets"][0]["kind"] = json!("CH");
    input[DESIGN_EVIDENCE_PROFILE_FIELD] = json!({"version":LEGACY_DESIGN_EVIDENCE_PROFILE,
        "instructions":INSTRUCTIONS,"instructionsSha256":digest(INSTRUCTIONS)});
    let character = design_evidence_binding(&input, &input["assets"][0])
        .unwrap()
        .unwrap();
    assert!(character["readScenes"].as_array().unwrap().is_empty());
    assert_eq!(character["evidenceRowIds"].as_array().unwrap().len(), 2);
    freeze_design_evidence_profile(&mut input);
    let current = design_evidence_binding(&input, &input["assets"][0]).unwrap().unwrap();
    assert!(current["readScenes"].as_array().unwrap().len() <= 1);
    assert!(current["evidenceRowIds"].as_array().unwrap().len() <= CHARACTER_CONTEXT_ROWS + 2);
    let source: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
    let unread = source.rows.iter().find(|row|row.text.contains("木箱")).unwrap();
    assert!(!current["evidenceRowIds"].as_array().unwrap().contains(&json!(unread.id)));
}

#[test]
fn binding_changes_with_consumed_text_not_unread_scene() {
    let mut input = fixture();
    freeze_design_evidence_profile(&mut input);
    let before = design_evidence_binding(&input, &input["assets"][0]).unwrap();
    let mut changed = input.clone();
    let rows = changed["source"]["rows"].as_array_mut().unwrap();
    rows.iter_mut()
        .find(|row| row["text"].as_str().unwrap().contains("绿植"))
        .unwrap()["text"] = json!("△一盆兰花被飞石劈开。");
    // Even with an unchanged claimed source SHA a receipt binds actual rows.
    assert_ne!(
        before,
        design_evidence_binding(&changed, &changed["assets"][0]).unwrap()
    );
    let mut unrelated = input.clone();
    unrelated["source"]["rows"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["text"].as_str().unwrap().contains("木箱"))
        .unwrap()["text"] = json!("△木箱挪走。");
    assert_eq!(
        before,
        design_evidence_binding(&unrelated, &unrelated["assets"][0]).unwrap()
    );
}

#[tokio::test]
async fn unrelated_broken_parent_does_not_block_healthy_branch() {
    for kind in ["LOC", "CH"] {
        let mut input = fixture();
        freeze_design_evidence_profile(&mut input);
        input["assets"][0]["kind"] = json!(kind);
        let expected = design_evidence_binding(&input, &input["assets"][0]).unwrap();
        let mut bad = input["assets"][0].clone();
        bad["id"] = json!("broken");
        bad["baseAssetId"] = json!("unrelated");
        bad["sourceAssetId"] = json!("missing");
        input["assets"].as_array_mut().unwrap().push(bad);
        assert_eq!(
            design_evidence_binding(&input, &input["assets"][0]).unwrap(),
            expected
        );
        assert!(design_evidence_binding(&input, &input["assets"][1]).is_err());
        input["selection"] = json!({"assetIds":["door:base"]});
        let executor = Designer::default();
        let result = run_asset_design(&input, &executor, "offline", "medium")
            .await
            .unwrap();
        assert!(result["blocked"].as_array().unwrap().is_empty(), "{result}");
        assert_eq!(executor.requests.lock().unwrap().len(), 1);
    }
}

#[derive(Default)]
struct Designer {
    requests: std::sync::Mutex<Vec<Value>>,
}
#[async_trait]
impl TextExecutor for Designer {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        if request.stage == "assetVisualFacts" {
            return Ok(
                json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":[],"issues":[]}),
            );
        }
        assert_eq!(request.stage, "assetDesign");
        self.requests.lock().unwrap().push(request.input.clone());
        let asset = &request.input["asset"];
        let refs: Vec<_> = request.input["parentAsset"]["id"]
            .as_str()
            .into_iter()
            .collect();
        Ok(
            json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":"可制作的完整美术方案","freeDesignScope":"普通美术补足不是原文事实","sourceRefs":asset["sourceRefs"],"referenceVariantIds":refs}),
        )
    }
}

#[tokio::test]
async fn new_request_binds_full_scene_and_completion_without_rewriting_notes_or_facts() {
    let mut input = fixture();
    let old = Designer::default();
    run_asset_design(&input, &old, "offline", "medium")
        .await
        .unwrap();
    let old_wire = old.requests.lock().unwrap()[0].clone();
    assert!(old_wire.get("designEvidenceContext").is_none());
    freeze_design_evidence_profile(&mut input);
    let before = input.clone();
    let new = Designer::default();
    let result = run_asset_design(&input, &new, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(input, before);
    assert!(result["blocked"].as_array().unwrap().is_empty(), "{result}");
    let wire = new.requests.lock().unwrap()[0].clone();
    assert_eq!(
        wire["asset"]["preparation"],
        input["assets"][0]["preparation"]
    );
    assert_eq!(wire["facts"], old_wire["facts"]);
    let text = wire["evidence"].to_string();
    assert!(text.contains("绿植") && text.contains("门缝") && !text.contains("木箱"));
    assert_eq!(
        wire["designEvidenceContext"]["version"],
        DESIGN_EVIDENCE_PROFILE
    );
    let mut rows = wire["evidence"].clone();
    for row in rows.as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("evidenceUses");
    }
    assert_eq!(
        wire["designEvidenceContext"]["evidenceSha256"],
        digest(&rows.to_string())
    );
    assert_eq!(
        result["designs"][0]["frozenContext"]["designEvidenceContext"]["evidenceSha256"],
        wire["designEvidenceContext"]["evidenceSha256"]
    );
    let legacy = Designer::default();
    let mut replay = input;
    replay
        .as_object_mut()
        .unwrap()
        .remove(DESIGN_EVIDENCE_PROFILE_FIELD);
    run_asset_design(&replay, &legacy, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(
        legacy.requests.lock().unwrap()[0],
        old_wire,
        "legacy body/cache input stays byte-equivalent"
    );
}

#[tokio::test]
async fn unknown_costume_note_kept_with_priority_manual_donor_and_parent_chain() {
    let mut input = fixture();
    input["assets"][0]["kind"] = json!("CH");
    input["assets"][0]["preparation"]["note"] =
        json!("原文未明确日常基线服装，不补造款式。禁止品牌标识。");
    input["assets"][0]["prompt"] = json!("用户写的白衣、布鞋，不带品牌标识。");
    input["assets"][0]["promptOrigin"] = json!("MANUAL");
    let mut child = input["assets"][0].clone();
    child["id"] = json!("door:coat");
    child["variantId"] = json!("coat");
    child["sourceAssetId"] = json!("door:base");
    child["prompt"] = json!("");
    child["promptOrigin"] = Value::Null;
    input["assets"].as_array_mut().unwrap().push(child);
    input["selection"] = json!({"assetIds":["door:coat"]});
    input["automaticDesign"] = json!(true);
    freeze_design_evidence_profile(&mut input);
    let original = input.clone();
    let executor = Designer::default();
    let result = run_asset_design(&input, &executor, "offline", "medium")
        .await
        .unwrap();
    assert!(result["blocked"].as_array().unwrap().is_empty(), "{result}");
    assert_eq!(input, original);
    let requests = executor.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        request["asset"]["preparation"]["note"],
        input["assets"][1]["preparation"]["note"]
    );
    assert!(request["designEvidenceContext"]["instructions"]
        .as_str()
        .unwrap()
        .contains("表示未知，不等于用户禁止普通美术设计"));
    assert!(request["designEvidenceContext"]["instructions"]
        .as_str()
        .unwrap()
        .contains("不取消其中的具体明示约束"));
    assert_eq!(
        request["parentDesign"]["prompt"],
        original["assets"][0]["prompt"]
    );
    assert_eq!(
        result["designs"][0]["referenceAssetIds"],
        json!(["door:base"])
    );
    assert!(
        result["assets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|asset| asset["id"] != "door:base"),
        "a manual donor must not appear in the changed asset delta"
    );
}

#[tokio::test]
async fn invalid_scene_scope_only_blocks_its_own_branch() {
    let mut input = fixture();
    freeze_design_evidence_profile(&mut input);
    let mut bad = input["assets"][0].clone();
    bad["id"] = json!("broken-scope");
    bad["baseAssetId"] = json!("other-place");
    bad["scope"]["scenes"][0]["episodeId"] = json!("wrong-episode");
    input["assets"].as_array_mut().unwrap().push(bad);
    let executor = Designer::default();
    let output = run_asset_design(&input, &executor, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(output["designs"].as_array().unwrap().len(), 1);
    assert_eq!(output["designs"][0]["assetId"], "door:base");
    assert_eq!(output["blocked"].as_array().unwrap().len(), 1);
    assert_eq!(output["blocked"][0]["assetId"], "broken-scope");
    assert!(output["blocked"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("集场坐标"));
    assert_eq!(executor.requests.lock().unwrap().len(), 1);
}
