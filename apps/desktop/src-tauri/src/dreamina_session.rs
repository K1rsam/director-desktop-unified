//! Official OAuth device flow. Authentication stays in the CLI's native store.
//! Only safe account projections leave this module; device codes never enter SQLite or logs.
use crate::{
    providers::{self, ProviderConfig, ProviderError, ProviderResult},
    store::{id, now, s, Result},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Flow {
    id: String,
    device_code: String,
    expires: Instant,
    next_poll: Instant,
    interval: Duration,
    transient_failures: u8,
}
struct Entry {
    public: Value,
    checked: Instant,
    flow: Option<Flow>,
    config: String,
    failure: Option<ProviderError>,
}
const EXECUTION_IDENTITY_TTL: Duration = Duration::from_secs(15);
const EXECUTION_FAILURE_TTL: Duration = Duration::from_secs(1);
#[derive(Default)]
pub struct DreaminaSessions {
    entries: Mutex<HashMap<String, Entry>>,
    revisions: Mutex<HashMap<String, u64>>,
    identity_checks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}
fn config_signature(config: &ProviderConfig) -> String {
    serde_json::to_string(config).unwrap_or_default()
}
fn local_state_error() -> ProviderError {
    ProviderError::new("LOCAL_ACCOUNT_STATE", "即梦本地账户状态无法读取或保存。")
}
pub fn config_key(config: &ProviderConfig) -> String {
    let path = config
        .cli_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path
    }
}
fn unknown() -> Value {
    json!({"status":"unknown","ready":false,"identityVerified":false,"accountKey":null,"checkedAt":null,"credits":{"total":null,"member":null,"gift":null,"purchase":null},"message":"正在检查本机即梦 CLI 登录；也可在渠道设置中刷新。"})
}
pub fn authorization_url(value: &str) -> Result<String> {
    let url = reqwest::Url::parse(value).map_err(|_| "即梦授权链接无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some("jimeng.jianying.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || url.fragment().is_some()
        || url.query_pairs().any(|(k, _)| {
            let k = k.to_ascii_lowercase();
            k == "device_code" || k == "devicecode" || k.contains("token")
        })
    {
        return Err("授权页必须为即梦官方 HTTPS 链接，不得携带内部设备凭据".into());
    }
    Ok(url.to_string())
}
impl DreaminaSessions {
    pub fn snapshot(&self, config: &ProviderConfig) -> Result<Value> {
        Ok(self
            .entries
            .lock()
            .map_err(|e| e.to_string())?
            .get(&config_key(config))
            .filter(|e| e.config == config_signature(config))
            .map(|e| e.public.clone())
            .unwrap_or_else(unknown))
    }
    fn save(&self, config: &ProviderConfig, public: Value) -> Result<Value> {
        let mut revisions = self.revisions.lock().map_err(|e| e.to_string())?;
        *revisions.entry(config_key(config)).or_default() += 1;
        self.entries.lock().map_err(|e| e.to_string())?.insert(
            config_key(config),
            Entry {
                public: public.clone(),
                checked: Instant::now(),
                flow: None,
                config: config_signature(config),
                failure: None,
            },
        );
        Ok(public)
    }
    #[cfg(test)]
    pub(crate) fn seed_for_test(&self, config: &ProviderConfig, public: Value) -> Result<Value> {
        self.save(config, public)
    }
    pub fn due(&self, config: &ProviderConfig) -> Result<bool> {
        let entries = self.entries.lock().map_err(|e| e.to_string())?;
        Ok(match entries.get(&config_key(config)) {
            None => true,
            Some(e) if e.config != config_signature(config) => true,
            Some(e) => match &e.flow {
                Some(f) => Instant::now() >= f.next_poll || Instant::now() >= f.expires,
                None => e.checked.elapsed() >= Duration::from_secs(60),
            },
        })
    }
    pub fn forget(&self, config: &ProviderConfig) -> Result<()> {
        let mut revisions = self.revisions.lock().map_err(|e| e.to_string())?;
        *revisions.entry(config_key(config)).or_default() += 1;
        self.entries
            .lock()
            .map_err(|e| e.to_string())?
            .remove(&config_key(config));
        Ok(())
    }
    pub fn refresh(&self, config: &ProviderConfig) -> Result<Value> {
        if self
            .entries
            .lock()
            .map_err(|e| e.to_string())?
            .get(&config_key(config))
            .is_some_and(|e| e.flow.is_some())
        {
            return self.poll_or_refresh(config);
        }
        // Explicit refresh always performs a check; a concurrent OAuth/logout
        // wins the revision check instead of being overwritten by a late query.
        let _ = self.fresh_identity(config);
        self.snapshot(config)
    }
    pub fn fresh_identity(&self, config: &ProviderConfig) -> ProviderResult<String> {
        self.fresh_identity_with(config, providers::dreamina_account)
    }
    /// A shared known logout pauses the queue without one identical CLI call
    /// per item. Explicit UI refresh/OAuth and the normal refresh interval are
    /// still responsible for obtaining a new authenticated account identity.
    pub fn execution_identity(&self, config: &ProviderConfig) -> ProviderResult<String> {
        let public = self.snapshot(config).map_err(|_| local_state_error())?;
        if public["status"] == "logged_out" && !self.due(config).map_err(|_| local_state_error())? {
            let mut error = ProviderError::auth();
            error.message = s(&public, "message").to_owned();
            return Err(error);
        }
        self.fresh_identity(config)
    }
    /// Images in the same local queue share a short verified identity check.
    /// Video submission and explicit recovery retain execution_identity/fresh_identity.
    pub fn image_execution_identity(&self, config: &ProviderConfig) -> ProviderResult<String> {
        self.image_execution_identity_with(config, providers::dreamina_account)
    }
    fn image_execution_identity_with(
        &self,
        config: &ProviderConfig,
        account: impl FnOnce(&ProviderConfig) -> ProviderResult<Value>,
    ) -> ProviderResult<String> {
        let gate = self.identity_gate(config)?;
        let _check = gate.lock().map_err(|_| local_state_error())?;
        // Recheck after acquiring the per-installation gate: concurrent queue
        // items share the leader's verified result, not one CLI each.
        if let Some(entry) = self.entries.lock().map_err(|_| local_state_error())?
            .get(&config_key(config)).filter(|e| e.config == config_signature(config)) {
            if entry.flow.is_some() || entry.public["status"] == "authorizing" {
                return Err(ProviderError::auth());
            }
            if entry.public["status"] == "logged_out" && entry.checked.elapsed() < Duration::from_secs(60) {
                let mut error = ProviderError::auth();
                error.message = s(&entry.public, "message").to_owned();
                return Err(error);
            }
            if entry.checked.elapsed() < EXECUTION_IDENTITY_TTL
                && entry.public["ready"] == true && entry.public["identityVerified"] == true
                && !s(&entry.public, "accountKey").is_empty() {
                return Ok(s(&entry.public, "accountKey").to_owned());
            }
            if entry.checked.elapsed() < EXECUTION_FAILURE_TTL {
                if let Some(error) = &entry.failure { return Err(error.clone()); }
            }
        }
        self.check_identity_with(config, account)
    }
    fn identity_gate(&self, config: &ProviderConfig) -> ProviderResult<Arc<Mutex<()>>> {
        Ok(self.identity_checks.lock().map_err(|_| local_state_error())?
            .entry(config_key(config)).or_insert_with(|| Arc::new(Mutex::new(()))).clone())
    }
    fn fresh_identity_with(
        &self,
        config: &ProviderConfig,
        account: impl FnOnce(&ProviderConfig) -> ProviderResult<Value>,
    ) -> ProviderResult<String> {
        let gate = self.identity_gate(config)?;
        let _check = gate.lock().map_err(|_| local_state_error())?;
        self.check_identity_with(config, account)
    }
    fn check_identity_with(
        &self,
        config: &ProviderConfig,
        account: impl FnOnce(&ProviderConfig) -> ProviderResult<Value>,
    ) -> ProviderResult<String> {
        let changed_config = self.entries.lock().map_err(|_| local_state_error())?
            .get(&config_key(config)).is_some_and(|entry| entry.config != config_signature(config));
        if changed_config { self.forget(config).map_err(|_| local_state_error())?; }
        let revision = *self.revisions.lock().map_err(|_| local_state_error())?
            .entry(config_key(config)).or_default();
        if self
            .entries
            .lock()
            .map_err(|_| ProviderError::new("LOCAL_ACCOUNT_STATE", "即梦本地账户状态无法读取。"))?
            .get(&config_key(config))
            .is_some_and(|e| e.flow.is_some() || e.public["status"] == "authorizing")
        {
            let mut error = ProviderError::auth();
            error.message = "即梦正在等待用户授权，请完成官方授权后继续。".into();
            return Err(error);
        }
        // UI refresh returns a safe status projection. Execution must retain the
        // provider's error flags instead of turning every failed check into logout.
        let outcome = account(config);
        let revisions = self.revisions.lock().map_err(|_| local_state_error())?;
        if revisions.get(&config_key(config)).copied().unwrap_or_default() != revision {
            return Err(ProviderError::new("ACCOUNT_STATE_CHANGED", "即梦登录或配置已变化，请按最新状态继续。"));
        }
        let (account, failure) = match outcome {
            Ok(value) => (value, None),
            Err(error) => {
                let mut value = unknown();
                value["status"] = json!(if error.auth_required {
                    "logged_out"
                } else {
                    "error"
                });
                value["message"] = json!(error.message);
                value["errorCode"] = json!(error.code);
                value["checkedAt"] = json!(now());
                (value, Some(error))
            }
        };
        self.entries.lock().map_err(|_| local_state_error())?.insert(config_key(config), Entry {
            public: account.clone(), checked: Instant::now(), flow: None,
            config: config_signature(config), failure: failure.clone(),
        });
        drop(revisions);
        if let Some(error) = failure { return Err(error); }
        if account["ready"] != true
            || account["identityVerified"] != true
            || s(&account, "accountKey").is_empty()
        {
            let mut error = ProviderError::new(
                "ACCOUNT_IDENTITY_UNVERIFIED",
                s(&account, "message").to_owned(),
            );
            error.auth_required = true;
            return Err(error);
        }
        Ok(s(&account, "accountKey").to_owned())
    }
    pub fn begin(&self, config: &ProviderConfig) -> Result<Value> {
        let current = self.snapshot(config)?;
        if s(&current, "status") == "authorizing" {
            let active = self
                .entries
                .lock()
                .map_err(|e| e.to_string())?
                .get(&config_key(config))
                .and_then(|e| e.flow.as_ref())
                .is_some_and(|f| Instant::now() < f.expires);
            if active {
                return Ok(current);
            }
            self.forget(config)?;
        }
        // Invalidate queue checks before opening a new OAuth challenge, including
        // any account query already running on another thread.
        let mut pending = unknown();
        pending["status"] = json!("authorizing");
        pending["message"] = json!("正在打开即梦官方授权。");
        self.save(config, pending)?;
        let response = match providers::dreamina_begin_login(config) {
            Ok(response) => response,
            Err(error) => {
                let mut value = unknown();
                value["status"] = json!("error");
                value["message"] = json!(error.message);
                self.save(config, value)?;
                return Err(error.message);
            }
        };
        if s(&response, "status") == "authenticated" {
            return self.save(config, response);
        }
        self.install_flow(config, &response)
    }
    fn install_flow(&self, config: &ProviderConfig, response: &Value) -> Result<Value> {
        let code = response
            .get("device_code")
            .or_else(|| response.get("deviceCode"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("CLI 未返回可检查的授权流程")?
            .to_owned();
        let user_code = response
            .get("user_code")
            .or_else(|| response.get("userCode"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let url = response
            .get("verification_uri_complete")
            .or_else(|| response.get("verification_url"))
            .or_else(|| response.get("verification_uri"))
            .and_then(Value::as_str)
            .ok_or("CLI 未返回官方授权链接")?;
        let url = authorization_url(url)?;
        if reqwest::Url::parse(&url)
            .map_err(|_| "即梦授权链接无效")?
            .query_pairs()
            .any(|(_, v)| v.contains(&code))
        {
            return Err("CLI 授权链接含内部设备凭据，未打开授权页。".into());
        }
        let seconds = response["expires_in"].as_u64().unwrap_or(600).min(900);
        if seconds == 0 {
            return Err("即梦授权流程已过期，请重新连接。".into());
        }
        let interval = response["interval"].as_u64().unwrap_or(5).max(3);
        let value = json!({"status":"authorizing","ready":false,"identityVerified":false,"accountKey":null,"checkedAt":now(),"credits":{"total":null,"member":null,"gift":null,"purchase":null},"message":"请在即梦官方授权页确认，工作台会自动同步登录与积分。","authorization":{"verificationUrl":url,"userCode":user_code,"expiresAt":(chrono::Utc::now()+chrono::Duration::seconds(seconds as i64)).to_rfc3339(),"pollIntervalSeconds":interval}});
        let mut revisions = self.revisions.lock().map_err(|e| e.to_string())?;
        *revisions.entry(config_key(config)).or_default() += 1;
        self.entries.lock().map_err(|e| e.to_string())?.insert(
            config_key(config),
            Entry {
                public: value.clone(),
                checked: Instant::now(),
                config: config_signature(config),
                failure: None,
                flow: Some(Flow {
                    id: id(),
                    device_code: code,
                    expires: Instant::now() + Duration::from_secs(seconds),
                    next_poll: Instant::now() + Duration::from_secs(interval),
                    interval: Duration::from_secs(interval),
                    transient_failures: 0,
                }),
            },
        );
        Ok(value)
    }
    pub fn poll_or_refresh(&self, config: &ProviderConfig) -> Result<Value> {
        self.poll_or_refresh_with(config, providers::dreamina_check_login)
    }
    fn poll_or_refresh_with(
        &self,
        config: &ProviderConfig,
        check: impl FnOnce(&ProviderConfig, &str) -> ProviderResult<Value>,
    ) -> Result<Value> {
        let flow = {
            let entries = self.entries.lock().map_err(|e| e.to_string())?;
            entries
                .get(&config_key(config))
                .and_then(|e| e.flow.as_ref())
                .map(|f| (f.id.clone(), f.device_code.clone(), f.expires, f.next_poll))
        };
        let Some((flow_id, code, expires, next_poll)) = flow else {
            return self.refresh(config);
        };
        if Instant::now() >= expires {
            let mut value = unknown();
            value["status"] = json!("logged_out");
            value["message"] = json!("授权已超时，请重新连接即梦。");
            value["checkedAt"] = json!(now());
            return self.save(config, value);
        }
        if Instant::now() < next_poll {
            return self.snapshot(config);
        }
        let outcome = check(config, &code);
        let mut revisions = self.revisions.lock().map_err(|e| e.to_string())?;
        let mut entries = self.entries.lock().map_err(|e| e.to_string())?;
        let Some(entry) = entries.get_mut(&config_key(config)) else {
            return Ok(unknown());
        };
        if !entry.flow.as_ref().is_some_and(|f| f.id == flow_id) {
            return Ok(entry.public.clone());
        }
        *revisions.entry(config_key(config)).or_default() += 1;
        entry.checked = Instant::now();
        match outcome {
            Ok(account) if s(&account, "status") == "pending" => {
                if let Some(f) = &mut entry.flow {
                    f.transient_failures = 0;
                    if account["slowDown"] == true {
                        f.interval += Duration::from_secs(5);
                    }
                    f.next_poll = Instant::now() + f.interval;
                    entry.public["authorization"]["pollIntervalSeconds"] =
                        json!(f.interval.as_secs());
                }
            }
            Ok(account) => {
                entry.public = account;
                entry.flow = None;
            }
            Err(error) => {
                // Preserve the official challenge through brief transport failures.
                // Bound retries so an unavailable CLI never waits indefinitely.
                if !error.auth_required && matches!(error.code.as_str(), "CLI_NETWORK" | "CLI_TIMEOUT" | "CLI_QUERY_FAILED") {
                    if let Some(f) = &mut entry.flow {
                        f.transient_failures += 1;
                        if f.transient_failures < 3 {
                            f.next_poll = Instant::now() + f.interval;
                            entry.public["message"] = json!("连接暂时中断，正在重新检查授权结果。");
                            return Ok(entry.public.clone());
                        }
                    }
                }
                entry.public = unknown();
                entry.public["status"] = json!(if error.auth_required {
                    "logged_out"
                } else {
                    "error"
                });
                entry.public["message"] = json!(error.message);
                entry.public["checkedAt"] = json!(now());
                entry.flow = None;
            }
        }
        Ok(entry.public.clone())
    }
    pub fn logout(&self, config: &ProviderConfig) -> Result<Value> {
        let mut value = unknown();
        value["status"] = json!("logged_out");
        value["message"] = json!("正在退出即梦登录。");
        self.save(config, value.clone())?;
        providers::dreamina_logout(config).map_err(|e| e.message)?;
        value["message"] = json!("本机官方 CLI 已退出登录，云端任务不会取消。");
        value["checkedAt"] = json!(now());
        self.save(config, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oauth_transport_retry_is_bounded_and_keeps_the_original_challenge() {
        let sessions = DreaminaSessions::default();
        let c: ProviderConfig = serde_json::from_value(json!({"kind":"dreamina","cliPath":"C:/offline/never-run.exe"})).unwrap();
        let response = json!({"device_code":"private-challenge","verification_uri":"https://jimeng.jianying.com/ai-tool/cli-auth","expires_in":600,"interval":3});
        sessions.install_flow(&c, &response).unwrap();
        let due = || {
            sessions.entries.lock().unwrap().get_mut(&config_key(&c)).unwrap().flow.as_mut().unwrap().next_poll = Instant::now();
        };
        for attempt in 1..=3 {
            due();
            let value = sessions.poll_or_refresh_with(&c, |_, code| {
                assert_eq!(code, "private-challenge");
                Err(ProviderError::new("CLI_NETWORK", "网络连接暂时失败"))
            }).unwrap();
            assert_eq!(value["status"], if attempt < 3 { "authorizing" } else { "error" });
            assert!(!value.to_string().contains("private-challenge"));
        }
        assert!(sessions.entries.lock().unwrap().get(&config_key(&c)).unwrap().flow.is_none());
        // A fresh user retry can recover. A successful pending response resets
        // consecutive transport failures; verified completion replaces the flow.
        sessions.install_flow(&c, &response).unwrap();
        for _ in 0..3 {
            due();
            assert_eq!(sessions.poll_or_refresh_with(&c, |_, _| Err(ProviderError::new("CLI_TIMEOUT", "timeout"))).unwrap()["status"], "authorizing");
            due();
            sessions.poll_or_refresh_with(&c, |_, _| Ok(json!({"status":"pending"}))).unwrap();
        }
        due();
        let completed = sessions.poll_or_refresh_with(&c, |_, _| Ok(json!({"status":"authenticated","ready":true,"identityVerified":true,"accountKey":"verified","credits":{"total":14414}}))).unwrap();
        assert_eq!(completed["credits"]["total"], 14414);
        assert!(sessions.entries.lock().unwrap().get(&config_key(&c)).unwrap().flow.is_none());
        sessions.install_flow(&c, &response).unwrap();
        due();
        assert_eq!(sessions.poll_or_refresh_with(&c, |_, _| Err(ProviderError::new("CLI_AUTH_DENIED", "授权被拒绝"))).unwrap()["status"], "error");
        assert!(sessions.entries.lock().unwrap().get(&config_key(&c)).unwrap().flow.is_none());
    }
    #[test]
    fn known_logout_pauses_shared_queue_checks_until_refresh_without_trusting_cached_identity() {
        let sessions = DreaminaSessions::default();
        let config: ProviderConfig = serde_json::from_value(json!({"kind":"dreamina","cliPath":"C:/offline/never-run.exe"})).unwrap();
        sessions.seed_for_test(&config,json!({"status":"logged_out","ready":false,"message":"请重新登录"})).unwrap();
        for _ in 0..16 {
            let error = sessions.image_execution_identity_with(&config, |_| panic!("known logout must not start one CLI process per queued image")).unwrap_err();
            assert!(error.auth_required);
        }
        // A verified refresh is reusable only for the short execution TTL.
        sessions.seed_for_test(&config,json!({"status":"authenticated","ready":true,"identityVerified":true,"accountKey":"old-account"})).unwrap();
        assert_eq!(sessions.image_execution_identity_with(&config, |_| panic!("verified short cache must be shared")).unwrap(),"old-account");
        assert_eq!(sessions.fresh_identity_with(&config, |_| Ok(json!({"status":"authenticated","ready":true,"identityVerified":true,"accountKey":"new-account"}))).unwrap(),"new-account");
    }
    fn offline_config() -> ProviderConfig {
        serde_json::from_value(json!({"kind":"dreamina","cliPath":"C:/offline/never-run.exe"})).unwrap()
    }
    fn verified(key: &str) -> Value {
        json!({"status":"authenticated","ready":true,"identityVerified":true,"accountKey":key})
    }
    #[test]
    fn execution_identity_concurrent_queue_items_share_one_verified_check() {
        use std::sync::{atomic::{AtomicUsize, Ordering}, Barrier};
        let sessions = DreaminaSessions::default();
        let config = offline_config();
        let calls = AtomicUsize::new(0);
        let barrier = Barrier::new(12);
        std::thread::scope(|scope| {
            for _ in 0..12 {
                scope.spawn(|| {
                    barrier.wait();
                    assert_eq!(sessions.image_execution_identity_with(&config, |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(30));
                        Ok(verified("shared"))
                    }).unwrap(), "shared");
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn execution_identity_expires_and_config_changes_bypass_verified_cache() {
        let sessions = DreaminaSessions::default();
        let config = offline_config();
        sessions.seed_for_test(&config, verified("old")).unwrap();
        assert_eq!(sessions.image_execution_identity_with(&config, |_| panic!("fresh cache")).unwrap(), "old");
        sessions.entries.lock().unwrap().get_mut(&config_key(&config)).unwrap().checked = Instant::now() - EXECUTION_IDENTITY_TTL;
        assert_eq!(sessions.image_execution_identity_with(&config, |_| Ok(verified("expired-refresh"))).unwrap(), "expired-refresh");
        let mut changed = config.clone();
        changed.endpoint = Some("https://offline.invalid".into());
        assert_eq!(sessions.image_execution_identity_with(&changed, |_| Ok(verified("new-config"))).unwrap(), "new-config");
        assert_eq!(sessions.snapshot(&config).unwrap()["status"], "unknown");
        sessions.forget(&changed).unwrap();
        assert_eq!(sessions.image_execution_identity_with(&changed, |_| Ok(verified("after-forget"))).unwrap(), "after-forget");
    }
    #[test]
    fn execution_identity_failure_is_shared_as_failure_and_never_reuses_old_success() {
        use std::sync::{atomic::{AtomicUsize, Ordering}, Barrier};
        let sessions = DreaminaSessions::default();
        let config = offline_config();
        sessions.seed_for_test(&config, verified("old")).unwrap();
        sessions.entries.lock().unwrap().get_mut(&config_key(&config)).unwrap().checked = Instant::now() - EXECUTION_IDENTITY_TTL;
        let calls = AtomicUsize::new(0);
        let barrier = Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    barrier.wait();
                    let error = sessions.image_execution_identity_with(&config, |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(30));
                        Err(ProviderError::new("CLI_NETWORK", "offline outage"))
                    }).unwrap_err();
                    assert_eq!(error.code, "CLI_NETWORK");
                    assert!(!error.auth_required);
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(sessions.snapshot(&config).unwrap()["ready"], false);
        // Explicit refresh bypasses even the short failed-flight coalescing.
        assert_eq!(sessions.fresh_identity_with(&config, |_| Ok(verified("recovered"))).unwrap(), "recovered");
    }
    #[test]
    fn execution_identity_late_query_cannot_resurrect_logout_forget_or_oauth() {
        for transition in ["logout", "forget", "oauth"] {
            let sessions = DreaminaSessions::default();
            let config = offline_config();
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (finish_tx, finish_rx) = std::sync::mpsc::channel();
            std::thread::scope(|scope| {
                let sessions_ref = &sessions;
                let config_ref = &config;
                let check = scope.spawn(move || sessions_ref.image_execution_identity_with(config_ref, |_| {
                    started_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    Ok(verified("stale-account"))
                }));
                started_rx.recv().unwrap();
                match transition {
                    "logout" => { sessions.save(&config, json!({"status":"logged_out","ready":false,"identityVerified":false})).unwrap(); }
                    "forget" => sessions.forget(&config).unwrap(),
                    "oauth" => { sessions.install_flow(&config, &json!({"device_code":"private","verification_uri":"https://jimeng.jianying.com/ai-tool/cli-auth","expires_in":60})).unwrap(); }
                    _ => unreachable!(),
                }
                finish_tx.send(()).unwrap();
                assert_eq!(check.join().unwrap().unwrap_err().code, "ACCOUNT_STATE_CHANGED");
            });
            let expected = match transition { "logout"=>"logged_out", "forget"=>"unknown", _=>"authorizing" };
            assert_eq!(sessions.snapshot(&config).unwrap()["status"], expected);
            if transition != "forget" {
                assert!(sessions.image_execution_identity_with(&config, |_| panic!("paused identity must not query")).unwrap_err().auth_required);
            }
        }
    }
    #[test]
    fn fresh_identity_keeps_transient_provider_errors_typed_without_claiming_logout() {
        let sessions = DreaminaSessions::default();
        let config: ProviderConfig =
            serde_json::from_value(json!({"kind":"dreamina","cliPath":"C:/offline/never-run.exe"}))
                .unwrap();
        let expected = ProviderError::new("CLI_QUERY_FAILED", "safe diagnostic");
        let error = sessions
            .fresh_identity_with(&config, |_| Err(expected.clone()))
            .unwrap_err();
        assert_eq!(error.code, expected.code);
        assert_eq!(error.message, expected.message);
        assert!(!error.auth_required);
        assert!(!error.submission_unknown);
        let view = sessions.snapshot(&config).unwrap();
        assert_eq!(view["status"], "error");
        assert_eq!(view["errorCode"], "CLI_QUERY_FAILED");
        assert_eq!(view["ready"], false);
    }
    #[test]
    fn fresh_identity_keeps_real_authentication_failures_and_requires_verified_identity() {
        let sessions = DreaminaSessions::default();
        let config: ProviderConfig =
            serde_json::from_value(json!({"kind":"dreamina","cliPath":"C:/offline/never-run.exe"}))
                .unwrap();
        let error = sessions
            .fresh_identity_with(&config, |_| Err(ProviderError::auth()))
            .unwrap_err();
        assert!(error.auth_required);
        assert_eq!(sessions.snapshot(&config).unwrap()["status"], "logged_out");
        let error=sessions.fresh_identity_with(&config, |_|Ok(json!({"ready":true,"identityVerified":false,"accountKey":"untrusted","message":"identity missing"}))).unwrap_err();
        assert!(error.auth_required);
        assert_eq!(error.code, "ACCOUNT_IDENTITY_UNVERIFIED");
        let identity=sessions.fresh_identity_with(&config, |_|Ok(json!({"ready":true,"identityVerified":true,"accountKey":"verified-offline-account"}))).unwrap();
        assert_eq!(identity, "verified-offline-account");
    }
    #[test]
    fn authorization_links_require_official_https_and_never_expose_internal_codes() {
        assert!(
            authorization_url("https://jimeng.jianying.com/ai-tool/auth?user_code=ABCD").is_ok()
        );
        for url in [
            "https://jimeng.jianying.com.evil.test/",
            "http://jimeng.jianying.com/",
            "https://evil.test/",
            "https://u:p@jimeng.jianying.com/",
            "https://jimeng.jianying.com/?device_code=secret",
            "https://jimeng.jianying.com/#token",
        ] {
            assert!(authorization_url(url).is_err(), "{url}");
        }
    }
    #[test]
    fn account_cache_is_scoped_to_cli_and_device_secrets_are_never_projected() {
        let sessions = DreaminaSessions::default();
        let c: ProviderConfig =
            serde_json::from_value(json!({"kind":"dreamina","cliPath":"C:/official/dreamina.exe"}))
                .unwrap();
        let view = json!({"status":"authorizing","ready":false,"authorization":{"userCode":"PUBLIC","verificationUrl":"https://jimeng.jianying.com/"}});
        sessions.entries.lock().unwrap().insert(
            config_key(&c),
            Entry {
                public: view.clone(),
                checked: Instant::now(),
                config: config_signature(&c),
                failure: None,
                flow: Some(Flow {
                    id: "flow".into(),
                    device_code: "INTERNAL_SECRET".into(),
                    expires: Instant::now() + Duration::from_secs(20),
                    next_poll: Instant::now() + Duration::from_secs(20),
                    interval: Duration::from_secs(5),
                    transient_failures: 0,
                }),
            },
        );
        assert_eq!(sessions.snapshot(&c).unwrap(), view);
        assert!(!sessions
            .snapshot(&c)
            .unwrap()
            .to_string()
            .contains("INTERNAL_SECRET"));
        assert!(!sessions.due(&c).unwrap());
        // Neither repeated status refresh nor a generation preflight destroys a live flow.
        // The CLI path intentionally does not exist: these calls must not launch it.
        assert_eq!(sessions.poll_or_refresh(&c).unwrap(), view);
        assert_eq!(sessions.refresh(&c).unwrap(), view);
        assert!(sessions.fresh_identity(&c).is_err());
        assert_eq!(sessions.snapshot(&c).unwrap(), view);
        let other: ProviderConfig =
            serde_json::from_value(json!({"kind":"dreamina","cliPath":"C:/other/dreamina.exe"}))
                .unwrap();
        assert_eq!(sessions.snapshot(&other).unwrap()["status"], "unknown");
    }
    #[test]
    fn oauth_server_timing_is_respected_and_secret_urls_are_rejected() {
        let sessions = DreaminaSessions::default();
        let c: ProviderConfig = serde_json::from_value(
            json!({"kind":"dreamina","cliPath":"C:/nonexistent/dreamina.exe"}),
        )
        .unwrap();
        let response = json!({"device_code":"PRIVATE_DEVICE","verification_uri":"https://jimeng.jianying.com/ai-tool/auth","user_code":"PUBLIC","expires_in":5,"interval":60});
        let view = sessions.install_flow(&c, &response).unwrap();
        assert_eq!(view["authorization"]["pollIntervalSeconds"], 60);
        let entries = sessions.entries.lock().unwrap();
        let flow = entries.get(&config_key(&c)).unwrap().flow.as_ref().unwrap();
        assert!(flow.expires.saturating_duration_since(Instant::now()) <= Duration::from_secs(5));
        assert_eq!(flow.interval, Duration::from_secs(60));
        drop(entries);
        let mut bad = response.clone();
        bad["verification_uri"] = json!("https://jimeng.jianying.com/auth?code=PRIVATE_DEVICE");
        assert!(sessions.install_flow(&c, &bad).is_err());
        for url in [
            "https://jimeng.jianying.com/auth?deviceCode=private",
            "https://jimeng.jianying.com/auth?session_token=private",
        ] {
            assert!(authorization_url(url).is_err());
        }
    }
}
