use super::*;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn checked_file(file: &Path, expected: &str, limit: u64) -> Result<()> {
    let meta = fs::symlink_metadata(file).map_err(|e| e.to_string())?;
    if !file.is_absolute()
        || meta.file_type().is_symlink()
        || !meta.is_file()
        || meta.len() > limit
        || hash_file(file)?.0 != expected
    {
        return Err("VISUAL_FILE_IDENTITY".into());
    }
    Ok(())
}
pub(crate) fn verify_manifest(file: &Path, expected: &str) -> Result<Value> {
    verify_manifest_mode(file, expected, true)
}
pub(crate) fn verify_historical_manifest(file: &Path, expected: &str) -> Result<Value> {
    verify_manifest_mode(file, expected, false)
}
fn verify_manifest_mode(file: &Path, expected: &str, current: bool) -> Result<Value> {
    checked_file(file, expected, 4 * 1024 * 1024)?;
    let plan = read(file)?;
    if plan["version"] != VERSION
        || plan["permissionProfile"] != POLICY
        || (current && plan["runnerSha256"] != digest(RUNNER))
        || plan["model"] != "gpt-6.1-sol"
        || plan["reasoning"] != "medium"
        || !["capability", "candidate"].contains(&s(&plan, "purpose"))
    {
        return Err("VISUAL_MANIFEST_CONTRACT".into());
    }
    let identity: crate::text_sdk::SdkIdentity =
        serde_json::from_value(plan["baseIdentity"].clone()).map_err(|e| e.to_string())?;
    if current {
        identity.validate()?;
    } else {
        identity.validate_receipt()?;
    }
    let root = fs::canonicalize(file.parent().ok_or("VISUAL_ROOT")?).map_err(|e| e.to_string())?;
    if fs::symlink_metadata(file.parent().unwrap())
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("VISUAL_ROOT_LINK".into());
    }
    let images = plan["images"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 12)
        .ok_or("VISUAL_IMAGE_COUNT")?;
    for (i, image) in images.iter().enumerate() {
        if image["index"].as_u64() != Some((i + 1) as u64)
            || !["candidate", "reference", "probe"].contains(&s(image, "role"))
        {
            return Err("VISUAL_IMAGE_ROLE".into());
        }
        let path = Path::new(s(image, "path"));
        checked_file(path, s(image, "sha256"), 20 * 1024 * 1024)?;
        if fs::canonicalize(path).map_err(|e| e.to_string())?.parent()
            != Some(root.join("images").as_path())
            || hash_file(path)?.1 != image["bytes"].as_u64().unwrap_or(0)
        {
            return Err("VISUAL_IMAGE_SCOPE".into());
        }
    }
    if fs::symlink_metadata(root.join("images"))
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("VISUAL_IMAGES_LINK".into());
    }
    checked_file(
        &root.join("runner.mjs"),
        s(&plan, "runnerSha256"),
        1024 * 1024,
    )?;
    // Historical result verification never launches or substitutes today's
    // executable. The frozen runner, input images and receipt chain suffice.
    if !current {
        return Ok(plan);
    }
    for (path_key, sha_key) in [
        ("runtimeExecutable", "runtimeSha256"),
        ("supervisorExecutable", "supervisorSha256"),
        ("nodeExecutable", "nodeSha256"),
    ] {
        checked_file(
            Path::new(s(&plan["baseIdentity"], path_key)),
            s(&plan["baseIdentity"], sha_key),
            1024 * 1024 * 1024,
        )?;
    }
    checked_file(
        &Path::new(s(&plan["baseIdentity"], "resourceDirectory"))
            .join("node_modules/@openai/codex-sdk/dist/index.js"),
        crate::text_sdk::SDK_IMPLEMENTATION_SHA,
        2 * 1024 * 1024,
    )?;
    checked_file(
        &Path::new(s(&plan["baseIdentity"], "resourceDirectory")).join("src/visual-review.mjs"),
        &digest(RUNNER),
        1024 * 1024,
    )?;
    Ok(plan)
}
fn contain(command: &mut Command) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004);
    }
    #[cfg(not(windows))]
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "VISUAL_PLATFORM_UNVERIFIED",
        ));
    }
    #[allow(unreachable_code)]
    command.spawn()
}
pub(crate) fn execute(root: &Path) -> Result<Value> {
    if !crate::text_sdk::native_execution_enabled() {
        return Err("VISUAL_NATIVE_REQUIRED".into());
    }
    let file = root.join("manifest.json");
    let sha = hash_file(&file)?.0;
    let plan = verify_manifest(&file, &sha)?;
    let work = workdir::prepare(root, &plan, &sha)?;
    // Node/libuv mkdtemp on Windows does not add the long-path namespace for
    // its prefix. Keep the canonical \\?\ form for TEMP (not script arguments).
    let temp = fs::canonicalize(root.join("tmp")).map_err(|e| e.to_string())?;
    let intent = root.join("dispatch-intent.json");
    let mut lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&intent)
        .map_err(|_| "VISUAL_ATTEMPT_ALREADY_DISPATCHED: 保留原任务，不重新调用")?;
    lock.write_all(json!({"version":"visual-dispatch/v1","manifestSha256":sha,"attemptId":plan["attemptId"],"createdAt":now(),"maximumModelCalls":1}).to_string().as_bytes()).map_err(|e|e.to_string())?;
    lock.sync_all().map_err(|e| e.to_string())?;
    let mut command = Command::new(s(&plan["baseIdentity"], "nodeExecutable"));
    command
        .arg(root.join("runner.mjs"))
        .arg(&file)
        .arg(&sha)
        .env_clear()
        .env("TEMP", &temp)
        .env("TMP", &temp)
        .env("TMPDIR", &temp)
        .current_dir(&work)
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            fs::File::create(root.join("runner.stdout.log")).map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(
            fs::File::create(root.join("runner.stderr.log")).map_err(|e| e.to_string())?,
        ));
    #[cfg(windows)]
    {
        let system = crate::text_sdk::node_system_root()?;
        command.env("SystemRoot", &system).env("WINDIR", system);
    }
    let mut child = match contain(&mut command) {
        Ok(child) => child,
        Err(error) => {
            atomic_json(
                &root.join("spawn-failure.json"),
                &json!({"version":"visual-process-start-failure/v1",
                "attemptId":plan["attemptId"],"manifestSha256":sha,"stage":"outer-node","childCreated":false,
                "rawOsError":error.raw_os_error(),"error":error.to_string(),"workingDirectory":work,
                "executionDirectorySha256":hash_file(&root.join("execution-directory.json"))?.0,
                "nodeExecutableSha256":plan["baseIdentity"]["nodeSha256"],"modelCalls":0,"failedAt":now()}),
            )?;
            return Err(format!("VISUAL_PROCESS_START_FAILED: {error}"));
        }
    };
    let job = match crate::text_sdk::ProcessJob::attach_std(&child) {
        Ok(v) => v,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    job.start()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(status);
        }
        if start.elapsed() > Duration::from_secs(210) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let proof = job.stop()?;
    let _ = child.wait();
    verify_manifest(&file, &sha)?;
    let artifact_hash = |name: &str| hash_file(&root.join(name)).ok().map(|v| v.0);
    let result = json!({"version":"visual-process/v1","attemptId":plan["attemptId"],"manifestSha256":sha,"tree":proof,"exitCode":status.and_then(|s|s.code()),"maximumModelCalls":1,
        "sdkReceiptSha256":artifact_hash("sdk-receipt.json"),"runtimeReceiptSha256":artifact_hash("runtime-process-receipt.json"),"eventsSha256":artifact_hash("events.ndjson")});
    atomic_json(&root.join("process-receipt.json"), &result)?;
    Ok(result)
}
fn arguments(args: &[String], plan: &Value, root: &Path) -> Result<PathBuf> {
    if args.first().map(String::as_str) != Some("exec") {
        return Err("VISUAL_WRAPPER_EXEC_ONLY".into());
    }
    let mut flags = BTreeMap::new();
    let mut images = vec![];
    let mut configs = vec![];
    let mut i = 1;
    while i < args.len() {
        let flag = &args[i];
        let value = if ["--experimental-json", "--skip-git-repo-check"].contains(&flag.as_str()) {
            String::new()
        } else if [
            "--model",
            "--sandbox",
            "--cd",
            "--output-schema",
            "--config",
            "--image",
        ]
        .contains(&flag.as_str())
        {
            i += 1;
            args.get(i)
                .ok_or("VISUAL_WRAPPER_ARGUMENT_MISSING")?
                .clone()
        } else {
            return Err("VISUAL_WRAPPER_ARGUMENT_BLOCKED".into());
        };
        if flag == "--image" {
            images.push(value);
        } else if flag == "--config" {
            configs.push(value);
        } else if flags.insert(flag.clone(), value).is_some() {
            return Err("VISUAL_WRAPPER_DUPLICATE_ARGUMENT".into());
        }
        i += 1;
    }
    configs.sort();
    let mut expected = vec![
        "model_reasoning_effort=\"medium\"".to_string(),
        "sandbox_workspace_write.network_access=false".into(),
        "web_search=\"disabled\"".into(),
        "approval_policy=\"never\"".into(),
    ];
    expected.sort();
    let frozen: Vec<_> = plan["images"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| s(v, "path").to_owned())
        .collect();
    if flags.len() != 6
        || !flags.contains_key("--experimental-json")
        || !flags.contains_key("--skip-git-repo-check")
        || flags.get("--model").map(String::as_str) != Some("gpt-6.1-sol")
        || flags.get("--sandbox").map(String::as_str) != Some("read-only")
        || configs != expected
        || images != frozen
    {
        return Err("VISUAL_WRAPPER_FROZEN_ARGUMENTS".into());
    }
    let cwd = fs::canonicalize(flags.get("--cd").ok_or("VISUAL_WORK_DIRECTORY")?)
        .map_err(|e| e.to_string())?;
    if cwd != fs::canonicalize(root.join("work")).map_err(|e| e.to_string())? {
        return Err("VISUAL_WORK_DIRECTORY".into());
    }
    let schema = PathBuf::from(
        flags
            .get("--output-schema")
            .ok_or("VISUAL_SCHEMA_MISSING")?,
    );
    let schema = fs::canonicalize(schema).map_err(|e| e.to_string())?;
    if !schema.starts_with(root.join("tmp")) || read(&schema)? != plan["schema"] {
        return Err("VISUAL_SCHEMA_IDENTITY".into());
    }
    Ok(schema)
}
pub fn run_runtime_wrapper() -> Result<bool> {
    let Some(file) = std::env::var_os("DIRECTOR_VISUAL_PLAN") else {
        return Ok(false);
    };
    let file = PathBuf::from(file);
    let expected =
        std::env::var("DIRECTOR_VISUAL_SHA256").map_err(|_| "VISUAL_PLAN_SHA_REQUIRED")?;
    let plan = verify_manifest(&file, &expected)?;
    let root = fs::canonicalize(file.parent().ok_or("VISUAL_ROOT")?).map_err(|e| e.to_string())?;
    let work = workdir::verify(&root, &plan, &expected)?;
    if hash_file(&std::env::current_exe().map_err(|e| e.to_string())?)?.0
        != plan["baseIdentity"]["supervisorSha256"]
    {
        return Err("VISUAL_WRAPPER_IDENTITY".into());
    }
    let schema = arguments(&std::env::args().skip(1).collect::<Vec<_>>(), &plan, &root)?;
    let mut input = vec![];
    std::io::stdin()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    if input != s(&plan, "prompt").as_bytes() {
        return Err("VISUAL_PROMPT_IDENTITY".into());
    }
    let dispatch = read(&root.join("dispatch-intent.json"))?;
    if dispatch["version"] != "visual-dispatch/v1"
        || dispatch["attemptId"] != plan["attemptId"]
        || dispatch["manifestSha256"] != expected
        || dispatch["maximumModelCalls"] != 1
    {
        return Err("VISUAL_DISPATCH_IDENTITY".into());
    }
    let mut runtime_lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("runtime-dispatch-intent.json"))
        .map_err(|_| "VISUAL_RUNTIME_ALREADY_DISPATCHED")?;
    runtime_lock
        .write_all(
            json!({"manifestSha256":expected,"attemptId":plan["attemptId"],"maximumModelCalls":1})
                .to_string()
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
    runtime_lock.sync_all().map_err(|e| e.to_string())?;
    let mut command = Command::new(s(&plan["baseIdentity"], "runtimeExecutable"));
    command
        .args([
            "exec",
            "--ignore-user-config",
            "--ephemeral",
            "--json",
            "--sandbox",
            "read-only",
            "--skip-git-repo-check",
            "--cd",
        ])
        .arg(&work)
        .arg("--output-schema")
        .arg(schema)
        .args(["--model", "gpt-6.1-sol"]);
    for image in plan["images"].as_array().unwrap() {
        command.arg("--image").arg(s(image, "path"));
    }
    for config in crate::text_executor::CLI_CONFIG {
        command.arg("--config").arg(config);
    }
    for config in [
        "model_reasoning_effort=\"medium\"",
        "model_provider=\"openai\"",
        "chatgpt_base_url=\"https://chatgpt.com/backend-api\"",
    ] {
        command.arg("--config").arg(config);
    }
    command
        .env_clear()
        .env("USERPROFILE", s(&plan["baseIdentity"], "credentialHome"))
        .env("HOME", s(&plan["baseIdentity"], "credentialHome"))
        .env("CODEX_HOME", s(&plan["baseIdentity"], "codexHome"))
        .env(
            "TEMP",
            fs::canonicalize(root.join("tmp")).map_err(|e| e.to_string())?,
        )
        .env(
            "TMP",
            fs::canonicalize(root.join("tmp")).map_err(|e| e.to_string())?,
        )
        .env(
            "TMPDIR",
            fs::canonicalize(root.join("tmp")).map_err(|e| e.to_string())?,
        )
        .current_dir(&work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        let system = crate::text_sdk::node_system_root()?;
        command.env("SystemRoot", &system).env("WINDIR", system);
    }
    let mut child =
        contain(&mut command).map_err(|e| format!("VISUAL_RUNTIME_START_FAILED: {e}"))?;
    let job = match crate::text_sdk::ProcessJob::attach_std(&child) {
        Ok(v) => v,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    job.start()?;
    let mut stdin = child.stdin.take().ok_or("VISUAL_STDIN")?;
    let out = child.stdout.take().ok_or("VISUAL_STDOUT")?;
    let err = child.stderr.take().ok_or("VISUAL_STDERR")?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = std::thread::spawn(move || relay(out, std::io::stdout(), 8 * 1024 * 1024));
    let error = std::thread::spawn(move || relay(err, std::io::stderr(), 128 * 1024));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(status);
        }
        if start.elapsed() > Duration::from_secs(190) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stopped = job.stop()?;
    let _ = child.wait();
    let input_ok = writer.join().is_ok_and(|v| v.is_ok());
    let output_ok = output.join().is_ok_and(|v| v.is_ok());
    let error_ok = error.join().is_ok_and(|v| v.is_ok());
    verify_manifest(&file, &expected)?;
    atomic_json(
        &root.join("runtime-process-receipt.json"),
        &json!({"version":"visual-runtime-process/v1","attemptId":plan["attemptId"],"manifestSha256":expected,"tree":stopped,"exitCode":status.and_then(|s|s.code()),"ioComplete":input_ok&&output_ok&&error_ok}),
    )?;
    if !input_ok || !output_ok || !error_ok || status.is_none_or(|s| !s.success()) {
        std::process::exit(1);
    }
    Ok(true)
}
fn relay(mut input: impl Read, mut output: impl Write, limit: usize) -> Result<()> {
    let mut buffer = [0u8; 8192];
    let mut total = 0;
    loop {
        let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        total += n;
        if total > limit {
            return Err("VISUAL_STREAM_LIMIT".into());
        }
        output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf, Value, Vec<String>) {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("work")).unwrap();
        fs::create_dir(root.join("tmp")).unwrap();
        let schema = root.join("tmp/schema.json");
        let plan = json!({"schema":{"type":"object"},"images":[{"path":"C:\\frozen\\1.png"},{"path":"C:\\frozen\\2.png"}]});
        atomic_json(&schema, &plan["schema"]).unwrap();
        let args = vec![
            "exec".into(),
            "--experimental-json".into(),
            "--model".into(),
            "gpt-6.1-sol".into(),
            "--sandbox".into(),
            "read-only".into(),
            "--cd".into(),
            root.join("work").to_string_lossy().into_owned(),
            "--skip-git-repo-check".into(),
            "--output-schema".into(),
            schema.to_string_lossy().into_owned(),
            "--config".into(),
            "model_reasoning_effort=\"medium\"".into(),
            "--config".into(),
            "sandbox_workspace_write.network_access=false".into(),
            "--config".into(),
            "web_search=\"disabled\"".into(),
            "--config".into(),
            "approval_policy=\"never\"".into(),
            "--image".into(),
            "C:\\frozen\\1.png".into(),
            "--image".into(),
            "C:\\frozen\\2.png".into(),
        ];
        (temp, root, plan, args)
    }
    #[test]
    fn visual_supervisor_only_accepts_frozen_sdk_argv_and_image_order() {
        let (_temp, root, plan, args) = fixture();
        assert!(arguments(&args, &plan, &root).is_ok());
        let mut wrong = args.clone();
        wrong[20] = "C:\\unapproved.png".into();
        assert!(arguments(&wrong, &plan, &root).is_err());
        let mut swapped = args.clone();
        swapped.swap(20, 22);
        assert!(arguments(&swapped, &plan, &root).is_err());
        for additional in [
            vec!["resume", "thread"],
            vec!["--image", "C:\\extra.png"],
            vec!["--config", "features.shell_tool=true"],
            vec!["--model", "other"],
        ] {
            let mut wrong = args.clone();
            wrong.extend(additional.into_iter().map(str::to_owned));
            assert!(arguments(&wrong, &plan, &root).is_err());
        }
    }
    #[test]
    fn changed_schema_or_working_directory_cannot_expand_visual_contract() {
        let (_temp, root, plan, args) = fixture();
        atomic_json(&root.join("tmp/schema.json"), &json!({"different":true})).unwrap();
        assert!(arguments(&args, &plan, &root).is_err());
        atomic_json(&root.join("tmp/schema.json"), &plan["schema"]).unwrap();
        let mut wrong = args;
        wrong[7] = root.to_string_lossy().into_owned();
        assert!(arguments(&wrong, &plan, &root).is_err());
    }
}
