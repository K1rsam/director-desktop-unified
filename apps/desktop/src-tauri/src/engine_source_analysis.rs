//! A recoverable text-only compatibility pass before foundation is frozen.
use super::*;

const RUNNER: &str = "source-analysis-runner/v1";

fn selection(request: &Value) -> Value {
    let ids = request
        .get("episodeIds")
        .or_else(|| request.pointer("/options/selection/episodeIds"));
    ids.map(|ids| json!({"episodeIds":ids}))
        .unwrap_or(json!({}))
}

fn validate_selection(source: &Value, selected: &Value) -> Result<()> {
    if let Some(ids) = selected.get("episodeIds") {
        let ids = ids
            .as_array()
            .filter(|ids| !ids.is_empty())
            .ok_or("请选择非空的制作集范围")?;
        let mut seen = HashSet::new();
        for id in ids {
            let id = id.as_str().ok_or("制作集ID无效")?;
            if !seen.insert(id)
                || !source["episodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|e| e["id"] == id)
            {
                return Err("所选集已变化，请刷新后重试".into());
            }
        }
    }
    Ok(())
}

impl Engine {
    pub(super) fn spawn_source_analysis(
        self: &Arc<Self>,
        store: Store,
        request: &Value,
    ) -> Result<()> {
        let _start = self.acquire_keys(vec![format!(
            "source-analysis-start:{}",
            s(&store.project()?, "id")
        )])?;
        let lease = self.acquire_pipeline(&store)?;
        let source = store.source()?;
        let parsed: pipeline::LockedSource =
            serde_json::from_value(source.clone()).map_err(|_| "请先导入完整剧本")?;
        pipeline::validate_source(&parsed)?;
        if request
            .get("expectedSourceVersion")
            .is_some_and(|v| v != &source["version"])
        {
            return Err("原文已变化，请刷新后重试".into());
        }
        let selected = selection(request);
        validate_selection(&source, &selected)?;
        // Every repeated click/resume keeps the first frozen selection and
        // provider. In particular it cannot bypass a dispatched UNKNOWN node.
        for job in store.jobs()? {
            let snapshot = &job["snapshot"];
            if job["kind"] != "workflow"
                || job["entityId"] != "source.analyze"
                || snapshot["runnerVersion"] != RUNNER
            {
                continue;
            }
            let original = &snapshot["input"]["source"];
            let is_current = original == &source || job["result"]["source"] == source;
            if !is_current {
                continue;
            }
            if job["status"] == "partially_blocked" {
                return Ok(());
            }
            if job["status"] == "completed" {
                if request["continueFoundation"] != true {
                    return Ok(());
                }
                if snapshot["input"]["selection"] == selected {
                    store.request_source_analysis_continuation(s(&job, "id"))?;
                } else {
                    // The analysis is source-wide; a later explicitly selected
                    // foundation scope is a new ordinary production action.
                    drop(lease);
                    let mut next = json!({"action":"foundation.run"});
                    if let Some(ids) = selected.get("episodeIds") {
                        next["episodeIds"] = ids.clone();
                    }
                    return self.spawn_production(store, next);
                }
            } else if snapshot["input"]["selection"] != selected
                || snapshot["continueFoundation"] != (request["continueFoundation"] == true)
            {
                return Err(
                    "当前原文已有格式识别任务，先继续原任务；原制作范围与回执保留。".into(),
                );
            }
            return self.execute_source_analysis_owned(
                store,
                s(&job, "id").into(),
                snapshot.clone(),
                lease,
            );
        }
        crate::text_executor::check_cli_unknown_hold(&store)?;
        store.ensure_workflow_storage()?;
        let mut input = json!({"source":source,"settings":store.settings()?,"selection":selected,"sourceVersion":source["version"],"sourceSha256":source["sha256"],"sourceStructureProfile":"source-structure-analysis/v1"});
        let needs_model = pipeline::source_analysis::requires_model(&input)?;
        let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        let settings_hash = freeze_production_execution_settings(
            &store,
            "source.analyze",
            &mut input,
            |settings| {
                // A deterministic reparse must not depend on an installed or
                // logged-in model channel. The pipeline repeats this test.
                if !needs_model {
                    return Ok(json!({"sourceAnalysisLocalOnly":true}));
                }
                let mut settings = if self.standard_native {
                    crate::app_settings::effective(&self.home, settings)?
                } else {
                    settings.clone()
                };
                crate::app_settings::freeze_credentials(&mut settings);
                runtime.block_on(resolve_text_execution_settings(&settings))
            },
        )?;
        if store.source()? != source {
            return Err("格式识别准备期间原文已更新，未提交模型请求。".into());
        }
        let snapshot = json!({"runnerVersion":RUNNER,"stage":"source.analyze","sourceVersion":source["version"],"sourceSha256":source["sha256"],"parserVersion":source["parserVersion"],"savedSettingsSha256":settings_hash,"input":input,"rules":pipeline::source_analysis::rule_snapshot(),"continueFoundation":request["continueFoundation"]==true});
        let run = store.add_job(
            "workflow",
            "source.analyze",
            "local",
            "local",
            &snapshot,
            "queued",
        )?;
        self.execute_source_analysis_owned(store, run, snapshot, lease)
    }

    pub(super) fn resume_source_analysis(
        self: &Arc<Self>,
        store: Store,
        request: &Value,
        job: &Value,
    ) -> Result<()> {
        if request["action"] != "workflows.resume"
            || job["kind"] != "workflow"
            || job["snapshot"]["runnerVersion"] != RUNNER
        {
            return Err("格式识别恢复记录无效，未新建请求".into());
        }
        let lease = self.acquire_pipeline(&store)?;
        self.execute_source_analysis_owned(
            store,
            s(job, "id").into(),
            job["snapshot"].clone(),
            lease,
        )
    }

    fn execute_source_analysis_owned(
        self: &Arc<Self>,
        store: Store,
        run: String,
        snapshot: Value,
        lease: ActiveGuard,
    ) -> Result<()> {
        if self.stop.load(Ordering::SeqCst) {
            return Err("客户端正在退出".into());
        }
        let engine = Arc::clone(self);
        std::thread::spawn(move || {
            let result = (|| -> Result<Value> {
                let job = store.job(&run)?;
                if job["snapshot"] != snapshot {
                    return Err("格式识别冻结规则已变化；原请求与回包保留。".into());
                }
                if ["completed", "partially_blocked"].contains(&s(&job, "status")) {
                    return Ok(job["result"].clone());
                }
                if snapshot["rules"] != pipeline::source_analysis::rule_snapshot() {
                    return Err("格式识别冻结规则已变化；原请求与回包保留。".into());
                }
                if store.source()? != snapshot["input"]["source"] {
                    return Err("原文版本已变化，未提交或覆盖格式识别结果。".into());
                }
                let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
                let settings = &snapshot["input"]["settings"];
                let exec = SavedTextExecutor {
                    store: store.clone(),
                    settings: settings.clone(),
                    workflow_id: Some(run.clone()),
                };
                store.update_job(&run, "running", &json!({}))?;
                let output = runtime.block_on(pipeline::source_analysis::run(
                    &snapshot["input"],
                    &exec,
                    s(settings, "textModel"),
                    s(settings, "reasoning"),
                ))?;
                store.publish_source_analysis(&run, &snapshot, &output)
            })();
            match result {
                Ok(_) => {}
                Err(error) => {
                    if store.job(&run).is_ok_and(|job| {
                        ["completed", "partially_blocked"].contains(&s(&job, "status"))
                    }) {
                        let _ = store.event(
                            &run,
                            "source_analysis_resume_held",
                            &json!({"error":error,"originalResultPreserved":true}),
                        );
                        drop(lease);
                        return;
                    }
                    let unknown = store.source_analysis_unknown(&run).unwrap_or(false);
                    let _ = store.update_job(
                        &run,
                        if unknown {
                            "submission_unknown"
                        } else {
                            "failed"
                        },
                        &json!({"error":error}),
                    );
                }
            }
            // The successor must own a new lease and a snapshot of the newly
            // published source. Never mutate the analysis's frozen input.
            drop(lease);
            if let Err(error) = engine.continue_foundation_after_source(&store, &run) {
                let _ = store.event(
                    &run,
                    "source_analysis_continuation_held",
                    &json!({"error":error,"originalResultPreserved":true}),
                );
            }
        });
        Ok(())
    }

    fn continue_foundation_after_source(self: &Arc<Self>, store: &Store, run: &str) -> Result<()> {
        let parent = store.job(run)?;
        let result = &parent["result"];
        if parent["status"] != "completed" || result["continuation"]["requested"] != true {
            return Ok(());
        }
        if store.source()? != result["source"] {
            return Err("格式识别后原文已变化，未自动开始制作基础。".into());
        }
        if let Some(child) = result["continuation"]["workflowId"].as_str() {
            let job = store.job(child)?;
            if result["continuation"]["existing"] == true {
                store.link_existing_source_analysis_successor(run, child)?;
            } else if job["snapshot"]["sourceAnalysisContinuation"]["workflowId"] != run
                || job["snapshot"]["input"]["source"] != result["source"]
            {
                return Err("格式识别后继绑定不一致".into());
            }
            return match s(&job, "status") {
                "queued" | "interrupted" => {
                    let lease = self.acquire_pipeline(store)?;
                    self.execute_production_owned(
                        store.clone(),
                        child,
                        job["snapshot"].clone(),
                        lease,
                    )
                }
                // Failed/unknown successors remain visible for explicit
                // recovery, rather than auto-looping the original model call.
                _ => Ok(()),
            };
        }
        let mut request = json!({"action":"foundation.run","projectId":store.project()?["id"],"sourceAnalysisWorkflowId":run});
        if let Some(ids) = parent["snapshot"]["input"]["selection"].get("episodeIds") {
            request["episodeIds"] = ids.clone();
        }
        self.spawn_production(store.clone(), request)
    }
}

#[cfg(test)]
#[path = "engine_source_analysis_tests.rs"]
mod tests;
