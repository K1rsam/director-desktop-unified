//! Real synthetic native executables; no host account, provider, or model access.
use super::*;
use std::process::Command;

fn fixture_compiler(
    explicit: Option<std::ffi::OsString>,
    cargo_home: Option<std::ffi::OsString>,
    user_home: Option<std::ffi::OsString>,
) -> PathBuf {
    let executable = if cfg!(windows) { "rustc.exe" } else { "rustc" };
    if let Some(explicit) = explicit {
        // An explicit compiler is authoritative, including a broken override.
        return PathBuf::from(explicit);
    }
    cargo_home
        .map(PathBuf::from)
        .into_iter()
        .chain(user_home.map(|home| PathBuf::from(home).join(".cargo")))
        .map(|home| home.join("bin").join(executable))
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(executable))
}

#[test]
fn fixture_compiler_keeps_explicit_override_and_resolves_user_shim_without_path_changes() {
    let home = tempfile::tempdir().unwrap();
    let cargo = tempfile::tempdir().unwrap();
    let executable = if cfg!(windows) { "rustc.exe" } else { "rustc" };
    let user_shim = home.path().join(".cargo").join("bin").join(executable);
    fs::create_dir_all(user_shim.parent().unwrap()).unwrap();
    fs::write(&user_shim, b"synthetic compiler identity").unwrap();
    let home_value = Some(home.path().as_os_str().to_owned());
    assert_eq!(fixture_compiler(None, None, home_value.clone()), user_shim);
    let cargo_shim = cargo.path().join("bin").join(executable);
    fs::create_dir_all(cargo_shim.parent().unwrap()).unwrap();
    fs::write(&cargo_shim, b"explicit cargo home compiler").unwrap();
    assert_eq!(fixture_compiler(None, Some(cargo.path().as_os_str().to_owned()), home_value.clone()), cargo_shim);
    let unavailable = home.path().join("explicit-missing-compiler");
    assert_eq!(fixture_compiler(Some(unavailable.as_os_str().to_owned()), Some(cargo.path().as_os_str().to_owned()), home_value), unavailable);
    assert_eq!(fixture_compiler(None, None, None), PathBuf::from(executable));
}

pub(super) fn native_fixture(dir: &Path, kind: &str) -> PathBuf {
    let path = dir.join(if cfg!(windows) {
        format!("{kind}-fixture.exe")
    } else {
        format!("{kind}-fixture")
    });
    let source = dir.join(format!("{kind}.rs"));
    let code=r#"
use std::io::{BufRead,Write};
fn note(value:&str) {std::fs::OpenOptions::new().create(true).append(true).open(__LOG__).unwrap().write_all(format!("{}\n",value).as_bytes()).unwrap();}
fn main() {
 let a:Vec<String>=std::env::args().skip(1).collect();
 if __KIND__.starts_with("dreamina") {
  if a==["version"] {note("version");if __KIND__=="dreamina-version-failure" {std::process::exit(2);}if __KIND__=="dreamina-log-denied" {eprintln!("initialize logs failed: initialize file logger: open /PRIVATE_SENTINEL/.dreamina_cli/logs: operation not permitted");}if __KIND__=="dreamina-build-json-mismatch" {println!("{}",__BAD_BUILD__);}else if __KIND__.starts_with("dreamina-build-json") {println!("{}",__BUILD__);}else{println!("1.4.19");}return;}
  if a==["--help"] {note("--help");println!("Usage:\n  dreamina [command]\nAvailable Commands:\n login\n user_credit\n query_result\n text2image\n image2image\n text2video\n multimodal2video\nFlags:\n --version");return;}
  if a.len()==2 && a[1]=="-h" && ["multimodal2video","text2video","text2image","image2image"].contains(&a[0].as_str()) {note(&format!("{} -h",a[0]));if __KIND__=="dreamina-build-json-wrong-help" {println!("Usage:\n  dreamina-canvas [command]\n --model --ratio --resolution");return;}println!("Usage:\n  dreamina {} [flags]\nFlags:\n --model_version 5.0Pro --ratio 16:9 --resolution_type 2k --video_resolution 720p --duration 5 --prompt synthetic --image --video --audio",a[0]);return;}
  if a==["user_credit"] {note("user_credit");println!("{{\"data\":{{\"user_id\":\"PRIVATE_SENTINEL\",\"total_credit\":42,\"gift_credit\":7}}}}");return;}
 } else {
  if a==["--version"] {note("version");if __KIND__=="codex-cold-start" {std::thread::sleep(std::time::Duration::from_millis(3500));}if __KIND__=="codex-output-flood" {for _ in 0..2048 {std::io::stdout().write_all(&[b'x';1024]).unwrap();}return;}println!("codex-cli synthetic-readiness");return;}
  if a==["exec","--help"] {note("exec-help");println!("--json --output-schema --output-last-message --ignore-user-config --ignore-rules --ephemeral");return;}
  if a.first().map(String::as_str)==Some("app-server") {
   note("app-server");
   if __KIND__=="codex-rpc-failure" {note("rpc-unavailable");return;}
   if __KIND__=="codex-rpc-flood" {note("rpc-flood");for _ in 0..2048 {std::io::stdout().write_all(&[b'x';1024]).unwrap();}return;}
   for line in std::io::stdin().lock().lines() {
    let line=line.unwrap();
    if line.contains("\"initialize\"") {note("initialize");println!("{{\"id\":1,\"result\":{{}}}}");}
    else if line.contains("account/read") {assert!(line.contains("\"refreshToken\":false"));note("account/read:no-refresh");println!("{{\"id\":2,\"result\":{{\"account\":{{\"type\":\"chatgpt\",\"email\":\"PRIVATE_SENTINEL\",\"planType\":\"synthetic\"}}}}}}");}
    else if line.contains("model/list") {note("model/list");println!("{{\"id\":10,\"result\":{{\"data\":[{{\"model\":\"gpt-6.1-sol\",\"isDefault\":true,\"defaultReasoningEffort\":\"medium\",\"supportedReasoningEfforts\":[{{\"reasoningEffort\":\"medium\"}}]}}],\"nextCursor\":null}}}}");}
    else if line.contains("account/rateLimits/read") {note("account/rateLimits/read");println!("{{\"id\":30,\"result\":{{\"rateLimitsByLimitId\":{{}}}}}}");}
    else if !line.contains("\"initialized\"") {note("FORBIDDEN_RPC");std::process::exit(92);}
    std::io::stdout().flush().unwrap();
   }
   return;
  }
 }
 note("FORBIDDEN_COMMAND");std::process::exit(91);
}
"#.replace("__LOG__",&json!(dir.join(format!("{kind}-calls.txt"))).to_string()).replace("__KIND__",&json!(kind).to_string())
    .replace("__BUILD__", &json!(json!({"version":"1d0bd17-dirty","commit":"1d0bd17","build_time":"2026-10-03T13:51:54Z"}).to_string()).to_string())
    .replace("__BAD_BUILD__", &json!(json!({"version":"1d0bd17-dirty","commit":"deadbee","build_time":"2026-10-03T13:51:54Z"}).to_string()).to_string());
    fs::write(&source, code).unwrap();
    // Explorer-context native runs do not inherit the packaged host's temporary
    // PATH. Resolve the existing user rustup shim; never install a compiler or
    // alter the process/global PATH just to make the fixture pass.
    let compiler = fixture_compiler(
        std::env::var_os("RUSTC"),
        std::env::var_os("CARGO_HOME"),
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }),
    );
    let output = Command::new(compiler)
        .args(["--edition=2021", "--crate-name", "readiness_fixture"])
        .arg(&source)
        .arg("-o")
        .arg(&path)
        .output()
        .unwrap_or_else(|error| panic!("synthetic native compiler could not start: {error}"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}
fn enabled_engine(home: PathBuf) -> Arc<Engine> {
    let mut engine = Engine::new(home).unwrap();
    // Test-only private state; no product flag, environment variable or IPC enables this.
    Arc::get_mut(&mut engine).unwrap().readiness_enabled = true;
    engine
}
#[test]
fn readiness_codex_cold_native_start_reads_only_and_preserves_the_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let path = native_fixture(dir.path(), "codex-cold-start");
    let engine = enabled_engine(dir.path().join("home"));
    let project = engine.dispatch(json!({"action":"projects.create","title":"synthetic cold launch"})).unwrap();
    let start = std::time::Instant::now();
    let r = engine.dispatch(json!({"action":"readiness.check","provider":"codex_cli","projectId":project["id"],"settings":{"codexPath":path}})).unwrap();
    assert_eq!(r["installation"]["state"], "ready", "{r}");
    assert_eq!(r["protocol"]["state"], "ready", "{r}");
    assert_eq!(r["generation"]["calls"], 0);
    assert!(start.elapsed() < std::time::Duration::from_secs(25));
    assert!(engine.store(s(&project,"id")).unwrap().jobs().unwrap().is_empty());
    let calls = fs::read_to_string(dir.path().join("codex-cold-start-calls.txt")).unwrap();
    assert!(!calls.contains("FORBIDDEN"));
}
#[test]
fn explicit_codex_check_uses_only_read_rpc_preserves_settings_and_never_unlocks_generation() {
    let dir = tempfile::tempdir().unwrap();
    let path = native_fixture(dir.path(), "codex");
    let engine = enabled_engine(dir.path().join("home"));
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"synthetic readiness"}))
        .unwrap();
    let store = engine.store(s(&project, "id")).unwrap();
    let before = store.settings().unwrap();
    let result=engine.dispatch(json!({"action":"readiness.check","projectId":project["id"],"provider":"codex_cli","settings":{"codexPath":path}})).unwrap();
    assert_eq!(result["schemaVersion"], crate::readiness::VERSION);
    assert_eq!(result["installation"]["state"], "ready", "{result}");
    assert_eq!(result["protocol"]["state"], "ready");
    assert_eq!(result["authentication"]["state"], "authenticated");
    assert_eq!(result["authentication"]["identityVerified"], true);
    assert_eq!(result["details"]["accountChecked"], true);
    assert_eq!(result["catalog"]["state"], "available");
    assert_eq!(result["network"]["state"], "not_tested");
    assert_eq!(result["generation"]["calls"], 0);
    assert!(!result.to_string().contains("PRIVATE_SENTINEL"));
    assert_eq!(store.settings().unwrap(), before);
    assert!(store.jobs().unwrap().is_empty());
    let policy = engine
        .dispatch(json!({"action":"getExecutionPolicy"}))
        .unwrap();
    assert_eq!(policy["liveExecutionAllowed"], false);
    assert_eq!(policy["readiness"]["enabled"], true);
    let log = fs::read_to_string(dir.path().join("codex-calls.txt")).unwrap();
    assert!(log.contains("account/read:no-refresh"));
    assert!(log.contains("model/list"));
    assert!(!log.contains("FORBIDDEN"));
    assert!(engine
        .dispatch(json!({"action":"foundation.run","projectId":project["id"]}))
        .unwrap_err()
        .contains("OFFLINE_EXECUTION_BLOCKED"));
    assert_eq!(
        fs::read_to_string(dir.path().join("codex-calls.txt")).unwrap(),
        log
    );
}
#[test]
fn dreamina_check_does_not_poll_oauth_resume_tasks_cache_catalogs_or_claim_free_images() {
    let dir = tempfile::tempdir().unwrap();
    let path = native_fixture(dir.path(), "dreamina");
    let engine = enabled_engine(dir.path().join("home"));
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"synthetic readiness"}))
        .unwrap();
    let store = engine.store(s(&project, "id")).unwrap();
    let job = store
        .add_job(
            "image",
            "synthetic",
            "dreamina",
            "original-account",
            &json!({"prompt":"synthetic"}),
            "auth_required",
        )
        .unwrap();
    let before = store.job(&job).unwrap();
    let settings = store.settings().unwrap();
    let result=engine.dispatch(json!({"action":"readiness.check","projectId":project["id"],"provider":"dreamina","settings":{"dreaminaPath":path}})).unwrap();
    assert_eq!(result["authentication"]["state"], "authenticated");
    assert_eq!(result["details"]["credits"]["total"], 42.0);
    assert_eq!(result["details"]["freeImageEligibility"], "unverified");
    assert_eq!(result["generation"]["calls"], 0);
    assert!(!result.to_string().contains("PRIVATE_SENTINEL"));
    assert_eq!(store.job(&job).unwrap(), before);
    assert_eq!(store.settings().unwrap(), settings);
    let log = fs::read_to_string(dir.path().join("dreamina-calls.txt")).unwrap();
    assert_eq!(log.lines().filter(|l| *l == "user_credit").count(), 1);
    assert!(!log.contains("FORBIDDEN"));
    assert!(log.lines().all(|l| [
        "version",
        "--help",
        "multimodal2video -h",
        "text2video -h",
        "text2image -h",
        "image2image -h",
        "user_credit"
    ]
    .contains(&l)));
}

#[test]
fn codex_account_verification_survives_public_identity_redaction_without_generation_claims() {
    let status = json!({"available":true,"version":"codex-cli 0.162.0-alpha.2",
        "readinessRpcReady":true,"loggedIn":true,"ready":true,
        "account":{"type":"chatgpt","planType":"pro","email":"PRIVATE_ACCOUNT_EMAIL","id":"PRIVATE_ACCOUNT_ID"},
        "models":[{"id":"synthetic-model"}],"limits":[],"generationRequestsSent":0});
    let report = crate::readiness::codex_report(&status, &json!({}));
    assert_eq!(report["authentication"]["state"], "authenticated");
    assert_eq!(report["authentication"]["identityVerified"], true);
    assert_eq!(report["details"]["accountChecked"], true);
    assert_eq!(report["details"]["accountIdentityNotExported"], true);
    assert_eq!(report["details"]["cliStatus"]["account"]["planType"], "pro");
    for secret in ["PRIVATE_ACCOUNT_EMAIL", "PRIVATE_ACCOUNT_ID"] { assert!(!report.to_string().contains(secret)); }
    assert_eq!(report["generation"]["enabled"], false);
    assert_eq!(report["generation"]["testPerformed"], false);
    assert_eq!(report["generation"]["calls"], 0);
    assert_eq!(report["network"]["state"], "not_tested");
    assert!(report["messages"].to_string().contains("不是套餐权限或真实模型调用证明"));
    for (field, value) in [
        ("available", json!(false)), ("version", json!("unknown")),
        ("readinessRpcReady", json!(false)), ("readinessRpcReady", Value::Null),
        ("loggedIn", json!(false)), ("account", Value::Null), ("account", json!({})),
    ] {
        let mut incomplete = status.clone(); incomplete[field] = value;
        let report = crate::readiness::codex_report(&incomplete, &json!({}));
        assert_eq!(report["authentication"]["identityVerified"], false, "{field}");
        assert_eq!(report["generation"]["calls"], 0);
    }
    let version_only = crate::readiness::codex_report(&json!({"available":true,"version":"codex-cli 0.162.0-alpha.2"}), &json!({}));
    assert_eq!(version_only["authentication"]["identityVerified"], false);
    assert_eq!(version_only["details"]["accountChecked"], false);
}
#[test]
fn legacy_build_json_readiness_preserves_original_job_and_settings_and_uses_only_read_probes() {
    let dir = tempfile::tempdir().unwrap();
    let path = native_fixture(dir.path(), "dreamina-build-json");
    let engine = enabled_engine(dir.path().join("home"));
    let project = engine.dispatch(json!({"action":"projects.create","title":"Synthetic Windows build readiness"})).unwrap();
    let store = engine.store(s(&project, "id")).unwrap();
    let job = store.add_job("image", "synthetic", "dreamina", "original-account", &json!({"prompt":"synthetic"}), "auth_required").unwrap();
    let before_job = store.job(&job).unwrap();
    let before_settings = store.settings().unwrap();
    let report = engine.dispatch(json!({"action":"readiness.check","provider":"dreamina","projectId":project["id"],"settings":{"dreaminaPath":path}})).unwrap();
    assert_eq!(report["installation"]["version"], "1d0bd17-dirty");
    assert_eq!(report["protocol"]["state"], "ready", "{report}");
    assert_eq!(report["authentication"]["state"], "authenticated");
    assert_eq!(report["catalog"]["state"], "available");
    assert_eq!(report["generation"]["calls"], 0);
    assert_eq!(report["generation"]["enabled"], false);
    assert!(!report.to_string().contains("PRIVATE_SENTINEL"));
    assert_eq!(store.job(&job).unwrap(), before_job);
    assert_eq!(store.settings().unwrap(), before_settings);
    assert_eq!(fs::read_to_string(dir.path().join("dreamina-build-json-calls.txt")).unwrap(),
        "version\n--help\nmultimodal2video -h\ntext2video -h\ntext2image -h\nimage2image -h\nuser_credit\n");
}
#[test]
fn legacy_build_json_requires_matching_commit_and_old_command_help_before_account_probe() {
    for (kind, expected) in [("dreamina-build-json-mismatch", "version\n"),
        ("dreamina-build-json-wrong-help", "version\n--help\nmultimodal2video -h\n")] {
        let dir = tempfile::tempdir().unwrap();
        let path = native_fixture(dir.path(), kind);
        let engine = enabled_engine(dir.path().join("home"));
        let project = engine.dispatch(json!({"action":"projects.create","title":"Synthetic protocol mismatch"})).unwrap();
        let report = engine.dispatch(json!({"action":"readiness.check","provider":"dreamina","projectId":project["id"],"settings":{"dreaminaPath":path}})).unwrap();
        assert_eq!(report["protocol"]["state"], "unsupported", "{report}");
        assert_eq!(report["authentication"]["state"], "not_checked");
        assert_eq!(report["generation"]["calls"], 0);
        assert_eq!(fs::read_to_string(dir.path().join(format!("{kind}-calls.txt"))).unwrap(), expected);
        assert!(engine.store(s(&project, "id")).unwrap().jobs().unwrap().is_empty());
    }
}
#[test]
fn isolated_startup_and_unsupported_readiness_requests_fail_before_any_probe() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new_offline_verification(dir.path().join("probe")).unwrap();
    let policy = engine
        .dispatch(json!({"action":"getExecutionPolicy"}))
        .unwrap();
    assert_eq!(policy["readiness"]["enabled"], false);
    assert!(engine
        .dispatch(
            json!({"action":"readiness.check","provider":"codex_cli","projectId":"does-not-exist"})
        )
        .unwrap_err()
        .contains("READINESS_NATIVE_ONLY"));
    for provider in ["api", "bh", "unknown"] {
        assert!(
            crate::readiness::request_settings(&json!({"provider":provider}), &json!({})).is_err()
        );
    }
    for field in [
        "textApiKey",
        "sdkOfflineFixture",
        "execute",
        "login",
        "resourceRoot",
    ] {
        assert!(crate::readiness::request_settings(
            &json!({"provider":"codex_sdk","settings":{field:"forbidden"}}),
            &json!({})
        )
        .is_err());
    }
}

#[test]
fn sdk_readiness_accepts_both_draft_executables_without_other_settings() {
    let (_, settings) = crate::readiness::request_settings(
        &json!({"provider":"codex_sdk","settings":{"textSdkNodePath":" C:/runtime/node.exe ","codexPath":" C:/runtime/codex.exe "}}),
        &json!({"codexPath":"old.exe","textModel":"model","reasoning":"medium","textApiKey":"not-copied"}),
    ).unwrap();
    assert_eq!(settings["textSdkNodePath"], "C:/runtime/node.exe");
    assert_eq!(settings["codexPath"], "C:/runtime/codex.exe");
    assert_eq!(settings["textModel"], "model");
    assert!(settings.get("textApiKey").is_none());
    assert!(crate::readiness::request_settings(&json!({"provider":"codex_sdk","settings":{"codexPath":7}}), &json!({})).is_err());
    assert!(crate::readiness::request_settings(&json!({"provider":"dreamina","settings":{"codexPath":"foreign.exe"}}), &json!({})).is_err());
}

#[test]
fn rpc_failure_keeps_verified_cli_version_and_does_not_claim_logged_out_or_model_access() {
    let dir = tempfile::tempdir().unwrap();
    let path = native_fixture(dir.path(), "codex-rpc-failure");
    let engine = enabled_engine(dir.path().join("home"));
    let p = engine
        .dispatch(json!({"action":"projects.create","title":"synthetic RPC failure"}))
        .unwrap();
    let r=engine.dispatch(json!({"action":"readiness.check","provider":"codex_cli","projectId":p["id"],"settings":{"codexPath":path}})).unwrap();
    assert_eq!(r["installation"]["state"], "ready", "{r}");
    assert_eq!(
        r["installation"]["version"],
        "codex-cli synthetic-readiness"
    );
    assert_eq!(r["protocol"]["state"], "failed");
    assert_eq!(r["authentication"]["state"], "unknown");
    assert_eq!(r["catalog"]["state"], "unavailable");
    assert_eq!(r["generation"]["calls"], 0);
    assert!(engine
        .store(s(&p, "id"))
        .unwrap()
        .jobs()
        .unwrap()
        .is_empty());
}

#[test]
fn selected_cli_output_floods_fail_bounded_without_account_or_generation_claims() {
    for kind in ["codex-output-flood", "codex-rpc-flood"] {
        let dir = tempfile::tempdir().unwrap();
        let path = native_fixture(dir.path(), kind);
        let engine = enabled_engine(dir.path().join("home"));
        let p = engine
            .dispatch(json!({"action":"projects.create","title":"synthetic oversized CLI"}))
            .unwrap();
        let start = std::time::Instant::now();
        let r=engine.dispatch(json!({"action":"readiness.check","provider":"codex_cli","projectId":p["id"],"settings":{"codexPath":path}})).unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(r["protocol"]["state"], "failed");
        assert_eq!(r["authentication"]["state"], "unknown");
        assert_eq!(r["generation"]["calls"], 0);
        assert!(
            r["messages"]
                .to_string()
                .contains(if kind == "codex-output-flood" {
                    "CLI_INSPECTION_OUTPUT_LIMIT"
                } else {
                    "CLI_RPC_LINE_LIMIT"
                }),
            "{r}"
        );
        assert!(r.to_string().len() < 8192);
        assert!(engine
            .store(s(&p, "id"))
            .unwrap()
            .jobs()
            .unwrap()
            .is_empty());
        let log = fs::read_to_string(dir.path().join(format!("{kind}-calls.txt"))).unwrap();
        assert!(!log.contains("FORBIDDEN"));
        assert!(!log.contains("account/read"));
    }
}

#[test]
fn dreamina_log_write_denial_stops_further_probes_and_does_not_claim_auth_failure() {
 let dir=tempfile::tempdir().unwrap();let path=native_fixture(dir.path(),"dreamina-log-denied");
 let engine=enabled_engine(dir.path().join("home"));let p=engine.dispatch(json!({"action":"projects.create","title":"synthetic logger denial"})).unwrap();
 let r=engine.dispatch(json!({"action":"readiness.check","provider":"dreamina","projectId":p["id"],"settings":{"dreaminaPath":path}})).unwrap();
 assert_eq!(r["details"]["localEnvironmentIssue"],"CLI_LOCAL_LOG_WRITE_RESTRICTED");assert_eq!(r["authentication"]["state"],"not_checked");assert_eq!(r["protocol"]["state"],"failed");assert!(!r.to_string().contains("PRIVATE_SENTINEL"));
 assert_eq!(fs::read_to_string(dir.path().join("dreamina-log-denied-calls.txt")).unwrap(),"version\n");
 assert!(engine.store(s(&p,"id")).unwrap().jobs().unwrap().is_empty());
}

// Synthetic stream payloads are never written to the fixture's command log.
const CANVAS_ACCOUNT_STDERR_ERROR: &str = r#"{"schemaVersion":"1","ok":false,"error":{"code":"service.10000","class":"auth_required","reason":"oauth_scope_insufficient","requiredAction":"login","message":"PRIVATE_ACCOUNT_MESSAGE","token":"PRIVATE_ACCOUNT_TOKEN","path":"/PRIVATE_ACCOUNT_HOME/credentials.json","userId":987654321},"meta":{"requestId":"PRIVATE_REQUEST_ID"}}"#;
const CANVAS_MODELS_STDERR_ERROR: &str = r#"{"schemaVersion":"1","ok":false,"error":{"code":"service.10000","class":"access_denied","requiredAction":"none","message":"PRIVATE_MODEL_MESSAGE","token":"PRIVATE_MODEL_TOKEN","path":"/PRIVATE_MODEL_HOME/credentials.json","userId":987654321},"meta":{"requestId":"PRIVATE_REQUEST_ID"}}"#;
const CANVAS_MODELS_STDERR_TEXT: &str = "TLS certificate, proxy and authentication failure: PRIVATE_MODEL_MESSAGE token=PRIVATE_MODEL_TOKEN path=/PRIVATE_MODEL_HOME/credentials.json userId=987654321";

fn canvas_native_fixture(dir: &Path, kind: &str) -> PathBuf {
    use crate::readiness_canvas::ReadCommand;
    let path=dir.join(if cfg!(windows) {format!("{kind}.exe")} else {kind.into()});
    let mut arms=String::new();
    for (command, response) in [
        (ReadCommand::Version,include_str!("../../fixtures/synthetic/canvas-readiness/version.json")),
        (ReadCommand::Schema,include_str!("../../fixtures/synthetic/canvas-readiness/schema.json")),
        (ReadCommand::AuthStatus,include_str!("../../fixtures/synthetic/canvas-readiness/auth-status.json")),
        (ReadCommand::AuthAccount,include_str!("../../fixtures/synthetic/canvas-readiness/account.json")),
        (ReadCommand::ImageModels,include_str!("../../fixtures/synthetic/canvas-readiness/models.json")),
    ] {
        if kind == "canvas-version-timeout" && command == ReadCommand::Version {
            arms.push_str(&format!("{} => {{ std::thread::sleep(std::time::Duration::from_secs(15)); }},\n", json!(command.args().join(" "))));
            continue;
        }
        let stderr_failure = match (kind, command) {
            (
                "canvas-stderr-failures" | "canvas-unstructured-model-failure",
                ReadCommand::AuthAccount,
            ) => Some((CANVAS_ACCOUNT_STDERR_ERROR, 11)),
            ("canvas-stderr-failures", ReadCommand::ImageModels) => {
                Some((CANVAS_MODELS_STDERR_ERROR, 12))
            }
            ("canvas-unstructured-model-failure", ReadCommand::ImageModels) => {
                Some((CANVAS_MODELS_STDERR_TEXT, 12))
            }
            _ => None,
        };
        if let Some((stderr, exit_code)) = stderr_failure {
            arms.push_str(&format!(
                "{} => {{ std::io::stderr().write_all({}.as_bytes()).unwrap(); std::process::exit({exit_code}); }},\n",
                json!(command.args().join(" ")),
                json!(stderr)
            ));
            continue;
        }
        if kind == "canvas-empty-account" && command == ReadCommand::AuthAccount {
            arms.push_str(&format!("{} => {{}},\n", json!(command.args().join(" "))));
            continue;
        }
        let response=if kind=="canvas-output-flood" && command==ReadCommand::Version {"x".repeat(70_000)} else {response.into()};
        arms.push_str(&format!("{} => println!(\"{{}}\", {}),\n",json!(command.args().join(" ")),json!(response)));
    }
    // The old readiness path invokes `version` without globals; emit public Canvas JSON
    // so the discriminator can stop before any legacy helper or account command.
    arms.push_str(&format!("\"version\" => println!(\"{{}}\", {}),\n",json!(include_str!("../../fixtures/synthetic/canvas-readiness/version.json"))));
    let code=format!(r#"use std::io::Write;
fn main() {{ let args=std::env::args().skip(1).collect::<Vec<_>>().join(" ");
let mut log=std::fs::OpenOptions::new().create(true).append(true).open({}).unwrap(); writeln!(log,"{{}}",args).unwrap();
match args.as_str() {{ {} _ => {{ writeln!(log,"FORBIDDEN").unwrap(); std::process::exit(91); }} }} }}"#,json!(dir.join(format!("{kind}-calls.txt"))),arms);
    let source=dir.join(format!("{kind}.rs"));fs::write(&source,code).unwrap();
    let compiler=fixture_compiler(std::env::var_os("RUSTC"),std::env::var_os("CARGO_HOME"),std::env::var_os(if cfg!(windows){"USERPROFILE"}else{"HOME"}));
    let output=Command::new(compiler).args(["--edition=2021","--crate-name","canvas_readiness_fixture"]).arg(&source).arg("-o").arg(&path).output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));path
}

#[test]
fn canvas_readiness_native_allowlist_preserves_legacy_jobs_settings_and_generation_gates() {
    let dir=tempfile::tempdir().unwrap();let path=canvas_native_fixture(dir.path(),"canvas-success");let engine=enabled_engine(dir.path().join("home"));
    let project=engine.dispatch(json!({"action":"projects.create","title":"Synthetic Canvas readiness"})).unwrap();let store=engine.store(s(&project,"id")).unwrap();
    store.save_settings(&json!({"dreaminaPath":"/legacy/unchanged","imageProvider":"dreamina","imageModel":"5.0"})).unwrap();
    let job=store.add_job("image","synthetic","dreamina","original-account",&json!({"prompt":"synthetic"}),"auth_required").unwrap();
    let settings=store.settings().unwrap();let before=store.job(&job).unwrap();
    let report=engine.dispatch(json!({"action":"readiness.check","projectId":project["id"],"provider":"dreamina_canvas","settings":{"dreaminaCanvasPath":path}})).unwrap();
    assert_eq!(report["installation"]["version"],"1.0.2");assert_eq!(report["protocol"]["state"],"ready");assert_eq!(report["authentication"]["identityVerified"],true);assert_eq!(report["catalog"]["models"].as_array().unwrap().len(),3);
    assert_eq!(report["generation"]["calls"],0);assert_eq!(report["details"]["quota"]["remaining"],Value::Null);assert!(!report.to_string().contains("PRIVATE_"));assert!(!report.to_string().contains("123456789"));
    assert_eq!(store.settings().unwrap(),settings);assert_eq!(store.job(&job).unwrap(),before);
    let log=fs::read_to_string(dir.path().join("canvas-success-calls.txt")).unwrap();assert_eq!(log.lines().count(),5);assert!(!log.contains("FORBIDDEN"));
    for forbidden in ["login","logout","user_credit","--yes","node","create","quote","run"] {assert!(!log.split_whitespace().any(|arg|arg==forbidden));}
    assert!(engine.dispatch(json!({"action":"images.run","projectId":project["id"]})).unwrap_err().contains("OFFLINE_EXECUTION_BLOCKED"));
    assert_eq!(fs::read_to_string(dir.path().join("canvas-success-calls.txt")).unwrap(),log);
    for field in ["dreaminaPath","login","command","profile","region","authToken","sdkOfflineFixture"] {
        assert!(crate::readiness::request_settings(&json!({"provider":"dreamina_canvas","settings":{field:"forbidden"}}),&json!({})).is_err());
    }
}

#[test]
fn canvas_native_stderr_failures_preserve_local_auth_and_only_export_safe_process_diagnostics() {
    use crate::readiness_canvas::ReadCommand;
    for (kind, model_stderr, model_shape, model_source) in [
        (
            "canvas-stderr-failures",
            CANVAS_MODELS_STDERR_ERROR,
            "error_envelope",
            "stderr",
        ),
        (
            "canvas-unstructured-model-failure",
            CANVAS_MODELS_STDERR_TEXT,
            "malformed_json",
            "none",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = canvas_native_fixture(dir.path(), kind);
        let engine = enabled_engine(dir.path().join("home"));
        let project = engine
            .dispatch(
                json!({"action":"projects.create","title":"Synthetic stderr-only Canvas failures"}),
            )
            .unwrap();
        let store = engine.store(s(&project, "id")).unwrap();
        let before_settings = store.settings().unwrap();
        let report = engine
        .dispatch(json!({"action":"readiness.check","projectId":project["id"],"provider":"dreamina_canvas","settings":{"dreaminaCanvasPath":path}}))
        .unwrap();

        assert_eq!(report["installation"]["state"], "ready");
        assert_eq!(report["installation"]["version"], "1.0.2");
        assert_eq!(report["protocol"]["state"], "ready");
        assert_eq!(report["authentication"]["state"], "authenticated");
        assert_eq!(report["authentication"]["identityVerified"], false);
        assert_eq!(report["details"]["accountQuery"]["state"], "unavailable");
        assert_eq!(report["catalog"]["state"], "unavailable");
        assert_eq!(report["network"]["state"], "not_tested");
        assert_eq!(report["generation"]["calls"], 0);
        assert_eq!(
            report["details"]["checks"]["auth account"]["state"],
            "failed"
        );
        assert_eq!(
            report["details"]["checks"]["auth account"]["code"],
            "CANVAS_OAUTH_SCOPE_INSUFFICIENT"
        );
        assert_eq!(
            report["details"]["checks"]["model list --type image"]["state"],
            "failed"
        );

        let diagnostics = report["details"]["processDiagnostics"].as_object().unwrap();
        assert_eq!(diagnostics.len(), 5);
        for label in ["version", "schema", "auth status"] {
            let diagnostic = &diagnostics[label];
            assert_eq!(diagnostic["termination"], "exit");
            assert_eq!(diagnostic["exitCode"], 0);
            assert_eq!(diagnostic["signal"], Value::Null);
            assert!(diagnostic["stdoutBytes"].as_u64().unwrap() > 0);
            assert_eq!(diagnostic["stderrBytes"], 0);
            assert_eq!(diagnostic["stdoutShape"], "success_envelope");
            assert_eq!(diagnostic["stderrShape"], "empty");
            assert_eq!(diagnostic["structuredErrorSource"], "none");
        }
        for (label, exit_code, stderr, shape, source) in [
            (
                "auth account",
                11,
                CANVAS_ACCOUNT_STDERR_ERROR,
                "error_envelope",
                "stderr",
            ),
            (
                "model list --type image",
                12,
                model_stderr,
                model_shape,
                model_source,
            ),
        ] {
            let diagnostic = &diagnostics[label];
            assert_eq!(diagnostic["termination"], "exit");
            assert_eq!(diagnostic["exitCode"], exit_code);
            assert_eq!(diagnostic["signal"], Value::Null);
            assert_eq!(diagnostic["stdoutBytes"], 0);
            assert_eq!(diagnostic["stderrBytes"], stderr.len());
            assert_eq!(diagnostic["stdoutShape"], "empty");
            assert_eq!(diagnostic["stderrShape"], shape);
            assert_eq!(diagnostic["structuredErrorSource"], source);
            assert!(diagnostic["errors"].get("stdout").is_none());
        }
        assert_eq!(
            diagnostics["auth account"]["errors"]["stderr"],
            json!({
                "code":"service.10000", "class":"auth_required", "reason":"oauth_scope_insufficient",
                "requiredAction":"login"
            })
        );
        if model_source == "stderr" {
            assert_eq!(
                diagnostics["model list --type image"]["errors"]["stderr"],
                json!({
                    "code":"service.10000", "class":"access_denied", "requiredAction":"none"
                })
            );
        } else {
            assert_eq!(diagnostics["model list --type image"]["errors"], json!({}));
        }
        // Unstructured stderr and conventional exit numbers cannot establish TLS,
        // proxy, authentication, or transport diagnoses.
        let model_check = report["details"]["checks"]["model list --type image"].to_string();
        for inferred in ["TLS", "PROXY", "CERTIFICATE"] {
            assert!(!model_check.contains(inferred));
        }
        if model_source == "none" {
            assert!(!model_check.contains("AUTH"));
        }
        let serialized = report.to_string();
        for private in [
            "PRIVATE_",
            "987654321",
            "123456789",
            "userId",
            "credentials.json",
        ] {
            assert!(!serialized.contains(private));
        }
        assert_eq!(store.settings().unwrap(), before_settings);
        assert!(store.jobs().unwrap().is_empty());

        let expected_calls = [
            ReadCommand::Version,
            ReadCommand::Schema,
            ReadCommand::AuthStatus,
            ReadCommand::AuthAccount,
            ReadCommand::ImageModels,
        ]
        .iter()
        .map(|command| command.args().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        let log_path = dir.path().join(format!("{kind}-calls.txt"));
        assert_eq!(fs::read_to_string(&log_path).unwrap(), expected_calls);
        assert!(engine
            .dispatch(json!({"action":"images.run","projectId":project["id"]}))
            .unwrap_err()
            .contains("OFFLINE_EXECUTION_BLOCKED"));
        assert_eq!(fs::read_to_string(log_path).unwrap(), expected_calls);
    }
}

#[test]
fn canvas_native_zero_exit_with_empty_account_stdout_never_verifies_remote_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = canvas_native_fixture(dir.path(), "canvas-empty-account");
    let engine = enabled_engine(dir.path().join("home"));
    let project = engine
        .dispatch(json!({"action":"projects.create","title":"Synthetic empty Canvas account"}))
        .unwrap();
    let report = engine
        .dispatch(json!({"action":"readiness.check","projectId":project["id"],"provider":"dreamina_canvas","settings":{"dreaminaCanvasPath":path}}))
        .unwrap();
    assert_eq!(report["protocol"]["state"], "ready");
    assert_eq!(report["authentication"]["state"], "authenticated");
    assert_eq!(report["authentication"]["identityVerified"], false);
    assert_eq!(report["details"]["accountQuery"]["state"], "unavailable");
    assert_eq!(
        report["details"]["checks"]["auth account"]["state"],
        "failed"
    );
    assert_eq!(report["catalog"]["state"], "available");
    assert_eq!(report["network"]["state"], "not_tested");
    assert_eq!(report["generation"]["calls"], 0);
    let diagnostic = &report["details"]["processDiagnostics"]["auth account"];
    assert_eq!(diagnostic["termination"], "exit");
    assert_eq!(diagnostic["exitCode"], 0);
    assert_eq!(diagnostic["signal"], Value::Null);
    assert_eq!(diagnostic["stdoutBytes"], 0);
    assert_eq!(diagnostic["stderrBytes"], 0);
    assert_eq!(diagnostic["stdoutShape"], "empty");
    assert_eq!(diagnostic["stderrShape"], "empty");
    assert_eq!(diagnostic["structuredErrorSource"], "none");
    assert_eq!(diagnostic["errors"], json!({}));
    let log = fs::read_to_string(dir.path().join("canvas-empty-account-calls.txt")).unwrap();
    assert_eq!(log.lines().count(), 5);
    assert!(!log.contains("FORBIDDEN"));
    assert!(engine
        .store(s(&project, "id"))
        .unwrap()
        .jobs()
        .unwrap()
        .is_empty());
    for private in ["PRIVATE_", "123456789", "userId"] {
        assert!(!report.to_string().contains(private));
    }
}

#[test]
fn canvas_selected_in_legacy_readiness_stops_before_any_legacy_account_or_help() {
    let dir=tempfile::tempdir().unwrap();let path=canvas_native_fixture(dir.path(),"canvas-as-legacy");let engine=enabled_engine(dir.path().join("home"));
    let p=engine.dispatch(json!({"action":"projects.create","title":"Synthetic wrong protocol"})).unwrap();
    let report=engine.dispatch(json!({"action":"readiness.check","projectId":p["id"],"provider":"dreamina","settings":{"dreaminaPath":path}})).unwrap();
    assert_eq!(report["protocol"]["state"],"unsupported");assert_eq!(report["authentication"]["state"],"not_checked");
    assert_eq!(fs::read_to_string(dir.path().join("canvas-as-legacy-calls.txt")).unwrap(),"version\n");
}

#[test]
fn canvas_oversize_version_stops_before_schema_or_account_and_missing_path_never_falls_back() {
    let dir=tempfile::tempdir().unwrap();let path=canvas_native_fixture(dir.path(),"canvas-output-flood");let engine=enabled_engine(dir.path().join("home"));
    let p=engine.dispatch(json!({"action":"projects.create","title":"Synthetic bounded Canvas"})).unwrap();
    let report=engine.dispatch(json!({"action":"readiness.check","projectId":p["id"],"provider":"dreamina_canvas","settings":{"dreaminaCanvasPath":path}})).unwrap();
    assert_eq!(report["protocol"]["state"],"failed");assert!(report.to_string().contains("CANVAS_OUTPUT_LIMIT"));assert!(report.to_string().len()<8192);
    assert_eq!(fs::read_to_string(dir.path().join("canvas-output-flood-calls.txt")).unwrap(),"--format json version\n");
    let missing=crate::readiness_canvas::check(&json!({"dreaminaPath":path}));assert_eq!(missing["installation"]["state"],"missing");
    assert_eq!(fs::read_to_string(dir.path().join("canvas-output-flood-calls.txt")).unwrap(),"--format json version\n");
}

#[test]
fn failed_legacy_version_never_reaches_user_credit() {
    let dir=tempfile::tempdir().unwrap();let path=native_fixture(dir.path(),"dreamina-version-failure");
    let engine=enabled_engine(dir.path().join("home"));let project=engine.dispatch(json!({"action":"projects.create","title":"Synthetic legacy version failure"})).unwrap();
    let report=engine.dispatch(json!({"action":"readiness.check","projectId":project["id"],"provider":"dreamina","settings":{"dreaminaPath":path}})).unwrap();
    assert_eq!(report["protocol"]["state"],"failed");assert_eq!(report["authentication"]["state"],"not_checked");
    assert_eq!(fs::read_to_string(dir.path().join("dreamina-version-failure-calls.txt")).unwrap(),"version\n");
}

#[test]
fn canvas_supervisor_timeout_survives_the_full_readiness_error_path() {
    let dir=tempfile::tempdir().unwrap();let path=canvas_native_fixture(dir.path(),"canvas-version-timeout");
    let engine=enabled_engine(dir.path().join("home"));let p=engine.dispatch(json!({"action":"projects.create","title":"Synthetic phase timeout"})).unwrap();
    let report=engine.dispatch(json!({"action":"readiness.check","projectId":p["id"],"provider":"dreamina_canvas","settings":{"dreaminaCanvasPath":path}})).unwrap();
    assert_eq!(report["details"]["checks"]["version"]["code"],"CLI_TIMEOUT");
    let observation=&report["details"]["processDiagnostics"]["version"]["supervisor"];
    assert!(observation["pid"].as_u64().is_some());assert_eq!(observation["timeout"]["kind"],"run");
    assert_eq!(observation["preCleanup"]["state"],"running");assert_eq!(observation["cleanup"]["killRequested"],true);assert_eq!(observation["cleanup"]["reapConfirmed"],true);
    assert_eq!(observation["stdout"]["bytes"],0);assert_eq!(report["authentication"]["state"],"not_checked");assert_eq!(report["generation"]["calls"],0);
    assert_eq!(fs::read_to_string(dir.path().join("canvas-version-timeout-calls.txt")).unwrap(),"--format json version\n");
}

#[test]
fn canvas_oauth_is_native_only_and_separate_from_generation() {
    let dir = tempfile::tempdir().unwrap();
    let disabled = Engine::new_offline_verification(dir.path().join("disabled")).unwrap();
    assert!(disabled.dispatch(json!({"action":"canvas.loginStatus","projectId":"missing"})).is_err());
    let engine = enabled_engine(dir.path().join("enabled"));
    let p = engine.dispatch(json!({"action":"projects.create","title":"synthetic oauth"})).unwrap();
    let fixture=native_fixture(dir.path(),"dreamina-oauth-status-only");
    let status = engine.dispatch(json!({"action":"canvas.loginStatus","projectId":p["id"],"settings":{"dreaminaCanvasPath":fixture}}));
    assert!(status.is_ok(), "native OAuth status must be usable without enabling generation: {status:?}");
    assert_eq!(status.unwrap()["state"], "idle");
    assert!(engine.dispatch(json!({"action":"canvas.logout","projectId":p["id"],"settings":{"dreaminaCanvasPath":fixture}})).unwrap_err().contains("请先确认"));
    let policy = engine.dispatch(json!({"action":"getExecutionPolicy"})).unwrap();
    assert_eq!(policy["canvasOAuth"]["enabled"],true);
    assert_eq!(policy["liveExecutionAllowed"],false);
    assert!(engine.dispatch(json!({"action":"images.run","projectId":p["id"]})).is_err());
}
