//! Real local HTTP fixtures only. These tests never contact a logged-in browser.
use super::*;
use super::super::wire_tests::fixture;

fn tab(id: &str, url: &str) -> Value {
    json!({"id":id,"url":url,"ready":true,"compatible":true,"transport":"extension",
        "backgroundEpoch":"one-profile-runtime","extensionBundleId":"one-extension","browserProfileId":"b84d4521-56ba-4f84-9948-090f82d639e7","browserIncognito":false,
        "serverInstanceId":"one-server","focused":false,"visibilityState":"hidden","activeRequest":null})
}
fn config(endpoint: &str, env: &str) -> ProviderConfig {
    ProviderConfig { kind:"chrome".into(),endpoint:Some(endpoint.trim_end_matches("/v1").into()),
        bridge_token_env:Some(env.into()),chrome_binding:Some(crate::chrome_setup::BrowserBinding{profile_id:"b84d4521-56ba-4f84-9948-090f82d639e7".into(),bundle_id:"one-extension".into(),endpoint:endpoint.trim_end_matches("/v1").trim_end_matches('/').into()}),credential_env:None,cli_path:None,timeout_seconds:5,adapted_models:vec![] }
}
fn request() -> GenerationRequest {
    GenerationRequest {request_id:"offline-task".into(),kind:"image".into(),model:"chatgpt-image".into(),
        prompt:"synthetic scene".into(),duration:None,ratio:"16:9".into(),resolution:"native".into(),references:vec![],options:json!({"count":1})}
}
fn prepared() -> PreparedRequest {
    PreparedRequest {provider:"chrome".into(),request:request(),capability_version:"fixture".into(),
        upload_receipts:vec![],wire:json!({"sourceClientId":"worker","metadata":{"promptSha256":"hash"}}),estimated_cost:None,prepared_at:now()}
}
fn result() -> QueryResult {
    QueryResult {task_id:"offline-task".into(),status:"succeeded".into(),result_urls:vec!["synthetic-original".into()],
        error:None,actual_cost:None,raw:json!({"nativeProof":{"status":"matched_complete","proofVersion":2,
            "requestId":"offline-task","promptSha256":"hash","conversationId":"original-conversation"}})}
}
fn credential() -> String {
    let env = format!("DIRECTOR_CHROME_BACKGROUND_TEST_{}",uuid::Uuid::new_v4().simple());
    std::env::set_var(&env,"synthetic-not-a-real-credential"); env
}

fn empty_preflight()->Value {json!({"ok":true,"result":{"sourceClientId":"worker","url":"https://chatgpt.com/","reconciliation":{"source":"browser-empty-preflight","status":"matched_empty","version":1,"proofVersion":2}}})}
fn recovery_owned()->Value {let mut page=tab("worker","https://chatgpt.com/");page["launchToken"]=json!("bridge-director-desktop-offline-task");page}
fn recovery_proof()->Value {json!({"version":"chrome-prepare-recovery/v1","requestHash":hex_hash(json!(request()).to_string().as_bytes()),"noLocalSubmit":true,"intent":null,"legacyNoSubmitAudit":{"version":"chrome-legacy-no-submit/v1","snapshotSha256":"a".repeat(64),"stateChainProven":true}})}
fn recovery_replies(page:&Value)->Vec<Value> {vec![json!({"ok":false,"needsSelection":true,"pendingRequests":0,"pendingCommands":0}),
    json!({"ok":true,"clients":[page]}),json!({"ok":true,"clients":[page]}),
    json!({"__httpStatus":404,"detail":"Turn not found: offline-task"}),json!({"ok":true,"authoritative":true,"requests":{"state":null,"history":[],"deadlines":[]}}),empty_preflight(),json!({"ok":true,"clients":[page]})]}

fn native_first_open_fixture()->(GenerationRequest,Value,Value,Vec<Value>) {
    let mut r=request();r.request_id="a4d27dca-a0c4-4a43-b6be-75528f9fd27f".into();
    let mut source=tab("control","https://chatgpt.com/");source["extensionBundleId"]=json!("a".repeat(32));
    source["launchToken"]=json!("bridge-director-control-b84d4521-56ba-4f84-9948-090f82d639e7");source["browserTabId"]=json!(10);source["contentEpoch"]=json!("one-content");
    let mut page=source.clone();page["id"]=json!("worker");page["browserTabId"]=json!(11);page["launchToken"]=json!(format!("bridge-director-desktop-{}",r.request_id));
    let inventory=json!({"tabs":[{"id":10,"urlSha256":"a".repeat(64),"incognito":false,"active":false,"discarded":false}],"launchRecords":[{"key":"controller","recordSha256":"a".repeat(64)}],"sourceTabId":10});
    let raw=inventory.to_string();let native=json!({"ok":true,"version":"director-owned-launch-inventory/v1","status":"confirmed_absent","requestId":r.request_id,
        "launchToken":page["launchToken"],"complete":true,"componentVersion":"6.4.0","runtime":{"profileId":source["browserProfileId"],"bundleId":source["extensionBundleId"],
            "backgroundEpoch":source["backgroundEpoch"],"serverInstanceId":source["serverInstanceId"],"sourceClientId":"control","sourceTabId":10,"extensionVersion":"2.4.5","contentEpoch":"one-content"},
        "inventoryGeneration":"b3900d30-f8f3-444b-8f32-d4f4f96883ca","inventorySha256":hex_hash(raw.as_bytes()),"inventoryCanonicalJson":raw,"inventory":inventory,
        "observedAt":chrono::Utc::now().timestamp_millis(),"absenceScope":"current_browser_inventory_only","sourceClientId":"control"});
    let proof=json!({"version":"chrome-prepare-recovery/v1","requestHash":hex_hash(json!(r).to_string().as_bytes()),"noLocalSubmit":true,"intent":null,
        "legacyNoSubmitAudit":{"version":"chrome-legacy-no-submit/v1","snapshotSha256":"a".repeat(64),"stateChainProven":true}});
    let replies=vec![json!({"ok":true,"pendingRequests":0,"pendingCommands":0}),json!({"ok":true,"clients":[source]}),json!({"ok":true,"clients":[source]}),
        json!({"__httpStatus":404,"detail":format!("Turn not found: {}",r.request_id)}),json!({"ok":true,"authoritative":true,"requests":{"state":null,"history":[],"deadlines":[]}}),native,
        json!({"ok":true,"clients":[source]}),json!({"ok":true,"client":page}),empty_preflight(),json!({"ok":true,"clients":[source,page]})];
    (r,source,proof,replies)
}
#[test]
fn chrome_connect_snapshot_requires_independent_live_proof_before_intent_or_open(){
    let (r,mut source,_proof,replies)=native_first_open_fixture();source["transportHealth"]=json!({"kind":"connect_snapshot","tabQueue":{"running":true,"pending":1}});
    let f=fixture(vec![replies[0].clone(),json!({"ok":true,"clients":[source]}),json!({"ok":true,"clients":[source]}),replies[5].clone()]);
    let env=credential();let mut cfg=config(&f.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);
    let error=prepare_observed(&cfg,&r,&|_|panic!("missing live queue proof cannot persist intent")).unwrap_err();assert_eq!(error.code,"BRIDGE_BUSY");f.thread.join().unwrap();
    assert_eq!(f.requests.lock().unwrap().len(),4);assert!(f.requests.lock().unwrap().iter().all(|(route,_)|route.starts_with("GET ")));std::env::remove_var(env);
}
#[test]
fn chrome_bound_disconnected_browser_is_queue_wait_before_any_prepare_or_execution(){
    let f=fixture(vec![json!({"ok":false,"needsSelection":false,"pendingRequests":0,"pendingCommands":0}),json!({"ok":true,"clients":[]})]);
    let env=credential();let cfg=config(&f.endpoint,&env);
    let error=prepare_observed(&cfg,&request(),&|_|panic!("disconnection cannot start execution")).unwrap_err();
    assert_eq!(error.code,"BRIDGE_BUSY");assert!(!error.submission_unknown);f.thread.join().unwrap();
    assert_eq!(f.requests.lock().unwrap().len(),2);assert!(f.requests.lock().unwrap().iter().all(|(route,_)|route.starts_with("GET ")));std::env::remove_var(env);
}
#[test]
fn chrome_fresh_exact_queue_read_permits_one_owned_prepare_after_readback_with_zero_generation(){
    let (r,mut source,_,replies)=native_first_open_fixture();source["transportHealth"]=json!({"kind":"connect_snapshot","tabQueue":{"running":true,"pending":1}});
    let mut native=replies[5].clone();native["commandId"]=json!("live-cmd");native["inventoryCommandId"]=json!("live-cmd");native["rpcRequestId"]=json!("ext-live-rpc-1234");
    native["inventory"]["sourceExecutionProof"]=json!({"version":"director-source-execution-idle/v1","sourceTabId":10,"inventoryCommandId":"live-cmd","rpcRequestId":"ext-live-rpc-1234","nativeSamples":2,"observedAt":chrono::Utc::now().timestamp_millis(),"lease":null,
        "ownCommand":{"commandId":"live-cmd","commandType":"browser.tab.identify","scope":"standalone","mode":"result","operation":"control","status":"dispatched","requestId":""},"otherUnresolvedCommands":[],"otherPendingOutbox":[],"ownAcceptedMessageIds":[],"queue":{"version":"tab-operation-snapshot/v1","tabId":10,"running":{"sequence":1,"label":"content:bridge.tab.open","meta":{"type":"bridge.tab.open","requestId":"ext-live-rpc-1234"}},"queued":[]}});
    let raw=native["inventory"].to_string();native["inventoryCanonicalJson"]=json!(raw);native["inventorySha256"]=json!(hex_hash(raw.as_bytes()));
    let f=fixture(vec![replies[0].clone(),json!({"ok":true,"clients":[source]}),json!({"ok":true,"clients":[source]}),native,json!({"ok":true,"clients":[source]}),replies[7].clone(),empty_preflight()]);
    let env=credential();let mut cfg=config(&f.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);
    let captured=std::sync::Mutex::new(vec![]);let p=prepare_observed(&cfg,&r,&|v|{assert_eq!(f.requests.lock().unwrap().len(),5);captured.lock().unwrap().push(v.clone());Ok(())}).unwrap();f.thread.join().unwrap();
    assert_eq!(p.wire["sourceClientId"],"worker");assert_eq!(captured.lock().unwrap().len(),1);assert_eq!(f.requests.lock().unwrap().len(),7);assert_eq!(f.requests.lock().unwrap().iter().filter(|(route,_)|route.starts_with("POST /browser/tabs/open ")).count(),1);assert!(f.requests.lock().unwrap().iter().all(|(route,_)|!route.starts_with("POST /turns")));std::env::remove_var(env);
}
#[test]
fn chrome_first_connection_bootstraps_one_fixed_background_control_without_touching_user_page(){
    let (_,mut control,_,_)=native_first_open_fixture();let mut user=control.clone();user["id"]=json!("user");user["browserTabId"]=json!(20);user["launchToken"]=json!("");user["focused"]=json!(true);user["visibilityState"]=json!("visible");user["url"]=json!("https://chatgpt.com/c/user-conversation");
    control["transportHealth"]=json!({"kind":"connect_snapshot"});let f=fixture(vec![json!({"ok":true,"clients":[user]}),json!({"ok":true,"client":control,"tabState":{"active":false,"autoDiscardable":false,"discarded":false}}),json!({"ok":true,"clients":[user,control]})]);
    let env=credential();let mut cfg=config(&f.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);let client=wire::client(5).unwrap();
    let pages=execution_proof::bootstrap_control(&cfg,&client,&user,cfg.chrome_binding.as_ref().unwrap()).unwrap();f.thread.join().unwrap();assert_eq!(pages.iter().find(|p|p["id"]=="user").unwrap(),&user);
    let requests=f.requests.lock().unwrap();assert_eq!(requests.len(),3);let body:Value=serde_json::from_str(&requests[1].1).unwrap();assert!(requests[1].0.starts_with("POST /browser/tabs/open "));assert_eq!(body["sourceClientId"],"user");assert_eq!(body["launchToken"],control["launchToken"]);assert_eq!(body["active"],false);assert_eq!(body["select"],false);assert_eq!(body["allowSystemFallback"],false);std::env::remove_var(env);
}
#[test]
fn chrome_bootstrap_existing_canonical_uses_zero_create_and_unconfirmed_create_never_prepares_generation(){
    let (_,control,_,_)=native_first_open_fixture();let mut user=control.clone();user["id"]=json!("user");user["launchToken"]=json!("");
    let env=credential();let existing=fixture(vec![json!({"ok":true,"clients":[user,control]})]);let mut cfg=config(&existing.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);let client=wire::client(5).unwrap();
    execution_proof::bootstrap_control(&cfg,&client,&user,cfg.chrome_binding.as_ref().unwrap()).unwrap();existing.thread.join().unwrap();assert_eq!(existing.requests.lock().unwrap().len(),1);
    for detail in ["Controller creation remains unresolved; original reservation was preserved","Controller creation has unknown tab identity"] {
        let failed=fixture(vec![json!({"ok":true,"clients":[user]}),json!({"__httpStatus":500,"detail":detail})]);let mut cfg=config(&failed.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);
        assert_eq!(execution_proof::bootstrap_control(&cfg,&client,&user,cfg.chrome_binding.as_ref().unwrap()).unwrap_err().code,"BRIDGE_BUSY");failed.thread.join().unwrap();assert_eq!(failed.requests.lock().unwrap().iter().filter(|(route,_)|route.starts_with("POST ")).count(),1);assert!(failed.requests.lock().unwrap().iter().all(|(route,_)|!route.starts_with("POST /turns")));
    }std::env::remove_var(env);
}
#[test]
fn chrome_prepare_native_absent_legacy_first_open_freezes_intent_before_one_open_and_never_reopens_after_timeout() {
    for timeout in [false,true] {
        let (r,source,proof,mut replies)=native_first_open_fixture();if timeout {replies[7]=json!({"__httpStatus":500,"detail":"Browser handshake timed out"});replies.truncate(8);}
        let f=fixture(replies);let env=credential();let mut cfg=config(&f.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);
        let captured=std::sync::Mutex::new(vec![]);let observe=|v:&Value|{assert_eq!(f.requests.lock().unwrap().len(),7);captured.lock().unwrap().push(v.clone());Ok(())};
        let prepared=recover_prepare(&cfg,&r,&proof,&observe);if timeout {assert_eq!(prepared.unwrap_err().code,"CHROME_PREPARE_OPEN_UNCONFIRMED");}else{assert_eq!(prepared.unwrap().wire["sourceClientId"],"worker");}
        f.thread.join().unwrap();let intent=captured.lock().unwrap()[0].clone();assert_eq!(intent["mode"],"legacy_first_open");assert_eq!(intent["legacyNoSubmitAudit"]["nativeCurrentAbsence"]["absenceScope"],"current_browser_inventory_only");
        assert_eq!(f.requests.lock().unwrap().iter().filter(|(route,_)|route.starts_with("POST /browser/tabs/open ")).count(),1);
        let again=fixture(vec![json!({"ok":true,"pendingRequests":0}),json!({"ok":true,"clients":[source]}),json!({"ok":true,"clients":[source]})]);let mut next_cfg=cfg.clone();next_cfg.endpoint=Some(again.endpoint.trim_end_matches("/v1").into());next_cfg.chrome_binding.as_mut().unwrap().endpoint=next_cfg.endpoint.clone().unwrap();
        // A restart is bound to the saved first-open intent. No second open,
        // even when the original page has not connected after a timeout.
        let mut saved=proof.clone();saved["intent"]=intent;
        assert_eq!(recover_prepare(&next_cfg,&r,&saved,&|_|panic!("never refreeze")).unwrap_err().code,"CHROME_PREPARE_WAIT_ORIGINAL");again.thread.join().unwrap();
        assert!(again.requests.lock().unwrap().iter().all(|(route,_)|!route.contains("/tabs/open") && !route.contains("/owned-launch/")));std::env::remove_var(env);
    }
}
#[test]
fn chrome_prepare_native_absence_unknown_matched_runtime_hash_or_inventory_drift_never_opens() {
    for case in ["unknown","matched","hash","runtime","source_tab","source_active","stale","missing_content","post_epoch"] {
        let (r,_,proof,mut replies)=native_first_open_fixture();
        match case {
            "unknown"|"matched"=>replies[5]["status"]=json!(case),"hash"=>replies[5]["inventorySha256"]=json!("0".repeat(64)),
            "runtime"=>replies[5]["runtime"]["backgroundEpoch"]=json!("changed"),"source_tab"=>replies[5]["runtime"]["sourceTabId"]=json!(20),
            "source_active"=>{replies[5]["inventory"]["tabs"][0]["active"]=json!(true);let raw=replies[5]["inventory"].to_string();replies[5]["inventoryCanonicalJson"]=json!(raw);replies[5]["inventorySha256"]=json!(hex_hash(raw.as_bytes()));},
            "stale"=>replies[5]["observedAt"]=json!(chrono::Utc::now().timestamp_millis()-31_000),
            "missing_content"=>{replies[5]["runtime"].as_object_mut().unwrap().remove("contentEpoch");},
            _=>replies[6]["clients"][0]["backgroundEpoch"]=json!("changed"),
        }
        replies.truncate(if case=="post_epoch"{7}else{6});let f=fixture(replies);let env=credential();let mut cfg=config(&f.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);
        assert!(recover_prepare(&cfg,&r,&proof,&|_|panic!("no intent before full native proof")).is_err(),"{case}");f.thread.join().unwrap();
        assert!(f.requests.lock().unwrap().iter().all(|(route,_)|!route.contains("/tabs/open") && !route.starts_with("POST /turns")),"{case}");std::env::remove_var(env);
    }
}
#[test]
fn chrome_prepare_clients_absence_and_user_homepage_alone_never_authorize_first_open() {
    let (r,_,proof,mut replies)=native_first_open_fixture();
    for index in [1,2] {replies[index]["clients"][0].as_object_mut().unwrap().remove("launchToken");}
    replies.truncate(3);let f=fixture(replies);let env=credential();let mut cfg=config(&f.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);
    assert!(recover_prepare(&cfg,&r,&proof,&|_|panic!("no first runtime adoption from clients absence")).is_err());f.thread.join().unwrap();
    assert!(f.requests.lock().unwrap().iter().all(|(route,_)|!route.contains("/owned-launch/") && !route.contains("/tabs/open") && !route.starts_with("POST /turns")));std::env::remove_var(env);
}
fn active_background_native(replies:&mut [Value],window_state:&str) {
    replies[5]["inventory"]["tabs"][0]["active"]=json!(true);replies[5]["inventory"]["tabs"][0]["windowId"]=json!(20);
    replies[5]["inventory"]["sourceBackgroundProof"]=json!({"version":"director-source-background-proof/v1","sourceTabId":10,"sourceWindowId":20,"nativeSamples":2,
        "native":{"tabActive":true,"windowFocused":false,"windowState":window_state},
        "content":{"visibilityState":"hidden","focused":false,"contentEpoch":"one-content","sampledAt":chrono::Utc::now().timestamp_millis()}});
    let raw=replies[5]["inventory"].to_string();replies[5]["inventoryCanonicalJson"]=json!(raw);replies[5]["inventorySha256"]=json!(hex_hash(raw.as_bytes()));
}
#[test]
fn chrome_prepare_native_active_source_requires_hashed_two_sample_background_window_and_current_content_proof() {
    for state in ["normal","minimized","maximized","fullscreen"] {
        let (r,_,proof,mut replies)=native_first_open_fixture();active_background_native(&mut replies,state);
        let f=fixture(replies);let env=credential();let mut cfg=config(&f.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);
        let prepared=recover_prepare(&cfg,&r,&proof,&|intent|{assert_eq!(intent["legacyNoSubmitAudit"]["nativeCurrentAbsence"]["inventory"]["tabs"][0]["active"],true);Ok(())}).unwrap();
        assert_eq!(prepared.wire["sourceClientId"],"worker");f.thread.join().unwrap();
        let calls=f.requests.lock().unwrap();assert_eq!(calls.iter().filter(|(route,_)|route.starts_with("POST /browser/tabs/open ")).count(),1);
        let open:Value=serde_json::from_str(&calls.iter().find(|(route,_)|route.starts_with("POST /browser/tabs/open ")).unwrap().1).unwrap();assert_eq!(open["active"],false);assert_eq!(open["select"],false);
        assert!(calls.iter().all(|(route,_)|!route.starts_with("POST /turns")));std::env::remove_var(env);
    }
}
#[test]
fn chrome_prepare_native_active_source_foreground_unknown_stale_or_identity_drift_is_held_before_open() {
    for case in ["missing","version","samples","tab","window","missing_window","native_active","window_focused","window_state","content_visible","content_focused","content_epoch","content_stale","content_future","hash","post_visible","post_focused"] {
        let (r,_,proof,mut replies)=native_first_open_fixture();active_background_native(&mut replies,"normal");
        match case {
            "missing"=>{replies[5]["inventory"].as_object_mut().unwrap().remove("sourceBackgroundProof");},
            "version"=>replies[5]["inventory"]["sourceBackgroundProof"]["version"]=json!("unknown"),
            "samples"=>replies[5]["inventory"]["sourceBackgroundProof"]["nativeSamples"]=json!(1),
            "tab"=>replies[5]["inventory"]["sourceBackgroundProof"]["sourceTabId"]=json!(99),
            "window"=>replies[5]["inventory"]["sourceBackgroundProof"]["sourceWindowId"]=json!(99),
            "missing_window"=>{replies[5]["inventory"]["tabs"][0].as_object_mut().unwrap().remove("windowId");},
            "native_active"=>replies[5]["inventory"]["sourceBackgroundProof"]["native"]["tabActive"]=json!(false),
            "window_focused"=>replies[5]["inventory"]["sourceBackgroundProof"]["native"]["windowFocused"]=json!(true),
            "window_state"=>replies[5]["inventory"]["sourceBackgroundProof"]["native"]["windowState"]=json!("unknown"),
            "content_visible"=>replies[5]["inventory"]["sourceBackgroundProof"]["content"]["visibilityState"]=json!("visible"),
            "content_focused"=>replies[5]["inventory"]["sourceBackgroundProof"]["content"]["focused"]=json!(true),
            "content_epoch"=>replies[5]["inventory"]["sourceBackgroundProof"]["content"]["contentEpoch"]=json!("changed"),
            "content_stale"=>replies[5]["inventory"]["sourceBackgroundProof"]["content"]["sampledAt"]=json!(chrono::Utc::now().timestamp_millis()-31_000),
            "content_future"=>replies[5]["inventory"]["sourceBackgroundProof"]["content"]["sampledAt"]=json!(chrono::Utc::now().timestamp_millis()+31_000),
            "post_visible"=>replies[6]["clients"][0]["visibilityState"]=json!("visible"),
            "post_focused"=>replies[6]["clients"][0]["focused"]=json!(true),
            _=>{},
        }
        let raw=replies[5]["inventory"].to_string();replies[5]["inventoryCanonicalJson"]=json!(raw);replies[5]["inventorySha256"]=json!(if case=="hash"{"0".repeat(64)}else{hex_hash(raw.as_bytes())});
        replies.truncate(if case.starts_with("post_"){7}else{6});let f=fixture(replies);let env=credential();let mut cfg=config(&f.endpoint,&env);cfg.chrome_binding.as_mut().unwrap().bundle_id="a".repeat(32);
        assert!(recover_prepare(&cfg,&r,&proof,&|_|panic!("no first-open intent from unproved active source")).is_err(),"{case}");f.thread.join().unwrap();
        assert!(f.requests.lock().unwrap().iter().all(|(route,_)|!route.contains("/tabs/open") && !route.starts_with("POST /turns")),"{case}");std::env::remove_var(env);
    }
}

#[test]
fn chrome_prepare_legacy_first_runtime_is_persisted_before_preflight_and_reuses_same_page_without_open_or_submit() {
    let page=recovery_owned();let f=fixture(recovery_replies(&page));let env=credential();let cfg=config(&f.endpoint,&env);
    let captured=std::sync::Mutex::new(vec![]);let observe=|value:&Value|{assert_eq!(f.requests.lock().unwrap().len(),5,"runtime persisted before empty probe");captured.lock().unwrap().push(value.clone());Ok(())};
    let p=recover_prepare(&cfg,&request(),&recovery_proof(),&observe).unwrap();f.thread.join().unwrap();
    assert_eq!(p.wire["sourceClientId"],"worker");assert_eq!(p.wire["prepareRuntime"]["mode"],"legacy_first_prepare");
    assert_eq!(p.wire["prepareRuntime"]["runtime"]["serverInstanceId"],"one-server");assert_eq!(captured.lock().unwrap().len(),1);
    let calls=f.requests.lock().unwrap();assert!(calls.iter().all(|(route,_)|!route.starts_with("POST /turns") && !route.contains("/tabs/open")));
    std::env::remove_var(env);
}

#[test]
fn chrome_prepare_new_intent_is_persisted_before_only_open_side_effect_even_when_open_times_out() {
    let page=tab("home","https://chatgpt.com/");let f=fixture(vec![json!({"ok":false,"needsSelection":true,"pendingRequests":0}),
        json!({"ok":true,"clients":[page]}),json!({"ok":true,"clients":[page]}),json!({"__httpStatus":500,"detail":"Browser tab handshake timed out"})]);
    let env=credential();let captured=std::sync::Mutex::new(vec![]);let observe=|value:&Value|{assert_eq!(f.requests.lock().unwrap().len(),3);captured.lock().unwrap().push(value.clone());Ok(())};
    let error=prepare_observed(&config(&f.endpoint,&env),&request(),&observe).unwrap_err();f.thread.join().unwrap();
    assert_eq!(error.code,"CHROME_PREPARE_OPEN_UNCONFIRMED");assert_eq!(error.context.as_ref().unwrap()["phase"],"tab_open");
    assert_eq!(error.context.as_ref().unwrap()["httpStatus"],500);
    assert!(error.message.contains("handshake"));assert_eq!(captured.lock().unwrap().len(),1);
    assert_eq!(f.requests.lock().unwrap().iter().filter(|(route,_)|route.contains("/tabs/open")).count(),1);std::env::remove_var(env);
}

#[test]
fn chrome_prepare_saved_runtime_or_request_drift_is_held_without_another_open() {
    let page=recovery_owned();
    for field in ["epoch","server","profile","bundle","hash","launch"] {
        let f=fixture(recovery_replies(&page).into_iter().take(3).collect());let env=credential();let cfg=config(&f.endpoint,&env);
        let mut proof=recovery_proof();proof["intent"]=prepare_intent(&request(),cfg.chrome_binding.as_ref().unwrap(),&page,"new_prepare",Value::Null).unwrap();
        match field {"epoch"=>proof["intent"]["runtime"]["backgroundEpoch"]=json!("changed"),"server"=>proof["intent"]["runtime"]["serverInstanceId"]=json!("changed"),
            "profile"=>proof["intent"]["browserBinding"]["profileId"]=json!("changed"),"bundle"=>proof["intent"]["browserBinding"]["bundleId"]=json!("changed"),
            "hash"=>proof["intent"]["requestHash"]=json!("changed"),_=>proof["intent"]["launchToken"]=json!("changed")};
        assert_eq!(recover_prepare(&cfg,&request(),&proof,&|_|Ok(())).unwrap_err().code,"CHROME_PREPARE_IDENTITY_HELD","{field}");
        f.thread.join().unwrap();assert_eq!(f.requests.lock().unwrap().len(),3);std::env::remove_var(env);
    }
}

#[test]
fn chrome_prepare_legacy_requires_authoritative_no_turn_and_no_canonical_submit_then_empty_native_proof() {
    let page=recovery_owned();
    for phase in ["turn","canonical","canonical_history","canonical_deadline","wrong_array_schema","missing_state","empty","post_runtime"] {
        let mut replies=recovery_replies(&page);
        match phase {"turn"=>{replies[3]=json!({"ok":true,"turn":{"id":"offline-task","status":"queued"}});replies.truncate(4);},
            "canonical"=>{replies[4]["requests"]["state"]=json!({"status":"unknown"});replies.truncate(5);},
            "canonical_history"=>{replies[4]["requests"]["history"]=json!([{"status":"previous_submit"}]);replies.truncate(5);},
            "canonical_deadline"=>{replies[4]["requests"]["deadlines"]=json!([{"state":"pending"}]);replies.truncate(5);},
            "wrong_array_schema"=>{replies[4]["requests"]=json!([]);replies.truncate(5);},
            "missing_state"=>{replies[4]["requests"].as_object_mut().unwrap().remove("state");replies.truncate(5);},
            "empty"=>{replies[5]["result"]["reconciliation"]["status"]=json!("not_empty");replies.truncate(6);},
            _=>replies[6]["clients"][0]["backgroundEpoch"]=json!("changed")};
        let f=fixture(replies);let env=credential();assert!(recover_prepare(&config(&f.endpoint,&env),&request(),&recovery_proof(),&|_|Ok(())).is_err(),"{phase}");
        f.thread.join().unwrap();assert!(f.requests.lock().unwrap().iter().all(|(route,_)|!route.contains("/tabs/open") && !route.starts_with("POST /turns")));std::env::remove_var(env);
    }
}

#[test]
fn chrome_prepare_standalone_busy_is_queue_wait_with_safe_detail_code_and_phase() {
    let page=tab("home","https://chatgpt.com/");let f=fixture(vec![json!({"ok":false,"needsSelection":true,"pendingRequests":0}),
        json!({"ok":true,"clients":[page]}),json!({"ok":true,"clients":[page]}),json!({"__httpStatus":500,"code":"STANDALONE_COMMAND_BUSY","detail":"Client has an unreleased standalone command"})]);
    let env=credential();let error=prepare(&config(&f.endpoint,&env),&request()).unwrap_err();f.thread.join().unwrap();
    assert_eq!(error.code,"BRIDGE_BUSY");assert_eq!(error.context.unwrap()["phase"],"tab_open");assert!(error.message.contains("standalone"));assert!(!error.submission_unknown);std::env::remove_var(env);
}

#[test]
fn chrome_prepare_pending_standalone_client_prevents_open_before_model_prepare() {
    for field in ["releasePending","running","pending"] {
        let mut page=tab("home","https://chatgpt.com/");match field {"releasePending"=>page[field]=json!(true),"running"=>page["transportHealth"]["tabQueue"][field]=json!({"commandId":"old-command"}),_=>page["transportHealth"]["tabQueue"][field]=json!(["old-command"])};
        let f=fixture(vec![json!({"ok":false,"needsSelection":true,"pendingRequests":0,"pendingCommands":0}),json!({"ok":true,"clients":[page]})]);let env=credential();
        assert_eq!(prepare(&config(&f.endpoint,&env),&request()).unwrap_err().code,"BRIDGE_BUSY","{field}");f.thread.join().unwrap();assert_eq!(f.requests.lock().unwrap().len(),2);std::env::remove_var(env);
    }
}

#[test]
fn chrome_prepare_submit_rechecks_saved_runtime_and_never_sends_after_epoch_or_server_changes() {
    for field in ["backgroundEpoch","serverInstanceId"] {
        let page=recovery_owned();let mut changed=page.clone();changed[field]=json!("changed");let f=fixture(vec![json!({"ok":true,"clients":[changed]})]);let env=credential();let cfg=config(&f.endpoint,&env);
        let mut p=prepared();p.wire["browserBinding"]=json!(cfg.chrome_binding.as_ref().unwrap());
        p.wire["prepareRuntime"]=prepare_intent(&request(),cfg.chrome_binding.as_ref().unwrap(),&page,"new_prepare",Value::Null).unwrap();
        assert_eq!(submit(&cfg,&p).unwrap_err().code,"CHROME_PREPARE_IDENTITY_HELD","{field}");f.thread.join().unwrap();
        assert_eq!(f.requests.lock().unwrap().len(),1);std::env::remove_var(env);
    }
}

fn pending_homepage(status: &str) -> (PreparedRequest, Value, Value) {
    let mut p=prepared();
    let message="synthetic frozen input";
    p.wire=json!({"id":"offline-task","sourceClientId":"worker","message":message,"attachments":["reference-file"],
        "metadata":{"jobId":"offline-task","promptSha256":hex_hash(message.as_bytes())}});
    let mut worker=tab("worker","https://chatgpt.com/");
    worker["launchToken"]=json!("bridge-director-desktop-offline-task");
    if status=="running" {worker["activeRequest"]=json!({"requestId":"offline-task"});}
    let turn=json!({"id":"offline-task","status":status,"createdAt":now(),"input":{
        "sourceClientId":"worker","message":message,"attachments":["reference-file"],"metadata":p.wire["metadata"]}});
    (p,turn,worker)
}

#[test]
fn original_homepage_upload_or_queued_turn_is_processing_without_recovery_or_resubmit() {
    for status in ["running","queued"] {
        let (p,turn,worker)=pending_homepage(status);
        let f=fixture(vec![json!({"ok":true,"turn":turn}),json!({"ok":true,"clients":[worker]})]);
        let env=credential();
        let result=query_with_prepared(&config(&f.endpoint,&env),"offline-task",Some(&p)).unwrap();
        f.thread.join().unwrap();
        assert_eq!(result.status,"processing");
        assert!(result.result_urls.is_empty());
        assert_eq!(result.raw["progress"]["phase"],"preparing_submission");
        assert_eq!(result.raw["turn"],turn,"original receipt is retained");
        let calls=f.requests.lock().unwrap();
        assert_eq!(calls.len(),2);
        assert!(calls.iter().all(|(route,_)|route.starts_with("GET ")));
        std::env::remove_var(env);
    }
}

#[test]
fn homepage_wait_requires_full_frozen_source_task_input_and_ownership() {
    let (p,turn,worker)=pending_homepage("running");
    assert!(homepage_preparation_pending(&turn,&worker,"offline-task",Some(&p)).unwrap().is_some());
    assert!(homepage_preparation_pending(&turn,&worker,"offline-task",None).is_err());
    for field in ["id","status","message","source","attachments","hash","jobId","error","session","createdAt"] {
        let mut changed=turn.clone();
        match field {
            "id" => changed["id"]=json!("other-task"),
            "status" => changed["status"]=json!("unknown"),
            "message" => changed["input"]["message"]=json!("changed prompt"),
            "source" => changed["input"]["sourceClientId"]=json!("another-page"),
            "attachments" => changed["input"]["attachments"]=json!(["different-reference"]),
            "hash" => changed["input"]["metadata"]["promptSha256"]=json!("wrong"),
            "jobId" => changed["input"]["metadata"]["jobId"]=json!("another-job"),
            "error" => changed["error"]=json!({"code":"CANONICAL_DEADLINE_EXCEEDED"}),
            "session" => changed["output"]=json!({"response":{"session":{"id":"original-conversation"}}}),
            "createdAt" => changed["createdAt"]=json!("bad-time"),
            _ => unreachable!(),
        }
        assert!(homepage_preparation_pending(&changed,&worker,"offline-task",Some(&p)).is_err(),"{field}");
    }
    for field in ["provider","kind","requestId","wireId","source","hash","launch"] {
        let mut changed_p=p.clone();let mut changed_worker=worker.clone();
        match field {
            "provider" => changed_p.provider="another-provider".into(),
            "kind" => changed_p.request.kind="video".into(),
            "requestId" => changed_p.request.request_id="another-task".into(),
            "wireId" => changed_p.wire["id"]=json!("another-task"),
            "source" => changed_p.wire["sourceClientId"]=json!("another-page"),
            "hash" => changed_p.wire["metadata"]["promptSha256"]=json!("wrong"),
            "launch" => changed_worker["launchToken"]=json!("bridge-reload-original"),
            _ => unreachable!(),
        }
        assert!(homepage_preparation_pending(&turn,&changed_worker,"offline-task",Some(&changed_p)).is_err(),"{field}");
    }
}

#[test]
fn homepage_wait_has_original_fixed_deadline_and_never_masks_terminal_turns() {
    let (mut p,mut turn,worker)=pending_homepage("queued");
    for status in ["failed","interrupted","cancelled","completed","completed_without_artifact","unknown"] {
        turn["status"]=json!(status);
        assert!(homepage_preparation_pending(&turn,&worker,"offline-task",Some(&p)).is_err(),"{status}");
    }
    turn["status"]=json!("queued");
    for created in [(chrono::Utc::now()-chrono::Duration::minutes(6)).to_rfc3339(),
        (chrono::Utc::now()+chrono::Duration::minutes(1)).to_rfc3339()] {
        turn["createdAt"]=json!(created);
        assert_eq!(homepage_preparation_pending(&turn,&worker,"offline-task",Some(&p)).unwrap_err().code,"IMAGE_PREPARATION_TIMEOUT");
    }
    turn.as_object_mut().unwrap().remove("createdAt");
    assert!(homepage_preparation_pending(&turn,&worker,"offline-task",Some(&p)).unwrap().is_some());
    p.prepared_at=(chrono::Utc::now()-chrono::Duration::minutes(6)).to_rfc3339();
    assert!(homepage_preparation_pending(&turn,&worker,"offline-task",Some(&p)).is_err());
}

#[test]
fn same_runtime_tabs_choose_idle_homepage_without_account_inference() {
    let old = tab("old","https://chatgpt.com/c/previous");
    let home = tab("home","https://chatgpt.com/");
    let mut pages = vec![old,home];
    assert_eq!(crate::chrome_setup::execution_client(&pages).unwrap()["id"],"home");
    pages[0]["backgroundEpoch"] = json!("another-profile-runtime");
    assert!(crate::chrome_setup::execution_client(&pages).is_none());
    pages[0]["selected"] = json!(true);
    assert_eq!(crate::chrome_setup::execution_client(&pages).unwrap()["id"],"old");
    pages[1]["selected"] = json!(true);
    assert!(crate::chrome_setup::execution_client(&pages).is_none());
    for field in ["backgroundEpoch","transport","extensionBundleId","serverInstanceId"] {
        let mut mismatch = vec![tab("a","https://chatgpt.com/"),tab("b","https://chatgpt.com/")];
        mismatch[1][field] = json!("different");
        assert!(crate::chrome_setup::execution_client(&mismatch).is_none(),"{field}");
    }
    let missing = vec![json!({"id":"a","ready":true}),json!({"id":"b","ready":true})];
    assert!(crate::chrome_setup::execution_client(&missing).is_none());
}

#[test]
fn prepare_opens_one_background_owned_tab_and_never_submits() {
    let pages=vec![tab("old","https://chatgpt.com/c/previous"),tab("home","https://chatgpt.com/")];
    let mut worker=tab("worker","https://chatgpt.com/");
    worker["launchToken"]=json!("bridge-director-desktop-offline-task");
    let f=fixture(vec![json!({"ok":false,"needsSelection":true,"pendingRequests":0}),json!({"ok":true,"clients":pages}),
        json!({"ok":true,"clients":pages}),json!({"ok":true,"client":worker}),
        json!({"ok":true,"result":{"sourceClientId":"worker","url":"https://chatgpt.com/",
            "reconciliation":{"source":"browser-empty-preflight","status":"matched_empty","version":1,"proofVersion":2}}})]);
    let env=credential();
    assert_eq!(prepare(&config(&f.endpoint,&env),&request()).unwrap().wire["sourceClientId"],"worker");
    f.thread.join().unwrap();
    let calls=f.requests.lock().unwrap();
    assert_eq!(calls.len(),5);
    assert!(calls[3].0.starts_with("POST /browser/tabs/open "));
    let body:Value=serde_json::from_str(&calls[3].1).unwrap();
    assert_eq!(body["active"],false);assert_eq!(body["select"],false);
    assert_eq!(body["allowSystemFallback"],false);assert_eq!(body["sourceClientId"],"home");
    assert!(calls.iter().all(|(route,_)| !route.contains("/turns")));
    std::env::remove_var(env);
}

#[test]
fn cleanup_only_closes_hidden_owned_original_conversation_with_launch_guard() {
    let mut worker=tab("worker","https://chatgpt.com/c/original-conversation");
    worker["launchToken"]=json!("bridge-director-desktop-offline-task");
    let f=fixture(vec![json!({"ok":true,"clients":[worker,tab("home","https://chatgpt.com/")]}),json!({"ok":true,"closing":true})]);
    let env=credential();
    assert_eq!(release_completed_tab(&config(&f.endpoint,&env),&prepared(),&result()).unwrap()["status"],"close_requested");
    f.thread.join().unwrap();
    let calls=f.requests.lock().unwrap();
    assert_eq!(calls.len(),2);assert!(calls[1].0.starts_with("POST /browser/tabs/close "));
    let body:Value=serde_json::from_str(&calls[1].1).unwrap();
    assert_eq!(body["expectedLaunchToken"],"bridge-director-desktop-offline-task");
    assert_eq!(body["expectedUrl"],"https://chatgpt.com/c/original-conversation");
    assert_eq!(body["sourceClientId"],"worker");std::env::remove_var(env);
}

#[test]
fn new_generation_requires_bound_profile_but_does_not_submit_when_identity_is_missing_or_mismatched() {
    for case in ["legacy","unbound","other-profile","incognito","wrong-open-profile"] {
        let mut home=tab("home","https://chatgpt.com/");
        match case {
            "legacy"=>{home.as_object_mut().unwrap().remove("browserProfileId");},
            "other-profile"=>home["browserProfileId"]=json!("f913ed04-6d73-4f40-bd74-040418327d79"),
            "incognito"=>home["browserIncognito"]=json!(true),
            _=>{},
        }
        let mut responses=vec![json!({"ok":true,"pendingRequests":0}),json!({"ok":true,"clients":[home.clone()]})];
        if ["unbound","wrong-open-profile"].contains(&case){responses.push(json!({"ok":true,"clients":[home]}));}
        if case=="wrong-open-profile" {
            let mut worker=tab("worker","https://chatgpt.com/");worker["launchToken"]=json!("bridge-director-desktop-offline-task");worker["browserProfileId"]=json!("f913ed04-6d73-4f40-bd74-040418327d79");
            responses.push(json!({"ok":true,"client":worker}));
        }
        let f=fixture(responses);let env=credential();let mut cfg=config(&f.endpoint,&env);if case=="unbound"{cfg.chrome_binding=None;}
        assert!(prepare(&cfg,&request()).is_err(),"{case}");f.thread.join().unwrap();
        let calls=f.requests.lock().unwrap();assert!(calls.iter().all(|(route,_)|!route.contains("/turns")),"{case}");
        if case!="wrong-open-profile"{assert!(calls.iter().all(|(route,_)|!route.contains("/tabs/open")),"{case}");}
        std::env::remove_var(env);
    }
}

#[test]
fn cleanup_retains_focused_changed_busy_unowned_or_last_page() {
    for case in ["focused","visible","navigated","busy","reloaded","last"] {
        let mut worker=tab("worker","https://chatgpt.com/c/original-conversation");
        worker["launchToken"]=json!("bridge-director-desktop-offline-task");
        match case {
            "focused" => worker["focused"]=json!(true),
            "visible" => worker["visibilityState"]=json!("visible"),
            "navigated" => worker["url"]=json!("https://chatgpt.com/c/user-new-conversation"),
            "busy" => worker["activeRequest"]=json!({"requestId":"another-request"}),
            "reloaded" => worker["launchToken"]=json!("bridge-reload-new"),
            _ => {},
        }
        let mut pages=vec![worker];if case!="last" {pages.push(tab("home","https://chatgpt.com/"));}
        let f=fixture(vec![json!({"ok":true,"clients":pages})]);let env=credential();
        let mut cfg=config(&f.endpoint,&env);if case=="last"{cfg.chrome_binding=None;}
        let outcome=release_completed_tab(&cfg,&prepared(),&result()).unwrap();
        assert_eq!(outcome["status"],"retained","{case}");
        if case=="reloaded" {assert_eq!(outcome["reason"],"ownership_changed");}
        if case=="busy" {assert_eq!(outcome["reason"],"idle_not_proven");}
        f.thread.join().unwrap();assert_eq!(f.requests.lock().unwrap().len(),1,"{case}");std::env::remove_var(env);
    }
    let env=credential();let cfg=config("http://127.0.0.1:9/v1",&env);
    for field in ["requestId","conversationId","promptSha256","proofVersion"] {
        let mut wrong=result();wrong.raw["nativeProof"][field]=json!("different");
        assert!(release_completed_tab(&cfg,&prepared(),&wrong).is_err(),"{field}");
    }
    std::env::remove_var(env);
}
