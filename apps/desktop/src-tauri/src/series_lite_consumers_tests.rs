//! Synthetic adoption/Store tests; content review is deliberately not claimed.
use super::*;
use crate::store::Store;
use artifacts::{Adopt, Artifact, Inputs, Kind, StoredArtifact, Verdict, Verifier};

struct StorageVerifier(Capability);
impl Verifier for StorageVerifier {
    fn id(&self)->&str {"synthetic-consumer-storage/v1"}
    fn capability(&self)->Capability {self.0}
    fn verify(&self,c:&StoredArtifact,e:&StoredArtifact)->Result<Verdict> {
        let accepted=match self.0 {
            Capability::ContentReview=>e.artifact.payload["reviewedQuote"]==c.artifact.payload["text"]
                && e.artifact.payload["verdict"]=="approved" && e.artifact.payload["syntheticOnly"]==true,
            _=>c.artifact.payload==e.artifact.payload,
        };
        Ok(Verdict{accepted,findings:json!({"syntheticOnly":true,"contentQuality":"NOT_PROVEN"})})
    }
}
struct Entry {episode:String,unit:Value,adoption:Adoption,record:Value,native:String}
struct Fixture {_temp:tempfile::TempDir,store:Store,source:Value}
fn fixture()->Fixture {
    let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("synthetic consumers","synthetic").unwrap();
    let source=store.save_source(json!(crate::pipeline::prepare_source("第1集\n场景：门口\n甲：走。\n第2集\n场景：走廊\n甲：到。\n第3集\n场景：室内\n甲：坐。\n").unwrap())).unwrap();
    Fixture{_temp:temp,store,source}
}
fn entry(f:&Fixture,index:usize)->Entry {
    let episode=s(&f.source["episodes"][index],"id").to_owned();
    let unit=f.store.save_unit(json!({"id":format!("unit-{index}"),"episodeId":episode,"title":format!("合成稿{index}"),"prompt":format!("合成完整正文{index}"),"assetRefs":[],"sourceRefs":[],"duration":4})).unwrap();
    let native=json!({"syntheticOnly":true,"episodes":[{"number":index+1,"draft":{"units":[{"id":format!("native-{index}"),"picture":"合成边界，不代表真实内容采用"}]}}]}).to_string();
    let inputs=Inputs::from([("source".into(),s(&f.source,"sha256").into())]);
    let mut c=f.store.conn().unwrap();let tx=c.transaction().unwrap();artifacts::install(&tx).unwrap();
    let raw=Artifact{id:format!("raw-{index}"),identity:format!("synthetic/{index}"),kind:Kind::Raw,contract_version:"synthetic/v1".into(),source_sha256:s(&f.source,"sha256").into(),inputs:inputs.clone(),producer:"synthetic-storage".into(),author_attempt:format!("attempt-{index}"),parents:vec![],payload:json!({"text":native})};
    artifacts::put(&tx,&raw).unwrap();let mut candidate=raw.clone();candidate.id=format!("candidate-{index}");candidate.kind=Kind::Candidate;candidate.parents=vec![raw.id.clone()];let stored_candidate=artifacts::put(&tx,&candidate).unwrap();
    let mut review=raw.clone();review.id=format!("independent-review-{index}");review.kind=Kind::ReviewRaw;
    review.author_attempt=format!("reviewer-attempt-{index}");review.parents=vec![candidate.id.clone()];
    review.payload=json!({"candidateSha256":stored_candidate.payload_sha256,"inputSha256":stored_candidate.input_sha256,
        "authorAttempt":candidate.author_attempt,"reviewerAttempt":review.author_attempt,"reviewedQuote":candidate.payload["text"],"verdict":"approved","syntheticOnly":true,"contentQuality":"NOT_PROVEN"});
    artifacts::put(&tx,&review).unwrap();
    let caps=[Capability::Structure,Capability::SourceCoverage,Capability::ContentReview];let mut validations=Vec::new();
    for (i,cap) in caps.iter().enumerate() {
        let id=format!("validation-{index}-{i}");let evidence=if *cap==Capability::ContentReview {&review.id}else{&raw.id};
        artifacts::validate(&tx,&id,&candidate.id,evidence,&StorageVerifier(*cap)).unwrap();validations.push(id);
    }
    let adoption=artifacts::adopt(&tx,Adopt{id:&format!("adoption-{index}"),candidate_id:&candidate.id,identity:&candidate.identity,expected_previous:0,source_sha256:&candidate.source_sha256,contract_version:&candidate.contract_version,current_inputs:&inputs,required:&caps,validation_ids:&validations}).unwrap();
    let record=json!({"version":"desktop-lite-delivery/v1","episodeId":episode,"sourceSha256":f.source["sha256"],"nativeSha256":crate::series_lite_edit::sha256(native.as_bytes()),"candidateArtifactId":candidate.id,"adoption":adoption,"finalDeliveryReady":true,"units":[unit]});
    tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2)",params![format!("lite.delivery:{episode}"),record.to_string()]).unwrap();tx.commit().unwrap();
    Entry{episode,unit,adoption,record,native}
}
fn boundary(e:&Entry)->Value {json!({"kind":"supplied","episodeId":e.episode,"adoptionIdentity":e.adoption.identity,"version":e.adoption.version,"nativeSha256":e.record["nativeSha256"],"previousBundleUtf8":e.native})}
fn register_entry(f:&Fixture,e:&mut Entry,bound:&Value) {
    let mut c=f.store.conn().unwrap();let tx=c.transaction().unwrap();register(&tx,&e.episode,bound,&e.adoption,&[e.unit.clone()]).unwrap();
    e.record["boundaryConsumersVersion"]=json!(VERSION);
    tx.execute("UPDATE metadata SET value=?1 WHERE key=?2",params![e.record.to_string(),format!("lite.delivery:{}",e.episode)]).unwrap();tx.commit().unwrap();
}

#[test]
fn boundary_checks_actual_adoption_and_current_text_while_allowing_media_only_storage_updates() {
    let f=fixture();let e=entry(&f,0);let bound=boundary(&e);
    verify_boundary(&f.store.conn().unwrap(),&bound,s(&f.source,"sha256")).unwrap();
    let mut media=e.unit.clone();media["mediaPlan"]=json!({"ready":true});media["status"]=json!("ready");let media=f.store.save_unit(media).unwrap();
    assert_ne!(media["version"],e.unit["version"]);verify_boundary(&f.store.conn().unwrap(),&bound,s(&f.source,"sha256")).unwrap();
    let mut manual=media;manual["prompt"]=json!("人工新稿");f.store.save_unit(manual).unwrap();
    assert!(verify_boundary(&f.store.conn().unwrap(),&bound,s(&f.source,"sha256")).unwrap_err().contains("PREVIOUS_CURRENT_TEXT_CHANGED"));
    assert_eq!(f.store.conn().unwrap().query_row("SELECT COUNT(*) FROM workflow_adoptions_v1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}

#[test]
fn registered_real_predecessor_chain_is_checked_transitively_without_rewriting_successors() {
    let f=fixture();let mut a=entry(&f,0);let mut b=entry(&f,1);let mut c=entry(&f,2);
    register_entry(&f,&mut a,&json!({"kind":"firstInScope"}));register_entry(&f,&mut b,&boundary(&a));register_entry(&f,&mut c,&boundary(&b));
    verify_consumers(&f.store.conn().unwrap(),&c.record).unwrap();
    let b_before=f.store.get("units",s(&b.unit,"id")).unwrap();let c_before=f.store.get("units",s(&c.unit,"id")).unwrap();
    let mut manual=a.unit.clone();manual["prompt"]=json!("首集发生真实人工改稿");f.store.save_unit(manual).unwrap();
    assert!(verify_consumers(&f.store.conn().unwrap(),&c.record).unwrap_err().contains("PREVIOUS_CURRENT_TEXT_CHANGED"));
    assert_eq!(f.store.get("units",s(&b.unit,"id")).unwrap(),b_before);assert_eq!(f.store.get("units",s(&c.unit,"id")).unwrap(),c_before);
}

#[test]
fn consumers_are_immutable_idempotent_and_cannot_drop_their_registration_marker() {
    let f=fixture();let a=entry(&f,0);let mut b=entry(&f,1);let bound=boundary(&a);register_entry(&f,&mut b,&bound);
    let mut c=f.store.conn().unwrap();let tx=c.transaction().unwrap();register(&tx,&b.episode,&bound,&b.adoption,&[b.unit.clone()]).unwrap();tx.commit().unwrap();
    assert_eq!(f.store.conn().unwrap().query_row("SELECT COUNT(*) FROM series_lite_boundary_consumers_v1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert!(f.store.conn().unwrap().execute("UPDATE series_lite_boundary_consumers_v1 SET proof='{}'",[]).unwrap_err().to_string().contains("LITE_BOUNDARY_CONSUMER_IMMUTABLE"));
    let mut missing=b.record.clone();missing.as_object_mut().unwrap().remove("boundaryConsumersVersion");
    assert!(verify_consumers(&f.store.conn().unwrap(),&missing).unwrap_err().contains("CONSUMER_BINDING_CHANGED"));
    assert_eq!(f.store.get("units",s(&b.unit,"id")).unwrap(),b.unit);
}

#[test]
fn missing_provisional_or_changed_predecessors_do_not_become_formal_consumption_by_name() {
    let f=fixture();let a=entry(&f,0);let b=entry(&f,1);
    let mut c=f.store.conn().unwrap();let tx=c.transaction().unwrap();
    assert!(register(&tx,&b.episode,&json!({"kind":"missingPrevious","episodeId":a.episode}),&b.adoption,&[b.unit.clone()]).unwrap_err().contains("PREVIOUS_NOT_SUPPLIED"));tx.rollback().unwrap();
    let mut false_record=a.record.clone();false_record["finalDeliveryReady"]=json!(false);
    f.store.conn().unwrap().execute("UPDATE metadata SET value=?1 WHERE key=?2",params![false_record.to_string(),format!("lite.delivery:{}",a.episode)]).unwrap();
    assert!(verify_boundary(&f.store.conn().unwrap(),&boundary(&a),s(&f.source,"sha256")).unwrap_err().contains("PREVIOUS_CURRENT_ADOPTION_CHANGED"));
    let mut wrong=boundary(&a);wrong["episodeId"]=json!("same-title-different-identity");
    assert!(verify_boundary(&f.store.conn().unwrap(),&wrong,s(&f.source,"sha256")).is_err());
    verify_consumers(&f.store.conn().unwrap(),&b.record).unwrap();
}

#[test]
fn late_boundary_is_an_immutable_direct_predecessor_transition_and_keeps_the_original_missing_pin() {
    let f=fixture();let a=entry(&f,0);let bound=boundary(&a);
    let original=json!({"kind":"missingPrevious","episodeId":a.episode});let original_pin=digest(&original.to_string());
    let target=s(&f.source["episodes"][1],"id");
    let before=f.store.get("units",s(&a.unit,"id")).unwrap();
    let saved=crate::series_lite_late_boundary::freeze(&f.store,"synthetic-run",target,&f.store.root,&original,&bound,s(&f.source,"sha256")).unwrap();
    assert_eq!(saved["originalBoundarySha256"],original_pin);
    assert_eq!(saved["boundary"],bound);
    assert_eq!(crate::series_lite_late_boundary::freeze(&f.store,"synthetic-run",target,&f.store.root,&original,&bound,s(&f.source,"sha256")).unwrap(),saved);
    assert_eq!(f.store.get("units",s(&a.unit,"id")).unwrap(),before);
    assert_eq!(crate::series_lite_late_boundary::load(&f.store.conn().unwrap(),"synthetic-run",target).unwrap(),Some(saved));
    assert!(f.store.conn().unwrap().execute("UPDATE series_lite_late_boundary_v1 SET data='{}'",[]).is_err());
    let tail=crate::series_lite_edit::parse(&std::fs::read(f.store.root.join("late-previous-boundary.json")).unwrap()).unwrap();
    assert_eq!(tail.get("id").unwrap().string().unwrap(),"native-0");
}

#[test]
fn late_boundary_cannot_bind_a_non_adjacent_or_provisional_draft_and_does_not_create_a_model_job() {
    let f=fixture();let a=entry(&f,0);let bound=boundary(&a);let original=json!({"kind":"missingPrevious","episodeId":a.episode});
    assert!(crate::series_lite_late_boundary::freeze(&f.store,"synthetic-run",s(&f.source["episodes"][2],"id"),&f.store.root,&original,&bound,s(&f.source,"sha256")).unwrap_err().contains("NOT_DIRECT_PREDECESSOR"));
    let mut provisional=a.record.clone();provisional["finalDeliveryReady"]=json!(false);
    f.store.conn().unwrap().execute("UPDATE metadata SET value=?1 WHERE key=?2",params![provisional.to_string(),format!("lite.delivery:{}",a.episode)]).unwrap();
    assert!(crate::series_lite_late_boundary::freeze(&f.store,"synthetic-run",s(&f.source["episodes"][1],"id"),&f.store.root,&original,&bound,s(&f.source,"sha256")).is_err());
    assert_eq!(f.store.conn().unwrap().query_row("SELECT COUNT(*) FROM jobs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(crate::series_lite_late_boundary::load(&f.store.conn().unwrap(),"synthetic-run",s(&f.source["episodes"][1],"id")).unwrap(),None);
}
