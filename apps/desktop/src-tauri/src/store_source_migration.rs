//! Atomic explicit source adoption. All preimages are read under the same
//! IMMEDIATE transaction; no task snapshot, receipt, budget or media is edited.
use crate::{pipeline, source_migration::{self, Plan}, store::{now, parse, s, Result, Store}};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn rows(connection: &Connection, sql: &str) -> Result<Vec<Value>> {
    let mut statement = connection.prepare(sql).map_err(|e|e.to_string())?;
    let result = statement.query_map([], |r|r.get::<_,String>(0)).map_err(|e|e.to_string())?
        .map(|r|parse(r.map_err(|e|e.to_string())?)).collect();
    result
}

fn assert_idle(connection: &Connection) -> Result<()> {
    let busy: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE
        (kind='workflow' AND state NOT IN ('completed','failed','interrupted','cancelled','closed_by_user','partially_blocked','blocked','waiting_decision'))
        OR (kind IN ('text','image','video') AND state NOT IN ('completed','downloaded','rejected','failed','cancelled','closed_by_user','stale'))
        OR state IN ('unknown','submission_unknown')
        OR json_extract(receipt,'$.state') IN ('unknown','submission_unknown','cancel_requested')
        OR json_extract(prepared,'$.submissionUnknown')=1)", [], |r|r.get(0)).map_err(|e|e.to_string())?;
    if busy { return Err("SOURCE_MIGRATION_ACTIVE: 原执行尚未终态或提交结果未知，未重解析/覆盖".into()); }
    let has_steps: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_steps')", [], |r|r.get(0)).map_err(|e|e.to_string())?;
    if has_steps {
        let active: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE state NOT IN ('accepted','rejected','failed','cancelled','interrupted','blocked'))", [], |r|r.get(0)).map_err(|e|e.to_string())?;
        if active { return Err("SOURCE_MIGRATION_ACTIVE: 原流程仍持有未终态节点".into()); }
    }
    Ok(())
}

struct AcceptedStep { request: pipeline::TextRequest, result: Value, proof: Value }

fn accepted_steps(connection: &Connection, run: &str, episode: &str, plan: &Plan) -> Result<Vec<AcceptedStep>> {
    let mut query = connection.prepare("SELECT j.id,w.stage,w.request_hash,j.snapshot,w.result,j.receipt,j.result,
        (SELECT e.data FROM events e WHERE e.job_id=j.id AND e.type='text_received_raw' ORDER BY e.id DESC LIMIT 1)
        FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id
        WHERE w.workflow_id=?1 AND w.state='accepted' AND j.kind='text' AND j.state='completed'
        AND w.error IS NULL AND j.error IS NULL
        AND COALESCE(json_extract(j.snapshot,'$.input.episodeId'),json_extract(j.snapshot,'$.input.episode.id'))=?2
        ORDER BY w.rowid").map_err(|e|e.to_string())?;
    let records = query.query_map(params![run,episode], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,Option<String>>(7)?)))
        .map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let mut accepted = Vec::new();
    let mut seen = BTreeSet::new();
    for (id,stage,hash,snapshot,output,receipt,raw,event) in records {
        if !seen.insert(hash.clone()) { continue; }
        let snapshot_value = parse(snapshot.clone())?;
        let request: pipeline::TextRequest = serde_json::from_value(snapshot_value.clone()).map_err(|e|e.to_string())?;
        let receipt = parse(receipt)?;
        let raw = parse(raw.ok_or("SOURCE_MIGRATION_RAW: 原回包缺失")?)?;
        let event = parse(event.ok_or("SOURCE_MIGRATION_RAW: 原接收事件缺失")?)?;
        let skill = match stage.as_str() {
            "storyboardPlan" => "episode-emotion-planner",
            "storyboardCreate" => "unit-performance-director",
            "storyboardFinish" => "episode-finishing-editor",
            _ => return Err("SOURCE_MIGRATION_NODE: 尚不迁移额外修订/精修链".into()),
        };
        if pipeline::digest(&snapshot) != hash || request.stage != stage || request.skill != skill
            || receipt["validation"] != "accepted" || receipt["contract"] != "desktop-runner/v1"
            || receipt.get("acceptedRequestHash").is_some_and(|h|h != &json!(hash))
            || event["requestHash"] != hash || event["value"].is_null() || event["value"] != raw {
            return Err("SOURCE_MIGRATION_NODE: 原请求、原回包与验收绑定不一致".into());
        }
        plan.require_unconsumed(&snapshot_value)?;
        let result = parse(output)?;
        plan.require_unconsumed(&result)?;
        let result = if result["materializedDraft"].is_object() { result["materializedDraft"].clone() } else {result};
        let proof = json!({"jobId":id,"stage":stage,"requestSha256":hash,
            "canonicalDomainRequestSha256":pipeline::digest(&serde_json::to_string(&request).map_err(|e|e.to_string())?),
            "acceptedResultSha256":pipeline::digest(&result.to_string()),"rawResultSha256":pipeline::digest(&raw.to_string())});
        accepted.push(AcceptedStep {request,result,proof});
    }
    for stage in ["storyboardPlan","storyboardCreate","storyboardFinish"] {
        if !accepted.iter().any(|step|step.request.stage==stage) {
            return Err(format!("SOURCE_MIGRATION_NODE: 缺少完整三岗采用证据 {stage}"));
        }
    }
    Ok(accepted)
}

fn verify_episode(connection: &Connection, plan: &Plan, previous: &Value,
    foundation: &Value, assets: &[Value], episode: &str, units: &[Value]) -> Result<Value> {
    if !plan.episode_eligible(episode) {
        return Err("SOURCE_MIGRATION_AFFECTED: 本集解释或前集连续性变化，旧稿保留待修订".into());
    }
    if units.iter().any(|u|u["sourceVersion"]!=previous["version"] || u["continuityStale"]==true || u.get("preparedPackage").is_some()) {
        return Err("SOURCE_MIGRATION_UNIT: 单元来源、连续性或素材准备状态不适合沿用".into());
    }
    let mut query = connection.prepare("SELECT id,snapshot,prepared,result FROM jobs WHERE kind='workflow' AND entity_id='storyboard.run' AND result IS NOT NULL AND json_extract(snapshot,'$.input.source.sha256')=?1 ORDER BY rowid DESC").map_err(|e|e.to_string())?;
    let records = query.query_map([&plan.old.sha256],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?)))
        .map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let mut origins = Vec::new();
    for (run,snapshot,prepared,result) in records {
        let snapshot = parse(snapshot)?;
        let prepared = prepared.map(parse).transpose()?.unwrap_or(Value::Null);
        let output = parse(result)?;
        let publication = &prepared["publishedEpisodes"][episode];
        if publication["version"] != "complete-episode-publication/v1" {continue;}
        let published: Vec<_> = output["units"].as_array().into_iter().flatten().filter(|u|s(u,"episodeId")==episode).collect();
        if published.len()!=units.len() || !units.iter().all(|u|published.iter().any(|p|p["id"]==u["id"] && source_migration::adopted_body(p)==source_migration::adopted_body(u))) {continue;}
        origins.push((run,snapshot,prepared,output));
    }
    if origins.len()!=1 {return Err("SOURCE_MIGRATION_ORIGIN: 当前整集稿没有唯一逐值相等的正式逐集采用来源".into());}
    let (run,snapshot,prepared,output) = &origins[0];
    let publication = &prepared["publishedEpisodes"][episode];
    let unit_ids: BTreeSet<_> = units.iter().map(|u|s(u,"id")).collect();
    let published_ids: BTreeSet<_> = publication["unitIds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if snapshot["runnerVersion"]!="desktop-runner/v1" || snapshot["input"]["source"]!=*previous
        || snapshot["input"]["selection"]["unitIds"].as_array().is_some_and(|x|!x.is_empty())
        || publication["episodeId"]!=episode || publication["sourceVersion"]!=previous["version"]
        || publication["sourceSha256"]!=plan.old.sha256 || unit_ids!=published_ids
        || publication["episodeDeliverySha256"].as_str().is_none_or(|s|s.len()!=64)
        || !output["updatedEpisodeIds"].as_array().is_some_and(|ids|ids.iter().any(|id|id==episode)) {
        return Err("SOURCE_MIGRATION_PUBLICATION: 正式单集采用身份不完整".into());
    }
    let events: i64 = connection.query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='episode_published' AND data=?2",params![run,publication.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if events!=1 {return Err("SOURCE_MIGRATION_PUBLICATION: 单集采用事件不唯一或缺失".into());}
    let strip_counters = |v:&Value| {let mut v=v.clone();if let Some(o)=v.as_object_mut(){o.remove("version");o.remove("sourceVersion");}v};
    if strip_counters(&snapshot["input"]["foundation"])!=strip_counters(foundation)
        || snapshot["input"]["assets"].as_array()!=Some(&assets.to_vec()) {
        return Err("SOURCE_MIGRATION_MATERIAL: 已采用稿的基础/资产材料已改变，不能仅按原文沿用".into());
    }
    let steps = accepted_steps(connection,run,episode,plan)?;
    let plans: Vec<_> = output["plans"].as_array().into_iter().flatten().filter(|p|p["episodeId"]==episode).collect();
    if plans.len()!=1 || steps.iter().filter(|step|step.request.stage=="storyboardPlan" && step.result==plans[0]["plan"]).count()!=1 {
        return Err("SOURCE_MIGRATION_PLAN: 原整集规划没有唯一验收节点".into());
    }
    let provenance = publication["finishProvenance"].as_array().filter(|p|!p.is_empty()).ok_or("SOURCE_MIGRATION_FINISH: 缺少整编采用证明")?;
    let mut finished_units = Vec::new();
    for binding in provenance {
        let matches: Vec<_> = steps.iter().filter(|step|step.request.stage=="storyboardFinish"
            && binding["version"]=="accepted-finish-domain/v1"
            && binding["canonicalDomainRequestSha256"]==step.proof["canonicalDomainRequestSha256"]
            && binding["draftSha256"]==pipeline::finish_base_hash(&step.result)).collect();
        if matches.len()!=1 {return Err("SOURCE_MIGRATION_FINISH: 整编采用证明没有唯一原稿，保留待核对".into());}
        finished_units.extend(matches[0].result["units"].as_array().ok_or("整编原稿缺少单元")?.iter());
    }
    if finished_units.len()!=units.len() || units.iter().any(|unit|finished_units.iter().filter(|f|f["id"]==unit["id"] && source_migration::adopted_body(f)==source_migration::adopted_body(unit)).count()!=1) {
        return Err("SOURCE_MIGRATION_FINISH: 正式单元与原整编接受稿不等".into());
    }
    let draft = json!({"schemaVersion":"storyboard-draft/v2","units":units});
    plan.require_unconsumed(&draft)?;
    pipeline::validate_storyboard(&plan.next,&draft,assets,&snapshot["input"]["capabilities"])?;
    let expected: BTreeSet<_> = plan.next.rows.iter().filter(|r|r.kind=="body" && r.episode_id.as_deref()==Some(episode)).map(|r|r.id.as_str()).collect();
    let covered: BTreeSet<_> = units.iter().flat_map(|u|u["sourceRefs"].as_array().into_iter().flatten().filter_map(Value::as_str)).collect();
    if covered!=expected {return Err("SOURCE_MIGRATION_COVERAGE: 整集正文覆盖不完整".into());}
    Ok(json!({"episodeId":episode,"originWorkflowId":run,"publicationSha256":pipeline::digest(&publication.to_string()),
        "workflowSnapshotSha256":pipeline::digest(&snapshot.to_string()),"validatedDraftSha256":pipeline::digest(&draft.to_string()),
        "acceptedNodes":steps.iter().map(|step|&step.proof).collect::<Vec<_>>(),"newModelCalls":0}))
}

impl Store {
    pub fn save_v10_reparsed_source_if_current(&self, mut candidate: Value, expected: i64) -> Result<Value> {
        let mut connection = self.conn()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        let raw: String = tx.query_row("SELECT data FROM sources ORDER BY version DESC LIMIT 1",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        let previous = parse(raw)?;
        let current = previous["version"].as_i64().ok_or("当前来源版本缺失")?;
        if current!=expected {return Err("SOURCE_MIGRATION_CAS: 原文版本已更新，未迁移".into());}
        candidate["version"] = json!(current);
        if candidate==previous {return Ok(previous);}
        assert_idle(&tx)?;
        let plan = source_migration::plan(&previous,&candidate)?;
        let assets = rows(&tx,"SELECT data FROM assets ORDER BY rowid")?;
        let active_assets: Vec<_> = assets.iter().filter(|a|!crate::store::asset_deleted(a)).cloned().collect();
        for asset in &active_assets {plan.require_unconsumed(&pipeline::asset_design_contract::asset_semantics(asset))?;}
        let foundations = rows(&tx,"SELECT data FROM foundations ORDER BY version DESC LIMIT 1")?;
        let foundation = foundations.first().cloned().unwrap_or(Value::Null);
        if !foundation.is_null() {
            if foundation["sourceVersion"]!=previous["version"] {return Err("SOURCE_MIGRATION_FOUNDATION: 制作基础不是当前原文版本".into());}
            let typed: pipeline::FoundationProposal = serde_json::from_value(foundation.clone()).map_err(|e|e.to_string())?;
            plan.require_unconsumed(&serde_json::to_value(&typed).map_err(|e|e.to_string())?)?;
            pipeline::validate_foundation(&plan.next,&typed)?;
        }
        let units = rows(&tx,"SELECT data FROM units WHERE COALESCE(json_extract(data,'$.retired'),0)<>1 ORDER BY rowid")?;
        let mut accepted = Vec::new();
        let mut blocked = Vec::new();
        let mut allowed = BTreeSet::new();
        let mut predecessor_unverified = false;
        for episode in &plan.old.episodes {
            let local: Vec<_> = units.iter().filter(|u|u["episodeId"]==episode.id).cloned().collect();
            if local.is_empty() {predecessor_unverified=true;continue;}
            let verified = if predecessor_unverified {
                Err("SOURCE_MIGRATION_CONTINUITY: 前集没有已证明的当前采用版本，后续旧稿保留待核对".into())
            } else {verify_episode(&tx,&plan,&previous,&foundation,&active_assets,&episode.id,&local)};
            match verified {
                Ok(proof) => {allowed.insert(episode.id.clone());accepted.push(proof);},
                Err(reason) => {predecessor_unverified=true;blocked.push(json!({"episodeId":episode.id,"reason":reason,"oldUnitsPreserved":true}));},
            }
        }
        let new_version = current.checked_add(1).ok_or("来源版本溢出")?;
        candidate["version"] = json!(new_version);
        tx.execute("INSERT INTO sources VALUES(?1,?2,?3,?4,?5)",params![new_version,plan.next.text,plan.next.sha256,candidate.to_string(),now()]).map_err(|e|e.to_string())?;
        let mut foundation_proof = Value::Null;
        let mut next_foundation_version = 0;
        if !foundation.is_null() {
            let old_version = foundation["version"].as_i64().ok_or("制作基础版本缺失")?;
            next_foundation_version = old_version.checked_add(1).ok_or("制作基础版本溢出")?;
            let mut next = foundation.clone();
            next["version"] = json!(next_foundation_version);
            next["sourceVersion"] = json!(new_version);
            tx.execute("INSERT INTO foundations VALUES(?1,?2,?3,?4)",params![next_foundation_version,new_version,next.to_string(),now()]).map_err(|e|e.to_string())?;
            foundation_proof=json!({"oldVersion":old_version,"newVersion":next_foundation_version,"oldSha256":pipeline::digest(&foundation.to_string()),"newSha256":pipeline::digest(&next.to_string())});
        }
        let mut bindings = Vec::new();
        for unit in units.iter().filter(|u|allowed.contains(s(u,"episodeId"))) {
            let mut next = unit.clone();
            next["expectedVersion"] = unit["version"].clone();
            next["sourceVersion"] = json!(new_version);
            next["foundationVersion"] = json!(next_foundation_version);
            let next = crate::store::save_unit_tx(&tx,next)?;
            if source_migration::adopted_body(&next)!=source_migration::adopted_body(unit) {return Err("SOURCE_MIGRATION_CONTENT: 派生版本意外改变创作正文，已回滚".into());}
            bindings.push(json!({"unitId":unit["id"],"episodeId":unit["episodeId"],"oldVersion":unit["version"],"newVersion":next["version"],"oldSha256":pipeline::digest(&unit.to_string()),"newSha256":pipeline::digest(&next.to_string())}));
        }
        let mut proof=json!({"version":source_migration::VERSION,"source":plan.evidence,
            "newSourceVersion":new_version,"newSourceSha256":pipeline::digest(&candidate.to_string()),
            "foundation":foundation_proof,"assetCatalogueSha256":pipeline::digest(&json!(assets).to_string()),
            "adoptedEpisodes":accepted,"blockedEpisodes":blocked,"unitBindings":bindings,
            "oldRequestsReceiptsBudgetsAndMediaPreserved":true,"assetDesignsUnchanged":true,"newModelCalls":0});
        proof["proofSha256"]=json!(pipeline::digest(&proof.to_string()));
        tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2)",params![format!("source-reparse-migration:{new_version}"),proof.to_string()]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES('','source_reparse_migrated',?1,?2)",params![proof.to_string(),now()]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())?;
        Ok(candidate)
    }

    /// Independent proof lookup. This is not a design-receipt validity bypass.
    /// Historical source/foundation/unit rows are rehashed; later ordinary edits
    /// remain ordinary edits and are not silently adopted by this proof.
    pub fn verified_source_reparse_migration(&self) -> Result<Option<Value>> {
        let mut connection=self.conn()?;
        let tx=connection.transaction().map_err(|e|e.to_string())?;
        let source:Option<String>=tx.query_row("SELECT data FROM sources ORDER BY version DESC LIMIT 1",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        // A project containing only manually entered image prompts has no
        // source migration to prove. Missing rows are valid here; malformed
        // saved sources and mismatched migration receipts still fail below.
        let Some(source)=source else{return Ok(None);};
        let source:Value=parse(source)?;
        let version=source["version"].as_i64().ok_or("来源版本缺失")?;
        let raw:Option<String>=tx.query_row("SELECT value FROM metadata WHERE key=?1",[format!("source-reparse-migration:{version}")],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let Some(raw)=raw else{return Ok(None);};
        let proof=parse(raw)?;
        let mut payload=proof.clone();payload.as_object_mut().ok_or("迁移证明不是对象")?.remove("proofSha256");
        if proof["version"]!=source_migration::VERSION || proof["proofSha256"]!=pipeline::digest(&payload.to_string())
            || proof["newSourceVersion"]!=version || proof["newSourceSha256"]!=pipeline::digest(&source.to_string()) {
            return Err("SOURCE_MIGRATION_PROOF: 当前来源与独立迁移证明不符".into());
        }
        let old_version=proof["source"]["oldSourceVersion"].as_i64().ok_or("迁移证明缺旧来源版本")?;
        let old=parse(tx.query_row("SELECT data FROM sources WHERE version=?1",[old_version],|r|r.get(0)).map_err(|e|e.to_string())?)?;
        let plan=source_migration::plan(&old,&source)?;
        if plan.evidence!=proof["source"] {return Err("SOURCE_MIGRATION_PROOF: 旧来源或范围映射已变化".into());}
        for adopted in proof["adoptedEpisodes"].as_array().ok_or("迁移证明缺采用集")? {
            let run=s(adopted,"originWorkflowId");let episode=s(adopted,"episodeId");
            let (snapshot,prepared):(String,Option<String>)=tx.query_row("SELECT snapshot,prepared FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
            let snapshot=parse(snapshot)?;let prepared=prepared.map(parse).transpose()?.unwrap_or(Value::Null);
            let publication=&prepared["publishedEpisodes"][episode];
            if adopted["workflowSnapshotSha256"]!=pipeline::digest(&snapshot.to_string())
                || adopted["publicationSha256"]!=pipeline::digest(&publication.to_string()) {
                return Err("SOURCE_MIGRATION_PROOF: 原采用请求或发布回执已变化".into());
            }
            let steps=accepted_steps(&tx,run,episode,&plan)?;
            if adopted["acceptedNodes"]!=json!(steps.iter().map(|step|&step.proof).collect::<Vec<_>>()) {
                return Err("SOURCE_MIGRATION_PROOF: 原验收节点已变化".into());
            }
            let count:i64=tx.query_row("SELECT COUNT(*) FROM events WHERE job_id=?1 AND type='episode_published' AND data=?2",params![run,publication.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
            if count!=1 {return Err("SOURCE_MIGRATION_PROOF: 原单集采用事件缺失或不唯一".into());}
        }
        for (table,bindings) in [("foundations",vec![proof["foundation"].clone()]),("unit_versions",proof["unitBindings"].as_array().cloned().ok_or("迁移证明缺单元绑定")?)] {
            for binding in bindings.into_iter().filter(|b|!b.is_null()) {
                for prefix in ["old","new"] {
                    let version=binding[format!("{prefix}Version")].as_i64().ok_or("迁移版本绑定缺失")?;
                    let raw:String=if table=="foundations" {tx.query_row("SELECT data FROM foundations WHERE version=?1",[version],|r|r.get(0))}
                        else {tx.query_row("SELECT data FROM unit_versions WHERE unit_id=?1 AND version=?2",params![s(&binding,"unitId"),version],|r|r.get(0))}.map_err(|e|e.to_string())?;
                    if binding[format!("{prefix}Sha256")]!=pipeline::digest(&parse(raw)?.to_string()) {return Err("SOURCE_MIGRATION_PROOF: 历史采用版本与保存摘要不符".into());}
                }
            }
        }
        let count:i64=tx.query_row("SELECT COUNT(*) FROM events WHERE type='source_reparse_migrated' AND data=?1",[proof.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if count!=1 {return Err("SOURCE_MIGRATION_PROOF: 迁移事件缺失或不唯一".into());}
        Ok(Some(proof))
    }

    /// Freeze engineering evidence only at a new Stage2 request boundary.
    /// Never rewrite historical requests, accepted caches or repair budgets.
    pub fn freeze_visual_fact_source_reparse(&self, input: &mut Value) -> Result<()> {
        let Some(proof) = self.verified_source_reparse_migration()? else { return Ok(()); };
        if proof["newSourceSha256"] != pipeline::digest(&input["source"].to_string()) {
            return Err("SOURCE_MIGRATION_PROOF: Stage2 来源与已验证迁移不一致".into());
        }
        let old_version = proof["source"]["oldSourceVersion"].as_i64().ok_or("迁移证明缺旧来源版本")?;
        let old = parse(self.conn()?.query_row("SELECT data FROM sources WHERE version=?1",[old_version],|r|r.get(0)).map_err(|e|e.to_string())?)?;
        pipeline::asset_visual_source_reparse::freeze(input, old, proof)
    }
}

#[cfg(test)]
#[path="store_source_migration_tests.rs"]
mod tests;
