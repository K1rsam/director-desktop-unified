use super::*;
use crate::providers::*;
use std::sync::atomic::AtomicUsize;

fn imported_prompt_fixture() -> (Fixture, Value, Value, String) {
    let f = Fixture::new(Duration::ZERO);
    let text = "第1集\n1-1\n场景：外来阳台\n片段1\n时长：4秒\n[场景] @[外来阳台SC]\n分镜1｜0-4s｜4s\n画面：外部原文。\n";
    crate::lingmo_import::import(&f.store, &json!({"text":text,"expectedSha256":pipeline::digest(text)})).unwrap();
    let asset = f.store.values("assets").unwrap().into_iter().find(|a| a["lingmoType"] == "SC").unwrap();
    let path = f._temp.path().join("external-scene.png");
    image::RgbImage::from_pixel(3, 2, image::Rgb([40, 80, 120])).save(&path).unwrap();
    let media = f.store.import_media(&path, "image", Some(s(&asset,"id")), Some(s(&asset,"variantId"))).unwrap();
    f.store.adopt(s(&asset,"id"), s(&asset,"variantId"), s(&media,"id")).unwrap();
    let unit = f.store.values("units").unwrap().into_iter().find(|u| u["origin"] == "imported-storyboard").unwrap();
    let snapshot = binding::bind(&f.store, &unit, "offline", "offline-video").unwrap();
    assert_eq!(snapshot["problems"], json!([]));
    let job = f.store.add_job("video",s(&unit,"id"),"offline","offline-account",&snapshot,"ready").unwrap();
    (f,unit,snapshot,job)
}

#[test]
fn imported_prepared_prompt_proof_survives_lazy_summary_without_requiring_stage_one_version() {
    let (f, unit, snapshot, job) = imported_prompt_fixture();
    let before = raw_job_record(&f.store, &job);
    let workspace = f.engine.workspace(&f.store, None).unwrap();
    let summary = workspace["units"].as_array().unwrap().iter().find(|u| u["id"] == unit["id"]).unwrap();
    assert_eq!(summary["prompt"], "");
    assert!(summary["mentions"].is_null());
    let package = &summary["preparedPackage"];
    assert_eq!(package["sourcePrompt"], unit["prompt"]);
    assert_eq!(package["sourcePromptProvenance"]["compiledPromptVerified"], true, "{package}");
    assert_eq!(package["sourcePromptProvenance"]["origin"], "imported-unit-version-and-reference-compilation");
    assert_eq!(package["sourcePromptProvenance"]["importSource"], unit["importSource"]);
    let evidence = ready_prompt_evidence(&f.store.conn().unwrap()).unwrap();
    let (selected_snapshot, selected_unit) = &evidence[&job];
    assert_eq!(selected_snapshot["mentions"], snapshot["mentions"]);
    assert_eq!(selected_unit["mentions"], unit["mentions"]);
    for source_version in [0,999] {
        assert!(prepared_source_prompt_provenance(&job,summary,selected_unit,selected_snapshot,
            workspace["assets"].as_array().unwrap(),source_version,&f.store.settings().unwrap()).is_ok());
    }
    assert_eq!(before, raw_job_record(&f.store, &job));
    assert_eq!(f.channels.calls.load(Ordering::SeqCst),0);
}

#[test]
fn imported_prepared_prompt_proof_rejects_changed_provenance_refs_or_frozen_prompt() {
    let (f, unit, snapshot, job) = imported_prompt_fixture();
    let workspace = f.engine.workspace(&f.store, None).unwrap();
    let summary = workspace["units"].as_array().unwrap().iter().find(|u| u["id"] == unit["id"]).unwrap();
    let assets = workspace["assets"].as_array().unwrap();
    let settings = f.store.settings().unwrap();
    for field in ["origin","importSource","mentions","audioAssetRefs","sourcePrompt","sourcePromptSha256","prompt"] {
        let mut wrong = snapshot.clone();
        wrong[field] = json!("changed");
        assert!(prepared_source_prompt_provenance(&job,summary,&unit,&wrong,assets,0,&settings).is_err(),"{field}");
    }
    for field in ["id","version","assetRefs","importSource","audioAssetRefs","mediaRefs","origin"] {
        let mut wrong = summary.clone();
        wrong[field] = json!("changed");
        assert!(prepared_source_prompt_provenance(&job,&wrong,&unit,&snapshot,assets,0,&settings).is_err(),"{field}");
    }
    let mut changed_assets = assets.clone();
    let reference_id = s(&snapshot["references"][0],"assetId");
    changed_assets.iter_mut().find(|a|s(a,"id")==reference_id).unwrap()["version"] = json!(999);
    assert!(prepared_source_prompt_provenance(&job,summary,&unit,&snapshot,&changed_assets,0,&settings).is_err());
}

struct OfflineChannels {
    calls: AtomicUsize,
    submitting: AtomicBool,
    delay: Duration,
}
impl OfflineChannels {
    fn new(delay: Duration) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            submitting: AtomicBool::new(false),
            delay,
        }
    }
}
impl Channels for OfflineChannels {
    fn capabilities(&self, _: &ProviderConfig) -> ProviderResult<Capabilities> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::new(
            "OFFLINE",
            "No live capacity query is allowed",
        ))
    }
    fn prepare(
        &self,
        _: &ProviderConfig,
        request: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(PreparedRequest {
            provider: "offline".into(),
            request: request.clone(),
            capability_version: "offline/v1".into(),
            upload_receipts: vec![],
            wire: json!({"offline":true}),
            estimated_cost: None,
            prepared_at: now(),
        })
    }
    fn submit(&self, _: &ProviderConfig, _: &PreparedRequest) -> ProviderResult<SubmitReceipt> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.submitting.store(true, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        Ok(SubmitReceipt {
            task_id: "offline-durable-original-id".into(),
            status: "processing".into(),
            result_urls: vec![],
            raw: json!({"fixture":true}),
        })
    }
    fn query(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::new("OFFLINE", "No query is expected"))
    }
    fn download(&self, _: &ProviderConfig, _: &str, _: &Path) -> ProviderResult<DownloadReceipt> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::new("OFFLINE", "No download is expected"))
    }
    fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::new(
            "OFFLINE",
            "No reconciliation is expected",
        ))
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    engine: Arc<Engine>,
    store: Store,
    project_id: String,
    channels: Arc<OfflineChannels>,
}

#[test]
fn image_partial_completion_and_resume_are_limited_to_the_frozen_original_production() {
    assert_eq!(
        image_stage_completion_state(&json!({"blocked":[]})),
        "completed"
    );
    assert_eq!(
        image_stage_completion_state(&json!({"blocked":[{"assetId":"one"}]})),
        "partially_blocked"
    );
    let original = json!({"kind":"workflow","entityId":"images.produce:","status":"completed","snapshot":{"stage":"images.produce","request":{"action":"images.produce"},"settings":{},"sourceVersion":1},"result":{"status":"partiallyQueued","blocked":[{"assetId":"one"}]}});
    assert!(image_production_resume_eligible(&original));
    for (pointer, value) in [
        ("/result/blocked", json!([])),
        ("/result/status", json!("queuedForGeneration")),
        ("/snapshot/stage", json!("storyboard.run")),
        ("/snapshot/request/action", json!("images.run")),
        ("/snapshot/sourceVersion", json!(0)),
        ("/entityId", json!("foundation.run")),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(
            !image_production_resume_eligible(&changed),
            "must not reopen {pointer}"
        );
    }
}

#[test]
fn workspace_job_summary_preserves_explicit_manual_video_submission_without_altering_snapshot() {
    let f = Fixture::new(Duration::ZERO);
    let snapshot = json!({"autoSubmit":false,"model":"offline","unitVersion":2,"prompt":"完整素材包正文","references":[],"ruleSnapshot":{"content":"private full rule"}});
    let key = f
        .store
        .add_job(
            "video",
            "video-manual-unit",
            "offline",
            "local",
            &snapshot,
            "ready",
        )
        .unwrap();
    let original = raw_job_record(&f.store, &key);
    let workspace = f.engine.workspace(&f.store, None).unwrap();
    let summary = workspace["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == key)
        .unwrap();
    assert_eq!(summary["snapshot"]["autoSubmit"], false);
    assert_eq!(summary["snapshot"]["model"], "offline");
    assert!(summary["snapshot"].get("prompt").is_none());
    assert!(summary["snapshot"].get("references").is_none());
    assert!(summary["snapshot"].get("ruleSnapshot").is_none());
    assert_eq!(raw_job_record(&f.store, &key), original);
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn native_resume_of_legacy_completed_but_blocked_image_production_preserves_record_and_never_submits(
) {
    let f = Fixture::new(Duration::ZERO);
    f.store.save_asset(json!({"id":"manual","variantId":"base","baseAssetId":"manual","name":"手写形象","kind":"CH","prompt":"完整手写提示词","promptOrigin":"MANUAL","sourceRefs":[],"applicability":{"episodes":[],"scenes":[]}})).unwrap();
    let saved_settings = f.store.settings().unwrap();
    let request = json!({"action":"images.produce","projectId":f.project_id,"provider":"offline","model":"offline-image","assetIds":["manual"],"imageProductionPolicy":"automatic"});
    let source_version = f.store.source().unwrap()["version"].clone();
    let snapshot = json!({"stage":"images.produce","sourceVersion":source_version,"settings":saved_settings,"request":request,"imageProductionPolicy":"automatic"});
    let old = f
        .store
        .add_job(
            "workflow",
            "images.produce:",
            "local",
            "local",
            &snapshot,
            "completed",
        )
        .unwrap();
    f.store.update_job(&old,"completed",&json!({"result":{"status":"partiallyQueued","blocked":[{"assetId":"manual","reason":"legacy extraction blocked"}],"assets":[],"imagePlan":[]}})).unwrap();
    let before = raw_job_record(&f.store, &old);
    f.engine
        .dispatch(json!({"action":"workflows.resume","projectId":f.project_id,"workflowId":old}))
        .unwrap();
    assert!(f.engine.await_worker_shutdown(3));
    assert_eq!(
        raw_job_record(&f.store, &old),
        before,
        "resuming adds an audit event, never rewrites the original snapshot/receipt/result"
    );
    let jobs = f.store.jobs().unwrap();
    assert_eq!(jobs.iter().filter(|job| job["kind"] == "text").count(), 0);
    let resumed = jobs
        .iter()
        .find(|job| job["kind"] == "workflow" && job["id"] != old)
        .unwrap();
    assert_eq!(resumed["status"], "completed");
    assert!(resumed["snapshot"]["stage2Context"].get(pipeline::asset_design_contract::COMPARISON_PROFILE_FIELD).is_none(),
        "reconstructing missing legacy context must not opt a resume into a new comparison policy");
    assert!(resumed["snapshot"]["stage2Context"].get(pipeline::asset_design_contract::FROZEN_COMPARISON_REQUESTS).is_none());
    assert!(resumed["snapshot"]["stage2Context"].get(pipeline::DESIGN_EVIDENCE_PROFILE_FIELD).is_none(),
        "legacy resume must not gain source material or a new art completion policy");
    assert_eq!(resumed["snapshot"]["settings"], snapshot["settings"]);
    assert_eq!(
        resumed["snapshot"]["request"]["assetIds"],
        snapshot["request"]["assetIds"]
    );
    assert_eq!(
        resumed["snapshot"]["request"]["imageResumeOrigin"]["workflowId"],
        old
    );
    assert_eq!(jobs.iter().filter(|job| job["kind"] == "image").count(), 1);
    assert_eq!(
        f.channels.calls.load(Ordering::SeqCst),
        0,
        "resume queues; no real channel may be invoked"
    );
}

#[test]
fn startup_channel_contention_defers_catalog_without_marking_a_refresh_attempt() {
    let f = Fixture::new(Duration::ZERO);
    f.store
        .save_settings(&json!({"chromeEndpoint":"http://offline.test"}))
        .unwrap();
    let guard = f
        .engine
        .acquire_keys(vec![
            "channel:dreamina".into(),
            "channel:bh".into(),
            "channel:chrome".into(),
        ])
        .unwrap();
    f.engine.refresh_due_catalogs(&f.store).unwrap();
    let count: i64 = f
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM metadata WHERE key='catalog_attempt:chrome'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "a worker that did not acquire the channel must remain due"
    );
    let error = match f.engine.acquire_keys(vec!["channel:dreamina".into()]) {
        Err(error) => error,
        Ok(_) => panic!("same channel cannot execute twice"),
    };
    assert!(error.contains("即梦渠道"));
    assert!(!error.contains("相关任务正在执行"));
    drop(guard);
    f.engine.refresh_due_catalogs(&f.store).unwrap();
    assert!(f.engine.await_worker_shutdown(2));
    let count: i64 = f
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM metadata WHERE key='catalog_attempt:chrome'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "next scheduler tick must make the real attempt");
    assert!(f.channels.calls.load(Ordering::SeqCst) > 0);
    assert!(
        f.store.jobs().unwrap().is_empty(),
        "catalog checks do not create generation jobs"
    );
}

#[test]
fn missing_source_blocks_text_design_but_manual_image_prompt_can_queue_without_text_cli() {
    let temp = tempfile::tempdir().unwrap();
    let channels = Arc::new(OfflineChannels::new(Duration::ZERO));
    let engine = Engine::with_channels(temp.path().join("home"), channels.clone()).unwrap();
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"仅有图片资产"}))
        .unwrap();
    let pid = s(&project, "id");
    let store = engine.store(pid).unwrap();
    store
        .save_settings(&json!({"trialApproved":true,"codexPath":"Z:/absent-codex.exe"}))
        .unwrap();
    store.save_asset(json!({"id":"manual","variantId":"base","name":"手写形象","kind":"character","prompt":"完整的手写图片提示词","sourceRefs":[],"applicability":{"episodes":[],"scenes":[]}})).unwrap();
    let error = engine
        .dispatch(json!({"action":"images.design","projectId":pid}))
        .unwrap_err();
    assert!(error.contains("来源"),"{error}");
    assert!(store.jobs().unwrap().is_empty());
    engine.dispatch(json!({"action":"images.run","projectId":pid,"provider":"offline","model":"offline-image","assetIds":["manual"]})).unwrap();
    assert!(engine.await_worker_shutdown(2));
    let jobs = store.jobs().unwrap();
    assert_eq!(
        jobs.iter().filter(|job| s(job, "kind") == "text").count(),
        0
    );
    assert_eq!(
        jobs.iter().filter(|job| s(job, "kind") == "image").count(),
        1
    );
    assert!(!jobs.iter().any(|job| s(job,"kind")=="workflow"),"image-only action must not create text workflow");
    assert_eq!(
        channels.calls.load(Ordering::SeqCst),
        0,
        "queueing must not submit to the channel"
    );
}

#[test]
fn offline_startup_text_cli_status_never_reads_a_real_account_or_logs_in() {
    let f = Fixture::new(Duration::ZERO);
    let state = f.engine.dispatch(json!({"action":"textCli.status","projectId":f.project_id,"settings":{"codexPath":"Z:/absent-codex.exe"}})).unwrap();
    assert_eq!(state["diagnostic"], true);
    assert_eq!(state["generationRequestsSent"], 0);
    assert!(f
        .engine
        .dispatch(json!({"action":"textCli.login","projectId":f.project_id}))
        .is_err());
    assert!(f.store.jobs().unwrap().is_empty());
}

#[test]
fn new_images_only_ready_does_not_reserve_missing_mothers_or_block_independent_images() {
    let f=Fixture::new(Duration::ZERO);
    for id in ["mother","child","independent"] {
        f.store.save_asset(json!({"id":id,"variantId":"v","name":id,"kind":"character","identityId":"person","workflowProfile":crate::asset_graph::PROFILE,"filename":format!("{id}.png"),"prompt":"完整图片提示词","promptStatus":"ready","imagePrompt":{"text":"完整中文图片提示词","layoutKind":"character"},"referenceRequirements":if id=="child"{json!([{"targetAssetId":"mother","targetVariantId":"v","role":"identity"}])}else{json!([])}})).unwrap();
    }
    let request=json!({"provider":"offline","model":"image","workflowProfile":crate::asset_graph::PROFILE,"assetIds":["child","independent"],"onlyReady":true,"imageProductionPolicy":"automatic"});
    let report=f.engine.queue_images(&f.store,&request).unwrap();
    assert_eq!(report["queuedCount"],1);assert_eq!(report["blockedCount"],1);
    assert_eq!(report["queued"][0]["assetId"],"independent");
    let jobs=f.store.jobs().unwrap();assert_eq!(jobs.len(),1);assert_eq!(jobs[0]["snapshot"]["ratio"],"1:1");
    let mut child_only=request.clone();child_only["assetIds"]=json!(["child"]);
    let blocked=f.engine.queue_images(&f.store,&child_only);
    // Report-only waiting never creates a hidden dependency/ancestor job.
    assert!(blocked.is_ok());assert_eq!(f.store.jobs().unwrap().len(),1);
    assert_eq!(f.channels.calls.load(Ordering::SeqCst),0);
}
#[test]
fn public_known_task_query_keeps_cli_setup_failure_distinct_from_auth_and_never_submits() {
    let mut f = Fixture::new(Duration::ZERO);
    Arc::get_mut(&mut f.engine).unwrap().live_accounts = true;
    let absent = f._temp.path().join("deliberately-absent-official-cli.exe");
    f.store
        .save_settings(&json!({"dreaminaPath":absent}))
        .unwrap();
    let key = f
        .store
        .add_job(
            "image",
            "asset",
            "dreamina",
            "original-account",
            &json!({"requestId":"existing-known","prompt":"frozen"}),
            "processing",
        )
        .unwrap();
    f.store.update_job(&key,"processing",&json!({"providerTaskId":"original-cloud-task","receipt":{"taskId":"original-cloud-task"},"result":{"original":true}})).unwrap();
    let before = f.store.job(&key).unwrap();
    f.engine
        .dispatch(json!({"action":"jobs.query","projectId":f.project_id,"jobId":key}))
        .unwrap();
    assert!(f.engine.await_worker_shutdown(2));
    let after = f.store.job(&key).unwrap();
    assert_eq!(after["status"], "query_failed");
    assert_eq!(after["queryAttempts"], 1);
    for field in [
        "snapshot",
        "receipt",
        "result",
        "accountId",
        "providerTaskId",
    ] {
        assert_eq!(after[field], before[field]);
    }
    assert_eq!(f.store.jobs().unwrap().len(), 1);
    assert_eq!(
        f.channels.calls.load(Ordering::SeqCst),
        0,
        "the failed identity check cannot query or submit a model"
    );
    assert!(!f.channels.submitting.load(Ordering::SeqCst));
    let view = f
        .engine
        .dreamina_sessions
        .snapshot(&f.engine.dreamina_config(&f.store).unwrap())
        .unwrap();
    assert_eq!(view["status"], "error");
    assert_eq!(view["errorCode"], "CLI_NOT_FOUND");
}
impl Fixture {
    fn new(delay: Duration) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let channels = Arc::new(OfflineChannels::new(delay));
        let engine = Engine::with_channels(temp.path().join("home"), channels.clone()).unwrap();
        let project = engine
            .dispatch(json!({"action":"projects.create","title":"offline repair acceptance"}))
            .unwrap();
        let project_id = s(&project, "id").to_owned();
        let store = engine.store(&project_id).unwrap();
        store.save_settings(&json!({"trialApproved":true,"textProvider":"api","textEndpoint":"","textModel":"offline-text","videoModel":""})).unwrap();
        let source = pipeline::prepare_source(
            "第1集\n场景：门口\n甲：走吧。\n第2集\n场景：院子\n甲：回来了。\n",
        )
        .unwrap();
        store
            .save_source(serde_json::to_value(source).unwrap())
            .unwrap();
        Self {
            _temp: temp,
            engine,
            store,
            project_id,
            channels,
        }
    }
    fn asset(&self, id: &str) {
        self.store.save_asset(json!({"id":id,"variantId":"base","baseAssetId":id,"name":id,"kind":"character","prompt":"offline complete asset design","sourceRefs":[],"applicability":{"episodes":[],"scenes":[]}})).unwrap();
    }
    fn image_action(&self, ids: Value) -> Value {
        json!({"action":"images.run","projectId":self.project_id,"provider":"offline","model":"offline-image","assetIds":ids,"trial":true})
    }
}

fn raw_job_record(store: &Store, key: &str) -> Vec<rusqlite::types::Value> {
    store
        .conn()
        .unwrap()
        .query_row("SELECT * FROM jobs WHERE id=?1", [key], |row| {
            (0..row.as_ref().column_count())
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
}

fn prepared_prompt_fixture(legacy_snapshot: bool) -> (Fixture, Value, Value, String) {
    let f = Fixture::new(Duration::ZERO);
    let source = f.store.source().unwrap();
    let episode = source["episodes"][0]["id"].clone();
    let scene = source["scenes"][0]["id"].clone();
    let row = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["episodeId"] == episode && r["kind"] == "body")
        .unwrap()["id"]
        .clone();
    f.store.save_asset(json!({"id":"hero-base","variantId":"hero:base","baseAssetId":"hero","name":"甲基础形象","kind":"CH","prompt":"基础参考图","sourceRefs":[row],"scope":{"kind":"GLOBAL","scenes":[]},"preparation":{"mode":"REFERENCE"}})).unwrap();
    f.store.save_asset(json!({"id":"hero-injured","variantId":"hero:injured","baseAssetId":"hero","name":"甲受伤形象","kind":"CH","description":"手臂擦伤","sourceVariantId":"hero:base","sourceAssetId":"hero-base","sourceRefs":[row],"scope":{"kind":"SCENES","scenes":[{"episodeId":episode,"sceneId":scene}]},"preparation":{"mode":"INLINE","fromKey":"hero:base"}})).unwrap();
    f.store.save_asset(json!({"id":"score-text","variantId":"score:base","name":"积分器界面","kind":"PROP","description":"原文积分显示","sourceRefs":[row],"scope":{"kind":"SCENES","scenes":[{"episodeId":episode,"sceneId":scene}]},"preparation":{"mode":"INLINE","note":"直接显示源文积分，不另生图片"}})).unwrap();
    let path = f._temp.path().join("parent-reference.png");
    image::RgbImage::from_pixel(3, 2, image::Rgb([20, 40, 80]))
        .save(&path)
        .unwrap();
    let media = f
        .store
        .import_media(&path, "image", Some("hero-base"), Some("hero:base"))
        .unwrap();
    f.store
        .adopt("hero-base", "hero:base", s(&media, "id"))
        .unwrap();
    let unit=f.store.save_unit(json!({"id":"human-prompt-unit","episodeId":episode,"sceneId":scene,"sourceVersion":source["version"],"sourceRefs":[row],"duration":5,"storyboardCreativeProfile":"original-production-contract/v1","prompt":"甲受伤后站在门口，积分器保持原文显示。\n原话：甲（现场）：“走吧。”","assetRefs":[{"assetId":"hero-injured","variantId":"hero:injured","use":"character","order":0},{"assetId":"hero-base","variantId":"hero:base","use":"character","order":1},{"assetId":"score-text","variantId":"score:base","use":"prop","order":2}]})).unwrap();
    let mut snapshot = binding::bind(&f.store, &unit, "offline", "offline-video").unwrap();
    assert_eq!(snapshot["problems"], json!([]));
    assert_eq!(snapshot["references"].as_array().unwrap().len(), 1);
    assert_eq!(
        snapshot["references"][0]["semanticBindings"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(snapshot["inlineReferences"].as_array().unwrap().len(), 1);
    if legacy_snapshot {
        snapshot.as_object_mut().unwrap().remove("sourcePrompt");
        snapshot
            .as_object_mut()
            .unwrap()
            .remove("sourcePromptSha256");
    }
    let job = f
        .store
        .add_job(
            "video",
            s(&unit, "id"),
            "offline",
            "offline-account",
            &snapshot,
            "ready",
        )
        .unwrap();
    (f, unit, snapshot, job)
}

fn prompt_package(f: &Fixture) -> Value {
    let workspace = f.engine.workspace(&f.store, None).unwrap();
    workspace["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == "human-prompt-unit")
        .unwrap()["preparedPackage"]
        .clone()
}

#[test]
fn prepared_prompt_provenance_proves_old_and_new_human_inline_donor_packages_without_media_reads() {
    for legacy in [false, true] {
        let (f, unit, snapshot, job) = prepared_prompt_fixture(legacy);
        binding::validate_snapshot(&f.store, &snapshot).unwrap();
        let before = raw_job_record(&f.store, &job);
        let package = prompt_package(&f);
        assert_eq!(package["sourcePrompt"], unit["prompt"]);
        let proof = &package["sourcePromptProvenance"];
        assert_eq!(proof["version"], "unit-source-prompt/v1");
        assert_eq!(proof["origin"], "unit-version-and-reference-compilation");
        assert_eq!(proof["jobId"], job);
        assert_eq!(proof["unitId"], unit["id"]);
        assert_eq!(proof["unitVersion"], unit["version"]);
        assert_eq!(proof["sourceVersion"], unit["sourceVersion"]);
        assert_eq!(
            proof["sourcePromptSha256"],
            pipeline::digest(s(&unit, "prompt"))
        );
        assert_eq!(proof["compiledPromptVerified"], true);
        assert_eq!(proof["compiledPrompt"], snapshot["prompt"]);
        assert_eq!(package["references"][0]["assetId"], "hero-base");
        assert!(s(&package, "prompt").contains("@图片1：甲受伤形象"));
        assert!(s(&package, "prompt")
            .contains("【文字素材：积分器界面；原文积分显示；直接显示源文积分，不另生图片】"));
        assert!(!s(&package, "prompt").contains("@图片2"));
        // Listing is deliberately metadata-only. Submission still detects a
        // missing original image through the normal authoritative validator.
        let original = Path::new(s(&snapshot["references"][0], "path"));
        fs::rename(original, f._temp.path().join("temporarily-unavailable.png")).unwrap();
        assert_eq!(
            prompt_package(&f)["sourcePromptProvenance"]["compiledPromptVerified"],
            true
        );
        assert!(binding::validate_snapshot(&f.store, &snapshot).is_err());
        assert_eq!(
            raw_job_record(&f.store, &job),
            before,
            "a read must never rewrite a frozen task"
        );
        assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn prepared_prompt_provenance_refuses_text_identity_history_and_carrier_drift() {
    for case in [
        "current-prompt",
        "current-version",
        "current-reference",
        "history-missing",
        "inline-version",
        "donor",
        "compiled-prompt",
        "source-prompt",
        "source-hash",
        "ratio",
        "resolution",
        "duration",
    ] {
        let (f, mut unit, mut snapshot, job) = prepared_prompt_fixture(false);
        let connection = f.store.conn().unwrap();
        match case {
            "current-prompt" => unit["prompt"] = json!("用户手动修改的完整新提示词"),
            "current-version" => unit["version"] = json!(2),
            "current-reference" => unit["assetRefs"][0]["variantId"] = json!("wrong-variant"),
            "history-missing" => {
                connection
                    .execute(
                        "DELETE FROM unit_versions WHERE unit_id=?1",
                        [s(&unit, "id")],
                    )
                    .unwrap();
            }
            "inline-version" => {
                connection.execute("UPDATE assets SET data=json_set(data,'$.version',99) WHERE id='score-text'",[]).unwrap();
            }
            "donor" => snapshot["references"][0]["assetId"] = json!("hero-injured"),
            "compiled-prompt" => snapshot["prompt"] = json!("其它编译文本"),
            "source-prompt" => snapshot["sourcePrompt"] = json!("伪造原文"),
            "source-hash" => snapshot["sourcePromptSha256"] = json!("0".repeat(64)),
            "ratio" => {
                f.store.save_settings(&json!({"ratio":"9:16"})).unwrap();
            }
            "resolution" => {
                f.store
                    .save_settings(&json!({"videoResolution":"1080p"}))
                    .unwrap();
            }
            "duration" => unit["duration"] = json!(6),
            _ => unreachable!(),
        }
        if [
            "current-prompt",
            "current-version",
            "current-reference",
            "duration",
        ]
        .contains(&case)
        {
            connection
                .execute(
                    "UPDATE units SET data=?2 WHERE id=?1",
                    rusqlite::params![s(&unit, "id"), unit.to_string()],
                )
                .unwrap();
        }
        if ["donor", "compiled-prompt", "source-prompt", "source-hash"].contains(&case) {
            connection
                .execute(
                    "UPDATE jobs SET snapshot=?2 WHERE id=?1",
                    rusqlite::params![job, snapshot.to_string()],
                )
                .unwrap();
        }
        let before = raw_job_record(&f.store, &job);
        let package = prompt_package(&f);
        assert_eq!(
            package["sourcePromptProvenance"]["compiledPromptVerified"], false,
            "{case}"
        );
        assert!(
            package["sourcePrompt"].is_null(),
            "no current prompt may masquerade as frozen evidence: {case}"
        );
        assert_eq!(raw_job_record(&f.store, &job), before, "{case}");
        assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn new_bound_source_prompt_is_frozen_and_submit_validation_refuses_forged_or_edited_text() {
    let (f, mut unit, snapshot, _) = prepared_prompt_fixture(false);
    assert_eq!(snapshot["sourcePrompt"], unit["prompt"]);
    assert_eq!(
        snapshot["sourcePromptSha256"],
        pipeline::digest(s(&unit, "prompt"))
    );
    for field in ["sourcePrompt", "sourcePromptSha256", "prompt"] {
        let mut changed = snapshot.clone();
        changed[field] = json!("changed");
        assert!(
            binding::validate_snapshot(&f.store, &changed).is_err(),
            "{field}"
        );
    }
    unit["prompt"] = json!("同版本也不能借用旧提示词证明");
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE units SET data=?2 WHERE id=?1",
            rusqlite::params![s(&unit, "id"), unit.to_string()],
        )
        .unwrap();
    assert!(binding::validate_snapshot(&f.store, &snapshot).is_err());
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
}

struct RevisionOriginFixture {
    fixture: Fixture,
    run: String,
    plan_job: String,
    author_job: String,
    finish_job: String,
    episode: String,
    plan: Value,
    candidate: Value,
    draft: Value,
}
fn seed_revision_accepted_node(
    store: &Store,
    run: &str,
    stage: &str,
    episode: &str,
    mut input: Value,
    result: &Value,
) -> String {
    input["episodeId"] = json!(episode);
    let skill = match stage {
        "storyboardPlan" => "episode-emotion-planner",
        "storyboardCreate" => "unit-performance-director",
        _ => "episode-finishing-editor",
    };
    let snapshot = json!({"stage":stage,"skill":skill,"model":"offline","reasoning":"medium","input":input,"schema":{"type":"object"},"ruleSnapshot":{"fixture":"accepted provenance only"}});
    let hash = pipeline::digest(&snapshot.to_string());
    let job = store
        .add_job("text", stage, "codex", "local", &snapshot, "completed")
        .unwrap();
    store.update_job(&job, "completed", &json!({"result":result,"receipt":{"validation":"accepted","contract":"desktop-runner/v1"}})).unwrap();
    store
        .checkpoint(
            run,
            &format!("{stage}:{episode}:{}", id()),
            stage,
            &hash,
            "accepted",
            Some(&job),
            Some(result),
            None,
        )
        .unwrap();
    job
}
fn revision_origin_fixture() -> RevisionOriginFixture {
    let f = Fixture::new(Duration::ZERO);
    f.store.ensure_workflow_storage().unwrap();
    let source = f.store.source().unwrap();
    let episode = s(&source["episodes"][0], "id").to_owned();
    let rows: Vec<_> = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["episodeId"] == episode && row["kind"] == "body")
        .cloned()
        .collect();
    let refs: Vec<_> = rows.iter().map(|row| row["id"].clone()).collect();
    let candidate = json!({"id":"fantasy:actual","sourceRefs":refs,"emotionFunction":"把行动决定具象化","entry":"主创已选进入","exit":"主创已选退出"});
    let plan = json!({"schemaVersion":"storyboard-plan/v2","emotionArc":"采用的真实旧情绪合同","sourceFingerprints":rows.iter().map(|row|json!({"lineId":row["id"],"sha256":pipeline::digest(s(row,"text"))})).collect::<Vec<_>>(),"events":[{"id":"event:original","sourceRefs":refs,"emotionContract":{"subject":"甲","change":"决定行动","landing":"原落点","retain":["旧合同"]}}],"workPackages":[{"id":"package:original","eventId":"event:original","sourceRefs":refs,"scenarioKeys":["old-scenario"]}],"fantasyCandidates":[candidate]});
    let units:Vec<_> = (0..2).map(|n|json!({"id":format!("adopted:{n}"),"episodeId":episode,"sourceRefs":refs,"eventId":"event:original","prompt":format!("formal prompt {n}"),"intent":{"subject":"甲","change":"决定行动","landing":"原落点","retain":["旧合同"]},"assetRefs":[],"shots":[{"id":format!("shot:{n}"),"duration":5.0}]})).collect();
    let realized = json!([{"candidateId":"fantasy:actual","unitIds":["adopted:0","adopted:1"],"shotIds":["shot:0","shot:1"],"screenSeconds":1.0,"entry":"原进入","exit":"原退出","preservedFunction":"原情绪落点"}]);
    let draft =
        json!({"schemaVersion":"storyboard-draft/v2","units":units,"fantasyRealization":realized});
    let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":{"source":source,"selection":{"episodeIds":[episode]}}});
    let run = f
        .store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &snapshot,
            "completed",
        )
        .unwrap();
    let plan_job = seed_revision_accepted_node(
        &f.store,
        &run,
        "storyboardPlan",
        &episode,
        json!({"episode":{"id":episode},"sourceRows":rows}),
        &plan,
    );
    let author_job = seed_revision_accepted_node(
        &f.store,
        &run,
        "storyboardCreate",
        &episode,
        json!({"selectedFantasy":[candidate]}),
        &draft,
    );
    let finish_job = seed_revision_accepted_node(
        &f.store,
        &run,
        "storyboardFinish",
        &episode,
        json!({"selectedFantasy":[candidate],"previousDraft":draft}),
        &draft,
    );
    let output = json!({"schemaVersion":"storyboard-delivery/v1","sourceSha256":source["sha256"],"scope":"episodes","updatedEpisodeIds":[episode],"units":units,"plans":[{"episodeId":episode,"plan":plan}],"fantasyRealization":realized});
    f.store
        .update_job(&run, "completed", &json!({"result":output}))
        .unwrap();
    for unit in &units {
        let mut published = unit.clone();
        published["continuityStale"] = json!(false);
        published["foundationVersion"] = json!(0);
        published["planningProfile"] = json!({"fixture":"adopted planning metadata"});
        published["retired"] = json!(false);
        published["sourceVersion"] = source["version"].clone();
        f.store.save_unit(published).unwrap();
    }
    RevisionOriginFixture {
        fixture: f,
        run,
        plan_job,
        author_job,
        finish_job,
        episode,
        plan,
        candidate,
        draft,
    }
}
fn revision_request(origin: &RevisionOriginFixture) -> Value {
    json!({"action":"storyboard.run","episodeIds":[origin.episode],"unitIds":["adopted:0"],"revisionInstruction":"沿原情绪落点检查本范围表演"})
}
#[test]
fn local_revision_freezes_unique_adopted_plan_and_actual_fantasy_chain_without_changing_whole_runs()
{
    let origin = revision_origin_fixture();
    let store = &origin.fixture.store;
    let before = raw_job_record(store, &origin.run);
    let jobs = store.jobs().unwrap().len();
    let whole = production_input(
        store,
        &json!({"action":"storyboard.run","episodeIds":[origin.episode]}),
    )
    .unwrap();
    assert!(whole.get("storyboardRevisionProfile").is_none());
    assert!(whole.get("revisionContexts").is_none());
    let revision = production_input(store, &revision_request(&origin)).unwrap();
    assert_eq!(
        revision["storyboardRevisionProfile"],
        STORYBOARD_REVISION_PROFILE
    );
    let context = &revision["revisionContexts"][0];
    assert_eq!(context["version"], STORYBOARD_REVISION_CONTEXT);
    assert_eq!(context["originWorkflowId"], origin.run);
    assert_eq!(context["planJobId"], origin.plan_job);
    assert_eq!(
        context["plan"], origin.plan,
        "no reconstructed emotion contract or latest name lookup"
    );
    assert_eq!(context["selectedFantasy"], json!([origin.candidate]));
    assert_eq!(
        context["fantasyRealization"],
        origin.draft["fantasyRealization"]
    );
    assert_eq!(
        context["unitBindings"].as_array().unwrap().len(),
        2,
        "the whole actual fantasy/shared-source group can be revalidated"
    );
    for binding in context["unitBindings"].as_array().unwrap() {
        let unit = store.get("units", s(binding, "unitId")).unwrap();
        assert_eq!(binding["unitVersion"], unit["version"]);
        assert_eq!(binding["unitSha256"], pipeline::digest(&unit.to_string()));
    }
    let expected = revision.clone();
    assert_eq!(
        production_input(store, &revision_request(&origin)).unwrap(),
        expected,
        "provenance freezing must be deterministic"
    );
    assert_eq!(raw_job_record(store, &origin.run), before);
    assert_eq!(store.jobs().unwrap().len(), jobs);
    assert_eq!(origin.fixture.channels.calls.load(Ordering::SeqCst), 0);
    // The creative transport carries the exact approved card, and removes only
    // its program exports from the consumed Author draft. Formal adoption must
    // still retain its full prompt when matched against the current unit.
    let card = json!({"id":origin.candidate["id"],"sourceRefs":origin.candidate["sourceRefs"]});
    for job in [&origin.author_job, &origin.finish_job] {
        let mut snapshot = store.job(job).unwrap()["snapshot"].clone();
        snapshot["input"]["selectedFantasy"] = json!([card]);
        if job == &origin.finish_job {
            snapshot["input"]["storyboardCreativeProfile"] =
                json!("original-production-contract/v1");
            for unit in snapshot["input"]["previousDraft"]["units"]
                .as_array_mut()
                .unwrap()
            {
                *unit = consumed_author_unit(unit, true);
            }
        }
        let hash = pipeline::digest(&snapshot.to_string());
        let connection = store.conn().unwrap();
        connection
            .execute(
                "UPDATE jobs SET snapshot=?1 WHERE id=?2",
                rusqlite::params![snapshot.to_string(), job],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",
                rusqlite::params![hash, job],
            )
            .unwrap();
    }
    let card_revision = production_input(store, &revision_request(&origin)).unwrap();
    assert_eq!(card_revision["revisionContexts"][0]["plan"], origin.plan);
    assert_eq!(
        card_revision["revisionContexts"][0]["selectedFantasy"],
        json!([card])
    );
    assert!(!selected_revision_candidate_matches(
        &origin.candidate,
        &json!({"id":origin.candidate["id"],"sourceRefs":[],"suggestedProposal":"invented"})
    ));
}
#[test]
fn local_revision_refuses_missing_ambiguous_changed_or_forged_adoption_before_creating_work() {
    for mutation in [
        "prompt",
        "intent",
        "extra-field",
        "source",
        "ambiguous",
        "planner-state",
        "plan-hash",
        "author-state",
        "finish-state",
        "candidate",
        "realization",
        "published-scope",
    ] {
        let origin = revision_origin_fixture();
        let store = &origin.fixture.store;
        let connection = store.conn().unwrap();
        match mutation {
            "prompt" | "intent" | "extra-field" => {
                let mut unit = store.get("units", "adopted:0").unwrap();
                let field = match mutation {
                    "extra-field" => "unapprovedMetadata",
                    value => value,
                };
                unit[field] = json!("not a program-only addition");
                store.save_unit(unit).unwrap();
            }
            "source" => {
                connection.execute("UPDATE jobs SET result=json_set(result,'$.sourceSha256','other') WHERE id=?1",[&origin.run]).unwrap();
            }
            "ambiguous" => {
                let original = store.job(&origin.run).unwrap();
                let duplicate = store
                    .add_job(
                        "workflow",
                        "storyboard.run",
                        "local",
                        "local",
                        &original["snapshot"],
                        "completed",
                    )
                    .unwrap();
                store
                    .update_job(
                        &duplicate,
                        "completed",
                        &json!({"result":original["result"]}),
                    )
                    .unwrap();
            }
            "planner-state" => {
                connection
                    .execute(
                        "UPDATE jobs SET state='rejected' WHERE id=?1",
                        [&origin.plan_job],
                    )
                    .unwrap();
            }
            "plan-hash" => {
                connection
                    .execute(
                        "UPDATE workflow_steps SET request_hash='wrong' WHERE text_job_id=?1",
                        [&origin.plan_job],
                    )
                    .unwrap();
            }
            "author-state" => {
                connection
                    .execute(
                        "UPDATE jobs SET state='rejected' WHERE id=?1",
                        [&origin.author_job],
                    )
                    .unwrap();
            }
            "finish-state" => {
                connection
                    .execute(
                        "UPDATE workflow_steps SET state='rejected' WHERE text_job_id=?1",
                        [&origin.finish_job],
                    )
                    .unwrap();
            }
            "candidate" => {
                let mut snapshot = store.job(&origin.finish_job).unwrap()["snapshot"].clone();
                snapshot["input"]["selectedFantasy"][0]["entry"] =
                    json!("not selected by the original Planner");
                let hash = pipeline::digest(&snapshot.to_string());
                connection
                    .execute(
                        "UPDATE jobs SET snapshot=?1 WHERE id=?2",
                        rusqlite::params![snapshot.to_string(), origin.finish_job],
                    )
                    .unwrap();
                connection
                    .execute(
                        "UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",
                        rusqlite::params![hash, origin.finish_job],
                    )
                    .unwrap();
            }
            "realization" => {
                connection.execute("UPDATE jobs SET result=json_set(result,'$.fantasyRealization[0].unitIds[1]','outside') WHERE id=?1",[&origin.run]).unwrap();
            }
            "published-scope" => {
                connection.execute("UPDATE jobs SET result=json_set(result,'$.updatedEpisodeIds',json('[]')) WHERE id=?1",[&origin.run]).unwrap();
            }
            _ => unreachable!(),
        }
        drop(connection);
        let before = store.jobs().unwrap().len();
        let records = raw_job_record(store, &origin.run);
        let error = production_input(store, &revision_request(&origin)).unwrap_err();
        assert!(
            error.contains("请按当前原文生成整集") && error.contains("未调用模型"),
            "{mutation}: {error}"
        );
        assert_eq!(store.jobs().unwrap().len(), before, "{mutation}");
        assert_eq!(raw_job_record(store, &origin.run), records, "{mutation}");
        assert_eq!(
            origin.fixture.channels.calls.load(Ordering::SeqCst),
            0,
            "{mutation}"
        );
    }
}
#[test]
fn a_revision_adoption_inherits_the_exact_original_planner_and_rebinds_current_unit_hashes() {
    let origin = revision_origin_fixture();
    let store = &origin.fixture.store;
    let input = production_input(store, &revision_request(&origin)).unwrap();
    let snapshot =
        json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":input});
    let revision = store
        .add_job(
            "workflow",
            "storyboard.run",
            "local",
            "local",
            &snapshot,
            "completed",
        )
        .unwrap();
    let mut draft = origin.draft.clone();
    for unit in draft["units"].as_array_mut().unwrap() {
        unit["prompt"] = json!(format!("actually revised {}", s(unit, "id")));
    }
    seed_revision_accepted_node(
        store,
        &revision,
        "storyboardCreate",
        &origin.episode,
        json!({"selectedFantasy":[origin.candidate]}),
        &draft,
    );
    seed_revision_accepted_node(
        store,
        &revision,
        "storyboardFinish",
        &origin.episode,
        json!({"selectedFantasy":[origin.candidate],"draft":draft}),
        &draft,
    );
    let output = json!({"schemaVersion":"storyboard-delivery/v1","scope":"units","sourceSha256":input["source"]["sha256"],"updatedEpisodeIds":[origin.episode],"units":draft["units"],"plans":[],"fantasyRealization":draft["fantasyRealization"]});
    store
        .update_job(&revision, "completed", &json!({"result":output}))
        .unwrap();
    for unit in draft["units"].as_array().unwrap() {
        store.save_unit(unit.clone()).unwrap();
    }
    let current = production_input(store, &revision_request(&origin)).unwrap();
    let context = &current["revisionContexts"][0];
    assert_eq!(
        context["originWorkflowId"], origin.run,
        "an accepted revision cannot invent a new Planner"
    );
    assert_eq!(context["planJobId"], origin.plan_job);
    assert_eq!(context["plan"], origin.plan);
    for binding in context["unitBindings"].as_array().unwrap() {
        let unit = store.get("units", s(binding, "unitId")).unwrap();
        assert_eq!(binding["unitVersion"], 2);
        assert_eq!(binding["unitSha256"], pipeline::digest(&unit.to_string()));
    }
    let before = store.jobs().unwrap().len();
    store.conn().unwrap().execute("UPDATE jobs SET snapshot=json_set(snapshot,'$.input.revisionContexts[0].planJobId','invented') WHERE id=?1",[&revision]).unwrap();
    assert!(production_input(store, &revision_request(&origin))
        .unwrap_err()
        .contains("继承的Planner不是原流程精确已采用节点"));
    assert_eq!(store.jobs().unwrap().len(), before);
    assert_eq!(origin.fixture.channels.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn zero_unit_storyboard_selects_source_episodes_and_versioned_planning_profile() {
    let f = Fixture::new(Duration::ZERO);
    // Current production requires an explicitly adopted visual baseline. This
    // scope test must satisfy that prerequisite rather than test an old gate.
    let coverage=json!({"form":"自然三维国漫造型","rendering":"连续柔和明暗","materials":"布料与金属分明","colorLighting":"统一冷暖且暗部可读","aspectRatio":"画幅16:9","localExceptions":"局部Q版需要回接"});
    let visual=coverage.as_object().unwrap().values().map(|v|v.as_str().unwrap()).collect::<Vec<_>>().join("；");
    f.store.save_settings(&json!({"ratio":"16:9","visualStyle":visual,"projectVisualStyle":visual,"visualBaseline":{"version":"series-visual-baseline-v1","id":"synthetic-scope-baseline","revision":1,"status":"adopted","visualStyle":visual,"coverage":coverage,"adoptionBasis":"synthetic explicit adoption"}})).unwrap();
    assert!(f.store.values("units").unwrap().is_empty());
    let source = f.store.source().unwrap();
    let first = source["episodes"][0]["id"].clone();
    let second = source["episodes"][1]["id"].clone();
    let input = storyboard_input(&f.store, &json!({"episodeIds":[second,first]})).unwrap();
    assert_eq!(input["selection"]["episodeIds"], json!([first, second]));
    assert_eq!(input["capabilities"]["planningOnly"], true);
    assert_eq!(
        input["capabilities"]["version"],
        "storyboard-planning/seedance2.5/v1"
    );
    assert_eq!(input["capabilities"]["minDuration"], 4);
    assert_eq!(input["capabilities"]["maxDuration"], 30);
    assert_eq!(input["capabilities"]["imageLimit"], 30);
    assert_eq!(input["capabilities"]["videoLimit"], 10);
    assert_eq!(input["capabilities"]["audioLimit"], 10);
    assert!(input["previousBoundary"].is_null());
    for selection in [
        json!([]),
        json!(["outside-source"]),
        json!([first, first]),
        json!([5]),
    ] {
        assert!(f
            .engine
            .dispatch(
                json!({"action":"storyboard.run","projectId":f.project_id,"episodeIds":selection})
            )
            .is_err());
    }
    assert!(
        f.store.jobs().unwrap().is_empty(),
        "invalid selection must not create a workflow"
    );
    f.engine
        .dispatch(json!({"action":"storyboard.run","projectId":f.project_id,"episodeIds":[second]}))
        .unwrap();
    assert!(f.engine.await_worker_shutdown(2));
    let jobs = f.store.jobs().unwrap();
    let workflow = jobs
        .iter()
        .find(|job| s(job, "kind") == "workflow")
        .unwrap();
    assert_eq!(
        workflow["snapshot"]["selection"]["episodeIds"],
        json!([second])
    );
    assert_eq!(workflow["snapshot"]["sourceVersion"], source["version"]);
    assert_eq!(
        workflow["snapshot"]["planningProfile"]["planningOnly"],
        true
    );
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
    assert!(
        !s(workflow, "error").contains("视频"),
        "the offline text failure must not be caused by absent video configuration"
    );
}

#[test]
fn explicit_video_model_requires_real_cached_adaptation_and_prior_boundary_is_ordered() {
    let f = Fixture::new(Duration::ZERO);
    f.store
        .save_settings(&json!({"videoModel":"selected-video","videoProvider":"offline"}))
        .unwrap();
    assert!(self_capabilities(&f.store).is_err());
    let caps = json!({"version":"cached/v2","models":[{"id":"selected-video","kind":"video","adapted":false,"minDuration":4,"maxDuration":15,"images":9,"videos":3,"audios":3}]});
    f.store
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO catalogs VALUES('offline',?1,?2)",
            rusqlite::params![caps.to_string(), now()],
        )
        .unwrap();
    assert!(self_capabilities(&f.store).is_err());
    let mut verified = caps.clone();
    verified["models"][0]["adapted"] = json!(true);
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE catalogs SET data=?1 WHERE provider='offline'",
            [verified.to_string()],
        )
        .unwrap();
    let profile = self_capabilities(&f.store).unwrap();
    assert_eq!(profile["maxDuration"], 15);
    assert_eq!(profile["planningOnly"], false);
    assert_eq!(profile["version"], "cached/v2");
    let source = f.store.source().unwrap();
    let first = source["episodes"][0]["id"].clone();
    let second = source["episodes"][1]["id"].clone();
    let rows = source["rows"].as_array().unwrap();
    let prior = rows
        .iter()
        .find(|row| row["episodeId"] == first && s(row, "kind") == "body")
        .unwrap();
    f.store.save_unit(json!({"id":"previous-unit","episodeId":first,"sourceRefs":[prior["id"]],"exitState":"prior continuity","assetRefs":[]})).unwrap();
    f.store.save_unit(json!({"id":"future-unit","episodeId":second,"sourceRefs":[],"exitState":"future state","assetRefs":[]})).unwrap();
    let input = storyboard_input(&f.store, &json!({"episodeIds":[second]})).unwrap();
    assert_eq!(input["previousBoundary"]["id"], "previous-unit");
    assert_eq!(input["previousBoundary"]["exitState"], "prior continuity");
    assert!(
        storyboard_input(&f.store, &json!({"episodeIds":[first]})).unwrap()["previousBoundary"]
            .is_null()
    );
}

#[test]
fn unknown_closure_is_local_atomic_preserves_evidence_and_unlocks_new_explicit_work() {
    let f = Fixture::new(Duration::ZERO);
    f.asset("asset");
    let snapshot =
        json!({"requestId":"unknown-original","prompt":"unchanged original","assetVersion":1});
    let key = f
        .store
        .add_job(
            "image",
            "asset",
            "offline",
            "original-account",
            &snapshot,
            "submission_unknown",
        )
        .unwrap();
    f.store.update_job(&key,"submission_unknown",&json!({"prepared":{"wire":"original"},"receipt":{"opaque":"original"},"result":{"retained":true},"error":"original submission uncertainty"})).unwrap();
    let before = f.store.job(&key).unwrap();
    let original_record = raw_job_record(&f.store, &key);
    let action = json!({"action":"jobs.close_unknown","projectId":f.project_id,"jobId":key,"confirmedNotAccepted":true,"reason":"I checked the provider record and no task was accepted"});
    let mut invalid = action.clone();
    invalid["confirmedNotAccepted"] = json!(false);
    assert!(f.engine.dispatch(invalid).is_err());
    let mut invalid = action.clone();
    invalid["reason"] = json!("   ");
    assert!(f.engine.dispatch(invalid).is_err());
    let unresolved = f.engine.dispatch(f.image_action(json!(["asset"]))).unwrap();
    let report = &unresolved["imageQueueReport"];
    assert_eq!(report["queuedCount"], 0);
    assert_eq!(report["reusedCount"], 1);
    assert_eq!(report["blockedCount"], 1);
    assert_eq!(report["reused"][0]["jobId"], key);
    assert_eq!(report["reused"][0]["status"], "submission_unknown");
    assert_eq!(report["reused"][0]["reused"], true);
    assert_eq!(report["blocked"][0]["jobId"], key);
    assert_eq!(report["blocked"][0]["status"], "submission_unknown");
    assert!(report["blocked"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("不会再次提交"));
    assert_eq!(unresolved["jobs"][0]["id"], key);
    assert_eq!(unresolved["jobs"][0]["submissionUnknown"], true);
    assert_eq!(
        f.store.jobs().unwrap().len(),
        1,
        "reuse creates no image job"
    );
    assert_eq!(f.store.job(&key).unwrap(), before);
    assert_eq!(
        raw_job_record(&f.store, &key),
        original_record,
        "reuse must preserve every stored column, including exact snapshot bytes"
    );
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
    assert!(!f.channels.submitting.load(Ordering::SeqCst));
    let workspace = f.engine.dispatch(action.clone()).unwrap();
    let after = f.store.job(&key).unwrap();
    assert_eq!(after["status"], "closed_by_user");
    assert_eq!(after["submissionUnknown"], false);
    for field in ["snapshot", "prepared", "receipt", "result", "error"] {
        assert_eq!(
            after[field], before[field],
            "original {field} must be preserved"
        );
    }
    assert_eq!(workspace["jobs"][0]["submissionUnknown"], false);
    assert!(
        f.engine.dispatch(action).is_err(),
        "closing is a one-time CAS transition"
    );
    let record: String = f
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT data FROM events WHERE job_id=?1 AND type='unknown_submission_closed_by_user'",
            [&key],
            |row| row.get(0),
        )
        .unwrap();
    let record: Value = serde_json::from_str(&record).unwrap();
    assert_eq!(record["confirmedNotAccepted"], true);
    assert_eq!(record["declaredBy"], "user");
    assert_eq!(record["channelOperationPerformed"], false);
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
    let next = f.engine.dispatch(f.image_action(json!(["asset"]))).unwrap();
    assert_eq!(next["imageQueueReport"]["queuedCount"], 1);
    assert_eq!(f.store.jobs().unwrap().len(), 2);
    assert_eq!(
        f.channels.calls.load(Ordering::SeqCst),
        0,
        "local closure and explicit queueing must not invoke submit/query/download"
    );
    for (entity, state, remote) in [
        ("known", "submission_unknown", Some("known-cloud")),
        ("queued", "queued", None),
    ] {
        let key = f
            .store
            .add_job("image", entity, "offline", "a", &json!({}), state)
            .unwrap();
        if let Some(remote) = remote {
            f.store
                .update_job(&key, state, &json!({"providerTaskId":remote}))
                .unwrap();
        }
        assert!(f.engine.dispatch(json!({"action":"jobs.close_unknown","projectId":f.project_id,"jobId":key,"confirmedNotAccepted":true,"reason":"checked"})).is_err());
        assert_eq!(f.store.job(&key).unwrap()["status"], state);
    }
}

#[test]
fn image_batch_partial_queue_reports_active_unknown_and_missing_branches() {
    let f = Fixture::new(Duration::ZERO);
    f.asset("blocked");
    f.asset("ready");
    let original_key = f
        .store
        .add_job(
            "image",
            "blocked",
            "offline",
            "a",
            &json!({}),
            "submission_unknown",
        )
        .unwrap();
    let original = f.store.job(&original_key).unwrap();
    let original_record = raw_job_record(&f.store, &original_key);
    let workspace = f
        .engine
        .dispatch(f.image_action(json!(["blocked", "ready", "missing"])))
        .unwrap();
    let report = &workspace["imageQueueReport"];
    assert_eq!(report["queuedCount"], 1);
    assert_eq!(report["reusedCount"], 1);
    assert_eq!(report["blockedCount"], 2);
    assert_eq!(report["queued"][0]["assetId"], "ready");
    assert_eq!(report["reused"][0]["assetId"], "blocked");
    assert_eq!(report["reused"][0]["jobId"], original_key);
    assert_eq!(report["reused"][0]["status"], "submission_unknown");
    assert!(report["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["assetId"] == "blocked" && item["status"] == "submission_unknown"));
    assert!(report["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["assetId"] == "missing"));
    assert_eq!(f.store.job(&original_key).unwrap(), original);
    assert_eq!(raw_job_record(&f.store, &original_key), original_record);
    let ready_key = report["queued"][0]["jobId"].as_str().unwrap().to_owned();
    let ready = f.store.job(&ready_key).unwrap();
    let ready_record = raw_job_record(&f.store, &ready_key);
    let blocked_reuse = f
        .engine
        .dispatch(f.image_action(json!(["blocked"])))
        .unwrap();
    let blocked_report = &blocked_reuse["imageQueueReport"];
    assert_eq!(blocked_report["queuedCount"], 0);
    assert_eq!(blocked_report["reusedCount"], 1);
    assert_eq!(blocked_report["blockedCount"], 1);
    assert_eq!(blocked_report["reused"][0]["jobId"], original_key);
    assert_eq!(blocked_report["blocked"][0]["jobId"], original_key);
    assert_eq!(blocked_report["blocked"][0]["status"], "submission_unknown");
    assert_eq!(f.store.jobs().unwrap().len(), 2);
    assert_eq!(f.store.job(&original_key).unwrap(), original);
    assert_eq!(raw_job_record(&f.store, &original_key), original_record);
    assert_eq!(f.store.job(&ready_key).unwrap(), ready);
    assert_eq!(raw_job_record(&f.store, &ready_key), ready_record);
    let ready_reuse = f.engine.dispatch(f.image_action(json!(["ready"]))).unwrap();
    let ready_report = &ready_reuse["imageQueueReport"];
    assert_eq!(ready_report["queuedCount"], 0);
    assert_eq!(ready_report["reusedCount"], 1);
    assert_eq!(ready_report["blockedCount"], 0);
    assert_eq!(ready_report["reused"][0]["jobId"], ready_key);
    assert_eq!(ready_report["reused"][0]["status"], "queued");
    assert_eq!(f.store.job(&original_key).unwrap(), original);
    assert_eq!(raw_job_record(&f.store, &original_key), original_record);
    assert_eq!(f.store.job(&ready_key).unwrap(), ready);
    assert_eq!(raw_job_record(&f.store, &ready_key), ready_record);
    assert_eq!(f.store.jobs().unwrap().len(), 2);
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
    assert!(!f.channels.submitting.load(Ordering::SeqCst));
}

#[test]
fn worker_drain_returns_after_receipt_commit_and_shutdown_rejects_new_guards() {
    let f = Fixture::new(Duration::from_millis(150));
    f.asset("asset");
    let workspace = f.engine.dispatch(f.image_action(json!(["asset"]))).unwrap();
    let key = workspace["imageQueueReport"]["queued"][0]["jobId"]
        .as_str()
        .unwrap()
        .to_owned();
    let (config, account) = f.engine.config(&f.store, "offline").unwrap();
    let pipeline_guard = f
        .engine
        .acquire_keys(vec!["pipeline:waiting-final-write".into()])
        .unwrap();
    let store = f.store.clone();
    let channels = f.channels.clone();
    let worker_key = key.clone();
    f.engine
        .spawn_exclusive_keys(
            vec![format!("job:{key}"), "channel:offline".into()],
            move || {
                tasks::run_generation(&store, &worker_key, &config, &account, channels.as_ref())
            },
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !f.channels.submitting.load(Ordering::SeqCst) {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    f.engine.stop.store(true, Ordering::SeqCst);
    assert!(!f.engine.await_channel_shutdown(0));
    assert!(f
        .engine
        .acquire_keys(vec!["channel:rejected".into()])
        .is_err());
    assert!(f.engine.await_channel_shutdown(2));
    let job = f.store.job(&key).unwrap();
    assert_eq!(job["status"], "submitted");
    assert_eq!(job["providerTaskId"], "offline-durable-original-id");
    assert_eq!(job["receipt"]["taskId"], "offline-durable-original-id");
    assert!(
        !f.engine.await_worker_shutdown(0),
        "all-worker drain must include final pipeline writes"
    );
    f.store
        .event("", "pipeline_final_write", &json!({"persisted":true}))
        .unwrap();
    drop(pipeline_guard);
    assert!(f.engine.await_worker_shutdown(2));
    assert!(
        f.engine.await_worker_shutdown(0),
        "repeated drain calls are safe"
    );
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 2);
}

struct ScriptedStoryboard;

/// The public desktop entry point exercises the real saved executor, compact
/// wire and guarded publication against a local response fixture only.
#[test]
fn auto_author_public_foundation_run_uses_q1_only_and_publishes_source_bound_receipt() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let f = Fixture::new(Duration::ZERO);
    let original = f.store.source().unwrap();
    let source: pipeline::LockedSource = serde_json::from_value(original.clone()).unwrap();
    let episode = &source.episodes[0].id;
    let evidence = source
        .rows
        .iter()
        .filter(|row| row.episode_id.as_ref() == Some(episode))
        .take(2)
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let asset_refs = source
        .rows
        .iter()
        .filter(|row| row.scene_id.is_some())
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let places = source
        .scenes
        .iter()
        .map(|scene| json!({"episodeId":scene.episode_id,"sceneId":scene.id}))
        .collect::<Vec<_>>();
    let choice = json!({"decisionKey":"source/name","kind":"SOURCE_CONFLICT","question":"制作解释如何保留原标注？","reason":"制作解释与原文标注分开保存","options":["源文不改，制作解释分离","源文不改，沿用标注"],"recommended":"源文不改，制作解释分离","recommendationReason":"逐字保留原文","evidence":"原文说者为甲","episodes":[episode],"sourceRefs":evidence,"changeTargets":{"assetKeys":["CH_fixture"],"assetFields":["description"],"sceneFields":["speaker"]}});
    let basis=f.store.save_foundation(json!({"schemaVersion":"foundation/v1","sourceSha256":source.sha256,"overview":"已有F2成果","direction":"已确定方向","episodes":source.episodes.iter().map(|episode|json!({"episodeId":episode.id,"summary":"","context":"","sourceRefs":episode.row_ids})).collect::<Vec<_>>(),"assets":[{"id":"CH_fixture","name":"甲","kind":"CH","aliases":[],"description":"已有形象","sourceRefs":asset_refs,"variants":[{"id":"base","name":"基础形象","description":"已有形象","sourceVariantId":null,"sourceRefs":asset_refs,"applicability":places,"scope":{"kind":"SCENES","scenes":places,"reason":null}}]}],"facts":[],"issues":[{"coordinate":"choice:source/name","problem":"制作解释待完成","sourceRefs":evidence,"blocking":true,"episodeIds":[episode],"details":{"owner":"AUTHOR","kind":"HUMAN_CHOICE","choice":choice,"recommendedIsAdopted":false}}]})).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let captured = Arc::new(Mutex::new(None));
    let captured_server = captured.clone();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut connection = loop {
            match listener.accept() {
                Ok((connection, _)) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "no local fixture request"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("local fixture accept: {error}"),
            }
        };
        // Windows can inherit the listener's nonblocking mode; the bounded
        // fixture reader must explicitly use blocking reads with its timeout.
        connection.set_nonblocking(false).unwrap();
        connection
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = vec![];
        let mut buffer = [0u8; 8192];
        let (body_start, length) = loop {
            let read = connection.read(&mut buffer).unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]);
                let length = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break (end + 4, length);
            }
        };
        while bytes.len() < body_start + length {
            let read = connection.read(&mut buffer).unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
        }
        let payload: Value =
            serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap();
        let input: Value =
            serde_json::from_str(payload["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(input["delivery"]["phase"], "AUTO_INTERPRET");
        assert_eq!(
            input["foundationDecisionPolicy"],
            pipeline::foundation_delivery::AUTO_INTERPRET_POLICY
        );
        assert_eq!(input["automaticDecisions"].as_array().unwrap().len(), 1);
        let decision = &input["automaticDecisions"][0];
        assert_eq!(decision["subjectAssetKeys"], json!(["a1"]));
        let result = json!({"format":"ASSET_OPTIMIZATION_DESKTOP_V2","sourceSha256":input["sourceSha256"],"coverage":input["scope"],"changes":{},"appliedDecisions":[{"id":decision["id"],"answer":"e1只是文字说明；保留甲的源说者与台词，制作解释独立保存。","note":"本次原Q1已采用，不新增身份或别名。","subjectAssetKeys":decision["subjectAssetKeys"]}]});
        *captured_server.lock().unwrap() = Some(payload);
        let response=json!({"choices":[{"message":{"content":result.to_string()}}],"usage":{"prompt_tokens":0,"completion_tokens":0}}).to_string();
        let header=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",response.len());
        connection.write_all(header.as_bytes()).unwrap();
        connection.write_all(response.as_bytes()).unwrap();
    });
    struct TemporaryCredential(String);
    impl Drop for TemporaryCredential {
        fn drop(&mut self) { crate::platform::remove_synthetic_credential(&self.0); }
    }
    let credential = TemporaryCredential(format!("{}:text", f.project_id));
    crate::platform::write_credential(&credential.0, "offline-local-fixture-only").unwrap();
    f.store.save_settings(&json!({"textProvider":"api","textEndpoint":endpoint,"textModel":"offline-model","reasoning":"medium"})).unwrap();
    f.engine
        .dispatch(json!({"action":"foundation.run","projectId":f.project_id}))
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let workflow = loop {
        let jobs = f.store.jobs().unwrap();
        if let Some(job) = jobs.into_iter().find(|job| {
            job["kind"] == "workflow"
                && matches!(
                    job["status"].as_str(),
                    Some("completed" | "failed" | "blocked" | "partially_blocked")
                )
        }) {
            break job;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "public foundation entry did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    server.join().unwrap();
    assert_eq!(workflow["status"], "completed", "{:?}", workflow["error"]);
    assert_eq!(
        workflow["snapshot"]["input"]["foundationPhase"],
        "AUTO_INTERPRET"
    );
    assert!(f.engine.await_worker_shutdown(2));
    assert_eq!(
        f.store.source().unwrap(),
        original,
        "source SHA, source rows, speakers and dialogue must be byte-preserved"
    );
    let jobs = f.store.jobs().unwrap();
    let text = jobs
        .iter()
        .filter(|job| job["kind"] == "text")
        .collect::<Vec<_>>();
    assert_eq!(text.len(), 1);
    assert!(s(text[0], "entityId").starts_with("foundation:Q1"));
    assert_eq!(text[0]["status"], "completed");
    assert_eq!(text[0]["receipt"]["validation"], "accepted");
    assert!(!jobs
        .iter()
        .any(|job| s(job, "entityId").starts_with("foundation:F2")));
    let next = f.store.foundation().unwrap();
    assert_eq!(next["assets"][0]["id"], basis["assets"][0]["id"]);
    assert_eq!(next["assets"][0]["aliases"], basis["assets"][0]["aliases"]);
    assert_eq!(next["facts"], basis["facts"]);
    let receipt = &next["authorDecisions"][0];
    assert_eq!(receipt["status"], "APPLIED");
    assert_eq!(receipt["sourceSha256"], source.sha256);
    assert_eq!(receipt["sourceRefs"], json!(evidence));
    assert_eq!(receipt["subjectAssetKeys"], json!(["CH_fixture"]));
    assert!(receipt["receipts"][0]["note"]
        .as_str()
        .is_some_and(|note| !note.is_empty()));
    assert!(receipt["answer"]
        .as_str()
        .unwrap()
        .starts_with("e1只是文字说明"));
    assert!(next["issues"]
        .as_array()
        .unwrap()
        .iter()
        .all(|issue| issue["blocking"] == false));
    assert!(captured.lock().unwrap().is_some());
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
}
#[async_trait::async_trait]
impl pipeline::TextExecutor for ScriptedStoryboard {
    async fn execute(&self, request: pipeline::TextRequest) -> Result<Value> {
        assert_eq!(request.input["capabilities"]["planningOnly"], true);
        assert_eq!(request.input["capabilities"]["maxTotalReferences"], 50);
        let row = &request.input["sourceRows"][0];
        let episode = if request.stage == "storyboardPlan" {
            request.input["episode"]["id"].clone()
        } else {
            request.input["episodeId"].clone()
        };
        let emotion =
            json!({"subject":"甲","change":"决定行动","landing":"开口","retain":["原话"]});
        if request.stage == "storyboardPlan" {
            return Ok(
                json!({"schemaVersion":"storyboard-plan/v2","unitPackingPolicy":"event-first-v1","emotionArc":"决定行动","events":[{"id":"event","sceneId":row["sceneId"],"sourceRefs":[row["id"]],"boundaryBefore":{"kind":"episode_start","reason":"当前源集开始"},"emotionContract":{"subject":"甲","contextTone":"当前源情境","audienceEffect":"行动决定可读","change":"决定行动","landing":"开口","retain":["原话"]}}],"workPackages":[{"id":"package","eventId":"event","sourceRefs":[row["id"]],"contextRefs":[],"entryFactIds":[],"dependsOn":[],"scenarioKeys":[],"emotionContract":{"subject":"甲","contextTone":"当前源情境","audienceEffect":"行动决定可读","change":"决定行动","landing":"开口","retain":["原话"]}}],"provisionalUnits":[{"id":"provisional","packageId":"package","sourceRefs":[row["id"]],"secondsRange":[5,5]}],"entryFacts":[],"fantasyCandidates":[]}),
            );
        }
        if request.stage == "storyboardFinish" {
            assert_eq!(request.input["finishMode"], "delta");
            assert_eq!(
                request.schema["properties"]["schemaVersion"]["enum"][0],
                "storyboard-delta/v2"
            );
            let base = &request.input["previousDraft"];
            let base_hash = pipeline::digest(
                &json!({"schemaVersion":base["schemaVersion"],"units":base["units"],"fantasyRealization":base["fantasyRealization"]}).to_string(),
            );
            assert_eq!(request.input["baseDraftHash"], base_hash);
            assert_eq!(
                request.schema["properties"]["baseDraftHash"]["enum"][0],
                base_hash
            );
            let units = base["units"].as_array().unwrap();
            assert!(!units.is_empty());
            let retained = units
                .iter()
                .map(|unit| unit["id"].clone())
                .collect::<Vec<_>>();
            return Ok(
                json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":base_hash,"retainedUnitIds":retained,"replacements":[],"revisionNotes":[],"unresolved":[]}),
            );
        }
        let dialogue = &row["dialogue"];
        Ok(
            json!({"schemaVersion":"storyboard-draft/v2","units":[{"id":"unit","episodeId":episode,"eventId":"event","sceneId":row["sceneId"],"boundaryBefore":{"kind":"episode_start","reason":"当前源集开始"},"title":"甲开口","unitDirection":"甲抬头说话。","intent":emotion,"scene":"原文指定场景","participants":["甲"],"props":[],"lighting":"自然侧光","initialState":"甲站定。","exitState":"甲说完后看向门口。","sourceRefs":[row["id"]],"assetRefs":[],"shots":[{"id":"shot","duration":5.0,"camera":"近景","picture":"甲抬头开口。","lighting":"自然侧光","sound":"环境声与现场对白","dialogueParts":[{"lineId":dialogue["id"],"quoteStart":dialogue["spokenSegments"][0]["range"]["start"],"quoteEnd":dialogue["spokenSegments"][0]["range"]["end"],"presentation":"画内","delivery":"自然说话","voiceStart":0.0,"voiceEnd":2.0}],"transition":"切","sourceRefs":[row["id"]],"contextRefs":[]}]}],"fantasyRealization":[],"revisionNotes":[],"unresolved":[]}),
        )
    }
}

#[tokio::test]
async fn planning_profile_generates_all_selected_episodes_with_exact_dialogue_and_no_video_model() {
    let f = Fixture::new(Duration::ZERO);
    let source = f.store.source().unwrap();
    let first = source["episodes"][0]["id"].clone();
    let second = source["episodes"][1]["id"].clone();
    let input = storyboard_input(&f.store, &json!({"episodeIds":[second,first]})).unwrap();
    let output = pipeline::run_storyboard(&input, &ScriptedStoryboard, "offline", "medium")
        .await
        .unwrap();
    assert!(
        output["blocked"].as_array().unwrap().is_empty(),
        "{:?}",
        output["blocked"]
    );
    let units = output["units"].as_array().unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0]["episodeId"], first);
    assert_eq!(units[1]["episodeId"], second);
    assert!(s(&units[0], "prompt").contains("走吧。"));
    assert!(s(&units[1], "prompt").contains("回来了。"));
    assert_eq!(
        units[0]["sourceCoverage"][0]["rowId"],
        source["rows"][2]["id"]
    );
    let input = storyboard_input(&f.store, &json!({"episodeIds":[second]})).unwrap();
    let filtered = pipeline::run_storyboard(&input, &ScriptedStoryboard, "offline", "medium")
        .await
        .unwrap();
    assert!(
        filtered["blocked"].as_array().unwrap().is_empty(),
        "{:?}",
        filtered["blocked"]
    );
    assert_eq!(filtered["units"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["units"][0]["episodeId"], second);
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn stage2_visible_text_preview_matches_new_frozen_image_and_preserves_old_snapshot() {
    let f = Fixture::new(Duration::ZERO);
    let body = "店铺门头招牌写“春山医馆”，桌上纸条写“明日休息”。";
    f.store.save_asset(json!({"id":"shop","variantId":"base","kind":"LOC","prompt":body,"promptOrigin":"MANUAL","sourceRefs":[]})).unwrap();
    f.store.save_settings(&json!({"projectVisualStyle":"三渲二国漫；无文字","ratio":"16:9"})).unwrap();
    let original_asset = f.store.get("assets", "shop").unwrap();
    let original_settings = f.store.settings().unwrap();
    let legacy_snapshot = json!({"requestId":"old-image-request","kind":"image","provider":"offline","model":"old","prompt":"已经提交的原提示词","promptBody":body,"ratio":"16:9","resolution":"original","autoSubmit":true});
    let old = f.store.add_job("image", "old-shop", "offline", "local", &legacy_snapshot, "downloaded").unwrap();
    let preview = f.engine.dispatch(json!({"action":"images.previewPrompt","projectId":f.project_id,"assetId":"shop"})).unwrap();
    assert_eq!(preview["promptBody"], body);
    assert_eq!(preview["promptCompilerVersion"], pipeline::asset_design_contract::IMAGE_PROMPT_COMPILER_VERSION);
    assert!(s(&preview,"prompt").contains(body));
    let queued = f.engine.queue_images(&f.store, &json!({"provider":"offline","model":"fake","assetIds":["shop"]})).unwrap();
    let image = f.store.job(s(&queued["queued"][0],"jobId")).unwrap();
    assert_eq!(image["snapshot"]["prompt"], preview["prompt"]);
    assert_eq!(image["snapshot"]["promptCompilerVersion"], preview["promptCompilerVersion"]);
    assert_eq!(tasks::generation_request(&image["snapshot"]).unwrap().prompt, s(&preview,"prompt"));
    assert_eq!(f.store.get("assets","shop").unwrap(), original_asset);
    assert_eq!(f.store.settings().unwrap(), original_settings);
    assert_eq!(f.store.job(&old).unwrap()["snapshot"], legacy_snapshot);
    assert_eq!(tasks::generation_request(&legacy_snapshot).unwrap().prompt, "已经提交的原提示词");
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn legacy_stage2_evidence_policy_persists_on_exact_resume_without_calls() {
    let f = Fixture::new(Duration::ZERO);
    f.store.save_asset(json!({"id":"manual","variantId":"base","kind":"CH","prompt":"用户手写完整形象","promptOrigin":"MANUAL","sourceRefs":[]})).unwrap();
    let request = json!({"action":"images.produce","projectId":f.project_id,"provider":"offline","model":"fake","assetIds":["manual"]});
    // This is a historical frozen request. A newly created images.produce now
    // uses the 1.1.2 contract, so construct the actual legacy artifact explicitly.
    let settings=f.store.settings().unwrap();
    let mut context=json!({"source":f.store.source().unwrap(),"foundation":f.store.foundation().unwrap(),"assets":f.store.values("assets").unwrap(),"selection":{"assetIds":["manual"]},"settings":settings,"designRuleSha256":pipeline::rule_snapshot("asset-art-director").unwrap()["sha256"]});
    pipeline::freeze_design_evidence_profile(&mut context);
    let snapshot=json!({"stage":"images.produce","sourceVersion":f.store.source().unwrap()["version"],"settings":settings,"request":request,"stage2Context":context});
    let key=f.store.add_job("workflow","images.produce:","local","local",&snapshot,"completed").unwrap();
    let old=f.store.job(&key).unwrap();
    let profile = old["snapshot"]["stage2Context"][pipeline::DESIGN_EVIDENCE_PROFILE_FIELD].clone();
    assert!(!profile.is_null(), "legacy snapshot must retain its original evidence policy");
    f.store.update_job(s(&old,"id"),"completed",&json!({"result":{"status":"partiallyQueued","blocked":[{"assetId":"manual","reason":"offline hold"}]}})).unwrap();
    let before = raw_job_record(&f.store, s(&old,"id"));
    let reopened = Store::open(f.store.root.clone()).unwrap();
    f.engine.dispatch(json!({"action":"workflows.resume","projectId":f.project_id,"workflowId":old["id"]})).unwrap();
    assert!(f.engine.await_worker_shutdown(3));
    let jobs = reopened.jobs().unwrap();
    let resumed = jobs.iter().find(|j| j["kind"] == "workflow" && j["id"] != old["id"]).unwrap();
    assert_eq!(resumed["snapshot"]["stage2Context"][pipeline::DESIGN_EVIDENCE_PROFILE_FIELD],profile);
    assert_eq!(raw_job_record(&f.store, s(&old,"id")),before);
    assert_eq!(jobs.iter().filter(|j| j["kind"] == "image").count(),1);
    assert_eq!(f.store.get("assets","manual").unwrap()["prompt"],"用户手写完整形象");
    assert_eq!(f.channels.calls.load(Ordering::SeqCst),0);
}

#[test]
fn stage2_restart_resume_uses_original_settings_and_final_submission_snapshot() {
    let f = Fixture::new(Duration::ZERO);
    f.store.save_asset(json!({"id":"manual","variantId":"base","kind":"CH","prompt":"六十岁深肤色人物，招牌写第十集","promptOrigin":"MANUAL","sourceRefs":[]})).unwrap();
    f.store.save_settings(&json!({"projectVisualStyle":"原水墨画风","ratio":"3:2","imageResolution":"original-size","imagePresentation":"原明确构图","dreaminaPath":"frozen-original-cli"})).unwrap();
    let settings=f.store.settings().unwrap();
    let request=json!({"action":"images.produce","projectId":f.project_id,"provider":"offline","model":"original-model","assetIds":["manual"],"imageProductionPolicy":"automatic"});
    let original_snapshot=json!({"stage":"images.produce","sourceVersion":f.store.source().unwrap()["version"],"settings":settings,"request":request,"imageProductionPolicy":"automatic"});
    let old=f.store.add_job("workflow","images.produce:","local","local",&original_snapshot,"completed").unwrap();
    f.store.update_job(&old,"completed",&json!({"result":{"status":"partially_blocked","blocked":[{"assetId":"manual","reason":"offline fixture"}]}})).unwrap();
    let original=f.store.job(&old).unwrap();
    f.store.save_settings(&json!({"projectVisualStyle":"新画风不得混入","ratio":"9:16","imageResolution":"new-size","imagePresentation":"新构图","dreaminaPath":"changed-cli"})).unwrap();
    let reopened=Store::open(f.store.root.clone()).unwrap();
    f.engine.dispatch(json!({"action":"workflows.resume","projectId":f.project_id,"workflowId":old})).unwrap();
    assert!(f.engine.await_worker_shutdown(3));
    let jobs=reopened.jobs().unwrap();
    let image=jobs.iter().find(|j|j["kind"]=="image").expect("original workflow must queue one image");
    let snapshot=&image["snapshot"];
    assert_eq!(snapshot["model"],"original-model");
    assert_eq!(snapshot["ratio"],"3:2");
    assert_eq!(snapshot["resolution"],"original-size");
    assert_eq!(snapshot["visualStyle"],"原水墨画风");
    assert_eq!(snapshot["imagePresentation"]["composition"],"原明确构图");
    assert_eq!(snapshot["frozenSettings"],settings);
    let summary=reopened.pending_jobs().unwrap().into_iter().find(|j|j["id"]==image["id"]).unwrap();
    assert!(summary["snapshot"].get("frozenSettings").is_none());
    let (configuration,_)=f.engine.generation_config(&reopened,&summary).unwrap();
    assert_eq!(configuration.cli_path.as_deref(),Some(Path::new("frozen-original-cli")));
    let submitted=tasks::generation_request(snapshot).unwrap();
    assert_eq!(submitted.model,"original-model");
    assert_eq!(submitted.ratio,"3:2");
    assert_eq!(submitted.prompt,s(snapshot,"prompt"));
    assert!(s(snapshot,"prompt").contains("第十集"));
    assert!(!s(snapshot,"prompt").contains("新画风"));
    assert_eq!(reopened.job(&old).unwrap(),original);
    assert_eq!(f.channels.calls.load(Ordering::SeqCst),0);
    reopened.invalidate_unsubmitted_settings("new defaults").unwrap();
    assert_eq!(reopened.job(s(image,"id")).unwrap()["status"],"queued");
    assert_eq!(reopened.job(s(image,"id")).unwrap()["snapshot"],*snapshot);
}

#[test]
fn stage2_busy_new_and_resume_preserve_original_jobs_events_and_frozen_budgets() {
    let f = Fixture::new(Duration::ZERO);
    let source_version = f.store.source().unwrap()["version"].clone();
    let mut originals = Vec::new();
    for stage in ["images.design", "images.produce"] {
        let snapshot = json!({"runnerVersion":"stage2-runner/v1","stage":stage,"sourceVersion":source_version,
            "settings":f.store.settings().unwrap(),"request":{"action":stage,"assetIds":["original-target"]},
            "nativeAcceptance":{"reservationId":"original-offline-budget","attemptLimit":1}});
        let run = f.store.add_job("workflow", &format!("{stage}:"), "local", "local", &snapshot, "partially_blocked").unwrap();
        f.store.update_job(&run,"partially_blocked",&json!({"error":"original diagnostic", "result":{"status":"partially_blocked","blocked":[{"assetId":"original-target","reason":"original reason"}]},"receipt":{"originalRaw":"unchanged","budgetConsumed":1}})).unwrap();
        originals.push((run.clone(), raw_job_record(&f.store, &run)));
    }
    let unknown = f.store.add_job("image", "original-target", "offline", "original-account", &json!({"originalFrozen":true}), "submission_unknown").unwrap();
    originals.push((unknown.clone(), raw_job_record(&f.store, &unknown)));
    let jobs = f.store.jobs().unwrap();
    let events: u64 = f.store.conn().unwrap().query_row("SELECT count(*) FROM events", [], |row|row.get(0)).unwrap();
    let lease = f.engine.acquire_pipeline(&f.store).unwrap();
    for stage in ["images.design", "images.produce"] {
        let error = f.engine.spawn_stage(f.store.clone(), json!({"action":stage,"assetIds":["original-target"]})).unwrap_err();
        assert!(error.contains("本项目的文字制作流程正在执行"), "{stage}: {error}");
    }
    for (run, _) in originals.iter().take(2) {
        let error = f.engine.resume_production(f.store.clone(), &json!({"action":"workflows.resume","workflowId":run})).unwrap_err();
        assert!(error.contains("本项目的文字制作流程正在执行"), "{error}");
    }
    assert_eq!(f.store.jobs().unwrap(), jobs, "no new or failed workflow, no attempt or media request");
    for (run, raw) in originals { assert_eq!(raw_job_record(&f.store, &run), raw, "all original persisted columns remain exact"); }
    assert_eq!(f.store.conn().unwrap().query_row("SELECT count(*) FROM events", [], |row|row.get::<_,u64>(0)).unwrap(), events);
    assert_eq!(f.channels.calls.load(Ordering::SeqCst), 0);
    drop(lease);
    assert!(f.engine.active.lock().unwrap().is_empty());
}

#[test]
fn stage2_direct_images_existing_prompt_cannot_bypass_unassigned_facts() {
    let f = Fixture::new(Duration::ZERO);
    let source=f.store.source().unwrap();
    let row=source["rows"].as_array().unwrap()[0]["id"].clone();
    f.store.save_asset(json!({"id":"a","baseAssetId":"person","kind":"CH","variantId":"base","promptOrigin":"MANUAL","prompt":"已有完整设计","sourceRefs":[row]})).unwrap();
    f.store.save_foundation(json!({"facts":[{"id":"future","assetId":"person","variantId":null,"attribute":"hair","value":"未来短发","scope":"TRANSITION","claim":"SOURCE","sourceRefs":[row]}]})).unwrap();
    let result=f.engine.queue_images(&f.store,&json!({"provider":"offline","model":"fake","assetIds":["a"]}));
    assert!(result.unwrap_err().contains("适用形象尚未确定"));
    assert!(f.store.jobs().unwrap().is_empty());
    assert_eq!(f.channels.calls.load(Ordering::SeqCst),0);
}

#[test]
fn stage2_direct_images_stale_not_applicable_receipt_cannot_bypass_gate() {
    let f=Fixture::new(Duration::ZERO);
    f.store.save_asset(json!({"id":"a","baseAssetId":"person","kind":"CH","variantId":"base","promptOrigin":"MANUAL","prompt":"已有完整设计","sourceRefs":[]})).unwrap();
    f.store.save_foundation(json!({"facts":[],"visualFactBatches":[{"deferredFacts":[{"fact":{"assetId":"person"},"notApplicableAssetIds":["a"]}]}]})).unwrap();
    let result=f.engine.queue_images(&f.store,&json!({"provider":"offline","model":"fake","assetIds":["a"]}));
    assert!(result.unwrap_err().contains("本批不适用裁决已失效"));
    assert!(f.store.jobs().unwrap().is_empty());
    assert_eq!(f.channels.calls.load(Ordering::SeqCst),0);
}
