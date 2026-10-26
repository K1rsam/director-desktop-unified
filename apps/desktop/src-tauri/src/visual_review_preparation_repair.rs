//! One narrowly proved recovery: pinned SDK schema mkdtemp failed before exec.
//! Original UNKNOWN, raw events, images and manifest are never rewritten.
use super::*;

fn records_prove_local_schema_failure(
    plan: &Value,
    sdk: &Value,
    process: &Value,
    dispatch: &Value,
    expected_error: &str,
) -> bool {
    plan["purpose"] == "capability"
        && plan["basis"]["recoveryOf"].is_null()
        && plan["baseIdentity"]["sdkImplementationSha256"]
            == crate::text_sdk::SDK_IMPLEMENTATION_SHA
        && plan["baseIdentity"]["sdkVersion"] == crate::text_sdk::SDK_VERSION
        && preparation_runner_known(&plan["runnerSha256"])
        && sdk["version"] == "visual-sdk-receipt/v1"
        && sdk["state"] == "submission_unknown"
        && sdk["attemptId"] == plan["attemptId"]
        && sdk["error"] == expected_error
        && sdk["eventCount"] == 0
        && sdk["threadId"].is_null()
        && sdk["usage"].is_null()
        && sdk["finalText"].is_null()
        && sdk["eventsSha256"] == digest("")
        && sdk["model"] == plan["model"]
        && sdk["reasoning"] == plan["reasoning"]
        && process["version"] == "visual-process/v1"
        && process["attemptId"] == plan["attemptId"]
        && process["exitCode"] == 0
        && process["maximumModelCalls"] == 1
        && process["runtimeReceiptSha256"].is_null()
        && process["tree"]["processTreeStopProven"] == true
        && process["tree"]["activeProcesses"] == 0
        && process["tree"]["totalProcesses"]
            .as_u64()
            .is_some_and(|n| (1..=2).contains(&n))
        && dispatch["version"] == "visual-dispatch/v1"
        && dispatch["attemptId"] == plan["attemptId"]
        && dispatch["maximumModelCalls"] == 1
}
fn schema_evidence(store: &Store, pointer: &Value) -> Result<Value> {
    if !cfg!(windows) {
        return Err("VISUAL_PREPARATION_NOT_PROVEN".into());
    }
    let path = recovery::pointer_path(store, pointer)?;
    let manifest_sha = hash_file(&path.join("manifest.json"))?.0;
    let plan = native::verify_historical_manifest(&path.join("manifest.json"), &manifest_sha)?;
    let sdk = read(&path.join("sdk-receipt.json"))?;
    let process = read(&path.join("process-receipt.json"))?;
    let dispatch = read(&path.join("dispatch-intent.json"))?;
    let temp = crate::text_sdk::node_path_argument(
        &fs::canonicalize(path.join("tmp")).map_err(|e| e.to_string())?,
    )?;
    let prefix = temp.join("codex-output-schema-XXXXXX");
    let expected_error = format!(
        "ENOENT: no such file or directory, mkdtemp '{}'",
        prefix.to_string_lossy()
    );
    let sdk_path = Path::new(s(&plan["baseIdentity"], "resourceDirectory"))
        .join("node_modules/@openai/codex-sdk/dist/index.js");
    let sdk_sha = hash_file(&path.join("sdk-receipt.json"))?.0;
    let events = hash_file(&path.join("events.ndjson"))?;
    if prefix.to_string_lossy().len() <= 260
        || !records_prove_local_schema_failure(&plan, &sdk, &process, &dispatch, &expected_error)
        || hash_file(&sdk_path)?.0 != crate::text_sdk::SDK_IMPLEMENTATION_SHA
        || sdk["manifestSha256"] != manifest_sha
        || process["manifestSha256"] != manifest_sha
        || dispatch["manifestSha256"] != manifest_sha
        || process["sdkReceiptSha256"] != sdk_sha
        || events.1 != 0
        || process["eventsSha256"] != events.0
        || path.join("runtime-dispatch-intent.json").exists()
        || path.join("runtime-process-receipt.json").exists()
        || fs::read_dir(&temp)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err(
            "VISUAL_PREPARATION_NOT_PROVEN: 原结果保留 UNKNOWN，未通过精确前置失败证明".into(),
        );
    }
    Ok(
        json!({"version":"visual-local-preparation-evidence/v1","attemptId":plan["attemptId"],"manifestSha256":manifest_sha,
        "sdkReceiptSha256":sdk_sha,"eventsSha256":events.0,"processReceiptSha256":hash_file(&path.join("process-receipt.json"))?.0,
        "dispatchIntentSha256":hash_file(&path.join("dispatch-intent.json"))?.0,"sdkImplementationSha256":crate::text_sdk::SDK_IMPLEMENTATION_SHA,
        "runnerSha256":plan["runnerSha256"],"stage":"official-sdk-schema-mkdtemp-before-exec","error":expected_error,
        "originalStatus":"submission_unknown","runtimeDispatchStarted":false,"processTreeStopProven":true,"maximumSuccessors":1}),
    )
}
fn evidence(store: &Store, pointer: &Value) -> Result<Value> {
    schema_evidence(store, pointer).or_else(|_| spawn_repair::evidence(store, pointer))
}
pub(super) fn settle(store: &Store, pointer: &Value) -> Result<Value> {
    let evidence = evidence(store, pointer)?;
    let path = recovery::pointer_path(store, pointer)?;
    let file = path.join("local-preparation-closure.json");
    if file.exists() {
        return verified_closure(store, pointer)?
            .ok_or("VISUAL_PREPARATION_CLOSURE_MISSING".into());
    }
    let reason = if evidence["stage"] == spawn_repair::STAGE {
        "已核实 Windows 在启动 SDK 进程前拒绝工作目录，尚未启动模型运行器；原错误与证据完整保留，可显式恢复一次。"
    } else {
        "已核实官方 SDK 在建立输出格式临时目录时失败，尚未启动模型运行器；原 UNKNOWN 与回包完整保留，可显式恢复一次。"
    };
    let closure = json!({"version":"visual-local-preparation-closure/v1","status":"not_submitted","modelCalls":0,
        "originalAttemptId":pointer["attemptId"],"originalManifestSha256":pointer["manifestSha256"],"evidence":evidence,"evidenceSha256":hash(&evidence),
        "closedAt":now(),"maximumSuccessors":1,"reason":reason});
    atomic_json(&file, &closure)?;
    Ok(closure)
}
pub(super) fn verified_closure(store: &Store, pointer: &Value) -> Result<Option<Value>> {
    let file = recovery::pointer_path(store, pointer)?.join("local-preparation-closure.json");
    if !file.exists() {
        return Ok(None);
    }
    let saved = read(&file)?;
    let current = evidence(store, pointer)?;
    if saved["version"] != "visual-local-preparation-closure/v1"
        || saved["evidence"] != current
        || saved["evidenceSha256"] != hash(&current)
        || saved["originalAttemptId"] != pointer["attemptId"]
        || saved["originalManifestSha256"] != pointer["manifestSha256"]
        || saved["maximumSuccessors"] != 1
        || saved["status"] != "not_submitted"
        || saved["modelCalls"] != 0
    {
        return Err("VISUAL_PREPARATION_CLOSURE_CHANGED".into());
    }
    Ok(Some(saved))
}
fn compatible_identity(old: &Value, new: &Value) -> bool {
    let mut comparable = old.clone();
    for key in [
        "supervisorExecutable",
        "supervisorSha256",
        "resourceDirectory",
        "bridgeSha256",
    ] {
        comparable[key] = new[key].clone();
    }
    comparable == *new
}
pub(super) fn prepare_successor(
    store: &Store,
    pointer: &Value,
    current: &Value,
) -> Result<PathBuf> {
    let closure =
        verified_closure(store, pointer)?.ok_or("VISUAL_PREPARATION_RECONCILE_REQUIRED")?;
    let old_path = recovery::pointer_path(store, pointer)?;
    let old = read(&old_path.join("manifest.json"))?;
    if !compatible_identity(&old["baseIdentity"], current) {
        return Err("VISUAL_PREPARATION_ACCOUNT_OR_RUNTIME_CHANGED: 恢复不可改变账户、SDK、模型或上游运行器".into());
    }
    if old_path.join("local-preparation-successor.json").exists() {
        return Err(
            "VISUAL_PREPARATION_RECOVERY_ALREADY_RESERVED: 已有一次后继，核对原记录而非再提交"
                .into(),
        );
    }
    let key = format!(
        "probe-repair-{}",
        hash(&json!({"original":pointer["manifestSha256"],"scope":scope(current)}))
    );
    let path = allocate(store, &key)?;
    let mut images = Vec::new();
    for image in old["images"].as_array().ok_or("VISUAL_IMAGE_COUNT")? {
        let frozen = freeze_image(
            &path,
            images.len() + 1,
            "probe",
            &json!({}),
            Path::new(s(image, "path")),
        )?;
        if frozen["sha256"] != image["sha256"] {
            return Err("VISUAL_PREPARATION_IMAGE_CHANGED".into());
        }
        images.push(frozen);
    }
    atomic_json(
        &path.join("expected.private.json"),
        &read(&old_path.join("expected.private.json"))?,
    )?;
    manifest(
        &path,
        "capability",
        current.clone(),
        images,
        s(&old, "prompt").into(),
        old["schema"].clone(),
        json!({"scopeSha256":scope(current),"recoveryOf":closure}),
    )?;
    Ok(path)
}
pub(super) fn validate_successor(store: &Store, previous: &Value, next: &Value) -> Result<()> {
    let closure =
        verified_closure(store, previous)?.ok_or("VISUAL_PREPARATION_RECONCILE_REQUIRED")?;
    let old_path = recovery::pointer_path(store, previous)?;
    let new_path = recovery::pointer_path(store, next)?;
    let old = read(&old_path.join("manifest.json"))?;
    let new = read(&new_path.join("manifest.json"))?;
    let image_identity = |v: &Value| {
        v["images"].as_array().map(|rows|rows.iter().map(|v|json!({"sha256":v["sha256"],"bytes":v["bytes"],"role":v["role"],"index":v["index"]})).collect::<Vec<_>>())
    };
    if new["purpose"] != "capability"
        || new["basis"]["recoveryOf"] != closure
        || !compatible_identity(&old["baseIdentity"], &new["baseIdentity"])
        || new["prompt"] != old["prompt"]
        || new["schema"] != old["schema"]
        || image_identity(&old) != image_identity(&new)
        || hash_file(&old_path.join("expected.private.json"))?.0
            != hash_file(&new_path.join("expected.private.json"))?.0
    {
        return Err("VISUAL_PREPARATION_SUCCESSOR_CHANGED".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_successor_cannot_change_account_sdk_or_upstream_runtime() {
        let old = json!({"accountFingerprint":"a","sdkImplementationSha256":"sdk","runtimeSha256":"runtime","nodeSha256":"node","supervisorExecutable":"old","supervisorSha256":"old-sha","resourceDirectory":"old-res","bridgeSha256":"old-bridge"});
        let mut next = old.clone();
        next["supervisorExecutable"] = json!("new");
        next["supervisorSha256"] = json!("new-sha");
        next["resourceDirectory"] = json!("new-res");
        next["bridgeSha256"] = json!("new-bridge");
        assert!(compatible_identity(&old, &next));
        for key in [
            "accountFingerprint",
            "sdkImplementationSha256",
            "runtimeSha256",
            "nodeSha256",
        ] {
            let mut changed = next.clone();
            changed[key] = json!("different");
            assert!(!compatible_identity(&old, &changed));
        }
    }
    #[test]
    fn empty_events_alone_cannot_prove_schema_preparation_failure() {
        let plan = json!({"purpose":"capability","basis":{},"baseIdentity":{"sdkImplementationSha256":crate::text_sdk::SDK_IMPLEMENTATION_SHA,"sdkVersion":crate::text_sdk::SDK_VERSION},"runnerSha256":digest(RUNNER),"attemptId":"original","model":"gpt-6.1-sol","reasoning":"medium"});
        let sdk = json!({"version":"visual-sdk-receipt/v1","state":"submission_unknown","attemptId":"original","error":"exact schema mkdtemp failure","eventCount":0,"threadId":null,"usage":null,"finalText":null,"eventsSha256":digest(""),"model":"gpt-6.1-sol","reasoning":"medium"});
        let process = json!({"version":"visual-process/v1","attemptId":"original","exitCode":0,"maximumModelCalls":1,"runtimeReceiptSha256":null,"tree":{"processTreeStopProven":true,"activeProcesses":0,"totalProcesses":2}});
        let dispatch =
            json!({"version":"visual-dispatch/v1","attemptId":"original","maximumModelCalls":1});
        assert!(records_prove_local_schema_failure(
            &plan,
            &sdk,
            &process,
            &dispatch,
            "exact schema mkdtemp failure"
        ));
        for (field, value) in [
            ("error", json!("tls handshake eof")),
            ("eventCount", json!(1)),
            ("threadId", json!("thread")),
            ("usage", json!({})),
            ("finalText", json!("returned")),
        ] {
            let mut altered = sdk.clone();
            altered[field] = value;
            assert!(!records_prove_local_schema_failure(
                &plan,
                &altered,
                &process,
                &dispatch,
                "exact schema mkdtemp failure"
            ));
        }
        let mut tree = process.clone();
        tree["tree"]["processTreeStopProven"] = json!(false);
        assert!(!records_prove_local_schema_failure(
            &plan,
            &sdk,
            &tree,
            &dispatch,
            "exact schema mkdtemp failure"
        ));
        let mut repeated = plan;
        repeated["basis"]["recoveryOf"] = json!({"originalAttemptId":"older"});
        assert!(!records_prove_local_schema_failure(
            &repeated,
            &sdk,
            &process,
            &dispatch,
            "exact schema mkdtemp failure"
        ));
    }
}
