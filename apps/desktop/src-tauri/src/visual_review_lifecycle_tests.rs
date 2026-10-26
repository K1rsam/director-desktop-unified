//! Synthetic receipt/storage tests only: no SDK, browser or generation calls.
use super::*;

fn candidate(f: &Fixture, asset: &Value, seed: u8, references: Value) -> Value {
    let file = f.dir.path().join(format!("{}-{seed}.png", s(asset, "id")));
    image::RgbImage::from_fn(32, 32, |x, y| image::Rgb([seed, x as u8, y as u8]))
        .save(&file)
        .unwrap();
    let media = f
        .store
        .import_media(
            &file,
            "image",
            Some(s(asset, "id")),
            Some(s(asset, "variantId")),
        )
        .unwrap();
    let job = f.store.add_job("image", s(asset, "id"), "offline", "offline",
        &json!({"workflowProfile":crate::asset_graph::PROFILE,"assetId":asset["id"],"variantId":asset["variantId"],
            "assetVersion":asset["version"],"promptBody":crate::pipeline::asset_workflow::prompt_text(asset),
            "references":references,"frozenSettings":{},"imageAdoptionPolicy":"manual"}), "downloaded").unwrap();
    f.store.attach_result(&job, s(&media, "id")).unwrap();
    media
}

fn repair_receipt(f: &Fixture, asset_id: &str, media: &Value) -> Value {
    let basis = candidate_basis(&f.store, asset_id, s(media, "id")).unwrap();
    let path = allocate(&f.store, &format!("candidate-synthetic-{}", id())).unwrap();
    let image = freeze_image(&path, 1, "candidate", media, Path::new(s(media, "path"))).unwrap();
    // Historical structural identity is enough for an offline evidence test;
    // no binary exists at this deliberately non-executable synthetic path.
    let identity = json!({"version":"official-sdk-identity/v1","provider":"codex_sdk",
        "sdkPackage":"@openai/codex-sdk","sdkVersion":crate::text_sdk::SDK_VERSION,
        "sdkImplementationSha256":crate::text_sdk::SDK_IMPLEMENTATION_SHA,"bridgeVersion":"0.1.0",
        "bridgeSha256":"a".repeat(64),"protocolVersion":crate::text_sdk::PROTOCOL,
        "nodeExecutable":f.dir.path().join("not-executed-node"),"nodeSha256":"b".repeat(64),"nodeVersion":"v1.0.0",
        "runtimePackageVersion":crate::text_sdk::SDK_VERSION,"runtimeIdentity":"bundled-synthetic-fixture",
        "permissionProfile":crate::text_sdk::POLICY,"accountFingerprint":"synthetic-no-account"});
    let plan = manifest(
        &path,
        "candidate",
        identity.clone(),
        vec![image.clone()],
        "Synthetic completed image review; never dispatched".into(),
        review_schema(),
        basis.clone(),
    )
    .unwrap();
    let manifest_sha = hash_file(&path.join("manifest.json")).unwrap().0;
    let mut output = passed(1);
    output["decision"] = json!("REPAIR");
    output["checks"][0]["result"] = json!("fail");
    output["issues"] =
        json!(["Synthetic visible style mismatch; the original picture remains preserved"]);
    fs::write(path.join("events.ndjson"), b"synthetic offline receipt\n").unwrap();
    let events_sha = hash_file(&path.join("events.ndjson")).unwrap().0;
    atomic_json(&path.join("sdk-receipt.json"), &json!({"version":"visual-sdk-receipt/v1",
        "state":"transport_completed","attemptId":plan["attemptId"],"manifestSha256":manifest_sha,
        "inputHashes":[image["sha256"]],"finalText":output.to_string(),"finalTextSha256":digest(&output.to_string()),
        "model":"gpt-6.1-sol","reasoning":"medium","visualInputAttached":true,"eventsSha256":events_sha})).unwrap();
    atomic_json(&path.join("runtime-process-receipt.json"), &json!({"attemptId":plan["attemptId"],
        "manifestSha256":manifest_sha,"exitCode":0,"ioComplete":true,"tree":{"processTreeStopProven":true}})).unwrap();
    atomic_json(
        &path.join("process-receipt.json"),
        &json!({"attemptId":plan["attemptId"],"manifestSha256":manifest_sha,
        "exitCode":0,"tree":{"processTreeStopProven":true},"eventsSha256":events_sha,
        "sdkReceiptSha256":hash_file(&path.join("sdk-receipt.json")).unwrap().0,
        "runtimeReceiptSha256":hash_file(&path.join("runtime-process-receipt.json")).unwrap().0}),
    )
    .unwrap();
    assert_eq!(terminal(&path).unwrap().1, output);
    json!({"version":"asset-visual-review/v1","status":"needs_repair","assetId":asset_id,
        "mediaId":media["id"],"basisSha256":hash(&basis),"basis":basis,"attemptDirectory":path,
        "manifestSha256":manifest_sha,"scopeSha256":scope(&identity),"checkedAt":now(),"output":output,"adopted":false})
}

fn adopted_with_reference(
    f: &Fixture,
    id: &str,
    mother: &Value,
    parent_media: &Value,
    seed: u8,
) -> Value {
    let mut child = f.asset(id);
    child["referenceRequirements"] =
        json!([{"targetAssetId":mother["id"],"role":"reference","required":true}]);
    let child = f.store.save_asset(child).unwrap();
    let references = json!([{"assetId":mother["id"],"variantId":mother["variantId"],"mediaId":parent_media["id"],
        "sha256":parent_media["sha256"],"usage":"reference"}]);
    let media = candidate(f, &child, seed, references);
    f.store
        .adopt(id, s(&child, "variantId"), s(&media, "id"))
        .unwrap();
    media
}

#[test]
fn current_adopted_repair_blocks_export_and_media_consumers_but_preserves_selection_and_history() {
    let f = Fixture::new();
    let mother = f.asset("mother");
    let media = candidate(&f, &mother, 20, json!([]));
    f.store.adopt("mother", "v1", s(&media, "id")).unwrap();
    let child_media = adopted_with_reference(&f, "child", &mother, &media, 40);
    let child = f.store.active_asset("child").unwrap();
    adopted_with_reference(&f, "grandchild", &child, &child_media, 60);
    let mut independent = f.asset("family-only");
    independent["sourceAssetId"] = json!("mother");
    let independent = f.store.save_asset(independent).unwrap();
    let independent_media = candidate(&f, &independent, 80, json!([]));
    f.store
        .adopt("family-only", "v1", s(&independent_media, "id"))
        .unwrap();
    let before = f.store.values("assets").unwrap();
    let before_jobs = f.store.jobs().unwrap();
    let original_bytes = fs::read(s(&media, "path")).unwrap();
    let review = repair_receipt(&f, "mother", &media);
    let raw_path = Path::new(s(&review, "attemptDirectory")).join("sdk-receipt.json");
    let raw_bytes = fs::read(&raw_path).unwrap();
    f.store
        .record_visual_review(s(&media, "id"), &review)
        .unwrap();
    let current = f.store.active_asset("mother").unwrap();
    assert_eq!(current["imageStatus"], "needs_repair");
    assert_eq!(
        current["imageReviewReasons"][0]["kind"],
        "visual-review-needs-repair"
    );
    for id in ["child", "grandchild"] {
        assert_eq!(
            f.store.active_asset(id).unwrap()["imageStatus"],
            "needs_review"
        );
    }
    assert_eq!(
        f.store.active_asset("family-only").unwrap()["imageStatus"],
        "adopted"
    );
    let assets = f.store.values("assets").unwrap();
    assert!(crate::asset_graph::media_references(
        &f.store,
        &assets,
        &f.store.active_asset("child").unwrap()
    )
    .unwrap()
    .is_none());
    for old in &before {
        let after = f.store.active_asset(s(old, "id")).unwrap();
        for key in [
            "version",
            "designRevision",
            "adoptedDesign",
            "imagePrompt",
            "imageAdoptionBasis",
            "referenceBindings",
        ] {
            assert_eq!(after[key], old[key], "{}:{key}", s(old, "id"));
        }
    }
    assert_eq!(
        f.store.adopted("mother", "v1").unwrap().as_deref(),
        media["id"].as_str()
    );
    let directory = f.dir.path().join("export");
    fs::create_dir(&directory).unwrap();
    let exported = f
        .store
        .export_current_images(&json!({"path":directory}))
        .unwrap();
    assert_eq!(exported["count"], 1);
    assert_eq!(exported["missingCount"], 3);
    assert!(exported["missing"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["reason"] == "needs_review"));
    let manifest = read(Path::new(s(&exported, "manifestPath"))).unwrap();
    assert_eq!(manifest["exported"][0]["mediaId"], independent_media["id"]);
    // Neither replay nor reselecting identical bytes is a fresh quality decision.
    f.store
        .record_visual_review(s(&media, "id"), &review)
        .unwrap();
    f.store.adopt("mother", "v1", s(&media, "id")).unwrap();
    assert_eq!(
        f.store.active_asset("mother").unwrap()["imageStatus"],
        "needs_repair"
    );
    let count: i64 = f
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type='visual_review_current_image_needs_repair'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(f.store.jobs().unwrap(), before_jobs);
    assert_eq!(fs::read(s(&media, "path")).unwrap(), original_bytes);
    assert_eq!(fs::read(raw_path).unwrap(), raw_bytes);
    let reopened = Store::open(f.store.root.clone()).unwrap();
    assert_eq!(
        reopened.active_asset("mother").unwrap()["imageStatus"],
        "needs_repair"
    );
    assert_eq!(
        reopened.active_asset("child").unwrap()["imageStatus"],
        "needs_review"
    );
    assert_eq!(
        reopened.adopted("mother", "v1").unwrap(),
        f.store.adopted("mother", "v1").unwrap()
    );
}

#[test]
fn historical_candidate_repair_never_invalidates_later_current_image_or_its_consumers() {
    let f = Fixture::new();
    let mother = f.asset("mother");
    let old = candidate(&f, &mother, 20, json!([]));
    f.store.adopt("mother", "v1", s(&old, "id")).unwrap();
    let review = repair_receipt(&f, "mother", &old);
    let new = candidate(&f, &mother, 80, json!([]));
    assert_ne!(old["id"], new["id"]);
    f.store.adopt("mother", "v1", s(&new, "id")).unwrap();
    adopted_with_reference(&f, "child", &mother, &new, 40);
    let before = f.store.values("assets").unwrap();
    let jobs = f.store.jobs().unwrap();
    f.store
        .record_visual_review(s(&old, "id"), &review)
        .unwrap();
    assert_eq!(f.store.values("assets").unwrap(), before);
    assert_eq!(
        f.store.get("media", s(&old, "id")).unwrap()["visualReview"]["status"],
        "needs_repair"
    );
    assert_eq!(
        f.store.adopted("mother", "v1").unwrap().as_deref(),
        new["id"].as_str()
    );
    assert!(crate::asset_graph::media_references(
        &f.store,
        &before,
        &f.store.active_asset("child").unwrap()
    )
    .unwrap()
    .is_some());
    assert_eq!(f.store.jobs().unwrap(), jobs);
}

#[test]
fn current_repair_requires_original_receipt_and_current_basis_before_any_projection_write() {
    for fault in [
        "result",
        "basis",
        "manifest",
        "current-prompt",
        "current-bytes",
    ] {
        let f = Fixture::new();
        let asset = f.asset("a");
        let media = candidate(&f, &asset, 20, json!([]));
        f.store.adopt("a", "v1", s(&media, "id")).unwrap();
        let mut review = repair_receipt(&f, "a", &media);
        match fault {
            "result" => review["output"]["issues"] = json!(["tampered"]),
            "basis" => review["basis"]["name"] = json!("tampered"),
            "manifest" => review["manifestSha256"] = json!("f".repeat(64)),
            "current-prompt" => {
                let mut next = f.store.active_asset("a").unwrap();
                next["imagePrompt"]["text"] = json!("Changed current design direction");
                f.store.save_asset(next).unwrap();
            }
            "current-bytes" => fs::write(s(&media, "path"), b"changed media bytes").unwrap(),
            _ => unreachable!(),
        }
        let before = f.store.active_asset("a").unwrap();
        let jobs = f.store.jobs().unwrap();
        assert!(
            f.store
                .record_visual_review(s(&media, "id"), &review)
                .is_err(),
            "{fault}"
        );
        assert_eq!(f.store.active_asset("a").unwrap(), before, "{fault}");
        assert!(
            f.store.get("media", s(&media, "id")).unwrap()["visualReview"].is_null(),
            "{fault}"
        );
        assert_eq!(f.store.jobs().unwrap(), jobs, "{fault}");
        assert_eq!(
            f.store.adopted("a", "v1").unwrap().as_deref(),
            media["id"].as_str()
        );
    }
}

#[test]
fn unknown_current_image_review_preserves_selection_and_never_creates_a_generation() {
    let f = Fixture::new();
    let asset = f.asset("a");
    let media = candidate(&f, &asset, 20, json!([]));
    f.store.adopt("a", "v1", s(&media, "id")).unwrap();
    let before = f.store.active_asset("a").unwrap();
    let jobs = f.store.jobs().unwrap();
    f.store
        .record_visual_review(
            s(&media, "id"),
            &json!({"status":"unknown","assetId":"a","mediaId":media["id"],"adopted":false}),
        )
        .unwrap();
    assert_eq!(f.store.active_asset("a").unwrap(), before);
    assert_eq!(f.store.jobs().unwrap(), jobs);
    assert_eq!(
        f.store.adopted("a", "v1").unwrap().as_deref(),
        media["id"].as_str()
    );
}

/// Reproduce the old storage contract only inside this isolated fixture. The
/// product is never allowed to patch an on-site database to obtain acceptance.
fn legacy_review_projection(f: &Fixture, media: &Value, review: &Value) {
    let connection = f.store.conn().unwrap();
    connection.execute(
        "INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![format!("visual-review.media:{}",s(media,"id")),review.to_string()],
    ).unwrap();
    connection
        .execute(
            "UPDATE media SET data=json_set(data,'$.visualReview',json(?1)) WHERE id=?2",
            rusqlite::params![review.to_string(), s(media, "id")],
        )
        .unwrap();
}

#[test]
fn legacy_negative_is_read_only_pending_review_without_sdk_or_receipt_io() {
    let f = Fixture::new();
    let asset = f.asset("a");
    let media = candidate(&f, &asset, 20, json!([]));
    f.store.adopt("a", "v1", s(&media, "id")).unwrap();
    let review = repair_receipt(&f, "a", &media);
    legacy_review_projection(&f, &media, &review);
    // Public current-basis validation would now fail. The old picture still
    // exists and must not become usable merely because the words changed.
    let mut changed = f.store.get("assets", "a").unwrap();
    changed["imagePrompt"]["text"] = json!("A revised manual prompt after the old review");
    f.store.save_asset(changed).unwrap();
    let raw_asset = f.store.get("assets", "a").unwrap();
    assert_eq!(raw_asset["imageStatus"], "adopted");
    let jobs = f.store.jobs().unwrap();
    let raw_review = f.store.get("media", s(&media, "id")).unwrap();
    let bytes = fs::read(s(&media, "path")).unwrap();
    // Cheap projection must survive unavailable historical receipt files.
    fs::rename(
        Path::new(s(&review, "attemptDirectory")),
        f.dir.path().join("receipts-offline"),
    )
    .unwrap();
    for _ in 0..3 {
        let current = f.store.active_asset("a").unwrap();
        assert_eq!(current["imageStatus"], "needs_review");
        assert_eq!(current["imageReviewReasons"][0]["mediaId"], media["id"]);
        assert_eq!(
            f.store.values("assets").unwrap()[0]["imageStatus"],
            "needs_review"
        );
    }
    let child = f.asset("child");
    let mut child = child.clone();
    child["referenceRequirements"] =
        json!([{"targetAssetId":"a","role":"reference","required":true}]);
    let child = f.store.save_asset(child).unwrap();
    assert!(crate::asset_graph::media_references(
        &f.store,
        &f.store.values("assets").unwrap(),
        &child
    )
    .unwrap()
    .is_none());
    let directory = f.dir.path().join("export");
    fs::create_dir(&directory).unwrap();
    let exported = f
        .store
        .export_current_images(&json!({"path":directory}))
        .unwrap();
    assert_eq!(exported["count"], 0);
    assert!(exported["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["assetId"] == "a" && row["reason"] == "needs_review"));
    assert_eq!(f.store.get("assets", "a").unwrap(), raw_asset);
    assert_eq!(f.store.get("media", s(&media, "id")).unwrap(), raw_review);
    assert_eq!(f.store.jobs().unwrap(), jobs);
    assert_eq!(fs::read(s(&media, "path")).unwrap(), bytes);
    let reopened = Store::open(f.store.root.clone()).unwrap();
    assert_eq!(
        reopened.active_asset("a").unwrap()["imageStatus"],
        "needs_review"
    );
}

#[test]
fn drifted_original_replay_retains_negative_and_does_not_taint_new_adoption() {
    let f = Fixture::new();
    let asset = f.asset("a");
    let old = candidate(&f, &asset, 20, json!([]));
    f.store.adopt("a", "v1", s(&old, "id")).unwrap();
    let review = repair_receipt(&f, "a", &old);
    recovery::bind_pointer(&f.store, Path::new(s(&review, "attemptDirectory")), false).unwrap();
    legacy_review_projection(&f, &old, &review);
    let mut changed = f.store.get("assets", "a").unwrap();
    changed["imagePrompt"]["text"] = json!("Revised manual words after the completed old review");
    f.store.save_asset(changed).unwrap();
    let before_jobs = f.store.jobs().unwrap();
    for _ in 0..2 {
        let result = reconcile(&f.store, &json!({}), &json!({"mediaId":old["id"]})).unwrap();
        assert_eq!(result["modelCalls"], 0);
        assert_eq!(result["status"], "manual_required");
        let saved: String = f
            .store
            .conn()
            .unwrap()
            .query_row(
                "SELECT value FROM metadata WHERE key=?1",
                [format!("visual-review.media:{}", s(&old, "id"))],
                |r| r.get(0),
            )
            .unwrap();
        let saved: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved["originalReview"], review);
        assert_eq!(
            f.store.active_asset("a").unwrap()["imageStatus"],
            "needs_review"
        );
        assert_eq!(
            f.store.get("media", s(&old, "id")).unwrap()["visualReview"]["originalReview"],
            review
        );
    }
    assert_eq!(f.store.jobs().unwrap(), before_jobs);
    let changed = f.store.get("assets", "a").unwrap();
    let new = candidate(&f, &changed, 80, json!([]));
    f.store.adopt("a", "v1", s(&new, "id")).unwrap();
    assert_eq!(f.store.active_asset("a").unwrap()["imageStatus"], "adopted");
    assert_eq!(
        f.store.values("assets").unwrap()[0]["imageStatus"],
        "adopted"
    );
    assert_eq!(
        f.store.adopted("a", "v1").unwrap().as_deref(),
        new["id"].as_str()
    );
    let directory = f.dir.path().join("export");
    fs::create_dir(&directory).unwrap();
    let exported = f
        .store
        .export_current_images(&json!({"path":directory}))
        .unwrap();
    assert_eq!(exported["count"], 1);
    assert_eq!(exported["missingCount"], 0);
    assert_eq!(
        f.store.get("media", s(&old, "id")).unwrap()["visualReview"]["originalReview"],
        review
    );
}
