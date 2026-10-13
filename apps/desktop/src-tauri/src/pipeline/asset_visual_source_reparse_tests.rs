use super::*;
use asset_visual_source_reparse::{freeze, Context, FIELD};
use std::sync::Mutex;

pub(crate) fn fixture() -> (Value, Value, Value) {
    let next = prepare_source("第1集\n场景：屋内\n乙：在。\n甲：回来。乙：等等。\n").unwrap();
    let mut old = next.clone();
    old.parser_version = "source-parser/v4".into();
    let mut joined = old.rows.remove(3);
    let second = old.rows.remove(3);
    joined.text.push_str(&second.text); joined.range.end = second.range.end;
    joined.id = format!("row:{}:{}:{}", &old.sha256[..12], joined.range.start, joined.range.end);
    let dialogue = joined.dialogue.as_mut().unwrap();
    dialogue.id = format!("dialogue:{}", joined.id); dialogue.range.end = second.range.end - 1;
    dialogue.quote = "回来。乙：等等。".into();
    dialogue.spoken_segments = vec![SourceSegment { text: dialogue.quote.clone(), range: dialogue.range.clone() }];
    for refs in old.episodes.iter_mut().map(|e|&mut e.row_ids).chain(old.scenes.iter_mut().map(|s|&mut s.row_ids)) {
        refs.retain(|id|id != &second.id);
        for id in refs { if *id == next.rows[3].id { *id = joined.id.clone(); } }
    }
    old.rows.push(joined);
    validate_source(&old).unwrap();
    let mut old_value = json!(old); old_value["version"] = json!(1);
    let mut next_value = json!(next); next_value["version"] = json!(2);
    let plan = crate::source_migration::plan(&old_value, &next_value).unwrap();
    let mut proof = json!({"version":crate::source_migration::VERSION,"source":plan.evidence,
        "newSourceVersion":2,"newSourceSha256":digest(&next_value.to_string())});
    proof["proofSha256"] = json!(digest(&proof.to_string()));
    let asset = json!({"id":"person:base","baseAssetId":"person","variantId":"base","kind":"CH","name":"乙",
        "version":1,"description":"基础造型","sourceRefs":[old.rows[2].id],
        "scope":{"kind":"GLOBAL","scenes":[],"reason":"统一基础造型"},
        "applicability":{"episodes":[old.episodes[0].id],"scenes":[old.scenes[0].id]},
        "preparation":{"mode":"REFERENCE","note":"基础长发；后期短发不用于本批。"}});
    let mut input = json!({"source":next_value,"assets":[asset],"selection":{"assetIds":["person:base"]},
        "foundation":{"sourceSha256":old.sha256,"facts":[]}});
    let identities = identity_cards(&input, input["assets"].as_array().unwrap());
    let wire = PacketWire::compile(&old, &identities, &old.rows.iter().collect::<Vec<_>>());
    let fact = json!({"id":"fact-base","assetId":"person","variantId":null,"attribute":"hair.color","value":"黑发",
        "scope":"STABLE","claim":"SOURCE","sourceRefs":[old.rows[2].id],"sourceSha256":old.sha256});
    input["foundation"]["visualFactBatches"] = json!([{"schemaVersion":"asset-visual-facts-cache/v1","status":"accepted",
        "sourceSha256":old.sha256,"inputFingerprint":wire.packet_hash,"rowIds":wire.row_ids,"identityCards":identities,
        "ruleSha256":rule_snapshot("asset-visual-facts-extractor").unwrap()["sha256"],"facts":[fact]}]);
    (input, old_value, proof)
}

#[derive(Default)]
struct NoDispatch(Mutex<Vec<TextRequest>>);
#[async_trait]
impl TextExecutor for NoDispatch {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        self.0.lock().unwrap().push(request);
        Err("NO_MODEL_DISPATCH".into())
    }
}

#[tokio::test]
async fn source_reparse_visual_cache_preserves_original_and_reuses_split_coverage_without_dispatch() {
    let (mut input, old, proof) = fixture();
    let cache = input["foundation"]["visualFactBatches"].clone();
    freeze(&mut input, old, proof).unwrap();
    let before = input.clone();
    let exec = NoDispatch::default();
    let result = run_asset_visual_facts(&input, &exec, "model", "medium").await.unwrap();
    assert_eq!(input, before);
    assert!(exec.0.lock().unwrap().is_empty());
    assert_eq!(result["blocked"], json!([]));
    assert_eq!(result["visualFactBatches"], cache);
    assert!(result["visualFactCacheReuses"].as_array().unwrap().iter().any(|p|p["coverageExpandedByVerifiedMapping"] == true));
    let source = serde_json::from_value(input["source"].clone()).unwrap();
    assert_eq!(accepted_source_facts(&input, &source).unwrap(), cache[0]["facts"].as_array().unwrap().clone());
}

#[tokio::test]
async fn source_reparse_visual_cache_keeps_legacy_absence_and_rejects_null_unknown_or_drift() {
    let (legacy, old, proof) = fixture();
    let exec = NoDispatch::default();
    run_asset_visual_facts(&legacy, &exec, "model", "medium").await.unwrap();
    assert_eq!(exec.0.lock().unwrap().len(), 1);
    assert!(legacy.get(FIELD).is_none());
    for mutation in ["null", "unknown", "source", "new-rebound", "old", "proof", "mapping", "duplicate", "missing-binding", "foreign-binding"] {
        let mut input = legacy.clone(); freeze(&mut input, old.clone(), proof.clone()).unwrap();
        match mutation {
            "null" => input[FIELD] = Value::Null,
            "unknown" => input[FIELD]["version"] = json!("unsupported/v99"),
            "source" => input["source"]["version"] = json!(3),
            "new-rebound" => {
                input["source"]["rows"][2]["text"] = json!("污染的新原文");
                input[FIELD]["migrationProof"]["newSourceSha256"] = json!(digest(&input["source"].to_string()));
            }
            "old" => input[FIELD]["originalSource"]["rows"][2]["text"] = json!("污染的原文"),
            "proof" => input[FIELD]["migrationProof"]["proofSha256"] = json!("wrong"),
            "mapping" => input[FIELD]["migrationProof"]["source"]["rowMapping"][0]["newRowIds"] = json!([]),
            "duplicate" => { let packet = input[FIELD]["packets"][0].clone(); input[FIELD]["packets"].as_array_mut().unwrap().push(packet); }
            "missing-binding" => input[FIELD]["packets"] = json!([]),
            _ => input[FIELD]["packets"][0]["inputFingerprint"] = json!("foreign"),
        }
        if ["new-rebound", "mapping"].contains(&mutation) {
            let mut payload = input[FIELD]["migrationProof"].clone(); payload.as_object_mut().unwrap().remove("proofSha256");
            input[FIELD]["migrationProof"]["proofSha256"] = json!(digest(&payload.to_string()));
        }
        let exec = NoDispatch::default();
        assert!(run_asset_visual_facts(&input, &exec, "model", "medium").await.is_err(), "{mutation}");
        assert!(exec.0.lock().unwrap().is_empty(), "{mutation}");
        let mut valid = legacy.clone(); freeze(&mut valid, old.clone(), proof.clone()).unwrap();
        let good_source = serde_json::from_value(valid["source"].clone()).unwrap();
        assert!(!accepted_source_facts(&valid, &good_source).unwrap().is_empty(), "failed validation cannot poison the next valid input: {mutation}");
    }
}

#[test]
fn source_reparse_visual_same_row_id_metadata_change_needs_no_cache_adapter() {
    let old = crate::source_migration::tests::fixture();
    let mut next = crate::source_migration::tests::candidate(&old); next["version"] = json!(2);
    let changed = old["rows"].as_array().unwrap().last().unwrap()["id"].clone();
    let mut input = json!({"source":next,"foundation":{"visualFactBatches":[{"inputFingerprint":"same-rows","rowIds":[changed]}]}});
    let before = input.clone();
    // No adapter or proof is needed when the existing packet can still read
    // exactly its original rows. Ordinary packet validation remains in force.
    freeze(&mut input, old, Value::Null).unwrap();
    assert_eq!(input, before);
    assert!(input.get(FIELD).is_none());
}

#[tokio::test]
async fn source_reparse_visual_cache_rejects_consumed_changed_rows_tampered_cache_and_unresolved_without_reextracting() {
    for mutation in ["consumed", "pending", "raw", "budget", "row"] {
        let (mut input, old, proof) = fixture();
        if mutation == "consumed" {
            input["foundation"]["visualFactBatches"][0]["facts"][0]["sourceRefs"] = json!([old["rows"][3]["id"]]);
        }
        if mutation == "pending" {
            let fact = input["foundation"]["visualFactBatches"][0]["facts"][0].clone();
            input["foundation"]["visualFactBatches"][0]["unassignedFacts"] = json!([{"fact":fact}]);
        }
        freeze(&mut input, old, proof).unwrap();
        if mutation == "raw" { input["foundation"]["visualFactBatches"][0]["repairProvenance"]["originalRawSha256"] = json!("changed"); }
        if mutation == "budget" { input["foundation"]["visualFactBatches"][0]["repairProvenance"]["nodes"] = json!([]); }
        if mutation == "row" { input["foundation"]["visualFactBatches"][0]["rowIds"][0] = json!("unknown"); }
        let before = input.clone(); let exec = NoDispatch::default();
        let result = run_asset_visual_facts(&input, &exec, "model", "medium").await;
        assert!(result.is_err() || !result.unwrap()["blocked"].as_array().unwrap().is_empty(), "{mutation}");
        assert!(exec.0.lock().unwrap().is_empty(), "{mutation}");
        assert_eq!(input, before, "{mutation}");
    }
}

#[tokio::test]
async fn source_reparse_visual_deferred_decision_revalidates_original_constraints_and_budget() {
    let (mut input, old, proof) = fixture();
    let source: LockedSource = serde_json::from_value(old.clone()).unwrap();
    let identities = identity_cards(&input, input["assets"].as_array().unwrap());
    let wire = PacketWire::compile(&source, &identities, &source.rows.iter().collect::<Vec<_>>());
    let constraints = production_constraints(&source, &wire, &identities, input["assets"].as_array().unwrap()).unwrap();
    let fact = json!({"id":"deferred-fact","assetId":"person","variantId":null,"attribute":"hair.style","value":"短发",
        "scope":"TRANSITION","claim":"SOURCE","sourceRefs":[source.rows[2].id],"sourceSha256":source.sha256});
    let correction = json!({"variantId":null,"reason":"原文变化保留；当前基础范围不适用。","productionConstraintRefs":[constraints[0]["constraintSha256"]],"notApplicableAssetIds":["person:base"]});
    input["foundation"]["visualFactBatches"][0]["deferredFacts"] = json!([deferred_record(&correction, &fact, 1, &constraints).unwrap()]);
    input["foundation"]["visualFactBatches"][0]["repairProvenance"] = json!({"version":VISUAL_FACT_REPAIR_PROFILE,
        "identityCards":identities,"originalAccepted":false,"originalRawSha256":"raw-preserved","originalDomainRequestSha256":"request-preserved",
        "nodes":[{"attempt":1,"domainRequestSha256":"first","rawDeltaSha256":"delta"}],
        "acceptedProductionConstraints":constraints,"productionConstraintsSha256":digest(&json!(constraints).to_string())});
    freeze(&mut input, old, proof).unwrap();
    let cache = input["foundation"]["visualFactBatches"].clone();
    assert!(!cached_visual_check_required(&input).unwrap());
    input["assets"][0]["version"] = json!(2);
    assert!(!cached_visual_check_required(&input).unwrap());
    let exec = NoDispatch::default();
    let result = run_asset_visual_facts(&input, &exec, "model", "medium").await.unwrap();
    assert!(exec.0.lock().unwrap().is_empty());
    assert_eq!(result["blocked"], json!([]));
    assert_eq!(result["visualFactBatches"], cache);
    input["assets"][0]["preparation"]["note"] = json!("已经改为短发");
    assert!(cached_visual_check_required(&input).unwrap());
    let result = run_asset_visual_facts(&input, &exec, "model", "medium").await.unwrap();
    assert!(!result["blocked"].as_array().unwrap().is_empty());
    assert!(exec.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn source_reparse_visual_engineering_proof_is_not_sent_to_asset_design_model() {
    let (mut input, old, proof) = fixture(); freeze(&mut input, old, proof).unwrap();
    input["foundation"]["productionDirection"] = json!({"visualStyle":"3D国漫"});
    let exec = NoDispatch::default();
    let _ = run_asset_design(&input, &exec, "model", "medium").await.unwrap();
    let requests = exec.0.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].stage, "assetDesign");
    assert!(requests[0].input.get(FIELD).is_none());
    let serialized = requests[0].input.to_string();
    assert!(!serialized.contains("originalSource") && !serialized.contains("migrationProof"));
    let source = serde_json::from_value(input["source"].clone()).unwrap();
    let ctx = Context::new(&input, &source).unwrap();
    assert!(ctx.proof(&input["foundation"]["visualFactBatches"][0], &[]).is_some());
}

#[test]
#[ignore = "Explicit isolated SQLite capture only; no model, images or production writes"]
fn source_reparse_visual_isolated_current_dependencies_audit() {
    use crate::store::{Store, atomic_json};
    use std::path::PathBuf;
    use std::collections::{BTreeMap, BTreeSet};
    let root = PathBuf::from(std::env::var("DIRECTOR_VISUAL_REPARSE_AUDIT_ROOT").expect("explicit isolated capture required"));
    let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../.local").canonicalize().unwrap();
    assert!(root.canonicalize().unwrap().starts_with(allowed));
    assert!(root.join("isolated/project.sqlite").is_file());
    let capture: Value = serde_json::from_slice(&std::fs::read(root.join("snapshot.json")).unwrap()).unwrap();
    let raw = |v: &Value| -> Value { serde_json::from_str(v.as_str().unwrap()).unwrap() };
    let snapshot = raw(capture["workflowSnapshotRaw"].as_array().unwrap().last().unwrap());
    let mut input = snapshot["stage2Context"].clone();
    input["source"] = raw(&capture["sourceRow"]["data"]);
    input["foundation"] = raw(&capture["foundationRow"]["data"]);
    input["assets"] = json!(capture["assetRowsRaw"].as_array().unwrap().iter().map(raw).collect::<Vec<_>>());
    let store = Store::open(root.join("isolated")).unwrap();
    assert_eq!(input["source"], store.source().unwrap());
    assert_eq!(input["foundation"], store.foundation().unwrap());
    let before = (store.jobs().unwrap(), store.values("assets").unwrap(), store.foundation().unwrap());
    store.freeze_visual_fact_source_reparse(&mut input).unwrap();
    input["selection"] = json!({"assetIds":input["assets"].as_array().unwrap().iter().map(|a|&a["id"]).collect::<Vec<_>>()});
    input["selection"]["explicitAssetIds"] = input["selection"]["assetIds"].clone();
    asset_design_contract::freeze_identity_production_materials(&mut input).unwrap();
    asset_production_context::freeze(&mut input);
    let props = asset_production_context::candidates(&input).unwrap();
    let assets = input["assets"].as_array().unwrap();
    let mut graph = BTreeMap::<String, BTreeSet<String>>::new();
    let mut records = Vec::new();
    for asset in assets {
        let id = text(asset, "id");
        let expected = asset_design_contract::design_receipt(&input, asset).unwrap();
        let parent = resolve_asset_dependency(assets, asset).unwrap();
        let prop_ids: Vec<_> = props.get(id).into_iter().flatten().map(|p|text(p,"assetId").to_owned()).collect();
        let mut deps: BTreeSet<String> = prop_ids.iter().cloned().collect();
        if let Some(parent) = &parent { deps.insert(parent.clone()); }
        graph.insert(id.to_owned(), deps);
        records.push(json!({"id":id,"name":asset["name"],"directParent":parent,"propCandidateIds":prop_ids,
            "currentReceipt":asset_design_contract::design_receipt_current(&input,asset),
            "needsAutomaticDesign":automatic_design_required(&input,asset),
            "basisDifferences":expected["basis"].as_object().unwrap().keys().filter(|key|expected["basis"][*key]!=asset["design"]["designReceipt"]["basis"][*key]).collect::<Vec<_>>()}));
    }
    let mut levels = Vec::new();
    while !graph.is_empty() {
        let ready: Vec<_> = graph.iter().filter(|(_,deps)|deps.is_empty()).map(|(id,_)|id.clone()).collect();
        assert!(!ready.is_empty(), "cycle or missing actual dependency");
        for id in &ready { graph.remove(id); }
        for deps in graph.values_mut() { for id in &ready { deps.remove(id); } }
        levels.push(ready);
    }
    assert!(!cached_visual_check_required(&input).unwrap());
    assert_eq!(before, (store.jobs().unwrap(),store.values("assets").unwrap(),store.foundation().unwrap()));
    atomic_json(&root.join("formal-current-dependencies-v4.json"), &json!({"generationCalls":0,"productionReadOnly":true,"isolatedBusinessRowsUnchanged":true,
        "assets":records,"formalDependencyLevels":levels,"cachedVisualCheckRequired":false,
        "stale":records.iter().filter(|a|a["needsAutomaticDesign"]==true).count(),"current":records.iter().filter(|a|a["currentReceipt"]==true).count()})).unwrap();
}
