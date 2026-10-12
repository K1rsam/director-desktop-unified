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
}
pub(super) fn observer<'a>(store:&'a Store,job:&'a str,p:&'a PreparedRequest,cfg:&'a ProviderConfig)->impl Fn(&Value)->ProviderResult<()>+'a{
 move |event|{
  let fail=||ProviderError::new("CHROME_LINK_PERSISTENCE_HELD","原会话链接或恢复预约无法完整保存，未开页或重新生成");
  let mut conn=store.conn().map_err(|_|fail())?;let tx=conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|fail())?;
  let raw:Option<String>=tx.query_row("SELECT value FROM metadata WHERE key=?1",[key(job)],|r|r.get(0)).optional().map_err(|_|fail())?;
  let mut state=raw.map(|v|serde_json::from_str::<Value>(&v)).transpose().map_err(|_|fail())?.unwrap_or(json!({}));
  if event["type"]=="locator"{
   let loc=&event["locator"];
   let mut same=state["locator"].clone();if same["userMessageId"].is_null()&&!same.is_null(){same["userMessageId"]=loc["userMessageId"].clone();}
   if !providers::chrome_locator_valid(cfg,p,loc)||!state["locator"].is_null()&&same!=*loc{return Err(fail());}
   if state["locator"]==*loc{return Ok(());}state["locator"]=loc.clone();
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
