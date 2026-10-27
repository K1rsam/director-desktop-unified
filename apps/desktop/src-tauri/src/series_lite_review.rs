//! Desktop response contract for the frozen lite reviewer. This validates the
//! report's bindings, not whether a model actually read or understood the draft.
//! The controller must independently prove the persisted reviewer execution and
//! its exclusion from ALL creative contexts before granting ContentReview.
use crate::series_lite_edit::{parse, sha256};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(crate) const VERSION: &str = "desktop-lite-content-review/v1";
pub(crate) const RULE: &str = "只交一份独审JSON。bundleSha256绑定当前完整候选原字节；sourceRowIds和unitIds按实际阅读的完整顺序填写，不以坐标引用代替检查演出。问题给具体代码、对象坐标、源依据、影响和说明。priorIssues每项必须保留为未决issue，或写closedIssues的本版回查依据，不能只删标签。只有全部必修问题关闭才能pass。boundary按实际供给的相邻稿核对；材料缺失须pending，不能编造已通过。此响应不修改正文。";
type Result<T> = std::result::Result<T, String>;
fn need(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("LITE_REVIEW:{code}"))
    }
}
fn nonempty(v: &str) -> bool {
    !v.trim().is_empty()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Issue {
    pub id: String,
    pub code: String,
    pub unit_ids: Vec<String>,
    pub source_refs: Vec<String>,
    pub detail: String,
    pub impact: String,
    pub must_fix: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Closure {
    pub issue_id: String,
    pub unit_ids: Vec<String>,
    pub source_refs: Vec<String>,
    pub basis: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Boundary {
    /// not_applicable, pending, pass or revise; determined against supplied facts.
    pub status: String,
    pub previous_bundle_sha256: Option<String>,
    pub previous_unit_id: Option<String>,
    pub current_unit_id: Option<String>,
    pub basis: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Report {
    pub version: String,
    pub bundle_sha256: String,
    pub source_row_ids: Vec<String>,
    pub unit_ids: Vec<String>,
    pub issues: Vec<Issue>,
    pub unresolved: Vec<String>,
    pub closed_issues: Vec<Closure>,
    pub boundary: Boundary,
    pub verdict: String,
    pub summary: String,
}
/// Constructed by the controller from frozen scope and actual adjacent delivery;
/// never deserialize this from the reviewer's self-reported response.
pub(crate) enum BoundaryInput<'a> {
    FirstInScope,
    MissingPrevious,
    Supplied {
        previous_bytes: &'a [u8],
        previous_sha256: &'a str,
    },
}
#[derive(Debug)]
pub(crate) struct Checked {
    pub report: Report,
    pub response_sha256: String,
    /// Consistent model verdict only; independent execution proof is separate.
    pub declared_content_pass: bool,
    pub boundary_pending: bool,
}
fn names(value: &crate::series_lite_edit::Json, field: &str) -> Result<Vec<String>> {
    value
        .array()?
        .iter()
        .map(|v| v.get(field)?.string().map(str::to_owned))
        .collect()
}
fn unique(values: &[String]) -> bool {
    values.iter().all(|v| nonempty(v))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn coordinates(
    units: &[String],
    rows: &[String],
    all_units: &[String],
    all_rows: &[String],
) -> Result<()> {
    need(
        !units.is_empty() || !rows.is_empty(),
        "ISSUE_COORDINATES_REQUIRED",
    )?;
    need(
        unique(units)
            && unique(rows)
            && units.iter().all(|id| all_units.contains(id))
            && rows.iter().all(|id| all_rows.contains(id)),
        "ISSUE_COORDINATES_UNKNOWN",
    )
}
pub(crate) fn validate(
    raw: &[u8],
    bundle: &[u8],
    expected_bundle_sha256: &str,
    previous_issues: &[Issue],
    boundary: BoundaryInput<'_>,
) -> Result<Checked> {
    need(sha256(bundle) == expected_bundle_sha256, "BUNDLE_DRIFT")?;
    // Reject duplicate keys before serde's convenience parsing of metadata.
    parse(raw)?;
    let report: Report =
        serde_json::from_slice(raw).map_err(|e| format!("LITE_REVIEW:SHAPE:{e}"))?;
    need(
        report.version == VERSION && report.bundle_sha256 == expected_bundle_sha256,
        "REVIEW_VERSION_OR_DRAFT_MISMATCH",
    )?;
    let candidate = parse(bundle)?;
    let episodes = candidate.get("episodes")?.array()?;
    need(episodes.len() == 1, "SINGLE_EPISODE_REQUIRED")?;
    let ep = &episodes[0];
    let rows = names(ep.get("source")?.get("sourceRows")?, "id")?;
    let units = names(ep.get("draft")?.get("units")?, "id")?;
    need(
        !rows.is_empty() && !units.is_empty() && unique(&rows) && unique(&units),
        "CANDIDATE_SCOPE_INVALID",
    )?;
    need(
        report.source_row_ids == rows && report.unit_ids == units,
        "INCOMPLETE_READING_RANGE",
    )?;
    need(nonempty(&report.summary), "REVIEW_SUMMARY_REQUIRED")?;
    let mut ids = BTreeSet::new();
    for issue in &report.issues {
        need(
            nonempty(&issue.id)
                && ids.insert(issue.id.as_str())
                && nonempty(&issue.code)
                && nonempty(&issue.detail)
                && nonempty(&issue.impact),
            "ISSUE_INVALID",
        )?;
        coordinates(&issue.unit_ids, &issue.source_refs, &units, &rows)?;
    }
    need(
        unique(&report.unresolved) && report.unresolved.iter().all(|id| ids.contains(id.as_str())),
        "UNRESOLVED_UNKNOWN",
    )?;
    need(
        report
            .issues
            .iter()
            .filter(|i| i.must_fix)
            .all(|i| report.unresolved.contains(&i.id)),
        "REQUIRED_ISSUE_HIDDEN",
    )?;
    let mut prior = BTreeSet::new();
    for issue in previous_issues {
        need(
            nonempty(&issue.id) && prior.insert(issue.id.as_str()),
            "PRIOR_ISSUE_INVALID",
        )?;
    }
    let mut closed = BTreeSet::new();
    for closure in &report.closed_issues {
        need(
            prior.contains(closure.issue_id.as_str())
                && closed.insert(closure.issue_id.as_str())
                && !ids.contains(closure.issue_id.as_str())
                && nonempty(&closure.basis),
            "CLOSURE_INVALID",
        )?;
        coordinates(&closure.unit_ids, &closure.source_refs, &units, &rows)?;
    }
    for old in previous_issues {
        need(
            closed.contains(old.id.as_str()) || report.unresolved.contains(&old.id),
            "PRIOR_ISSUE_SILENTLY_DROPPED",
        )?;
        if let Some(current) = report.issues.iter().find(|i| i.id == old.id) {
            need(
                current.code == old.code && (!old.must_fix || current.must_fix),
                "ISSUE_DOWNGRADED_OR_REIDENTIFIED",
            )?;
        }
    }
    let b = &report.boundary;
    need(nonempty(&b.basis), "BOUNDARY_BASIS_REQUIRED")?;
    let boundary_pending = match boundary {
        BoundaryInput::FirstInScope => {
            need(
                b.status == "not_applicable"
                    && b.previous_bundle_sha256.is_none()
                    && b.previous_unit_id.is_none()
                    && b.current_unit_id.is_none(),
                "UNSUPPLIED_BOUNDARY_CLAIM",
            )?;
            false
        }
        BoundaryInput::MissingPrevious => {
            need(
                b.status == "pending"
                    && b.previous_bundle_sha256.is_none()
                    && b.previous_unit_id.is_none()
                    && b.current_unit_id.is_none(),
                "MISSING_BOUNDARY_CLAIMED_PASS",
            )?;
            true
        }
        BoundaryInput::Supplied {
            previous_bytes,
            previous_sha256,
        } => {
            need(
                sha256(previous_bytes) == previous_sha256,
                "PREVIOUS_DRAFT_DRIFT",
            )?;
            let previous = parse(previous_bytes)?;
            let previous_episodes = previous.get("episodes")?.array()?;
            need(previous_episodes.len() == 1, "PREVIOUS_EPISODE_SCOPE")?;
            let previous_units = names(previous_episodes[0].get("draft")?.get("units")?, "id")?;
            need(
                !previous_units.is_empty()
                    && matches!(b.status.as_str(), "pass" | "revise")
                    && b.previous_bundle_sha256.as_deref() == Some(previous_sha256)
                    && b.previous_unit_id.as_ref() == previous_units.last()
                    && b.current_unit_id.as_ref() == units.first(),
                "BOUNDARY_VERSION_OR_RANGE",
            )?;
            if b.status == "revise" {
                need(
                    !report.unresolved.is_empty(),
                    "BOUNDARY_FAILURE_WITHOUT_ISSUE",
                )?;
            }
            false
        }
    };
    let declared_content_pass = report.unresolved.is_empty() && b.status != "revise";
    need(
        report.verdict
            == if declared_content_pass {
                "pass"
            } else {
                "revise"
            },
        "VERDICT_CONTRADICTS_ISSUES",
    )?;
    Ok(Checked {
        report,
        response_sha256: sha256(raw),
        declared_content_pass,
        boundary_pending,
    })
}

/// Strict metadata schema, while creative bundle numbers never traverse Value.
pub(crate) fn schema() -> Value {
    fn object(p: Value) -> Value {
        json!({"type":"object","additionalProperties":false,"required":p.as_object().unwrap().keys().collect::<Vec<_>>(),"properties":p})
    }
    let text = json!({"type":"string"});
    let nullable = json!({"type":["string","null"]});
    let list = json!({"type":"array","items":text});
    let issue = object(
        json!({"id":text,"code":text,"unitIds":list,"sourceRefs":list,"detail":text,"impact":text,"mustFix":{"type":"boolean"}}),
    );
    let closure = object(json!({"issueId":text,"unitIds":list,"sourceRefs":list,"basis":text}));
    let boundary = object(
        json!({"status":{"type":"string","enum":["not_applicable","pending","pass","revise"]},"previousBundleSha256":nullable,"previousUnitId":nullable,"currentUnitId":nullable,"basis":text}),
    );
    object(
        json!({"version":{"type":"string","enum":[VERSION]},"bundleSha256":text,"sourceRowIds":list,"unitIds":list,"issues":{"type":"array","items":issue},"unresolved":list,"closedIssues":{"type":"array","items":closure},"boundary":boundary,"verdict":{"type":"string","enum":["pass","revise"]},"summary":text}),
    )
}

#[cfg(test)]
#[path = "series_lite_review_tests.rs"]
mod tests;
