//! Synthetic stored approval exercises transport/storage only. These fixtures
//! deliberately do not claim a live model, semantic review or image approval.
use super::*;
use crate::series_lite_contract::VerifiedPack;
use crate::workflow_artifacts::{Adopt, Artifact, Inputs, Verdict, Verifier};

struct SyntheticOnly(Capability);
impl Verifier for SyntheticOnly {
    fn id(&self) -> &str {
        "synthetic-only/export-boundary-not-content-acceptance/v1"
    }
    fn capability(&self) -> Capability {
        self.0
    }
    fn verify(&self, _: &StoredArtifact, _: &StoredArtifact) -> Result<Verdict> {
        Ok(Verdict {
            accepted: true,
            findings: json!({"syntheticOnly":true,"contentQuality":"NOT_PROVEN"}),
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
    target: std::path::PathBuf,
    record: Value,
    native: Vec<u8>,
    markdown: Vec<u8>,
    catalog: Vec<u8>,
    lock: Vec<u8>,
    registry: Vec<u8>,
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store.create("synthetic export", "synthetic").unwrap();
    let corpus: Value =
        serde_json::from_str(include_str!("../series_lite_execution_golden.json")).unwrap();
    let workspace = temp.path().join("mechanical");
    fs::create_dir(&workspace).unwrap();
    // Production canonicalizes the workspace before using the strict package
    // path reader. Match that contract (including Windows verbatim prefixes).
    let workspace = root_path(&workspace).unwrap();
    for (name, text) in corpus["files"].as_object().unwrap() {
        let path = workspace.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text.as_str().unwrap().as_bytes()).unwrap();
    }
    let locked =
        crate::pipeline::prepare_source(s(&corpus["files"], "source-original.txt")).unwrap();
    let decisions: Vec<_> = [(0, 1, "门厅"), (2, 3, "走廊")]
        .into_iter()
        .map(|(first, last, title)| crate::pipeline::BoundaryDecision {
            start: locked.rows[first].range.start,
            end: locked.rows[last].range.end,
            kind: "body".into(),
            episode_number: Some(1),
            scene_title: Some(title.into()),
            reason: "合成测试中的明确用户范围决定".into(),
            decision_version: None,
            confirmation: None,
        })
        .collect();
    let recognized = crate::pipeline::apply_boundary_decisions(&locked, &decisions).unwrap();
    assert_eq!(recognized.sha256, locked.sha256);
    assert_eq!(recognized.episodes.len(), 1);
    let source = serde_json::to_value(recognized).unwrap();
    let episode = source["episodes"][0].clone();
    let source = store.save_source(source).unwrap();
    let execution = crate::series_lite_execution::run(
        &workspace,
        "execution-request.json",
        "out",
        &VerifiedPack::embedded().unwrap(),
        None,
    )
    .unwrap();
    let native = s(&corpus["files"], "bundle.json").as_bytes().to_vec();
    let markdown = fs::read(workspace.join(&execution.preview_path)).unwrap();
    let catalog =
        fs::read(workspace.join(format!("{}.assets.json", execution.preview_path))).unwrap();
    let receipt = value(&fs::read(workspace.join(&execution.receipt_path)).unwrap()).unwrap();
    let registry = pin(&workspace, &receipt["registrySnapshot"]).unwrap();
    let lock = s(&corpus["files"], "source-lock.json").as_bytes().to_vec();
    let mut conn = store.conn().unwrap();
    let tx = conn.transaction().unwrap();
    artifacts::install(&tx).unwrap();
    let inputs = Inputs::from([("source".into(), s(&source, "sha256").into())]);
    let raw = Artifact {
        id: "synthetic-original".into(),
        identity: "synthetic-episode/native".into(),
        kind: Kind::Raw,
        contract_version: "synthetic-export/v1".into(),
        source_sha256: s(&source, "sha256").into(),
        inputs: inputs.clone(),
        producer: "synthetic-only".into(),
        author_attempt: "synthetic-author".into(),
        parents: vec![],
        payload: json!({"text":std::str::from_utf8(&native).unwrap(),"syntheticOnly":true}),
    };
    artifacts::put(&tx, &raw).unwrap();
    let mut candidate = raw.clone();
    candidate.id = "synthetic-candidate".into();
    candidate.kind = Kind::Candidate;
    candidate.parents = vec![raw.id.clone()];
    let candidate = artifacts::put(&tx, &candidate).unwrap();
    let evidence = Artifact {
        id: "synthetic-independent-review".into(),
        identity: raw.identity.clone(),
        kind: Kind::ReviewRaw,
        contract_version: raw.contract_version.clone(),
        source_sha256: raw.source_sha256.clone(),
        inputs: inputs.clone(),
        producer: "synthetic-only".into(),
        author_attempt: "synthetic-reviewer".into(),
        parents: vec![candidate.artifact.id.clone()],
        payload: json!({"candidateSha256":candidate.payload_sha256,"inputSha256":candidate.input_sha256,
            "authorAttempt":candidate.artifact.author_attempt,"reviewerAttempt":"synthetic-reviewer","syntheticOnly":true}),
    };
    artifacts::put(&tx, &evidence).unwrap();
    let capabilities = [
        Capability::Structure,
        Capability::SourceCoverage,
        Capability::ContentReview,
    ];
    let mut validations = Vec::new();
    for (i, cap) in capabilities.iter().enumerate() {
        let id = format!("synthetic-validation-{i}");
        artifacts::validate(
            &tx,
            &id,
            &candidate.artifact.id,
            &evidence.id,
            &SyntheticOnly(*cap),
        )
        .unwrap();
        validations.push(id);
    }
    let adoption = artifacts::adopt(
        &tx,
        Adopt {
            id: "synthetic-adoption",
            candidate_id: &candidate.artifact.id,
            identity: &candidate.artifact.identity,
            expected_previous: 0,
            source_sha256: &candidate.artifact.source_sha256,
            contract_version: &candidate.artifact.contract_version,
            current_inputs: &inputs,
            required: &capabilities,
            validation_ids: &validations,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    let record = json!({"version":"desktop-lite-delivery/v1","episodeId":episode["id"],"episodeNumber":1,
        "sourceSha256":source["sha256"],"nativeSha256":sha256(&native),"workspaceRoot":workspace,
        "receiptPath":execution.receipt_path,"receiptSha256":execution.receipt_file_sha256,
        "candidateArtifactId":candidate.artifact.id,"adoption":adoption,"finalDeliveryReady":true,
        "markdownSha256":sha256(&markdown),"manifestSha256":sha256(&catalog),"syntheticOnly":true});
    let target = temp.path().join("exports");
    fs::create_dir(&target).unwrap();
    Fixture {
        _temp: temp,
        store,
        target,
        record,
        native,
        markdown,
        catalog,
        lock,
        registry,
    }
}

#[test]
fn standalone_export_auto_catalog_import_and_reopen_preserve_bytes_identity_and_no_generation() {
    let f = fixture();
    let exported = write(&f.store, &[f.record.clone()], &f.target).unwrap();
    let root = Path::new(s(&exported, "path"));
    assert_eq!(exported["generationSubmissions"], 0);
    assert!(
        !root.join("manifest.json").exists(),
        "must not impersonate the old complete-package reader"
    );
    for (name, raw) in [
        ("第1集.md", &f.markdown),
        ("第1集.md.assets.json", &f.catalog),
        ("第1集.bundle.json", &f.native),
        ("第1集.source-lock.json", &f.lock),
        ("第1集.registry.json", &f.registry),
    ] {
        assert_eq!(fs::read(root.join(name)).unwrap(), *raw);
    }
    let index = value(&fs::read(root.join("delivery-index.json")).unwrap()).unwrap();
    assert_eq!(index["episodes"][0]["sourceOrdinal"], 0);
    assert_eq!(index["episodes"][0]["episodeNumber"], 1);
    assert_eq!(index["episodes"][0]["adoption"], f.record["adoption"]);
    assert_eq!(verify_export(root).unwrap()["mechanicalVerified"], true);

    // Move only the independent export into a clean location. It must not depend
    // on the original workspace for existing local MD/catalog import.
    let isolated = f._temp.path().join("independent-handoff");
    fs::rename(root, &isolated).unwrap();
    fs::rename(
        Path::new(s(&f.record, "workspaceRoot")),
        f._temp.path().join("original-offline"),
    )
    .unwrap();
    assert_eq!(verify_export(&isolated).unwrap()["episodeCount"], 1);
    let mut request = json!({"path":isolated.join("第1集.md")});
    let preview = crate::lingmo_import::preview_file(&request).unwrap();
    assert_eq!(
        preview["catalogMatched"], true,
        "exact .md.assets.json auto discovery"
    );
    assert_eq!(preview["handoff"]["sourceKind"], "imported-md");
    assert_eq!(preview["handoff"]["level"], "md-and-catalog");
    assert_eq!(preview["handoff"]["completePackageVerified"], false);
    request["previewToken"] = preview["previewToken"].clone();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    let imported_root = f._temp.path().join("clean-import");
    let imported = Store::open(imported_root.clone()).unwrap();
    imported.create("synthetic imported", "synthetic").unwrap();
    let report = crate::lingmo_import::import(&imported, &request).unwrap();
    assert_eq!(report["generationSubmissions"], 0);
    assert_eq!(report["counts"], preview["counts"]);
    assert!(
        imported.source().unwrap().is_null(),
        "do not invent stage-one source"
    );
    assert!(imported.jobs().unwrap().is_empty());
    let original =
        crate::lingmo_import::original(&imported, s(&report, "documentId"), None).unwrap();
    assert_eq!(s(&original, "text").as_bytes(), f.markdown);
    let catalog = value(&f.catalog).unwrap();
    let assets = imported.values("assets").unwrap();
    for expected in catalog["assets"].as_array().unwrap() {
        let actual = assets
            .iter()
            .find(|a| a["externalId"] == expected["id"])
            .unwrap();
        assert_eq!(actual["assetRevision"], expected["assetRevision"]);
        assert_eq!(actual["filename"], expected["filename"]);
    }
    let units = imported.values("units").unwrap();
    let expected_units = catalog["units"].as_array().unwrap();
    assert_eq!(units.len(), expected_units.len());
    for expected in expected_units {
        let actual = units
            .iter()
            .find(|u| u["importSource"]["externalUnitId"] == expected["unitId"])
            .unwrap();
        assert_eq!(actual["sceneNumber"], expected["sceneId"]);
        assert_eq!(actual["fragmentNumber"], expected["fragment"]);
        assert_eq!(
            actual["durationSeconds"].as_f64(),
            expected["containerSeconds"].as_f64()
        );
    }
    drop(imported);
    let reopened = Store::open(imported_root).unwrap();
    assert!(reopened.source().unwrap().is_null());
    assert!(reopened.jobs().unwrap().is_empty());
    assert_eq!(
        crate::lingmo_import::original(&reopened, s(&report, "documentId"), None).unwrap(),
        original
    );
    assert_eq!(reopened.values("units").unwrap(), units);
    assert_eq!(reopened.values("assets").unwrap(), assets);
}

#[test]
fn output_tampering_is_detected_and_separate_exports_never_overwrite_prior_files() {
    let f = fixture();
    let first = write(&f.store, &[f.record.clone()], &f.target).unwrap();
    let second = write(&f.store, &[f.record.clone()], &f.target).unwrap();
    assert_ne!(first["path"], second["path"]);
    let root = Path::new(s(&first, "path"));
    fs::write(root.join("第1集.md"), "人工更改，不能成为旧版采用").unwrap();
    assert!(verify_export(root)
        .unwrap_err()
        .contains("FROZEN_FILE_CHANGED"));
    assert_eq!(
        fs::read(Path::new(s(&second, "path")).join("第1集.md")).unwrap(),
        f.markdown
    );
    verify_export(Path::new(s(&second, "path"))).unwrap();
}

#[test]
fn invalid_selection_and_changed_provenance_fail_before_creating_any_export() {
    let f = fixture();
    for (pointer, replacement, expected) in [
        ("/finalDeliveryReady", json!(false), "UNADOPTED"),
        ("/sourceSha256", json!("f".repeat(64)), "SOURCE_CHANGED"),
        ("/nativeSha256", json!("f".repeat(64)), "NATIVE_CHANGED"),
        ("/markdownSha256", json!("f".repeat(64)), "DELIVERY_CHANGED"),
        ("/adoption/version", json!(2), "ADOPTION_CHANGED"),
        (
            "/episodeNumber",
            json!(2),
            "ORIGINAL_EPISODE_NUMBER_CHANGED",
        ),
    ] {
        let mut bad = f.record.clone();
        *bad.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            write(&f.store, &[bad], &f.target)
                .unwrap_err()
                .contains(expected),
            "{pointer}"
        );
        assert_eq!(fs::read_dir(&f.target).unwrap().count(), 0);
    }
    assert!(
        write(&f.store, &[f.record.clone(), f.record.clone()], &f.target)
            .unwrap_err()
            .contains("DUPLICATE_EPISODE")
    );
    assert!(write(&f.store, &[], &f.target)
        .unwrap_err()
        .contains("EMPTY_SELECTION"));
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 0);
    assert!(f.store.jobs().unwrap().is_empty());
}

#[test]
fn corrupted_frozen_delivery_remains_retained_and_is_not_regenerated() {
    let f = fixture();
    let root = Path::new(s(&f.record, "workspaceRoot"));
    let receipt = value(&fs::read(root.join(s(&f.record, "receiptPath"))).unwrap()).unwrap();
    let md = root.join(s(&receipt, "preview"));
    fs::write(&md, "changed network receipt must never trigger generation").unwrap();
    assert!(write(&f.store, &[f.record.clone()], &f.target).is_err());
    assert_eq!(
        fs::read(&md).unwrap(),
        b"changed network receipt must never trigger generation"
    );
    assert_eq!(fs::read_dir(&f.target).unwrap().count(), 0);
    assert!(f.store.jobs().unwrap().is_empty());
}
