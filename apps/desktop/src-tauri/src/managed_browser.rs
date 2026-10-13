//! App-owned Edge lifecycle. Never creates image jobs or changes old bindings.
use crate::store::{s, Result};
use serde_json::{json, Value};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}, process::{Command, Stdio}, time::{Duration, Instant}};

const HELPER: &str = include_str!("../../scripts/managed-browser-bridge.mjs");
const VERSION: &str = "managed-browser/v1";
const OUTPUT_LIMIT: u64 = 128 * 1024;

fn helper_path(home: &Path) -> PathBuf {
    home.join("managed-edge/runtime").join(crate::pipeline::digest(HELPER)).join("managed-browser-bridge.mjs")
}
fn paths_safe(home: &Path) -> bool {
    [home.to_path_buf(),home.join("managed-edge"),home.join("managed-edge/profile"),helper_path(home)]
        .iter().all(|p|crate::runtime_setup::safe_absolute(p))
}
fn installed() -> bool {
    ["ProgramFiles(x86)","ProgramFiles","LOCALAPPDATA"].into_iter().filter_map(std::env::var_os)
        .map(|root|PathBuf::from(root).join("Microsoft/Edge/Application/msedge.exe"))
        .any(|p|crate::runtime_setup::safe_absolute(&p)&&fs::File::open(p).is_ok_and(|mut file|{let mut magic=[0;2];file.read_exact(&mut magic).is_ok()&&magic==*b"MZ"}))
}
fn base_report(home: &Path, state: &str, message: &str) -> Value {
    json!({"schemaVersion":VERSION,"browser":"edge","ok":false,"status":state,"state":state,"message":message,
        "reason":state,"installed":installed(),"componentInstalled":false,"running":false,"connected":false,"authenticated":false,"ready":false,
        "extensionReady":false,"process":{"owned":false,"running":false},
        "profilePath":home.join("managed-edge/profile"),"canLogin":false,"canStart":false,"canRecover":false,
        "newBinding":false,"modelSubmissions":0,"videoSubmissions":0})
}

/// Read every registered project, including archived projects, without running
/// Store migrations. Queued originals are also protected during browser setup.
pub(crate) fn blocker(registry: &Value) -> Result<Option<Value>> {
    let rows=registry.as_array().ok_or("无法核对项目列表，未操作专属浏览器。")?;
    let mut ids=std::collections::HashSet::new();
    for project in rows {
        let id=s(project,"id");let root=Path::new(s(project,"rootPath"));
        if id.is_empty() || !ids.insert(id) || !crate::runtime_setup::safe_absolute(&root.join("project.sqlite")) {
            return Err("项目登记身份或路径无效，未操作专属浏览器。".into());
        }
        let db=rusqlite::Connection::open_with_flags(root.join("project.sqlite"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_|"无法只读核对已有项目，未操作专属浏览器。")?;
        db.busy_timeout(Duration::from_secs(2)).map_err(|e|e.to_string())?;
        let actual:String=db.query_row("SELECT id FROM project LIMIT 1",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if actual!=id {return Err("项目目录与登记身份不一致，未操作专属浏览器。".into());}
        let mut query=db.prepare("SELECT id,state,task_id,snapshot,prepared,result,receipt FROM jobs WHERE provider='chrome' AND kind='image' ORDER BY created_at,rowid").map_err(|e|e.to_string())?;
        let records=query.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?))).map_err(|e|e.to_string())?;
        for row in records {
            let (job,state,task,snapshot,prepared,result,receipt)=row.map_err(|e|e.to_string())?;
            let parse=|v:Option<String>|v.and_then(|s|serde_json::from_str::<Value>(&s).ok()).unwrap_or(Value::Null);
            let candidate=json!({"kind":"image","provider":"chrome","status":state,"providerTaskId":task,
                "snapshot":parse(Some(snapshot)),"prepared":parse(prepared),"result":parse(result)});
            let active=["queued","ready","prepared","waiting_dependency","preparing","submitting","submitted","processing","succeeded","downloading","submission_unknown","query_failed","download_failed","recovery_paused","auth_required"].contains(&state.as_str());
            let original=task.as_deref().is_some_and(|v|!v.is_empty()) || receipt.as_deref().is_some_and(|v|!["","null","{}"].contains(&v));
            let unresolved=original && !["downloaded","closed_by_user"].contains(&state.as_str()) && !crate::chrome_terminal::settled_job(&candidate);
            if active||unresolved {return Ok(Some(json!({"projectId":id,"jobId":job,"status":state,"unknown":state=="submission_unknown"})));}
        }
    }
    Ok(None)
}
pub(crate) fn ensure_clear(registry: &Value) -> Result<()> {
    if let Some(job)=blocker(registry)? {
        return Err(format!("MANAGED_BROWSER_JOBS_HELD: 项目 {} 的原网页图片任务 {}（{}）尚未结束；保留原队列与浏览器，未启动或切换执行端。",s(&job,"projectId"),s(&job,"jobId"),s(&job,"status")));
    }
    Ok(())
}
fn prepare_helper(home: &Path, write: bool) -> Result<Option<PathBuf>> {
    if !paths_safe(home) {return Err("专属浏览器目录含外链或非法路径，未启动。".into());}
    let path=helper_path(home);
    if path.exists() {
        let meta=fs::metadata(&path).map_err(|e|e.to_string())?;
        if !meta.is_file() || meta.len()!=HELPER.len() as u64 || fs::read(&path).map_err(|e|e.to_string())?!=HELPER.as_bytes() {
            return Err("专属浏览器运行组件指纹不一致，未执行。".into());
        }
        return Ok(Some(path));
    }
    if !write {return Ok(None);}
    fs::create_dir_all(path.parent().unwrap()).map_err(|e|e.to_string())?;
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|e.to_string())?;
    file.write_all(HELPER.as_bytes()).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;
    Ok(Some(path))
}
fn helper_action(action: &str) -> Result<&'static str> {
    match action {"chrome.managed.status"=>Ok("status"),"chrome.managed.start"=>Ok("start-background"),
        "chrome.managed.login"=>Ok("open-login"),"chrome.managed.recover"=>Ok("recover"),_=>Err("未知专属浏览器操作。".into())}
}
fn helper_request(home: &Path, action: &str, component: &Path, connection: &Value) -> Result<Value> {
    let operation=helper_action(action)?;
    let mut request=json!({"schemaVersion":"managed-browser-request/v1","action":operation,"home":home,"componentHome":component,"connection":connection});
    if operation!="status" {request["safety"]=json!({"verified":true,"activeJobs":0,"unknownJobs":0});}
    if operation=="open-login" {request["visibleLogin"]=json!(true);}
    Ok(request)
}
fn helper_input(path: Option<&Path>, request: &Value) -> Result<Vec<u8>> {
    let request=serde_json::to_string(request).map_err(|e|e.to_string())?;
    if path.is_some(){return Ok(request.into_bytes());}
    // Read-only status after an upgrade must inspect the existing instance
    // even before the new SHA-bound helper has been materialized. The embedded
    // module and request travel over stdin; secrets never become process args.
    Ok(format!("{HELPER}\nprocess.stdout.write(JSON.stringify(await run({request})) + '\\n');\n").into_bytes())
}
fn run_helper(path: Option<&Path>, node: &Path, request: &Value) -> Result<Value> {
    let mut cmd=Command::new(node);cmd.env_clear();
    for key in ["PATH","SystemRoot","WINDIR","USERPROFILE","APPDATA","LOCALAPPDATA","TEMP","TMP","ProgramFiles","ProgramFiles(x86)","ProgramW6432"] {if let Some(value)=std::env::var_os(key){cmd.env(key,value);}}
    if let Some(path)=path{cmd.arg(path);}else{cmd.arg("--input-type=module");}
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)] {use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
    // The browser is an intentionally persistent, separately owned process;
    // do not put it in a kill-on-helper-exit process tree.
    let mut child=cmd.spawn().map_err(|_|"无法启动专属浏览器管理组件。")?;
    let bytes=helper_input(path,request)?;
    if child.stdin.take().ok_or("管理组件输入不可用。")?.write_all(&bytes).is_err(){let _=child.kill();let _=child.wait();return Err("管理组件请求未传输完成；未自动重试。".into());}
    let output=child.stdout.take().ok_or("管理组件输出不可用。")?;
    let reader=std::thread::spawn(move||{let mut bytes=Vec::new();let result=output.take(OUTPUT_LIMIT+1).read_to_end(&mut bytes);(result,bytes)});
    let deadline=Instant::now()+Duration::from_secs(45);
    loop {
        match child.try_wait() {
            Ok(Some(_))=>break,
            Ok(None) if Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(50)),
            _=>{let _=child.kill();let _=child.wait();return Err("专属浏览器管理超时；保留已有专属进程，先检查原状态，未重复启动。".into());}
        }
    }
    let (read,bytes)=reader.join().map_err(|_|"无法读取专属浏览器管理结果。")?;
    read.map_err(|_|"无法读取专属浏览器管理结果。")?;
    if bytes.len()>OUTPUT_LIMIT as usize{return Err("专属浏览器管理结果超出限制。".into());}
    serde_json::from_slice(&bytes).map_err(|_|"专属浏览器返回格式无效；未自动重试。".into())
}
fn public_report(home: &Path, result: &Value) -> Result<Value> {
    if result["schemaVersion"]!=VERSION || result["newBinding"]!=false || result["modelSubmissions"]!=0 || result["videoSubmissions"]!=0 {
        return Err("专属浏览器返回契约不符，未采用执行身份。".into());
    }
    let running=result["process"]["running"]==true&&result["process"]["owned"]==true;
    let connected=running&&result["connected"]==true&&result["extensionReady"]==true;
    let authenticated=connected&&result["authenticated"]==true;
    let profile=s(result,"profileId");let identity=uuid::Uuid::parse_str(profile).is_ok_and(|v|v.get_version_num()==4&&v.to_string()==profile);
    let ready=installed()&&result["state"]=="ready"&&result["ok"]==true&&result["ready"]==true&&authenticated&&identity&&result["requiresLogin"]!=true;
    let mut report=base_report(home,s(result,"state"),s(result,"message"));
    report["reason"]=json!(s(result,"reason"));
    report["extensionReady"]=json!(running&&result["extensionReady"]==true);
    report["process"]=json!({"owned":result["process"]["owned"]==true,"running":running});
    if let Some(pid)=result["process"]["pid"].as_u64().filter(|pid|*pid>0&&*pid<=u32::MAX as u64){report["process"]["pid"]=json!(pid);}
    for (key,value) in [("ok",json!(result["ok"]==true)),("componentInstalled",json!(true)),("running",json!(running)),("connected",json!(connected)),("authenticated",json!(authenticated)),("ready",json!(ready)),("requiresLogin",json!(result["requiresLogin"]==true)),("browserMode",result["browserMode"].clone())] {report[key]=value;}
    if identity&&connected {report["profileId"]=json!(profile);}
    for flag in ["canStart","canLogin","canRecover"] {report[flag]=json!(report["installed"]==true);}
    Ok(report)
}
pub(crate) fn perform(home: &Path, settings: &Value, action: &str, token: Option<&str>) -> Result<Value> {
    let operation=helper_action(action)?;
    if !paths_safe(home){return Err("专属浏览器路径不安全。".into());}
    let connection=token.ok_or("请先连接并核验现有本机浏览器桥接。".into()).and_then(|token|crate::chrome_setup::managed_connection(home,settings,token));
    let (component,connection)=match connection {Ok(value)=>value,Err(error) if operation=="status"=>return Ok(base_report(home,"bridge_unverified",&error)),Err(error)=>return Err(error)};
    let helper=prepare_helper(home,operation!="status")?;
    let node=crate::chrome_setup::node(settings)?;
    let result=run_helper(helper.as_deref(),&node,&helper_request(home,action,&component,&connection)?)?;
    public_report(home,&result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn managed_browser_status_never_materializes_runtime_or_profile(){
        let temp=tempfile::tempdir().unwrap();let home=temp.path().join("absent");
        assert!(prepare_helper(&home,false).unwrap().is_none());assert!(!home.exists());
        let report=perform(&home,&json!({}),"chrome.managed.status",None).unwrap();
        assert_eq!(report["status"],"bridge_unverified");assert!(!home.exists());
    }
    #[test] fn managed_browser_visibility_only_explicit_login_and_fixed_profile(){
        let temp=tempfile::tempdir().unwrap();let conn=json!({"endpoint":"http://127.0.0.1:1"});
        for action in ["status","start","login","recover"] {
            let request=helper_request(temp.path(),&format!("chrome.managed.{action}"),temp.path(),&conn).unwrap();
            assert_eq!(request["visibleLogin"]==true,action=="login");assert!(request.get("profileDir").is_none());
            assert_eq!(request["safety"]["verified"]==true,action!="status");
        }
        assert!(helper_action("chrome.managed.stop").is_err());
    }
    #[test] fn managed_browser_public_contract_needs_independent_auth_and_identity(){
        let home=tempfile::tempdir().unwrap();let mut raw=json!({"schemaVersion":VERSION,"ok":true,"state":"ready","ready":true,"newBinding":false,"modelSubmissions":0,"videoSubmissions":0,
            "process":{"owned":true,"running":true},"connected":true,"extensionReady":true,"profileId":"11111111-1111-4111-8111-111111111111","apiToken":"must-never-leak"});
        assert_eq!(public_report(home.path(),&raw).unwrap()["ready"],false);
        raw["authenticated"]=json!(true);let report=public_report(home.path(),&raw).unwrap();assert_eq!(report["ready"],installed());assert!(!report.to_string().contains("must-never-leak"));
        assert_eq!(report["process"]["owned"],true);assert_eq!(report["extensionReady"],true);
        raw["requiresLogin"]=json!(true);assert_eq!(public_report(home.path(),&raw).unwrap()["ready"],false);raw["requiresLogin"]=json!(false);
        raw["profileId"]=json!("other");assert_eq!(public_report(home.path(),&raw).unwrap()["ready"],false);
        raw["newBinding"]=json!(true);assert!(public_report(home.path(),&raw).is_err());
    }
    #[test] fn managed_browser_runtime_fingerprint_is_verified_before_execution(){
        let temp=tempfile::tempdir().unwrap();let path=prepare_helper(temp.path(),true).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(),HELPER);fs::write(&path,"modified").unwrap();assert!(prepare_helper(temp.path(),false).is_err());
    }
    #[test] fn managed_browser_unmaterialized_status_uses_current_embedded_code_without_writes(){
        let temp=tempfile::tempdir().unwrap();let home=temp.path().join("uncreated");
        assert!(prepare_helper(&home,false).unwrap().is_none());
        let request=json!({"action":"status","home":home,"connection":{"bridgeToken":"quote\"\\\n-test"}});
        // Invalid input exits before browser/process/network discovery. Running
        // the actual embedded module catches stdin/module-entrypoint regressions.
        let report=run_helper(None,Path::new("node"),&request).unwrap();
        assert_eq!(report["errorCode"],"INVALID_REQUEST");
        assert_eq!(report["modelSubmissions"],0);assert!(!report.to_string().contains("quote"));
        assert!(!home.exists());
    }
    #[test] fn managed_browser_base_report_always_matches_ui_contract(){
        let home=tempfile::tempdir().unwrap();
        let report=base_report(home.path(),"not_started","首次登录");
        for key in ["ok","installed","running","connected","authenticated","ready","extensionReady"] {assert!(report[key].is_boolean(),"{key}");}
        assert!(report["reason"].is_string());assert!(report["message"].is_string());
        assert_eq!(report["process"],json!({"owned":false,"running":false}));
    }
    #[test] fn managed_browser_unfinished_original_and_archived_projects_are_held(){
        let home=tempfile::tempdir().unwrap();let root=home.path().join("project");
        let store=crate::store::Store::open(root.clone()).unwrap();store.create("fixture","project").unwrap();
        let registry=json!([{"id":"project","rootPath":root,"archivedAt":"2026-10-01"}]);
        assert!(ensure_clear(&registry).is_ok());
        let job=store.add_job("image","asset","chrome","account",&json!({}),"submission_unknown").unwrap();
        assert_eq!(blocker(&registry).unwrap().unwrap()["jobId"],job);
        store.conn().unwrap().execute("UPDATE jobs SET state='downloaded' WHERE id=?1",[&job]).unwrap();
        assert!(ensure_clear(&registry).is_ok());
        store.conn().unwrap().execute("UPDATE jobs SET state='queued' WHERE id=?1",[&job]).unwrap();
        assert!(ensure_clear(&registry).is_err());
        let wrong=json!([{"id":"wrong","rootPath":root}]);assert!(ensure_clear(&wrong).is_err());
    }
}
