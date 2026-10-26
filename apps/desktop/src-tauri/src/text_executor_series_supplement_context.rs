//! Freeze pre-existing identity reservations inside the original workflow.
//! No asset, asset version, media, ordinal allocator or provider is mutated.
use super::*;
use std::collections::BTreeMap;

const FIELD:&str="seriesSupplementIdentities";
const EVENT:&str="series_supplement_identities_frozen";
const HELD:&str="SERIES_SUPPLEMENT_IDENTITIES_HELD: 原作者、身份历史或冻结补料上下文不一致；未派发、未写资产或预留序号";
fn hash(value:&Value)->String {crate::pipeline::digest(&value.to_string())}
const V1:&str="series-asset-supplement/v1";
const V2:&str="series-asset-supplement/v2";
const V3:&str="series-asset-supplement/v3";
fn known_sha(context:&Value,known:&Value)->String {
    if ["series-supplement-identities/v2","series-supplement-identities/v3"].contains(&s(context,"version")) {hash(&crate::pipeline::series::supplement_supply_catalogue(known))}else{hash(known)}
}

fn read_context(c:&rusqlite::Connection,origin:&TextRequest,raw:&Value,known:&Value,desired:&str)->Result<Value> {
    let saved:Option<String>=c.query_row("SELECT value FROM metadata WHERE key='series.placeholder.nextOrdinal'",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let mut next=match saved {Some(value)=>serde_json::from_str::<Value>(&value).map_err(|_|HELD)?.as_u64().filter(|n|*n<=5701).ok_or(HELD)?,None=>0};
    // Read only identity columns. Large prompts, designs and media history are
    // not duplicated into the model's reservation packet.
    let mut q=c.prepare("SELECT json_object('id',json_extract(data,'$.id'),'variantId',json_extract(data,'$.variantId'),'kind',json_extract(data,'$.kind'),'type',json_extract(data,'$.type'),'importName',json_extract(data,'$.importName'),'filename',json_extract(data,'$.filename'),'placeholderOrdinal',json_extract(data,'$.placeholderOrdinal')) FROM assets UNION ALL SELECT json_object('id',json_extract(data,'$.id'),'variantId',json_extract(data,'$.variantId'),'kind',json_extract(data,'$.kind'),'type',json_extract(data,'$.type'),'importName',json_extract(data,'$.importName'),'filename',json_extract(data,'$.filename'),'placeholderOrdinal',json_extract(data,'$.placeholderOrdinal')) FROM asset_versions").map_err(|e|e.to_string())?;
    let rows=q.query_map([],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?;
    let mut reservations=BTreeMap::<String,Value>::new();
    let mut ordinals=BTreeMap::<u64,String>::new();
    let mut identities=BTreeMap::<String,u64>::new();
    for row in rows {
        let value:Value=serde_json::from_str(&row.map_err(|e|e.to_string())?).map_err(|_|HELD)?;
        if value["placeholderOrdinal"].is_null(){continue;}
        let ordinal=value["placeholderOrdinal"].as_u64().filter(|n|*n<=5700).ok_or(HELD)?;
        let id=s(&value,"id");
        if id.is_empty() || ordinals.get(&ordinal).is_some_and(|owner|owner!=id) || identities.get(id).is_some_and(|old|*old!=ordinal){return Err(HELD.into());}
        next=next.max(ordinal+1);ordinals.insert(ordinal,id.into());identities.insert(id.into(),ordinal);
        if let Some(reserved)=crate::pipeline::series::supplement_identity_reservation(&value) {
            if reservations.get(id).is_some_and(|old|old!=&reserved){return Err(format!("{HELD}（{id} 的历史映射冲突）"));}
            reservations.insert(id.into(),reserved);
        }
    }
    let mut context=json!({"version":"series-supplement-identities/v1","sourceSha256":origin.input["sourceSha256"],"episodeId":origin.input["episodeId"],
        "originRequestSha256":crate::pipeline::digest(&serde_json::to_string(origin).map_err(|e|e.to_string())?),"originRawSha256":hash(raw),"knownCatalogSha256":hash(known),
        "nextOrdinal":next,"reservedAssets":reservations.into_values().collect::<Vec<_>>()});
    if [V2,V3].contains(&desired) {
        // Current DB versions are authoritative. Read full designs locally to
        // bind their digests, but put only states/hashes in the model packet.
        let mut q=c.prepare("SELECT id,version,data FROM assets WHERE COALESCE(json_extract(data,'$.deletedAt'),'')='' ORDER BY id").map_err(|e|e.to_string())?;
        let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
        let mut states=Vec::new();
        for row in rows {
            let(id,version,bytes)=row.map_err(|e|e.to_string())?;
            let mut asset:Value=serde_json::from_str(&bytes).map_err(|_|HELD)?;
            if id.is_empty() || asset["id"]!=id || version<=0 {return Err(HELD.into());}
            asset["version"]=json!(version);
            states.push(crate::pipeline::series::supplement_supply_state(&asset,s(&origin.input,"sourceSha256")));
        }
        context["version"]=json!(if desired==V3 {"series-supplement-identities/v3"}else{"series-supplement-identities/v2"});
        context["contractVersion"]=json!(desired);
        context["assetStates"]=json!(states);
        context["knownCatalogSha256"]=json!(known_sha(&context,known));
    }
    crate::pipeline::series::validate_supplement_identity_context(&context,origin,raw,known)?;Ok(context)
}
impl SavedTextExecutor {
    /// The SDK caller has already verified the origin's owned raw. This second
    /// check is read-only so historical ancestor validation never nests writers.
    pub(super) fn verified_series_supplement_identity_context(&self,origin:&TextRequest,raw:&Value,known:&Value,origin_job:&Value)->Result<Value> {
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        let request_sha=crate::pipeline::digest(&serde_json::to_string(origin).map_err(|e|e.to_string())?);
        let c=self.store.conn()?;
        let(parent_text,prepared_text):(String,String)=c.query_row("SELECT snapshot,prepared FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|HELD)?;
        let prepared:Value=serde_json::from_str(&prepared_text).map_err(|_|HELD)?;
        let context=&prepared[FIELD][&request_sha];
        crate::pipeline::series::validate_supplement_identity_context(context,origin,raw,known)?;
        let mut q=c.prepare("SELECT data FROM events WHERE job_id=?1 AND type=?2 AND json_extract(data,'$.originRequestSha256')=?3").map_err(|e|e.to_string())?;
        let proofs=q.query_map(params![run,EVENT,request_sha],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?
            .collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        if proofs.len()!=1{return Err(HELD.into());}
        let proof:Value=serde_json::from_str(&proofs[0]).map_err(|_|HELD)?;
        let expected=json!({"version":"series-supplement-identity-freeze/v1","workflowId":run,"originJobId":origin_job["id"],"originRequestSha256":request_sha,
            "originSnapshotSha256":hash(&origin_job["snapshot"]),"originRawSha256":hash(raw),"knownCatalogSha256":known_sha(context,known),"parentSnapshotSha256":crate::pipeline::digest(&parent_text),
            "context":context,"contextSha256":hash(context)});
        if proof!=expected{return Err(HELD.into());}Ok(context.clone())
    }
    pub(super) fn freeze_series_supplement_identity_context(&self,origin:&TextRequest,raw:&Value,known:&Value,desired_contract:&str)->Result<Value> {
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        if ![V1,V2,V3].contains(&desired_contract) || origin.stage!="seriesAuthor" || origin.skill!="series-author" || origin.input["workflowProfile"]!=crate::asset_graph::PROFILE
            || origin.input.get("seriesAssetSupplement").is_some() || origin.input.get("validationRepair").is_some()
            || raw["draft"]["unresolved"].as_array().is_none_or(Vec::is_empty) {return Err(HELD.into());}
        let request_sha=crate::pipeline::digest(&serde_json::to_string(origin).map_err(|e|e.to_string())?);
        let mut c=self.store.conn()?;
        let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        let(parent_text,prepared_text):(String,Option<String>)=tx.query_row("SELECT snapshot,prepared FROM jobs WHERE id=?1 AND kind='workflow' AND entity_id='storyboard.run'",[run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|HELD)?;
        let parent:Value=serde_json::from_str(&parent_text).map_err(|_|HELD)?;
        if parent["runnerVersion"]!="desktop-runner/v1" || parent["input"]["workflowProfile"]!=crate::asset_graph::PROFILE
            || parent["input"]["settings"]!=self.settings || parent["input"]["source"]["sha256"]!=origin.input["sourceSha256"] {return Err(HELD.into());}
        let snapshot=self.request_snapshot(origin)?;let snapshot_sha=hash(&snapshot);
        let provider=match s(&self.settings,"textProvider") {""|"codex"=>"codex","codex_sdk"=>"codex_sdk","api"=>"api",_=>return Err(HELD.into())};
        let job:String=tx.query_row("SELECT j.id FROM jobs j JOIN workflow_steps w ON w.text_job_id=j.id WHERE w.workflow_id=?1 AND w.node_key=?2 AND w.request_hash=?3 AND w.stage='seriesAuthor' AND w.state='rejected' AND j.kind='text' AND j.provider=?4 AND j.snapshot=?5 AND j.state='rejected' AND j.result=?6 AND w.result=j.result AND w.error=j.error AND j.error IS NOT NULL AND length(j.error)>0",params![run,step_key(origin),snapshot_sha,provider,snapshot.to_string(),raw.to_string()],|r|r.get(0)).map_err(|_|HELD)?;
        let saved=self.sdk_job(&job)?;
        if saved["receipt"]!=json!({"contract":"desktop-runner/v1","validation":"rejected"}) {return Err(HELD.into());}
        // SDK's fast DB replay intentionally returns None: its original files,
        // attempt, receipt and migration proof must all pass sdk_raw instead.
        // Requiring both paths would reject every new SDK supplement before
        // its identity context can be frozen.
        let verified_raw=if provider=="codex_sdk" {
            self.sdk_raw(&saved,&snapshot)?
        } else {
            self.exact_workflow_v1_raw(origin,&snapshot,&snapshot_sha)?.ok_or(HELD)?
        };
        if verified_raw!=*raw {return Err(HELD.into());}
        let mut prepared:Value=prepared_text.as_deref().map(serde_json::from_str).transpose().map_err(|_|HELD)?.unwrap_or_else(||json!({}));
        if prepared.is_null(){prepared=json!({});}
        if !prepared.is_object() || prepared.get(FIELD).is_some_and(|v|!v.is_object()){return Err(HELD.into());}
        let mut q=tx.prepare("SELECT data FROM events WHERE job_id=?1 AND type=?2 AND json_extract(data,'$.originRequestSha256')=?3 ORDER BY id").map_err(|e|e.to_string())?;
        let proofs=q.query_map(params![run,EVENT,request_sha],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?
            .collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;drop(q);
        let mut binding=json!({"version":"series-supplement-identity-freeze/v1","workflowId":run,"originJobId":job,"originRequestSha256":request_sha,
            "originSnapshotSha256":snapshot_sha,"originRawSha256":hash(raw),"knownCatalogSha256":hash(known),"parentSnapshotSha256":crate::pipeline::digest(&parent_text)});
        if let Some(context)=prepared.get(FIELD).and_then(|v|v.get(&request_sha)) {
            crate::pipeline::series::validate_supplement_identity_context(context,origin,raw,known)?;
            binding["knownCatalogSha256"]=json!(known_sha(context,known));
            if proofs.len()!=1 {return Err(HELD.into());}
            let proof:Value=serde_json::from_str(&proofs[0]).map_err(|_|HELD)?;
            let mut expected=binding;expected["context"]=context.clone();expected["contextSha256"]=json!(hash(context));
            if proof!=expected{return Err(HELD.into());}
            // Recovery returns the original reservation snapshot. Do not scan
            // current assets or silently advance the frozen allocation floor.
            return Ok(context.clone());
        }
        if !proofs.is_empty(){return Err(HELD.into());}
        let context=read_context(&tx,origin,raw,known,desired_contract)?;
        binding["knownCatalogSha256"]=json!(known_sha(&context,known));
        if prepared.get(FIELD).is_none(){prepared[FIELD]=json!({});}
        prepared[FIELD][&request_sha]=context.clone();
        let timestamp=now();
        let changed=tx.execute("UPDATE jobs SET prepared=?1,updated_at=?2 WHERE id=?3 AND snapshot=?4 AND prepared IS ?5",params![prepared.to_string(),timestamp,run,parent_text,prepared_text]).map_err(|e|e.to_string())?;
        if changed!=1{return Err(HELD.into());}
        let mut proof=binding;proof["context"]=context.clone();proof["contextSha256"]=json!(hash(&context));
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",params![run,EVENT,proof.to_string(),timestamp]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e|e.to_string())?;Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(tempfile::TempDir,SavedTextExecutor,TextRequest,Value,Value,String) {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("补料身份离线核验","identity-fixture").unwrap();store.ensure_workflow_storage().unwrap();
        let settings=json!({"textProvider":"codex","textModel":"gpt-6.1-sol","reasoning":"medium","textCliExecutable":std::env::current_exe().unwrap(),"textCliVersion":"codex-cli 0.162.0-alpha.2"});
        let origin=TextRequest{stage:"seriesAuthor".into(),skill:"series-author".into(),model:"gpt-6.1-sol".into(),reasoning:"medium".into(),input:json!({"workflowProfile":crate::asset_graph::PROFILE,"sourceSha256":"a".repeat(64),"episodeId":"e1","executionRoleId":"e1/author/0"}),schema:json!({"type":"object"}),rule_snapshot:json!({"synthetic":true})};
        let parent=json!({"runnerVersion":"desktop-runner/v1","input":{"workflowProfile":crate::asset_graph::PROFILE,"source":{"sha256":origin.input["sourceSha256"]},"settings":settings}});
        let run=store.add_job("workflow","storyboard.run","local","local",&parent,"blocked").unwrap();
        let executor=SavedTextExecutor{store,settings,workflow_id:Some(run.clone())};
        let snapshot=executor.request_snapshot(&origin).unwrap();
        let raw=json!({"schemaVersion":"series-episode-candidate/v1","draft":{"unresolved":[{"coordinate":"u1","problem":"缺一项文字设计"}]}});
        let job=executor.store.add_job("text","original-author","codex","local",&snapshot,"rejected").unwrap();
        executor.store.update_job(&job,"rejected",&json!({"result":raw,"error":"SERIES_CREATIVE_DEPENDENCY_UNRESOLVED","receipt":{"contract":"desktop-runner/v1","validation":"rejected"}})).unwrap();
        executor.store.event(&job,"text_received_raw",&json!({"requestHash":hash(&snapshot),"value":raw})).unwrap();
        executor.store.checkpoint(&run,&step_key(&origin),&origin.stage,&hash(&snapshot),"rejected",Some(&job),Some(&raw),Some("SERIES_CREATIVE_DEPENDENCY_UNRESOLVED")).unwrap();
        let active=json!({"id":"active","variantId":"base","kind":"PR","name":"已有道具","importName":"已有道具","filename":"PR-已有道具.png","placeholderOrdinal":3});
        executor.store.save_asset(active.clone()).unwrap();
        (temp,executor,origin,raw,json!([active]),job)
    }
    fn rows(c:&rusqlite::Connection,sql:&str)->Value {
        let mut q=c.prepare(sql).unwrap();json!(q.query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap())
    }
    fn asset_state(executor:&SavedTextExecutor)->Value {
        let c=executor.store.conn().unwrap();json!({"assets":rows(&c,"SELECT data FROM assets ORDER BY id"),"versions":rows(&c,"SELECT data FROM asset_versions ORDER BY rowid"),"ordinal":rows(&c,"SELECT value FROM metadata WHERE key='series.placeholder.nextOrdinal'")})
    }
    fn proof_count(executor:&SavedTextExecutor)->i64 {
        executor.store.conn().unwrap().query_row("SELECT count(*) FROM events WHERE job_id=?1 AND type=?2",params![executor.workflow_id,EVENT],|r|r.get(0)).unwrap()
    }
    #[tokio::test]
    async fn series_supplement_identity_context_cli_freezes_all_history_without_reserving_or_recomputing() {
        let(_temp,executor,origin,raw,known,job)=fixture();
        let historical=json!({"id":"historical","variantId":"base","kind":"PR","name":"旧道具","importName":"旧道具","filename":"PR-旧道具.png","placeholderOrdinal":40});
        executor.store.save_asset(historical.clone()).unwrap();
        executor.store.save_asset(json!({"id":"historical","variantId":"base","kind":"PR","name":"旧道具"})).unwrap();
        executor.store.save_asset(json!({"id":"ordinal-only","variantId":"base","kind":"PR","name":"不完整旧映射","placeholderOrdinal":90})).unwrap();
        executor.store.conn().unwrap().execute("INSERT INTO metadata(key,value) VALUES('series.placeholder.nextOrdinal','12')",[]).unwrap();
        let before=asset_state(&executor);let author=executor.sdk_job(&job).unwrap();
        let context=executor.series_supplement_identity_context(&origin,&raw,&known,V1).await.unwrap();
        assert_eq!(context["nextOrdinal"],91);assert_eq!(context["reservedAssets"].as_array().unwrap().len(),2);
        assert_eq!(context["reservedAssets"][0]["id"],"active");assert_eq!(context["reservedAssets"][1],crate::pipeline::series::supplement_identity_reservation(&historical).unwrap());
        assert_eq!(asset_state(&executor),before,"neither asset versions nor the allocator high-water is written");assert_eq!(executor.sdk_job(&job).unwrap(),author);assert_eq!(proof_count(&executor),1);
        executor.store.save_asset(json!({"id":"later","variantId":"base","kind":"PR","name":"后来素材","placeholderOrdinal":200,"importName":"后来素材","filename":"PR-后来素材.png"})).unwrap();
        let after=asset_state(&executor);
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        assert_eq!(reopened.series_supplement_identity_context(&origin,&raw,&known,V1).await.unwrap(),context,"resume does not silently advance to 201");
        assert_eq!(asset_state(&reopened),after);assert_eq!(proof_count(&reopened),1);assert_eq!(reopened.store.jobs().unwrap().len(),2);
        assert_eq!(reopened.verified_series_supplement_identity_context(&origin,&raw,&known,&author).unwrap(),context);
    }
    #[test]
    fn series_supplement_identity_context_atomic_reservation_creates_one_proof_and_no_assets() {
        let(_temp,executor,origin,raw,known,_job)=fixture();let before=asset_state(&executor);let barrier=Arc::new(std::sync::Barrier::new(2));let mut handles=Vec::new();
        for _ in 0..2 {
            let e=SavedTextExecutor{store:executor.store.clone(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
            let(o,r,k,b)=(origin.clone(),raw.clone(),known.clone(),barrier.clone());
            handles.push(std::thread::spawn(move||{b.wait();e.freeze_series_supplement_identity_context(&o,&r,&k,V1).unwrap()}));
        }
        let first=handles.remove(0).join().unwrap();let second=handles.remove(0).join().unwrap();assert_eq!(first,second);
        assert_eq!(asset_state(&executor),before);assert_eq!(proof_count(&executor),1);assert_eq!(executor.store.jobs().unwrap().len(),2);
    }
    #[test]
    fn series_supplement_identity_context_tampering_or_known_drift_never_rewrites_frozen_context() {
        let(_temp,executor,origin,raw,known,job)=fixture();let context=executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V1).unwrap();
        let mut changed_known=known.clone();changed_known[0]["description"]=json!("改变目录");assert!(executor.freeze_series_supplement_identity_context(&origin,&raw,&changed_known,V1).is_err());
        let run=executor.workflow_id.as_deref().unwrap();let mut prepared=executor.store.job(run).unwrap()["prepared"].clone();
        prepared[FIELD][s(&context,"originRequestSha256")]["nextOrdinal"]=json!(100);
        executor.store.conn().unwrap().execute("UPDATE jobs SET prepared=?2 WHERE id=?1",params![run,prepared.to_string()]).unwrap();
        let before=executor.store.jobs().unwrap();assert!(executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V1).is_err());assert_eq!(executor.store.jobs().unwrap(),before);
        assert!(executor.verified_series_supplement_identity_context(&origin,&raw,&known,&executor.sdk_job(&job).unwrap()).is_err());assert_eq!(proof_count(&executor),1);
    }
    #[test]
    fn series_supplement_identity_context_conflicts_and_unproven_origin_fail_before_any_write() {
        for fault in ["ordinal-conflict","mapping-conflict","filename-conflict","high-water","unknown","accepted","raw","source","known"] {
            let(_temp,executor,origin,mut raw,mut known,job)=fixture();
            match fault {
                "ordinal-conflict"=>{executor.store.save_asset(json!({"id":"other","variantId":"base","kind":"PR","name":"别的道具","placeholderOrdinal":3})).unwrap();},
                "mapping-conflict"=>{let mut changed=known[0].clone();changed["filename"]=json!("PR-改名.png");changed["importName"]=json!("改名");executor.store.save_asset(changed).unwrap();},
                "filename-conflict"=>{let mut changed=known[0].clone();changed["id"]=json!("other");changed["placeholderOrdinal"]=json!(4);executor.store.save_asset(changed).unwrap();},
                "high-water"=>{executor.store.conn().unwrap().execute("INSERT INTO metadata(key,value) VALUES('series.placeholder.nextOrdinal','5702')",[]).unwrap();},
                "unknown"=>executor.store.update_job(&job,"submission_unknown",&json!({})).unwrap(),"accepted"=>executor.store.update_job(&job,"completed",&json!({})).unwrap(),
                "raw"=>raw["extra"]=json!(true),
                "source"=>{let run=executor.workflow_id.as_deref().unwrap();let mut parent=executor.store.job(run).unwrap()["snapshot"].clone();parent["input"]["source"]["sha256"]=json!("other");executor.store.conn().unwrap().execute("UPDATE jobs SET snapshot=?2 WHERE id=?1",params![run,parent.to_string()]).unwrap();},
                "known"=>known[0]["id"]=json!("invented"),_=>unreachable!(),
            }
            let before=asset_state(&executor);let jobs=executor.store.jobs().unwrap();
            assert!(executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V1).is_err(),"{fault}");
            assert_eq!(asset_state(&executor),before,"{fault}");assert_eq!(executor.store.jobs().unwrap(),jobs,"{fault}");assert_eq!(proof_count(&executor),0,"{fault}");
        }
    }
    #[tokio::test]
    async fn series_supplement_identity_context_v2_freezes_current_design_hashes_and_reopens_without_rescanning() {
        let(_temp,executor,origin,raw,known,job)=fixture();
        let mut asset=known[0].clone();
        asset["designRevision"]=json!(4);asset["designStatus"]=json!("adopted");
        asset["adoptedDesign"]=json!({"assetId":"active","variantId":"base","sourceSha256":origin.input["sourceSha256"],"status":"ADOPTED", "provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT",
            "designRevision":4,"full":"PRIVATE_FULL_DESIGN_MUST_NOT_ENTER_THE_RESERVATION_PACKET","adoptionBasis":{"synthetic":true}});
        let saved=executor.store.save_asset(asset).unwrap();let known=json!([saved]);
        let before=asset_state(&executor);let origin_before=executor.sdk_job(&job).unwrap();
        let context=executor.series_supplement_identity_context(&origin,&raw,&known,V2).await.unwrap();
        assert_eq!(context["version"],"series-supplement-identities/v2");assert_eq!(context["contractVersion"],V2);
        assert_eq!(context["knownCatalogSha256"],hash(&crate::pipeline::series::supplement_supply_catalogue(&known)));
        assert_eq!(context["assetStates"],json!([crate::pipeline::series::supplement_supply_state(&saved,s(&origin.input,"sourceSha256"))]));
        assert_eq!(context["assetStates"][0]["designUsable"],true);assert!(!context.to_string().contains("PRIVATE_FULL_DESIGN"));
        assert_eq!(asset_state(&executor),before);assert_eq!(executor.sdk_job(&job).unwrap(),origin_before);
        let mut changed=saved.clone();changed["adoptedDesign"]["full"]=json!("Later complete design");executor.store.save_asset(changed).unwrap();
        executor.store.save_asset(json!({"id":"later","variantId":"base","kind":"PR","name":"后来资产"})).unwrap();
        let changed=asset_state(&executor);
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        assert_eq!(reopened.series_supplement_identity_context(&origin,&raw,&known,V2).await.unwrap(),context);
        assert_eq!(reopened.verified_series_supplement_identity_context(&origin,&raw,&crate::pipeline::series::supplement_supply_catalogue(&known),&origin_before).unwrap(),context,"model's compact catalogue has the same frozen proof");
        assert_eq!(asset_state(&reopened),changed);assert_eq!(proof_count(&reopened),1);
    }
    #[test]
    fn series_supplement_identity_context_new_default_never_upgrades_a_frozen_v1() {
        let(_temp,executor,origin,raw,known,_job)=fixture();
        let context=executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V1).unwrap();
        let before=executor.store.jobs().unwrap();let assets=asset_state(&executor);
        assert_eq!(executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V2).unwrap(),context);
        assert!(context.get("assetStates").is_none());assert!(context.get("contractVersion").is_none());
        assert_eq!(executor.store.jobs().unwrap(),before);assert_eq!(asset_state(&executor),assets);assert_eq!(proof_count(&executor),1);
    }
    #[test]
    fn series_supplement_identity_context_v2_requires_exact_active_scope_and_rejects_frozen_state_tampering() {
        let(_temp,executor,origin,raw,known,_job)=fixture();
        executor.store.save_asset(json!({"id":"extra","variantId":"base","kind":"PR","name":"额外资产"})).unwrap();
        let before=executor.store.jobs().unwrap();
        assert!(executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V2).is_err());
        assert_eq!(executor.store.jobs().unwrap(),before);assert_eq!(proof_count(&executor),0);
        let(_temp,executor,origin,raw,known,job)=fixture();
        let context=executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V2).unwrap();
        let run=executor.workflow_id.as_deref().unwrap();let mut prepared=executor.store.job(run).unwrap()["prepared"].clone();
        prepared[FIELD][s(&context,"originRequestSha256")]["assetStates"][0]["supplyCardSha256"]=json!("0".repeat(64));
        executor.store.conn().unwrap().execute("UPDATE jobs SET prepared=?2 WHERE id=?1",params![run,prepared.to_string()]).unwrap();
        let before=executor.store.jobs().unwrap();
        assert!(executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V2).is_err());
        assert!(executor.verified_series_supplement_identity_context(&origin,&raw,&known,&executor.sdk_job(&job).unwrap()).is_err());
        assert_eq!(executor.store.jobs().unwrap(),before);assert_eq!(proof_count(&executor),1);
    }
    #[test]
    fn series_supplement_identity_context_v2_audio_prop_freezes_without_png_but_visible_prop_cannot() {
        for mode in ["AUDIO","IMAGE"] {
            let(_temp,executor,origin,raw,mut known,job)=fixture();
            let asset=executor.store.save_asset(json!({"id":"system-voice","variantId":"base","kind":"PROP","name":"系统声音",
                "preparation":{"mode":mode,"note":"实际源记录决定媒介类型"}})).unwrap();
            known.as_array_mut().unwrap().push(asset);
            let before_assets=asset_state(&executor);let before_author=executor.sdk_job(&job).unwrap();let before_jobs=executor.store.jobs().unwrap();
            let result=executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V2);
            if mode=="AUDIO" {
                let context=result.unwrap();
                let compact=crate::pipeline::series::supplement_supply_catalogue(&known);
                assert_eq!(compact[1]["preparation"]["mode"],"AUDIO");
                assert_eq!(context["reservedAssets"].as_array().unwrap().len(),1,"voice is not given an image ordinal");
                assert_eq!(context["assetStates"].as_array().unwrap().len(),2,"voice still retains its source identity and frozen state");
                assert_eq!(executor.freeze_series_supplement_identity_context(&origin,&raw,&known,V2).unwrap(),context);
                assert_eq!(executor.verified_series_supplement_identity_context(&origin,&raw,&compact,&before_author).unwrap(),context);
                assert_eq!(proof_count(&executor),1);
            } else {
                assert_eq!(result.unwrap_err(),"SERIES_SUPPLEMENT_ACTIVE_IDENTITY_INCOMPLETE");
                assert_eq!(executor.store.jobs().unwrap(),before_jobs);
                assert_eq!(proof_count(&executor),0);
            }
            assert_eq!(asset_state(&executor),before_assets);assert_eq!(executor.sdk_job(&job).unwrap(),before_author);
            assert_eq!(executor.store.jobs().unwrap().len(),before_jobs.len());
        }
    }
}
