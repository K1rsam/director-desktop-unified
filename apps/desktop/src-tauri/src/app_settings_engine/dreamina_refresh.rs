//! A read-only status click may share a check already owned by this Engine.
//! Once busy is observed this request never starts a second CLI operation.
use super::*;

const WAIT: Duration = Duration::from_secs(15);
const INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn refresh(
    engine: &Arc<Engine>,
    doc: &Value,
    config: &ProviderConfig,
    check: impl FnOnce() -> Result<Value>,
) -> Result<Value> {
    refresh_with_wait(engine, doc, config, WAIT, INTERVAL, check)
}

fn refresh_with_wait(
    engine: &Arc<Engine>,
    doc: &Value,
    config: &ProviderConfig,
    wait: Duration,
    interval: Duration,
    check: impl FnOnce() -> Result<Value>,
) -> Result<Value> {
    let baseline = engine.dreamina_sessions.snapshot(config)?;
    match engine.acquire_keys(vec!["channel:dreamina".into()]) {
        Ok(_guard) => return check(), // Preserve the existing explicit check/OAuth poll.
        Err(error) => {
            if engine.stop.load(Ordering::SeqCst) {
                return Err(error);
            }
            // A poisoned mutex is a local failure, never an account refresh in progress.
            drop(engine.active.lock().map_err(|e| e.to_string())?);
        }
    }
    let started = Instant::now();
    loop {
        if engine.stop.load(Ordering::SeqCst) {
            return Err("客户端正在退出".into());
        }
        let current_doc =
            app_settings::load(&engine.home)?.ok_or("软件设置已变化，请重新刷新。")?;
        let current_config = engine.dreamina_config_with_settings(
            &app_settings::effective_from(&current_doc, &json!({})),
        )?;
        if current_doc["revision"] != doc["revision"]
            || serde_json::to_value(&current_config).map_err(|e| e.to_string())?
                != serde_json::to_value(config).map_err(|e| e.to_string())?
        {
            return Err("等待期间即梦连接设置已变化，请按当前设置刷新。".into());
        }
        let account = engine.dreamina_sessions.snapshot(config)?;
        if session_changed(&baseline, &account) {
            return Err("等待期间即梦登录状态已切换，请按当前账户刷新。".into());
        }
        let completed = fresh_snapshot(&baseline, &account);
        if completed || started.elapsed() >= wait {
            return Ok(json!({
                "schemaVersion":"dreamina-refresh/v1",
                "refreshState":if completed {"completed"} else {"pending"},
                "settingsRevision":doc["revision"],
                "configuredPath":doc["settings"]["dreaminaPath"],
                "afterCheckedAt":baseline["checkedAt"],
                "afterStatus":baseline["status"],
                "authorizationExpiresAt":baseline["authorization"]["expiresAt"],
                "accountKey":baseline["accountKey"],
                "account":account,
                "message":if completed {
                    if s(&account,"message").is_empty() {"已读取已有操作完成后的最新账户状态。"} else {s(&account,"message")}
                } else {"即梦已有操作正在进行，正在等待最新账户状态；不会重复发起检查。"}
            }));
        }
        std::thread::sleep(interval.min(wait.saturating_sub(started.elapsed())));
    }
}

fn fresh_snapshot(baseline: &Value, account: &Value) -> bool {
    if account["status"] == "authorizing" {
        return false;
    }
    let Ok(current) = chrono::DateTime::parse_from_rfc3339(s(account, "checkedAt")) else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(s(baseline, "checkedAt")) {
        Ok(previous) => current > previous,
        Err(_) => true,
    }
}

fn session_changed(baseline: &Value, account: &Value) -> bool {
    let prior = s(baseline, "accountKey");
    let current = s(account, "accountKey");
    (!prior.is_empty() && !current.is_empty() && prior != current)
        || (account["status"] == "authorizing"
            && (baseline["status"] != "authorizing"
                || baseline["authorization"]["expiresAt"] != account["authorization"]["expiresAt"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn setup() -> (tempfile::TempDir, Arc<Engine>, Value, ProviderConfig) {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::new_offline_verification(temp.path().join("home")).unwrap();
        let doc = engine.global_settings_document(&json!({})).unwrap();
        let config = engine
            .dreamina_config_with_settings(&doc["settings"])
            .unwrap();
        (temp, engine, doc, config)
    }
    fn account(time: &str) -> Value {
        json!({"status":"authenticated","ready":true,"identityVerified":true,
            "accountKey":"synthetic-account","checkedAt":time,"message":"synthetic verified"})
    }
    const OLD: &str = "2026-10-10T00:00:00Z";
    const NEW: &str = "2026-10-10T00:00:01Z";

    #[test]
    fn busy_refresh_reuses_new_snapshot_without_dispatch_and_retains_owner_lock() {
        let (_temp, engine, doc, config) = setup();
        engine
            .dreamina_sessions
            .seed_for_test(&config, account(OLD))
            .unwrap();
        let owner = engine
            .acquire_keys(vec!["channel:dreamina".into()])
            .unwrap();
        let calls = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(20));
                engine
                    .dreamina_sessions
                    .seed_for_test(&config, account(NEW))
                    .unwrap();
            });
            let result = refresh_with_wait(
                &engine,
                &doc,
                &config,
                Duration::from_secs(2),
                Duration::from_millis(2),
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    panic!("busy click must not execute a CLI check")
                },
            )
            .unwrap();
            assert_eq!(result["refreshState"], "completed");
            assert_eq!(result["account"]["checkedAt"], NEW);
            assert_eq!(result["message"], "synthetic verified");
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert!(engine
                .acquire_keys(vec!["channel:dreamina".into()])
                .is_err());
        });
        drop(owner);
        assert!(engine.acquire_keys(vec!["channel:dreamina".into()]).is_ok());
        assert_eq!(engine.registry().unwrap(), json!([]));
    }

    #[test]
    fn busy_refresh_timeout_keeps_old_checked_at_and_never_dispatches_for_any_owner() {
        for owner in ["channel:dreamina", "image:dreamina:synthetic"] {
            let (_temp, engine, doc, config) = setup();
            engine
                .dreamina_sessions
                .seed_for_test(&config, account(OLD))
                .unwrap();
            let _owner = engine.acquire_keys(vec![owner.into()]).unwrap();
            let result = refresh_with_wait(
                &engine,
                &doc,
                &config,
                Duration::ZERO,
                Duration::ZERO,
                || panic!("no dispatch on busy"),
            )
            .unwrap();
            assert_eq!(result["refreshState"], "pending");
            assert_eq!(result["account"]["checkedAt"], OLD);
            assert_eq!(result["afterCheckedAt"], OLD);
            assert_eq!(
                engine.dreamina_sessions.snapshot(&config).unwrap(),
                account(OLD)
            );
            assert_eq!(engine.registry().unwrap(), json!([]));
        }
    }

    #[test]
    fn stale_authorizing_or_changed_identity_is_never_reported_as_a_fresh_check() {
        assert!(!fresh_snapshot(&account(OLD), &account(OLD)));
        assert!(!fresh_snapshot(&account(NEW), &account(OLD)));
        assert!(!fresh_snapshot(&account(OLD), &json!({"status":"unknown"})));
        assert!(!fresh_snapshot(
            &account(OLD),
            &json!({"status":"authorizing","checkedAt":NEW})
        ));
        let mut switched = account(NEW);
        switched["accountKey"] = json!("other-synthetic-account");
        assert!(session_changed(&account(OLD), &switched));
        assert!(session_changed(
            &account(OLD),
            &json!({"status":"authorizing"})
        ));
        assert!(!session_changed(
            &json!({"status":"authorizing","authorization":{"expiresAt":NEW,"pollIntervalSeconds":3}}),
            &json!({"status":"authorizing","authorization":{"expiresAt":NEW,"pollIntervalSeconds":8}})
        ));
        // A completed failed account check is fresh evidence, not authenticated success.
        assert!(fresh_snapshot(
            &account(OLD),
            &json!({"status":"error","checkedAt":NEW})
        ));
    }

    #[test]
    fn waiting_refresh_aborts_on_settings_change_shutdown_or_account_switch() {
        for change in ["settings", "path", "account", "shutdown"] {
            let (_temp, engine, doc, config) = setup();
            engine
                .dreamina_sessions
                .seed_for_test(&config, account(OLD))
                .unwrap();
            let _owner = engine
                .acquire_keys(vec!["channel:dreamina".into()])
                .unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    std::thread::sleep(Duration::from_millis(20));
                    match change {
                        "settings" | "path" => {
                            let patch = if change == "path" {
                                json!({"dreaminaPath":"C:/synthetic/changed.exe"})
                            } else {
                                json!({"reasoning":"high"})
                            };
                            app_settings::save(
                                &engine.home,
                                &patch,
                                doc["revision"].as_u64().unwrap(),
                            )
                            .unwrap();
                        }
                        "account" => {
                            let mut switched = account(NEW);
                            switched["accountKey"] = json!("changed-synthetic-account");
                            engine
                                .dreamina_sessions
                                .seed_for_test(&config, switched)
                                .unwrap();
                        }
                        _ => engine.stop.store(true, Ordering::SeqCst),
                    }
                });
                assert!(refresh_with_wait(
                    &engine,
                    &doc,
                    &config,
                    Duration::from_secs(2),
                    Duration::from_millis(2),
                    || panic!("no dispatch")
                )
                .is_err());
            });
        }
    }

    #[test]
    fn idle_refresh_still_checks_once_and_offline_dispatch_remains_blocked() {
        let (_temp, engine, doc, config) = setup();
        let calls = AtomicUsize::new(0);
        let result = refresh_with_wait(
            &engine,
            &doc,
            &config,
            Duration::ZERO,
            Duration::ZERO,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                assert!(engine
                    .acquire_keys(vec!["channel:dreamina".into()])
                    .is_err());
                Ok(account(NEW))
            },
        )
        .unwrap();
        assert_eq!(result, account(NEW));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(engine
            .dispatch(json!({"action":"dreamina.status","scope":"global"}))
            .is_err());
    }
}
