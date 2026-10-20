//! Native-only capability and text-only supervisor for the genuine pinned SDK.
//! No IPC setting can register this capability; the desktop entry owns it.
use super::*;
use sha2::Digest;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::OnceLock,
    time::Instant,
};

#[derive(Clone)]
struct NativeContext {
    resources: PathBuf,
    supervisor: PathBuf,
    supervisor_sha: String,
}
static NATIVE: OnceLock<NativeContext> = OnceLock::new();
pub fn native_execution_enabled() -> bool {
    cfg!(windows) && NATIVE.get().is_some()
}
pub fn enable_native_execution(resource_root: PathBuf) -> Result<()> {
    if !cfg!(windows) {
        return Err("SDK_NATIVE_PLATFORM_UNVERIFIED: 本轮只开放 Windows 原生 SDK".into());
    }
    if crate::windows_runtime::is_packaged()? {
        return Err("SDK_PACKAGED_CONTEXT_REJECTED".into());
    }
    crate::windows_runtime::verify_shared_directory(&resource_root)?;
    let resources = std::fs::canonicalize(resource_root).map_err(|e| e.to_string())?;
    let supervisor = node_path_argument(&std::env::current_exe().map_err(|e| e.to_string())?)?;
    let supervisor_sha = crate::store::hash_file(&supervisor)?.0;
    NATIVE
        .set(NativeContext {
            resources,
            supervisor,
            supervisor_sha,
        })
        .map_err(|_| "SDK_NATIVE_CONTEXT_ALREADY_SET".into())
}
fn context() -> Result<&'static NativeContext> {
    NATIVE.get().filter(|_| cfg!(windows)).ok_or_else(||
        "SDK_LIVE_DISABLED: 仅标准 Windows 原生桌面可显式选择 SDK；离线/headless 不执行，不回退 CLI".into())
}
fn valid_sha(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value).is_absolute()
        && !value.chars().any(char::is_control)
        && !Path::new(value)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}
pub(super) fn validate_identity(identity: &SdkIdentity) -> Result<()> {
    let paths = [
        &identity.runtime_executable,
        &identity.supervisor_executable,
        &identity.resource_directory,
        &identity.credential_home,
        &identity.codex_home,
    ];
    if identity.permission_profile != NATIVE_POLICY
        || identity.bridge_sha256 != compiled_native_bridge_sha()
        || !identity.runtime_package_version.starts_with("codex-cli ")
        || !valid_sha(&identity.account_fingerprint)
        || !identity.runtime_sha256.as_deref().is_some_and(valid_sha)
        || !identity.supervisor_sha256.as_deref().is_some_and(valid_sha)
        || paths.iter().any(|p| !p.as_deref().is_some_and(safe_path))
        || identity.runtime_identity
            != format!(
                "official-codex-runtime/{}/{}",
                identity.runtime_package_version,
                identity.runtime_sha256.as_deref().unwrap_or("")
            )
    {
        return Err("SDK_IDENTITY_MISMATCH: 原生运行器、包、权限、账户指纹或路径无效".into());
    }
    Ok(())
}
fn credentials() -> Result<(String, String)> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or("SDK_ACCOUNT_HOME_MISSING")?;
    let codex = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let home = node_path_argument(&home)?.to_string_lossy().into_owned();
    let codex = node_path_argument(&codex)?.to_string_lossy().into_owned();
    Ok((home, codex))
}
pub(super) async fn resolve(settings: &Value) -> Result<Value> {
    let native = context()?;
    if s(settings, "textProvider") != "codex_sdk" {
        return Err("SDK_SELECTION_REQUIRED".into());
    }
    let model = s(settings, "textModel");
    let reasoning = s(settings, "reasoning");
    if !valid_model(model) || !valid_reasoning(reasoning) {
        return Err("SDK_MODEL_SELECTION_REQUIRED: SDK 必须明确选择模型与推理强度".into());
    }
    let (node, node_sha, node_version, resources) =
        crate::readiness_sdk::native_components(settings, &native.resources).await?;
    // Execution needs one fresh account/catalog inspection. Settings' separate
    // local --version fallback is useful to users but duplicates this preflight.
    let inspected = crate::text_cli::status(settings,true).await;
    crate::text_cli::validate_selection(&inspected, model, reasoning)?;
    if inspected["loggedIn"] != true || !inspected["account"].is_object() {
        return Err("SDK_ACCOUNT_NOT_VERIFIED".into());
    }
    let runtime = crate::text_cli::verified_native_path(Path::new(s(&inspected, "executable")))?;
    let runtime = node_path_argument(&runtime)?;
    let runtime_sha = crate::store::hash_file(&runtime)?.0;
    let runtime_version = s(&inspected, "version").to_owned();
    let (home, codex_home) = credentials()?;
    let frozen = SdkIdentity {
        version: "official-sdk-identity/v2".into(),
        provider: "codex_sdk".into(),
        sdk_package: "@openai/codex-sdk".into(),
        sdk_version: SDK_VERSION.into(),
        sdk_implementation_sha256: SDK_IMPLEMENTATION_SHA.into(),
        bridge_version: "0.1.0".into(),
        bridge_sha256: compiled_native_bridge_sha(),
        protocol_version: PROTOCOL.into(),
        node_executable: node_path_argument(&node)?.to_string_lossy().into(),
        node_sha256: node_sha,
        node_version,
        runtime_package_version: runtime_version.clone(),
        runtime_identity: format!("official-codex-runtime/{runtime_version}/{runtime_sha}"),
        permission_profile: NATIVE_POLICY.into(),
        account_fingerprint: digest(&inspected["account"].to_string()),
        runtime_executable: Some(runtime.to_string_lossy().into()),
        runtime_sha256: Some(runtime_sha),
        supervisor_executable: Some(native.supervisor.to_string_lossy().into()),
        supervisor_sha256: Some(native.supervisor_sha.clone()),
        resource_directory: Some(node_path_argument(&resources)?.to_string_lossy().into()),
        credential_home: Some(home),
        codex_home: Some(codex_home),
    };
    frozen.validate()?;
    let mut resolved = settings.clone();
    resolved["textSdkIdentity"] = serde_json::to_value(frozen).map_err(|e| e.to_string())?;
    Ok(resolved)
}

pub(super) fn supervisor_handoff_identity_bound(old: &Value, new: &Value) -> bool {
    let (Ok(old_identity), Ok(new_identity)) = (
        serde_json::from_value::<SdkIdentity>(old.clone()),
        serde_json::from_value::<SdkIdentity>(new.clone()),
    ) else {
        return false;
    };
    if old_identity.version != "official-sdk-identity/v2"
        || old_identity.validate().is_err()
        || new_identity.validate().is_err()
    {
        return false;
    }
    let mut allowed = old.clone();
    for field in ["supervisorExecutable", "supervisorSha256", "resourceDirectory"] {
        allowed[field] = new[field].clone();
    }
    allowed == *new && old != new
}

fn handoff_resource_hashes(root: &Path) -> Result<Value> {
    let metadata = std::fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if !root.is_absolute() || metadata.is_symlink() || !metadata.is_dir() {
        return Err("SDK_SUPERVISOR_HANDOFF_RESOURCE_CHANGED: 资源目录不能交接".into());
    }
    let mut hashes = serde_json::Map::new();
    for relative in [
        "src/health.mjs",
        "src/main.mjs",
        "src/bridge.mjs",
        "src/native-runtime.mjs",
        "THIRD_PARTY_NOTICES.md",
        "package-lock.json",
        "node_modules/@openai/codex-sdk/package.json",
        "node_modules/@openai/codex-sdk/dist/index.js",
        "node_modules/@openai/codex-sdk/LICENSE",
    ] {
        let mut path = root.to_path_buf();
        for part in Path::new(relative).components() {
            path.push(part);
            if std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?.is_symlink() {
                return Err("SDK_SUPERVISOR_HANDOFF_RESOURCE_CHANGED: 资源链接不能交接".into());
            }
        }
        let bytes = frozen_file(&path, 16_777_216)?;
        hashes.insert(relative.into(), json!(format!("{:x}", sha2::Sha256::digest(&bytes))));
    }
    Ok(Value::Object(hashes))
}

pub(super) fn verify_supervisor_handoff_bytes(old: &Value, new: &Value, hashes: &Value) -> Result<()> {
    if !supervisor_handoff_identity_bound(old, new)
        || handoff_resource_hashes(Path::new(s(old, "resourceDirectory")))? != *hashes
        || handoff_resource_hashes(Path::new(s(new, "resourceDirectory")))? != *hashes
    {
        return Err("SDK_SUPERVISOR_HANDOFF_BYTES_CHANGED: 旧新资源或冻结身份不同".into());
    }
    for (path, sha) in [
        ("nodeExecutable", "nodeSha256"),
        ("runtimeExecutable", "runtimeSha256"),
        ("supervisorExecutable", "supervisorSha256"),
    ] {
        if crate::store::hash_file(Path::new(s(new, path)))?.0 != s(new, sha) {
            return Err("SDK_SUPERVISOR_HANDOFF_BYTES_CHANGED: 核验期间程序字节改变".into());
        }
    }
    Ok(())
}

pub(super) async fn prepare_supervisor_handoff(
    settings: &Value,
) -> Result<Option<SupervisorHandoff>> {
    let native = context()?;
    let old = identity(settings)?;
    let current_resources = if native.resources.join("sdk-readiness").is_dir() {
        native.resources.join("sdk-readiness")
    } else {
        native.resources.clone()
    };
    let current_resources = node_path_argument(&std::fs::canonicalize(current_resources).map_err(|e| e.to_string())?)?;
    // Never weaken the frozen guard on an existing dispatch. The executor calls
    // this only after proving that its later logical node has no task or attempt.
    if s(&old, "supervisorExecutable") == native.supervisor.to_string_lossy()
        && s(&old, "supervisorSha256") == native.supervisor_sha
        && s(&old, "resourceDirectory") == current_resources.to_string_lossy()
    {
        return Ok(None);
    }
    let resolved = resolve(settings).await?; // account/model probes, zero model calls
    let new = identity(&resolved)?;
    if !supervisor_handoff_identity_bound(&old, &new) {
        return Err("SDK_SUPERVISOR_HANDOFF_IDENTITY_CHANGED: SDK/桥/Node/运行器/账户/权限身份变化；保留原流程，不交接".into());
    }
    let old_resources = handoff_resource_hashes(Path::new(s(&old, "resourceDirectory")))?;
    let new_resources = handoff_resource_hashes(Path::new(s(&new, "resourceDirectory")))?;
    if old_resources != new_resources {
        return Err("SDK_SUPERVISOR_HANDOFF_RESOURCE_CHANGED: 旧新资源正文不同，不交接".into());
    }
    verify_supervisor_handoff_bytes(&old, &new, &new_resources)?;
    Ok(Some(SupervisorHandoff {
        identity: new,
        resource_hashes: new_resources,
    }))
}
pub(super) async fn prepare_local_recovery_handoff(settings: &Value) -> Result<SupervisorHandoff> {
    let old = receipt_identity(settings)?;
    let resolved = resolve(settings).await?;
    let new = identity(&resolved)?;
    if !local_recovery_identity_bound(&old, &new) { return Err("SDK_LOCAL_RECOVERY_IDENTITY_CHANGED".into()); }
    let resource_hashes = handoff_resource_hashes(Path::new(s(&new, "resourceDirectory")))?;
    verify_local_recovery_bytes(&old, &new, &resource_hashes)?;
    Ok(SupervisorHandoff { identity: new, resource_hashes })
}
pub(super) fn verify_local_recovery_bytes(old: &Value, new: &Value, hashes: &Value) -> Result<()> {
    serde_json::from_value::<SdkIdentity>(new.clone()).map_err(|e|e.to_string())?.validate()?;
    if !local_recovery_identity_bound(old, new)
        || handoff_resource_hashes(Path::new(s(new,"resourceDirectory")))? != *hashes { return Err("SDK_LOCAL_RECOVERY_IDENTITY_CHANGED".into()); }
    for (field, sha) in [("nodeExecutable","nodeSha256"),("runtimeExecutable","runtimeSha256"),("supervisorExecutable","supervisorSha256")] {
        if crate::store::hash_file(Path::new(s(new,field)))?.0 != s(new,sha) { return Err("SDK_LOCAL_RECOVERY_BYTES_CHANGED".into()); }
    }
    let root = Path::new(s(new,"resourceDirectory"));
    let bridge = digest(&format!("{}{}{}", std::fs::read_to_string(root.join("src/main.mjs")).map_err(|e|e.to_string())?,std::fs::read_to_string(root.join("src/bridge.mjs")).map_err(|e|e.to_string())?,std::fs::read_to_string(root.join("src/native-runtime.mjs")).map_err(|e|e.to_string())?));
    if bridge != compiled_native_bridge_sha() || hashes["node_modules/@openai/codex-sdk/dist/index.js"] != SDK_IMPLEMENTATION_SHA { return Err("SDK_LOCAL_RECOVERY_PACKAGE_CHANGED".into()); }
    Ok(())
}
pub(super) async fn prepare_schema_recovery_handoff(settings: &Value) -> Result<SupervisorHandoff> {
    let old=identity(settings)?;let resolved=resolve(settings).await?;let new=identity(&resolved)?;
    let hashes=handoff_resource_hashes(Path::new(s(&new,"resourceDirectory")))?;
    verify_schema_recovery_bytes(&old,&new,&hashes)?;
    Ok(SupervisorHandoff{identity:new,resource_hashes:hashes})
}
pub(super) fn verify_schema_recovery_bytes(old: &Value, new: &Value, hashes: &Value) -> Result<()> {
    if !schema_recovery_identity_bound(old,new) || handoff_resource_hashes(Path::new(s(old,"resourceDirectory")))?!=*hashes
        || handoff_resource_hashes(Path::new(s(new,"resourceDirectory")))?!=*hashes {return Err("SDK_SCHEMA_RECOVERY_IDENTITY_CHANGED".into());}
    for identity in [old,new] {
        for (field,sha) in [("nodeExecutable","nodeSha256"),("runtimeExecutable","runtimeSha256"),("supervisorExecutable","supervisorSha256")] {
            if crate::store::hash_file(Path::new(s(identity,field)))?.0!=s(identity,sha) {return Err("SDK_SCHEMA_RECOVERY_BYTES_CHANGED".into());}
        }
    }Ok(())
}
pub(super) async fn add_readiness(settings: &Value, result: &mut Value) {
    let inspected = crate::text_cli::readiness_status(settings).await;
    let cli = crate::readiness::codex_report(&inspected, settings);
    result["authentication"] = cli["authentication"].clone();
    result["authentication"]["identityVerified"] = json!(
        inspected["ready"] == true
            && inspected["loggedIn"] == true
            && inspected["account"].is_object()
    );
    result["catalog"] = cli["catalog"].clone();
    result["details"]["officialRuntimeChecked"] = json!(inspected["readinessRpcReady"] == true);
    result["details"]["accountChecked"] = json!(inspected["readinessRpcReady"] == true);
    result["details"]["runtimeExecutable"] = cli["installation"]["executable"].clone();
    result["details"]["runtimeVersion"] = cli["installation"]["version"].clone();
    result["details"]["cliStatus"] = cli["details"]["cliStatus"].clone();
    result["details"]["accountIdentityNotExported"] = json!(true);
    result["generation"]["enabled"] = json!(
        result["authentication"]["identityVerified"] == true
            && result["catalog"]["state"] == "available"
    );
    result["messages"] = json!([
        "官方 SDK 与 Node/桥已核验，复用既有官方 Codex 账户与只读模型目录；检查调用模型次数为 0。",
        "显式选择 SDK 并启动文字工作流时再次冻结包/运行器/模型身份；取消或未知提交不会自动重发。"
    ]);
    if let Some(error) = inspected["error"].as_str().filter(|e| !e.is_empty()) {
        result["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!(error.chars().take(512).collect::<String>()));
    }
}
pub(super) fn config(settings: &Value, root: &Path) -> Result<Config> {
    context()?;
    let identity = identity(settings)?;
    if identity["version"] != "official-sdk-identity/v2"
        || s(settings, "textProvider") != "codex_sdk"
    {
        return Err("SDK_LIVE_DISABLED".into());
    }
    Ok(Config {
        node: PathBuf::from(s(&identity, "nodeExecutable")),
        bridge: PathBuf::from(s(&identity, "resourceDirectory")).join("src/main.mjs"),
        state: root.join("sdk-worker"),
        fixture: None,
        deadline: Duration::from_secs(900),
        identity,
    })
}
pub(super) fn verify_config(config: &Config) -> Result<()> {
    let native = context()?;
    let frozen: SdkIdentity =
        serde_json::from_value(config.identity.clone()).map_err(|e| e.to_string())?;
    frozen.validate()?;
    if frozen.supervisor_executable.as_deref() != Some(native.supervisor.to_string_lossy().as_ref())
        || frozen.supervisor_sha256.as_deref() != Some(native.supervisor_sha.as_str())
        || crate::store::hash_file(Path::new(frozen.runtime_executable.as_deref().unwrap()))?.0
            != frozen.runtime_sha256.as_deref().unwrap()
        || crate::store::hash_file(&native.supervisor)?.0 != native.supervisor_sha
    {
        return Err("SDK_NATIVE_RUNTIME_CHANGED: 冻结运行器改变；保留原请求，不回退或重发".into());
    }
    Ok(())
}
fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 128
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
}
fn valid_reasoning(reasoning: &str) -> bool {
    [
        "minimal",
        "low",
        "medium",
        "high",
        "xhigh",
        "max",
        "ultra",
        "persistent",
    ]
    .contains(&reasoning)
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimePlan {
    version: String,
    attempt_id: String,
    identity: SdkIdentity,
    model: String,
    reasoning: String,
    compiled_prompt_sha256: String,
    compiled_schema_sha256: String,
    runtime_schema_sha256: String,
    semantic_request_hash: String,
    execution_fingerprint: String,
    working_directory: PathBuf,
    temp_directory: PathBuf,
}
fn frozen_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > limit {
        return Err("SDK_WRAPPER_FILE_REJECTED".into());
    }
    std::fs::read(path).map_err(|e| e.to_string())
}
fn canonical(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).map_err(|e| e.to_string())
}
fn validate_argv(args: &[String], plan: &RuntimePlan) -> Result<PathBuf> {
    if args.first().map(String::as_str) != Some("exec") {
        return Err("SDK_WRAPPER_EXEC_ONLY".into());
    }
    let mut flags = BTreeMap::new();
    let mut configs = Vec::new();
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
        ]
        .contains(&flag.as_str())
        {
            i += 1;
            args.get(i).ok_or("SDK_WRAPPER_ARGUMENT_MISSING")?.clone()
        } else {
            return Err("SDK_WRAPPER_ARGUMENT_BLOCKED".into());
        };
        if flag == "--config" {
            configs.push(value);
        } else if flags.insert(flag.clone(), value).is_some() {
            return Err("SDK_WRAPPER_DUPLICATE_ARGUMENT".into());
        }
        i += 1;
    }
    let expected = vec![
        format!("model_reasoning_effort=\"{}\"", plan.reasoning),
        "sandbox_workspace_write.network_access=false".into(),
        "web_search=\"disabled\"".into(),
        "approval_policy=\"never\"".into(),
    ];
    configs.sort();
    let mut expected = expected;
    expected.sort();
    if flags.len() != 6
        || !flags.contains_key("--experimental-json")
        || !flags.contains_key("--skip-git-repo-check")
        || flags.get("--model") != Some(&plan.model)
        || flags.get("--sandbox").map(String::as_str) != Some("read-only")
        || flags
            .get("--cd")
            .map(PathBuf::from)
            .map(|p| canonical(&p))
            .transpose()?
            .as_ref()
            != Some(&canonical(&plan.working_directory)?)
        || configs != expected
    {
        return Err("SDK_WRAPPER_FROZEN_ARGUMENTS_MISMATCH".into());
    }
    let schema = canonical(Path::new(
        flags
            .get("--output-schema")
            .ok_or("SDK_WRAPPER_SCHEMA_MISSING")?,
    ))?;
    if !schema.starts_with(canonical(&plan.temp_directory)?)
        || crate::store::hash_file(&schema)?.0 != plan.runtime_schema_sha256
    {
        return Err("SDK_WRAPPER_SCHEMA_IDENTITY".into());
    }
    Ok(schema)
}
/// Invoked before any GUI, project registry or runtime worker is created.
/// The child SDK wrapper has no native product capability of its own.
pub fn run_runtime_wrapper() -> Result<bool> {
    let Some(file) = std::env::var_os("DIRECTOR_SDK_RUNTIME_PLAN") else {
        return Ok(false);
    };
    let code = run_runtime_wrapper_plan(
        Path::new(&file),
        &std::env::var("DIRECTOR_SDK_RUNTIME_PLAN_SHA256").unwrap_or_default(),
        &std::env::args().skip(1).collect::<Vec<_>>(),
        std::io::stdin(),
        run_supervised,
    )?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(true)
}
// Tests use this same production validation path and replace only the final
// process launch with a counter. No test switch can enter a production plan.
fn run_runtime_wrapper_plan(
    file: &Path,
    expected_plan_sha256: &str,
    args: &[String],
    input: impl Read,
    launch: impl FnOnce(&RuntimePlan, &Path, Vec<u8>, &Path) -> Result<i32>,
) -> Result<i32> {
    if !cfg!(windows) || crate::windows_runtime::is_packaged()? {
        return Err("SDK_WRAPPER_NATIVE_CONTEXT_REQUIRED".into());
    }
    if !file.is_absolute() {
        return Err("SDK_WRAPPER_PLAN_PATH".into());
    }
    crate::windows_runtime::verify_shared_directory(file.parent().ok_or("SDK_WRAPPER_PLAN_PATH")?)?;
    let bytes = frozen_file(&file, FRAME_LIMIT as u64)?;
    if digest(std::str::from_utf8(&bytes).map_err(|_| "SDK_WRAPPER_PLAN_UTF8")?)
        != expected_plan_sha256
    {
        return Err("SDK_WRAPPER_PLAN_HASH".into());
    }
    let plan: RuntimePlan =
        serde_json::from_slice(&bytes).map_err(|_| "SDK_WRAPPER_PLAN_INVALID")?;
    plan.identity.validate()?;
    if plan.version != "sdk-native-runtime/v1"
        || !valid_model(&plan.model)
        || !valid_reasoning(&plan.reasoning)
        || ![
            &plan.compiled_prompt_sha256,
            &plan.compiled_schema_sha256,
            &plan.runtime_schema_sha256,
            &plan.semantic_request_hash,
            &plan.execution_fingerprint,
        ]
        .iter()
        .all(|s| valid_sha(s))
        || plan.attempt_id.is_empty()
        || !plan
            .attempt_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("SDK_WRAPPER_PLAN_IDENTITY".into());
    }
    let root = canonical(file.parent().unwrap())?;
    if file.file_name().and_then(|s| s.to_str()) != Some("runtime-plan.json")
        || ![Some("native-runtime"), Some("native-runtime-wrapper-repair"),Some(schema_recovery::SCHEMA_RECOVERY_ROOT)].contains(&root.file_name().and_then(|s| s.to_str()))
        || root
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            != Some(plan.attempt_id.as_str())
        || root
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            != Some("sdk-worker")
    {
        return Err("SDK_WRAPPER_ATTEMPT_DIRECTORY".into());
    }
    let record: Value = serde_json::from_slice(&frozen_file(
        &root.parent().unwrap().join("receipt.json"),
        4_194_304,
    )?)
    .map_err(|_| "SDK_WRAPPER_RECEIPT_INVALID")?;
    if record.get("wrapperPreparationRepair").is_some() != (root.file_name().and_then(|s|s.to_str()) == Some("native-runtime-wrapper-repair"))
        || record.get("schemaPreparationRecovery").is_some() != (root.file_name().and_then(|s|s.to_str()) == Some(schema_recovery::SCHEMA_RECOVERY_ROOT))
        || record["state"] != "dispatching"
        || record["attempt"]["attemptId"] != plan.attempt_id
        || !wrapper_runtime_identity_bound(&record, &plan, root.parent().unwrap())?
        || record["attempt"]["request"]["model"] != plan.model
        || record["attempt"]["request"]["reasoning"] != plan.reasoning
        || record["attempt"]["semanticRequestHash"] != plan.semantic_request_hash
        || record["attempt"]["executionFingerprint"] != plan.execution_fingerprint
        || record["compiledPromptSha256"] != plan.compiled_prompt_sha256
        || record["compiledSchemaSha256"] != plan.compiled_schema_sha256
    {
        return Err("SDK_WRAPPER_FROZEN_ATTEMPT_MISMATCH".into());
    }
    if canonical(&plan.working_directory)?.parent() != Some(root.as_path())
        || canonical(&plan.temp_directory)?.parent() != Some(root.as_path())
    {
        return Err("SDK_WRAPPER_WORKSPACE_IDENTITY".into());
    }
    let supervisor = std::env::current_exe().map_err(|e| e.to_string())?;
    if canonical(&supervisor)?
        != canonical(Path::new(
            plan.identity.supervisor_executable.as_deref().unwrap(),
        ))?
        || crate::store::hash_file(&supervisor)?.0
            != plan.identity.supervisor_sha256.as_deref().unwrap()
        || crate::store::hash_file(Path::new(
            plan.identity.runtime_executable.as_deref().unwrap(),
        ))?
        .0 != plan.identity.runtime_sha256.as_deref().unwrap()
    {
        return Err("SDK_WRAPPER_EXECUTABLE_IDENTITY".into());
    }
    let schema = validate_argv(args, &plan)?;
    let mut prompt = Vec::new();
    input
        .take(FRAME_LIMIT as u64 + 1)
        .read_to_end(&mut prompt)
        .map_err(|e| e.to_string())?;
    if prompt.len() > FRAME_LIMIT
        || digest(std::str::from_utf8(&prompt).map_err(|_| "SDK_WRAPPER_PROMPT_UTF8")?)
            != plan.compiled_prompt_sha256
    {
        return Err("SDK_WRAPPER_PROMPT_IDENTITY".into());
    }
    launch(&plan, &schema, prompt, &root)
}
fn wrapper_runtime_identity_bound(record: &Value, plan: &RuntimePlan, attempt_root: &Path) -> Result<bool> {
    let effective = serde_json::to_value(&plan.identity).map_err(|e| e.to_string())?;
    if record.get("schemaPreparationRecovery").is_some() {
        return schema_recovery_record_bound(attempt_root,record,&effective);
    }
    let Some(recovery) = record.get("preDispatchRecovery") else {
        return Ok(record["attempt"]["runtimeIdentity"] == effective);
    };
    let recovery_identity = if record.get("wrapperPreparationRepair").is_some() {
        if !wrapper_repair_record_bound(attempt_root, record, &effective)? {return Ok(false);}
        recovery["effectiveIdentity"].clone()
    } else { effective };
    let proof = &recovery["proof"];
    if recovery.as_object().is_none_or(|o| o.len() != 2)
        || recovery["effectiveIdentity"] != recovery_identity
        || !local_recovery_identity_bound(&record["attempt"]["runtimeIdentity"], &recovery_identity)
        || proof["version"] != "sdk-local-preparation-recovery/v1"
        || proof["attemptId"] != plan.attempt_id
        || proof["attemptEnvelopeHash"] != record["attempt"]["attemptEnvelopeHash"]
        || proof["effectiveIdentitySha256"] != digest(&recovery_identity.to_string())
        || proof["sdkEvents"] != 0 || proof["processTreeStopProven"] != true
        || !valid_sha(s(proof, "evidenceSha256"))
    { return Ok(false); }
    let mut attempt: FrozenAttempt = serde_json::from_value(record["attempt"].clone()).map_err(|_| "SDK_WRAPPER_RECOVERY_ATTEMPT")?;
    attempt.begin_dispatch(Backend::CodexSdk)?;
    let project = attempt_root.parent().and_then(Path::parent).ok_or("SDK_WRAPPER_ATTEMPT_DIRECTORY")?;
    // Validate the actual sealed current record, its unchanged frozen request
    // and journal before recognizing an execution-only identity handoff.
    historical_receipt(project, &attempt)?;
    let intent: Value = serde_json::from_slice(&frozen_file(&attempt_root.join("local-preparation-recovery.json"), FRAME_LIMIT as u64)?)
        .map_err(|_| "SDK_WRAPPER_RECOVERY_INTENT")?;
    let archive = frozen_file(&attempt_root.join("local-preparation-original.json"), 4_194_304)?;
    let archive = std::str::from_utf8(&archive).map_err(|_| "SDK_WRAPPER_RECOVERY_ARCHIVE")?;
    let original: Value = serde_json::from_str(archive).map_err(|_| "SDK_WRAPPER_RECOVERY_ARCHIVE")?;
    let seal = s(&original, "receiptEnvelopeSha256");
    let suffix = format!(",\"receiptEnvelopeSha256\":\"{seal}\"}}");
    let sealed = archive.strip_suffix(&suffix).is_some_and(|body| digest(&format!("{body}}}")) == seal);
    Ok(intent == *proof && sealed && proof["originalReceiptSeal"] == seal
        && original.get("preDispatchRecovery").is_none()
        && original["state"] == "submission_unknown"
        && original["error"] == json!({"code":"SDK_STREAM_ERROR","message":"SDK_NATIVE_IDENTITY: attempt directory"})
        && original["lastSequence"] == 0 && original["threadId"].is_null()
        && original["turnId"].is_null() && original["rawResult"].is_null()
        && original["usage"].is_null() && original["rawResultSha256"].is_null()
        && original["journalSha256"] == digest("")
        && record["error"].is_null()
        && ["protocolVersion", "attempt", "frozenAttemptSha256", "requestSnapshotSha256",
            "compiledPrompt", "compiledSchema", "compiledPromptSha256", "compiledSchemaSha256",
            "journalSha256", "lastSequence", "threadId", "turnId", "rawResult", "rawResultSha256", "usage", "domainAccepted"]
            .iter().all(|key| original[key] == record[key]))
}
fn run_supervised(plan: &RuntimePlan, schema: &Path, prompt: Vec<u8>, root: &Path) -> Result<i32> {
    let mut command =
        std::process::Command::new(plan.identity.runtime_executable.as_deref().unwrap());
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
        .arg(&plan.working_directory)
        .arg("--output-schema")
        .arg(schema)
        .arg("--model")
        .arg(&plan.model);
    for config in crate::text_executor::CLI_CONFIG {
        command.arg("--config").arg(config);
    }
    for config in [
        format!("model_reasoning_effort=\"{}\"", plan.reasoning),
        "model_provider=\"openai\"".into(),
        "chatgpt_base_url=\"https://chatgpt.com/backend-api\"".into(),
    ] {
        command.arg("--config").arg(config);
    }
    command
        .env_clear()
        .env(
            "USERPROFILE",
            plan.identity.credential_home.as_deref().unwrap(),
        )
        .env("HOME", plan.identity.credential_home.as_deref().unwrap())
        .env("CODEX_HOME", plan.identity.codex_home.as_deref().unwrap())
        .env("TEMP", &plan.temp_directory)
        .env("TMP", &plan.temp_directory)
        .env("TMPDIR", &plan.temp_directory)
        .current_dir(&plan.working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004);
        let system = node_system_root()?;
        command.env("SystemRoot", &system).env("WINDIR", &system);
    }
    let mut child = command.spawn().map_err(|_| "SDK_RUNTIME_START_FAILED")?;
    let job = match ProcessJob::attach_std(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    if let Err(error) = job.start() {
        let _ = child.wait();
        return Err(error);
    }
    let stdout = child.stdout.take().ok_or("SDK_RUNTIME_STDOUT")?;
    let stderr = child.stderr.take().ok_or("SDK_RUNTIME_STDERR")?;
    let stdin = child.stdin.take().ok_or("SDK_RUNTIME_STDIN")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    let input = std::thread::spawn(move || {
        let mut stdin = stdin;
        stdin
            .write_all(&prompt)
            .map_err(|_| "SDK_RUNTIME_INPUT_FAILED".to_string())
    });
    let output_sender = sender.clone();
    let output = std::thread::spawn(move || {
        let result = relay(stdout, std::io::stdout(), JOURNAL_LIMIT, FRAME_LIMIT);
        let _ = output_sender.send(result.as_ref().err().cloned());
        result
    });
    let errors = std::thread::spawn(move || {
        let result = relay(stderr, std::io::stderr(), STDERR_LIMIT, STDERR_LIMIT);
        let _ = sender.send(result.as_ref().err().cloned());
        result
    });
    let started = Instant::now();
    let mut failure = None;
    let status = loop {
        if let Ok(Some(error)) = receiver.try_recv() {
            failure = Some(error);
            break None;
        }
        if started.elapsed() > Duration::from_secs(890) {
            failure = Some("SDK_RUNTIME_DEADLINE".into());
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                failure = Some("SDK_RUNTIME_WAIT_FAILED".into());
                break None;
            }
        }
    };
    let proof = job.stop()?;
    let _ = child.wait();
    for result in [
        input
            .join()
            .map_err(|_| "SDK_RUNTIME_INPUT_PANIC".to_string())
            .and_then(|r| r),
        output
            .join()
            .map_err(|_| "SDK_RUNTIME_OUTPUT_PANIC".to_string())
            .and_then(|r| r),
        errors
            .join()
            .map_err(|_| "SDK_RUNTIME_ERROR_PANIC".to_string())
            .and_then(|r| r),
    ] {
        if failure.is_none() {
            failure = result.err();
        }
    }
    crate::store::atomic_json(
        &root.join("process-receipt.json"),
        &json!({"version":"sdk-native-process/v1","attemptId":plan.attempt_id,"runtimeSha256":plan.identity.runtime_sha256,"tree":proof,"failure":failure,"exitCode":status.and_then(|s|s.code()),"modelCallsMax":1}),
    )?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(status.and_then(|s| s.code()).unwrap_or(1))
}
fn relay(
    mut reader: impl Read,
    mut writer: impl Write,
    total_limit: usize,
    line_limit: usize,
) -> Result<()> {
    let mut chunk = [0; 8192];
    let mut total = 0;
    let mut line = 0;
    loop {
        let count = reader
            .read(&mut chunk)
            .map_err(|_| "SDK_RUNTIME_PIPE_FAILED")?;
        if count == 0 {
            return Ok(());
        }
        total += count;
        if total > total_limit {
            return Err("SDK_RUNTIME_STREAM_LIMIT".into());
        }
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                line = 0
            } else {
                line += 1;
                if line > line_limit {
                    return Err("SDK_RUNTIME_LINE_LIMIT".into());
                }
            }
        }
        writer
            .write_all(&chunk[..count])
            .map_err(|_| "SDK_RUNTIME_OUTPUT_CLOSED")?;
        writer.flush().map_err(|_| "SDK_RUNTIME_OUTPUT_CLOSED")?;
    }
}

#[cfg(windows)]
mod jobs {
    use super::*;
    use std::{ffi::c_void, mem::size_of, os::windows::io::AsRawHandle, ptr};
    type Handle = *mut c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Basic {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min: usize,
        max: usize,
        active: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Limits {
        basic: Basic,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process: usize,
        peak_job: usize,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Accounting {
        user: i64,
        kernel: i64,
        period_user: i64,
        period_kernel: i64,
        faults: u32,
        total: u32,
        active: u32,
        terminated: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ThreadEntry {
        size: u32,
        usage: u32,
        id: u32,
        owner: u32,
        priority: i32,
        delta: i32,
        flags: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: Handle, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, info: Handle, length: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn QueryInformationJobObject(
            job: Handle,
            class: i32,
            info: Handle,
            length: u32,
            returned: *mut u32,
        ) -> i32;
        fn TerminateJobObject(job: Handle, code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
        fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
        fn OpenThread(access: u32, inherit: i32, tid: u32) -> Handle;
        fn ResumeThread(thread: Handle) -> u32;
    }
    pub struct ProcessJob(isize, u32);
    impl ProcessJob {
        fn attach(process: Handle, pid: u32) -> Result<Self> {
            let handle = unsafe { CreateJobObjectW(ptr::null_mut(), ptr::null()) };
            if handle.is_null() {
                return Err("SDK_JOB_CREATE_FAILED".into());
            }
            let job = Self(handle as isize, pid);
            let mut limits = Limits::default();
            // KILL_ON_CLOSE, aggregate memory (including SDK hidden buffers),
            // and process count. No breakaway flag or inherited job handle.
            limits.basic.flags = 0x2000 | 0x200 | 0x8;
            limits.basic.active = 8;
            limits.job_memory = 1_073_741_824;
            if unsafe {
                SetInformationJobObject(
                    handle,
                    9,
                    (&mut limits as *mut Limits).cast(),
                    size_of::<Limits>() as u32,
                )
            } == 0
                || unsafe { AssignProcessToJobObject(handle, process) } == 0
            {
                return Err("SDK_JOB_CONTAINMENT_FAILED".into());
            }
            Ok(job)
        }
        pub(crate) fn attach_tokio(child: &tokio::process::Child) -> Result<Self> {
            Self::attach(
                child.raw_handle().ok_or("SDK_JOB_PROCESS_HANDLE_MISSING")?,
                child.id().ok_or("SDK_JOB_PROCESS_ID_MISSING")?,
            )
        }
        pub(crate) fn attach_std(child: &std::process::Child) -> Result<Self> {
            Self::attach(child.as_raw_handle(), child.id())
        }
        /// CREATE_SUSPENDED prevents Node imports, runtime startup helpers and
        /// user code until membership and limits have been established. A failed
        /// resume destroys this owned job; there is no uncontained fallback.
        pub(crate) fn start(&self) -> Result<()> {
            self.start_with(|| resume_primary(self.1))
        }
        pub(crate) fn start_with(&self, resume: impl FnOnce() -> Result<()>) -> Result<()> {
            if let Err(error) = resume() {
                let stopped = self.stop();
                return Err(match stopped {
                    Ok(_) => error,
                    Err(stop) => format!("{error};{stop}"),
                });
            }
            Ok(())
        }
        pub(crate) fn stop(&self) -> Result<Value> {
            let handle = self.0 as Handle;
            if unsafe { TerminateJobObject(handle, 124) } == 0 {
                return Err("SDK_JOB_STOP_FAILED".into());
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut state = Accounting::default();
                if unsafe {
                    QueryInformationJobObject(
                        handle,
                        1,
                        (&mut state as *mut Accounting).cast(),
                        size_of::<Accounting>() as u32,
                        ptr::null_mut(),
                    )
                } == 0
                {
                    return Err("SDK_JOB_QUERY_FAILED".into());
                }
                if state.active == 0 {
                    return Ok(
                        json!({"processTreeStopProven":true,"activeProcesses":0,"totalProcesses":state.total,"memoryLimitBytes":1_073_741_824u64,"processLimit":8}),
                    );
                }
                if Instant::now() >= deadline {
                    return Err("SDK_JOB_STOP_UNCONFIRMED".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    fn resume_primary(pid: u32) -> Result<()> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(4, 0) };
        if snapshot as isize == -1 {
            return Err("SDK_PRIMARY_THREAD_SNAPSHOT_FAILED".into());
        }
        let mut entry = ThreadEntry {
            size: size_of::<ThreadEntry>() as u32,
            ..Default::default()
        };
        let mut found = Vec::new();
        let mut valid = unsafe { Thread32First(snapshot, &mut entry) };
        while valid != 0 {
            if entry.owner == pid {
                found.push(entry.id);
            }
            entry.size = size_of::<ThreadEntry>() as u32;
            valid = unsafe { Thread32Next(snapshot, &mut entry) };
        }
        unsafe { CloseHandle(snapshot) };
        if found.len() != 1 {
            return Err("SDK_PRIMARY_THREAD_IDENTITY".into());
        }
        let thread = unsafe { OpenThread(2, 0, found[0]) };
        if thread.is_null() {
            return Err("SDK_PRIMARY_THREAD_OPEN_FAILED".into());
        }
        let previous = unsafe { ResumeThread(thread) };
        unsafe { CloseHandle(thread) };
        if previous != 1 {
            return Err("SDK_PRIMARY_THREAD_RESUME_FAILED".into());
        }
        Ok(())
    }
    impl Drop for ProcessJob {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0 as Handle) };
        }
    }
}
#[cfg(windows)]
pub(super) use jobs::ProcessJob;
#[cfg(not(windows))]
pub(super) struct ProcessJob;
#[cfg(not(windows))]
impl ProcessJob {
    pub(super) fn attach_tokio(_: &tokio::process::Child) -> Result<Self> {
        Err("SDK_NATIVE_PLATFORM_UNVERIFIED".into())
    }
    fn attach_std(_: &std::process::Child) -> Result<Self> {
        Err("SDK_NATIVE_PLATFORM_UNVERIFIED".into())
    }
    pub(super) fn stop(&self) -> Result<Value> {
        Err("SDK_NATIVE_PLATFORM_UNVERIFIED".into())
    }
    pub(super) fn start(&self) -> Result<()> {
        Err("SDK_NATIVE_PLATFORM_UNVERIFIED".into())
    }
}

#[cfg(test)]
#[path = "sdk_native_wrapper_tests.rs"]
mod wrapper_tests;

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn plan(root: &Path) -> RuntimePlan {
        std::fs::create_dir(root.join("work")).unwrap();
        std::fs::create_dir(root.join("tmp")).unwrap();
        let mut identity: SdkIdentity = serde_json::from_value(
            super::super::offline_settings("happy")["textSdkIdentity"].clone(),
        )
        .unwrap();
        identity.version = "official-sdk-identity/v2".into();
        identity.permission_profile = NATIVE_POLICY.into();
        identity.bridge_sha256 = compiled_native_bridge_sha();
        identity.account_fingerprint = "a".repeat(64);
        identity.runtime_package_version = "codex-cli 0.162.0-alpha.2".into();
        identity.runtime_sha256 = Some("b".repeat(64));
        identity.runtime_identity = format!(
            "official-codex-runtime/{}/{}",
            identity.runtime_package_version,
            "b".repeat(64)
        );
        let exe = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        identity.runtime_executable = Some(exe.clone());
        identity.supervisor_executable = Some(exe);
        identity.supervisor_sha256 = Some("c".repeat(64));
        identity.resource_directory = Some(root.to_string_lossy().into());
        identity.credential_home = Some(root.to_string_lossy().into());
        identity.codex_home = Some(root.to_string_lossy().into());
        std::fs::write(root.join("tmp/schema.json"), b"{}").unwrap();
        RuntimePlan {
            version: "sdk-native-runtime/v1".into(),
            attempt_id: "attempt-synthetic".into(),
            identity,
            model: "gpt-6.1-sol".into(),
            reasoning: "medium".into(),
            compiled_prompt_sha256: digest("synthetic"),
            compiled_schema_sha256: digest("{}"),
            runtime_schema_sha256: digest("{}"),
            semantic_request_hash: "d".repeat(64),
            execution_fingerprint: "e".repeat(64),
            working_directory: root.join("work"),
            temp_directory: root.join("tmp"),
        }
    }
    pub(super) fn args(plan: &RuntimePlan) -> Vec<String> {
        [
            "exec".into(),
            "--experimental-json".into(),
            "--model".into(),
            plan.model.clone(),
            "--sandbox".into(),
            "read-only".into(),
            "--cd".into(),
            plan.working_directory.to_string_lossy().into(),
            "--skip-git-repo-check".into(),
            "--output-schema".into(),
            plan.temp_directory
                .join("schema.json")
                .to_string_lossy()
                .into(),
            "--config".into(),
            format!("model_reasoning_effort=\"{}\"", plan.reasoning),
            "--config".into(),
            "sandbox_workspace_write.network_access=false".into(),
            "--config".into(),
            "web_search=\"disabled\"".into(),
            "--config".into(),
            "approval_policy=\"never\"".into(),
        ]
        .into()
    }
    #[test]
    fn sdk_native_wrapper_rejects_media_resume_redirects_duplicate_flags_and_configuration_injection(
    ) {
        let temp = tempfile::tempdir().unwrap();
        let plan = plan(temp.path());
        let good = args(&plan);
        assert!(validate_argv(&good, &plan).is_ok());
        for tail in [
            vec!["resume", "remote-thread"],
            vec!["--image", "private.png"],
            vec!["--add-dir", "C:/"],
            vec!["--output-last-message", "private.json"],
            vec!["--config", "model_provider=\"other\""],
            vec!["--model", "gpt-6.1-sol"],
            vec!["--config", "approval_policy=\"never\""],
        ] {
            let mut bad = good.clone();
            bad.extend(tail.into_iter().map(str::to_owned));
            assert!(validate_argv(&bad, &plan).is_err());
        }
        let mut bad = good.clone();
        bad[0] = "login".into();
        assert!(validate_argv(&bad, &plan).is_err());
        bad = good.clone();
        bad[7] = temp.path().to_string_lossy().into();
        assert!(validate_argv(&bad, &plan).is_err());
        std::fs::write(temp.path().join("foreign.json"), b"{}").unwrap();
        bad = good.clone();
        bad[10] = temp.path().join("foreign.json").to_string_lossy().into();
        assert!(validate_argv(&bad, &plan).is_err());
        std::fs::write(
            plan.temp_directory.join("schema.json"),
            b"{\"changed\":true}",
        )
        .unwrap();
        assert!(validate_argv(&good, &plan).is_err());
    }
    #[test]
    fn sdk_native_identity_cannot_downgrade_to_offline_or_use_unbounded_model_parameters() {
        let temp = tempfile::tempdir().unwrap();
        let mut identity = plan(temp.path()).identity;
        identity.validate().unwrap();
        identity.version = "official-sdk-identity/v1".into();
        assert!(identity.validate().is_err());
        for model in ["", "gpt-6.1-sol --image foo", "gpt-6.1-sol\n", "/private"] {
            assert!(!valid_model(model));
        }
        assert!(valid_model("gpt-6.1-sol"));
        assert!(!valid_reasoning("medium\"\nfeatures.shell_tool=true"));
    }
    #[test]
    fn sdk_native_runtime_relay_bounds_hidden_sdk_stderr_and_unterminated_lines() {
        let mut output = Vec::new();
        assert!(relay(&b"line1\nline2\n"[..], &mut output, 64, 8).is_ok());
        assert_eq!(output, b"line1\nline2\n");
        assert!(relay(&vec![b'x'; 65][..], Vec::new(), 64, 64)
            .unwrap_err()
            .contains("STREAM_LIMIT"));
        assert!(relay(&vec![b'x'; 65][..], Vec::new(), 128, 64)
            .unwrap_err()
            .contains("LINE_LIMIT"));
    }
    #[tokio::test]
    async fn sdk_native_settings_cannot_enable_headless_execution_or_runtime_fallback() {
        assert!(!native_execution_enabled());
        let settings = json!({"textProvider":"codex_sdk","textEnabled":true,"sdkLiveEnabled":true,"codexPath":std::env::current_exe().unwrap(),"textModel":"gpt-6.1-sol","reasoning":"medium"});
        assert!(resolve(&settings)
            .await
            .unwrap_err()
            .starts_with("SDK_LIVE_DISABLED"));
    }
    #[cfg(windows)]
    #[test]
    fn sdk_native_job_reclaims_sdk_runtime_and_detached_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let node = PathBuf::from("C:/Program Files/nodejs/node.exe");
        assert!(node.is_file());
        let script="const fs=require('fs'),{spawn}=require('child_process');process.stdin.once('data',()=>{const p=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{detached:true,stdio:'ignore'});fs.writeFileSync(process.argv[1],JSON.stringify({parent:process.pid,child:p.pid}));setInterval(()=>{},1000);});";
        let mut command = std::process::Command::new(node);
        command
            .args(["--eval", script])
            .arg(temp.path().join("pids.json"))
            .env_clear()
            .env("SystemRoot", node_system_root().unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004);
        let mut child = command.spawn().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(!temp.path().join("pids.json").exists());
        let job = ProcessJob::attach_std(&child).unwrap();
        job.start().unwrap();
        child.stdin.take().unwrap().write_all(b"start").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !temp.path().join("pids.json").is_file() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let proof = job.stop().unwrap();
        assert_eq!(proof["processTreeStopProven"], true);
        assert_eq!(proof["activeProcesses"], 0);
        assert!(proof["totalProcesses"].as_u64().unwrap() >= 2);
        child.wait().unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn sdk_native_primary_resume_failure_kills_owned_job_before_any_user_code() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("must-not-execute.txt");
        let mut command = std::process::Command::new("C:/Program Files/nodejs/node.exe");
        command.args(["--eval","require('fs').writeFileSync(process.argv[1],'unexpected');setInterval(()=>{},1000)"]).arg(&marker)
            .env_clear().env("SystemRoot",node_system_root().unwrap()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004);
        let mut child = command.spawn().unwrap();
        let job = ProcessJob::attach_std(&child).unwrap();
        let error = job
            .start_with(|| Err("SDK_PRIMARY_THREAD_RESUME_FAILED: synthetic".into()))
            .unwrap_err();
        assert!(error.starts_with("SDK_PRIMARY_THREAD_RESUME_FAILED"));
        assert_eq!(job.stop().unwrap()["activeProcesses"], 0);
        child.wait().unwrap();
        assert!(!marker.exists());
    }
}
