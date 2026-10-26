//! Test-only synchronous SDK evidence profiling. No provider entry point is used.
use serde_json::{json, Value};
use std::{cell::RefCell, collections::BTreeMap, time::Instant};

#[derive(Default)]
struct Counts {
    calls: u64,
    inclusive: u128,
    exclusive: u128,
}
struct Frame {
    key: (String, String),
    children: u128,
}
#[derive(Default)]
struct State {
    counts: BTreeMap<(String, String), Counts>,
    stack: Vec<Frame>,
    max_depth: usize,
}
thread_local! { static PROFILE: RefCell<Option<State>> = const { RefCell::new(None) }; }
pub(crate) struct Span {
    started: Instant,
    active: bool,
}
pub(crate) fn enter(name: &str, identity: &str) -> Span {
    let active = PROFILE.with(|profile| {
        let mut profile = profile.borrow_mut();
        let Some(state) = profile.as_mut() else {
            return false;
        };
        state.stack.push(Frame {
            key: (name.into(), identity.into()),
            children: 0,
        });
        state.max_depth = state.max_depth.max(state.stack.len());
        true
    });
    Span {
        started: Instant::now(),
        active,
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let elapsed = self.started.elapsed().as_nanos();
        PROFILE.with(|profile| {
            let mut profile = profile.borrow_mut();
            let state = profile
                .as_mut()
                .expect("profile remains inside synchronous scope");
            let frame = state.stack.pop().expect("matching profile entry");
            if let Some(parent) = state.stack.last_mut() {
                parent.children += elapsed;
            }
            let row = state.counts.entry(frame.key).or_default();
            row.calls += 1;
            row.inclusive += elapsed;
            row.exclusive += elapsed.saturating_sub(frame.children);
        });
    }
}
pub(super) fn begin() {
    PROFILE.with(|p| assert!(p.replace(Some(State::default())).is_none()));
}
pub(super) fn finish() -> Value {
    let state = PROFILE.with(|p| p.take().unwrap());
    assert!(state.stack.is_empty());
    json!({"maxDepth":state.max_depth,"calls":state.counts.into_iter().map(|((name,identity),count)|
        json!({"name":name,"identity":identity,"calls":count.calls,"inclusiveNs":count.inclusive,"exclusiveNs":count.exclusive})).collect::<Vec<_>>()})
}

#[test]
#[ignore = "requires an explicitly prepared private evidence copy; never reads a default project"]
fn profile_sdk_saved_evidence_only() {
    use super::{SavedTextExecutor, TextRequest};
    use crate::store::Store;
    let root = std::path::PathBuf::from(
        std::env::var_os("DIRECTOR_SDK_VALIDATION_FIXTURE")
            .expect("explicit private fixture required"),
    );
    let report_path = std::path::PathBuf::from(
        std::env::var_os("DIRECTOR_SDK_VALIDATION_REPORT")
            .expect("explicit private report required"),
    );
    let root = root.canonicalize().unwrap();
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("profile-fixture.json")).unwrap()).unwrap();
    assert_eq!(manifest["version"], "sdk-validation-private-fixture/v1");
    assert_eq!(manifest["copiedEvidence"], true);
    for file in manifest["files"].as_array().unwrap() {
        let path = root
            .join(file["path"].as_str().unwrap())
            .canonicalize()
            .unwrap();
        assert!(path.starts_with(&root));
        assert_eq!(crate::store::hash_file(&path).unwrap().0, file["sha256"]);
    }
    let store = Store::open(root.clone()).unwrap();
    let run = manifest["workflowId"].as_str().unwrap();
    let parent = store.job(run).unwrap();
    let executor = SavedTextExecutor {
        store,
        settings: parent["snapshot"]["input"]["settings"].clone(),
        workflow_id: Some(run.into()),
    };
    let db_before = std::fs::read(root.join("project.sqlite")).unwrap();
    let mut reports = vec![];
    for id in manifest["jobIds"].as_array().unwrap() {
        let job = executor.sdk_job(id.as_str().unwrap()).unwrap();
        let request: TextRequest = serde_json::from_value(job["snapshot"].clone()).unwrap();
        for operation in ["request_snapshot", "sdk_raw"] {
            eprintln!("profiling {operation} job={id}");
            begin();
            let started = Instant::now();
            let result = if operation == "request_snapshot" {
                executor.request_snapshot(&request)
            } else {
                executor.sdk_raw(&job, &job["snapshot"])
            };
            let elapsed = started.elapsed().as_millis();
            let counts = finish();
            let expected = if operation == "request_snapshot" {
                &job["snapshot"]
            } else {
                &job["result"]
            };
            reports.push(json!({"operation":operation,"jobId":id,"elapsedMs":elapsed,"success":result.as_ref().is_ok_and(|v|v==expected),"error":result.as_ref().err(),"profile":counts}));
            std::fs::write(&report_path, serde_json::to_vec_pretty(&json!({"version":"sdk-synchronous-evidence-profile/v1","fixtureManifestSha256":crate::store::hash_file(&root.join("profile-fixture.json")).unwrap().0,"reports":reports,"providerCalls":0,"adoptionCalls":0})).unwrap()).unwrap();
            assert_eq!(result.unwrap(), *expected);
            eprintln!("completed {operation} in {elapsed} ms");
        }
    }
    assert_eq!(
        std::fs::read(root.join("project.sqlite")).unwrap(),
        db_before,
        "diagnostics cannot change copied evidence"
    );
}
