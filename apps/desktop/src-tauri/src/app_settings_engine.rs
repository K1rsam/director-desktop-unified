use super::*;
use crate::app_settings;

#[path = "app_settings_engine/dreamina_refresh.rs"]
mod dreamina_refresh;

impl Engine {
    pub(super) fn global_capability_cache(
        &self,
        provider: &str,
        fingerprint: &str,
    ) -> Result<Option<Capabilities>> {
        if !self.standard_native {
            return Ok(None);
        }
        let Some(doc) = app_settings::load(&self.home)? else {
            return Ok(None);
        };
        let entry = &doc["catalogs"][provider];
        if entry["fingerprint"] != fingerprint {
            return Ok(None);
        }
        let Ok(cap) = serde_json::from_value::<Capabilities>(entry["capabilities"].clone()) else {
            return Ok(None);
        };
        Ok((cap.provider == provider
            && !cap.version.is_empty()
            && timestamp_fresh(&cap.checked_at, 900))
        .then_some(cap))
    }
    pub(super) fn global_planning_capabilities(&self, store: &Store) -> Result<Option<Value>> {
        let settings = store.settings()?;
        if s(&settings, "videoModel").is_empty() {
            return Ok(None);
        }
        let provider = s(&settings, "videoProvider");
        let (config, account) = self.config(store, provider)?;
        self.global_capability_cache(provider, &capability_fingerprint(&config, &account))?
            .map(|cap| planning_capabilities_from_catalog(&settings, &json!(cap)))
            .transpose()
    }
    pub(super) fn effective_settings(&self, store: &Store) -> Result<Value> {
        let project = store.settings()?;
        if self.standard_native {
            app_settings::effective(&self.home, &project)
        } else {
            Ok(project)
        }
    }
    fn global_settings_document(&self, request: &Value) -> Result<Value> {
        app_settings::initialize(
            &self.home,
            &self.registry()?,
            request["migrationSourceProjectId"].as_str(),
        )
    }
    fn global_settings_view(&self, doc: &Value) -> Result<Value> {
        let config = self.dreamina_config_with_settings(&doc["settings"])?;
        let account = self.dreamina_sessions.snapshot(&config)?;
        let rules: Vec<_> = pipeline::rule_snapshots()
            .into_iter()
            .map(|r| json!({"id":r.id,"version":r.version,"sha256":r.sha256}))
            .collect();
        Ok(app_settings::public(doc, account, json!(rules)))
    }
    pub(super) fn dispatch_global_settings(self: &Arc<Self>, request: &Value) -> Result<Value> {
        let action = s(request, "action");
        let allowed = [
            "settings.global.get",
            "settings.global.save",
            "runtime.status",
            "runtime.prepare",
            "chrome.status",
            "chrome.connect",
            "chrome.check",
            "chrome.bind",
            "chrome.open",
            "chrome.managed.status",
            "chrome.managed.start",
            "chrome.managed.login",
            "chrome.managed.recover",
            "readiness.check",
            "textSdk.status",
            "textCli.status",
            "textCli.login",
            "textCli.loginStatus",
            "textCli.cancelLogin",
            "textCli.openLogin",
            "dreamina.status",
            "dreamina.login",
            "dreamina.logout",
            "dreamina.open_authorization",
            "providers.refresh",
        ];
        if !allowed.contains(&action) {
            return Err("此操作不属于软件渠道设置。".into());
        }
        if action == "chrome.open" {
            if !self.standard_native || !self.live_accounts {
                return Err("Chrome 连接只由本机标准桌面显式触发。".into());
            }
            if !request["browser"].is_null() && !request["browser"].is_string() {return Err("浏览器选择格式无效，未打开。".into());}
            return crate::chrome_setup::open(&self.home, request["browser"].as_str());
        }
        if action.starts_with("chrome.managed.") {
            if !self.standard_native || !self.live_accounts {
                return Err("专属浏览器仅由本机标准桌面显式使用。".into());
            }
            let saved = app_settings::load(&self.home)?
                .map(|doc| app_settings::effective_from(&doc, &json!({})))
                .unwrap_or_else(app_settings::defaults);
            let token = app_settings::credential(&saved, "global", "chrome").ok();
            if action == "chrome.managed.status" {
                return crate::managed_browser::perform(&self.home, &saved, action, token.as_deref());
            }
            let _channel = self.acquire_keys(vec!["channel:chrome".into()])?;
            let _lifecycle = self.lifecycle_mutex.lock().map_err(|e|e.to_string())?;
            let _registry = crate::project_library::lock(&self.home)?;
            if self.active.lock().map_err(|e|e.to_string())?.iter().any(|key|key.starts_with("project:")) {
                return Err("本地项目操作尚未结束，保留专属浏览器；请等待当前操作完成后再恢复。".into());
            }
            crate::managed_browser::ensure_clear(&self.registry()?)?;
            return crate::managed_browser::perform(&self.home, &saved, action, token.as_deref());
        }
        if action == "runtime.status" || action == "chrome.status" {
            let saved = app_settings::load(&self.home)?
                .map(|doc| app_settings::effective_from(&doc, &json!({})))
                .unwrap_or_else(app_settings::defaults);
            if action == "chrome.status" {
                return Ok(crate::chrome_setup::status(&self.home, &saved));
            }
            return crate::runtime_setup::status(
                request,
                &saved,
                &self.home,
                self.readiness_resources.as_deref(),
            );
        }
        let doc = self.global_settings_document(request)?;
        let settings = app_settings::effective_from(&doc, &json!({}));
        match action {
            "settings.global.get" => self.global_settings_view(&doc),
            "settings.global.save" => {
                let expected = request["expectedRevision"]
                    .as_u64()
                    .ok_or("请刷新软件设置后保存。")?;
                let updated = app_settings::save(&self.home, &request["settings"], expected)?;
                self.global_settings_view(&updated)
            }
            "chrome.connect" | "chrome.check" | "chrome.bind" => {
                if !self.standard_native || !self.live_accounts {
                    return Err("Chrome 连接只由本机标准桌面显式触发。".into());
                }
                let _guard = self.acquire_keys(vec!["channel:chrome".into()])?;
                let token = app_settings::credential(&settings, "global", "chrome").ok();
                if action=="chrome.bind" {
                    let token=token.as_deref().ok_or("请先连接 Chrome 网页。")?;
                    let profile=request["profileId"].as_str().filter(|s|!s.is_empty()).ok_or("请选择执行浏览器。")?;
                    let managed=request["managed"]==true;
                    let _managed_lifecycle=if managed {Some(self.lifecycle_mutex.lock().map_err(|e|e.to_string())?)}else{None};
                    let _managed_registry=if managed {Some(crate::project_library::lock(&self.home)?)}else{None};
                    if request["managed"]==true {
                        if self.active.lock().map_err(|e|e.to_string())?.iter().any(|key|key.starts_with("project:")) {
                            return Err("本地项目操作尚未结束，未切换图片执行浏览器。".into());
                        }
                        crate::managed_browser::ensure_clear(&self.registry()?)?;
                        let status=crate::managed_browser::perform(&self.home,&settings,"chrome.managed.status",Some(token))?;
                        if status["ready"]!=true || status["profileId"]!=profile {
                            return Err("专属 Edge 尚未验证登录或执行身份，未切换新任务的浏览器。".into());
                        }
                    }
                    let observed=crate::chrome_setup::inspect(&settings,token)?;
                    let patch=crate::chrome_setup::binding_patch(&settings,&observed,Some(profile))?.ok_or("所选浏览器无法绑定。")?;
                    let updated=app_settings::save(&self.home,&patch,doc["revision"].as_u64().unwrap_or(0))?;
                    return crate::chrome_setup::check(&self.home,&app_settings::effective_from(&updated,&json!({})),token);
                }
                if action == "chrome.check" {
                    return crate::chrome_setup::check(
                        &self.home,
                        &settings,
                        token.as_deref().ok_or("请先连接 Chrome 网页。")?,
                    );
                }
                let (_, patch) = crate::chrome_setup::connect(
                    &self.home,
                    &settings,
                    token.as_deref(),
                    request["sourceConfigPath"].as_str(),
                )?;
                if let Some(patch) = patch {
                    let updated = app_settings::save(
                        &self.home,
                        &patch,
                        doc["revision"].as_u64().unwrap_or(0),
                    )?;
                    let updated = app_settings::effective_from(&updated, &json!({}));
                    let token = app_settings::credential(&updated, "global", "chrome")?;
                    return crate::chrome_setup::check(&self.home, &updated, &token);
                }
                crate::chrome_setup::check(
                    &self.home,
                    &settings,
                    token.as_deref().ok_or("请先连接 Chrome 网页。")?,
                )
            }
            "runtime.prepare" => {
                let _guard = self.acquire_keys(vec!["runtime:prepare".into()])?;
                crate::runtime_setup::prepare(
                    request,
                    &settings,
                    &self.home,
                    self.readiness_resources.as_deref(),
                )
            }
            "readiness.check" => {
                let (provider, settings) = crate::readiness::request_settings(request, &settings)?;
                let _guard = self.acquire_keys(vec![format!("readiness:{provider}")])?;
                Ok(
                    crate::text_cli::control_runtime()?.block_on(crate::readiness::check(
                        &provider,
                        &settings,
                        self.readiness_resources.as_deref(),
                    )),
                )
            }
            "textSdk.status" => Ok(
                crate::text_cli::control_runtime()?.block_on(crate::text_sdk::status(&settings))
            ),
            "textCli.status"
            | "textCli.login"
            | "textCli.loginStatus"
            | "textCli.cancelLogin"
            | "textCli.openLogin" => {
                if !self.live_accounts {
                    return Err("离线验收模式不读取宿主账户或启动登录。".into());
                }
                let runtime = crate::text_cli::control_runtime()?;
                match action {
                    "textCli.status" => {
                        let (_, settings) = crate::readiness::request_settings(
                            &json!({"provider":"codex_cli","settings":request["settings"]}),
                            &settings,
                        )?;
                        Ok(runtime.block_on(crate::text_cli::status(&settings, true)))
                    }
                    "textCli.login" => runtime.block_on(crate::text_cli::start_login(&settings)),
                    "textCli.loginStatus" => Ok(crate::text_cli::login_status()),
                    "textCli.cancelLogin" => Ok(crate::text_cli::cancel_login()),
                    _ => runtime.block_on(crate::text_cli::open_login_url()),
                }
            }
            "dreamina.status"
            | "dreamina.login"
            | "dreamina.logout"
            | "dreamina.open_authorization" => {
                if !self.live_accounts {
                    return Err("离线验收模式不读取宿主即梦账户或启动登录。".into());
                }
                let config = self.dreamina_config_with_settings(&settings)?;
                if action == "dreamina.status" {
                    return dreamina_refresh::refresh(self, &doc, &config, || {
                        self.dreamina_sessions.poll_or_refresh(&config)
                    });
                }
                let _guard = self.acquire_keys(vec!["channel:dreamina".into()])?;
                let account = match action {
                    "dreamina.login" => {
                        let account = self.dreamina_sessions.begin(&config)?;
                        if s(&account, "status") == "authorizing" {
                            open_authorization(s(&account["authorization"], "verificationUrl"))?;
                        }
                        account
                    }
                    "dreamina.logout" => self.dreamina_sessions.logout(&config)?,
                    "dreamina.open_authorization" => {
                        let account = self.dreamina_sessions.snapshot(&config)?;
                        if s(&account, "status") != "authorizing" {
                            return Err("没有正在进行的即梦授权，请先连接。".into());
                        }
                        open_authorization(s(&account["authorization"], "verificationUrl"))?;
                        account
                    }
                    _ => self.dreamina_sessions.poll_or_refresh(&config)?,
                };
                // Global checks never iterate projects or resume media jobs.
                Ok(account)
            }
            "providers.refresh" => {
                let provider = s(request, "provider");
                if !["dreamina", "bh", "chrome"].contains(&provider) {
                    return Err("不支持的渠道。".into());
                }
                let _guard = self.acquire_keys(vec![format!("channel:{provider}")])?;
                let (config, mut account) =
                    self.config_for_identity("global", provider, &settings)?;
                if provider == "dreamina" && self.live_accounts {
                    model_refresh_account(
                        self.dreamina_sessions.snapshot(&config)?,
                        self.dreamina_sessions.due(&config)?,
                        || self.dreamina_sessions.poll_or_refresh(&config),
                    )?;
                    account = self.config_for_identity("global", provider, &settings)?.1;
                }
                let fingerprint = capability_fingerprint(&config, &account);
                let cap = self.channels.capabilities(&config).map_err(|e| e.message)?;
                if cap.provider != provider
                    || cap.version.is_empty()
                    || !timestamp_fresh(&cap.checked_at, 900)
                {
                    return Err("渠道返回的模型目录身份或时间无效。".into());
                }
                let current = app_settings::load(&self.home)?.ok_or("软件设置已变化。")?;
                let (current_config, current_account) = self.config_for_identity(
                    "global",
                    provider,
                    &app_settings::effective_from(&current, &json!({})),
                )?;
                if capability_fingerprint(&current_config, &current_account) != fingerprint {
                    return Err("渠道或账户在检查期间变化，请重新检查。".into());
                }
                app_settings::cache(
                    &self.home,
                    provider,
                    doc["revision"].as_u64().unwrap_or(0),
                    &fingerprint,
                    &serde_json::to_value(&cap).map_err(|e| e.to_string())?,
                )?;
                Ok(json!(cap))
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
#[path = "app_settings_engine/tests.rs"]
mod tests;
