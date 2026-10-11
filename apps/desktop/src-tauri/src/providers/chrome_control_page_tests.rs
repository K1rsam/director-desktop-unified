//! Local HTTP fixtures for controller-only opens; no browser or generation.
use super::*;
use super::super::wire_tests::fixture;
const PROFILE:&str="b84d4521-56ba-4f84-9948-090f82d639e7";
fn page(id:&str,url:&str)->Value {json!({"id":id,"url":url,"ready":true,"compatible":true,"transport":"extension","browserProfileId":PROFILE,"browserIncognito":false,"extensionBundleId":"bundle","backgroundEpoch":"epoch","serverInstanceId":"server","focused":false,"visibilityState":"hidden","activeRequest":null})}
fn worker()->Value {let mut p=page("worker","https://chatgpt.com/c/original");p["launchToken"]=json!("bridge-director-desktop-offline-task");p}
fn control()->Value {let mut p=page("control","https://chatgpt.com/");p["launchToken"]=json!(format!("bridge-director-control-{PROFILE}"));p}
fn cfg(endpoint:&str)->ProviderConfig {
    let env=format!("DIRECTOR_CONTROLLER_FIXTURE_{}",uuid::Uuid::new_v4().simple());std::env::set_var(&env,"synthetic-only");
    serde_json::from_value(json!({"kind":"chrome","endpoint":endpoint.trim_end_matches("/v1"),"bridgeTokenEnv":env,
        "chromeBinding":{"profileId":PROFILE,"bundleId":"bundle","endpoint":endpoint.trim_end_matches("/v1")}})).unwrap()
}
fn prepared(config:&ProviderConfig)->PreparedRequest {serde_json::from_value(json!({"provider":"chrome","request":{"requestId":"offline-task","kind":"image","model":"chatgpt-image","prompt":"synthetic","ratio":"16:9","resolution":"native","references":[],"options":{}},"capabilityVersion":"fixture","uploadReceipts":[],"wire":{"sourceClientId":"worker","browserBinding":config.chrome_binding,"metadata":{"promptSha256":"hash"}},"preparedAt":now()})).unwrap()}
fn result()->QueryResult {serde_json::from_value(json!({"taskId":"offline-task","status":"succeeded","resultUrls":["original"],"raw":{"nativeProof":{"status":"matched_complete","proofVersion":2,"requestId":"offline-task","promptSha256":"hash","conversationId":"original"}}})).unwrap()}
fn list(pages:Vec<Value>)->Value {json!({"ok":true,"clients":pages})}
fn opened(page:Value)->Value {json!({"ok":true,"client":page,"tabState":{"active":false,"autoDiscardable":false,"discarded":false}})}
#[test] fn chrome_control_last_work_page_creates_one_background_root_and_closes_only_original() {
    let mut heartbeat=worker();heartbeat["lastSeenAt"]=json!(now());heartbeat["selected"]=json!(false);
    let f=fixture(vec![list(vec![worker()]),list(vec![heartbeat.clone()]),opened(control()),list(vec![heartbeat,control()]),json!({"ok":true,"closing":true}),list(vec![control()])]);
    let cfg=cfg(&f.endpoint);let p=prepared(&cfg);let r=result();
    assert_eq!(release_completed_tab(&cfg,&p,&r).unwrap()["status"],"close_requested");
    assert_eq!(release_completed_tab(&cfg,&p,&r).unwrap()["status"],"already_closed");
    f.thread.join().unwrap();let calls=f.requests.lock().unwrap();
    let opens:Vec<_>=calls.iter().filter(|(route,_)|route.starts_with("POST /browser/tabs/open ")).collect();assert_eq!(opens.len(),1);
    let body:Value=serde_json::from_str(&opens[0].1).unwrap();assert_eq!(body["url"],"https://chatgpt.com/");assert_eq!(body["sourceClientId"],"worker");
    assert_eq!(body["active"],false);assert_eq!(body["select"],false);assert_eq!(body["allowSystemFallback"],false);assert_eq!(body["launchToken"],format!("bridge-director-control-{PROFILE}"));
    let close:Value=serde_json::from_str(&calls[4].1).unwrap();assert_eq!(close["sourceClientId"],"worker");
    assert!(calls.iter().all(|(route,_)|!route.contains("/turns")&&!route.contains("/upload")&&!route.contains("recover-latest")));
    std::env::remove_var(cfg.bridge_token_env.unwrap());
}
#[test] fn chrome_control_reuses_existing_profile_control_without_touching_user_page() {
    for is_control in [true,false] {
        let mut other=if is_control {control()} else {page("user","https://chatgpt.com/c/user")};other["focused"]=json!(true);other["visibilityState"]=json!("visible");
        let f=fixture(vec![list(vec![worker(),other]),json!({"ok":true,"closing":true})]);let cfg=cfg(&f.endpoint);
        assert_eq!(release_completed_tab(&cfg,&prepared(&cfg),&result()).unwrap()["status"],"close_requested");f.thread.join().unwrap();
        let calls=f.requests.lock().unwrap();assert_eq!(calls.len(),2);assert!(calls[1].0.contains("/tabs/close"));assert!(!calls[1].1.contains("user"));std::env::remove_var(cfg.bridge_token_env.unwrap());
    }
}
#[test] fn chrome_control_unready_duplicate_unknown_idle_or_runtime_collision_never_opens() {
    for case in ["unready","busy","duplicate","collision","source-idle-unknown","source-profile","binding"] {
        let mut source=worker();let mut other=control();let mut pages=vec![];
        match case {
            "unready"=>{other["ready"]=json!(false);pages.push(other);},
            "busy"=>{other["activeRequest"]=json!({"requestId":"busy"});pages.push(other);},
            "duplicate"=>{pages.push(other.clone());other["id"]=json!("duplicate");pages.push(other);},
            "collision"=>{other["backgroundEpoch"]=json!("copied-profile");pages.push(other);},
            "source-idle-unknown"=>{source.as_object_mut().unwrap().remove("activeRequest");},
            "source-profile"=>source["browserProfileId"]=json!("f913ed04-6d73-4f40-bd74-040418327d79"),
            _=>{},
        }
        pages.push(source);let f=fixture(vec![list(pages)]);let mut cfg=cfg(&f.endpoint);if case=="binding"{cfg.chrome_binding=None;}
        let out=release_completed_tab(&cfg,&prepared(&cfg),&result()).unwrap();assert_eq!(out["status"],"retained","{case}");
        f.thread.join().unwrap();assert_eq!(f.requests.lock().unwrap().len(),1,"{case}");std::env::remove_var(cfg.bridge_token_env.unwrap());
    }
}
#[test] fn chrome_control_open_requires_real_tab_state_and_exact_source_runtime() {
    for case in ["profile","bundle","epoch","server","url","ready","active","discardable","discarded","unknown-state","source-changed"] {
        let mut reply=opened(control());
        match case {
            "profile"=>reply["client"]["browserProfileId"]=json!("f913ed04-6d73-4f40-bd74-040418327d79"),
            "bundle"=>reply["client"]["extensionBundleId"]=json!("other"),
            "epoch"=>reply["client"]["backgroundEpoch"]=json!("other"),
            "server"=>reply["client"]["serverInstanceId"]=json!("other"),
            "url"=>reply["client"]["url"]=json!("https://chatgpt.com/c/user"),
            "ready"=>reply["client"]["ready"]=json!(false),
            "active"=>reply["tabState"]["active"]=json!(true),
            "discardable"=>reply["tabState"]["autoDiscardable"]=json!(true),
            "discarded"=>reply["tabState"]["discarded"]=json!(true),
            "unknown-state"=>{reply.as_object_mut().unwrap().remove("tabState");},
            _=>{},
        }
        let mut replies=vec![list(vec![worker()]),list(vec![worker()]),reply];
        if case=="source-changed"{let mut changed=worker();changed["url"]=json!("https://chatgpt.com/c/user");replies.push(list(vec![changed,control()]));}
        let f=fixture(replies);let cfg=cfg(&f.endpoint);assert_eq!(release_completed_tab(&cfg,&prepared(&cfg),&result()).unwrap()["status"],"retained","{case}");
        f.thread.join().unwrap();assert!(f.requests.lock().unwrap().iter().all(|(route,_)|!route.contains("/tabs/close")&&!route.contains("/turns")),"{case}");std::env::remove_var(cfg.bridge_token_env.unwrap());
    }
}
