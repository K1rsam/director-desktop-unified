//! Frozen lite 0.1.2 local mechanical execution/recovery. No model dispatch,
//! independent content approval or desktop adoption. Receipts always NOT_PROVEN.
//! Inputs stay byte-locked; caller additionally owns workspace/task SQL leases.
use crate::series_lite_bundle::{
    export_lingmo_v7, sync_asset_consumption, verify_lingmo_v7, BundleGate,
};
use crate::series_lite_bundle_gate::NativeLegacyBundleGate;
use crate::series_lite_contract::VerifiedPack;
use crate::series_lite_edit::{canonical, fields, parse, sha256, Json};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
#[path = "series_lite_execution_inputs.rs"]
mod binding;
#[path = "series_lite_execution_fs.rs"]
mod filesystem;
#[path = "series_lite_execution_reports.rs"]
mod reports;
#[path = "series_lite_execution_runtime.rs"]
mod runtime;
use binding::{inputs, BoundInputs};
use filesystem::Job;
type Result<T> = std::result::Result<T, String>;
pub const REQUEST: &str = "series-episode-execution-v1";
pub const RECEIPT: &str = "series-episode-adoption-v1";
pub const NATIVE_CONTRACT: &str = "series-lite-native-execution/v1";
fn err(s: &str) -> String {
    format!("EPISODE_EXECUTION: {s}")
}
fn need(ok: bool, s: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(err(s))
    }
}
fn hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn text(s: impl Into<String>) -> Json {
    Json::String(s.into())
}
fn object<const N: usize>(rows: [(&str, Json); N]) -> Json {
    Json::Object(rows.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}
fn s<'a>(v: &'a Json, k: &str) -> Result<&'a str> {
    v.get(k)?.string()
}
fn from_value(v: &Value) -> Result<Json> {
    parse(&serde_json::to_vec(v).map_err(|_| err("metadata JSON failed"))?)
}
// Original node() parses Node's JSON.stringify output. Keep its numeric report
// convention (10, not serde's 10.0); never apply this to frozen source inputs.
fn from_native_report(v: &Value) -> Result<Json> {
    parse(crate::series_lite_registry::canonical(v)?.as_bytes())
}
fn number_equal(a: &Json, b: &Json) -> bool {
    fn normalized(v: &Json) -> Option<(bool, String, i128)> {
        let Json::Number { canonical: raw, .. } = v else {
            return None;
        };
        let (mantissa, exponent) = raw.split_once(['e', 'E']).unwrap_or((raw, "0"));
        let mut exponent: i128 = exponent.parse().ok()?;
        let negative = mantissa.starts_with('-');
        let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        exponent -= fraction.len() as i128;
        let mut digits = format!("{whole}{fraction}")
            .trim_start_matches('0')
            .to_string();
        if digits.is_empty() {
            return Some((false, "0".into(), 0));
        }
        while digits.ends_with('0') {
            digits.pop();
            exponent += 1;
        }
        Some((negative, digits, exponent))
    }
    match (normalized(a), normalized(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}
fn value(v: &Json) -> Result<Value> {
    serde_json::from_slice(&canonical(v))
        .map_err(|_| err("native downstream JSON range unsupported"))
}
fn put(v: &mut Json, key: &str, item: Json) -> Result<()> {
    match v {
        Json::Object(o) => {
            o.insert(key.to_owned(), item);
            Ok(())
        }
        _ => Err(err("expected object")),
    }
}

#[derive(Debug)]
pub struct InputEvidence {
    pub signature_utf8: Vec<u8>,
    pub execution_fingerprint: String,
    pub registry_snapshot_utf8: Vec<u8>,
}
#[derive(Debug)]
pub struct ExecutionResult {
    pub reused: bool,
    pub receipt_path: String,
    pub receipt_file_sha256: String,
    pub steps_run: Vec<String>,
    pub preview_path: String,
    pub content_review: &'static str,
}
#[derive(Debug)]
pub struct VerifiedReceipt {
    pub bytes: Vec<u8>,
    pub file_sha256: String,
    pub execution_fingerprint: String,
    pub preview_path: String,
    pub content_review: &'static str,
}
pub fn inspect_inputs(root: &Path, manifest: &str, pack: &VerifiedPack) -> Result<InputEvidence> {
    let job = Job::new(root)?;
    let bound = inputs(&job, manifest, Some(pack), None)?;
    Ok(InputEvidence {
        signature_utf8: canonical(&bound.signature),
        execution_fingerprint: bound.fingerprint,
        registry_snapshot_utf8: canonical(&bound.registry.snapshot),
    })
}
/// Requires an independently frozen SQL/attempt-ledger receipt file pin. Never
/// derive it from the receipt being verified. A matching receipt is mechanical
/// evidence only and cannot close independent review issues.
pub fn verify_receipt(
    root: &Path,
    path: &str,
    trusted_file_sha256: &str,
    episode: Option<i64>,
) -> Result<VerifiedReceipt> {
    verify_bound(&Job::new(root)?, path, trusted_file_sha256, episode)
}
fn verify_bound(
    job: &Job,
    path: &str,
    trusted: &str,
    episode: Option<i64>,
) -> Result<VerifiedReceipt> {
    let raw = job.read(path)?;
    need(
        hex(trusted) && sha256(&raw) == trusted,
        "receipt external pin mismatch; retain for reconciliation",
    )?;
    let receipt = parse(&raw)?;
    fields(
        &receipt,
        &[
            "version",
            "episode",
            "manifest",
            "bundle",
            "executionFingerprint",
            "inputs",
            "steps",
            "registrySnapshot",
            "outputs",
            "preview",
            "contentReview",
            "receiptSha256",
        ],
        &[],
    )?;
    need(
        s(&receipt, "version")? == RECEIPT && s(&receipt, "contentReview")? == "NOT_PROVEN",
        "invalid mechanical receipt",
    )?;
    let mut unsigned = receipt.object()?.clone();
    unsigned.remove("receiptSha256");
    need(
        sha256(&canonical(&Json::Object(unsigned))) == s(&receipt, "receiptSha256")?,
        "receipt changed",
    )?;
    if let Some(n) = episode {
        need(
            number_equal(receipt.get("episode")?, &parse(n.to_string().as_bytes())?),
            "receipt episode differs",
        )?;
    }
    let bound = inputs(
        job,
        s(&receipt, "manifest")?,
        None,
        Some(receipt.get("inputs")?),
    )?;
    need(
        bound.fingerprint == s(&receipt, "executionFingerprint")?
            && bound.signature == *receipt.get("inputs")?,
        "execution inputs changed; retain old candidate",
    )?;
    need(
        number_equal(receipt.get("episode")?, bound.request.get("episode")?)
            && *receipt.get("bundle")? == job.record(s(&bound.request, "bundle")?)?,
        "receipt bundle mismatch",
    )?;
    let mut paths = BTreeSet::new();
    for record in receipt.get("outputs")?.array()? {
        job.record_matches(record)?;
        need(
            paths.insert(s(record, "path")?.to_string()),
            "duplicate receipt output",
        )?;
    }
    job.record_matches(receipt.get("registrySnapshot")?)?;
    let steps = receipt
        .get("steps")?
        .array()?
        .iter()
        .map(|v| v.string())
        .collect::<Result<BTreeSet<_>>>()?;
    need(
        steps == ["sync", "check", "export", "verify"].into_iter().collect()
            && receipt.get("steps")?.array()?.len() == 4,
        "incomplete execution steps",
    )?;
    let preview = s(&receipt, "preview")?;
    need(
        paths.contains(preview) && paths.contains(&format!("{preview}.assets.json")),
        "receipt omits actual delivery",
    )?;
    Ok(VerifiedReceipt {
        bytes: raw,
        file_sha256: trusted.to_owned(),
        execution_fingerprint: bound.fingerprint,
        preview_path: preview.into(),
        content_review: "NOT_PROVEN",
    })
}
struct NativeStages<'a> {
    bound: &'a BoundInputs,
    registry: Value,
    lock: Vec<u8>,
    scope: Option<Value>,
}
impl<'a> NativeStages<'a> {
    fn new(job: &Job, bound: &'a BoundInputs) -> Result<Self> {
        Ok(Self {
            bound,
            registry: value(&bound.registry.value)?,
            lock: job.read(s(&bound.request, "sourceLock")?)?,
            scope: bound
                .request
                .object()?
                .get("seasonScope")
                .map(|p| job.json(p.string()?).and_then(|v| value(&v)))
                .transpose()?,
        })
    }
    fn gate(&self) -> NativeLegacyBundleGate<'_> {
        NativeLegacyBundleGate {
            registry: Some(&self.registry),
            source_lock: Some(&self.lock),
            season_scope: self.scope.as_ref(),
        }
    }
    fn check(&self) -> Result<Value> {
        self.gate().check(&self.bound.bundle).map(|e| e.report)
    }
}
fn finish(
    job: &Job,
    run: &Path,
    state: &mut Json,
    name: &str,
    result: Json,
    extra: &[PathBuf],
) -> Result<()> {
    let report = run.join(format!("{name}.json"));
    job.atomic(&report, &result)?;
    let mut records = vec![job.record(&job.relative(&report)?)?];
    for path in extra {
        records.push(job.record(&job.relative(path)?)?);
    }
    let mut steps = state.get("steps")?.clone();
    put(&mut steps, name, Json::Array(records))?;
    put(state, "steps", steps)?;
    job.atomic(&run.join("execution.json"), state)
}
/// Run/resume local mechanics. Existing completed outputs require the prior
/// receipt pin; lack of it is recoverable reconciliation, never a fresh submit.
pub fn run(
    root: &Path,
    manifest: &str,
    output: &str,
    pack: &VerifiedPack,
    completed_receipt_pin: Option<&str>,
) -> Result<ExecutionResult> {
    run_inner(
        root,
        manifest,
        output,
        pack,
        completed_receipt_pin,
        &mut |_| Ok(()),
    )
}
fn run_inner(
    root: &Path,
    manifest: &str,
    output: &str,
    pack: &VerifiedPack,
    pin: Option<&str>,
    hook: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<ExecutionResult> {
    let job = Job::new(root)?;
    let run = job.path(output)?;
    let state_file = run.join("execution.json");
    if run.exists() && !state_file.is_file() {
        need(
            run.is_dir()
                && fs::read_dir(&run)
                    .map_err(|_| err("output unreadable"))?
                    .next()
                    .is_none(),
            "output must be new/empty or execution run",
        )?;
    }
    job.ensure_dir(&run)?;
    let _workspace_lock = job.lock(&job.root.join(".state.lock"))?;
    let _run_lock = job.lock(&run.join(".state.lock"))?;
    let receipt_path = run.join("adoption.json");
    let rel_receipt = job.relative(&receipt_path)?;
    if receipt_path.exists() {
        let receipt = verify_bound(
            &job,
            &rel_receipt,
            pin.ok_or_else(|| {
                err("completed receipt needs external pin; reconcile without rerun")
            })?,
            None,
        )?;
        need(
            s(&parse(&receipt.bytes)?, "manifest")? == manifest,
            "output belongs to another request",
        )?;
        return Ok(ExecutionResult {
            reused: true,
            receipt_path: rel_receipt,
            receipt_file_sha256: receipt.file_sha256,
            steps_run: vec![],
            preview_path: receipt.preview_path,
            content_review: "NOT_PROVEN",
        });
    }
    need(
        pin.is_none(),
        "pinned completed receipt missing; do not recreate",
    )?;
    let bound = inputs(&job, manifest, Some(pack), None)?;
    let mut state = if state_file.exists() {
        job.json(&job.relative(&state_file)?)?
    } else {
        from_value(
            &json!({"version":REQUEST,"executionFingerprint":bound.fingerprint,"steps":{},"attempt":0}),
        )?
    };
    fields(
        &state,
        &["version", "executionFingerprint", "steps", "attempt"],
        &["preview"],
    )?;
    need(
        s(&state, "version")? == REQUEST && s(&state, "executionFingerprint")? == bound.fingerprint,
        "execution inputs changed; use new candidate/output",
    )?;
    let attempt = state
        .get("attempt")?
        .integer()
        .filter(|n| *n >= 0 && *n < i64::MAX as i128)
        .ok_or_else(|| err("invalid export attempt"))?;
    let steps = state.get("steps")?.object()?;
    need(
        steps
            .keys()
            .all(|k| ["sync", "check", "export", "verify"].contains(&k.as_str())),
        "unknown execution step",
    )?;
    for (i, name) in ["sync", "check", "export", "verify"].iter().enumerate() {
        if steps.contains_key(*name) {
            need(
                ["sync", "check", "export", "verify"][..i]
                    .iter()
                    .all(|n| steps.contains_key(*n)),
                "execution journal skips prerequisite",
            )?;
        }
    }
    let mut protected = bound
        .signature
        .get("files")?
        .array()?
        .iter()
        .map(|r| s(r, "path"))
        .collect::<Result<Vec<_>>>()?;
    protected.extend(
        bound
            .packet
            .get("bindings")?
            .array()?
            .iter()
            .map(|r| s(r, "path"))
            .collect::<Result<Vec<_>>>()?,
    );
    protected.push(s(&bound.request, "registry")?);
    protected.push(s(bound.packet.get("review")?, "path")?);
    protected.push(s(bound.packet.get("requestOrigin")?, "path")?);
    for path in protected {
        need(
            !job.path(path)?.starts_with(&run),
            "execution output overlaps input",
        )?;
    }
    for records in steps.values() {
        for record in records.array()? {
            job.record_matches(record)?;
        }
    }
    job.atomic(&state_file, &state)?;
    let native = NativeStages::new(&job, &bound)?;
    let bundle_value = parse(&bound.bundle)?;
    let mut image_keys = BTreeSet::new();
    for episode in bundle_value.get("episodes")?.array()? {
        for unit in episode.get("draft")?.get("units")?.array()? {
            for key in unit.get("imageBindings")?.array()? {
                image_keys.insert(key.string()?);
            }
        }
    }
    let image_assets = image_keys.len();
    let mut performed = vec![];
    if !state.get("steps")?.object()?.contains_key("sync") {
        let consumption = run.join("consumption.json");
        if !consumption.exists() {
            let absolute = job.path(s(&bound.registry.snapshot, "path")?)?;
            let mut report = sync_asset_consumption(
                &bound.bundle,
                &native.registry,
                &absolute.to_string_lossy(),
                &bound.registry.raw,
                &mut native.gate(),
            )?;
            if let Some(scope) = native.scope.as_ref() {
                report["productionScopeHash"] =
                    json!(crate::series_lite_registry::hash_season_scope(scope)?);
            }
            let mut raw = canonical(&from_native_report(&report)?);
            raw.push(b'\n');
            job.exclusive(&consumption, &raw)?;
            hook("after_sync_write")?;
        } else {
            let prior = job.json(&job.relative(&consumption)?)?;
            need(
                s(&prior, "bundleSha256")? == sha256(&bound.bundle),
                "interrupted consumption belongs to other bundle",
            )?;
            let snapshot = job.snapshot_absolute(s(&prior, "registrySnapshot")?)?;
            need(
                sha256(&fs::read(snapshot).map_err(|_| err("snapshot read failed"))?)
                    == s(&prior, "registrySha256")?,
                "interrupted snapshot changed",
            )?;
        }
        finish(
            &job,
            &run,
            &mut state,
            "sync",
            from_value(&json!({"ok":true}))?,
            &[consumption],
        )?;
        performed.push("sync".into());
    }
    if !state.get("steps")?.object()?.contains_key("check") {
        finish(
            &job,
            &run,
            &mut state,
            "check",
            from_native_report(&native.check()?)?,
            &[],
        )?;
        performed.push("check".into());
    }
    if !state.get("steps")?.object()?.contains_key("export") {
        hook("before_export")?;
        let mut preview = run.join(format!("export-{attempt}/preview.md"));
        let mut catalog = PathBuf::from(format!("{}.assets.json", preview.display()));
        let report = if preview.exists() && catalog.exists() {
            let md = String::from_utf8(job.read(&job.relative(&preview)?)?)
                .map_err(|_| err("preview is not UTF-8"))?;
            let cat = String::from_utf8(job.read(&job.relative(&catalog)?)?)
                .map_err(|_| err("catalog is not UTF-8"))?;
            verify_lingmo_v7(&bound.bundle, &md, &cat, &mut native.gate())?;
            reports::export_report(
                &native.check()?,
                &md,
                &preview,
                &catalog,
                true,
                image_assets,
            )
        } else {
            if preview.exists() || catalog.exists() {
                put(
                    &mut state,
                    "attempt",
                    parse((attempt + 1).to_string().as_bytes())?,
                )?;
                job.atomic(&state_file, &state)?;
                preview = run.join(format!("export-{}/preview.md", attempt + 1));
                catalog = PathBuf::from(format!("{}.assets.json", preview.display()));
            }
            let delivery = export_lingmo_v7(&bound.bundle, &mut native.gate())?;
            job.exclusive(&preview, delivery.markdown.as_bytes())?;
            hook("after_preview_write")?;
            job.exclusive(&catalog, delivery.manifest_json.as_bytes())?;
            hook("after_export_write")?;
            reports::export_report(
                &delivery.gate_report,
                &delivery.markdown,
                &preview,
                &catalog,
                false,
                image_assets,
            )
        };
        put(&mut state, "preview", text(job.relative(&preview)?))?;
        finish(
            &job,
            &run,
            &mut state,
            "export",
            from_native_report(&report)?,
            &[preview, catalog],
        )?;
        performed.push("export".into());
    }
    if !state.get("steps")?.object()?.contains_key("verify") {
        let preview = job.path(s(&state, "preview")?)?;
        let catalog = PathBuf::from(format!("{}.assets.json", preview.display()));
        let md = String::from_utf8(job.read(&job.relative(&preview)?)?)
            .map_err(|_| err("preview is not UTF-8"))?;
        let cat = String::from_utf8(job.read(&job.relative(&catalog)?)?)
            .map_err(|_| err("catalog is not UTF-8"))?;
        verify_lingmo_v7(&bound.bundle, &md, &cat, &mut native.gate())?;
        let report = reports::export_report(
            &native.check()?,
            &md,
            &preview,
            &catalog,
            true,
            image_assets,
        );
        finish(
            &job,
            &run,
            &mut state,
            "verify",
            from_native_report(&report)?,
            &[],
        )?;
        performed.push("verify".into());
    }
    hook("before_receipt")?;
    let current = inputs(&job, manifest, Some(pack), None)?;
    need(
        current.fingerprint == bound.fingerprint && current.signature == bound.signature,
        "inputs changed during execution; no receipt written",
    )?;
    // Recheck all output pins immediately before committing the receipt too.
    let steps = state.get("steps")?.object()?;
    let mut outputs = vec![];
    for group in steps.values() {
        for record in group.array()? {
            job.record_matches(record)?;
            outputs.push(record.clone());
        }
    }
    let mut receipt = object([
        ("version", text(RECEIPT)),
        ("episode", bound.request.get("episode")?.clone()),
        ("manifest", text(manifest)),
        ("bundle", job.record(s(&bound.request, "bundle")?)?),
        ("executionFingerprint", text(bound.fingerprint)),
        ("inputs", bound.signature),
        ("steps", Json::Array(steps.keys().map(text).collect())),
        ("registrySnapshot", bound.registry.snapshot),
        ("outputs", Json::Array(outputs)),
        ("preview", state.get("preview")?.clone()),
        ("contentReview", text("NOT_PROVEN")),
    ]);
    let hash = sha256(&canonical(&receipt));
    put(&mut receipt, "receiptSha256", text(hash))?;
    let mut raw = canonical(&receipt);
    raw.push(b'\n');
    job.atomic_new(&receipt_path, &raw)?;
    Ok(ExecutionResult {
        reused: false,
        receipt_path: rel_receipt,
        receipt_file_sha256: sha256(&raw),
        steps_run: performed,
        preview_path: s(&state, "preview")?.into(),
        content_review: "NOT_PROVEN",
    })
}
#[cfg(test)]
#[path = "series_lite_execution_tests.rs"]
mod tests;
