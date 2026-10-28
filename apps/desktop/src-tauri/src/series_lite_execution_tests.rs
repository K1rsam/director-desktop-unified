use super::*;
fn corpus() -> Value {
    serde_json::from_str(include_str!("series_lite_execution_golden.json")).unwrap()
}
fn write(root: &Path, files: &Value) {
    for (path, body) in files.as_object().unwrap() {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body.as_str().unwrap().as_bytes()).unwrap();
    }
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &corpus()["files"]);
    root
}
fn read(root: &Path, path: &str) -> Value {
    serde_json::from_slice(&fs::read(root.join(path)).unwrap()).unwrap()
}
fn signature_without_native(raw: &[u8]) -> Json {
    let mut v = parse(raw).unwrap();
    let mut runtime = v.get("runtime").unwrap().object().unwrap().clone();
    runtime.retain(|k, _| !k.starts_with("_native/"));
    put(&mut v, "runtime", Json::Object(runtime)).unwrap();
    v
}
#[test]
fn original_20_scenarios_match_input_signatures_resume_decisions_and_delivery_bytes() {
    let corpus = corpus();
    assert_eq!(
        sha256(include_bytes!(
            "../../../../rules/series-lite/20261027-v0.1.2/scripts/episode_execution.py"
        )),
        corpus["episodeExecutionSha256"].as_str().unwrap()
    );
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 20);
    let pack = VerifiedPack::embedded().unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let dir = fixture();
        write(dir.path(), &case["beforePatches"]);
        let inspected = inspect_inputs(dir.path(), "execution-request.json", &pack);
        assert_eq!(
            inspected.is_ok(),
            case.get("inputs").is_some(),
            "{name}: {inspected:?}"
        );
        if let Ok(inspected) = inspected {
            let signature = signature_without_native(&inspected.signature_utf8);
            assert_eq!(
                signature,
                from_value(&case["inputs"]["signature"]).unwrap(),
                "{name}: all original dependency bytes"
            );
            assert_eq!(
                sha256(&canonical(&signature)),
                case["inputs"]["fingerprint"].as_str().unwrap(),
                "{name}: original fingerprint"
            );
            assert_eq!(
                parse(&inspected.registry_snapshot_utf8).unwrap(),
                from_value(&case["inputs"]["snapshot"]).unwrap()
            );
        }
        let first = run_inner(
            dir.path(),
            "execution-request.json",
            "adoption",
            &pack,
            None,
            &mut |phase| {
                if phase == name || (name == "journal_output_drift" && phase == "before_export") {
                    return Err(err("synthetic interruption"));
                }
                if name == "inputs_during_execution" && phase == "before_receipt" {
                    let file = dir.path().join("direction.txt");
                    let mut bytes = fs::read(&file).unwrap();
                    bytes.extend(b"drift");
                    fs::write(file, bytes).unwrap();
                }
                Ok(())
            },
        );
        assert_eq!(
            first.is_ok(),
            case.get("first").is_some(),
            "{name}: {first:?}"
        );
        if let Some(expected) = case.get("initialSteps") {
            let state = read(dir.path(), "adoption/execution.json");
            assert_eq!(
                state["steps"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                expected
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect(),
                "{name}"
            );
        }
        assert_eq!(
            dir.path().join("adoption/adoption.json").exists(),
            case["receiptInitiallyExists"].as_bool().unwrap(),
            "{name}"
        );
        let pin = first.as_ref().ok().map(|r| r.receipt_file_sha256.as_str());
        let preserve = if name == "after_preview_write" {
            Some(fs::read(dir.path().join("adoption/export-0/preview.md")).unwrap())
        } else {
            None
        };
        let prior_receipt = fs::read(dir.path().join("adoption/adoption.json")).ok();
        write(dir.path(), &case["postPatches"]);
        let resumed = run(dir.path(), "execution-request.json", "adoption", &pack, pin);
        assert_eq!(
            resumed.is_ok(),
            case.get("resumed").is_some(),
            "{name}: {resumed:?}"
        );
        if let Some(bytes) = preserve {
            assert_eq!(
                fs::read(dir.path().join("adoption/export-0/preview.md")).unwrap(),
                bytes
            );
        }
        if let Some(bytes) = prior_receipt {
            assert_eq!(
                fs::read(dir.path().join("adoption/adoption.json")).unwrap(),
                bytes,
                "{name}: old receipt overwritten"
            );
        }
        if let Ok(result) = resumed {
            assert_eq!(
                serde_json::to_value(&result.steps_run).unwrap(),
                case["resumed"]["stepsRun"],
                "{name}"
            );
            assert_eq!(json!(result.reused), case["resumed"]["reused"]);
            let verified = verify_receipt(
                dir.path(),
                &result.receipt_path,
                &result.receipt_file_sha256,
                Some(1),
            )
            .unwrap();
            assert_eq!(verified.content_review, "NOT_PROVEN");
            assert_eq!(
                fs::read(dir.path().join(&result.preview_path)).unwrap(),
                case["markdown"].as_str().unwrap().as_bytes(),
                "{name}: MD"
            );
            assert_eq!(
                fs::read(
                    dir.path()
                        .join(format!("{}.assets.json", result.preview_path))
                )
                .unwrap(),
                case["catalog"].as_str().unwrap().as_bytes(),
                "{name}: catalog"
            );
            assert_eq!(
                read(dir.path(), "adoption/verify.json")["fieldCapacity"],
                case["fieldCapacity"],
                "{name}: field observation"
            );
            let mut verify_report = read(dir.path(), "adoption/verify.json");
            verify_report.as_object_mut().unwrap().remove("assetsIndex");
            assert_eq!(
                verify_report, case["verifyReport"],
                "{name}: complete report apart from actual output path"
            );
        }
    }
}
#[test]
fn asset_view_transitive_eleven_levels_and_field_budget_match_original() {
    let corpus = corpus();
    for case in corpus["assetViews"].as_array().unwrap() {
        let view = binding::relevant(
            &from_value(&case["registry"]).unwrap(),
            &from_value(&case["bundle"]).unwrap(),
        );
        assert_eq!(
            view.is_ok(),
            case.get("fingerprint").is_some(),
            "{}",
            case["name"]
        );
        if let Ok(view) = view {
            assert_eq!(
                sha256(&canonical(&view)),
                case["fingerprint"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
    for case in corpus["fieldBudgets"].as_array().unwrap() {
        assert_eq!(
            reports::field_budget(case["markdown"].as_str().unwrap()),
            case["report"]
        );
    }
}
#[test]
fn normalized_packet_path_is_recoverable_without_losing_frozen_pin() {
    let dir = fixture();
    let pack = VerifiedPack::embedded().unwrap();
    let mut request = read(dir.path(), "execution-request.json");
    request["packet"] = json!("./packet.json");
    fs::write(
        dir.path().join("execution-request.json"),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    let r = run(dir.path(), "execution-request.json", "out", &pack, None).unwrap();
    assert!(verify_receipt(dir.path(), &r.receipt_path, &r.receipt_file_sha256, None).is_ok());
    assert!(run(
        dir.path(),
        "execution-request.json",
        "out",
        &pack,
        Some(&r.receipt_file_sha256)
    )
    .unwrap()
    .steps_run
    .is_empty());
}
#[test]
fn completed_receipt_requires_external_pin_and_never_recreates_missing_adoption() {
    let dir = fixture();
    let pack = VerifiedPack::embedded().unwrap();
    let r = run(dir.path(), "execution-request.json", "out", &pack, None).unwrap();
    let before = fs::read(dir.path().join(&r.receipt_path)).unwrap();
    assert!(
        run(dir.path(), "execution-request.json", "out", &pack, None)
            .unwrap_err()
            .contains("external pin")
    );
    assert!(verify_receipt(dir.path(), &r.receipt_path, &"a".repeat(64), None).is_err());
    let mut receipt = parse(&before).unwrap();
    put(&mut receipt, "contentReview", text("PASS")).unwrap();
    let mut o = receipt.object().unwrap().clone();
    o.remove("receiptSha256");
    let h = sha256(&canonical(&Json::Object(o)));
    put(&mut receipt, "receiptSha256", text(h)).unwrap();
    fs::write(dir.path().join(&r.receipt_path), canonical(&receipt)).unwrap();
    assert!(verify_receipt(dir.path(), &r.receipt_path, &r.receipt_file_sha256, None).is_err());
    // Even a newly supplied pin cannot promote mechanics to semantic approval.
    assert!(verify_receipt(
        dir.path(),
        &r.receipt_path,
        &sha256(&canonical(&receipt)),
        None
    )
    .is_err());
    fs::remove_file(dir.path().join(&r.receipt_path)).unwrap();
    assert!(run(
        dir.path(),
        "execution-request.json",
        "out",
        &pack,
        Some(&r.receipt_file_sha256)
    )
    .is_err());
    assert!(!dir.path().join(&r.receipt_path).exists());
}
#[test]
fn frozen_runtime_survives_current_adapter_identity_difference() {
    let dir = fixture();
    let pack = VerifiedPack::embedded().unwrap();
    let r = run(dir.path(), "execution-request.json", "out", &pack, None).unwrap();
    let path = dir.path().join(&r.receipt_path);
    let mut receipt = parse(&fs::read(&path).unwrap()).unwrap();
    let mut signature = receipt.get("inputs").unwrap().clone();
    let mut runtime = signature.get("runtime").unwrap().clone();
    put(
        &mut runtime,
        "_native/historical-adapter.rs",
        text("b".repeat(64)),
    )
    .unwrap();
    put(&mut signature, "runtime", runtime).unwrap();
    put(
        &mut receipt,
        "executionFingerprint",
        text(sha256(&canonical(&signature))),
    )
    .unwrap();
    put(&mut receipt, "inputs", signature).unwrap();
    let mut o = receipt.object().unwrap().clone();
    o.remove("receiptSha256");
    put(
        &mut receipt,
        "receiptSha256",
        text(sha256(&canonical(&Json::Object(o)))),
    )
    .unwrap();
    let bytes = canonical(&receipt);
    fs::write(path, &bytes).unwrap();
    // Simulates a separately frozen historical controller pin, not self-discovery.
    assert!(verify_receipt(dir.path(), &r.receipt_path, &sha256(&bytes), None).is_ok());
}
#[test]
fn malicious_paths_existing_outputs_and_concurrent_lock_fail_closed() {
    let dir = fixture();
    let pack = VerifiedPack::embedded().unwrap();
    for output in ["../escape", "C:/escape", "registry/../../escape", "."] {
        assert!(run(dir.path(), "execution-request.json", output, &pack, None).is_err());
    }
    let job = Job::new(dir.path()).unwrap();
    let lock = job.lock(&job.root.join(".state.lock")).unwrap();
    assert!(
        run(dir.path(), "execution-request.json", "out", &pack, None)
            .unwrap_err()
            .contains("busy")
    );
    drop(lock);
    // Empty failed lock directory is not blindly treated as a new execution.
    let r = run(dir.path(), "execution-request.json", "other", &pack, None).unwrap();
    assert!(verify_receipt(dir.path(), &r.receipt_path, &r.receipt_file_sha256, Some(2)).is_err());
}
#[test]
fn immutable_receipt_publication_cannot_overwrite_existing_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let job = Job::new(dir.path()).unwrap();
    let output = job.path("run/adoption.json").unwrap();
    job.atomic_new(&output, b"complete-first").unwrap();
    assert!(job.atomic_new(&output, b"replacement").is_err());
    assert_eq!(fs::read(output).unwrap(), b"complete-first");
}

fn saved_intent(dir: &Path, pack: &VerifiedPack) -> (String, Vec<u8>) {
    let mut saved = None;
    let result = run_with_commit_intent(
        dir,
        "execution-request.json",
        "out",
        pack,
        None,
        &mut |path, bytes| {
            saved = Some((path.to_owned(), bytes.to_vec()));
            Err(err(
                "synthetic crash after durable SQL intent, before FS receipt",
            ))
        },
    );
    assert!(result.unwrap_err().contains("synthetic crash"));
    let saved = saved.unwrap();
    assert!(!dir.join(&saved.0).exists());
    saved
}

#[test]
fn receipt_commit_intent_closes_both_sql_filesystem_crash_windows_without_rerunning_steps() {
    let dir = fixture();
    let pack = VerifiedPack::embedded().unwrap();
    let (path, intended) = saved_intent(dir.path(), &pack);
    // This hash represents the caller's durable SQL intent captured in the
    // pre-publication callback, never the untrusted on-disk adoption.json.
    let frozen_pin = sha256(&intended);
    let protected = [
        "out/execution.json",
        "out/consumption.json",
        "out/sync.json",
        "out/check.json",
        "out/export.json",
        "out/verify.json",
        "out/export-0/preview.md",
        "out/export-0/preview.md.assets.json",
    ]
    .into_iter()
    .map(|p| (p, fs::read(dir.path().join(p)).unwrap()))
    .collect::<Vec<_>>();
    assert!(run(
        dir.path(),
        "execution-request.json",
        "out",
        &pack,
        Some(&frozen_pin)
    )
    .unwrap_err()
    .contains("missing"));
    let repaired = reconcile_commit_intent(
        dir.path(),
        "execution-request.json",
        "out",
        &intended,
        &frozen_pin,
    )
    .unwrap();
    assert!(repaired.reused);
    assert!(repaired.steps_run.is_empty());
    assert_eq!(repaired.content_review, "NOT_PROVEN");
    assert_eq!(repaired.receipt_path, path);
    assert_eq!(fs::read(dir.path().join(&path)).unwrap(), intended);
    for (path, bytes) in protected {
        assert_eq!(
            fs::read(dir.path().join(path)).unwrap(),
            bytes,
            "{path}: no re-export/recheck journal rewrite"
        );
    }
    verify_receipt(dir.path(), &path, &frozen_pin, Some(1)).unwrap();
    let again = reconcile_commit_intent(
        dir.path(),
        "execution-request.json",
        "out",
        &intended,
        &frozen_pin,
    )
    .unwrap();
    assert!(again.reused);
    assert!(again.steps_run.is_empty());
    // A crash after the receipt was published is also idempotent because the
    // callback already committed this pin before the filesystem operation.
    let mut called = false;
    let resumed = run_with_commit_intent(
        dir.path(),
        "execution-request.json",
        "out",
        &pack,
        Some(&frozen_pin),
        &mut |_, _| {
            called = true;
            Ok(())
        },
    )
    .unwrap();
    assert!(resumed.reused);
    assert!(!called);
}

#[test]
fn receipt_intent_reconciliation_rejects_changed_input_output_journal_or_untrusted_bytes() {
    for drift in ["input", "output", "journal", "missing_journal", "intent"] {
        let dir = fixture();
        let pack = VerifiedPack::embedded().unwrap();
        let (path, mut intended) = saved_intent(dir.path(), &pack);
        let pin = sha256(&intended);
        match drift {
            "input" => fs::write(
                dir.path().join("direction.txt"),
                b"changed frozen direction",
            )
            .unwrap(),
            "output" => fs::write(
                dir.path().join("out/export-0/preview.md"),
                b"changed delivery",
            )
            .unwrap(),
            "journal" => {
                let file = dir.path().join("out/execution.json");
                let mut state: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
                state["steps"].as_object_mut().unwrap().remove("verify");
                fs::write(file, serde_json::to_vec(&state).unwrap()).unwrap();
            }
            "missing_journal" => fs::remove_file(dir.path().join("out/execution.json")).unwrap(),
            "intent" => intended.push(b' '),
            _ => unreachable!(),
        }
        assert!(
            reconcile_commit_intent(dir.path(), "execution-request.json", "out", &intended, &pin)
                .is_err(),
            "{drift}"
        );
        assert!(
            !dir.path().join(path).exists(),
            "{drift}: no recreated authority"
        );
    }
}

#[test]
fn receipt_commit_callback_cannot_hide_input_drift_or_self_adopt_an_unpinned_existing_receipt() {
    let dir = fixture();
    let pack = VerifiedPack::embedded().unwrap();
    let mut saved = None;
    let result = run_with_commit_intent(
        dir.path(),
        "execution-request.json",
        "out",
        &pack,
        None,
        &mut |_, bytes| {
            saved = Some(bytes.to_vec());
            fs::write(
                dir.path().join("direction.txt"),
                b"drift during SQL callback",
            )
            .unwrap();
            Ok(())
        },
    );
    assert!(result.is_err());
    assert!(saved.is_some());
    assert!(!dir.path().join("out/adoption.json").exists());
    let dir = fixture();
    let result = run(dir.path(), "execution-request.json", "out", &pack, None).unwrap();
    let original = fs::read(dir.path().join(&result.receipt_path)).unwrap();
    let mut callback_called = false;
    let error = run_with_commit_intent(
        dir.path(),
        "execution-request.json",
        "out",
        &pack,
        None,
        &mut |_, _| {
            callback_called = true;
            Ok(())
        },
    )
    .unwrap_err();
    assert!(error.contains("external pin"));
    assert!(!callback_called);
    assert_eq!(
        fs::read(dir.path().join(result.receipt_path)).unwrap(),
        original
    );
}

#[test]
fn receipt_intent_never_overwrites_a_conflicting_receipt_or_accepts_a_different_run() {
    let dir = fixture();
    let pack = VerifiedPack::embedded().unwrap();
    let (path, intended) = saved_intent(dir.path(), &pack);
    let pin = sha256(&intended);
    fs::write(
        dir.path().join(&path),
        b"independently existing receipt bytes",
    )
    .unwrap();
    assert!(
        reconcile_commit_intent(dir.path(), "execution-request.json", "out", &intended, &pin)
            .unwrap_err()
            .contains("never overwrite")
    );
    assert_eq!(
        fs::read(dir.path().join(path)).unwrap(),
        b"independently existing receipt bytes"
    );
    assert!(
        reconcile_commit_intent(dir.path(), "different-request.json", "out", &intended, &pin)
            .is_err()
    );
    assert!(reconcile_commit_intent(
        dir.path(),
        "execution-request.json",
        "different-run",
        &intended,
        &pin
    )
    .is_err());
}
