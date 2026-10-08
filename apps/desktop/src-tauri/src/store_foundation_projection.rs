//! Evidence-only foundation context for a new workflow. No historical row,
//! result, request, file or receipt is updated by this module.
use crate::pipeline::foundation_delivery::{
    accepted_scope_reason_raw_matches, project_missing_scope_reasons, AcceptedFoundationReasonStep,
    METHOD_VERSION,
};
use crate::pipeline::{digest, FoundationProposal, LockedSource, TextRequest};
use crate::store::{s, Result, Store};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::{fs, path::Path, time::Duration};

const CONTEXT_VERSION: &str = "foundation-scope-reason-context/v1";
const MAX_WORKFLOWS: usize = 32;
const MAX_STEPS: usize = 4096;

pub(crate) struct FoundationContextProjection {
    pub foundation: Value,
    pub diagnostic: Value,
}

struct AcceptedStep {
    stage: String,
    input: Value,
    result: Value,
    request_hash: String,
}

fn read_only_history_snapshot(root: &Path) -> Result<tempfile::TempDir> {
    let identity = |directory: &Path| -> Result<_> {
        let database = crate::store::hash_file(&directory.join("project.sqlite"))
            .map_err(|_| "FOUNDATION_SNAPSHOT_UNAVAILABLE".to_string())?;
        let wal_path = directory.join("project.sqlite-wal");
        let wal = if wal_path
            .try_exists()
            .map_err(|_| "FOUNDATION_SNAPSHOT_UNAVAILABLE")?
        {
            Some(
                crate::store::hash_file(&wal_path)
                    .map_err(|_| "FOUNDATION_SNAPSHOT_UNAVAILABLE")?,
            )
        } else {
            None
        };
        Ok((database, wal))
    };
    let before = identity(root)?;
    let snapshot = tempfile::Builder::new()
        .prefix("director-foundation-read-")
        .tempdir()
        .map_err(|_| "FOUNDATION_SNAPSHOT_UNAVAILABLE")?;
    fs::copy(
        root.join("project.sqlite"),
        snapshot.path().join("project.sqlite"),
    )
    .map_err(|_| "FOUNDATION_SNAPSHOT_UNAVAILABLE")?;
    if before.1.is_some() {
        fs::copy(
            root.join("project.sqlite-wal"),
            snapshot.path().join("project.sqlite-wal"),
        )
        .map_err(|_| "FOUNDATION_SNAPSHOT_CHANGED")?;
    }
    if identity(root)? != before || identity(snapshot.path())? != before {
        return Err("FOUNDATION_SNAPSHOT_CHANGED: 原记录在读取期间变化，未构建新上下文".into());
    }
    // SQLITE_OPEN_READ_ONLY may create an empty WAL/index even for a SELECT.
    // Only the private byte-identical DB+existing-WAL copy may acquire those
    // sidecars. Never open/init/checkpoint the original or ignore its WAL.
    Ok(snapshot)
}

impl Store {
    /// Call once when freezing a *new* Stage2/3 context. Existing workflow
    /// resumes must retain their original snapshot and must not call this API.
    pub(crate) fn foundation_context_for_new_workflow(
        &self,
    ) -> Result<FoundationContextProjection> {
        let snapshot = read_only_history_snapshot(&self.root)?;
        let mut connection = Connection::open_with_flags(
            snapshot.path().join("project.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        // One deferred read transaction gives source, catalogue and all receipts
        // a consistent SQLite snapshot, without DDL or write transactions.
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let saved: Option<(i64, i64, String)> = transaction
            .query_row(
                "SELECT version,source_version,data FROM foundations ORDER BY version DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((version, source_version, serialized)) = saved else {
            return Ok(preserved(Value::Null, "NO_FOUNDATION"));
        };
        let current: Value =
            serde_json::from_str(&serialized).map_err(|error| error.to_string())?;
        let has_missing = current["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|asset| {
                asset["variants"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|variant| {
                        variant["scope"]["kind"] == "SCENES"
                            && variant["scope"]["reason"]
                                .as_str()
                                .is_none_or(|reason| reason.trim().is_empty())
                    })
            });
        if !has_missing {
            return Ok(preserved(current, "NO_MISSING_SCOPE_REASON"));
        }
        let outcome = assemble(&transaction, version, source_version, &current);
        Ok(match outcome {
            Ok(Some((foundation, diagnostic))) => FoundationContextProjection {
                foundation,
                diagnostic,
            },
            Ok(None) => preserved(current, "NO_BOUND_ACCEPTED_FOUNDATION"),
            Err(code) => preserved(current, &code),
        })
    }
}

fn preserved(foundation: Value, code: &str) -> FoundationContextProjection {
    FoundationContextProjection {
        foundation,
        diagnostic: json!({"version":CONTEXT_VERSION,"status":"preserved","code":code,"newModelCalls":0,"historyUnchanged":true}),
    }
}

fn source_at(connection: &Connection, version: Option<i64>) -> Result<(i64, Value, LockedSource)> {
    let sql = if version.is_some() {
        "SELECT version,text,sha256,data FROM sources WHERE version=?1"
    } else {
        "SELECT version,text,sha256,data FROM sources ORDER BY version DESC LIMIT 1"
    };
    let read = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    };
    let (version, text, sha, serialized) = if let Some(version) = version {
        connection.query_row(sql, [version], read)
    } else {
        connection.query_row(sql, [], read)
    }
    .map_err(|_| "SOURCE_RECORD_MISSING".to_string())?;
    let value: Value =
        serde_json::from_str(&serialized).map_err(|_| "SOURCE_RECORD_INVALID".to_string())?;
    let locked: LockedSource =
        serde_json::from_value(value.clone()).map_err(|_| "SOURCE_RECORD_INVALID".to_string())?;
    if value["version"] != version
        || locked.text != text
        || locked.sha256 != sha
        || digest(&text) != sha
    {
        return Err("SOURCE_IDENTITY_MISMATCH".into());
    }
    crate::pipeline::validate_source(&locked)
        .map_err(|_| "SOURCE_IDENTITY_MISMATCH".to_string())?;
    Ok((version, value, locked))
}

fn assemble(
    connection: &Connection,
    foundation_version: i64,
    foundation_source_version: i64,
    current: &Value,
) -> Result<Option<(Value, Value)>> {
    let (source_version, _, source) = source_at(connection, None)?;
    if current["version"] != foundation_version
        || current["sourceVersion"] != foundation_source_version
        || foundation_source_version != source_version
        || current["sourceSha256"] != source.sha256
    {
        return Err("FOUNDATION_SOURCE_IDENTITY_MISMATCH".into());
    }
    let current_proposal: FoundationProposal = serde_json::from_value(current.clone())
        .map_err(|_| "FOUNDATION_RECORD_INVALID".to_string())?;
    let has_steps: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_steps')", [], |row| row.get(0))
        .map_err(|_| "FOUNDATION_EVIDENCE_UNAVAILABLE".to_string())?;
    if !has_steps {
        return Ok(None);
    }
    let project_id: String = connection
        .query_row("SELECT id FROM project LIMIT 1", [], |row| row.get(0))
        .map_err(|_| "FOUNDATION_PROJECT_IDENTITY_MISSING".to_string())?;
    let mut statement = connection.prepare(
        "SELECT id,snapshot,result FROM jobs WHERE kind='workflow' AND entity_id='foundation.run' AND state='completed' AND error IS NULL ORDER BY rowid DESC LIMIT ?1",
    ).map_err(|_| "FOUNDATION_EVIDENCE_UNAVAILABLE".to_string())?;
    let workflows = statement
        .query_map([MAX_WORKFLOWS as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|_| "FOUNDATION_EVIDENCE_UNAVAILABLE".to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| "FOUNDATION_EVIDENCE_UNAVAILABLE".to_string())?;
    let mut rejection = None;
    for (workflow_id, snapshot_text, result_text) in workflows {
        match project_workflow(
            connection,
            &workflow_id,
            &snapshot_text,
            result_text.as_deref(),
            &project_id,
            &current_proposal,
            current,
            &source,
        ) {
            Ok(Some(projected)) => return Ok(Some(projected)),
            Ok(None) => {}
            Err(code) => rejection = Some(code),
        }
    }
    if let Some(rejection) = rejection {
        return Err(rejection);
    }
    Ok(None)
}

fn project_workflow(
    connection: &Connection,
    workflow_id: &str,
    snapshot_text: &str,
    result_text: Option<&str>,
    project_id: &str,
    current_proposal: &FoundationProposal,
    current: &Value,
    current_source: &LockedSource,
) -> Result<Option<(Value, Value)>> {
    let snapshot: Value =
        serde_json::from_str(snapshot_text).map_err(|_| "WORKFLOW_SNAPSHOT_INVALID".to_string())?;
    if snapshot["stage"] != "foundation.run"
        || snapshot["runnerVersion"] != "desktop-runner/v1"
        || snapshot["foundationMethod"] != METHOD_VERSION
        || snapshot["input"]["source"]["sha256"] != current_source.sha256
    {
        return Ok(None);
    }
    let original_version = snapshot["input"]["source"]["version"]
        .as_i64()
        .ok_or("WORKFLOW_SOURCE_IDENTITY_MISSING")?;
    let (_, original_source_value, original_source) =
        source_at(connection, Some(original_version))?;
    if snapshot["sourceVersion"] != original_version
        || snapshot["guard"]["sourceVersion"] != original_version
        || original_source_value != snapshot["input"]["source"]
        || snapshot["input"]["projectId"]
            .as_str()
            .is_some_and(|id| id != project_id)
    {
        return Err("WORKFLOW_SOURCE_IDENTITY_MISMATCH".into());
    }
    // The original publication must still be an exact saved foundation row.
    // Later Stage2 top-level additions may exist in the current foundation.
    let published: Value = serde_json::from_str(result_text.ok_or("WORKFLOW_PUBLICATION_MISSING")?)
        .map_err(|_| "WORKFLOW_PUBLICATION_INVALID".to_string())?;
    let published_version = published["version"]
        .as_i64()
        .ok_or("WORKFLOW_PUBLICATION_INVALID")?;
    let published_source_version = published["sourceVersion"]
        .as_i64()
        .ok_or("WORKFLOW_PUBLICATION_INVALID")?;
    let saved: Option<String> = connection
        .query_row(
            "SELECT data FROM foundations WHERE version=?1 AND source_version=?2",
            params![published_version, published_source_version],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "WORKFLOW_PUBLICATION_INVALID".to_string())?;
    let saved: Value =
        serde_json::from_str(saved.as_deref().ok_or("WORKFLOW_PUBLICATION_MISSING")?)
            .map_err(|_| "WORKFLOW_PUBLICATION_INVALID".to_string())?;
    if published != saved || published["sourceSha256"] != current_source.sha256 {
        return Err("WORKFLOW_PUBLICATION_MISMATCH".into());
    }
    let steps = accepted_steps(connection, workflow_id, &snapshot, &original_source)?;
    let borrowed: Vec<_> = steps
        .iter()
        .map(|step| AcceptedFoundationReasonStep {
            stage: &step.stage,
            input: &step.input,
            result: &step.result,
        })
        .collect();
    let projection = project_missing_scope_reasons(
        &original_source,
        &snapshot["input"],
        current_proposal,
        &borrowed,
    )
    .map_err(|_| "FOUNDATION_CATALOGUE_REPLAY_MISMATCH".to_string())?;
    if serde_json::to_value(&projection.projected_source)
        .map_err(|_| "SOURCE_PROJECTION_INVALID")?
        != serde_json::to_value(current_source).map_err(|_| "SOURCE_PROJECTION_INVALID")?
    {
        return Err("PUBLISHED_SOURCE_PROJECTION_MISMATCH".into());
    }
    // Keep every unknown/top-level/production property in the original JSON;
    // only copy the explicitly restored reasons from the validated projection.
    let mut foundation = current.clone();
    let mut restored_ids = Vec::new();
    for asset in foundation["assets"]
        .as_array_mut()
        .ok_or("FOUNDATION_RECORD_INVALID")?
    {
        let asset_id = s(asset, "id").to_string();
        let projected_asset = projection
            .foundation
            .assets
            .iter()
            .find(|entry| entry.id == asset_id)
            .ok_or("FOUNDATION_CATALOGUE_REPLAY_MISMATCH")?;
        for variant in asset["variants"]
            .as_array_mut()
            .ok_or("FOUNDATION_RECORD_INVALID")?
        {
            let id = s(variant, "id");
            // IDs are unique within their owner, not across the catalogue.
            // Compare the actual owner/id projection; a second owner's `base`
            // must never overwrite an existing reason or select a GLOBAL row.
            let projected_variant = projected_asset
                .variants
                .iter()
                .find(|entry| entry.id == id)
                .ok_or("FOUNDATION_CATALOGUE_REPLAY_MISMATCH")?;
            if variant["scope"]["kind"] == "SCENES"
                && variant["scope"]["reason"]
                    .as_str()
                    .is_none_or(|reason| reason.trim().is_empty())
                && projected_variant
                    .scope
                    .reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty())
            {
                restored_ids.push(id.to_owned());
                variant["scope"]["reason"] = json!(projected_variant.scope.reason);
            }
        }
    }
    if restored_ids != projection.restored_variant_ids {
        return Err("FOUNDATION_REASON_PROJECTION_INVALID".into());
    }
    Ok(Some((
        foundation,
        json!({"version":CONTEXT_VERSION,"status":"projected","workflowId":workflow_id,"workflowSnapshotSha256":digest(snapshot_text),"restoredVariantIds":projection.restored_variant_ids,"acceptedRequestHashes":steps.iter().map(|step|&step.request_hash).collect::<Vec<_>>(),"sourceSha256":current_source.sha256,"foundationVersion":current["version"],"newModelCalls":0,"historyUnchanged":true}),
    )))
}

fn rule_bound(rule: &Value, frozen: &Value) -> bool {
    let content = s(rule, "content");
    !s(rule, "id").is_empty()
        && !s(rule, "version").is_empty()
        && !content.is_empty()
        && digest(content) == s(rule, "sha256")
        && frozen
            .as_array()
            .is_some_and(|rules| rules.iter().any(|saved| saved == rule))
}

fn accepted_steps(
    connection: &Connection,
    workflow: &str,
    parent: &Value,
    source: &LockedSource,
) -> Result<Vec<AcceptedStep>> {
    let mut statement = connection.prepare(
        "SELECT w.stage,w.request_hash,w.result,w.error,j.kind,j.provider,j.state,j.snapshot,j.result,j.receipt,j.error,j.prepared,(SELECT e.data FROM events e WHERE e.job_id=j.id AND e.type='text_received_raw' ORDER BY e.id DESC LIMIT 1) FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.state='accepted' AND w.stage IN ('foundation:F2','foundation:Q1') ORDER BY w.rowid LIMIT ?2",
    ).map_err(|_| "ACCEPTED_FOUNDATION_EVIDENCE_INVALID".to_string())?;
    let rows = statement
        .query_map(params![workflow, MAX_STEPS as i64 + 1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                (2..13)
                    .map(|index| row.get::<_, Option<String>>(index))
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            ))
        })
        .map_err(|_| "ACCEPTED_FOUNDATION_EVIDENCE_INVALID".to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| "ACCEPTED_FOUNDATION_EVIDENCE_INVALID".to_string())?;
    if rows.is_empty() || rows.len() > MAX_STEPS {
        return Err("ACCEPTED_FOUNDATION_EVIDENCE_MISSING".into());
    }
    let mut steps = Vec::with_capacity(rows.len());
    for (stage, hash, fields) in rows {
        let field = |index: usize| {
            fields[index]
                .as_deref()
                .ok_or("ACCEPTED_FOUNDATION_EVIDENCE_MISSING")
        };
        // fields are checkpoint result/error, kind/provider/state,
        // snapshot/result/receipt/error/prepared/raw-event, in that order.
        let snapshot_text = field(5)?;
        let snapshot: Value =
            serde_json::from_str(snapshot_text).map_err(|_| "ACCEPTED_REQUEST_INVALID")?;
        let request: TextRequest =
            serde_json::from_value(snapshot.clone()).map_err(|_| "ACCEPTED_REQUEST_INVALID")?;
        let result: Value =
            serde_json::from_str(field(0)?).map_err(|_| "ACCEPTED_RESULT_INVALID")?;
        let job_result: Value =
            serde_json::from_str(field(6)?).map_err(|_| "ACCEPTED_RESULT_INVALID")?;
        let receipt: Value =
            serde_json::from_str(field(7)?).map_err(|_| "ACCEPTED_RECEIPT_INVALID")?;
        let event: Value =
            serde_json::from_str(field(10)?).map_err(|_| "ACCEPTED_RAW_EVENT_INVALID")?;
        let provider = field(3)?;
        let expected_skill = if stage == "foundation:F2" {
            "foundation-asset-catalog"
        } else {
            "foundation-finishing-editor"
        };
        if field(2)? != "text"
            || field(4)? != "completed"
            || fields[1].is_some()
            || fields[8].is_some()
            || digest(snapshot_text) != hash
            || request.stage != stage
            || request.skill != expected_skill
            || request.input["sourceSha256"] != source.sha256
            || s(&snapshot["executorIdentity"], "provider") != provider
            || !["codex", "codex_sdk", "api"].contains(&provider)
            || parent["input"]["settings"]["textProvider"]
                .as_str()
                .is_some_and(|original| original != provider)
            || parent["input"]["settings"]["textModel"]
                .as_str()
                .is_some_and(|model| model != request.model)
            || parent["input"]["settings"]["reasoning"]
                .as_str()
                .is_some_and(|reasoning| reasoning != request.reasoning)
            || receipt["contract"] != "desktop-runner/v1"
            || receipt["validation"] != "accepted"
            || receipt
                .get("migrationPolicy")
                .is_some_and(|policy| !policy.is_null())
            || receipt
                .get("acceptedRequestHash")
                .is_some_and(|bound| bound != &json!(hash))
            || event["requestHash"] != hash
            || event["value"].is_null()
            || s(&request.rule_snapshot["primary"], "id") != expected_skill
            || !rule_bound(&request.rule_snapshot["primary"], &parent["rules"])
            || !request.rule_snapshot["methods"]
                .as_array()
                .is_some_and(|rules| rules.iter().all(|rule| rule_bound(rule, &parent["rules"])))
        {
            return Err("ACCEPTED_FOUNDATION_BINDING_MISMATCH".into());
        }
        if provider == "codex_sdk" {
            let prepared: Value =
                serde_json::from_str(field(9)?).map_err(|_| "ACCEPTED_SDK_DOMAIN_INVALID")?;
            if job_result != event["value"]
                || prepared["sdkDomainResult"] != result
                || prepared["sdkDomainResultSha256"] != digest(&result.to_string())
                || !accepted_scope_reason_raw_matches(&request.input, &event["value"], &result)
            {
                return Err("ACCEPTED_SDK_DOMAIN_MISMATCH".into());
            }
        } else if job_result != result
            || !accepted_scope_reason_raw_matches(&request.input, &event["value"], &result)
        {
            return Err("ACCEPTED_RAW_RESULT_MISMATCH".into());
        }
        steps.push(AcceptedStep {
            stage,
            input: request.input,
            result,
            request_hash: hash,
        });
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::foundation_delivery::reason_projection_test_fixture;
    use std::{
        collections::BTreeMap,
        fs,
        io::Write,
        path::{Path, PathBuf},
    };

    struct Fixture {
        _directory: tempfile::TempDir,
        store: Store,
        workflow: String,
        jobs: Vec<String>,
        current: Value,
    }

    fn fixture(provider: &str) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("project")).unwrap();
        store
            .create("Synthetic reason projection", "synthetic-project")
            .unwrap();
        store.ensure_workflow_storage().unwrap();
        let (source, mut input, current, steps) = reason_projection_test_fixture();
        let source = store.save_source(json!(source)).unwrap();
        input["source"] = source.clone();
        input["projectId"] = json!("synthetic-project");
        input["settings"] =
            json!({"textProvider":provider,"textModel":"gpt-6.1-sol","reasoning":"medium"});
        let rules: Vec<_> = crate::pipeline::rule_snapshots()
            .into_iter()
            .filter(|rule| {
                [
                    "screenplay-fidelity",
                    "foundation-asset-catalog",
                    "foundation-finishing-editor",
                ]
                .contains(&rule.id.as_str())
            })
            .collect();
        let parent = json!({"stage":"foundation.run","runnerVersion":"desktop-runner/v1","foundationMethod":METHOD_VERSION,
            "sourceVersion":source["version"],"guard":{"sourceVersion":source["version"]},"input":input,"rules":rules});
        let workflow = store
            .add_job(
                "workflow",
                "foundation.run",
                provider,
                "",
                &parent,
                "running",
            )
            .unwrap();
        let mut jobs = Vec::new();
        for (stage, input, result) in steps {
            let skill = if stage == "foundation:F2" {
                "foundation-asset-catalog"
            } else {
                "foundation-finishing-editor"
            };
            let primary = rules.iter().find(|rule| rule.id == skill).unwrap();
            let request = TextRequest {
                stage: stage.clone(),
                skill: skill.into(),
                model: "gpt-6.1-sol".into(),
                reasoning: "medium".into(),
                input,
                schema: json!({"type":"object"}),
                rule_snapshot: json!({"primary":primary,"methods":[]}),
            };
            let mut snapshot = json!(request);
            snapshot["executorIdentity"] = json!({"provider":provider,"synthetic":true});
            let hash = digest(&snapshot.to_string());
            let job = store
                .add_job("text", &stage, provider, "", &snapshot, "running")
                .unwrap();
            // Raw and accepted domain are independently bound even when this
            // fixture does not need source-reference normalization.
            let raw = result.clone();
            let mut fields = json!({"result":raw,"receipt":{"contract":"desktop-runner/v1","validation":"accepted"}});
            if provider == "codex_sdk" {
                fields["prepared"] = json!({"sdkDomainResult":result,"sdkDomainResultSha256":digest(&result.to_string())});
            }
            store.update_job(&job, "completed", &fields).unwrap();
            store
                .event(
                    &job,
                    "text_received_raw",
                    &json!({"requestHash":hash,"value":raw}),
                )
                .unwrap();
            store
                .checkpoint(
                    &workflow,
                    &stage,
                    &stage,
                    &hash,
                    "accepted",
                    Some(&job),
                    Some(&result),
                    None,
                )
                .unwrap();
            fs::create_dir_all(store.root.join("text-runs").join(&job)).unwrap();
            fs::write(
                store
                    .root
                    .join("text-runs")
                    .join(&job)
                    .join("snapshot.json"),
                snapshot.to_string(),
            )
            .unwrap();
            fs::write(
                store.root.join("text-runs").join(&job).join("raw.json"),
                raw.to_string(),
            )
            .unwrap();
            jobs.push(job);
        }
        let published = store.save_foundation(json!(current)).unwrap();
        store
            .update_job(&workflow, "completed", &json!({"result":published}))
            .unwrap();
        let mut next = published;
        next["direction"] = json!("Preserved production direction");
        next["stage2Extension"] = json!({"opaque":"preserve unknown top-level fields"});
        let current = store.save_foundation(next).unwrap();
        Fixture {
            _directory: directory,
            store,
            workflow,
            jobs,
            current,
        }
    }

    // Hash bytes without opening/initing Store or printing private file names.
    // SQLite shared-memory lock pages are ephemeral; DB/WAL and all original
    // source/raw/request files are included, as are additions/deletions.
    fn file_evidence(root: &Path) -> BTreeMap<String, (String, u64)> {
        fn visit(root: &Path, directory: &Path, out: &mut BTreeMap<String, (String, u64)>) {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let kind = entry.file_type().unwrap();
                assert!(
                    !kind.is_symlink(),
                    "read-only evidence does not follow symlinks"
                );
                if kind.is_dir() {
                    visit(root, &path, out);
                } else if kind.is_file() && entry.file_name() != "project.sqlite-shm" {
                    let relative = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.insert(relative, crate::store::hash_file(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        visit(root, root, &mut out);
        out
    }

    fn verify_preserved(fixture: &Fixture) {
        let before = file_evidence(&fixture.store.root);
        let original = fixture.store.foundation().unwrap();
        let projection = fixture.store.foundation_context_for_new_workflow().unwrap();
        assert_eq!(projection.diagnostic["status"], "preserved");
        assert_eq!(projection.foundation, original);
        assert_eq!(projection.diagnostic["newModelCalls"], 0);
        assert!(
            before == file_evidence(&fixture.store.root),
            "projection changed historical file bytes"
        );
    }

    #[test]
    fn accepted_chain_restores_only_missing_reasons_without_writing_history() {
        let fixture = fixture("codex");
        let before = file_evidence(&fixture.store.root);
        let projection = fixture.store.foundation_context_for_new_workflow().unwrap();
        assert_eq!(projection.diagnostic["status"], "projected");
        assert_eq!(
            projection.diagnostic["restoredVariantIds"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let mut only_reasons_removed = projection.foundation.clone();
        for asset in only_reasons_removed["assets"].as_array_mut().unwrap() {
            for variant in asset["variants"].as_array_mut().unwrap() {
                assert!(!s(&variant["scope"], "reason").is_empty());
                variant["scope"]["reason"] = Value::Null;
            }
        }
        assert_eq!(only_reasons_removed, fixture.current);
        assert_eq!(fixture.store.foundation().unwrap(), fixture.current);
        assert!(
            before == file_evidence(&fixture.store.root),
            "old DB/raw/snapshots must remain byte-identical"
        );

        let mut current = fixture.current.clone();
        current["assets"][0]["variants"][0]["scope"]["reason"] = json!("User-confirmed reason");
        fixture
            .store
            .conn()
            .unwrap()
            .execute(
                "UPDATE foundations SET data=?1 WHERE version=?2",
                params![current.to_string(), current["version"].as_i64().unwrap()],
            )
            .unwrap();
        let projection = fixture.store.foundation_context_for_new_workflow().unwrap();
        assert_eq!(
            projection.diagnostic["restoredVariantIds"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            projection.foundation["assets"][0]["variants"][0]["scope"]["reason"],
            "User-confirmed reason"
        );
    }

    #[test]
    fn wal_backed_snapshot_reads_latest_commit_without_changing_original_files() {
        let fixture = fixture("codex");
        let writer = fixture.store.conn().unwrap();
        writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        writer.execute("UPDATE foundations SET data=json_set(data,'$.direction','latest committed WAL direction') WHERE version=2",[]).unwrap();
        assert!(
            fs::metadata(fixture.store.root.join("project.sqlite-wal"))
                .unwrap()
                .len()
                > 0
        );
        let before = file_evidence(&fixture.store.root);
        let projection = fixture.store.foundation_context_for_new_workflow().unwrap();
        assert_eq!(projection.diagnostic["status"], "projected");
        assert_eq!(
            projection.foundation["direction"],
            "latest committed WAL direction"
        );
        assert!(
            before == file_evidence(&fixture.store.root),
            "original committed WAL and DB must remain byte-identical"
        );
        // Keep the original writer alive through the audit: its close checkpoint
        // belongs to fixture setup/teardown, never the read-only projector.
        drop(writer);
    }

    #[test]
    fn changed_raw_request_source_catalogue_receipt_or_rules_do_not_project() {
        for mutation in [
            "raw",
            "request",
            "hash",
            "source",
            "source-version",
            "catalogue",
            "receipt",
            "rule",
            "publication",
            "checkpoint",
            "job-state",
        ] {
            let fixture = fixture("codex");
            let connection = fixture.store.conn().unwrap();
            let job = &fixture.jobs[0];
            match mutation {
                "raw" => {
                    connection.execute("UPDATE events SET data=json_set(data,'$.value.changed',true) WHERE job_id=?1 AND type='text_received_raw'",[job]).unwrap();
                }
                "request" => {
                    connection.execute("UPDATE jobs SET snapshot=json_set(snapshot,'$.input.scope',json('[]')) WHERE id=?1",[job]).unwrap();
                }
                "hash" => {
                    connection
                        .execute(
                            "UPDATE workflow_steps SET request_hash='changed' WHERE text_job_id=?1",
                            [job],
                        )
                        .unwrap();
                }
                "source" => {
                    connection
                        .execute("UPDATE sources SET sha256='changed'", [])
                        .unwrap();
                }
                "source-version" => {
                    connection.execute("UPDATE foundations SET data=json_set(data,'$.sourceVersion',2) WHERE version=2",[]).unwrap();
                }
                "catalogue" => {
                    connection.execute("UPDATE foundations SET data=json_set(data,'$.assets[0].description','manual catalogue change') WHERE version=2",[]).unwrap();
                }
                "receipt" => {
                    connection
                        .execute(
                            "UPDATE jobs SET receipt=json_remove(receipt,'$.contract') WHERE id=?1",
                            [job],
                        )
                        .unwrap();
                }
                "publication" => {
                    connection.execute("UPDATE jobs SET result=json_set(result,'$.direction','changed') WHERE id=?1",[&fixture.workflow]).unwrap();
                }
                "checkpoint" => {
                    connection
                        .execute(
                            "UPDATE workflow_steps SET error='not accepted' WHERE text_job_id=?1",
                            [job],
                        )
                        .unwrap();
                }
                "job-state" => {
                    connection
                        .execute(
                            "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                            [job],
                        )
                        .unwrap();
                }
                "rule" => {
                    let text: String = connection
                        .query_row("SELECT snapshot FROM jobs WHERE id=?1", [job], |row| {
                            row.get(0)
                        })
                        .unwrap();
                    let mut snapshot: Value = serde_json::from_str(&text).unwrap();
                    snapshot["ruleSnapshot"]["primary"]["content"] =
                        json!("different rule with a valid new digest");
                    snapshot["ruleSnapshot"]["primary"]["sha256"] =
                        json!(digest(s(&snapshot["ruleSnapshot"]["primary"], "content")));
                    let hash = digest(&snapshot.to_string());
                    connection
                        .execute(
                            "UPDATE jobs SET snapshot=?1 WHERE id=?2",
                            params![snapshot.to_string(), job],
                        )
                        .unwrap();
                    connection
                        .execute(
                            "UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",
                            params![hash, job],
                        )
                        .unwrap();
                    connection.execute("UPDATE events SET data=json_set(data,'$.requestHash',?1) WHERE job_id=?2 AND type='text_received_raw'",params![hash,job]).unwrap();
                }
                _ => unreachable!(),
            }
            drop(connection);
            verify_preserved(&fixture);
        }
    }

    #[test]
    fn sdk_projection_binds_separate_accepted_domain_and_original_raw() {
        let fixture = fixture("codex_sdk");
        let before = file_evidence(&fixture.store.root);
        let projection = fixture.store.foundation_context_for_new_workflow().unwrap();
        assert_eq!(projection.diagnostic["status"], "projected");
        assert_eq!(
            projection.diagnostic["restoredVariantIds"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(before == file_evidence(&fixture.store.root));
        fixture.store.conn().unwrap().execute("UPDATE jobs SET prepared=json_set(prepared,'$.sdkDomainResultSha256','changed') WHERE id=?1",[&fixture.jobs[0]]).unwrap();
        verify_preserved(&fixture);
    }

    #[test]
    fn incomplete_or_unknown_workflow_preserves_current_context() {
        let fixture = fixture("codex");
        fixture
            .store
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET state='submission_unknown' WHERE id=?1",
                [&fixture.workflow],
            )
            .unwrap();
        verify_preserved(&fixture);
        fixture
            .store
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET state='completed' WHERE id=?1",
                [&fixture.workflow],
            )
            .unwrap();
        fixture
            .store
            .conn()
            .unwrap()
            .execute("DELETE FROM workflow_steps WHERE stage='foundation:Q1'", [])
            .unwrap();
        verify_preserved(&fixture);
    }

    #[test]
    #[ignore = "explicit private project/receipt/count env only; read-only replay, no model or media calls"]
    fn native_read_only_foundation_reason_projection_receipt() {
        let root = fs::canonicalize(PathBuf::from(
            std::env::var_os("DIRECTOR_FOUNDATION_REASON_PROJECT_ROOT")
                .expect("explicit project root required"),
        ))
        .unwrap();
        let receipt = PathBuf::from(
            std::env::var_os("DIRECTOR_FOUNDATION_REASON_RECEIPT")
                .expect("explicit private receipt path required"),
        );
        let expected: usize = std::env::var("DIRECTOR_FOUNDATION_REASON_EXPECTED_COUNT")
            .expect("explicit positive audit count required")
            .parse()
            .unwrap();
        assert!(expected > 0 && receipt.is_absolute() && !receipt.exists());
        let receipt_parent =
            fs::canonicalize(receipt.parent().unwrap()).expect("pre-create private receipt parent");
        assert!(
            !receipt_parent.starts_with(&root),
            "receipt must be outside original project"
        );
        let receipt = receipt_parent.join(receipt.file_name().unwrap());
        let before = file_evidence(&root);
        assert!(before.contains_key("project.sqlite"));
        // Direct construction: Store::open/init/schema migration are forbidden.
        let outcome = Store { root: root.clone() }.foundation_context_for_new_workflow();
        let material_counts = outcome.as_ref().ok().map(|projection| {
            let snapshot = read_only_history_snapshot(&root).unwrap();
            let connection = Connection::open_with_flags(
                snapshot.path().join("project.sqlite"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let (_, source, _) = source_at(&connection, None).unwrap();
            let mut statement = connection
                .prepare("SELECT data FROM assets ORDER BY rowid")
                .unwrap();
            let assets: Vec<Value> = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
                .collect();
            let mut input =
                json!({"source":source,"assets":assets,"foundation":projection.foundation});
            use crate::pipeline::asset_design_contract::{
                freeze_identity_production_materials, IDENTITY_MATERIALS_FIELD,
                SCOPE_MATERIALS_FIELD,
            };
            freeze_identity_production_materials(&mut input)
                .expect("restored foundation must reach actual Stage2 frozen materials");
            (
                input[SCOPE_MATERIALS_FIELD].as_object().unwrap().len(),
                input[IDENTITY_MATERIALS_FIELD].as_object().unwrap().len(),
            )
        });
        let (scope_material_count, identity_material_count) = material_counts.unwrap_or((0, 0));
        let after = file_evidence(&root);
        let (status, code, count) = match &outcome {
            Ok(projection) => (
                s(&projection.diagnostic, "status"),
                s(&projection.diagnostic, "code"),
                projection.diagnostic["restoredVariantIds"]
                    .as_array()
                    .map_or(0, Vec::len),
            ),
            Err(_) => ("error", "READ_ONLY_PROJECTION_FAILED", 0),
        };
        let unchanged = before == after;
        let compact = json!({"version":"foundation-reason-readonly-receipt/v1","status":status,"code":code,"restoredReasonCount":count,"expectedCount":expected,
            "stage2ScopeMaterialCount":scope_material_count,"stage2IdentityMaterialCount":identity_material_count,
            "newModelCalls":0,"images":0,"videos":0,"historyUnchanged":unchanged,"evidenceFileCount":before.len(),
            "beforeEvidenceSha256":digest(&serde_json::to_string(&before).unwrap()),"afterEvidenceSha256":digest(&serde_json::to_string(&after).unwrap())});
        // No source text, raw values, names, IDs, tokens or paths in the receipt.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(receipt)
            .unwrap();
        file.write_all(serde_json::to_string_pretty(&compact).unwrap().as_bytes())
            .unwrap();
        file.sync_all().unwrap();
        assert!(
            unchanged,
            "read-only projection changed DB/WAL/raw/snapshot bytes"
        );
        assert!(
            outcome.is_ok(),
            "read-only projection failed; inspect compact private receipt"
        );
        assert_eq!(
            status, "projected",
            "evidence mismatch must not silently pass fallback"
        );
        assert_eq!(
            count, expected,
            "restored count differs from explicit audit"
        );
        assert_eq!(
            scope_material_count, expected,
            "restored reasons must reach actual Stage2 frozen scope map"
        );
        assert!(
            identity_material_count > 0,
            "actual Stage2 identity materials must not be empty"
        );
    }
}
