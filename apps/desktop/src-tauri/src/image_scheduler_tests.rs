//! Offline acceptance for the bounded Dreamina image lanes. This is an Engine child
//! module so the authorization reservation can be tested without invoking a CLI.
use super::*;
use crate::providers::{
    DownloadReceipt, GenerationRequest, PreparedRequest, QueryResult, SubmitReceipt,
};
use std::{collections::BTreeMap, thread};
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(8);

#[test]
fn failed_chrome_original_task_cannot_be_bypassed_by_batch_or_provider_switch() {
    let f = Fixture::new();
    f.asset("original-chrome", None);
    let key=f.store.add_job("image","original-chrome","chrome","original-account",&json!({"frozenOriginal":true}),"failed").unwrap();
    f.store.update_job(&key,"failed",&json!({"providerTaskId":"original-chrome-task","receipt":{"original":true}})).unwrap();
    let before=f.raw(&key);
    let result=f.queue_assets(&["original-chrome".into()]);
    assert_eq!(result["queuedCount"],0);
    assert_eq!(result["reusedCount"],1);
    assert_eq!(result["reused"][0]["jobId"],key);
    assert_eq!(f.raw(&key),before);
    assert_eq!(f.channels.total_submits(),0);
}

#[derive(Default)]
struct Gate {
    held: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn hold(&self) {
        *self.held.lock().unwrap() = true;
    }
    fn release(&self) {
        *self.held.lock().unwrap() = false;
        self.changed.notify_all();
    }
    fn wait(&self) -> ProviderResult<()> {
        let guard = self.held.lock().unwrap();
        let (guard, timeout) = self
            .changed
            .wait_timeout_while(guard, WAIT * 3, |held| *held)
            .unwrap();
        if timeout.timed_out() && *guard {
            Err(ProviderError::new(
                "OFFLINE_GATE_TIMEOUT",
                "bounded test gate expired",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Counts {
    active: BTreeMap<String, usize>,
    peak: BTreeMap<String, usize>,
    prepares: BTreeMap<String, Vec<GenerationRequest>>,
    configurations: BTreeMap<String, Vec<Value>>,
    submits: BTreeMap<String, usize>,
    submit_kinds: BTreeMap<String, usize>,
    queries: BTreeMap<String, usize>,
    downloads: BTreeMap<String, usize>,
}
#[derive(Default)]
struct ImageChannels {
    counts: Mutex<Counts>,
    successful_queries: Mutex<BTreeMap<String, String>>,
    submit_gate: Gate,
    query_gate: Gate,
    slow_download_gate: Gate,
}
struct Call<'a> {
    channels: &'a ImageChannels,
    lane: &'static str,
}
impl Drop for Call<'_> {
    fn drop(&mut self) {
        *self
            .channels
            .counts
            .lock()
            .unwrap()
            .active
            .get_mut(self.lane)
            .unwrap() -= 1;
    }
}
impl ImageChannels {
    fn record_config(&self, lane: &str, key: &str, config: &ProviderConfig) {
        self.counts
            .lock()
            .unwrap()
            .configurations
            .entry(format!("{lane}:{key}"))
            .or_default()
            .push(serde_json::to_value(config).unwrap());
    }
    fn enter(&self, lane: &'static str) -> Call<'_> {
        let mut counts = self.counts.lock().unwrap();
        let active = counts.active.entry(lane.into()).or_default();
        *active += 1;
        let current = *active;
        let peak = counts.peak.entry(lane.into()).or_default();
        *peak = (*peak).max(current);
        Call {
            channels: self,
            lane,
        }
    }
    fn submit_count(&self, request: &str) -> usize {
        self.counts
            .lock()
            .unwrap()
            .submits
            .get(request)
            .copied()
            .unwrap_or(0)
    }
    fn total_submits(&self) -> usize {
        self.counts.lock().unwrap().submits.values().sum()
    }
    fn query_count(&self, task: &str) -> usize {
        self.counts
            .lock()
            .unwrap()
            .queries
            .get(task)
            .copied()
            .unwrap_or(0)
    }
    fn active(&self, lane: &str) -> usize {
        self.counts
            .lock()
            .unwrap()
            .active
            .get(lane)
            .copied()
            .unwrap_or(0)
    }
    fn release_all(&self) {
        self.submit_gate.release();
        self.query_gate.release();
        self.slow_download_gate.release();
    }
}
impl Channels for ImageChannels {
    fn capabilities(&self, cfg: &ProviderConfig) -> ProviderResult<Capabilities> {
        assert_eq!(
            cfg.kind, "dreamina",
            "fixture must stay on the simulated channel"
        );
        let models = ["image", "video"].map(|kind| {
            json!({
                "id":format!("offline-{kind}"),"kind":kind,"adapted":true,
                "minDuration":if kind=="video"{json!(4)}else{Value::Null},
                "maxDuration":if kind=="video"{json!(15)}else{Value::Null},
                "allowedDurations":[],"ratios":["16:9"],"resolutions":["2K","720p"],
                "images":9,"videos":3,"audios":3,"totalReferences":12,"requiredImages":0,
                "price":null,"billing":null,"currency":null,"videoPriceMultiplier":null,"raw":{}
            })
        });
        Ok(serde_json::from_value(
            json!({"provider":"dreamina","version":"offline-image-scheduler/v1",
            "source":"in-memory Channels","checkedAt":now(),"models":models,
            "canCancel":false,"canReconcileUnknown":false,"raw":{}}),
        )
        .unwrap())
    }
    fn prepare(
        &self,
        cfg: &ProviderConfig,
        request: &GenerationRequest,
    ) -> ProviderResult<PreparedRequest> {
        assert_eq!(cfg.kind, "dreamina");
        let _call = self.enter("prepare");
        self.record_config("prepare", &request.request_id, cfg);
        self.counts
            .lock()
            .unwrap()
            .prepares
            .entry(request.request_id.clone())
            .or_default()
            .push(request.clone());
        Ok(PreparedRequest {
            provider: cfg.kind.clone(),
            request: request.clone(),
            capability_version: "offline-image-scheduler/v1".into(),
            upload_receipts: vec![],
            wire: json!({"offline":true}),
            estimated_cost: None,
            prepared_at: now(),
        })
    }
    fn submit(
        &self,
        cfg: &ProviderConfig,
        prepared: &PreparedRequest,
    ) -> ProviderResult<SubmitReceipt> {
        assert_eq!(cfg.kind, "dreamina");
        let _call = self.enter("submit");
        let request = &prepared.request;
        self.record_config("submit", &request.request_id, cfg);
        {
            let mut counts = self.counts.lock().unwrap();
            *counts
                .submits
                .entry(request.request_id.clone())
                .or_default() += 1;
            *counts.submit_kinds.entry(request.kind.clone()).or_default() += 1;
        }
        self.submit_gate.wait()?;
        if request.request_id == "uncertain" {
            return Err(ProviderError::uncertain(
                "offline timeout after possible cloud acceptance",
            ));
        }
        Ok(SubmitReceipt {
            task_id: format!("offline:{}", request.request_id),
            status: "processing".into(),
            result_urls: vec![],
            raw: json!({"requestId":request.request_id,"offline":true}),
        })
    }
    fn query(&self, cfg: &ProviderConfig, task: &str) -> ProviderResult<QueryResult> {
        assert_eq!(cfg.kind, "dreamina");
        let _call = self.enter("query");
        self.record_config("query", task, cfg);
        *self
            .counts
            .lock()
            .unwrap()
            .queries
            .entry(task.into())
            .or_default() += 1;
        self.query_gate.wait()?;
        let output = self.successful_queries.lock().unwrap().get(task).cloned();
        Ok(QueryResult {
            task_id: task.into(),
            status: if output.is_some() {
                "succeeded"
            } else {
                "processing"
            }
            .into(),
            result_urls: output.into_iter().collect(),
            error: None,
            actual_cost: None,
            raw: json!({"offline":true}),
        })
    }
    fn download(
        &self,
        cfg: &ProviderConfig,
        url: &str,
        path: &Path,
    ) -> ProviderResult<DownloadReceipt> {
        assert_eq!(cfg.kind, "dreamina");
        let _call = self.enter("download");
        self.record_config("download", url, cfg);
        *self
            .counts
            .lock()
            .unwrap()
            .downloads
            .entry(url.into())
            .or_default() += 1;
        if url.contains("slow-") {
            self.slow_download_gate.wait()?;
        }
        // Distinct fake outputs prevent Store's content-addressed media deduplication
        // from making every appearance accidentally share the root's image identity.
        let mut pixels = crate::pipeline::digest(url).into_bytes();
        pixels.resize(8 * 3 * 3, 0);
        image::RgbImage::from_raw(8, 3, pixels)
            .unwrap()
            .save(path)
            .map_err(|e| ProviderError::new("OFFLINE_PNG", &e.to_string()))?;
        let (sha256, bytes) =
            crate::store::hash_file(path).map_err(|e| ProviderError::new("OFFLINE_HASH", &e))?;
        Ok(DownloadReceipt {
            path: path.into(),
            sha256,
            bytes,
            mime: Some("image/png".into()),
        })
    }
    fn reconcile(&self, _: &ProviderConfig, _: &str) -> ProviderResult<QueryResult> {
        panic!("background recovery must not guess an unknown submission")
    }
}

struct Fixture {
    _temp: TempDir,
    engine: Arc<Engine>,
    store: Store,
    channels: Arc<ImageChannels>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let channels = Arc::new(ImageChannels::default());
        let engine = Engine::with_channels(temp.path().join("home"), channels.clone()).unwrap();
        let project = engine
            .dispatch(json!({"action":"projects.create","title":"offline image lanes"}))
            .unwrap();
        let store = engine.store(s(&project, "id")).unwrap();
        let source = pipeline::prepare_source("第一集\n场景：测试门口\n△人物抬头看门。\n").unwrap();
        assert!(source.diagnostics.is_empty());
        let row = source.rows.iter().find(|row| row.kind == "body").unwrap();
        store
            .save_source(serde_json::to_value(&source).unwrap())
            .unwrap();
        store
            .save_unit(json!({"id":"review-unit","episodeId":source.episodes[0].id,
            "sceneId":source.scenes[0].id,"sourceRefs":[row.id],"duration":5,
            "prompt":"门口人物抬头看门，独立镜头。","assetRefs":[]}))
            .unwrap();
        store.save_settings(&json!({"dreaminaPath":"offline-never-executed-cli",
            "dreaminaAccountId":"offline-account","trialApproved":true,"ratio":"16:9",
            "imageProductionPolicy":"automatic","ffprobePath":temp.path().join("unused-ffprobe.exe")})).unwrap();
        // Bypass only scheduled catalog/backup housekeeping; all media operations still
        // execute the real Engine, task CAS, snapshot validation and Store code.
        for key in [
            "catalog_attempt:dreamina",
            "catalog_attempt:bh",
            "catalog_attempt:chrome",
            "backed_up_at",
        ] {
            store.conn().unwrap().execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                rusqlite::params![key,now()]).unwrap();
        }
        Self {
            _temp: temp,
            engine,
            store,
            channels,
        }
    }
    fn asset(&self, asset: &str, parent: Option<&str>) {
        let mut value = json!({"id":asset,"baseAssetId":"offline-subject","variantId":asset,
            "name":asset,"kind":"CH","prompt":"离线人物设定图",
            "preparation":{"mode":if parent.is_some(){"DERIVE"}else{"REFERENCE"}}});
        if let Some(parent) = parent {
            value["sourceAssetId"] = json!(parent);
        }
        self.store.save_asset(value).unwrap();
    }
    fn image_job(
        &self,
        asset: &str,
        request: &str,
        state: &str,
        dependencies: &[String],
    ) -> String {
        let appearance = self.store.active_asset(asset).unwrap();
        let snapshot = json!({"requestId":request,"kind":"image","provider":"dreamina",
            "assetId":asset,"variantId":appearance["variantId"],"assetVersion":appearance["version"],
            "sourceVersion":self.store.source().unwrap()["version"],"model":"offline-image",
            "prompt":appearance["prompt"],"ratio":"16:9","resolution":"2K","references":[],
            "dependencyJobIds":dependencies,"options":{},"autoSubmit":true});
        binding::validate_generation_snapshot(&self.store, &snapshot).unwrap();
        let (_, account) = self.engine.config(&self.store, "dreamina").unwrap();
        self.store
            .add_job("image", asset, "dreamina", &account, &snapshot, state)
            .unwrap()
    }
    fn base(&self, name: &str) -> String {
        self.asset(name, None);
        self.image_job(name, name, "queued", &[])
    }
    fn known(&self, name: &str, state: &str) -> String {
        let key = self.base(name);
        let task = format!("offline:{name}");
        let urls = if state == "succeeded" {
            vec![format!("https://offline.invalid/{name}.png")]
        } else {
            vec![]
        };
        let receipt = SubmitReceipt {
            task_id: task.clone(),
            status: state.into(),
            result_urls: urls.clone(),
            raw: json!({"offline":true}),
        };
        let result = QueryResult {
            task_id: task,
            status: state.into(),
            result_urls: urls,
            error: None,
            actual_cost: None,
            raw: json!({"offline":true}),
        };
        self.store
            .update_job(
                &key,
                state,
                &json!({"providerTaskId":receipt.task_id,"receipt":receipt,"result":result}),
            )
            .unwrap();
        key
    }
    fn review_video(&self) -> String {
        let unit = self.store.get("units", "review-unit").unwrap();
        let mut snapshot = binding::bind(&self.store, &unit, "dreamina", "offline-video").unwrap();
        assert_eq!(snapshot["problems"], json!([]));
        snapshot["requestId"] = json!("review-video");
        snapshot["autoSubmit"] = json!(false);
        let (_, account) = self.engine.config(&self.store, "dreamina").unwrap();
        self.store
            .add_job(
                "video",
                "review-unit",
                "dreamina",
                &account,
                &snapshot,
                "queued",
            )
            .unwrap()
    }
    fn wait(&self, condition: impl Fn() -> bool) {
        let start = Instant::now();
        while !condition() {
            assert!(
                start.elapsed() < WAIT,
                "bounded offline scheduler condition did not become true"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn drain(&self) {
        assert!(
            self.engine.await_channel_shutdown(WAIT.as_secs()),
            "offline workers did not release their owned slots"
        );
    }
    fn raw(&self, key: &str) -> (String, Option<String>, Option<String>) {
        self.store
            .conn()
            .unwrap()
            .query_row(
                "SELECT snapshot,task_id,receipt FROM jobs WHERE id=?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }
    fn queue_assets(&self, selected: &[String]) -> Value {
        self.engine
            .queue_images_with_account_check(
                &self.store,
                &json!({"provider":"dreamina","model":"offline-image","assetIds":selected,
                "imageProductionPolicy":"automatic"}),
                |_, account| Ok(account.to_owned()),
            )
            .unwrap()
    }
    fn successful_job_map(&self) -> BTreeMap<String, Value> {
        let jobs: BTreeMap<_, _> = self
            .store
            .jobs()
            .unwrap()
            .into_iter()
            .map(|job| (s(&job, "entityId").to_owned(), job))
            .collect();
        let mut queries = self.channels.successful_queries.lock().unwrap();
        for (asset, job) in &jobs {
            queries.insert(
                format!("offline:{}", s(&job["snapshot"], "requestId")),
                format!("https://offline.invalid/{asset}.png"),
            );
        }
        jobs
    }
    fn pump(&self, condition: impl Fn() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(
                started.elapsed() < WAIT,
                "bounded fake chain did not advance"
            );
            self.engine.tick().unwrap();
            thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.channels.release_all();
        let _ = self.engine.await_channel_shutdown(WAIT.as_secs());
    }
}

#[test]
fn one_tick_fills_bounded_image_submit_slots_and_concurrent_ticks_cannot_resubmit() {
    let f = Fixture::new();
    let names: Vec<_> = (0..IMAGE_SUBMIT_WORKERS + 2)
        .map(|n| format!("base-{n:02}"))
        .collect();
    let jobs: Vec<_> = names.iter().map(|name| f.base(name)).collect();
    f.channels.submit_gate.hold();
    f.engine.tick().unwrap();
    f.wait(|| f.channels.active("submit") == IMAGE_SUBMIT_WORKERS);
    assert!(
        IMAGE_SUBMIT_WORKERS > 1,
        "image submit lane must allow genuine parallel operations"
    );
    assert_eq!(f.channels.total_submits(), IMAGE_SUBMIT_WORKERS);
    let ticks: Vec<_> = (0..4)
        .map(|_| {
            let e = f.engine.clone();
            thread::spawn(move || e.tick().unwrap())
        })
        .collect();
    for tick in ticks {
        tick.join().unwrap();
    }
    assert_eq!(
        f.channels.total_submits(),
        IMAGE_SUBMIT_WORKERS,
        "busy jobs and slots must stay reserved across repeated tick calls"
    );
    f.channels.submit_gate.release();
    f.drain();
    f.engine.tick().unwrap();
    f.drain();
    let counts = f.channels.counts.lock().unwrap();
    assert_eq!(counts.peak["submit"], IMAGE_SUBMIT_WORKERS);
    for name in &names {
        assert_eq!(counts.submits.get(name), Some(&1));
        assert_eq!(counts.prepares[name].len(), 1);
    }
    drop(counts);
    for (job, name) in jobs.iter().zip(&names) {
        assert_eq!(
            f.store.job(job).unwrap()["providerTaskId"],
            format!("offline:{name}")
        );
    }
}

#[test]
fn slow_download_slots_do_not_block_independent_submit_or_original_id_query() {
    let f = Fixture::new();
    let downloads: Vec<_> = (0..IMAGE_DOWNLOAD_WORKERS + 1)
        .map(|n| f.known(&format!("slow-{n}"), "succeeded"))
        .collect();
    let fresh = f.base("new-during-download");
    let queried = f.known("query-during-download", "submitted");
    let saved = f.raw(&queried);
    f.channels.slow_download_gate.hold();
    f.engine.tick().unwrap();
    f.wait(|| {
        f.channels.active("download") == IMAGE_DOWNLOAD_WORKERS
            && f.channels.submit_count("new-during-download") == 1
            && f.channels.query_count("offline:query-during-download") == 1
    });
    assert_eq!(
        f.channels.counts.lock().unwrap().downloads.len(),
        IMAGE_DOWNLOAD_WORKERS
    );
    assert_eq!(f.channels.submit_count("query-during-download"), 0);
    assert_eq!(
        f.raw(&queried),
        saved,
        "queries cannot rewrite original ID, receipt or generation snapshot"
    );
    f.channels.slow_download_gate.release();
    f.drain();
    f.engine.tick().unwrap();
    f.drain();
    for key in &downloads {
        assert_eq!(f.store.job(key).unwrap()["status"], "downloaded");
    }
    assert_eq!(
        f.store.job(&fresh).unwrap()["providerTaskId"],
        "offline:new-during-download"
    );
    assert_eq!(
        f.channels.counts.lock().unwrap().peak["download"],
        IMAGE_DOWNLOAD_WORKERS
    );
}

#[test]
fn original_id_queries_fill_their_own_slots_without_blocking_image_submission() {
    let f = Fixture::new();
    let names: Vec<_> = (0..IMAGE_QUERY_WORKERS + 1)
        .map(|n| format!("query-{n:02}"))
        .collect();
    for name in &names {
        f.known(name, "submitted");
    }
    f.base("independent-submit");
    f.channels.query_gate.hold();
    f.engine.tick().unwrap();
    f.wait(|| {
        f.channels.active("query") == IMAGE_QUERY_WORKERS
            && f.channels.submit_count("independent-submit") == 1
    });
    let ticks: Vec<_> = (0..3)
        .map(|_| {
            let e = f.engine.clone();
            thread::spawn(move || e.tick().unwrap())
        })
        .collect();
    for tick in ticks {
        tick.join().unwrap();
    }
    assert_eq!(
        f.channels
            .counts
            .lock()
            .unwrap()
            .queries
            .values()
            .sum::<usize>(),
        IMAGE_QUERY_WORKERS
    );
    f.channels.query_gate.release();
    f.drain();
    f.engine.tick().unwrap();
    f.drain();
    let counts = f.channels.counts.lock().unwrap();
    assert_eq!(counts.peak["query"], IMAGE_QUERY_WORKERS);
    for name in &names {
        assert_eq!(counts.queries.get(&format!("offline:{name}")), Some(&1));
    }
    assert_eq!(
        counts.submits.values().sum::<usize>(),
        1,
        "known IDs must never enter the submit lane"
    );
}

#[test]
fn downloaded_parent_unlocks_derived_image_next_tick_while_unrelated_download_is_still_slow() {
    let f = Fixture::new();
    let parent = f.known("parent", "succeeded");
    let unrelated = f.known("slow-unrelated", "succeeded");
    f.asset("child", Some("parent"));
    let child = f.image_job(
        "child",
        "derived-child",
        "waiting_dependency",
        &[parent.clone()],
    );
    f.channels.slow_download_gate.hold();
    f.engine.tick().unwrap();
    f.wait(|| {
        f.channels.active("download") >= 1
            && f.store.job(&parent).unwrap()["status"] == "downloaded"
    });
    assert_eq!(f.store.job(&child).unwrap()["status"], "waiting_dependency");
    let media = f
        .store
        .adopted("parent", "parent")
        .unwrap()
        .expect("completed parent must be adopted atomically");
    assert_eq!(
        f.store.job(&parent).unwrap()["result"]["mediaIds"][0],
        media
    );
    f.engine.tick().unwrap();
    f.wait(|| f.channels.submit_count("derived-child") == 1);
    assert_eq!(
        f.store.job(&unrelated).unwrap()["status"],
        "downloading",
        "unrelated download must remain held while the child progresses"
    );
    let counts = f.channels.counts.lock().unwrap();
    let request = &counts.prepares["derived-child"][0];
    assert_eq!(request.references.len(), 1);
    assert_eq!(request.references[0].media_id, media);
    assert_eq!(request.references[0].order, 0);
    assert_eq!(
        request.references[0].sha256,
        f.store.get("media", &media).unwrap()["sha256"]
    );
    drop(counts);
    f.channels.slow_download_gate.release();
    f.drain();
    assert_eq!(f.channels.submit_count("parent"), 0);
    assert_eq!(f.channels.submit_count("slow-unrelated"), 0);
}

#[test]
fn unknown_submission_and_review_only_video_stay_non_generating_across_ticks_and_restart() {
    let f = Fixture::new();
    let unknown = f.base("uncertain");
    let review = f.review_video();
    for _ in 0..3 {
        f.engine.tick().unwrap();
        f.drain();
    }
    assert_eq!(
        f.store.job(&unknown).unwrap()["status"],
        "submission_unknown"
    );
    assert_eq!(f.store.job(&review).unwrap()["status"], "ready");
    let saved = f.raw(&unknown);
    let recovered = Engine::with_channels(f.engine.home.clone(), f.channels.clone()).unwrap();
    for _ in 0..3 {
        recovered.tick().unwrap();
        assert!(recovered.await_channel_shutdown(WAIT.as_secs()));
    }
    assert_eq!(f.raw(&unknown), saved);
    assert_eq!(f.channels.submit_count("uncertain"), 1);
    let counts = f.channels.counts.lock().unwrap();
    assert_eq!(counts.submit_kinds.get("video").copied().unwrap_or(0), 0);
    assert_eq!(
        counts.prepares["review-video"].len(),
        1,
        "autoSubmit false may prepare once but never generate"
    );
    assert_eq!(f.store.job(&review).unwrap()["providerTaskId"], Value::Null);
}

#[test]
fn exclusive_authorization_and_all_image_lanes_share_the_same_channel_reservation() {
    let f = Fixture::new();
    f.base("auth-exclusion");
    let authorization = f
        .engine
        .acquire_keys(vec!["channel:dreamina".into()])
        .unwrap();
    f.engine.tick().unwrap();
    assert_eq!(f.channels.total_submits(), 0);
    assert!(f.channels.counts.lock().unwrap().prepares.is_empty());
    drop(authorization);
    f.channels.submit_gate.hold();
    f.engine.tick().unwrap();
    f.wait(|| f.channels.active("submit") == 1);
    assert!(
        f.engine
            .acquire_keys(vec!["channel:dreamina".into()])
            .is_err(),
        "authorization cannot race any active image worker"
    );
    let unrelated = f.engine.acquire_keys(vec!["channel:bh".into()]).unwrap();
    drop(unrelated);
    f.channels.submit_gate.release();
    f.drain();
    for lane in ["submit", "query", "download"] {
        let image = f
            .engine
            .acquire_keys(vec![format!("image:dreamina:{lane}:0")])
            .unwrap();
        assert!(f
            .engine
            .acquire_keys(vec!["channel:dreamina".into()])
            .is_err());
        assert!(f
            .engine
            .acquire_keys(vec![format!("image:dreamina:{lane}:0")])
            .is_err());
        drop(image);
        let auth = f
            .engine
            .acquire_keys(vec!["channel:dreamina".into()])
            .unwrap();
        assert!(f
            .engine
            .acquire_keys(vec![format!("image:dreamina:{lane}:0")])
            .is_err());
        drop(auth);
    }
    assert_eq!(f.channels.submit_count("auth-exclusion"), 1);
}

#[test]
fn read_only_image_preflight_retries_twice_with_persisted_delays_but_never_requeues_uncertain_or_known_tasks(
) {
    let f = Fixture::new();
    let queued = f.base("preflight-queued");
    let ready = f.base("preflight-ready");
    f.store.update_job(&ready, "ready", &json!({})).unwrap();
    let error = ProviderError::new("CLI_NETWORK", "offline account inspection unavailable");
    for key in [&queued, &ready] {
        let initial = f.raw(key);
        let original_state = f.store.job(key).unwrap()["status"].clone();
        for attempt in 1..=2 {
            let before = chrono::Utc::now().timestamp();
            Engine::image_preflight_failed(&f.store, key, &error).unwrap();
            let after = chrono::Utc::now().timestamp();
            let job = f.store.job(key).unwrap();
            assert_eq!(job["status"], original_state);
            assert_eq!(job["queryAttempts"], attempt);
            let due = job["nextAttemptAt"].as_i64().unwrap();
            assert!(due >= before + 10 * attempt && due <= after + 10 * attempt);
            assert_eq!(
                f.raw(key),
                initial,
                "preflight retry cannot alter a frozen request or cloud identity"
            );
        }
        Engine::image_preflight_failed(&f.store, key, &error).unwrap();
        let job = f.store.job(key).unwrap();
        assert_eq!(job["status"], "failed");
        assert_eq!(job["queryAttempts"], 2);
        assert_eq!(f.raw(key), initial);
    }
    let unknown = f.base("preflight-unknown");
    f.store
        .update_job(
            &unknown,
            "submission_unknown",
            &json!({"error":"original uncertainty"}),
        )
        .unwrap();
    let known = f.known("preflight-known", "submitted");
    let auth = f.base("preflight-auth");
    let closing = f.base("preflight-closing");
    let unknown_raw = f.raw(&unknown);
    let known_raw = f.raw(&known);
    Engine::image_preflight_failed(&f.store, &unknown, &error).unwrap();
    Engine::image_preflight_failed(&f.store, &known, &error).unwrap();
    Engine::image_preflight_failed(&f.store, &auth, &ProviderError::auth()).unwrap();
    Engine::image_preflight_failed(
        &f.store,
        &closing,
        &ProviderError::new("CLIENT_CLOSING", "offline close"),
    )
    .unwrap();
    assert_eq!(
        f.store.job(&unknown).unwrap()["status"],
        "submission_unknown"
    );
    assert_eq!(f.raw(&unknown), unknown_raw);
    assert_eq!(f.store.job(&known).unwrap()["status"], "query_failed");
    assert_eq!(f.raw(&known), known_raw);
    assert_eq!(f.store.job(&auth).unwrap()["status"], "auth_required");
    assert_eq!(f.store.job(&auth).unwrap()["queryAttempts"], 0);
    assert_eq!(f.store.job(&closing).unwrap()["status"], "queued");
    assert_eq!(f.store.job(&closing).unwrap()["queryAttempts"], 0);
    assert_eq!(
        f.channels.total_submits(),
        0,
        "account inspections never invoke generation"
    );
    assert!(f.channels.counts.lock().unwrap().prepares.is_empty());
}

#[test]
fn eleven_layer_chain_uses_each_direct_parent_and_independent_chain_finishes_while_one_layer_download_waits(
) {
    let f = Fixture::new();
    let main: Vec<String> = (0..11)
        .map(|depth| {
            if depth == 1 {
                "slow-main-01".into()
            } else {
                format!("main-{depth:02}")
            }
        })
        .collect();
    let independent: Vec<String> = (0..3)
        .map(|depth| format!("independent-{depth:02}"))
        .collect();
    for chain in [&main, &independent] {
        for (depth, name) in chain.iter().enumerate() {
            f.asset(
                name,
                depth.checked_sub(1).map(|index| chain[index].as_str()),
            );
        }
    }
    // Selecting only the two leaves must queue all missing direct ancestors.
    let selection = vec![
        main.last().unwrap().clone(),
        independent.last().unwrap().clone(),
    ];
    let queued = f.queue_assets(&selection);
    assert_eq!(queued["queuedCount"], 14);
    assert_eq!(queued["blockedCount"], 0);
    let jobs = f.successful_job_map();
    assert_eq!(jobs.len(), 14);
    for chain in [&main, &independent] {
        for (depth, name) in chain.iter().enumerate() {
            let job = &jobs[name];
            if depth == 0 {
                assert_eq!(job["status"], "queued");
                assert_eq!(job["snapshot"]["dependencyJobIds"], json!([]));
            } else {
                assert_eq!(job["status"], "waiting_dependency");
                assert_eq!(
                    job["snapshot"]["dependencyJobIds"],
                    json!([jobs[&chain[depth - 1]]["id"]]),
                    "the queued dependency must name the direct parent's task, not every ancestor"
                );
            }
        }
    }
    f.channels.slow_download_gate.hold();
    // Both roots follow the actual submit -> original-ID query -> download/adopt path.
    for _ in 0..3 {
        f.engine.tick().unwrap();
        f.drain();
    }
    for chain in [&main, &independent] {
        assert_eq!(
            f.store.job(s(&jobs[&chain[0]], "id")).unwrap()["status"],
            "downloaded"
        );
        assert!(f.store.adopted(&chain[0], &chain[0]).unwrap().is_some());
    }
    // Exactly the next tick after the roots complete starts both direct children.
    f.engine.tick().unwrap();
    f.drain();
    for chain in [&main, &independent] {
        assert_eq!(
            f.channels
                .submit_count(s(&jobs[&chain[1]]["snapshot"], "requestId")),
            1
        );
    }
    f.engine.tick().unwrap();
    f.drain();
    f.engine.tick().unwrap(); // Main depth 1 holds download; independent depth 1 can finish.
    f.wait(|| {
        f.store.job(s(&jobs[&independent[1]], "id")).unwrap()["status"] == "downloaded"
            && f.channels.active("download") == 1
    });
    assert_eq!(
        f.store.job(s(&jobs[&main[1]], "id")).unwrap()["status"],
        "downloading"
    );
    assert_eq!(
        f.store.job(s(&jobs[&main[2]], "id")).unwrap()["status"],
        "waiting_dependency"
    );
    f.engine.tick().unwrap();
    f.wait(|| {
        !s(
            &f.store.job(s(&jobs[&independent[2]], "id")).unwrap(),
            "providerTaskId",
        )
        .is_empty()
    });
    f.pump(|| f.store.job(s(&jobs[&independent[2]], "id")).unwrap()["status"] == "downloaded");
    assert_eq!(
        f.store.job(s(&jobs[&main[1]], "id")).unwrap()["status"],
        "downloading"
    );
    assert_eq!(
        f.channels
            .submit_count(s(&jobs[&main[2]]["snapshot"], "requestId")),
        0,
        "only the affected direct child waits for the held parent"
    );
    f.channels.slow_download_gate.release();
    f.drain();
    assert_eq!(
        f.store.job(s(&jobs[&main[1]], "id")).unwrap()["status"],
        "downloaded"
    );
    // The remaining nine depths each start on the first tick after their immediate
    // parent has downloaded and been adopted. No all-images or root-image barrier.
    for depth in 2..main.len() {
        let job = &jobs[&main[depth]];
        assert_eq!(
            f.store.job(s(job, "id")).unwrap()["status"],
            "waiting_dependency"
        );
        assert!(f
            .store
            .adopted(&main[depth - 1], &main[depth - 1])
            .unwrap()
            .is_some());
        f.engine.tick().unwrap();
        f.drain();
        assert_eq!(
            f.channels.submit_count(s(&job["snapshot"], "requestId")),
            1,
            "the direct child must submit in the next scheduler round"
        );
        f.engine.tick().unwrap();
        f.drain();
        assert_eq!(f.store.job(s(job, "id")).unwrap()["status"], "succeeded");
        f.engine.tick().unwrap();
        f.drain();
        assert_eq!(f.store.job(s(job, "id")).unwrap()["status"], "downloaded");
    }
    let counts = f.channels.counts.lock().unwrap();
    assert_eq!(counts.submits.values().sum::<usize>(), 14);
    assert!(counts.submits.values().all(|count| *count == 1));
    for chain in [&main, &independent] {
        let root_media = f.store.adopted(&chain[0], &chain[0]).unwrap().unwrap();
        for (depth, name) in chain.iter().enumerate() {
            let job = f.store.job(s(&jobs[name], "id")).unwrap();
            let own_media = f.store.adopted(name, name).unwrap().unwrap();
            assert_eq!(job["result"]["mediaIds"][0], own_media);
            let request = &counts.prepares[s(&job["snapshot"], "requestId")][0];
            if depth == 0 {
                assert!(request.references.is_empty());
            } else {
                let parent = &chain[depth - 1];
                let parent_media = f.store.adopted(parent, parent).unwrap().unwrap();
                let media = f.store.get("media", &parent_media).unwrap();
                assert_eq!(request.references.len(), 1);
                assert_eq!(request.references[0].media_id, parent_media);
                assert_eq!(request.references[0].sha256, s(&media, "sha256"));
                assert_eq!(request.references[0].order, 0);
                assert_eq!(job["snapshot"]["references"][0]["assetId"], *parent);
                assert_eq!(job["snapshot"]["references"][0]["variantId"], *parent);
                if depth > 1 {
                    assert_ne!(request.references[0].media_id,root_media,
                    "deep descendants must receive their direct parent's image, not the root baseline");
                }
            }
        }
    }
}

#[test]
fn cyclic_or_missing_parent_blocks_only_that_dependency_family_and_healthy_images_continue() {
    let f = Fixture::new();
    for (asset, parent) in [
        ("cycle-a", Some("cycle-b")),
        ("cycle-b", Some("cycle-a")),
        ("cycle-child", Some("cycle-a")),
        ("missing-a", Some("absent-parent")),
        ("missing-child", Some("missing-a")),
        ("healthy-root", None),
        ("healthy-child", Some("healthy-root")),
    ] {
        f.asset(asset, parent);
    }
    let selected: Vec<String> = [
        "cycle-a",
        "cycle-b",
        "cycle-child",
        "missing-a",
        "missing-child",
        "healthy-root",
        "healthy-child",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let report = f.queue_assets(&selected);
    assert_eq!(report["queuedCount"], 2);
    assert_eq!(report["blockedCount"], 5);
    let blocked: BTreeMap<_, _> = report["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| {
            (
                s(issue, "assetId").to_owned(),
                s(issue, "reason").to_owned(),
            )
        })
        .collect();
    for name in ["cycle-a", "cycle-b", "cycle-child"] {
        assert!(
            blocked[name].contains("循环"),
            "cyclic family must have a specific dependency error"
        );
    }
    for name in ["missing-a", "missing-child"] {
        assert!(
            blocked[name].contains("不存在"),
            "missing-parent family must retain the missing-identity error"
        );
    }
    let jobs = f.successful_job_map();
    assert_eq!(
        jobs.keys().cloned().collect::<Vec<_>>(),
        vec!["healthy-child".to_owned(), "healthy-root".to_owned()]
    );
    f.pump(|| {
        jobs.values()
            .all(|job| f.store.job(s(job, "id")).unwrap()["status"] == "downloaded")
    });
    f.drain();
    for name in ["healthy-root", "healthy-child"] {
        assert!(f.store.adopted(name, name).unwrap().is_some());
        assert_eq!(
            f.channels
                .submit_count(s(&jobs[name]["snapshot"], "requestId")),
            1
        );
    }
    assert_eq!(
        f.channels.total_submits(),
        2,
        "invalid dependency families must not fan out or prevent the independent family"
    );
    let parent_media = f
        .store
        .adopted("healthy-root", "healthy-root")
        .unwrap()
        .unwrap();
    let counts = f.channels.counts.lock().unwrap();
    let child = &counts.prepares[s(&jobs["healthy-child"]["snapshot"], "requestId")][0];
    assert_eq!(child.references.len(), 1);
    assert_eq!(child.references[0].media_id, parent_media);
}

#[test]
fn selecting_leaf_with_a_promptless_ancestor_reports_local_failures_without_orphan_waiting_jobs() {
    let f = Fixture::new();
    f.asset("promptless-parent", None);
    let mut parent = f.store.active_asset("promptless-parent").unwrap();
    parent["prompt"] = json!("");
    f.store.save_asset(parent).unwrap();
    f.asset("prompt-child", Some("promptless-parent"));
    f.asset("prompt-grandchild", Some("prompt-child"));
    f.asset("healthy-separate", None);
    let report = f.queue_assets(&["prompt-grandchild".into(), "healthy-separate".into()]);
    assert_eq!(report["queuedCount"], 1);
    assert_eq!(report["blockedCount"], 3);
    let blocked: BTreeMap<_, _> = report["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| {
            (
                s(issue, "assetId").to_owned(),
                s(issue, "reason").to_owned(),
            )
        })
        .collect();
    assert!(blocked["promptless-parent"].contains("缺少图片设计提示词"));
    for name in ["prompt-child", "prompt-grandchild"] {
        assert!(
            blocked[name].contains("没有当前图片且未能排入生成队列"),
            "descendants need a specific upstream failure instead of an orphan waiting state"
        );
    }
    let jobs = f.successful_job_map();
    assert_eq!(jobs.len(), 1);
    assert!(jobs.contains_key("healthy-separate"));
    assert!(f
        .store
        .pending_jobs()
        .unwrap()
        .iter()
        .all(|job| s(job, "status") != "waiting_dependency"));
    f.pump(|| f.store.job(s(&jobs["healthy-separate"], "id")).unwrap()["status"] == "downloaded");
    f.drain();
    assert_eq!(f.channels.total_submits(), 1);
    assert_eq!(
        f.store.jobs().unwrap().len(),
        1,
        "scheduler ticks cannot create jobs for a blocked dependency family"
    );
    assert!(f
        .store
        .adopted("healthy-separate", "healthy-separate")
        .unwrap()
        .is_some());
}

#[test]
fn invalid_second_reference_rolls_back_partial_ancestors_without_discarding_valid_selections() {
    let f = Fixture::new();
    f.asset("partial-root", None);
    f.asset("bad-child", Some("partial-root"));
    let mut bad = f.store.active_asset("bad-child").unwrap();
    bad["design"] = json!({"referenceAssetIds":["missing-reference"]});
    f.store.save_asset(bad).unwrap();
    f.asset("healthy-selection", None);
    let request = json!({"provider":"dreamina","model":"offline-image",
        "assetIds":["bad-child"],"imageProductionPolicy":"automatic"});
    let error = f
        .engine
        .queue_images_with_account_check(&f.store, &request, |_, account| Ok(account.to_owned()))
        .unwrap_err();
    assert!(error.contains("missing-reference") && error.contains("不存在"));
    assert!(
        f.store.jobs().unwrap().is_empty(),
        "visiting a valid first parent must not leak billable work when a later reference fails"
    );
    f.engine.tick().unwrap();
    f.drain();
    assert_eq!(f.channels.total_submits(), 0);
    assert!(f.channels.counts.lock().unwrap().prepares.is_empty());

    // An existing reference with a broken parent fails only after the valid
    // direct parent was traversed; this exercises the order/done rollback itself.
    f.asset("broken-reference", Some("missing-reference"));
    f.asset("bad-via-nested-reference", Some("partial-root"));
    let mut nested = f.store.active_asset("bad-via-nested-reference").unwrap();
    nested["design"] = json!({"referenceAssetIds":["broken-reference"]});
    f.store.save_asset(nested).unwrap();
    let nested_request = json!({"provider":"dreamina","model":"offline-image",
        "assetIds":["bad-via-nested-reference"],"imageProductionPolicy":"automatic"});
    assert!(f
        .engine
        .queue_images_with_account_check(&f.store, &nested_request, |_, account| Ok(
            account.to_owned()
        ))
        .is_err());
    assert!(
        f.store.jobs().unwrap().is_empty(),
        "a traversed parent must roll back with its invalid selected branch"
    );

    // Visit the independent branch first, then fail the bad branch. Its rollback
    // must preserve the earlier valid selection while removing partial-root.
    let report = f.queue_assets(&[
        "healthy-selection".into(),
        "bad-child".into(),
        "bad-via-nested-reference".into(),
    ]);
    assert_eq!(report["queuedCount"], 1);
    assert_eq!(report["blockedCount"], 2);
    assert_eq!(report["blocked"][0]["assetId"], "bad-child");
    let jobs = f.successful_job_map();
    assert_eq!(jobs.len(), 1);
    assert!(jobs.contains_key("healthy-selection"));
    f.pump(|| f.store.job(s(&jobs["healthy-selection"], "id")).unwrap()["status"] == "downloaded");
    f.drain();
    assert_eq!(f.channels.total_submits(), 1);
    assert_eq!(f.store.jobs().unwrap().len(), 1);
    assert!(f
        .store
        .adopted("partial-root", "partial-root")
        .unwrap()
        .is_none());
}

#[test]
fn parallel_image_lanes_keep_m2_frozen_config_prompt_and_receipt_after_restart() {
    let mut f = Fixture::new();
    f.store
        .save_settings(&json!({"providerTimeoutSeconds":19,"projectVisualStyle":"水墨国漫"}))
        .unwrap();
    f.asset("frozen", None);
    let mut asset = f.store.active_asset("frozen").unwrap();
    let context = json!({"source":f.store.source().unwrap(),"foundation":f.store.foundation().unwrap(),"assets":f.store.values("assets").unwrap(),"settings":f.store.settings().unwrap()});
    let receipt = pipeline::asset_design_contract::design_receipt(&context, &asset).unwrap();
    let design_context = json!({"version":pipeline::asset_design_contract::DESIGN_PROFILE,"comparisonDesigns":[],"projectVisualStyle":"水墨国漫"});
    asset["expectedVersion"] = asset["version"].clone();
    asset["design"] = json!({"designReceipt":receipt,"frozenContext":design_context});
    f.store.save_asset(asset).unwrap();
    let report = f.queue_assets(&["frozen".into()]);
    let key = s(&report["queued"][0], "jobId").to_owned();
    let before = f.store.job(&key).unwrap();
    let original = f.raw(&key).0;
    let request_id = s(&before["snapshot"], "requestId").to_owned();
    let provider_config = before["snapshot"]["providerConfig"].clone();
    assert_eq!(before["snapshot"]["designReceipt"], receipt);
    assert_eq!(before["snapshot"]["designContext"], design_context);
    assert_eq!(before["snapshot"]["sourceChain"], json!([]));
    assert_eq!(before["snapshot"]["ratio"], "16:9");
    assert!(s(&before["snapshot"], "prompt").contains("水墨国漫"));
    assert!(
        s(&before["snapshot"], "prompt").contains(pipeline::asset_design_contract::CH_PRESENTATION)
    );
    let summary = f
        .store
        .pending_jobs()
        .unwrap()
        .into_iter()
        .find(|job| job["id"] == key)
        .unwrap();
    assert!(
        summary["snapshot"]["frozenSettings"].is_null(),
        "exercise the real compact pending projection"
    );
    f.store.save_settings(&json!({"dreaminaPath":"changed-never-executed-cli","providerTimeoutSeconds":99,"projectVisualStyle":"CG国漫","ratio":"3:2","imageResolution":"8K","imagePresentation":"当前新默认单幅"})).unwrap();
    f.engine = Engine::with_channels(f.engine.home.clone(), f.channels.clone()).unwrap();
    f.successful_job_map();
    f.pump(|| f.store.job(&key).unwrap()["status"] == "downloaded");
    f.drain();
    assert_eq!(
        f.raw(&key).0,
        original,
        "every lane retains the complete original M2 request"
    );
    let counts = f.channels.counts.lock().unwrap();
    let submitted = &counts.prepares[&request_id][0];
    assert_eq!(submitted.prompt, s(&before["snapshot"], "prompt"));
    assert_eq!(submitted.ratio, "16:9");
    assert_eq!(submitted.resolution, "2K");
    for key in [
        format!("prepare:{request_id}"),
        format!("submit:{request_id}"),
        format!("query:offline:{request_id}"),
        "download:https://offline.invalid/frozen.png".into(),
    ] {
        assert!(
            counts.configurations[&key]
                .iter()
                .all(|config| *config == provider_config),
            "{key}: current defaults must not replace the saved provider configuration"
        );
    }
}

#[test]
fn one_invalid_frozen_config_does_not_block_healthy_parallel_images_or_rewrite_its_request() {
    let f = Fixture::new();
    f.asset("bad-config", None);
    f.asset("healthy-config", None);
    f.queue_assets(&["bad-config".into(), "healthy-config".into()]);
    let jobs = f.successful_job_map();
    let bad_key = s(&jobs["bad-config"], "id");
    let healthy_key = s(&jobs["healthy-config"], "id");
    let mut damaged = jobs["bad-config"]["snapshot"].clone();
    damaged["providerConfig"]["timeoutSeconds"] = json!("invalid");
    f.store
        .conn()
        .unwrap()
        .execute(
            "UPDATE jobs SET snapshot=?1 WHERE id=?2",
            rusqlite::params![damaged.to_string(), bad_key],
        )
        .unwrap();
    let original = f.raw(bad_key);
    f.pump(|| f.store.job(healthy_key).unwrap()["status"] == "downloaded");
    f.drain();
    let bad = f.store.job(bad_key).unwrap();
    assert_eq!(bad["status"], "queued");
    assert!(s(&bad, "error").contains("冻结图片渠道配置无效"));
    assert!(bad["nextAttemptAt"].as_i64().unwrap() > chrono::Utc::now().timestamp());
    assert_eq!(
        f.raw(bad_key),
        original,
        "a damaged packet preserves snapshot, original ID and receipt"
    );
    assert_eq!(f.channels.submit_count(s(&damaged, "requestId")), 0);
    assert_eq!(f.channels.total_submits(), 1);
}

#[test]
fn edited_parent_design_after_download_blocks_waiting_child_and_retains_frozen_source_chain() {
    let mut f = Fixture::new();
    f.asset("slow-frozen-source", None);
    f.asset("frozen-child", Some("slow-frozen-source"));
    let mut child = f.store.active_asset("frozen-child").unwrap();
    let context = json!({"source":f.store.source().unwrap(),"foundation":f.store.foundation().unwrap(),"assets":f.store.values("assets").unwrap(),"settings":f.store.settings().unwrap()});
    let receipt = pipeline::asset_design_contract::design_receipt(&context, &child).unwrap();
    child["expectedVersion"] = child["version"].clone();
    child["design"] = json!({"designReceipt":receipt,"frozenContext":{"version":pipeline::asset_design_contract::DESIGN_PROFILE}});
    f.store.save_asset(child).unwrap();
    let report = f.queue_assets(&["frozen-child".into()]);
    assert_eq!(report["queuedCount"], 2);
    let jobs = f.successful_job_map();
    let parent_key = s(&jobs["slow-frozen-source"], "id");
    let child_key = s(&jobs["frozen-child"], "id");
    let original_child = f.raw(child_key);
    assert_eq!(jobs["frozen-child"]["status"], "waiting_dependency");
    assert_eq!(
        jobs["frozen-child"]["snapshot"]["sourceChain"][0]["assetId"],
        "slow-frozen-source"
    );
    assert_eq!(jobs["frozen-child"]["snapshot"]["designReceipt"], receipt);
    // Stop driving ticks once the parent download starts, then release only
    // that worker. Otherwise a tick can legitimately unlock the child before
    // the edit is made, which would test an already submitted request instead.
    f.channels.slow_download_gate.hold();
    f.pump(|| f.channels.active("download") == 1);
    assert_eq!(
        f.store.job(child_key).unwrap()["status"],
        "waiting_dependency"
    );
    f.channels.slow_download_gate.release();
    f.wait(|| f.store.job(parent_key).unwrap()["status"] == "downloaded");
    f.drain();
    let original_parent = f.raw(parent_key);
    let mut parent = f.store.active_asset("slow-frozen-source").unwrap();
    parent["expectedVersion"] = parent["version"].clone();
    parent["prompt"] = json!("人工重新编辑来源人物：深蓝长袍。");
    f.store.save_asset(parent).unwrap();
    f.engine = Engine::with_channels(f.engine.home.clone(), f.channels.clone()).unwrap();
    let healthy_key = f.base("healthy-after-source-edit");
    f.successful_job_map();
    f.pump(|| f.store.job(&healthy_key).unwrap()["status"] == "downloaded");
    f.drain();
    let held_child = f.store.job(child_key).unwrap();
    assert_eq!(held_child["status"], "blocked");
    assert!(s(&held_child, "error").contains("等待期间指定来源设计已变化"));
    assert_eq!(
        f.raw(child_key),
        original_child,
        "source invalidation retains the original request, receipt and task ID"
    );
    assert_eq!(
        f.raw(parent_key),
        original_parent,
        "already downloaded parent history is retained"
    );
    assert_eq!(
        f.channels
            .submit_count(s(&held_child["snapshot"], "requestId")),
        0
    );
    assert_eq!(f.channels.total_submits(), 2);
}

#[test]
fn trial_review_counts_missing_ancestors_of_a_selected_leaf_against_the_three_image_limit() {
    let f = Fixture::new();
    f.store
        .save_settings(&json!({"trialApproved":false}))
        .unwrap();
    let chain: Vec<_> = (0..11).map(|depth| format!("trial-{depth:02}")).collect();
    for (depth, name) in chain.iter().enumerate() {
        f.asset(
            name,
            depth.checked_sub(1).map(|index| chain[index].as_str()),
        );
    }
    let mut request = json!({"provider":"dreamina","model":"offline-image",
        "assetIds":[chain.last().unwrap()],"imageProductionPolicy":"trialReview","trial":true});
    let error = f
        .engine
        .queue_images_with_account_check(&f.store, &request, |_, account| Ok(account.to_owned()))
        .unwrap_err();
    assert!(
        error.contains("最多选择3个资产"),
        "one selected leaf still requires eleven new images before its reference chain can exist"
    );
    assert!(f.store.jobs().unwrap().is_empty());
    f.engine.tick().unwrap();
    f.drain();
    assert_eq!(f.channels.total_submits(), 0);
    assert!(f.channels.counts.lock().unwrap().prepares.is_empty());

    // The actual three-image boundary remains valid: one leaf with two missing
    // ancestors may enter the normal submit/query/download path as a trial.
    request["assetIds"] = json!([chain[2]]);
    let report = f
        .engine
        .queue_images_with_account_check(&f.store, &request, |_, account| Ok(account.to_owned()))
        .unwrap();
    assert_eq!(report["queuedCount"], 3);
    assert_eq!(report["blockedCount"], 0);
    let jobs = f.successful_job_map();
    assert_eq!(jobs.len(), 3);
    f.pump(|| {
        jobs.values()
            .all(|job| f.store.job(s(job, "id")).unwrap()["status"] == "downloaded")
    });
    f.drain();
    assert_eq!(f.channels.total_submits(), 3);
    for name in chain.iter().take(3) {
        assert!(f.store.adopted(name, name).unwrap().is_some());
    }
    for name in chain.iter().skip(3) {
        assert!(!jobs.contains_key(name));
    }
}
