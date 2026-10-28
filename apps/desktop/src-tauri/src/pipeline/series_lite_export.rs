//! Local lite handoff. Preserve adopted bytes and provenance; export is neither
//! a new creative approval nor the legacy complete-package import contract.
use crate::lingmo_package::{read, relative, root_path, MAX_TOTAL};
use crate::series_lite_bundle::verify_lingmo_v7;
use crate::series_lite_bundle_gate::NativeLegacyBundleGate;
use crate::series_lite_edit::{parse, sha256};
use crate::store::{s, Result, Store};
use crate::workflow_artifacts::{self as artifacts, Adoption, Capability, Kind, StoredArtifact};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};

const VERSION: &str = "desktop-lite-handoff/v1";
fn need(ok: bool, code: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("LITE_EXPORT:{code}"))
    }
}
fn value(raw: &[u8]) -> Result<Value> {
    parse(raw)?; // Reject duplicate keys before using metadata; retain raw bytes.
    serde_json::from_slice(raw).map_err(|e| e.to_string())
}
fn encoded(v: &impl serde::Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(v).map_err(|e| e.to_string())
}
fn pin(root: &Path, record: &Value) -> Result<Vec<u8>> {
    let bytes = read(&relative(root, s(record, "path"))?)?;
    need(sha256(&bytes) == record["sha256"], "FROZEN_FILE_CHANGED")?;
    Ok(bytes)
}
fn file(name: &str, bytes: &[u8]) -> Value {
    json!({"path":name,"sha256":sha256(bytes),"bytes":bytes.len()})
}
fn add(files: &mut BTreeMap<String, Vec<u8>>, name: &str, bytes: Vec<u8>) -> Result<Value> {
    need(!files.contains_key(name), "DUPLICATE_OUTPUT_NAME")?;
    let record = file(name, &bytes);
    files.insert(name.into(), bytes);
    need(
        files.values().map(|v| v.len() as u64).sum::<u64>() <= MAX_TOTAL,
        "TOTAL_LIMIT",
    )?;
    Ok(record)
}
fn final_adoption(adoption: &Adoption) -> Result<()> {
    need(
        [
            Capability::Structure,
            Capability::SourceCoverage,
            Capability::ContentReview,
        ]
        .iter()
        .all(|c| adoption.capabilities.contains(c)),
        "FINAL_ADOPTION_REQUIRED",
    )
}
fn candidate_binding(candidate: &StoredArtifact, adoption: &Adoption, native: &[u8]) -> Result<()> {
    need(
        candidate.artifact.kind == Kind::Candidate
            && candidate.artifact.id == adoption.candidate_id
            && candidate.artifact.identity == adoption.identity
            && candidate.artifact.source_sha256 == adoption.source_sha256
            && candidate.artifact.contract_version == adoption.contract_version
            && candidate.payload_sha256 == adoption.candidate_sha256
            && candidate.input_sha256 == adoption.input_sha256
            && super::digest(&candidate.artifact.payload.to_string()) == candidate.payload_sha256
            && super::digest(
                &serde_json::to_string(&candidate.artifact.inputs).map_err(|e| e.to_string())?,
            ) == candidate.input_sha256
            && s(&candidate.artifact.payload, "text").as_bytes() == native,
        "CANDIDATE_ADOPTION_CHANGED",
    )
}
fn mechanical(
    native: &[u8],
    markdown: &[u8],
    catalog: &[u8],
    lock: &[u8],
    registry: &[u8],
    scope: Option<&[u8]>,
) -> Result<()> {
    value(lock)?;
    let registry = value(registry)?;
    let scope = scope.map(value).transpose()?;
    verify_lingmo_v7(
        native,
        std::str::from_utf8(markdown).map_err(|e| e.to_string())?,
        std::str::from_utf8(catalog).map_err(|e| e.to_string())?,
        &mut NativeLegacyBundleGate {
            registry: Some(&registry),
            source_lock: Some(lock),
            season_scope: scope.as_ref(),
        },
    )?;
    Ok(())
}

fn prepare(store: &Store, records: &[Value]) -> Result<(Value, BTreeMap<String, Vec<u8>>)> {
    need(!records.is_empty(), "EMPTY_SELECTION")?;
    let source = store.source()?;
    let source_hash = s(&source, "sha256");
    need(!source_hash.is_empty(), "SOURCE_MISSING")?;
    let episodes = source["episodes"]
        .as_array()
        .ok_or("LITE_EXPORT:SOURCE_EPISODES_MISSING")?;
    let mut selected = BTreeMap::new();
    for record in records {
        let id = s(record, "episodeId");
        need(
            !id.is_empty() && selected.insert(id, record).is_none(),
            "DUPLICATE_EPISODE",
        )?;
        need(
            episodes.iter().any(|e| e["id"] == id),
            "EPISODE_OUTSIDE_SOURCE",
        )?;
    }
    let mut files = BTreeMap::new();
    let mut exported = Vec::new();
    let mut numbers = BTreeSet::new();
    let conn = store.conn()?;
    for (source_ordinal, ep) in episodes.iter().enumerate() {
        let Some(record) = selected.get(s(ep, "id")).copied() else {
            continue;
        };
        let number = record["episodeNumber"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or("LITE_EXPORT:EPISODE_NUMBER")?;
        need(
            super::number_value(s(ep, "sourceNumber")).map(i64::from) == Some(number)
                && numbers.insert(number),
            "ORIGINAL_EPISODE_NUMBER_CHANGED",
        )?;
        need(
            record["version"] == "desktop-lite-delivery/v1"
                && record["finalDeliveryReady"] == true
                && record["sourceSha256"] == source_hash,
            "UNADOPTED_OR_SOURCE_CHANGED",
        )?;
        let adoption: Adoption =
            serde_json::from_value(record["adoption"].clone()).map_err(|e| e.to_string())?;
        final_adoption(&adoption)?;
        need(
            adoption.source_sha256 == source_hash
                && artifacts::adoption(&conn, &adoption.identity, adoption.version)?.as_ref()
                    == Some(&adoption),
            "ADOPTION_CHANGED",
        )?;
        let candidate = artifacts::artifact(&conn, &adoption.candidate_id)?
            .ok_or("LITE_EXPORT:CANDIDATE_MISSING")?;
        let native = s(&candidate.artifact.payload, "text").as_bytes();
        candidate_binding(&candidate, &adoption, native)?;
        need(
            record["candidateArtifactId"] == candidate.artifact.id
                && record["nativeSha256"] == sha256(native),
            "NATIVE_CHANGED",
        )?;
        let root = root_path(Path::new(s(record, "workspaceRoot")))?;
        let receipt = crate::series_lite_execution::verify_receipt(
            &root,
            s(record, "receiptPath"),
            s(record, "receiptSha256"),
            Some(number),
        )?;
        let receipt_value = value(&receipt.bytes)?;
        need(
            pin(&root, &receipt_value["bundle"])? == native,
            "EXECUTION_CANDIDATE_CHANGED",
        )?;
        let manifest_name = s(&receipt_value, "manifest");
        let manifest_pin = receipt_value["inputs"]["files"]
            .as_array()
            .ok_or("LITE_EXPORT:INPUT_PINS_MISSING")?
            .iter()
            .find(|p| p["path"] == manifest_name)
            .ok_or("LITE_EXPORT:MANIFEST_PIN_MISSING")?;
        let manifest = value(&pin(&root, manifest_pin)?)?;
        let lock_path = s(&manifest, "sourceLock");
        let lock_pin = receipt_value["inputs"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["path"] == lock_path)
            .ok_or("LITE_EXPORT:SOURCE_LOCK_PIN_MISSING")?;
        let lock = pin(&root, lock_pin)?;
        let registry = pin(&root, &receipt_value["registrySnapshot"])?;
        let scope = if let Some(name) = manifest["seasonScope"].as_str() {
            let scope_pin = receipt_value["inputs"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["path"] == name)
                .ok_or("LITE_EXPORT:SCOPE_PIN_MISSING")?;
            Some(pin(&root, scope_pin)?)
        } else {
            None
        };
        let markdown = read(&relative(&root, &receipt.preview_path)?)?;
        let catalog = read(&relative(
            &root,
            &format!("{}.assets.json", receipt.preview_path),
        )?)?;
        need(
            sha256(&markdown) == record["markdownSha256"]
                && sha256(&catalog) == record["manifestSha256"],
            "DELIVERY_CHANGED",
        )?;
        mechanical(
            native,
            &markdown,
            &catalog,
            &lock,
            &registry,
            scope.as_deref(),
        )?;
        let prefix = format!("第{number}集");
        let mut row = json!({"episodeId":ep["id"],"episodeNumber":number,"sourceOrdinal":source_ordinal,
            "nativeSha256":sha256(native),"adoption":adoption,"originalReceiptSha256":receipt.file_sha256,
            "markdown":add(&mut files,&format!("{prefix}.md"),markdown)?,
            "catalog":add(&mut files,&format!("{prefix}.md.assets.json"),catalog)?,
            "bundle":add(&mut files,&format!("{prefix}.bundle.json"),native.to_vec())?,
            "sourceLock":add(&mut files,&format!("{prefix}.source-lock.json"),lock)?,
            "registry":add(&mut files,&format!("{prefix}.registry.json"),registry)?,
            "candidate":add(&mut files,&format!("{prefix}.candidate.json"),encoded(&candidate)?)?,
            "adoptionFile":add(&mut files,&format!("{prefix}.adoption.json"),encoded(&adoption)?)?,
            "executionReceipt":add(&mut files,&format!("{prefix}.execution-receipt.json"),receipt.bytes)?,
            "deliveryRecord":add(&mut files,&format!("{prefix}.delivery.json"),encoded(record)?)?});
        if let Some(scope) = scope {
            row["seasonScope"] = add(&mut files, &format!("{prefix}.season-scope.json"), scope)?;
        }
        exported.push(row);
    }
    need(exported.len() == records.len(), "SELECTION_CHANGED")?;
    Ok((
        json!({"version":VERSION,"sourceSha256":source_hash,"episodes":exported,
        "importContract":{"sourceKind":"imported-md","level":"md-and-catalog","completePackageVerified":false},
        "evidenceBoundary":"Original adopted evidence preserved. Mechanical export verification does not repeat content review or grant new approval.",
        "generationSubmissions":0}),
        files,
    ))
}

/// Independent local readback of the exported bytes. This has no project writes,
/// provider calls or authority to turn the handoff into a newly adopted result.
pub(crate) fn verify_export(path: &Path) -> Result<Value> {
    let root = root_path(path)?;
    let index = value(&read(&relative(&root, "delivery-index.json")?)?)?;
    need(
        index["version"] == VERSION && index["generationSubmissions"] == 0,
        "INDEX_VERSION",
    )?;
    let episodes = index["episodes"]
        .as_array()
        .filter(|e| !e.is_empty())
        .ok_or("LITE_EXPORT:INDEX_EPISODES")?;
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    let mut previous = None;
    for ep in episodes {
        let number = ep["episodeNumber"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or("LITE_EXPORT:INDEX_EPISODE_NUMBER")?;
        let ordinal = ep["sourceOrdinal"]
            .as_u64()
            .ok_or("LITE_EXPORT:INDEX_SOURCE_ORDINAL")?;
        need(
            previous.is_none_or(|p| ordinal > p)
                && seen.insert(format!("episode:{number}"))
                && !s(ep, "episodeId").is_empty()
                && seen.insert(format!("id:{}", s(ep, "episodeId"))),
            "INDEX_EPISODE_ORDER",
        )?;
        previous = Some(ordinal);
        let prefix = format!("第{number}集");
        let mut values = BTreeMap::new();
        for (key, suffix) in [
            ("markdown", ".md"),
            ("catalog", ".md.assets.json"),
            ("bundle", ".bundle.json"),
            ("sourceLock", ".source-lock.json"),
            ("registry", ".registry.json"),
            ("candidate", ".candidate.json"),
            ("adoptionFile", ".adoption.json"),
            ("executionReceipt", ".execution-receipt.json"),
            ("deliveryRecord", ".delivery.json"),
            ("seasonScope", ".season-scope.json"),
        ] {
            if key == "seasonScope" && ep[key].is_null() {
                continue;
            }
            need(
                ep[key]["path"] == format!("{prefix}{suffix}")
                    && seen.insert(s(&ep[key], "path").into()),
                "INDEX_FILENAME",
            )?;
            let bytes = pin(&root, &ep[key])?;
            need(ep[key]["bytes"] == bytes.len(), "INDEX_FILE_LENGTH")?;
            total += bytes.len() as u64;
            need(total <= MAX_TOTAL, "TOTAL_LIMIT")?;
            values.insert(key, bytes);
        }
        let adoption: Adoption =
            serde_json::from_slice(&values["adoptionFile"]).map_err(|e| e.to_string())?;
        let candidate: StoredArtifact =
            serde_json::from_slice(&values["candidate"]).map_err(|e| e.to_string())?;
        let record = value(&values["deliveryRecord"])?;
        let receipt = value(&values["executionReceipt"])?;
        final_adoption(&adoption)?;
        candidate_binding(&candidate, &adoption, &values["bundle"])?;
        need(
            serde_json::to_value(&adoption).map_err(|e| e.to_string())? == ep["adoption"]
                && record["adoption"] == ep["adoption"]
                && record["finalDeliveryReady"] == true
                && record["episodeId"] == ep["episodeId"]
                && record["episodeNumber"] == number
                && record["sourceSha256"] == index["sourceSha256"]
                && adoption.source_sha256 == s(&index, "sourceSha256")
                && record["nativeSha256"] == ep["nativeSha256"]
                && ep["nativeSha256"] == sha256(&values["bundle"])
                && record["candidateArtifactId"] == candidate.artifact.id
                && record["receiptSha256"] == ep["executionReceipt"]["sha256"]
                && ep["originalReceiptSha256"] == ep["executionReceipt"]["sha256"]
                && record["markdownSha256"] == ep["markdown"]["sha256"]
                && record["manifestSha256"] == ep["catalog"]["sha256"]
                && receipt["bundle"]["sha256"] == ep["nativeSha256"]
                && receipt["episode"] == number
                && receipt["registrySnapshot"]["sha256"] == ep["registry"]["sha256"],
            "EXPORTED_BINDING_CHANGED",
        )?;
        mechanical(
            &values["bundle"],
            &values["markdown"],
            &values["catalog"],
            &values["sourceLock"],
            &values["registry"],
            values.get("seasonScope").map(Vec::as_slice),
        )?;
    }
    Ok(
        json!({"version":VERSION,"episodeCount":episodes.len(),"mechanicalVerified":true,
        "contentReview":"PRESERVED_ADOPTION_NOT_REPEATED","generationSubmissions":0,"importLevel":"md-and-catalog"}),
    )
}

fn exclusive(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    // All names below are program-owned single components in a newly created
    // directory; never replace or delete an existing export on failure.
    need(
        Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name),
        "OUTPUT_FILENAME",
    )?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .map_err(|e| e.to_string())?;
    output
        .write_all(bytes)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())?;
    need(
        read(&relative(root, name)?)? == bytes,
        "WRITE_READBACK_CHANGED",
    )
}
pub(crate) fn write(store: &Store, records: &[Value], target_parent: &Path) -> Result<Value> {
    let parent = root_path(target_parent)?;
    need(parent.is_dir(), "TARGET_DIRECTORY_REQUIRED")?;
    let (index, files) = prepare(store, records)?;
    need(
        store.source()?["sha256"] == index["sourceSha256"],
        "SOURCE_CHANGED_DURING_EXPORT",
    )?;
    let child = parent.join(format!(
        "分镜交付-lite-{}-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        &crate::store::id()[..8]
    ));
    fs::create_dir(&child).map_err(|e| e.to_string())?;
    let result = (|| {
        let child = root_path(&child)?;
        for (name, bytes) in &files {
            exclusive(&child, name, bytes)?;
        }
        exclusive(&child, "delivery-index.json", &encoded(&index)?)?;
        let verified = verify_export(&child)?;
        need(
            store.source()?["sha256"] == index["sourceSha256"],
            "SOURCE_CHANGED_DURING_EXPORT",
        )?;
        Ok(
            json!({"path":child,"count":records.len(),"episodeCount":records.len(),"version":VERSION,
            "manifestPath":child.join("delivery-index.json"),"verification":verified,"generationSubmissions":0}),
        )
    })();
    result.map_err(|error: String| {
        format!("{error}；局部导出保留于 {}，未覆盖旧成果", child.display())
    })
}

#[cfg(test)]
#[path = "series_lite_export_tests.rs"]
mod tests;
