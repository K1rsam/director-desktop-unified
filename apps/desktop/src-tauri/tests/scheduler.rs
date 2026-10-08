//! Actual local scheduler acceptance. All upstream operations are in-memory fakes.
//! These tests prove queue/recovery contracts, not generated-video visual quality.
use director_core::{
    binding, pipeline,
    providers::*,
    store::{s, Store},
    tasks::{self, Channels},
    Engine,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

#[derive(Default)]
struct Statistics {
    active: BTreeMap<String, usize>,
    peak: BTreeMap<String, usize>,
    prepares: BTreeMap<String, GenerationRequest>,
    submits: BTreeMap<String, usize>,
    queries: BTreeMap<String, usize>,
    remote_kinds: BTreeMap<String, String>,
}

struct Fake {
    stats: Mutex<Statistics>,
    query_delay: Duration,
    prepare_hook: Mutex<Option<PrepareHook>>,
}
type PrepareHook =
    Box<dyn Fn(&GenerationRequest, &mut PreparedRequest) -> ProviderResult<()> + Send + Sync>;

struct Operation<'a> {
    fake: &'a Fake,
    channel: String,
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        let mut stats = self.fake.stats.lock().unwrap();
        *stats.active.get_mut(&self.channel).unwrap() -= 1;
    }
}

impl Fake {
    fn new(query_delay: Duration) -> Self {
        Self {
            stats: Mutex::new(Statistics::default()),
            query_delay,
            prepare_hook: Mutex::new(None),
        }
    }
    fn enter(&self, channel: &str) -> Operation<'_> {
        let mut stats = self.stats.lock().unwrap();
        let active = stats.active.entry(channel.into()).or_default();
        *active += 1;
        let count = *active;
        let peak = stats.peak.entry(channel.into()).or_default();
        *peak = (*peak).max(count);
        drop(stats);
        Operation {
            fake: self,
            channel: channel.into(),
        }
    }
    fn submission_count(&self) -> usize {
        self.stats.lock().unwrap().submits.values().sum()
    }
    fn query_count(&self, remote: &str) -> usize {
        *self.stats.lock().unwrap().queries.get(remote).unwrap_or(&0)
    }
    fn assert_serial(&self) {
        let stats = self.stats.lock().unwrap();
        assert!(
            !stats.peak.is_empty(),
            "fixture must actually call a channel"
        );
        assert!(
            stats.peak.values().all(|n| *n <= 1),
            "local channel operations overlapped: {:?}",
            stats.peak
        );
    }
    fn idle(&self) -> bool {
        self.stats.lock().unwrap().active.values().all(|n| *n == 0)
    }
}

impl Channels for Fake {
    fn capabilities(&self, cfg: &ProviderConfig) -> ProviderResult<Capabilities> {
        let _guard = self.enter(&cfg.kind);
        let models = ["image","video"].map(|kind|json!({"id":format!("mock-{kind}"),"kind":kind,"adapted":true,"minDuration":if kind=="video"{json!(4)}else{json!(null)},"maxDuration":if kind=="video"{json!(15)}else{json!(null)},"allowedDurations":[],"ratios":["16:9"],"resolutions":["720p","2K"],"images":9,"videos":3,"audios":3,"totalReferences":12,"requiredImages":0,"price":1.0,"billing":"task","currency":"mock credits","videoPriceMultiplier":1.0,"raw":{}}));
        Ok(serde_json::from_value(json!({"provider":cfg.kind,"version":"scheduler-fake/v1","source":"offline fake","checkedAt":"test","models":models,"canCancel":false,"canReconcileUnknown":false,"raw":{}})).unwrap())
    }
    fn prepare(
        &self,
        cfg: &ProviderConfig,
        request: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        let _guard = self.enter(&cfg.kind);
        thread::sleep(Duration::from_millis(3));
        self.stats
            .lock()
            .unwrap()
            .prepares
            .insert(request.request_id.clone(), request.clone());
        let mut prepared = PreparedRequest {
            provider: cfg.kind.clone(),
            request: request.clone(),
            capability_version: "scheduler-fake/v1".into(),
            upload_receipts: vec![],
            wire: json!({"requestId":request.request_id}),
            estimated_cost: Some(1.0),
            prepared_at: "test".into(),
        };
        if let Some(hook) = self.prepare_hook.lock().unwrap().as_ref() {
            hook(request, &mut prepared)?;
        }
        Ok(prepared)
    }
    fn submit(
        &self,
        cfg: &ProviderConfig,
        prepared: &PreparedRequest,
    ) -> ProviderResult<SubmitReceipt> {
        let _guard = self.enter(&cfg.kind);
        thread::sleep(Duration::from_millis(3));
        let request = &prepared.request;
        *self
            .stats
            .lock()
            .unwrap()
            .submits
            .entry(request.request_id.clone())
            .or_default() += 1;
        if request.request_id == "episode-unknown" {
            return Err(ProviderError::uncertain(
                "mock timeout after upstream accepted the request",
            ));
        }
        let remote = format!("remote:{}", request.request_id);
        self.stats
            .lock()
            .unwrap()
            .remote_kinds
            .insert(remote.clone(), request.kind.clone());
        Ok(SubmitReceipt {
            task_id: remote,
            status: "processing".into(),
            result_urls: vec![],
            raw: json!({"requestId":request.request_id}),
        })
    }
    fn query(&self, cfg: &ProviderConfig, remote: &str) -> ProviderResult<QueryResult> {
        let _guard = self.enter(&cfg.kind);
        *self
            .stats
            .lock()
            .unwrap()
            .queries
            .entry(remote.into())
            .or_default() += 1;
        thread::sleep(self.query_delay);
        if remote == "remote:forever-processing" {
            return Ok(QueryResult {
                task_id: remote.into(),
                status: "processing".into(),
                result_urls: vec![],
                error: None,
                actual_cost: None,
                raw: json!({"offline":true}),
            });
        }
        if remote == "remote:not-found" {
            return Err(ProviderError::new(
                "CLI_QUERY_FAILED",
                "offline fixture: task not found",
            ));
        }
        if remote == "remote:episode-query-fails" {
            return Err(ProviderError::new(
                "MOCK_QUERY_OUTAGE",
                "only this original task remains unavailable",
            ));
        }
        let kind = self
            .stats
            .lock()
            .unwrap()
            .remote_kinds
            .get(remote)
            .cloned()
            .unwrap_or_else(|| "video".into());
        Ok(QueryResult {
            task_id: remote.into(),
            status: "succeeded".into(),
            result_urls: vec![format!(
                "https://fake.invalid/{remote}.{}",
                if kind == "image" { "png" } else { "mp4" }
            )],
            error: None,
            actual_cost: Some(1.1),
            raw: json!({"offline":true}),
        })
    }
    fn download(
        &self,
        cfg: &ProviderConfig,
        url: &str,
        path: &Path,
    ) -> ProviderResult<DownloadReceipt> {
        let _guard = self.enter(&cfg.kind);
        thread::sleep(Duration::from_millis(3));
        if url.ends_with(".png") {
            image::RgbImage::from_pixel(2, 2, image::Rgb([19, 31, 73]))
                .save(path)
                .unwrap();
        } else {
            fs::write(path, format!("offline video fixture for {url}")).unwrap();
        }
        let (hash, bytes) = director_core::store::hash_file(path).unwrap();
        Ok(DownloadReceipt {
            path: path.into(),
            sha256: hash,
            bytes,
            mime: Some(
                if url.ends_with(".png") {
                    "image/png"
                } else {
                    "video/mp4"
                }
                .into(),
            ),
        })
    }
    fn reconcile(&self, _cfg: &ProviderConfig, _request: &str) -> ProviderResult<QueryResult> {
        Err(ProviderError::new(
            "MANUAL_REQUIRED",
            "unknown submission cannot be guessed",
        ))
    }
}

struct Fixture {
    _temp: TempDir,
    engine: Arc<Engine>,
    store: Store,
    fake: Arc<Fake>,
    project_id: String,
}
impl Fixture {
    fn new(episodes: usize, query_delay: Duration) -> Self {
        let temp = TempDir::new().unwrap();
        let fake = Arc::new(Fake::new(query_delay));
        let engine = Engine::with_channels(temp.path().join("new-home"), fake.clone()).unwrap();
        let project = engine
            .dispatch(json!({"action":"projects.create","title":"offline scheduler fixture"}))
            .unwrap();
        let project_id = s(&project, "id").to_string();
        let store = engine.store(&project_id).unwrap();
        let text = (1..=episodes)
            .map(|n| format!("第{n}集\n场景：测试门口{n}\n△本集人物抬头看门。\n"))
            .collect::<String>();
        let source = pipeline::prepare_source(&text).unwrap();
        assert!(source.diagnostics.is_empty());
        store
            .save_source(serde_json::to_value(&source).unwrap())
            .unwrap();
        for (index, episode) in source.episodes.iter().enumerate() {
            let row = source
                .rows
                .iter()
                .find(|r| r.kind == "body" && r.episode_id.as_ref() == Some(&episode.id))
                .unwrap();
            store.save_unit(json!({"id":format!("unit-{index}"),"episodeId":episode.id,"sceneId":episode.scene_ids[0],"sourceRefs":[row.id],"duration":5,"prompt":format!("第{}集门口人物抬头看门，独立镜头。",index+1),"assetRefs":[]})).unwrap();
        }
        // Fake result bytes are intentionally not visual-acceptance fixtures; do not launch ffprobe.
        store.save_settings(&json!({"trialApproved":true,"ffprobePath":store.root.join("missing-scheduler-fixture-ffprobe.exe")})).unwrap();
        // No configured real channels or credential reads: skip the optional BH catalogue refresh.
        for key in ["catalog_attempt:bh", "backed_up_at"] {
            store.conn().unwrap().execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,director_core::store::now()]).unwrap();
        }
        Self {
            _temp: temp,
            engine,
            store,
            fake,
            project_id,
        }
    }
    fn queue_video(&self, index: usize, request_id: &str) -> String {
        self.queue_video_on(index, request_id, "mock")
    }
    fn queue_video_on(&self, index: usize, request_id: &str, provider: &str) -> String {
        self.queue_video_mode(index, request_id, provider, true)
    }
    fn queue_review_video(&self, index: usize, request_id: &str) -> String {
        self.queue_video_mode(index, request_id, "mock", false)
    }
    fn queue_video_mode(
        &self,
        index: usize,
        request_id: &str,
        provider: &str,
        submit: bool,
    ) -> String {
        let uid = format!("unit-{index}");
        let unit = self.store.get("units", &uid).unwrap();
        let mut snapshot = binding::bind(&self.store, &unit, provider, "mock-video").unwrap();
        assert!(
            snapshot["problems"].as_array().unwrap().is_empty(),
            "{:?}",
            snapshot["problems"]
        );
        snapshot["requestId"] = json!(request_id);
        snapshot["autoSubmit"] = json!(submit);
        let (_, account) = self.engine.config(&self.store, provider).unwrap();
        self.store
            .add_job("video", &uid, provider, &account, &snapshot, "queued")
            .unwrap()
    }
    fn queue_image(&self, request_id: &str) -> String {
        let asset_id = format!("image-{request_id}");
        self.store.save_asset(json!({"id":asset_id,"variantId":"base","name":asset_id,"kind":"CH","prompt":"离线调度人物设定图"})).unwrap();
        let asset = self.store.active_asset(&asset_id).unwrap();
        let snapshot = json!({"requestId":request_id,"kind":"image","assetId":asset_id,"variantId":"base","assetVersion":asset["version"],"model":"mock-image","prompt":"离线调度人物设定图","ratio":"16:9","resolution":"2K","references":[],"options":{},"autoSubmit":true});
        binding::validate_generation_snapshot(&self.store, &snapshot).unwrap();
        let (_, account) = self.engine.config(&self.store, "mock").unwrap();
        self.store
            .add_job("image", &asset_id, "mock", &account, &snapshot, "queued")
            .unwrap()
    }
    fn known_image(&self, request_id: &str, remote: &str, state: &str) -> String {
        let key = self.queue_image(request_id);
        self.store
            .update_job(
                &key,
                state,
                &json!({"providerTaskId":remote,"receipt":{"taskId":remote,"original":true}}),
            )
            .unwrap();
        self.fake
            .stats
            .lock()
            .unwrap()
            .remote_kinds
            .insert(remote.into(), "image".into());
        key
    }
    fn one_tick(&self) {
        self.engine.tick().unwrap();
        assert!(
            self.engine.await_channel_shutdown(5),
            "owned channel worker must finish within the offline fixture budget"
        );
    }
}

#[test]
fn thirty_seven_review_only_videos_prepare_in_bounded_channel_bursts_without_any_submission() {
    let f = Fixture::new(37, Duration::ZERO);
    let calls = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let counted = calls.clone();
    *f.fake.prepare_hook.lock().unwrap() = Some(Box::new(move |request, _| {
        *counted
            .lock()
            .unwrap()
            .entry(request.request_id.clone())
            .or_default() += 1;
        Ok(())
    }));
    let originals = (0..37)
        .map(|index| {
            let key = f.queue_review_video(index, &format!("review-{index}"));
            let snapshot = f.store.job(&key).unwrap()["snapshot"].clone();
            (key, snapshot)
        })
        .collect::<Vec<_>>();
    // The production burst yields after 64 attempts OR two seconds. Parallel
    // tests/builds may consume the wall-clock budget before all 37 units finish.
    let mut ready = 0;
    for round in 1..=5 {
        let previous_ready = ready;
        let previous_calls: usize = calls.lock().unwrap().values().sum();
        f.one_tick();
        ready = originals
            .iter()
            .filter(|(key, _)| f.store.job(key).unwrap()["status"] == "ready")
            .count();
        let total_calls: usize = calls.lock().unwrap().values().sum();
        assert!(ready > previous_ready, "bounded preparation round {round} must advance queued work: {previous_ready} -> {ready}");
        assert!(
            total_calls - previous_calls <= 64,
            "one reservation must never exceed the attempt cap"
        );
        assert_eq!(
            total_calls, ready,
            "each prepared unit has exactly one channel preparation call"
        );
        assert!(
            calls.lock().unwrap().values().all(|count| *count == 1),
            "ready units must not be prepared again between rounds"
        );
        assert_eq!(f.fake.submission_count(), 0);
        eprintln!("offline review preparation round {round}: {previous_ready} -> {ready}/37 ready, total prepare calls {total_calls}, submits 0");
        if ready == originals.len() {
            break;
        }
    }
    assert_eq!(
        ready, 37,
        "the bounded fixture must converge within five scheduler ticks"
    );
    for (key, snapshot) in &originals {
        let job = f.store.job(key).unwrap();
        assert_eq!(job["status"], "ready", "{job}");
        assert_eq!(&job["snapshot"], snapshot);
        assert!(s(&job, "providerTaskId").is_empty());
        assert!(job["receipt"].is_null());
    }
    assert_eq!(f.fake.stats.lock().unwrap().prepares.len(), 37);
    assert_eq!(f.fake.submission_count(), 0);
    f.one_tick();
    assert_eq!(
        calls.lock().unwrap().values().sum::<usize>(),
        37,
        "idle manual-ready tasks must not call prepare again"
    );
    assert_eq!(
        f.fake.stats.lock().unwrap().prepares.len(),
        37,
        "manual-ready jobs never prepare again"
    );
    assert_eq!(f.fake.submission_count(), 0);
    f.fake.assert_serial();
}

#[test]
fn review_burst_is_bounded_and_yields_the_next_slot_to_original_id_recovery() {
    let f = Fixture::new(80, Duration::ZERO);
    for index in 0..80 {
        f.queue_review_video(index, &format!("bounded-review-{index}"));
    }
    let known = f.known_image(
        "pending-before-burst",
        "remote:forever-processing",
        "submitted",
    );
    f.one_tick();
    assert_eq!(f.fake.query_count("remote:forever-processing"), 1);
    assert_eq!(f.fake.stats.lock().unwrap().prepares.len(), 0);
    f.one_tick();
    let first_count = f.fake.stats.lock().unwrap().prepares.len();
    assert!(
        (2..=64).contains(&first_count),
        "local burst must make progress without crossing its count/time cap: {first_count}"
    );
    assert_eq!(f.fake.submission_count(), 0);
    f.store
        .conn()
        .unwrap()
        .execute("UPDATE jobs SET next_attempt_at=0 WHERE id=?1", [&known])
        .unwrap();
    f.one_tick();
    assert_eq!(
        f.fake.query_count("remote:forever-processing"),
        2,
        "a local preparation burst cannot consume the following recovery turn"
    );
    assert_eq!(f.fake.stats.lock().unwrap().prepares.len(), first_count);
    // The following normal generation turn remains exactly one paid submission.
    let paid = f.queue_image("explicit-paid-slot");
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET updated_at='2025-01-01T00:00:00+00:00' WHERE id=?1",
            [&paid],
        )
        .unwrap();
    f.one_tick();
    assert_eq!(f.fake.submission_count(), 1);
    assert_eq!(f.fake.stats.lock().unwrap().prepares.len(), first_count + 1);
    f.fake.assert_serial();
}

#[test]
fn preparation_checks_its_soft_time_budget_and_shutdown_between_items() {
    for shutdown in [false, true] {
        let f = Fixture::new(3, Duration::ZERO);
        for index in 0..3 {
            f.queue_review_video(index, &format!("stop-review-{index}"));
        }
        let engine = Arc::downgrade(&f.engine);
        *f.fake.prepare_hook.lock().unwrap() = Some(Box::new(move |_, _| {
            if shutdown {
                engine
                    .upgrade()
                    .unwrap()
                    .stop
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            } else {
                thread::sleep(Duration::from_millis(2100));
            }
            Ok(())
        }));
        f.one_tick();
        assert_eq!(
            f.fake.stats.lock().unwrap().prepares.len(),
            1,
            "stop/time budget is checked before another preparation"
        );
        assert_eq!(
            f.store
                .jobs()
                .unwrap()
                .iter()
                .filter(|job| job["status"] == "queued")
                .count(),
            2
        );
        assert_eq!(f.fake.submission_count(), 0);
        f.fake.assert_serial();
    }
}

#[test]
fn independent_prepare_failure_and_concurrent_edit_do_not_block_or_overwrite_other_units() {
    let f = Fixture::new(4, Duration::ZERO);
    let failed = f.queue_review_video(0, "review-fails");
    let edited = f.queue_review_video(1, "review-edited");
    let good = [
        f.queue_review_video(2, "review-good-2"),
        f.queue_review_video(3, "review-good-3"),
    ];
    let store = f.store.clone();
    *f.fake.prepare_hook.lock().unwrap() = Some(Box::new(move |request, _| {
        if request.request_id == "review-fails" {
            return Err(ProviderError::new(
                "MOCK_PREPARE_FAILED",
                "offline independent prepare failure",
            ));
        }
        if request.request_id == "review-edited" {
            let mut unit = store.get("units", "unit-1").unwrap();
            unit["prompt"] = json!("离线期间明确编辑后的镜头内容。");
            store.save_unit(unit).unwrap();
            return Err(ProviderError::new(
                "MOCK_PREPARE_FAILED",
                "old preparation returned after unit edit",
            ));
        }
        Ok(())
    }));
    f.one_tick();
    assert_eq!(f.store.job(&failed).unwrap()["status"], "failed");
    assert_eq!(
        f.store.job(&edited).unwrap()["status"],
        "stale",
        "from-state CAS must preserve the concurrent edit's terminal state"
    );
    for key in good {
        assert_eq!(f.store.job(&key).unwrap()["status"], "ready");
    }
    assert_eq!(f.fake.submission_count(), 0);
    f.one_tick();
    assert_eq!(
        f.fake.stats.lock().unwrap().prepares.len(),
        4,
        "failed/stale/manual-ready work is not retried automatically"
    );
    f.fake.assert_serial();
}

#[test]
fn review_burst_excludes_future_unknown_and_saved_task_id_without_changing_original_receipts() {
    let f = Fixture::new(4, Duration::ZERO);
    let good = f.queue_review_video(0, "only-review-now");
    let future = f.queue_review_video(1, "future-review");
    f.store
        .update_job(
            &future,
            "queued",
            &json!({"nextAttemptAt":chrono::Utc::now().timestamp()+3600}),
        )
        .unwrap();
    let unknown = f.queue_review_video(2, "unknown-review");
    f.store
        .update_job(
            &unknown,
            "submission_unknown",
            &json!({"error":"fixture unknown acceptance"}),
        )
        .unwrap();
    let known = f.queue_review_video(3, "legacy-known-review");
    f.store.update_job(&known, "ready", &json!({"providerTaskId":"remote:forever-processing","receipt":{"taskId":"remote:forever-processing","original":true}})).unwrap();
    let before_future = f.store.job(&future).unwrap();
    let before_unknown = f.store.job(&unknown).unwrap();
    let before_known = f.store.job(&known).unwrap();
    f.one_tick();
    f.one_tick();
    assert_eq!(f.store.job(&good).unwrap()["status"], "ready");
    assert_eq!(f.store.job(&future).unwrap(), before_future);
    assert_eq!(f.store.job(&unknown).unwrap(), before_unknown);
    let after_known = f.store.job(&known).unwrap();
    assert_eq!(after_known["status"], "processing", "saved-ID legacy ready/autoSubmit=false remains query-only, never failed/requeued for generation");
    for field in ["providerTaskId", "receipt", "snapshot"] {
        assert_eq!(after_known[field], before_known[field], "{field}");
    }
    assert_eq!(f.fake.stats.lock().unwrap().prepares.len(), 1);
    assert_eq!(f.fake.submission_count(), 0);
    f.fake.assert_serial();
}

#[test]
fn a_known_or_unknown_submission_discovered_during_preparation_keeps_its_identity_and_state() {
    for known in [false, true] {
        let f = Fixture::new(1, Duration::ZERO);
        let key = f.queue_review_video(0, "identity-arrives-during-prepare");
        let store = f.store.clone();
        let job_key = key.clone();
        *f.fake.prepare_hook.lock().unwrap() = Some(Box::new(move |_, _| {
            if known {
                store.update_job(&job_key, "preparing", &json!({"providerTaskId":"remote:original-backfill","receipt":{"taskId":"remote:original-backfill","original":true}})).unwrap();
            } else {
                store
                    .update_job(
                        &job_key,
                        "submission_unknown",
                        &json!({"error":"fixture original acceptance unknown"}),
                    )
                    .unwrap();
            }
            Err(ProviderError::new(
                "MOCK_LATE_PREPARE_ERROR",
                "old preparation must not overwrite submission identity",
            ))
        }));
        f.one_tick();
        let after = f.store.job(&key).unwrap();
        if known {
            assert_eq!(after["status"], "preparing");
            assert_eq!(after["providerTaskId"], "remote:original-backfill");
            assert_eq!(
                after["receipt"],
                json!({"taskId":"remote:original-backfill","original":true})
            );
        } else {
            assert_eq!(after["status"], "submission_unknown");
            assert!(s(&after, "providerTaskId").is_empty());
            assert!(s(&after, "error").contains("original acceptance unknown"));
        }
        assert_eq!(f.fake.submission_count(), 0);
    }
}

#[test]
fn prepare_only_has_no_submit_route_even_if_auto_submit_changes_during_the_provider_call() {
    let f = Fixture::new(1, Duration::ZERO);
    let key = f.queue_review_video(0, "no-submit-route");
    let store = f.store.clone();
    let job_key = key.clone();
    *f.fake.prepare_hook.lock().unwrap() = Some(Box::new(move |_, _| {
        store.conn().unwrap().execute("UPDATE jobs SET snapshot=json_set(snapshot,'$.autoSubmit',json('true')) WHERE id=?1", [&job_key]).unwrap();
        Ok(())
    }));
    let (cfg, account) = f.engine.config(&f.store, "mock").unwrap();
    tasks::prepare_only(&f.store, &key, &cfg, &account, f.fake.as_ref()).unwrap();
    assert_eq!(f.store.job(&key).unwrap()["status"], "ready");
    assert_eq!(f.fake.submission_count(), 0);
}

#[test]
fn malformed_prepared_and_sync_receipt_reach_typed_terminal_states_without_resubmission() {
    let f = Fixture::new(1, Duration::ZERO);
    let malformed = f.queue_video(0, "malformed-prepared");
    f.store
        .update_job(&malformed, "ready", &json!({"prepared":{"broken":true}}))
        .unwrap();
    let sync = f.known_image("malformed-sync", "sync:offline-original", "succeeded");
    let before_prepared = f.store.job(&malformed).unwrap();
    let before_sync = f.store.job(&sync).unwrap();
    for _ in 0..4 {
        f.one_tick();
    }
    let after_prepared = f.store.job(&malformed).unwrap();
    let after_sync = f.store.job(&sync).unwrap();
    assert_eq!(after_prepared["status"], "failed");
    assert_eq!(after_sync["status"], "failed");
    assert_eq!(after_prepared["snapshot"], before_prepared["snapshot"]);
    for field in ["providerTaskId", "receipt", "snapshot"] {
        assert_eq!(after_sync[field], before_sync[field]);
    }
    let codes = f
        .store
        .conn()
        .unwrap()
        .prepare(
            "SELECT json_extract(data,'$.code') FROM events WHERE type='local_error' ORDER BY id",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(
        codes.len(),
        2,
        "deterministic local errors must not recur once per scheduler tick"
    );
    assert!(codes.contains(&"LOCAL_PREPARED_CONTRACT".to_owned()));
    assert!(codes.contains(&"LOCAL_SYNC_RECEIPT_CONTRACT".to_owned()));
    assert_eq!(f.fake.submission_count(), 0);
    assert!(f.fake.stats.lock().unwrap().prepares.is_empty());
    assert!(f.fake.stats.lock().unwrap().queries.is_empty());
}

#[test]
fn failed_local_ready_persistence_preserves_upload_receipts_and_other_preparations_continue() {
    let f = Fixture::new(2, Duration::ZERO);
    let bad = f.queue_review_video(0, "ready-write-fails");
    let good = f.queue_review_video(1, "ready-write-good");
    f.store.conn().unwrap().execute_batch("CREATE TRIGGER reject_fixture_ready BEFORE UPDATE OF state ON jobs WHEN json_extract(OLD.snapshot,'$.requestId')='ready-write-fails' AND NEW.state='ready' BEGIN SELECT RAISE(ABORT,'offline ready persistence failure'); END;").unwrap();
    *f.fake.prepare_hook.lock().unwrap() = Some(Box::new(|request, prepared| {
        if request.request_id == "ready-write-fails" {
            prepared.upload_receipts.push(UploadReceipt {
                media_id: "offline-upload".into(),
                sha256: "a".repeat(64),
                order: 0,
                kind: "image".into(),
                url: None,
                file_id: Some("offline-upload-receipt".into()),
                uploaded_at: "test".into(),
                upstream_read_confirmed: false,
            });
        }
        Ok(())
    }));
    f.one_tick();
    let failed = f.store.job(&bad).unwrap();
    assert_eq!(failed["status"], "failed");
    assert!(s(&failed, "error").contains("本地素材准备结果无法保存"));
    assert!(s(&failed, "providerTaskId").is_empty());
    assert!(
        failed["receipt"].is_null(),
        "upload receipts must never become generation receipts"
    );
    assert_eq!(
        f.store
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM uploads WHERE job_id=?1",
                [&bad],
                |row| row.get::<_, usize>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(f.store.job(&good).unwrap()["status"], "ready");
    f.one_tick();
    assert_eq!(f.fake.stats.lock().unwrap().prepares.len(), 2);
    assert_eq!(f.fake.submission_count(), 0);
}

#[test]
fn unable_to_persist_terminal_state_reports_the_storage_failure_without_fake_success() {
    let f = Fixture::new(1, Duration::ZERO);
    let key = f.queue_review_video(0, "all-state-writes-fail");
    f.store.conn().unwrap().execute_batch("CREATE TRIGGER reject_fixture_results BEFORE UPDATE OF state ON jobs WHEN json_extract(OLD.snapshot,'$.requestId')='all-state-writes-fail' AND NEW.state IN ('ready','failed') BEGIN SELECT RAISE(ABORT,'offline terminal write failure'); END;").unwrap();
    f.one_tick();
    assert_eq!(
        f.store.job(&key).unwrap()["status"],
        "preparing",
        "an unpersisted terminal state must not be presented as durable"
    );
    let message: String = f.store.conn().unwrap().query_row("SELECT json_extract(data,'$.message') FROM events WHERE job_id=?1 AND type='local_error' ORDER BY id DESC LIMIT 1", [&key], |row| row.get(0)).unwrap();
    assert!(message.contains("终态也未能持久化"));
    f.one_tick();
    assert_eq!(f.fake.stats.lock().unwrap().prepares.len(), 1);
    assert_eq!(f.fake.submission_count(), 0);
}

#[test]
fn fifty_four_older_image_generations_cannot_starve_newer_original_id_download() {
    let f = Fixture::new(1, Duration::from_millis(3));
    for n in 0..54 {
        f.queue_image(&format!("older-{n}"));
    }
    let known = f.known_image("already-submitted", "remote:already-submitted", "submitted");
    f.store.conn().unwrap().execute("UPDATE jobs SET updated_at=CASE WHEN id=?1 THEN '2026-01-02T00:00:00+00:00' ELSE '2026-01-01T00:00:00+00:00' END",[&known]).unwrap();
    let original = f.store.job(&known).unwrap();
    f.one_tick();
    let downloaded = f.store.job(&known).unwrap();
    assert_eq!(
        downloaded["status"], "downloaded",
        "first due slot must recover the saved ID before the older generation backlog"
    );
    assert_eq!(downloaded["providerTaskId"], original["providerTaskId"]);
    assert_eq!(downloaded["snapshot"], original["snapshot"]);
    assert_eq!(downloaded["receipt"], original["receipt"]);
    assert_eq!(downloaded["resultMediaIds"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.fake.submission_count(),
        0,
        "recovery adds no generation submission"
    );
    assert_eq!(f.fake.query_count("remote:already-submitted"), 1);
    assert_eq!(
        f.store
            .jobs()
            .unwrap()
            .iter()
            .filter(|j| s(j, "status") == "queued")
            .count(),
        54
    );
    f.one_tick();
    assert_eq!(
        f.fake.submission_count(),
        1,
        "the next slot still advances a new generation"
    );
    f.fake.assert_serial();
}

#[test]
fn perpetually_processing_original_id_and_generation_backlog_both_get_bounded_slots() {
    let f = Fixture::new(1, Duration::from_millis(3));
    for n in 0..12 {
        f.queue_image(&format!("generation-{n}"));
    }
    let known = f.known_image("pending", "remote:forever-processing", "submitted");
    for round in 0..3 {
        // Explicit fixture clock control: mark this known task due, without
        // touching failed-task backoff or any production database.
        f.store.conn().unwrap().execute("UPDATE jobs SET next_attempt_at=0,updated_at='2026-01-01T00:00:00+00:00' WHERE id=?1",[&known]).unwrap();
        f.one_tick();
        assert_eq!(f.fake.query_count("remote:forever-processing"), round + 1);
        assert_eq!(
            f.fake.submission_count(),
            round,
            "a due original ID gets the recovery turn"
        );
        f.store
            .conn()
            .unwrap()
            .execute("UPDATE jobs SET next_attempt_at=0 WHERE id=?1", [&known])
            .unwrap();
        f.one_tick();
        assert_eq!(
            f.fake.submission_count(),
            round + 1,
            "even an always-due processing task must yield the generation turn"
        );
        assert_eq!(f.fake.query_count("remote:forever-processing"), round + 1);
    }
    assert_eq!(f.store.job(&known).unwrap()["status"], "processing");
    let stats = f.fake.stats.lock().unwrap();
    assert!(stats.submits.values().all(|n| *n == 1));
    drop(stats);
    f.fake.assert_serial();
}

#[test]
fn future_due_query_backoff_and_unknown_submission_are_preserved_by_fair_selection() {
    let f = Fixture::new(1, Duration::from_millis(3));
    let fresh = f.queue_image("fresh");
    let future = f.known_image("future", "remote:future", "query_failed");
    f.store.update_job(&future,"query_failed",&json!({"nextAttemptAt":chrono::Utc::now().timestamp()+3600,"queryAttempts":2,"error":"fixture cooldown"})).unwrap();
    let future_before = f.store.job(&future).unwrap();
    let unknown = f.queue_image("unknown");
    f.store
        .update_job(
            &unknown,
            "submission_unknown",
            &json!({"error":"original acceptance unknown"}),
        )
        .unwrap();
    let unknown_before = f.store.job(&unknown).unwrap();
    let notfound = f.known_image("not-found", "remote:not-found", "submitted");
    let original = f.store.job(&notfound).unwrap();
    f.one_tick();
    let failed = f.store.job(&notfound).unwrap();
    assert_eq!(
        failed["status"], "query_failed",
        "a missing-task/ordinary CLI error is retryable, never inferred expired"
    );
    assert_eq!(failed["queryAttempts"], 1);
    assert!(failed["nextAttemptAt"].as_i64().unwrap() > chrono::Utc::now().timestamp());
    for field in ["providerTaskId", "snapshot", "receipt"] {
        assert_eq!(failed[field], original[field]);
    }
    f.one_tick();
    assert_eq!(f.store.job(&fresh).unwrap()["status"], "submitted");
    assert_eq!(
        f.store.job(&future).unwrap(),
        future_before,
        "an alternate turn cannot bypass due/backoff"
    );
    assert_eq!(
        f.store.job(&unknown).unwrap(),
        unknown_before,
        "unknown submits never enter either automatic class"
    );
    assert_eq!(f.fake.query_count("remote:future"), 0);
    assert_eq!(f.fake.query_count("remote:not-found"), 1);
    assert_eq!(f.fake.submission_count(), 1);
    assert!(!f
        .fake
        .stats
        .lock()
        .unwrap()
        .prepares
        .contains_key("unknown"));
    f.fake.assert_serial();
}

#[test]
fn a_saved_id_in_legacy_queued_state_is_query_only_and_never_submitted_again() {
    let f = Fixture::new(1, Duration::from_millis(3));
    let key = f.known_image("legacy-local-state", "remote:legacy-local-state", "queued");
    let original = f.store.job(&key).unwrap();
    f.one_tick();
    let recovered = f.store.job(&key).unwrap();
    assert_eq!(recovered["status"], "downloaded");
    for field in ["providerTaskId", "snapshot", "receipt"] {
        assert_eq!(recovered[field], original[field]);
    }
    assert_eq!(f.fake.submission_count(), 0);
    assert_eq!(f.fake.stats.lock().unwrap().prepares.len(), 0);
    assert_eq!(f.fake.query_count("remote:legacy-local-state"), 1);
    f.fake.assert_serial();
}

// Ordinary successful polling is made immediately due to keep the test short. Failed polling keeps
// the production backoff, so an unavailable original task cannot monopolize sibling jobs.
fn make_successes_due(store: &Store) {
    store.conn().unwrap().execute("UPDATE jobs SET next_attempt_at=0 WHERE state IN ('submitted','processing','succeeded')",[]).unwrap();
}

fn pump_until(engine: &Arc<Engine>, store: &Store, limit: Duration, done: impl Fn() -> bool) {
    let started = Instant::now();
    for _ in 0..3000 {
        if done() {
            return;
        }
        assert!(
            started.elapsed() < limit,
            "actual scheduler did not reach the expected state within {limit:?}; jobs: {:?}",
            store
                .jobs()
                .unwrap()
                .iter()
                .map(|j| (&j["entityId"], &j["status"], &j["error"]))
                .collect::<Vec<_>>()
        );
        make_successes_due(store);
        engine.tick().unwrap();
        thread::sleep(Duration::from_millis(5));
    }
    panic!("bounded scheduler iteration count exhausted");
}

#[test]
fn eighty_real_episode_jobs_keep_local_failures_independent_and_never_resubmit_on_restart() {
    let f = Fixture::new(80, Duration::from_millis(3));
    let unavailable = f.queue_video(0, "episode-query-fails");
    let unknown = f.queue_video(1, "episode-unknown");
    for n in 2..80 {
        f.queue_video_on(
            n,
            &format!("episode-{n}"),
            if n < 40 { "mock" } else { "mock-second" },
        );
    }
    pump_until(&f.engine, &f.store, Duration::from_secs(40), || {
        let jobs = f.store.jobs().unwrap();
        jobs.iter()
            .filter(|j| s(j, "status") == "downloaded")
            .count()
            == 78
            && s(&f.store.job(&unavailable).unwrap(), "status") == "query_failed"
            && s(&f.store.job(&unknown).unwrap(), "status") == "submission_unknown"
            && f.fake.idle()
    });
    assert_eq!(
        f.fake.submission_count(),
        80,
        "each unit has one explicit original generation attempt, including the unknown result"
    );
    let stats = f.fake.stats.lock().unwrap();
    assert_eq!(stats.submits.len(), 80);
    assert!(stats.submits.values().all(|n| *n == 1));
    assert_eq!(
        stats.remote_kinds.len(),
        79,
        "known receipts must have distinct IDs, not one shared fake task"
    );
    assert!(
        stats.peak.contains_key("mock") && stats.peak.contains_key("mock-second"),
        "both independent channels must actually execute"
    );
    drop(stats);
    assert!(f.fake.query_count("remote:episode-query-fails") > 0);
    f.fake.assert_serial();
    // A plain repeat preparation must not bypass an unresolved original generation.
    let old_unknown = f.store.job(&unknown).unwrap();
    assert!(f
        .store
        .add_job(
            "video",
            "unit-1",
            "mock",
            s(&old_unknown, "accountId"),
            &old_unknown["snapshot"],
            "queued"
        )
        .is_err());

    let recovered = Engine::with_channels(f.engine.home.clone(), f.fake.clone()).unwrap();
    let before_queries = f.fake.query_count("remote:episode-query-fails");
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET next_attempt_at=0 WHERE id=?1",
            [&unavailable],
        )
        .unwrap();
    pump_until(&recovered, &f.store, Duration::from_secs(5), || {
        f.fake.query_count("remote:episode-query-fails") > before_queries && f.fake.idle()
    });
    for _ in 0..10 {
        recovered.tick().unwrap();
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        f.fake.submission_count(),
        80,
        "restart/resume must add zero generation submissions"
    );
    assert_eq!(
        s(&f.store.job(&unknown).unwrap(), "status"),
        "submission_unknown"
    );
    f.fake.assert_serial();
}

#[test]
fn tick_and_manual_query_share_the_same_channel_and_task_exclusion() {
    let f = Fixture::new(1, Duration::from_millis(200));
    let job = f.queue_video(0, "overlap-check");
    // Start one generation operation, then wait without another scheduler tick. This guarantees
    // a known durable receipt before the overlap phase rather than racing past it into download.
    f.engine.tick().unwrap();
    let started = Instant::now();
    while s(&f.store.job(&job).unwrap(), "providerTaskId").is_empty() {
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(f.fake.query_count("remote:overlap-check"), 0);
    make_successes_due(&f.store);
    let e = f.engine.clone();
    let tick = thread::spawn(move || e.tick().unwrap());
    let start = Instant::now();
    while f.fake.query_count("remote:overlap-check") == 0 {
        assert!(start.elapsed() < Duration::from_secs(3));
        f.engine.tick().unwrap();
        thread::sleep(Duration::from_millis(1));
    }
    // Deliberately request a manual query while the background query is still inside the channel.
    let manual = f
        .engine
        .dispatch(json!({"action":"jobs.query","projectId":f.project_id,"jobId":job}));
    if let Err(error) = manual {
        assert!(
            error.contains("执行") || error.contains("运行"),
            "unexpected manual-query failure: {error}"
        );
    }
    let competitors: Vec<_> = (0..4)
        .map(|_| {
            let e = f.engine.clone();
            thread::spawn(move || e.tick().unwrap())
        })
        .collect();
    tick.join().unwrap();
    for t in competitors {
        t.join().unwrap();
    }
    pump_until(&f.engine, &f.store, Duration::from_secs(5), || {
        s(&f.store.job(&job).unwrap(), "status") == "downloaded" && f.fake.idle()
    });
    assert_eq!(f.fake.submission_count(), 1);
    f.fake.assert_serial();
}

#[test]
fn a_derived_image_waits_for_every_adopted_parent_then_freezes_the_ordered_references() {
    let f = Fixture::new(1, Duration::from_millis(3));
    let source = f.store.source().unwrap();
    let row = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| s(r, "kind") == "body")
        .unwrap();
    for aid in ["parent-a", "parent-b", "child"] {
        let mut asset = json!({"id":aid,"baseAssetId":"subject","variantId":aid,"name":aid,"kind":"CH","prompt":format!("{aid}人物设定图"),"sourceRefs":[row["id"]],"applicability":{"episodes":[source["episodes"][0]["id"]],"scenes":[source["scenes"][0]["id"]]}});
        if aid == "child" {
            asset["sourceAssetId"] = json!("parent-a");
            asset["design"] = json!({"referenceAssetIds":["parent-a","parent-b"]});
        }
        f.store.save_asset(asset).unwrap();
    }
    // Legacy waiting records can have no pinned parent jobs and be satisfied
    // by explicit image imports. New queue closure/ancestor submission is
    // covered separately by engine::image_scheduler_tests.
    let asset = f.store.active_asset("child").unwrap();
    let snapshot = json!({"requestId":"legacy-manual-parents","kind":"image","assetId":"child","variantId":"child","assetVersion":asset["version"],"model":"mock-image","prompt":asset["prompt"],"ratio":"16:9","resolution":"2K","references":[],"dependencyJobIds":[],"options":{},"autoSubmit":true});
    let (_, account) = f.engine.config(&f.store, "mock").unwrap();
    let key = f.store.add_job("image", "child", "mock", &account, &snapshot, "waiting_dependency").unwrap();
    let job = f.store.job(&key).unwrap();
    let job_id = s(&job, "id").to_string();
    let request_id = s(&job["snapshot"], "requestId").to_string();
    assert_eq!(s(&job, "status"), "waiting_dependency");
    for _ in 0..4 {
        f.engine.tick().unwrap();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f
        .fake
        .stats
        .lock()
        .unwrap()
        .prepares
        .contains_key(&request_id));

    let mut adopted = Vec::new();
    for (index, aid) in ["parent-a", "parent-b"].iter().enumerate() {
        let path = f.store.root.join(format!("{aid}.png"));
        image::RgbImage::from_pixel(2, 2, image::Rgb([index as u8 * 100, 19, 31]))
            .save(&path)
            .unwrap();
        let media = f
            .store
            .import_media(&path, "image", Some(aid), Some(aid))
            .unwrap();
        let mid = s(&media, "id").to_string();
        f.store.adopt(aid, aid, &mid).unwrap();
        adopted.push(mid);
        if index == 0 {
            for _ in 0..4 {
                f.engine.tick().unwrap();
                thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                s(&f.store.job(&job_id).unwrap(), "status"),
                "waiting_dependency",
                "adopting only the primary parent must not bypass the second dependency"
            );
            assert!(!f
                .fake
                .stats
                .lock()
                .unwrap()
                .prepares
                .contains_key(&request_id));
        }
    }
    pump_until(&f.engine, &f.store, Duration::from_secs(5), || {
        s(&f.store.job(&job_id).unwrap(), "status") == "downloaded" && f.fake.idle()
    });
    let stats = f.fake.stats.lock().unwrap();
    let prepared = stats.prepares.get(&request_id).unwrap();
    assert_eq!(
        prepared
            .references
            .iter()
            .map(|r| r.media_id.clone())
            .collect::<Vec<_>>(),
        adopted
    );
    assert_eq!(
        prepared
            .references
            .iter()
            .map(|r| r.order)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(stats.submits.get(&request_id), Some(&1));
    drop(stats);
    f.fake.assert_serial();
    let completed = f.store.job(&job_id).unwrap();
    assert_eq!(
        f.store.adopted("child", "child").unwrap().as_deref(),
        completed["resultMediaIds"][0].as_str(),
        "a completed image automatically becomes the child's current reference"
    );
}
