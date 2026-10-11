//! Restore the installed browser bridge without opening a turn or replaying a job.
//! Legacy configuration is read only. A newly started bridge uses app-owned data.
use crate::store::{atomic_json, s, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, io::Read, net::{SocketAddr, TcpStream}, path::{Path, PathBuf}, process::{Command, Stdio}, time::{Duration, Instant}};

const VERSION: &str = "chrome-setup/v1";
const COMPONENT: &str = "Programs/Director-3008-ChatGPT-Bridge/671815e";
const CONNECTION: &str = "chrome-connection.json";
const STATUS: &str = "chrome-status.json";
#[cfg(test)]
#[path = "chrome_profile_binding_tests.rs"]
mod profile_binding_tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserBinding { pub profile_id: String, pub endpoint: String, pub bundle_id: String }
pub fn browser_binding(settings: &Value) -> Result<Option<BrowserBinding>> {
    let id=s(settings,"chromeProfileId"); let bound_endpoint=s(settings,"chromeProfileEndpoint"); let bundle=s(settings,"chromeProfileBundleId");
    if id.is_empty() && bound_endpoint.is_empty() && bundle.is_empty() { return Ok(None); }
    if !valid_profile_id(id) || bundle.is_empty() || bound_endpoint.is_empty()
        || endpoint(settings)?.as_str().trim_end_matches('/') != bound_endpoint {
        return Err("Chrome 执行浏览器绑定与当前连接不一致，请在软件设置重新选择；未改用其他浏览器。".into());
    }
    Ok(Some(BrowserBinding{profile_id:id.into(),endpoint:bound_endpoint.into(),bundle_id:bundle.into()}))
}
pub fn binding_settings(base: &str, binding: Option<&BrowserBinding>) -> Value {
    let mut settings=json!({"chromeEndpoint":base});
    if let Some(binding)=binding {settings["chromeProfileId"]=json!(binding.profile_id);settings["chromeProfileEndpoint"]=json!(binding.endpoint);settings["chromeProfileBundleId"]=json!(binding.bundle_id);}
    settings
}
fn valid_profile_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id|id.get_version_num()==4 && id.to_string()==value)
}
fn profile_clients(clients: &[Value]) -> Vec<&Value> {
    clients.iter().filter(|c| c["ready"]==true && c["compatible"]!=false && c["quarantined"]!=true
        && c["transport"]=="extension" && c["browserIncognito"]==false
        && c["browserProfileId"].as_str().is_some_and(valid_profile_id)
        && c["backgroundEpoch"].as_str().is_some_and(|s|!s.is_empty())
        && c["extensionBundleId"].as_str().is_some_and(|s|!s.is_empty())
        && c["serverInstanceId"].as_str().is_some_and(|s|!s.is_empty())).collect()
}
fn profile_collision(clients: &[Value], profile: &str) -> bool {
    let runtimes: std::collections::BTreeSet<_>=profile_clients(clients).iter().filter(|c|c["browserProfileId"]==profile)
        .map(|c|(c["backgroundEpoch"].to_string(),c["extensionBundleId"].to_string(),c["serverInstanceId"].to_string())).collect();
    runtimes.len()>1
}
pub(crate) fn profile_execution_client<'a>(clients: &'a [Value], binding: &BrowserBinding) -> Result<&'a Value> {
    if profile_collision(clients,&binding.profile_id) {return Err("Chrome 浏览器身份出现多个运行实例，已停止新提交；请核对复制的浏览器配置。".into());}
    let filtered: Vec<Value>=profile_clients(clients).into_iter().filter(|c|c["browserProfileId"]==binding.profile_id && c["extensionBundleId"]==binding.bundle_id).cloned().collect();
    let chosen=execution_client(&filtered).ok_or("已绑定的 Chrome 浏览器未连接或正在忙碌，请重新连接原浏览器；未改用其他浏览器。")?;
    clients.iter().find(|c|c["id"]==chosen["id"]).ok_or("Chrome 执行页面身份未确认。".into())
}
fn profiles(clients: &[Value]) -> Vec<Value> {
    let mut groups=std::collections::BTreeMap::new();
    for client in profile_clients(clients) {
        let id=s(client,"browserProfileId");
        groups.entry(id.to_owned()).or_insert_with(||json!({"profileId":id,"bundleId":client["extensionBundleId"],"collision":profile_collision(clients,id)}));
    }
    groups.into_values().collect()
}
pub fn binding_patch(settings: &Value, observed: &Value, explicit: Option<&str>) -> Result<Option<Value>> {
    let candidates=observed["browserProfiles"].as_array().ok_or("Chrome 浏览器身份列表不可用，请重新检查连接。")?;
    if explicit.is_none() && browser_binding(settings)?.is_some() {return Ok(None);}
    let chosen=if let Some(id)=explicit {candidates.iter().find(|c|c["profileId"]==id).ok_or("所选 Chrome 浏览器不再连接，请重新检查。")?}
        else if candidates.len()==1 && observed["unidentifiedClients"].as_u64()==Some(0) {&candidates[0]} else {return Ok(None)};
    if chosen["collision"]!=false {return Err("Chrome 浏览器身份冲突，未绑定或发送任务。".into());}
    let profile=s(chosen,"profileId");let bundle=s(chosen,"bundleId");
    if !valid_profile_id(profile)||bundle.is_empty(){return Err("Chrome 浏览器身份尚未核实，请重新连接扩展。".into());}
    Ok(Some(json!({"chromeProfileId":profile,"chromeProfileEndpoint":endpoint(settings)?.as_str().trim_end_matches('/'),"chromeProfileBundleId":bundle})))
}

fn bounded(path: &Path, limit: u64) -> Result<String> {
    if !crate::runtime_setup::safe_absolute(path) { return Err("Chrome 配置路径包含外链，未读取。".into()); }
    let meta = fs::symlink_metadata(path).map_err(|_| "未找到 Chrome 桥接配置。")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit {
        return Err("Chrome 桥接配置不是有效的本机文件。".into());
    }
    fs::read_to_string(path).map_err(|_| "无法读取 Chrome 桥接配置。".into())
}
fn stored(home: &Path, file: &str) -> Value {
    bounded(&home.join(file), 64 * 1024).ok().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or(Value::Null)
}
fn component() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(COMPONENT);
    let pkg: Value = serde_json::from_str(&bounded(&root.join("package.json"), 64 * 1024).ok()?).ok()?;
    (pkg["name"] == "chatgpt-browser-bridge-node" && pkg["version"] == "6.4.0"
        && ["src/index.js","node_modules/express/package.json","extension/manifest.json"].iter().all(|p|crate::runtime_setup::safe_absolute(&root.join(p)))
        && root.join("src/index.js").is_file() && root.join("node_modules/express/package.json").is_file()
        && root.join("extension/manifest.json").is_file()).then_some(root)
}
fn config_candidates(home: &Path) -> Vec<PathBuf> {
    let mut paths = vec![];
    let connection = stored(home, CONNECTION);
    if let Some(path) = connection["sourceConfig"].as_str() { paths.push(path.into()); }
    let starts = [std::env::current_exe().ok(), std::env::current_dir().ok()];
    for start in starts.into_iter().flatten() {
        for ancestor in start.ancestors().take(9) {
            for sibling in ["director-web-3008-stage2-images", "director-web-3008"] {
                let candidate = ancestor.join(sibling).join("runtime-3008/image-browser/bridge.env");
                if candidate.is_file() && !paths.contains(&candidate) { paths.push(candidate); }
            }
        }
    }
    paths
}
struct Legacy { api: String, bridge: String, port: u16 }
fn parse_env(text: &str) -> Result<Legacy> {
    let mut values = std::collections::BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some((k,v)) = line.split_once('=') {
            if values.insert(k.trim(),v.trim()).is_some() { return Err("Chrome 配置存在重复字段。".into()); }
        }
    }
    let secret = |key| -> Result<String> {
        let value = *values.get(key).ok_or("Chrome 配置缺少本机连接凭据。")?;
        if !(32..=256).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err("Chrome 本机连接凭据格式无效。".into());
        }
        Ok(value.into())
    };
    let port = values.get("PORT").unwrap_or(&"53179").parse::<u16>().map_err(|_| "Chrome 桥接端口无效。")?;
    if port < 1024 || values.get("HOST").is_some_and(|h| *h != "127.0.0.1") { return Err("Chrome 桥接仅允许本机回环地址。".into()); }
    Ok(Legacy {api:secret("API_TOKEN")?, bridge:secret("BRIDGE_TOKEN")?, port})
}
fn discovered(home: &Path) -> Result<(PathBuf, Legacy)> {
    let candidates: Vec<_> = config_candidates(home).into_iter().filter_map(|p| bounded(&p, 16 * 1024).ok().and_then(|v| parse_env(&v).ok()).map(|c|(p,c))).collect();
    let Some(first) = candidates.first() else { return Err("未找到旧版 Chrome 连接配置。请在连接详情中导入 bridge.env 或填写本机桥接连接。".into()); };
    if candidates.iter().any(|(_,c)| c.port != first.1.port || c.api != first.1.api || c.bridge != first.1.bridge) { return Err("发现多份不同的 Chrome 连接，请在连接详情中明确选择。".into()); }
    Ok(candidates.into_iter().next().unwrap())
}
fn report(state: &str, message: &str, installed: bool, configured: bool) -> Value {
    json!({"schemaVersion":VERSION,"installed":installed,"configured":configured,"status":state,"message":message,"clients":0,"usableClients":0})
}
pub fn status(home: &Path, settings: &Value) -> Value {
    let installed = component().is_some();
    let configured = !s(settings,"chromeEndpoint").is_empty();
    let mut value = stored(home, STATUS);
    if value["schemaVersion"] != VERSION || value["configFingerprint"] != fingerprint(settings) {
        value = report(if configured {"offline"} else {"unconfigured"}, if configured {"尚未检查浏览器连接。"} else {"连接旧版 Chrome 桥接后可使用网页出图。"}, installed, configured);
    }
    if value["status"]=="ready" && !value["checkedAt"].as_str().and_then(|v|chrono::DateTime::parse_from_rfc3339(v).ok()).is_some_and(|at| (0..=120).contains(&(chrono::Utc::now().timestamp()-at.timestamp()))) {
        value["status"]=json!("offline");value["message"]=json!("上次已连接，请重新检查当前浏览器状态。");
    }
    value["installed"] = json!(installed);
    value["configured"] = json!(configured);
    if let Some(root) = component() { value["componentVersion"] = json!("6.4.0"); value["extensionPath"] = json!(root.join("extension")); }
    value["legacyConfigFound"] = json!(discovered(home).is_ok());
    value.as_object_mut().unwrap().remove("endpoint");
    value.as_object_mut().unwrap().remove("configFingerprint");
    value
}
fn fingerprint(settings:&Value)->Value {
    json!(crate::pipeline::digest(&format!("{}:{}:{}:{}:{}",s(settings,"chromeEndpoint"),settings[crate::app_settings::SCOPE]["credentialRefs"]["chrome"],s(settings,"chromeProfileId"),s(settings,"chromeProfileEndpoint"),s(settings,"chromeProfileBundleId"))))
}
fn endpoint(settings: &Value) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(s(settings,"chromeEndpoint")).map_err(|_| "请先连接 Chrome 桥接。")?;
    if url.scheme() != "http" || !matches!(url.host_str(),Some("127.0.0.1")|Some("localhost")|Some("[::1]")) || url.path() != "/" || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("Chrome 桥接只允许本机回环 HTTP 地址。".into());
    }
    Ok(url)
}
fn get(client: &reqwest::blocking::Client, url: &str, token: &str) -> Result<Value> {
    let mut response = client.get(url).bearer_auth(token).send().map_err(|_| "Chrome 桥接未启动或暂时离线。")?;
    if matches!(response.status().as_u16(),401|403) { return Err("Chrome 桥接凭据不匹配，请重新连接。".into()); }
    if !response.status().is_success() { return Err("Chrome 桥接检查未成功。".into()); }
    let mut bytes=Vec::new(); response.by_ref().take(1024*1024+1).read_to_end(&mut bytes).map_err(|_|"Chrome 状态读取失败。")?;
    if bytes.len()>1024*1024 { return Err("Chrome 状态响应超出限制。".into()); }
    serde_json::from_slice(&bytes).map_err(|_|"Chrome 状态返回格式无效。".into())
}
/// Multiple tabs in one extension runtime share the same browser profile.
/// This selects an opener only; generation still gets its own owned empty tab.
/// Never infer a shared runtime from tab IDs, URL, or an absent epoch.
pub(crate) fn execution_client(clients: &[Value]) -> Option<&Value> {
    let usable: Vec<_> = clients.iter().filter(|c| c["ready"] == true
        && c["compatible"] != false && c["quarantined"] != true
        && c["activeRequest"].is_null()).collect();
    let selected: Vec<_> = usable.iter().copied().filter(|c| c["selected"] == true).collect();
    if selected.len() == 1 { return Some(selected[0]); }
    if selected.len() > 1 { return None; }
    if usable.len() == 1 { return Some(usable[0]); }
    let first = *usable.first()?;
    let epoch = first["backgroundEpoch"].as_str().filter(|s| !s.is_empty())?;
    if !usable.iter().all(|c| c["transport"] == "extension"
        && c["backgroundEpoch"] == epoch && c["extensionBundleId"] == first["extensionBundleId"]
        && c["serverInstanceId"] == first["serverInstanceId"]) { return None; }
    usable.into_iter().min_by_key(|c| (
        c["url"] != "https://chatgpt.com/",
        c["launchToken"].as_str().unwrap_or("").starts_with("bridge-director-desktop-"),
        c["id"].as_str().unwrap_or(""),
    ))
}

pub fn inspect(settings: &Value, token: &str) -> Result<Value> {
    let base = endpoint(settings)?.as_str().trim_end_matches('/').to_string();
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(3)).redirect(reqwest::redirect::Policy::none()).build().map_err(|_|"无法检查 Chrome 连接。")?;
    let health = get(&client,&format!("{base}/health"),token)?;
    // A generic HTTP 200 page is not proof of a compatible bridge.
    if !health["ok"].is_boolean() || !health["pendingRequests"].is_number() { return Err("本机服务不是兼容的 Chrome 图片桥接。".into()); }
    let response = get(&client,&format!("{base}/browser/clients"),token)?;
    if response["ok"] != true { return Err("Chrome 浏览器列表不可用。".into()); }
    let clients = response["clients"].as_array().ok_or("Chrome 浏览器列表格式无效。")?;
    let usable: Vec<_> = clients.iter().filter(|c| c["ready"]==true && c["compatible"]!=false && c["quarantined"]!=true && c["activeRequest"].is_null()).collect();
    let browser_profiles=profiles(clients);
    let unidentified=clients.iter().filter(|c|c["ready"]==true && c["transport"]=="extension" && c["browserIncognito"]!=true
        && !profile_clients(clients).iter().any(|p|p["id"]==c["id"])).count();
    let binding=browser_binding(settings);
    let suggestion=browser_profiles.first().filter(|_|browser_profiles.len()==1 && unidentified==0)
        .filter(|p|p["collision"]==false).map(|p|BrowserBinding{profile_id:s(p,"profileId").into(),bundle_id:s(p,"bundleId").into(),endpoint:base.clone()});
    let selected_binding=binding.as_ref().ok().and_then(|b|b.as_ref()).or(suggestion.as_ref());
    let opener=selected_binding.and_then(|b|profile_execution_client(clients,b).ok());
    let busy = health["pendingRequests"].as_u64().unwrap_or(0)>0
        || health["pendingCommands"].as_u64().unwrap_or(0)>0
        || clients.iter().any(|c|!c["activeRequest"].is_null());
    // Upstream health.ok describes its generic selected-client fallback. An
    // explicitly bound opener is usable when only that fallback is ambiguous.
    let connected = health["ok"] == true || health["needsSelection"] == true && opener.is_some();
    let (state,message) = if binding.is_err() {("needs_selection","执行浏览器绑定与当前连接不一致，请在软件设置重新选择。")}
        else if selected_binding.is_some_and(|b|profile_collision(clients,&b.profile_id)) {("needs_selection","浏览器身份出现多个运行实例，已停止新提交；请核对复制的配置。")}
        else if selected_binding.is_none() && browser_profiles.iter().any(|p|p["collision"]==true) {("needs_selection","浏览器身份出现多个运行实例，未绑定或发送任务；请核对复制的配置。")}
        else if binding.as_ref().ok().and_then(|b|b.as_ref()).is_some() && opener.is_none() && !busy {("waiting_browser","已绑定浏览器未连接，请重新打开原浏览器；未改用其他配置。")}
        else if browser_profiles.len()>1 && selected_binding.is_none() {("needs_selection","发现多个浏览器配置，请在软件设置绑定执行浏览器一次。")}
        else if busy && selected_binding.is_some_and(|b|profile_clients(clients).iter().any(|c|c["browserProfileId"]==b.profile_id && c["extensionBundleId"]==b.bundle_id)) {("busy","浏览器正在执行已有任务，完成后可继续。")}
        else if opener.is_none() && !usable.is_empty() {("waiting_browser","浏览器尚未提供持久身份，请重新连接扩展；隐身页面不用于新生成。")}
        else if busy {("busy","浏览器正在执行已有任务，完成后可继续。")}
        else if !usable.is_empty() && connected {("ready","Chrome 网页已连接，可在图片资产中选择 Chrome 网页生成。")}
        else {("waiting_browser","桥接已启动。请打开 Chrome 的 ChatGPT 页面并连接扩展。")};
    let mut value=report(state,message,component().is_some(),true);
    value["clients"]=json!(clients.len()); value["usableClients"]=json!(usable.len());
    value["pendingRequests"] = health["pendingRequests"].clone();
    value["browserProfiles"]=json!(browser_profiles);value["unidentifiedClients"]=json!(unidentified);
    value["boundProfileId"]=settings["chromeProfileId"].clone();
    Ok(value)
}
pub fn check(home: &Path, settings: &Value, token: &str) -> Result<Value> {
    let mut value=inspect(settings,token).unwrap_or_else(|e|report("offline",&e,component().is_some(),true));
    let mut effective=settings.clone();
    if let Ok(Some(patch))=binding_patch(settings,&value,None) {
        if let Some(doc)=crate::app_settings::load(home)? {
            let current=crate::app_settings::effective_from(&doc,&json!({}));
            if fingerprint(&current)!=fingerprint(settings) {return Err("Chrome 设置在检查期间变化，请重新检查。".into());}
            let updated=crate::app_settings::save(home,&patch,doc["revision"].as_u64().unwrap_or(0))?;
            effective=crate::app_settings::effective_from(&updated,&json!({}));
            value["boundProfileId"]=patch["chromeProfileId"].clone();
        }
    }
    value["checkedAt"]=json!(crate::store::now()); value["endpoint"]=effective["chromeEndpoint"].clone();
    value["configFingerprint"]=fingerprint(&effective);
    atomic_json(&home.join(STATUS),&value)?;
    value.as_object_mut().unwrap().remove("endpoint");value.as_object_mut().unwrap().remove("configFingerprint"); Ok(value)
}
fn hide(command: &mut Command) {
    #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    #[cfg(not(windows))] { let _=command; }
}
fn node(settings: &Value) -> Result<PathBuf> {
    let explicit=PathBuf::from(s(settings,"textSdkNodePath"));
    if native_exe(&explicit,"node.exe") { return Ok(explicit); }
    for key in ["ProgramFiles","LOCALAPPDATA"] { if let Some(root)=std::env::var_os(key) { let p=PathBuf::from(root).join("nodejs/node.exe"); if native_exe(&p,"node.exe"){return Ok(p);} } }
    Err("Chrome 桥接需要 Node.js，请先安装 OpenAI SDK 组件。".into())
}
fn native_exe(path: &Path, name: &str) -> bool {
    if !crate::runtime_setup::safe_absolute(path) || !path.file_name().is_some_and(|n|n.to_string_lossy().eq_ignore_ascii_case(name)) { return false; }
    let mut magic=[0;2]; fs::File::open(path).and_then(|mut f|f.read_exact(&mut magic)).is_ok() && magic==*b"MZ"
}
fn browser_profile() -> Option<String> {
    let root=PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Google/Chrome/User Data");
    let extension=component()?.join("extension");
    let mut profiles=vec!["Default".to_string()];
    profiles.extend((1..=16).map(|i|format!("Profile {i}")));
    for name in profiles {
        for file in ["Secure Preferences","Preferences"] {
            let Ok(text)=bounded(&root.join(&name).join(file),8*1024*1024) else {continue};
            let Ok(prefs)=serde_json::from_str::<Value>(&text) else {continue};
            if prefs["extensions"]["settings"].as_object().into_iter().flat_map(|m|m.values()).any(|item|
                item["path"].as_str().is_some_and(|p|PathBuf::from(p).canonicalize().ok()==extension.canonicalize().ok() && extension.is_dir())) { return Some(name); }
        }
    }
    None
}
pub fn open(_home: &Path) -> Result<Value> {
    let chrome=["ProgramFiles","ProgramFiles(x86)","LOCALAPPDATA"].into_iter().filter_map(std::env::var_os)
        .map(|root|PathBuf::from(root).join("Google/Chrome/Application/chrome.exe")).find(|p|native_exe(p,"chrome.exe")).ok_or("未找到 Chrome，请先安装 Google Chrome 浏览器。")?;
    let mut cmd=Command::new(chrome);
    // The old runner's scratch chrome-profile can be empty. Open the actual
    // Chrome profile that already owns the extension; never install or copy it.
    if let Some(profile)=browser_profile(){cmd.arg(format!("--profile-directory={profile}"));}
    cmd.args(["--new-window","https://chatgpt.com/"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map_err(|_|"无法打开 Chrome，请从桌面打开浏览器后连接扩展。")?;
    Ok(json!({"opened":true,"message":"已打开 Chrome，请确认 ChatGPT 已登录并连接桥接扩展，再检查连接。"}))
}
struct LaunchGuard(Option<std::process::Child>);
impl Drop for LaunchGuard {fn drop(&mut self){if let Some(child)=self.0.as_mut(){let _=child.kill();let _=child.wait();}}}
fn ensure_no_workflow_restore(data: &Path) -> Result<()> {
    // Bridge 6.4.0 always calls WorkflowManager.restore() at startup. Its
    // schema-v4 state can resume automation even without WORKFLOW_CONFIG.
    // Never clear/quarantine its evidence merely to make a connection succeed.
    const LIMIT: u64 = 1024 * 1024;
    let directory = data.join("workflows");
    let path = directory.join("state.json");
    let rejected = "Chrome 运行目录含已有或无法核实的工作流；已保留全部数据，未启动或恢复任务。";
    if !crate::runtime_setup::safe_absolute(&directory)
        || !crate::runtime_setup::safe_absolute(&path) {
        return Err(rejected.into());
    }
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(rejected.into()),
    };
    if !meta.is_file() || meta.len() > LIMIT { return Err(rejected.into()); }
    let file = fs::File::open(&path).map_err(|_| rejected)?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|_| rejected)?;
    if bytes.len() > LIMIT as usize { return Err(rejected.into()); }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| rejected)?;
    if value["schemaVersion"].as_u64() != Some(4)
        || !value["workflows"].as_object().is_some_and(|items| items.is_empty()) {
        return Err(rejected.into());
    }
    Ok(())
}
pub fn connect(home: &Path, settings: &Value, token: Option<&str>, source: Option<&str>) -> Result<(Value, Option<Value>)> {
    // An already configured, reachable bridge is never replaced or restarted.
    if source.is_none() {if let Some(token)=token { if let Ok(v)=inspect(settings,token) { return Ok((v,None)); } }}
    let root=component().ok_or("未找到已安装的 Chrome 桥接组件，请先安装本机浏览器桥接。")?;
    let (path,legacy)=if let Some(path)=source.filter(|p|!p.is_empty()) { let p=PathBuf::from(path); if !p.is_absolute(){return Err("请选择完整 bridge.env 文件路径。".into());} let parsed=parse_env(&bounded(&p,16*1024)?)?; (p,parsed) }else{discovered(home)?};
    let base=format!("http://127.0.0.1:{}",legacy.port);
    if source.is_none() && token.is_some() && !s(settings,"chromeEndpoint").is_empty() && s(settings,"chromeEndpoint").trim_end_matches('/')!=base {
        return Err("已有手动 Chrome 连接与旧版地址不同，请检查原连接或明确导入配置。".into());
    }
    let patch=json!({"chromeEndpoint":base,"chromeToken":legacy.api});
    if let Ok(value)=inspect(&patch,&legacy.api){return Ok((value,Some(patch)));}
    let addr:SocketAddr=format!("127.0.0.1:{}",legacy.port).parse().map_err(|_|"桥接端口无效。")?;
    if TcpStream::connect_timeout(&addr,Duration::from_millis(500)).is_ok(){return Err("Chrome 桥接端口被另一服务占用，未启动或覆盖。".into());}
    let state=home.join("chrome-bridge");
    for path in [state.clone(),state.join("data"),state.join("data/metadata.sqlite"),state.join("stdout.log"),state.join("stderr.log"),state.join("managed.env"),home.join(CONNECTION),home.join(STATUS)] {
        if !crate::runtime_setup::safe_absolute(&path) {return Err("Chrome 运行目录含外链，未启动或接管旧工作流目录。".into());}
    }
    ensure_no_workflow_restore(&state.join("data"))?;
    fs::create_dir_all(&state).map_err(|_|"无法创建 Chrome 本机运行目录。")?;
    // Never point at the old workflow database: startup there could restore work.
    let data=state.join("data");fs::create_dir_all(&data).map_err(|_|"无法创建 Chrome 本机数据目录。")?;
    let out=fs::OpenOptions::new().create(true).append(true).open(state.join("stdout.log")).map_err(|_|"无法创建 Chrome 日志。")?;
    let err=fs::OpenOptions::new().create(true).append(true).open(state.join("stderr.log")).map_err(|_|"无法创建 Chrome 日志。")?;
    let mut cmd=Command::new(node(settings)?);cmd.env_clear();
    for key in ["PATH","SystemRoot","WINDIR","USERPROFILE","APPDATA","LOCALAPPDATA","TEMP","TMP","ProgramFiles","ProgramFiles(x86)","ProgramW6432"] {if let Some(value)=std::env::var_os(key){cmd.env(key,value);}}
    cmd.arg(root.join("src/index.js")).args(["--server","--no-auto-open-tab","--no-reload-extension"])
        .current_dir(&state).env_remove("WORKFLOW_CONFIG").env("DATA_DIR",&data).env("ENV_FILE",state.join("managed.env"))
        .env("HOST","127.0.0.1").env("PORT",legacy.port.to_string()).env("API_TOKEN",&legacy.api).env("BRIDGE_TOKEN",&legacy.bridge)
        .env("PUBLIC_BASE_URL",&base).env("PAYLOAD_DEBUG","0").env("ATTACHMENT_TRANSPORT","url").env("ANSWER_TIMEOUT_MS","900000")
        .stdin(Stdio::null()).stdout(Stdio::from(out)).stderr(Stdio::from(err));hide(&mut cmd);
    let mut child=LaunchGuard(Some(cmd.spawn().map_err(|_|"无法启动 Chrome 桥接组件。")?));
    let until=Instant::now()+Duration::from_secs(12);
    loop {
        if let Ok(value)=inspect(&patch,&legacy.api) {
            atomic_json(&home.join(CONNECTION),&json!({"schemaVersion":VERSION,"sourceConfig":path,"componentHome":root,"pid":child.0.as_ref().unwrap().id()}))?;
            child.0.take();
            return Ok((value,Some(patch)));
        }
        if child.0.as_mut().unwrap().try_wait().map_err(|_|"无法核对 Chrome 桥接进程。")?.is_some(){return Err("Chrome 桥接启动失败，请查看连接诊断。".into());}
        if Instant::now()>=until {return Err("Chrome 桥接启动超时，已停止本次启动。".into());}
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn rejects_remote_and_ambiguous_config(){let t="a".repeat(40);let valid=format!("HOST=127.0.0.1\nPORT=53179\nAPI_TOKEN={t}\nBRIDGE_TOKEN={t}");assert_eq!(parse_env(&valid).unwrap().port,53179);assert!(parse_env(&(valid.clone()+"\nPORT=8080")).is_err());assert!(parse_env(&valid.replace("127.0.0.1","0.0.0.0")).is_err());assert!(endpoint(&json!({"chromeEndpoint":"http://remote.invalid"})).is_err());assert!(endpoint(&json!({"chromeEndpoint":"http://127.0.0.1:53179?token=x"})).is_err());}
    fn fixture(health:Value, browsers:Option<Value>)->(Value,std::thread::JoinHandle<()>) {
        use std::{net::TcpListener,io::Write};
        let server=TcpListener::bind("127.0.0.1:0").unwrap(); let endpoint=format!("http://{}",server.local_addr().unwrap());
        let join=std::thread::spawn(move||{
            let mut responses=vec![("/health",health)];if let Some(b)=browsers {responses.push(("/browser/clients",json!({"ok":true,"clients":b})));}
            for (route,body) in responses {
                let (mut socket,_)=server.accept().unwrap();socket.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
                let mut request=vec![];let mut chunk=[0;1024];while !request.ends_with(b"\r\n\r\n") {let n=socket.read(&mut chunk).unwrap();assert!(n>0);request.extend_from_slice(&chunk[..n]);assert!(request.len()<8192);}
                let request=String::from_utf8(request).unwrap();assert!(request.starts_with(&format!("GET {route} HTTP/1.1")),"connection check must never POST");assert!(request.to_lowercase().contains("authorization: bearer fixture-credential"));
                let bytes=serde_json::to_vec(&body).unwrap();write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();socket.write_all(&bytes).unwrap();
            }
        });(json!({"chromeEndpoint":endpoint}),join)
    }
    #[test] fn connected_server_is_not_mistaken_for_a_ready_browser() {
        for (clients,expected) in [(json!([]),"waiting_browser"),(json!([{"id":"one","ready":true}]),"waiting_browser"),(json!([{"id":"one","ready":true},{"id":"two","ready":true}]),"waiting_browser"),(json!([{"id":"one","ready":true,"activeRequest":{"id":"existing"}}]),"busy")] {
            let (settings,server)=fixture(json!({"ok":true,"pendingRequests":0}),Some(clients));
            let result=inspect(&settings,"fixture-credential").unwrap();assert_eq!(result["status"],expected);assert!(!result.to_string().contains("fixture-credential"));server.join().unwrap();
        }
    }
    #[test] fn unrelated_http_service_is_rejected_before_browser_or_submission_calls() {
        let (settings,server)=fixture(json!({"ok":true,"unrelated":true}),None);assert!(inspect(&settings,"fixture-credential").is_err());server.join().unwrap();
    }
    #[test] fn inactive_health_cannot_claim_browser_ready_and_cached_identity_must_match() {
        let (mut settings,server)=fixture(json!({"ok":false,"pendingRequests":0}),Some(json!([{"id":"one","ready":true}])));
        settings[crate::app_settings::SCOPE]=json!({"credentialRefs":{"chrome":"original-reference"}});
        let home=tempfile::tempdir().unwrap();let result=check(home.path(),&settings,"fixture-credential").unwrap();server.join().unwrap();
        assert_eq!(result["status"],"waiting_browser");assert_eq!(status(home.path(),&settings)["status"],"waiting_browser");
        settings[crate::app_settings::SCOPE]["credentialRefs"]["chrome"]=json!("replacement-reference");
        assert_eq!(status(home.path(),&settings)["status"],"offline");
    }
    #[test] fn checking_missing_program_does_not_accept_a_shell_script() {
        let dir=tempfile::tempdir().unwrap();let script=dir.path().join("node.cmd");fs::write(&script,"MZecho unsafe").unwrap();assert!(!native_exe(&script,"node.exe"));
    }
    #[test] fn workflow_restore_guard_accepts_only_absent_or_valid_empty_state_without_writing() {
        let root=tempfile::tempdir().unwrap();let data=root.path().join("data");
        ensure_no_workflow_restore(&data).unwrap();assert!(!data.exists());
        fs::create_dir_all(data.join("workflows")).unwrap();
        let path=data.join("workflows/state.json");
        let empty=b"{\"schemaVersion\":4,\"workflows\":{},\"events\":[],\"artifacts\":{}}";
        fs::write(&path,empty).unwrap();ensure_no_workflow_restore(&data).unwrap();
        assert_eq!(fs::read(&path).unwrap(),empty);assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(),1);
    }
    #[test] fn workflow_restore_guard_rejects_active_stopped_invalid_and_oversized_state_without_changes() {
        let root=tempfile::tempdir().unwrap();let data=root.path().join("data");
        fs::create_dir_all(data.join("workflows")).unwrap();let path=data.join("workflows/state.json");
        let fixtures=vec![
            json!({"schemaVersion":4,"workflows":{"existing":{"configPath":"original.json","lifecycle":"running","workflowStateSchemaVersion":3}}}).to_string(),
            json!({"schemaVersion":4,"workflows":{"existing":{"lifecycle":"stopped"}}}).to_string(),
            json!({"schemaVersion":3,"workflows":{}}).to_string(),
            json!({"schemaVersion":4,"workflows":[]}).to_string(),
            json!({"schemaVersion":4}).to_string(), "{broken".into()," ".repeat(1024*1024+1),
        ];
        for original in fixtures {fs::write(&path,&original).unwrap();assert!(ensure_no_workflow_restore(&data).is_err());assert_eq!(fs::read_to_string(&path).unwrap(),original);}
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(),1);
    }
    #[cfg(windows)]
    #[test] fn workflow_restore_guard_rejects_directory_junction_and_preserves_target() {
        let root=tempfile::tempdir().unwrap();let data=root.path().join("data");let external=root.path().join("other-data");
        fs::create_dir_all(&data).unwrap();fs::create_dir_all(&external).unwrap();
        let original=b"{\"schemaVersion\":4,\"workflows\":{}}";fs::write(external.join("state.json"),original).unwrap();
        let mut cmd=Command::new("cmd.exe");cmd.args(["/d","/c","mklink","/J"]).arg(data.join("workflows")).arg(&external);hide(&mut cmd);
        assert!(cmd.output().unwrap().status.success(),"isolated junction fixture could not be created");
        assert!(ensure_no_workflow_restore(&data).is_err());assert_eq!(fs::read(external.join("state.json")).unwrap(),original);
        assert!(fs::symlink_metadata(data.join("workflows")).is_ok());
    }
    #[cfg(unix)]
    #[test] fn workflow_restore_guard_rejects_leaf_symlink_and_preserves_target() {
        let root=tempfile::tempdir().unwrap();let data=root.path().join("data");fs::create_dir_all(data.join("workflows")).unwrap();
        let target=root.path().join("state.json");let original=b"{\"schemaVersion\":4,\"workflows\":{}}";fs::write(&target,original).unwrap();
        std::os::unix::fs::symlink(&target,data.join("workflows/state.json")).unwrap();
        assert!(ensure_no_workflow_restore(&data).is_err());assert_eq!(fs::read(&target).unwrap(),original);
        assert!(fs::symlink_metadata(data.join("workflows/state.json")).unwrap().file_type().is_symlink());
    }
}
