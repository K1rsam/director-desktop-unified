//! Profile identity fixtures only; no real browser, account, or project is touched.
use super::*;
const PROFILE: &str="b84d4521-56ba-4f84-9948-090f82d639e7";
const OTHER: &str="f913ed04-6d73-4f40-bd74-040418327d79";
fn page(id: &str, profile: &str, epoch: &str) -> Value {
    json!({"id":id,"url":"https://chatgpt.com/","ready":true,"transport":"extension","compatible":true,
        "browserProfileId":profile,"browserIncognito":false,"backgroundEpoch":epoch,"extensionBundleId":"bundle","serverInstanceId":"server"})
}
fn binding() -> BrowserBinding {BrowserBinding{profile_id:PROFILE.into(),endpoint:"http://127.0.0.1:53179".into(),bundle_id:"bundle".into()}}
fn connection(id:&str)->Value {let mut c=page(id,PROFILE,"epoch");c["launchToken"]=json!("");c["contentEpoch"]=json!("content");c["focused"]=json!(false);c["visibilityState"]=json!("hidden");c["activeRequest"]=Value::Null;
    for key in ["pageReady","composerReady","chatMainReady"]{c[key]=json!(true);}c}
#[test] fn chrome_connection_page_is_transport_only_and_does_not_rebind_or_rewrite_the_old_controller(){
    let mut legacy=page("legacy-control",PROFILE,"epoch");legacy["launchToken"]=json!(format!("bridge-director-control-{PROFILE}"));legacy["pageReady"]=json!(false);legacy["focused"]=json!(false);legacy["visibilityState"]=json!("hidden");
    let pages=vec![legacy.clone(),connection("dedicated")];let before=json!(pages);
    assert_eq!(profile_execution_client(&pages,&binding()).unwrap()["id"],"dedicated");assert_eq!(json!(pages),before);
}
#[test] fn chrome_connection_page_requires_real_ready_hidden_same_profile_and_a_unique_home(){
    let c=connection("dedicated");assert!(connection_page(&c,&binding()));
    for (key,value) in [("launchToken",json!("owned")),("pageReady",json!(false)),("composerReady",json!(false)),("chatMainReady",json!(false)),("focused",json!(true)),("visibilityState",json!("visible")),("url",json!("https://chatgpt.com/c/other")),("browserProfileId",json!(OTHER)),("activeRequest",json!({"requestId":"old"})),("contentEpoch",json!("")),("transportHealth",json!({"backgroundOnly":true}))]{let mut bad=c.clone();bad[key]=value;assert!(!connection_page(&bad,&binding()),"{key}");}
    assert!(profile_execution_client(&[c,connection("second")],&binding()).is_err());
}
#[test] fn chrome_controller_accepts_root_queries_without_accepting_conversations_or_other_origins(){
    let mut control=page("control",PROFILE,"epoch");
    control["launchToken"]=json!(format!("bridge-director-control-{PROFILE}"));
    control["focused"]=json!(false);control["visibilityState"]=json!("hidden");
    for url in ["https://chatgpt.com/","https://chatgpt.com/?ref=desktop#home","https://chatgpt.com/?__cf_chl_rt_tk=fixture"] {
        control["url"]=json!(url);assert!(profile_execution_client(&[control.clone()],&binding()).is_ok(),"{url}");
    }
    for url in ["https://chatgpt.com/c/fixture","https://chatgpt.com/auth/login","http://chatgpt.com/","https://chatgpt.com:444/","https://chatgpt.com.evil.test/","https://user@chatgpt.com/","about:blank"] {
        control["url"]=json!(url);assert!(profile_execution_client(&[control.clone()],&binding()).is_err(),"{url}");
    }
}
#[test] fn chrome_hello_snapshot_is_history_but_release_and_untyped_queue_still_hold(){
    let mut client=json!({"transportHealth":{"tabQueue":{"running":true,"pending":1}}});assert!(client_command_busy(&client));
    client["transportHealth"]["kind"]=json!("connect_snapshot");assert!(!client_command_busy(&client));
    client["releasePending"]=json!(true);assert!(client_command_busy(&client));
}
#[test] fn chrome_bound_controller_wins_over_selected_user_page_and_cannot_be_bypassed_when_busy(){
    let mut user=page("user",PROFILE,"epoch");user["selected"]=json!(true);
    let mut control=page("control",PROFILE,"epoch");control["launchToken"]=json!(format!("bridge-director-control-{PROFILE}"));control["focused"]=json!(false);control["visibilityState"]=json!("hidden");
    assert_eq!(profile_execution_client(&[user.clone(),control.clone()],&binding()).unwrap()["id"],"control");
    for field in ["releasePending","focused","activeRequest"] {let mut bad=control.clone();bad[field]=if field=="activeRequest" {json!({"requestId":"other"})}else{json!(true)};assert!(profile_execution_client(&[user.clone(),bad],&binding()).is_err(),"{field}");}
    assert!(profile_execution_client(&[user,control.clone(),control],&binding()).is_err());
}
#[test] fn chrome_binding_tracks_profile_across_new_tabs_and_worker_epochs_without_switching_profiles() {
    let mut other=page("other",OTHER,"other-epoch");other["selected"]=json!(true);
    for epoch in ["before-restart","after-restart"] {
        let pages=vec![page("new-home",PROFILE,epoch),page("new-other",PROFILE,epoch),other.clone()];
        let chosen=profile_execution_client(&pages,&binding()).unwrap();
        assert_eq!(chosen["browserProfileId"],PROFILE);assert_ne!(chosen["id"],"other");
    }
    assert!(profile_execution_client(&[other],&binding()).is_err());
}
#[test] fn chrome_binding_refuses_uuid_collisions_hidden_incognito_legacy_and_wrong_bundle() {
    let mut busy_duplicate=page("copy",PROFILE,"another-epoch");busy_duplicate["activeRequest"]=json!({"id":"old-task"});
    assert!(profile_execution_client(&[page("one",PROFILE,"epoch"),busy_duplicate],&binding()).is_err());
    for (field,value) in [("browserIncognito",json!(true)),("browserProfileId",json!("")),("extensionBundleId",json!("other-bundle")),("backgroundEpoch",json!("")),("serverInstanceId",json!(""))] {
        let mut wrong=page("one",PROFILE,"epoch");wrong[field]=value;
        assert!(profile_execution_client(&[wrong],&binding()).is_err(),"{field}");
    }
}
#[test] fn chrome_binding_auto_suggestion_is_unique_complete_and_manual_choice_checks_collision() {
    let settings=json!({"chromeEndpoint":binding().endpoint});
    let one=profiles(&[page("one",PROFILE,"epoch"),page("two",PROFILE,"epoch")]);
    let observed=json!({"browserProfiles":one,"unidentifiedClients":0});
    let patch=binding_patch(&settings,&observed,None).unwrap().unwrap();assert_eq!(patch["chromeProfileId"],PROFILE);
    assert!(binding_patch(&settings,&json!({"browserProfiles":profiles(&[page("a",PROFILE,"a"),page("b",OTHER,"b")]),"unidentifiedClients":0}),None).unwrap().is_none());
    assert!(binding_patch(&settings,&json!({"browserProfiles":observed["browserProfiles"],"unidentifiedClients":1}),None).unwrap().is_none());
    assert!(binding_patch(&settings,&json!({"browserProfiles":profiles(&[page("a",PROFILE,"a"),page("copy",PROFILE,"b")]),"unidentifiedClients":0}),Some(PROFILE)).is_err());
    assert!(binding_patch(&settings,&observed,Some(OTHER)).is_err());
    assert!(binding_patch(&binding_settings(&binding().endpoint,Some(&binding())),&observed,None).unwrap().is_none());
}
#[test] fn chrome_binding_global_roundtrip_does_not_change_old_settings_and_rejects_partial_endpoint_switch() {
    let home=tempfile::tempdir().unwrap();let doc=crate::app_settings::initialize(home.path(),&json!([]),None).unwrap();
    let old=json!({"chromeEndpoint":"http://127.0.0.1:53179","sourceClientId":"original-tab","originalPrepared":{"task":"old"}});
    let patch=json!({"chromeProfileId":PROFILE,"chromeProfileEndpoint":binding().endpoint,"chromeProfileBundleId":"bundle"});
    let saved=crate::app_settings::save(home.path(),&patch,doc["revision"].as_u64().unwrap()).unwrap();
    let reopened=crate::app_settings::load(home.path()).unwrap().unwrap();assert_eq!(saved,reopened);
    assert_eq!(old["originalPrepared"]["task"],"old");assert!(old.get("chromeProfileId").is_none());
    let effective=crate::app_settings::effective_from(&saved,&old);assert_eq!(browser_binding(&effective).unwrap().unwrap().profile_id,PROFILE);
    assert!(browser_binding(&json!({"chromeEndpoint":"http://127.0.0.1:9999","chromeProfileId":PROFILE,"chromeProfileEndpoint":binding().endpoint,"chromeProfileBundleId":"bundle"})).is_err());
    assert!(crate::app_settings::save(home.path(),&json!({"chromeProfileId":OTHER}),saved["revision"].as_u64().unwrap()).is_err());
    assert!(browser_binding(&old).unwrap().is_none());
}
