use super::*;
use crate::providers::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct MockChannels {
    catalogs: AtomicUsize,
    media: AtomicUsize,
    recovery: AtomicBool,
    captured: Mutex<Vec<Value>>,
}
impl MockChannels {
    fn record(&self, action: &str, c: &ProviderConfig) {
        assert!(
            self.recovery.load(Ordering::SeqCst),
            "unexpected media {action}"
        );
        self.captured.lock().unwrap().push(json!({"action":action,"endpoint":c.endpoint,"secret":std::env::var(c.credential_env.as_deref().unwrap()).unwrap()}));
    }
    fn result(task: &str) -> QueryResult {
        QueryResult {
            task_id: task.into(),
            status: "succeeded".into(),
            result_urls: vec!["https://fixture.invalid/result.png".into()],
            error: None,
            actual_cost: None,
            raw: json!({}),
        }
    }
}
impl Channels for MockChannels {
    fn capabilities(&self, c: &ProviderConfig) -> ProviderResult<Capabilities> {
        self.catalogs.fetch_add(1, Ordering::SeqCst);
        Ok(Capabilities {
            provider: c.kind.clone(),
            version: "fixture".into(),
            source: "synthetic".into(),
            checked_at: now(),
            models: vec![],
            can_cancel: false,
            can_reconcile_unknown: false,
            raw: json!({}),
        })
    }
    fn prepare(
        &self,
        _: &ProviderConfig,
        _: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        self.media.fetch_add(1, Ordering::SeqCst);
        panic!("unexpected media prepare")
    }
    fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
        self.media.fetch_add(1, Ordering::SeqCst);
        panic!("unexpected media submit")
    }
    fn query(&self, c: &ProviderConfig, task: &str) -> ProviderResult<QueryResult> {
        self.media.fetch_add(1, Ordering::SeqCst);
        self.record("query", c);
        Ok(Self::result(task))
    }
    fn download(&self, c: &ProviderConfig, _: &str, _: &Path) -> ProviderResult<DownloadReceipt> {
        self.media.fetch_add(1, Ordering::SeqCst);
        self.record("download", c);
        Err(ProviderError::new(
            "SYNTHETIC_DOWNLOAD",
            "local fixture; no media downloaded",
        ))
    }
    fn reconcile(&self, c: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        self.media.fetch_add(1, Ordering::SeqCst);
        self.record("reconcile", c);
        Ok(Self::result("reconciled-original-task"))
    }
}
fn engine(home: PathBuf) -> (Arc<Engine>, Arc<MockChannels>) {
    let channels = Arc::new(MockChannels::default());
    let mut e = Engine::with_channels(home, channels.clone()).unwrap();
    let inner = Arc::get_mut(&mut e).unwrap();
    inner.standard_native = true;
    inner.live_accounts = true;
    inner.readiness_enabled = true;
    (e, channels)
}
fn get(e: &Arc<Engine>) -> Value {
    e.dispatch(json!({"action":"settings.global.get"})).unwrap()
}
fn save(e: &Arc<Engine>, patch: Value) -> Value {
    let doc = get(e);
    e.dispatch(json!({"action":"settings.global.save","settings":patch,"expectedRevision":doc["revision"]})).unwrap()
}

#[test]
fn global_settings_without_project_survive_restart_and_do_not_create_one() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let (e, channels) = engine(home.clone());
    let doc = get(&e);
    assert_eq!(doc["schemaVersion"], "app-settings/v1");
    assert!(!doc["rules"].as_array().unwrap().is_empty());
    save(
        &e,
        json!({"textProvider":"codex_sdk","textModel":"configured","reasoning":"high"}),
    );
    drop(e);
    let (e, _) = engine(home.clone());
    assert_eq!(get(&e)["settings"]["textModel"], "configured");
    assert_eq!(e.registry().unwrap(), json!([]));
    assert_eq!(fs::read_dir(home.join("projects")).unwrap().count(), 0);
    assert_eq!(channels.media.load(Ordering::SeqCst), 0);
    assert_eq!(channels.catalogs.load(Ordering::SeqCst), 0);
}

#[test]
fn global_component_status_does_not_initialize_preferences_or_project() {
    let temp = tempfile::tempdir().unwrap();
    let (e, channels) = engine(temp.path().join("home"));
    for (provider, action) in [("codex_cli", "runtime.status"), ("chrome", "chrome.status")] {
        let result = e
            .dispatch(json!({"action":action,"scope":"global","provider":provider}))
            .unwrap();
        assert!(result["schemaVersion"].is_string());
        assert!(!e.home.join(app_settings::FILE).exists());
        assert_eq!(e.registry().unwrap(), json!([]));
    }
    assert_eq!(channels.media.load(Ordering::SeqCst), 0);
    assert_eq!(channels.catalogs.load(Ordering::SeqCst), 0);
}

#[test]
fn global_chrome_connect_then_status_keeps_cache_and_rotation_invalidates_it() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut count = 0;
        while count < 4 && Instant::now() < deadline {
            let (mut socket, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 2048];
            while !request.ends_with(b"\r\n\r\n") {
                let n = socket.read(&mut chunk).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                assert!(request.len() < 8192);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET "));
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer fixture-chrome-token"));
            let body = if request.starts_with("GET /health ") {
                json!({"ok":true,"pendingRequests":0})
            } else {
                assert!(request.starts_with("GET /browser/clients "));
                json!({"ok":true,"clients":[{"ready":true,"selected":true}]})
            }
            .to_string();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            count += 1;
        }
        count
    });
    let temp = tempfile::tempdir().unwrap();
    let (e, channels) = engine(temp.path().join("home"));
    save(
        &e,
        json!({"chromeEndpoint":endpoint,"chromeToken":"fixture-chrome-token"}),
    );
    let connected = e
        .dispatch(json!({"action":"chrome.connect","scope":"global"}))
        .unwrap();
    assert_eq!(connected["status"], "ready");
    let cached = e
        .dispatch(json!({"action":"chrome.status","scope":"global"}))
        .unwrap();
    assert_eq!(cached["status"], "ready");
    assert_eq!(cached["usableClients"], 1);
    assert!(!cached.to_string().contains("fixture-chrome-token"));
    assert_eq!(server.join().unwrap(), 4);
    save(&e, json!({"chromeToken":"rotated-fixture-token"}));
    assert_eq!(
        e.dispatch(json!({"action":"chrome.status","scope":"global"}))
            .unwrap()["status"],
        "offline"
    );
    assert_eq!(e.registry().unwrap(), json!([]));
    assert_eq!(channels.media.load(Ordering::SeqCst), 0);
}

#[test]
fn global_api_channels_use_credential_identity_without_user_account_label() {
    let temp = tempfile::tempdir().unwrap();
    let (e, channels) = engine(temp.path().join("home"));
    let p = e
        .dispatch(json!({"action":"projects.create","title":"no account label"}))
        .unwrap();
    let store = e.store(s(&p, "id")).unwrap();
    let before = fs::read(store.root.join("project.sqlite")).unwrap();
    let empty = e.workspace(&store, None).unwrap();
    assert_eq!(empty["credentialStatus"]["chrome"]["configured"], false);
    assert_eq!(empty["credentialStatus"]["bh"]["configured"], false);
    save(
        &e,
        json!({"bhEndpoint":"https://fixture.invalid","bhToken":"synthetic-bh","chromeEndpoint":"http://127.0.0.1:9","chromeToken":"synthetic-chrome"}),
    );
    for provider in ["bh", "chrome"] {
        e.require_account(&store, provider).unwrap();
        let (_, account) = e.config(&store, provider).unwrap();
        assert!(account.contains(":credential:"));
        let report=e.queue_images_frozen(&store,&json!({"provider":provider,"model":"fixture","assetIds":[],"imageProductionPolicy":"automatic"}),&e.effective_settings(&store).unwrap(),|_,account|Ok(account.into())).unwrap();
        assert_eq!(report["queued"], json!([]));
    }
    let workspace = e.workspace(&store, None).unwrap();
    for provider in ["bh", "chrome"] {
        assert_eq!(workspace["credentialStatus"][provider], json!({"configured":true}));
        assert!(s(&workspace["settings"], &format!("{provider}AccountId")).is_empty());
    }
    let public = workspace.to_string();
    assert!(!public.contains("synthetic-bh") && !public.contains("synthetic-chrome"));
    assert!(!public.contains("credentialRefs") && !public.contains("credentialHashes"));
    assert_eq!(fs::read(store.root.join("project.sqlite")).unwrap(), before);
    assert_eq!(channels.media.load(Ordering::SeqCst), 0);
}

#[test]
fn global_save_and_catalog_check_preserve_existing_project_database_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let (e, channels) = engine(temp.path().join("home"));
    let p = e
        .dispatch(json!({"action":"projects.create","title":"fixture"}))
        .unwrap();
    let store = e.store(s(&p, "id")).unwrap();
    store
        .save_settings(&json!({"textModel":"old-project","ratio":"9:16"}))
        .unwrap();
    store
        .add_job(
            "image",
            "asset",
            "bh",
            "original-account",
            &json!({"requestId":"original","autoSubmit":false}),
            "submission_unknown",
        )
        .unwrap();
    store
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO units VALUES('unit','episode',1,?1)",
            [
                json!({"id":"unit","episodeId":"episode","version":1,"prompt":"existing result"})
                    .to_string(),
            ],
        )
        .unwrap();
    let before = fs::read(store.root.join("project.sqlite")).unwrap();
    get(&e);
    save(
        &e,
        json!({"bhEndpoint":"https://fixture.invalid","bhAccountId":"new","bhToken":"synthetic","textModel":"new-global"}),
    );
    let cap = e
        .dispatch(json!({"action":"providers.refresh","scope":"global","provider":"bh"}))
        .unwrap();
    assert_eq!(cap["provider"], "bh");
    assert_eq!(fs::read(store.root.join("project.sqlite")).unwrap(), before);
    assert_eq!(channels.media.load(Ordering::SeqCst), 0);
    assert_eq!(channels.catalogs.load(Ordering::SeqCst), 1);
    assert_eq!(get(&e)["providers"][0]["provider"], "bh");
}

#[test]
fn global_native_readiness_runs_with_no_project_and_with_existing_stage3_units() {
    let temp = tempfile::tempdir().unwrap();
    let path = super::super::readiness_engine_tests::native_fixture(temp.path(), "codex");
    let (e, channels) = engine(temp.path().join("home"));
    save(&e, json!({"codexPath":path}));
    for with_project in [false, true] {
        let store = if with_project {
            let p = e
                .dispatch(json!({"action":"projects.create","title":"has completed third stage"}))
                .unwrap();
            let store = e.store(s(&p, "id")).unwrap();
            store
                .conn()
                .unwrap()
                .execute(
                    "INSERT INTO units VALUES('unit','episode',1,?1)",
                    [json!({"id":"unit","prompt":"existing"}).to_string()],
                )
                .unwrap();
            Some(store)
        } else {
            None
        };
        let before = store
            .as_ref()
            .map(|s| fs::read(s.root.join("project.sqlite")).unwrap());
        let result = e
            .dispatch(json!({"action":"readiness.check","scope":"global","provider":"codex_cli"}))
            .unwrap();
        assert_eq!(
            result["authentication"]["state"], "authenticated",
            "{result}"
        );
        assert_eq!(result["generation"]["calls"], 0);
        if let Some(store) = store {
            assert_eq!(
                fs::read(store.root.join("project.sqlite")).unwrap(),
                before.unwrap()
            );
        }
    }
    assert_eq!(channels.media.load(Ordering::SeqCst), 0);
    assert!(!fs::read_to_string(temp.path().join("codex-calls.txt"))
        .unwrap()
        .contains("FORBIDDEN"));
}

#[test]
fn global_dreamina_status_and_catalog_never_resume_project_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let path = super::super::readiness_engine_tests::native_fixture(temp.path(), "dreamina");
    let (e, channels) = engine(temp.path().join("home"));
    save(&e, json!({"dreaminaPath":path}));
    let p = e
        .dispatch(json!({"action":"projects.create","title":"paused media"}))
        .unwrap();
    let store = e.store(s(&p, "id")).unwrap();
    let job = store
        .add_job(
            "image",
            "fixture",
            "dreamina",
            "original-account",
            &json!({"autoSubmit":true}),
            "auth_required",
        )
        .unwrap();
    let before = fs::read(store.root.join("project.sqlite")).unwrap();
    let status = e
        .dispatch(json!({"action":"dreamina.status","scope":"global"}))
        .unwrap();
    assert_eq!(status["ready"], true, "{status}");
    e.dispatch(json!({"action":"providers.refresh","scope":"global","provider":"dreamina"}))
        .unwrap();
    assert_eq!(fs::read(store.root.join("project.sqlite")).unwrap(), before);
    assert_eq!(store.job(&job).unwrap()["status"], "auth_required");
    assert_eq!(channels.media.load(Ordering::SeqCst), 0);
    assert_eq!(channels.catalogs.load(Ordering::SeqCst), 1);
    let calls = fs::read_to_string(temp.path().join("dreamina-calls.txt")).unwrap();
    assert!(!calls.contains("FORBIDDEN"));
    assert!(calls.contains("user_credit"));
}

#[test]
fn global_new_input_freezes_model_but_refinement_hash_and_old_workflow_stay_project_bound() {
    let temp = tempfile::tempdir().unwrap();
    let (e, _) = engine(temp.path().join("home"));
    let p = e
        .dispatch(json!({"action":"projects.create","title":"fixture"}))
        .unwrap();
    let store = e.store(s(&p, "id")).unwrap();
    store
        .save_settings(&json!({"textModel":"old","reasoning":"low","ratio":"9:16"}))
        .unwrap();
    let project = store.settings().unwrap();
    let old = store
        .add_job(
            "workflow",
            "storyboard",
            "codex",
            "local",
            &json!({"input":{"settings":project}}),
            "blocked",
        )
        .unwrap();
    let old_before = store.job(&old).unwrap();
    save(&e, json!({"textModel":"new-model","reasoning":"high"}));
    let mut input = json!({"settings":project});
    let hash =
        freeze_production_execution_settings(&store, "storyboard.refine", &mut input, |settings| {
            app_settings::effective(&e.home, settings)
        })
        .unwrap();
    assert_eq!(input["settings"]["textModel"], "new-model");
    assert_eq!(input["settings"]["reasoning"], "high");
    assert_eq!(input["settings"]["ratio"], "9:16");
    assert_eq!(hash, pipeline::digest(&project.to_string()));
    assert_eq!(store.settings().unwrap(), project);
    assert_eq!(store.job(&old).unwrap(), old_before);
}

#[test]
fn global_catalog_feeds_new_storyboard_and_preflight_without_project_cache_or_generation() {
    let temp = tempfile::tempdir().unwrap();
    let (e, channels) = engine(temp.path().join("home"));
    let p = e
        .dispatch(json!({"action":"projects.create","title":"global catalog"}))
        .unwrap();
    let store = e.store(s(&p, "id")).unwrap();
    let source = store
        .save_source(json!(pipeline::prepare_source(
            "第一集\n场景：门口\n甲：开门。\n"
        )
        .unwrap()))
        .unwrap();
    store
        .save_settings(&json!({"videoProvider":"bh","videoModel":"fixture-model"}))
        .unwrap();
    let old = store
        .add_job(
            "workflow",
            "storyboard.run",
            "codex",
            "local",
            &json!({"input":{"capabilities":{"model":"old-model","version":"old-version"}}}),
            "blocked",
        )
        .unwrap();
    let old_before = store.job(&old).unwrap();
    save(
        &e,
        json!({"bhEndpoint":"https://fixture.invalid","bhToken":"fixture-token"}),
    );
    let (config, account) = e.config(&store, "bh").unwrap();
    let fingerprint = capability_fingerprint(&config, &account);
    let cap = json!({"provider":"bh","version":"global-fixture/v1","source":"synthetic","checkedAt":now(),"canCancel":false,"canReconcileUnknown":false,"raw":{},"models":[{"id":"fixture-model","kind":"video","adapted":true,"minDuration":4,"maxDuration":15,"images":9,"videos":0,"audios":0,"requiredImages":0,"raw":{}}]});
    let doc = app_settings::load(&e.home).unwrap().unwrap();
    app_settings::cache(
        &e.home,
        "bh",
        doc["revision"].as_u64().unwrap(),
        &fingerprint,
        &cap,
    )
    .unwrap();
    let before = fs::read(store.root.join("project.sqlite")).unwrap();
    let planning = e.global_planning_capabilities(&store).unwrap().unwrap();
    let input = production_input_for_new_workflow_with_capabilities(
        &store,
        &json!({"action":"storyboard.run","episodeIds":[source["episodes"][0]["id"]]}),
        Some(&planning),
    )
    .unwrap();
    assert_eq!(input["capabilities"]["model"], "fixture-model");
    assert_eq!(input["capabilities"]["version"], "global-fixture/v1");
    let verified = e.video_channel(&store, "bh", &json!({})).unwrap();
    assert_eq!(verified.report["source"], "cache");
    assert_eq!(channels.catalogs.load(Ordering::SeqCst), 0);
    assert_eq!(channels.media.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(store.root.join("project.sqlite")).unwrap(), before);
    assert_eq!(store.job(&old).unwrap(), old_before);
    assert!(e
        .global_capability_cache("bh", "different-fingerprint")
        .unwrap()
        .is_none());
    let mut expired = cap;
    expired["checkedAt"] = json!("2020-01-01T00:00:00Z");
    app_settings::cache(
        &e.home,
        "bh",
        doc["revision"].as_u64().unwrap(),
        &fingerprint,
        &expired,
    )
    .unwrap();
    assert!(e
        .global_capability_cache("bh", &fingerprint)
        .unwrap()
        .is_none());
}

#[test]
fn media_config_after_global_rotation_uses_original_ref_endpoint_and_account() {
    let temp = tempfile::tempdir().unwrap();
    let (e, _) = engine(temp.path().join("home"));
    let p = e
        .dispatch(json!({"action":"projects.create","title":"fixture"}))
        .unwrap();
    let store = e.store(s(&p, "id")).unwrap();
    store
        .save_settings(&json!({"bhEndpoint":"https://legacy.invalid","bhAccountId":"legacy"}))
        .unwrap();
    crate::platform::write_credential(&format!("{}:bh", s(&p, "id")), "legacy-token").unwrap();
    let legacy = store
        .add_job(
            "image",
            "legacy",
            "bh",
            "legacy",
            &json!({"requestId":"legacy"}),
            "submission_unknown",
        )
        .unwrap();
    save(
        &e,
        json!({"bhEndpoint":"https://first.invalid","bhAccountId":"first","bhToken":"first-token"}),
    );
    let mut frozen = e.effective_settings(&store).unwrap();
    app_settings::freeze_credentials(&mut frozen);
    let (config, account) = e.config_with_settings(&store, "bh", &frozen).unwrap();
    let job = store
        .add_job(
            "image",
            "first",
            "bh",
            &account,
            &json!({"frozenSettings":frozen,"providerConfig":config,"requestId":"frozen"}),
            "submitted",
        )
        .unwrap();
    let before = store.job(&job).unwrap();
    save(
        &e,
        json!({"bhEndpoint":"https://second.invalid","bhAccountId":"second","bhToken":"second-token"}),
    );
    let (current, current_account) = e.config(&store, "bh").unwrap();
    assert_eq!(current.endpoint.as_deref(), Some("https://second.invalid"));
    assert_ne!(current_account, account);
    let (original, original_account) = e.generation_config(&store, &json!({"id":job})).unwrap();
    assert_eq!(original_account, account);
    assert_eq!(original.endpoint.as_deref(), Some("https://first.invalid"));
    assert_eq!(
        std::env::var(original.credential_env.unwrap()).unwrap(),
        "first-token"
    );
    let (legacy_config, _) = e.generation_config(&store, &json!({"id":legacy})).unwrap();
    assert_eq!(
        legacy_config.endpoint.as_deref(),
        Some("https://legacy.invalid")
    );
    assert_eq!(
        std::env::var(legacy_config.credential_env.unwrap()).unwrap(),
        "legacy-token"
    );
    assert_eq!(store.job(&job).unwrap(), before);
}

#[test]
fn public_query_download_and_reconcile_routes_keep_original_global_credential() {
    let temp = tempfile::tempdir().unwrap();
    let (e, channels) = engine(temp.path().join("home"));
    channels.recovery.store(true, Ordering::SeqCst);
    let p = e
        .dispatch(json!({"action":"projects.create","title":"recover"}))
        .unwrap();
    let store = e.store(s(&p, "id")).unwrap();
    save(
        &e,
        json!({"bhEndpoint":"https://original.invalid","bhAccountId":"original","bhToken":"original-token"}),
    );
    let mut frozen = e.effective_settings(&store).unwrap();
    app_settings::freeze_credentials(&mut frozen);
    let (config, account) = e.config_with_settings(&store, "bh", &frozen).unwrap();
    let snapshot = json!({"frozenSettings":frozen,"providerConfig":config,"requestId":"original-request","autoSubmit":false});
    let job = store
        .add_job("image", "asset", "bh", &account, &snapshot, "submitted")
        .unwrap();
    store
        .update_job(
            &job,
            "submitted",
            &json!({"providerTaskId":"original-task"}),
        )
        .unwrap();
    let unknown = store
        .add_job(
            "image",
            "asset-unknown",
            "bh",
            &account,
            &snapshot,
            "submission_unknown",
        )
        .unwrap();
    save(
        &e,
        json!({"bhEndpoint":"https://changed.invalid","bhAccountId":"changed","bhToken":"changed-token"}),
    );
    for (action, key) in [
        ("jobs.query", &job),
        ("jobs.download", &job),
        ("jobs.reconcile", &unknown),
    ] {
        e.dispatch(json!({"action":action,"projectId":p["id"],"jobId":key}))
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !e.active.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            e.active.lock().unwrap().is_empty(),
            "recovery worker deadline"
        );
    }
    let captured = channels.captured.lock().unwrap();
    for action in ["query", "download", "reconcile"] {
        assert!(
            captured.iter().any(|c| c["action"] == action),
            "missing {action}: {captured:?}"
        );
    }
    assert!(captured
        .iter()
        .all(|c| c["secret"] == "original-token" && c["endpoint"] == "https://original.invalid"));
    assert_eq!(store.job(&job).unwrap()["snapshot"], snapshot);
    assert_eq!(store.job(&unknown).unwrap()["snapshot"], snapshot);
    assert_eq!(store.job(&job).unwrap()["providerTaskId"], "original-task");
}
