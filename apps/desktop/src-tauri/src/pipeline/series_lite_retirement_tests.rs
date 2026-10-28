//! Real Store/adoption/version-history contract tests using synthetic content.
//! These verifier fixtures prove storage behavior, never content-quality PASS.
use super::*;
use crate::workflow_artifacts::{Adopt, Artifact, Capability, Inputs, Kind, StoredArtifact, Verdict, Verifier};

struct SyntheticVerifier(Capability);
impl Verifier for SyntheticVerifier {
    fn id(&self)->&str {"synthetic-storage-only/v1"}
    fn capability(&self)->Capability {self.0}
    fn verify(&self,c:&StoredArtifact,e:&StoredArtifact)->Result<Verdict> {
        let accepted=match self.0 {
            Capability::ContentReview=>e.artifact.payload["reviewedQuote"]==c.artifact.payload["text"]
                && e.artifact.payload["verdict"]=="approved" && e.artifact.payload["syntheticOnly"]==true,
            _=>c.artifact.payload==e.artifact.payload,
        };
        Ok(Verdict{accepted,findings:json!({"syntheticOnly":true,"contentReview":"NOT_PROVEN"})})
    }
}
struct Fixture {
    _temp:tempfile::TempDir,
    store:Store,
    input:Value,
    snapshot:Value,
    run:String,
    episode:String,
    old:Vec<Value>,
    boundary:Value,
    adoption:artifacts::Adoption,
    projected:Vec<Value>,
    media:Value,
    historical_job:String,
}
fn fixture(complete_adoption:bool)->Fixture {
    let temp=tempfile::tempdir().unwrap();
    let store=Store::open(temp.path().join("project")).unwrap();
    store.create("synthetic retirement","synthetic").unwrap();
    let source=store.save_source(json!(super::super::prepare_source("第1集\n场景：门口\n甲：走吧。\n第2集\n场景：走廊\n甲：到了。\n").unwrap())).unwrap();
    let episode=s(&source["episodes"][0],"id").to_owned();
    let mut old=Vec::new();
    for id in ["old-adopted-a","old-adopted-b"] {
        old.push(store.save_unit(json!({"id":id,"episodeId":episode,"prompt":format!("原采用稿 {id}"),"title":id,"duration":4,"sourceRefs":[],"assetRefs":[],"exitState":"原末态"})).unwrap());
    }
    let boundary=store.save_unit(json!({"id":"outside-protected","episodeId":source["episodes"][1]["id"],"prompt":"范围外的保护稿","duration":4,"assetRefs":[]})).unwrap();
    let image=temp.path().join("retained.png");
    image::RgbImage::from_pixel(2,2,image::Rgb([10,30,50])).save(&image).unwrap();
    let media=store.import_media(&image,"image",None,None).unwrap();
    let historical_job=store.add_job("video",s(&old[0],"id"),"synthetic","synthetic",&json!({"unitVersion":old[0]["version"],"mediaId":media["id"]}),"completed").unwrap();
    let mut input=json!({"source":source,"selection":{"episodeIds":[episode]},"existingUnits":store.values("units").unwrap(),"assets":[],"previousBoundary":boundary});
    mark_input(&mut input).unwrap();
    let guard=crate::series_lite_guard::from_input(&input);
    let snapshot=json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","input":input,"guard":guard});
    let run=store.add_job("workflow","storyboard.run","local","local",&snapshot,"running").unwrap();
    let native="{\"syntheticStorageFixture\":true}";
    let inputs=Inputs::from([("source".into(),s(&source,"sha256").to_owned())]);
    let mut c=store.conn().unwrap();let tx=c.transaction().unwrap();artifacts::install(&tx).unwrap();
    let raw=Artifact{id:"replacement-raw".into(),identity:"synthetic-episode/final".into(),kind:Kind::Raw,contract_version:"synthetic/v1".into(),source_sha256:s(&source,"sha256").into(),inputs:inputs.clone(),producer:"synthetic-storage-only".into(),author_attempt:"synthetic".into(),parents:vec![],payload:json!({"text":native})};
    artifacts::put(&tx,&raw).unwrap();
    let mut candidate=raw.clone();candidate.id="replacement-candidate".into();candidate.kind=Kind::Candidate;candidate.parents=vec![raw.id.clone()];let stored_candidate=artifacts::put(&tx,&candidate).unwrap();
    let mut review=raw.clone();review.id="replacement-independent-review".into();review.kind=Kind::ReviewRaw;
    review.author_attempt="synthetic-independent-reviewer".into();review.parents=vec![candidate.id.clone()];
    review.payload=json!({"candidateSha256":stored_candidate.payload_sha256,"inputSha256":stored_candidate.input_sha256,
        "authorAttempt":candidate.author_attempt,"reviewerAttempt":review.author_attempt,"reviewedQuote":candidate.payload["text"],"verdict":"approved","syntheticOnly":true,"contentQuality":"NOT_PROVEN"});
    if complete_adoption {artifacts::put(&tx,&review).unwrap();}
    let capabilities=if complete_adoption {vec![Capability::Structure,Capability::SourceCoverage,Capability::ContentReview]}else{vec![Capability::Structure]};
    let mut validations=Vec::new();
    for (i,cap) in capabilities.iter().enumerate() {
        let id=format!("storage-validation-{i}");
        let evidence=if *cap==Capability::ContentReview {&review.id}else{&raw.id};
        artifacts::validate(&tx,&id,&candidate.id,evidence,&SyntheticVerifier(*cap)).unwrap();validations.push(id);
    }
    let adoption=artifacts::adopt(&tx,Adopt{id:"replacement-adoption",candidate_id:&candidate.id,identity:&candidate.identity,expected_previous:0,source_sha256:&candidate.source_sha256,contract_version:&candidate.contract_version,current_inputs:&inputs,required:&capabilities,validation_ids:&validations}).unwrap();
    tx.commit().unwrap();
    let projected=vec![json!({"id":"new-adopted","episodeId":episode,"prompt":"新整集正式采用稿","title":"新稿","duration":4,"assetRefs":[],"sourceRefs":[],"exitState":"新末态","seriesLite":{"nativeUnitId":"U1","nativeSha256":sha256(native.as_bytes()),"adoptionIdentity":adoption.identity,"adoptionVersion":adoption.version,"finalDeliveryReady":true}})];
    Fixture{_temp:temp,store,input,snapshot,run,episode,old,boundary,adoption,projected,media,historical_job}
}
fn publish(f:&Fixture,units:&[Value],ready:bool)->Result<Vec<Value>> {
    let mut c=f.store.conn()?;let tx=c.transaction().map_err(|e|e.to_string())?;
    let saved=publish_units(&tx,&f.run,&f.episode,&f.snapshot["guard"],units,&f.adoption,ready)?;
    if ready {
        let record=json!({"version":DELIVERY,"run":f.run,"episodeId":f.episode,"sourceSha256":f.input["source"]["sha256"],"nativeSha256":units[0]["seriesLite"]["nativeSha256"],"adoption":f.adoption,"units":saved,"projectionPublished":true,"finalDeliveryReady":true});
        tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2)",params![format!("lite.delivery:{}",f.episode),record.to_string()]).map_err(|e|e.to_string())?;
    }
    tx.commit().map_err(|e|e.to_string())?;Ok(saved)
}
fn version_count(f:&Fixture)->i64 {f.store.conn().unwrap().query_row("SELECT COUNT(*) FROM unit_versions",[],|r|r.get(0)).unwrap()}

#[test]
fn final_adoption_retires_only_frozen_episode_and_refresh_preserves_other_guard_entries() {
    let f=fixture(true);let original_job=f.store.job(&f.historical_job).unwrap();let media=f.store.values("media").unwrap();
    let media_readback=f.store.get("media",s(&f.media,"id")).unwrap();
    let media_path=f.store.root.join(s(&f.media,"relativePath"));
    let media_bytes=std::fs::read(&media_path).unwrap();
    assert_eq!(sha256(&media_bytes),s(&f.media,"sha256"));
    let saved=publish(&f,&f.projected,true).unwrap();assert_eq!(saved.len(),1);
    for old in &f.old {
        let current=f.store.get("units",s(old,"id")).unwrap();
        assert_eq!(current["retired"],true);assert_eq!(current["prompt"],old["prompt"]);
        let raw:String=f.store.conn().unwrap().query_row("SELECT data FROM unit_versions WHERE unit_id=?1 AND version=1",[s(old,"id")],|r|r.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(),*old);
    }
    assert_eq!(f.store.get("units",s(&f.boundary,"id")).unwrap(),f.boundary);
    assert_eq!(f.store.values("media").unwrap(),media);
    let after_media=f.store.get("media",s(&f.media,"id")).unwrap();
    assert_eq!(after_media,media_readback,"compare the same normalized Store read contract");
    assert_eq!(after_media["sha256"],f.media["sha256"]);
    assert_eq!(std::fs::read(&media_path).unwrap(),media_bytes);
    assert_eq!(f.store.job(&f.historical_job).unwrap(),original_job);
    refresh_guard(&f.store,&f.run,&f.snapshot).unwrap();
    let guard=f.store.job(&f.run).unwrap()["prepared"]["publicationGuard"].clone();
    for old in &f.old {assert!(guard["units"].get(s(old,"id")).is_none());}
    assert_eq!(guard["units"]["new-adopted"],saved[0]["version"]);
    assert_eq!(guard["units"][s(&f.boundary,"id")],f.boundary["version"]);
    assert_eq!(f.store.job(&f.run).unwrap()["snapshot"],f.snapshot);
    let count=version_count(&f);refresh_guard(&f.store,&f.run,&f.snapshot).unwrap();assert_eq!(version_count(&f),count);
    let mut manual=f.boundary.clone();manual["prompt"]=json!("无关受保护边界发生人工变更");f.store.save_unit(manual).unwrap();
    assert!(refresh_guard(&f.store,&f.run,&f.snapshot).is_err(),"retirement must not clear unrelated guards");
}

#[test]
fn provisional_candidate_and_incomplete_adoption_cannot_replace_active_or_historic_units() {
    for complete in [false,true] {
        let f=fixture(complete);let units=f.store.values("units").unwrap();let versions=version_count(&f);let job=f.store.job(&f.historical_job).unwrap();
        assert!(publish(&f,&f.projected,false).unwrap().is_empty());
        assert_eq!(f.store.values("units").unwrap(),units);assert_eq!(version_count(&f),versions);assert_eq!(f.store.job(&f.historical_job).unwrap(),job);
        if !complete {
            assert!(publish(&f,&f.projected,true).unwrap_err().contains("FINAL_ADOPTION_REQUIRED"));
            assert_eq!(f.store.values("units").unwrap(),units);assert_eq!(version_count(&f),versions);
        }
    }
}

#[test]
fn same_stable_id_gets_a_new_version_while_only_removed_ids_are_retired() {
    let f=fixture(true);let mut units=f.projected.clone();units[0]["id"]=f.old[0]["id"].clone();
    let saved=publish(&f,&units,true).unwrap();assert_eq!(saved[0]["version"],2);
    assert_eq!(f.store.get("units",s(&f.old[1],"id")).unwrap()["retired"],true);
    refresh_guard(&f.store,&f.run,&f.snapshot).unwrap();
    let guard=f.store.job(&f.run).unwrap()["prepared"]["publicationGuard"].clone();
    assert_eq!(guard["units"][s(&f.old[0],"id")],2);assert!(guard["units"].get(s(&f.old[1],"id")).is_none());
}

#[test]
fn foreign_addition_and_restored_retirement_are_not_mistaken_for_own_publication() {
    let f=fixture(true);publish(&f,&f.projected,true).unwrap();refresh_guard(&f.store,&f.run,&f.snapshot).unwrap();
    let prepared=f.store.job(&f.run).unwrap()["prepared"].clone();
    f.store.save_unit(json!({"id":"manual-added","episodeId":f.episode,"prompt":"人工新增正文","assetRefs":[]})).unwrap();
    assert!(refresh_guard(&f.store,&f.run,&f.snapshot).is_err());assert_eq!(f.store.job(&f.run).unwrap()["prepared"],prepared);
    let mut restored=f.store.get("units",s(&f.old[0],"id")).unwrap();restored["retired"]=json!(false);f.store.save_unit(restored).unwrap();
    assert!(refresh_guard(&f.store,&f.run,&f.snapshot).unwrap_err().contains("RETIRED_UNIT_CHANGED"));
    assert_eq!(f.store.job(&f.run).unwrap()["prepared"],prepared);
}

#[test]
fn frozen_text_edit_blocks_retirement_and_replacement_proof_cannot_be_rewritten() {
    let f=fixture(true);let mut edited=f.old[0].clone();edited["prompt"]=json!("用户新草稿");let edited=f.store.save_unit(edited).unwrap();
    assert!(publish(&f,&f.projected,true).is_err());assert_eq!(f.store.get("units",s(&edited,"id")).unwrap(),edited);
    assert!(f.store.get("units","new-adopted").is_err());
    let f=fixture(true);publish(&f,&f.projected,true).unwrap();
    assert!(f.store.conn().unwrap().execute("UPDATE series_lite_unit_replacements_v1 SET proof='{}'",[]).unwrap_err().to_string().contains("LITE_REPLACEMENT_IMMUTABLE"));
    assert!(f.store.conn().unwrap().execute("DELETE FROM series_lite_unit_replacements_v1",[]).is_err());
}

#[test]
fn projection_refuses_a_delivery_built_from_different_native_bytes() {
    let f=fixture(true);
    let delivery=crate::series_lite_bundle::Delivery{bundle_sha256:"f".repeat(64),markdown:String::new(),markdown_sha256:String::new(),manifest_json:String::new(),manifest_sha256:String::new(),projection:json!({}),gate_report:json!({}),gate_validator:String::new(),validation:String::new()};
    assert!(project_units(&f.episode,b"{}",&delivery,&json!({}),&json!({}),&f.adoption,true).unwrap_err().contains("LITE_PROJECTION_NATIVE_DELIVERY_MISMATCH"));
}
