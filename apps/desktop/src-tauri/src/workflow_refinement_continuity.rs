//! Explicit review of an adopted refinement's exclusive metadata-only fallout.
//! This is not an API for clearing arbitrary continuity flags.
use super::*;
use std::collections::{BTreeSet, HashMap};
const FIELD: &str = "refinementContinuityReview";
const PLAN: &str = "refinement-continuity-review-plan/v1";
const REVIEW: &str = "refinement-continuity-review/v1";
const RECEIPT: &str = "refinement-continuity-review-adoption/v1";
const HELD: &str =
    "CONTINUITY_REVIEW_HELD: 来源、唯一影响链或单位/依赖图已变化，保留原稿及失效标记";
fn sha(value: &Value) -> String {
    crate::pipeline::digest(&value.to_string())
}
fn seal(mut value: Value) -> Value {
    value["sha256"] = json!(sha(&value));
    value
}
fn valid_seal(value: &Value) -> bool {
    let mut raw = value.clone();
    if let Some(m) = raw.as_object_mut() {
        m.remove("sha256");
    }
    value["sha256"] == sha(&raw)
}
fn values(c: &Connection, table: &str) -> Result<Vec<Value>> {
    let mut q = c
        .prepare(&format!("SELECT data FROM {table} ORDER BY rowid"))
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = rows
        .map(|r| parse(r.map_err(|e| e.to_string())?))
        .collect::<Result<Vec<_>>>()?;
    out.retain(|v| table != "assets" || !crate::store::asset_deleted(v));
    out.sort_by_key(|v| s(v, "id").to_owned());
    Ok(out)
}
fn bindings(units: &[Value]) -> Value {
    json!(units
        .iter()
        .map(|u| json!({"unitId":u["id"],"unitVersion":u["version"],"unitSha256":sha(u)}))
        .collect::<Vec<_>>())
}
fn no_derived_metadata(value: &Value) -> Value {
    let mut v = value.clone();
    if let Some(m) = v.as_object_mut() {
        m.remove("continuityStale");
        m.remove("version");
    }
    v
}
fn job(c: &Connection, run: &str) -> Result<(String, String, Value, Value, Value)> {
    let (snap,raw,prep,state):(String,Option<String>,Option<String>,String)=c.query_row("SELECT snapshot,result,prepared,state FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.refine'",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|HELD)?;
    if state != "completed" {
        return Err(HELD.into());
    }
    let raw = raw.ok_or(HELD)?;
    let snapshot = parse(snap.clone())?;
    let candidate = parse(raw.clone())?;
    let prepared = prep.map(parse).transpose()?.unwrap_or(Value::Null);
    Ok((snap, raw, snapshot, candidate, prepared))
}
fn linear_graph(
    c: &Connection,
    units: &[Value],
    source: &Value,
) -> Result<(Value, HashMap<String, String>)> {
    let mut q = c
        .prepare(
            "SELECT unit_id,dependency_id FROM unit_dependencies ORDER BY unit_id,dependency_id",
        )
        .map_err(|e| e.to_string())?;
    let edges = q
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if units.len() < 2 || edges.len() + 1 != units.len() {
        return Err(HELD.into());
    }
    let map: HashMap<_, _> = units.iter().map(|u| (s(u, "id"), u)).collect();
    if map.len() != units.len() {
        return Err(HELD.into());
    }
    let rows = source["rows"].as_array().ok_or(HELD)?;
    let position = |u: &Value| {
        u["sourceRefs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| rows.iter().position(|r| r["id"] == *id))
            .min()
    };
    let mut next = HashMap::new();
    let mut incoming = HashSet::new();
    for (child, parent) in &edges {
        let (Some(a), Some(b)) = (map.get(parent.as_str()), map.get(child.as_str())) else {
            return Err(HELD.into());
        };
        if next.insert(parent.clone(), child.clone()).is_some()
            || !incoming.insert(child.clone())
            || position(a).is_none()
            || position(a) > position(b)
        {
            return Err(HELD.into());
        }
    }
    let heads: Vec<_> = map
        .keys()
        .filter(|id| !incoming.contains(**id))
        .copied()
        .collect();
    let [head] = heads.as_slice() else {
        return Err(HELD.into());
    };
    let mut seen = HashSet::new();
    let mut at = (*head).to_owned();
    loop {
        if !seen.insert(at.clone()) {
            return Err(HELD.into());
        }
        match next.get(&at) {
            Some(n) => at = n.clone(),
            None => break,
        }
    }
    if seen.len() != units.len() {
        return Err(HELD.into());
    }
    Ok((
        json!(edges
            .iter()
            .map(|(child, parent)| json!({"unitId":child,"dependencyId":parent}))
            .collect::<Vec<_>>()),
        next,
    ))
}
fn preview(c: &Connection, run: &str, expected: &str) -> Result<Value> {
    let (snapshot_raw, candidate_raw, snapshot, candidate, prepared) = job(c, run)?;
    if candidate["candidateSha256"] != expected || prepared.get(FIELD).is_some() {
        return Err(HELD.into());
    }
    let adopted = &prepared["refinementAdoption"];
    if adopted["version"] != "storyboard-refinement-adoption/v1"
        || adopted["workflowId"] != run
        || adopted["candidateSha256"] != expected
    {
        return Err(HELD.into());
    }
    let event:Option<String>=c.query_row("SELECT data FROM events WHERE job_id=?1 AND type='storyboard_refinement_adopted' ORDER BY id DESC LIMIT 1",[run],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if event.as_deref() != Some(adopted.to_string().as_str()) {
        return Err(HELD.into());
    }
    let mut frozen = snapshot["input"].clone();
    crate::pipeline::storyboard_refinement_identity::attach_recovery(
        &mut frozen,
        &snapshot,
        &prepared,
    )?;
    let delivery = crate::pipeline::adopt_storyboard_refinement(&frozen, &candidate)?;
    if adopted["deliverySha256"] != sha(&delivery) {
        return Err(HELD.into());
    }
    let mut q = c
        .prepare(
            "SELECT node_key FROM workflow_steps WHERE workflow_id=?1 AND stage='storyboardRefine'",
        )
        .map_err(|e| e.to_string())?;
    let nodes = q
        .query_map([run], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    let mut accepted = 0;
    for node in nodes {
        if let Some(step) = progress_step(c, run, &node)? {
            let mut raw = step.result.clone();
            if let Some(m) = raw.as_object_mut() {
                m.remove("materializedDraft");
            }
            if step.status == "accepted"
                && step.domain_hash == candidate["domainRequestSha256"]
                && step.request.rule_snapshot["creativePack"] == candidate["ruleReceipt"]
                && raw == candidate["rawDelta"]
                && step.event["value"] == raw
                && step.event["requestHash"] == step.hash
            {
                accepted += 1;
            }
        }
    }
    if accepted != 1 {
        return Err(HELD.into());
    }
    let source_raw: String = c
        .query_row(
            "SELECT data FROM sources ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| HELD)?;
    let source_raw = parse(source_raw)?;
    let locked: crate::pipeline::LockedSource =
        serde_json::from_value(source_raw.clone()).map_err(|_| HELD)?;
    let locked = crate::pipeline::upgrade_source_dialogues(&locked)?;
    crate::pipeline::validate_source(&locked)?;
    let mut source = serde_json::to_value(locked).map_err(|e| e.to_string())?;
    source["version"] = source_raw["version"].clone();
    if source != frozen["source"] || source["sha256"] != adopted["sourceSha256"] {
        return Err(HELD.into());
    }
    let assets = values(c, "assets")?;
    let mut old_assets = frozen["assets"].as_array().ok_or(HELD)?.clone();
    old_assets.sort_by_key(|a| s(a, "id").to_owned());
    if assets != old_assets {
        return Err(HELD.into());
    }
    let units: Vec<_> = values(c, "units")?
        .into_iter()
        .filter(|u| u["retired"] != true)
        .collect();
    let old: Vec<_> = frozen["existingUnits"]
        .as_array()
        .ok_or(HELD)?
        .iter()
        .filter(|u| u["retired"] != true)
        .cloned()
        .collect();
    if old.iter().any(|u| u["continuityStale"] == true) {
        return Err(HELD.into());
    }
    let (graph, next) = linear_graph(c, &units, &source)?;
    let impacts = delivery["continuityImpacts"]
        .as_array()
        .filter(|v| v.len() == 1)
        .ok_or(HELD)?;
    let impact = &impacts[0];
    let from = s(impact, "fromUnitId");
    let frontier = s(impact, "nextUnitId");
    if next.get(from).map(String::as_str) != Some(frontier) {
        return Err(HELD.into());
    }
    let published: HashSet<_> = delivery["units"]
        .as_array()
        .ok_or(HELD)?
        .iter()
        .map(|u| s(u, "id"))
        .collect();
    let mut affected = BTreeSet::new();
    let mut at = frontier.to_owned();
    loop {
        if published.contains(at.as_str()) || !affected.insert(at.clone()) {
            return Err(HELD.into());
        }
        match next.get(&at) {
            Some(n) => at = n.clone(),
            None => break,
        }
    }
    let stale: BTreeSet<_> = units
        .iter()
        .filter(|u| u["continuityStale"] == true)
        .map(|u| s(u, "id").to_owned())
        .collect();
    if affected != stale || affected.is_empty() {
        return Err(HELD.into());
    }
    let adopted_units = adopted["adoptedUnits"].as_array().ok_or(HELD)?;
    let expected_ids: HashSet<_> = old
        .iter()
        .filter(|u| u["episodeId"] != candidate["episodeId"])
        .chain(adopted_units.iter())
        .map(|u| s(u, "id"))
        .collect();
    if expected_ids.len() != units.len() || units.iter().any(|u| !expected_ids.contains(s(u, "id")))
    {
        return Err(HELD.into());
    }
    for unit in &units {
        let prior = old.iter().find(|o| o["id"] == unit["id"]);
        if affected.contains(s(unit, "id")) {
            let prior = prior.ok_or(HELD)?;
            let mut expected = prior.clone();
            let version = prior["version"].as_u64().filter(|v| *v > 0).ok_or(HELD)?;
            expected["continuityStale"] = json!(true);
            expected["version"] = json!(version.checked_add(1).ok_or(HELD)?);
            if expected != *unit || no_derived_metadata(prior) != no_derived_metadata(unit) {
                return Err(HELD.into());
            }
        }
        let expected = if unit["episodeId"] == candidate["episodeId"] {
            adopted_units
                .iter()
                .find(|o| o["id"] == unit["id"])
                .ok_or(HELD)?
        } else {
            prior.ok_or(HELD)?
        };
        if affected.contains(s(unit, "id")) {
            if unit["episodeId"] == candidate["episodeId"] && unit != expected {
                return Err(HELD.into());
            }
        } else if unit != expected {
            return Err(HELD.into());
        }
    }
    let from_unit = units.iter().find(|u| s(u, "id") == from).ok_or(HELD)?;
    let first = units.iter().find(|u| s(u, "id") == frontier).ok_or(HELD)?;
    let group = candidate["validatedDelta"]["replacements"]
        .as_array()
        .ok_or(HELD)?
        .iter()
        .find(|g| {
            g["units"]
                .as_array()
                .and_then(|u| u.last())
                .is_some_and(|u| s(u, "id") == from)
        })
        .ok_or(HELD)?;
    let old_last = group["replaces"]
        .as_array()
        .and_then(|u| u.last())
        .ok_or(HELD)?;
    let old_from = old.iter().find(|u| u["id"] == *old_last).ok_or(HELD)?;
    if old_from["exitState"] == from_unit["exitState"] {
        return Err(HELD.into());
    }
    let mut source_refs = from_unit["sourceRefs"].as_array().ok_or(HELD)?.clone();
    for id in first["sourceRefs"].as_array().ok_or(HELD)? {
        if !source_refs.contains(id) {
            source_refs.push(id.clone());
        }
    }
    let affected_units: Vec<_> = units
        .iter()
        .filter(|u| affected.contains(s(u, "id")))
        .cloned()
        .collect();
    let project: String = c
        .query_row("SELECT id FROM project LIMIT 1", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let foundation: Option<String> = c
        .query_row(
            "SELECT data FROM foundations ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(seal(
        json!({"version":PLAN,"policy":"exclusive-refinement-postimage/v1","projectId":project,"workflowId":run,"candidateSha256":expected,"snapshotSha256":crate::pipeline::digest(&snapshot_raw),"rawCandidateSha256":crate::pipeline::digest(&candidate_raw),"originalAdoptionSha256":sha(adopted),"sourceSha256":source["sha256"],"sourceVersion":source["version"],"sourceStateSha256":sha(&source),"assetStateSha256":sha(&json!(assets)),"foundationStateSha256":sha(&json!(foundation)),"unitBindings":bindings(&units),"affectedUnitBindings":bindings(&affected_units),"dependencyGraph":graph,"dependencyGraphSha256":sha(&graph),
        "frontiers":[{"fromUnitId":from,"nextUnitId":frontier,"oldExitState":old_from["exitState"],"newExitState":from_unit["exitState"],"nextInitialState":first["initialState"],"nextBoundaryBefore":first["boundaryBefore"],"fromSourceRefs":from_unit["sourceRefs"],"nextSourceRefs":first["sourceRefs"],"sourceRefs":source_refs,
        "sourceRows":source["rows"].as_array().unwrap().iter().filter(|r|source_refs.contains(&r["id"])).collect::<Vec<_>>(),"sourceScenes":source["scenes"].as_array().unwrap().iter().filter(|s|s["id"]==from_unit["sceneId"]||s["id"]==first["sceneId"]).collect::<Vec<_>>()}],"automaticCompatibility":false,"reviewRequired":true}),
    ))
}
fn validate_review(plan: &Value, review: &Value) -> Result<()> {
    if review.as_object().is_none_or(|m| m.len() != 4)
        || review["version"] != REVIEW
        || review["planSha256"] != plan["sha256"]
        || review["reviewer"] != "authorized_director_review"
    {
        return Err(HELD.into());
    }
    let reviews = review["frontierReviews"]
        .as_array()
        .filter(|a| a.len() == 1)
        .ok_or(HELD)?;
    let decision = &reviews[0];
    let frontier = &plan["frontiers"][0];
    if decision.as_object().is_none_or(|m| m.len() != 5)
        || decision["fromUnitId"] != frontier["fromUnitId"]
        || decision["nextUnitId"] != frontier["nextUnitId"]
        || decision["decision"] != "compatible"
        || s(decision, "reason").trim().is_empty()
    {
        return Err(HELD.into());
    }
    let refs = decision["sourceRefs"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or(HELD)?;
    let mut seen = HashSet::new();
    if refs.iter().any(|id| {
        !id.is_string()
            || !seen.insert(id.to_string())
            || !frontier["sourceRefs"].as_array().unwrap().contains(id)
    }) || ["fromSourceRefs", "nextSourceRefs"].iter().any(|k| {
        !refs
            .iter()
            .any(|id| frontier[k].as_array().unwrap().contains(id))
    }) {
        return Err(HELD.into());
    }
    Ok(())
}
impl Store {
    pub(crate) fn preview_refinement_continuity(
        &self,
        run: &str,
        candidate: &str,
    ) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        let plan = preview(&tx, run, candidate)?;
        tx.rollback().map_err(|e| e.to_string())?;
        Ok(plan)
    }
    pub(crate) fn adopt_refinement_continuity(
        &self,
        run: &str,
        candidate: &str,
        review: &Value,
    ) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (snapshot_raw, raw, _snapshot, candidate_value, mut prepared) = job(&tx, run)?;
        if let Some(receipt) = prepared.get(FIELD) {
            if !valid_seal(receipt)
                || receipt["sourceSha256"] != _snapshot["input"]["source"]["sha256"]
            {
                return Err(HELD.into());
            }
            verify_history_for_input(
                &tx,
                &json!({"source":{"sha256":receipt["sourceSha256"]},"existingUnits":values(&tx,"units")?}),
            )?;
            if receipt["version"] == RECEIPT
                && receipt["candidateSha256"] == candidate
                && receipt["review"] == *review
                && receipt["rawCandidateSha256"] == crate::pipeline::digest(&raw)
                && receipt["snapshotSha256"] == crate::pipeline::digest(&snapshot_raw)
                && receipt["originalAdoptionSha256"] == sha(&prepared["refinementAdoption"])
            {
                return Ok(receipt.clone());
            }
            return Err(HELD.into());
        }
        let plan = preview(&tx, run, candidate)?;
        validate_review(&plan, review)?;
        let mut after = Vec::new();
        for binding in plan["affectedUnitBindings"].as_array().unwrap() {
            let raw: String = tx
                .query_row(
                    "SELECT data FROM units WHERE id=?1",
                    [s(binding, "unitId")],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut unit = parse(raw)?;
            if sha(&unit) != binding["unitSha256"] {
                return Err(HELD.into());
            }
            unit["continuityStale"] = json!(false);
            after.push(save_unit_tx(&tx, unit)?);
        }
        let receipt = seal(
            json!({"version":RECEIPT,"workflowId":run,"candidateSha256":candidate,"plan":plan,"planSha256":plan["sha256"],"sourceSha256":plan["sourceSha256"],"originalAdoptionSha256":plan["originalAdoptionSha256"],"rawCandidateSha256":plan["rawCandidateSha256"],"snapshotSha256":plan["snapshotSha256"],"dependencyGraphSha256":plan["dependencyGraphSha256"],"beforeUnitBindings":plan["affectedUnitBindings"],"afterUnitBindings":bindings(&after),"review":review,"reviewSha256":sha(review),"policy":plan["policy"],"adoptedAt":now(),"generatedContentChanged":false,"modelCalls":0}),
        );
        prepared[FIELD] = receipt.clone();
        let changed=tx.execute("UPDATE jobs SET prepared=?1,updated_at=?2 WHERE id=?3 AND result=?4 AND snapshot=?5 AND state='completed'",params![prepared.to_string(),now(),run,raw,snapshot_raw]).map_err(|e|e.to_string())?;
        if changed != 1 || candidate_value["candidateSha256"] != candidate {
            return Err(HELD.into());
        }
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'storyboard_refinement_continuity_review_adopted',?2,?3)",params![run,receipt.to_string(),now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(receipt)
    }
}

/// Metadata-only versions have their own durable proof, without rewriting the
/// original creative adoption's unitBindings or attaching invented model output.
pub(super) fn verify_history_for_input(c: &Connection, input: &Value) -> Result<()> {
    let mut q=c.prepare("SELECT j.id FROM jobs j WHERE j.kind='workflow' AND j.entity_id='storyboard.refine' AND (json_type(j.prepared,'$.refinementContinuityReview') IS NOT NULL OR EXISTS(SELECT 1 FROM events e WHERE e.job_id=j.id AND e.type='storyboard_refinement_continuity_review_adopted')) AND json_extract(j.snapshot,'$.input.source.sha256')=?1").map_err(|e|e.to_string())?;
    let ids = q
        .query_map([s(&input["source"], "sha256")], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    for run in ids {
        let (snap_raw, raw, _snapshot, candidate, prepared) = job(c, &run)?;
        let receipt = &prepared[FIELD];
        let plan = &receipt["plan"];
        let (event_count,event):(i64,Option<String>)=c.query_row("SELECT COUNT(*),MIN(data) FROM events WHERE job_id=?1 AND type='storyboard_refinement_continuity_review_adopted'",[&run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        if receipt["version"] != RECEIPT
            || !valid_seal(receipt)
            || !valid_seal(plan)
            || event_count != 1
            || event.as_deref() != Some(receipt.to_string().as_str())
            || receipt["workflowId"] != run
            || receipt["candidateSha256"] != candidate["candidateSha256"]
            || receipt["planSha256"] != plan["sha256"]
            || receipt["reviewSha256"] != sha(&receipt["review"])
            || receipt["rawCandidateSha256"] != crate::pipeline::digest(&raw)
            || receipt["snapshotSha256"] != crate::pipeline::digest(&snap_raw)
            || receipt["originalAdoptionSha256"] != sha(&prepared["refinementAdoption"])
            || receipt["sourceSha256"] != input["source"]["sha256"]
            || receipt["beforeUnitBindings"] != plan["affectedUnitBindings"]
        {
            return Err(HELD.into());
        }
        validate_review(plan, &receipt["review"])?;
        let before = receipt["beforeUnitBindings"].as_array().ok_or(HELD)?;
        let after = receipt["afterUnitBindings"].as_array().ok_or(HELD)?;
        let mut seen = HashSet::new();
        if before.len() != after.len() || before.is_empty() {
            return Err(HELD.into());
        }
        for old in before {
            let new = after
                .iter()
                .find(|b| b["unitId"] == old["unitId"])
                .ok_or(HELD)?;
            if !seen.insert(s(old, "unitId"))
                || new["unitVersion"].as_u64()
                    != old["unitVersion"].as_u64().and_then(|v| v.checked_add(1))
            {
                return Err(HELD.into());
            }
            let version = |binding: &Value| -> Result<Value> {
                let raw: String = c
                    .query_row(
                        "SELECT data FROM unit_versions WHERE unit_id=?1 AND version=?2",
                        params![
                            s(binding, "unitId"),
                            binding["unitVersion"].as_i64().ok_or(HELD)?
                        ],
                        |r| r.get(0),
                    )
                    .map_err(|_| HELD)?;
                let value = parse(raw)?;
                if sha(&value) != binding["unitSha256"] {
                    return Err(HELD.into());
                }
                Ok(value)
            };
            let old_unit = version(old)?;
            let new_unit = version(new)?;
            let mut expected = old_unit.clone();
            expected["continuityStale"] = json!(false);
            expected["version"] = new["unitVersion"].clone();
            if old_unit["continuityStale"] != true || new_unit != expected {
                return Err(HELD.into());
            }
            for current in input["existingUnits"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|u| u["id"] == new["unitId"])
            {
                if current["version"].as_u64() < new["unitVersion"].as_u64()
                    || (current["version"] == new["unitVersion"]
                        && sha(current) != new["unitSha256"])
                {
                    return Err(HELD.into());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "workflow_refinement_continuity_tests.rs"]
pub(crate) mod tests;
