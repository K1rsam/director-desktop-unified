//! Loopback HTTP and synthetic source identities only.
use super::*;
use super::super::wire_tests::fixture;
use std::sync::Mutex;
fn setup(endpoint:&str)->(ProviderConfig,PreparedRequest,Value,Value,Value,Value){
 let (cfg,mut p,mut root,replies)=prepared_refresh_tests::setup(endpoint);
 p.wire["metadata"]["promptSha256"]=json!(hex_hash(s(&p.wire,"message").as_bytes()));
 let loc=json!({"version":"chrome-conversation-locator/v1","requestId":p.request.request_id,"sourceClientId":p.wire["sourceClientId"],"browserBinding":cfg.chrome_binding,"preparedSha256":hex_hash(json!(p).to_string().as_bytes()),
  "promptSha256":p.wire["metadata"]["promptSha256"],"conversationId":"11111111-1111-4111-8111-111111111111","url":"https://chatgpt.com/c/11111111-1111-4111-8111-111111111111","userMessageId":"native-user"});
 root["id"]=json!("controller");root["launchToken"]=json!(control_launch(cfg.chrome_binding.as_ref().unwrap()));
 let mut native=replies[7].clone();native["status"]=json!("confirmed_absent");native["match"]=Value::Null;native["sourceClientId"]=root["id"].clone();native["runtime"]["sourceClientId"]=root["id"].clone();
 let mut alias=root.clone();alias["id"]=json!("read-page");alias["browserTabId"]=json!(22);alias["launchToken"]=json!("__RECOVERY_ALIAS__");alias["url"]=loc["url"].clone();
 (cfg,p,loc,root,native,alias)
}
#[test]fn chrome_conversation_reopens_exact_url_once_after_durable_intent_without_generation_or_upload(){
 let tmp=fixture(vec![]);let (c,_,_,root,native,alias)=setup(&tmp.endpoint);std::env::remove_var(c.bridge_token_env.unwrap());tmp.thread.join().unwrap();
 let list=json!({"ok":true,"clients":[root]});
 let replies=vec![list.clone(),native,list,json!({"ok":true,"client":alias,"tabState":{"active":false,"discarded":false},"__echoRecoveryAlias":true}),json!({"ok":true,"clients":[root,alias],"__echoRecoveryAlias":true})];
 let f=fixture(replies);let (cfg,p,loc,_,_,_)=setup(&f.endpoint);let before=json!(p);let events=Mutex::new(vec![]);
 let restored=conversation::restore_source(&cfg,&wire::client(5).unwrap(),&p,&json!({"locator":loc}),&|event|{
  if event["type"]=="recovery_intent"{assert_eq!(f.requests.lock().unwrap().len(),3,"intent must precede open");}
  events.lock().unwrap().push(event.clone());Ok(())
 }).unwrap();f.thread.join().unwrap();assert_eq!(json!(p),before);assert_eq!(restored["url"],loc["url"]);assert_eq!(restored["focused"],false);
 let calls=f.requests.lock().unwrap();assert_eq!(calls.len(),5);assert_eq!(calls.iter().filter(|(r,_)|r.starts_with("POST ")).count(),1);assert!(calls[3].0.starts_with("POST /browser/tabs/open "));
 let body:Value=serde_json::from_str(&calls[3].1).unwrap();assert_eq!(body["url"],loc["url"]);assert_eq!(body["active"],false);assert_eq!(body["select"],false);assert_eq!(body["allowSystemFallback"],false);
 assert_ne!(body["launchToken"],format!("bridge-director-desktop-{}",p.request.request_id));assert_eq!(events.lock().unwrap().len(),2);std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test]fn chrome_conversation_unknown_open_intent_never_creates_another_page(){
 let tmp=fixture(vec![]);let(c,_,_,root,native,_)=setup(&tmp.endpoint);std::env::remove_var(c.bridge_token_env.unwrap());tmp.thread.join().unwrap();let list=json!({"ok":true,"clients":[root]});
 let f=fixture(vec![list.clone(),native,list]);let(cfg,p,loc,_,_,_)=setup(&f.endpoint);
 let saved=json!({"locator":loc,"recovery":{"type":"recovery_intent","launchToken":"bridge-director-desktop-22222222-2222-4222-8222-222222222222"}});
 assert!(conversation::restore_source(&cfg,&wire::client(5).unwrap(),&p,&saved,&|_|panic!("no new intent")).is_err());f.thread.join().unwrap();assert!(f.requests.lock().unwrap().iter().all(|(r,_)|r.starts_with("GET ")));std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test]fn chrome_conversation_foreign_saved_url_user_or_frozen_prepared_changes_are_rejected(){
 let f=fixture(vec![]);let(cfg,p,loc,_,_,_)=setup(&f.endpoint);assert!(conversation::locator_valid(&cfg,&p,&loc));
 for field in ["requestId","sourceClientId","preparedSha256","promptSha256","conversationId","url","userMessageId","browserBinding"]{
  let mut bad=loc.clone();bad[field]=json!("");assert!(!conversation::locator_valid(&cfg,&p,&bad),"{field}");
  assert!(conversation::restore_source(&cfg,&wire::client(5).unwrap(),&p,&json!({"locator":bad}),&|_|panic!("invalid receipt cannot write")).is_err());
 }
 f.thread.join().unwrap();assert!(f.requests.lock().unwrap().is_empty());std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test]fn chrome_conversation_no_image_original_is_settled_without_a_new_submission(){
 let tmp=fixture(vec![]);let(c,mut p,loc,mut worker,_,_)=setup(&tmp.endpoint);std::env::remove_var(c.bridge_token_env.unwrap());tmp.thread.join().unwrap();
 p.wire["correlationVersion"]=json!("conversation/v1");worker["id"]=p.wire["sourceClientId"].clone();worker["launchToken"]=json!(format!("bridge-director-desktop-{}",p.request.request_id));worker["url"]=loc["url"].clone();
 let proof=json!({"source":"conversation-record","version":1,"proofVersion":2,"status":"matched_no_image","requestId":p.request.request_id,"promptSha256":p.wire["metadata"]["promptSha256"],"conversationId":loc["conversationId"],"userMessageId":"native-user","assistantMessageId":"native-assistant","backendStatus":"finished_successfully","endTurn":true,"backendImageCount":0,"bindings":[],"recordSha256":"a".repeat(64),"recordRecheckSha256":"b".repeat(64),"assistantTextSha256":"c".repeat(64),"currentNode":"final","branchIdentity":[{"nodeKey":"final","id":"native-assistant","role":"assistant","status":"finished_successfully","endTurn":true},{"nodeKey":"user","id":"native-user","role":"user"},{"nodeKey":"root","id":"","role":""}]});
 let f=fixture(vec![json!({"ok":true,"turn":{"id":p.request.request_id,"status":"failed","input":{"sourceClientId":p.wire["sourceClientId"]}}}),json!({"ok":true,"clients":[worker]}),json!({"ok":true,"result":{"sourceClientId":p.wire["sourceClientId"],"session":{"id":loc["conversationId"]},"artifacts":[],"reconciliation":proof}})]);
 let(cfg,mut p,_,_,_,_)=setup(&f.endpoint);p.wire["correlationVersion"]=json!("conversation/v1");let events=Mutex::new(vec![]);
 let result=query_with_saved(&cfg,&p.request.request_id,&p,&json!({}),&|e|{events.lock().unwrap().push(e.clone());Ok(())}).unwrap();f.thread.join().unwrap();
 assert_eq!(result.status,"failed");assert!(result.result_urls.is_empty());assert_eq!(result.raw["taskSettlement"]["id"],p.request.request_id);assert_eq!(events.lock().unwrap().len(),1);
 let calls=f.requests.lock().unwrap();assert_eq!(calls.len(),3);assert!(calls.iter().all(|(r,_)|r.starts_with("GET ")||r.starts_with("POST /browser/recover-latest ")));std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test]fn chrome_conversation_cleanup_closes_only_recovery_alias_and_keeps_visible_original(){
 let tmp=fixture(vec![]);let(c,p,_loc,root,_,mut alias)=setup(&tmp.endpoint);std::env::remove_var(c.bridge_token_env.unwrap());tmp.thread.join().unwrap();
 alias["launchToken"]=json!("bridge-director-desktop-33333333-3333-4333-8333-333333333333");let mut original=alias.clone();original["id"]=p.wire["sourceClientId"].clone();original["launchToken"]=json!(format!("bridge-director-desktop-{}",p.request.request_id));original["focused"]=json!(true);original["visibilityState"]=json!("visible");
 let f=fixture(vec![json!({"ok":true,"clients":[root,alias,original]}),json!({"ok":true,"closing":true})]);let(cfg,p,loc,_,_,_)=setup(&f.endpoint);
 let result:QueryResult=serde_json::from_value(json!({"taskId":p.request.request_id,"status":"succeeded","resultUrls":["native-original"],"raw":{"nativeProof":{"status":"matched_complete","proofVersion":2,"requestId":p.request.request_id,"promptSha256":p.wire["metadata"]["promptSha256"],"conversationId":loc["conversationId"],"userMessageId":loc["userMessageId"]},"readOnlyRecovery":{"locator":loc,"client":alias}}})).unwrap();
 assert_eq!(release_completed_tab(&cfg,&p,&result).unwrap()["status"],"close_requested");f.thread.join().unwrap();let calls=f.requests.lock().unwrap();assert_eq!(calls.len(),2);let body:Value=serde_json::from_str(&calls[1].1).unwrap();assert_eq!(body["sourceClientId"],alias["id"]);assert_eq!(body["expectedLaunchToken"],alias["launchToken"]);assert_ne!(body["sourceClientId"],p.wire["sourceClientId"]);
 let mut bad=result.clone();bad.raw["readOnlyRecovery"]["locator"]["userMessageId"]=json!("foreign");assert!(release_completed_tab(&cfg,&p,&bad).is_err());std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test]fn chrome_upload_settlement_only_cancels_halted_original_between_two_native_empty_reads(){
 let base=crate::chrome_terminal::tests::upload_fixture();let mut source=json!({"id":"worker","url":"https://chatgpt.com/"});source["focused"]=json!(false);
 let mut proof=base["result"]["raw"]["uploadNotSubmitted"].clone();let task=s(&base,"providerTaskId");let prepared:PreparedRequest=serde_json::from_value({let mut p=base["prepared"].clone();p["capabilityVersion"]=json!("fixture");p["uploadReceipts"]=json!([]);p["preparedAt"]=json!("original");p["request"]["prompt"]=json!("synthetic image");p["request"]["ratio"]=json!("16:9");p["request"]["resolution"]=json!("native");p["request"]["references"]=json!([]);p["request"]["options"]=json!({});p}).unwrap();
 proof["preparedSha256"]=json!(hex_hash(json!(prepared).to_string().as_bytes()));
 let diagnostic=json!({"ok":true,"authoritative":true,"requests":{"state":proof["canonicalState"]}});
 let f=fixture(vec![diagnostic.clone(),proof["nativeEmptyBefore"].clone(),json!({"ok":true,"turn":{"id":task,"status":"cancelled"}}),proof["nativeEmptyAfter"].clone()]);let(cfg,_,_,_,_,_)=setup(&f.endpoint);
 let result=upload_settlement::settle(&cfg,&wire::client(5).unwrap(),&json!({"id":task,"status":"running"}),&source,&prepared).unwrap().unwrap();f.thread.join().unwrap();assert_eq!(result.status,"failed");assert!(crate::chrome_terminal::upload_not_submitted_valid(&result.raw["uploadNotSubmitted"],&json!(prepared),task));
 let calls=f.requests.lock().unwrap();assert_eq!(calls.len(),4);assert!(calls[1].0.starts_with("POST /browser/recover-latest "));assert!(calls[2].0.starts_with(&format!("POST /turns/{task}/interrupt ")));assert!(calls[3].0.starts_with("POST /browser/recover-latest "));std::env::remove_var(cfg.bridge_token_env.unwrap());
 let mut bad=diagnostic;bad["requests"]["state"]["effect"]["coordinator"]["lastResult"]["data"]["result"]["activeRequest"]["sentAt"]=json!(1);
 let f=fixture(vec![bad]);let(cfg,_,_,_,_,_)=setup(&f.endpoint);assert!(upload_settlement::settle(&cfg,&wire::client(5).unwrap(),&json!({"id":task,"status":"running"}),&source,&prepared).unwrap().is_none());f.thread.join().unwrap();assert_eq!(f.requests.lock().unwrap().len(),1);std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test]fn chrome_reference_upload_names_are_unique_per_request_and_reference_while_bytes_identity_stays_in_receipts(){
 assert_ne!(reference_upload_name("request-a",0,"png"),reference_upload_name("request-b",0,"png"));assert_ne!(reference_upload_name("request-a",0,"png"),reference_upload_name("request-a",1,"png"));assert_eq!(reference_upload_name("request-a",0,"png"),"request-a-ref-0.png");
}
