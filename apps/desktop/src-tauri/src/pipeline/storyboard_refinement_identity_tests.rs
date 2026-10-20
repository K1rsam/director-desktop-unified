use super::super::{
    asset_use_binding as binding, storyboard_refinement as refinement, TextExecutor, TextRequest,
};
use super::*;

fn fixture() -> (Value, Vec<(String, Value)>, Value) {
    let (mut input, mut raw) = binding::tests::fixture();
    input["assets"][0]["version"] = json!(2);
    input["assets"][0]["name"] = json!("小王");
    input["assets"][0]["design"] = json!({"description":"旧美术"});
    input["assets"][0]["prompt"] = json!("旧提示词");
    raw["units"][0]["episodeId"] = json!("ep1");
    raw["units"][0]["version"] = json!(1);
    let extra = raw["units"][0]["assetRefs"][0].clone();
    raw["units"][0]["assetRefs"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    raw["units"][0]["assetRefs"][1]["order"] = json!(2);
    let draft = binding::materialize(&input, &raw).unwrap();
    let origin = json!({"input":{"source":{"sha256":digest("source")},"assets":input["assets"]}});
    input["source"] = origin["input"]["source"].clone();
    input["episodeId"] = json!("ep1");
    input["existingUnits"] = draft["units"].clone();
    input["revisionContexts"] = json!([{"episodeId":"ep1","sourceSha256":input["source"]["sha256"],"originWorkflowId":"origin","unitBindings":bindings(&input).unwrap()}]);
    input["assets"][0]["version"] = json!(3);
    input["assets"][0]["design"] = json!({"description":"新美术"});
    input["assets"][0]["prompt"] = json!("新提示词");
    (input, vec![("origin".into(), origin)], draft)
}

#[test]
fn art_only_rebinding_preserves_original_bytes_and_all_same_unit_references() {
    let (mut input, origins, draft) = fixture();
    let before = input.clone();
    let bytes = serde_json::to_vec(&draft).unwrap();
    assert!(
        binding::validate_unit(&draft["units"][0], input["assets"].as_array().unwrap()).is_err()
    );
    let packet = build(&input, &origins).unwrap().unwrap();
    assert_eq!(packet["entries"].as_array().unwrap().len(), 2);
    assert_eq!(input, before);
    input[FIELD] = packet;
    let view = validation_view(&input, &draft).unwrap();
    for reference in view["units"][0]["assetRefs"].as_array().unwrap() {
        assert_eq!(reference["identityEvidence"]["assetVersion"], 3);
        assert_eq!(reference["sourceRefs"], json!(["r2"]));
    }
    assert_eq!(serde_json::to_vec(&draft).unwrap(), bytes);
    assert_eq!(input["existingUnits"], draft["units"]);
    let reopened = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(validation_view(&reopened, &draft).unwrap(), view);
}

#[test]
fn identity_source_scope_and_non_art_changes_are_never_rebound() {
    let (input, origins, _) = fixture();
    for key in [
        "id",
        "variantId",
        "identityId",
        "name",
        "kind",
        "role",
        "sourceRefs",
        "sourceRefsPurpose",
        "scope",
        "applicability",
        "preparation",
        "sourceVariantId",
        "materialRole",
        "unknownFutureField",
    ] {
        let mut changed = input.clone();
        changed["assets"][0][key] = json!("changed");
        assert!(build(&changed, &origins).is_err(), "{key}");
    }
    for version in [json!(0), json!(1), json!("3"), json!(null)] {
        let mut changed = input.clone();
        changed["assets"][0]["version"] = version;
        assert!(build(&changed, &origins).is_err());
    }
    let mut duplicate = origins.clone();
    duplicate.push(origins[0].clone());
    assert!(
        build(&input, &duplicate).is_err(),
        "ambiguous origins fail closed"
    );
}

#[test]
fn changed_anchors_units_seals_origins_and_current_versions_are_rejected() {
    let (mut input, origins, draft) = fixture();
    input[FIELD] = build(&input, &origins).unwrap().unwrap();
    for mode in [
        "source",
        "asset",
        "unit",
        "seal",
        "origin-hash",
        "wrapper-extra",
    ] {
        let mut bad = input.clone();
        match mode {
            "source" => bad["source"]["sha256"] = json!(digest("other")),
            "asset" => bad["assets"][0]["version"] = json!(4),
            "unit" => bad["existingUnits"][0]["shots"][0]["picture"] = json!("changed"),
            "seal" => bad[FIELD]["sha256"] = json!("forged"),
            "origin-hash" => {
                bad[FIELD]["entries"][0]["originSnapshotSha256"] = json!("not-sha256");
                let mut unsigned = bad[FIELD].clone();
                unsigned.as_object_mut().unwrap().remove("sha256");
                bad[FIELD] = sealed(unsigned);
            }
            _ => bad[FIELD]["extra"] = json!(true),
        }
        assert!(validation_view(&bad, &draft).is_err(), "{mode}");
    }
    for key in ["useShotIds", "sourceRefs"] {
        let mut changed = draft.clone();
        changed["units"][0]["assetRefs"][0][key] = json!(["wrong"]);
        assert!(validation_view(&input, &changed).is_err(), "{key}");
    }
    let mut snapshot_input = input.clone();
    snapshot_input.as_object_mut().unwrap().remove(FIELD);
    let snapshot = json!({"input":snapshot_input});
    let prepared = json!({RECOVERY:recovery(&snapshot, &input[FIELD])});
    let mut effective = snapshot["input"].clone();
    attach_recovery(&mut effective, &snapshot, &prepared).unwrap();
    assert_eq!(effective, input);
    let mut bad = prepared.clone();
    bad[RECOVERY]["extra"] = json!(true);
    assert!(attach_recovery(&mut snapshot["input"].clone(), &snapshot, &bad).is_err());
    let mut changed = snapshot.clone();
    changed["extra"] = json!(true);
    assert!(attach_recovery(&mut changed["input"].clone(), &changed, &prepared).is_err());
}

struct WireModel(refinement::tests::Model);
#[async_trait::async_trait]
impl TextExecutor for WireModel {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let mut raw = self.0.execute(request).await?;
        for group in raw["replacements"].as_array_mut().into_iter().flatten() {
            for unit in group["units"].as_array_mut().into_iter().flatten() {
                unit.as_object_mut().unwrap().remove(binding::FIELD);
                for reference in unit["assetRefs"].as_array_mut().into_iter().flatten() {
                    for key in ["sourceRefs", "sourceRefsPurpose", "identityEvidence"] {
                        reference.as_object_mut().unwrap().remove(key);
                    }
                }
            }
        }
        Ok(raw)
    }
}

#[tokio::test]
async fn basis_delta_adopt_keep_retained_v2_bytes_and_materialize_new_v3_proofs() {
    let (_dir, _store, mut input, origin) = refinement::tests::fixture().await;
    input["capabilities"]["imageLimit"] = json!(1);
    binding::freeze_new(&mut input).unwrap();
    let asset = json!({"id":"actor","variantId":"base","version":2,"kind":"character","name":"小王","sourceRefs":[],"sourceRefsPurpose":"SOURCE_SCOPE"});
    let mut old_input = input.clone();
    old_input["assets"] = json!([asset]);
    old_input["sourceRows"] = input["source"]["rows"].clone();
    let mut raw = json!({"units":input["existingUnits"]});
    for unit in raw["units"].as_array_mut().unwrap() {
        unit["assetRefs"] = json!([{"assetId":"actor","variantId":"base","use":"character","order":1,"useShotIds":[unit["shots"][0]["id"]]}]);
    }
    input["existingUnits"] = binding::materialize(&old_input, &raw).unwrap()["units"].clone();
    input["assets"] = old_input["assets"].clone();
    input["assets"][0]["version"] = json!(3);
    input["assets"][0]["prompt"] = json!("三视图");
    input["revisionContexts"][0]["unitBindings"] = bindings(&input).unwrap();
    let origins = vec![(
        origin,
        json!({"input":{"source":input["source"],"assets":old_input["assets"]}}),
    )];
    let original_units = input["existingUnits"].clone();
    let mut model = refinement::tests::Model::default();
    model.mode = "changed";
    let model = WireModel(model);
    assert!(
        refinement::run_storyboard_refinement(&input, &model, "offline", "medium")
            .await
            .is_err(),
        "unmarked historic strict validation remains strict"
    );
    assert!(model.0.calls.lock().await.is_empty());
    input[FIELD] = build(&input, &origins).unwrap().unwrap();
    let candidate = refinement::run_storyboard_refinement(&input, &model, "offline", "medium")
        .await
        .unwrap();
    let delivery = refinement::adopt_storyboard_refinement(&input, &candidate).unwrap();
    assert_eq!(candidate[FIELD], input[FIELD]);
    assert_eq!(input["existingUnits"], original_units);
    assert_eq!(candidate["draft"]["units"][1], original_units[1]);
    assert_eq!(
        candidate["validatedDelta"]["replacements"][0]["units"][0]["assetRefs"][0]
            ["identityEvidence"]["assetVersion"],
        3
    );
    assert_eq!(
        delivery["units"][0]["assetRefs"][0]["identityEvidence"]["assetVersion"],
        3
    );
    let calls = model.0.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["baseDraftHash"], candidate["baseDraftHash"]);
    assert_eq!(
        calls[0].input["previousDraft"]["units"][1]["assetRefs"][0]["identityEvidence"]
            ["assetVersion"],
        3
    );
    let mut forged = calls[0].input["previousDraft"].clone();
    forged["units"][0]["id"] = json!("new-model-unit");
    assert!(
        binding::materialize(&calls[0].input, &forged).is_err(),
        "rebind never authorizes model metadata"
    );
    let mut drift = input.clone();
    drift["assets"][0]["version"] = json!(4);
    assert!(refinement::adopt_storyboard_refinement(&drift, &candidate).is_err());
    let mut legacy = input.clone();
    legacy.as_object_mut().unwrap().remove(FIELD);
    assert!(refinement::adopt_storyboard_refinement(&legacy, &candidate).is_err());
}
