use super::*;
fn pack() -> VerifiedPack {
    VerifiedPack::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../rules/series-lite/20261027-v0.1.2"),
    )
    .unwrap()
}

#[test]
fn embedded_release_matches_pinned_files_without_runtime_script_dependency() {
    let embedded = VerifiedPack::embedded().unwrap();
    assert_eq!(embedded.files, pack().files);
    assert!(embedded.require_new_production().is_err());
}

#[test]
fn current_release_is_exact_published_041_and_never_replaces_historical_pack() {
    let old = VerifiedPack::embedded().unwrap();
    let current = VerifiedPack::current_embedded().unwrap();
    let disk = VerifiedPack::load(&Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../rules/series-lite/20261028-v0.4.1")).unwrap();
    assert_eq!(current.files, disk.files);
    assert_eq!(current.files.len(), 106);
    assert_eq!(current.release(), CURRENT_RELEASE);
    assert_eq!(current.compiler_version(), "lingmo-lite-role-rules-v2");
    assert_eq!(current.inspection()["contract"]["sourceCommit"], CURRENT_SOURCE_COMMIT);
    assert_eq!(current.manifest_sha256(), hash(CURRENT_MANIFEST));
    assert!(current.file_bytes("scripts/prepare_episode.py").is_ok());
    assert!(old.file_bytes("scripts/prepare_episode.py").is_err());
    assert_eq!(old.release(), RELEASE);
    for role in ["author", "finisher", "reviewer"] {
        let rules = current.inspection_role_rules(role, true).unwrap();
        assert!(current.verify_existing(&json!({"seriesLiteContract":current.contract_identity()}), role, true, &rules).is_ok());
        assert!(old.verify_existing(&json!({"seriesLiteContract":current.contract_identity()}), role, true, &rules).is_err());
        assert!(current.verify_existing(&json!({"seriesLiteContract":old.contract_identity()}), role, true, &rules).is_err());
    }
    // A frozen file, compiler or active flag cannot grant a still-unaccepted route.
    let mut drift = current.files.clone();
    drift.get_mut("scripts/prepare_episode.py").unwrap().push(b' ');
    assert!(VerifiedPack::verify_files(CURRENT_MANIFEST, drift).is_err());
    assert!(current.require_new_production().is_err());
}

#[test]
fn role_selection_includes_system_contract_and_full_shared_methods_once() {
    let p = pack();
    let methods = [
        "acting-blocking-methods",
        "camera-emotion-methods",
        "sound-rhythm-methods",
        "lighting-space-methods",
        "director-refinement",
        "fantasy-expression-methods",
    ];
    let contracts = [
        "system-presentation",
        "asset-registry-contract",
        "shot-handoffs",
    ];
    for role in ["author", "finisher", "reviewer"] {
        let rules = p.selected_role_rules(role, &methods, &contracts).unwrap();
        assert_eq!(rules.len(), 5);
        assert_eq!(
            rules["references/system.md"],
            p.files["references/system.md"]
        );
        assert_eq!(
            rules["references/creative-core.md"],
            p.files["references/creative-core.md"]
        );
        assert!(p.selected_role_rules(role, &["unknown"], &[]).is_err());
        assert!(p.selected_role_rules(role, &[], &["unknown"]).is_err());
        assert!(p
            .selected_role_rules(role, &["director-refinement", "director-refinement"], &[])
            .is_err());
        assert!(p
            .selected_role_rules(role, &[], &["system-presentation", "system-presentation"])
            .is_err());
    }
}

#[test]
fn pinned_release_checks_all_actual_bytes_and_keeps_activation_closed() {
    let p = pack();
    assert_eq!(p.files.len(), 90);
    assert_eq!(hash(MANIFEST), MANIFEST_SHA256);
    let report = p.inspection();
    assert_eq!(report["contract"]["creativeVersion"], "0.1.2");
    assert_eq!(report["contract"]["sourceCommit"], SOURCE_COMMIT);
    assert_eq!(report["active"], false);
    assert_eq!(report["fullRustGateAccepted"], false);
    assert!(p
        .require_new_production()
        .unwrap_err()
        .contains("SERIES_LITE_INACTIVE"));
}
#[test]
fn complete_role_bytes_reject_missing_extra_changed_or_unknown_material() {
    let p = pack();
    for role in ["author", "finisher", "reviewer"] {
        for fantasy in [false, true] {
            let rules = p.inspection_role_rules(role, fantasy).unwrap();
            assert_eq!(rules.len(), if fantasy { 4 } else { 3 });
            p.verify_runtime_rules(role, fantasy, &rules).unwrap();
            let mut changed = rules.clone();
            changed.get_mut("references/schema.md").unwrap().push(b' ');
            assert!(p.verify_runtime_rules(role, fantasy, &changed).is_err());
            let mut missing = rules.clone();
            missing.remove("references/creative-core.md");
            assert!(p.verify_runtime_rules(role, fantasy, &missing).is_err());
            let mut extra = rules;
            extra.insert("summary-instead-of-rules.md".into(), b"PASS".to_vec());
            assert!(p.verify_runtime_rules(role, fantasy, &extra).is_err());
        }
    }
    assert!(p.inspection_role_rules("planner", false).is_err());
}
#[test]
fn provenance_rejects_any_frozen_file_drift_even_non_role_scripts() {
    let p = pack();
    for name in ["SKILL.md", "references/schema.md", "scripts/lite_edit.py"] {
        let mut files = p.files.clone();
        files.get_mut(name).unwrap().push(b' ');
        assert!(
            VerifiedPack::verify_files(MANIFEST, files).is_err(),
            "{name}"
        );
    }
    let mut missing = p.files.clone();
    missing.remove("VERSION");
    assert!(VerifiedPack::verify_files(MANIFEST, missing).is_err());
    let mut unknown = p.files;
    unknown.insert("unknown.md".into(), vec![]);
    assert!(VerifiedPack::verify_files(MANIFEST, unknown).is_err());
    let mut manifest = MANIFEST.to_vec();
    manifest.push(b'\n');
    assert!(super::manifest(&manifest).is_err());
}
#[test]
fn old_requests_are_not_resigned_and_version_spoofs_never_activate() {
    let p = pack();
    let rules = p.inspection_role_rules("author", false).unwrap();
    let old = json!({"workflowProfile":"series672","rules":{"original":"immutable"},"status":"completed"});
    let before = old.clone();
    assert_eq!(
        p.verify_existing(&old, "author", false, &BTreeMap::new())
            .unwrap(),
        ExistingContract::LegacyUnchanged
    );
    assert_eq!(old, before);
    let frozen = json!({"seriesLiteContract":identity()});
    assert_eq!(
        p.verify_existing(&frozen, "author", false, &rules).unwrap(),
        ExistingContract::VerifiedFrozenRulesOnly
    );
    for field in [
        "creativeVersion",
        "releaseVersion",
        "sourceCommit",
        "checksumFileSha256",
        "rustAdapterVersion",
        "active",
        "dataContracts",
    ] {
        let mut forged = frozen.clone();
        forged["seriesLiteContract"][field] = if field == "active" {
            json!(true)
        } else {
            json!("unknown")
        };
        assert!(
            p.verify_existing(&forged, "author", false, &rules).is_err(),
            "{field}"
        );
        assert!(p.require_new_production().is_err());
    }
}
