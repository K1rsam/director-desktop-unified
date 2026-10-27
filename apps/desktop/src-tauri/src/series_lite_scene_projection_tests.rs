use super::*;
fn fixture() -> (Vec<u8>, Vec<u8>) {
    let candidate = br#"{"version":"series-storyboard-bundle-v1","sourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sourceLockHash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","assets":[{"key":"balconySC","assetId":"A0001","assetRevision":3},{"key":"diningSC","assetId":"A0002","assetRevision":4},{"key":"leadCH","assetId":"A0003","assetRevision":1},{"key":"parentCH","assetId":"A0004","assetRevision":1}],"episodes":[{"number":150,"source":{"quote":"Do not change this source."},"settings":{"precise":1.234567891234567891},"scenes":[{"id":"150-1","sourceRefs":["row1","row2"],"bindings":["balconySC"]}],"draft":{"units":[{"id":"u1","sceneId":"150-1","bindings":["balconySC","leadCH"],"imageBindings":["balconySC","leadCH"],"seconds":2.123456789},{"id":"u2","sceneId":"150-1","bindings":["diningSC","leadCH"],"imageBindings":["diningSC"],"seconds":3.123456789}]}}]}"#.to_vec();
    let exact = parse(&candidate).unwrap();
    let ep = &exact.get("episodes").unwrap().array().unwrap()[0];
    let supply = SceneSupply {
        version: SUPPLY.into(),
        source_hash: "a".repeat(64),
        source_lock_hash: "b".repeat(64),
        episode: 150,
        source_value_sha256: sha256(&canonical(ep.get("source").unwrap())),
        registry_snapshot_sha256: "c".repeat(64),
        evidence_sha256: "d".repeat(64),
        scenes: vec![SuppliedScene {
            id: "150-1".into(),
            protected_fields_sha256: protected_scene(
                &ep.get("scenes").unwrap().array().unwrap()[0],
            )
            .unwrap(),
            allowed_assets: vec!["balconySC".into(), "diningSC".into(), "leadCH".into()],
        }],
        assets: [
            ("balconySC", "A0001", 3),
            ("diningSC", "A0002", 4),
            ("leadCH", "A0003", 1),
            ("parentCH", "A0004", 1),
        ]
        .into_iter()
        .map(|(key, id, rev)| SuppliedAsset {
            key: key.into(),
            asset_id: id.into(),
            asset_revision: rev,
            full_design_sha256: "e".repeat(64),
            material_source_only: key == "parentCH",
            image_allowed: key != "parentCH",
        })
        .collect(),
    };
    (candidate, serde_json::to_vec(&supply).unwrap())
}
fn run(candidate: &[u8], supply: &[u8]) -> Result<Projection> {
    project(candidate, &sha256(candidate), supply, &sha256(supply))
}

#[test]
fn one_numbered_scene_can_use_two_location_assets_without_splitting_or_changing_source() {
    let (candidate, supply) = fixture();
    let out = run(&candidate, &supply).unwrap();
    assert!(out.changed);
    let before = parse(&candidate).unwrap();
    let mut after = parse(&out.bytes).unwrap();
    let Json::Object(root) = &mut after else {
        panic!()
    };
    let Json::Array(eps) = root.get_mut("episodes").unwrap() else {
        panic!()
    };
    let Json::Object(ep) = &mut eps[0] else {
        panic!()
    };
    let Json::Array(scenes) = ep.get_mut("scenes").unwrap() else {
        panic!()
    };
    assert_eq!(scenes.len(), 1);
    assert_eq!(
        strings(scenes[0].get("bindings").unwrap()).unwrap(),
        vec!["balconySC", "leadCH", "diningSC"]
    );
    let Json::Object(scene) = &mut scenes[0] else {
        panic!()
    };
    scene.insert(
        "bindings".into(),
        before.get("episodes").unwrap().array().unwrap()[0]
            .get("scenes")
            .unwrap()
            .array()
            .unwrap()[0]
            .get("bindings")
            .unwrap()
            .clone(),
    );
    assert_eq!(
        before, after,
        "only scene bindings may change; source/settings/assets/units/precise numbers stay intact"
    );
    assert!(String::from_utf8(out.bytes.clone())
        .unwrap()
        .contains("1.234567891234567891"));
    assert_eq!(out.sha256, sha256(&out.bytes));
    assert_eq!(out.receipt["contentReview"], "NOT_RUN");
    assert_eq!(out.receipt["adoption"], "NOT_GRANTED");
    let again = run(&out.bytes, &supply).unwrap();
    assert!(!again.changed);
    assert_eq!(again.bytes, out.bytes);
}
#[test]
fn source_scene_design_and_outer_pin_drift_are_rejected() {
    let (candidate, supply) = fixture();
    assert!(project(&candidate, &"f".repeat(64), &supply, &sha256(&supply)).is_err());
    assert!(project(&candidate, &sha256(&candidate), &supply, &"f".repeat(64)).is_err());
    for (old, new) in [
        ("Do not change this source.", "rewritten source"),
        ("\"number\":150", "\"number\":1"),
        ("\"assetRevision\":3", "\"assetRevision\":99"),
        ("\"row2\"", "\"row3\""),
    ] {
        let changed = String::from_utf8(candidate.clone())
            .unwrap()
            .replace(old, new);
        assert!(run(changed.as_bytes(), &supply).is_err(), "{old}");
    }
}
#[test]
fn unavailable_cross_scene_and_material_only_assets_cannot_be_promoted_by_projection() {
    let (candidate, supply) = fixture();
    for (old, new) in [
        (
            "\"bindings\":[\"diningSC\",\"leadCH\"]",
            "\"bindings\":[\"missingCH\",\"leadCH\"]",
        ),
        (
            "\"bindings\":[\"diningSC\",\"leadCH\"]",
            "\"bindings\":[\"parentCH\",\"leadCH\"]",
        ),
        ("\"sceneId\":\"150-1\"", "\"sceneId\":\"150-2\""),
    ] {
        assert!(run(
            String::from_utf8(candidate.clone())
                .unwrap()
                .replace(old, new)
                .as_bytes(),
            &supply
        )
        .is_err());
    }
    let mut scope: SceneSupply = serde_json::from_slice(&supply).unwrap();
    scope.scenes[0].allowed_assets.retain(|k| k != "diningSC");
    assert!(run(&candidate, &serde_json::to_vec(&scope).unwrap())
        .unwrap_err()
        .contains("ASSET_REQUIRES_SCENE_SUPPLY"));
    scope.scenes[0].allowed_assets.push("parentCH".into());
    assert!(run(&candidate, &serde_json::to_vec(&scope).unwrap()).is_err());
}
#[test]
fn voice_only_pin_cannot_become_an_image_and_empty_scene_is_not_deleted() {
    let (candidate, supply) = fixture();
    let mut scope: SceneSupply = serde_json::from_slice(&supply).unwrap();
    scope.assets[2].image_allowed = false;
    assert!(run(&candidate, &serde_json::to_vec(&scope).unwrap())
        .unwrap_err()
        .contains("IMAGE_USE_NOT_AUTHORIZED"));
    let mut c: Value = serde_json::from_slice(&candidate).unwrap();
    c["episodes"][0]["draft"]["units"] = json!([]);
    assert!(run(&serde_json::to_vec(&c).unwrap(), &supply)
        .unwrap_err()
        .contains("EMPTY_SCENE_REQUIRES_REVIEW"));
}

#[test]
fn unprovided_assets_cannot_enter_the_candidate_even_when_unused() {
    let (candidate, supply) = fixture();
    let mut scope: SceneSupply = serde_json::from_slice(&supply).unwrap();
    scope.assets.retain(|a| a.key != "parentCH");
    assert!(run(&candidate, &serde_json::to_vec(&scope).unwrap())
        .unwrap_err()
        .contains("BUNDLE_SUPPLY_SCOPE_MISMATCH"));
}
