use super::*;
use serde_json::{json, Value};

fn golden() -> Value {
    serde_json::from_str(include_str!("series_lite_job_packet_golden.json")).unwrap()
}
fn fixture(case: &Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, value) in case["files"].as_object().unwrap() {
        let target = dir.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, value.as_str().unwrap().as_bytes()).unwrap();
    }
    dir
}
fn base_fixture() -> tempfile::TempDir {
    fixture(&golden()["cases"][0])
}
fn compile(root: &Path) -> CompiledPacket {
    build(
        root,
        "request.json",
        "packets/one.json",
        DEFAULT_MAX_BYTES,
        &VerifiedPack::embedded().unwrap(),
        &RejectUnverifiedWorkspace,
    )
    .unwrap()
}
fn mutate_request(root: &Path, edit: impl FnOnce(&mut Value)) {
    let mut v: Value =
        serde_json::from_slice(&fs::read(root.join("request.json")).unwrap()).unwrap();
    edit(&mut v);
    fs::write(root.join("request.json"), serde_json::to_vec(&v).unwrap()).unwrap();
}
fn rewrite_packet(root: &Path, edit: impl FnOnce(&mut BTreeMap<String, Json>)) -> String {
    let raw = fs::read(root.join("packets/one.json")).unwrap();
    let mut p = parse(&raw).unwrap().object().unwrap().clone();
    edit(&mut p);
    p.remove("packetSha256");
    let hash = sha256(&canonical(&Json::Object(p.clone())));
    p.insert("packetSha256".into(), text(hash));
    let raw = canonical(&Json::Object(p));
    fs::write(root.join("packets/one.json"), &raw).unwrap();
    sha256(&raw)
}

#[test]
fn series_lite_job_packet_matches_44_frozen_python_oracles() {
    let oracle = golden();
    assert_eq!(
        oracle["sourceHashes"]["job_packet.py"],
        sha256(COMPILER_RAW)
    );
    assert_eq!(
        oracle["sourceHashes"]["context_packet.py"],
        crate::series_lite_edit::SOURCE_CONTEXT_PACKET_SHA256
    );
    assert_eq!(
        oracle["sourceHashes"]["visual_baseline.py"],
        crate::series_lite_edit::SOURCE_VISUAL_BASELINE_SHA256
    );
    let pack = VerifiedPack::embedded().unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let dir = fixture(case);
        let result = build(
            dir.path(),
            "request.json",
            "packets/one.json",
            case["maxBytes"].as_u64().unwrap() as usize,
            &pack,
            &RejectUnverifiedWorkspace,
        );
        assert_eq!(
            result.is_ok(),
            case["ok"].as_bool().unwrap(),
            "{name}: {result:?}"
        );
        if let Ok(result) = result {
            assert_eq!(
                result.bytes,
                case["encoded"].as_str().unwrap().as_bytes(),
                "complete packet bytes: {name}"
            );
            assert_eq!(result.sha256, case["sha256"].as_str().unwrap(), "{name}");
            assert_eq!(
                result.input_fingerprint,
                case["inputFingerprint"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(
                verify(
                    dir.path(),
                    "packets/one.json",
                    &pack,
                    &RejectUnverifiedWorkspace
                )
                .unwrap()
                .bytes,
                result.bytes
            );
            assert_eq!(
                load_frozen(
                    dir.path(),
                    "packets/one.json",
                    &result.sha256,
                    &RejectUnverifiedWorkspace
                )
                .unwrap()
                .bytes,
                result.bytes
            );
        } else {
            assert!(
                !dir.path().join("packets/one.json").exists(),
                "failed compile wrote: {name}"
            );
            assert!(
                !result.unwrap_err().contains("secret"),
                "source leaked in diagnostics"
            );
        }
    }
}

#[test]
fn series_lite_job_packet_rules_and_full_bodies_are_lossless() {
    let dir = base_fixture();
    mutate_request(dir.path(), |r| {
        r["methods"] = json!(["fantasy-expression-methods", "acting-blocking-methods"]);
        r["contracts"] = json!([
            "system-presentation",
            "asset-registry-contract",
            "shot-handoffs"
        ]);
        r["inputs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"entry","path":"source.txt"}));
    });
    let packet = compile(dir.path());
    let p = parse(&packet.bytes).unwrap();
    let bindings = p.get("bindings").unwrap().array().unwrap();
    let source = bindings[0].get("blob").unwrap();
    assert_eq!(source, bindings.last().unwrap().get("blob").unwrap());
    let body = p
        .get("blobs")
        .unwrap()
        .get(source.string().unwrap())
        .unwrap()
        .get("value")
        .unwrap()
        .string()
        .unwrap();
    assert_eq!(
        body.as_bytes(),
        fs::read(dir.path().join("source.txt")).unwrap()
    );
    let paths: Vec<_> = p
        .get("rules")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|v| v.get("path").unwrap().string().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "references/creative-core.md",
            "references/schema.md",
            "references/author.md",
            "references/fantasy.md",
            "references/system.md"
        ]
    );
    let rules = VerifiedPack::embedded()
        .unwrap()
        .selected_role_rules(
            "author",
            &["fantasy-expression-methods"],
            &["system-presentation"],
        )
        .unwrap();
    for record in p.get("rules").unwrap().array().unwrap() {
        let path = record.get("path").unwrap().string().unwrap();
        let body = p
            .get("blobs")
            .unwrap()
            .get(record.get("blob").unwrap().string().unwrap())
            .unwrap()
            .get("value")
            .unwrap()
            .string()
            .unwrap();
        assert_eq!(body.as_bytes(), rules[path]);
        assert_eq!(
            record.get("sha256").unwrap().string().unwrap(),
            sha256(&rules[path])
        );
    }
    assert!(VerifiedPack::embedded()
        .unwrap()
        .require_new_production()
        .is_err());
}

#[test]
fn series_lite_job_packet_frozen_input_and_review_drift_refuse_reuse() {
    let dir = base_fixture();
    let p = compile(dir.path());
    let pack = VerifiedPack::embedded().unwrap();
    for path in [
        "source.txt",
        "assets.txt",
        "baseline.json",
        "review.md",
        "request.json",
    ] {
        let target = dir.path().join(path);
        let raw = fs::read(&target).unwrap();
        let mut changed = raw.clone();
        changed.extend_from_slice(b"\n ");
        fs::write(&target, changed).unwrap();
        assert!(
            verify(
                dir.path(),
                "packets/one.json",
                &pack,
                &RejectUnverifiedWorkspace
            )
            .is_err(),
            "{path}"
        );
        assert!(
            load_frozen(
                dir.path(),
                "packets/one.json",
                &p.sha256,
                &RejectUnverifiedWorkspace
            )
            .is_err(),
            "{path}"
        );
        fs::write(target, raw).unwrap();
    }
    fs::write(dir.path().join("unrelated.txt"), b"unrelated changed").unwrap();
    assert_eq!(
        verify(
            dir.path(),
            "packets/one.json",
            &pack,
            &RejectUnverifiedWorkspace
        )
        .unwrap()
        .bytes,
        p.bytes
    );
}

#[test]
fn series_lite_job_packet_full_origin_hash_binds_unselected_json() {
    let dir = base_fixture();
    fs::write(
        dir.path().join("all.json"),
        b"{\"selected\":1.250000001,\"other\":1}",
    )
    .unwrap();
    mutate_request(dir.path(), |r| {
        r["inputs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"plan","path":"all.json","pointer":"/selected"}))
    });
    let p = compile(dir.path());
    fs::write(
        dir.path().join("all.json"),
        b"{\"selected\":1.250000001,\"other\":2}",
    )
    .unwrap();
    assert!(load_frozen(
        dir.path(),
        "packets/one.json",
        &p.sha256,
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert!(verify(
        dir.path(),
        "packets/one.json",
        &VerifiedPack::embedded().unwrap(),
        &RejectUnverifiedWorkspace
    )
    .is_err());
}

#[test]
fn series_lite_job_packet_historical_skill_is_not_live_dispatch() {
    let dir = base_fixture();
    let p = compile(dir.path());
    // Simulate a valid already-adopted packet from an older Skill. It is not
    // silently resealed by production: the supplied receipt pins these bytes.
    let old_hash = rewrite_packet(dir.path(), |map| {
        map.insert(
            "skill".into(),
            object([
                ("version", text("older frozen release")),
                ("sha256", text("a".repeat(64))),
            ]),
        );
        let inputs = Json::Object(
            INPUT_KEYS
                .iter()
                .map(|k| ((*k).to_owned(), map[*k].clone()))
                .collect(),
        );
        map.insert("inputFingerprint".into(), text(sha256(&canonical(&inputs))));
    });
    assert!(load_frozen(
        dir.path(),
        "packets/one.json",
        &p.sha256,
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert!(load_frozen(
        dir.path(),
        "packets/one.json",
        &old_hash,
        &RejectUnverifiedWorkspace
    )
    .is_ok());
    assert!(verify(
        dir.path(),
        "packets/one.json",
        &VerifiedPack::embedded().unwrap(),
        &RejectUnverifiedWorkspace
    )
    .is_err());
}

#[test]
fn series_lite_job_packet_resealed_nested_tamper_is_rejected() {
    let edits: Vec<Box<dyn Fn(&mut BTreeMap<String, Json>)>> = vec![
        Box::new(|p| {
            p.insert("inputFingerprint".into(), text("0".repeat(64)));
        }),
        Box::new(|p| {
            p.insert("maxBytes".into(), Json::Bool(true));
        }),
        Box::new(|p| {
            p.insert(
                "scope".into(),
                object([("scope", Json::Array(vec![number(1)]))]),
            );
        }),
        Box::new(|p| {
            let mut blobs = p["blobs"].object().unwrap().clone();
            let key = p["rules"].array().unwrap()[0]
                .get("blob")
                .unwrap()
                .string()
                .unwrap();
            blobs.insert(
                key.into(),
                object([("format", text("text")), ("value", text("tampered"))]),
            );
            p.insert("blobs".into(), Json::Object(blobs));
        }),
        Box::new(|p| {
            let mut blobs = p["blobs"].object().unwrap().clone();
            add_blob(&mut blobs, "text", text("unreferenced"));
            p.insert("blobs".into(), Json::Object(blobs));
        }),
        Box::new(|p| {
            let mut rows = p["rules"].array().unwrap().to_vec();
            let mut row = rows[0].object().unwrap().clone();
            row.insert("path".into(), text("../outside.md"));
            rows[0] = Json::Object(row);
            p.insert("rules".into(), Json::Array(rows));
        }),
        Box::new(|p| {
            let mut rows = p["bindings"].array().unwrap().to_vec();
            let mut row = rows[0].object().unwrap().clone();
            row.insert("originBytes".into(), Json::Bool(true));
            rows[0] = Json::Object(row);
            p.insert("bindings".into(), Json::Array(rows));
        }),
    ];
    for (index, edit) in edits.into_iter().enumerate() {
        let dir = base_fixture();
        compile(dir.path());
        let hash = rewrite_packet(dir.path(), edit);
        assert!(
            load_frozen(
                dir.path(),
                "packets/one.json",
                &hash,
                &RejectUnverifiedWorkspace
            )
            .is_err(),
            "mutation {index}"
        );
    }
}

#[test]
fn series_lite_job_packet_budget_and_existing_output_never_overwrite() {
    let dir = base_fixture();
    let pack = VerifiedPack::embedded().unwrap();
    assert!(build(
        dir.path(),
        "request.json",
        "absent/too-small.json",
        100,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .unwrap_err()
    .contains("no input truncated"));
    assert!(!dir.path().join("absent").exists());
    let p = compile(dir.path());
    assert!(build(
        dir.path(),
        "request.json",
        "packets/one.json",
        DEFAULT_MAX_BYTES,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert_eq!(
        fs::read(dir.path().join("packets/one.json")).unwrap(),
        p.bytes
    );
    let source = fs::read(dir.path().join("source.txt")).unwrap();
    assert!(build(
        dir.path(),
        "request.json",
        "source.txt",
        DEFAULT_MAX_BYTES,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert_eq!(fs::read(dir.path().join("source.txt")).unwrap(), source);
}

#[test]
fn series_lite_job_packet_paths_and_non_utf8_fail_closed() {
    let dir = base_fixture();
    let pack = VerifiedPack::embedded().unwrap();
    for path in [
        "",
        ".",
        "../outside",
        "..\\outside",
        "C:relative",
        "source.txt:stream",
        "/absolute",
        "\\rooted",
    ] {
        assert!(
            build(
                dir.path(),
                "request.json",
                path,
                DEFAULT_MAX_BYTES,
                &pack,
                &RejectUnverifiedWorkspace
            )
            .is_err(),
            "{path}"
        );
    }
    fs::write(dir.path().join("source.txt"), [0xff]).unwrap();
    assert!(assemble(
        dir.path(),
        "request.json",
        DEFAULT_MAX_BYTES,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .is_err());
}

#[test]
fn series_lite_job_packet_workspace_is_explicitly_fail_closed_and_bound() {
    struct TestOnlyScope;
    impl WorkspaceScopeVerifier for TestOnlyScope {
        fn verify_workspace(&self, root: &Path) -> Result<Vec<u8>> {
            read_file(root, "state.json")
        }
    }
    let dir = base_fixture();
    let state = json!({"version":"series-workspace-v1","sourceHash":"a".repeat(64),"sourceIndexHash":"b".repeat(64),"scope":[1,2],"episodes":{"2":{"owner":"one"}}});
    fs::write(
        dir.path().join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let pack = VerifiedPack::embedded().unwrap();
    assert!(assemble(
        dir.path(),
        "request.json",
        DEFAULT_MAX_BYTES,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .unwrap_err()
    .contains("WORKSPACE_UNIMPLEMENTED"));
    let p = build(
        dir.path(),
        "request.json",
        "packets/one.json",
        DEFAULT_MAX_BYTES,
        &pack,
        &TestOnlyScope,
    )
    .unwrap();
    let mut changed = state.clone();
    changed["episodes"]["2"]["owner"] = json!("two");
    fs::write(
        dir.path().join("state.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert_eq!(
        verify(dir.path(), "packets/one.json", &pack, &TestOnlyScope)
            .unwrap()
            .bytes,
        p.bytes
    );
    changed["scope"] = json!([2]);
    fs::write(
        dir.path().join("state.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert!(verify(dir.path(), "packets/one.json", &pack, &TestOnlyScope).is_err());
    fs::remove_file(dir.path().join("state.json")).unwrap();
    assert!(load_frozen(
        dir.path(),
        "packets/one.json",
        &p.sha256,
        &RejectUnverifiedWorkspace
    )
    .is_err());
}

#[test]
fn series_lite_job_packet_noncanonical_bytes_cannot_resume() {
    let dir = base_fixture();
    let p = compile(dir.path());
    let mut raw = p.bytes.clone();
    raw.push(b'\n');
    fs::write(dir.path().join("packets/one.json"), &raw).unwrap();
    assert!(load_frozen(
        dir.path(),
        "packets/one.json",
        &sha256(&raw),
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert!(verify(
        dir.path(),
        "packets/one.json",
        &VerifiedPack::embedded().unwrap(),
        &RejectUnverifiedWorkspace
    )
    .is_err());
}

#[test]
fn series_lite_job_packet_symlink_escape_and_broken_links_are_rejected() {
    let dir = base_fixture();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("source.txt"), b"outside secret").unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(outside.path(), dir.path().join("escape")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
    let pack = VerifiedPack::embedded().unwrap();
    assert!(build(
        dir.path(),
        "request.json",
        "escape/out.json",
        DEFAULT_MAX_BYTES,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert!(!outside.path().join("out.json").exists());
    mutate_request(dir.path(), |r| {
        r["inputs"][0]["path"] = json!("escape/source.txt")
    });
    assert!(assemble(
        dir.path(),
        "request.json",
        DEFAULT_MAX_BYTES,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .is_err());
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(
        dir.path().join("missing.json"),
        dir.path().join("broken.json"),
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        dir.path().join("missing.json"),
        dir.path().join("broken.json"),
    )
    .unwrap();
    assert!(build(
        dir.path(),
        "request.json",
        "broken.json",
        DEFAULT_MAX_BYTES,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert!(!dir.path().join("missing.json").exists());
}
