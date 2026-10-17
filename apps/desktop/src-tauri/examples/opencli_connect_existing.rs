//! Explicit channel setup only. Never ticks workers or queues generation.
use director_core::{Engine,engine::background_chrome_images::HomeExecutionLease};use serde_json::json;use std::path::PathBuf;
fn main()->Result<(),Box<dyn std::error::Error>>{
    if director_core::windows_runtime::ensure_unvirtualized()?{return Ok(());}
    let args:Vec<_>=std::env::args().filter(|s|s!="--unvirtualized-desktop").collect();if args.len()!=2{return Err("application home required".into());}
    let home=PathBuf::from(&args[1]);if !home.is_absolute()||!home.join("director-next-home.json").is_file(){return Err("existing desktop home required".into());}
    let _lease=HomeExecutionLease::acquire(&home)?;let before=std::fs::read(home.join("projects.json"))?;
    let backup=home.join("opencli-channel-before.json");if !backup.exists(){std::fs::copy(home.join("app-settings.json"),&backup)?;}
    let engine=Engine::new_standard(home.clone())?;
    engine.dispatch(json!({"action":"chrome.connect","scope":"global","transport":"opencli"}))?;
    let models=engine.dispatch(json!({"action":"providers.refresh","scope":"global","provider":"chrome"}))?;
    let status=engine.dispatch(json!({"action":"chrome.check","scope":"global"}))?;
    assert_eq!(before,std::fs::read(home.join("projects.json"))?,"channel setup cannot replace project registry");
    director_core::store::atomic_json(&home.join("opencli-channel-acceptance.json"),&json!({"version":"opencli-channel-acceptance/v1","status":status["status"],"transport":status["transport"],"models":models["models"],"projectRegistryUnchanged":true,"generationSubmissions":0,"videoSubmissions":0}))?;Ok(())
}
