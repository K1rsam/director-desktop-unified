use super::*;
use reqwest::blocking::{Client, Response};
use std::io::{Read, Write};
use std::time::Duration;

pub(super) const MAX_JSON: u64 = 8 * 1024 * 1024;

pub(super) fn credential(config: &ProviderConfig, chrome: bool) -> ProviderResult<String> {
    let name = if chrome {
        config.bridge_token_env.as_ref()
    } else {
        config.credential_env.as_ref()
    }
    .ok_or_else(ProviderError::auth)?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(ProviderError::new(
            "INVALID_CREDENTIAL_ENV",
            "凭据环境变量名称无效。",
        ));
    }
    let value = std::env::var(name).map_err(|_| ProviderError::auth())?;
    if value.trim().is_empty() {
        return Err(ProviderError::auth());
    }
    Ok(value)
}

pub(super) fn client(seconds: u64) -> ProviderResult<Client> {
    Client::builder()
        .timeout(Duration::from_secs(seconds.clamp(1, 1800)))
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ProviderError::new("HTTP_CLIENT", "无法建立渠道连接。"))
}

pub(super) fn base(config: &ProviderConfig, chrome: bool) -> ProviderResult<String> {
    let default = if chrome {
        "http://127.0.0.1:53179"
    } else {
        "https://api.7tai.cc/v1"
    };
    let value = config
        .endpoint
        .as_deref()
        .unwrap_or(default)
        .trim_end_matches('/');
    let url = reqwest::Url::parse(value)
        .map_err(|_| ProviderError::new("INVALID_ENDPOINT", "渠道地址无效。"))?;
    let loopback = matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]") | Some("::1")
    );
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (chrome && (!loopback || url.scheme() != "http" || url.path() != "/"))
        || (!chrome && url.scheme() != "https" && !(loopback && url.scheme() == "http"))
    {
        return Err(ProviderError::new(
            "INVALID_ENDPOINT",
            "渠道地址需为 HTTPS，Chrome 桥接只允许本机回环 HTTP。",
        ));
    }
    if !chrome && matches!(config.kind.as_str(), "bh" | "binghuo") && url.path() == "/" {
        Ok(format!("{value}/v1"))
    } else {
        Ok(value.to_string())
    }
}

pub(super) fn read_json(mut response: Response, submitting: bool) -> ProviderResult<Value> {
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err(ProviderError::auth());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            if submitting {
                ProviderError::uncertain("渠道受理回执未读取完整；请核对原任务，不能自动重发。")
            } else {
                ProviderError::new("READ_FAILED", "渠道响应未读取完整。")
            }
        })?;
    if bytes.len() as u64 > MAX_JSON {
        return Err(if submitting {
            ProviderError::uncertain("渠道提交响应超出大小限制，需人工核对。")
        } else {
            ProviderError::new("RESPONSE_LIMIT", "渠道响应超出大小限制。")
        });
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| {
        if submitting {
            ProviderError::uncertain("渠道提交响应格式无法确认，需核对原任务。")
        } else {
            ProviderError::new("INVALID_RESPONSE", "渠道响应格式无效。")
        }
    })?;
    if !status.is_success() {
        // A 5xx after a POST may have occurred AFTER the upstream accepted it.
        if submitting && (status.is_server_error() || status.as_u16() == 408) {
            return Err(ProviderError::uncertain(
                "渠道返回服务异常，受理结果未知；不会自动重发。",
            ));
        }
        let mut error=ProviderError::new(
            safe_code(&body).as_deref().unwrap_or(&format!("HTTP_{}", status.as_u16())),
            safe_message(&body).unwrap_or_else(|| "渠道请求被拒绝。".into()),
        );
        error.context=Some(json!({"httpStatus":status.as_u16()}));
        return Err(error);
    }
    Ok(sanitize(body))
}

pub(super) fn safe_message(body: &Value) -> Option<String> {
    let text = body
        .get("error")
        .and_then(|v| {
            v.as_str()
                .or_else(|| v.get("message").and_then(Value::as_str))
        })
        .or_else(|| body.get("message").and_then(Value::as_str))
        .or_else(|| body.get("detail").and_then(Value::as_str))?;
    // Do not expose URLs, authorization strings or arbitrary upstream object dumps.
    if text.to_ascii_lowercase().contains("http") || text.len() > 800
        || ["bearer","authorization","cookie","api_key","apikey","access_token","refresh_token","password","secret","token="].iter().any(|word|text.to_ascii_lowercase().contains(word)) {
        return None;
    }
    Some(text.to_string())
}
pub(super) fn safe_code(body:&Value)->Option<String> {
    let code=body["code"].as_str().or_else(||body["error"]["code"].as_str())?;
    (!code.is_empty() && code.len()<=80 && code.bytes().all(|b|b.is_ascii_uppercase() || b.is_ascii_digit() || b==b'_')).then(||code.into())
}

pub(super) fn sanitize(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| {
                    ![
                        "authorization",
                        "cookie",
                        "token",
                        "access_token",
                        "refresh_token",
                        "apikey",
                        "api_key",
                        "credential",
                        "device_code",
                    ]
                    .contains(&k.to_ascii_lowercase().as_str())
                })
                .map(|(k, v)| (k, sanitize(v)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize).collect()),
        other => other,
    }
}

pub(super) fn download_public(url: &str, target: &Path) -> ProviderResult<DownloadReceipt> {
    download_result(url, target, false)
}

/// The official CDN can be unreachable through a local proxy even when its signed
/// result works directly. Retry only the idempotent result GET on transport failure;
/// never retry submit, downgrade TLS, or treat HTTP rejection as a route problem.
pub(super) fn download_official_result(
    url: &str,
    target: &Path,
) -> ProviderResult<DownloadReceipt> {
    download_result(url, target, true)
}

fn result_client(direct: bool, official: bool) -> ProviderResult<Client> {
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(15))
        .referer(false)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let next = attempt.url();
            if attempt.previous().len() >= 5
                || next.scheme() != "https"
                || !next.username().is_empty()
                || next.password().is_some()
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }));
    if direct {
        builder = builder.no_proxy();
    } else if official {
        builder = super::dreamina::apply_result_proxies(builder)?;
    }
    builder
        .build()
        .map_err(|_| ProviderError::new("HTTP_CLIENT", "无法建立结果下载连接。"))
}

fn download_result(
    url: &str,
    target: &Path,
    direct_fallback: bool,
) -> ProviderResult<DownloadReceipt> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| ProviderError::new("INVALID_RESULT_URL", "结果地址无效。"))?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ProviderError::new(
            "INVALID_RESULT_URL",
            "结果下载地址必须为 HTTPS。",
        ));
    }
    let mut primary_category = None;
    let response = match result_client(false, direct_fallback)?.get(url).send() {
        Err(error) if direct_fallback && (error.is_connect() || error.is_timeout()) => {
            primary_category = Some(download_transport_category(&error));
            result_client(true, true)?.get(url).send()
        }
        response => response,
    }
    .map_err(|error| {
        let route = if primary_category.is_some() {
            "direct-fallback"
        } else if direct_fallback {
            "official-policy"
        } else {
            "public-policy"
        };
        download_transport_error(&error, route, primary_category)
    })?;
    save_response(response, target, None)
}

fn transport_category(
    timeout: bool,
    connect: bool,
    builder: bool,
    body: bool,
    causes: &str,
) -> &'static str {
    if timeout {
        "TIMEOUT"
    } else if [
        "dns error",
        "failed to lookup",
        "name resolution",
        "no such host",
        "nodename nor servname",
    ]
    .iter()
    .any(|word| causes.contains(word))
    {
        "DNS"
    } else if [
        "certificate",
        "tls handshake",
        "tls alert",
        "invalid peer",
        "unknownissuer",
    ]
    .iter()
    .any(|word| causes.contains(word))
    {
        "TLS"
    } else if connect {
        "CONNECT"
    } else if builder {
        "CLIENT_CONFIG"
    } else if body {
        "READ"
    } else {
        "REQUEST"
    }
}

fn download_transport_category(error: &reqwest::Error) -> &'static str {
    // Never copy Error::to_string(), which includes signed URLs. The bounded
    // underlying type chain is inspected only to select a fixed category.
    let mut cause = std::error::Error::source(error);
    let mut messages = String::new();
    for _ in 0..8 {
        let Some(current) = cause else { break };
        messages.push_str(&current.to_string().to_ascii_lowercase());
        messages.push('\n');
        cause = current.source();
    }
    transport_category(
        error.is_timeout(),
        error.is_connect(),
        error.is_builder(),
        error.is_body(),
        &messages,
    )
}

fn download_transport_error(
    error: &reqwest::Error,
    route: &str,
    primary: Option<&str>,
) -> ProviderError {
    let category = download_transport_category(error);
    let primary = primary
        .map(|category| format!("; primary={category}"))
        .unwrap_or_default();
    ProviderError::new(
        "DOWNLOAD_FAILED",
        format!("结果下载失败；请重新查询原任务刷新地址。[DOWNLOAD_FAILED; category={category}; route={route}{primary}]"),
    )
}

pub(super) fn save_response(
    mut response: Response,
    target: &Path,
    expected_mime: Option<&str>,
) -> ProviderResult<DownloadReceipt> {
    if !response.status().is_success() {
        return Err(ProviderError::new(
            "DOWNLOAD_FAILED",
            format!("下载地址已失效或暂时不可用；请查询原任务。[DOWNLOAD_FAILED; category=HTTP; status={}]", response.status().as_u16()),
        ));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|x| x.to_str().ok())
        .map(|x| x.split(';').next().unwrap_or("").trim().to_owned());
    if expected_mime.is_some_and(|expected| mime.as_deref() != Some(expected)) {
        return Err(ProviderError::new(
            "MIME_MISMATCH",
            "原图下载格式与来源回执不一致。",
        ));
    }
    if !target.is_absolute() || target.exists() {
        return Err(ProviderError::new(
            "OUTPUT_EXISTS",
            "下载目标必须为尚不存在的绝对路径。",
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| ProviderError::new("OUTPUT_PATH", "输出目录无效。"))?;
    fs::create_dir_all(parent)
        .map_err(|_| ProviderError::new("OUTPUT_PATH", "输出目录无法创建。"))?;
    let temp = parent.join(format!(".download-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| ProviderError::new("OUTPUT_PATH", "无法创建下载文件。"))?;
    let result = (|| {
        let expected = response.content_length();
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut chunk = [0; 64 * 1024];
        loop {
            let n = response.read(&mut chunk).map_err(|_| {
                ProviderError::new("DOWNLOAD_INTERRUPTED", "下载中断；原生成任务保持可恢复。")
            })?;
            if n == 0 {
                break;
            }
            size += n as u64;
            if size > 2 * 1024 * 1024 * 1024 {
                return Err(ProviderError::new(
                    "DOWNLOAD_LIMIT",
                    "下载超过 2 GiB 限制。",
                ));
            }
            file.write_all(&chunk[..n])
                .map_err(|_| ProviderError::new("DISK_WRITE", "下载文件写入失败。"))?;
            hash.update(&chunk[..n]);
        }
        if size == 0 || expected.is_some_and(|n| n != size) {
            return Err(ProviderError::new(
                "DOWNLOAD_INCOMPLETE",
                "下载内容不完整。",
            ));
        }
        file.sync_all()
            .map_err(|_| ProviderError::new("DISK_WRITE", "下载文件未能同步到磁盘。"))?;
        // Hard-link creation refuses overwrite on both Win/Mac; then remove the temporary name.
        fs::hard_link(&temp, target).map_err(|_| {
            ProviderError::new("OUTPUT_CONFLICT", "下载目标发生冲突，未覆盖已有文件。")
        })?;
        Ok(DownloadReceipt {
            path: target.to_path_buf(),
            sha256: format!("{:x}", hash.finalize()),
            bytes: size,
            mime,
        })
    })();
    drop(file);
    let _ = fs::remove_file(&temp);
    result
}

#[cfg(test)]
mod result_diagnostics_tests {
    use super::*;
    use std::net::TcpListener;

    fn accept_bounded(listener: &TcpListener) -> std::net::TcpStream {
        listener.set_nonblocking(true).unwrap();
        let started = std::time::Instant::now();
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    // Windows may inherit the listener's nonblocking mode. The fixture
                    // polls accept only; its request read uses a bounded blocking socket.
                    stream.set_nonblocking(false).unwrap();
                    return stream;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && started.elapsed() < Duration::from_secs(2) =>
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => {
                    panic!("bounded loopback fixture did not receive its request: {error}")
                }
            }
        }
    }

    #[test]
    fn download_categories_use_fixed_labels_and_preserve_transport_types() {
        for (timeout, connect, builder, body, causes, expected) in [
            (true, true, false, false, "PRIVATE_TIMEOUT_URL", "TIMEOUT"),
            (
                false,
                true,
                false,
                false,
                "dns error: failed to lookup PRIVATE_HOST",
                "DNS",
            ),
            (
                false,
                true,
                false,
                false,
                "invalid peer certificate PRIVATE_SIGNATURE",
                "TLS",
            ),
            (false, true, false, false, "connection refused", "CONNECT"),
            (false, false, true, false, "PRIVATE_PROXY", "CLIENT_CONFIG"),
            (false, false, false, true, "PRIVATE_RESPONSE", "READ"),
            (false, false, false, false, "PRIVATE_UNKNOWN", "REQUEST"),
        ] {
            let result = transport_category(
                timeout,
                connect,
                builder,
                body,
                &causes.to_ascii_lowercase(),
            );
            assert_eq!(result, expected);
            assert!(!result.contains("PRIVATE"));
        }
    }

    #[test]
    fn actual_loopback_timeout_diagnostic_has_no_signed_url_or_secret() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let _stream = accept_bounded(&listener);
            std::thread::sleep(Duration::from_millis(250));
        });
        let url = format!("http://{address}/PRIVATE_PATH?x-signature=PRIVATE_SIGNATURE");
        let error = Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap()
            .get(&url)
            .send()
            .unwrap_err();
        let safe = download_transport_error(&error, "direct-fallback", Some("CONNECT"));
        worker.join().unwrap();
        assert_eq!(safe.code, "DOWNLOAD_FAILED");
        assert!(safe.message.contains("category=TIMEOUT"));
        assert!(safe
            .message
            .contains("route=direct-fallback; primary=CONNECT"));
        for hidden in ["PRIVATE", &url, &address.to_string(), "x-signature"] {
            assert!(!safe.message.contains(hidden));
        }
        assert!(!safe.auth_required);
        assert!(!safe.submission_unknown);
    }

    #[test]
    fn actual_unreachable_loopback_error_keeps_its_type_without_copying_error_url() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{address}/PRIVATE_PATH?token=PRIVATE_TOKEN");
        let error = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
            .get(&url)
            .send()
            .unwrap_err();
        let safe = download_transport_error(&error, "official-policy", None);
        assert_eq!(safe.code, "DOWNLOAD_FAILED");
        assert!(error.is_connect() || error.is_timeout());
        let expected = if error.is_timeout() {
            "TIMEOUT"
        } else {
            "CONNECT"
        };
        assert!(safe.message.contains(&format!("category={expected}")));
        assert!(!safe.message.contains("PRIVATE"));
        assert!(!safe.message.contains(&url));
        assert!(!safe.message.contains(&address.to_string()));
    }

    #[test]
    fn http_rejection_exposes_only_status_and_is_never_auth_or_expiry_proof() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let mut stream = accept_bounded(&listener);
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 8192];
            assert!(stream.read(&mut request).unwrap() > 0);
            stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 8\r\nConnection: close\r\n\r\nPRIVATE!").unwrap();
        });
        let response = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
            .get(format!(
                "http://{address}/PRIVATE_PATH?signature=PRIVATE_SIGNATURE"
            ))
            .send()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("result.png");
        let error = save_response(response, &target, None).unwrap_err();
        worker.join().unwrap();
        assert_eq!(error.code, "DOWNLOAD_FAILED");
        assert!(error.message.contains("category=HTTP; status=403"));
        assert!(!error.message.contains("PRIVATE"));
        assert!(!error.message.contains(&address.to_string()));
        assert!(!error.auth_required);
        assert!(!error.submission_unknown);
        assert!(!target.exists());
    }
}
