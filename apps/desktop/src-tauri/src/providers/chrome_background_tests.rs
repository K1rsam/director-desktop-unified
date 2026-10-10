//! Real local HTTP fixtures only. These tests never contact a logged-in browser.
use super::*;
use super::super::wire_tests::fixture;

fn tab(id: &str, url: &str) -> Value {
    json!({"id":id,"url":url,"ready":true,"compatible":true,"transport":"extension",
        "backgroundEpoch":"one-profile-runtime","extensionBundleId":"one-extension",
        "serverInstanceId":"one-server","focused":false,"visibilityState":"hidden"})
}
fn config(endpoint: &str, env: &str) -> ProviderConfig {
    ProviderConfig { kind:"chrome".into(),endpoint:Some(endpoint.trim_end_matches("/v1").into()),
        bridge_token_env:Some(env.into()),credential_env:None,cli_path:None,timeout_seconds:5,adapted_models:vec![] }
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
        assert_eq!(release_completed_tab(&config(&f.endpoint,&env),&prepared(),&result()).unwrap()["status"],"retained","{case}");
        f.thread.join().unwrap();assert_eq!(f.requests.lock().unwrap().len(),1,"{case}");std::env::remove_var(env);
    }
    let env=credential();let cfg=config("http://127.0.0.1:9/v1",&env);
    for field in ["requestId","conversationId","promptSha256","proofVersion"] {
        let mut wrong=result();wrong.raw["nativeProof"][field]=json!("different");
        assert!(release_completed_tab(&cfg,&prepared(),&wrong).is_err(),"{field}");
    }
    std::env::remove_var(env);
}
