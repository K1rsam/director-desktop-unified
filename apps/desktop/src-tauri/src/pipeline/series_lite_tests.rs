//! Synthetic integration of real deterministic projection/storage boundaries.
//! No live executor, media generation, UI control or content-quality PASS.
use super::*;
use crate::series_lite_bundle::{export_lingmo_v7, Delivery};
use crate::series_lite_bundle_gate::NativeLegacyBundleGate;
use crate::workflow_artifacts::{
    Adopt, Adoption, Artifact, Capability, Inputs, Kind, StoredArtifact, Verdict, Verifier,
};

fn adoption(source_hash: &str) -> Adoption {
    Adoption {
        id: "synthetic-adoption".into(),
        identity: "synthetic-episode/native".into(),
        version: 1,
        previous_version: 0,
        candidate_id: "synthetic-candidate".into(),
        candidate_sha256: "b".repeat(64),
        source_sha256: source_hash.into(),
        input_sha256: "c".repeat(64),
        capabilities: vec![Capability::Structure],
        validation_ids: vec![],
        contract_version: "synthetic/v1".into(),
    }
}
struct ProjectionFixture {
    bytes: Vec<u8>,
    bundle: Value,
    delivery: Delivery,
    registry: Value,
    evidence: Value,
}
fn projection_fixture() -> ProjectionFixture {
    let mut bundle: Value = serde_json::from_str(include_str!(
        "../../../../../rules/series-lite/20261028-v0.4.1/tests/fixtures/lingmo-v7-bundle.json"
    ))
    .unwrap();
    bundle["generationBoundaryVersion"] = json!("video-prompt-v1");
    bundle["sourceHash"] = json!("a".repeat(64));
    bundle["assetRegistry"] = json!({"id":"synthetic-registry","revision":1});
    bundle["episodes"][0]["number"] = json!(150);
    bundle["scope"][0]["number"] = json!(150);
    for (i, scene) in bundle["episodes"][0]["scenes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        scene["id"] = json!(format!("150-{}", i + 1));
        scene["desktopSceneId"] = json!(format!("desktop-scene-{}", i + 1));
    }
    bundle["scope"][0]["sceneIds"] = json!(["150-1", "150-2"]);
    for (i, unit) in bundle["episodes"][0]["draft"]["units"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        unit["sceneId"] = json!(format!("150-{}", i + 1));
    }
    bundle["episodes"][0]["draft"]["units"][0]["shots"][0]["duration"] = json!(2.500000001);
    let mut rows = Vec::new();
    for (i, a) in bundle["assets"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        a["assetId"] = json!(format!("A{:04}", i + 1));
        a["assetRevision"] = json!(1);
        a["aliases"] = json!([]);
        a["origin"] = json!({"kind":"source","sourceRefs":["E01:R1"],"basis":"合成源中的资产"});
        a["design"] =
            json!({"state":"TEXT_ADOPTED","text":"合成采用文字设计，保留源中身份与使用关系"});
        let mut row = a.clone();
        row["id"] = a["assetId"].clone();
        row["revision"] = json!(1);
        row["desktopIdentity"] = json!({"desktopAssetId":format!("desktop-asset-{i}"),"variantId":format!("desktop-variant-{i}")});
        rows.push(row);
    }
    // Extra reading-only parent must not become a unit image just because it is
    // in the registry. Production supply additionally enforces this permission.
    rows.push(json!({"id":"A0099","key":"母设计CH","name":"母设计","type":"CH","status":"常态","importName":"母设计","revision":1,
        "placeholderOrdinal":4,"aliases":[],"origin":{"kind":"source","sourceRefs":["E01:R1"],"basis":"合成母设计"},
        "design":{"state":"TEXT_ADOPTED","text":"仅供阅读的完整母设计，不授予上屏或生图权限"},
        "desktopIdentity":{"desktopAssetId":"material-only-parent","variantId":"normal"}}));
    let registry = json!({"version":"series-assets-v1","id":"synthetic-registry","revision":1,
        "sourceHash":"a".repeat(64),"importProfile":"lingmo-v7","assets":rows,"requests":[],"changes":[]});
    let evidence = json!({"sourceMap":{
        "rowMappings":[
            {"standardRowId":"E01:R1","desktopRows":[{"desktopRowId":"row:z-original-first"}]},
            {"standardRowId":"E01:R2","desktopRows":[{"desktopRowId":"row:a-original-second"}]},
            {"standardRowId":"E01:R3","desktopRows":[{"desktopRowId":"row:y-original-third"}]},
            {"standardRowId":"E01:R4","desktopRows":[{"desktopRowId":"row:b-original-fourth"}]}],
        "sceneMappings":[
            {"desktopSceneId":"desktop-scene-1","originalScene":{"id":"desktop-scene-1","title":"150-1 门厅 夜 内"}},
            {"desktopSceneId":"desktop-scene-2","originalScene":{"id":"desktop-scene-2","title":"150-2 走廊 夜 内"}}]}});
    let bytes = serde_json::to_vec(&bundle).unwrap();
    let delivery = export_lingmo_v7(
        &bytes,
        &mut NativeLegacyBundleGate {
            registry: Some(&registry),
            source_lock: None,
            season_scope: None,
        },
    )
    .unwrap();
    ProjectionFixture {
        bytes,
        bundle,
        delivery,
        registry,
        evidence,
    }
}

#[test]
fn real_lingmo_projection_keeps_original_scene_refs_nanos_images_and_source_bound_voice() {
    let f = projection_fixture();
    let before = f.bytes.clone();
    let units = project_units(
        "desktop-episode-150",
        &f.bytes,
        &f.delivery,
        &f.registry,
        &f.evidence,
        &adoption(&"a".repeat(64)),
        false,
    )
    .unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0]["episodeNumber"], 150);
    assert_eq!(units[0]["sceneId"], "desktop-scene-1");
    assert_eq!(units[1]["sceneId"], "desktop-scene-2");
    assert_eq!(
        units[0]["sourceRefs"],
        json!(["row:z-original-first", "row:a-original-second"])
    );
    assert_eq!(
        units[0]["shots"][0]["sourceRefs"],
        json!(["row:z-original-first", "row:a-original-second"])
    );
    assert_eq!(units[0]["actualDurationTicks"], 6_000_000_001i64);
    assert_eq!(units[0]["shots"][0]["startTicks"], 0);
    assert_eq!(units[0]["shots"][0]["endTicks"], 2_500_000_001i64);
    assert_eq!(units[0]["shots"][1]["endTicks"], 6_000_000_001i64);
    assert_eq!(
        units[0]["durationSeconds"], 7,
        "outer ceiling differs from exact inner duration"
    );
    let exact = parse(s(&units[0]["seriesLite"], "nativeUnitUtf8").as_bytes()).unwrap();
    assert_eq!(
        canonical(
            exact.get("shots").unwrap().array().unwrap()[0]
                .get("duration")
                .unwrap()
        ),
        b"2.500000001"
    );
    assert_eq!(
        units[0]["timingProfile"], "lingmo-v7-nanoseconds/v1",
        "must keep the existing desktop protection from legacy regenerate/refine"
    );
    let normalized =
        crate::series_lite_bundle_standard::normalize_source(&f.bundle["episodes"][0]["source"])
            .unwrap();
    let voice = normalized["dialogue"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == "E01:R2:D1")
        .unwrap();
    let parts: Vec<_> = units[0]["shots"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["dialogueParts"].as_array().unwrap())
        .collect();
    assert_eq!(
        parts.iter().map(|p| s(p, "text")).collect::<String>(),
        voice["quote"].as_str().unwrap()
    );
    for part in parts {
        assert_eq!(part["speaker"], voice["speaker"]);
        assert_eq!(part["channel"], voice["channel"]);
    }
    assert_eq!(
        units[0]["assetRefs"].as_array().unwrap().len(),
        f.bundle["episodes"][0]["draft"]["units"][0]["imageBindings"]
            .as_array()
            .unwrap()
            .len()
    );
    assert!(units
        .iter()
        .flat_map(|u| u["assetRefs"].as_array().unwrap())
        .all(|a| a["assetId"] != "material-only-parent"));
    assert!(s(&units[0], "prompt").contains("{{asset:desktop-asset-0:desktop-variant-0}}"));
    assert!(!s(&units[0], "prompt").contains("@["));
    assert_eq!(units[0]["reviewStatus"], "boundary_pending");
    assert_eq!(f.bytes, before);
}

#[test]
fn native_unit_identity_survives_readback_and_changes_in_text_but_is_namespaced_by_episode() {
    let mut f = projection_fixture();
    let adopted = adoption(&"a".repeat(64));
    let first = project_units(
        "E150",
        &f.bytes,
        &f.delivery,
        &f.registry,
        &f.evidence,
        &adopted,
        true,
    )
    .unwrap();
    f.bundle["episodes"][0]["draft"]["units"][0]["title"] = json!("合法新创作版本的标题");
    let bytes = serde_json::to_vec(&f.bundle).unwrap();
    let same_version_delivery = export_lingmo_v7(
        &bytes,
        &mut NativeLegacyBundleGate {
            registry: Some(&f.registry),
            source_lock: None,
            season_scope: None,
        },
    )
    .unwrap();
    let second = project_units(
        "E150",
        &bytes,
        &same_version_delivery,
        &f.registry,
        &f.evidence,
        &adopted,
        true,
    )
    .unwrap();
    assert_eq!(
        first.iter().map(|u| s(u, "id")).collect::<Vec<_>>(),
        second.iter().map(|u| s(u, "id")).collect::<Vec<_>>()
    );
    let other = project_units(
        "E151",
        &f.bytes,
        &f.delivery,
        &f.registry,
        &f.evidence,
        &adopted,
        true,
    )
    .unwrap();
    assert_ne!(first[0]["id"], other[0]["id"]);
    assert_ne!(first[0]["id"], first[1]["id"]);
    assert_eq!(first[0]["seriesLite"]["nativeUnitId"], "U1");
    assert_eq!(first[1]["seriesLite"]["nativeUnitId"], "U2");
}

#[test]
fn source_ref_projection_uses_frozen_row_order_and_refuses_missing_native_coordinates() {
    let f = projection_fixture();
    assert_eq!(
        refs(&json!(["E01:R2", "E01:R1"]), &f.evidence).unwrap(),
        vec![
            json!("row:z-original-first"),
            json!("row:a-original-second")
        ]
    );
    assert!(refs(&json!(["absent-native-row"]), &f.evidence).is_err());
    let mut missing = f.evidence.clone();
    missing["sourceMap"]["rowMappings"][1]["desktopRows"] = json!([]);
    assert!(refs(&json!(["E01:R2"]), &missing).is_err());
}

struct StructureOnly;
impl Verifier for StructureOnly {
    fn id(&self) -> &str {
        "synthetic-json-structure/v1"
    }
    fn capability(&self) -> Capability {
        Capability::Structure
    }
    fn verify(&self, c: &StoredArtifact, e: &StoredArtifact) -> Result<Verdict> {
        Ok(Verdict {
            accepted: c.artifact.payload == e.artifact.payload
                && c.artifact.payload["text"]
                    .as_str()
                    .is_some_and(|s| serde_json::from_str::<Value>(s).is_ok()),
            findings: json!({"syntheticOnly":true,"contentReview":"NOT_PROVEN"}),
        })
    }
}
struct SavedFixture {
    _temp: tempfile::TempDir,
    store: Store,
    input: Value,
    snapshot: Value,
    run: String,
    unit: Value,
    record: Value,
}
fn saved_fixture() -> SavedFixture {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("project")).unwrap();
    store
        .create("synthetic lite continuation", "synthetic")
        .unwrap();
    let corpus: Value =
        serde_json::from_str(include_str!("../series_lite_execution_golden.json")).unwrap();
    let locked =
        crate::pipeline::prepare_source(s(&corpus["files"], "source-original.txt")).unwrap();
    // The frozen execution corpus intentionally contains no textual headings.
    // Use the real explicit-user boundary contract instead of inventing parser
    // output or changing the raw source whose hash the receipt already pins.
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
    let source = store.save_source(json!(recognized)).unwrap();
    let episode = s(&source["episodes"][0], "id").to_owned();
    let mut input = json!({"source":source,"selection":{"episodeIds":[episode]},"existingUnits":[],"assets":[]});
    mark_input(&mut input).unwrap();
    let guard = crate::series_lite_guard::from_input(&input);
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":input,"guard":guard});
    let run = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &snapshot,
            "partially_blocked",
        )
        .unwrap();
    let unit=store.save_unit(json!({"id":"synthetic-stable-unit","episodeId":episode,"title":"保留的整集单元","prompt":"原文完整提示词",
        "sourceRefs":[source["rows"][0]["id"]],"assetRefs":[],"shots":[],"duration":4,"durationSeconds":4,
        "initialState":"原始状态","exitState":"结束状态","seriesLite":{"nativeUnitId":"U1"}})).unwrap();
    let workspace = temp.path().join("mechanical");
    fs::create_dir(&workspace).unwrap();
    for (path, text) in corpus["files"].as_object().unwrap() {
        let file = workspace.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text.as_str().unwrap().as_bytes()).unwrap();
    }
    let mechanical = crate::series_lite_execution::run(
        &workspace,
        "execution-request.json",
        "out",
        &VerifiedPack::embedded().unwrap(),
        None,
    )
    .unwrap();
    let native = s(&corpus["files"], "bundle.json");
    let mut c = store.conn().unwrap();
    let tx = c.transaction().unwrap();
    artifacts::install(&tx).unwrap();
    let inputs = Inputs::from([("source".into(), s(&source, "sha256").into())]);
    let raw = Artifact {
        id: "synthetic-original-raw".into(),
        identity: "synthetic-episode/native".into(),
        kind: Kind::Raw,
        contract_version: "synthetic/v1".into(),
        source_sha256: s(&source, "sha256").into(),
        inputs: inputs.clone(),
        producer: "synthetic-only".into(),
        author_attempt: "synthetic-author".into(),
        parents: vec![],
        payload: json!({"text":native}),
    };
    artifacts::put(&tx, &raw).unwrap();
    let mut candidate = raw.clone();
    candidate.id = "synthetic-candidate".into();
    candidate.kind = Kind::Candidate;
    candidate.parents = vec![raw.id.clone()];
    artifacts::put(&tx, &candidate).unwrap();
    artifacts::validate(
        &tx,
        "synthetic-structure",
        &candidate.id,
        &raw.id,
        &StructureOnly,
    )
    .unwrap();
    let adopted = artifacts::adopt(
        &tx,
        Adopt {
            id: "synthetic-adoption",
            candidate_id: &candidate.id,
            identity: &candidate.identity,
            expected_previous: 0,
            source_sha256: &candidate.source_sha256,
            contract_version: &candidate.contract_version,
            current_inputs: &inputs,
            required: &[Capability::Structure],
            validation_ids: &["synthetic-structure".into()],
        },
    )
    .unwrap();
    let record = json!({"version":DELIVERY,"run":run,"episodeId":episode,"episodeNumber":1,"sourceSha256":source["sha256"],
        "nativeSha256":sha256(native.as_bytes()),"workspaceRoot":workspace,"receiptPath":mechanical.receipt_path,"receiptSha256":mechanical.receipt_file_sha256,
        "adoption":adopted,"candidateArtifactId":candidate.id,"finalDeliveryReady":false,"units":[unit],"guard":guard});
    tx.execute(
        "INSERT INTO metadata(key,value) VALUES(?1,?2)",
        params![format!("lite.delivery:{episode}"), record.to_string()],
    )
    .unwrap();
    tx.commit().unwrap();
    SavedFixture {
        _temp: temp,
        store,
        input,
        snapshot,
        run,
        unit,
        record,
    }
}

#[test]
fn saved_and_parent_guard_accept_program_media_preparation_but_keep_manual_prompt_versions_held() {
    let f = saved_fixture();
    validate_saved(&f.store, &f.record, &f.input).unwrap();
    let mut media = f.unit.clone();
    media["expectedVersion"] = f.unit["version"].clone();
    media["mediaPlan"] = json!({"status":"ready","items":["synthetic-image"]});
    media["status"] = json!("ready");
    let media = f.store.save_unit(media).unwrap();
    assert_ne!(media["version"], f.unit["version"]);
    validate_saved(&f.store, &f.record, &f.input).unwrap();
    refresh_guard(&f.store, &f.run, &f.snapshot).unwrap();
    let prepared = f.store.job(&f.run).unwrap()["prepared"].clone();
    assert_eq!(
        prepared["publicationGuard"]["units"][s(&f.unit, "id")],
        media["version"]
    );
    let mut manual = media;
    manual["expectedVersion"] = manual["version"].clone();
    manual["prompt"] = json!("用户的新人工草稿，不得被旧采用稿覆盖");
    let manual = f.store.save_unit(manual).unwrap();
    assert!(validate_saved(&f.store, &f.record, &f.input)
        .unwrap_err()
        .contains("人工稿"));
    assert!(refresh_guard(&f.store, &f.run, &f.snapshot)
        .unwrap_err()
        .contains("TEXT_CHANGED"));
    assert_eq!(f.store.job(&f.run).unwrap()["prepared"], prepared);
    assert_eq!(
        f.store.get("units", s(&manual, "id")).unwrap()["prompt"],
        manual["prompt"]
    );
}

#[test]
fn actual_previous_boundary_uses_adopted_native_and_does_not_ignore_a_new_manual_draft() {
    let f = saved_fixture();
    let previous = s(&f.record, "episodeId");
    let input = json!({"source":{"sha256":f.input["source"]["sha256"],"episodes":[{"id":previous},{"id":"next"}]}});
    let supplied = boundary(&f.store, &input, 1).unwrap();
    assert_eq!(supplied["kind"], "supplied");
    assert_eq!(
        sha256(s(&supplied, "previousBundleUtf8").as_bytes()),
        f.record["nativeSha256"]
    );
    let mut manual = f.unit.clone();
    manual["expectedVersion"] = manual["version"].clone();
    manual["prompt"] = json!("人工已改，旧尾稿不能静默作为当前入口");
    f.store.save_unit(manual).unwrap();
    assert!(
        boundary(&f.store, &input, 1).is_err(),
        "changed manual predecessor must be reconciled before consuming its old native tail"
    );
}

struct NoDispatch;
#[async_trait::async_trait]
impl TextExecutor for NoDispatch {
    async fn execute(
        &self,
        _request: super::super::TextRequest,
    ) -> std::result::Result<Value, String> {
        panic!("invalid local setup and already received results must not submit a model request");
    }
}
#[tokio::test]
async fn invalid_episode_setup_is_local_and_does_not_discard_later_saved_results() {
    let f = saved_fixture();
    let episode = s(&f.record, "episodeId").to_owned();
    let mut input = f.input.clone();
    let original = input["source"]["episodes"][0].clone();
    let mut unresolved = original.clone();
    unresolved["id"] = json!("unresolved-local-episode");
    unresolved["sourceNumber"] = json!("尚未解析");
    input["source"]["episodes"] = json!([unresolved, original]);
    input["selection"]["episodeIds"] = json!(["unresolved-local-episode", episode]);
    let count = || {
        f.store
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    let before = count();
    let output = run(
        &f.store,
        &f.run,
        &input,
        &NoDispatch,
        "gpt-6.1-sol",
        "medium",
    )
    .await
    .unwrap();
    assert_eq!(output["selectedEpisodes"], 2);
    assert_eq!(output["episodes"].as_array().unwrap().len(), 2);
    assert_eq!(
        output["episodes"][0]["episodeId"],
        "unresolved-local-episode"
    );
    assert_eq!(output["episodes"][0]["status"], "needs_attention");
    assert_eq!(output["episodes"][1]["episodeId"], episode);
    assert_eq!(output["episodes"][1]["reused"], true);
    assert_eq!(
        count(),
        before,
        "no replacement workflow or duplicate model request"
    );
    assert_eq!(
        f.store.get("units", s(&f.unit, "id")).unwrap()["prompt"],
        f.unit["prompt"]
    );
}
