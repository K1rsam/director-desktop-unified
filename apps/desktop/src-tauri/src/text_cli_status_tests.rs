//! Actual local subprocess/RPC fixtures. They never load credentials or send a turn.
use super::*;
const SOURCE:&str=r##"
use std::{io::{BufRead,Write},fs::{self,OpenOptions},time::Duration};
fn main(){
 let root=std::env::current_exe().unwrap().parent().unwrap().to_owned();
 let scenario=fs::read_to_string(root.join("scenario")).unwrap();let args=std::env::args().skip(1).collect::<Vec<_>>();
 if args==["--version"]{println!("codex-cli 0.162.0-fixture");return;}
 if args==["exec","--help"]{println!("--json --output-schema --output-last-message --ignore-user-config --ignore-rules --ephemeral");return;}
 assert_eq!(args.first().unwrap(),"app-server");
 let mut count=1;while OpenOptions::new().write(true).create_new(true).open(root.join(format!("spawn-{count}"))).is_err(){count+=1;}
 fs::write(root.join(format!("pid-{count}")),std::process::id().to_string()).unwrap();
 for line in std::io::stdin().lock().lines(){let line=line.unwrap();
  let method=line.split("\"method\":\"").nth(1).unwrap().split('"').next().unwrap();
  writeln!(OpenOptions::new().create(true).append(true).open(root.join("calls")).unwrap(),"{count} {method}").unwrap();
  assert!(["initialize","initialized","account/read","model/list","account/rateLimits/read"].contains(&method));
  if method=="initialized"{continue;}
  let id=line.split("\"id\":").nth(1).unwrap().chars().take_while(|c|c.is_ascii_digit()).collect::<String>();
  let hang=(scenario=="init-timeout"&&method=="initialize")||(scenario=="account-timeout"&&method=="account/read")
   ||(method=="model/list"&&(scenario=="catalog-timeout"||scenario=="null-email-timeout"||scenario=="api-key-timeout"||((scenario=="catalog-retry"||scenario=="account-drift"||scenario=="raw-account-drift")&&count==1)||(scenario=="partial"&&id!="10")))
   ||(scenario=="quota-timeout"&&method=="account/rateLimits/read");
  if hang{std::thread::sleep(Duration::from_secs(10));continue;}
  if scenario=="disconnect"&&method=="model/list"&&count==1{return;}
  if scenario=="server-refusal"&&method=="model/list"{println!("{{\"id\":{id},\"error\":{{\"code\":-32000,\"message\":\"synthetic refusal\"}}}}");std::io::stdout().flush().unwrap();continue;}
  let result=match method {
   "account/read"=>match scenario.as_str(){
    "account-empty"=>r#"{"account":{}}"#,
    "account-missing-email"=>r#"{"account":{"type":"chatgpt","planType":"plus"}}"#,
    "account-missing-type"=>r#"{"account":{"email":"original@example.test","planType":"plus"}}"#,
    "account-null-email"|"null-email-timeout"=>r#"{"account":{"type":"chatgpt","email":null,"planType":"plus"}}"#,
    "account-api-key"|"api-key-timeout"=>r#"{"account":{"type":"apiKey"}}"#,
    "raw-account-drift" if count==1=>r#"{"account":{"type":"chatgpt","email":"original@example.test","planType":"plus","extensionField":"PRIVATE_RAW_A"}}"#,
    "raw-account-drift"=>r#"{"account":{"type":"chatgpt","email":"original@example.test","planType":"plus","extensionField":"PRIVATE_RAW_B"}}"#,
    "account-drift" if count==2=>r#"{"account":{"type":"chatgpt","email":"changed@example.test","planType":"plus"}}"#,
    _=>r#"{"account":{"type":"chatgpt","email":"original@example.test","planType":"plus"}}"#,
   },
   "model/list"=>{if scenario=="coalesced"{std::thread::sleep(Duration::from_millis(150));}if scenario=="partial"||scenario=="cursor"{r#"{"data":[{"model":"gpt-fixture","supportedReasoningEfforts":[{"reasoningEffort":"medium"}]}],"nextCursor":"more"}"#}else{r#"{"data":[{"model":"gpt-fixture","supportedReasoningEfforts":[{"reasoningEffort":"medium"}]}],"nextCursor":null}"#}},
   _=>"{}",
  };
  println!("{{\"id\":{id},\"result\":{result}}}");std::io::stdout().flush().unwrap();
 }
}
"##;
fn fixture(scenario:&str)->(tempfile::TempDir,PathBuf) {
    static COMPILED:OnceLock<(tempfile::TempDir,PathBuf)>=OnceLock::new();
    let (_,binary)=COMPILED.get_or_init(||{
        let temp=tempfile::tempdir().unwrap();let source=temp.path().join("status_fixture.rs");std::fs::write(&source,SOURCE).unwrap();
        let binary=temp.path().join(if cfg!(windows){"status_fixture.exe"}else{"status_fixture"});
        let compiler=std::env::var_os("RUSTC").map(PathBuf::from).unwrap_or_else(||{
            std::env::var_os("CARGO_HOME").map(PathBuf::from)
                .or_else(||std::env::var_os(if cfg!(windows){"USERPROFILE"}else{"HOME"}).map(|p|PathBuf::from(p).join(".cargo")))
                .map(|p|p.join("bin").join(if cfg!(windows){"rustc.exe"}else{"rustc"})).unwrap_or_else(||PathBuf::from("rustc"))});
        let mut command=std::process::Command::new(compiler);command.args(["--edition=2021","--crate-name","status_fixture"]).arg(&source).arg("-o").arg(&binary);
        #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
        let result=command.output().unwrap();assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));(temp,binary)
    });
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join(if cfg!(windows){"codex.exe"}else{"codex"});
    std::fs::copy(binary,&path).unwrap();std::fs::write(temp.path().join("scenario"),scenario).unwrap();(temp,path)
}
fn budgets()->StatusBudgets {StatusBudgets{initialize:Duration::from_millis(500),account:Duration::from_millis(500),catalog:Duration::from_millis(200),quota:Duration::from_millis(100)}}
fn calls(root:&Path)->String {std::fs::read_to_string(root.join("calls")).unwrap_or_default()}
fn assert_stopped(root:&Path) {
    #[cfg(windows)] {
        #[link(name="kernel32")]
        unsafe extern "system" {fn OpenProcess(access:u32,inherit:i32,pid:u32)->*mut std::ffi::c_void;fn GetExitCodeProcess(handle:*mut std::ffi::c_void,code:*mut u32)->i32;fn CloseHandle(handle:*mut std::ffi::c_void)->i32;}
        for file in std::fs::read_dir(root).unwrap().map(|r|r.unwrap()).filter(|r|r.file_name().to_string_lossy().starts_with("pid-")) {
            let pid=std::fs::read_to_string(file.path()).unwrap().parse().unwrap();unsafe {let process=OpenProcess(0x1000,0,pid);if !process.is_null(){let mut code=259;assert_ne!(GetExitCodeProcess(process,&mut code),0);CloseHandle(process);assert_ne!(code,259,"inspection child {pid} remains active");}}
        }
    }
    assert!(calls(root).lines().all(|line|["initialize","initialized","account/read","model/list","account/rateLimits/read"].contains(&line.split_once(' ').unwrap().1)));
}
#[tokio::test]
async fn cli_readonly_status_catalog_retry_rechecks_same_account_and_stops_both_children() {
    for scenario in ["catalog-retry","disconnect"] {
        let(temp,path)=fixture(scenario);let value=read_status_with_budgets(&path,budgets()).await.unwrap();
        assert_eq!(value["ready"],true,"{value}");assert_eq!(value["inspection"]["attempts"],2);assert_eq!(value["generationRequestsSent"],0);
        assert!(validate_selection(&value,"gpt-fixture","medium").is_ok());assert!(validate_selection(&value,"missing","medium").is_err());
        assert_eq!(calls(temp.path()).lines().filter(|line|line.ends_with(" account/read")).count(),2);assert_stopped(temp.path());
    }
}
#[tokio::test]
async fn cli_readonly_status_catalog_timeout_preserves_account_but_never_partial_ready() {
    for scenario in ["catalog-timeout","partial"] {
        let(temp,path)=fixture(scenario);let value=read_status_with_budgets(&path,budgets()).await.unwrap();
        assert_eq!(value["ready"],false);assert_eq!(value["loggedIn"],true);assert_eq!(value["version"],"codex-cli 0.162.0-fixture");
        assert_eq!(value["account"]["email"],"original@example.test");assert_eq!(value["models"],json!([]));
        assert!(s(&value,"error").starts_with("CLI_STATUS_CATALOG_TIMEOUT:"));assert_eq!(value["inspection"]["attempts"],2);assert_stopped(temp.path());
    }
}
#[tokio::test]
async fn cli_readonly_status_phase_errors_identity_drift_and_server_refusal_remain_closed() {
    for (scenario,code,attempts) in [("init-timeout","CLI_STATUS_INITIALIZE_TIMEOUT:",2),("account-timeout","CLI_STATUS_ACCOUNT_TIMEOUT:",2),("account-drift","CLI_STATUS_ACCOUNT_CHANGED:",2),("cursor","CLI_STATUS_CATALOG_CURSOR:",1),("server-refusal","Codex CLI 暂时无法读取此项",1)] {
        let(temp,path)=fixture(scenario);let value=read_status_with_budgets(&path,budgets()).await.unwrap();
        assert_eq!(value["ready"],false,"{value}");assert!(s(&value,"error").starts_with(code),"{value}");assert_eq!(value["inspection"]["attempts"],attempts);assert!(value["models"].as_array().unwrap().is_empty());assert_stopped(temp.path());
    }
}
#[tokio::test]
async fn cli_readonly_status_optional_quota_timeout_keeps_verified_catalog() {
    let(temp,path)=fixture("quota-timeout");let value=read_status_with_budgets(&path,budgets()).await.unwrap();
    assert_eq!(value["ready"],true,"{value}");assert_eq!(value["inspection"]["attempts"],1);assert_eq!(value["limits"],json!([]));
    assert_eq!(value["warnings"].as_array().unwrap().len(),1);assert_stopped(temp.path());
}
#[tokio::test]
async fn cli_readonly_status_concurrent_refreshes_share_one_fresh_probe_and_cache_has_short_failure_age() {
    let(temp,path)=fixture("coalesced");let settings=json!({"codexPath":path});
    let(a,b,c)=tokio::join!(status(&settings,true),status(&settings,true),status(&settings,false));
    assert_eq!(a["ready"],true,"{a}");assert_eq!(a["checkedAt"],b["checkedAt"]);assert_eq!(a["checkedAt"],c["checkedAt"]);
    assert_eq!(calls(temp.path()).lines().filter(|line|line.ends_with(" initialize")).count(),1);assert_stopped(temp.path());
    let sha=crate::store::hash_file(&path).unwrap().0;let failed=json!({"ready":false});
    *CACHE.get().unwrap().lock().unwrap()=Some((path.clone(),sha.clone(),Instant::now()-Duration::from_secs(3),failed));
    assert!(cached_status(&path,&sha,false,Instant::now()).is_none());
    *CACHE.get().unwrap().lock().unwrap()=Some((path.clone(),sha.clone(),Instant::now(),a));
    assert!(cached_status(&path,"changed-executable-sha",false,Instant::now()).is_none());
}
#[tokio::test]
async fn cli_readonly_status_malformed_accounts_never_become_logged_in_or_read_models() {
    for scenario in ["account-empty","account-missing-type","account-missing-email"] {
        let(temp,path)=fixture(scenario);let value=read_status_with_budgets(&path,budgets()).await.unwrap();
        assert_eq!(value["ready"],false);assert_eq!(value["loggedIn"],false);assert_eq!(value["account"],Value::Null);
        assert_eq!(value["inspection"]["attempts"],1);assert!(s(&value,"error").starts_with("CLI_STATUS_ACCOUNT_INVALID:"));
        assert!(!calls(temp.path()).contains("model/list"));assert_stopped(temp.path());
    }
    for account in [json!({"type":"unknown"}),json!({"type":"chatgpt","email":"","planType":"plus"}),json!({"type":"chatgpt","email":null}),json!({"type":"amazonBedrock"})] {
        assert!(!valid_status_account(&account));
    }
    assert!(valid_status_account(&json!({"type":"amazonBedrock","usesCodexManagedCredentials":false})));
    assert!(valid_status_account(&json!({"type":"gcp"})));
}
#[tokio::test]
async fn cli_readonly_status_valid_unidentified_accounts_work_once_but_never_claim_retry_identity() {
    for scenario in ["account-null-email","account-api-key"] {
        let(temp,path)=fixture(scenario);let value=read_status_with_budgets(&path,budgets()).await.unwrap();
        assert_eq!(value["ready"],true,"{value}");assert_eq!(value["loggedIn"],true);assert_eq!(value["inspection"]["attempts"],1);assert_stopped(temp.path());
    }
    for scenario in ["null-email-timeout","api-key-timeout"] {
        let(temp,path)=fixture(scenario);let value=read_status_with_budgets(&path,budgets()).await.unwrap();
        assert_eq!(value["ready"],false);assert_eq!(value["loggedIn"],true);assert_eq!(value["inspection"]["attempts"],1);
        assert!(s(&value,"error").starts_with("CLI_STATUS_ACCOUNT_IDENTITY_UNVERIFIABLE:"));
        assert_eq!(calls(temp.path()).lines().filter(|line|line.ends_with(" initialize")).count(),1);assert_stopped(temp.path());
    }
}
#[tokio::test]
async fn cli_readonly_status_retry_compares_complete_account_without_exporting_private_fields() {
    let(temp,path)=fixture("raw-account-drift");let value=read_status_with_budgets(&path,budgets()).await.unwrap();
    assert_eq!(value["ready"],false);assert_eq!(value["inspection"]["attempts"],2);
    assert!(s(&value,"error").starts_with("CLI_STATUS_ACCOUNT_CHANGED:"));
    assert!(!value.to_string().contains("PRIVATE_RAW"));assert!(!value.to_string().contains("extensionField"));
    assert_eq!(calls(temp.path()).lines().filter(|line|line.ends_with(" model/list")).count(),1,"second account mismatch stops before catalog");assert_stopped(temp.path());
}
