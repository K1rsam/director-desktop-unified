//! A single explicitly authorized new probe after the old startup-diagnostic
//! runner defect. The previous attempt remains UNKNOWN; this is not a retry.
use super::*;
use std::io::Write;

const CLAIM: &str = "explicit-capability-restart.json";
const AUTHORITY: &str = "explicit-current-operator";

pub(super) fn request_binding(request: &Value, pointer: &Value) -> Result<()> {
    if request["acknowledgePossiblePriorUsage"] != true
        || request["expectedAttemptId"] != pointer["attemptId"]
        || request["expectedManifestSha256"] != pointer["manifestSha256"]
        || pointer["purpose"] != "capability"
    {
        return Err("VISUAL_RESTART_CONFIRMATION_REQUIRED: 请明确确认原验证可能已消耗额度，并绑定原验证记录；本次是新增一次验证".into());
    }
    Ok(())
}

fn chain_root(store: &Store, pointer: &Value) -> Result<PathBuf> {
    let mut current = pointer.clone();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..32 {
        let path = recovery::pointer_path(store, &current)?;
        if !seen.insert(s(&current, "attemptId").to_owned())
            || read(&path.join("pointer.json"))? != current
        {
            return Err("VISUAL_RESTART_CHAIN_CHANGED".into());
        }
        if current["previous"].is_null() {
            return Ok(path);
        }
        current = current["previous"].clone();
    }
    Err("VISUAL_RESTART_CHAIN_LIMIT".into())
}

fn compatible_identity(old: &Value, current: &Value) -> bool {
    let mut comparison = old.clone();
    for key in [
        "supervisorExecutable",
        "supervisorSha256",
        "resourceDirectory",
        "bridgeSha256",
    ] {
        comparison[key] = current[key].clone();
    }
    comparison == *current
}

fn bound_stopped(
    plan: &Value,
    sdk: &Value,
    process: &Value,
    dispatch: &Value,
    runtime: &Value,
    sha: &str,
) -> bool {
    plan["purpose"] == "capability"
        && plan["runnerSha256"] == LEGACY_DIAGNOSTIC_RUNNER_SHA
        && plan["basis"]["supersedesUnknown"].is_null()
        && sdk["state"] == "submission_unknown"
        && sdk["error"] == "VISUAL_TOOL_BLOCKED"
        && sdk["finalText"].is_null()
        && sdk["usage"].is_null()
        && sdk["model"] == plan["model"]
        && sdk["reasoning"] == plan["reasoning"]
        && sdk["visualInputAttached"] == true
        && process["version"] == "visual-process/v1"
        && process["attemptId"] == plan["attemptId"]
        && process["manifestSha256"] == sha
        && process["maximumModelCalls"] == 1
        && process["tree"]["processTreeStopProven"] == true
        && process["tree"]["activeProcesses"] == 0
        && process["tree"]["totalProcesses"]
            .as_u64()
            .is_some_and(|n| n > 0)
        && dispatch["version"] == "visual-dispatch/v1"
        && dispatch["attemptId"] == plan["attemptId"]
        && dispatch["manifestSha256"] == sha
        && dispatch["maximumModelCalls"] == 1
        && runtime["attemptId"] == plan["attemptId"]
        && runtime["manifestSha256"] == sha
        && runtime["maximumModelCalls"] == 1
}

fn evidence(store: &Store, pointer: &Value) -> Result<Value> {
    let path = recovery::pointer_path(store, pointer)?;
    let sha = s(pointer, "manifestSha256");
    let plan = native::verify_historical_manifest(&path.join("manifest.json"), sha)?;
    let sdk = read(&path.join("sdk-receipt.json"))?;
    let process = read(&path.join("process-receipt.json"))?;
    let dispatch = read(&path.join("dispatch-intent.json"))?;
    let runtime = read(&path.join("runtime-dispatch-intent.json"))?;
    let bytes = fs::read(path.join("events.ndjson")).map_err(|e| e.to_string())?;
    let hashes: Vec<_> = plan["images"]
        .as_array()
        .ok_or("VISUAL_IMAGE_COUNT")?
        .iter()
        .map(|v| v["sha256"].clone())
        .collect();
    if !bound_stopped(&plan, &sdk, &process, &dispatch, &runtime, sha)
        || plan["images"].as_array().map(Vec::len) != Some(2)
        || sdk["inputHashes"] != json!(hashes)
        || process["sdkReceiptSha256"] != hash_file(&path.join("sdk-receipt.json"))?.0
        || process["eventsSha256"] != hash_file(&path.join("events.ndjson"))?.0
        || !interruption::reason(&plan, &sdk, sha, &bytes)?.contains("启动提示误判")
    {
        return Err("VISUAL_RESTART_NOT_ELIGIBLE: 原进程未证明停止，或不属于已修复的启动提示误判；保留原 UNKNOWN".into());
    }
    let runtime_receipt = path.join("runtime-process-receipt.json");
    if runtime_receipt.exists() {
        let receipt = read(&runtime_receipt)?;
        if process["runtimeReceiptSha256"] != hash_file(&runtime_receipt)?.0
            || receipt["attemptId"] != plan["attemptId"]
            || receipt["manifestSha256"] != sha
            || receipt["tree"]["processTreeStopProven"] != true
            || receipt["tree"]["activeProcesses"] != 0
        {
            return Err("VISUAL_RESTART_RUNTIME_EVIDENCE_CHANGED".into());
        }
    } else if !process["runtimeReceiptSha256"].is_null() {
        return Err("VISUAL_RESTART_RUNTIME_EVIDENCE_MISSING".into());
    }
    let mut artifacts = serde_json::Map::new();
    for name in [
        "manifest.json",
        "sdk-receipt.json",
        "events.ndjson",
        "process-receipt.json",
        "dispatch-intent.json",
        "runtime-dispatch-intent.json",
        "expected.private.json",
        "runner.mjs",
        "pointer.json",
    ] {
        artifacts.insert(name.into(), json!(hash_file(&path.join(name))?.0));
    }
    artifacts.insert(
        "runtime-process-receipt.json".into(),
        if runtime_receipt.exists() {
            json!(hash_file(&runtime_receipt)?.0)
        } else {
            Value::Null
        },
    );
    Ok(
        json!({"version":"visual-explicit-restart-evidence/v1","originalStatus":"submission_unknown","processTreeStopProven":true,"artifacts":artifacts,"inputHashes":hashes,"originalAttemptId":pointer["attemptId"],"originalManifestSha256":sha}),
    )
}

fn create_new_json(file: &Path, value: &Value) -> Result<()> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .map_err(|_| {
            "VISUAL_RESTART_ALREADY_RESERVED: 原链已有一次显式验证预约，读取已有预约，不再新增"
        })?;
    output
        .write_all(
            serde_json::to_string_pretty(value)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())
}

fn ordinary(path: &Path, directory: bool) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("VISUAL_RESTART_PATH_LINK".into());
        }
    }
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err("VISUAL_RESTART_PATH_LINK".into());
    }
    Ok(())
}

fn reserved_plan(
    store: &Store,
    pointer: &Value,
    current: &Value,
    evidence: &Value,
    attempt_id: &str,
    created_at: &Value,
    runner_sha: &Value,
) -> Result<(PathBuf, Value, Value)> {
    let old = read(&recovery::pointer_path(store, pointer)?.join("manifest.json"))?;
    if !compatible_identity(&old["baseIdentity"], current) {
        return Err("VISUAL_RESTART_ACCOUNT_OR_RUNTIME_CHANGED: 新验证不可更换账户、模型、SDK、Node 或上游运行器".into());
    }
    let path = root(store).join(format!("probe-explicit-{}", s(pointer, "manifestSha256")));
    let target_scope = hash(
        &json!({"identity":current,"runnerSha256":runner_sha,"policy":POLICY,"model":"gpt-6.1-sol","reasoning":"medium"}),
    );
    let mut plan = old.clone();
    plan["attemptId"] = json!(attempt_id);
    plan["createdAt"] = created_at.clone();
    plan["baseIdentity"] = current.clone();
    plan["runnerSha256"] = runner_sha.clone();
    plan["basis"] = json!({"scopeSha256":target_scope,"supersedesUnknown":{"attemptId":pointer["attemptId"],"manifestSha256":pointer["manifestSha256"],"evidenceSha256":hash(evidence),"originalStatus":"submission_unknown","authority":AUTHORITY,"acknowledgePossiblePriorUsage":true}});
    for image in plan["images"].as_array_mut().ok_or("VISUAL_IMAGE_COUNT")? {
        let original = PathBuf::from(s(image, "path"));
        image["path"] = json!(crate::text_sdk::node_path_argument(
            &path
                .join("images")
                .join(original.file_name().ok_or("VISUAL_IMAGE_PATH")?)
        )?);
    }
    let next = json!({"version":"visual-attempt-pointer/v1","purpose":"capability","attemptId":attempt_id,"attemptDirectory":path,
        "manifestSha256":digest(&serde_json::to_string_pretty(&plan).map_err(|e|e.to_string())?),"mediaId":null,"assetId":null,"scopeSha256":target_scope,"createdAt":created_at,"previous":pointer});
    Ok((path, plan, next))
}

fn validate_claim(
    store: &Store,
    pointer: &Value,
    claim: &Value,
    actual_evidence: &Value,
) -> Result<Value> {
    let (_, plan, next) = reserved_plan(
        store,
        pointer,
        &claim["currentIdentity"],
        actual_evidence,
        s(claim, "newAttemptId"),
        &claim["authorizedAt"],
        &claim["plan"]["runnerSha256"],
    )?;
    if claim["version"] != "visual-explicit-capability-restart/v1"
        || claim["authority"] != AUTHORITY
        || claim["acknowledgePossiblePriorUsage"] != true
        || claim["maximumNewCalls"] != 1
        || claim["sourcePointer"] != *pointer
        || claim["evidence"] != *actual_evidence
        || claim["plan"] != plan
        || claim["successor"] != next
        || s(claim, "newAttemptId").is_empty()
    {
        return Err("VISUAL_RESTART_AUTHORIZATION_CHANGED".into());
    }
    Ok(next)
}

fn reserve(
    store: &Store,
    pointer: &Value,
    current: &Value,
    actual_evidence: &Value,
) -> Result<Value> {
    let file = chain_root(store, pointer)?.join(CLAIM);
    if file.exists() {
        let claim = read(&file)?;
        validate_claim(store, pointer, &claim, actual_evidence)?;
        if claim["currentIdentity"] != *current {
            return Err("VISUAL_RESTART_FROZEN_COMPONENTS_CHANGED".into());
        }
        return Ok(claim);
    }
    let created = json!(now());
    let attempt = id();
    let (_, plan, next) = reserved_plan(
        store,
        pointer,
        current,
        actual_evidence,
        &attempt,
        &created,
        &json!(digest(RUNNER)),
    )?;
    let claim = json!({"version":"visual-explicit-capability-restart/v1","authority":AUTHORITY,"acknowledgePossiblePriorUsage":true,
        "maximumNewCalls":1,"authorizedAt":created,"newAttemptId":attempt,"sourcePointer":pointer,"evidence":actual_evidence,"currentIdentity":current,"plan":plan,"successor":next});
    create_new_json(&file, &claim)?;
    Ok(claim)
}

fn materialize(store: &Store, claim: &Value) -> Result<PathBuf> {
    let original = recovery::pointer_path(store, &claim["sourcePointer"])?;
    let next = &claim["successor"];
    let path = PathBuf::from(s(next, "attemptDirectory"));
    let dispatched = path.join("dispatch-intent.json").exists();
    if !path.exists() {
        allocate(
            store,
            path.file_name()
                .and_then(|v| v.to_str())
                .ok_or("VISUAL_RESTART_PATH")?,
        )?;
    }
    ordinary(&path, true)?;
    for name in ["images", "work", "tmp"] {
        let directory = path.join(name);
        if !directory.exists() {
            if dispatched {
                return Err("VISUAL_RESTART_FROZEN_FILE_MISSING".into());
            }
            fs::create_dir(&directory).map_err(|e| e.to_string())?;
        }
        ordinary(&directory, true)?;
    }
    // A claim is durable before any work. Repeated clicks only complete the
    // same frozen files; an existing different file is never overwritten.
    let freeze = |file: &Path, bytes: &[u8]| -> Result<()> {
        if file.exists() {
            ordinary(file, false)?;
            if fs::metadata(file).map_err(|e| e.to_string())?.len() != bytes.len() as u64
                || fs::read(file).map_err(|e| e.to_string())? != bytes
            {
                return Err("VISUAL_RESTART_FROZEN_FILE_CHANGED".into());
            }
            return Ok(());
        }
        if dispatched {
            return Err("VISUAL_RESTART_FROZEN_FILE_MISSING".into());
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(file)
            .map_err(|e| e.to_string())?;
        output.write_all(bytes).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())
    };
    freeze(
        &path.join("work/.director-text-root"),
        b"isolated visual review\n",
    )?;
    for image in claim["plan"]["images"]
        .as_array()
        .ok_or("VISUAL_IMAGE_COUNT")?
    {
        let target = PathBuf::from(s(image, "path"));
        let file = target.file_name().ok_or("VISUAL_IMAGE_PATH")?;
        freeze(
            &target,
            &fs::read(original.join("images").join(file)).map_err(|e| e.to_string())?,
        )?;
    }
    freeze(
        &path.join("expected.private.json"),
        &fs::read(original.join("expected.private.json")).map_err(|e| e.to_string())?,
    )?;
    freeze(&path.join("runner.mjs"), RUNNER.as_bytes())?;
    freeze(
        &path.join("manifest.json"),
        serde_json::to_string_pretty(&claim["plan"])
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )?;
    freeze(
        &path.join("pointer.json"),
        serde_json::to_string_pretty(next)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )?;
    recovery::pointer_path(store, next)?;
    store.dirty()?;
    Ok(path)
}

pub(super) fn successor(store: &Store, pointer: &Value) -> Result<Option<Value>> {
    let file = chain_root(store, pointer)?.join(CLAIM);
    if !file.exists() {
        return Ok(None);
    }
    let claim = read(&file)?;
    if claim["sourcePointer"] != *pointer {
        return Ok(None);
    }
    let proof = evidence(store, pointer)?;
    let next = validate_claim(store, pointer, &claim, &proof)?;
    // Crash during preparation is resumed only by the same explicit action.
    let path = PathBuf::from(s(&next, "attemptDirectory"));
    if !path.join("pointer.json").exists() {
        return Ok(None);
    }
    recovery::pointer_path(store, &next)?;
    if read(&path.join("pointer.json"))? != next {
        return Err("VISUAL_RESTART_POINTER_CHANGED".into());
    }
    Ok(Some(next))
}

pub(crate) fn projection(store: &Store) -> Result<Value> {
    let Some(pointer) = recovery::pointer(store, None)? else {
        return Ok(Value::Null);
    };
    let path = recovery::pointer_path(store, &pointer)?;
    let plan = read(&path.join("manifest.json"))?;
    let sdk = read(&path.join("sdk-receipt.json")).unwrap_or(Value::Null);
    let claim_path = chain_root(store, &pointer)?.join(CLAIM);
    if claim_path.exists() {
        let claim = read(&claim_path)?;
        let successor = PathBuf::from(s(&claim["successor"], "attemptDirectory"));
        let pending = !successor.join("dispatch-intent.json").exists()
            && (claim["sourcePointer"] == pointer || claim["successor"] == pointer);
        return Ok(
            json!({"available":pending,"reserved":true,"expectedAttemptId":claim["sourcePointer"]["attemptId"],"expectedManifestSha256":claim["sourcePointer"]["manifestSha256"],"requiresAcknowledgement":true,"maximumNewCalls":1}),
        );
    }
    let eligible = plan["runnerSha256"] == LEGACY_DIAGNOSTIC_RUNNER_SHA
        && sdk["state"] == "submission_unknown"
        && sdk["error"] == "VISUAL_TOOL_BLOCKED";
    Ok(
        json!({"available":eligible,"expectedAttemptId":pointer["attemptId"],"expectedManifestSha256":pointer["manifestSha256"],"requiresAcknowledgement":true,"maximumNewCalls":1}),
    )
}

pub async fn restart_capability(store: &Store, settings: &Value, request: &Value) -> Result<Value> {
    let latest = recovery::pointer(store, None)?.ok_or("VISUAL_RESTART_ORIGINAL_REQUIRED")?;
    let pointer = if latest["attemptId"] == request["expectedAttemptId"] {
        latest.clone()
    } else if latest["previous"]["attemptId"] == request["expectedAttemptId"] {
        latest["previous"].clone()
    } else {
        return Err("VISUAL_RESTART_ORIGINAL_CHANGED".into());
    };
    request_binding(request, &pointer)?;
    let proof = evidence(store, &pointer)?;
    let current = identity(settings).await?;
    let claim = reserve(store, &pointer, &current, &proof)?;
    let path = materialize(store, &claim)?;
    native::verify_manifest(
        &path.join("manifest.json"),
        s(&claim["successor"], "manifestSha256"),
    )?;
    if !path.join("dispatch-intent.json").exists() {
        execute(path.clone()).await?;
    }
    if identity(settings).await? != current {
        return Err("VISUAL_ACCOUNT_CHANGED: 新验证期间账户或组件变化，未启用审核".into());
    }
    finish_capability(store, settings, &path)
}

#[cfg(test)]
#[path = "visual_review_restart_tests.rs"]
mod tests;
