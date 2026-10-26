//! New-profile acceptance through the real Engine queue/tick/adoption path.
//! The parent's in-memory Channels writes synthetic PNGs without any network,
//! account inspection, image service invocation or video submission.
use super::*;

fn import_module22_fixture(f: &Fixture) -> Vec<Value> {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/module22-native");
    let mut request = json!({"path":root.join("adopted.md")});
    let preview = crate::lingmo_import::preview_file(&request).unwrap();
    request["expectedSha256"] = preview["markdownSha256"].clone();
    request["previewToken"] = preview["previewToken"].clone();
    crate::lingmo_import::import(&f.store, &request).unwrap();
    f.store.values("assets").unwrap().into_iter().map(|mut a|{
        a["expectedVersion"]=a["version"].clone();a["workflowProfile"]=json!(crate::asset_graph::PROFILE);
        a["referenceRequirements"]=json!([]);a["promptStatus"]=json!("ready");a["preparation"]=json!({"mode":"REFERENCE"});
        a["imagePrompt"]=json!({"version":pipeline::asset_workflow::PROMPT_VERSION,"text":"合成测试提示词；不得调用真实服务","status":"ready","layoutKind":if a["lingmoType"]=="SC"{"scene"}else if a["lingmoType"]=="CH"{"character"}else{"prop"}});
        f.store.save_asset(a).unwrap()
    }).collect()
}

#[test]
fn module22_native_queue_and_actual_send_keep_four_original_tasks_and_readonly_parent() {
    let f = Fixture::new();
    let assets = import_module22_fixture(&f);
    let selected: Vec<_> = assets.iter().map(|a| a["id"].clone()).collect();
    let started = Instant::now();
    let report=f.engine.queue_images_with_account_check(&f.store,&json!({"provider":"dreamina","model":"offline-image","workflowProfile":crate::asset_graph::PROFILE,"assetIds":selected,"imageProductionPolicy":"automatic"}),|_,a|Ok(a.to_owned())).unwrap();
    assert_eq!(report["queuedCount"], 4, "{report}");
    assert_eq!(report["blockedCount"], 0, "{report}");
    eprintln!(
        "module22 native four-image queue milliseconds={}",
        started.elapsed().as_millis()
    );
    let jobs = f.successful_job_map();
    assert_eq!(jobs.len(), 4);
    let mut tasks = std::collections::HashSet::new();
    for job in jobs.values() {
        binding::validate_generation_snapshot(&f.store, &job["snapshot"]).unwrap();
        assert_eq!(
            job["snapshot"]["module22DispatchContract"],
            "module22-image-dispatch/v1"
        );
        let batches = job["snapshot"]["module22Batches"].as_array().unwrap();
        assert_eq!(batches.len(), 1);
        assert!(tasks.insert(s(&batches[0], "externalTaskId").to_owned()));
        assert_ne!(batches[0]["externalAssetId"], "A0005");
    }
    f.pump(|| {
        jobs.values()
            .all(|j| f.store.job(s(j, "id")).unwrap()["status"] == "downloaded")
    });
    f.drain();
    assert_eq!(f.channels.total_submits(), 4);
    for a in assets {
        assert_eq!(
            f.store.adopted(s(&a, "id"), s(&a, "variantId")).unwrap(),
            None
        );
    }
}

#[test]
fn module22_native_queue_rechecks_changed_package_at_real_send_boundary() {
    let f = Fixture::new();
    let assets = import_module22_fixture(&f);
    let a = &assets[0];
    let report=f.engine.queue_images_with_account_check(&f.store,&json!({"provider":"dreamina","model":"offline-image","workflowProfile":crate::asset_graph::PROFILE,"assetIds":[a["id"]],"imageProductionPolicy":"automatic"}),|_,a|Ok(a.to_owned())).unwrap();
    assert_eq!(report["queuedCount"], 1, "{report}");
    let job = f.store.jobs().unwrap().remove(0);
    let sha = s(&a["handoffPackage"], "packageFingerprint");
    fs::write(
        f.store.root.join(format!(
            "imports/complete-packages/{sha}/source-original.txt"
        )),
        "changed synthetic original",
    )
    .unwrap();
    assert!(binding::validate_generation_snapshot(&f.store, &job["snapshot"]).is_err());
    f.engine.tick().unwrap();
    f.drain();
    assert_eq!(f.channels.total_submits(), 0);
}

fn save_modern_asset(f: &Fixture, name: &str, parent: Option<&str>) -> Value {
    let references = parent
        .map(|id| {
            vec![json!({"targetAssetId":id,"targetVariantId":id,
                "role":"topology","required":true,"mustPreserve":["door-placement"],
                "mayChange":["lighting"]})]
        })
        .unwrap_or_default();
    f.store
        .save_asset(json!({"id":name,"variantId":name,"baseAssetId":"offline-scene-identity",
            "name":name,"kind":"SC","workflowProfile":crate::asset_graph::PROFILE,
            "sourceAssetId":"unselected-family-design","filename":format!("{name}.png"),
            "preparation":{"mode":if parent.is_some(){"DERIVE"}else{"REFERENCE"}},
            "adoptedDesign":{"revision":1,"full":"既定场景空间，门在左窗在右。"},
            "imagePrompt":{"version":pipeline::asset_workflow::PROMPT_VERSION,
                "layoutKind":"scene","status":"ready","text":"无人场景，依照明确参考保持门窗拓扑。"},
            "promptStatus":"ready","referenceRequirements":references}))
        .unwrap()
}

fn assert_candidate(f: &Fixture, job: &Value, asset: &str) -> String {
    let current = f.store.job(s(job, "id")).unwrap();
    assert_eq!(current["status"], "downloaded");
    assert_eq!(current["snapshot"]["imageAdoptionPolicy"], "manual");
    assert_eq!(current["result"]["mediaIds"].as_array().unwrap().len(), 1);
    let media_id = current["result"]["mediaIds"][0]
        .as_str()
        .unwrap()
        .to_owned();
    let media = f.store.get("media", &media_id).unwrap();
    assert_eq!(media["kind"], "image");
    assert_ne!(media["placeholder"], true);
    f.store.verify_media(&media).unwrap();
    assert_eq!(f.store.adopted(asset, asset).unwrap(), None);
    media_id
}

fn run_modern_manual_chain(depth: usize) {
    let f = Fixture::new();
    let chain: Vec<_> = (0..depth).map(|n| format!("modern-{n:02}")).collect();
    let independent = "modern-independent";
    // This family/design relationship has no media requirement and no visual
    // use. It must never silently expand the explicitly selected image scope.
    f.store
        .save_asset(
            json!({"id":"unselected-family-design","variantId":"unselected",
            "kind":"SC","name":"未入画的文字设计父项","prompt":"仅供追溯"}),
        )
        .unwrap();
    let assets: Vec<_> = chain
        .iter()
        .enumerate()
        .map(|(index, name)| {
            save_modern_asset(&f, name, index.checked_sub(1).map(|i| chain[i].as_str()))
        })
        .collect();
    save_modern_asset(&f, independent, None);
    let mut unit = f.store.get("units", "review-unit").unwrap();
    unit["imageBindings"] = json!(chain
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(independent))
        .map(|id| json!({"assetId":id,"variantId":id}))
        .collect::<Vec<_>>());
    f.store.save_unit(unit).unwrap();
    let selected: Vec<_> = chain
        .iter()
        .rev()
        .cloned()
        .chain(std::iter::once(independent.into()))
        .collect();
    let result = f
        .engine
        .queue_images_with_account_check(
            &f.store,
            &json!({"provider":"dreamina","model":"offline-image",
                "workflowProfile":crate::asset_graph::PROFILE,"assetIds":selected,
                "imageProductionPolicy":"automatic","onlyReady":false}),
            |_, account| Ok(account.to_owned()),
        )
        .unwrap();
    assert_eq!(result["queuedCount"], depth + 1, "{result}");
    assert_eq!(result["blockedCount"], 0, "{result}");
    let jobs = f.successful_job_map();
    assert_eq!(jobs.len(), depth + 1);
    assert!(!jobs.contains_key("unselected-family-design"));
    for (index, name) in chain.iter().enumerate() {
        let snapshot = &jobs[name]["snapshot"];
        assert_eq!(snapshot["workflowProfile"], crate::asset_graph::PROFILE);
        assert_eq!(snapshot["imageAdoptionPolicy"], "manual");
        assert_eq!(snapshot["assetVersion"], assets[index]["version"]);
        assert_eq!(
            snapshot["referenceRequirements"],
            assets[index]["referenceRequirements"]
        );
        assert_eq!(
            jobs[name]["status"],
            if index == 0 {
                "queued"
            } else {
                "waiting_dependency"
            }
        );
        assert_eq!(snapshot["references"], json!([]));
        assert_eq!(snapshot["referenceBindings"], json!([]));
    }
    assert_eq!(jobs[independent]["status"], "queued");
    f.pump(|| {
        [chain[0].as_str(), independent]
            .iter()
            .all(|name| f.store.job(s(&jobs[*name], "id")).unwrap()["status"] == "downloaded")
    });
    f.drain();
    let independent_media = assert_candidate(&f, &jobs[independent], independent);
    assert_eq!(f.channels.total_submits(), 2);
    for _ in 0..2 {
        f.engine.tick().unwrap();
        f.drain();
    }
    assert_eq!(
        f.channels.total_submits(),
        2,
        "download alone cannot release the direct child"
    );
    assert_eq!(
        f.store.ensure_current_images().unwrap()["selectedCount"],
        0,
        "history initialization cannot auto-adopt modern candidates"
    );
    let mut adopted: Vec<String> = Vec::new();
    for (index, name) in chain.iter().enumerate() {
        let job = &jobs[name];
        if index > 0 {
            // Only the immediately preceding explicit adoption can release this
            // layer. Later descendants still have no real adopted parent image.
            assert_eq!(
                f.store.job(s(job, "id")).unwrap()["status"],
                "waiting_dependency"
            );
            f.engine.tick().unwrap();
            f.drain();
            assert_eq!(f.channels.submit_count(s(&job["snapshot"], "requestId")), 1);
            for later in chain.iter().skip(index + 1) {
                assert_eq!(
                    f.store.job(s(&jobs[later], "id")).unwrap()["status"],
                    "waiting_dependency"
                );
                assert_eq!(
                    f.channels
                        .submit_count(s(&jobs[later]["snapshot"], "requestId")),
                    0
                );
            }
            f.pump(|| f.store.job(s(job, "id")).unwrap()["status"] == "downloaded");
            f.drain();
        }
        let candidate = assert_candidate(&f, job, name);
        let original = f.raw(s(job, "id"));
        for _ in 0..2 {
            f.engine.tick().unwrap();
            f.drain();
        }
        if let Some(child) = chain.get(index + 1) {
            assert_eq!(
                f.store.job(s(&jobs[child], "id")).unwrap()["status"],
                "waiting_dependency"
            );
            assert_eq!(
                f.channels
                    .submit_count(s(&jobs[child]["snapshot"], "requestId")),
                0
            );
        }
        f.engine
            .dispatch(
                json!({"action":"media.adopt","projectId":f.store.project().unwrap()["id"],
                "assetId":name,"variantId":name,"mediaId":candidate}),
            )
            .unwrap();
        assert_eq!(
            f.store.adopted(name, name).unwrap().as_deref(),
            Some(candidate.as_str())
        );
        assert_eq!(
            f.store.active_asset(name).unwrap()["imageStatus"],
            "adopted"
        );
        assert_eq!(
            f.raw(s(job, "id")),
            original,
            "explicit adoption preserves the original request and receipt"
        );
        let current = f.store.job(s(job, "id")).unwrap();
        assert_eq!(
            current["snapshot"]["referenceBindings"],
            current["snapshot"]["references"]
        );
        if index > 0 {
            let refs = current["snapshot"]["referenceBindings"].as_array().unwrap();
            assert_eq!(refs.len(), 1);
            let mother = f.store.get("media", &adopted[index - 1]).unwrap();
            assert_eq!(refs[0]["assetId"], chain[index - 1]);
            assert_eq!(refs[0]["variantId"], chain[index - 1]);
            assert_eq!(refs[0]["assetVersion"], assets[index - 1]["version"]);
            assert_eq!(refs[0]["mediaId"], adopted[index - 1]);
            assert_eq!(refs[0]["sha256"], mother["sha256"]);
            let counts = f.channels.counts.lock().unwrap();
            let prepared = &counts.prepares[s(&job["snapshot"], "requestId")][0];
            assert_eq!(prepared.references.len(), 1);
            assert_eq!(prepared.references[0].media_id, adopted[index - 1]);
            assert_eq!(prepared.references[0].sha256, s(&mother, "sha256"));
            if index > 1 {
                assert_ne!(
                    refs[0]["mediaId"], adopted[0],
                    "deep layers must not substitute the root image"
                );
            }
        }
        assert_eq!(
            f.store.active_asset(name).unwrap()["adoptedDesign"],
            assets[index]["adoptedDesign"]
        );
        adopted.push(candidate);
    }
    for _ in 0..2 {
        f.engine.tick().unwrap();
        f.drain();
    }
    assert_eq!(
        assert_candidate(&f, &jobs[independent], independent),
        independent_media
    );
    assert_eq!(f.store.jobs().unwrap().len(), depth + 1);
    let counts = f.channels.counts.lock().unwrap();
    assert_eq!(counts.submits.values().sum::<usize>(), depth + 1);
    assert!(counts.submits.values().all(|count| *count == 1));
    assert!(counts.prepares.values().all(|requests| requests.len() == 1));
    assert_eq!(counts.submit_kinds.get("video").copied().unwrap_or(0), 0);
}

#[test]
fn modern_three_level_engine_queue_requires_manual_adoption_before_each_child() {
    run_modern_manual_chain(3);
}

#[test]
fn modern_eleven_level_engine_queue_keeps_direct_parent_bindings_and_independent_progress() {
    run_modern_manual_chain(11);
}
