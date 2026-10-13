//! Exercise the actual images.produce projection through the domain runner.
//! The executor is in-memory: it cannot submit images, video or paid requests.
use super::*;

fn context() -> Value {
    let source=pipeline::prepare_source("第一集\n场景：门口\n△门口有台阶。\n△一盆绿植被飞石劈开。\n△甲从门缝向外看。\n第二集\n场景：门口\n△远处另一栋楼的门口堆着木箱。\n").unwrap();
    let marker=source.rows.iter().find(|row|row.kind=="format"&&row.scene_id.is_some()).unwrap();
    let fact=source.rows.iter().find(|row|row.text.contains("台阶")).unwrap();
    json!({"source":source,"settings":{},"assets":[{"id":"door:base","baseAssetId":"door","variantId":"base","kind":"LOC","name":"门口","sourceRefs":[marker.id],"sourceRefsPurpose":"SOURCE_SCOPE","sourceScope":{"kind":"PROGRAM_SCOPE","sourceSha256":source.sha256},"scope":{"kind":"SCENES","scenes":[{"episodeId":marker.episode_id,"sceneId":marker.scene_id}]},"preparation":{"mode":"GENERATE","note":"破裂仅本场变化"}}],
        "foundation":{"sourceSha256":source.sha256,"facts":[{"id":"stairs","assetId":"door","variantId":"base","attribute":"结构","value":"台阶","scope":"STABLE","claim":"SOURCE","sourceRefs":[fact.id]}]}})
}

fn project(context: &Value) -> Value {
    image_production_design_input(context,context["assets"].as_array().unwrap(),&context["settings"],"automatic",&[],&["door:base".into()])
}

#[derive(Default)]
struct LocalDesigner { requests:Mutex<Vec<pipeline::TextRequest>> }
#[async_trait::async_trait]
impl pipeline::TextExecutor for LocalDesigner {
    async fn execute(&self, request:pipeline::TextRequest)->Result<Value> {
        self.requests.lock().unwrap().push(request.clone());
        if request.stage=="assetVisualFacts" {
            return Ok(json!({"schemaVersion":"asset-visual-facts-wire/v1","coveredPacketRef":request.input["coveredPacketRef"],"facts":[],"issues":[]}));
        }
        assert_eq!(request.stage,"assetDesign");
        let asset=&request.input["asset"];
        Ok(json!({"schemaVersion":"asset-design/v1","assetId":asset["id"],"variantId":asset["variantId"],"prompt":"台阶、完整花盆和门缝组成的可复用门口基础设计","freeDesignScope":"基础花盆完整，破裂是本场事件","sourceRefs":asset["sourceRefs"],"referenceVariantIds":[]}))
    }
}

#[tokio::test]
async fn stage2_frozen_input_new_and_resumed_one_click_keep_evidence_and_reuse_saved_design() {
    let mut frozen=context();pipeline::freeze_design_evidence_profile(&mut frozen);
    let original_bytes=serde_json::to_vec(&frozen).unwrap();
    let input=project(&frozen);let designer=LocalDesigner::default();
    let result=pipeline::run_asset_design(&input,&designer,"offline","medium").await.unwrap();
    assert!(result["blocked"].as_array().unwrap().is_empty(),"{result}");
    let calls=designer.requests.lock().unwrap();
    let requests:Vec<_>=calls.iter().filter(|r|r.stage=="assetDesign").collect();assert_eq!(requests.len(),1);
    let wire=&requests[0].input;
    assert_eq!(wire["designEvidenceContext"]["instructions"],frozen[pipeline::DESIGN_EVIDENCE_PROFILE_FIELD]["instructions"]);
    assert!(wire["evidence"].to_string().contains("绿植"));assert!(!wire["evidence"].to_string().contains("木箱"));
    assert_eq!(result["designs"][0]["designReceipt"]["basis"]["designEvidenceContext"]["evidenceSha256"],wire["designEvidenceContext"]["evidenceSha256"]);
    drop(calls);

    // Loading a saved workflow must reconstruct the same domain request without
    // consulting new rules or mutating its original input/cache identity.
    let restored:Value=serde_json::from_slice(&original_bytes).unwrap();
    let restored_input=project(&restored);assert_eq!(restored_input,input);
    let replay=LocalDesigner::default();let replay_result=pipeline::run_asset_design(&restored_input,&replay,"offline","medium").await.unwrap();
    assert_eq!(replay_result,result);
    assert_eq!(serde_json::to_vec(&frozen).unwrap(),original_bytes);

    // Publication followed by the next automatic run must keep this accepted
    // profile; dropping it used to mark the new design stale and downgrade it.
    let mut next=restored;next["assets"]=result["assets"].clone();
    next["foundation"]["facts"]=result["facts"].clone();
    next["foundation"]["visualFactBatches"]=result["visualFactBatches"].clone();
    let next_input=project(&next);
    assert!(!pipeline::automatic_design_required(&next_input,&next_input["assets"][0]));
    let again=LocalDesigner::default();let after=pipeline::run_asset_design(&next_input,&again,"offline","medium").await.unwrap();
    assert!(after["designs"].as_array().unwrap().is_empty());
    assert!(again.requests.lock().unwrap().is_empty(),"repeated automatic work cannot spend a new model call");
}

#[tokio::test]
async fn stage2_frozen_input_legacy_resume_preserves_absent_or_null_contract() {
    let legacy=context();
    let first=LocalDesigner::default();let result=pipeline::run_asset_design(&project(&legacy),&first,"offline","medium").await.unwrap();
    let original_calls=first.requests.lock().unwrap().iter().map(|r|serde_json::to_value(r).unwrap()).collect::<Vec<_>>();
    for null in [false,true] {
        let mut frozen=legacy.clone();if null {frozen[pipeline::DESIGN_EVIDENCE_PROFILE_FIELD]=Value::Null;}
        let bytes=serde_json::to_vec(&frozen).unwrap();let restored:Value=serde_json::from_slice(&bytes).unwrap();let input=project(&restored);
        assert_eq!(input.get(pipeline::DESIGN_EVIDENCE_PROFILE_FIELD),restored.get(pipeline::DESIGN_EVIDENCE_PROFILE_FIELD));
        let executor=LocalDesigner::default();let output=pipeline::run_asset_design(&input,&executor,"offline","medium").await.unwrap();
        assert_eq!(output,result);assert!(output["designs"][0]["designReceipt"]["basis"].get("designEvidenceContext").is_none());
        assert_eq!(executor.requests.lock().unwrap().iter().map(|r|serde_json::to_value(r).unwrap()).collect::<Vec<_>>(),original_calls);
        assert_eq!(serde_json::to_vec(&restored).unwrap(),bytes);
    }
}

#[tokio::test]
async fn stage2_frozen_input_corrupt_restored_profile_blocks_before_model_call() {
    let mut frozen=context();pipeline::freeze_design_evidence_profile(&mut frozen);
    frozen[pipeline::DESIGN_EVIDENCE_PROFILE_FIELD]["instructionsSha256"]=json!("corrupted");
    let original=frozen.clone();let executor=LocalDesigner::default();
    let error=pipeline::run_asset_design(&project(&frozen),&executor,"offline","medium").await.unwrap_err();
    assert!(error.contains("合同版本"),"{error}");assert!(executor.requests.lock().unwrap().is_empty());assert_eq!(frozen,original);
}
