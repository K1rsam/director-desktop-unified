//! Source classification is published with its workflow receipt in one CAS.
//! No rewritten source text, historical task replacement or implicit retry.
use super::*;

impl Store {
    /// A later explicit primary-button click may request continuation without
    /// rewriting the completed analysis request or resubmitting its model call.
    pub(crate) fn request_source_analysis_continuation(&self, parent: &str) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let raw:String=tx.query_row("SELECT result FROM jobs WHERE id=?1 AND entity_id='source.analyze' AND state='completed'",[parent],|r|r.get(0)).map_err(|e|e.to_string())?;
        let mut result = parse(raw)?;
        let current: String = tx
            .query_row(
                "SELECT data FROM sources ORDER BY version DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if parse(current)? != result["source"] {
            return Err("SOURCE_ANALYSIS_SOURCE_VERSION_CHANGED".into());
        }
        if result["continuation"]["requested"] != true {
            result["continuation"] = json!({"requested":true,"status":"pending"});
            tx.execute(
                "UPDATE jobs SET result=?1,updated_at=?2 WHERE id=?3",
                params![result.to_string(), now(), parent],
            )
            .map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'source_analysis_continuation_requested','{}',?2)",params![parent,now()]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()
    }

    /// Attach an existing exact-scope foundation recovery instead of minting a
    /// second child or altering its historical frozen snapshot.
    pub(crate) fn link_existing_source_analysis_successor(
        &self,
        parent: &str,
        child: &str,
    ) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (raw,original):(String,String)=tx.query_row("SELECT result,snapshot FROM jobs WHERE id=?1 AND entity_id='source.analyze' AND state='completed'",[parent],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        let mut result = parse(raw)?;
        let original = parse(original)?;
        let raw:String=tx.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='foundation.run'",[child],|r|r.get(0)).map_err(|e|e.to_string())?;
        let frozen = parse(raw)?;
        let current: String = tx
            .query_row(
                "SELECT data FROM sources ORDER BY version DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if result["continuation"]["requested"] != true
            || parse(current)? != result["source"]
            || frozen["input"]["source"] != result["source"]
            || !source_analysis_scope_matches(&original, &frozen)
        {
            return Err("SOURCE_ANALYSIS_CONTINUATION_MISMATCH".into());
        }
        if let Some(existing) = result["continuation"]["workflowId"].as_str() {
            return if existing == child {
                Ok(())
            } else {
                Err("SOURCE_ANALYSIS_SUCCESSOR_ALREADY_FROZEN".into())
            };
        }
        result["continuation"] =
            json!({"requested":true,"status":"reserved","workflowId":child,"existing":true});
        tx.execute(
            "UPDATE jobs SET result=?1,updated_at=?2 WHERE id=?3",
            params![result.to_string(), now(), parent],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'source_analysis_existing_continuation_linked',?2,?3)",params![parent,result["continuation"].to_string(),now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()
    }
    pub(crate) fn source_analysis_unknown(&self, run: &str) -> Result<bool> {
        self.ensure_workflow_storage()?;
        self.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND j.state='submission_unknown')",[run],|r|r.get(0)).map_err(|e|e.to_string())
    }

    pub(crate) fn publish_source_analysis(
        &self,
        run: &str,
        snapshot: &Value,
        output: &Value,
    ) -> Result<Value> {
        if snapshot["runnerVersion"] != "source-analysis-runner/v1"
            || snapshot["stage"] != "source.analyze"
            || output["schemaVersion"] != "source-structure-analysis/v1"
        {
            return Err("SOURCE_ANALYSIS_CONTRACT_MISMATCH".into());
        }
        let old: crate::pipeline::LockedSource =
            serde_json::from_value(snapshot["input"]["source"].clone())
                .map_err(|e| e.to_string())?;
        let next: crate::pipeline::LockedSource =
            serde_json::from_value(output["source"].clone()).map_err(|e| e.to_string())?;
        crate::pipeline::validate_source(&old)?;
        crate::pipeline::validate_source(&next)?;
        let decisions: Vec<crate::pipeline::BoundaryDecision> =
            serde_json::from_value(output["decisions"].clone()).map_err(|e| e.to_string())?;
        let reparsed = crate::pipeline::reparse_source(&old)?;
        let rebuilt = crate::pipeline::apply_boundary_decisions(&reparsed, &decisions)?;
        if old.text != next.text
            || old.sha256 != next.sha256
            || reparsed.parser_version != next.parser_version
            || serde_json::to_value(rebuilt).map_err(|e| e.to_string())? != output["source"]
        {
            return Err("SOURCE_ANALYSIS_SOURCE_CHANGED".into());
        }
        let complete = !next.diagnostics.iter().any(|d| d.blocking)
            && output["unresolved"].as_array().is_some_and(Vec::is_empty);
        if output["status"]
            != if complete {
                "completed"
            } else {
                "partially_blocked"
            }
        {
            return Err("SOURCE_ANALYSIS_STATUS_MISMATCH".into());
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (state,frozen,result):(String,String,Option<String>)=tx.query_row("SELECT state,snapshot,result FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='source.analyze'",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
        if parse(frozen)? != *snapshot {
            return Err("SOURCE_ANALYSIS_FROZEN_INPUT_CHANGED".into());
        }
        if ["completed", "partially_blocked"].contains(&state.as_str()) {
            let saved = parse(result.ok_or("SOURCE_ANALYSIS_RECEIPT_MISSING")?)?;
            if saved["analysisOutputSha256"] != crate::pipeline::digest(&output.to_string()) {
                return Err("SOURCE_ANALYSIS_PUBLICATION_COLLISION".into());
            }
            return Ok(saved);
        }
        let unknown:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND j.state='submission_unknown')",[run],|r|r.get(0)).map_err(|e|e.to_string())?;
        if unknown {
            return Err("SOURCE_ANALYSIS_UNKNOWN_HELD".into());
        }
        if output["usedModel"] == true {
            let accepted:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND stage='sourceStructure' AND state='accepted' AND result IS NOT NULL)",[run],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !accepted {
                return Err("SOURCE_ANALYSIS_ACCEPTED_RECEIPT_MISSING".into());
            }
        }
        let (version, raw): (i64, String) = tx
            .query_row(
                "SELECT version,data FROM sources ORDER BY version DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let current = parse(raw)?;
        if snapshot["sourceVersion"] != version || snapshot["input"]["source"] != current {
            return Err("SOURCE_ANALYSIS_SOURCE_VERSION_CHANGED".into());
        }
        let mut published = output["source"].clone();
        published["version"] = json!(version);
        if published != current {
            published["version"] = json!(version + 1);
            tx.execute(
                "INSERT INTO sources VALUES(?1,?2,?3,?4,?5)",
                params![
                    version + 1,
                    s(&published, "text"),
                    s(&published, "sha256"),
                    published.to_string(),
                    now()
                ],
            )
            .map_err(|e| e.to_string())?;
            // Same invalidation contract as an explicit source.resolve. Do not
            // touch submitted/unknown jobs, media, historical adoption or raw.
            tx.execute("UPDATE jobs SET state='stale',error='原文已更新',updated_at=?1,next_attempt_at=0 WHERE task_id IS NULL AND state IN ('ready','queued','preparing','waiting_dependency') AND kind IN ('video','image','text')",[now()]).map_err(|e|e.to_string())?;
            if source_channel_metadata_only(&current, &published) {
                let foundation:Option<(i64,i64,String)>=tx.query_row("SELECT version,source_version,data FROM foundations ORDER BY version DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
                if let Some((fv, sv, raw)) = foundation {
                    let mut foundation = parse(raw)?;
                    if sv == version && s(&foundation, "sourceSha256") == s(&published, "sha256") {
                        foundation["version"] = json!(fv + 1);
                        foundation["sourceVersion"] = published["version"].clone();
                        foundation["sourceMetadataRebind"] = json!({"version":"source-channel-metadata-rebind/v1","previousSourceVersion":sv,"sourceVersion":published["version"],"sourceSha256":published["sha256"],"fromParser":current["parserVersion"],"toParser":published["parserVersion"],"rawTextAndCoordinatesUnchanged":true,"assetAndMediaVersionsUnchanged":true});
                        tx.execute(
                            "INSERT INTO foundations VALUES(?1,?2,?3,?4)",
                            params![fv + 1, version + 1, foundation.to_string(), now()],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        let requested = snapshot["continueFoundation"] == true;
        let mut result = output.clone();
        result["source"] = published;
        result["analysisOutputSha256"] = json!(crate::pipeline::digest(&output.to_string()));
        result["continuation"] = json!({"requested":requested,"status":if requested && complete {"pending"} else if requested {"blocked"} else {"not_requested"}});
        tx.execute(
            "UPDATE jobs SET state=?1,result=?2,error=NULL,updated_at=?3 WHERE id=?4",
            params![
                if complete {
                    "completed"
                } else {
                    "partially_blocked"
                },
                result.to_string(),
                now(),
                run
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'source_analysis_published',?2,?3)",params![run,json!({"previousSourceVersion":version,"sourceVersion":result["source"]["version"],"sourceSha256":old.sha256,"outputSha256":result["analysisOutputSha256"],"continuation":result["continuation"]}).to_string(),now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()?;
        Ok(result)
    }

    /// Reservation and the parent link commit together. A process exit between
    /// reservation and worker spawn can only resume this same child workflow.
    pub(crate) fn reserve_foundation_after_source_analysis(
        &self,
        parent: &str,
        snapshot: &Value,
    ) -> Result<String> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (raw,original):(String,String)=tx.query_row("SELECT result,snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='source.analyze' AND state='completed'",[parent],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        let mut result = parse(raw)?;
        if result["continuation"]["requested"] != true
            || snapshot["stage"] != "foundation.run"
            || snapshot["sourceAnalysisContinuation"]["workflowId"] != parent
            || snapshot["input"]["source"] != result["source"]
            || !source_analysis_scope_matches(&parse(original)?, snapshot)
        {
            return Err("SOURCE_ANALYSIS_CONTINUATION_MISMATCH".into());
        }
        if let Some(existing) = result["continuation"]["workflowId"].as_str() {
            let frozen:String=tx.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='foundation.run'",[existing],|r|r.get(0)).map_err(|e|e.to_string())?;
            if parse(frozen)? != *snapshot {
                return Err("SOURCE_ANALYSIS_SUCCESSOR_ALREADY_FROZEN".into());
            }
            return Ok(existing.into());
        }
        crate::workflow::verify_guard(&tx, &snapshot["guard"])?;
        let key = id();
        tx.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,created_at,updated_at) VALUES(?1,'workflow','foundation.run','local','queued','local',?2,?3,?3)",params![key,snapshot.to_string(),now()]).map_err(|e|e.to_string())?;
        result["continuation"] = json!({"requested":true,"status":"reserved","workflowId":key});
        tx.execute(
            "UPDATE jobs SET result=?1,updated_at=?2 WHERE id=?3",
            params![result.to_string(), now(), parent],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'source_analysis_continuation_reserved',?2,?3)",params![parent,result["continuation"].to_string(),now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()?;
        Ok(key)
    }
}

fn source_analysis_scope_matches(parent: &Value, child: &Value) -> bool {
    let selected = parent["input"]["selection"]["episodeIds"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| {
            child["input"]["source"]["episodes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|ep| ep["id"].clone())
                .collect()
        });
    child["input"]["selection"]["episodeIds"] == json!(selected)
}
