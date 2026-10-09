use super::*;
use reqwest::blocking::Client;

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

pub(super) fn capabilities(config: &ProviderConfig) -> ProviderResult<Capabilities> {
    let health = crate::chrome_setup::inspect(
        &json!({"chromeEndpoint":wire::base(config,true)?}),
        &wire::credential(config,true)?,
    ).map_err(|message|ProviderError::new("BRIDGE_OFFLINE",message))?;
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
    let token = format!("director-desktop-{id}");
    let matches: Vec<_> = clients(config, client)?
        .into_iter()
        .filter(|c| c.get("launchToken").and_then(Value::as_str) == Some(token.as_str()))
        .collect();
    if matches.len() != 1 || !usable(&matches[0]) {
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
    let launch = format!("director-desktop-{}", request.request_id);
    // Existing owned tabs represent an earlier prepare or submit. Only read-only reconciliation is safe.
    if available
        .iter()
        .any(|c| c.get("launchToken").and_then(Value::as_str) == Some(launch.as_str()))
    {
        return Err(ProviderError::new(
            "EXISTING_TAB_REQUIRES_RECONCILIATION",
            "任务标签页已存在，请核对原关联码；不会重复创建或发送。",
        ));
    }
    let usable_clients: Vec<_> = available.iter().filter(|c| usable(c)).collect();
    let opener = usable_clients
        .iter()
        .find(|c| c.get("selected").and_then(Value::as_bool) == Some(true))
        .copied()
        .or_else(|| {
            if usable_clients.len() == 1 {
                Some(usable_clients[0])
            } else {
                None
            }
        })
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
            &json!({"url":"https://chatgpt.com/","sourceClientId":opener_id,"active":true,"select":false,"launchToken":launch,"timeoutMs":12000,"allowSystemFallback":false}),
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
    if [
        "failed",
        "interrupted",
        "cancelled",
        "completed_without_artifact",
    ]
    .contains(&status)
    {
        return Ok(QueryResult {
            task_id: id.into(),
            status: "failed".into(),
            result_urls: vec![],
            error: Some("网页任务结束但没有可验证的原图。".into()),
            actual_cost: None,
            raw,
        });
    }
    if status != "completed" {
        return Ok(QueryResult {
            task_id: id.into(),
            status: "processing".into(),
            result_urls: vec![],
            error: None,
            actual_cost: None,
            raw,
        });
    }
    let source = owned_client(config, &client, id)?;
    let conversation = turn["output"]["response"]["session"]["id"]
        .as_str()
        .ok_or_else(|| ProviderError::new("IMAGE_PROVENANCE", "图片会话记录缺失。"))?;
    // Stable request ID + owned tab + native conversation permit exact source reconciliation.
    if prepared
        .is_some_and(|p| p.request.request_id != id || p.wire["sourceClientId"] != source["id"])
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
            &json!({"sourceClientId":source["id"],"index":1,"timeoutMs":40000,"reconcileConversation":{"mode":"image_record_v1","requestId":id,"conversationId":conversation,"promptSha256":hash,"imageCount":1}}),
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
    let urls = verify_result(turn, proof, &source, id)?;
    Ok(QueryResult {
        task_id: id.into(),
        status: "succeeded".into(),
        result_urls: urls,
        error: None,
        actual_cost: None,
        raw: json!({"turn":turn,"nativeProof":proof}),
    })
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
