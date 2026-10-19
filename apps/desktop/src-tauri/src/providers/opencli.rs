//! Pinned OpenCLI browser transport. Keeps legacy Chrome snapshots on their original adapter.
use super::*;
use crate::store::s;
use std::{io::{Read,Write},process::{Command,Stdio},time::{Duration,Instant}};

pub fn enabled(c:&ProviderConfig)->bool { c.browser_transport.as_ref().is_some_and(|v|v["transport"]=="opencli/v1") }
pub fn setup_status(c:&ProviderConfig)->Value {
    setup_view(c,invoke(c,"status",None))
}
fn setup_view(c:&ProviderConfig,result:ProviderResult<Value>)->Value {
    let configured=c.browser_transport.as_ref().is_some_and(|t|!s(t,"profile").is_empty()&&!s(t,"accountKey").is_empty());
    let (state,message,clients,usable,code)=match result {
        Ok(data) if data["ready"]==true && configured=>("ready","已连接，可以到第二阶段选择资产并生成图片。".to_owned(),1,1,String::new()),
        Ok(_)=>("unconfigured","浏览器已就绪，点击“连接并检查”保存连接。".to_owned(),1,0,String::new()),
        Err(e)=>(if e.code=="BRIDGE_BUSY"{"busy"}else if e.code=="BROWSER_SELECTION_REQUIRED"{"needs_selection"}else{"waiting_browser"},e.message,0,0,e.code),
    };
    let mut value=json!({"schemaVersion":"chrome-setup/v1","installed":true,"configured":configured,"componentVersion":"OpenCLI 1.8.8 / Bridge 1.0.24","status":state,"message":message,"clients":clients,"usableClients":usable,"checkedAt":now(),"browserProfiles":[],"transport":"opencli","errorCode":code});
    if let Some(root)=std::env::var_os("LOCALAPPDATA") {let extension=PathBuf::from(root).join("Programs/Director-OpenCLI-Bridge/1.0.24/extension");if extension.join("manifest.json").is_file(){value["extensionPath"]=json!(extension);}}
    value
}
pub fn config(home:&Path,settings:&Value)->ProviderResult<(ProviderConfig,String)> {
    let node= settings["textSdkNodePath"].as_str().filter(|p|!p.is_empty()).map(PathBuf::from)
        .or_else(||std::env::var_os("ProgramFiles").map(|p|PathBuf::from(p).join("nodejs/node.exe")))
        .filter(|p|p.is_file()).ok_or_else(||ProviderError::new("OPENCLI_COMPONENT_MISSING","请先在 OpenAI 设置安装 Node 组件。"))?;
    let profile=settings["opencliProfileId"].as_str().unwrap_or("");
    let account=settings["opencliAccountKey"].as_str().unwrap_or("");
    let deployed=std::env::current_exe().ok().and_then(|p|p.parent().map(|parent|crate::store::hash_file(&p).ok().map(|(sha,_)|parent.join(".client-resources").join(sha).join("opencli/bridge.mjs")).filter(|p|p.is_file()).unwrap_or_else(||parent.join("opencli/bridge.mjs"))));
    let installed=std::env::var_os("LOCALAPPDATA").map(|p|PathBuf::from(p).join("Programs/Director-OpenCLI-Bridge/runtime-1.0.0/bridge.mjs"));
    let dev=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web-image-bridge/bridge.mjs");
    let runner=deployed.filter(|p|p.is_file()).or_else(||installed.filter(|p|p.is_file())).or_else(||(cfg!(debug_assertions)&&dev.is_file()).then_some(dev)).ok_or_else(||ProviderError::new("OPENCLI_COMPONENT_MISSING","未安装网页出图组件。"))?;
    let runtime_sha=include_str!("../../../../../web-image-bridge/runtime.sha256").trim();
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
    if v["ok"]!=true {let code=v["error"]["code"].as_str().unwrap_or("OPENCLI_ERROR");let mut e=ProviderError::new(code,match code {"BRIDGE_BUSY"=>"网页正在处理任务，请稍后再检查。","BROWSER_SELECTION_REQUIRED"=>"首次连接检测到多个浏览器。请暂时只保留本次用于出图的浏览器，再点击连接；保存绑定后可同时打开其他浏览器。","AUTH_REQUIRED"=>"请在浏览器中登录 ChatGPT，再点击“连接并检查”。","ACCOUNT_CHANGED"=>"当前登录账户与已保存账户不同，请切回原账户后连接。","OPENCLI_BROWSER_NOT_CONNECTED"=>"请打开已安装组件的浏览器，登录 ChatGPT 后再连接。","LOCAL_SERVICE_UNAVAILABLE"=>"本机连接服务暂时不可用，请点击“连接并检查”恢复。","OPENCLI_OWNED_PAGE_REQUIRED"=>"请先打开 ChatGPT 网页，再点击“连接并检查”。",_=>"网页连接检查未完成，请重试检查；已有图片任务不会重新发送。"});e.submission_unknown=v["error"]["submissionUnknown"]==true;e.auth_required=matches!(code,"AUTH_REQUIRED"|"ACCOUNT_CHANGED");return Err(e);}
    Ok(v["data"].clone())
}
pub fn capabilities(c:&ProviderConfig)->ProviderResult<Capabilities>{
    let status=invoke(c,"status",None)?;
    if status["ready"]!=true {return Err(ProviderError::new("OPENCLI_NOT_READY","网页未就绪"));}
    serde_json::from_value(json!({"provider":"chrome","version":"opencli-image/v1","source":"opencli_web","checkedAt":now(),"canCancel":false,"canReconcileUnknown":true,"raw":status,"models":[{"id":"chatgpt-image","kind":"image","adapted":true,"ratios":["1:1","16:9","9:16","4:3","3:4","3:2","2:3","21:9"],"resolutions":["native"],"allowedDurations":[],"images":10,"videos":0,"audios":0,"totalReferences":10,"requiredImages":0,"raw":{"transport":"opencli/v1","outputsPerAsset":1,"requestedRatioIsPromptInstruction":true}}]})).map_err(|e|ProviderError::new("OPENCLI_PACKET",e.to_string()))
}
pub fn prepare(c:&ProviderConfig,r:&GenerationRequest)->ProviderResult<PreparedRequest>{
    if r.kind!="image" {return Err(ProviderError::new("IMAGE_ONLY","网页渠道仅支持图片"));}
    let data=invoke(c,"prepare",Some(r))?;
    Ok(PreparedRequest{provider:"chrome".into(),request:r.clone(),capability_version:"opencli-image/v1".into(),upload_receipts:vec![],wire:json!({"transport":"opencli/v1","requestHash":data["requestHash"],"profile":data["profile"],"accountKey":data["accountKey"]}),estimated_cost:None,prepared_at:now()})
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
    let v=invoke(c,"query",Some(&p.request))?;let mut result:QueryResult=serde_json::from_value(v).map_err(|e|ProviderError::new("OPENCLI_PACKET",e.to_string()))?;result.actual_cost=None;Ok(result)
}
pub fn reconcile(c:&ProviderConfig,id:&str)->ProviderResult<QueryResult>{
    valid_id(id)?;
    let root=PathBuf::from(c.browser_transport.as_ref().unwrap()["stateRoot"].as_str().unwrap_or(""));
    let saved:Value=serde_json::from_slice(&fs::read(root.join(id).join("request.json")).map_err(|_|ProviderError::new("ORIGINAL_REQUEST_REQUIRED","未找到原网页请求，禁止重发"))?).map_err(|_|ProviderError::new("ORIGINAL_REQUEST_REQUIRED","原网页请求损坏"))?;
    let r:GenerationRequest=serde_json::from_value(saved["request"].clone()).map_err(|_|ProviderError::new("ORIGINAL_REQUEST_REQUIRED","原冻结请求缺失"))?;
    if r.request_id!=id{return Err(ProviderError::new("IMAGE_PROVENANCE","原请求身份不匹配"));}
    let v=invoke(c,"query",Some(&r))?;
    serde_json::from_value(v).map_err(|_|ProviderError::new("OPENCLI_PACKET","原网页回执无效"))
}
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
