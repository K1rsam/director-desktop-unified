//! Exact SDK validator regressions: revalidate original bytes, never dispatch again.
use super::*;
use crate::text_transport::{AttemptState, Backend, FrozenAttempt};

const POLICY: &str = "sdk-f2-scene-episode-projection-replay/v1";
const REJECTION: &str = "资产用途集号与本次场引用不一致";
const Q1_POLICY: &str = "sdk-q1-confirmed-scene-split-replay/v1";
const Q1_REJECTION: &str = "制作选择场范围不在当前原文";
const PLAN_POLICY: &str = "sdk-planner-package-source-order-replay/v1";
const PLAN_REJECTION: &str = "工作包必须依源顺序覆盖正文恰好一次";
const FRAGMENT_POLICY: &str = "sdk-f2-registered-fragments-replay/v1";
const FRAGMENT_REJECTION: &str = "本批资产身份重复";

struct ReplayPolicy {
    name: &'static str,
    stage: &'static str,
    workflow: &'static str,
    rejection: &'static str,
    accepted_event: &'static str,
    rejected_event: &'static str,
}

const F2: ReplayPolicy = ReplayPolicy { name: POLICY, stage: "foundation:F2", workflow:"foundation.run", rejection: REJECTION,
    accepted_event: "sdk_f2_episode_replay_accepted", rejected_event: "sdk_f2_episode_replay_rejected" };
const Q1: ReplayPolicy = ReplayPolicy { name: Q1_POLICY, stage: "foundation:Q1", workflow:"foundation.run", rejection: Q1_REJECTION,
    accepted_event: "sdk_q1_scene_split_replay_accepted", rejected_event: "sdk_q1_scene_split_replay_rejected" };
const PLAN: ReplayPolicy = ReplayPolicy { name:PLAN_POLICY, stage:"storyboardPlan", workflow:"storyboard.run", rejection:PLAN_REJECTION,
    accepted_event:"sdk_planner_source_order_replay_accepted", rejected_event:"sdk_planner_source_order_replay_rejected" };

const FRAGMENT: ReplayPolicy = ReplayPolicy { name:FRAGMENT_POLICY, stage:"foundation:F2", workflow:"foundation.run", rejection:FRAGMENT_REJECTION,
    accepted_event:"sdk_f2_fragment_replay_accepted", rejected_event:"sdk_f2_fragment_replay_rejected" };

fn policy(snapshot: &Value) -> Option<&'static ReplayPolicy> {
    let input = &snapshot["input"];
    if snapshot["executorIdentity"]["provider"] != "codex_sdk" {return None;}
    if snapshot["stage"]==PLAN.stage && snapshot["skill"]=="episode-emotion-planner"
        && input["storyboardCreativeProfile"]=="original-production-contract/v1"
        && input["sourceRows"].is_array() && input["contextRows"].is_array() && input["episode"]["id"].is_string()
    {return Some(&PLAN);}
    if input["method"] != "foundation-delivery/v2"
        || input["modelInputProfile"] != "foundation-compact/v1"
        || input["catalogueContract"] != "light-catalogue/v1"
        || input["sourceReferenceEncoding"] != "row-coordinates/v1"
        || input["delivery"]["method"] != "foundation-delivery/v2"
        || input["delivery"]["scope"] != input["scope"]
        || input["delivery"]["batchIndex"] != input["batchIndex"]
    { return None; }
    if snapshot["stage"] == F2.stage && snapshot["skill"] == "foundation-asset-catalog"
        && input["delivery"]["stage"] == "F2" && input["delivery"]["phase"] == "GENERATE_F2"
    { return Some(&F2); }
    if snapshot["stage"] == Q1.stage && snapshot["skill"] == "foundation-finishing-editor"
        && input["delivery"]["stage"] == "Q1" && input["delivery"]["phase"] == "EDIT"
        && input["foundationDecisionPolicy"] == crate::pipeline::foundation_delivery::AUTO_INTERPRET_POLICY
    { return Some(&Q1); }
    None
}

// Select the narrow validator migration by the original durable rejection.
// The ordinary stage policy remains unchanged for every existing audit.
fn replay_policy(snapshot: &Value, rejection: &Value) -> Option<&'static ReplayPolicy> {
    let base = policy(snapshot)?;
    if base.name == POLICY && rejection == FRAGMENT_REJECTION { Some(&FRAGMENT) }
    else if rejection == base.rejection { Some(base) } else { None }
}

fn registered_fragment_present(snapshot:&Value,raw:&Value)->bool {
    let (Some(registry),Some(assets))=(snapshot["input"]["registry"].as_array(),raw["assets"].as_array()) else{return false;};
    let mut seen=std::collections::HashSet::new();
    assets.iter().filter_map(|a|a["key"].as_str()).any(|key|!seen.insert(key)&&registry.iter().any(|a|a["key"]==key))
}

fn fragment_context_bound(snapshot:&Value,parent:&Value,prepared:&Value,run:&str)->bool {
    let Some(fields)=prepared.as_object() else{return false;};
    let base=["compiledPromptSha256","compiledSchemaSha256","sdkAttemptId","sdkDispatchStarted","sdkOriginWorkflowId","sdkRawTextSha256","sdkRequestHash"];
    if base.iter().any(|k|!fields.contains_key(*k)) || fields.keys().any(|k|!base.contains(&k.as_str())&&! ["sdkSupervisorHandoff","sdkWorkerScope"].contains(&k.as_str())) {return false;}
    let settings=&parent["input"]["settings"];
    let Ok(request)=serde_json::from_value::<TextRequest>(snapshot.clone()) else{return false;};
    if snapshot.get("sdkSupervisorHandoff").is_some() {
        if prepared["sdkSupervisorHandoff"]!=snapshot["sdkSupervisorHandoff"]
            || snapshot["sdkSupervisorHandoff"]["parentSnapshotSha256"]!=crate::pipeline::digest(&parent.to_string())
            || !sdk_supervisor_handoff::proof_valid(snapshot,settings,run,&step_key(&request)) {return false;}
    } else if prepared.get("sdkSupervisorHandoff").is_some() || settings["textSdkIdentity"]!=snapshot["executorIdentity"] {return false;}
    if let Some(marker)=snapshot["input"].get("foundationParallelTrial") {
        let scope=&prepared["sdkWorkerScope"];
        if foundation_parallel::validate_markers(&request).is_err() || marker["workflowId"]!=run
            || scope.as_object().is_none_or(|m|m.len()!=4) || scope["version"]!="foundation-sdk-lane-worker/v1"
            || scope["authorizationSha256"]!=marker["authorizationSha256"] || scope["laneId"]!=marker["laneId"]
            || scope["workerRoot"].as_str().is_none_or(|s|s.is_empty()) {return false;}
    } else if prepared.get("sdkWorkerScope").is_some() {return false;}
    true
}

fn eligible(snapshot: &Value) -> bool {
    policy(snapshot).is_some()
}

fn source_bound(snapshot: &Value, parent: &Value) -> bool {
    let source = &parent["input"]["source"];
    let Ok(locked) = serde_json::from_value::<crate::pipeline::LockedSource>(source.clone()) else {
        return false;
    };
    let input = &snapshot["input"];
    if policy(snapshot).is_some_and(|p|p.name==PLAN_POLICY) {
        let Some(episode)=locked.episodes.iter().find(|e|input["episode"]["id"]==e.id) else{return false;};
        let expected:Vec<_>=locked.rows.iter().filter(|r|r.kind=="body"&&r.episode_id.as_deref()==Some(&episode.id)).collect();
        return crate::pipeline::validate_source(&locked).is_ok() && source["version"]==parent["sourceVersion"]
            && parent["sourceVersion"].as_u64().is_some_and(|v|v>0) && json!(episode)==input["episode"]
            && json!(expected)==input["sourceRows"] && input["contextRows"].as_array().is_some_and(|rows|rows.iter().all(|r|locked.rows.iter().any(|old|json!(old)==*r)));
    }
    if crate::pipeline::digest(&locked.text) != locked.sha256
        || input["sourceSha256"] != locked.sha256
        || parent["sourceVersion"] != parent["input"]["sourceVersion"]
        || parent["sourceVersion"].as_u64().is_none()
        || source["version"] != parent["sourceVersion"]
        || input["source"]["columns"]
            != json!([
                "rowId",
                "line",
                "episodeId",
                "sceneId",
                "kind",
                "speaker",
                "channel",
                "start",
                "end",
                "text"
            ])
    {
        return false;
    }
    let Some(rows) = input["source"]["rows"].as_array().filter(|r| !r.is_empty()) else {
        return false;
    };
    let mut seen = std::collections::HashSet::new();
    rows.iter().all(|tuple| {
        let Some(parts) = tuple.as_array().filter(|r| r.len() == 10) else {
            return false;
        };
        let Some(row) = locked.rows.iter().find(|row| parts[0] == row.id) else {
            return false;
        };
        let (Some(start), Some(end)) = (parts[7].as_u64(), parts[8].as_u64()) else {
            return false;
        };
        let range = crate::pipeline::SourceRange {
            start: start as usize,
            end: end as usize,
        };
        seen.insert((row.id.as_str(), start, end))
            && parts[1] == row.number
            && parts[2] == json!(row.episode_id)
            && parts[3] == json!(row.scene_id)
            && parts[4] == row.kind
            && parts[5] == json!(row.dialogue.as_ref().map(|d| d.speaker.as_str()))
            && parts[6] == json!(row.dialogue.as_ref().map(|d| d.channel.as_str()))
            && start < end
            && range.start >= row.range.start
            && range.end <= row.range.end
            && crate::pipeline::slice_utf16(&locked.text, &range)
                .ok()
                .as_deref()
                == parts[9].as_str()
    })
}

fn audit_valid(audit: &Value) -> bool {
    if !audit.as_object().is_some_and(|f| f.len() == 21)
        || audit["generationRequestsSent"] != 0
        || audit["acceptedAutomatically"] != false
        || [
            "workflowId",
            "nodeKey",
            "originalJobId",
            "requestHash",
            "originalAttemptSha256",
            "originalCompiledSha256",
            "originalRawTextSha256",
        ]
        .iter()
        .any(|k| s(audit, k).is_empty())
    {
        return false;
    }
    let parse = |key| serde_json::from_str::<Value>(s(audit, key));
    let (Ok(snapshot), Ok(raw), Ok(parent), Ok(receipt), Ok(prepared), Ok(event), Ok(sdk_event)) = (
        parse("originalSnapshotJson"),
        parse("originalResultJson"),
        parse("originalWorkflowSnapshotJson"),
        parse("originalReceiptJson"),
        parse("originalPreparedJson"),
        parse("originalRawEventDataJson"),
        parse("originalSdkRawEventDataJson"),
    ) else {
        return false;
    };
    let Ok(request) = serde_json::from_value::<TextRequest>(snapshot.clone()) else {
        return false;
    };
    let Some(policy) = replay_policy(&snapshot, &audit["previousError"]) else { return false; };
    if audit["policy"] != policy.name || audit["stage"] != policy.stage
        || audit["previousError"] != policy.rejection { return false; }
    let rules = parent["rules"].as_array();
    let settings = &parent["input"]["settings"];
    eligible(&snapshot)
        && snapshot.to_string() == s(audit, "originalSnapshotJson")
        && crate::pipeline::digest(s(audit, "originalSnapshotJson")) == s(audit, "requestHash")
        && parent["stage"] == policy.workflow
        && frozen_text_parent_matches(policy.workflow, &parent, &request)
        && settings["textProvider"] == "codex_sdk"
        && (policy.name == FRAGMENT_POLICY || settings["textSdkIdentity"] == snapshot["executorIdentity"])
        && settings["textModel"] == request.model
        && settings["reasoning"] == request.reasoning
        && snapshot["executorPolicy"] == CLI_POLICY_VERSION
        && source_bound(&snapshot, &parent)
        && snapshot["ruleSnapshot"]["primary"]["id"] == request.skill
        && rules.is_some_and(|rules| rules.contains(&snapshot["ruleSnapshot"]["primary"]))
        && std::iter::once(&snapshot["ruleSnapshot"]["primary"])
            .chain(
                snapshot["ruleSnapshot"]["methods"]
                    .as_array()
                    .into_iter()
                    .flatten(),
            )
            .all(|rule| {
                !s(rule, "content").is_empty()
                    && rule["sha256"] == crate::pipeline::digest(s(rule, "content"))
                    && rules.is_some_and(|rules| rules.contains(rule))
            })
        && audit["nodeKey"] == step_key(&request)
        && receipt == json!({"contract":"desktop-runner/v1","validation":"rejected"})
        && (if policy.name == FRAGMENT_POLICY {
            registered_fragment_present(&snapshot,&raw) && fragment_context_bound(&snapshot,&parent,&prepared,s(audit,"workflowId"))
        } else { prepared.as_object().is_some_and(|f| f.len() == 7) })
        && prepared["sdkDispatchStarted"] == true
        && prepared["sdkOriginWorkflowId"] == audit["workflowId"]
        && prepared["sdkAttemptId"] == audit["originalJobId"]
        && prepared["sdkRequestHash"] == audit["requestHash"]
        && prepared["sdkRawTextSha256"] == audit["originalRawTextSha256"]
        && event == json!({"requestHash":audit["requestHash"],"value":raw})
        && audit["originalRawEventId"]
            .as_i64()
            .is_some_and(|id| id > 0)
        && audit["originalSdkRawEventId"]
            .as_i64()
            .is_some_and(|id| id > 0)
        && sdk_event["domainAccepted"] == false
        && sdk_event["parseError"].is_null()
        && sdk_event["rawText"].as_str().is_some_and(|text| {
            crate::pipeline::digest(text) == s(audit, "originalRawTextSha256")
                && serde_json::from_str::<Value>(text).ok().as_ref()
                    == Some(&sdk_event["parsedJson"])
        })
        && compile_model_transport(&request).ok().is_some_and(|wire| {
            restore_model_output(&request, wire.as_ref(), sdk_event["parsedJson"].clone())
                .ok().as_ref() == Some(&raw)
        })
}

fn files_bound(root: &Path, audit: &Value) -> bool {
    let base = root.join("text-runs").join(s(audit, "originalJobId"));
    if [
        ("sdk-attempt.json", "originalAttemptSha256", 4_194_304),
        ("sdk-compiled.json", "originalCompiledSha256", 2_097_152),
        ("result.json", "originalRawTextSha256", 1_048_576),
    ]
    .iter()
    .any(|(file, key, limit)| {
        crate::store::hash_file(&base.join(file))
            .ok()
            .is_none_or(|(hash, size)| hash != s(audit, key) || size > *limit)
    }) {
        return false;
    }
    let (Ok(attempt), Ok(snapshot), Ok(sdk_event), Ok(prepared)) = (
        FrozenAttempt::load(&base.join("sdk-attempt.json")),
        serde_json::from_str::<Value>(s(audit, "originalSnapshotJson")),
        serde_json::from_str::<Value>(s(audit, "originalSdkRawEventDataJson")),
        serde_json::from_str::<Value>(s(audit, "originalPreparedJson")),
    ) else {
        return false;
    };
    let Ok(request) = serde_json::from_value::<TextRequest>(snapshot.clone()) else {
        return false;
    };
    let Ok(compiled) = compile_sdk_attempt_input(&attempt) else {
        return false;
    };
    let Ok(bytes) = std::fs::read(base.join("sdk-compiled.json")) else {
        return false;
    };
    let expected = json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":crate::pipeline::digest(&compiled.prompt),"schemaSha256":crate::pipeline::digest(&compiled.schema)});
    attempt.backend == Backend::CodexSdk
        && attempt.state == AttemptState::Completed
        && attempt.dispatch_started
        && attempt.attempt_id == s(audit, "originalJobId")
        && attempt.workflow_id == s(audit, "workflowId")
        && attempt.logical_node_id == s(audit, "nodeKey")
        && attempt.runtime_identity == snapshot["executorIdentity"]
        && serde_json::to_value(&attempt.request).ok() == serde_json::to_value(&request).ok()
        && attempt.raw_result.as_ref() == Some(&sdk_event)
        && serde_json::from_slice::<Value>(&bytes).ok().as_ref() == Some(&expected)
        && prepared["compiledPromptSha256"] == expected["promptSha256"]
        && prepared["compiledSchemaSha256"] == expected["schemaSha256"]
}

fn domain_result_matches(snapshot:&Value,parent:&Value,raw:&Value,result:&Value)->bool {
    if policy(snapshot).is_some_and(|p|p.name==PLAN_POLICY) {
        let Ok(source)=serde_json::from_value(parent["input"]["source"].clone()) else{return false;};
        return crate::pipeline::planner_source_order_replay_result(&source,&snapshot["input"],raw).is_ok_and(|value|value==*result);
    }
    crate::pipeline::foundation_delivery::accepted_scope_reason_raw_matches(&snapshot["input"],raw,result)
}

fn database_bound(
    c: &rusqlite::Connection,
    audit: &Value,
    accepted: bool,
    prepared: &Value,
    receipt: &Value,
    result: &Value,
) -> Result<bool> {
    let snapshot: Value = serde_json::from_str(s(audit, "originalSnapshotJson")).map_err(|e|e.to_string())?;
    let Some(policy) = replay_policy(&snapshot, &audit["previousError"]) else { return Ok(false); };
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id JOIN jobs p ON p.id=w.workflow_id WHERE j.id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND j.snapshot=?2 AND j.result=?3 AND j.prepared=?4 AND j.receipt=?5 AND j.state=?6 AND j.error IS ?7 AND w.workflow_id=?8 AND w.node_key=?9 AND w.stage=?18 AND w.request_hash=?10 AND w.state=?11 AND w.result=?12 AND w.error IS ?7 AND p.kind='workflow' AND p.entity_id=?19 AND p.snapshot=?13 AND EXISTS(SELECT 1 FROM events e WHERE e.job_id=j.id AND e.type='text_received_raw' AND e.id=?14 AND e.data=?15 AND e.id=(SELECT MAX(id) FROM events WHERE job_id=j.id AND type='text_received_raw')) AND EXISTS(SELECT 1 FROM events e WHERE e.job_id=j.id AND e.type='sdk_raw_result' AND e.id=?16 AND e.data=?17 AND e.id=(SELECT MAX(id) FROM events WHERE job_id=j.id AND type='sdk_raw_result')))",
        params![s(audit,"originalJobId"),s(audit,"originalSnapshotJson"),s(audit,"originalResultJson"),prepared.to_string(),receipt.to_string(),if accepted {"completed"} else {"rejected"},if accepted {None} else {Some(policy.rejection)},s(audit,"workflowId"),s(audit,"nodeKey"),s(audit,"requestHash"),if accepted {"accepted"} else {"rejected"},result.to_string(),s(audit,"originalWorkflowSnapshotJson"),audit["originalRawEventId"].as_i64(),s(audit,"originalRawEventDataJson"),audit["originalSdkRawEventId"].as_i64(),s(audit,"originalSdkRawEventDataJson"),policy.stage,policy.workflow],
        |r|r.get(0)).map_err(|e|e.to_string())
}

impl SavedTextExecutor {
    pub(super) fn sdk_f2_migrated_receipt_bound(
        &self,
        job: &Value,
        snapshot: &Value,
    ) -> Result<bool> {
        let receipt = &job["receipt"];
        let Some(policy) = replay_policy(snapshot, &receipt["migrationAudit"]["previousError"]) else { return Ok(false); };
        if receipt["migrationPolicy"] != policy.name {
            return Ok(false);
        }
        let audit = &receipt["migrationAudit"];
        if policy.name == FRAGMENT_POLICY {
            // sdk_raw owns the bounded handoff ancestry worklist; do not recurse
            // into it from sdk_raw_local's receipt validation.
            self.sdk_parallel_worker_bound(job,snapshot)?;
        }
        let result = &job["prepared"]["sdkDomainResult"];
        let original: Value =
            serde_json::from_str(s(audit, "originalResultJson")).map_err(|e| e.to_string())?;
        let mut expected_prepared: Value =
            serde_json::from_str(s(audit, "originalPreparedJson")).map_err(|e| e.to_string())?;
        expected_prepared["sdkDomainResult"] = result.clone();
        expected_prepared["sdkDomainResultSha256"] =
            json!(crate::pipeline::digest(&result.to_string()));
        let completed_event_bound = if matches!(policy.name,Q1_POLICY|PLAN_POLICY|FRAGMENT_POLICY) {
            let expected = json!({"policy":policy.name,"migrationAudit":audit,"generationRequestsSent":0,
                "validation":"accepted","error":null,"validatedResultSha256":crate::pipeline::digest(&result.to_string()),"originalEvidencePreserved":true});
            self.store.conn()?.query_row("SELECT COUNT(*)=1 AND SUM(CASE WHEN type=?2 AND data=?4 THEN 1 ELSE 0 END)=1 FROM events WHERE job_id=?1 AND type IN (?2,?3)",
                params![s(job,"id"),policy.accepted_event,policy.rejected_event,expected.to_string()],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())?
        } else { true };
        Ok(audit_valid(audit)
            && completed_event_bound
            && files_bound(&self.store.root, audit)
            && job["snapshot"] == *snapshot
            && snapshot.to_string() == s(audit, "originalSnapshotJson")
            && audit["originalJobId"] == job["id"]
            && receipt
                == &json!({"contract":"desktop-runner/v1","validation":"accepted","migrationPolicy":policy.name,"migrationAudit":audit,"acceptedRequestHash":audit["requestHash"],"validatedResultSha256":crate::pipeline::digest(&result.to_string())})
            && job["prepared"] == expected_prepared
            && domain_result_matches(snapshot,&serde_json::from_str::<Value>(s(audit,"originalWorkflowSnapshotJson")).map_err(|e|e.to_string())?,&original,result)
            && database_bound(
                &self.store.conn()?,
                audit,
                true,
                &expected_prepared,
                receipt,
                result,
            )?)
    }

    pub(super) fn sdk_f2_replay_commit(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        job: &Value,
        value: &Value,
        reason: Option<&str>,
    ) -> Result<bool> {
        let Some(policy) = replay_policy(snapshot, &job["error"]) else { return Ok(false); };
        if job["status"] != "rejected" || job["error"] != policy.rejection {
            return Ok(false);
        }
        let audit = self.sdk_f2_replay_audit(request, snapshot, job)?;
        if policy.name == FRAGMENT_POLICY {
            self.sdk_handoff_job_bound(job,snapshot)?;
            self.sdk_parallel_worker_bound(job,snapshot)?;
        }
        if !self
            .store
            .commit_sdk_f2_episode_replay(&audit, value, reason)?
        {
            return Err("SDK_FOUNDATION_REPLAY_HELD: 原冻结节点变化；保留原任务且不重发".into());
        }
        Ok(true)
    }

    fn sdk_f2_replay_audit(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        job: &Value,
    ) -> Result<Value> {
        let held = || {
            "SDK_FOUNDATION_REPLAY_HELD: 原请求、父流程、回包、回执或接收证据不匹配；保留原任务且不重发"
                .to_string()
        };
        let policy = replay_policy(snapshot, &job["error"]).ok_or_else(held)?;
        let run = self.workflow_id.as_deref().ok_or_else(held)?;
        let c = self.store.conn()?;
        let parent: String = c.query_row("SELECT snapshot FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id=?2",params![run,policy.workflow],|r|r.get(0)).map_err(|_|held())?;
        let (event_id,event):(i64,String) = c.query_row("SELECT id,data FROM events WHERE job_id=?1 AND type='text_received_raw' ORDER BY id DESC LIMIT 1",[s(job,"id")],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|held())?;
        let (sdk_id,sdk_event):(i64,String) = c.query_row("SELECT id,data FROM events WHERE job_id=?1 AND type='sdk_raw_result' ORDER BY id DESC LIMIT 1",[s(job,"id")],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|held())?;
        let path = self.store.root.join("text-runs").join(s(job, "id"));
        let audit = json!({"policy":policy.name,"stage":policy.stage,"previousError":policy.rejection,
            "workflowId":run,"nodeKey":step_key(request),"originalJobId":job["id"],"requestHash":crate::pipeline::digest(&snapshot.to_string()),
            "originalSnapshotJson":snapshot.to_string(),"originalResultJson":job["result"].to_string(),"originalPreparedJson":job["prepared"].to_string(),"originalReceiptJson":job["receipt"].to_string(),"originalWorkflowSnapshotJson":parent,
            "originalRawEventId":event_id,"originalRawEventDataJson":event,"originalSdkRawEventId":sdk_id,"originalSdkRawEventDataJson":sdk_event,
            "originalAttemptSha256":crate::store::hash_file(&path.join("sdk-attempt.json"))?.0,"originalCompiledSha256":crate::store::hash_file(&path.join("sdk-compiled.json"))?.0,"originalRawTextSha256":crate::store::hash_file(&path.join("result.json"))?.0,
            "generationRequestsSent":0,"acceptedAutomatically":false});
        Ok(audit)
    }

    pub(super) fn sdk_q1_replay_already_rejected(&self, job: &Value, snapshot: &Value) -> Result<bool> {
        let Some(policy)=replay_policy(snapshot,&job["error"]).filter(|p|matches!(p.name,Q1_POLICY|PLAN_POLICY|FRAGMENT_POLICY)) else{return Ok(false);};
        if job["status"] != "rejected" || job["error"] != policy.rejection { return Ok(false); }
        self.store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type=?2)",
            params![s(job,"id"), policy.rejected_event], |r|r.get(0)).map_err(|e|e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        tempfile::TempDir,
        SavedTextExecutor,
        TextRequest,
        String,
        Value,
    ) {
        fixture_for(false)
    }

    fn fixture_for(q1: bool) -> (tempfile::TempDir, SavedTextExecutor, TextRequest, String, Value) {
        fixture_kind(q1,false)
    }

    pub(super) fn fixture_kind(q1:bool,fragment:bool) -> (tempfile::TempDir, SavedTextExecutor, TextRequest, String, Value) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("isolated")).unwrap();
        store
            .create("合成 SDK F2 用途恢复", "synthetic-f2")
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        let locked = crate::pipeline::prepare_source(
            "第一集\n场景：路口\n△甲向前走。\n第二集\n场景：门前\n△甲停步。\n",
        )
        .unwrap();
        let source = store.save_source(json!(locked)).unwrap();
        let locked: crate::pipeline::LockedSource = serde_json::from_value(source.clone()).unwrap();
        let settings = crate::text_sdk::offline_settings("happy");
        let skill = if q1 { "foundation-finishing-editor" } else { "foundation-asset-catalog" };
        let stage = if q1 { "foundation:Q1" } else { "foundation:F2" };
        let rejection = if fragment {FRAGMENT_REJECTION} else if q1 { Q1_REJECTION } else { REJECTION };
        let rules = crate::pipeline::snapshot(skill, &["screenplay-fidelity"])
            .unwrap();
        let scope: Vec<_> = locked.episodes.iter().map(|e| e.id.clone()).collect();
        let mut request = TextRequest {
            stage: stage.into(),
            skill: skill.into(),
            model: "gpt-6.1-sol".into(),
            reasoning: "medium".into(),
            input: json!({"method":"foundation-delivery/v2","modelInputProfile":"foundation-compact/v1",
                "catalogueContract":"light-catalogue/v1","sourceReferenceEncoding":"row-coordinates/v1","sourceSha256":locked.sha256,
                "scope":scope,"batchIndex":0,"batch":"synthetic-batch","delivery":{"method":"foundation-delivery/v2","stage":"F2","phase":"GENERATE_F2","scope":scope,"batchIndex":0,"batchCount":1,"fragment":null},
                "source":{"columns":["rowId","line","episodeId","sceneId","kind","speaker","channel","start","end","text"],"rows":locked.rows.iter().map(|r|json!([r.id,r.number,r.episode_id,r.scene_id,r.kind,r.dialogue.as_ref().map(|d|d.speaker.as_str()),r.dialogue.as_ref().map(|d|d.channel.as_str()),r.range.start,r.range.end,r.text])).collect::<Vec<_>>()},
                "scenes":locked.scenes.iter().map(|s|json!([s.episode_id,s.id,s.title])).collect::<Vec<_>>() }),
            schema: json!({"type":"object","properties":{"ok":{"type":"boolean"},"facts":{"type":"array","items":{"type":"string"}}},"required":["ok","facts"],"additionalProperties":false}),
            rule_snapshot: rules.clone(),
        };
        if q1 {
            request.input["delivery"]["stage"] = json!("Q1");
            request.input["delivery"]["phase"] = json!("EDIT");
            request.input["foundationDecisionPolicy"] = json!(crate::pipeline::foundation_delivery::AUTO_INTERPRET_POLICY);
        }
        if fragment {
            request.input["registry"]=json!([{"key":"LOC_registered","type":"scene","name":"实机竞技场","variants":[]}]);
            request.schema=json!({"type":"object","properties":{"assets":{"type":"array","items":{"type":"object","properties":{"key":{"type":"string"},"description":{"type":"string"}},"required":["key","description"],"additionalProperties":false}}},"required":["assets"],"additionalProperties":false});
        }
        let parent_rules: Vec<_> = std::iter::once(rules["primary"].clone())
            .chain(rules["methods"].as_array().unwrap().iter().cloned())
            .collect();
        let run = store.add_job("workflow","foundation.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run","sourceVersion":source["version"],"rules":parent_rules,"input":{"source":source,"sourceVersion":source["version"],"settings":settings}}),"running").unwrap();
        let executor = SavedTextExecutor {
            store,
            settings,
            workflow_id: Some(run.clone()),
        };
        let snapshot = executor.request_snapshot(&request).unwrap();
        let hash = crate::pipeline::digest(&snapshot.to_string());
        let compiled = compile_codex_input(&request).unwrap();
        let job = executor
            .store
            .add_job(
                "text",
                stage,
                "codex_sdk",
                "codex_sdk",
                &snapshot,
                "rejected",
            )
            .unwrap();
        let raw = if fragment {json!({"assets":[{"key":"LOC_registered","description":"实景"},{"key":"LOC_registered","description":"闪回"}]})} else {json!({"ok":true,"facts":["合成原始事实"]})};
        let envelope = json!({"domainAccepted":false,"parseError":null,"parsedJson":raw,"rawText":raw.to_string()});
        let prepared = json!({"compiledPromptSha256":crate::pipeline::digest(&compiled.prompt),"compiledSchemaSha256":crate::pipeline::digest(&compiled.schema),"sdkAttemptId":job,"sdkDispatchStarted":true,"sdkOriginWorkflowId":run,"sdkRawTextSha256":crate::pipeline::digest(&raw.to_string()),"sdkRequestHash":hash});
        let receipt = json!({"contract":"desktop-runner/v1","validation":"rejected"});
        let c = executor.store.conn().unwrap();
        c.execute(
            "UPDATE jobs SET result=?1,error=?2,prepared=?3,receipt=?4 WHERE id=?5",
            params![
                raw.to_string(),
                rejection,
                prepared.to_string(),
                receipt.to_string(),
                job
            ],
        )
        .unwrap();
        c.execute(
            "INSERT INTO workflow_steps VALUES(?1,?2,?3,?4,'rejected',?5,?6,?7,?8)",
            params![
                run,
                step_key(&request),
                request.stage,
                hash,
                job,
                raw.to_string(),
                rejection,
                now()
            ],
        )
        .unwrap();
        executor
            .store
            .event(&job, "sdk_raw_result", &envelope)
            .unwrap();
        executor
            .store
            .event(
                &job,
                "text_received_raw",
                &json!({"requestHash":hash,"value":raw}),
            )
            .unwrap();
        let base = executor.store.root.join("text-runs").join(&job);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("result.json"), raw.to_string()).unwrap();
        crate::store::atomic_json(&base.join("sdk-compiled.json"),&json!({"prompt":compiled.prompt,"schema":compiled.schema,"promptSha256":prepared["compiledPromptSha256"],"schemaSha256":prepared["compiledSchemaSha256"]})).unwrap();
        let mut attempt = FrozenAttempt::prepare(
            &job,
            &run,
            &step_key(&request),
            Backend::CodexSdk,
            request.clone(),
            &snapshot["executorIdentity"],
        )
        .unwrap();
        attempt.begin_dispatch(Backend::CodexSdk).unwrap();
        attempt.state = AttemptState::Completed;
        attempt.raw_result = Some(envelope);
        attempt.persist(&base.join("sdk-attempt.json")).unwrap();
        (temp, executor, request, job, raw)
    }

    pub(super) fn preserved(executor: &SavedTextExecutor, job: &str) -> Value {
        let pair: (String, String) = executor
            .store
            .conn()
            .unwrap()
            .query_row("SELECT snapshot,result FROM jobs WHERE id=?1", [job], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        let hashes: Vec<_> = ["sdk-attempt.json", "sdk-compiled.json", "result.json"]
            .iter()
            .map(|f| {
                crate::store::hash_file(&executor.store.root.join("text-runs").join(job).join(f))
                    .unwrap()
            })
            .collect();
        json!({"snapshotBytes":pair.0,"rawBytes":pair.1,"files":hashes})
    }

    #[tokio::test]
    async fn sdk_f2_episode_replay_accept_restart_preserves_original_evidence_zero_dispatch() {
        let (_temp, executor, request, job, raw) = fixture();
        let before = preserved(&executor, &job);
        assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
        executor.accept(&request, &raw).await.unwrap();
        let adopted = executor.sdk_job(&job).unwrap();
        assert_eq!(adopted["status"], "completed");
        assert_eq!(adopted["receipt"]["migrationPolicy"], POLICY);
        assert_eq!(adopted["prepared"]["sdkDomainResult"], raw);
        let snapshot = executor.request_snapshot(&request).unwrap();
        assert!(executor
            .sdk_f2_migrated_receipt_bound(&adopted, &snapshot)
            .unwrap());
        let restart = SavedTextExecutor {
            store: Store::open(executor.store.root.clone()).unwrap(),
            settings: executor.settings.clone(),
            workflow_id: executor.workflow_id.clone(),
        };
        for _ in 0..2 {
            assert_eq!(restart.execute(request.clone()).await.unwrap(), raw);
            restart.accept(&request, &raw).await.unwrap();
        }
        assert_eq!(preserved(&restart, &job), before);
        assert_eq!(restart.store.jobs().unwrap().len(), 2);
        assert!(
            !restart.store.root.join("sdk-worker").exists(),
            "zero-call replay never starts the SDK worker"
        );
        let n:i64=restart.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='sdk_f2_episode_replay_accepted'",[&job],|r|r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn sdk_f2_episode_replay_secondary_rejection_is_diagnostic_not_retry_permission() {
        let (_temp, executor, request, job, raw) = fixture();
        let before = preserved(&executor, &job);
        for _ in 0..2 {
            assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
            executor
                .reject(&request, &raw, "合成第二级领域拒绝")
                .await
                .unwrap();
        }
        let old = executor.sdk_job(&job).unwrap();
        assert_eq!(old["error"], REJECTION);
        assert_eq!(
            old["receipt"],
            json!({"contract":"desktop-runner/v1","validation":"rejected"})
        );
        assert_eq!(old["status"], "rejected");
        let n:i64=executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='sdk_f2_episode_replay_rejected'",[&job],|r|r.get(0)).unwrap();
        assert_eq!(n, 1);
        executor.accept(&request, &raw).await.unwrap();
        assert_eq!(preserved(&executor, &job), before);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }

    #[tokio::test]
    async fn sdk_f2_episode_replay_rejects_changed_facts_unknown_stage_and_profile() {
        let (_temp, executor, request, job, raw) = fixture();
        let before = preserved(&executor, &job);
        assert!(executor
            .accept(&request, &json!({"ok":true,"facts":["改写事实"]}))
            .await
            .unwrap_err()
            .contains("DOMAIN_RESULT_CHANGED"));
        for change in ["profile", "stage", "source", "rules", "provider"] {
            let mut changed = request.clone();
            match change {
                "profile" => changed.input["modelInputProfile"] = json!("other/v1"),
                "stage" => changed.stage = "foundation:Q1".into(),
                "source" => changed.input["sourceSha256"] = json!("different"),
                "rules" => changed.rule_snapshot["primary"]["sha256"] = json!("different"),
                _ => changed.model = "different".into(),
            }
            assert!(executor.accept(&changed, &raw).await.is_err(), "{change}");
        }
        executor
            .store
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                [&job],
            )
            .unwrap();
        assert!(executor.accept(&request, &raw).await.is_err());
        assert_eq!(preserved(&executor, &job), before);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }

    #[test]
    fn sdk_f2_episode_replay_store_proof_and_cas_reject_tampered_or_newer_evidence() {
        let (_temp, executor, request, job, raw) = fixture();
        let snapshot = executor.request_snapshot(&request).unwrap();
        let audit = executor
            .sdk_f2_replay_audit(&request, &snapshot, &executor.sdk_job(&job).unwrap())
            .unwrap();
        assert!(audit_valid(&audit));
        assert!(files_bound(&executor.store.root, &audit));
        for field in [
            "previousError",
            "workflowId",
            "nodeKey",
            "originalJobId",
            "requestHash",
            "originalSnapshotJson",
            "originalResultJson",
            "originalPreparedJson",
            "originalReceiptJson",
            "originalWorkflowSnapshotJson",
            "originalRawEventDataJson",
            "originalSdkRawEventDataJson",
            "originalAttemptSha256",
            "originalCompiledSha256",
            "originalRawTextSha256",
        ] {
            let mut forged = audit.clone();
            forged[field] = json!("changed");
            assert!(
                !executor
                    .store
                    .commit_sdk_f2_episode_replay(&forged, &raw, None)
                    .unwrap_or(false),
                "{field}"
            );
        }
        let c = executor.store.conn().unwrap();
        c.execute(
            "UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",
            [&job],
        )
        .unwrap();
        assert!(!executor
            .store
            .commit_sdk_f2_episode_replay(&audit, &raw, None)
            .unwrap());
        c.execute(
            "UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",
            params![s(&audit, "requestHash"), job],
        )
        .unwrap();
        executor
            .store
            .event(
                &job,
                "text_received_raw",
                &json!({"requestHash":audit["requestHash"],"value":raw}),
            )
            .unwrap();
        assert!(
            !executor
                .store
                .commit_sdk_f2_episode_replay(&audit, &raw, None)
                .unwrap(),
            "even an identical newer raw event invalidates the old CAS proof"
        );
        assert_eq!(executor.sdk_job(&job).unwrap()["status"], "rejected");
    }

    #[tokio::test]
    async fn sdk_q1_split_replay_accept_restart_keeps_original_raw_and_zero_dispatch() {
        let (_temp, executor, request, job, raw) = fixture_for(true);
        let before = preserved(&executor, &job);
        assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
        executor.accept(&request, &raw).await.unwrap();
        let adopted = executor.sdk_job(&job).unwrap();
        assert_eq!(adopted["receipt"]["migrationPolicy"], Q1_POLICY);
        assert_eq!(adopted["receipt"]["migrationAudit"]["previousError"], Q1_REJECTION);
        let snapshot = executor.request_snapshot(&request).unwrap();
        assert!(executor.sdk_f2_migrated_receipt_bound(&adopted, &snapshot).unwrap());
        let restart = SavedTextExecutor { store: Store::open(executor.store.root.clone()).unwrap(),
            settings: executor.settings.clone(), workflow_id: executor.workflow_id.clone() };
        for _ in 0..2 {
            assert_eq!(restart.execute(request.clone()).await.unwrap(), raw);
            restart.accept(&request, &raw).await.unwrap();
        }
        assert_eq!(preserved(&restart, &job), before);
        assert_eq!(restart.store.jobs().unwrap().len(), 2);
        assert!(!restart.store.root.join("sdk-worker").exists());
        let n:i64 = restart.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type=?2",params![job,Q1.accepted_event],|r|r.get(0)).unwrap();
        assert_eq!(n,1);
    }

    #[tokio::test]
    async fn sdk_q1_second_domain_rejection_exhausts_replay_without_retry_or_promotion() {
        let (_temp, executor, request, job, raw) = fixture_for(true);
        let before = preserved(&executor, &job);
        assert_eq!(executor.execute(request.clone()).await.unwrap(), raw);
        executor.reject(&request, &raw, "合成仍未解决的领域错误").await.unwrap();
        assert!(executor.execute(request.clone()).await.unwrap_err().contains("SDK_Q1_REPLAY_ALREADY_REJECTED"));
        assert!(executor.accept(&request, &raw).await.unwrap_err().contains("REPLAY_HELD"));
        assert!(executor.reject(&request, &raw, "另一个合成错误").await.is_err());
        let original = executor.sdk_job(&job).unwrap();
        assert_eq!(original["status"], "rejected");
        assert_eq!(original["error"], Q1_REJECTION);
        assert_eq!(original["receipt"], json!({"contract":"desktop-runner/v1","validation":"rejected"}));
        assert_eq!(preserved(&executor,&job), before);
        assert_eq!(executor.store.jobs().unwrap().len(), 2);
        assert!(!executor.store.root.join("sdk-worker").exists());
        let n:i64 = executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type=?2",params![job,Q1.rejected_event],|r|r.get(0)).unwrap();
        assert_eq!(n,1);
    }

    #[tokio::test]
    async fn sdk_q1_accepted_cache_requires_one_exact_replay_event() {
        let (_temp,executor,request,job,raw)=fixture_for(true);
        executor.accept(&request,&raw).await.unwrap();
        let original=preserved(&executor,&job);
        let c=executor.store.conn().unwrap();
        let data:String=c.query_row("SELECT data FROM events WHERE job_id=?1 AND type=?2",params![job,Q1.accepted_event],|r|r.get(0)).unwrap();
        executor.store.event(&job,Q1.accepted_event,&serde_json::from_str::<Value>(&data).unwrap()).unwrap();
        assert!(executor.execute(request.clone()).await.unwrap_err().contains("SDK_RAW_BINDING_MISMATCH"));
        assert_eq!(preserved(&executor,&job),original);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }

    #[tokio::test]
    async fn sdk_q1_replay_exact_contract_and_unknown_state_cannot_be_bypassed() {
        let (_temp, executor, request, job, raw) = fixture_for(true);
        let before = preserved(&executor, &job);
        for change in ["phase","profile","decisionPolicy","stage","skill","source","rules","model","schema"] {
            let mut changed=request.clone();
            match change {
                "phase" => changed.input["delivery"]["phase"]=json!("AUTO_INTERPRET"),
                "profile" => changed.input["modelInputProfile"]=json!("other/v1"),
                "decisionPolicy" => changed.input["foundationDecisionPolicy"]=json!("other/v1"),
                "stage" => changed.stage="foundation:F2".into(),
                "skill" => changed.skill="foundation-asset-catalog".into(),
                "source" => changed.input["sourceSha256"]=json!("other"),
                "rules" => changed.rule_snapshot["primary"]["content"]=json!("changed rules"),
                "model" => changed.model="other".into(),
                _ => changed.schema["additionalProperties"]=json!(true),
            }
            assert!(executor.accept(&changed,&raw).await.is_err(),"{change}");
        }
        assert!(executor.accept(&request,&json!({"ok":true,"facts":["改写事实"]})).await.unwrap_err().contains("DOMAIN_RESULT_CHANGED"));
        executor.store.conn().unwrap().execute("UPDATE jobs SET state='submission_unknown' WHERE id=?1",[&job]).unwrap();
        assert!(executor.accept(&request,&raw).await.is_err());
        assert_eq!(preserved(&executor,&job),before);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }

    #[test]
    fn sdk_q1_proof_parent_event_checkpoint_and_file_pollution_fail_closed() {
        for changed in ["policy","stage","previousError","workflowId","nodeKey","originalJobId","requestHash",
            "originalSnapshotJson","originalResultJson","originalPreparedJson","originalReceiptJson",
            "originalWorkflowSnapshotJson","originalRawEventId","originalRawEventDataJson",
            "originalSdkRawEventId","originalSdkRawEventDataJson","originalAttemptSha256",
            "originalCompiledSha256","originalRawTextSha256","generationRequestsSent","acceptedAutomatically"] {
            let (_temp,executor,request,job,raw)=fixture_for(true);
            let snapshot=executor.request_snapshot(&request).unwrap();
            let audit=executor.sdk_f2_replay_audit(&request,&snapshot,&executor.sdk_job(&job).unwrap()).unwrap();
            assert!(audit_valid(&audit) && files_bound(&executor.store.root,&audit));
            let mut forged=audit.clone(); forged[changed]=json!("polluted");
            assert!(!executor.store.commit_sdk_f2_episode_replay(&forged,&raw,None).unwrap_or(false),"{changed}");
            assert_eq!(executor.sdk_job(&job).unwrap()["status"],"rejected");
        }
        for changed in ["parent","checkpoint","rawEvent","sdkEvent","sdk-attempt.json","sdk-compiled.json","result.json"] {
            let (_temp,executor,request,job,raw)=fixture_for(true);
            let before=preserved(&executor,&job);
            let snapshot=executor.request_snapshot(&request).unwrap();
            let audit=executor.sdk_f2_replay_audit(&request,&snapshot,&executor.sdk_job(&job).unwrap()).unwrap();
            let c=executor.store.conn().unwrap();
            match changed {
                "parent" => { c.execute("UPDATE jobs SET snapshot='{}' WHERE id=?1",[executor.workflow_id.as_ref().unwrap()]).unwrap(); }
                "checkpoint" => { c.execute("UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",[&job]).unwrap(); }
                "rawEvent" => executor.store.event(&job,"text_received_raw",&json!({"requestHash":audit["requestHash"],"value":raw})).unwrap(),
                "sdkEvent" => executor.store.event(&job,"sdk_raw_result",&serde_json::from_str::<Value>(s(&audit,"originalSdkRawEventDataJson")).unwrap()).unwrap(),
                file => std::fs::write(executor.store.root.join("text-runs").join(&job).join(file),b"polluted file").unwrap(),
            }
            assert!(!executor.store.commit_sdk_f2_episode_replay(&audit,&raw,None).unwrap_or(false),"{changed}");
            assert_eq!(executor.sdk_job(&job).unwrap()["status"],"rejected");
            if matches!(changed,"parent"|"checkpoint"|"rawEvent"|"sdkEvent") { assert_eq!(preserved(&executor,&job),before); }
            assert!(!executor.store.root.join("sdk-worker").exists());
        }
    }

    #[tokio::test]
    async fn sdk_q1_commit_failure_is_atomic_and_does_not_consume_original_result() {
        let (_temp,executor,request,job,raw)=fixture_for(true);
        let before=preserved(&executor,&job);
        executor.store.conn().unwrap().execute_batch("CREATE TRIGGER replay_interrupted BEFORE UPDATE ON workflow_steps BEGIN SELECT RAISE(ROLLBACK,'synthetic checkpoint conflict'); END;").unwrap();
        assert!(executor.accept(&request,&raw).await.unwrap_err().contains("synthetic checkpoint conflict"));
        assert_eq!(executor.sdk_job(&job).unwrap()["status"],"rejected");
        assert_eq!(preserved(&executor,&job),before);
        let n:i64=executor.store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type=?2",params![job,Q1.accepted_event],|r|r.get(0)).unwrap();
        assert_eq!(n,0);
        assert!(!executor.store.root.join("sdk-worker").exists());
    }

    #[test]
    #[ignore = "requires explicit captured SDK Q1 sqlite/root/job; source files read-only, isolated proof only, zero model or adoption"]
    fn sdk_q1_captured_original_proof_is_bound_without_dispatch_or_adoption() {
        let required = |name| std::env::var(name).expect("explicit private proof fixture path required");
        let source_db=PathBuf::from(required("DIRECTOR_SDK_Q1_PROOF_DB"));
        let source_root=PathBuf::from(required("DIRECTOR_SDK_Q1_PROOF_ROOT"));
        let job=required("DIRECTOR_SDK_Q1_PROOF_JOB");
        let temp=tempfile::tempdir().unwrap();
        let root=temp.path().join("isolated"); std::fs::create_dir(&root).unwrap();
        std::fs::copy(source_db,root.join("project.sqlite")).unwrap();
        let store=Store::open(root.clone()).unwrap();
        let c=store.conn().unwrap();
        let (snapshot,parent,run):(String,String,String)=c.query_row(
            "SELECT j.snapshot,p.snapshot,p.id FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id JOIN jobs p ON p.id=w.workflow_id WHERE j.id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND j.state='rejected'",[&job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let snapshot:Value=serde_json::from_str(&snapshot).unwrap();
        let parent:Value=serde_json::from_str(&parent).unwrap();
        let request:TextRequest=serde_json::from_value(snapshot.clone()).unwrap();
        let executor=SavedTextExecutor {store,settings:parent["input"]["settings"].clone(),workflow_id:Some(run)};
        let target=root.join("text-runs").join(&job);std::fs::create_dir_all(&target).unwrap();
        for name in ["sdk-attempt.json","sdk-compiled.json","result.json"] {
            std::fs::copy(source_root.join("text-runs").join(&job).join(name),target.join(name)).unwrap();
        }
        let original=executor.sdk_job(&job).unwrap();
        let audit=executor.sdk_f2_replay_audit(&request,&snapshot,&original).unwrap();
        assert_eq!(audit["policy"],Q1_POLICY);
        assert!(audit_valid(&audit),"captured exact parent/request/raw/rules proof must validate");
        assert!(files_bound(&root,&audit),"captured completed attempt and immutable compilation must validate");
        assert!(database_bound(&executor.store.conn().unwrap(),&audit,false,&original["prepared"],&original["receipt"],&original["result"]).unwrap());
        assert_eq!(executor.sdk_job(&job).unwrap(),original);
        assert!(!root.join("sdk-worker").exists());
    }
}

impl Store {
    /// Only the exact original SDK foundation node advances. Raw/request/transport files stay untouched.
    fn commit_sdk_f2_episode_replay(
        &self,
        audit: &Value,
        result: &Value,
        reason: Option<&str>,
    ) -> Result<bool> {
        if !audit_valid(audit) || !files_bound(&self.root, audit) {
            return Err("SDK_F2_REPLAY_PROOF_INVALID".into());
        }
        let prepared: Value =
            serde_json::from_str(s(audit, "originalPreparedJson")).map_err(|e| e.to_string())?;
        let receipt: Value =
            serde_json::from_str(s(audit, "originalReceiptJson")).map_err(|e| e.to_string())?;
        let raw: Value =
            serde_json::from_str(s(audit, "originalResultJson")).map_err(|e| e.to_string())?;
        if reason.is_some() && result != &raw {
            return Err("SDK_F2_REPLAY_REJECTION_NOT_ORIGINAL_RAW".into());
        }
        let snapshot: Value =
            serde_json::from_str(s(audit, "originalSnapshotJson")).map_err(|e| e.to_string())?;
        let policy = replay_policy(&snapshot,&audit["previousError"]).ok_or("SDK_FOUNDATION_REPLAY_PROOF_INVALID")?;
        let parent:Value=serde_json::from_str(s(audit,"originalWorkflowSnapshotJson")).map_err(|e|e.to_string())?;
        if reason.is_none() && !domain_result_matches(&snapshot,&parent,&raw,result)
        {
            return Err(
                "SDK_F2_REPLAY_DOMAIN_RESULT_CHANGED: 仅允许原冻结引用规范化，不改事实".into(),
            );
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if !database_bound(&tx, audit, false, &prepared, &receipt, &raw)?
            || !files_bound(&self.root, audit)
        {
            return Ok(false);
        }
        if matches!(policy.name,Q1_POLICY|PLAN_POLICY|FRAGMENT_POLICY) && tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type IN (?2,?3))",
            params![s(audit,"originalJobId"), policy.accepted_event, policy.rejected_event], |r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
            return Ok(false);
        }
        let event = json!({"policy":policy.name,"migrationAudit":audit,"generationRequestsSent":0,"validation":if reason.is_none(){"accepted"}else{"rejected"},"error":reason,"validatedResultSha256":crate::pipeline::digest(&result.to_string()),"originalEvidencePreserved":true});
        let event_kind = if reason.is_none() {
            policy.accepted_event
        } else {
            policy.rejected_event
        };
        if reason.is_none() {
            let mut next = prepared;
            next["sdkDomainResult"] = result.clone();
            next["sdkDomainResultSha256"] = json!(crate::pipeline::digest(&result.to_string()));
            let next_receipt = json!({"contract":"desktop-runner/v1","validation":"accepted","migrationPolicy":policy.name,"migrationAudit":audit,"acceptedRequestHash":audit["requestHash"],"validatedResultSha256":crate::pipeline::digest(&result.to_string())});
            if tx.execute("UPDATE jobs SET state='completed',error=NULL,prepared=?1,receipt=?2,updated_at=?3 WHERE id=?4 AND state='rejected'",params![next.to_string(),next_receipt.to_string(),now(),s(audit,"originalJobId")]).map_err(|e|e.to_string())? != 1 { return Err("SDK_F2_REPLAY_CAS".into()); }
            if tx.execute("UPDATE workflow_steps SET state='accepted',error=NULL,result=?1,updated_at=?2 WHERE workflow_id=?3 AND node_key=?4 AND text_job_id=?5 AND state='rejected' AND request_hash=?6",params![result.to_string(),now(),s(audit,"workflowId"),s(audit,"nodeKey"),s(audit,"originalJobId"),s(audit,"requestHash")]).map_err(|e|e.to_string())? != 1 { return Err("SDK_F2_REPLAY_CHECKPOINT_CAS".into()); }
        }
        // A new rejection is diagnostic only: retain the original rejection and
        // original bytes. It does not grant another model attempt or general replay.
        tx.execute("INSERT INTO events(job_id,type,data,created_at) SELECT ?1,?2,?3,?4 WHERE NOT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type=?2 AND data=?3)",params![s(audit,"originalJobId"),event_kind,event.to_string(),now()]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }
}

#[cfg(test)]
#[path="text_executor_sdk_planner_replay_tests.rs"]
mod planner_replay_tests;

#[cfg(test)]
#[path="text_executor_sdk_fragment_replay_tests.rs"]
mod fragment_replay_tests;
