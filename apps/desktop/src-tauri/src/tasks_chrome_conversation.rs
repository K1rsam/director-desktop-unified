//! Durable link and recovery intent, independent of immutable prepared requests.
use super::*;
use rusqlite::OptionalExtension;
fn key(job:&str)->String{format!("chrome_conversation:{job}")}
pub(super) fn saved(store:&Store,job:&str)->Result<Value>{
 let text:Option<String>=store.conn()?.query_row("SELECT value FROM metadata WHERE key=?1",[key(job)],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
 text.map(|v|serde_json::from_str(&v).map_err(|e|e.to_string())).unwrap_or(Ok(json!({})))
}

#[cfg(test)]mod tests{
 use super::*;
 fn fixture()->(tempfile::TempDir,Store,PreparedRequest,ProviderConfig,Value){
  let tmp=tempfile::tempdir().unwrap();let store=Store::open(tmp.path().join("project")).unwrap();
  let task="12345678-1234-4234-8234-123456789abc";let hash=crate::pipeline::digest("synthetic image");
  let cfg:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","endpoint":"http://127.0.0.1:53179","chromeBinding":{"profileId":"11111111-1111-4111-8111-111111111111","bundleId":"a".repeat(32),"endpoint":"http://127.0.0.1:53179"}})).unwrap();
  let p:PreparedRequest=serde_json::from_value(json!({"provider":"chrome","request":{"requestId":task,"kind":"image","model":"chatgpt-image","prompt":"synthetic image","ratio":"16:9","resolution":"native","references":[],"options":{}},"capabilityVersion":"fixture","uploadReceipts":[],"preparedAt":"original","wire":{"id":task,"sourceClientId":"worker","message":"synthetic image","correlationVersion":"conversation/v1","browserBinding":cfg.chrome_binding,"metadata":{"promptSha256":hash}}})).unwrap();
  let loc=json!({"version":"chrome-conversation-locator/v1","requestId":task,"sourceClientId":"worker","browserBinding":cfg.chrome_binding,"preparedSha256":crate::pipeline::digest(&json!(p).to_string()),"promptSha256":hash,"conversationId":"22222222-2222-4222-8222-222222222222","url":"https://chatgpt.com/c/22222222-2222-4222-8222-222222222222","userMessageId":"native-user"});
  (tmp,store,p,cfg,loc)
 }
 #[test]fn chrome_conversation_metadata_pins_identity_and_survives_reopen_without_editing_prepared(){
  let(_tmp,store,p,cfg,loc)=fixture();let observe=observer(&store,"job",&p,&cfg);let original=json!(p);
  observe(&json!({"type":"locator","locator":loc})).unwrap();observe(&json!({"type":"locator","locator":loc})).unwrap();
  for field in ["url","requestId","conversationId","sourceClientId","promptSha256","preparedSha256","userMessageId"]{let mut bad=loc.clone();bad[field]=json!("foreign");assert!(observe(&json!({"type":"locator","locator":bad})).is_err());}
  assert_eq!(json!(p),original);let reopened=Store::open(store.root.clone()).unwrap();assert_eq!(saved(&reopened,"job").unwrap()["locator"],loc);
  let n:i64=store.conn().unwrap().query_row("SELECT COUNT(*) FROM events WHERE type='chrome_conversation_receipt'",[],|r|r.get(0)).unwrap();assert_eq!(n,1);
 }
 #[test]fn chrome_conversation_metadata_legacy_user_binding_only_upgrades_once(){
  let(_tmp,store,mut p,cfg,mut loc)=fixture();p.wire.as_object_mut().unwrap().remove("correlationVersion");p.wire["message"]=json!(format!("synthetic image 任务关联码：{}。",p.request.request_id));
  p.wire["metadata"]["promptSha256"]=json!(crate::pipeline::digest(p.wire["message"].as_str().unwrap()));loc["preparedSha256"]=json!(crate::pipeline::digest(&json!(p).to_string()));loc["promptSha256"]=p.wire["metadata"]["promptSha256"].clone();loc["userMessageId"]=Value::Null;
  let observe=observer(&store,"job",&p,&cfg);observe(&json!({"type":"locator","locator":loc})).unwrap();let mut bound=loc.clone();bound["userMessageId"]=json!("native-user");observe(&json!({"type":"locator","locator":bound})).unwrap();
  assert!(observe(&json!({"type":"locator","locator":loc})).is_err());bound["userMessageId"]=json!("other-user");assert!(observe(&json!({"type":"locator","locator":bound})).is_err());
 }
 #[test]fn chrome_conversation_metadata_unknown_open_intent_cannot_be_replaced_or_misbound(){
  let(_tmp,store,p,cfg,loc)=fixture();let observe=observer(&store,"job",&p,&cfg);observe(&json!({"type":"locator","locator":loc})).unwrap();
  let intent=json!({"type":"recovery_intent","requestId":p.request.request_id,"launchToken":"bridge-director-desktop-33333333-3333-4333-8333-333333333333","url":loc["url"],"submitCalled":false});observe(&intent).unwrap();assert!(observe(&intent).is_err());
  let opened=json!({"type":"recovery_opened","requestId":p.request.request_id,"launchToken":intent["launchToken"],"client":{"url":loc["url"],"launchToken":intent["launchToken"]}});
  for pointer in ["/requestId","/launchToken","/client/url","/client/launchToken"]{let mut bad=opened.clone();*bad.pointer_mut(pointer).unwrap()=json!("foreign");assert!(observe(&bad).is_err());assert_eq!(saved(&store,"job").unwrap()["recovery"],intent);}
  observe(&opened).unwrap();assert_eq!(saved(&store,"job").unwrap()["recovery"],opened);
 }
 #[test]fn chrome_conversation_navigation_is_reserved_once_without_replacing_recovery_or_locator(){
  let(_tmp,store,p,cfg,loc)=fixture();let observe=observer(&store,"job",&p,&cfg);
  observe(&json!({"type":"locator","locator":loc})).unwrap();
  let recovery=json!({"type":"recovery_intent","requestId":p.request.request_id,"launchToken":"bridge-director-desktop-33333333-3333-4333-8333-333333333333","url":loc["url"],"submitCalled":false});
  observe(&recovery).unwrap();
  let nav=json!({"type":"recovery_navigation_intent","requestId":p.request.request_id,"launchToken":recovery["launchToken"],"url":loc["url"],"expectedCurrentUrl":"https://chatgpt.com/","sourceClientId":"alias","browserTabId":22,"submitCalled":false});
  for field in ["requestId","launchToken","url","expectedCurrentUrl","sourceClientId","browserTabId","submitCalled"]{let mut bad=nav.clone();bad[field]=json!("");assert!(observe(&bad).is_err(),"{field}");}
  observe(&nav).unwrap();assert!(observe(&nav).is_err());
  let reopened=Store::open(store.root.clone()).unwrap();let state=saved(&reopened,"job").unwrap();
  assert_eq!(state["recovery"],recovery);assert_eq!(state["locator"],loc);assert_eq!(state["recoveryNavigation"],nav);
 }
 #[test]fn chrome_readonly_reader_replacement_is_bounded_preserves_old_receipts_and_never_changes_original_generation(){
  let(_tmp,store,p,cfg,loc)=fixture();let frozen=json!(p);let observe=observer(&store,"job",&p,&cfg);
  observe(&json!({"type":"locator","locator":loc})).unwrap();
  let old=json!({"type":"recovery_intent","requestId":p.request.request_id,"launchToken":"bridge-director-desktop-33333333-3333-4333-8333-333333333333","url":loc["url"],"submitCalled":false});observe(&old).unwrap();
  let b=cfg.chrome_binding.as_ref().unwrap();let connection=json!({"id":"connection","ready":true,"compatible":true,"transport":"extension","browserIncognito":false,
   "browserProfileId":b.profile_id,"extensionBundleId":b.bundle_id,"backgroundEpoch":"background","contentEpoch":"content","serverInstanceId":"server","launchToken":"","url":"https://chatgpt.com/","pageReady":true,"composerReady":true,"chatMainReady":true,"focused":false,"visibilityState":"hidden","activeRequest":null});
  let event=json!({"type":"recovery_reader_replacement_intent","requestId":p.request.request_id,"launchToken":"bridge-director-desktop-44444444-4444-4444-8444-444444444444","previousLaunchToken":old["launchToken"],"previousRecoverySha256":crate::pipeline::digest(&old.to_string()),"url":loc["url"],"readOnly":true,"submitCalled":false,"connectionSource":connection,"nativeOriginalProof":{"requestId":p.request.request_id,"sourceClientId":"connection","inventory":{"sourceRole":"connection"}}});
  for pointer in ["/requestId","/previousLaunchToken","/previousRecoverySha256","/url","/readOnly","/submitCalled","/connectionSource/pageReady","/nativeOriginalProof/sourceClientId","/nativeOriginalProof/inventory/sourceRole"]{
   let mut bad=event.clone();*bad.pointer_mut(pointer).unwrap()=json!("foreign");assert!(observe(&bad).is_err(),"{pointer}");assert_eq!(saved(&store,"job").unwrap()["recovery"],old);
  }
  observe(&event).unwrap();let state=saved(&store,"job").unwrap();assert_eq!(state["locator"],loc);assert_eq!(state["retiredReadOnlyRecoveries"][0]["recovery"],old);assert_eq!(state["recovery"],event);assert!(state["recoveryNavigation"].is_null());assert_eq!(json!(p),frozen);
  assert!(observe(&event).is_err());let reopened=Store::open(store.root.clone()).unwrap();assert_eq!(saved(&reopened,"job").unwrap(),state);
 }
 #[test]fn chrome_conversation_metadata_unsent_upload_proof_is_durable_immutable_and_bound_to_original_prepared(){
  let(_tmp,store,_,cfg,_)=fixture();let base=crate::chrome_terminal::tests::upload_fixture();let mut value=base["prepared"].clone();value["wire"]["browserBinding"]=json!(cfg.chrome_binding.as_ref());
  value["capabilityVersion"]=json!("fixture");value["uploadReceipts"]=json!([]);value["preparedAt"]=json!("original");for(key,val)in [("prompt",json!("synthetic image")),("ratio",json!("16:9")),("resolution",json!("native")),("references",json!([])),("options",json!({}))]{value["request"][key]=val;}
  let p:PreparedRequest=serde_json::from_value(value).unwrap();let mut proof=base["result"]["raw"]["uploadNotSubmitted"].clone();proof["preparedSha256"]=json!(crate::pipeline::digest(&json!(p).to_string()));let observe=observer(&store,"job",&p,&cfg);
  let event=json!({"type":"upload_not_submitted","proof":proof});observe(&event).unwrap();observe(&event).unwrap();let reopened=Store::open(store.root.clone()).unwrap();assert_eq!(saved(&reopened,"job").unwrap()["uploadNotSubmitted"],proof);
  let mut wrong=event.clone();wrong["proof"]["preparedSha256"]=json!("changed");assert!(observe(&wrong).is_err());assert_eq!(saved(&store,"job").unwrap()["uploadNotSubmitted"],proof);
 }
}
pub(super) fn observer<'a>(store:&'a Store,job:&'a str,p:&'a PreparedRequest,cfg:&'a ProviderConfig)->impl Fn(&Value)->ProviderResult<()>+'a{
 move |event|{
  let fail=||ProviderError::new("CHROME_LINK_PERSISTENCE_HELD","原会话链接或恢复预约无法完整保存，未开页或重新生成");
  let mut conn=store.conn().map_err(|_|fail())?;let tx=conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|fail())?;
  let raw:Option<String>=tx.query_row("SELECT value FROM metadata WHERE key=?1",[key(job)],|r|r.get(0)).optional().map_err(|_|fail())?;
  let mut state=raw.map(|v|serde_json::from_str::<Value>(&v)).transpose().map_err(|_|fail())?.unwrap_or(json!({}));
  if event["type"]=="upload_not_submitted"{
   let proof=&event["proof"];
   if cfg.kind!="chrome"||p.wire["browserBinding"]!=json!(cfg.chrome_binding.as_ref())||!crate::chrome_terminal::upload_not_submitted_valid(proof,&json!(p),&p.request.request_id)
    ||!state["uploadNotSubmitted"].is_null()&&state["uploadNotSubmitted"]!=*proof{return Err(fail());}
   if state["uploadNotSubmitted"]==*proof{return Ok(());}state["uploadNotSubmitted"]=proof.clone();
  }else if event["type"]=="locator"{
   let loc=&event["locator"];
   let mut same=state["locator"].clone();if same["userMessageId"].is_null()&&!same.is_null(){same["userMessageId"]=loc["userMessageId"].clone();}
   if !providers::chrome_locator_valid(cfg,p,loc)||!state["locator"].is_null()&&same!=*loc{return Err(fail());}
   if state["locator"]==*loc{return Ok(());}state["locator"]=loc.clone();
  }else if event["type"]=="recovery_reader_replacement_intent"{
   let binding=cfg.chrome_binding.as_ref().ok_or_else(fail)?;
   let old=s(&state["recovery"],"launchToken");let new=s(event,"launchToken");
   if !providers::chrome_locator_valid(cfg,p,&state["locator"])||event["requestId"]!=p.request.request_id
    ||state["recovery"].is_null()||!state["retiredReadOnlyRecoveries"].as_array().is_none_or(Vec::is_empty)
    ||event["previousLaunchToken"]!=old||event["previousRecoverySha256"]!=crate::pipeline::digest(&state["recovery"].to_string())
    ||new==old||new==format!("bridge-director-desktop-{}",p.request.request_id)
    ||!new.strip_prefix("bridge-director-desktop-").is_some_and(|id|uuid::Uuid::parse_str(id).is_ok_and(|v|v.get_version_num()==4&&v.to_string()==id))
    ||event["url"]!=state["locator"]["url"]||event["readOnly"]!=true||event["submitCalled"]!=false
    ||!crate::chrome_setup::connection_page(&event["connectionSource"],binding)
    ||event["nativeOriginalProof"]["requestId"]!=p.request.request_id||event["nativeOriginalProof"]["inventory"]["sourceRole"]!="connection"
    ||event["nativeOriginalProof"]["sourceClientId"]!=event["connectionSource"]["id"] {return Err(fail());}
   state["retiredReadOnlyRecoveries"]=json!([{"recovery":state["recovery"],"navigation":state["recoveryNavigation"],"reason":"previous_reader_unresolved","replacementIntent":event}]);
   state["recovery"]=event.clone();state["recoveryNavigation"]=Value::Null;
  }else if event["type"]=="recovery_navigation_intent"{
   if !providers::chrome_locator_valid(cfg,p,&state["locator"])||event["requestId"]!=p.request.request_id
    ||state["recovery"].is_null()||!state["recoveryNavigation"].is_null()
    ||event["launchToken"]!=state["recovery"]["launchToken"]||event["url"]!=state["locator"]["url"]
    ||event["expectedCurrentUrl"]!="https://chatgpt.com/"||event["submitCalled"]!=false
    ||!event["sourceClientId"].as_str().is_some_and(|s|!s.is_empty())
    ||!event["browserTabId"].as_i64().is_some_and(|id|id>0){return Err(fail());}
   state["recoveryNavigation"]=event.clone();
  }else if ["recovery_intent","recovery_opened"].contains(&s(event,"type")){
   if !providers::chrome_locator_valid(cfg,p,&state["locator"])||event["requestId"]!=p.request.request_id{return Err(fail());}
   if event["type"]=="recovery_intent"&&!state["recovery"].is_null(){return Err(fail());}
   if event["type"]=="recovery_opened"&&(event["client"]["url"]!=state["locator"]["url"]||state["recovery"].is_null()||event["launchToken"]!=state["recovery"]["launchToken"]||event["client"]["launchToken"]!=event["launchToken"]){return Err(fail());}
   state["recovery"]=event.clone();
  }else{return Err(fail());}
  tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key(job),state.to_string()]).map_err(|_|fail())?;
  tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'chrome_conversation_receipt',?2,?3)",rusqlite::params![job,event.to_string(),now()]).map_err(|_|fail())?;
  tx.commit().map_err(|_|fail())?;Ok(())
 }
}
