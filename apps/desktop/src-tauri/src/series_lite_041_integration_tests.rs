//! Synthetic 0.4.1 cross-module regression. No provider, project DB, browser,
//! model, image/video generation, or semantic-content approval is involved.
use crate::series_lite_asset_uses::RegistryValidator;
use crate::series_lite_contract::VerifiedPack;
use crate::series_lite_edit::{compose_derived_bytes, sha256};
use crate::series_lite_execution::{self as execution, ExecutionResult};
use crate::series_lite_job_packet::{self as packets, PreparationOptions};
use crate::series_lite_transport::{self as transport, Contributor, Role, RoleContext};
use crate::series_lite_workspace::NativeWorkspaceScopeVerifier;
use serde_json::{json, Value};
use std::{fs, path::Path};

fn corpus() -> Value {
    serde_json::from_str(include_str!("series_lite_execution_golden.json")).unwrap()
}
fn write_json(root: &Path, relative: &str, value: &Value) {
    let target = root.join(relative);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, serde_json::to_vec(value).unwrap()).unwrap();
}
fn read_json(root: &Path, relative: &str) -> Value {
    serde_json::from_slice(&fs::read(root.join(relative)).unwrap()).unwrap()
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, raw) in corpus()["files"].as_object().unwrap() {
        let target = dir.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, raw.as_str().unwrap().as_bytes()).unwrap();
    }
    dir
}
fn context(role: Role) -> RoleContext {
    let mut contributors = vec![];
    if role != Role::Author {
        contributors.push(Contributor {
            role: Role::Author,
            context_id: "synthetic-041-author-context".into(),
        });
    }
    if role == Role::Reviewer {
        contributors.push(Contributor {
            role: Role::Finisher,
            context_id: "synthetic-041-finisher-context".into(),
        });
    }
    RoleContext {
        episode: "1".into(),
        role,
        iteration_id: format!("synthetic-041-{}-slot", role.as_str()),
        context_id: format!("synthetic-041-{}-context", role.as_str()),
        prior_contributors: contributors,
    }
}
fn roundtrip_transport(root: &Path, packet_path: &str, role: Role, pack: &VerifiedPack) {
    let compiled = packets::verify(root, packet_path, pack, &NativeWorkspaceScopeVerifier).unwrap();
    let context = context(role);
    let carrier = transport::freeze_verified(&compiled, &context, pack).unwrap();
    let name = format!("transport-{}.json", role.as_str());
    write_json(root, &name, &carrier.input);
    let restored =
        transport::restore(&read_json(root, &name), &carrier.identity, &context).unwrap();
    assert_eq!(restored.bytes, compiled.bytes);
    assert_eq!(restored.identity, carrier.identity);
    assert!(
        transport::freeze_verified(&compiled, &context, &VerifiedPack::embedded().unwrap())
            .is_err()
    );
    let mut wrong_pin = carrier.identity.clone();
    wrong_pin.packet_file_sha256 = "a".repeat(64);
    assert!(transport::restore(&carrier.input, &wrong_pin, &context).is_err());
}

/// The new base is a new file. Historical fixture bundle/packet/role records are
/// never retroactively relabelled or used as evidence of a new model response.
fn prepare_complete_candidate(root: &Path, pack: &VerifiedPack) {
    let original = read_json(root, "bundle.json");
    let mut complete = original.clone();
    complete["generationBoundaryVersion"] = json!("video-prompt-v1");
    complete["episodes"][0]["settings"]["fantasyMode"] = json!("off");
    let events = vec![
        json!({"id":"EV1","sourceRefs":["E01:R1","E01:R2"],"sceneKey":"1-1",
            "boundaryBefore":{"kind":"episode_start","reason":"原源首场开始"},
            "emotionContract":{"subject":"值守者","contextTone":"戒备","audienceEffect":"读到克制提醒",
                "change":"等待转为提醒","landing":"继续保留面罩与持物","retain":["门未开","原话与右手钥匙保持"]}}),
        json!({"id":"EV2","sourceRefs":["E01:R3","E01:R4"],"sceneKey":"1-2",
            "boundaryBefore":{"kind":"new_event","reason":"原源转入空走廊与系统声"},
            "emotionContract":{"subject":"空走廊","contextTone":"等待","audienceEffect":"收到门锁确认",
                "change":"等待转为确认","landing":"无人走廊保持","retain":["系统音不具象化","原话原声保持"]}}),
    ];
    complete["episodes"][0]["plan"] = json!({"schemaVersion":"performance-core-v4-plan",
        "unitPackingPolicy":"event-first-v1","emotionArc":"从闭门等待到系统确认",
        "events":events,"entryFacts":[],"fantasyCandidates":[]});
    for index in 0..2 {
        complete["episodes"][0]["draft"]["units"][index]["eventId"] = events[index]["id"].clone();
        complete["episodes"][0]["draft"]["units"][index]["boundaryBefore"] =
            events[index]["boundaryBefore"].clone();
    }
    let mut base = complete.clone();
    for field in ["plan", "selection", "draft"] {
        base["episodes"][0][field] = json!({});
    }
    write_json(root, "base-041.json", &base);
    let prepared = packets::prepare(
        root,
        "base-041.json",
        "registry",
        "author",
        "author-041",
        &PreparationOptions::default(),
        pack,
        &NativeWorkspaceScopeVerifier,
    )
    .unwrap();
    let p = read_json(root, &prepared.packet);
    assert!(p["review"].is_null());
    let kinds: Vec<_> = p["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["source", "scenes", "visual-baseline", "direction", "assets"]
    );
    roundtrip_transport(root, &prepared.packet, Role::Author, pack);
    let response = json!({"plan":complete["episodes"][0]["plan"],"selection":complete["episodes"][0]["selection"],"draft":complete["episodes"][0]["draft"]});
    let raw = fs::read(root.join("base-041.json")).unwrap();
    let result =
        compose_derived_bytes(&raw, &sha256(&raw), &serde_json::to_vec(&response).unwrap())
            .unwrap();
    assert!(result.derived_bookkeeping);
    let candidate: Value = serde_json::from_slice(&result.edit.bytes).unwrap();
    assert_eq!(
        candidate["episodes"][0]["source"],
        original["episodes"][0]["source"]
    );
    assert_eq!(candidate["assets"], original["assets"]);
    assert_eq!(
        candidate["episodes"][0]["plan"]["provisionalUnits"][0]["secondsRange"],
        json!([6, 6])
    );
    assert_eq!(
        candidate["episodes"][0]["plan"]["provisionalUnits"][1]["secondsRange"],
        json!([4, 4])
    );
    assert_eq!(
        candidate["episodes"][0]["draft"]["units"][0]["shots"],
        original["episodes"][0]["draft"]["units"][0]["shots"]
    );
    fs::write(root.join("candidate-041.json"), result.edit.bytes).unwrap();
    let prepared = packets::prepare(
        root,
        "candidate-041.json",
        "registry",
        "finisher",
        "finisher-041",
        &PreparationOptions::default(),
        pack,
        &NativeWorkspaceScopeVerifier,
    )
    .unwrap();
    assert!(read_json(root, &prepared.packet)["review"].is_null());
    roundtrip_transport(root, &prepared.packet, Role::Finisher, pack);
    write_json(
        root,
        "execution-041.json",
        &json!({"version":"series-episode-execution-v1","episode":1,
        "packet":prepared.packet,"bundle":"candidate-041.json","sourceLock":"source-lock.json","registry":"registry"}),
    );
}
fn run_complete(root: &Path, pack: &VerifiedPack) -> ExecutionResult {
    prepare_complete_candidate(root, pack);
    let result = execution::run(root, "execution-041.json", "delivery-041", pack, None).unwrap();
    assert_eq!(result.steps_run, ["sync", "check", "export", "verify"]);
    assert_eq!(result.content_review, "NOT_PROVEN");
    assert!(!result.reused);
    result
}
#[test]
fn lite041_one_call_author_transport_derived_compose_finisher_delivery_and_reopen() {
    let dir = fixture();
    let root = dir.path();
    let pack = VerifiedPack::current_embedded().unwrap();
    let original_bundle = fs::read(root.join("bundle.json")).unwrap();
    let original_packet = fs::read(root.join("packet.json")).unwrap();
    let original_lock = fs::read(root.join("source-lock.json")).unwrap();
    let result = run_complete(root, &pack);
    let receipt = execution::verify_receipt(
        root,
        &result.receipt_path,
        &result.receipt_file_sha256,
        Some(1),
    )
    .unwrap();
    let preview = fs::read(root.join(&result.preview_path)).unwrap();
    let catalog = fs::read(root.join(format!("{}.assets.json", result.preview_path))).unwrap();
    assert!(String::from_utf8(preview.clone())
        .unwrap()
        .contains("门还没开，"));
    assert!(String::from_utf8(preview.clone())
        .unwrap()
        .contains("先别摘下面罩。"));
    assert_eq!(
        read_json(root, "delivery-041/verify.json")["ok"],
        json!(true)
    );
    drop(pack);
    let reopened = execution::run(
        root,
        "execution-041.json",
        "delivery-041",
        &VerifiedPack::current_embedded().unwrap(),
        Some(&result.receipt_file_sha256),
    )
    .unwrap();
    assert!(reopened.reused && reopened.steps_run.is_empty());
    assert_eq!(reopened.receipt_file_sha256, receipt.file_sha256);
    assert_eq!(fs::read(root.join(&result.preview_path)).unwrap(), preview);
    assert_eq!(
        fs::read(root.join(format!("{}.assets.json", result.preview_path))).unwrap(),
        catalog
    );
    assert_eq!(fs::read(root.join("bundle.json")).unwrap(), original_bundle);
    assert_eq!(fs::read(root.join("packet.json")).unwrap(), original_packet);
    assert_eq!(
        fs::read(root.join("source-lock.json")).unwrap(),
        original_lock
    );
    // Reopen without its independent ledger pin is reconciliation, not permission
    // to replace the existing receipt or perform another producer submission.
    assert!(execution::run(
        root,
        "execution-041.json",
        "delivery-041",
        &VerifiedPack::current_embedded().unwrap(),
        None
    )
    .is_err());
}

struct NativeRegistry;
impl RegistryValidator for NativeRegistry {
    fn validate(&mut self, registry: &Value, require_locked: bool) -> Result<(), String> {
        crate::series_lite_registry::validate_registry(registry, require_locked)
    }
}
fn publish_registry(root: &Path, registry: &Value) {
    crate::series_lite_registry::validate_registry(registry, false).unwrap();
    let relative = format!(
        "registry/v{:06}.json",
        registry["revision"].as_u64().unwrap()
    );
    let raw = serde_json::to_vec(registry).unwrap();
    assert!(!root.join(&relative).exists());
    fs::write(root.join(relative), &raw).unwrap();
    write_json(
        root,
        "registry/current.json",
        &json!({"version":"series-assets-head-v1","id":registry["id"],"revision":registry["revision"],"sha256":sha256(&raw)}),
    );
}
fn current_registry(root: &Path) -> Value {
    let head = read_json(root, "registry/current.json");
    read_json(
        root,
        &format!("registry/v{:06}.json", head["revision"].as_u64().unwrap()),
    )
}
#[test]
fn lite041_image_only_design_and_unrelated_changes_preserve_text_receipt_actual_consumer_change_holds(
) {
    let dir = fixture();
    let root = dir.path();
    let pack = VerifiedPack::current_embedded().unwrap();
    let mut registry = current_registry(root);
    let mut unrelated = registry["assets"][3].clone();
    unrelated["id"] = json!("A9000");
    unrelated["name"] = json!("备用盒");
    unrelated["key"] = json!("备用盒PR");
    unrelated["importName"] = json!("备用盒");
    unrelated["placeholderOrdinal"] = json!(4);
    registry["assets"].as_array_mut().unwrap().push(unrelated);
    registry["revision"] = json!(2);
    publish_registry(root, &registry);
    let result = run_complete(root, &pack);
    let signature = execution::inspect_inputs(root, "execution-041.json", &pack).unwrap();
    let original_assets = registry["assets"].clone();
    let change = json!({"version":"series-image-design-v1","baseRevision":registry["revision"],"requestId":"synthetic-041-art-only",
        "reason":"补展示布局，不变分镜必要设计","entries":[{"assetId":"A0001","assetRevision":1,
            "design":{"state":"TEXT_ADOPTED","text":"完整制图设计：保持已采用面罩与衣装","full":{"body":"仅增补展示布局与照明；身份、面罩遮口鼻与右手钥匙动作条件保持。"}},
            "compatibilityBasis":"只补制图展示；保留身份、面罩遮口鼻与动作条件"}]});
    let recorded = crate::series_lite_image_design::record_image_design(
        &registry,
        &change,
        &mut NativeRegistry,
    )
    .unwrap();
    assert!(recorded.changed && !recorded.asset_revisions_changed);
    assert_eq!(recorded.registry["assets"], original_assets);
    publish_registry(root, &recorded.registry);
    let after_art = execution::inspect_inputs(root, "execution-041.json", &pack).unwrap();
    assert_eq!(
        after_art.execution_fingerprint,
        signature.execution_fingerprint
    );
    assert_eq!(after_art.signature_utf8, signature.signature_utf8);
    execution::verify_receipt(
        root,
        &result.receipt_path,
        &result.receipt_file_sha256,
        Some(1),
    )
    .unwrap();
    assert!(execution::run(
        root,
        "execution-041.json",
        "delivery-041",
        &pack,
        Some(&result.receipt_file_sha256)
    )
    .unwrap()
    .steps_run
    .is_empty());
    let mut unrelated_change = recorded.registry.clone();
    let last = unrelated_change["assets"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap();
    last["revision"] = json!(2);
    last["design"]["text"] = json!("备用盒局部设计已改，本集从未消费。");
    unrelated_change["revision"] = json!(4);
    publish_registry(root, &unrelated_change);
    assert_eq!(
        execution::inspect_inputs(root, "execution-041.json", &pack)
            .unwrap()
            .signature_utf8,
        signature.signature_utf8
    );
    execution::verify_receipt(
        root,
        &result.receipt_path,
        &result.receipt_file_sha256,
        Some(1),
    )
    .unwrap();
    let receipt_before = fs::read(root.join(&result.receipt_path)).unwrap();
    let preview_before = fs::read(root.join(&result.preview_path)).unwrap();
    let mut actual_change = unrelated_change;
    // An actual narrative design (used SC A0002), unlike art-only supplementation,
    // changes the true consumed input. Do not update frozen candidate/packet pins.
    actual_change["assets"][1]["revision"] = json!(2);
    actual_change["assets"][1]["design"]["text"] =
        json!("门厅的真实出入口及可操作空间已修订；需复核本集使用。");
    actual_change["revision"] = json!(5);
    publish_registry(root, &actual_change);
    assert_ne!(
        execution::inspect_inputs(root, "execution-041.json", &pack)
            .unwrap()
            .execution_fingerprint,
        signature.execution_fingerprint
    );
    assert!(execution::verify_receipt(
        root,
        &result.receipt_path,
        &result.receipt_file_sha256,
        Some(1)
    )
    .is_err());
    assert!(execution::run(
        root,
        "execution-041.json",
        "delivery-041",
        &pack,
        Some(&result.receipt_file_sha256)
    )
    .is_err());
    assert_eq!(
        fs::read(root.join(&result.receipt_path)).unwrap(),
        receipt_before
    );
    assert_eq!(
        fs::read(root.join(&result.preview_path)).unwrap(),
        preview_before
    );
}
