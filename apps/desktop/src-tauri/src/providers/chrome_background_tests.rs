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
