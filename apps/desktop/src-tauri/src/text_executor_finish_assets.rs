use super::*;

const POLICY: &str = "finisher-asset-input-projection/v1";
fn held() -> String {
    "FINISH_ASSET_PROJECTION_UNBOUND: 整编资产卡与原流程或已存在请求不一致；保留原任务，未提交新请求".into()
}

impl SavedTextExecutor {
    /// Select only the exact old/new domain at the same persisted node. This
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
        let proof = &projected.input["finisherAssetInputProjection"];
        let sha = crate::pipeline::digest;
        let original = serde_json::to_value(&legacy).map_err(|_| held())?;
        let mut restored = serde_json::to_value(&projected).map_err(|_| held())?;
        let expected = json!({
            "version":POLICY,"assetInputProfile":"asset-production-card/v2",
            "sourceSha256":legacy.input["assetCatalogScope"]["sourceSha256"],
            "episodeId":legacy.input["episodeId"],
            "frozenWorkflowInputSha256":sha(&parent["input"].to_string()),
            "legacyDomainRequestSha256":sha(&serde_json::to_string(&legacy).map_err(|_| held())?),
            "projectedAssetsSha256":sha(&projected.input["assets"].to_string()),
            "projectedAssetSelectionSha256":sha(&projected.input["assetSelection"].to_string()),
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
            return if domain == old_domain {
                Ok(legacy)
            } else if domain == new_domain {
                Ok(projected)
            } else {
                Err(held())
            };
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
            if domain == old_domain || domain == new_domain {
                return Err(held());
            }
        }
        Ok(projected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
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
        let settings = json!({"textProvider":"api","textEndpoint":"https://invalid.example"});
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
    fn save(
        executor: &SavedTextExecutor,
        request: &TextRequest,
        state: &str,
        step: bool,
    ) -> String {
        let mut snapshot = executor.request_domain_snapshot(request).unwrap();
        snapshot["executorIdentity"] =
            json!({"provider":"api","endpoint":"https://invalid.example"});
        let job = executor
            .store
            .add_job("text", "fixture", "api", "local", &snapshot, state)
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
                let before = e.store.job(&job).unwrap();
                let selected = e
                    .saved_finish_asset_projection(old.clone(), new.clone())
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(selected).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
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
}
