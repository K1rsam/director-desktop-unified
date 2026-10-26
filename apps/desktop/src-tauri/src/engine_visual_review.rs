use super::*;

impl Engine {
    pub(super) fn visual_review_capability(
        &self,
        store: &Store,
        settings: &Value,
    ) -> Result<Value> {
        let mut projection = crate::visual_review::capability(store, settings)?;
        let status = crate::visual_review::probe_status(store);
        if status.is_object() {
            let checking = self
                .active
                .lock()
                .map_err(|e| e.to_string())?
                .contains(&format!("visual-probe:{}", s(&store.project()?, "id")));
            projection["status"] = if status["status"] == "checking" && !checking {
                json!("unknown")
            } else {
                status["status"].clone()
            };
            if projection["verified"] != true
                && status["reason"].as_str().is_some_and(|v| !v.is_empty())
            {
                projection["reason"] = status["reason"].clone();
            }
        }
        if projection["status"] == "unknown" {
            projection["explicitRestart"] =
                crate::visual_review::restart_projection(store).unwrap_or(Value::Null);
        }
        Ok(projection)
    }
    pub(super) fn spawn_visual_review(
        self: &Arc<Self>,
        store: Store,
        request: Value,
    ) -> Result<()> {
        if !self.standard_native || !crate::text_sdk::native_execution_enabled() {
            return Err("VISUAL_NATIVE_REQUIRED: 真实审图只在已启用的 Windows SDK 入口执行".into());
        }
        let settings = self.effective_settings(&store)?;
        let restart = request["action"] == "images.reviewCapability.restartUnknown";
        let probe = request["action"] == "images.reviewCapability" || restart;
        let project = store.project()?;
        let mut keys = vec![
            "channel:sdk-visual-review".into(),
            format!("pipeline:{}", s(&project, "id")),
        ];
        if probe {
            keys.push(format!("visual-probe:{}", s(&project, "id")));
        }
        let guard = self.acquire_keys(project_worker_keys(&store, keys)?)?;
        if !probe {
            crate::visual_review::ensure_original_available(&store, s(&request, "mediaId"))?;
            crate::visual_review::candidate_basis(
                &store,
                s(&request, "assetId"),
                s(&request, "mediaId"),
            )?;
        }
        if probe {
            crate::visual_review::set_probe_status(
                &store,
                "checking",
                "正在用两张本地合成图验证真实读图；不会生成图片",
            )?;
        }
        let engine = self.clone();
        Self::spawn_owned(guard, move || {
            let runtime = crate::text_cli::control_runtime()?;
            let result = if restart {
                runtime.block_on(crate::visual_review::restart_capability(
                    &store, &settings, &request,
                ))
            } else if probe {
                runtime.block_on(crate::visual_review::check_capability(&store, &settings))
            } else {
                runtime.block_on(crate::visual_review::review_candidate(
                    &store, &settings, &request,
                ))
            };
            if probe {
                match &result {
                    Ok(_) => crate::visual_review::set_probe_status(
                        &store,
                        "verified",
                        "实际读图验证通过；资产仍需逐张审核",
                    )?,
                    Err(error) => crate::visual_review::set_probe_status(
                        &store,
                        if restart || error.contains("UNKNOWN") {
                            "unknown"
                        } else {
                            "manual_required"
                        },
                        error,
                    )?,
                }
            }
            store.event("",if result.is_ok(){"visual_review_completed"}else{"visual_review_manual_required"},&match &result{Ok(v)=>v.clone(),Err(e)=>json!({"message":e,"assetId":request["assetId"],"mediaId":request["mediaId"]})})?;
            engine.wake_images();
            result.map(|_| ())
        })
    }
    pub(super) fn schedule_visual_reviews(self: &Arc<Self>, store: &Store) -> Result<()> {
        if !self.standard_native || !crate::text_sdk::native_execution_enabled() {
            return Ok(());
        }
        // Reopen performs one local receipt reconciliation. UNKNOWN is terminal
        // for this scheduler; it neither polls indefinitely nor buys another turn.
        let mut interrupted = store.interrupted_visual_reviews()?;
        if crate::visual_review::probe_status(store)["status"] == "checking" {
            interrupted.push(json!({}));
        }
        for request in interrupted {
            let Ok(_guard) = self.acquire_keys(project_worker_keys(
                store,
                vec![
                    "channel:sdk-visual-review".into(),
                    format!("pipeline:{}", s(&store.project()?, "id")),
                ],
            )?) else {
                continue;
            };
            if let Err(error) =
                crate::visual_review::reconcile(store, &self.effective_settings(store)?, &request)
            {
                if s(&request, "mediaId").is_empty() {
                    crate::visual_review::set_probe_status(store, "manual_required", &error)?;
                } else {
                    crate::visual_review::preserve_missing_evidence(
                        store,
                        s(&request, "mediaId"),
                        &error,
                    )?;
                }
            }
        }
        for mut request in store.due_visual_reviews()? {
            request["action"] = json!("images.reviewCandidate");
            // Reservations are acquired before any durable claim. Busy work is
            // queued; only an actual review attempt records a terminal result.
            if let Err(error) = self.spawn_visual_review(store.clone(), request.clone()) {
                if error.starts_with("VISUAL_ORIGINAL_EVIDENCE_MISSING") {
                    crate::visual_review::preserve_missing_evidence(
                        store,
                        s(&request, "mediaId"),
                        &error,
                    )?;
                } else if error.starts_with("VISUAL_") {
                    store.record_visual_review(s(&request,"mediaId"),&json!({"status":"manual_required","reason":error,"assetId":request["assetId"],"mediaId":request["mediaId"],"checkedAt":now(),"adopted":false}))?;
                }
            }
        }
        Ok(())
    }
}
