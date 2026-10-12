use super::*;
use reqwest::blocking::Client;
use crate::store::s;

#[cfg(test)]
#[path = "chrome_background_tests.rs"]
mod background_tests;
#[cfg(test)]
#[path = "chrome_control_page_tests.rs"]
mod control_page_tests;
#[cfg(test)]
#[path = "chrome_prepared_refresh_tests.rs"]
mod prepared_refresh_tests;
#[cfg(test)]
#[path="chrome_conversation_tests.rs"]
mod conversation_tests;
#[path="chrome_execution_proof.rs"]
mod execution_proof;
#[path="chrome_conversation.rs"]
pub(super) mod conversation;
#[path="chrome_upload_settlement.rs"]
mod upload_settlement;

fn json_request(
    config: &ProviderConfig,
    client: &Client,
    route: &str,
    body: Option<&Value>,
    submitting: bool,
) -> ProviderResult<Value> {
    let url = format!("{}{route}", wire::base(config, true)?);
    let token = wire::credential(config, true)?;
    let request = if let Some(body) = body {
        client.post(url).json(body)
    } else {
        client.get(url)
    };
    let response = request.bearer_auth(token).send().map_err(|_| {
        if submitting {
            ProviderError::uncertain("Chrome 发送回执未确认；后续只能查询原关联码。")
        } else {
            ProviderError::new("BRIDGE_OFFLINE", "Chrome 桥接离线；已提交任务不会重发。")
        }
    })?;
    wire::read_json(response, submitting).map_err(|mut error| {
        let phase=match route {"/browser/tabs/open"=>"tab_open","/browser/recover-latest"=>"empty_preflight","/files/from-path"=>"reference_upload",_=>"bridge_read"};
        let original=error.code.clone();
        if !submitting && (["STANDALONE_COMMAND_BUSY","COMMAND_BUSY","CLIENT_BUSY","BRIDGE_BUSY"].contains(&error.code.as_str())
            || error.message.to_ascii_lowercase().contains("unreleased standalone")
            || error.message.to_ascii_lowercase().contains("standalone command") && error.message.to_ascii_lowercase().contains("busy")) {
            error.code="BRIDGE_BUSY".into();
        }
        let http=error.context.as_ref().and_then(|context|context.get("httpStatus")).cloned().unwrap_or(Value::Null);
        error.context=Some(json!({"phase":phase,"transportCode":original,"httpStatus":http}));
        error
    })
}
fn clients(config: &ProviderConfig, client: &Client) -> ProviderResult<Vec<Value>> {
    let body = json_request(config, client, "/browser/clients", None, false)?;
    if body.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(ProviderError::new(
            "BRIDGE_STATE",
            "无法核对浏览器连接状态。",
        ));
    }
    body.get("clients")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| ProviderError::new("BRIDGE_STATE", "浏览器连接列表格式无效。"))
}
fn usable(value: &Value) -> bool {
    value.get("ready").and_then(Value::as_bool) == Some(true)
        && value.get("compatible").and_then(Value::as_bool) != Some(false)
        && value.get("quarantined").and_then(Value::as_bool) != Some(true)
        && value.get("activeRequest").is_none_or(Value::is_null)
        && value["releasePending"] != true && value["releaseState"] != "pending"
        && !crate::chrome_setup::client_command_busy(value)
}

fn control_launch(binding:&crate::chrome_setup::BrowserBinding)->String {
    format!("bridge-director-control-{}",binding.profile_id)
}
fn same_browser_runtime(value:&Value,source:&Value,binding:&crate::chrome_setup::BrowserBinding)->bool {
    value["transport"]=="extension" && value["browserIncognito"]==false
        && value["browserProfileId"]==binding.profile_id && value["extensionBundleId"]==binding.bundle_id
        && source["backgroundEpoch"].as_str().is_some_and(|s|!s.is_empty()) && value["backgroundEpoch"]==source["backgroundEpoch"]
        && source["serverInstanceId"].as_str().is_some_and(|s|!s.is_empty()) && value["serverInstanceId"]==source["serverInstanceId"]
}
fn explicit_idle(value:&Value)->bool {usable(value) && value.get("activeRequest")==Some(&Value::Null)}
fn same_idle_owned_source(value:&Value,source:&Value,binding:&crate::chrome_setup::BrowserBinding)->bool {
    value["id"]==source["id"] && value["launchToken"]==source["launchToken"] && value["url"]==source["url"]
        && same_browser_runtime(value,source,binding) && explicit_idle(value)
        && value["focused"]==false && value["visibilityState"]=="hidden"
}
fn compatible_profile_clients(available:&[Value],source:&Value,binding:&crate::chrome_setup::BrowserBinding)->bool {
    // Reject a copied profile even if the second runtime is busy or not ready.
    available.iter().filter(|c|c["browserProfileId"]==binding.profile_id)
        .all(|c|same_browser_runtime(c,source,binding))
}
fn keep_profile_control(config:&ProviderConfig,client:&Client,source:&Value,available:&[Value],binding:&crate::chrome_setup::BrowserBinding)->ProviderResult<bool> {
    let launch=control_launch(binding);
    let controls:Vec<_>=available.iter().filter(|c|c["launchToken"]==launch).collect();
    if controls.len()>1 || controls.iter().any(|c|!same_browser_runtime(c,source,binding) || !explicit_idle(c)) {return Ok(false);}
    if available.iter().any(|c|c["id"]!=source["id"] && same_browser_runtime(c,source,binding) && explicit_idle(c)) {return Ok(true);}
    if !controls.is_empty(){return Ok(false);}
    // The controller-specific extension operation durably holds uncertain opens;
    // serialize local checks too, without storing or changing a generation job.
    static OPENS:std::sync::OnceLock<std::sync::Mutex<()>>=std::sync::OnceLock::new();
    let _guard=OPENS.get_or_init(Default::default).lock().map_err(|_|ProviderError::new("TAB_CONTROL_UNCONFIRMED","控制页创建状态未确认，保留完成页。"))?;
    // Re-read under the local open guard. An earlier completed cleanup may have
    // already established the one control page since our first client snapshot.
    let latest=clients(config,client)?;
    if !compatible_profile_clients(&latest,source,binding){return Ok(false);}
    let controls:Vec<_>=latest.iter().filter(|c|c["launchToken"]==launch).collect();
    if !controls.is_empty(){return Ok(controls.len()==1 && same_browser_runtime(controls[0],source,binding) && explicit_idle(controls[0]));}
    let originals:Vec<_>=latest.iter().filter(|c|c["id"]==source["id"]).collect();
    if originals.len()!=1 || !same_idle_owned_source(originals[0],source,binding) {return Ok(false);}
    let opened=match json_request(config,client,"/browser/tabs/open",Some(&json!({"url":"https://chatgpt.com/","sourceClientId":source["id"],"active":false,"select":false,"launchToken":launch,"timeoutMs":8000,"allowSystemFallback":false})),false){Ok(value)=>value,Err(_)=>return Ok(false)};
    let page=&opened["client"];
    if opened["ok"]!=true || page["launchToken"]!=launch || page["id"]==source["id"]
        || !page["id"].as_str().is_some_and(|s|!s.is_empty()) || page["url"]!="https://chatgpt.com/"
        || !same_browser_runtime(page,source,binding) || !explicit_idle(page)
        || page["focused"]!=false || page["visibilityState"]!="hidden"
        || opened["tabState"]["active"]!=false || opened["tabState"]["autoDiscardable"]!=false || opened["tabState"]["discarded"]!=false {return Ok(false);}
    let verified=clients(config,client)?;
    if !compatible_profile_clients(&verified,source,binding){return Ok(false);}
    let controls:Vec<_>=verified.iter().filter(|c|c["launchToken"]==launch).collect();
    let originals:Vec<_>=verified.iter().filter(|c|c["id"]==source["id"]).collect();
    if controls.len()!=1 || controls[0]["id"]!=page["id"] || !same_browser_runtime(controls[0],source,binding) || !explicit_idle(controls[0])
        || originals.len()!=1 || !same_idle_owned_source(originals[0],source,binding) {return Ok(false);}
    Ok(true)
}

pub(super) fn release_completed_tab(config: &ProviderConfig, prepared: &PreparedRequest, result: &QueryResult) -> ProviderResult<Value> {
    let task = result.task_id.as_str();
    let proof = &result.raw["nativeProof"];
    if prepared.provider != "chrome" || prepared.request.kind != "image" || prepared.request.request_id != task
        || result.status != "succeeded" || result.result_urls.is_empty()
        || proof["status"] != "matched_complete" || proof["proofVersion"] != 2 || proof["requestId"] != task
        || proof["promptSha256"] != prepared.wire["metadata"]["promptSha256"] {
        return Err(ProviderError::new("TAB_CLEANUP_IDENTITY", "任务身份未核对，保留工作页。"));
    }
    let original_conversation = proof["conversationId"].as_str().filter(|s| !s.is_empty())
        .ok_or_else(|| ProviderError::new("TAB_CLEANUP_IDENTITY", "原会话证明缺失，保留页面。"))?;
    let recovery=&result.raw["readOnlyRecovery"];
    let recovered=!recovery.is_null();
    if recovered && (!conversation::locator_valid(config,prepared,&recovery["locator"])
        || recovery["locator"]["conversationId"]!=original_conversation
        || recovery["locator"]["userMessageId"]!=proof["userMessageId"]
        || !s(&recovery["client"],"launchToken").strip_prefix("bridge-director-desktop-").is_some_and(|v|uuid::Uuid::parse_str(v).is_ok())
        || recovery["client"]["launchToken"]==format!("bridge-director-desktop-{task}")) {
        return Err(ProviderError::new("TAB_CLEANUP_IDENTITY","恢复工作页的原会话证明不完整，保留页面。"));
    }
    let source = if recovered {recovery["client"]["id"].as_str()}else{prepared.wire["sourceClientId"].as_str()}.filter(|s| !s.is_empty())
        .ok_or_else(|| ProviderError::new("TAB_CLEANUP_IDENTITY", "原工作页身份缺失，保留页面。"))?;
    let client = wire::client(12)?;
    let available = clients(config, &client)?;
    let matches: Vec<_> = available.iter().filter(|c| c["id"] == source).collect();
    if matches.is_empty() { return Ok(json!({"status":"already_closed"})); }
    if matches.len() != 1 { return Err(ProviderError::new("TAB_CLEANUP_IDENTITY", "工作页身份不唯一，保留页面。")); }
    let owned = matches[0];
    let launch = if recovered {s(&recovery["client"],"launchToken").to_owned()}else{format!("bridge-director-desktop-{task}")};
    if owned["launchToken"] != launch {
        return Ok(json!({"status":"retained","reason":"ownership_changed"}));
    }
    if !explicit_idle(owned) {
        return Ok(json!({"status":"retained","reason":"idle_not_proven"}));
    }
    if owned["focused"] != false || owned["visibilityState"] != "hidden" {
        return Ok(json!({"status":"retained","reason":"user_viewing_or_visibility_unknown"}));
    }
    let url = owned_conversation(owned)?;
    if url != original_conversation {
        return Ok(json!({"status":"retained","reason":"conversation_changed"}));
    }
    let Some(binding)=config.chrome_binding.as_ref() else {return Ok(json!({"status":"retained","reason":"last_connected_page"}));};
    if crate::chrome_setup::browser_binding(&crate::chrome_setup::binding_settings(&wire::base(config,true)?,Some(binding))).is_err()
        || prepared.wire.get("browserBinding").is_some_and(|frozen|serde_json::to_value(binding).ok().as_ref()!=Some(frozen))
        || !same_browser_runtime(owned,owned,binding) || !compatible_profile_clients(&available,owned,binding) {
        return Ok(json!({"status":"retained","reason":"ownership_changed"}));
    }
    if !keep_profile_control(config,&client,owned,&available,binding)? {
        return Ok(json!({"status":"retained","reason":"last_connected_page"}));
    }
    let response = json_request(config, &client, "/browser/tabs/close", Some(&json!({
        "sourceClientId":source,"expectedLaunchToken":launch,
        "expectedUrl":format!("https://chatgpt.com/c/{url}"),"timeoutMs":8000
    })), false)?;
    if response["ok"] != true {
        return Err(ProviderError::new("TAB_CLEANUP_UNCONFIRMED", "关闭工作页未确认，已生成图片仍已保存。"));
    }
    Ok(json!({"status":"close_requested","sourceClientId":source}))
}

pub(super) fn capabilities(config: &ProviderConfig) -> ProviderResult<Capabilities> {
    let health = crate::chrome_setup::inspect(
        &crate::chrome_setup::binding_settings(&wire::base(config,true)?,config.chrome_binding.as_ref()),
        &wire::credential(config,true)?,
    ).map_err(|message|ProviderError::new("BRIDGE_OFFLINE",message))?;
    if health["status"] == "busy" || health["status"] == "waiting_browser" {
        return Err(ProviderError::new("BRIDGE_BUSY", if health["status"]=="waiting_browser" {
            "等待已绑定的 Chrome 后台连接恢复，图片保持排队；未开始生成。"
        } else {"浏览器正在执行已有任务，图片保持排队等待。"}));
    }
    if health["status"] != "ready" {
        return Err(ProviderError::new("BRIDGE_BROWSER_NOT_READY",health["message"].as_str().unwrap_or("Chrome 网页尚未就绪。")));
    }
    Ok(Capabilities {
        provider: "chrome".into(),
        version: "chrome-image-proof-v2".into(),
        source: "bridge_protocol".into(),
        checked_at: now(),
        can_cancel: false,
        can_reconcile_unknown: true,
        models: vec![ModelCapability {
            id: "chatgpt-image".into(),
            kind: "image".into(),
            adapted: true,
            min_duration: None,
            max_duration: None,
            allowed_durations: vec![],
            ratios: ["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "21:9"]
                .iter()
                .map(|x| x.to_string())
                .collect(),
            resolutions: vec!["native".into()],
            images: 10,
            videos: 0,
            audios: 0,
            total_references: Some(10),
            required_images: 0,
            price: None,
            billing: None,
            currency: None,
            video_price_multiplier: None,
            raw: json!({"resolutionGuarantee":false,"requiresNativeProofVersion":2,"requestedRatioIsPromptInstruction":true}),
        }],
        raw: health,
    })
}

fn owned_client(config: &ProviderConfig, client: &Client, id: &str) -> ProviderResult<Value> {
    owned_client_for(config, client, id, false)
}
fn owned_client_for(config: &ProviderConfig, client: &Client, id: &str, recovering: bool) -> ProviderResult<Value> {
    original_client_for(config, client, id, recovering, None)
}
fn original_client_for(config: &ProviderConfig, client: &Client, id: &str, recovering: bool, original_source: Option<&str>) -> ProviderResult<Value> {
    let token = format!("bridge-director-desktop-{id}");
    let legacy_token = format!("director-desktop-{id}");
    let matches: Vec<_> = clients(config, client)?
        .into_iter()
        .filter(|c| {
            let launch = c["launchToken"].as_str().unwrap_or("");
            launch == token || launch == legacy_token || recovering
                && original_source.is_some_and(|source| !source.is_empty() && c["id"] == source)
                && (launch.is_empty() || launch.starts_with("bridge-reload-"))
        })
        .collect();
    let ready = matches.len() == 1 && (usable(&matches[0]) || recovering
        && matches[0]["ready"] == true && matches[0]["compatible"] != false
        && matches[0]["quarantined"] != true && matches[0]["activeRequest"]["requestId"] == id);
    if !ready {
        return Err(ProviderError::new(
            "OWNED_TAB_UNAVAILABLE",
            "本任务专属标签页尚未就绪或身份不唯一；不会重复打开或发送。",
        ));
    }
    Ok(matches[0].clone())
}

pub(super) fn prepare(
    config: &ProviderConfig,
    request: &GenerationRequest,
) -> ProviderResult<PreparedRequest> {
    prepare_inner(config,request,None,None)
}
pub(super) fn prepare_observed(config:&ProviderConfig,request:&GenerationRequest,observer:&PrepareObserver<'_>) -> ProviderResult<PreparedRequest> {
    prepare_inner(config,request,Some(observer),None)
}
pub(super) fn recover_prepare(config:&ProviderConfig,request:&GenerationRequest,proof:&Value,observer:&PrepareObserver<'_>) -> ProviderResult<PreparedRequest> {
    prepare_inner(config,request,Some(observer),Some(proof))
}
fn prepare_inner(config:&ProviderConfig,request:&GenerationRequest,observer:Option<&PrepareObserver<'_>>,recovery:Option<&Value>) -> ProviderResult<PreparedRequest> {
    if !request.options.as_object().is_some_and(|m| {
        m.iter()
            .all(|(key, value)| key == "count" && value.as_u64() == Some(1))
    }) {
        return Err(ProviderError::new(
            "UNSUPPORTED_OPTION",
            "Chrome 渠道未适配这些额外参数，不会静默忽略。",
        ));
    }
    let caps = capabilities(config)?;
    let model = &caps.models[0];
    if request.model != model.id {
        return Err(ProviderError::new(
            "MODEL_UNAVAILABLE",
            "Chrome 图片渠道使用当前网页模型，模型 ID 为 chatgpt-image。",
        ));
    }
    check_model(model, request)?;
    if caps
        .raw
        .get("pendingRequests")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0
        || caps.raw.get("activeRequests").is_some_and(|x| {
            x.as_array().is_some_and(|a| !a.is_empty()) || x.as_u64().is_some_and(|n| n > 0)
        })
    {
        return Err(ProviderError::new(
            "BRIDGE_BUSY",
            "浏览器正在处理其他请求，请稍后准备。",
        ));
    }
    let client = wire::client(45)?;
    let mut available = clients(config, &client)?;
    let launch = format!("bridge-director-desktop-{}", request.request_id);
    let legacy_launch = format!("director-desktop-{}", request.request_id);
    // A normal prepare never guesses whether an existing page was submitted.
    if recovery.is_none() && available
        .iter()
        .any(|c| matches!(c.get("launchToken").and_then(Value::as_str), Some(t) if t==launch || t==legacy_launch))
    {
        return Err(ProviderError::new(
            "EXISTING_TAB_REQUIRES_RECONCILIATION",
            "任务标签页已存在，请核对原关联码；不会重复创建或发送。",
        ));
    }
    let binding=config.chrome_binding.as_ref().ok_or_else(||ProviderError::new("BROWSER_BINDING_REQUIRED","请在软件设置连接并绑定 Chrome 执行浏览器；旧任务只核对原页面，不重新发送。"))?;
    crate::chrome_setup::browser_binding(&crate::chrome_setup::binding_settings(&wire::base(config,true)?,Some(binding)))
        .map_err(|message|ProviderError::new("BROWSER_BINDING_REQUIRED",message))?;
    let mut opener=crate::chrome_setup::profile_execution_client(&available,binding)
        .map_err(|message|ProviderError::new("BROWSER_SELECTION_REQUIRED",message))?.clone();
    let live_execution_required=opener["transportHealth"]["kind"]=="connect_snapshot";
    if live_execution_required && !execution_proof::owned_root(&opener,binding) {
        available=execution_proof::bootstrap_control(config,&client,&opener,binding)?;
        opener=crate::chrome_setup::profile_execution_client(&available,binding).map_err(|message|ProviderError::new("BROWSER_SELECTION_REQUIRED",message))?.clone();
    }
    let opener_id = opener
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::new("BRIDGE_STATE", "浏览器身份无效。"))?;
    let mut intent=Value::Null;
    if live_execution_required {
        execution_proof::fresh_idle(config,&client,&opener,binding,request)?;
        if recovery.is_some() {if let Some(observer)=observer {observer(&json!({"version":"chrome-prepare-execution-start/v1","requestId":request.request_id,"requestHash":hex_hash(json!(request).to_string().as_bytes()),"configSha256":hex_hash(json!(config).to_string().as_bytes())}))?;}}
    }
    let tab=if let Some(recovery)=recovery {
        recover_blank_tab(config,&client,&available,binding,request,recovery,observer.ok_or_else(||ProviderError::new("CHROME_PREPARE_PROOF_REQUIRED","恢复缺少持久化证据回调"))?,&mut intent)?
    } else {
    intent=prepare_intent(request,binding,&opener,"new_prepare",Value::Null)?;
    if let Some(observer)=observer {observer(&intent)?;}
    let opened = json_request(
        config,
        &client,
        "/browser/tabs/open",
        Some(
            &json!({"url":"https://chatgpt.com/","sourceClientId":opener_id,"active":false,"select":false,"launchToken":launch,"timeoutMs":30000,"allowSystemFallback":false}),
        ),
        false,
    ).map_err(|mut error|{if error.code!="BRIDGE_BUSY" && !error.auth_required {let transport=error.code.clone();error.code="CHROME_PREPARE_OPEN_UNCONFIRMED".into();let mut context=error.context.take().unwrap_or_else(||json!({}));context["phase"]=json!("tab_open");context["transportCode"]=json!(transport);context["sameRequestRecoveryOnly"]=json!(true);error.context=Some(context);}error})?;
    let tab = opened
        .get("client")
        .ok_or_else(|| ProviderError::new("TAB_OPEN_UNKNOWN", "标签页创建回执未确认。"))?;
    if opened.get("ok").and_then(Value::as_bool) != Some(true)
        || tab.get("launchToken").and_then(Value::as_str) != Some(launch.as_str())
        || tab["browserProfileId"] != binding.profile_id || tab["browserIncognito"] != false
        || !same_browser_runtime(tab,&opener,binding) || tab["url"]!="https://chatgpt.com/"
        || tab["focused"]!=false || tab["visibilityState"]!="hidden"
        || !usable(tab)
    {
        return Err(ProviderError::new(
            "TAB_OPEN_UNKNOWN",
            "任务标签页创建身份未确认。",
        ));
    }
    tab.clone()
    };
    let source_id = tab
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::new("TAB_OPEN_UNKNOWN", "任务标签页没有身份。"))?;
    let empty = json_request(
        config,
        &client,
        "/browser/recover-latest",
        Some(
            &json!({"sourceClientId":source_id,"index":1,"timeoutMs":10000,"reconcileConversation":{"mode":"image_preflight"}}),
        ),
        false,
    )?;
    let result = &empty["result"];
    let proof = &result["reconciliation"];
    if result["sourceClientId"] != source_id
        || result["url"] != "https://chatgpt.com/"
        || proof["source"] != "browser-empty-preflight"
        || proof["status"] != "matched_empty"
        || proof["version"] != 1
        || proof["proofVersion"] != 2
    {
        return Err(ProviderError::new(
            "EMPTY_PREFLIGHT",
            "Chrome 新会话未通过原生空白检查，请更新桥接扩展。",
        ));
    }
    if recovery.is_some() {
        let verified=clients(config,&client)?;
        let owners:Vec<_>=verified.iter().filter(|page|page["launchToken"]==launch).collect();
        if owners.len()!=1 || !same_idle_owned_source(owners[0],&tab,binding) || !compatible_profile_clients(&verified,&tab,binding) {
            return Err(ProviderError::new("CHROME_PREPARE_IDENTITY_HELD","原空白页身份在核验期间变化，未准备或发送"));
        }
    }
    let mut attachments = Vec::new();
    let mut receipts = Vec::new();
    for r in sorted_references(request) {
        let size = fs::metadata(&r.path)
            .map_err(|_| ProviderError::new("MEDIA_MISSING", "参考图不存在。"))?
            .len();
        if size > 20 * 1024 * 1024 {
            return Err(ProviderError::new(
                "MEDIA_SIZE",
                "Chrome 参考图不超过 20 MiB。",
            ));
        }
        let ext = r
            .path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => {
                return Err(ProviderError::new(
                    "MEDIA_FORMAT",
                    "Chrome 参考图仅支持 PNG/JPG/WebP。",
                ))
            }
        };
        let uploaded = json_request(
            config,
            &client,
            "/files/from-path",
            Some(&json!({"path":r.path,"name":reference_upload_name(&request.request_id,r.order,&ext),"mime":mime})),
            false,
        )?;
        let file_id = uploaded["file"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ProviderError::new("REFERENCE_UPLOAD", "浏览器参考图上传未确认。"))?;
        if uploaded["ok"] != true {
            return Err(ProviderError::new(
                "REFERENCE_UPLOAD",
                "浏览器参考图上传未确认。",
            ));
        }
        attachments.push(file_id.to_string());
        receipts.push(UploadReceipt {
            media_id: r.media_id,
            sha256: r.sha256,
            order: r.order,
            kind: r.kind,
            url: None,
            file_id: Some(file_id.into()),
            uploaded_at: now(),
            upstream_read_confirmed: false,
        });
    }
    let message=format!("{}\n\n画面比例：{}。请生成 1 张原图。",request.prompt,request.ratio);
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut frozen = request.clone();
    frozen.references = sorted_references(request);
    Ok(PreparedRequest {
        provider: "chrome".into(),
        request: frozen,
        capability_version: caps.version,
        upload_receipts: receipts,
        wire: json!({"id":request.request_id,"message":message,"correlationVersion":"conversation/v1","attachments":attachments,"newSession":false,"sourceClientId":source_id,"browserBinding":binding,"prepareRuntime":intent,"metadata":{"jobId":request.request_id,"promptSha256":hex_hash(normalized.as_bytes())},"emptyPreflight":proof}),
        estimated_cost: None,
        prepared_at: now(),
    })
}

fn prepare_intent(request:&GenerationRequest,binding:&crate::chrome_setup::BrowserBinding,source:&Value,mode:&str,audit:Value) -> ProviderResult<Value> {
    if !same_browser_runtime(source,source,binding) || s(source,"id").is_empty() {
        return Err(ProviderError::new("CHROME_PREPARE_IDENTITY_HELD","首次准备缺少完整浏览器运行身份"));
    }
    let request_hash=hex_hash(json!(request).to_string().as_bytes());
    Ok(json!({"version":"chrome-prepare-intent/v1","mode":mode,"requestId":request.request_id,"requestHash":request_hash,
        "launchToken":format!("bridge-director-desktop-{}",request.request_id),"browserBinding":binding,"sourceClientId":source["id"],
        "runtime":{"browserProfileId":source["browserProfileId"],"extensionBundleId":source["extensionBundleId"],"backgroundEpoch":source["backgroundEpoch"],"serverInstanceId":source["serverInstanceId"],"transport":"extension","browserIncognito":false},
        "createdAt":now(),"legacyNoSubmitAudit":audit}))
}
fn recover_blank_tab(config:&ProviderConfig,client:&Client,available:&[Value],binding:&crate::chrome_setup::BrowserBinding,request:&GenerationRequest,recovery:&Value,observer:&PrepareObserver<'_>,intent:&mut Value) -> ProviderResult<Value> {
    let held=||ProviderError::new("CHROME_PREPARE_IDENTITY_HELD","原图片准备证据不足；保留原任务，不新开页或发送");
    let launch=format!("bridge-director-desktop-{}",request.request_id);
    let hash=hex_hash(json!(request).to_string().as_bytes());
    if recovery["version"]!="chrome-prepare-recovery/v1" || recovery["requestHash"]!=hash || recovery["noLocalSubmit"]!=true {return Err(held());}
    let owners:Vec<_>=available.iter().filter(|page|page["launchToken"]==launch).collect();
    if owners.is_empty() {
        if !recovery["intent"].is_null() {return Err(ProviderError::new("CHROME_PREPARE_WAIT_ORIGINAL","原开页还未形成唯一可用页；只等待同关联码，不再次开页"));}
        return legacy_first_open(config,client,available,binding,request,recovery,observer,intent);
    }
    if owners.len()!=1 {return Err(held());}
    let page=owners[0];
    if page["url"]!="https://chatgpt.com/" || page["focused"]!=false || page["visibilityState"]!="hidden" || !explicit_idle(page)
        || !same_browser_runtime(page,page,binding) || !compatible_profile_clients(available,page,binding) {return Err(held());}
    let frozen=&recovery["intent"];
    if !frozen.is_null() {
        if frozen["version"]!="chrome-prepare-intent/v1" || frozen["requestId"]!=request.request_id || frozen["requestHash"]!=hash
            || frozen["launchToken"]!=launch || frozen["browserBinding"]!=json!(binding)
            || !same_browser_runtime(page,&frozen["runtime"],binding) {return Err(held());}
        *intent=frozen.clone();
    } else if recovery["legacyNoSubmitAudit"]["version"]!="chrome-legacy-no-submit/v1"
        || recovery["legacyNoSubmitAudit"]["snapshotSha256"].as_str().is_none_or(|v|v.len()!=64)
        || recovery["legacyNoSubmitAudit"]["stateChainProven"]!=true {return Err(held());}
    // A 404 alone is insufficient. Require the exact known endpoint's not-found
    // response, authoritative canonical absence, local zero-submit history and
    // the original launch page's independent native empty preflight below.
    prove_bridge_prepare_absence(config,client,request)?;
    if frozen.is_null() {
        let mut audit=recovery["legacyNoSubmitAudit"].clone();
        audit["bridgeNoTurn"]=json!(true);audit["bridgeCanonicalAbsent"]=json!(true);
        // This is explicitly the FIRST completed prepare runtime, not an
        // invented assertion that an old epoch existed or remained unchanged.
        *intent=prepare_intent(request,binding,page,"legacy_first_prepare",audit)?;
        observer(intent)?;
    }
    Ok(page.clone())
}
fn prove_bridge_prepare_absence(config:&ProviderConfig,client:&Client,request:&GenerationRequest)->ProviderResult<()> {
    bridge_prepare_absence_proof(config,client,request).map(|_|())
}
fn bridge_prepare_absence_proof(config:&ProviderConfig,client:&Client,request:&GenerationRequest)->ProviderResult<Value> {
    let held=||ProviderError::new("CHROME_PREPARE_IDENTITY_HELD","原请求存在执行记录或无可信未提交证明，未开页或发送");
    match json_request(config,client,&format!("/turns/{}",request.request_id),None,false) {
        Err(error) if error.context.as_ref().and_then(|c|c.get("httpStatus"))==Some(&json!(404))
            && error.message==format!("Turn not found: {}",request.request_id)=>{},
        _=>return Err(held()),
    }
    let canonical=json_request(config,client,&format!("/diagnostics/request-state?requestId={}",request.request_id),None,false)?;
    let tracked=&canonical["requests"];
    if canonical["ok"]!=true || canonical["authoritative"]!=true || !tracked.is_object()
        || tracked.get("state")!=Some(&Value::Null)
        || !tracked["history"].as_array().is_some_and(Vec::is_empty)
        || !tracked["deadlines"].as_array().is_some_and(Vec::is_empty) {return Err(held());}
    Ok(json!({"requestId":request.request_id,"turnStatus":404,"turnDetail":format!("Turn not found: {}",request.request_id),"canonical":canonical,"observedAt":now()}))
}

/// Refresh only an already frozen, provably unsubmitted request on its SAME
/// owned empty page. Caller must additionally prove its local no-submit chain
/// and persist this result before submit; this adapter cannot inspect job state.
/// Never opens a page, uploads a reference, or sends a generation request.
pub(super) fn refresh_unsubmitted_prepared(config:&ProviderConfig,prepared:&PreparedRequest)->ProviderResult<PreparedRequest> {
    let held=||ProviderError::new("CHROME_PREPARE_IDENTITY_HELD","原准备请求的未提交与原空白页身份未完整核对，保留原快照，未发送");
    let request=&prepared.request;
    let binding=config.chrome_binding.as_ref().ok_or_else(held)?;
    let frozen=&prepared.wire["prepareRuntime"];
    let launch=format!("bridge-director-desktop-{}",request.request_id);
    let hash=hex_hash(json!(request).to_string().as_bytes());
    let source_id=s(&prepared.wire,"sourceClientId");
    if config.kind!="chrome" || prepared.provider!="chrome" || request.kind!="image" || request.model!="chatgpt-image"
        || !uuid::Uuid::parse_str(&request.request_id).is_ok_and(|id|id.get_version_num()==4 && id.to_string()==request.request_id)
        || source_id.is_empty() || prepared.wire["id"]!=request.request_id || prepared.wire["newSession"]!=false
        || frozen["version"]!="chrome-prepare-intent/v1" || frozen["requestId"]!=request.request_id || frozen["requestHash"]!=hash
        // new_prepare/legacy_first_open freeze the opener, not the resulting
        // work page. Preserve that original opener identity; never retarget it.
        || frozen["launchToken"]!=launch || s(frozen,"sourceClientId").is_empty() || frozen["browserBinding"]!=json!(binding)
        || !["new_prepare","legacy_first_open","legacy_first_prepare"].contains(&s(frozen,"mode"))
        || frozen["mode"]=="legacy_first_prepare" && frozen["sourceClientId"]!=source_id
        || prepared.wire["browserBinding"]!=json!(binding) || !same_browser_runtime(&frozen["runtime"],&frozen["runtime"],binding)
        || !prepared.wire["attachments"].is_array() || !prepared.wire["message"].is_string() {return Err(held());}
    crate::chrome_setup::browser_binding(&crate::chrome_setup::binding_settings(&wire::base(config,true)?,Some(binding))).map_err(|_|held())?;
    let client=wire::client(15)?;
    let check_page=|available:&[Value],expected:Option<&Value>|->ProviderResult<Value>{
        let owners:Vec<_>=available.iter().filter(|p|p["launchToken"]==launch).collect();
        let sources:Vec<_>=available.iter().filter(|p|p["id"]==source_id).collect();
        if owners.len()!=1 || sources.len()!=1 || owners[0]!=sources[0] {return Err(held());}
        let page=owners[0];
        if page["url"]!="https://chatgpt.com/" || page["focused"]!=false || page["visibilityState"]!="hidden" || !explicit_idle(page)
            || !same_browser_runtime(page,page,binding) || !compatible_profile_clients(available,page,binding)
            || !page["browserTabId"].as_u64().is_some_and(|id|id>0) || s(page,"contentEpoch").is_empty()
            || available.iter().filter(|p|p["browserProfileId"]==binding.profile_id).any(|p|!explicit_idle(p))
            || expected.is_some_and(|old|!same_idle_owned_source(page,old,binding) || page["browserTabId"]!=old["browserTabId"] || page["contentEpoch"]!=old["contentEpoch"]) {return Err(held());}
        Ok(page.clone())
    };
    let source=check_page(&clients(config,&client)?,None)?;
    let before=bridge_prepare_absence_proof(config,&client,request)?;
    let empty=json_request(config,&client,"/browser/recover-latest",Some(&json!({"sourceClientId":source_id,"index":1,"timeoutMs":10000,"reconcileConversation":{"mode":"image_preflight"}})),false)?;
    let result=&empty["result"];let proof=&result["reconciliation"];
    if empty["ok"]!=true || result["sourceClientId"]!=source_id || result["url"]!="https://chatgpt.com/"
        || proof["source"]!="browser-empty-preflight" || proof["status"]!="matched_empty" || proof["version"]!=1 || proof["proofVersion"]!=2 {return Err(held());}
    check_page(&clients(config,&client)?,Some(&source))?;
    let after=bridge_prepare_absence_proof(config,&client,request)?;
    let native=execution_proof::prove_idle(config,&client,&source,binding,request)?;
    let native_age=native["inventory"]["sourceExecutionProof"]["observedAt"].as_i64().map(|at|chrono::Utc::now().timestamp_millis()-at);
    if native["status"]!="matched" || native["match"]["tabId"]!=source["browserTabId"]
        || native["match"]["url"]!="https://chatgpt.com/" || native["match"]["launchToken"]!=launch || native["match"]["connected"]!=true
        || !native_age.is_some_and(|age|(0..=5000).contains(&age)) {return Err(held());}
    let mut refreshed=prepared.clone();
    let at=now();
    for key in ["backgroundEpoch","serverInstanceId","contentEpoch"] {refreshed.wire["prepareRuntime"]["runtime"][key]=source[key].clone();}
    // Keep the original request, message, files and receipts byte-for-byte in
    // their serialized representation; the independent audit key is local only.
    let mut history=prepared.wire.get("preparedRuntimeRefresh").cloned().unwrap_or_else(||json!([]));
    let entries=history.as_array_mut().ok_or_else(held)?;
    entries.push(json!({"version":"chrome-prepared-runtime-refresh/v1","requestId":request.request_id,"requestHash":hash,
        "sourceClientId":source_id,"launchToken":launch,"browserBinding":binding,"refreshedAt":at,
        "originalPreparedAt":prepared.prepared_at,"originalPrepareRuntime":frozen,"originalRuntimeSha256":hex_hash(frozen["runtime"].to_string().as_bytes()),"originalPrepareRuntimeSha256":hex_hash(frozen.to_string().as_bytes()),
        "originalPreparedSha256":hex_hash(json!(prepared).to_string().as_bytes()),"bridgeAbsenceBefore":before,"bridgeAbsenceAfter":after,
        "emptyPreflight":empty,"nativeExecutionProof":native,"newRuntime":refreshed.wire["prepareRuntime"]["runtime"]}));
    refreshed.wire["preparedRuntimeRefresh"]=history;
    refreshed.prepared_at=at;
    Ok(refreshed)
}
fn inventory_source_background_proven(inventory:&Value,runtime:&Value,source:&Value,tab:&Value)->bool {
    if tab["active"]==false {return true;}
    if tab["active"]!=true {return false;}
    let proof=&inventory["sourceBackgroundProof"];
    let Some(sampled_at)=proof["content"]["sampledAt"].as_i64() else {return false;};
    let age=chrono::Utc::now().timestamp_millis()-sampled_at;
    proof["version"]=="director-source-background-proof/v1" && proof["nativeSamples"]==2
        && proof["sourceTabId"]==tab["id"] && tab["id"]==runtime["sourceTabId"]
        && tab["windowId"].as_u64().is_some_and(|id|id>0) && proof["sourceWindowId"]==tab["windowId"]
        && proof["native"]["tabActive"]==true && proof["native"]["windowFocused"]==false
        && ["normal","minimized","maximized","fullscreen"].contains(&s(&proof["native"],"windowState"))
        && proof["content"]["visibilityState"]=="hidden" && proof["content"]["focused"]==false
        && source["visibilityState"]=="hidden" && source["focused"]==false
        && proof["content"]["contentEpoch"]==runtime["contentEpoch"] && runtime["contentEpoch"]==source["contentEpoch"]
        && !s(runtime,"contentEpoch").is_empty() && (0..=30_000).contains(&age)
}
fn legacy_first_open(config:&ProviderConfig,client:&Client,available:&[Value],binding:&crate::chrome_setup::BrowserBinding,request:&GenerationRequest,recovery:&Value,observer:&PrepareObserver<'_>,intent:&mut Value)->ProviderResult<Value> {
    let held=||ProviderError::new("CHROME_PREPARE_IDENTITY_HELD","原页是否存在未由原生inventory证实，保留原任务，不开页或发送");
    let valid_uuid=|v:&str|uuid::Uuid::parse_str(v).is_ok_and(|id|id.get_version_num()==4 && id.to_string()==v);
    if !valid_uuid(&request.request_id) || recovery["legacyNoSubmitAudit"]["version"]!="chrome-legacy-no-submit/v1"
        || recovery["legacyNoSubmitAudit"]["snapshotSha256"].as_str().is_none_or(|v|v.len()!=64)
        || recovery["legacyNoSubmitAudit"]["stateChainProven"]!=true {return Err(held());}
    let owned_idle=|page:&&Value|page["url"]=="https://chatgpt.com/" && page["focused"]==false && page["visibilityState"]=="hidden"
        && same_browser_runtime(page,page,binding) && explicit_idle(page)
        && (page["launchToken"]==control_launch(binding) || s(page,"launchToken").strip_prefix("bridge-director-desktop-").is_some_and(&valid_uuid));
    let controllers:Vec<_>=available.iter().filter(owned_idle).collect();
    let source=controllers.iter().find(|page|page["launchToken"]==control_launch(binding)).copied().or_else(||controllers.first().copied()).ok_or_else(held)?;
    if !compatible_profile_clients(available,source,binding) || available.iter().filter(|c|c["browserProfileId"]==binding.profile_id).any(|c|!explicit_idle(c)) {return Err(held());}
    prove_bridge_prepare_absence(config,client,request)?;
    let mut url=reqwest::Url::parse(&format!("http://localhost/browser/owned-launch/{}",request.request_id)).map_err(|_|held())?;
    url.query_pairs_mut().append_pair("sourceClientId",s(source,"id")).append_pair("profileId",&binding.profile_id).append_pair("bundleId",&binding.bundle_id)
        .append_pair("backgroundEpoch",s(source,"backgroundEpoch")).append_pair("serverInstanceId",s(source,"serverInstanceId"));
    let native=json_request(config,client,&format!("{}?{}",url.path(),url.query().unwrap_or("")),None,false)?;
    let runtime=&native["runtime"];let inventory=&native["inventory"];
    let raw=native["inventoryCanonicalJson"].as_str().ok_or_else(held)?;
    let parsed:Value=serde_json::from_str(raw).map_err(|_|held())?;
    let age=chrono::Utc::now().timestamp_millis()-native["observedAt"].as_i64().ok_or_else(held)?;
    let source_tabs:Vec<_>=inventory["tabs"].as_array().ok_or_else(held)?.iter().filter(|t|t["id"]==runtime["sourceTabId"]).collect();
    if native["ok"]!=true || native["version"]!="director-owned-launch-inventory/v1" || native["status"]!="confirmed_absent" || native["complete"]!=true
        || native["requestId"]!=request.request_id || native["launchToken"]!=format!("bridge-director-desktop-{}",request.request_id)
        || native["absenceScope"]!="current_browser_inventory_only" || native["componentVersion"]!="6.4.0"
        || runtime["extensionVersion"]!="2.4.5" || runtime["profileId"]!=binding.profile_id || runtime["bundleId"]!=binding.bundle_id
        || runtime["backgroundEpoch"]!=source["backgroundEpoch"] || runtime["serverInstanceId"]!=source["serverInstanceId"]
        || runtime["sourceClientId"]!=source["id"] || native["sourceClientId"]!=source["id"] || runtime["sourceTabId"]!=source["browserTabId"]
        || !runtime["sourceTabId"].as_u64().is_some_and(|id|id>0) || runtime["contentEpoch"]!=source["contentEpoch"] || s(runtime,"contentEpoch").is_empty()
        || inventory["sourceTabId"]!=runtime["sourceTabId"] || !inventory["launchRecords"].is_array() || parsed!=*inventory
        || native["inventorySha256"]!=hex_hash(raw.as_bytes()) || native["inventoryGeneration"].as_str().is_none_or(|id|!valid_uuid(id))
        || !(0..=30_000).contains(&age) || source_tabs.len()!=1 || !inventory_source_background_proven(inventory,runtime,source,source_tabs[0]) || source_tabs[0]["incognito"]!=false
        || source_tabs[0]["discarded"]!=false {return Err(held());}
    let verified=clients(config,client)?;
    let sources:Vec<_>=verified.iter().filter(|p|p["id"]==source["id"]).collect();
    if sources.len()!=1 || !same_idle_owned_source(sources[0],source,binding) || sources[0]["browserTabId"]!=source["browserTabId"]
        || sources[0]["contentEpoch"]!=source["contentEpoch"] || !compatible_profile_clients(&verified,source,binding)
        || verified.iter().filter(|p|p["browserProfileId"]==binding.profile_id).any(|p|!explicit_idle(p))
        || verified.iter().any(|p|p["launchToken"]==native["launchToken"]) {return Err(held());}
    let mut audit=recovery["legacyNoSubmitAudit"].clone();audit["bridgeNoTurn"]=json!(true);audit["bridgeCanonicalAbsent"]=json!(true);
    // Current native absence permits the first versioned open adoption only.
    // It is never presented as proof of an unavailable historical runtime nonce.
    audit["nativeCurrentAbsence"]=native;
    *intent=prepare_intent(request,binding,source,"legacy_first_open",audit)?;observer(intent)?;
    let opened=json_request(config,client,"/browser/tabs/open",Some(&json!({"url":"https://chatgpt.com/","sourceClientId":source["id"],"active":false,"select":false,
        "launchToken":format!("bridge-director-desktop-{}",request.request_id),"timeoutMs":30000,"allowSystemFallback":false})),false)
        .map_err(|mut error|{if error.code!="BRIDGE_BUSY" && !error.auth_required {error.code="CHROME_PREPARE_OPEN_UNCONFIRMED".into();}error})?;
    let tab=&opened["client"];
    if opened["ok"]!=true || tab["launchToken"]!=intent["launchToken"] || tab["url"]!="https://chatgpt.com/" || tab["focused"]!=false || tab["visibilityState"]!="hidden"
        || !same_browser_runtime(tab,source,binding) || !explicit_idle(tab) {return Err(held());}
    Ok(tab.clone())
}

pub(super) fn submit(
    config: &ProviderConfig,
    prepared: &PreparedRequest,
) -> ProviderResult<SubmitReceipt> {
    let client = wire::client(15)?;
    let source = owned_client(config, &client, &prepared.request.request_id)?;
    if source["id"] != prepared.wire["sourceClientId"] {
        return Err(ProviderError::new(
            "TAB_IDENTITY_CHANGED",
            "任务标签页身份已变化。",
        ));
    }
    if let Some(frozen)=prepared.wire.get("browserBinding") {
        if source["browserProfileId"]!=frozen["profileId"] || source["extensionBundleId"]!=frozen["bundleId"] || source["browserIncognito"]!=false
            || serde_json::to_value(&config.chrome_binding).ok().as_ref()!=Some(frozen) {
            return Err(ProviderError::new("BROWSER_BINDING_CHANGED","任务原浏览器绑定已变化，未发送请求；请核对原工作页。"));
        }
    }
    if let Some(frozen)=prepared.wire.get("prepareRuntime") {
        let binding=config.chrome_binding.as_ref().ok_or_else(||ProviderError::new("CHROME_PREPARE_IDENTITY_HELD","原准备浏览器绑定缺失，未发送"))?;
        if frozen["version"]!="chrome-prepare-intent/v1" || frozen["requestId"]!=prepared.request.request_id
            || frozen["requestHash"]!=hex_hash(json!(prepared.request).to_string().as_bytes())
            || frozen["browserBinding"]!=json!(binding) || !same_browser_runtime(&source,&frozen["runtime"],binding) {
            return Err(ProviderError::new("CHROME_PREPARE_IDENTITY_HELD","浏览器运行身份在准备后变化，保留原请求，未发送"));
        }
    }
    let mut body = prepared.wire.clone();
    body.as_object_mut()
        .ok_or_else(|| ProviderError::new("PREPARED_INVALID", "浏览器准备快照无效。"))?
        .remove("emptyPreflight");
    body.as_object_mut().unwrap().remove("prepareRuntime");
    body.as_object_mut().unwrap().remove("preparedRuntimeRefresh");
    body.as_object_mut().unwrap().remove("correlationVersion");
    if shutting_down() {
        return Err(ProviderError::new(
            "CLIENT_CLOSING",
            "客户端正在关闭，未向浏览器发送生成请求。",
        ));
    }
    let response = client
        .post(format!("{}/turns", wire::base(config, true)?))
        .bearer_auth(wire::credential(config, true)?)
        .header(
            "Idempotency-Key",
            format!("director-desktop:{}", prepared.request.request_id),
        )
        .json(&body)
        .send()
        .map_err(|_| ProviderError::uncertain("浏览器发送回执未确认；后续只能查询原任务。"))?;
    let raw = wire::read_json(response, true)?;
    if raw["ok"] != true || raw["turn"]["id"] != prepared.request.request_id {
        return Err(ProviderError::uncertain(
            "浏览器提交回执身份不匹配；不能再次发送。",
        ));
    }
    Ok(SubmitReceipt {
        task_id: prepared.request.request_id.clone(),
        status: "processing".into(),
        result_urls: vec![],
        raw,
    })
}

fn reference_upload_name(request:&str,order:u32,ext:&str)->String{format!("{request}-ref-{order}.{ext}")}
fn image_file_id(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if parsed.origin().ascii_serialization() != "https://chatgpt.com"
        || !parsed.path().starts_with("/backend-api/estuary/content")
    {
        return None;
    }
    parsed
        .query_pairs()
        .find(|(k, _)| k == "id" || k == "file_id")
        .map(|(_, v)| v.into_owned())
        .or_else(|| parsed.path().split("/content/").nth(1).map(str::to_string))
}
pub(super) fn verify_result(
    turn: &Value,
    proof: &Value,
    source: &Value,
    id: &str,
) -> ProviderResult<Vec<String>> {
    let response = &turn["output"]["response"];
    let artifacts = turn["output"]["artifacts"]
        .as_array()
        .ok_or_else(|| ProviderError::new("IMAGE_PROVENANCE", "原图来源记录缺失。"))?;
    let bindings = proof["bindings"]
        .as_array()
        .ok_or_else(|| ProviderError::new("IMAGE_PROVENANCE", "原生图片字节证明缺失。"))?;
    if turn["status"] != "completed"
        || turn["id"] != id
        || response["requestId"] != id
        || response["sourceClientId"] != source["id"]
        || proof["requestId"] != id
        || proof["status"] != "matched_complete"
        || proof["conversationId"] != response["session"]["id"]
        || proof["assistantMessageId"] != response["turnKey"]
        || artifacts.len() != 1
        || bindings.len() != 1
        || !response["session"]["id"]
            .as_str()
            .is_some_and(|x| !x.is_empty())
        || !response["turnKey"].as_str().is_some_and(|x| !x.is_empty())
        || !source["id"].as_str().is_some_and(|x| !x.is_empty())
    {
        return Err(ProviderError::new(
            "IMAGE_PROVENANCE",
            "原生记录与当前任务回复不匹配。",
        ));
    }
    let expected_url = format!(
        "https://chatgpt.com/c/{}",
        response["session"]["id"].as_str().unwrap_or("")
    );
    if response["url"] != expected_url {
        return Err(ProviderError::new("IMAGE_PROVENANCE", "图片会话来源无效。"));
    }
    let image = &artifacts[0];
    let binding = &bindings[0];
    let file_id = image["url"]
        .as_str()
        .and_then(image_file_id)
        .ok_or_else(|| ProviderError::new("IMAGE_PROVENANCE", "原生图片文件身份无效。"))?;
    let mime = image["mime"].as_str().unwrap_or("");
    let hash = binding["bytesSha256"].as_str().unwrap_or("");
    if image["kind"] != "image"
        || image["phase"] != "READY"
        || image["downloadable"] != true
        || !(image["generatedImage"] == true
            || image["generatedImage"].is_null()
                && image["blockTestId"] == "generated-image-preview")
        || image["requestId"] != id
        || image["sourceTurnKey"] != response["turnKey"]
        || image["sourceClientId"] != response["sourceClientId"]
        || binding["imageFileId"] != file_id
        || !["image/png", "image/jpeg", "image/webp"].contains(&mime)
        || hash.len() != 64
        || !hash.bytes().all(|x| x.is_ascii_hexdigit())
        || image
            .get("sessionId")
            .is_some_and(|x| x != &response["session"]["id"])
        || binding.get("mime").is_some_and(|x| x != mime)
        || image
            .get("size")
            .zip(binding.get("size"))
            .is_some_and(|(a, b)| a != b)
    {
        return Err(ProviderError::new(
            "IMAGE_PROVENANCE",
            "生成原图与字节证明不匹配。",
        ));
    }
    let artifact_id = image["id"]
        .as_str()
        .ok_or_else(|| ProviderError::new("IMAGE_PROVENANCE", "原图 ID 缺失。"))?;
    valid_id(artifact_id)?;
    let mut result = reqwest::Url::parse(&format!("artifact:///{artifact_id}"))
        .map_err(|_| ProviderError::new("ARTIFACT_ID", "原图 ID 无效。"))?;
    result
        .query_pairs_mut()
        .append_pair("mime", mime)
        .append_pair("sha256", hash);
    if let Some(size) = binding["size"].as_u64() {
        result
            .query_pairs_mut()
            .append_pair("size", &size.to_string());
    }
    Ok(vec![result.into()])
}

pub(super) fn query(config: &ProviderConfig, id: &str) -> ProviderResult<QueryResult> {
    query_with_prepared(config, id, None)
}
pub(super) fn query_with_prepared(
    config: &ProviderConfig,
    id: &str,
    prepared: Option<&PreparedRequest>,
) -> ProviderResult<QueryResult> {
    query_inner(config,id,prepared,&json!({}),&|_|Ok(()))
}
pub(super) fn query_with_saved(config:&ProviderConfig,id:&str,p:&PreparedRequest,saved:&Value,observe:&PrepareObserver<'_>)->ProviderResult<QueryResult>{
    query_inner(config,id,Some(p),saved,observe)
}
fn query_inner(config:&ProviderConfig,id:&str,prepared:Option<&PreparedRequest>,saved:&Value,observe:&PrepareObserver<'_>)->ProviderResult<QueryResult>{
    let client = wire::client(45)?;
    let raw = match json_request(config, &client, &format!("/turns/{id}"), None, false){
        Ok(raw)=>raw,
        Err(e) if e.context.as_ref().is_some_and(|c|c["httpStatus"]==404)&&prepared.is_some_and(|p|conversation::locator_valid(config,p,&saved["locator"]))=>json!({"ok":true,"turn":{"id":id,"status":"original_record_unavailable"},"originalTurn404":true}),
        Err(e)=>return Err(e)
    };
    let turn = &raw["turn"];
    if raw["ok"] != true || turn["id"] != id {
        return Err(ProviderError::new(
            "TASK_MISMATCH",
            "浏览器未返回此任务；请人工核对，不会重发。",
        ));
    }
    let status = turn["status"].as_str().unwrap_or("unknown");
    // A missing DOM anchor can keep a completed image in running until bridge timeout.
    // Read the original native branch even before the legacy bridge reports completed.
    let original_source = prepared.and_then(|p| p.wire["sourceClientId"].as_str())
        .or_else(|| turn["input"]["sourceClientId"].as_str());
    let original_client=original_client_for(config, &client, id, true, original_source);
    let mut saved_state=saved.clone();
    if saved_state["locator"].is_null(){if let (Some(p),Ok(source))=(prepared,&original_client){
        if p.wire["correlationVersion"]!="conversation/v1"&&source["url"].as_str().is_some_and(|u|u.starts_with("https://chatgpt.com/c/")){
            let locator=conversation::legacy_seed(config,p,source,observe)?;if !locator.is_null(){saved_state["locator"]=locator;}
        }
    }}
    let saved=&saved_state;
    let (source,restored)=match original_client{
        Ok(source) if !(source["focused"]==true&&prepared.is_some_and(|p|p.wire["correlationVersion"]!="conversation/v1")&&prepared.is_some_and(|p|conversation::locator_valid(config,p,&saved["locator"])))=>(source,false),
        Ok(_)=>(conversation::restore_source(config,&client,prepared.unwrap(),saved,observe)?,true),
        Err(original_error)=>if let Some(p)=prepared.filter(|p|conversation::locator_valid(config,p,&saved["locator"])){(conversation::restore_source(config,&client,p,saved,observe)?,true)}else{return Err(original_error);}
    };
    // Stable request ID + owned tab + native conversation permit exact source reconciliation.
    if !restored && (prepared
        .is_some_and(|p| p.request.request_id != id || p.wire["sourceClientId"] != source["id"])
        || turn["input"]["sourceClientId"].as_str().is_some_and(|original| source["id"] != original))
    {
        return Err(ProviderError::new(
            "IMAGE_PROVENANCE",
            "保存的准备快照与任务标签页身份不一致。",
        ));
    }
    if let Some(p)=prepared{if let Some(failed)=upload_settlement::settle(config,&client,turn,&source,p)?{return Ok(failed);}}
    if let Some(pending) = homepage_preparation_pending(turn, &source, id, prepared)? {
        return Ok(pending);
    }
    let conversation = owned_conversation(&source)?;
    if !saved["locator"].is_null()&&saved["locator"]["conversationId"]!=conversation{return Err(ProviderError::new("IMAGE_PROVENANCE","当前页面不是保存的原会话"));}
    if let Some(original) = turn["output"]["response"]["session"]["id"].as_str() {
        if original != conversation { return Err(ProviderError::new("IMAGE_PROVENANCE", "原任务与当前会话身份不一致。")); }
    }
    let prompt_hash = prepared
        .and_then(|p| p.wire["metadata"]["promptSha256"].as_str())
        .or_else(|| turn["input"]["metadata"]["promptSha256"].as_str())
        .or_else(|| turn["metadata"]["promptSha256"].as_str());
    let hash = if let Some(hash) = prompt_hash {
        hash.to_string()
    } else {
        let message = turn["input"]["message"]
            .as_str()
            .or_else(|| turn["message"].as_str())
            .ok_or_else(|| {
                ProviderError::new(
                    "IMAGE_PROVENANCE",
                    "网页任务未保存提示词关联证明，需人工核对。",
                )
            })?;
        hex_hash(
            message
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .as_bytes(),
        )
    };
    let recovered = json_request(
        config,
        &client,
        "/browser/recover-latest",
        Some(
            &json!({"sourceClientId":source["id"],"index":1,"timeoutMs":40000,"reconcileConversation":{"mode":"image_record_v1","requestId":id,"conversationId":conversation,"promptSha256":hash,"imageCount":1,"allowReleasedRequest":true,"allowReloadedOriginalTab":original_source.is_some_and(|original|source["id"]==original),"savedConversation":saved["locator"],"readOnlyRecoveryLaunch":if restored{source["launchToken"].clone()}else{Value::Null}}}),
        ),
        false,
    )?;
    let proof = &recovered["result"]["reconciliation"];
    let locator=if let Some(p)=prepared{conversation::pin(config,p,&source,proof,saved,observe)?}else{Value::Null};
    if proof["status"]=="matched_no_image" {
        let p=prepared.ok_or_else(||ProviderError::new("IMAGE_PROVENANCE","无图终态缺少原准备记录"))?;
        if !crate::chrome_terminal::proof_valid(proof,&json!(p),id) || recovered["result"]["sourceClientId"]!=source["id"]
            || recovered["result"]["session"]["id"]!=conversation || recovered["result"]["artifacts"].as_array().is_none_or(|a|!a.is_empty()) {
            return Err(ProviderError::new("IMAGE_PROVENANCE","无图终态的原会话与分支证明不完整"));
        }
        let settled=if ["failed","cancelled","interrupted","completed_without_artifact"].contains(&status){turn.clone()}
            else {json_request(config,&client,&format!("/turns/{id}/interrupt"),Some(&json!({"reason":"原生会话双读已证明本次仅返回最终文字、未生成图片；结束原任务等待，保留回执"})),false)?["turn"].clone()};
        if settled["id"]!=id || !["failed","cancelled","interrupted","completed_without_artifact"].contains(&s(&settled,"status")) {
            return Err(ProviderError::new("IMAGE_TERMINAL_SETTLEMENT_PENDING","网页已确认无图，等待原任务结算；未重新生成"));
        }
        return Ok(QueryResult{task_id:id.into(),status:"failed".into(),result_urls:vec![],error:Some("ChatGPT 本次未能生成图片，原会话与失败回执已保存；其他资产继续排队。".into()),actual_cost:None,
            raw:json!({"turn":turn,"recoveredResponse":recovered["result"],"nativeProof":proof,"taskSettlement":{"id":id,"status":settled["status"]},"progress":{"phase":"generation_failed_no_image","message":"网页已结束，但没有生成图片。","checkedAt":now()}})});
    }
    if proof["promptSha256"] != hash {
        return Err(ProviderError::new(
            "IMAGE_PROVENANCE",
            "原生记录的提示词哈希不匹配。",
        ));
    }
    if proof["status"] != "matched_complete" {
        if proof["status"] == "matched_incomplete" && !["failed","interrupted","cancelled","completed_without_artifact"].contains(&status) {
            return Ok(QueryResult { task_id:id.into(),status:"processing".into(),result_urls:vec![],error:None,actual_cost:None,
                raw:json!({"turn":turn,"nativeProof":proof,"progress":{"phase":"waiting_image","message":"网页仍在生成，已核对本次原任务；完成后自动回收原图。","checkedAt":now()}}) });
        }
        return Err(ProviderError::new("IMAGE_RECOVERY_PENDING",format!("原图回收尚未完成（{}）：保留原任务，检查网页连接后恢复查询，不会重新生成。",proof["reason"].as_str().unwrap_or("来源校验未通过"))));
    }
    let recovered_turn = verified_recovery_turn(turn, &recovered["result"], &source, id, &hash)?;
    let urls = verify_result(&recovered_turn, proof, &source, id)?;
    Ok(QueryResult {
        task_id: id.into(),
        status: "succeeded".into(),
        result_urls: urls,
        error: None,
        actual_cost: None,
        raw: json!({"turn":turn,"recoveredResponse":recovered["result"],"nativeProof":proof,"readOnlyRecovery":if restored{json!({"locator":locator,"client":source})}else{Value::Null},"progress":{"phase":"image_verified","message":"网页原图已核对，正在下载并保存到资产。","checkedAt":now()}}),
    })
}

// The bridge can accept a turn while its owned blank page is still uploading
// references or creating the first conversation. This is not native image proof:
// only exact frozen inputs may wait, with a fixed original-start deadline.
fn homepage_preparation_pending(turn: &Value, source: &Value, id: &str, prepared: Option<&PreparedRequest>) -> ProviderResult<Option<QueryResult>> {
    let homepage = source["url"].as_str().and_then(|s| reqwest::Url::parse(s).ok())
        .is_some_and(|u| u.origin().ascii_serialization() == "https://chatgpt.com"
            && u.path() == "/" && u.query().is_none() && u.fragment().is_none());
    if !homepage { return Ok(None); }
    let invalid = || ProviderError::new("IMAGE_PROVENANCE", "首页等待缺少本次原任务的完整身份，保留记录，不会重新发送。");
    let p = prepared.ok_or_else(invalid)?;
    let message = p.wire["message"].as_str().filter(|s| !s.is_empty()).ok_or_else(invalid)?;
    let hash = hex_hash(message.split_whitespace().collect::<Vec<_>>().join(" ").as_bytes());
    if p.provider != "chrome" || p.request.kind != "image" || p.request.request_id != id
        || p.wire["id"] != id || turn["id"] != id
        || p.wire["sourceClientId"] != source["id"] || turn["input"]["sourceClientId"] != source["id"]
        || source["launchToken"] != format!("bridge-director-desktop-{id}")
        || p.wire["metadata"]["jobId"] != id || turn["input"]["metadata"]["jobId"] != id
        || p.wire["metadata"]["promptSha256"] != hash || turn["input"]["metadata"]["promptSha256"] != hash
        || turn["input"]["message"].as_str() != Some(message.trim())
        || !p.wire["attachments"].is_array() || turn["input"]["attachments"] != p.wire["attachments"]
        || !["running", "queued"].contains(&turn["status"].as_str().unwrap_or(""))
        || !turn["error"].is_null() && turn["error"] != json!({})
        || turn["output"]["response"]["session"]["id"].as_str().is_some_and(|s| !s.is_empty())
        || turn["input"]["sessionId"].as_str().is_some_and(|s| !s.is_empty()) {
        return Err(invalid());
    }
    let started = turn["createdAt"].as_str().unwrap_or(&p.prepared_at);
    let started = chrono::DateTime::parse_from_rfc3339(started).map_err(|_| invalid())?;
    let age = chrono::Utc::now().signed_duration_since(started);
    if age < chrono::Duration::zero() || age >= chrono::Duration::minutes(5) {
        return Err(ProviderError::new("IMAGE_PREPARATION_TIMEOUT", "原任务超过 5 分钟仍未进入生成会话，需检查网页上传或连接；保留原任务，不会重新发送。"));
    }
    Ok(Some(QueryResult { task_id:id.into(), status:"processing".into(), result_urls:vec![],
        error:None, actual_cost:None, raw:json!({"turn":turn,"progress":{"phase":"preparing_submission",
            "message":"网页正在上传参考图或创建本次会话，完成后自动继续回收原图。","checkedAt":now(),
            "preparationDeadline":(started + chrono::Duration::minutes(5)).to_rfc3339()}}) }))
}

fn owned_conversation(source: &Value) -> ProviderResult<String> {
    let url = source["url"].as_str().and_then(|s|reqwest::Url::parse(s).ok())
        .filter(|u|u.origin().ascii_serialization()=="https://chatgpt.com" && u.query().is_none() && u.fragment().is_none())
        .ok_or_else(||ProviderError::new("IMAGE_PROVENANCE","任务标签页尚未进入原生成会话。"))?;
    let id = url.path().strip_prefix("/c/").filter(|s|!s.is_empty() && !s.contains('/'))
        .ok_or_else(||ProviderError::new("IMAGE_PROVENANCE","任务标签页没有明确的原会话身份。"))?;
    Ok(id.into())
}

fn verified_recovery_turn(original: &Value, response: &Value, source: &Value, id: &str, hash: &str) -> ProviderResult<Value> {
    let proof=&response["reconciliation"];
    let standalone = response["requestId"].is_null() || response["requestId"] == "";
    if original["id"] != id || response["sourceClientId"] != source["id"]
        || proof["source"] != "conversation-record" || proof["version"] != 1 || proof["proofVersion"] != 2
        || proof["status"] != "matched_complete" || proof["promptSha256"] != hash
        || proof["requestId"] != id || proof["backendStatus"] != "finished_successfully" || proof["endTurn"] != true
        || response["session"]["id"] != owned_conversation(source)?
        || proof["assistantMessageId"] != response["turnKey"]
        || !standalone && response["requestId"] != id {
        return Err(ProviderError::new("IMAGE_PROVENANCE","原生回收记录、提示词或会话身份不匹配，未采用图片。"));
    }
    // This is a local verification envelope, never an edit to the original bridge receipt.
    // Protocol 5 clears the outer requestId and the coordinator substitutes
    // "recovered" on artifacts. Only an unbound standalone reply may adopt the
    // original identity from the native proof. Never rewrite a different task.
    let mut verified_response=response.clone();
    verified_response["requestId"]=json!(id);
    let mut artifacts=response["artifacts"].clone();
    let images=artifacts.as_array_mut().ok_or_else(||ProviderError::new("IMAGE_PROVENANCE","原图来源记录缺失。"))?;
    for image in images {
        let request=&image["requestId"];
        if request != id && !(standalone && (request.is_null() || request == "" || request == "recovered")) {
            return Err(ProviderError::new("IMAGE_PROVENANCE","回收图片属于其他任务，未采用图片。"));
        }
        image["requestId"]=json!(id);
    }
    let verified=json!({"id":id,"status":"completed","output":{"response":verified_response,"artifacts":artifacts}});
    // Validate file, assistant, source, session and byte bindings before returning
    // the local envelope. The original raw bridge response remains untouched.
    verify_result(&verified,proof,source,id)?;
    Ok(verified)
}
pub(super) fn download(
    config: &ProviderConfig,
    url: &str,
    target: &Path,
) -> ProviderResult<DownloadReceipt> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| ProviderError::new("ARTIFACT_URL", "原图下载凭据无效。"))?;
    if parsed.scheme() != "artifact" {
        return Err(ProviderError::new(
            "ARTIFACT_URL",
            "Chrome 必须使用已核对的原图 artifact 凭据。",
        ));
    }
    let id = parsed.path().trim_start_matches('/');
    valid_id(id)?;
    let params: std::collections::HashMap<_, _> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let hash = params
        .get("sha256")
        .ok_or_else(|| ProviderError::new("ARTIFACT_URL", "原图字节证明缺失。"))?;
    let response = wire::client(60)?
        .get(format!(
            "{}/artifacts/{id}/download",
            wire::base(config, true)?
        ))
        .bearer_auth(wire::credential(config, true)?)
        .send()
        .map_err(|_| ProviderError::new("DOWNLOAD_FAILED", "原图下载失败，可以重试原任务下载。"))?;
    let receipt = wire::save_response(response, target, params.get("mime").map(String::as_str))?;
    if &receipt.sha256 != hash
        || params
            .get("size")
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|size| size != receipt.bytes)
    {
        let _ = fs::remove_file(target);
        return Err(ProviderError::new(
            "IMAGE_BYTES_MISMATCH",
            "下载字节与原生来源证明不一致，未导入。",
        ));
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recovered_fixture(id: &str, hash: &str) -> (Value, Value) {
        let source=json!({"id":"original-tab","ready":true,"url":"https://chatgpt.com/c/conversation","launchToken":format!("bridge-director-desktop-{id}"),"activeRequest":null});
        let image=json!({"id":"OriginalImage","kind":"image","phase":"READY","downloadable":true,"generatedImage":true,"requestId":id,"sourceTurnKey":"assistant","sourceClientId":"original-tab","sessionId":"conversation","mime":"image/png","size":12,"url":"https://chatgpt.com/backend-api/estuary/content?id=file-original"});
        let proof=json!({"source":"conversation-record","version":1,"proofVersion":2,"status":"matched_complete","requestId":id,"promptSha256":hash,"conversationId":"conversation","assistantMessageId":"assistant","backendStatus":"finished_successfully","endTurn":true,"bindings":[{"imageFileId":"file-original","bytesSha256":"a".repeat(64),"size":12,"mime":"image/png"}]});
        let response=json!({"sourceClientId":"original-tab","requestId":id,"turnKey":"assistant","url":source["url"],"session":{"id":"conversation"},"artifacts":[image],"reconciliation":proof});
        (source,response)
    }
    #[test]
    fn running_and_timed_out_original_images_are_recovered_without_submit() {
        let id="12345678-1234-1234-1234-123456789abc";
        let hash="b".repeat(64);
        for status in ["running","failed","reloaded"] {
            let (mut source,mut response)=recovered_fixture(id,&hash);
            if status=="running" {source["activeRequest"]=json!({"requestId":id});}
            if status=="reloaded" {source["launchToken"]=json!("bridge-reload-original");response["requestId"]=json!("");response["artifacts"][0]["requestId"]=json!("recovered");}
            let turn=json!({"id":id,"status":status,"error":{"code":"CANONICAL_DEADLINE_EXCEEDED"},"input":{"sourceClientId":"original-tab","metadata":{"promptSha256":hash}}});
            let fixture=super::super::wire_tests::fixture(vec![json!({"ok":true,"turn":turn}),json!({"ok":true,"clients":[source]}),json!({"ok":true,"result":response})]);
            let env=format!("DIRECTOR_CHROME_TEST_{}",uuid::Uuid::new_v4().simple());
            std::env::set_var(&env,"synthetic-not-a-real-credential");
            let config=ProviderConfig{kind:"chrome".into(),endpoint:Some(fixture.endpoint.trim_end_matches("/v1").into()),bridge_token_env:Some(env.clone()),chrome_binding:None,credential_env:None,cli_path:None,timeout_seconds:5,adapted_models:vec![]};
            let result=query(&config,id).unwrap();
            std::env::remove_var(env);
            fixture.thread.join().unwrap();
            assert_eq!(result.status,"succeeded");
            assert_eq!(result.result_urls.len(),1);
            assert_eq!(result.raw["turn"]["status"],status,"original receipt must remain untouched");
            if status=="reloaded" {
                assert_eq!(result.raw["recoveredResponse"]["requestId"],"");
                assert_eq!(result.raw["recoveredResponse"]["artifacts"][0]["requestId"],"recovered");
            }
            let requests=fixture.requests.lock().unwrap();
            assert_eq!(requests.len(),3);
            assert!(requests[0].0.starts_with("GET /turns/"));
            assert!(requests[1].0.starts_with("GET /browser/clients"));
            assert!(requests[2].0.starts_with("POST /browser/recover-latest"));
            let body:Value=serde_json::from_str(&requests[2].1).unwrap();
            assert_eq!(body["reconcileConversation"]["requestId"],id);
            assert_eq!(body["reconcileConversation"]["promptSha256"],hash);
            assert_eq!(body["reconcileConversation"]["allowReleasedRequest"],true);
        }
    }
    #[test]
    fn reloaded_tab_requires_original_source_and_rejects_competing_ownership() {
        let id="12345678-1234-1234-1234-123456789abc";
        for (client_id,launch,active,original,allowed) in [
            ("original-tab","bridge-reload-original",Value::Null,Some("original-tab"),true),
            ("original-tab","",Value::Null,Some("original-tab"),true),
            ("different-tab","bridge-reload-original",Value::Null,Some("original-tab"),false),
            ("original-tab","bridge-director-desktop-other",Value::Null,Some("original-tab"),false),
            ("original-tab","bridge-reload-original",json!({"requestId":"other"}),Some("original-tab"),false),
            ("original-tab","bridge-reload-original",Value::Null,None,false),
        ] {
            let source=json!({"id":client_id,"launchToken":launch,"activeRequest":active,"ready":true});
            let fixture=super::super::wire_tests::fixture(vec![json!({"ok":true,"clients":[source]})]);
            let env=format!("DIRECTOR_CHROME_TEST_{}",uuid::Uuid::new_v4().simple());
            std::env::set_var(&env,"synthetic-not-a-real-credential");
            let config=ProviderConfig{kind:"chrome".into(),endpoint:Some(fixture.endpoint.trim_end_matches("/v1").into()),bridge_token_env:Some(env.clone()),chrome_binding:None,credential_env:None,cli_path:None,timeout_seconds:5,adapted_models:vec![]};
            assert_eq!(original_client_for(&config,&wire::client(5).unwrap(),id,true,original).is_ok(),allowed);
            std::env::remove_var(env);
            fixture.thread.join().unwrap();
        }
    }
    #[test]
    fn recovered_images_reject_changed_prompt_tab_branch_and_completion_proof() {
        let id="original-task";let hash="b".repeat(64);
        let (source,response)=recovered_fixture(id,&hash);
        let original=json!({"id":id,"status":"failed"});
        assert!(verified_recovery_turn(&original,&response,&source,id,&hash).is_ok());
        for (pointer,value) in [("/sourceClientId",json!("other-tab")),("/requestId",json!("other-task")),("/session/id",json!("other-conversation")),("/turnKey",json!("other-message")),("/reconciliation/promptSha256",json!("c".repeat(64))),("/reconciliation/proofVersion",json!(1)),("/reconciliation/endTurn",json!(false)),("/reconciliation/backendStatus",json!("in_progress"))] {
            let mut changed=response.clone();*changed.pointer_mut(pointer).unwrap()=value;
            assert!(verified_recovery_turn(&original,&changed,&source,id,&hash).is_err(),"{pointer}");
        }
        for url in ["https://evil.test/c/conversation","https://chatgpt.com/","https://chatgpt.com/c/conversation?other=1","https://chatgpt.com/c/conversation#different"] {
            assert!(owned_conversation(&json!({"url":url})).is_err());
        }
    }
    #[test]
    fn standalone_artifact_identity_requires_full_original_proof() {
        let id="original-task";let hash="b".repeat(64);
        let (source,mut response)=recovered_fixture(id,&hash);
        let original=json!({"id":id,"status":"failed"});
        response["requestId"]=json!("");
        for marker in [Value::Null,json!(""),json!("recovered"),json!(id)] {
            response["artifacts"][0]["requestId"]=marker.clone();
            let verified=verified_recovery_turn(&original,&response,&source,id,&hash).unwrap();
            assert_eq!(verified["output"]["artifacts"][0]["requestId"],id);
            assert_eq!(response["artifacts"][0]["requestId"],marker);
        }
        response["artifacts"][0]["requestId"]=json!("recovered");
        for (pointer,value) in [("/requestId",json!(id)),("/requestId",json!(5)),("/artifacts/0/requestId",json!("different-task")),("/artifacts/0/requestId",json!(false)),("/artifacts/0/sourceTurnKey",json!("different-assistant")),("/artifacts/0/sourceClientId",json!("different-tab")),("/artifacts/0/url",json!("https://chatgpt.com/backend-api/estuary/content?id=different-file")),("/artifacts/0/size",json!(99)),("/reconciliation/conversationId",json!("different-conversation")),("/reconciliation/promptSha256",json!("c".repeat(64)))] {
            let mut changed=response.clone();*changed.pointer_mut(pointer).unwrap()=value;
            assert!(verified_recovery_turn(&original,&changed,&source,id,&hash).is_err(),"{pointer}");
        }
    }
    #[test]
    fn unrelated_image_is_rejected() {
        let turn =
            json!({"id":"expected","output":{"response":{"requestId":"other"},"artifacts":[]}});
        assert!(verify_result(
            &turn,
            &json!({"bindings":[]}),
            &json!({"id":"client"}),
            "expected"
        )
        .is_err());
    }
    #[test]
    fn native_file_identity_is_bound_to_chatgpt() {
        assert_eq!(
            image_file_id("https://evil.test/backend-api/estuary/content?id=file"),
            None
        );
        assert_eq!(
            image_file_id("https://chatgpt.com/backend-api/estuary/content?id=file"),
            Some("file".into())
        );
    }
    #[test]
    fn proven_artifact_preserves_case_and_hash() {
        let hash = "a".repeat(64);
        let image = json!({"id":"Artifact_MixedCase","kind":"image","phase":"READY","downloadable":true,"blockTestId":"generated-image-preview","requestId":"task","sourceTurnKey":"reply","sourceClientId":"client","mime":"image/png","url":"https://chatgpt.com/backend-api/estuary/content?id=file-1"});
        let turn = json!({"id":"task","status":"completed","output":{"response":{"requestId":"task","turnKey":"reply","sourceClientId":"client","session":{"id":"conversation"},"url":"https://chatgpt.com/c/conversation"},"artifacts":[image]}});
        let proof = json!({"status":"matched_complete","requestId":"task","conversationId":"conversation","assistantMessageId":"reply","bindings":[{"imageFileId":"file-1","bytesSha256":hash,"mime":"image/png","size":12}]});
        let urls = verify_result(&turn, &proof, &json!({"id":"client"}), "task").unwrap();
        let parsed = reqwest::Url::parse(&urls[0]).unwrap();
        assert_eq!(parsed.path(), "/Artifact_MixedCase");
        assert_eq!(
            parsed.query_pairs().find(|(k, _)| k == "sha256").unwrap().1,
            "a".repeat(64)
        );
    }
}
