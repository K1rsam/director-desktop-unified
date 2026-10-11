// Included by dreamina::bounded_process_tests so these regressions use the same
// credential-free, single-execution fixture as the existing supervisor tests.

fn observed_fixture_result(
    command: Command,
    policy: CliProcessPolicy,
    stopped: Arc<dyn Fn() -> bool + Send + Sync>,
) -> (ProviderResult<CliOutput>, serde_json::Value) {
    let observation = CliObservation::new();
    let result = supervise_command_observed(
        command,
        false,
        "offline-no-credential".into(),
        policy,
        stopped,
        Some(observation.clone()),
    );
    (result, observation.snapshot())
}

fn observation_millis(value: &serde_json::Value) -> u64 {
    value
        .as_u64()
        .expect("diagnostic timing must be a nonnegative integer")
}

fn assert_observation_milestones(diagnostic: &serde_json::Value, phases: &[&str]) {
    let mut previous = 0;
    let elapsed = observation_millis(&diagnostic["elapsedMs"]);
    for phase in phases {
        let observed = observation_millis(&diagnostic["milestones"][*phase]);
        assert!(observed >= previous, "milestone {phase} moved backward");
        assert!(
            observed <= elapsed,
            "milestone {phase} exceeds elapsed time"
        );
        previous = observed;
    }
}

#[test]
fn observation_quiet_success_records_eof_exit_and_one_execution() {
    let dir = tempfile::tempdir().unwrap();
    let counter = dir.path().join("calls");
    let (result, diagnostic) =
        observed_fixture_result(command("quiet", &counter), policy(), Arc::new(|| false));

    let output = result.unwrap();
    assert!(output.success);
    assert_eq!(fs::read(&counter).unwrap(), b"1");
    assert_eq!(
        parse_output(&output.stdout, false).unwrap()["submit_id"],
        "offline-known"
    );
    assert!(diagnostic["pid"].as_u64().is_some_and(|pid| pid > 0));
    assert_eq!(diagnostic["stdout"]["bytes"], output.stdout.len() as u64);
    assert_eq!(diagnostic["stderr"]["bytes"], 0);
    assert_eq!(diagnostic["stdout"]["eof"], true);
    assert_eq!(diagnostic["stderr"]["eof"], true);
    assert_eq!(diagnostic["stdout"]["readError"], false);
    assert_eq!(diagnostic["stderr"]["readError"], false);
    assert!(diagnostic["stdout"]["firstByteMs"].is_u64());
    assert!(diagnostic["stderr"]["firstByteMs"].is_null());
    assert!(diagnostic["stdout"]["eofMs"].is_u64());
    assert!(diagnostic["stderr"]["eofMs"].is_u64());
    assert_eq!(diagnostic["processExit"]["code"], 0);
    assert_eq!(diagnostic["processExit"]["success"], true);
    assert_eq!(diagnostic["cleanup"]["killRequested"], false);
    assert_eq!(diagnostic["cleanup"]["reapConfirmed"], true);
    assert!(diagnostic["timeout"].is_null());
    assert_eq!(diagnostic["phase"], "thread_joined");
    assert_eq!(diagnostic["budgetMs"], 4000);
    assert_observation_milestones(
        &diagnostic,
        &[
            "created",
            "thread_started",
            "runtime_ready",
            "spawn_start",
            "spawn_end",
            "observing",
            "exit_observed",
            "runtime_finished",
            "runtime_dropped",
            "thread_joined",
        ],
    );
    assert!(observation_millis(&diagnostic["controlTicks"]) > 0);
    assert!(
        observation_millis(&diagnostic["processExit"]["observedAtMs"])
            <= observation_millis(&diagnostic["elapsedMs"])
    );
}

#[test]
fn observation_inherited_writer_is_drain_timeout_after_child_exit() {
    let dir = tempfile::tempdir().unwrap();
    let counter = dir.path().join("calls");
    // Compilation is deliberately outside the bounded process measurement.
    let _ = fixture();
    let started = Instant::now();
    let (result, diagnostic) =
        observed_fixture_result(command("inherited", &counter), policy(), Arc::new(|| false));

    let error = result.unwrap_err();
    assert_eq!(error.code, "CLI_OUTPUT_TIMEOUT");
    assert!(!error.auth_required);
    assert!(!error.submission_unknown);
    assert!(
        started.elapsed() < Duration::from_millis(1000),
        "the 80 ms drain budget must not wait for the inherited writer's 1.5 s EOF"
    );
    assert_eq!(fs::read(&counter).unwrap(), b"1");
    assert_eq!(diagnostic["timeout"]["kind"], "drain");
    assert_eq!(diagnostic["timeout"]["exitObserved"], true);
    assert_eq!(diagnostic["processExit"]["code"], 0);
    assert_eq!(diagnostic["processExit"]["success"], true);
    assert_eq!(diagnostic["stdout"]["eof"], false);
    assert_eq!(diagnostic["stderr"]["eof"], false);
    assert!(diagnostic["stdout"]["eofMs"].is_null());
    assert!(diagnostic["stderr"]["eofMs"].is_null());
    assert_eq!(diagnostic["preCleanup"]["state"], "exited");
    assert_eq!(diagnostic["preCleanup"]["exitCode"], 0);
    assert_eq!(diagnostic["cleanup"]["killRequested"], false);
    assert_eq!(diagnostic["cleanup"]["reapConfirmed"], true);
}

#[test]
fn observation_live_child_run_timeout_records_kill_and_reap() {
    let dir = tempfile::tempdir().unwrap();
    let counter = dir.path().join("calls");
    let mut process_policy = policy();
    process_policy.run = Duration::from_millis(100);
    let _ = fixture();
    let started = Instant::now();
    let (result, diagnostic) = observed_fixture_result(
        command("slow", &counter),
        process_policy,
        Arc::new(|| false),
    );

    let error = result.unwrap_err();
    assert_eq!(error.code, "CLI_TIMEOUT");
    assert!(!error.auth_required);
    assert!(!error.submission_unknown);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fs::read(&counter).unwrap(), b"1");
    assert_eq!(diagnostic["timeout"]["kind"], "run");
    assert_eq!(diagnostic["timeout"]["exitObserved"], false);
    assert_eq!(diagnostic["preCleanup"]["state"], "running");
    assert!(diagnostic["preCleanup"]["exitCode"].is_null());
    assert!(diagnostic["preCleanup"]["signal"].is_null());
    assert_eq!(diagnostic["cleanup"]["killRequested"], true);
    assert_eq!(diagnostic["cleanup"]["reapConfirmed"], true);
    assert!(observation_millis(&diagnostic["timeout"]["elapsedMs"]) >= 100);
    #[cfg(windows)]
    assert!(fixture_process_exited(&counter));
}

#[test]
fn observation_delayed_supervisor_reports_overshoot_without_false_success_or_kill() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let dir = tempfile::tempdir().unwrap();
    let counter = dir.path().join("calls");
    let process_policy = policy();
    // This test measures a deliberate control-loop stall, not the time Windows
    // needs to initialize pipes and spawn under suite load. Keep the normal
    // synthetic run budget and explicitly cross it after the first control tick.
    let deliberate_stall = process_policy.run + Duration::from_millis(250);
    let checks = Arc::new(AtomicUsize::new(0));
    let checks_in_callback = checks.clone();
    let counter_in_callback = counter.clone();
    let stopped = Arc::new(move || {
        // The first call precedes spawn. Stall only the first control tick so
        // the fixture can finish while this single-threaded supervisor cannot
        // poll child exit, either pipe, or its already-expired run deadline.
        if checks_in_callback.fetch_add(1, Ordering::SeqCst) == 1 {
            let marker_deadline = Instant::now() + Duration::from_secs(2);
            while !counter_in_callback.exists() && Instant::now() < marker_deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(
                counter_in_callback.exists(),
                "fixture never reached its start marker"
            );
            // The fixture cannot finish before this control tick. It releases
            // into its normal quiet exit while the supervisor is stalled.
            fs::write(counter_in_callback.with_extension("release"), b"released").unwrap();
            std::thread::sleep(deliberate_stall);
        }
        false
    });
    let (result, diagnostic) =
        observed_fixture_result(command("observation-gated", &counter), process_policy, stopped);

    let error = result.unwrap_err();
    assert_eq!(
        error.code, "CLI_TIMEOUT",
        "late exit observation must not erase the timeout"
    );
    assert!(!error.auth_required);
    assert!(!error.submission_unknown);
    assert!(checks.load(Ordering::SeqCst) >= 2, "control stall was not exercised: {diagnostic}");
    assert_eq!(fs::read(&counter).unwrap(), b"1");
    assert_eq!(diagnostic["timeout"]["kind"], "run");
    assert_eq!(diagnostic["preCleanup"]["state"], "exited");
    assert_eq!(diagnostic["milestones"]["exit_observed"], diagnostic["processExit"]["observedAtMs"]);
    assert_eq!(diagnostic["preCleanup"]["exitCode"], 0);
    assert_eq!(diagnostic["processExit"]["code"], 0);
    assert_eq!(diagnostic["processExit"]["success"], true);
    assert_eq!(diagnostic["cleanup"]["killRequested"], false);
    assert_eq!(diagnostic["cleanup"]["reapConfirmed"], true);
    assert!(observation_millis(&diagnostic["timeout"]["elapsedMs"]) >= deliberate_stall.as_millis() as u64);
    assert!(observation_millis(&diagnostic["timeout"]["overshootMs"]) >= 140);
    assert!(observation_millis(&diagnostic["maxControlGapMs"]) >= deliberate_stall.as_millis() as u64 - 10);
}
