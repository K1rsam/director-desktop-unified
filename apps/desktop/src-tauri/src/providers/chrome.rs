use super::*;
use reqwest::blocking::Client;

#[cfg(test)]
#[path = "chrome_background_tests.rs"]
mod background_tests;

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
    wire::read_json(response, submitting)
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
    let source = prepared.wire["sourceClientId"].as_str().filter(|s| !s.is_empty())
        .ok_or_else(|| ProviderError::new("TAB_CLEANUP_IDENTITY", "原工作页身份缺失，保留页面。"))?;
    let client = wire::client(12)?;
    let available = clients(config, &client)?;
    let matches: Vec<_> = available.iter().filter(|c| c["id"] == source).collect();
    if matches.is_empty() { return Ok(json!({"status":"already_closed"})); }
    if matches.len() != 1 { return Err(ProviderError::new("TAB_CLEANUP_IDENTITY", "工作页身份不唯一，保留页面。")); }
    let owned = matches[0];
    let launch = format!("bridge-director-desktop-{task}");
    if owned["launchToken"] != launch || !usable(owned) {
        return Ok(json!({"status":"retained","reason":"ownership_or_idle_not_proven"}));
    }
    if owned["focused"] != false || owned["visibilityState"] != "hidden" {
        return Ok(json!({"status":"retained","reason":"user_viewing_or_visibility_unknown"}));
    }
    if !available.iter().any(|c| c["id"] != source && usable(c)) {
        return Ok(json!({"status":"retained","reason":"last_connected_page"}));
    }
    let url = owned_conversation(owned)?;
    if url != original_conversation {
        return Ok(json!({"status":"retained","reason":"conversation_changed"}));
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
        &json!({"chromeEndpoint":wire::base(config,true)?}),
        &wire::credential(config,true)?,
    ).map_err(|message|ProviderError::new("BRIDGE_OFFLINE",message))?;
    if health["status"] == "busy" {
        return Err(ProviderError::new("BRIDGE_BUSY", "浏览器正在执行已有任务，图片保持排队等待。"));
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
    let available = clients(config, &client)?;
    let launch = format!("bridge-director-desktop-{}", request.request_id);
    let legacy_launch = format!("director-desktop-{}", request.request_id);
    // Existing owned tabs represent an earlier prepare or submit. Only read-only reconciliation is safe.
    if available
        .iter()
        .any(|c| matches!(c.get("launchToken").and_then(Value::as_str), Some(t) if t==launch || t==legacy_launch))
    {
        return Err(ProviderError::new(
            "EXISTING_TAB_REQUIRES_RECONCILIATION",
            "任务标签页已存在，请核对原关联码；不会重复创建或发送。",
        ));
    }
    let opener = crate::chrome_setup::execution_client(&available)
        .ok_or_else(|| {
            ProviderError::new(
                "BROWSER_SELECTION_REQUIRED",
                "请在 Chrome Bridge 中选择一个已连接的 ChatGPT 标签页。",
            )
        })?;
    let opener_id = opener
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::new("BRIDGE_STATE", "浏览器身份无效。"))?;
    let opened = json_request(
        config,
        &client,
        "/browser/tabs/open",
        Some(
            &json!({"url":"https://chatgpt.com/","sourceClientId":opener_id,"active":false,"select":false,"launchToken":launch,"timeoutMs":12000,"allowSystemFallback":false}),
        ),
        false,
    )?;
    let tab = opened
        .get("client")
        .ok_or_else(|| ProviderError::new("TAB_OPEN_UNKNOWN", "标签页创建回执未确认。"))?;
    if opened.get("ok").and_then(Value::as_bool) != Some(true)
        || tab.get("launchToken").and_then(Value::as_str) != Some(launch.as_str())
        || !usable(tab)
    {
        return Err(ProviderError::new(
            "TAB_OPEN_UNKNOWN",
            "任务标签页创建身份未确认。",
        ));
    }
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
            Some(&json!({"path":r.path,"name":format!("{}.{}",r.sha256,ext),"mime":mime})),
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
    let message=format!("{}\n\n画面比例：{}。请生成 1 张原图。\n任务关联码：{}。仅用于关联本次生成，请勿将编号或文字画入图片。",request.prompt,request.ratio,request.request_id);
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut frozen = request.clone();
    frozen.references = sorted_references(request);
    Ok(PreparedRequest {
        provider: "chrome".into(),
        request: frozen,
        capability_version: caps.version,
        upload_receipts: receipts,
        wire: json!({"id":request.request_id,"message":message,"attachments":attachments,"newSession":false,"sourceClientId":source_id,"metadata":{"jobId":request.request_id,"promptSha256":hex_hash(normalized.as_bytes())},"emptyPreflight":proof}),
        estimated_cost: None,
        prepared_at: now(),
    })
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
    let mut body = prepared.wire.clone();
    body.as_object_mut()
        .ok_or_else(|| ProviderError::new("PREPARED_INVALID", "浏览器准备快照无效。"))?
        .remove("emptyPreflight");
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
    let client = wire::client(45)?;
    let raw = json_request(config, &client, &format!("/turns/{id}"), None, false)?;
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
    let source = original_client_for(config, &client, id, true, original_source)?;
    let conversation = owned_conversation(&source)?;
    if let Some(original) = turn["output"]["response"]["session"]["id"].as_str() {
        if original != conversation { return Err(ProviderError::new("IMAGE_PROVENANCE", "原任务与当前会话身份不一致。")); }
    }
    // Stable request ID + owned tab + native conversation permit exact source reconciliation.
    if prepared
        .is_some_and(|p| p.request.request_id != id || p.wire["sourceClientId"] != source["id"])
        || turn["input"]["sourceClientId"].as_str().is_some_and(|original| source["id"] != original)
    {
        return Err(ProviderError::new(
            "IMAGE_PROVENANCE",
            "保存的准备快照与任务标签页身份不一致。",
        ));
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
            &json!({"sourceClientId":source["id"],"index":1,"timeoutMs":40000,"reconcileConversation":{"mode":"image_record_v1","requestId":id,"conversationId":conversation,"promptSha256":hash,"imageCount":1,"allowReleasedRequest":true,"allowReloadedOriginalTab":original_source.is_some_and(|original|source["id"]==original)}}),
        ),
        false,
    )?;
    let proof = &recovered["result"]["reconciliation"];
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
        raw: json!({"turn":turn,"recoveredResponse":recovered["result"],"nativeProof":proof,"progress":{"phase":"image_verified","message":"网页原图已核对，正在下载并保存到资产。","checkedAt":now()}}),
    })
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
            let config=ProviderConfig{kind:"chrome".into(),endpoint:Some(fixture.endpoint.trim_end_matches("/v1").into()),bridge_token_env:Some(env.clone()),credential_env:None,cli_path:None,timeout_seconds:5,adapted_models:vec![]};
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
            let config=ProviderConfig{kind:"chrome".into(),endpoint:Some(fixture.endpoint.trim_end_matches("/v1").into()),bridge_token_env:Some(env.clone()),credential_env:None,cli_path:None,timeout_seconds:5,adapted_models:vec![]};
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
