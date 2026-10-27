use super::*;
fn golden() -> Value {
    serde_json::from_str(include_str!("series_lite_workspace_golden.json")).unwrap()
}
fn materialize(root: &Path, files: &Value) {
    for (name, body) in files.as_object().unwrap() {
        let p = root.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body.as_str().unwrap()).unwrap();
    }
}
#[test]
fn frozen_workspace_load_state_conformance() {
    let data = golden();
    assert_eq!(
        data["sourceCommit"],
        crate::series_lite_registry::SOURCE_COMMIT
    );
    let mut passed = 0;
    let mut refused = 0;
    let mut unsupported = 0;
    for case in data["cases"].as_array().unwrap() {
        let dir = tempfile::tempdir().unwrap();
        let mut files = data["bases"][case["base"].as_str().unwrap()].clone();
        for (k, v) in case["patches"].as_object().unwrap() {
            files[k] = v.clone();
        }
        for k in case["remove"].as_array().unwrap() {
            files.as_object_mut().unwrap().remove(k.as_str().unwrap());
        }
        materialize(dir.path(), &files);
        let actual = verify(dir.path());
        if case["unsupported"] == true {
            assert_eq!(
                case["oracleOk"], true,
                "unsupported fixture must be valid upstream: {}",
                case["name"]
            );
            assert!(
                actual
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.starts_with("WORKSPACE_UNSUPPORTED_HANDOFF_PROFILE:")),
                "{} {actual:?}",
                case["name"]
            );
            unsupported += 1;
        } else {
            assert_eq!(
                actual.is_ok(),
                case["oracleOk"] == true,
                "{}: native={actual:?}, oracle={}",
                case["name"],
                case["oracleError"]
            );
            if let Ok(raw) = actual {
                assert_eq!(
                    raw,
                    case["stateRaw"].as_str().unwrap().as_bytes(),
                    "{}",
                    case["name"]
                );
                passed += 1;
            } else {
                refused += 1;
            }
        }
        for (name, body) in files.as_object().unwrap() {
            assert_eq!(
                fs::read(dir.path().join(name)).unwrap(),
                body.as_str().unwrap().as_bytes(),
                "verification modified input {name}"
            );
        }
    }
    assert!(
        passed >= 7 && refused >= 45 && unsupported == 1,
        "passed={passed} refused={refused} unsupported={unsupported}"
    );
}
#[test]
fn verification_never_caches_success_across_source_or_dependency_drift() {
    let data = golden();
    for base in ["legacy", "first", "next"] {
        let files = &data["bases"][base];
        let dir = tempfile::tempdir().unwrap();
        materialize(dir.path(), files);
        let original = verify(dir.path()).unwrap();
        let target = if base == "next" {
            "previous/registry.json"
        } else {
            "source-body.txt"
        };
        fs::write(dir.path().join(target), "changed").unwrap();
        assert!(verify(dir.path()).is_err());
        fs::write(dir.path().join(target), files[target].as_str().unwrap()).unwrap();
        assert_eq!(verify(dir.path()).unwrap(), original);
    }
}
#[test]
fn read_set_rechecks_bytes_before_return() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("evidence"), "before").unwrap();
    let mut reads = Reads::new(dir.path()).unwrap();
    reads.local("evidence").unwrap();
    fs::write(dir.path().join("evidence"), "after").unwrap();
    assert!(reads.finish().is_err());
}
#[test]
fn native_trait_returns_exact_state() {
    use crate::series_lite_job_packet::WorkspaceScopeVerifier;
    let data = golden();
    let dir = tempfile::tempdir().unwrap();
    materialize(dir.path(), &data["bases"]["next"]);
    assert_eq!(
        NativeWorkspaceScopeVerifier
            .verify_workspace(dir.path())
            .unwrap(),
        data["bases"]["next"]["state.json"]
            .as_str()
            .unwrap()
            .as_bytes()
    );
}
#[test]
fn native_workspace_closes_job_packet_build_verify_and_frozen_replay() {
    use crate::series_lite_job_packet::{
        build, load_frozen, verify as verify_packet, DEFAULT_MAX_BYTES,
    };
    let data = golden();
    let packets: Value =
        serde_json::from_str(include_str!("series_lite_job_packet_golden.json")).unwrap();
    let pack = crate::series_lite_contract::VerifiedPack::embedded().unwrap();
    for (base, episode) in [("first", 1), ("next", 2)] {
        let dir = tempfile::tempdir().unwrap();
        materialize(dir.path(), &packets["cases"][0]["files"]);
        materialize(dir.path(), &data["bases"][base]);
        let mut request: Value =
            serde_json::from_slice(&fs::read(dir.path().join("request.json")).unwrap()).unwrap();
        request["episode"] = json!(episode);
        fs::write(
            dir.path().join("request.json"),
            serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        let built = build(
            dir.path(),
            "request.json",
            "packet.json",
            DEFAULT_MAX_BYTES,
            &pack,
            &NativeWorkspaceScopeVerifier,
        )
        .unwrap();
        assert_eq!(
            verify_packet(
                dir.path(),
                "packet.json",
                &pack,
                &NativeWorkspaceScopeVerifier
            )
            .unwrap()
            .bytes,
            built.bytes
        );
        assert_eq!(
            load_frozen(
                dir.path(),
                "packet.json",
                &built.sha256,
                &NativeWorkspaceScopeVerifier
            )
            .unwrap()
            .bytes,
            built.bytes
        );
        fs::write(dir.path().join("source-body.txt"), "drift").unwrap();
        assert!(verify_packet(
            dir.path(),
            "packet.json",
            &pack,
            &NativeWorkspaceScopeVerifier
        )
        .is_err());
        assert!(load_frozen(
            dir.path(),
            "packet.json",
            &built.sha256,
            &NativeWorkspaceScopeVerifier
        )
        .is_err());
        assert_eq!(
            fs::read(dir.path().join("packet.json")).unwrap(),
            built.bytes
        );
    }
}
#[test]
fn symlink_escape_refused_and_internal_alias_tracks_real_bytes() {
    let inside = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "outside").unwrap();
    fs::write(inside.path().join("local"), "inside").unwrap();
    #[cfg(unix)]
    let link = std::os::unix::fs::symlink;
    #[cfg(windows)]
    let link = std::os::windows::fs::symlink_file;
    // Windows without Developer Mode may refuse creation; canonical escape is
    // separately exercised by the frozen parent-path oracle, never accepted.
    if link(outside.path().join("secret"), inside.path().join("escape")).is_err() {
        return;
    }
    link(inside.path().join("local"), inside.path().join("alias")).unwrap();
    let mut reads = Reads::new(inside.path()).unwrap();
    assert!(reads.local("escape").is_err());
    assert_eq!(reads.local("alias").unwrap(), b"inside");
    reads.finish().unwrap();
    fs::remove_file(inside.path().join("alias")).unwrap();
    link(outside.path().join("secret"), inside.path().join("alias")).unwrap();
    assert!(reads.finish().is_err());
}
