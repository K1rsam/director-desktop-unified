use super::*;
fn fixture() -> (Vec<u8>, Value) {
    let bundle=br#"{"episodes":[{"source":{"sourceRows":[{"id":"r1","text":"original"},{"id":"r2","text":"another"}]},"draft":{"units":[{"id":"u1","seconds":2.123456789},{"id":"u2","seconds":3.123456789}]}}]}"#.to_vec();
    let report = json!({"version":VERSION,"bundleSha256":sha256(&bundle),"sourceRowIds":["r1","r2"],"unitIds":["u1","u2"],"issues":[],"unresolved":[],"closedIssues":[],"boundary":{"status":"not_applicable","previousBundleSha256":null,"previousUnitId":null,"currentUnitId":null,"basis":"First episode in frozen scope."},"verdict":"pass","summary":"Read both source rows and both units; no mandatory correction found."});
    (bundle, report)
}
fn run(bundle: &[u8], r: &Value, prior: &[Issue]) -> Result<Checked> {
    validate(
        &serde_json::to_vec(r).unwrap(),
        bundle,
        &sha256(bundle),
        prior,
        BoundaryInput::FirstInScope,
    )
}
fn issue() -> Issue {
    Issue {
        id: "issue-1".into(),
        code: "SOURCE_ACTION_MISSING".into(),
        unit_ids: vec!["u1".into()],
        source_refs: vec!["r1".into()],
        detail: "Source action missing from the picture.".into(),
        impact: "Changes causal transition.".into(),
        must_fix: true,
    }
}
#[test]
fn complete_bound_review_is_checked_but_never_grants_adoption() {
    let (bundle, r) = fixture();
    let checked = run(&bundle, &r, &[]).unwrap();
    assert!(checked.declared_content_pass);
    assert!(!checked.boundary_pending);
    assert_eq!(
        checked.response_sha256,
        sha256(&serde_json::to_vec(&r).unwrap())
    );
    assert_eq!(checked.report.bundle_sha256, sha256(&bundle));
    assert_eq!(schema()["additionalProperties"], false);
    assert!(!RULE.is_empty());
}
#[test]
fn incomplete_scope_changed_version_and_forged_coordinates_fail() {
    let (bundle, r) = fixture();
    for (key, value) in [
        ("unitIds", json!(["u1"])),
        ("sourceRowIds", json!(["r2", "r1"])),
        ("bundleSha256", json!("a".repeat(64))),
    ] {
        let mut changed = r.clone();
        changed[key] = value;
        assert!(run(&bundle, &changed, &[]).is_err());
    }
    let mut changed = r.clone();
    let mut i = issue();
    i.unit_ids = vec!["u3".into()];
    changed["issues"] = json!([i]);
    changed["unresolved"] = json!(["issue-1"]);
    changed["verdict"] = json!("revise");
    assert!(run(&bundle, &changed, &[])
        .unwrap_err()
        .contains("COORDINATES_UNKNOWN"));
    assert!(validate(
        &serde_json::to_vec(&r).unwrap(),
        &bundle,
        &"a".repeat(64),
        &[],
        BoundaryInput::FirstInScope
    )
    .is_err());
}
#[test]
fn old_issue_cannot_disappear_be_downgraded_or_closed_with_only_a_label() {
    let (bundle, mut r) = fixture();
    let old = issue();
    assert!(run(&bundle, &r, &[old.clone()])
        .unwrap_err()
        .contains("SILENTLY_DROPPED"));
    r["closedIssues"] =
        json!([{"issueId":"issue-1","unitIds":[],"sourceRefs":[],"basis":"closed"}]);
    assert!(run(&bundle, &r, &[old.clone()]).is_err());
    r["closedIssues"] = json!([]);
    let mut downgraded = old.clone();
    downgraded.must_fix = false;
    r["issues"] = json!([downgraded]);
    r["unresolved"] = json!(["issue-1"]);
    r["verdict"] = json!("revise");
    assert!(run(&bundle, &r, &[old.clone()])
        .unwrap_err()
        .contains("DOWNGRADED"));
    r["issues"] = json!([old.clone()]);
    assert!(!run(&bundle, &r, &[old]).unwrap().declared_content_pass);
}
#[test]
fn closure_retains_same_issue_identity_and_coordinates_without_rewriting_draft() {
    let (bundle, mut r) = fixture();
    let before = bundle.clone();
    r["closedIssues"] = json!([{"issueId":"issue-1","unitIds":["u1"],"sourceRefs":["r1"],"basis":"Re-read the current u1 action against r1; the required action and consequence are now both performed."}]);
    assert!(run(&bundle, &r, &[issue()]).unwrap().declared_content_pass);
    assert_eq!(bundle, before);
    r["closedIssues"][0]["issueId"] = json!("invented");
    assert!(run(&bundle, &r, &[issue()]).is_err());
}
#[test]
fn missing_adjacent_draft_stays_pending_and_supplied_boundary_is_hash_bound() {
    let (bundle, mut r) = fixture();
    let sha = sha256(&bundle);
    r["boundary"]["status"] = json!("pending");
    r["boundary"]["basis"] = json!("Previous final draft unavailable; no seam review claimed.");
    let checked = validate(
        &serde_json::to_vec(&r).unwrap(),
        &bundle,
        &sha,
        &[],
        BoundaryInput::MissingPrevious,
    )
    .unwrap();
    assert!(checked.declared_content_pass && checked.boundary_pending);
    r["boundary"]["status"] = json!("pass");
    assert!(validate(
        &serde_json::to_vec(&r).unwrap(),
        &bundle,
        &sha,
        &[],
        BoundaryInput::MissingPrevious
    )
    .is_err());
    r["boundary"] = json!({"status":"pass","previousBundleSha256":sha,"previousUnitId":"u2","currentUnitId":"u1","basis":"Compared the supplied last and first unit states."});
    assert!(
        !validate(
            &serde_json::to_vec(&r).unwrap(),
            &bundle,
            &sha,
            &[],
            BoundaryInput::Supplied {
                previous_bytes: &bundle,
                previous_sha256: &sha
            }
        )
        .unwrap()
        .boundary_pending
    );
    r["boundary"]["previousUnitId"] = json!("u1");
    assert!(validate(
        &serde_json::to_vec(&r).unwrap(),
        &bundle,
        &sha,
        &[],
        BoundaryInput::Supplied {
            previous_bytes: &bundle,
            previous_sha256: &sha
        }
    )
    .is_err());
}
#[test]
fn duplicate_keys_and_pass_with_required_issue_are_rejected() {
    let (bundle, mut r) = fixture();
    r["issues"] = json!([issue()]);
    assert!(run(&bundle, &r, &[])
        .unwrap_err()
        .contains("REQUIRED_ISSUE_HIDDEN"));
    r["unresolved"] = json!(["issue-1"]);
    assert!(run(&bundle, &r, &[])
        .unwrap_err()
        .contains("VERDICT_CONTRADICTS"));
    let (_, r) = fixture();
    let raw = serde_json::to_string(&r)
        .unwrap()
        .replacen("{", "{\"version\":\"duplicate\",", 1);
    assert!(validate(
        raw.as_bytes(),
        &bundle,
        &sha256(&bundle),
        &[],
        BoundaryInput::FirstInScope
    )
    .is_err());
}
