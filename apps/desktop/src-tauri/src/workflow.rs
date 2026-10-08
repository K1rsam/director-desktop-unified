//! A small durable runner boundary shared by foundation and storyboard production.
//! Model rules remain versioned inputs; accepted steps and publication are Rust-owned.
use crate::store::{
    mark_changed, now, parse, s, same_content, save_asset_tx, save_unit_tx, Result, Store,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

fn has_steps(store: &Store) -> Result<bool> {
    store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_steps')",[],|r|r.get(0)).map_err(|e|e.to_string())
}

// Resolution is a read-only projection of durable domain receipts. It never
// turns an invalid author into an accepted checkpoint or rewrites its error.
#[derive(Clone)]
struct ProgressStep {
    node: String,
    hash: String,
    status: String,
    error: Value,
    snapshot: Value,
    request: crate::pipeline::TextRequest,
    domain_hash: String,
    result: Value,
    raw: Value,
    receipt: Value,
    event: Value,
}

struct RefinementAdoption<'a> {
    run: &'a str,
    expected_candidate_sha: &'a str,
}

fn rewire_refinement_groups(tx: &Transaction<'_>, delta: &Value) -> Result<()> {
    for group in delta["replacements"]
        .as_array()
        .ok_or("精修采用缺少替换组")?
    {
        let old: Vec<_> = group["replaces"]
            .as_array()
            .ok_or("精修采用缺少原身份")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let next: Vec<_> = group["units"]
            .as_array()
            .ok_or("精修采用缺少新稿")?
            .iter()
            .map(|unit| s(unit, "id"))
            .collect();
        let first = *old.first().ok_or("精修采用原组为空")?;
        let new_first = *next.first().ok_or("精修采用新组为空")?;
        let new_last = *next.last().unwrap();
        let mut query = tx
            .prepare("SELECT unit_id,dependency_id FROM unit_dependencies")
            .map_err(|e| e.to_string())?;
        let links = query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(query);
        let ingress: Vec<_> = links
            .iter()
            .filter(|(unit, parent)| unit == first && !old.contains(&parent.as_str()))
            .collect();
        let successors: Vec<_> = links
            .iter()
            .filter(|(unit, parent)| {
                !old.contains(&unit.as_str()) && old.contains(&parent.as_str())
            })
            .collect();
        for id in &old {
            tx.execute(
                "DELETE FROM unit_dependencies WHERE unit_id=?1 OR dependency_id=?1",
                [id],
            )
            .map_err(|e| e.to_string())?;
        }
        for id in &next {
            tx.execute("DELETE FROM unit_dependencies WHERE unit_id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
        for (_, parent) in ingress {
            if parent != new_first {
                tx.execute(
                    "INSERT OR IGNORE INTO unit_dependencies VALUES(?1,?2)",
                    params![new_first, parent],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        for pair in next.windows(2) {
            tx.execute(
                "INSERT OR IGNORE INTO unit_dependencies VALUES(?1,?2)",
                params![pair[1], pair[0]],
            )
            .map_err(|e| e.to_string())?;
        }
        for (unit, _) in successors {
            if unit != new_last && !next.contains(&unit.as_str()) {
                tx.execute(
                    "INSERT OR IGNORE INTO unit_dependencies VALUES(?1,?2)",
                    params![unit, new_last],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn progress_step(c: &Connection, run: &str, node: &str) -> Result<Option<ProgressStep>> {
    let row:Option<Value>=c.query_row("SELECT w.request_hash,w.state,w.error,w.result,j.snapshot,j.result,j.receipt,j.state,j.error,w.stage,j.kind,(SELECT e.data FROM events e WHERE e.job_id=j.id AND e.type='text_received_raw' ORDER BY e.rowid DESC LIMIT 1) FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",params![run,node],|r|Ok(json!({"hash":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"error":r.get::<_,Option<String>>(2)?,"result":r.get::<_,Option<String>>(3)?,"snapshot":r.get::<_,String>(4)?,"raw":r.get::<_,Option<String>>(5)?,"receipt":r.get::<_,Option<String>>(6)?,"jobStatus":r.get::<_,String>(7)?,"jobError":r.get::<_,Option<String>>(8)?,"stage":r.get::<_,String>(9)?,"kind":r.get::<_,String>(10)?,"event":r.get::<_,Option<String>>(11)?}))).optional().map_err(|e|e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let Some(snapshot_json) = row["snapshot"].as_str() else {
        return Ok(None);
    };
    let Ok(snapshot) = serde_json::from_str::<Value>(snapshot_json) else {
        return Ok(None);
    };
    let Ok(request) = serde_json::from_value::<crate::pipeline::TextRequest>(snapshot.clone())
    else {
        return Ok(None);
    };
    if row["kind"] != "text"
        || request.stage != s(&row, "stage")
        || crate::pipeline::digest(snapshot_json) != s(&row, "hash")
    {
        return Ok(None);
    }
    let parse_field = |field: &str| {
        row[field]
            .as_str()
            .and_then(|v| serde_json::from_str::<Value>(v).ok())
            .unwrap_or(Value::Null)
    };
    let receipt = parse_field("receipt");
    let accepted = row["status"] == "accepted"
        && row["jobStatus"] == "completed"
        && row["error"].is_null()
        && row["jobError"].is_null()
        && receipt["validation"] == "accepted";
    let rejected = row["status"] == "rejected"
        && row["jobStatus"] == "rejected"
        && row["error"] == row["jobError"]
        && receipt["validation"] == "rejected";
    if (!accepted && !rejected)
        || receipt["contract"] != "desktop-runner/v1"
        || receipt
            .get("acceptedRequestHash")
            .is_some_and(|h| *h != row["hash"])
    {
        return Ok(None);
    }
    let Ok(domain) = serde_json::to_string(&request) else {
        return Ok(None);
    };
    Ok(Some(ProgressStep {
        node: node.into(),
        hash: s(&row, "hash").into(),
        status: s(&row, "status").into(),
        error: row["error"].clone(),
        snapshot,
        request,
        domain_hash: crate::pipeline::digest(&domain),
        result: parse_field("result"),
        raw: parse_field("raw"),
        receipt,
        event: parse_field("event"),
    }))
}

fn progress_episode(step: &ProgressStep) -> &str {
    step.request.input["episodeId"]
        .as_str()
        .or_else(|| {
            step.request
                .input
                .pointer("/episode/id")
                .and_then(Value::as_str)
        })
        .unwrap_or("")
}

fn progress_source_bound(step: &ProgressStep, source: &Value) -> bool {
    let episode = progress_episode(step);
    if episode.is_empty()
        || !source["episodes"]
            .as_array()
            .is_some_and(|items| items.iter().any(|e| e["id"] == episode))
    {
        return false;
    }
    let Some(rows) = step.request.input["sourceRows"]
        .as_array()
        .filter(|rows| !rows.is_empty())
    else {
        return false;
    };
    let Some(source_rows) = source["rows"].as_array() else {
        return false;
    };
    rows.iter().all(|row| {
        source_rows.iter().any(|frozen| {
            frozen["id"] == row["id"]
                && frozen["episodeId"] == episode
                && ["text", "range", "kind", "episodeId", "sceneId"]
                    .iter()
                    .all(|key| frozen[*key] == row[*key])
        })
    })
}

fn progress_resolution(
    run: &str,
    kind: &str,
    origin: &ProgressStep,
    resolver: &ProgressStep,
) -> Value {
    json!({"version":"workflow-step-resolution/v1","kind":kind,"workflowId":run,"originNodeId":origin.node,"originRequestHash":origin.hash,"resolverNodeId":resolver.node,"resolverRequestHash":resolver.hash,"verified":true})
}

fn author_scope_resolution(origin: &ProgressStep, resolver: &ProgressStep, source: &Value) -> bool {
    let request = &resolver.request.input;
    let proof = &resolver.result["materializedDraft"]["authorRepairProvenance"];
    let descriptor = &request["authorRepair"];
    let Some(last) = proof["nodes"].as_array().and_then(|nodes| nodes.last()) else {
        return false;
    };
    let draft = &resolver.result["materializedDraft"];
    let draft_hash=crate::pipeline::digest(&json!({"schemaVersion":draft["schemaVersion"],"units":draft["units"],"fantasyRealization":draft["fantasyRealization"]}).to_string());
    origin.status == "rejected"
        && crate::text_executor::author_repairable_rejection(origin.error.as_str().unwrap_or(""))
        && origin.request.stage == "storyboardCreate"
        && origin.request.skill == "unit-performance-director"
        && origin.request.input["storyboardCreativeProfile"] == "original-production-contract/v1"
        && origin.receipt["migrationPolicy"].is_null()
        && origin.result == origin.raw
        && origin.event["requestHash"] == origin.hash
        && origin.event["value"] == origin.raw
        && resolver.status == "accepted"
        && resolver.request.stage == "storyboardFinish"
        && resolver.request.skill == "episode-finishing-editor"
        && request["storyboardCreativeProfile"] == "original-production-contract/v1"
        && resolver.event["requestHash"] == resolver.hash
        && resolver.event["value"] == resolver.raw
        && resolver.raw["schemaVersion"] == "storyboard-delta/v2"
        && resolver.raw["baseDraftHash"] == descriptor["baseDraftHash"]
        && resolver.result["baseDraftHash"] == resolver.raw["baseDraftHash"]
        && request["executionBatch"]["version"] == "author-domain-repair/v1"
        && descriptor["version"] == "rejected-author-scope-repair/v1"
        && request["finishMode"] == "delta"
        && descriptor["maximumAttempts"] == 2
        && matches!(descriptor["attempt"].as_u64(), Some(1 | 2))
        && proof["version"] == "accepted-author-scope-repair/v1"
        && proof["originalAuthorAccepted"] == false
        && proof["nodes"].as_array().is_some_and(|nodes| {
            !nodes.is_empty()
                && nodes.len() <= 2
                && nodes
                    .iter()
                    .enumerate()
                    .all(|(index, node)| node["attempt"] == json!(index + 1))
        })
        && descriptor["sourceSha256"] == source["sha256"]
        && proof["sourceSha256"] == source["sha256"]
        && progress_episode(origin) == progress_episode(resolver)
        && progress_source_bound(origin, source)
        && progress_source_bound(resolver, source)
        && origin.request.input["sourceRows"] == request["sourceRows"]
        && descriptor["originExecutionBatch"] == origin.request.input["executionBatch"]
        && proof["originDomainRequestSha256"] == origin.domain_hash
        && descriptor["originDomainRequestSha256"] == origin.domain_hash
        && proof["originalAuthorRawSha256"] == crate::pipeline::digest(&origin.raw.to_string())
        && descriptor["originalAuthorRawSha256"] == proof["originalAuthorRawSha256"]
        && last["canonicalDomainRequestSha256"] == resolver.domain_hash
        && last["attempt"] == descriptor["attempt"]
        && last["baseDraftHash"] == descriptor["baseDraftHash"]
        && descriptor["baseDraftHash"] == request["baseDraftHash"]
        && last["draftSha256"] == draft_hash
        && draft["units"].as_array().is_some_and(|units| {
            !units.is_empty()
                && units
                    .iter()
                    .all(|unit| unit["episodeId"] == progress_episode(origin))
        })
        && draft["schemaVersion"] == "storyboard-draft/v2"
        && origin.snapshot["executorIdentity"] == resolver.snapshot["executorIdentity"]
        && origin.snapshot["executorPolicy"] == resolver.snapshot["executorPolicy"]
        && origin.request.model == resolver.request.model
        && origin.request.reasoning == resolver.request.reasoning
        && origin.request.input["capabilities"] == request["capabilities"]
}

fn author_scope_attempt_resolution(
    origin: &ProgressStep,
    resolver: &ProgressStep,
    source: &Value,
    authors: &[ProgressStep],
) -> bool {
    if origin.status != "rejected"
        || !crate::text_executor::author_repairable_rejection(origin.error.as_str().unwrap_or(""))
        || origin.result != origin.raw
        || !origin.receipt["migrationPolicy"].is_null()
        || origin.event["requestHash"] != origin.hash
        || origin.event["value"] != origin.raw
        || origin.request.stage != "storyboardFinish"
        || origin.request.skill != "episode-finishing-editor"
        || origin.request.input["storyboardCreativeProfile"] != "original-production-contract/v1"
        || origin.request.input["executionBatch"]["version"] != "author-domain-repair/v1"
        || origin.request.input["authorRepair"]["version"] != "rejected-author-scope-repair/v1"
        || origin.request.input["authorRepair"]["attempt"] != 1
        || origin.request.input["authorRepair"]["maximumAttempts"] != 2
        || origin.request.input["finishMode"] != "delta"
        || resolver.request.input["authorRepair"]["attempt"] != 2
        || origin.request.model != resolver.request.model
        || origin.request.reasoning != resolver.request.reasoning
        || origin.request.schema != resolver.request.schema
        || origin.request.rule_snapshot != resolver.request.rule_snapshot
        || origin.request.input["capabilities"] != resolver.request.input["capabilities"]
        || origin.snapshot["executorIdentity"] != resolver.snapshot["executorIdentity"]
        || origin.snapshot["executorPolicy"] != resolver.snapshot["executorPolicy"]
        || progress_episode(origin) != progress_episode(resolver)
        || !progress_source_bound(origin, source)
        || origin.request.input["sourceRows"] != resolver.request.input["sourceRows"]
    {
        return false;
    }
    let proof = &resolver.result["materializedDraft"]["authorRepairProvenance"];
    let Some(nodes) = proof["nodes"].as_array().filter(|nodes| nodes.len() == 2) else {
        return false;
    };
    let earlier = &nodes[0];
    let last = &nodes[1];
    earlier["result"] == "rejected-scope"
        && earlier["canonicalDomainRequestSha256"] == origin.domain_hash
        && earlier["baseDraftHash"] == origin.request.input["baseDraftHash"]
        && earlier["draftSha256"] == last["baseDraftHash"]
        && origin.request.input["authorRepair"]["baseDraftHash"]
            == origin.request.input["baseDraftHash"]
        && authors.iter().any(|author| {
            author_scope_resolution(author, resolver, source)
                && origin.request.input["authorRepair"]["originDomainRequestSha256"]
                    == author.domain_hash
                && origin.request.input["authorRepair"]["originalAuthorRawSha256"]
                    == proof["originalAuthorRawSha256"]
                && origin.request.input["authorRepair"]["sourceSha256"] == source["sha256"]
                && origin.request.input["authorRepair"]["originExecutionBatch"]
                    == author.request.input["executionBatch"]
        })
}

/// Reuse the same durable repair proof for author-cache recovery as for the
/// progress projection. The rejected author stays rejected; only the original
/// editor's source-bound, adopted draft can supply a recovered predecessor.
pub(crate) fn accepted_author_repair_predecessor(
    store: &Store,
    run: &str,
    node: &str,
    tail: &Value,
) -> Result<bool> {
    let c = store.conn()?;
    let Some(resolver) = progress_step(&c, run, node)? else {
        return Ok(false);
    };
    let input = &resolver.request.input;
    let draft = &resolver.result["materializedDraft"];
    if input["executionBatch"]["version"] != "author-domain-repair/v1"
        || input["executionBatch"]["originExecutionBatchId"] != tail["executionBatchId"]
        || draft["units"].as_array().and_then(|units| units.last()) != Some(&tail["unit"])
    {
        return Ok(false);
    }
    let source: Option<String> = c.query_row(
        "SELECT json_extract(snapshot,'$.input.source') FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",
        [run], |r| r.get(0),
    ).optional().map_err(|e| e.to_string())?.flatten();
    let Some(source) = source.and_then(|v| serde_json::from_str::<Value>(&v).ok()) else {
        return Ok(false);
    };
    if source["sha256"].as_str().is_none_or(|sha| sha.len() != 64)
        || source["text"]
            .as_str()
            .is_none_or(|text| crate::pipeline::digest(text) != s(&source, "sha256"))
    {
        return Ok(false);
    }
    let mut query = c.prepare(
        "SELECT w.node_key FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.state='rejected' AND json_extract(j.snapshot,'$.input.episodeId')=?2 AND ((w.stage='storyboardCreate' AND json_extract(j.snapshot,'$.input.executionBatch.id')=?3) OR (w.stage='storyboardFinish' AND json_extract(j.snapshot,'$.input.authorRepair.originDomainRequestSha256')=?4))",
    ).map_err(|e| e.to_string())?;
    let keys = query
        .query_map(
            params![
                run,
                progress_episode(&resolver),
                s(tail, "executionBatchId"),
                s(&input["authorRepair"], "originDomainRequestSha256")
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut origins = Vec::new();
    for key in keys {
        if let Some(origin) = progress_step(&c, run, &key)? {
            origins.push(origin);
        }
    }
    Ok(origins.iter().any(|origin| {
        author_scope_resolution(origin, &resolver, &source)
            && (input["authorRepair"]["attempt"] == 1
                || origins.iter().any(|earlier| {
                    author_scope_attempt_resolution(earlier, &resolver, &source, &origins)
                }))
    }))
}

fn continuity_resolution(
    origin: &ProgressStep,
    resolver: &ProgressStep,
    source: &Value,
    events: &[Value],
) -> bool {
    let revision = &resolver.request.input["continuityRevision"];
    if resolver.status != "accepted"
        || revision["version"] != "accepted-tail-change/v1"
        || revision["sourceSha256"] != source["sha256"]
        || origin.request.stage != resolver.request.stage
        || origin.request.skill != resolver.request.skill
        || origin.request.model != resolver.request.model
        || origin.request.reasoning != resolver.request.reasoning
        || origin.request.schema != resolver.request.schema
        || origin.request.rule_snapshot != resolver.request.rule_snapshot
        || origin.snapshot["executorIdentity"] != resolver.snapshot["executorIdentity"]
        || origin.snapshot["executorPolicy"] != resolver.snapshot["executorPolicy"]
        || progress_episode(origin) != progress_episode(resolver)
        || !progress_source_bound(origin, source)
        || !progress_source_bound(resolver, source)
        || origin.request.input["sourceRows"] != resolver.request.input["sourceRows"]
    {
        return false;
    }
    let suffix = format!(
        ":continuity:{}",
        crate::pipeline::digest(&revision.to_string())
    );
    let Some(new_base) = resolver.node.strip_suffix(&suffix) else {
        return false;
    };
    let prior = &origin.request.input["continuityRevision"];
    let prior_suffix = format!(
        ":continuity:{}",
        crate::pipeline::digest(&prior.to_string())
    );
    let old_base = if prior["version"] == "accepted-tail-change/v1" {
        origin.node.strip_suffix(&prior_suffix).unwrap_or("")
    } else {
        &origin.node
    };
    if old_base.is_empty() || old_base != new_base {
        return false;
    }
    let Some(episode_index) = source["episodes"].as_array().and_then(|episodes| {
        episodes
            .iter()
            .position(|e| e["id"] == progress_episode(resolver))
    }) else {
        return false;
    };
    if episode_index == 0
        || source["episodes"][episode_index - 1]["id"] != revision["previousEpisodeId"]
    {
        return false;
    }
    if !events.iter().any(|event| {
        if event["episodeId"] != progress_episode(resolver)
            || event["action"] != "recompute_affected_chain"
        {
            return false;
        }
        let mut frozen = event.clone();
        if let Some(map) = frozen.as_object_mut() {
            for field in ["episodeId", "action", "previouslyAcceptedPlan"] {
                map.remove(field);
            }
        }
        frozen == *revision
    }) {
        return false;
    }
    // A Planner has explicit old/new previous tails. For later roles an
    // embedded revision alone cannot prove which actual chain supplied them.
    // Keep the original notice until a stronger adopted-chain receipt exists.
    origin.request.stage == "storyboardPlan"
        && !origin.request.input["previousBoundary"].is_null()
        && !resolver.request.input["previousBoundary"].is_null()
        && crate::pipeline::digest(&origin.request.input["previousBoundary"].to_string())
            == revision["priorTailSha256"]
        && crate::pipeline::digest(&resolver.request.input["previousBoundary"].to_string())
            == revision["currentTailSha256"]
}

fn project_transport_resolutions(
    store: &Store,
    c: &Connection,
    run: &str,
    nodes: &mut [Value],
) -> Result<()> {
    if !nodes.iter().any(|node| node["status"] == "failed") {
        return Ok(());
    }
    let mut query=c.prepare("SELECT w.node_key FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.state='accepted' AND j.kind='text' AND j.state='completed' AND json_extract(j.snapshot,'$.input.textTransportRetry.version')='stopped-text-transport-retry/v1'").map_err(|e|e.to_string())?;
    let keys = query
        .query_map([run], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for key in keys {
        let Some(resolver) = progress_step(c, run, &key)? else {
            continue;
        };
        if !crate::text_executor::accepted_transport_retry_proof(store, run, &resolver.request)? {
            continue;
        }
        let proof = &resolver.request.input["textTransportRetry"];
        if let Some(origin) = nodes.iter_mut().find(|node| {
            node["status"] == "failed"
                && node["id"] == proof["originNodeKey"]
                && node["requestHash"] == proof["originRequestHash"]
        }) {
            origin["resolution"] = json!({"version":"workflow-step-resolution/v1","kind":"text-transport-recovered","workflowId":run,"originNodeId":origin["id"],"originRequestHash":origin["requestHash"],"resolverNodeId":resolver.node,"resolverRequestHash":resolver.hash,"verified":true});
        }
        for prior in proof["priorFailures"].as_array().into_iter().flatten() {
            if let Some(origin) = nodes.iter_mut().find(|node| {
                node["status"] == "failed"
                    && node["id"] == prior["nodeKey"]
                    && node["requestHash"] == prior["requestHash"]
            }) {
                origin["resolution"] = json!({"version":"workflow-step-resolution/v1","kind":"text-transport-recovered","workflowId":run,"originNodeId":origin["id"],"originRequestHash":origin["requestHash"],"resolverNodeId":resolver.node,"resolverRequestHash":resolver.hash,"verified":true});
            }
        }
    }
    Ok(())
}
fn project_step_resolutions(
    store: &Store,
    c: &Connection,
    run: &str,
    nodes: &mut [Value],
) -> Result<()> {
    project_transport_resolutions(store, c, run, nodes)?;
    if !nodes.iter().any(|node| {
        node["stage"] == "storyboardCreate"
            && node["status"] == "rejected"
            && crate::text_executor::author_repairable_rejection(s(node, "error"))
            || node["stage"] == "storyboardPlan" && node["status"] != "running"
    }) {
        return Ok(());
    }
    let mut query=c.prepare("SELECT w.node_key FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.state='accepted' AND j.kind='text' AND j.state='completed' AND (json_extract(j.snapshot,'$.input.executionBatch.version')='author-domain-repair/v1' OR json_extract(j.snapshot,'$.input.continuityRevision.version')='accepted-tail-change/v1')").map_err(|e|e.to_string())?;
    let keys = query
        .query_map([run], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut resolvers = Vec::new();
    for key in keys {
        if nodes
            .iter()
            .any(|n| n["id"] == key && n["status"] == "accepted")
        {
            if let Some(step) = progress_step(c, run, &key)? {
                resolvers.push(step);
            }
        }
    }
    if resolvers.is_empty() {
        return Ok(());
    }
    // Ordinary progress never loads full source or draft histories. Only the
    // few actual repair/revision receipts need the immutable evidence below.
    let source:Option<String>=c.query_row("SELECT json_extract(snapshot,'$.input.source') FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|r.get::<_,Option<String>>(0)).optional().map_err(|e|e.to_string())?.flatten();
    let Some(source) = source.and_then(|v| serde_json::from_str::<Value>(&v).ok()) else {
        return Ok(());
    };
    if source["sha256"].as_str().is_none_or(|sha| sha.len() != 64)
        || source["text"]
            .as_str()
            .is_none_or(|text| crate::pipeline::digest(text) != s(&source, "sha256"))
    {
        return Ok(());
    }
    let mut events=c.prepare("SELECT data FROM events WHERE job_id=?1 AND type='storyboard_continuity_updated' ORDER BY rowid").map_err(|e|e.to_string())?;
    let events = events
        .query_map([run], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(|row| {
            row.ok()
                .and_then(|v| serde_json::from_str::<Value>(&v).ok())
        })
        .collect::<Vec<_>>();
    let mut origins = Vec::new();
    for node in nodes.iter().filter(|node| {
        node["stage"] == "storyboardPlan"
            || node["status"] == "rejected"
                && crate::text_executor::author_repairable_rejection(s(node, "error"))
    }) {
        if let Some(step) = progress_step(c, run, s(node, "id"))? {
            origins.push(step);
        }
    }
    for node in nodes {
        let Some(origin) = origins.iter().find(|origin| origin.node == s(node, "id")) else {
            continue;
        };
        let matches: Vec<_> = resolvers
            .iter()
            .filter_map(|resolver| {
                if resolver.node == origin.node {
                    return None;
                }
                let author_resolved = author_scope_resolution(origin, resolver, &source)
                    && (resolver.request.input["authorRepair"]["attempt"] == 1
                        || origins.iter().any(|earlier| {
                            author_scope_attempt_resolution(earlier, resolver, &source, &origins)
                        }));
                let kind = if author_resolved
                    || author_scope_attempt_resolution(origin, resolver, &source, &origins)
                {
                    "author-scope-repair"
                } else if continuity_resolution(origin, resolver, &source, &events) {
                    "continuity-superseded"
                } else {
                    return None;
                };
                Some(progress_resolution(run, kind, origin, resolver))
            })
            .collect();
        if matches.len() == 1 {
            node["resolution"] = matches[0].clone();
        }
    }
    Ok(())
}
impl Store {
    /// Additive tables are created only when starting an explicitly requested local run.
    /// Merely opening an old project remains read-only; project v2 media/task contracts stay intact.
    pub fn ensure_workflow_storage(&self) -> Result<()> {
        self.conn()?.execute_batch("CREATE TABLE IF NOT EXISTS workflow_steps(workflow_id TEXT NOT NULL REFERENCES jobs(id),node_key TEXT NOT NULL,stage TEXT NOT NULL,request_hash TEXT NOT NULL,state TEXT NOT NULL,text_job_id TEXT REFERENCES jobs(id),result TEXT,error TEXT,updated_at TEXT NOT NULL,PRIMARY KEY(workflow_id,node_key));CREATE INDEX IF NOT EXISTS workflow_step_hash_idx ON workflow_steps(workflow_id,request_hash,state);CREATE TABLE IF NOT EXISTS unit_dependencies(unit_id TEXT NOT NULL REFERENCES units(id),dependency_id TEXT NOT NULL REFERENCES units(id),PRIMARY KEY(unit_id,dependency_id));CREATE INDEX IF NOT EXISTS unit_dependency_reverse_idx ON unit_dependencies(dependency_id);") .map_err(|e|e.to_string())
    }
    pub fn checkpoint(
        &self,
        run: &str,
        key: &str,
        stage: &str,
        hash: &str,
        state: &str,
        text_job: Option<&str>,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn()?.execute("INSERT INTO workflow_steps VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(workflow_id,node_key) DO UPDATE SET stage=excluded.stage,request_hash=excluded.request_hash,state=excluded.state,text_job_id=excluded.text_job_id,result=excluded.result,error=excluded.error,updated_at=excluded.updated_at",params![run,key,stage,hash,state,text_job,result.map(Value::to_string),error,now()]).map_err(|e|e.to_string())?;
        self.dirty()
    }
    pub fn accepted_step(&self, run: &str, hash: &str) -> Result<Option<Value>> {
        let raw:Option<String>=self.conn()?.query_row("SELECT result FROM workflow_steps WHERE workflow_id=?1 AND request_hash=?2 AND state='accepted' ORDER BY updated_at DESC LIMIT 1",params![run,hash],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        raw.map(parse).transpose()
    }
    pub fn workflow_progress(&self) -> Result<Vec<Value>> {
        if !has_steps(self)? {
            return Ok(vec![]);
        }
        let c = self.conn()?;
        let mut q=c.prepare("SELECT id,entity_id,state,result FROM jobs WHERE kind='workflow' AND json_extract(snapshot,'$.runnerVersion')='desktop-runner/v1' ORDER BY created_at DESC,rowid DESC LIMIT 12").map_err(|e|e.to_string())?;
        let runs = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut values = vec![];
        for (id, stage, status, result) in runs {
            // Read only small progress fields; full input/source/results remain lazy.
            // An older node key can remain after a runner key migration. Its exact
            // cached request is still one step; keep every checkpoint on disk but
            // project only the latest state for each stage/request identity.
            let mut steps=c.prepare("SELECT w.node_key,w.stage,w.state,w.error,json_extract(j.snapshot,'$.input.delivery'),json_extract(j.snapshot,'$.input.scope'),json_extract(j.snapshot,'$.input.episodeId'),json_extract(j.snapshot,'$.input.episode.id'),CASE WHEN e.rowid IS NOT NULL THEN json_object('phase',json_extract(e.data,'$.phase'),'elapsedSeconds',json_extract(e.data,'$.elapsedSeconds'),'outputQuietSeconds',json_extract(e.data,'$.outputQuietSeconds'),'processAlive',json_extract(e.data,'$.processAlive'),'needsDiagnosis',json_extract(e.data,'$.needsDiagnosis'),'observedAt',e.created_at,'stderrCategory',json_extract(e.data,'$.stderrCategory')) END,w.request_hash,json_extract(j.snapshot,'$.input.executionBatch.version') FROM workflow_steps w LEFT JOIN jobs j ON j.id=w.text_job_id LEFT JOIN events e ON w.state='running' AND j.state='submitting' AND e.rowid=(SELECT x.rowid FROM events x WHERE x.job_id=w.text_job_id AND x.type='codex_heartbeat' ORDER BY x.rowid DESC LIMIT 1) WHERE w.workflow_id=?1 AND NOT EXISTS(SELECT 1 FROM workflow_steps newer WHERE newer.workflow_id=w.workflow_id AND newer.stage=w.stage AND newer.request_hash=w.request_hash AND (newer.updated_at>w.updated_at OR (newer.updated_at=w.updated_at AND newer.rowid>w.rowid))) ORDER BY w.rowid").map_err(|e|e.to_string())?;
            let mut nodes=steps.query_map([&id],|r| {
                let stage=r.get::<_,String>(1)?;
                let delivery=r.get::<_,Option<String>>(4)?.and_then(|v|serde_json::from_str::<Value>(&v).ok());
                let scope=r.get::<_,Option<String>>(5)?.and_then(|v|serde_json::from_str::<Value>(&v).ok());
                let episode=r.get::<_,Option<String>>(6)?.or(r.get::<_,Option<String>>(7)?);
                let execution=r.get::<_,Option<String>>(8)?.and_then(|v|serde_json::from_str::<Value>(&v).ok());
                Ok(json!({"id":r.get::<_,String>(0)?,"requestHash":r.get::<_,String>(9)?,"title":step_title(&stage),"stage":stage,"status":r.get::<_,String>(2)?,"error":r.get::<_,Option<String>>(3)?,"execution":execution,"input":{"delivery":delivery,"scope":scope,"episodeId":episode,"executionBatchVersion":r.get::<_,Option<String>>(10)?}}))
            }).map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            if stage == "storyboard.run" {
                project_step_resolutions(self, &c, &id, &mut nodes)?;
            }
            let result = result
                .and_then(|v| serde_json::from_str::<Value>(&v).ok())
                .unwrap_or(Value::Null);
            values.push(json!({"workflowId":id,"stage":stage,"status":status,"completedSteps":nodes.iter().filter(|n|n["status"]=="accepted").count(),"totalSteps":nodes.len(),"nodes":nodes,"waitingDecision":result["waitingDecision"],"blocked":result["blocked"],"publishedEpisodeIds":result["updatedEpisodeIds"]}));
        }
        Ok(values)
    }
    pub fn unit_versions(&self, id: &str) -> Result<Value> {
        self.get("units", id)?;
        let c = self.conn()?;
        let mut q = c
            .prepare(
                "SELECT version,data FROM unit_versions WHERE unit_id=?1 ORDER BY version DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        let mut values = vec![];
        for row in rows {
            let (v, data) = row.map_err(|e| e.to_string())?;
            values.push(json!({"version":v,"data":parse(data)?}));
        }
        Ok(json!({"versions":values}))
    }
    pub fn restore_unit_version(&self, id: &str, version: i64, expected: i64) -> Result<Value> {
        let raw: String = self
            .conn()?
            .query_row(
                "SELECT data FROM unit_versions WHERE unit_id=?1 AND version=?2",
                params![id, version],
                |r| r.get(0),
            )
            .map_err(|_| "历史单元版本不存在")?;
        let mut data = parse(raw)?;
        data["expectedVersion"] = json!(expected);
        data["retired"] = json!(false);
        let result = self.save_unit(data)?;
        self.invalidate_unit_dependents(id)?;
        Ok(result)
    }
    pub fn save_production_direction(
        &self,
        direction: &str,
        source_version: i64,
        foundation_version: Option<i64>,
    ) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let current_source: i64 = tx
            .query_row("SELECT COALESCE(MAX(version),0) FROM sources", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if current_source != source_version || current_source == 0 {
            return Err("原文已经更新，请刷新后再保存制作方向".into());
        }
        let (old_version, raw) = latest_foundation(&tx)?;
        if foundation_version.is_some_and(|v| v != old_version) {
            return Err("制作基础已经更新，请刷新后再保存方向".into());
        }
        let mut foundation=raw.unwrap_or(json!({"schemaVersion":"foundation/v1","episodes":[],"assets":[],"facts":[],"issues":[],"overview":"","direction":""}));
        foundation["productionDirection"] = json!(direction.trim());
        foundation["directionConfirmed"] = json!(true);
        write_foundation(&tx, foundation, current_source)?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn publish_foundation(
        &self,
        guard: &Value,
        foundation: Value,
        assets: Vec<Value>,
        run: Option<&str>,
    ) -> Result<Value> {
        self.publish_foundation_with_source(guard, foundation, assets, run, None)
    }
    /// Q1 changes scene projections and foundation together; original source bytes
    /// and all submitted cloud tasks remain immutable.
    pub fn publish_foundation_with_source(
        &self,
        guard: &Value,
        mut foundation: Value,
        assets: Vec<Value>,
        run: Option<&str>,
        revised_source: Option<Value>,
    ) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        verify_guard(&tx, guard)?;
        let mut source_version = guard["sourceVersion"].as_i64().ok_or("冻结源版本缺失")?;
        if let Some(mut revised) = revised_source {
            let locked: crate::pipeline::LockedSource =
                serde_json::from_value(revised.clone()).map_err(|e| e.to_string())?;
            crate::pipeline::validate_source(&locked)?;
            let original: String = tx
                .query_row(
                    "SELECT data FROM sources WHERE version=?1",
                    [source_version],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let original = parse(original)?;
            if revised["text"] != original["text"] || revised["sha256"] != original["sha256"] {
                return Err("场次修订不能改写原文".into());
            }
            revised["version"] = json!(source_version);
            if revised != original {
                source_version += 1;
                revised["version"] = json!(source_version);
                tx.execute(
                    "INSERT INTO sources VALUES(?1,?2,?3,?4,?5)",
                    params![
                        source_version,
                        s(&revised, "text"),
                        s(&revised, "sha256"),
                        revised.to_string(),
                        now()
                    ],
                )
                .map_err(|e| e.to_string())?;
                tx.execute("UPDATE jobs SET state='stale',error='集场结构已更新',updated_at=?1,next_attempt_at=0 WHERE task_id IS NULL AND state IN ('ready','queued','preparing','waiting_dependency') AND kind IN ('video','image','text')",[now()]).map_err(|e|e.to_string())?;
            }
        }
        if let (_, Some(current)) = latest_foundation(&tx)? {
            for key in ["productionDirection", "directionConfirmed"] {
                if !current[key].is_null() {
                    foundation[key] = current[key].clone();
                }
            }
            if current["sourceSha256"] == foundation["sourceSha256"]
                && !current["visualFactBatches"].is_null()
            {
                foundation["visualFactBatches"] = current["visualFactBatches"].clone();
            }
        }
        let foundation = write_foundation(&tx, foundation, source_version)?;
        for proposal in assets {
            let raw: Option<String> = tx
                .query_row(
                    "SELECT data FROM assets WHERE id=?1",
                    [s(&proposal, "id")],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let mut value = raw.map(parse).transpose()?.unwrap_or(json!({}));
            if crate::store::asset_deleted(&value) {
                // An earlier foundation remains historical evidence, not permission to
                // restore an asset the user removed from current production.
                continue;
            }
            for (k, v) in proposal.as_object().ok_or("资产提案必须为对象")? {
                if [
                    "prompt",
                    "design",
                    "designStatus",
                    "designDirection",
                    "designVersion",
                    "designSource",
                    "viewPrompt",
                ]
                .contains(&k.as_str())
                    && !value[k].is_null()
                {
                    continue;
                }
                value[k] = v.clone();
            }
            save_asset_tx(&tx, value)?;
        }
        if let Some(run) = run {
            tx.execute("UPDATE jobs SET state='completed',result=?1,error=NULL,updated_at=?2 WHERE id=?3 AND kind='workflow'",params![foundation.to_string(),now(),run]).map_err(|e|e.to_string())?;
        }
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(foundation)
    }
    pub fn publish_asset_design(
        &self,
        source_version: i64,
        foundation_version: i64,
        output: &Value,
    ) -> Result<()> {
        self.publish_asset_design_inner(source_version, foundation_version, output, None)
    }
    /// Stage2 text owns one parent. Hold partial domain output/checkpoints without
    /// advancing asset/foundation versions; complete publication and terminal parent
    /// state commit together so a crash cannot leave an adopted-but-runnable parent.
    pub fn finish_asset_design_workflow(
        &self,
        source_version: i64,
        foundation_version: i64,
        output: &Value,
        run: &str,
    ) -> Result<Value> {
        let mut result = output.clone();
        let blocked = result["blocked"].as_array().ok_or("设计结果缺少阻塞列表")?;
        if !blocked.is_empty() {
            result["publication"] = json!({"version":"stage2-atomic-publication/v1","status":"held_partial","workflowId":run});
            if !self.claim_job_fields(
                run,
                "running",
                "partially_blocked",
                &json!({"result":result}),
            )? {
                return Err("原资产设计父流程不再运行，未改写其结果".into());
            }
        } else {
            result["publication"] = json!({"version":"stage2-atomic-publication/v1","status":"completed","workflowId":run});
            self.publish_asset_design_inner(
                source_version,
                foundation_version,
                &result,
                Some(run),
            )?;
        }
        Ok(result)
    }
    fn publish_asset_design_inner(
        &self,
        source_version: i64,
        foundation_version: i64,
        output: &Value,
        run: Option<&str>,
    ) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (current_version, source_raw): (i64, String) = tx
            .query_row(
                "SELECT version,data FROM sources ORDER BY version DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let (current_foundation, previous) = latest_foundation(&tx)?;
        if current_version != source_version || current_foundation != foundation_version {
            return Err("原文或制作基础已更新，设计结果保留，未覆盖当前版本".into());
        }
        let source: crate::pipeline::LockedSource =
            serde_json::from_str(&source_raw).map_err(|e| e.to_string())?;
        // Validate against the complete prospective batch before any write. A
        // direct source edited during generation must not be silently adopted.
        let proposals = output["assets"].as_array().ok_or("设计结果缺少资产列表")?;
        let mut statement = tx
            .prepare("SELECT data FROM assets")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut effective = rows
            .map(|r| parse(r.map_err(|e| e.to_string())?))
            .collect::<Result<Vec<_>>>()?;
        drop(statement);
        for proposal in proposals {
            let current = effective
                .iter_mut()
                .find(|a| a["id"] == proposal["id"])
                .ok_or("设计资产已经不存在")?;
            if current["version"] != proposal["version"] {
                return Err("资产在设计期间已经编辑，结果保留供核对".into());
            }
            *current = proposal.clone();
        }
        for proposal in proposals {
            let receipt = &proposal["design"]["designReceipt"];
            if receipt.is_object() {
                let basis = &receipt["basis"];
                if receipt["sha256"] != crate::pipeline::digest(&basis.to_string())
                    || basis["assetSemanticVersion"]
                        != crate::pipeline::digest(
                            &crate::pipeline::asset_design_contract::asset_semantics(proposal)
                                .to_string(),
                        )
                    || basis["sourceChain"]
                        != json!(crate::pipeline::asset_design_contract::source_chain(
                            &effective, proposal
                        )?)
                {
                    return Err("资产设计或指定来源语义版本已变化，未采用过期设计".into());
                }
            }
        }
        let mut seen = HashSet::new();
        for design in proposals {
            let aid = s(design, "id");
            if aid.is_empty() || !seen.insert(aid) {
                return Err("设计结果资产身份为空或重复".into());
            }
            let raw: String = tx
                .query_row("SELECT data FROM assets WHERE id=?1", [aid], |r| r.get(0))
                .map_err(|_| "设计资产已经不存在")?;
            let mut current = parse(raw)?;
            if current["version"] != design["version"] {
                return Err(format!("资产 {} 在设计期间已经编辑，结果保留供核对", aid));
            }
            let expected = current["version"].clone();
            for (key, value) in design.as_object().ok_or("设计资产须为对象")? {
                current[key] = value.clone();
            }
            current["expectedVersion"] = expected;
            save_asset_tx(&tx, current)?;
        }
        let mut foundation=previous.unwrap_or(json!({"schemaVersion":"foundation/v1","sourceSha256":source.sha256,"episodes":[],"assets":[],"facts":[],"issues":[],"overview":"","direction":""}));
        if let Some(facts) = output["facts"].as_array() {
            let mut safe = vec![];
            for fact in facts {
                let source_binding = fact.get("sourceSha256").cloned();
                if source_binding
                    .as_ref()
                    .is_some_and(|sha| sha != &json!(source.sha256))
                {
                    return Err("视觉事实源摘要与当前锁定原文不一致".into());
                }
                let mut fact: crate::pipeline::FoundationFact =
                    serde_json::from_value(fact.clone()).map_err(|e| e.to_string())?;
                if fact.source_refs.is_empty() {
                    return Err("视觉事实没有原文依据".into());
                }
                fact.quotes = fact
                    .source_refs
                    .iter()
                    .map(|id| {
                        source
                            .rows
                            .iter()
                            .find(|r| &r.id == id)
                            .map(|r| r.text.clone())
                            .ok_or("视觉事实源引用已失效")
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?
                    .into_iter()
                    .collect();
                let mut saved = json!(fact);
                if let Some(sha) = source_binding {
                    saved["sourceSha256"] = sha;
                }
                safe.push(saved);
            }
            foundation["facts"] = json!(safe);
        }
        if let Some(receipts) = output["visualFactBatches"].as_array() {
            let mut all = foundation["visualFactBatches"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for receipt in receipts {
                if receipt["sourceSha256"] != source.sha256 || receipt["status"] != "accepted" {
                    return Err("视觉事实缓存回执身份无效".into());
                }
                all.retain(|old| old["inputFingerprint"] != receipt["inputFingerprint"]);
                all.push(receipt.clone());
            }
            foundation["visualFactBatches"] = json!(all);
        }
        write_foundation(&tx, foundation, source_version)?;
        if let Some(run) = run {
            let n=tx.execute("UPDATE jobs SET state='completed',result=?1,error=NULL,updated_at=?2 WHERE id=?3 AND kind='workflow' AND state='running' AND json_extract(snapshot,'$.stage')='images.design' AND json_extract(snapshot,'$.sourceVersion')=?4",params![output.to_string(),now(),run,source_version]).map_err(|e|e.to_string())?;
            if n != 1 {
                return Err("原资产设计父流程不再运行或源不符，整笔采用已回滚".into());
            }
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'asset_design_published',?2,?3)",params![run,json!({"version":"stage2-atomic-publication/v1","sourceVersion":source_version,"outputSha256":crate::pipeline::digest(&output.to_string())}).to_string(),now()]).map_err(|e|e.to_string())?;
        }
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn publish_storyboard(
        &self,
        guard: &Value,
        output: &Value,
        input: &Value,
        run: Option<&str>,
    ) -> Result<()> {
        self.publish_storyboard_inner(guard, output, input, run, false, None)
    }
    /// Complete episode publication and its running-workflow receipt share one
    /// transaction. A transport fragment or an accepted author is insufficient.
    pub fn publish_storyboard_episode(
        &self,
        guard: &Value,
        output: &Value,
        input: &Value,
        run: &str,
    ) -> Result<()> {
        crate::pipeline::validate_episode_delivery(input, output)?;
        self.publish_storyboard_inner(guard, output, input, Some(run), true, None)
    }

    /// Reserve a fresh refinement only while its original project settings and
    /// complete publication preimage still match. Preflight execution identity
    /// is frozen separately and never substituted for this settings guard.
    pub(crate) fn reserve_storyboard_refinement_workflow(&self, snapshot: &Value) -> Result<String> {
        if snapshot["stage"] != "storyboard.refine"
            || snapshot["runnerVersion"] != "desktop-runner/v1"
            || snapshot["guard"] != guard_from_input(&snapshot["input"])
        {
            return Err("新精修流程身份或冻结发布依据无效；未创建任务".into());
        }
        let mut connection = self.conn()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if snapshot["savedSettingsSha256"]
            != crate::pipeline::digest(&self.settings()?.to_string())
        {
            return Err("项目保存设置在精修创建前变化；未创建任务或派发请求，请刷新后重新开始".into());
        }
        verify_guard(&tx, &snapshot["guard"])?;
        let run = crate::store::id();
        tx.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,created_at,updated_at) VALUES(?1,'workflow','storyboard.refine','local','queued','local',?2,?3,?3)",
            params![run,snapshot.to_string(),now()]).map_err(|e|e.to_string())?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(run)
    }

    /// A completed director operation stores a candidate. Only this explicit
    /// publication path can adopt it. Candidate bytes and its text checkpoints
    /// stay immutable; unit writes and an independent adoption receipt commit
    /// together. Repeating the same adoption never creates new unit versions.
    pub fn publish_storyboard_refinement(
        &self,
        run: &str,
        expected_candidate_sha: &str,
        current_input: &Value,
    ) -> Result<()> {
        self.publish_storyboard_inner(
            &guard_from_input(current_input),
            &Value::Null,
            current_input,
            None,
            false,
            Some(RefinementAdoption {
                run,
                expected_candidate_sha,
            }),
        )
    }
    fn publish_storyboard_inner(
        &self,
        guard: &Value,
        output: &Value,
        input: &Value,
        run: Option<&str>,
        episode_progress: bool,
        adoption: Option<RefinementAdoption<'_>>,
    ) -> Result<()> {
        self.ensure_workflow_storage()?;
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut adoption_basis = None;
        let mut adopted_output = None;
        if let Some(adoption) = &adoption {
            let (snapshot_raw, candidate_raw, prepared_raw, state): (String, Option<String>, Option<String>, String) = tx
                .query_row("SELECT snapshot,result,prepared,state FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.refine'",
                    [adoption.run], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
                .map_err(|_| "导演精修来源流程不存在")?;
            let snapshot = parse(snapshot_raw)?;
            let candidate_raw = candidate_raw.ok_or("精修来源尚无完整候选")?;
            let candidate = parse(candidate_raw.clone())?;
            let prepared = prepared_raw.map(parse).transpose()?.unwrap_or(json!({}));
            if state != "completed"
                || snapshot["stage"] != "storyboard.refine"
                || snapshot["runnerVersion"] != "desktop-runner/v1"
                || candidate["candidateSha256"] != adoption.expected_candidate_sha
            {
                return Err("精修候选来源状态或预览指纹已变化；原稿保持".into());
            }
            if let Some(receipt) = prepared.get("refinementAdoption") {
                if receipt["version"] != "storyboard-refinement-adoption/v1"
                    || receipt["workflowId"] != adoption.run
                    || receipt["candidateSha256"] != adoption.expected_candidate_sha
                {
                    return Err("精修已有采用记录的身份不符，未改写历史记录".into());
                }
                return Ok(());
            }
            // The immediate write lock is already held, so the complete Store
            // settings view cannot change between this check and commit.
            if snapshot["savedSettingsSha256"]
                != crate::pipeline::digest(&self.settings()?.to_string())
            {
                return Err("项目保存设置已变化，精修候选保留但未采用".into());
            }
            verify_guard(&tx, guard)?;
            let episode = s(&candidate, "episodeId");
            if !guard["episodeIds"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == episode))
            {
                return Err("精修发布未保护完整采用集稿范围".into());
            }
            let mut query=tx.prepare("SELECT data FROM units WHERE episode_id=?1 AND COALESCE(json_extract(data,'$.retired'),0)<>1").map_err(|e|e.to_string())?;
            let actual = query
                .query_map([episode], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .map(|row| parse(row.map_err(|e| e.to_string())?))
                .collect::<Result<Vec<_>>>()?;
            drop(query);
            let supplied: Vec<_> = input["existingUnits"]
                .as_array()
                .ok_or("精修采用缺少当前集稿")?
                .iter()
                .filter(|unit| unit["episodeId"] == episode && unit["retired"] != true)
                .collect();
            if actual.len() != supplied.len()
                || actual.iter().any(|unit| {
                    supplied
                        .iter()
                        .filter(|saved| saved["id"] == unit["id"] && **saved == unit)
                        .count()
                        != 1
                })
            {
                return Err("精修采用依据不是当前数据库的完整集稿；未覆盖人工编辑".into());
            }
            // Require the original domain-accepted text checkpoint as well as
            // the top-level candidate. A copied result is not a new authority.
            let mut query=tx.prepare("SELECT node_key FROM workflow_steps WHERE workflow_id=?1 AND stage='storyboardRefine'").map_err(|e|e.to_string())?;
            let nodes = query
                .query_map([adoption.run], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(query);
            let mut matching = 0;
            for node in nodes {
                if let Some(step) = progress_step(&tx, adoption.run, &node)? {
                    let mut raw = step.result.clone();
                    if let Some(object) = raw.as_object_mut() {
                        object.remove("materializedDraft");
                    }
                    if step.status == "accepted"
                        && step.request.stage == "storyboardRefine"
                        && step.request.skill == "director-refinement"
                        && step.domain_hash == candidate["domainRequestSha256"]
                        && step.request.rule_snapshot["creativePack"] == candidate["ruleReceipt"]
                        && raw == candidate["rawDelta"]
                        && step.event["requestHash"] == step.hash
                        && step.event["value"] == candidate["rawDelta"]
                    {
                        matching += 1;
                    }
                }
            }
            if matching != 1 {
                return Err("精修候选未回绑唯一原始验收回包；未采用复制或不明来源成果".into());
            }
            adopted_output = Some(crate::pipeline::adopt_storyboard_refinement(
                input, &candidate,
            )?);
            adoption_basis = Some((candidate, candidate_raw, prepared));
        }
        let output = adopted_output.as_ref().unwrap_or(output);
        verify_guard(&tx, guard)?;
        let authored = output["units"].as_array().ok_or("创作结果缺少单元数组")?;
        // A successful transport group is a saved draft, not proof that the
        // episode is complete. Failed later groups must retain the old units.
        let units: Vec<_> = authored
            .iter()
            .filter(|unit| {
                output["scope"] != "episodes"
                    || output.get("updatedEpisodeIds").is_none()
                    || output["updatedEpisodeIds"]
                        .as_array()
                        .is_some_and(|ids| ids.iter().any(|eid| eid == &unit["episodeId"]))
            })
            .cloned()
            .collect();
        if units.is_empty() {
            return Ok(());
        }
        let published: HashSet<_> = units.iter().map(|u| s(u, "id").to_owned()).collect();
        if published.len() != units.len() || published.contains("") {
            return Err("单元ID重复或为空，未发布成果".into());
        }
        let delivery_sha = crate::pipeline::digest(&output.to_string());
        if episode_progress {
            let run = run.ok_or("逐集发布缺少流程身份")?;
            let (raw, state): (Option<String>, String) = tx
                .query_row(
                    "SELECT prepared,state FROM jobs WHERE id=?1 AND kind='workflow'",
                    [run],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|e| e.to_string())?;
            let prepared = raw.map(parse).transpose()?.unwrap_or(json!({}));
            let episode = s(output, "episodeId");
            if prepared["publishedEpisodes"][episode]["episodeDeliverySha256"] == delivery_sha {
                // Guard verification above still detects user edits. Replaying
                // accepted text cannot create another unit version or receipt.
                return Ok(());
            }
            if state != "running" {
                return Err("流程已停止，完整单集稿保留但不迟到发布".into());
            }
        }
        let episodes: HashSet<_> = units.iter().map(|u| s(u, "episodeId").to_owned()).collect();
        let replaced: HashSet<_> = output["replacedUnitIds"]
            .as_array()
            .filter(|v| !v.is_empty())
            .or_else(|| input["selection"]["unitIds"].as_array())
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut q = tx
            .prepare("SELECT data FROM units WHERE COALESCE(json_extract(data,'$.retired'),0)<>1")
            .map_err(|e| e.to_string())?;
        let current = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(q);
        let current: Vec<Value> = current.into_iter().map(parse).collect::<Result<_>>()?;
        let in_scope = |old: &Value| {
            if replaced.is_empty() {
                episodes.contains(s(old, "episodeId"))
            } else {
                replaced.contains(s(old, "id"))
            }
        };
        let scoped: HashSet<String> = current
            .iter()
            .filter(|old| in_scope(old))
            .map(|old| s(old, "id").to_owned())
            .collect();
        let mut external_successors = HashSet::new();
        for old in current.iter().filter(|old| in_scope(old)) {
            let id = s(old, "id");
            let replacement = units.iter().find(|u| s(u, "id") == id);
            let state_changed = if let Some((candidate, _, _)) = &adoption_basis {
                candidate["validatedDelta"]["replacements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|group| {
                        group["replaces"].as_array().unwrap().last() == Some(&old["id"])
                            && group["units"].as_array().unwrap().last().unwrap()["exitState"]
                                != old["exitState"]
                    })
            } else {
                replacement.is_none_or(|u| u["exitState"] != old["exitState"])
            };
            if state_changed {
                mark_dependents_tx(&tx, id, &published)?;
            }
            let mut successors = tx
                .prepare("SELECT unit_id FROM unit_dependencies WHERE dependency_id=?1")
                .map_err(|e| e.to_string())?;
            for row in successors
                .query_map([id], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?
            {
                let next = row.map_err(|e| e.to_string())?;
                if !scoped.contains(&next) && !published.contains(&next) {
                    external_successors.insert(next);
                }
            }
            if !published.contains(id) {
                let mut retired = old.clone();
                retired["retired"] = json!(true);
                save_unit_tx(&tx, retired)?;
            }
        }
        let mut prior = input
            .get("previousBoundary")
            .filter(|v| !v.is_null())
            .map(|u| s(u, "id").to_owned())
            .filter(|s| !s.is_empty());
        if !replaced.is_empty() {
            if let Some(first) = input["existingUnits"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|u| scoped.contains(s(u, "id")))
            {
                prior = tx
                    .query_row(
                        "SELECT dependency_id FROM unit_dependencies WHERE unit_id=?1 LIMIT 1",
                        [s(first, "id")],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?
                    .filter(|id| !scoped.contains(id))
                    .or(prior);
            }
        }
        for unit in units {
            let mut value = unit.clone();
            value["retired"] = json!(false);
            value["continuityStale"] = json!(false);
            value["planningProfile"] = input["capabilities"].clone();
            value["sourceVersion"] = guard["sourceVersion"].clone();
            value["foundationVersion"] = guard["foundationVersion"].clone();
            let saved = save_unit_tx(&tx, value)?;
            let uid = s(&saved, "id");
            if adoption.is_none() {
                tx.execute("DELETE FROM unit_dependencies WHERE unit_id=?1", [uid])
                    .map_err(|e| e.to_string())?;
                if let Some(ref previous) = prior {
                    if previous != uid {
                        tx.execute(
                            "INSERT OR IGNORE INTO unit_dependencies VALUES(?1,?2)",
                            params![uid, previous],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
            prior = Some(uid.to_owned());
        }
        if let Some(last) = prior.as_deref().filter(|_| adoption.is_none()) {
            for next in external_successors {
                for old in &scoped {
                    tx.execute(
                        "DELETE FROM unit_dependencies WHERE unit_id=?1 AND dependency_id=?2",
                        params![next, old],
                    )
                    .map_err(|e| e.to_string())?;
                }
                tx.execute(
                    "INSERT OR IGNORE INTO unit_dependencies VALUES(?1,?2)",
                    params![next, last],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        for impact in output["continuityImpacts"].as_array().into_iter().flatten() {
            let id = s(impact, "nextUnitId");
            if !id.is_empty() && !published.contains(id) {
                mark_unit_stale_tx(&tx, id)?;
                mark_dependents_tx(&tx, id, &published)?;
            }
        }
        if let (Some(adoption), Some((candidate, candidate_raw, mut prepared))) =
            (&adoption, adoption_basis)
        {
            rewire_refinement_groups(&tx, &candidate["validatedDelta"])?;
            let episode = s(&candidate, "episodeId");
            let mut query=tx.prepare("SELECT data FROM units WHERE episode_id=?1 AND COALESCE(json_extract(data,'$.retired'),0)<>1").map_err(|e|e.to_string())?;
            let adopted_units = query
                .query_map([episode], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .map(|row| parse(row.map_err(|e| e.to_string())?))
                .collect::<Result<Vec<_>>>()?;
            drop(query);
            let receipt = json!({"version":"storyboard-refinement-adoption/v1","workflowId":adoption.run,
                "candidateSha256":adoption.expected_candidate_sha,"sourceSha256":candidate["sourceSha256"],"episodeId":episode,
                "domainRequestSha256":candidate["domainRequestSha256"],"deliverySha256":delivery_sha,
                "acceptedPlanContext":candidate["acceptedContext"],"adoptedUnits":adopted_units,
                "unitBindings":adopted_units.iter().map(|unit|json!({"unitId":unit["id"],"unitVersion":unit["version"],"unitSha256":crate::pipeline::digest(&unit.to_string())})).collect::<Vec<_>>(),
                "fantasyRealization":candidate["draft"]["fantasyRealization"],"ruleReceipt":candidate["ruleReceipt"],"adoptedAt":now()});
            prepared["refinementAdoption"] = receipt.clone();
            let changed=tx.execute("UPDATE jobs SET prepared=?1,updated_at=?2 WHERE id=?3 AND kind='workflow' AND entity_id='storyboard.refine' AND state='completed' AND result=?4",
                params![prepared.to_string(),now(),adoption.run,candidate_raw]).map_err(|e|e.to_string())?;
            if changed != 1 {
                return Err("精修候选在采用期间改变，整笔发布已回滚".into());
            }
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'storyboard_refinement_adopted',?2,?3)",params![adoption.run,receipt.to_string(),now()]).map_err(|e|e.to_string())?;
        }
        if let Some(run) = run {
            let mut next_guard = guard.clone();
            let mut versions = serde_json::Map::new();
            let scopes: HashSet<_> = guard["episodeIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let mut q=tx.prepare("SELECT id,episode_id,version FROM units WHERE COALESCE(json_extract(data,'$.retired'),0)<>1").map_err(|e|e.to_string())?;
            for row in q
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?
            {
                let (id, ep, v) = row.map_err(|e| e.to_string())?;
                if scopes.contains(ep.as_str()) || !guard["units"][&id].is_null() {
                    versions.insert(id, json!(v));
                }
            }
            drop(q);
            next_guard["units"] = json!(versions);
            let raw: Option<String> = tx
                .query_row("SELECT prepared FROM jobs WHERE id=?1", [run], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let mut prepared = raw.map(parse).transpose()?.unwrap_or(json!({}));
            prepared["publicationGuard"] = next_guard;
            let mut result = output.clone();
            if episode_progress {
                let episode = s(output, "episodeId");
                let old: Option<String> = tx
                    .query_row("SELECT result FROM jobs WHERE id=?1", [run], |row| {
                        row.get(0)
                    })
                    .map_err(|e| e.to_string())?;
                result = old.map(parse).transpose()?.unwrap_or(json!({"schemaVersion":"storyboard-delivery/v1","scope":"episodes","units":[],"updatedEpisodeIds":[],"plans":[],"fantasyRealization":[],"revisionNotes":[],"unresolved":[],"blocked":[]}));
                let old_ids: HashSet<_> = result["units"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|unit| s(unit, "episodeId") == episode)
                    .map(|unit| s(unit, "id").to_owned())
                    .collect();
                for field in ["units", "plans"] {
                    let mut entries: Vec<_> = result[field]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|entry| s(entry, "episodeId") != episode)
                        .cloned()
                        .collect();
                    entries.extend(output[field].as_array().into_iter().flatten().cloned());
                    if field == "units" {
                        let order: Vec<_> = input["source"]["episodes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|e| s(e, "id"))
                            .collect();
                        entries.sort_by_key(|entry| {
                            order
                                .iter()
                                .position(|id| *id == s(entry, "episodeId"))
                                .unwrap_or(usize::MAX)
                        });
                    }
                    result[field] = json!(entries);
                }
                let mut realization: Vec<_> = result["fantasyRealization"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|entry| {
                        !entry["unitIds"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .any(|id| old_ids.contains(id))
                    })
                    .cloned()
                    .collect();
                realization.extend(
                    output["fantasyRealization"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
                result["fantasyRealization"] = json!(realization);
                for field in ["revisionNotes", "unresolved"] {
                    let mut notes = result[field].as_array().cloned().unwrap_or_default();
                    for note in output[field].as_array().into_iter().flatten() {
                        if !notes.contains(note) {
                            notes.push(note.clone());
                        }
                    }
                    result[field] = json!(notes);
                }
                let mut ids = result["updatedEpisodeIds"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if !ids.iter().any(|id| id.as_str() == Some(episode)) {
                    ids.push(json!(episode));
                }
                result["updatedEpisodeIds"] = json!(ids);
                result["sourceSha256"] = output["sourceSha256"].clone();
                result["schemaVersion"] = json!("storyboard-delivery/v1");
                result["scope"] = json!("episodes");
                result["status"] = json!("running");
                result["blocked"] = json!([]);
                result["waitingDecision"] = Value::Null;
                result["waitingDecisions"] = json!([]);
                result["ruleSnapshots"] = output["ruleSnapshots"].clone();
                result["planningProfile"] = input["capabilities"].clone();
                result["selection"] = input["selection"].clone();
                if !prepared["publishedEpisodes"].is_object() {
                    prepared["publishedEpisodes"] = json!({});
                }
                prepared["publishedEpisodes"][episode] = json!({"version":"complete-episode-publication/v1","episodeId":episode,"sourceSha256":output["sourceSha256"],"sourceVersion":guard["sourceVersion"],"episodeDeliverySha256":delivery_sha,"unitIds":output["units"].as_array().unwrap().iter().map(|u|&u["id"]).collect::<Vec<_>>(),"finishProvenance":output["finishProvenance"],"publishedAt":now()});
            }
            let status = if episode_progress {
                "running"
            } else {
                match s(output, "status") {
                    "waiting_decision" => "waiting_decision",
                    "partially_blocked" => "partially_blocked",
                    _ => "completed",
                }
            };
            tx.execute("UPDATE jobs SET state=?1,prepared=?2,result=?3,error=NULL,updated_at=?4 WHERE id=?5 AND kind='workflow'",params![status,prepared.to_string(),result.to_string(),now(),run]).map_err(|e|e.to_string())?;
            if episode_progress {
                tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'episode_published',?2,?3)",params![run,prepared["publishedEpisodes"][s(output,"episodeId")].to_string(),now()]).map_err(|e|e.to_string())?;
            }
        }
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn invalidate_unit_dependents(&self, id: &str) -> Result<()> {
        if !has_steps(self)? {
            return Ok(());
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        mark_dependents_tx(&tx, id, &HashSet::new())?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
}
pub fn guard_from_input(input: &Value) -> Value {
    let assets: BTreeMap<_, _> = input["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| !crate::store::asset_deleted(a))
        .map(|a| (s(a, "id").to_owned(), a["version"].clone()))
        .collect();
    let scope: HashSet<_> = input["selection"]["episodeIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut units: BTreeMap<_, _> = input["existingUnits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| scope.contains(s(u, "episodeId")))
        .map(|u| (s(u, "id").to_owned(), u["version"].clone()))
        .collect();
    let previous = &input["previousBoundary"];
    if !s(previous, "id").is_empty() {
        units.insert(s(previous, "id").to_owned(), previous["version"].clone());
    }
    json!({"sourceVersion":input["source"]["version"],"foundationVersion":input["foundation"]["version"].as_i64().unwrap_or(0),"assets":assets,"units":units,"episodeIds":input["selection"]["episodeIds"]})
}

impl Store {
    /// Image-stage audit receipts are not new creative input. Rebind only the
    /// publication guard when every foundation field consumed by Stage 3 is
    /// exactly unchanged; source, catalogue and unit guards still must pass.
    pub(crate) fn refresh_storyboard_audit_guard(&self, run: &str, snapshot: &Value) -> Result<()> {
        if snapshot["stage"] != "storyboard.run" || snapshot["runnerVersion"] != "desktop-runner/v1"
        {
            return Ok(());
        }
        let mut connection = self.conn()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (saved_snapshot,saved_prepared):(String,Option<String>)=tx.query_row("SELECT snapshot,prepared FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|e|e.to_string())?;
        if parse(saved_snapshot.clone())? != *snapshot {
            return Err("原分镜流程快照已变化，未更新发布依据".into());
        }
        let mut prepared = saved_prepared
            .as_ref()
            .map(|raw| parse(raw.clone()))
            .transpose()?
            .unwrap_or(json!({}));
        let guard = prepared
            .get("publicationGuard")
            .unwrap_or(&snapshot["guard"]);
        let (version, current) = latest_foundation(&tx)?;
        if guard["foundationVersion"] == version {
            return Ok(());
        }
        if guard["foundationVersion"]
            .as_i64()
            .is_none_or(|old| version <= old)
        {
            return Ok(());
        }
        let source_sha: String = tx
            .query_row(
                "SELECT sha256 FROM sources ORDER BY version DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if snapshot["input"]["source"]["sha256"] != source_sha {
            return Err("锁定原文摘要已变化，未更新分镜发布依据".into());
        }
        let Some(mut current) = current else {
            return Ok(());
        };
        let mut frozen = snapshot["input"]["foundation"].clone();
        if !frozen.is_object() || !current.is_object() {
            return Ok(());
        }
        if frozen["sourceSha256"] != snapshot["input"]["source"]["sha256"]
            || current["sourceSha256"] != frozen["sourceSha256"]
        {
            return Ok(());
        }
        for field in ["version", "visualFactBatches"] {
            frozen.as_object_mut().unwrap().remove(field);
            current.as_object_mut().unwrap().remove(field);
        }
        if frozen != current {
            return Ok(());
        }
        let mut next = guard.clone();
        next["foundationVersion"] = json!(version);
        verify_guard(&tx, &next)?;
        let proof = json!({"sourceVersion":next["sourceVersion"],"sourceSha256":snapshot["input"]["source"]["sha256"],"previousFoundationVersion":guard["foundationVersion"],"currentFoundationVersion":version,"unchangedCreativeInputSha256":crate::pipeline::digest(&current.to_string()),"frozenSnapshotSha256":crate::pipeline::digest(&saved_snapshot),"newModelCalls":0});
        prepared["publicationGuard"] = next;
        let updated = tx
            .execute(
                "UPDATE jobs SET prepared=?1 WHERE id=?2 AND snapshot=?3 AND prepared IS ?4",
                params![prepared.to_string(), run, saved_snapshot, saved_prepared],
            )
            .map_err(|e| e.to_string())?;
        if updated != 1 {
            return Err("分镜发布依据发生竞争，未更新或重新生成".into());
        }
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'foundation_audit_guard_refreshed',?2,?3)",params![run,proof.to_string(),now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}
pub fn verify_guard(tx: &Transaction<'_>, guard: &Value) -> Result<()> {
    let source: i64 = tx
        .query_row("SELECT COALESCE(MAX(version),0) FROM sources", [], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let foundation: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(version),0) FROM foundations",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if guard["sourceVersion"].as_i64() != Some(source)
        || guard["foundationVersion"].as_i64() != Some(foundation)
    {
        return Err("原文或制作基础已更新；本次成果已保留，未覆盖当前版本".into());
    }
    for table in ["assets", "units"] {
        for (id, expected) in guard[table].as_object().ok_or("冻结版本清单缺失")? {
            let actual: Option<i64> = tx
                .query_row(
                    &format!("SELECT version FROM {table} WHERE id=?1"),
                    [id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if actual != expected.as_i64() {
                return Err(format!("{table}中的{id}已更新；未覆盖当前成果"));
            }
        }
    }
    let asset_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM assets WHERE COALESCE(json_extract(data,'$.deletedAt'),'')=''",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if guard["assets"].as_object().map(|m| m.len() as i64) != Some(asset_count) {
        return Err("资产目录已新增或移除，请刷新后继续制作；本次不会覆盖当前目录".into());
    }
    let scope: HashSet<_> = guard["episodeIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !scope.is_empty() {
        let mut q=tx.prepare("SELECT id,episode_id FROM units WHERE COALESCE(json_extract(data,'$.retired'),0)<>1").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, ep) = row.map_err(|e| e.to_string())?;
            if scope.contains(ep.as_str()) && guard["units"][&id].is_null() {
                return Err("所选集的单元集合已更新，请刷新后重新制作".into());
            }
        }
    }
    Ok(())
}
fn latest_foundation(tx: &Transaction<'_>) -> Result<(i64, Option<Value>)> {
    let raw: Option<(i64, String)> = tx
        .query_row(
            "SELECT version,data FROM foundations ORDER BY version DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match raw {
        Some((v, data)) => Ok((v, Some(parse(data)?))),
        None => Ok((0, None)),
    }
}
fn mark_unit_stale_tx(tx: &Transaction<'_>, id: &str) -> Result<()> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT data FROM units WHERE id=?1 AND COALESCE(json_extract(data,'$.retired'),0)<>1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(raw) = raw {
        let mut value = parse(raw)?;
        value["continuityStale"] = json!(true);
        save_unit_tx(tx, value)?;
    }
    Ok(())
}
fn mark_dependents_tx(tx: &Transaction<'_>, id: &str, skip: &HashSet<String>) -> Result<()> {
    let mut q=tx.prepare("WITH RECURSIVE affected(id) AS (SELECT unit_id FROM unit_dependencies WHERE dependency_id=?1 UNION SELECT d.unit_id FROM unit_dependencies d JOIN affected a ON d.dependency_id=a.id) SELECT id FROM affected WHERE id<>?1").map_err(|e|e.to_string())?;
    let affected = q
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    for dependent in affected {
        if !skip.contains(&dependent) {
            mark_unit_stale_tx(tx, &dependent)?;
        }
    }
    Ok(())
}
fn write_foundation(tx: &Transaction<'_>, mut data: Value, source_version: i64) -> Result<Value> {
    let (version, old) = latest_foundation(tx)?;
    data["sourceVersion"] = json!(source_version);
    if old.as_ref().is_some_and(|old| same_content(old, &data)) {
        return Ok(old.unwrap());
    }
    data["version"] = json!(version + 1);
    tx.execute(
        "INSERT INTO foundations VALUES(?1,?2,?3,?4)",
        params![version + 1, source_version, data.to_string(), now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(data)
}
fn step_title(stage: &str) -> &str {
    match stage {
        "foundation:F2" => "资产整理",
        "foundation:Q1" => "资料整编",
        "foundationExtract" => "提取制作基础",
        "storyboardPlan" => "情绪与事件规划",
        "storyboardCreate" => "完整单元主创",
        "storyboardFinish" => "关联范围整编",
        _ => "整理制作内容",
    }
}
