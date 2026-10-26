//! A bounded Win32 CreateProcess cwd failure, before the SDK can submit.
use super::*;
use rusqlite::OptionalExtension;

pub(super) const STAGE: &str = "win32-createprocess-before-node";
const HELD: &str = "VISUAL_SPAWN_FAILURE_NOT_PROVEN";

fn allowable_history(plan: &Value) -> bool {
    plan["purpose"] == "capability"
        && (plan["basis"]["recoveryOf"].is_null()
            || plan["basis"]["recoveryOf"]["evidence"]["stage"]
                == "official-sdk-schema-mkdtemp-before-exec")
}

fn no_dispatch_artifacts(path: &Path) -> bool {
    [
        "sdk-receipt.json",
        "events.ndjson",
        "process-receipt.json",
        "runtime-dispatch-intent.json",
        "runtime-process-receipt.json",
    ]
    .iter()
    .all(|name| {
        fs::symlink_metadata(path.join(name))
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    })
}

fn immutable_file(path: &Path, expected: &Value) -> Result<Value> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let actual = hash_file(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || expected != &json!(actual.0) {
        return Err(HELD.into());
    }
    Ok(json!({"sha256":actual.0,"bytes":actual.1}))
}

fn failure_event(store: &Store, dispatch: &Value, expected_error: &str) -> Result<Value> {
    let c = store.conn()?;
    let event: Option<(i64,String,String,String)> = c.query_row(
        "SELECT id,type,data,created_at FROM events WHERE type IN ('visual_review_manual_required','visual_review_completed') AND created_at>=?1 AND json_extract(data,'$.assetId') IS NULL AND json_extract(data,'$.mediaId') IS NULL ORDER BY id LIMIT 1",
        [s(dispatch,"createdAt")], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
        .optional().map_err(|e|e.to_string())?;
    let (id, kind, raw, at) = event.ok_or(HELD)?;
    let data: Value = serde_json::from_str(&raw).map_err(|_| HELD)?;
    if kind != "visual_review_manual_required"
        || data != json!({"assetId":null,"mediaId":null,"message":expected_error})
        || chrono::DateTime::parse_from_rfc3339(&at).map_err(|_| HELD)?
            < chrono::DateTime::parse_from_rfc3339(s(dispatch, "createdAt")).map_err(|_| HELD)?
    {
        return Err(HELD.into());
    }
    Ok(json!({"id":id,"type":kind,"dataJson":raw,"createdAt":at}))
}

fn recorded_failure_bound(
    failure: &Value,
    plan: &Value,
    manifest_sha: &str,
    work: &Path,
    directory_sha: &str,
    os_error: &str,
) -> bool {
    failure["version"] == "visual-process-start-failure/v1"
        && failure["stage"] == "outer-node"
        && failure["attemptId"] == plan["attemptId"]
        && failure["manifestSha256"] == manifest_sha
        && failure["childCreated"] == false
        && failure["rawOsError"] == 267
        && failure["modelCalls"] == 0
        && failure["error"] == os_error
        && failure["workingDirectory"] == json!(work)
        && failure["executionDirectorySha256"] == directory_sha
        && failure["nodeExecutableSha256"] == plan["baseIdentity"]["nodeSha256"]
}

/// The diagnostic starts suspended. Even if Windows unexpectedly accepts the
/// directory, no Node code is resumed; that case cannot prove a prior failure.
#[cfg(windows)]
fn prove_invalid_cwd(node: &Path, cwd: &Path) -> Result<Value> {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new(node);
    command
        .args(["-e", "process.exit(0)"])
        .env_clear()
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x08000004);
    let system = crate::text_sdk::node_system_root()?;
    command.env("SystemRoot", &system).env("WINDIR", system);
    match command.spawn() {
        Err(error) if error.raw_os_error() == Some(267) => {
            Ok(json!({"rawOsError":267,"childCreated":false,"resumed":false,"modelCalls":0}))
        }
        Err(_) => Err(HELD.into()),
        Ok(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(
                "VISUAL_SPAWN_DIAGNOSTIC_NOT_REPRODUCED: 原工作目录目前可启动，未确认历史前置失败"
                    .into(),
            )
        }
    }
}
#[cfg(not(windows))]
fn prove_invalid_cwd(_: &Path, _: &Path) -> Result<Value> {
    Err(HELD.into())
}

pub(super) fn evidence(store: &Store, pointer: &Value) -> Result<Value> {
    if !cfg!(windows) {
        return Err(HELD.into());
    }
    let path = recovery::pointer_path(store, pointer)?;
    let manifest_sha = hash_file(&path.join("manifest.json"))?.0;
    let plan = native::verify_historical_manifest(&path.join("manifest.json"), &manifest_sha)?;
    let dispatch = read(&path.join("dispatch-intent.json"))?;
    if !allowable_history(&plan)
        || !no_dispatch_artifacts(&path)
        || !preparation_runner_known(&plan["runnerSha256"])
        || plan["baseIdentity"]["sdkImplementationSha256"]
            != crate::text_sdk::SDK_IMPLEMENTATION_SHA
        || dispatch["version"] != "visual-dispatch/v1"
        || dispatch["attemptId"] != plan["attemptId"]
        || dispatch["manifestSha256"] != manifest_sha
        || dispatch["maximumModelCalls"] != 1
    {
        return Err(HELD.into());
    }
    let mut executables = serde_json::Map::new();
    for (path_key, sha_key) in [
        ("nodeExecutable", "nodeSha256"),
        ("supervisorExecutable", "supervisorSha256"),
        ("runtimeExecutable", "runtimeSha256"),
    ] {
        executables.insert(
            path_key.into(),
            immutable_file(
                Path::new(s(&plan["baseIdentity"], path_key)),
                &plan["baseIdentity"][sha_key],
            )?,
        );
    }
    let sdk = Path::new(s(&plan["baseIdentity"], "resourceDirectory"))
        .join("node_modules/@openai/codex-sdk/dist/index.js");
    executables.insert(
        "sdkImplementation".into(),
        immutable_file(&sdk, &json!(crate::text_sdk::SDK_IMPLEMENTATION_SHA))?,
    );
    let mut logs = Vec::new();
    for name in ["runner.stdout.log", "runner.stderr.log"] {
        let file = immutable_file(&path.join(name), &json!(digest("")))?;
        if file["bytes"] != 0 {
            return Err(HELD.into());
        }
        logs.push(json!({"file":name,"sha256":file["sha256"],"bytes":0}));
    }
    let os_error = std::io::Error::from_raw_os_error(267).to_string();
    let failure_path = path.join("spawn-failure.json");
    let (mode, proof, event) = if failure_path.exists() {
        let failure = read(&failure_path)?;
        let work = workdir::verify(&path, &plan, &manifest_sha)?;
        if !recorded_failure_bound(
            &failure,
            &plan,
            &manifest_sha,
            &work,
            &hash_file(&path.join("execution-directory.json"))?.0,
            &os_error,
        ) {
            return Err(HELD.into());
        }
        (
            "recorded-start-failure",
            json!({"receipt":failure,"sha256":hash_file(&failure_path)?.0}),
            failure_event(
                store,
                &dispatch,
                &format!("VISUAL_PROCESS_START_FAILED: {os_error}"),
            )?,
        )
    } else {
        let work = path.join("work");
        // A legacy launch used this exact logical path. A namespace is not a
        // workaround for CreateProcessW's current-directory limit.
        if path.join("execution-directory.json").exists()
            || work.to_string_lossy().encode_utf16().count() < 260
            || fs::read(work.join(".director-text-root")).map_err(|e| e.to_string())?
                != b"isolated visual review\n"
        {
            return Err(HELD.into());
        }
        let event = failure_event(store, &dispatch, &os_error)?;
        let diagnostic =
            prove_invalid_cwd(Path::new(s(&plan["baseIdentity"], "nodeExecutable")), &work)?;
        (
            "legacy-cwd267",
            json!({"workingDirectory":work,"diagnostic":diagnostic}),
            event,
        )
    };
    // No post-start/unknown result may enter this recovery through a race.
    if !no_dispatch_artifacts(&path) {
        return Err(HELD.into());
    }
    Ok(
        json!({"version":"visual-local-preparation-evidence/v1","stage":STAGE,"mode":mode,
        "attemptId":plan["attemptId"],"manifestSha256":manifest_sha,"dispatchIntentSha256":hash_file(&path.join("dispatch-intent.json"))?.0,
        "runnerSha256":plan["runnerSha256"],"executables":executables,"logs":logs,"proof":proof,"originalFailureEvent":event,
        "originalStatus":"manual_required","runtimeDispatchStarted":false,"modelCalls":0,"maximumSuccessors":1,
        "priorRecovery":plan["basis"]["recoveryOf"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cwd_recovery_cannot_repeat_or_accept_a_runtime_or_other_recovery_stage() {
        let base = json!({"purpose":"capability","basis":{}});
        assert!(allowable_history(&base));
        let mut previous = base.clone();
        previous["basis"]["recoveryOf"] =
            json!({"evidence":{"stage":"official-sdk-schema-mkdtemp-before-exec"}});
        assert!(allowable_history(&previous));
        for stage in [STAGE, "network-timeout", "runtime-started"] {
            previous["basis"]["recoveryOf"]["evidence"]["stage"] = json!(stage);
            assert!(!allowable_history(&previous));
        }
        previous = base;
        previous["purpose"] = json!("candidate");
        assert!(!allowable_history(&previous));
    }
    #[test]
    fn any_sdk_or_runtime_artifact_blocks_pre_node_failure_proof() {
        let temp = tempfile::tempdir().unwrap();
        assert!(no_dispatch_artifacts(temp.path()));
        for name in [
            "sdk-receipt.json",
            "events.ndjson",
            "process-receipt.json",
            "runtime-dispatch-intent.json",
            "runtime-process-receipt.json",
        ] {
            let file = temp.path().join(name);
            fs::write(&file, "").unwrap();
            assert!(!no_dispatch_artifacts(temp.path()), "{name}");
            fs::remove_file(file).unwrap();
        }
    }
    #[test]
    fn recorded_spawn_failure_cannot_accept_other_errors_started_children_or_drift() {
        let plan = json!({"attemptId":"one","baseIdentity":{"nodeSha256":"node"}});
        let work = Path::new("isolated-work");
        let value = json!({"version":"visual-process-start-failure/v1","stage":"outer-node","attemptId":"one","manifestSha256":"manifest","childCreated":false,"rawOsError":267,"modelCalls":0,"error":"directory invalid","workingDirectory":work,"executionDirectorySha256":"directory","nodeExecutableSha256":"node"});
        assert!(recorded_failure_bound(
            &value,
            &plan,
            "manifest",
            work,
            "directory",
            "directory invalid"
        ));
        for (field, bad) in [
            ("stage", json!("model-runtime")),
            ("childCreated", json!(true)),
            ("rawOsError", json!(5)),
            ("modelCalls", json!(1)),
            ("error", json!("tls handshake eof")),
            ("attemptId", json!("other")),
            ("manifestSha256", json!("other")),
            ("workingDirectory", json!("other")),
            ("executionDirectorySha256", json!("other")),
            ("nodeExecutableSha256", json!("other")),
        ] {
            let mut changed = value.clone();
            changed[field] = bad;
            assert!(
                !recorded_failure_bound(
                    &changed,
                    &plan,
                    "manifest",
                    work,
                    "directory",
                    "directory invalid"
                ),
                "{field}"
            );
        }
    }
    #[test]
    fn legacy_error_event_must_be_exact_original_capability_failure() {
        for error in ["directory invalid", "tls handshake eof"] {
            let temp = tempfile::tempdir().unwrap();
            let store = Store::open(temp.path().join("p")).unwrap();
            store.create("spawn event", "p").unwrap();
            let dispatch = json!({"createdAt":now()});
            store
                .event(
                    "",
                    "visual_review_manual_required",
                    &json!({"assetId":null,"mediaId":null,"message":error}),
                )
                .unwrap();
            assert_eq!(
                failure_event(&store, &dispatch, "directory invalid").is_ok(),
                error == "directory invalid"
            );
        }
    }
    #[cfg(windows)]
    #[test]
    fn legacy_cwd_diagnostic_reproduces_real_267_and_never_resumes_a_process() {
        let temp = tempfile::tempdir().unwrap();
        let long = temp
            .path()
            .join("a".repeat(100))
            .join("b".repeat(120))
            .join("work");
        fs::create_dir_all(&long).unwrap();
        let long = crate::text_sdk::node_path_argument(&fs::canonicalize(long).unwrap()).unwrap();
        let executable = std::env::current_exe().unwrap();
        let proof = prove_invalid_cwd(&executable, &long).unwrap();
        assert_eq!(
            proof,
            json!({"rawOsError":267,"childCreated":false,"resumed":false,"modelCalls":0})
        );
        assert!(prove_invalid_cwd(&executable, temp.path()).is_err());
    }
}
