use super::*;

impl Engine {
    pub(super) fn reserve_image_workflow_v1(
        self: &Arc<Self>,
        store: Store,
        mut request: Value,
        lease: ActiveGuard,
    ) -> Result<()> {
        request["workflowProfile"] = json!(crate::asset_graph::PROFILE);
        let mut settings = self.effective_settings(&store)?;
        crate::app_settings::freeze_credentials(&mut settings);
        store.repair_md_only_identity_design_gates(&request)?;
        // Validate the supplied source and scope before a channel preflight.
        // An MD-only error must not look like a missing model/account error.
        store.image_workflow_input(&request, &settings)?;
        let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        settings = runtime.block_on(resolve_text_execution_settings(&settings))?;
        let input = store.image_workflow_input(&request, &settings)?;
        let native = self.reserve_native_workflow(&request, &input["imageSource"], &settings)?;
        let snapshot = json!({"runnerVersion":"stage2-runner/v1","workflowProfile":crate::asset_graph::PROFILE,"stage":request["action"],"sourceVersion":input["sourceVersion"],"settings":settings,"request":request,"stage2Context":input,"nativeAcceptance":native});
        let job = store.add_job(
            "workflow",
            &format!("{}:{}", s(&request, "action"), s(&request, "episodeId")),
            "local",
            "local",
            &snapshot,
            "queued",
        )?;
        self.bind_native_workflow(&store, &native, &job)?;
        self.execute_image_workflow_v1(store, job, snapshot, lease)
    }
    pub(super) fn execute_image_workflow_v1(
        self: &Arc<Self>,
        store: Store,
        job: String,
        snapshot: Value,
        lease: ActiveGuard,
    ) -> Result<()> {
        let engine = self.clone();
        store.ensure_workflow_storage()?;
        Self::spawn_owned(lease, move || {
            let result: Result<Value> = (|| {
                let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
                let input = &snapshot["stage2Context"];
                pipeline::asset_workflow::validate_source(&input["imageSource"])?;
                store.update_job(&job, "running", &json!({}))?;
                let settings = snapshot["settings"].clone();
                let executor = SavedTextExecutor {
                    store: store.clone(),
                    settings: settings.clone(),
                    workflow_id: Some(job.clone()),
                };
                let output = runtime.block_on(pipeline::run_asset_design(
                    input,
                    &executor,
                    s(&settings, "textModel"),
                    s(&settings, "reasoning"),
                ))?;
                let mut output = store.finish_image_prompts(input, &output, &job)?;
                // A historical explicit produce action still means words + images.
                // The new normal UI invokes design and run separately.
                if snapshot["stage"] == "images.produce" {
                    let mut generation = snapshot["request"].clone();
                    generation["action"] = json!("images.run");
                    generation["onlyReady"] = json!(true);
                    let report =
                        engine.queue_images_frozen(&store, &generation, &settings, |c, a| {
                            engine.checked_account(c, a)
                        })?;
                    output["imageQueueReport"] = report;
                    store.update_job(
                        &job,
                        image_stage_completion_state(&output),
                        &json!({"result":output}),
                    )?;
                }
                Ok(output)
            })();
            match result {
                Ok(_) => Ok(()),
                Err(error) => store.update_job(&job, "failed", &json!({"error":error})),
            }
        })
    }
}
