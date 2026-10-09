use super::*;
use std::cell::Cell;

fn fixture(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    #[cfg(windows)]
    fs::write(&path, b"MZ\x00\x00synthetic-no-execution").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&path, b"\x7fELFsynthetic-no-execution").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    path
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(20)
}
fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn raw_release(hash: &str) -> Release<'_> {
    Release {
        program: Program::Node,
        directory: "synthetic-node",
        url: "https://nodejs.org/synthetic-fixture",
        digest: hash,
        executable: "node.exe",
        max_download: 1024,
        tar_gzip: false,
    }
}
fn fetch_bytes(bytes: &[u8], target: &Path) -> Result<()> {
    fs::write(target, bytes).map_err(|e| e.to_string())
}

#[test]
fn runtime_prepare_returns_only_paths_without_saving_or_running_programs() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path(), "never-execute.exe");
    let home = dir.path().join("application");
    let saved = json!({"codexPath":path,"textSdkNodePath":path,"dreaminaPath":path,"textApiKey":"NEVER_COPY_SECRET","textModel":"gpt-6.1-sol","reasoning":"medium"});
    for (provider, expected) in [("codex_cli", 1), ("codex_sdk", 2), ("dreamina", 1)] {
        let result = prepare(
            &json!({"provider":provider,"settings":{}}),
            &saved,
            &home,
            None,
        )
        .unwrap();
        assert_eq!(result["schemaVersion"], "runtime-prepare/v1");
        assert_eq!(result["provider"], provider);
        assert_eq!(result["settings"].as_object().unwrap().len(), expected);
        assert_eq!(result["installed"], json!([]));
        assert!(!result.to_string().contains("NEVER_COPY_SECRET"));
        assert!(!home.join(crate::executable_paths::FILE_NAME).exists());
        assert!(!home.join("projects").exists());
    }
}

#[test]
fn runtime_status_reads_native_components_without_creating_home_or_running_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path(), "must-never-run.exe");
    let before = fs::read(&path).unwrap();
    let home = dir.path().join("nonexistent-application");
    let saved = json!({"codexPath":path,"textSdkNodePath":path,"dreaminaPath":path,"textApiKey":"PRIVATE"});
    for (provider, count) in [("codex_cli", 1), ("codex_sdk", 2), ("dreamina", 1)] {
        let report = status(&json!({"provider":provider}), &saved, &home, None).unwrap();
        assert_eq!(report["schemaVersion"], "runtime-status/v1");
        assert_eq!(report["installed"], true);
        assert_eq!(report["settings"].as_object().unwrap().len(), count);
        assert_eq!(report["components"].as_array().unwrap().len(), count);
        assert!(report["components"].as_array().unwrap().iter().all(|item| item["state"] == "installed" && item["version"].is_null()));
        assert!(!report.to_string().contains("PRIVATE"));
        assert!(!home.exists(), "status must not even create its data directory");
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn runtime_status_detects_managed_corruption_without_replacing_files_or_taking_install_lock() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("application");
    let root = home.join("runtimes");
    let spec = release(Program::Dreamina);
    let directory = root.join(spec.directory);
    fs::create_dir_all(&directory).unwrap();
    let path = fixture(&directory, spec.executable);
    let original = fs::read(&path).unwrap();
    let report = status(&json!({"provider":"dreamina","settings":{"dreaminaPath":path}}), &json!({}), &home, None).unwrap();
    assert_eq!(report["installed"], false);
    assert_eq!(report["installSupported"], false);
    assert_eq!(report["components"][0]["state"], "invalid");
    assert!(s(&report["components"][0], "message").contains("RUNTIME_INSTALL_INCOMPLETE"));
    assert_eq!(report["settings"], json!({}));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!root.join(".prepare.lock").exists());
    assert!(!directory.join("receipt.json").exists());
}

#[test]
fn runtime_status_missing_component_is_not_an_account_status() {
    let dir = tempfile::tempdir().unwrap();
    let component = component_status(Program::Node, Ok(None), dir.path());
    assert_eq!(component["state"], "missing");
    assert!(component.get("path").is_none());
    assert!(component.get("authentication").is_none());
    assert!(component["version"].is_null());
}

#[test]
fn runtime_status_rejects_unexpected_fields_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("missing");
    for request in [json!({"provider":"codex_sdk","settings":{"textApiKey":"secret"}}), json!({"provider":"dreamina_canvas"})] {
        assert!(status(&request, &json!({}), &home, None).is_err());
        assert!(!home.exists());
    }
}

#[test]
fn runtime_prepare_uses_machine_defaults_when_project_path_is_stale() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path(), "dreamina.exe");
    let home = dir.path().join("application");
    crate::executable_paths::remember(&home, &json!({"dreaminaPath":path})).unwrap();
    let prior = fs::read(home.join(crate::executable_paths::FILE_NAME)).unwrap();
    let result = prepare(
        &json!({"provider":"dreamina","settings":{"dreaminaPath":dir.path().join("missing.exe")}}),
        &json!({}),
        &home,
        None,
    )
    .unwrap();
    assert_eq!(result["settings"]["dreaminaPath"], json!(path));
    assert_eq!(
        fs::read(home.join(crate::executable_paths::FILE_NAME)).unwrap(),
        prior
    );
}

#[test]
fn runtime_prepare_rejects_credentials_execution_flags_and_canvas_before_writes() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("application");
    for request in [
        json!({"provider":"codex_sdk","settings":{"textApiKey":"secret"}}),
        json!({"provider":"codex_sdk","settings":{"args":["exec"]}}),
        json!({"provider":"dreamina_canvas"}),
        json!({"provider":"dreamina","settings":{"dreaminaPath":"bad\npath"}}),
    ] {
        assert!(prepare(&request, &json!({}), &home, None).is_err());
        assert!(!home.exists());
    }
}

#[test]
fn runtime_setup_is_denied_by_offline_and_bounded_acceptance_policy() {
    let request = json!({"action":"runtime.prepare","provider":"codex_sdk"});
    assert!(crate::execution_policy::check_standard(&request).is_ok());
    assert!(crate::execution_policy::check(&request).is_err());
    assert!(crate::execution_policy::check_native_action(&request).is_err());
}

#[test]
fn runtime_prepare_busy_does_not_wait_or_touch_settings() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("application");
    let root = home.join("runtimes");
    fs::create_dir_all(&root).unwrap();
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(root.join(".prepare.lock"))
        .unwrap();
    lease.try_lock().unwrap();
    let start = Instant::now();
    let error = prepare(&json!({"provider":"dreamina"}), &json!({}), &home, None).unwrap_err();
    assert!(error.starts_with("RUNTIME_SETUP_BUSY"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn runtime_private_install_is_atomic_reusable_and_detects_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture(dir.path(), "input.exe");
    let bytes = fs::read(input).unwrap();
    let hash = sha256(&bytes);
    let spec = raw_release(&hash);
    let calls = Cell::new(0);
    let install = || {
        install_with(&spec, dir.path(), deadline(), |_, target, _| {
            calls.set(calls.get() + 1);
            fetch_bytes(&bytes, target)
        })
    };
    let path = install().unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(install().unwrap(), path);
    assert_eq!(calls.get(), 1);
    assert_eq!(
        verify_discovered(path.clone(), dir.path(), &spec).unwrap(),
        path
    );
    assert!(path.parent().unwrap().join("receipt.json").is_file());
    fs::write(&path, b"MZ\x00\x00changed").unwrap();
    assert!(install().unwrap_err().contains("RUNTIME_INSTALL_CHANGED"));
    assert!(verify_discovered(path.clone(), dir.path(), &spec)
        .unwrap_err()
        .contains("RUNTIME_INSTALL_CHANGED"));
    assert_eq!(
        calls.get(),
        1,
        "never overwrites a possibly running installation"
    );
}

#[test]
fn runtime_failed_download_is_clean_and_can_retry() {
    let dir = tempfile::tempdir().unwrap();
    let file = fixture(dir.path(), "input.exe");
    let bytes = fs::read(file).unwrap();
    let hash = sha256(&bytes);
    let spec = raw_release(&hash);
    let error = install_with(&spec, dir.path(), deadline(), |_, target, _| {
        fetch_bytes(b"wrong", target)
    })
    .unwrap_err();
    assert!(error.starts_with("RUNTIME_DOWNLOAD_HASH_MISMATCH"));
    assert!(!dir.path().join(spec.directory).exists());
    assert!(!fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".install-")));
    assert!(
        install_with(&spec, dir.path(), deadline(), |_, target, _| fetch_bytes(
            &bytes, target
        ))
        .unwrap()
        .is_file()
    );
}

#[test]
fn runtime_size_limit_and_deadline_do_not_publish() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = vec![b'X'; 1025];
    let hash = sha256(&bytes);
    let spec = raw_release(&hash);
    assert!(
        install_with(&spec, dir.path(), deadline(), |_, target, _| fetch_bytes(
            &bytes, target
        ))
        .unwrap_err()
        .contains("RUNTIME_SIZE_LIMIT")
    );
    let file = fixture(dir.path(), "input.exe");
    let bytes = fs::read(file).unwrap();
    let hash = sha256(&bytes);
    let spec = raw_release(&hash);
    assert!(install_with(
        &spec,
        dir.path(),
        Instant::now() - Duration::from_secs(1),
        |_, target, _| fetch_bytes(&bytes, target)
    )
    .unwrap_err()
    .contains("RUNTIME_SETUP_TIMEOUT"));
    assert!(!dir.path().join(spec.directory).exists());
}

#[test]
fn runtime_paths_refuse_shells_traversal_and_alternate_streams() {
    let dir = tempfile::tempdir().unwrap();
    let batch = dir.path().join("node.exe");
    fs::write(&batch, b"@echo not-a-native-binary").unwrap();
    assert!(!native_file(&batch));
    for path in [
        "../escape",
        "package/../escape",
        "C:/outside",
        "/outside",
        "package\\outside",
        "package/a:stream",
        "package//a",
        "package/a.",
        "package/a ",
        "package/./a",
    ] {
        assert!(!safe_relative(path), "{path}");
    }
    assert!(safe_relative(CODEX_EXE));
}

fn tar_fixture(path: &Path, entries: &[(&str, &[u8], bool)]) {
    let output = File::create(path).unwrap();
    let gzip = flate2::write::GzEncoder::new(output, flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    for (name, bytes, link) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o700);
        if *link {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("../escape").unwrap();
            header.set_size(0);
        } else {
            header.set_size(bytes.len() as u64);
        }
        header.set_cksum();
        archive.append_data(&mut header, name, *bytes).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
}

#[test]
fn runtime_archive_rejects_links_and_case_collisions() {
    for link in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("test.tgz");
        let output = dir.path().join("output");
        fs::create_dir(&output).unwrap();
        let entries = if link {
            vec![("package/link", b"".as_slice(), true)]
        } else {
            vec![
                ("package/a", b"1".as_slice(), false),
                ("package/A", b"2".as_slice(), false),
            ]
        };
        tar_fixture(&archive, &entries);
        assert!(unpack(&archive, &output, deadline())
            .unwrap_err()
            .contains("RUNTIME_ARCHIVE_PATH_INVALID"));
        assert!(!dir.path().join("escape").exists());
    }
}

#[test]
fn runtime_archive_keeps_complete_codex_layout_and_helpers() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture(dir.path(), "input.exe");
    let bytes = fs::read(input).unwrap();
    let archive = dir.path().join("test.tgz");
    let names = [
        CODEX_EXE,
        "package/vendor/x86_64-pc-windows-msvc/codex-package.json",
        "package/vendor/x86_64-pc-windows-msvc/codex-resources/codex-command-runner.exe",
        "package/vendor/x86_64-pc-windows-msvc/codex-resources/codex-windows-sandbox-setup.exe",
        "package/vendor/x86_64-pc-windows-msvc/bin/codex-code-mode-host.exe",
        "package/vendor/x86_64-pc-windows-msvc/codex-path/rg.exe",
    ];
    let entries: Vec<_> = names
        .iter()
        .map(|p| (*p, bytes.as_slice(), false))
        .collect();
    tar_fixture(&archive, &entries);
    let hash = digest_file(&archive, 1024 * 1024, true).unwrap();
    let spec = Release {
        program: Program::Codex,
        directory: "fixture-codex",
        url: "https://registry.npmjs.org/fixture",
        digest: &hash,
        executable: CODEX_EXE,
        max_download: 1024 * 1024,
        tar_gzip: true,
    };
    let output = install_with(&spec, dir.path(), deadline(), |_, target, _| {
        fs::copy(&archive, target).map_err(|e| e.to_string())?;
        Ok(())
    })
    .unwrap();
    assert!(output.is_file());
    for name in names {
        assert!(dir.path().join(spec.directory).join(name).is_file());
    }
    assert_eq!(installed(dir.path(), &spec).unwrap(), Some(output));
}

#[test]
fn runtime_pinned_sources_are_official_and_versioned() {
    for program in [Program::Node, Program::Codex, Program::Dreamina] {
        let spec = release(program);
        let url = reqwest::Url::parse(spec.url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert!([
            "nodejs.org",
            "registry.npmjs.org",
            "lf3-static.bytednsdoc.com"
        ]
        .contains(&url.host_str().unwrap()));
        assert!(spec.digest.starts_with("sha256:") || spec.digest.starts_with("sha512:"));
        assert!(safe_relative(spec.executable));
    }
}

#[test]
#[ignore = "Explicit offline full official archive verification; requires DIRECTOR_TEST_CODEX_ARCHIVE; never executes binaries or connects accounts"]
fn runtime_actual_official_codex_archive_installs_all_dependencies_offline() {
    let archive = PathBuf::from(
        std::env::var_os("DIRECTOR_TEST_CODEX_ARCHIVE").expect("local verified archive path"),
    );
    let dir = tempfile::tempdir().unwrap();
    let spec = release(Program::Codex);
    let path = install_with(
        &spec,
        dir.path(),
        Instant::now() + TOTAL_TIMEOUT,
        |_, target, _| {
            fs::copy(&archive, target).map_err(|e| e.to_string())?;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(installed(dir.path(), &spec).unwrap(), Some(path));
    let receipt: Value = serde_json::from_slice(
        &fs::read(dir.path().join(spec.directory).join("receipt.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["files"].as_array().unwrap().len(), 47);
}

#[test]
#[cfg(all(windows, target_arch = "x86_64"))]
#[ignore = "Explicit official-download install verification in temporary directories only; never executes binaries or connects accounts"]
fn runtime_official_node_and_dreamina_downloads_verify_and_install() {
    let dir = tempfile::tempdir().unwrap();
    for program in [Program::Node, Program::Dreamina] {
        let spec = release(program);
        let path =
            install_with(&spec, dir.path(), Instant::now() + TOTAL_TIMEOUT, download).unwrap();
        assert_eq!(installed(dir.path(), &spec).unwrap(), Some(path));
        println!("{}: official download, digest, atomic install and readback verified; zero binary executions", label(program));
    }
}
