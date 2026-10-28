use super::*;
use serde_json::Value;

fn golden() -> Value {
    serde_json::from_str(include_str!("series_lite_prepare_golden.json")).unwrap()
}
fn fixture(case: &Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, raw) in case["files"].as_object().unwrap() {
        let target = dir.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, raw.as_str().unwrap().as_bytes()).unwrap();
    }
    dir
}
fn options(v: &Value) -> PreparationOptions {
    let input = |kind: &str| {
        v[kind].as_str().map(|path| {
            (
                path.into(),
                v[format!("{kind}_pointer")].as_str().map(str::to_owned),
            )
        })
    };
    PreparationOptions {
        context: input("context"),
        direction: input("direction"),
        issues: input("issues"),
        methods: v["methods"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().into()).collect())
            .unwrap_or_default(),
        contracts: v["contracts"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().into()).collect())
            .unwrap_or_default(),
        max_bytes: v["max_bytes"].as_u64().map(|n| n as usize),
    }
}

#[test]
fn series_lite_prepare_matches_16_frozen_python_output_and_refusal_cases() {
    let oracle = golden();
    let pack = VerifiedPack::current_embedded().unwrap();
    for name in ["prepare_episode.py", "packet_view.py", "job_packet.py"] {
        assert_eq!(
            oracle["sourceHashes"][name],
            sha256(pack.file_bytes(&format!("scripts/{name}")).unwrap())
        );
    }
    for case in oracle["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let dir = fixture(case);
        let result = prepare(
            dir.path(),
            "base.json",
            "registry",
            case["role"].as_str().unwrap(),
            "prepared",
            &options(&case["options"]),
            &pack,
            &RejectUnverifiedWorkspace,
        );
        assert_eq!(
            result.is_ok(),
            case["ok"].as_bool().unwrap(),
            "{name}: {result:?}"
        );
        if let Ok(result) = result {
            for (path, raw) in case["outputs"].as_object().unwrap() {
                let actual = fs::read(dir.path().join(path)).unwrap();
                let expected = raw.as_str().unwrap().as_bytes();
                assert_eq!(
                    sha256(&actual),
                    sha256(expected),
                    "exact output bytes differ: {name}: {path}"
                );
            }
            let expected = &case["result"];
            assert_eq!(
                result.episode,
                expected["episode"].as_i64().unwrap() as i128,
                "{name}"
            );
            assert_eq!(
                result.base_sha256,
                expected["baseSha256"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(
                result.reading_bytes,
                expected["readingBytes"].as_u64().unwrap() as usize,
                "{name}"
            );
            assert_eq!(
                result.selected_assets,
                expected["selectedAssets"].as_u64().unwrap() as usize,
                "{name}"
            );
            assert_eq!(
                result.packet,
                expected["packet"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(result.read, expected["read"].as_str().unwrap(), "{name}");
            assert_eq!(result.role, expected["role"].as_str().unwrap(), "{name}");
            assert_eq!(
                result.validation,
                expected["validation"].as_str().unwrap(),
                "{name}"
            );
            let saved = load_frozen(
                dir.path(),
                &result.packet,
                &result.packet_sha256,
                &RejectUnverifiedWorkspace,
            )
            .unwrap();
            assert_eq!(
                render_reading_view(&saved.bytes).unwrap(),
                fs::read(dir.path().join(&result.read)).unwrap()
            );
            assert!(prepare(
                dir.path(),
                "base.json",
                "registry",
                &result.role,
                "prepared",
                &PreparationOptions::default(),
                &pack,
                &RejectUnverifiedWorkspace
            )
            .is_err());
        } else {
            assert_eq!(
                dir.path().join("prepared").exists(),
                case["outputExists"].as_bool().unwrap(),
                "{name}"
            );
        }
        assert_eq!(
            fs::read(dir.path().join("base.json")).unwrap(),
            case["files"]["base.json"].as_str().unwrap().as_bytes(),
            "base changed: {name}"
        );
        assert_eq!(
            fs::read(dir.path().join("registry/v000001.json")).unwrap(),
            case["files"]["registry/v000001.json"]
                .as_str()
                .unwrap()
                .as_bytes(),
            "registry changed: {name}"
        );
    }
}

#[test]
fn series_lite_prepare_no_reports_but_complete_original_values_and_true_design_closure() {
    let dir = fixture(&golden()["cases"][6]);
    let pack = VerifiedPack::current_embedded().unwrap();
    let result = prepare(
        dir.path(),
        "base.json",
        "registry",
        "author",
        "prepared",
        &PreparationOptions::default(),
        &pack,
        &RejectUnverifiedWorkspace,
    )
    .unwrap();
    let raw = fs::read(dir.path().join(&result.packet)).unwrap();
    let p = parse(&raw).unwrap();
    assert_eq!(p.get("review").unwrap(), &Json::Null);
    let kinds: Vec<_> = p
        .get("bindings")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|b| b.get("kind").unwrap().string().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["source", "scenes", "visual-baseline", "direction", "assets"]
    );
    let m: Value =
        serde_json::from_slice(&fs::read(dir.path().join("prepared/materials.json")).unwrap())
            .unwrap();
    assert_eq!(
        m["assets"]["assets"].as_array().unwrap().len(),
        result.selected_assets
    );
    assert!(result.selected_assets >= 11);
    let view = String::from_utf8(fs::read(dir.path().join(&result.read)).unwrap()).unwrap();
    assert!(view.contains("遮口鼻，不可露嘴"));
    assert!(!view.contains("无关请求历史不供给分镜"));
    assert!(!view.contains("制图记录不供给分镜"));
    // No preparatory audit is fabricated to satisfy the old interface.
    assert!(!dir.path().join("review.json").exists());
    assert_eq!(
        serde_json::from_slice::<Value>(
            &fs::read(dir.path().join("prepared/request.json")).unwrap()
        )
        .unwrap()
        .get("review"),
        None
    );
}

#[test]
fn series_lite_prepare_path_escape_and_old_pack_do_not_create_output() {
    let dir = fixture(&golden()["cases"][0]);
    let pack = VerifiedPack::current_embedded().unwrap();
    for path in ["../escape", "C:relative", "base.json:ads", "/absolute"] {
        assert!(prepare(
            dir.path(),
            "base.json",
            "registry",
            "author",
            path,
            &PreparationOptions::default(),
            &pack,
            &RejectUnverifiedWorkspace
        )
        .is_err());
    }
    assert!(prepare(
        dir.path(),
        "base.json",
        "registry",
        "author",
        "prepared",
        &PreparationOptions::default(),
        &VerifiedPack::embedded().unwrap(),
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert!(!dir.path().join("prepared").exists());
    // A missing pointer is local to this preparation; another independent base
    // remains usable with no reset/mutation of its original source or registry.
    let options = PreparationOptions {
        context: Some(("base.json".into(), Some("/missing".into()))),
        ..Default::default()
    };
    assert!(prepare(
        dir.path(),
        "base.json",
        "registry",
        "author",
        "bad",
        &options,
        &pack,
        &RejectUnverifiedWorkspace
    )
    .is_err());
    assert!(prepare(
        dir.path(),
        "base.json",
        "registry",
        "author",
        "good",
        &PreparationOptions::default(),
        &pack,
        &RejectUnverifiedWorkspace
    )
    .is_ok());
}
