//! The one source-bound asset supplement chain may use the already-returned
//! author's rejected raw. It neither promotes nor reexecutes that author.
use super::*;
#[path = "text_executor_sdk_supplement_replay.rs"]
mod replay;

pub(super) const CONTRACT: &str = "series-asset-supplement/v1";
pub(super) const CONTRACT_V2: &str = "series-asset-supplement/v2";
pub(super) const CONTRACT_V3: &str = "series-asset-supplement/v3";
pub(super) fn supported_contract(value: &Value) -> bool {
    value == CONTRACT || value == CONTRACT_V2 || value == CONTRACT_V3
}

impl SavedTextExecutor {
    /// Only the proposal-key schema changed. Existing work keeps its exact
    /// request, original raw, attempt budget, and unresolved transport state.
    pub(in crate::text_executor) fn saved_supplement_design_request(
        &self,
        request: &TextRequest,
    ) -> Result<Option<TextRequest>> {
        let Some(legacy) = crate::pipeline::series::legacy_supplement_design_request(request)?
        else {
            return Ok(None);
        };
        let Some(run) = self.workflow_id.as_deref() else {
            return Ok(None);
        };
        let c = self.store.conn()?;
        let old:Option<(String,String,String)>=c.query_row("SELECT j.snapshot,w.request_hash,j.provider FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND w.stage='seriesDesign' AND j.kind='text'",
            params![run,step_key(request)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
        let Some((raw, sha, provider)) = old else {
            return Ok(None);
        };
        let saved: Value = serde_json::from_str(&raw).map_err(|_| HELD)?;
        let original: TextRequest = serde_json::from_value(saved.clone()).map_err(|_| HELD)?;
        let expected_provider = match s(&self.settings, "textProvider") {
            "" => "codex",
            value => value,
        };
        let serialized = |value: &TextRequest| serde_json::to_value(value).map_err(|_| HELD);
        if hash(&saved) != sha
            || saved.to_string() != raw
            || provider != expected_provider
            || (serialized(&original)? != serialized(request)?
                && serialized(&original)? != serialized(&legacy)?)
            || self.request_snapshot(&original)? != saved
        {
            return Err(HELD.into());
        }
        let parent = self.store.job(run)?;
        if parent["entityId"] != "storyboard.run"
            || parent["snapshot"]["input"]["settings"] != self.settings
            || !frozen_text_parent_matches("storyboard.run", &parent["snapshot"], &original)
        {
            return Err(HELD.into());
        }
        Ok(Some(original))
    }
    /// Hash/coverage return instructions do not replace any frozen review,
    /// including an unresolved one. Match only complete, exact old contracts.
    pub(in crate::text_executor) fn saved_supplement_review_request(
        &self,
        request: &TextRequest,
    ) -> Result<Option<TextRequest>> {
        if !supported_contract(&request.input["seriesAssetSupplement"]["version"]) {
            return Ok(None);
        }
        let coverage = crate::pipeline::series::legacy_supplement_coverage_request(request)?;
        let legacy = crate::pipeline::series::legacy_supplement_review_request(request)?;
        if coverage.is_none() && legacy.is_none() {
            return Ok(None);
        }
        let serialized = |value: &TextRequest| serde_json::to_value(value).map_err(|_| HELD);
        let mut candidates = vec![serialized(request)?];
        for candidate in [coverage, legacy].into_iter().flatten() {
            candidates.push(serialized(&candidate)?);
        }
        let Some(run) = self.workflow_id.as_deref() else {
            return Ok(None);
        };
        let c = self.store.conn()?;
        let old:Option<(String,String,String)>=c.query_row("SELECT j.snapshot,w.request_hash,j.provider FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2 AND w.stage='seriesDesignReview' AND j.kind='text'",
            params![run,step_key(request)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
        let Some((raw, sha, provider)) = old else {
            return Ok(None);
        };
        let saved: Value = serde_json::from_str(&raw).map_err(|_| HELD)?;
        let original: TextRequest = serde_json::from_value(saved.clone()).map_err(|_| HELD)?;
        let expected_provider = match s(&self.settings, "textProvider") {
            "" => "codex",
            value => value,
        };
        if hash(&saved) != sha
            || saved.to_string() != raw
            || provider != expected_provider
            || !candidates.contains(&serialized(&original)?)
            || self.request_snapshot(&original)? != saved
        {
            return Err(HELD.into());
        }
        let parent = self.store.job(run)?;
        if parent["entityId"] != "storyboard.run"
            || parent["snapshot"]["input"]["settings"] != self.settings
            || !frozen_text_parent_matches("storyboard.run", &parent["snapshot"], &original)
        {
            return Err(HELD.into());
        }
        Ok(Some(original))
    }
    pub(super) fn sdk_supplement_find_exact(&self, role: &str, sha: &str) -> Result<Value> {
        let run = self.workflow_id.as_deref().ok_or(HELD)?;
        if role.is_empty() || sha.len() != 64 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(HELD.into());
        }
        let c = self.store.conn()?;
        let mut q=c.prepare("SELECT j.id,j.snapshot FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND json_extract(j.snapshot,'$.input.executionRoleId')=?2").map_err(|e|e.to_string())?;
        let rows = q
            .query_map(params![run, role], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut found = None;
        for row in rows {
            let (id, raw) = row.map_err(|e| e.to_string())?;
            let request: TextRequest = serde_json::from_str(&raw).map_err(|_| HELD)?;
            if request_hash(&request)? == sha {
                if found.replace(id).is_some() {
                    return Err(HELD.into());
                }
            }
        }
        self.sdk_job(found.as_deref().ok_or(HELD)?)
    }
    fn sdk_supplement_context<'a>(&self, request: &'a TextRequest) -> Result<&'a Value> {
        let context = &request.input["seriesAssetSupplement"];
        if !supported_contract(&context["version"])
            || context["round"] != 0
            || context["maximumRounds"] != 1
            || context["maximumAttempts"] != 2
            || context["attempt"].as_u64().is_none_or(|n| n > 2)
            || request.input["workflowProfile"] != crate::asset_graph::PROFILE
            || request.input["freshContext"] != true
            || request.input.get("validationRepair").is_some()
            || !matches!(
                (
                    request.stage.as_str(),
                    request.skill.as_str(),
                    s(context, "phase")
                ),
                ("seriesDesign", "series-design", "design")
                    | (
                        "seriesDesignReview",
                        "series-design-review",
                        "design-review"
                    )
                    | ("seriesFinisher", "series-finisher", "finisher")
            )
        {
            return Err(HELD.into());
        }
        Ok(context)
    }
    pub(super) fn sdk_schema_supplement_predecessor(
        &self,
        request: &TextRequest,
    ) -> Result<Option<String>> {
        let context = self.sdk_supplement_context(request)?;
        let origin = self.sdk_supplement_find_exact(
            s(context, "originExecutionRoleId"),
            s(context, "originRequestSha256"),
        )?;
        // An ordinary current SDK workflow needs no schema-handoff capability.
        // Do not turn this narrowly-scoped proof into a general runtime upgrade.
        if origin["snapshot"].get(RETRY).is_none() && origin["snapshot"].get(CONTINUE).is_none() {
            return Ok(None);
        }
        let previous = self.sdk_supplement_find_exact(
            s(context, "predecessorExecutionRoleId"),
            s(context, "predecessorRequestSha256"),
        )?;
        Ok(Some(s(&previous, "id").to_owned()))
    }
    pub(super) fn sdk_schema_supplement_bound(
        &self,
        request: &TextRequest,
        previous: &Value,
        previous_raw: &Value,
    ) -> Result<()> {
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        let context = self.sdk_supplement_context(request)?;
        let origin = self.sdk_supplement_find_exact(
            s(context, "originExecutionRoleId"),
            s(context, "originRequestSha256"),
        )?;
        let exact_previous = self.sdk_supplement_find_exact(
            s(context, "predecessorExecutionRoleId"),
            s(context, "predecessorRequestSha256"),
        )?;
        if exact_previous != *previous
            || origin["status"] != "rejected"
            || origin["snapshot"]["stage"] != "seriesAuthor"
            || origin["snapshot"]["skill"] != "series-author"
            || origin["snapshot"]["input"]["freshContext"] != true
            || origin["receipt"] != json!({"contract":"desktop-runner/v1","validation":"rejected"})
            || s(&origin, "error").trim().is_empty()
            || origin["result"].is_null()
            || context["originRawSha256"] != hash(&origin["result"])
            || context["predecessorRawSha256"] != hash(previous_raw)
        {
            return Err(HELD.into());
        }
        let first = context["phase"] == "design" && context["attempt"] == 0;
        if first {
            if previous["id"] != origin["id"] || previous["status"] != "rejected" {
                return Err(HELD.into());
            }
        } else if previous["id"] == origin["id"]
            || previous["status"] != "completed"
            || [
                "version",
                "originExecutionRoleId",
                "originRequestSha256",
                "originRawSha256",
                "sourceSha256",
                "episodeId",
                "beforeAssetsSha256",
                "round",
                "maximumRounds",
                "maximumAttempts",
            ]
            .iter()
            .any(|key| previous["snapshot"]["input"]["seriesAssetSupplement"][key] != context[key])
        {
            return Err(HELD.into());
        }
        let origin_raw = if previous["id"] == origin["id"] {
            previous_raw.clone()
        } else {
            self.sdk_schema_owned_raw(&origin)?
        };
        let origin_request: TextRequest =
            serde_json::from_value(origin["snapshot"].clone()).map_err(|_| HELD)?;
        let previous_request: TextRequest =
            serde_json::from_value(previous["snapshot"].clone()).map_err(|_| HELD)?;
        let parent = self.store.job(self.workflow_id.as_deref().ok_or(HELD)?)?;
        if parent["entityId"] != "storyboard.run"
            || parent["snapshot"]["runnerVersion"] != "desktop-runner/v1"
        {
            return Err(HELD.into());
        }
        let value = |kind: &str| -> Result<&Value> {
            let packet = &request.input["packet"];
            let matches: Vec<_> = packet["bindings"]
                .as_array()
                .ok_or(HELD)?
                .iter()
                .filter(|b| b["kind"] == kind)
                .collect();
            if matches.len() != 1 {
                return Err(HELD.into());
            }
            packet["blobs"]
                .get(s(matches[0], "blob"))
                .and_then(|b| b.get("value"))
                .ok_or_else(|| HELD.into())
        };
        if self.verified_series_supplement_identity_context(
            &origin_request,
            &origin_raw,
            value("knownAssetCatalog")?,
            &origin,
        )? != *value("identityReservations")?
        {
            return Err(HELD.into());
        }
        // An exhausted chain is visited once for each historical receipt and
        // recursively again through its predecessor. Reuse only the pure
        // domain calculation after all current storage/identity gates above;
        // every frozen input is part of the key and the outer stack owns it.
        let inputs = serde_json::to_vec(&(
            "sdk-supplement-transition-domain/v1",
            request,
            &origin_request,
            &origin_raw,
            &previous_request,
            previous_raw,
            &parent["snapshot"]["input"],
        ))
        .map_err(|e| e.to_string())?;
        crate::text_executor::sdk_evidence_cache::pure_domain(&inputs, || {
            crate::pipeline::series::validate_supplement_transition(
                request,
                &origin_request,
                &origin_raw,
                Some(&previous_request),
                Some(previous_raw),
                &parent["snapshot"]["input"],
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{
        adopt_raw, continuation_snapshot, fixture_request_input, reject_synthetic,
    };
    use super::*;

    fn packet<'a>(request: &'a TextRequest, kind: &str) -> &'a Value {
        let packet = &request.input["packet"];
        let entry = packet["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["kind"] == kind)
            .unwrap();
        &packet["blobs"][s(entry, "blob")]["value"]
    }

    async fn fixture_without_identity_context() -> (
        tempfile::TempDir,
        SavedTextExecutor,
        Vec<(TextRequest, Value)>,
        String,
    ) {
        let (input, chain) = crate::pipeline::series::series_supplement_fixture().await;
        assert_eq!(
            chain.len(),
            7,
            "one original author, two design/review rounds, finisher and independent reviewer"
        );
        let origin = &chain[0].0;
        assert_eq!(origin.stage, "seriesAuthor");
        let (temp, executor, request, original, next) =
            fixture_request_input(origin.clone(), Some(input));
        let job = executor
            .sdk_reserve_schema_successor_checked(&request, &original, &next, || Ok(()))
            .unwrap();
        adopt_raw(&executor, &request, &next, &job, &chain[0].1);
        reject_synthetic(
            &executor,
            &request,
            &next,
            &job,
            "SERIES_CREATIVE_DEPENDENCY_UNRESOLVED",
        );
        let known = packet(&chain[1].0, "knownAssetCatalog");
        for asset in known.as_array().unwrap() {
            executor.store.save_asset(asset.clone()).unwrap();
        }
        (temp, executor, chain, job)
    }
    async fn fixture() -> (
        tempfile::TempDir,
        SavedTextExecutor,
        Vec<(TextRequest, Value)>,
        String,
    ) {
        let (temp, executor, chain, job) = fixture_without_identity_context().await;
        assert_eq!(
            executor
                .freeze_series_supplement_identity_context(
                    &chain[0].0,
                    &chain[0].1,
                    packet(&chain[1].0, "knownAssetCatalog"),
                    CONTRACT
                )
                .unwrap(),
            *packet(&chain[1].0, "identityReservations")
        );
        (temp, executor, chain, job)
    }
    include!("text_executor_sdk_supplement_identity_tests.rs");
    include!("text_executor_sdk_supplement_key_tests.rs");
    include!("text_executor_sdk_supplement_coverage_tests.rs");
    #[tokio::test]
    async fn sdk_supplement_domain_cache_keeps_live_identity_and_unknown_gates() {
        let (_temp, executor, chain, origin) = fixture().await;
        let request = &chain[1].0;
        let run = executor.workflow_id.as_deref().unwrap();
        let _scope = crate::text_executor::sdk_evidence_cache::scope();
        let first = executor
            .sdk_schema_anchor_for(&origin, Some(request))
            .unwrap();
        assert_eq!(
            executor
                .sdk_schema_anchor_for(&origin, Some(request))
                .unwrap(),
            first
        );
        let c = executor.store.conn().unwrap();
        let (event_id, original_event): (i64, String) = c.query_row(
            "SELECT id,data FROM events WHERE job_id=?1 AND type='series_supplement_identities_frozen'",
            [run], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        c.execute("UPDATE events SET data='{}' WHERE id=?1", [event_id])
            .unwrap();
        let before = executor.store.jobs().unwrap();
        assert!(executor
            .sdk_schema_anchor_for(&origin, Some(request))
            .is_err());
        assert_eq!(executor.store.jobs().unwrap(), before);
        c.execute(
            "UPDATE events SET data=?1 WHERE id=?2",
            params![original_event, event_id],
        )
        .unwrap();
        assert_eq!(
            executor
                .sdk_schema_anchor_for(&origin, Some(request))
                .unwrap(),
            first
        );
        executor
            .store
            .update_job(&origin, "submission_unknown", &json!({}))
            .unwrap();
        let before = executor.store.jobs().unwrap();
        assert!(executor
            .sdk_schema_anchor_for(&origin, Some(request))
            .is_err());
        assert_eq!(executor.store.jobs().unwrap(), before);
    }
    fn save_received(
        executor: &SavedTextExecutor,
        request: &TextRequest,
        raw: &Value,
    ) -> (String, Value) {
        let predecessor = executor
            .sdk_schema_continuation_job(request)
            .unwrap()
            .unwrap();
        let anchor = executor
            .sdk_schema_anchor_for(&predecessor, Some(request))
            .unwrap();
        let identity =
            executor.sdk_job(&predecessor).unwrap()["snapshot"]["executorIdentity"].clone();
        let snapshot = continuation_snapshot(executor, request, &identity, &anchor);
        executor.sdk_schema_continuation_bound(&snapshot).unwrap();
        let job = executor
            .sdk_allocate_pre_dispatch(request, &snapshot)
            .unwrap();
        assert_eq!(executor.request_snapshot(request).unwrap(), snapshot);
        adopt_raw(executor, request, &snapshot, &job, raw);
        (job, snapshot)
    }
    #[tokio::test]
    async fn sdk_series_supplement_exact_raw_reopens_two_review_rounds_and_finisher_without_repurchase(
    ) {
        let (_temp, executor, chain, origin) = fixture().await;
        let original = executor.sdk_job(&origin).unwrap();
        let original_events = events(&executor.store.conn().unwrap(), &origin).unwrap();
        let raw_path = executor
            .store
            .root
            .join("text-runs")
            .join(&origin)
            .join("result.json");
        let original_bytes = std::fs::read(&raw_path).unwrap();
        let mut previous = origin.clone();
        for (request, raw) in chain.iter().skip(1) {
            assert_eq!(
                executor.sdk_schema_continuation_job(request).unwrap(),
                Some(previous.clone())
            );
            let (job, snapshot) = save_received(&executor, request, raw);
            let count = executor.store.jobs().unwrap().len();
            let reopened = SavedTextExecutor {
                store: Store::open(executor.store.root.clone()).unwrap(),
                settings: executor.settings.clone(),
                workflow_id: executor.workflow_id.clone(),
            };
            assert_eq!(reopened.request_snapshot(request).unwrap(), snapshot);
            assert_eq!(
                reopened.execute(request.clone()).await.unwrap(),
                *raw,
                "saved real-shaped SDK raw is replayed, not resubmitted"
            );
            assert_eq!(
                reopened.sdk_execute(request, &snapshot).await.unwrap(),
                *raw,
                "SDK replay itself verifies the frozen attempt and original raw files"
            );
            assert_eq!(reopened.store.jobs().unwrap().len(), count);
            assert_eq!(reopened.sdk_job(&origin).unwrap(), original);
            assert_eq!(
                events(&reopened.store.conn().unwrap(), &origin).unwrap(),
                original_events
            );
            if matches!(
                request.stage.as_str(),
                "seriesDesign" | "seriesDesignReview"
            ) {
                assert!(
                    reopened.sdk_schema_anchor_for(&job, None).is_err(),
                    "design/review is not a general new-node capability"
                );
                assert_eq!(
                    reopened.sdk_schema_continuation_job(&chain[6].0).unwrap(),
                    None
                );
            }
            previous = job;
        }
        assert_eq!(std::fs::read(raw_path).unwrap(), original_bytes);
        assert!(
            executor.accept(&chain[0].0, &chain[0].1).await.is_err(),
            "supplement does not promote the rejected author"
        );
        assert_eq!(executor.sdk_job(&origin).unwrap(), original);
        assert_eq!(
            std::fs::read_dir(executor.store.root.join("sdk-worker"))
                .unwrap()
                .count(),
            1,
            "only the original synthetic refusal directory exists; no SDK worker was launched"
        );
    }
    #[tokio::test]
    async fn sdk_series_supplement_cannot_skip_review_or_reuse_design_as_an_arbitrary_anchor() {
        let (_temp, executor, chain, origin) = fixture().await;
        let original = executor.sdk_job(&origin).unwrap();
        assert!(
            executor
                .sdk_schema_anchor_for(&origin, Some(&chain[5].0))
                .is_err(),
            "finisher cannot skip missing design and independent review"
        );
        assert!(
            executor.sdk_schema_continuation_job(&chain[4].0).is_err(),
            "review1 cannot borrow design0"
        );
        let (design, _) = save_received(&executor, &chain[1].0, &chain[1].1);
        assert!(executor
            .sdk_schema_anchor_for(&design, Some(&chain[6].0))
            .is_err());
        let (review, _) = save_received(&executor, &chain[2].0, &chain[2].1);
        assert_eq!(chain[2].1["conclusion"], "revise");
        assert!(
            executor
                .sdk_schema_anchor_for(&review, Some(&chain[5].0))
                .is_err(),
            "revise is not a pass review"
        );
        save_received(&executor, &chain[3].0, &chain[3].1);
        let (pass, _) = save_received(&executor, &chain[4].0, &chain[4].1);
        executor
            .store
            .update_job(&pass, "received", &json!({"receipt":null,"error":null}))
            .unwrap();
        assert!(
            executor
                .sdk_schema_anchor_for(&pass, Some(&chain[5].0))
                .is_err(),
            "received review is not adopted/accepted"
        );
        assert_eq!(executor.sdk_job(&origin).unwrap(), original);
    }
    #[tokio::test]
    async fn sdk_series_supplement_source_rules_origin_budget_and_raw_drift_create_nothing() {
        let (_temp, executor, chain, origin) = fixture().await;
        let before = executor.store.jobs().unwrap();
        for fault in [
            "origin-raw",
            "origin-request",
            "previous-raw",
            "previous-request",
            "before-assets",
            "source",
            "episode",
            "round",
            "round-budget",
            "attempt",
            "attempt-budget",
            "context",
            "packet",
            "rules",
            "schema",
            "model",
            "role",
        ] {
            let mut request = chain[1].0.clone();
            match fault {
                "origin-raw" => {
                    request.input["seriesAssetSupplement"]["originRawSha256"] =
                        json!("0".repeat(64))
                }
                "origin-request" => {
                    request.input["seriesAssetSupplement"]["originRequestSha256"] =
                        json!("0".repeat(64))
                }
                "previous-raw" => {
                    request.input["seriesAssetSupplement"]["predecessorRawSha256"] =
                        json!("0".repeat(64))
                }
                "previous-request" => {
                    request.input["seriesAssetSupplement"]["predecessorRequestSha256"] =
                        json!("0".repeat(64))
                }
                "before-assets" => {
                    request.input["seriesAssetSupplement"]["beforeAssetsSha256"] =
                        json!("0".repeat(64))
                }
                "source" => request.input["seriesAssetSupplement"]["sourceSha256"] = json!("other"),
                "episode" => request.input["seriesAssetSupplement"]["episodeId"] = json!("other"),
                "round" => request.input["seriesAssetSupplement"]["round"] = json!(1),
                "round-budget" => {
                    request.input["seriesAssetSupplement"]["maximumRounds"] = json!(2)
                }
                "attempt" => request.input["seriesAssetSupplement"]["attempt"] = json!(3),
                "attempt-budget" => {
                    request.input["seriesAssetSupplement"]["maximumAttempts"] = json!(3)
                }
                "context" => request.input["freshContext"] = json!(false),
                "packet" => request.input["packet"]["sourceSha256"] = json!("replaced-source"),
                "rules" => request.rule_snapshot = json!({}),
                "schema" => request.schema = json!({}),
                "model" => request.model = "different".into(),
                "role" => request.stage = "seriesAuthor".into(),
                _ => unreachable!(),
            }
            assert!(
                executor
                    .sdk_schema_anchor_for(&origin, Some(&request))
                    .is_err(),
                "{fault}"
            );
            assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        }
    }
    #[tokio::test]
    async fn sdk_series_supplement_unknown_wrong_workflow_and_changed_receipt_cannot_dispatch() {
        for fault in [
            "unknown",
            "workflow",
            "receipt",
            "raw",
            "checkpoint",
            "account",
        ] {
            let (_temp, mut executor, chain, origin) = fixture().await;
            match fault {
                "unknown"=>executor.store.update_job(&origin,"submission_unknown",&json!({})).unwrap(),
                "workflow"=>executor.workflow_id=Some("other-workflow".into()),
                "receipt"=>executor.store.update_job(&origin,"rejected",&json!({"receipt":{"contract":"desktop-runner/v1","validation":"rejected","transportValidation":"invalid_json"}})).unwrap(),
                "raw"=>std::fs::write(executor.store.root.join("text-runs").join(&origin).join("result.json"),"{}").unwrap(),
                "checkpoint"=>{executor.store.conn().unwrap().execute("UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",[&origin]).unwrap();},
                "account"=>executor.settings["textSdkIdentity"]["accountFingerprint"]=json!("f".repeat(64)),_=>unreachable!(),
            }
            let before = executor.store.jobs().unwrap();
            assert!(
                executor
                    .sdk_schema_anchor_for(&origin, Some(&chain[1].0))
                    .is_err(),
                "{fault}"
            );
            assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        }
    }

    async fn replay_fixture() -> (
        tempfile::TempDir,
        SavedTextExecutor,
        Vec<(TextRequest, Value)>,
        String,
        String,
    ) {
        let (temp, executor, chain, origin) = fixture().await;
        let (job, snapshot) = save_received(&executor, &chain[1].0, &chain[1].1);
        let mut prepared = executor.sdk_job(&job).unwrap()["prepared"].clone();
        prepared.as_object_mut().unwrap().remove("sdkDomainResult");
        prepared
            .as_object_mut()
            .unwrap()
            .remove("sdkDomainResultSha256");
        executor
            .store
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET prepared=?1 WHERE id=?2",
                params![prepared.to_string(), job],
            )
            .unwrap();
        let attempt = FrozenAttempt::load(
            &executor
                .store
                .root
                .join("text-runs")
                .join(&job)
                .join("sdk-attempt.json"),
        )
        .unwrap();
        executor
            .store
            .event(&job, "sdk_raw_result", attempt.raw_result.as_ref().unwrap())
            .unwrap();
        reject_synthetic(
            &executor,
            &chain[1].0,
            &snapshot,
            &job,
            "SERIES_SUPPLEMENT_FACT_NOT_SOURCE_QUOTE",
        );
        (temp, executor, chain, origin, job)
    }
    fn file_state(executor: &SavedTextExecutor, job: &str) -> Value {
        json!(["sdk-attempt.json", "sdk-compiled.json", "result.json"]
            .iter()
            .map(|file| crate::store::hash_file(
                &executor.store.root.join("text-runs").join(job).join(file)
            )
            .unwrap())
            .collect::<Vec<_>>())
    }
    #[tokio::test]
    async fn sdk_series_supplement_revalidation_adopts_original_once_and_reopens_without_dispatch()
    {
        let (_temp, executor, chain, origin, job) = replay_fixture().await;
        let (request, raw) = &chain[1];
        let old = executor.sdk_job(&job).unwrap();
        let original = executor.sdk_job(&origin).unwrap();
        let prior_events = events(&executor.store.conn().unwrap(), &job).unwrap();
        let files = file_state(&executor, &job);
        assert_eq!(executor.execute(request.clone()).await.unwrap(), *raw);
        assert_eq!(
            executor.sdk_job(&job).unwrap(),
            old,
            "execute replays but does not promote"
        );
        executor.accept(request, raw).await.unwrap();
        let current = executor.sdk_job(&job).unwrap();
        assert_eq!(current["status"], "completed");
        assert_eq!(current["result"], old["result"]);
        assert_eq!(current["snapshot"], old["snapshot"]);
        assert_eq!(current["prepared"]["sdkDomainResult"], *raw);
        let reopened = SavedTextExecutor {
            store: Store::open(executor.store.root.clone()).unwrap(),
            settings: executor.settings.clone(),
            workflow_id: executor.workflow_id.clone(),
        };
        for _ in 0..2 {
            assert_eq!(reopened.execute(request.clone()).await.unwrap(), *raw);
            assert_eq!(
                reopened
                    .sdk_execute(request, &current["snapshot"])
                    .await
                    .unwrap(),
                *raw
            );
            reopened.accept(request, raw).await.unwrap();
        }
        let after = events(&executor.store.conn().unwrap(), &job).unwrap();
        assert_eq!(
            &after.as_array().unwrap()[..prior_events.as_array().unwrap().len()],
            prior_events.as_array().unwrap()
        );
        let audits: Vec<_> = after
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "sdk_series_supplement_revalidated")
            .collect();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0]["data"]["originalJob"]["error"], old["error"]);
        assert_eq!(audits[0]["data"]["generationRequestsSent"], 0);
        assert_eq!(file_state(&reopened, &job), files);
        assert_eq!(reopened.sdk_job(&origin).unwrap(), original);
        assert_eq!(
            reopened.store.jobs().unwrap().len(),
            executor.store.jobs().unwrap().len()
        );
        assert_eq!(
            std::fs::read_dir(executor.store.root.join("sdk-worker"))
                .unwrap()
                .count(),
            1
        );
        // The new accepted design remains usable only for its actual review.
        save_received(&reopened, &chain[2].0, &chain[2].1);
    }
    #[tokio::test]
    async fn sdk_series_supplement_revalidation_rejects_other_reason_unknown_raw_and_proof_drift() {
        for fault in [
            "reason",
            "unknown",
            "callback",
            "raw-file",
            "raw-event",
            "identity-proof",
        ] {
            let (_temp, executor, chain, _origin, job) = replay_fixture().await;
            let (request, raw) = &chain[1];
            let mut supplied = raw.clone();
            match fault {
                "reason" => {
                    let snapshot = executor.sdk_job(&job).unwrap()["snapshot"].clone();
                    reject_synthetic(
                        &executor,
                        request,
                        &snapshot,
                        &job,
                        "SOME_OTHER_DOMAIN_REJECTION",
                    );
                }
                "unknown" => executor
                    .store
                    .update_job(&job, "submission_unknown", &json!({}))
                    .unwrap(),
                "callback" => supplied["unexpected"] = json!(true),
                "raw-file" => std::fs::write(
                    executor
                        .store
                        .root
                        .join("text-runs")
                        .join(&job)
                        .join("result.json"),
                    "{}",
                )
                .unwrap(),
                "raw-event" => {
                    executor
                        .store
                        .conn()
                        .unwrap()
                        .execute(
                            "UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_raw_result'",
                            [&job],
                        )
                        .unwrap();
                }
                "identity-proof" => {
                    executor.store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='series_supplement_identities_frozen'",[executor.workflow_id.as_deref().unwrap()]).unwrap();
                }
                _ => unreachable!(),
            }
            let before = executor.store.jobs().unwrap();
            let evidence = events(&executor.store.conn().unwrap(), &job).unwrap();
            assert!(
                executor.accept(request, &supplied).await.is_err(),
                "{fault}"
            );
            assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
            assert_eq!(
                events(&executor.store.conn().unwrap(), &job).unwrap(),
                evidence,
                "{fault}"
            );
        }
    }
    #[tokio::test]
    async fn sdk_series_supplement_revalidation_receipt_requires_unique_audit_and_original_events()
    {
        for fault in ["audit", "duplicate", "receipt", "original-event"] {
            let (_temp, executor, chain, _origin, job) = replay_fixture().await;
            let (request, raw) = &chain[1];
            executor.accept(request, raw).await.unwrap();
            match fault {
                "audit" => {
                    executor.store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_series_supplement_revalidated'",[&job]).unwrap();
                }
                "duplicate" => {
                    executor.store.conn().unwrap().execute("INSERT INTO events(job_id,type,data,created_at) SELECT job_id,type,data,created_at FROM events WHERE job_id=?1 AND type='sdk_series_supplement_revalidated'",[&job]).unwrap();
                }
                "receipt" => {
                    let mut receipt = executor.sdk_job(&job).unwrap()["receipt"].clone();
                    receipt["migrationAuditSha256"] = json!("0".repeat(64));
                    executor
                        .store
                        .update_job(&job, "completed", &json!({"receipt":receipt}))
                        .unwrap();
                }
                "original-event" => {
                    executor
                        .store
                        .conn()
                        .unwrap()
                        .execute(
                            "UPDATE events SET data='{}' WHERE job_id=?1 AND type='sdk_raw_result'",
                            [&job],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let before = executor.store.jobs().unwrap();
            let snapshot = executor.sdk_job(&job).unwrap()["snapshot"].clone();
            assert!(
                executor.sdk_execute(request, &snapshot).await.is_err(),
                "{fault}"
            );
            assert_eq!(executor.store.jobs().unwrap(), before, "{fault}");
        }
    }
    #[tokio::test]
    async fn sdk_series_supplement_review_digest_upgrade_restores_exact_old_or_new_frozen_request()
    {
        let (_temp, mut executor, chain, _origin) = fixture().await;
        save_received(&executor, &chain[1].0, &chain[1].1);
        let current = &chain[2].0;
        let legacy = crate::pipeline::series::legacy_supplement_review_request(current)
            .unwrap()
            .unwrap();
        let (job, snapshot) = save_received(&executor, &legacy, &chain[2].1);
        assert_eq!(
            serde_json::to_value(
                executor
                    .prepare_transport_request(current.clone())
                    .await
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(&legacy).unwrap()
        );
        assert_eq!(executor.request_snapshot(&legacy).unwrap(), snapshot);
        // Restoring a request does not resolve UNKNOWN or grant resubmission.
        executor
            .store
            .update_job(&job, "submission_unknown", &json!({}))
            .unwrap();
        let before = executor.store.jobs().unwrap();
        let restored = executor
            .prepare_transport_request(current.clone())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&legacy).unwrap()
        );
        assert!(executor.sdk_commit(&restored, &chain[2].1, None).is_err());
        assert_eq!(executor.store.jobs().unwrap(), before);
        let mut changed = current.clone();
        changed.input["task"] = json!("changed");
        assert!(executor.prepare_transport_request(changed).await.is_err());
        executor.settings["textSdkIdentity"]["accountFingerprint"] = json!("f".repeat(64));
        assert!(executor
            .prepare_transport_request(current.clone())
            .await
            .is_err());
        let (_temp, executor, chain, _origin) = fixture().await;
        save_received(&executor, &chain[1].0, &chain[1].1);
        save_received(&executor, &chain[2].0, &chain[2].1);
        assert_eq!(
            serde_json::to_value(
                executor
                    .prepare_transport_request(chain[2].0.clone())
                    .await
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(&chain[2].0).unwrap()
        );
    }
    #[tokio::test]
    async fn sdk_series_supplement_v2_existing_supply_reopens_full_chain_with_frozen_design_identity(
    ) {
        let (input, chain) = crate::pipeline::series::series_supply_fixture().await;
        let assets = input["assets"].as_array().unwrap().clone();
        let (_temp, executor, origin, original, next) =
            fixture_request_input(chain[0].0.clone(), Some(input));
        for asset in assets {
            let saved = executor.store.save_asset(asset).unwrap();
            assert_eq!(saved["version"], 1);
        }
        let job = executor
            .sdk_reserve_schema_successor_checked(&origin, &original, &next, || Ok(()))
            .unwrap();
        adopt_raw(&executor, &origin, &next, &job, &chain[0].1);
        reject_synthetic(
            &executor,
            &origin,
            &next,
            &job,
            "SERIES_CREATIVE_DEPENDENCY_UNRESOLVED",
        );
        let known = packet(&chain[1].0, "knownAssetCatalog");
        let context = executor
            .freeze_series_supplement_identity_context(&origin, &chain[0].1, known, CONTRACT_V2)
            .unwrap();
        assert_eq!(context, *packet(&chain[1].0, "identityReservations"));
        assert_eq!(context["contractVersion"], CONTRACT_V2);
        assert_eq!(
            context["assetStates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["assetId"] == "existing-key")
                .unwrap()["designUsable"],
            true
        );
        assert!(
            known
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a.get("adoptedDesign").is_none()),
            "full supply is selected separately from the compact catalogue"
        );
        let original_author = executor.sdk_job(&job).unwrap();
        let mut bad = chain[1].0.clone();
        bad.input["seriesAssetSupplement"]["version"] = json!(CONTRACT);
        let count = executor.store.jobs().unwrap().len();
        assert!(executor.sdk_schema_anchor_for(&job, Some(&bad)).is_err());
        assert_eq!(executor.store.jobs().unwrap().len(), count);
        let mut previous = job.clone();
        for (request, raw) in chain.iter().skip(1) {
            assert_eq!(
                executor.sdk_schema_continuation_job(request).unwrap(),
                Some(previous.clone())
            );
            let (id, snapshot) = save_received(&executor, request, raw);
            let count = executor.store.jobs().unwrap().len();
            let reopened = SavedTextExecutor {
                store: Store::open(executor.store.root.clone()).unwrap(),
                settings: executor.settings.clone(),
                workflow_id: executor.workflow_id.clone(),
            };
            assert_eq!(
                reopened.sdk_execute(request, &snapshot).await.unwrap(),
                *raw
            );
            assert_eq!(reopened.store.jobs().unwrap().len(), count);
            if matches!(
                request.stage.as_str(),
                "seriesDesign" | "seriesDesignReview"
            ) {
                assert!(reopened.sdk_schema_anchor_for(&id, None).is_err());
            }
            previous = id;
        }
        assert_eq!(
            executor.sdk_job(&job).unwrap(),
            original_author,
            "supplying an existing identity never adopts or replaces the rejected author"
        );
        assert_eq!(
            executor
                .store
                .conn()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM assets WHERE id='existing-key'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            std::fs::read_dir(executor.store.root.join("sdk-worker"))
                .unwrap()
                .count(),
            1,
            "synthetic original refusal only; no model worker spawned"
        );
    }
}
