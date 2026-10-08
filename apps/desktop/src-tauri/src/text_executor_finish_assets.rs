use super::*;

const POLICY: &str = "finisher-asset-input-projection/v1";
const BOUNDARY_POLICY: &str = "finisher-boundary-input-projection/v1";
const BOUNDARY_PROFILE: &str = "program-export-boundary/v1";
const BOUNDARY_EXPORT_FIELDS: [&str; 6] = [
    "prompt",
    "channelPromptTemplate",
    "referenceBindings",
    "visualStyle",
    "seedanceExportVersion",
    "storyboardCreativeProfile",
];
const API_EXISTING_NODE_HELD: &str = "API_FINISH_EXISTING_NODE_HELD: 原整编API节点尚未处理完，或原采用缓存证据不完整；保留原请求、回包和预算，不创建或重发任务";
fn held() -> String {
    "FINISH_ASSET_PROJECTION_UNBOUND: 整编资产卡与原流程或已存在请求不一致；保留原任务，未提交新请求".into()
}

/// Reconstruct the already supported asset-only request, not a replacement for
/// a persisted request. Every boundary edit must be a proven top-level export
/// removal; creative material and the asset-v1 proof remain byte-equivalent.
fn asset_only_finish_request(legacy: &TextRequest, projected: &TextRequest) -> Result<TextRequest> {
    if legacy.input.get("finisherBoundaryInputProfile").is_some()
        || legacy.input.get("finisherBoundaryInputProjection").is_some()
    {
        return Err(held());
    }
    if projected.input.get("finisherBoundaryInputProfile").is_none()
        && projected.input.get("finisherBoundaryInputProjection").is_none()
    {
        return Ok(projected.clone());
    }
    let original_boundary = legacy.input.get("nextBoundary").ok_or_else(held)?;
    let mut stripped_boundary = original_boundary.clone();
    let boundary = stripped_boundary.as_object_mut().ok_or_else(held)?;
    let mut removed = false;
    for field in BOUNDARY_EXPORT_FIELDS {
        removed |= boundary.remove(field).is_some();
    }
    if !removed || projected.input.get("nextBoundary") != Some(&stripped_boundary) {
        return Err(held());
    }
    let mut asset_only = projected.clone();
    let input = asset_only.input.as_object_mut().ok_or_else(held)?;
    input.insert("nextBoundary".into(), original_boundary.clone());
    input.remove("finisherBoundaryInputProfile");
    input.remove("finisherBoundaryInputProjection");
    let asset_only_text = serde_json::to_string(&asset_only).map_err(|_| held())?;
    let expected = json!({
        "version":BOUNDARY_POLICY,"profile":BOUNDARY_PROFILE,
        "sourceSha256":asset_only.input["finisherAssetInputProjection"]["sourceSha256"],
        "episodeId":asset_only.input["episodeId"],
        "frozenWorkflowInputSha256":asset_only.input["finisherAssetInputProjection"]["frozenWorkflowInputSha256"],
        "assetOnlyDomainRequestSha256":crate::pipeline::digest(&asset_only_text),
        "originalBoundarySha256":crate::pipeline::digest(&original_boundary.to_string()),
        "projectedBoundarySha256":crate::pipeline::digest(&stripped_boundary.to_string()),
        "removedFields":BOUNDARY_EXPORT_FIELDS,"creativeInputPreserved":true
    });
    if projected.input["finisherBoundaryInputProfile"] != BOUNDARY_PROFILE
        || projected.input["finisherBoundaryInputProjection"] != expected
        || serde_json::to_string(projected).map_err(|_| held())?.encode_utf16().count()
            >= asset_only_text.encode_utf16().count()
    {
        return Err(held());
    }
    Ok(asset_only)
}

impl SavedTextExecutor {
    /// API has no passive reconciliation for an interrupted request. Existing
    /// nodes may only use a proven cache or the original sealed raw recovery;
    /// neither permission authorizes a replacement API task.
    pub(super) fn check_existing_api_finish_node(&self, request: &TextRequest) -> Result<()> {
        if s(&self.settings, "textProvider") != "api" || request.stage != "storyboardFinish" {
            return Ok(());
        }
        let Some(run) = self.workflow_id.as_deref() else {
            return Ok(());
        };
        let c = self.store.conn()?;
        let saved: Option<Value> = c.query_row(
            "SELECT w.stage,w.request_hash,w.state,w.result,w.error,j.id,j.kind,j.provider,j.state,j.snapshot,j.result,j.receipt,j.error FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",
            params![run,step_key(request)], |row| Ok(json!({
                "stage":row.get::<_,String>(0)?,"requestHash":row.get::<_,String>(1)?,
                "stepState":row.get::<_,String>(2)?,"accepted":row.get::<_,Option<String>>(3)?,
                "stepError":row.get::<_,Option<String>>(4)?,"jobId":row.get::<_,Option<String>>(5)?,
                "kind":row.get::<_,Option<String>>(6)?,"provider":row.get::<_,Option<String>>(7)?,
                "jobState":row.get::<_,Option<String>>(8)?,"snapshot":row.get::<_,Option<String>>(9)?,
                "result":row.get::<_,Option<String>>(10)?,"receipt":row.get::<_,Option<String>>(11)?,
                "jobError":row.get::<_,Option<String>>(12)?
            })),
        ).optional().map_err(|e|e.to_string())?;
        let Some(saved) = saved else {
            return Ok(());
        };
        let held = || API_EXISTING_NODE_HELD.to_owned();
        let expected = self.request_snapshot(request).map_err(|_|held())?;
        let expected_hash = crate::pipeline::digest(&expected.to_string());
        if saved["stepState"] != "accepted" || saved["jobState"] != "completed" {
            // These read-only predicates own the original raw, domain and
            // persistent attempt budget. In particular, an unresolved bounded
            // attempt must retain its precise no-resend diagnosis.
            let raw = if let Some((_, value)) =
                self.scope_rejected_author_result(request, &expected, &expected_hash)?
            {
                Some(value)
            } else if let Some(replay) =
                self.scope_group_validator_replay(request, &expected, &expected_hash)?
            {
                Some(replay.value)
            } else if let Some((_, value, _)) =
                self.carrier_rejected_result(request, &expected, &expected_hash)?
            {
                Some(value)
            } else {
                self.carrier_received_result(request, &expected, &expected_hash)?
                    .map(|replay| replay.value)
            };
            let Some(raw) = raw else { return Err(held()); };
            let event: Option<String> = c.query_row(
                "SELECT data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",
                [s(&saved,"jobId")],|row|row.get(0),
            ).optional().map_err(|e|e.to_string())?;
            let event: Value = serde_json::from_str(event.as_deref().ok_or_else(held)?).map_err(|_|held())?;
            let parent_bound: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run')",
                [run], |row|row.get(0),
            ).map_err(|e|e.to_string())?;
            if saved["stage"] != request.stage || saved["kind"] != "text"
                || saved["provider"] != "api" || !parent_bound
                || s(&saved,"snapshot") != expected.to_string()
                || saved["requestHash"] != expected_hash
                || serde_json::from_str::<Value>(s(&saved,"result")).map_err(|_|held())? != raw
                || event["requestHash"] != expected_hash || event["value"] != raw
            {
                return Err(held());
            }
            return Ok(());
        }
        if saved["stepState"] != "accepted" || saved["jobState"] != "completed"
            || saved["stage"] != request.stage || saved["kind"] != "text"
            || saved["provider"] != "api" || !saved["stepError"].is_null()
            || !saved["jobError"].is_null()
        {
            return Err(held());
        }
        let snapshot_text = saved["snapshot"].as_str().ok_or_else(held)?;
        let snapshot: Value = serde_json::from_str(snapshot_text).map_err(|_|held())?;
        let hash = crate::pipeline::digest(snapshot_text);
        let receipt: Value = serde_json::from_str(s(&saved,"receipt")).map_err(|_|held())?;
        let accepted: Value = serde_json::from_str(s(&saved,"accepted")).map_err(|_|held())?;
        let result: Value = serde_json::from_str(s(&saved,"result")).map_err(|_|held())?;
        let event: Option<(i64,String)> = c.query_row(
            "SELECT id,data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",
            [s(&saved,"jobId")],|row|Ok((row.get(0)?,row.get(1)?)),
        ).optional().map_err(|e|e.to_string())?;
        let (event_id,event_text) = event.ok_or_else(held)?;
        let event: Value = serde_json::from_str(&event_text).map_err(|_|held())?;
        if snapshot.to_string() != snapshot_text || snapshot != expected
            || saved["requestHash"] != hash
            || receipt["contract"] != "desktop-runner/v1" || receipt["validation"] != "accepted"
            || receipt.get("acceptedRequestHash").is_some_and(|value|value != &json!(hash))
            || event["requestHash"] != hash || event["value"].is_null()
            || accepted.is_null()
            || self.store.accepted_step(run,&hash)?.as_ref() != Some(&accepted)
        {
            return Err(held());
        }
        if receipt["migrationPolicy"].is_null() {
            if accepted != result { return Err(held()); }
        } else {
            // A replay's normalized adopted value belongs only to this exact
            // workflow node. The original raw remains in jobs.result and must
            // never become a generic API result cache.
            let policy = s(&receipt,"migrationPolicy");
            let audit = match policy {
                "carrier-received-validation/v1" if is_carrier_repair(request) =>
                    &receipt["carrierReceivedValidation"],
                "author-scope-received-validation/v1" if is_scope_finisher_repair(request) =>
                    &receipt["authorScopeReceivedValidation"],
                _ => return Err(held()),
            };
            let parent_bound: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run')",
                [run], |row|row.get(0),
            ).map_err(|e|e.to_string())?;
            if !parent_bound || receipt["acceptedRequestHash"] != hash
                || audit["version"] != policy || audit["workflowId"] != run
                || audit["nodeKey"] != step_key(request) || audit["requestHash"] != hash
                || audit["originalSnapshotSha256"] != hash
                || audit["originalResultSha256"] != crate::pipeline::digest(s(&saved,"result"))
                || audit["originalRawEventId"] != event_id
                || audit["generationRequestsSent"] != 0 || audit["acceptedAutomatically"] != false
                || event["value"] != result
            {
                return Err(held());
            }
        }
        Ok(())
    }

    /// Re-check after asynchronous slot/readiness waits. Even a node which was
    /// replayable at entry may not fall through to allocation if its evidence
    /// changes before the original recovery predicate runs.
    pub(super) fn check_api_finish_submission_boundary(&self, request: &TextRequest) -> Result<()> {
        if s(&self.settings,"textProvider") != "api" || request.stage != "storyboardFinish" {
            return Ok(());
        }
        let Some(run) = self.workflow_id.as_deref() else { return Ok(()); };
        let blocked: bool = self.store.conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2) OR EXISTS(SELECT 1 FROM jobs j WHERE j.kind='text' AND j.provider='api' AND j.snapshot=?3 AND NOT EXISTS(SELECT 1 FROM workflow_steps w WHERE w.text_job_id=j.id))",
            params![run,step_key(request),self.request_snapshot(request)?.to_string()],
            |row|row.get(0),
        ).map_err(|e|e.to_string())?;
        if blocked { return Err(API_EXISTING_NODE_HELD.to_owned()); }
        Ok(())
    }

    /// Select only an exact legacy/asset-v1/boundary-v1 domain at the same node. This
    /// never creates a task, spends a repair attempt, or rewrites a frozen input.
    pub(super) fn saved_finish_asset_projection(
        &self,
        legacy: TextRequest,
        projected: TextRequest,
    ) -> Result<TextRequest> {
        if serde_json::to_value(&legacy).map_err(|_| held())?
            == serde_json::to_value(&projected).map_err(|_| held())?
        {
            return Ok(legacy);
        }
        let run = self.workflow_id.as_deref().ok_or_else(held)?;
        let c = self.store.conn()?;
        let parent_text: String = c.query_row(
            "SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",
            [run], |r| r.get(0),
        ).map_err(|_| held())?;
        let parent: Value = serde_json::from_str(&parent_text).map_err(|_| held())?;
        let asset_only = asset_only_finish_request(&legacy, &projected)?;
        let proof = &asset_only.input["finisherAssetInputProjection"];
        let sha = crate::pipeline::digest;
        let original = serde_json::to_value(&legacy).map_err(|_| held())?;
        let mut restored = serde_json::to_value(&asset_only).map_err(|_| held())?;
        let expected = json!({
            "version":POLICY,"assetInputProfile":"asset-production-card/v2",
            "sourceSha256":legacy.input["assetCatalogScope"]["sourceSha256"],
            "episodeId":legacy.input["episodeId"],
            "frozenWorkflowInputSha256":sha(&parent["input"].to_string()),
            "legacyDomainRequestSha256":sha(&serde_json::to_string(&legacy).map_err(|_| held())?),
            "projectedAssetsSha256":sha(&asset_only.input["assets"].to_string()),
            "projectedAssetSelectionSha256":sha(&asset_only.input["assetSelection"].to_string()),
            "authorInputPreserved":true
        });
        if legacy.stage != "storyboardFinish"
            || legacy.skill != "episode-finishing-editor"
            || legacy.input.get("finisherAssetInputProjection").is_some()
            || projected.input["storyboardAssetInputProfile"] != "asset-production-card/v2"
            || parent["input"]["storyboardAssetInputProfile"] != "asset-production-card/v2"
            || parent["input"]["settings"] != self.settings
            || parent["input"]["source"]["sha256"]
                != legacy.input["assetCatalogScope"]["sourceSha256"]
            || legacy.input["assetCatalogScope"]["sourceSha256"]
                .as_str()
                .is_none_or(str::is_empty)
            || legacy.input["assetCatalogScope"]["version"] != "asset-production-card/v2"
            || legacy.input["assetCatalogScope"]["episodeId"] != legacy.input["episodeId"]
            || proof != &expected
            || step_key(&legacy) != step_key(&projected)
        {
            return Err(held());
        }
        let input = restored["input"].as_object_mut().ok_or_else(held)?;
        for field in [
            "assets",
            "assetSelection",
            "storyboardAssetInputProfile",
            "finisherAssetInputProjection",
        ] {
            if let Some(value) = legacy.input.get(field) {
                input.insert(field.into(), value.clone());
            } else {
                input.remove(field);
            }
        }
        if restored != original {
            return Err(held());
        }
        let old_domain = self.request_domain_snapshot(&legacy)?;
        let asset_only_domain = self.request_domain_snapshot(&asset_only)?;
        let new_domain = self.request_domain_snapshot(&projected)?;
        let saved: Option<(String, String, String, String, String)> = c.query_row(
            "SELECT w.request_hash,j.snapshot,j.kind,j.provider,w.stage FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",
            params![run,step_key(&legacy)], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional().map_err(|_| held())?;
        if let Some((hash, text, kind, provider, stage)) = saved {
            let snapshot: Value = serde_json::from_str(&text).map_err(|_| held())?;
            let mut domain = snapshot.clone();
            if domain
                .as_object_mut()
                .ok_or_else(held)?
                .remove("executorIdentity")
                .is_none()
                || snapshot.to_string() != text
                || sha(&text) != hash
                || kind != "text"
                || provider != s(&self.settings, "textProvider")
                || stage != legacy.stage
            {
                return Err(held());
            }
            // A supervisor handoff is executor evidence, not creative input.
            // Only its own complete persisted proof may authorize stripping it.
            if snapshot.get("sdkSupervisorHandoff").is_some() {
                domain = self
                    .sdk_handoff_existing_domain(&snapshot)?
                    .ok_or_else(held)?;
            }
            // Unknown/received/rejected/accepted states all retain the original
            // request. The ordinary execute/recovery gates own their disposition.
            let selected = if domain == old_domain {
                legacy
            } else if domain == asset_only_domain {
                asset_only
            } else if domain == new_domain {
                projected
            } else {
                return Err(held());
            };
            self.check_existing_api_finish_node(&selected)?;
            return Ok(selected);
        }
        // An interrupted local insertion must not become a new request simply
        // because its workflow step is missing. Match full domains, not names.
        let mut q = c.prepare("SELECT j.snapshot FROM jobs j WHERE j.kind='text' AND json_extract(j.snapshot,'$.stage')='storyboardFinish' AND NOT EXISTS(SELECT 1 FROM workflow_steps w WHERE w.text_job_id=j.id)").map_err(|_| held())?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| held())?;
        for row in rows {
            let mut domain: Value =
                serde_json::from_str(&row.map_err(|_| held())?).map_err(|_| held())?;
            domain
                .as_object_mut()
                .ok_or_else(held)?
                .remove("executorIdentity");
            if domain == old_domain || domain == asset_only_domain || domain == new_domain {
                return Err(held());
            }
        }
        Ok(projected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn fixture() -> (
        tempfile::TempDir,
        SavedTextExecutor,
        TextRequest,
        TextRequest,
    ) {
        fixture_at("https://invalid.example")
    }
    fn fixture_at(endpoint: &str) -> (
        tempfile::TempDir,
        SavedTextExecutor,
        TextRequest,
        TextRequest,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project")).unwrap();
        store
            .create("offline finisher projection", "fixture")
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        let settings = json!({"textProvider":"api","textEndpoint":endpoint,"textApiKey":"SYNTHETIC_TEST_KEY_NOT_A_CREDENTIAL"});
        let source_sha = crate::pipeline::digest("甲。乙。");
        let input = json!({"storyboardAssetInputProfile":"asset-production-card/v2","settings":settings,"source":{"sha256":source_sha}});
        let run = store
            .add_job(
                "workflow",
                "storyboard.run",
                "local",
                "local",
                &json!({"input":input}),
                "partially_blocked",
            )
            .unwrap();
        let executor = SavedTextExecutor {
            store,
            settings,
            workflow_id: Some(run),
        };
        let legacy = TextRequest {
            stage: "storyboardFinish".into(),
            skill: "episode-finishing-editor".into(),
            model: "offline".into(),
            reasoning: "medium".into(),
            input: json!({"episodeId":"episode-1","sourceSha256":source_sha,"assets":[{"id":"asset-a","design":{"large":"old design"}}],"assetSelection":{"unchanged":true},"previousDraft":{"units":[{"id":"u-1"}]}}),
            schema: json!({"type":"object"}),
            rule_snapshot: json!({"primary":{"content":"unchanged rules"}}),
        };
        let mut legacy = legacy;
        legacy.input.as_object_mut().unwrap().remove("sourceSha256");
        legacy.input["assetCatalogScope"] = json!({"version":"asset-production-card/v2","sourceSha256":source_sha,"episodeId":"episode-1"});
        let mut projected = legacy.clone();
        projected.input["assets"] = json!([{"id":"asset-a","kind":"CH","subject":"甲"}]);
        projected.input["storyboardAssetInputProfile"] = json!("asset-production-card/v2");
        projected.input["finisherAssetInputProjection"] = json!({"version":POLICY,"assetInputProfile":"asset-production-card/v2","sourceSha256":source_sha,"episodeId":"episode-1","frozenWorkflowInputSha256":crate::pipeline::digest(&input.to_string()),"legacyDomainRequestSha256":crate::pipeline::digest(&serde_json::to_string(&legacy).unwrap()),"projectedAssetsSha256":crate::pipeline::digest(&projected.input["assets"].to_string()),"projectedAssetSelectionSha256":crate::pipeline::digest(&projected.input["assetSelection"].to_string()),"authorInputPreserved":true});
        (dir, executor, legacy, projected)
    }

    fn boundary_projection(asset_only: &TextRequest) -> TextRequest {
        let mut projected = asset_only.clone();
        let boundary = projected.input["nextBoundary"].as_object_mut().unwrap();
        for field in BOUNDARY_EXPORT_FIELDS {
            boundary.remove(field);
        }
        projected.input["finisherBoundaryInputProfile"] = json!(BOUNDARY_PROFILE);
        projected.input["finisherBoundaryInputProjection"] = json!({
            "version":BOUNDARY_POLICY,"profile":BOUNDARY_PROFILE,
            "sourceSha256":asset_only.input["finisherAssetInputProjection"]["sourceSha256"],
            "episodeId":asset_only.input["episodeId"],
            "frozenWorkflowInputSha256":asset_only.input["finisherAssetInputProjection"]["frozenWorkflowInputSha256"],
            "assetOnlyDomainRequestSha256":crate::pipeline::digest(&serde_json::to_string(asset_only).unwrap()),
            "originalBoundarySha256":crate::pipeline::digest(&asset_only.input["nextBoundary"].to_string()),
            "projectedBoundarySha256":crate::pipeline::digest(&projected.input["nextBoundary"].to_string()),
            "removedFields":BOUNDARY_EXPORT_FIELDS,"creativeInputPreserved":true
        });
        projected
    }

    fn boundary_fixture_at(endpoint: &str) -> (
        tempfile::TempDir,
        SavedTextExecutor,
        TextRequest,
        TextRequest,
        TextRequest,
    ) {
        let (dir, executor, mut legacy, mut asset_only) = fixture_at(endpoint);
        legacy.input["nextBoundary"] = json!({
            "id":"SYNTHETIC_ACCEPTED_NEXT_UNIT","initialState":"乙仍站在门内。",
            "shots":[{"id":"next-shot","duration":3,"picture":"乙回应。",
                "dialogueParts":[{"lineId":"next-line","quoteStart":0,"quoteEnd":3,
                    "voiceStart":0,"voiceEnd":2}],"prompt":"nested field remains creative material"}],
            "unknownExtension":{"futureCreativeField":"must remain unchanged"},
            "prompt":"SYNTHETIC_PROGRAM_EXPORT ".repeat(150),
            "channelPromptTemplate":{"main":"SYNTHETIC_CHANNEL_EXPORT ".repeat(150)},
            "referenceBindings":[{"imageId":"synthetic-binding"}],
            "visualStyle":"SYNTHETIC_STYLE","seedanceExportVersion":"synthetic-export/v1",
            "storyboardCreativeProfile":"SYNTHETIC_PROFILE"
        });
        asset_only.input["nextBoundary"] = legacy.input["nextBoundary"].clone();
        asset_only.input["finisherAssetInputProjection"]["legacyDomainRequestSha256"] =
            json!(crate::pipeline::digest(&serde_json::to_string(&legacy).unwrap()));
        let projected = boundary_projection(&asset_only);
        (dir, executor, legacy, asset_only, projected)
    }
    fn save(
        executor: &SavedTextExecutor,
        request: &TextRequest,
        state: &str,
        step: bool,
    ) -> String {
        let mut snapshot = executor.request_domain_snapshot(request).unwrap();
        let provider = s(&executor.settings, "textProvider");
        snapshot["executorIdentity"] =
            json!({"provider":provider,"endpoint":s(&executor.settings,"textEndpoint").trim_end_matches('/')});
        let job = executor
            .store
            .add_job("text", "fixture", provider, "local", &snapshot, state)
            .unwrap();
        if step {
            executor
                .store
                .checkpoint(
                    executor.workflow_id.as_deref().unwrap(),
                    &step_key(request),
                    &request.stage,
                    &crate::pipeline::digest(&snapshot.to_string()),
                    state,
                    Some(&job),
                    None,
                    None,
                )
                .unwrap();
        }
        job
    }

    fn complete(executor: &SavedTextExecutor, request: &TextRequest, job: &str) -> Value {
        let value = json!({"savedAcceptedValue":"SYNTHETIC_ORIGINAL_EDITOR_RESULT"});
        let snapshot = executor.request_snapshot(request).unwrap();
        let hash = crate::pipeline::digest(&snapshot.to_string());
        executor.store.update_job(job,"completed",&json!({"result":value,
            "receipt":{"contract":"desktop-runner/v1","validation":"accepted"},"error":null})).unwrap();
        executor.store.checkpoint(executor.workflow_id.as_deref().unwrap(),&step_key(request),
            &request.stage,&hash,"accepted",Some(job),Some(&value),None).unwrap();
        executor.store.event(job,"text_received_raw",&json!({"requestHash":hash,"value":value})).unwrap();
        value
    }

    fn recovery_fixture_at(endpoint: &str, family: &str) -> (
        tempfile::TempDir, SavedTextExecutor, TextRequest, TextRequest, TextRequest,
    ) {
        let (dir,e,mut old,mut asset_only,_) = boundary_fixture_at(endpoint);
        for request in [&mut old,&mut asset_only] {
            request.rule_snapshot = crate::pipeline::snapshot("episode-finishing-editor",&[]).unwrap();
            let (batch,field,version) = match family {
                "carrier" => ("carrier-repair/v1","carrierRepair","accepted-finisher-carrier-repair/v1"),
                "author" => ("author-domain-repair/v1","authorRepair","rejected-author-scope-repair/v1"),
                "finish" => ("finish-domain-repair/v1","finishRepair","rejected-finish-scope-repair/v1"),
                _ => ("related-range-finish/v1","",""),
            };
            request.input["storyboardCreativeProfile"] = json!("original-production-contract/v1");
            request.input["executionBatch"] = json!({"version":batch,"id":"sealed-original-attempt"});
            if !field.is_empty() {
                request.input[field] = json!({"version":version,"attempt":1,"maximumAttempts":2});
            }
        }
        asset_only.input["finisherAssetInputProjection"]["legacyDomainRequestSha256"] =
            json!(crate::pipeline::digest(&serde_json::to_string(&old).unwrap()));
        let new = boundary_projection(&asset_only);
        (dir,e,old,asset_only,new)
    }

    fn receive(executor: &SavedTextExecutor, request: &TextRequest, job: &str) -> Value {
        let raw = json!({"sealedRaw":"SYNTHETIC_ORIGINAL_UNADOPTED_FINISHER"});
        let hash = crate::pipeline::digest(&executor.request_snapshot(request).unwrap().to_string());
        executor.store.update_job(job,"received",&json!({"result":raw})).unwrap();
        executor.store.event(job,"text_received_raw",&json!({"requestHash":hash,"value":raw})).unwrap();
        raw
    }

    fn evidence(executor: &SavedTextExecutor) -> Value {
        let c = executor.store.conn().unwrap();
        let mut steps = c.prepare("SELECT workflow_id,node_key,stage,request_hash,state,text_job_id,result,error,updated_at FROM workflow_steps ORDER BY rowid").unwrap();
        let steps: Vec<_> = steps.query_map([],|row|Ok(json!([
            row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,
            row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,Option<String>>(5)?,
            row.get::<_,Option<String>>(6)?,row.get::<_,Option<String>>(7)?,row.get::<_,String>(8)?
        ]))).unwrap().collect::<std::result::Result<_,_>>().unwrap();
        let mut events = c.prepare("SELECT id,job_id,type,data,created_at FROM events ORDER BY id").unwrap();
        let events: Vec<_> = events.query_map([],|row|Ok(json!([
            row.get::<_,i64>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,String>(2)?,
            row.get::<_,String>(3)?,row.get::<_,String>(4)?
        ]))).unwrap().collect::<std::result::Result<_,_>>().unwrap();
        json!({"jobs":executor.store.jobs().unwrap(),"steps":steps,"events":events})
    }

    /// Loopback only; every attempted request is counted and rejected. The guard
    /// must return before even this synthetic API transport is contacted.
    struct RefusingApi {
        endpoint: String,
        requests: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl RefusingApi {
        fn start() -> Self {
            let listener = std::net::TcpListener::bind(("127.0.0.1",0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = format!("http://{}",listener.local_addr().unwrap());
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let counted = requests.clone();let stopping = stop.clone();
            let worker = std::thread::spawn(move|| {
                while !stopping.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream,_)) => {
                            counted.fetch_add(1,Ordering::SeqCst);
                            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(50)));
                            let mut buffer = [0u8;4096];let _ = stream.read(&mut buffer);
                            let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                        }
                        Err(error) if error.kind()==std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self{endpoint,requests,stop,worker:Some(worker)}
        }
    }
    impl Drop for RefusingApi {
        fn drop(&mut self) {
            self.stop.store(true,Ordering::SeqCst);
            if let Some(worker)=self.worker.take(){let _=worker.join();}
        }
    }
    #[test]
    fn new_finisher_uses_bound_cards_without_creating_jobs_or_changing_parent() {
        let (_dir, e, old, new) = fixture();
        let before = e.store.job(e.workflow_id.as_deref().unwrap()).unwrap();
        let selected = e.saved_finish_asset_projection(old, new.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(selected).unwrap(),
            serde_json::to_value(new).unwrap()
        );
        assert_eq!(
            e.store.job(e.workflow_id.as_deref().unwrap()).unwrap(),
            before
        );
        assert_eq!(
            e.store
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn saved_legacy_and_projected_nodes_keep_exact_requests_including_unknown() {
        for projected in [false, true] {
            for state in ["submission_unknown", "received", "rejected", "completed"] {
                let (_dir, e, old, new) = fixture();
                let expected = if projected { &new } else { &old };
                let job = save(&e, expected, state, true);
                if state == "completed" { complete(&e,expected,&job); }
                let before = e.store.job(&job).unwrap();
                let selected = e.saved_finish_asset_projection(old.clone(), new.clone());
                if state == "completed" {
                    assert_eq!(serde_json::to_value(selected.unwrap()).unwrap(),serde_json::to_value(expected).unwrap());
                } else {
                    assert_eq!(selected.unwrap_err(),API_EXISTING_NODE_HELD,"{state} remains a held original API node");
                }
                assert_eq!(e.store.job(&job).unwrap(), before);
            }
        }
    }
    #[test]
    fn changed_rules_source_previous_draft_and_proof_cannot_change_frozen_nodes() {
        for field in ["rules", "source", "draft", "proof", "parent"] {
            let (_dir, e, old, mut new) = fixture();
            save(&e, &old, "submission_unknown", true);
            match field {
                "rules" => new.rule_snapshot = json!({"changed":true}),
                "source" => new.input["sourceSha256"] = json!("changed"),
                "draft" => new.input["previousDraft"] = json!({"changed":true}),
                "proof" => {
                    new.input["finisherAssetInputProjection"]["projectedAssetsSha256"] =
                        json!("forged")
                }
                _ => {
                    e.store.conn().unwrap().execute("UPDATE jobs SET snapshot=json_set(snapshot,'$.input.settings.textEndpoint','changed') WHERE id=?1",[e.workflow_id.as_deref().unwrap()]).unwrap();
                }
            }
            assert!(
                e.saved_finish_asset_projection(old, new).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn orphan_request_and_changed_existing_request_hold_without_new_dispatch() {
        let (_dir, e, old, new) = fixture();
        save(&e, &old, "submission_unknown", false);
        assert!(e.saved_finish_asset_projection(old, new).is_err());
        let (_dir, e, old, new) = fixture();
        let mut changed = old.clone();
        changed.model = "changed".into();
        save(&e, &changed, "failed", true);
        assert!(e.saved_finish_asset_projection(old, new).is_err());
    }

    #[test]
    fn new_boundary_projection_reconstructs_final_asset_v1_without_mutation() {
        let (_dir, e, old, asset_only, new) = boundary_fixture_at("https://invalid.example");
        let before = evidence(&e);
        let restored = asset_only_finish_request(&old, &new).unwrap();
        assert_eq!(serde_json::to_string(&restored).unwrap(), serde_json::to_string(&asset_only).unwrap());
        let selected = e.saved_finish_asset_projection(old, new.clone()).unwrap();
        assert_eq!(serde_json::to_string(&selected).unwrap(), serde_json::to_string(&new).unwrap());
        assert_eq!(selected.input["nextBoundary"]["shots"], asset_only.input["nextBoundary"]["shots"]);
        assert_eq!(selected.input["nextBoundary"]["unknownExtension"], asset_only.input["nextBoundary"]["unknownExtension"]);
        assert!(selected.input["nextBoundary"]["shots"][0].get("prompt").is_some());
        assert!(serde_json::to_string(&selected).unwrap().encode_utf16().count()
            < serde_json::to_string(&asset_only).unwrap().encode_utf16().count());
        assert_eq!(evidence(&e), before);
    }

    #[test]
    fn asset_v1_without_boundary_exports_keeps_existing_contract() {
        for boundary in [None, Some(Value::Null), Some(json!({"initialState":"synthetic continuity"}))] {
            let (_dir,e,mut old,mut asset_only)=fixture();
            for request in [&mut old,&mut asset_only] {
                if let Some(value)=&boundary {request.input["nextBoundary"]=value.clone();}
                else {request.input.as_object_mut().unwrap().remove("nextBoundary");}
            }
            asset_only.input["finisherAssetInputProjection"]["legacyDomainRequestSha256"] =
                json!(crate::pipeline::digest(&serde_json::to_string(&old).unwrap()));
            let before=evidence(&e);
            assert_eq!(serde_json::to_string(&e.saved_finish_asset_projection(old,asset_only.clone()).unwrap()).unwrap(),serde_json::to_string(&asset_only).unwrap());
            assert!(asset_only.input.get("finisherBoundaryInputProjection").is_none());
            assert_eq!(evidence(&e),before);
        }
    }

    #[test]
    fn three_finisher_domains_preserve_exact_cli_sdk_nodes_including_unknown() {
        for provider in ["codex","codex_sdk"] {
            for profile in ["legacy","asset-v1","boundary-v1"] {
                for state in ["submission_unknown","received","interrupted","rejected","completed"] {
                    let (_dir,mut e,old,mut asset_only,_new)=boundary_fixture_at("https://invalid.example");
                    e.settings["textProvider"]=json!(provider);
                    let mut parent=e.store.job(e.workflow_id.as_deref().unwrap()).unwrap()["snapshot"].clone();
                    parent["input"]["settings"]=e.settings.clone();
                    e.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?1 WHERE id=?2",params![parent.to_string(),e.workflow_id.as_deref().unwrap()]).unwrap();
                    asset_only.input["finisherAssetInputProjection"]["frozenWorkflowInputSha256"]=
                        json!(crate::pipeline::digest(&parent["input"].to_string()));
                    let new=boundary_projection(&asset_only);
                    let request=match profile {"legacy"=>old.clone(),"asset-v1"=>asset_only,_=>new.clone()};
                    save(&e,&request,state,true);
                    let before=evidence(&e);
                    let selected=e.saved_finish_asset_projection(old,new).unwrap();
                    assert_eq!(serde_json::to_string(&selected).unwrap(),serde_json::to_string(&request).unwrap(),"{provider}/{profile}/{state}");
                    assert_eq!(evidence(&e),before,"{provider}/{profile}/{state}");
                }
            }
        }
    }

    #[test]
    fn boundary_projection_tampering_or_non_shrinking_payload_holds() {
        for field in ["profile","missing-profile","missing-proof","policy","asset-hash","original-hash","projected-hash","removed-fields","preserved","extra-proof","source","parent","episode","next-shot","nested-prompt","extension","new-export","rules","asset-proof","asset-profile","legacy-proof","not-smaller"] {
            let (_dir,e,mut old,mut asset_only,mut new)=boundary_fixture_at("https://invalid.example");
            match field {
                "profile" => new.input["finisherBoundaryInputProfile"]=json!("forged"),
                "missing-profile" => {new.input.as_object_mut().unwrap().remove("finisherBoundaryInputProfile");}
                "missing-proof" => {new.input.as_object_mut().unwrap().remove("finisherBoundaryInputProjection");}
                "policy" => new.input["finisherBoundaryInputProjection"]["version"]=json!("forged"),
                "asset-hash" => new.input["finisherBoundaryInputProjection"]["assetOnlyDomainRequestSha256"]=json!("forged"),
                "original-hash" => new.input["finisherBoundaryInputProjection"]["originalBoundarySha256"]=json!("forged"),
                "projected-hash" => new.input["finisherBoundaryInputProjection"]["projectedBoundarySha256"]=json!("forged"),
                "removed-fields" => new.input["finisherBoundaryInputProjection"]["removedFields"]=json!(["prompt"]),
                "preserved" => new.input["finisherBoundaryInputProjection"]["creativeInputPreserved"]=json!(false),
                "extra-proof" => new.input["finisherBoundaryInputProjection"]["extra"]=json!(true),
                "source" => new.input["finisherBoundaryInputProjection"]["sourceSha256"]=json!("forged"),
                "parent" => new.input["finisherBoundaryInputProjection"]["frozenWorkflowInputSha256"]=json!("forged"),
                "episode" => new.input["finisherBoundaryInputProjection"]["episodeId"]=json!("forged"),
                "next-shot" => new.input["nextBoundary"]["shots"][0]["duration"]=json!(9),
                "nested-prompt" => {new.input["nextBoundary"]["shots"][0].as_object_mut().unwrap().remove("prompt");}
                "extension" => {new.input["nextBoundary"].as_object_mut().unwrap().remove("unknownExtension");}
                "new-export" => new.input["nextBoundary"]["prompt"]=json!("not stripped"),
                "rules" => {asset_only.rule_snapshot=json!({"forged":"even when boundary proof is rehashed"});new=boundary_projection(&asset_only);}
                "asset-proof" => {asset_only.input["finisherAssetInputProjection"]["authorInputPreserved"]=json!(false);new=boundary_projection(&asset_only);}
                "asset-profile" => {asset_only.input["storyboardAssetInputProfile"]=json!("forged");new=boundary_projection(&asset_only);}
                "legacy-proof" => old.input["finisherBoundaryInputProjection"]=json!({"forged":true}),
                _ => {
                    for request in [&mut old,&mut asset_only] {
                        for key in BOUNDARY_EXPORT_FIELDS {request.input["nextBoundary"].as_object_mut().unwrap().remove(key);}
                        request.input["nextBoundary"]["prompt"]=json!("");
                    }
                    asset_only.input["finisherAssetInputProjection"]["legacyDomainRequestSha256"]=
                        json!(crate::pipeline::digest(&serde_json::to_string(&old).unwrap()));
                    new=boundary_projection(&asset_only);
                }
            }
            let before=evidence(&e);
            assert!(e.saved_finish_asset_projection(old,new).is_err(),"{field}");
            assert_eq!(evidence(&e),before,"{field}: rejection is read-only");
        }
    }

    #[test]
    fn boundary_projection_orphans_and_unproven_sdk_handoff_hold() {
        for profile in ["legacy","asset-v1","boundary-v1"] {
            let (_dir,e,old,asset_only,new)=boundary_fixture_at("https://invalid.example");
            let request=match profile {"legacy"=>&old,"asset-v1"=>&asset_only,_=>&new};
            save(&e,request,"submission_unknown",false);
            let before=evidence(&e);
            assert!(e.saved_finish_asset_projection(old,new).is_err(),"orphan {profile}");
            assert_eq!(evidence(&e),before);
        }
        // A top-level executor proof is not covered by the creative boundary
        // proof. It still needs the SDK's persisted predecessor/parent evidence.
        for profile in ["legacy","asset-v1","boundary-v1"] {
            let (_dir,mut e,old,mut asset_only,_new)=boundary_fixture_at("https://invalid.example");
            e.settings["textProvider"]=json!("codex_sdk");
            let mut parent=e.store.job(e.workflow_id.as_deref().unwrap()).unwrap()["snapshot"].clone();
            parent["input"]["settings"]=e.settings.clone();
            e.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?1 WHERE id=?2",params![parent.to_string(),e.workflow_id.as_deref().unwrap()]).unwrap();
            asset_only.input["finisherAssetInputProjection"]["frozenWorkflowInputSha256"]=
                json!(crate::pipeline::digest(&parent["input"].to_string()));
            let new=boundary_projection(&asset_only);
            let request=match profile {"legacy"=>&old,"asset-v1"=>&asset_only,_=>&new};
            let job=save(&e,request,"submission_unknown",true);
            let mut snapshot=e.store.job(&job).unwrap()["snapshot"].clone();
            snapshot["sdkSupervisorHandoff"]=json!({"version":"forged"});
            e.store.update_job(&job,"submission_unknown",&json!({"prepared":{"sdkSupervisorHandoff":snapshot["sdkSupervisorHandoff"]}})).unwrap();
            e.store.event(&job,"sdk_supervisor_handoff_frozen",&snapshot["sdkSupervisorHandoff"]).unwrap();
            e.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?1 WHERE id=?2",params![snapshot.to_string(),job]).unwrap();
            e.store.conn().unwrap().execute("UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",params![crate::pipeline::digest(&snapshot.to_string()),job]).unwrap();
            let before=evidence(&e);
            assert!(e.saved_finish_asset_projection(old,new).is_err(),"unproven SDK handoff {profile}");
            assert_eq!(evidence(&e),before);
        }
    }

    #[tokio::test]
    async fn api_existing_unprocessed_finisher_execute_holds_without_dispatch_or_mutation() {
        let server = RefusingApi::start();
        for profile in ["legacy","asset-v1","boundary-v1"] {
            for state in ["queued","running","submitting","submission_unknown","received","interrupted","rejected","failed"] {
                let (_dir,e,old,asset_only,new)=boundary_fixture_at(&server.endpoint);
                let request=match profile {"legacy"=>&old,"asset-v1"=>&asset_only,_=>&new};
                let job=save(&e,request,state,true);
                e.store.update_job(&job,state,&json!({"result":{"originalRaw":"SYNTHETIC_RAW_DO_NOT_REPLACE"},"prepared":{"originalPrepared":"SYNTHETIC_PREPARED_DO_NOT_REPLACE"}})).unwrap();
                let before=evidence(&e);
                assert_eq!(e.saved_finish_asset_projection(old.clone(),new.clone()).unwrap_err(),API_EXISTING_NODE_HELD,"selector {state}");
                assert_eq!(e.execute(request.clone()).await.unwrap_err(),API_EXISTING_NODE_HELD,"execute {state}");
                assert_eq!(evidence(&e),before,"{state}: jobs/checkpoint/prepared/raw/events unchanged");
                assert_eq!(server.requests.load(Ordering::SeqCst),0,"{state}: no loopback API dispatch");
                assert!(!e.store.root.join("text-runs").exists());
            }
        }
    }

    #[tokio::test]
    async fn api_completed_finisher_reuses_only_original_accepted_cache_without_dispatch() {
        let server=RefusingApi::start();
        for profile in ["legacy","asset-v1","boundary-v1"] {
            let (_dir,e,old,asset_only,new)=boundary_fixture_at(&server.endpoint);
            let request=match profile {"legacy"=>&old,"asset-v1"=>&asset_only,_=>&new};
            let job=save(&e,request,"received",true);
            let value=complete(&e,request,&job);
            let before=evidence(&e);
            let selected=e.saved_finish_asset_projection(old.clone(),new.clone()).unwrap();
            assert_eq!(serde_json::to_value(&selected).unwrap(),serde_json::to_value(request).unwrap());
            for _ in 0..2 {assert_eq!(e.execute(selected.clone()).await.unwrap(),value);}
            assert_eq!(evidence(&e),before);
            assert_eq!(server.requests.load(Ordering::SeqCst),0);
            assert!(!e.store.root.join("text-runs").exists());
        }
    }

    #[tokio::test]
    async fn api_completed_finisher_unproven_cache_holds_without_dispatch_or_mutation() {
        let server=RefusingApi::start();
        for changed in ["missing-receipt","missing-event","changed-event-hash","changed-checkpoint-hash","changed-result","changed-identity","unaccepted-step","changed-domain"] {
            let (_dir,e,mut request,_new)=fixture_at(&server.endpoint);
            let job=save(&e,&request,"received",true);complete(&e,&request,&job);
            let c=e.store.conn().unwrap();
            match changed {
                "missing-receipt" => {c.execute("UPDATE jobs SET receipt=NULL WHERE id=?1",[&job]).unwrap();}
                "missing-event" => {c.execute("DELETE FROM events WHERE job_id=?1 AND type='text_received_raw'",[&job]).unwrap();}
                "changed-event-hash" => {c.execute("UPDATE events SET data=json_set(data,'$.requestHash','changed') WHERE job_id=?1 AND type='text_received_raw'",[&job]).unwrap();}
                "changed-checkpoint-hash" => {c.execute("UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",[&job]).unwrap();}
                "changed-result" => {c.execute("UPDATE jobs SET result='{}' WHERE id=?1",[&job]).unwrap();}
                "changed-identity" => {c.execute("UPDATE jobs SET snapshot=json_set(snapshot,'$.executorIdentity.endpoint','http://invalid.example') WHERE id=?1",[&job]).unwrap();}
                "unaccepted-step" => {c.execute("UPDATE workflow_steps SET state='running' WHERE text_job_id=?1",[&job]).unwrap();}
                _ => {request.input["previousDraft"]=json!({"changed":"not original accepted draft"});}
            }
            drop(c);
            let before=evidence(&e);
            assert_eq!(e.execute(request).await.unwrap_err(),API_EXISTING_NODE_HELD,"{changed}");
            assert_eq!(evidence(&e),before,"{changed}: no mutation");
            assert_eq!(server.requests.load(Ordering::SeqCst),0,"{changed}: no dispatch");
        }
    }

    #[tokio::test]
    async fn api_sealed_received_finishers_require_domain_validation_and_keep_zero_dispatch() {
        let server = RefusingApi::start();
        for family in ["carrier","author","finish"] {
            for profile in ["legacy","asset-v1","boundary-v1"] {
                for valid in [false,true] {
                    let (_dir,e,old,asset_only,new) = recovery_fixture_at(&server.endpoint,family);
                    let request = match profile { "legacy"=>old.clone(), "asset-v1"=>asset_only, _=>new.clone() };
                    let job = save(&e,&request,"received",true);
                    let raw = receive(&e,&request,&job);
                    let before = evidence(&e);
                    let selected = e.saved_finish_asset_projection(old,new).unwrap();
                    assert_eq!(serde_json::to_value(&selected).unwrap(),serde_json::to_value(&request).unwrap());
                    assert_eq!(evidence(&e),before,"selection is read-only");
                    assert_eq!(e.check_api_finish_submission_boundary(&selected).unwrap_err(),API_EXISTING_NODE_HELD);
                    let original_job = e.store.job(&job).unwrap();
                    let mut checked = false;
                    let result = crate::pipeline::call_validated(
                        &e,&selected.stage,&selected.skill,&selected.model,&selected.reasoning,
                        selected.input.clone(),selected.schema.clone(),&[],|value| {
                            checked = true;
                            assert_eq!(*value,raw);
                            assert_eq!(e.store.job(&job).unwrap(),original_job,"raw is not adopted by execute");
                            if valid { value["validatedDomain"]=json!(true); Ok(()) }
                            else { Err("strict original-domain rejection".into()) }
                        },
                    ).await;
                    assert!(checked,"{family}/{profile}: original validator runs");
                    if valid {
                        let domain = result.unwrap();
                        let adopted = evidence(&e);
                        for _ in 0..2 {
                            assert_eq!(e.execute(selected.clone()).await.unwrap(),domain);
                            e.accept(&selected,&domain).await.unwrap();
                        }
                        assert_eq!(evidence(&e),adopted,"exact normalized cache is read-only");
                    } else {
                        assert_eq!(result.unwrap_err(),"strict original-domain rejection");
                        let rejected_job = e.store.job(&job).unwrap();
                        assert_eq!(e.execute(selected.clone()).await.unwrap(),raw);
                        e.reject(&selected,&raw,"strict original-domain rejection").await.unwrap();
                        assert!(e.accept(&selected,&raw).await.is_err());
                        assert_eq!(e.store.job(&job).unwrap(),rejected_job);
                    }
                    assert_eq!(e.store.job(&job).unwrap()["snapshot"],original_job["snapshot"]);
                    // Store::job decorates the UI result with mediaIds; compare
                    // sealed model bytes in storage, not that compatibility view.
                    let stored_raw: String = e.store.conn().unwrap().query_row(
                        "SELECT result FROM jobs WHERE id=?1", [&job], |row| row.get(0),
                    ).unwrap();
                    assert_eq!(serde_json::from_str::<Value>(&stored_raw).unwrap(),raw);
                    assert_eq!(e.store.jobs().unwrap().len(),2,"persistent attempt never resets");
                    assert_eq!(server.requests.load(Ordering::SeqCst),0,"{family}/{profile}: no API dispatch");
                    assert!(!e.store.root.join("text-runs").exists());
                }
            }
        }
    }

    #[tokio::test]
    async fn api_sealed_rejected_finishers_keep_original_error_and_budget_without_dispatch() {
        let server = RefusingApi::start();
        for family in ["carrier","author","finish","ordinary"] {
            for profile in ["legacy","asset-v1","boundary-v1"] {
                let (_dir,e,old,asset_only,new) = recovery_fixture_at(&server.endpoint,family);
                let request = match profile { "legacy"=>old.clone(), "asset-v1"=>asset_only, _=>new.clone() };
                let job = save(&e,&request,"received",true);
                let raw = receive(&e,&request,&job);
                let hash = crate::pipeline::digest(&e.request_snapshot(&request).unwrap().to_string());
                e.store.update_job(&job,"rejected",&json!({"error":AUTHOR_ASSET_SCOPE_REJECTION,
                    "receipt":{"contract":"desktop-runner/v1","validation":"rejected"}})).unwrap();
                e.store.checkpoint(e.workflow_id.as_deref().unwrap(),&step_key(&request),&request.stage,
                    &hash,"rejected",Some(&job),Some(&raw),Some(AUTHOR_ASSET_SCOPE_REJECTION)).unwrap();
                let before = evidence(&e);
                let selected = e.saved_finish_asset_projection(old,new).unwrap();
                assert_eq!(serde_json::to_value(&selected).unwrap(),serde_json::to_value(&request).unwrap());
                assert_eq!(evidence(&e),before);
                for _ in 0..2 {
                    assert_eq!(e.execute(selected.clone()).await.unwrap(),raw);
                    e.reject(&selected,&raw,AUTHOR_ASSET_SCOPE_REJECTION).await.unwrap();
                    assert!(e.accept(&selected,&raw).await.is_err());
                }
                let after = evidence(&e);
                assert_eq!(after["jobs"],before["jobs"]);
                assert_eq!(after["steps"],before["steps"]);
                assert_eq!(server.requests.load(Ordering::SeqCst),0);
                assert_eq!(e.store.jobs().unwrap().len(),2);
            }
        }
    }

    #[tokio::test]
    async fn api_normalized_replay_proof_pollution_cannot_dispatch_or_reset_the_node() {
        let server = RefusingApi::start();
        for changed in ["policy","request-hash","event-id","raw-hash","snapshot-hash",
            "workflow","node","calls","automatic","received-event","receipt-hash","parent"] {
            let (_dir,e,_,_,request) = recovery_fixture_at(&server.endpoint,"carrier");
            let job = save(&e,&request,"received",true);
            let raw = receive(&e,&request,&job);
            let mut domain = raw.clone(); domain["validatedDomain"]=json!(true);
            e.accept(&request,&domain).await.unwrap();
            let mut receipt = e.store.job(&job).unwrap()["receipt"].clone();
            let audit = &mut receipt["carrierReceivedValidation"];
            match changed {
                "policy" => receipt["migrationPolicy"]=json!("unproven-policy"),
                "request-hash" => audit["requestHash"]=json!("changed"),
                "event-id" => audit["originalRawEventId"]=json!(-1),
                "raw-hash" => audit["originalResultSha256"]=json!("changed"),
                "snapshot-hash" => audit["originalSnapshotSha256"]=json!("changed"),
                "workflow" => audit["workflowId"]=json!("changed"),
                "node" => audit["nodeKey"]=json!("changed"),
                "calls" => audit["generationRequestsSent"]=json!(1),
                "automatic" => audit["acceptedAutomatically"]=json!(true),
                "received-event" => {e.store.event(&job,"text_received_raw",&json!({"requestHash":"changed","value":raw})).unwrap();}
                "receipt-hash" => receipt["acceptedRequestHash"]=json!("changed"),
                _ => {e.store.conn().unwrap().execute("UPDATE jobs SET entity_id='other-workflow' WHERE id=?1",[e.workflow_id.as_deref().unwrap()]).unwrap();}
            }
            e.store.update_job(&job,"completed",&json!({"receipt":receipt})).unwrap();
            let before = evidence(&e);
            assert_eq!(e.execute(request).await.unwrap_err(),API_EXISTING_NODE_HELD,"{changed}");
            assert_eq!(evidence(&e),before,"{changed}: no state mutation");
            assert_eq!(server.requests.load(Ordering::SeqCst),0);
        }
    }

    #[test]
    fn api_finisher_hold_does_not_apply_to_other_backends_or_stages() {
        let (_dir,mut e,mut request,_new)=fixture();
        save(&e,&request,"submission_unknown",true);
        for provider in ["codex","codex_sdk"] {
            e.settings["textProvider"]=json!(provider);
            assert!(e.check_existing_api_finish_node(&request).is_ok());
        }
        e.settings["textProvider"]=json!("api");request.stage="storyboardCreate".into();
        assert!(e.check_existing_api_finish_node(&request).is_ok());
        request.stage="storyboardFinish".into();e.workflow_id=None;
        assert!(e.check_existing_api_finish_node(&request).is_ok());
    }
}
