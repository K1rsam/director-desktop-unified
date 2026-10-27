use super::*;
fn pack() -> VerifiedPack {
    VerifiedPack::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../rules/series-lite/20261027-v0.1.2"))
        .unwrap()
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
    assert!(p.require_new_production().unwrap_err().contains("SERIES_LITE_INACTIVE"));
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
        assert!(VerifiedPack::verify_files(MANIFEST, files).is_err(), "{name}");
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
    assert_eq!(p.verify_existing(&old, "author", false, &BTreeMap::new()).unwrap(), ExistingContract::LegacyUnchanged);
    assert_eq!(old, before);
    let frozen = json!({"seriesLiteContract":identity()});
    assert_eq!(p.verify_existing(&frozen, "author", false, &rules).unwrap(), ExistingContract::VerifiedFrozenRulesOnly);
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
        forged["seriesLiteContract"][field] = if field == "active" { json!(true) } else { json!("unknown") };
        assert!(p.verify_existing(&forged, "author", false, &rules).is_err(), "{field}");
        assert!(p.require_new_production().is_err());
    }
}
