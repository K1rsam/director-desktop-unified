use super::*;
use crate::source_migration::tests::{fixture as source_fixture, candidate};

fn setup() -> (tempfile::TempDir,Store,String) {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path().to_owned()).unwrap();
    store.create("合成显式迁移","migration-fixture").unwrap();store.ensure_workflow_storage().unwrap();
    store.save_source(source_fixture()).unwrap();
    let source=store.source().unwrap();let changed=source["rows"].as_array().unwrap().last().unwrap()["id"].clone();
    let row=&source["rows"][2]["id"];
    let scope=json!({"kind":"GLOBAL","scenes":[],"reason":"合成测试已确认范围"});
    store.save_foundation(json!({"schemaVersion":"foundation/v1","sourceSha256":source["sha256"],"overview":"已有资料","direction":"原方向",
        "episodes":[{"episodeId":source["episodes"][0]["id"],"summary":"甲来了","context":"屋内","sourceRefs":[row]}],
        "assets":[{"id":"person","kind":"CH","name":"甲","aliases":[],"description":"主体制作说明","sourceRefs":[row],"legacyIdentity":{"stableId":"person","kept":true},"variants":[
            {"id":"base","name":"基础形象","description":"基础制作说明","sourceVariantId":null,"sourceRefs":[row],"applicability":[],"scope":scope,"assetRowId":"person-base","legacyVariant":{"kept":true}},
            {"id":"derived","name":"派生形象","description":"派生制作说明","sourceVariantId":"base","sourceRefs":[row],"applicability":[],"scope":scope,"assetRowId":"person-derived"}]}],
        "facts":[{"id":"fact-person","assetId":"person","variantId":null,"attribute":"身份","value":"甲","scope":"STABLE","claim":"SOURCE","sourceRefs":[row],"quotes":["甲"]}],
        "issues":[],"ruleSnapshots":[],"updatedEpisodeIds":[],
        "visualFactBatches":[{"rowIds":[changed],"original":"immutable"}],
        "legacyAssetCatalogue":{"sourceVersion":1,"rows":[{"id":"historical-loc","evidenceRowIds":[changed],"raw":"旧目录原值"}]}})).unwrap();
    store.save_asset(json!({"id":"scene","variantId":"base","kind":"scene","name":"屋内","sourceRefs":[],"design":{"designReceipt":{"basis":{"evidenceRowIds":[changed]}},"frozenContext":{"evidenceRowIds":[changed]}}})).unwrap();
    for (id,variant,parent) in [("person-base","base",None),("person-derived","derived",Some("person-base"))] {
        store.save_asset(json!({"id":id,"identityId":"person","baseAssetId":"person","variantId":variant,"sourceAssetId":parent,"sourceVariantId":parent.map(|_|"base"),"kind":"CH","name":"甲","description":"冻结形象说明","sourceRefs":[row],"scope":scope,
            "design":{"prompt":"原设计正文","designReceipt":{"basis":{"evidenceRowIds":[changed]},"accepted":true},"frozenContext":{"evidenceRowIds":[changed]}},"legacyImageCatalogue":[{"mediaId":"historical-media","status":"adopted"}]})).unwrap();
    }
    let input=json!({"source":source,"foundation":store.foundation().unwrap(),"assets":store.values("assets").unwrap(),"existingUnits":[],"selection":{"episodeIds":source["episodes"].as_array().unwrap().iter().map(|e|e["id"].clone()).collect::<Vec<_>>()},"capabilities":{"minDuration":4,"maxDuration":30,"imageLimit":10}});
    let guard=crate::workflow::guard_from_input(&input);
    let run=store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":input,"guard":guard}),"running").unwrap();
    for index in [0,1] {publish(&store,&run,&input,index);}
    store.update_job(&run,"interrupted",&json!({})).unwrap();
    (dir,store,run)
}

fn publish(store:&Store,run:&str,input:&Value,index:usize) {
    let source=&input["source"];let episode=&source["episodes"][index]["id"];
    let rows:Vec<_>=source["rows"].as_array().unwrap().iter().filter(|r|r["kind"]=="body"&&r["episodeId"]==*episode).cloned().collect();
    let units:Vec<_>=rows.iter().enumerate().map(|(i,row)|{
        let q=&row["dialogue"];let parts=if q.is_object(){json!([{"lineId":q["id"],"quoteStart":q["range"]["start"],"quoteEnd":q["range"]["end"],"text":q["quote"],"speaker":q["speaker"],"channel":q["channel"],"presentation":"画内","voiceStart":0.0,"voiceEnd":1.0,"delivery":"自然说出"}])}else{json!([])};
        json!({"id":format!("U{index}-{i}"),"episodeId":episode,"sceneId":row["sceneId"],"title":"原单元","unitDirection":"回应原句","scene":"屋内","initialState":"甲站在门口。","exitState":"甲仍站在门口。","lighting":"柔光","duration":4,"prompt":"保留的完整镜表","assetRefs":[],"sourceRefs":[row["id"]],"shots":[{"id":format!("S{index}-{i}"),"duration":4,"camera":"近景","picture":"甲看向门口。","lighting":"柔光","sound":"环境声","transition":"切出","sourceRefs":[row["id"]],"contextRefs":[],"dialogueParts":parts}]})
    }).collect();
    let draft=json!({"schemaVersion":"storyboard-draft/v2","units":units,"fantasyRealization":[],"revisionNotes":[],"unresolved":[]});
    let plan=json!({"events":[],"sourceFingerprints":{}});
    let mut finish_hash=String::new();
    for (stage,skill,result) in [("storyboardPlan","episode-emotion-planner",plan.clone()),("storyboardCreate","unit-performance-director",draft.clone()),("storyboardFinish","episode-finishing-editor",json!({"materializedDraft":draft}))] {
        let request=pipeline::TextRequest{stage:stage.into(),skill:skill.into(),model:"synthetic".into(),reasoning:"medium".into(),input:json!({"episodeId":episode,"sourceRows":rows,"contextRows":[],"executionBatch":{"id":format!("batch-{index}")}}),schema:json!({}),rule_snapshot:json!({})};
        let snapshot=serde_json::to_value(&request).unwrap();let hash=pipeline::digest(&snapshot.to_string());
        let id=store.add_job("text",&format!("{stage}:{index}"),"mock","local",&snapshot,"completed").unwrap();
        store.update_job(&id,"completed",&json!({"result":result,"receipt":{"validation":"accepted","contract":"desktop-runner/v1"}})).unwrap();
        store.event(&id,"text_received_raw",&json!({"requestHash":hash,"value":result})).unwrap();
        store.conn().unwrap().execute("INSERT INTO workflow_steps VALUES(?1,?2,?3,?4,'accepted',?5,?6,NULL,?7)",params![run,format!("{stage}:{index}"),stage,hash,id,result.to_string(),now()]).unwrap();
        if stage=="storyboardFinish"{finish_hash=pipeline::digest(&serde_json::to_string(&request).unwrap());}
    }
    let delivery=json!({"schemaVersion":"complete-episode-delivery/v1","status":"running","scope":"episodes","sourceSha256":source["sha256"],"episodeId":episode,"updatedEpisodeIds":[episode],"replacedUnitIds":[],"units":units,"plans":[{"episodeId":episode,"plan":plan}],"fantasyRealization":[],"revisionNotes":[],"unresolved":[],"blocked":[],"finishProvenance":[{"version":"accepted-finish-domain/v1","executionBatchId":format!("batch-{index}"),"canonicalDomainRequestSha256":finish_hash,"draftSha256":pipeline::finish_base_hash(&draft)}],"ruleSnapshots":[]});
    let record=store.job(run).unwrap();
    let guard=record["prepared"].get("publicationGuard").unwrap_or(&record["snapshot"]["guard"]);
    store.publish_storyboard_episode(guard,&delivery,input,run).unwrap();
}

fn protected_snapshot(store:&Store)->Value {
    let c=store.conn().unwrap();let mut out=json!({});
    for table in ["jobs","workflow_steps","assets","asset_versions","media","media_assets","adoptions","uploads","job_media","settings"] {
        let mut q=c.prepare(&format!("SELECT * FROM {table} ORDER BY rowid")).unwrap();let columns=q.column_count();
        let records=q.query_map([],|row|Ok((0..columns).map(|i|format!("{:?}",row.get_ref(i).unwrap())).collect::<Vec<_>>())).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();out[table]=json!(records);
    }out
}

#[test]
fn source_migration_transaction_preserves_receipts_designs_and_reopens_idempotently() {
    let (dir,store,_)=setup();let source=store.source().unwrap();let old_foundation=store.foundation().unwrap();let old_units=store.values("units").unwrap();let protected=protected_snapshot(&store);
    let next=store.save_v10_reparsed_source_if_current(candidate(&source),1).unwrap();
    assert_eq!(next["version"],2);assert_eq!(protected_snapshot(&store),protected);
    let proof=store.verified_source_reparse_migration().unwrap().unwrap();
    assert_eq!(proof["adoptedEpisodes"].as_array().unwrap().len(),2,"{proof}");assert_eq!(proof["blockedEpisodes"],json!([]));
    let mut expected_foundation=old_foundation.clone();expected_foundation["version"]=json!(2);expected_foundation["sourceVersion"]=json!(2);
    assert_eq!(store.foundation().unwrap(),expected_foundation);
    assert_eq!(proof["source"]["oldSourceVersion"],1);assert_eq!(proof["newSourceVersion"],2);
    assert_eq!(proof["newSourceSha256"],pipeline::digest(&next.to_string()));
    for (old,new) in old_units.iter().zip(store.values("units").unwrap()) {assert_eq!(new["version"],2);assert_eq!(new["sourceVersion"],2);assert_eq!(source_migration::adopted_body(old),source_migration::adopted_body(&new));assert!(new.get("sourceMigration").is_none());}
    let reopened=Store::open(dir.path().to_owned()).unwrap();
    assert_eq!(reopened.verified_source_reparse_migration().unwrap(),Some(proof.clone()));
    assert_eq!(reopened.save_v10_reparsed_source_if_current(candidate(&next),2).unwrap(),next);
    assert_eq!(reopened.verified_source_reparse_migration().unwrap(),Some(proof));
    assert!(reopened.save_v10_reparsed_source_if_current(candidate(&source),1).unwrap_err().contains("CAS"));
}

#[test]
fn source_migration_active_unknown_and_stale_nodes_do_not_write() {
    for state in ["running","submitting","submission_unknown","received"] {
        let (_dir,store,_)=setup();let source=store.source().unwrap();store.add_job("text","active","mock","local",&json!({}),state).unwrap();let before=protected_snapshot(&store);
        assert!(store.save_v10_reparsed_source_if_current(candidate(&source),1).unwrap_err().contains("ACTIVE"));
        assert_eq!(store.source().unwrap(),source);assert_eq!(protected_snapshot(&store),before);
    }
}

#[test]
fn source_migration_changed_context_missing_raw_and_user_edits_never_carry_the_episode() {
    for corruption in ["raw","context","unit","finish"] {
        let (_dir,store,run)=setup();let source=store.source().unwrap();let c=store.conn().unwrap();
        if corruption=="raw" {c.execute("DELETE FROM events WHERE type='text_received_raw' AND job_id=(SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1 AND node_key='storyboardPlan:0')",[&run]).unwrap();}
        else if corruption=="unit" {let mut u=store.get("units","U0-0").unwrap();u["title"]=json!("用户已经改过");store.save_unit(u).unwrap();}
        else if corruption=="finish" {c.execute("UPDATE jobs SET prepared=json_set(prepared,'$.publishedEpisodes.'||?2||'.finishProvenance[0].draftSha256','wrong') WHERE id=?1",params![run,source["episodes"][0]["id"].as_str().unwrap()]).unwrap();}
        else {
            let id:String=c.query_row("SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1 AND node_key='storyboardPlan:0'",[&run],|r|r.get(0)).unwrap();
            let mut snapshot=store.job(&id).unwrap()["snapshot"].clone();snapshot["input"]["contextRows"]=json!([source["rows"].as_array().unwrap().last().unwrap()]);
            let hash=pipeline::digest(&snapshot.to_string());c.execute("UPDATE jobs SET snapshot=?1 WHERE id=?2",params![snapshot.to_string(),id]).unwrap();c.execute("UPDATE workflow_steps SET request_hash=?1 WHERE text_job_id=?2",params![hash,id]).unwrap();c.execute("UPDATE events SET data=json_set(data,'$.requestHash',?1) WHERE job_id=?2 AND type='text_received_raw'",params![hash,id]).unwrap();
        }
        let before=store.get("units","U0-0").unwrap();let protected=protected_snapshot(&store);
        store.save_v10_reparsed_source_if_current(candidate(&source),1).unwrap();
        let proof=store.verified_source_reparse_migration().unwrap().unwrap();assert_eq!(proof["blockedEpisodes"].as_array().unwrap().len(),2,"{corruption}: {proof}");
        assert_eq!(proof["adoptedEpisodes"],json!([]));
        assert_eq!(store.get("units","U0-0").unwrap(),before);assert_eq!(protected_snapshot(&store),protected);
    }
}

#[test]
fn source_migration_never_adopts_affected_episode_or_rewrites_historical_evidence() {
    let (_dir,store,run)=setup();let input=store.job(&run).unwrap()["snapshot"]["input"].clone();
    store.update_job(&run,"running",&json!({})).unwrap();publish(&store,&run,&input,2);store.update_job(&run,"interrupted",&json!({})).unwrap();
    let source=store.source().unwrap();let unit=store.get("units","U2-0").unwrap();
    store.save_v10_reparsed_source_if_current(candidate(&source),1).unwrap();
    let proof=store.verified_source_reparse_migration().unwrap().unwrap();assert_eq!(proof["adoptedEpisodes"].as_array().unwrap().len(),2);
    assert!(proof["blockedEpisodes"][0]["reason"].as_str().unwrap().contains("AFFECTED"));assert_eq!(store.get("units","U2-0").unwrap(),unit);
}

#[test]
fn source_migration_proof_detects_tampered_historical_version() {
    let (_dir,store,_)=setup();let source=store.source().unwrap();store.save_v10_reparsed_source_if_current(candidate(&source),1).unwrap();
    store.conn().unwrap().execute("UPDATE unit_versions SET data=json_set(data,'$.title','changed') WHERE unit_id='U0-0' AND version=1",[]).unwrap();
    assert!(store.verified_source_reparse_migration().unwrap_err().contains("PROOF"));
}

#[test]
fn source_migration_proof_detects_tampered_original_raw_after_adoption() {
    let (_dir,store,run)=setup();let source=store.source().unwrap();store.save_v10_reparsed_source_if_current(candidate(&source),1).unwrap();
    store.conn().unwrap().execute("UPDATE jobs SET result='{}' WHERE id=(SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1 AND node_key='storyboardPlan:0')",[run]).unwrap();
    assert!(store.verified_source_reparse_migration().is_err());
}

#[test]
fn source_migration_changed_ordinary_foundation_reference_is_not_silently_rewritten() {
    let (_dir,store,_)=setup();let source=store.source().unwrap();let changed=source["rows"].as_array().unwrap().last().unwrap()["id"].clone();
    let mut foundation=store.foundation().unwrap();foundation["episodes"]=json!([{"episodeId":source["episodes"][2]["id"],"summary":"旧摘要","context":"旧语义","sourceRefs":[changed]}]);store.save_foundation(foundation).unwrap();
    let before=store.foundation().unwrap();let protected=protected_snapshot(&store);
    assert!(store.save_v10_reparsed_source_if_current(candidate(&source),1).unwrap_err().contains("CONSUMED"));
    assert_eq!(store.source().unwrap(),source);assert_eq!(store.foundation().unwrap(),before);assert_eq!(protected_snapshot(&store),protected);
}

#[test]
fn source_reparse_visual_freeze_requires_verified_store_proof_and_preserves_history() {
    let (_dir,store,_)=setup();let old=store.source().unwrap();
    let mut foundation=store.foundation().unwrap();foundation["visualFactBatches"][0]["inputFingerprint"]=json!("synthetic-packet");
    store.save_foundation(foundation).unwrap();
    let mut absent=json!({"source":old,"foundation":store.foundation().unwrap()});
    store.freeze_visual_fact_source_reparse(&mut absent).unwrap();
    assert!(absent.get(pipeline::asset_visual_source_reparse::FIELD).is_none());
    store.save_v10_reparsed_source_if_current(candidate(&old),1).unwrap();
    let before=protected_snapshot(&store);
    let mut input=json!({"source":store.source().unwrap(),"foundation":store.foundation().unwrap()});
    store.freeze_visual_fact_source_reparse(&mut input).unwrap();
    // This fixture changes dialogue metadata only; its A15 row IDs survive,
    // so the verified migration must not invent an unnecessary cache adapter.
    assert!(input.get(pipeline::asset_visual_source_reparse::FIELD).is_none());
    assert_eq!(protected_snapshot(&store),before);
    let mut wrong=input.clone();wrong.as_object_mut().unwrap().remove(pipeline::asset_visual_source_reparse::FIELD);wrong["source"]["version"]=json!(3);
    assert!(store.freeze_visual_fact_source_reparse(&mut wrong).is_err());
    store.conn().unwrap().execute("DELETE FROM events WHERE type='source_reparse_migrated'",[]).unwrap();
    let mut broken=input;broken.as_object_mut().unwrap().remove(pipeline::asset_visual_source_reparse::FIELD);
    assert!(store.freeze_visual_fact_source_reparse(&mut broken).is_err());
}
