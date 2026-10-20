use super::*;
use super::super::asset_production_context as production;
use std::sync::Mutex;

struct FactExtractor;

#[async_trait]
impl TextExecutor for FactExtractor {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        assert_eq!(request.stage, "assetVisualFacts");
        let rows = request.input["rows"].as_array().unwrap();
        let shared = &rows.iter().find(|row| text(row, "text").contains("雕塑和铜钟")).unwrap()["id"];
        let separate = &rows.iter().find(|row| text(row, "text").contains("另一角")).unwrap()["id"];
        let facts: Vec<_> = request.input["identities"].as_array().unwrap().iter().filter_map(|identity| {
            let id = text(identity, "assetId");
            let (row, value) = match id {
                "room" => (shared, "展室摆有烛龙雕塑和铜钟"),
                "statue" => (shared, "烛龙雕塑"),
                "bell" => (shared, "铜钟"),
                "separate" => (separate, "石鼎"),
                _ => return None,
            };
            Some(json!({"assetId":id,"variantId":null,"attribute":"外观与陈设","value":value,"scope":"STABLE","claim":"SOURCE","sourceRefs":[row]}))
        }).collect();
        Ok(json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":facts,"issues":[]}))
    }
}

fn freeze_v2(input: &mut Value) {
    input[production::PROFILE_FIELD] = json!({"version":production::PREVIOUS_PROFILE,"instructions":production::PREVIOUS_INSTRUCTIONS,"instructionsSha256":digest(production::PREVIOUS_INSTRUCTIONS)});
}

#[test]
fn scene_prop_old_instruction_bytes_and_unknown_profile_remain_locked() {
    // These hashes were read from the pre-change source at 3b7df2e, not
    // regenerated from today's constants. Historical frozen receipts use them.
    assert_eq!(digest(production::LEGACY_INSTRUCTIONS),"918274f96c798388a74c1f022d34146a885ae2134fd469f12b8e1955c6c443e1");
    assert_eq!(digest(production::PREVIOUS_INSTRUCTIONS),"acc62cb26ee5254e714b0ec895db962c0f5dd07346fd04df40237e2b39fff6f4");
    let mut input = json!({});
    freeze_v2(&mut input);
    let frozen = input.clone();
    assert_eq!(production::frozen_version(&input).unwrap(),Some("asset-production-context/v2"));
    assert_eq!(input,frozen,"reading old frozen input must not select v3");
    input[production::PROFILE_FIELD]["instructions"] = json!(production::INSTRUCTIONS);
    input[production::PROFILE_FIELD]["instructionsSha256"] = json!(digest(production::INSTRUCTIONS));
    assert!(production::frozen_version(&input).is_err(),"v3 instructions cannot be relabeled v2");
}

async fn fixture() -> Value {
    let source = prepare_source("第一集\n场景：展室\n△展室里摆着烛龙雕塑和铜钟。\n△同一展室另一角放着石鼎。\n").unwrap();
    let shared = source.rows.iter().find(|row| row.text.contains("雕塑和铜钟")).unwrap();
    let separate = source.rows.iter().find(|row| row.text.contains("另一角")).unwrap();
    let asset = |id: &str, owner: &str, kind: &str, row: &SourceRow| json!({
        "id":id,"baseAssetId":owner,"variantId":"base","name":id,"kind":kind,"sourceRefs":[row.id],
        "scope":{"kind":"SCENES","scenes":[{"episodeId":row.episode_id,"sceneId":row.scene_id}]},
        "applicability":{"episodes":[row.episode_id],"scenes":[row.scene_id]},"preparation":{"mode":"REFERENCE"}
    });
    let mut input = json!({"source":source,"assets":[
        asset("a-room","room","LOC",shared),asset("z-statue","statue","PROP",shared),
        asset("y-bell","bell","PROP",shared),asset("x-separate","separate","PROP",separate),
        asset("b-unrelated","unrelated","LOC",separate)
    ],"foundation":{"sourceSha256":source.sha256,"facts":[]}});
    production::freeze(&mut input);
    let visual = super::super::asset_visual_facts::run_asset_visual_facts(&input,&FactExtractor,"offline","medium").await.unwrap();
    assert!(visual["blocked"].as_array().unwrap().is_empty(), "{visual}");
    input["foundation"]["facts"] = visual["facts"].clone();
    input["foundation"]["visualFactBatches"] = visual["visualFactBatches"].clone();
    input
}

#[derive(Default)]
struct Designer {
    requests: Mutex<Vec<Value>>,
    accepted: Mutex<HashSet<String>>,
    events: Mutex<Vec<String>>,
    fail_statue: bool,
}

#[async_trait]
impl TextExecutor for Designer {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        assert_eq!(request.stage,"assetDesign","accepted A15 facts must be reused");
        let asset = &request.input["asset"];
        let id = text(asset,"id");
        self.events.lock().unwrap().push(format!("start:{id}"));
        self.requests.lock().unwrap().push(request.input.clone());
        if id == "z-statue" {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            if self.fail_statue { return Err("offline prop branch failed".into()); }
        }
        Ok(json!({"schemaVersion":"asset-design/v1","assetId":id,"variantId":asset["variantId"],"prompt":format!("{id}已接受的原文约束设计"),"freeDesignScope":"中性光照，陈设仅按源关系复用已接受设计","sourceRefs":asset["sourceRefs"],"referenceVariantIds":[]}))
    }

    async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
        if request.stage == "assetDesign" {
            let id = text(value,"assetId");
            if id == "z-statue" { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
            self.accepted.lock().unwrap().insert(id.to_owned());
            self.events.lock().unwrap().push(format!("accept:{id}"));
        }
        Ok(())
    }
}

#[tokio::test]
async fn scene_prop_v3_candidates_require_accepted_shared_body_evidence_not_scene_or_name() {
    let input = fixture().await;
    let before = input.clone();
    let candidates = production::candidates(&input).unwrap();
    let ids: Vec<_> = candidates["a-room"].iter().map(|p|text(p,"assetId")).collect();
    assert_eq!(ids,vec!["y-bell","z-statue"],"several proven row candidates remain distinct, not inferred as one object");
    assert!(candidates["b-unrelated"].is_empty(),"same scene without shared accepted fact evidence is insufficient");
    let context = production::context(&input,&input["assets"][0]).unwrap().unwrap();
    assert_eq!(context["version"],production::PROFILE);
    assert!(context["instructions"].as_str().unwrap().contains("同场、共行、名称相近"));
    for candidate in context["relatedProps"].as_array().unwrap() {
        assert!(candidate["acceptedDesign"].is_null());
        assert_eq!(candidate["disposition"],"RELATION_ONLY_UNTIL_VALID_DESIGN");
        assert_eq!(candidate["relation"],"SOURCE_ROW_CANDIDATE_NOT_OWNER_FACT");
        assert!(!candidate["locationFactIds"].as_array().unwrap().is_empty());
        assert!(candidate.get("characterFactIds").is_none());
    }
    let mut unreceipted = input.clone();
    unreceipted["foundation"]["visualFactBatches"] = json!([]);
    assert!(production::candidates(&unreceipted).unwrap()["a-room"].is_empty());
    let mut claimed = input.clone();
    for fact in claimed["foundation"]["facts"].as_array_mut().unwrap() { fact["claim"] = json!("CHARACTER_CLAIM"); }
    assert!(production::candidates(&claimed).unwrap()["a-room"].is_empty());
    assert_eq!(input,before);
}

#[tokio::test]
async fn scene_prop_v3_waits_for_all_prop_acceptances_without_adding_images_or_parent_edges() {
    let input = fixture().await;
    let before = input.clone();
    let executor = Designer::default();
    let output = run_asset_design(&input,&executor,"offline","medium").await.unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty(),"{output}");
    assert_eq!(output["designs"].as_array().unwrap().len(),5);
    assert_eq!(output["imagePlan"].as_array().unwrap().len(),5);
    let requests = executor.requests.lock().unwrap();
    let room = requests.iter().find(|r|r["asset"]["id"] == "a-room").unwrap();
    for prop in room["assetProductionContext"]["relatedProps"].as_array().unwrap() {
        assert!(!prop["acceptedDesign"].is_null(),"LOC starts only after a valid accepted PROP design");
        assert_eq!(prop["disposition"],"VERIFY_SAME_OBJECT_IN_SOURCE_THEN_REUSE");
    }
    assert!(room.get("referenceVariantIds").is_none(),"text candidates must not create an image-reference input");
    assert!(room["parentAsset"].is_null());
    let events = executor.events.lock().unwrap();
    let position = |event: &str| events.iter().position(|e|e==event).unwrap();
    assert!(position("accept:z-statue") < position("start:a-room"));
    assert!(position("accept:y-bell") < position("start:a-room"));
    assert!(position("start:b-unrelated") < position("accept:z-statue"),"unrelated work must not wait on the prop branch");
    let room_design = output["designs"].as_array().unwrap().iter().find(|d|d["assetId"] == "a-room").unwrap();
    assert_eq!(room_design["referenceAssetIds"],json!([]));
    assert_eq!(room_design["designReceipt"]["basis"]["assetProductionContext"]["consumedPropDesigns"].as_array().unwrap().len(),2);
    production::validate_consumed_prop_designs(room_design,output["assets"].as_array().unwrap()).unwrap();
    assert_eq!(input,before,"planning cannot mutate source/catalogue or create parent/image references");
}

#[tokio::test]
async fn scene_prop_v3_failed_prop_is_relation_only_and_unrelated_designs_continue() {
    let input = fixture().await;
    let executor = Designer { fail_statue:true,..Designer::default() };
    let output = run_asset_design(&input,&executor,"offline","medium").await.unwrap();
    assert_eq!(output["blocked"].as_array().unwrap().len(),1,"only failed prop branch blocks");
    assert_eq!(output["blocked"][0]["assetId"],"z-statue");
    assert_eq!(output["designs"].as_array().unwrap().len(),4);
    let requests = executor.requests.lock().unwrap();
    let room = requests.iter().find(|r|r["asset"]["id"] == "a-room").unwrap();
    let related = room["assetProductionContext"]["relatedProps"].as_array().unwrap();
    let statue = related.iter().find(|p|p["assetId"] == "z-statue").unwrap();
    assert!(statue["acceptedDesign"].is_null());
    assert_eq!(statue["disposition"],"RELATION_ONLY_UNTIL_VALID_DESIGN");
    assert!(!related.iter().find(|p|p["assetId"]=="y-bell").unwrap()["acceptedDesign"].is_null());
    assert!(executor.accepted.lock().unwrap().contains("b-unrelated"));
}

#[tokio::test]
async fn scene_prop_v2_frozen_context_and_receipts_keep_original_empty_loc_prop_contract() {
    let mut input = fixture().await;
    freeze_v2(&mut input);
    let before = input.clone();
    let candidates = production::candidates(&input).unwrap();
    assert!(!candidates.contains_key("a-room"));
    let context = production::context(&input,&input["assets"][0]).unwrap().unwrap();
    assert_eq!(context["version"],production::PREVIOUS_PROFILE);
    assert_eq!(context["instructions"],production::PREVIOUS_INSTRUCTIONS);
    assert_eq!(context["relatedProps"],json!([]));
    let executor = Designer::default();
    let output = run_asset_design(&input,&executor,"offline","medium").await.unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty());
    let events = executor.events.lock().unwrap();
    assert!(events.iter().position(|e|e=="start:a-room").unwrap() < events.iter().position(|e|e=="accept:z-statue").unwrap(),"old LOC must not acquire the v3 dependency");
    let room = output["designs"].as_array().unwrap().iter().find(|d|d["assetId"]=="a-room").unwrap();
    assert_eq!(room["frozenContext"]["assetProductionContext"],context);
    production::validate_consumed_prop_designs(room,output["assets"].as_array().unwrap()).unwrap();
    production::validate_resume(&input,&output,&json!({"assets":output["assets"]})).unwrap();
    assert_eq!(input,before);
}

#[tokio::test]
async fn scene_prop_v3_consumes_v2_prop_using_original_receipt_and_rejects_tampering() {
    let mut original = fixture().await;
    freeze_v2(&mut original);
    original["selection"] = json!({"assetIds":["z-statue","y-bell"]});
    let output = run_asset_design(&original,&Designer::default(),"offline","medium").await.unwrap();
    assert!(output["blocked"].as_array().unwrap().is_empty());
    let mut current = original.clone();
    for update in output["assets"].as_array().unwrap() {
        let saved = current["assets"].as_array_mut().unwrap().iter_mut().find(|a|a["id"]==update["id"]).unwrap();
        *saved = update.clone();
    }
    current["selection"] = json!({"assetIds":["a-room"]});
    production::freeze(&mut current);
    let before = current.clone();
    let context = production::context(&current,&current["assets"][0]).unwrap().unwrap();
    for prop in context["relatedProps"].as_array().unwrap() {
        assert!(!prop["acceptedDesign"].is_null(),"v3 may consume accepted v2 prop without re-designing it");
    }
    assert_eq!(current,before);
    let executor = Designer::default();
    let result = run_asset_design(&current,&executor,"offline","medium").await.unwrap();
    assert!(result["blocked"].as_array().unwrap().is_empty(),"{result}");
    assert_eq!(executor.requests.lock().unwrap().len(),1);
    let room = &result["designs"][0];
    let mut final_assets = current["assets"].as_array().unwrap().clone();
    final_assets[0] = result["assets"][0].clone();
    production::validate_consumed_prop_designs(room,&final_assets).unwrap();
    assert_eq!(final_assets[1]["design"],before["assets"][1]["design"],"original v2 receipt/frozen context stays byte-equivalent");
    let mut changed = before.clone();
    changed["assets"][1]["design"]["frozenContext"]["assetProductionContext"]["instructions"] = json!(production::INSTRUCTIONS);
    let invalid = production::context(&changed,&changed["assets"][0]).unwrap().unwrap();
    assert!(invalid["relatedProps"].as_array().unwrap().iter().find(|p|p["assetId"]=="z-statue").unwrap()["acceptedDesign"].is_null());
    assert!(production::validate_consumed_prop_designs(room,changed["assets"].as_array().unwrap()).is_err());
}
