//! Explicit synthetic, image-only acceptance through the standard desktop Engine.
//! Requires an absolute isolated home and the caller's free-web-image authorization.
use director_core::{Engine,store::{s,atomic_json}};
use serde_json::{json,Value};use std::{path::PathBuf,time::{Duration,Instant}};
fn main()->Result<(),Box<dyn std::error::Error>>{
    let args:Vec<_>=std::env::args().collect();if !(3..=4).contains(&args.len())||args[2]!="--allow-free-web-images"{return Err("usage: HOME --allow-free-web-images [EXISTING_APP_HOME_FOR_SAVED_BROWSER]".into());}
    let home=PathBuf::from(&args[1]);if !home.is_absolute()||!home.to_string_lossy().contains("opencli-native-acceptance"){return Err("isolated acceptance home required".into());}
    let marker=home.join("probe.json");let engine=Engine::new_standard(home.clone())?;
    let mut probe:Value=if marker.is_file(){serde_json::from_slice(&std::fs::read(&marker)?)?}else{
        let global=engine.dispatch(json!({"action":"settings.global.get"}))?;
        let mut settings=global["settings"].clone();settings["textSdkNodePath"]=json!("C:/Program Files/nodejs/node.exe");
        if let Some(source)=args.get(3) {
            let source=PathBuf::from(source);if !source.is_absolute()||!source.join("director-next-home.json").is_file(){return Err("existing application home required".into());}
            let saved:Value=serde_json::from_slice(&std::fs::read(source.join("app-settings.json"))?)?;
            for key in ["chromeTransport","opencliProfileId","opencliAccountKey"] {settings[key]=saved["settings"][key].clone();}
            if settings["chromeTransport"]!="opencli"{return Err("saved OpenCLI binding required".into());}
        }
        engine.dispatch(json!({"action":"settings.global.save","expectedRevision":global["revision"],"settings":settings}))?;
        engine.dispatch(json!({"action":"chrome.connect","scope":"global","transport":"opencli"}))?;
        let status=engine.dispatch(json!({"action":"chrome.check","scope":"global"}))?;
        assert_eq!(status["schemaVersion"],"chrome-setup/v1");assert_eq!(status["configured"],true);assert_eq!(status["status"],"ready");
        atomic_json(&home.join("connection-status.json"),&status)?;
        engine.dispatch(json!({"action":"providers.refresh","scope":"global","provider":"chrome"}))?;
        let project=engine.dispatch(json!({"action":"projects.create","title":"OpenCLI 网页接口验收"}))?;
        let v=json!({"version":"opencli-desktop-acceptance/v1","projectId":project["id"],"queued":false,"allowVideo":false});atomic_json(&marker,&v)?;v
    };
    let pid=s(&probe,"projectId").to_owned();let store=engine.store(&pid)?;
    for summary in store.job_summaries()? {
        let job=store.job(s(&summary,"id"))?;
        let original=home.join("opencli-requests").join(s(&job["snapshot"],"requestId")).join("request.json");
        if job["status"]=="failed" && s(&job,"providerTaskId").is_empty() && job["prepared"].is_null() && !original.exists() {
            let evidence:bool=store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type='chrome_prepare_error' AND json_extract(data,'$.generationCalled')=0 AND json_extract(data,'$.error.code')='OPENCLI_OWNED_PAGE_REQUIRED')",[s(&job,"id")],|r|r.get(0))?;
            if evidence {store.event(s(&job,"id"),"opencli_pre_send_recovered",&json!({"sameRequestId":job["snapshot"]["requestId"],"priorPrepareNeverSubmitted":true}))?;store.update_job(s(&job,"id"),"queued",&json!({"error":null,"nextAttemptAt":0}))?;}
        }
    }
    if probe["queued"]!=true {
        store.save_settings(&json!({"imageProvider":"chrome","imageModel":"chatgpt-image","imageResolution":"native","ratio":"16:9","trialApproved":true,"requireImageTrialReview":false}))?;
        for a in [json!({"id":"probe-robot","baseAssetId":"probe-robot","variantId":"base","kind":"CH","name":"角色-蓝色小机器人","prompt":"One cute compact blue robot toy, round head, amber circular eyes, silver joints, full body three-quarter front view, plain pale gray studio background, stylized 3D animation rendering, 16:9, no letters or watermark.","promptOrigin":"MANUAL"}),
            json!({"id":"probe-robot-scarf","baseAssetId":"probe-robot","variantId":"red-scarf","sourceAssetId":"probe-robot","kind":"CH","name":"角色-蓝色小机器人红围巾","prompt":"Edit the attached direct-parent robot reference. Preserve the blue robot identity, face, body, proportions and camera exactly; add a bright red knitted scarf around its neck. Full body, same pale gray studio background, 16:9, no writing or watermark.","promptOrigin":"MANUAL"})] {
            engine.dispatch(json!({"action":"assets.save","projectId":pid,"asset":a}))?;
        }
        // A durable guard is written before the only queue call. On an interruption it
        // resumes existing jobs; it never queues again based on a missing response.
        probe["queued"]=json!(true);atomic_json(&marker,&probe)?;
        let result=engine.dispatch(json!({"action":"images.run","projectId":pid,"provider":"chrome","model":"chatgpt-image","ratio":"16:9","resolution":"native","assetIds":["probe-robot","probe-robot-scarf"],"trial":false}))?;
        println!("{}",json!({"queue":result["imageQueueReport"]}));
    }
    let start=Instant::now();loop{
        engine.tick()?;std::thread::sleep(Duration::from_secs(3));
        let jobs=store.job_summaries()?;let brief:Vec<_>=jobs.iter().filter(|j|j["kind"]=="image").map(|j|json!({"id":j["id"],"asset":j["entityId"],"state":j["status"],"error":j["error"]})).collect();
        println!("{}",json!({"elapsedSeconds":start.elapsed().as_secs(),"jobs":brief}));
        if brief.len()==2&&brief.iter().all(|j|j["state"]=="downloaded") {break;}
        if start.elapsed()>Duration::from_secs(600)||brief.iter().any(|j|["submission_unknown","failed","auth_required","blocked","stale"].contains(&s(j,"state"))) {atomic_json(&home.join("result.json"),&json!({"passed":false,"jobs":brief,"noResubmission":true}))?;return Err("acceptance requires original-result inspection; no resend".into());}
    }
    assert!(engine.await_channel_shutdown(15));drop(engine);
    let reopened=Engine::new_standard(home.clone())?;let workspace=reopened.dispatch(json!({"action":"workspace.get","projectId":pid}))?;
    let jobs=workspace["jobs"].as_array().ok_or("jobs missing")?;assert_eq!(jobs.iter().filter(|j|j["kind"]=="image"&&j["status"]=="downloaded").count(),2);
    let assets=workspace["assets"].as_array().ok_or("assets missing")?;assert!(assets.iter().all(|a|!s(a,"adoptedMediaId").is_empty()));
    let report=json!({"passed":true,"projectId":pid,"images":2,"desktopQueue":true,"directParentDependency":true,"saveReopen":true,"videoSubmissions":0,"jobs":jobs,"assets":assets});atomic_json(&home.join("result.json"),&report)?;println!("{}",json!({"passed":true,"images":2,"saveReopen":true}));Ok(())
}
