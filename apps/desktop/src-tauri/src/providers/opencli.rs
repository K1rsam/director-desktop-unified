//! Pinned OpenCLI browser transport. Keeps legacy Chrome snapshots on their original adapter.
use super::*;
use crate::store::s;
use std::{io::{Read,Write},process::{Command,Stdio},time::{Duration,Instant}};

const COMPILED_RUNTIME_PIN: &str = include_str!("../../../../../web-image-bridge/runtime.sha256");
// Only this reviewed reader correction may observe an already submitted job.
// Future runtimes must explicitly review their compatibility before adding a pair.
const READ_ONLY_READER_MIGRATIONS: &[(&str, &str)] = &[(
    "054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2",
    "c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057",
), (
    "48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3",
    "c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057",
), (
    "d12cc432a530091340ca288f1b4bf2f202ce803d04b265aa39a1c844ee718ac8",
    "c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057",
), (
    "f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70",
    "c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057",
), (
    "54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0",
    "c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057",
), (
    "afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6",
    "c3d1031294a0a4f1f9a2bf5ca2320797e3e2efaa77c37dcc7c079c05b68c9057",
)];

pub(crate) fn read_only_reader_migration_allowed(from: &str, to: &str) -> bool {
    to == COMPILED_RUNTIME_PIN.trim()
        && READ_ONLY_READER_MIGRATIONS.iter().any(|(old,new)| *old==from && *new==to)
}

// These exact already-reviewed receipts remain valid after upgrading the app.
// They are not in reader_policy and cannot select or execute an old reader.
pub(crate) fn read_only_reader_audit_allowed(from: &str, to: &str) -> bool {
    read_only_reader_migration_allowed(from,to)
        || (to=="f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70"
            && ["054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2",
                "48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3",
                "d12cc432a530091340ca288f1b4bf2f202ce803d04b265aa39a1c844ee718ac8"].contains(&from))
        || (to=="54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0"
            && ["054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2",
                "48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3",
                "d12cc432a530091340ca288f1b4bf2f202ce803d04b265aa39a1c844ee718ac8",
                "f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70"].contains(&from))
        || (to=="afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6"
            && ["054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2",
                "48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3",
                "d12cc432a530091340ca288f1b4bf2f202ce803d04b265aa39a1c844ee718ac8",
                "f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70",
                "54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0"].contains(&from))
}

fn executable_readers(executable: &Path) -> Vec<PathBuf> {
    let Some(parent) = executable.parent() else { return vec![]; };
    let mut readers = vec![];
    if let Ok((sha, _)) = crate::store::hash_file(executable) {
        readers.push(parent.join(".client-resources").join(sha).join("opencli/bridge.mjs"));
    }
    readers.push(parent.join("opencli/bridge.mjs"));
    readers
}

fn current_executable_readers() -> Vec<PathBuf> {
    std::env::current_exe().map(|exe| executable_readers(&exe)).unwrap_or_default()
}

struct VerifiedReader {
    runner: PathBuf,
    runner_sha256: String,
    manifest_sha256: String,
}

fn reader_mismatch() -> ProviderError {
    ProviderError::new("OPENCLI_COMPONENT_MISMATCH", "网页只读组件校验失败，保留原任务，未重新发送。")
}

fn runtime_file(root: &Path, relative: &str) -> ProviderResult<PathBuf> {
    if relative.is_empty() || relative.contains('\\')
        || !Path::new(relative).components().all(|part| matches!(part, std::path::Component::Normal(_))) {
        return Err(reader_mismatch());
    }
    let file = root.join(relative);
    let metadata = fs::symlink_metadata(&file).map_err(|_| reader_mismatch())?;
    let canonical = fs::canonicalize(&file).map_err(|_| reader_mismatch())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || !canonical.starts_with(root) {
        return Err(reader_mismatch());
    }
    Ok(canonical)
}

fn verify_reader(runner: &Path, pin: &str) -> ProviderResult<VerifiedReader> {
    let sha_valid = |sha: &str| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit());
    if !runner.is_absolute() || runner.file_name().and_then(|p| p.to_str()) != Some("bridge.mjs") || !sha_valid(pin) {
        return Err(reader_mismatch());
    }
    let root = fs::canonicalize(runner.parent().ok_or_else(reader_mismatch)?).map_err(|_| reader_mismatch())?;
    let runner = runtime_file(&root, "bridge.mjs")?;
    let declared = fs::read_to_string(runtime_file(&root, "runtime.sha256")?).map_err(|_| reader_mismatch())?;
    if declared.trim() != pin { return Err(reader_mismatch()); }
    let manifest_path = runtime_file(&root, "runtime-manifest.json")?;
    let manifest_bytes = fs::read(manifest_path).map_err(|_| reader_mismatch())?;
    if manifest_bytes.len() > 4 * 1024 * 1024 { return Err(reader_mismatch()); }
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|_| reader_mismatch())?;
    let files = manifest["files"].as_array().filter(|files| !files.is_empty() && files.len() <= 16_384).ok_or_else(reader_mismatch)?;
    if manifest["version"] != "director-opencli-runtime/v1" { return Err(reader_mismatch()); }
    let owned=files.iter().any(|f|f["path"]=="composer-transport.mjs");
    let names:Vec<&str>=if owned {vec!["bridge.mjs","composer-transport.mjs","vendor-source.json","package-lock.json"]}else{vec!["bridge.mjs","vendor-source.json","package-lock.json"]};
    let mut aggregate=Sha256::new();for name in names {aggregate.update(fs::read(runtime_file(&root,name)?).map_err(|_|reader_mismatch())?);}
    if format!("{:x}",aggregate.finalize())!=pin{return Err(reader_mismatch());}
    let mut verified = std::collections::HashMap::new();
    for file in files {
        let relative = file["path"].as_str().ok_or_else(reader_mismatch)?;
        let expected = file["sha256"].as_str().filter(|sha| sha_valid(sha)).ok_or_else(reader_mismatch)?;
        let path = runtime_file(&root, relative)?;
        if verified.insert(relative.to_owned(), expected.to_owned()).is_some()
            || crate::store::hash_file(&path).map_err(|_| reader_mismatch())?.0 != expected {
            return Err(reader_mismatch());
        }
    }
    for required in ["bridge.mjs", "vendor-source.json", "package-lock.json", "package.json", "runtime.sha256"] {
        if !verified.contains_key(required) { return Err(reader_mismatch()); }
    }
    if owned&&!verified.contains_key("composer-transport.mjs"){return Err(reader_mismatch());}
    // The compiled aggregate pins vendor-source.json; its file hashes must also
    // agree with the complete package manifest, rather than trusting a sidecar.
    let vendor: Value = serde_json::from_slice(&fs::read(runtime_file(&root, "vendor-source.json")?).map_err(|_| reader_mismatch())?).map_err(|_| reader_mismatch())?;
    if vendor["commit"] != manifest["sourceCommit"] { return Err(reader_mismatch()); }
    let vendor_files = vendor["files"].as_array().filter(|files| !files.is_empty()).ok_or_else(reader_mismatch)?;
    for file in vendor_files {
        let relative = file["path"].as_str().ok_or_else(reader_mismatch)?;
        let expected = file["sha256"].as_str().ok_or_else(reader_mismatch)?;
        if verified.get(&format!("vendor/opencli/{relative}")).map(String::as_str) != Some(expected) {
            return Err(reader_mismatch());
        }
    }
    Ok(VerifiedReader { runner, runner_sha256: verified["bridge.mjs"].clone(), manifest_sha256: hex_hash(&manifest_bytes) })
}

#[derive(Clone, Copy)]
struct ReaderPolicy<'a> {
    compiled_pin: &'a str,
    migrations: &'a [(&'a str, &'a str)],
}

fn reader_policy() -> ReaderPolicy<'static> {
    ReaderPolicy { compiled_pin: COMPILED_RUNTIME_PIN.trim(), migrations: READ_ONLY_READER_MIGRATIONS }
}

fn read_only_reader_config(
    original: &ProviderConfig, request: &GenerationRequest, operation: &str,
    policy: ReaderPolicy<'_>, locate: impl FnOnce() -> Vec<PathBuf>,
) -> ProviderResult<(ProviderConfig, Option<Value>)> {
    let frozen = original.browser_transport.as_ref();
    let old_pin = frozen.and_then(|v| v["runtimeSha256"].as_str()).unwrap_or("");
    if !matches!(operation, "query" | "reconcile") || original.kind != "chrome" || !enabled(original)
        || request.kind != "image" || request.model != "chatgpt-image" || old_pin == policy.compiled_pin
        || !policy.migrations.iter().any(|(old, new)| *old == old_pin && *new == policy.compiled_pin) {
        return Ok((original.clone(), None));
    }
    // No executable-bound current reader means the original recovery remains in
    // charge. Installed/global/dev components are deliberately not candidates.
    let Some(current) = locate().iter().find_map(|path| verify_reader(path, policy.compiled_pin).ok()) else {
        return Ok((original.clone(), None));
    };
    let old_runner = PathBuf::from(frozen.and_then(|v| v["runner"].as_str()).unwrap_or(""));
    let old = verify_reader(&old_runner, old_pin)?;
    let mut temporary = original.clone();
    let transport = temporary.browser_transport.as_mut().and_then(Value::as_object_mut).ok_or_else(reader_mismatch)?;
    transport.insert("runner".into(), json!(current.runner));
    transport.insert("runtimeSha256".into(), json!(policy.compiled_pin));
    let audit = json!({"schemaVersion":"opencli-readonly-reader-migration/v1","operation":operation,
        "bridgeOperation":"query","readOnly":true,"resubmitted":false,"frozenConfigUnchanged":true,
        "fromRuntimeSha256":old_pin,"toRuntimeSha256":policy.compiled_pin,
        "fromRunnerSha256":old.runner_sha256,"toRunnerSha256":current.runner_sha256,
        "fromManifestSha256":old.manifest_sha256,"toManifestSha256":current.manifest_sha256,
        "frozenConfigSha256":crate::pipeline::digest(&json!(original).to_string()),
        "generationRequestSha256":crate::pipeline::digest(&json!(request).to_string())});
    Ok((temporary, Some(audit)))
}

fn query_with_reader(
    original: &ProviderConfig, request: &GenerationRequest, operation: &str, policy: ReaderPolicy<'_>,
    locate: impl FnOnce() -> Vec<PathBuf>,
    read: impl FnOnce(&ProviderConfig, &str, &GenerationRequest) -> ProviderResult<Value>,
) -> ProviderResult<QueryResult> {
    let (temporary, audit) = read_only_reader_config(original, request, operation, policy, locate)?;
    let value = read(&temporary, "query", request)?;
    let mut result: QueryResult = serde_json::from_value(value).map_err(|_| ProviderError::new("OPENCLI_PACKET", "原网页回执无效"))?;
    if let Some(audit) = audit {
        let raw = result.raw.as_object_mut().filter(|raw| !raw.contains_key("readerMigration")).ok_or_else(|| ProviderError::new("OPENCLI_PACKET", "只读组件回执无法保全原始证明"))?;
        raw.insert("readerMigration".into(), audit);
    }
    Ok(result)
}

pub fn enabled(c:&ProviderConfig)->bool { c.browser_transport.as_ref().is_some_and(|v|v["transport"]=="opencli/v1") }
pub fn setup_status(c:&ProviderConfig)->Value {
    setup_view(c,invoke(c,"status",None))
}
fn setup_view(c:&ProviderConfig,result:ProviderResult<Value>)->Value {
    let configured=c.browser_transport.as_ref().is_some_and(|t|!s(t,"profile").is_empty()&&!s(t,"accountKey").is_empty());
    let browser_kind=result.as_ref().ok().and_then(|v|v["browserKind"].as_str()).filter(|k|["chrome","edge"].contains(k))
        .or_else(||c.browser_transport.as_ref().and_then(|t|t["browserKind"].as_str()).filter(|k|["chrome","edge"].contains(k))).map(str::to_owned);
    let (state,message,clients,usable,code)=match result {
        Ok(data) if data["ready"]==true && configured=>("ready","已连接，可以到第二阶段选择资产并生成图片。".to_owned(),1,1,String::new()),
        Ok(_)=>("unconfigured","浏览器已就绪，点击“连接并检查”保存连接。".to_owned(),1,0,String::new()),
        Err(e)=>(if e.code=="BRIDGE_BUSY"{"busy"}else if e.code=="BROWSER_SELECTION_REQUIRED"{"needs_selection"}else if e.code=="BROWSER_NETWORK_UNAVAILABLE"{"network_unavailable"}else{"waiting_browser"},e.message,0,0,e.code),
    };
    let mut value=json!({"schemaVersion":"chrome-setup/v1","installed":true,"configured":configured,"componentVersion":"OpenCLI 1.8.8 / Bridge 1.0.24","status":state,"message":message,"clients":clients,"usableClients":usable,"checkedAt":now(),"browserProfiles":[],"transport":"opencli","errorCode":code});
    if let Some(kind)=browser_kind {value["browserKind"]=json!(kind);}
    if let Some(root)=std::env::var_os("LOCALAPPDATA") {let extension=PathBuf::from(root).join("Programs/Director-OpenCLI-Bridge/1.0.24/extension");if extension.join("manifest.json").is_file(){value["extensionPath"]=json!(extension);}}
    value
}
pub fn config(home:&Path,settings:&Value)->ProviderResult<(ProviderConfig,String)> {
    let node= settings["textSdkNodePath"].as_str().filter(|p|!p.is_empty()).map(PathBuf::from)
        .or_else(||std::env::var_os("ProgramFiles").map(|p|PathBuf::from(p).join("nodejs/node.exe")))
        .filter(|p|p.is_file()).ok_or_else(||ProviderError::new("OPENCLI_COMPONENT_MISSING","请先在 OpenAI 设置安装 Node 组件。"))?;
    let profile=settings["opencliProfileId"].as_str().unwrap_or("");
    let account=settings["opencliAccountKey"].as_str().unwrap_or("");
    let deployed=current_executable_readers().into_iter().find(|path|path.is_file());
    let installed=std::env::var_os("LOCALAPPDATA").map(|p|PathBuf::from(p).join("Programs/Director-OpenCLI-Bridge/runtime-1.0.0/bridge.mjs"));
    let dev=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web-image-bridge/bridge.mjs");
    let runner=deployed.filter(|p|p.is_file()).or_else(||installed.filter(|p|p.is_file())).or_else(||(cfg!(debug_assertions)&&dev.is_file()).then_some(dev)).ok_or_else(||ProviderError::new("OPENCLI_COMPONENT_MISSING","未安装网页出图组件。"))?;
    let runtime_sha=COMPILED_RUNTIME_PIN.trim();
    let transport=json!({"transport":"opencli/v1","runtimeSha256":runtime_sha,"runner":fs::canonicalize(runner).map_err(|e|ProviderError::new("OPENCLI_COMPONENT_MISSING",e.to_string()))?,"stateRoot":home.join("opencli-requests"),"profile":profile,"accountKey":account});
    let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","endpoint":"http://127.0.0.1:19825","cliPath":node,"browserTransport":transport,"timeoutSeconds":90,"chromeBinding":{"profileId":profile,"endpoint":"http://127.0.0.1:19825","bundleId":"opencli-1.0.24"}})).map_err(|e|ProviderError::new("OPENCLI_CONFIG",e.to_string()))?;
    Ok((c,format!("chrome:opencli:{profile}:{account}")))
}
pub fn invoke(c:&ProviderConfig,operation:&str,request:Option<&GenerationRequest>)->ProviderResult<Value>{
    let t=c.browser_transport.as_ref().ok_or_else(||ProviderError::new("OPENCLI_CONFIG","网页运输配置缺失"))?;
    let runner=PathBuf::from(t["runner"].as_str().unwrap_or(""));
    let expected=t["runtimeSha256"].as_str().unwrap_or("");
    let actual=runner.parent().and_then(|p|fs::read_to_string(p.join("runtime.sha256")).ok());
    if expected.len()!=64 || actual.as_deref().map(str::trim)!=Some(expected) {
        return Err(ProviderError::new("OPENCLI_COMPONENT_MISMATCH","网页组件与原冻结版本不一致，请恢复对应组件；原任务未重发。"));
    }
    // New send intents are meaningful only for these exact owned adapter bytes.
    // Historical frozen readers retain their old contract and are never resent.
    if expected==COMPILED_RUNTIME_PIN.trim(){
        let directory=runner.parent().ok_or_else(reader_mismatch)?;let mut aggregate=Sha256::new();
        for name in ["bridge.mjs","composer-transport.mjs","vendor-source.json","package-lock.json"]{
            aggregate.update(fs::read(directory.join(name)).map_err(|_|reader_mismatch())?);
        }
        if format!("{:x}",aggregate.finalize())!=expected{return Err(reader_mismatch());}
    }
    let root=PathBuf::from(t["stateRoot"].as_str().unwrap_or(""));
    if !root.is_absolute(){return Err(ProviderError::new("OPENCLI_CONFIG","网页状态目录必须为绝对路径"));}
    fs::create_dir_all(&root).map_err(|e|ProviderError::new("OPENCLI_STORAGE",e.to_string()))?;
    let lock=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(root.join(".command.lock")).map_err(|e|ProviderError::new("OPENCLI_STORAGE",e.to_string()))?;
    lock.try_lock().map_err(|_|ProviderError::new("BRIDGE_BUSY","网页接口正忙，保留队列等待。"))?;
    let mut cmd=Command::new(c.cli_path.as_ref().ok_or_else(||ProviderError::new("OPENCLI_COMPONENT_MISSING","Node 组件缺失"))?);
    cmd.arg(t["runner"].as_str().unwrap_or("").trim_start_matches(r"\\?\")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)] {use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
    let _guard=CliGuard::enter();
    let mut child=cmd.spawn().map_err(|_|ProviderError::new("OPENCLI_START","网页组件启动失败"))?;
    let input=serde_json::to_vec(&json!({"operation":operation,"config":t,"request":request})).map_err(|e|ProviderError::new("OPENCLI_CONFIG",e.to_string()))?;
    child.stdin.take().unwrap().write_all(&input).map_err(|_|ProviderError::uncertain("网页组件请求写入中断，保留原任务。"))?;
    let mut stdout=child.stdout.take().unwrap();let reader=std::thread::spawn(move||{let mut b=vec![];stdout.by_ref().take(2*1024*1024+1).read_to_end(&mut b).map(|_|b)});
    let deadline=Instant::now()+Duration::from_secs(if matches!(operation,"status"|"prepare"){20}else{90});
    loop {if child.try_wait().map_err(|_|ProviderError::new("OPENCLI_PROCESS","无法读取网页组件进程"))?.is_some(){break;}if shutting_down()||Instant::now()>deadline{let _=child.kill();let _=child.wait();let _=reader.join();return Err(if operation=="submit"{ProviderError::uncertain("网页发送中断，只能查询原任务，不会重发。")}else{ProviderError::new("OPENCLI_READ_TIMEOUT","网页读取超时，保留原任务等待恢复。")});}std::thread::sleep(Duration::from_millis(30));}
    let bytes=reader.join().ok().and_then(|r|r.ok()).filter(|b|b.len()<=2*1024*1024).ok_or_else(||ProviderError::new("OPENCLI_PACKET","网页组件回执无效"))?;
    let v:Value=serde_json::from_slice(&bytes).map_err(|_|{let _=fs::write(root.join("last-protocol-error.txt"),&bytes);if operation=="submit"{ProviderError::uncertain("网页发送回执缺失，保留原任务。")}else{ProviderError::new("OPENCLI_PACKET","网页组件回执格式无效")}})?;
    if v["ok"]!=true {let code=v["error"]["code"].as_str().unwrap_or("OPENCLI_ERROR");let mut e=ProviderError::new(code,connection_error_message(code));e.submission_unknown=v["error"]["submissionUnknown"]==true;e.auth_required=matches!(code,"AUTH_REQUIRED"|"ACCOUNT_CHANGED");return Err(e);}
    Ok(v["data"].clone())
}
fn connection_error_message(code:&str)->&'static str {
    match code {
        "BRIDGE_BUSY"=>"网页正在处理任务，请稍后再检查。",
        "BROWSER_SELECTION_REQUIRED"=>"所选类型存在多个浏览器配置，请只保留本次用于出图的配置后连接；已有任务不变。",
        "AUTH_REQUIRED"=>"请在所选浏览器中登录 ChatGPT，再点击“连接并检查”。",
        "ACCOUNT_CHANGED"=>"所选浏览器的登录账户与已保存账户不同。如要切换，请勾选“使用所选浏览器的当前账户”后连接；已有任务仍保留原账户。",
        "BROWSER_NETWORK_UNAVAILABLE"=>"浏览器暂时无法访问 ChatGPT，请检查网络或浏览器代理，确认网页可正常打开后再连接；无需重载扩展或重新登录。",
        "BROWSER_CHECK_REQUIRED"=>"ChatGPT 网页需要完成网站检查，请在所选浏览器中正常完成后再连接。",
        "CHATGPT_PAGE_LOADING"=>"ChatGPT 页面尚未加载完成，请等待网页打开后再连接。",
        "BROWSER_KIND_MISMATCH"=>"当前浏览器类型与保存的绑定不同，请在全局设置选择正确浏览器后重新连接；已有任务保留原绑定。",
        "OPENCLI_BROWSER_NOT_CONNECTED"=>"未找到所选浏览器的可用连接，请打开该浏览器、登录 ChatGPT 并确认已安装组件。",
        "LOCAL_SERVICE_UNAVAILABLE"=>"本机连接服务暂时不可用，请点击“连接并检查”恢复。",
        "OPENCLI_OWNED_PAGE_REQUIRED"=>"请先在所选浏览器打开 ChatGPT 网页，再点击“连接并检查”。",
        "OWNED_NO_CLICK_UNPROVEN"=>"旧网页任务的未发送证明未通过校验，已保留原任务；未提交新的图片生成。",
        _=>"网页连接检查未完成，请重试检查；已有图片任务不会重新发送。",
    }
}
pub fn capabilities(c:&ProviderConfig)->ProviderResult<Capabilities>{
    let status=invoke(c,"status",None)?;
    if status["ready"]!=true {return Err(ProviderError::new("OPENCLI_NOT_READY","网页未就绪"));}
    serde_json::from_value(json!({"provider":"chrome","version":"opencli-image/v1","source":"opencli_web","checkedAt":now(),"canCancel":false,"canReconcileUnknown":true,"raw":status,"models":[{"id":"chatgpt-image","kind":"image","adapted":true,"ratios":["1:1","16:9","9:16","4:3","3:4","3:2","2:3","21:9"],"resolutions":["native"],"allowedDurations":[],"images":10,"videos":0,"audios":0,"totalReferences":10,"requiredImages":0,"raw":{"transport":"opencli/v1","outputsPerAsset":1,"requestedRatioIsPromptInstruction":true}}]})).map_err(|e|ProviderError::new("OPENCLI_PACKET",e.to_string()))
}
pub fn prepare(c:&ProviderConfig,r:&GenerationRequest)->ProviderResult<PreparedRequest>{
    if r.kind!="image" {return Err(ProviderError::new("IMAGE_ONLY","网页渠道仅支持图片"));}
    let data=invoke(c,"prepare",Some(r))?;
    let mut wire=json!({"transport":"opencli/v1","requestHash":data["requestHash"],"profile":data["profile"],"accountKey":data["accountKey"]});
    if data["sendProtocol"]=="owned-composer-send/v2" {
        wire["sendProtocol"]=data["sendProtocol"].clone();wire["composerAdapterSha256"]=data["composerAdapterSha256"].clone();
    }
    Ok(PreparedRequest{provider:"chrome".into(),request:r.clone(),capability_version:"opencli-image/v1".into(),upload_receipts:vec![],wire,estimated_cost:None,prepared_at:now()})
}
pub fn recover_prepare(c:&ProviderConfig,r:&GenerationRequest,proof:&Value)->ProviderResult<PreparedRequest>{
    let held=||ProviderError::new("OPENCLI_PREPARE_RECOVERY_HELD","原网页准备证据不足，保留原任务，不重新提交。");
    let digest=|v:&Value|crate::pipeline::digest(&v.to_string());
    let audit=&proof["legacyNoSubmitAudit"];
    if !enabled(c) || r.kind!="image" || proof["version"]!="chrome-prepare-recovery/v1"
        || proof["noLocalSubmit"]!=true || proof["requestHash"]!=digest(&json!(r)) || !proof["intent"].is_null()
        || audit["stateChainProven"]!=true || audit["preparedNeverCompleted"]!=true
        || audit["configSha256"]!=digest(&json!(c)) || s(audit,"jobId").is_empty()
        || s(audit,"snapshotSha256").len()!=64 || s(audit,"eventHistorySha256").len()!=64 {return Err(held());}
    let root=PathBuf::from(c.browser_transport.as_ref().and_then(|t|t["stateRoot"].as_str()).unwrap_or(""));
    if !root.is_absolute() || valid_id(&r.request_id).is_err() {return Err(held());}
    // A canonical request journal is a possible send, even if the local job has
    // no receipt. Recovery must never reinterpret it as a fresh preparation.
    if root.join(&r.request_id).join("request.json").try_exists().map_err(|_|held())? {return Err(held());}
    let active=root.join("active.json");
    if active.try_exists().map_err(|_|held())? {
        let owner:Value=serde_json::from_slice(&fs::read(active).map_err(|_|held())?).map_err(|_|held())?;
        if owner["requestId"]==r.request_id {return Err(held());}
    }
    prepare(c,r) // Same frozen runtime/request; this operation cannot send an image.
}
pub fn submit(c:&ProviderConfig,p:&PreparedRequest)->ProviderResult<SubmitReceipt>{
    if p.wire["transport"]!="opencli/v1"{return Err(ProviderError::new("PROVIDER_MISMATCH","原任务不属于 OpenCLI，未迁移或重发"));}
    let v=invoke(c,"submit",Some(&p.request))?;serde_json::from_value(v).map_err(|_|ProviderError::uncertain("网页发送回执无法核对，保留原任务"))
}
pub fn query(c:&ProviderConfig,p:&PreparedRequest,id:&str)->ProviderResult<QueryResult>{
    if p.wire["transport"]!="opencli/v1"||p.request.request_id!=id{return Err(ProviderError::new("PROVIDER_MISMATCH","网页原任务身份不匹配"));}
    let mut result=query_with_reader(c,&p.request,"query",reader_policy(),current_executable_readers,|config,operation,request|invoke(config,operation,Some(request)))?;result.actual_cost=None;Ok(result)
}
pub fn reconcile(c:&ProviderConfig,id:&str)->ProviderResult<QueryResult>{
    valid_id(id)?;
    let root=PathBuf::from(c.browser_transport.as_ref().unwrap()["stateRoot"].as_str().unwrap_or(""));
    let saved:Value=serde_json::from_slice(&fs::read(root.join(id).join("request.json")).map_err(|_|ProviderError::new("ORIGINAL_REQUEST_REQUIRED","未找到原网页请求，禁止重发"))?).map_err(|_|ProviderError::new("ORIGINAL_REQUEST_REQUIRED","原网页请求损坏"))?;
    let r:GenerationRequest=serde_json::from_value(saved["request"].clone()).map_err(|_|ProviderError::new("ORIGINAL_REQUEST_REQUIRED","原冻结请求缺失"))?;
    if r.request_id!=id{return Err(ProviderError::new("IMAGE_PROVENANCE","原请求身份不匹配"));}
    query_with_reader(c,&r,"reconcile",reader_policy(),current_executable_readers,|config,operation,request|invoke(config,operation,Some(request)))
}

#[cfg(test)]
#[path = "opencli_reader_migration_tests.rs"]
mod reader_migration_tests;
pub fn download(c:&ProviderConfig,url:&str,target:&Path)->ProviderResult<DownloadReceipt>{
    let parts:Vec<_>=url.split(':').collect();if parts.len()!=3||parts[0]!="opencli-image"||parts[2].len()!=64||!parts[2].bytes().all(|b|b.is_ascii_hexdigit()) {return Err(ProviderError::new("OPENCLI_FILE","网页原图引用无效"));}valid_id(parts[1])?;
    let root=PathBuf::from(c.browser_transport.as_ref().unwrap()["stateRoot"].as_str().unwrap_or(""));let dir=root.join(parts[1]);
    let saved:Value=serde_json::from_slice(&fs::read(dir.join("request.json")).map_err(|_|ProviderError::new("OPENCLI_FILE","原任务文件缺失"))?).map_err(|_|ProviderError::new("OPENCLI_FILE","原任务回执损坏"))?;
    if saved["status"]!="downloaded"||saved["proof"]["originalIdentityVerified"]!=true||saved["proof"]["sha256"]!=parts[2] {return Err(ProviderError::new("IMAGE_PROVENANCE","原图身份未核实"));}
    let bytes=fs::read(dir.join(format!("{}.png",parts[2]))).map_err(|_|ProviderError::new("OPENCLI_FILE","原图文件缺失"))?;
    if hex_hash(&bytes)!=parts[2]||bytes.len() as u64!=saved["proof"]["bytes"].as_u64().unwrap_or(0){return Err(ProviderError::new("IMAGE_PROVENANCE","原图字节已变化"));}
    fs::write(target,&bytes).map_err(|e|ProviderError::new("DOWNLOAD_WRITE",e.to_string()))?;
    Ok(DownloadReceipt{path:target.into(),sha256:parts[2].into(),bytes:bytes.len() as u64,mime:Some("image/png".into())})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_owned_no_click_proof_failure_is_not_reported_as_network_or_login_failure() {
        let message=connection_error_message("OWNED_NO_CLICK_UNPROVEN");
        assert!(message.contains("未发送证明")&&message.contains("未提交"));
        assert!(!message.contains("连接")&&!message.contains("网络")&&!message.contains("登录"));
    }
    #[test]
    fn network_status_is_distinct_from_login_and_reports_the_real_browser() {
        let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","profile":"saved","accountKey":"digest","browserKind":"edge"}})).unwrap();
        let v=setup_view(&c,Err(ProviderError::new("BROWSER_NETWORK_UNAVAILABLE",connection_error_message("BROWSER_NETWORK_UNAVAILABLE"))));
        assert_eq!(v["status"],"network_unavailable");assert_eq!(v["browserKind"],"edge");assert_eq!(v["errorCode"],"BROWSER_NETWORK_UNAVAILABLE");
        assert!(!v.to_string().contains("accountKey"));assert!(v["message"].as_str().unwrap().contains("网络"));
        let v=setup_view(&c,Ok(json!({"ready":true,"browserKind":"chrome"})));
        assert_eq!(v["browserKind"],"chrome","observed browser takes precedence over stale display metadata");
    }
    #[test]
    fn recovery_routes_to_original_opencli_and_holds_any_possible_send() {
        let t=tempfile::tempdir().unwrap();
        let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","stateRoot":t.path(),"runner":t.path().join("missing.mjs"),"runtimeSha256":"a".repeat(64)}})).unwrap();
        let r:GenerationRequest=serde_json::from_value(json!({"requestId":"original","kind":"image","model":"chatgpt-image","prompt":"synthetic","ratio":"16:9","resolution":"native","references":[],"options":{}})).unwrap();
        let proof=json!({"version":"chrome-prepare-recovery/v1","noLocalSubmit":true,"requestHash":crate::pipeline::digest(&json!(r).to_string()),"intent":null,"legacyNoSubmitAudit":{"jobId":"synthetic","stateChainProven":true,"preparedNeverCompleted":true,"configSha256":crate::pipeline::digest(&json!(c).to_string()),"snapshotSha256":"a".repeat(64),"eventHistorySha256":"b".repeat(64)}});
        let no_observe=|_:&Value|->ProviderResult<()> {panic!("OpenCLI never invokes legacy Bridge observers")};
        assert_eq!(super::super::recover_chrome_prepare(&c,&r,&proof,&no_observe).unwrap_err().code,"OPENCLI_COMPONENT_MISMATCH","must reach frozen OpenCLI runtime validation without requesting a legacy Bridge token");
        fs::create_dir(t.path().join("original")).unwrap();fs::write(t.path().join("original/request.json"),"{}").unwrap();
        assert_eq!(recover_prepare(&c,&r,&proof).unwrap_err().code,"OPENCLI_PREPARE_RECOVERY_HELD");
        let mut changed=proof;changed["requestHash"]=json!("changed");assert_eq!(recover_prepare(&c,&r,&changed).unwrap_err().code,"OPENCLI_PREPARE_RECOVERY_HELD");
    }
    #[test] fn ready_status_matches_shared_ui_fixture(){let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","profile":"synthetic","accountKey":"synthetic"}})).unwrap();let v=setup_view(&c,Ok(json!({"ready":true})));let fixture:Value=serde_json::from_str(include_str!("../../../src/fixtures/web-image-status.json")).unwrap();for(key,value)in fixture.as_object().unwrap(){assert_eq!(&v[key],value,"public UI field {key}");}}
    #[test] fn setup_status_matches_frontend_contract_in_ready_waiting_and_busy_states(){let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","profile":"synthetic","accountKey":"synthetic"}})).unwrap();for result in [Ok(json!({"ready":true})),Err(ProviderError::new("AUTH_REQUIRED","请登录")),Err(ProviderError::new("BRIDGE_BUSY","请等待"))]{let v=setup_view(&c,result);assert_eq!(v["schemaVersion"],"chrome-setup/v1");assert_eq!(v["configured"],true);assert_eq!(v["installed"],true);assert!(v["clients"].as_u64().unwrap()>=v["usableClients"].as_u64().unwrap());assert!(v.get("version").is_none());assert!(!v.to_string().contains("accountKey"));}}
    #[test] fn legacy_snapshots_do_not_switch_transport(){let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome"})).unwrap();assert!(!enabled(&c));let raw=serde_json::to_value(&c).unwrap();assert!(raw.get("browserTransport").is_none());}
    #[test] fn frozen_transport_roundtrips_unchanged(){let raw=json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","profile":"bound","stateRoot":"synthetic","accountKey":"original"}});let c:ProviderConfig=serde_json::from_value(raw.clone()).unwrap();assert!(enabled(&c));assert_eq!(serde_json::to_value(c).unwrap()["browserTransport"],raw["browserTransport"]);}
    #[test] fn video_is_rejected_before_invoking_any_component(){let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome"})).unwrap();let r:GenerationRequest=serde_json::from_value(json!({"requestId":"synthetic","kind":"video","model":"x","prompt":"x","ratio":"16:9","resolution":"native","references":[],"options":{}})).unwrap();assert_eq!(prepare(&c,&r).unwrap_err().code,"IMAGE_ONLY");}
    #[test] fn frozen_runtime_mismatch_is_rejected_before_launch(){let t=tempfile::tempdir().unwrap();fs::write(t.path().join("runtime.sha256"),"different").unwrap();let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","runner":t.path().join("bridge.mjs"),"runtimeSha256":"a".repeat(64),"stateRoot":t.path()}})).unwrap();assert_eq!(invoke(&c,"submit",None).unwrap_err().code,"OPENCLI_COMPONENT_MISMATCH");assert!(!t.path().join(".command.lock").exists());}
    #[test] fn original_file_tampering_is_rejected(){let t=tempfile::tempdir().unwrap();let dir=t.path().join("original");fs::create_dir(&dir).unwrap();let hash=hex_hash(b"original");fs::write(dir.join(format!("{hash}.png")),b"changed").unwrap();fs::write(dir.join("request.json"),json!({"status":"downloaded","proof":{"sha256":hash,"bytes":8,"originalIdentityVerified":true}}).to_string()).unwrap();let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","stateRoot":t.path()}})).unwrap();assert_eq!(super::super::download(&c,&format!("opencli-image:original:{hash}"),&t.path().join("output.png")).unwrap_err().code,"IMAGE_PROVENANCE");assert!(!t.path().join("output.png").exists());}
    #[test] fn download_path_traversal_is_rejected(){let c:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","stateRoot":"synthetic"}})).unwrap();assert!(download(&c,&format!("opencli-image:../other:{}","a".repeat(64)),Path::new("synthetic")).is_err());}
}
