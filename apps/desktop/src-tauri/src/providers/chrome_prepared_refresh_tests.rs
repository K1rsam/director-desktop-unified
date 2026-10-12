//! Synthetic loopback fixtures only: no real credentials, pages or generation.
use super::*;
use super::super::wire_tests::fixture;
const ID:&str="a4d27dca-a0c4-4a43-b6be-75528f9fd27f";
const PROFILE:&str="b84d4521-56ba-4f84-9948-090f82d639e7";
pub(super) fn setup(endpoint:&str)->(ProviderConfig,PreparedRequest,Value,Vec<Value>){
    let env=format!("DIRECTOR_REFRESH_FIXTURE_{}",uuid::Uuid::new_v4().simple());std::env::set_var(&env,"synthetic-only");
    let endpoint=endpoint.trim_end_matches("/v1");
    let cfg:ProviderConfig=serde_json::from_value(json!({"kind":"chrome","endpoint":endpoint,"bridgeTokenEnv":env,"chromeBinding":{"profileId":PROFILE,"bundleId":"a".repeat(32),"endpoint":endpoint}})).unwrap();
    let request:GenerationRequest=serde_json::from_value(json!({"requestId":ID,"kind":"image","model":"chatgpt-image","prompt":"synthetic","ratio":"16:9","resolution":"native","options":{"count":1},"references":[{"mediaId":"ref","kind":"image","path":"not-a-real-local-file.png","sha256":"b".repeat(64),"order":0}]})).unwrap();
    let source=json!({"id":"worker","browserTabId":11,"launchToken":format!("bridge-director-desktop-{ID}"),"url":"https://chatgpt.com/","ready":true,"compatible":true,"transport":"extension","browserProfileId":PROFILE,"extensionBundleId":"a".repeat(32),"browserIncognito":false,"backgroundEpoch":"new-epoch","serverInstanceId":"new-server","contentEpoch":"new-content","focused":false,"visibilityState":"hidden","activeRequest":null});
    let mut intent=prepare_intent(&request,cfg.chrome_binding.as_ref().unwrap(),&source,"new_prepare",Value::Null).unwrap();
    intent["runtime"]["backgroundEpoch"]=json!("old-epoch");intent["runtime"]["serverInstanceId"]=json!("old-server");intent["sourceClientId"]=json!("controller-opener");
    let prepared:PreparedRequest=serde_json::from_value(json!({"provider":"chrome","request":request,"capabilityVersion":"fixture","uploadReceipts":[{"mediaId":"ref","sha256":"b".repeat(64),"order":0,"kind":"image","fileId":"existing-file","uploadedAt":"original-upload","upstreamReadConfirmed":false}],"preparedAt":"original-time","wire":{"id":ID,"message":"frozen original message","attachments":["existing-file"],"newSession":false,"sourceClientId":"worker","browserBinding":cfg.chrome_binding,"prepareRuntime":intent,"metadata":{"jobId":ID,"promptSha256":"c".repeat(64)},"emptyPreflight":{"original":true}}})).unwrap();
    let execution=json!({"version":"director-source-execution-idle/v1","sourceTabId":11,"inventoryCommandId":"inventory-cmd","rpcRequestId":"ext-refresh-123456","nativeSamples":2,"observedAt":chrono::Utc::now().timestamp_millis(),"lease":null,"ownCommand":{"commandId":"inventory-cmd","commandType":"browser.tab.identify","scope":"standalone","mode":"result","operation":"control","status":"dispatched","requestId":""},"otherUnresolvedCommands":[],"otherPendingOutbox":[],"ownAcceptedMessageIds":[],"queue":{"version":"tab-operation-snapshot/v1","tabId":11,"running":{"sequence":1,"label":"content:bridge.tab.open","meta":{"type":"bridge.tab.open","requestId":"ext-refresh-123456"}},"queued":[]}});
    let inventory=json!({"tabs":[{"id":11,"urlSha256":hex_hash(b"https://chatgpt.com/"),"incognito":false,"active":false,"discarded":false}],"launchRecords":[],"sourceTabId":11,"sourceExecutionProof":execution});
    let raw=inventory.to_string();
    let native=json!({"ok":true,"version":"director-owned-launch-inventory/v1","status":"matched","requestId":ID,"launchToken":source["launchToken"],"complete":true,"componentVersion":"6.4.0","sourceClientId":"worker","commandId":"inventory-cmd","inventoryCommandId":"inventory-cmd","rpcRequestId":"ext-refresh-123456","runtime":{"profileId":PROFILE,"bundleId":"a".repeat(32),"backgroundEpoch":"new-epoch","serverInstanceId":"new-server","sourceClientId":"worker","sourceTabId":11,"extensionVersion":"2.4.5","contentEpoch":"new-content"},"inventoryGeneration":"b3900d30-f8f3-444b-8f32-d4f4f96883ca","inventorySha256":hex_hash(raw.as_bytes()),"inventoryCanonicalJson":raw,"inventory":inventory,"observedAt":chrono::Utc::now().timestamp_millis(),"match":{"tabId":11,"url":"https://chatgpt.com/","launchToken":source["launchToken"],"connected":true}});
    let absent=json!({"__httpStatus":404,"detail":format!("Turn not found: {ID}")});
    let canonical=json!({"ok":true,"authoritative":true,"requests":{"state":null,"history":[],"deadlines":[]}});
    let list=json!({"ok":true,"clients":[source]});
    let empty=json!({"ok":true,"result":{"sourceClientId":"worker","url":"https://chatgpt.com/","reconciliation":{"source":"browser-empty-preflight","status":"matched_empty","version":1,"proofVersion":2}}});
    (cfg,prepared,source,vec![list.clone(),absent.clone(),canonical.clone(),empty,list.clone(),absent,canonical,native,list])
}
fn assert_no_generation(calls:&[(String,String)]){
    assert!(calls.iter().all(|(route,_)|route.starts_with("GET ") || route.starts_with("POST /browser/recover-latest ")));
    for (route,body) in calls.iter().filter(|(route,_)|route.starts_with("POST ")) {assert!(route.contains("recover-latest"));let body:Value=serde_json::from_str(body).unwrap();assert_eq!(body["sourceClientId"],"worker");assert_eq!(body["reconcileConversation"]["mode"],"image_preflight");}
}
#[test] fn chrome_prepared_refresh_preserves_complete_frozen_payload_and_records_original_and_native_proof(){
    let placeholder=fixture(vec![]);let (initial,_,_,replies)=setup(&placeholder.endpoint);std::env::remove_var(initial.bridge_token_env.unwrap());placeholder.thread.join().unwrap();
    let f=fixture(replies);let (cfg,p,_,_)=setup(&f.endpoint);let refreshed=refresh_unsubmitted_prepared(&cfg,&p).unwrap();f.thread.join().unwrap();
    assert_eq!(serde_json::to_vec(&refreshed.request).unwrap(),serde_json::to_vec(&p.request).unwrap());assert_eq!(serde_json::to_vec(&refreshed.upload_receipts).unwrap(),serde_json::to_vec(&p.upload_receipts).unwrap());
    let mut expected=p.wire.clone();for key in ["backgroundEpoch","serverInstanceId","contentEpoch"] {expected["prepareRuntime"]["runtime"][key]=refreshed.wire["prepareRuntime"]["runtime"][key].clone();}
    expected["preparedRuntimeRefresh"]=refreshed.wire["preparedRuntimeRefresh"].clone();assert_eq!(refreshed.wire,expected);
    let audit=&refreshed.wire["preparedRuntimeRefresh"][0];assert_eq!(audit["originalPrepareRuntime"],p.wire["prepareRuntime"]);assert_eq!(audit["originalPreparedAt"],p.prepared_at);assert_eq!(audit["nativeExecutionProof"]["status"],"matched");assert_eq!(audit["bridgeAbsenceAfter"]["turnStatus"],404);
    assert_ne!(refreshed.prepared_at,p.prepared_at);assert_eq!(refreshed.wire["prepareRuntime"]["runtime"]["backgroundEpoch"],"new-epoch");assert_eq!(f.requests.lock().unwrap().len(),9);assert_no_generation(&f.requests.lock().unwrap());std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test] fn chrome_prepared_refresh_invalid_frozen_request_identity_never_uses_transport(){
    for case in ["provider","kind","uuid","source","request_hash","launch","profile","binding","runtime","attachments","new_session"] {
        let f=fixture(vec![]);let (cfg,mut p,_,_)=setup(&f.endpoint);
        match case {"provider"=>p.provider="bh".into(),"kind"=>p.request.kind="video".into(),"uuid"=>p.request.request_id="not-uuid".into(),"source"=>p.wire["sourceClientId"]=Value::Null,"request_hash"=>p.request.prompt="changed".into(),"launch"=>p.wire["prepareRuntime"]["launchToken"]=json!("user"),"profile"=>p.wire["prepareRuntime"]["runtime"]["browserProfileId"]=json!("other"),"binding"=>p.wire["browserBinding"]["bundleId"]=json!("other"),"runtime"=>p.wire["prepareRuntime"]["runtime"]["backgroundEpoch"]=Value::Null,"attachments"=>p.wire["attachments"]=Value::Null,_=>p.wire["newSession"]=json!(true)};
        assert!(refresh_unsubmitted_prepared(&cfg,&p).is_err(),"{case}");f.thread.join().unwrap();assert!(f.requests.lock().unwrap().is_empty());std::env::remove_var(cfg.bridge_token_env.unwrap());
    }
}
#[test] fn chrome_prepared_refresh_preserves_prior_audit_history_and_survives_serde_roundtrip(){
    let tmp=fixture(vec![]);let (c,_,_,replies)=setup(&tmp.endpoint);std::env::remove_var(c.bridge_token_env.unwrap());tmp.thread.join().unwrap();
    let f=fixture(replies);let (cfg,mut p,_,_)=setup(&f.endpoint);let previous=json!({"version":"historic-audit","originalRuntimeSha256":"a".repeat(64)});p.wire["preparedRuntimeRefresh"]=json!([previous]);
    let restored:PreparedRequest=serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();let refreshed=refresh_unsubmitted_prepared(&cfg,&restored).unwrap();
    assert_eq!(refreshed.wire["preparedRuntimeRefresh"].as_array().unwrap().len(),2);assert_eq!(refreshed.wire["preparedRuntimeRefresh"][0],previous);assert_eq!(refreshed.wire["preparedRuntimeRefresh"][1]["originalPreparedSha256"],hex_hash(json!(p).to_string().as_bytes()));
    f.thread.join().unwrap();assert_no_generation(&f.requests.lock().unwrap());std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test] fn chrome_prepared_refresh_user_nonempty_busy_duplicate_or_changed_owner_never_reopens(){
    for case in ["url","visible","focused","busy","lease_hint","profile","duplicate","source","copied_profile","epoch_after","content_after"] {
        let tmp=fixture(vec![]);let (c,_,_,mut replies)=setup(&tmp.endpoint);std::env::remove_var(c.bridge_token_env.unwrap());tmp.thread.join().unwrap();
        let at=if case.ends_with("after"){4}else{0};let p=&mut replies[at]["clients"][0];
        match case {"url"=>p["url"]=json!("https://chatgpt.com/c/existing"),"visible"=>p["visibilityState"]=json!("visible"),"focused"=>p["focused"]=json!(true),"busy"=>p["activeRequest"]=json!({"requestId":"other"}),"lease_hint"=>p["releasePending"]=json!(true),"profile"=>p["browserProfileId"]=json!("other"),"source"=>p["id"]=json!("other"),"epoch_after"=>p["backgroundEpoch"]=json!("changed"),"content_after"=>p["contentEpoch"]=json!("changed"),"duplicate"|"copied_profile"=>{let mut duplicate=p.clone();duplicate["id"]=json!("duplicate");if case=="copied_profile"{duplicate["backgroundEpoch"]=json!("copied");duplicate["launchToken"]=json!("user");}replies[at]["clients"].as_array_mut().unwrap().push(duplicate);},_=>unreachable!()};
        replies.truncate(at+1);let f=fixture(replies);let (cfg,p,_,_)=setup(&f.endpoint);assert!(refresh_unsubmitted_prepared(&cfg,&p).is_err(),"{case}");f.thread.join().unwrap();assert_no_generation(&f.requests.lock().unwrap());std::env::remove_var(cfg.bridge_token_env.unwrap());
    }
}
#[test] fn chrome_prepared_refresh_remote_exists_non_authoritative_nonempty_or_unknown_never_changes_snapshot(){
    for case in ["turn_exists","wrong_404","history","state","deadline","authoritative","nonempty","wrong_empty_source","empty_unknown","post_preflight_turn"] {
        let tmp=fixture(vec![]);let (c,_,_,mut replies)=setup(&tmp.endpoint);std::env::remove_var(c.bridge_token_env.unwrap());tmp.thread.join().unwrap();
        let end=match case {"turn_exists"=>{replies[1]=json!({"ok":true,"turn":{"id":ID}});2},"wrong_404"=>{replies[1]["detail"]=json!("Route not found");2},"history"=>{replies[2]["requests"]["history"]=json!([{}]);3},"state"=>{replies[2]["requests"]["state"]=json!({});3},"deadline"=>{replies[2]["requests"]["deadlines"]=json!([{}]);3},"authoritative"=>{replies[2]["authoritative"]=json!(false);3},"nonempty"=>{replies[3]["result"]["reconciliation"]["status"]=json!("matched_complete");4},"wrong_empty_source"=>{replies[3]["result"]["sourceClientId"]=json!("user");4},"empty_unknown"=>{replies[3]["ok"]=json!(false);4},_=>{replies[5]=json!({"ok":true,"turn":{"id":ID}});6}};
        replies.truncate(end);let f=fixture(replies);let (cfg,p,_,_)=setup(&f.endpoint);let original=json!(p);assert!(refresh_unsubmitted_prepared(&cfg,&p).is_err(),"{case}");assert_eq!(json!(p),original);f.thread.join().unwrap();assert_no_generation(&f.requests.lock().unwrap());std::env::remove_var(cfg.bridge_token_env.unwrap());
    }
}
#[test] fn chrome_prepared_refresh_native_stale_busy_wrong_match_or_final_drift_never_reuses_prepare(){
    for case in ["absent","match_tab","match_url","match_launch","match_disconnected","lease","queue","outbox","stale","hash","final_epoch","final_content","final_duplicate"] {
        let tmp=fixture(vec![]);let (c,_,_,mut replies)=setup(&tmp.endpoint);std::env::remove_var(c.bridge_token_env.unwrap());tmp.thread.join().unwrap();
        let native=&mut replies[7];
        match case {"absent"=>native["status"]=json!("confirmed_absent"),"match_tab"=>native["match"]["tabId"]=json!(22),"match_url"=>native["match"]["url"]=json!("https://chatgpt.com/c/existing"),"match_launch"=>native["match"]["launchToken"]=json!("user"),"match_disconnected"=>native["match"]["connected"]=json!(false),"lease"=>native["inventory"]["sourceExecutionProof"]["lease"]=json!({}),"queue"=>native["inventory"]["sourceExecutionProof"]["otherUnresolvedCommands"]=json!([{}]),"outbox"=>native["inventory"]["sourceExecutionProof"]["otherPendingOutbox"]=json!([{}]),"stale"=>native["observedAt"]=json!(chrono::Utc::now().timestamp_millis()-31000),"hash"=>native["inventorySha256"]=json!("bad"),"final_epoch"=>replies[8]["clients"][0]["backgroundEpoch"]=json!("changed"),"final_duplicate"=>{let mut duplicate=replies[8]["clients"][0].clone();duplicate["id"]=json!("duplicate");replies[8]["clients"].as_array_mut().unwrap().push(duplicate);},_=>replies[8]["clients"][0]["contentEpoch"]=json!("changed")};
        if ["lease","queue","outbox"].contains(&case){let raw=replies[7]["inventory"].to_string();replies[7]["inventoryCanonicalJson"]=json!(raw);replies[7]["inventorySha256"]=json!(hex_hash(raw.as_bytes()));}
        if ["lease","queue","outbox","stale","hash"].contains(&case){replies.truncate(8);}
        let f=fixture(replies);let (cfg,p,_,_)=setup(&f.endpoint);assert!(refresh_unsubmitted_prepared(&cfg,&p).is_err(),"{case}");f.thread.join().unwrap();assert_no_generation(&f.requests.lock().unwrap());std::env::remove_var(cfg.bridge_token_env.unwrap());
    }
}
