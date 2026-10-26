//! Explicit native package adapter. The mapping is a desktop transport record,
//! never an invented upstream manifest or a substitute for original bytes.
use crate::{
    lingmo_package::{self, VerifiedPackage},
    module22::{self, check, rows},
    module22_json as js,
    pipeline::digest,
    store::{s, Result},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

pub(crate) const MAP_VERSION: &str = "desktop-module22-file-map/v1";
fn bytes_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
struct Files<'a> {
    root: &'a Path,
    files: BTreeMap<String, Vec<u8>>,
    total: u64,
    expected: HashMap<String, String>,
}
impl Files<'_> {
    fn bytes(&mut self, name: &str) -> Result<Vec<u8>> {
        if let Some(v) = self.files.get(name) {
            return Ok(v.clone());
        }
        let bytes = lingmo_package::read(&lingmo_package::relative(self.root, name)?)?;
        self.total += bytes.len() as u64;
        check(
            self.total <= lingmo_package::MAX_TOTAL,
            "PACKAGE_TOTAL_LIMIT",
        )?;
        if let Some(expected) = self.expected.get(name) {
            check(bytes_hash(&bytes) == *expected, "PACKAGE_FILE_HASH")?;
        }
        self.files.insert(name.into(), bytes.clone());
        Ok(bytes)
    }
    fn json(&mut self, name: &str) -> Result<Value> {
        serde_json::from_slice(&self.bytes(name)?).map_err(|e| e.to_string())
    }
    fn text(&mut self, name: &str) -> Result<String> {
        String::from_utf8(self.bytes(name)?).map_err(|e| e.to_string())
    }
}
pub(crate) fn load(root: &Path) -> Result<VerifiedPackage> {
    let map: Value = serde_json::from_slice(&lingmo_package::read(&lingmo_package::relative(
        &lingmo_package::root_path(root)?,
        "module22-map.json",
    )?)?)
    .map_err(|e| e.to_string())?;
    load_mapped(root, &map)
}
fn file(mapping: &Value, key: &str) -> Result<String> {
    mapping[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("MODULE22_REQUIRED_FILE:{key}"))
}
fn adopted(a: &Value) -> bool {
    matches!(
        s(&a["design"], "state"),
        "TEXT_ADOPTED" | "REFERENCE_ADOPTED"
    )
}
fn design_projection(a: &Value) -> Value {
    let mut result = a.clone();
    result["assetRevision"] = a["revision"].clone();
    result["variantId"] = json!(format!("module22:{}@{}", s(a, "id"), a["revision"]));
    result["adoptedDesign"] = json!({"version":"module22-adopted-design-projection/v1","designRevision":a["revision"],"state":if adopted(a){"adopted"}else{"stale"},"status":if adopted(a){"adopted"}else{"stale"},"full":a["design"]["full"],"text":a["design"]["text"],"designDependencies":a["design"]["dependencies"],"upstreamState":a["design"]["state"]});
    if result["adoptedDesign"]["designDependencies"].is_null() {
        result["adoptedDesign"]["designDependencies"] = json!([]);
    }
    result["designDependencies"] = result["adoptedDesign"]["designDependencies"].clone();
    result
}
fn declared_attachments(v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(m) => {
            if let (Some(p), Some(h)) = (
                m.get("path").and_then(Value::as_str),
                m.get("sha256").and_then(Value::as_str),
            ) {
                out.push((p.into(), h.into()));
            }
            for v in m.values() {
                declared_attachments(v, out);
            }
        }
        Value::Array(a) => {
            for v in a {
                declared_attachments(v, out);
            }
        }
        _ => {}
    }
}
fn mapped_path<'a>(mapping: &'a Value, path: &'a str) -> &'a str {
    mapping["relocations"][path].as_str().unwrap_or(path)
}
pub(crate) fn load_mapped(directory: &Path, mapping: &Value) -> Result<VerifiedPackage> {
    check(mapping["version"] == MAP_VERSION, "PACKAGE_MAP_VERSION")?;
    let root = lingmo_package::root_path(directory)?;
    let mut expected = HashMap::new();
    if let Some(list) = mapping.get("files") {
        for row in list.as_array().ok_or("MODULE22_FILE_LIST")? {
            check(
                expected
                    .insert(s(row, "path").to_owned(), s(row, "sha256").to_owned())
                    .is_none(),
                "FILE_DUPLICATE",
            )?;
        }
    }
    let mut files = Files {
        root: &root,
        files: BTreeMap::new(),
        total: 0,
        expected,
    };
    let registry_dir = file(mapping, "registryDirectory")?;
    let head = files.json(&format!("{registry_dir}/current.json"))?;
    let revision = head["revision"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or("MODULE22_REGISTRY_HEAD")?;
    let current_name = format!("{registry_dir}/v{revision:06}.json");
    let current_bytes = files.bytes(&current_name)?;
    check(
        head["version"] == "series-assets-head-v1" && head["sha256"] == bytes_hash(&current_bytes),
        "REGISTRY_HEAD_HASH",
    )?;
    let registry: Value = serde_json::from_slice(&current_bytes).map_err(|e| e.to_string())?;
    check(
        registry["version"] == "series-assets-v1"
            && registry["id"] == head["id"]
            && registry["revision"] == head["revision"],
        "REGISTRY_HEAD",
    )?;
    let queue = files.json(&file(mapping, "imageQueue")?)?;
    let q_revision = queue["assetRegistry"]["revision"]
        .as_u64()
        .ok_or("MODULE22_QUEUE_SNAPSHOT")?;
    let q_name = format!("v{q_revision:06}.json");
    check(
        queue["assetRegistry"]["snapshot"] == q_name,
        "QUEUE_SNAPSHOT",
    )?;
    let q_bytes = files.bytes(&format!("{registry_dir}/{q_name}"))?;
    let q_snapshot: Value = serde_json::from_slice(&q_bytes).map_err(|e| e.to_string())?;
    check(
        q_snapshot["id"] == registry["id"]
            && q_snapshot["revision"] == q_revision
            && q_snapshot["sourceHash"] == queue["sourceHash"],
        "QUEUE_SNAPSHOT",
    )?;
    let source_bytes = files.bytes(&file(mapping, "sourceOriginal")?)?;
    let source_sha = bytes_hash(&source_bytes);
    let source_text = if let Some(name) = mapping["sourceText"].as_str() {
        files.text(name)?
    } else {
        String::new()
    };
    let initial = files.json(&file(mapping, "initialRegistry")?)?;
    check(
        initial["version"] == "series-assets-v1"
            && initial["id"] == registry["id"]
            && initial["sourceHash"] == registry["sourceHash"]
            && initial["revision"].as_u64().is_some_and(|r| r <= revision),
        "INITIAL_REGISTRY",
    )?;
    let queue_receipt = module22::validate_batch(
        &queue,
        &module22::batch(&queue, rows(&queue, "items")?),
        &registry,
        &bytes_hash(&q_bytes),
        &source_sha,
    )?;
    let bundle_name = file(mapping, "bundle")?;
    let bundle_bytes = files.bytes(&bundle_name)?;
    let bundle: Value = serde_json::from_slice(&bundle_bytes).map_err(|e| e.to_string())?;
    let lock_bytes = files.bytes(&file(mapping, "sourceLock")?)?;
    let source_lock: Value = serde_json::from_slice(&lock_bytes).map_err(|e| e.to_string())?;
    check(
        bundle["version"] == "series-storyboard-bundle-v1"
            && bundle["testFixture"] == false
            && bundle["exportProfile"] == "lingmo-v7"
            && bundle["sourceHash"] == source_sha,
        "BUNDLE_IDENTITY",
    )?;
    check(
        source_lock["version"] == "series-source-lock-v1"
            && source_lock["sourceHash"] == source_sha
            && bundle["sourceLockHash"] == digest(&js::stringify_raw(&lock_bytes)?),
        "SOURCE_LOCK",
    )?;
    check(
        bundle["assetRegistry"]["id"] == registry["id"]
            && bundle["assetRegistry"]["revision"]
                .as_u64()
                .is_some_and(|r| r > 0 && r <= revision),
        "BUNDLE_REGISTRY",
    )?;
    check(
        bundle["unresolved"].as_array().is_some_and(Vec::is_empty),
        "BUNDLE_UNRESOLVED",
    )?;
    let markdown_name = file(mapping, "markdown")?;
    let markdown = files.text(&markdown_name)?;
    let catalog_name = file(mapping, "catalog")?;
    let catalog_bytes = files.bytes(&catalog_name)?;
    let catalog: Value = serde_json::from_slice(&catalog_bytes).map_err(|e| e.to_string())?;
    check(
        catalog["version"] == "lingmo-v7-assets-v1"
            && catalog["bundleSha256"] == bytes_hash(&bundle_bytes)
            && catalog["markdownSha256"] == digest(&markdown),
        "CATALOG_BYTES",
    )?;
    let consumption = files.json(&file(mapping, "consumptionReceipt")?)?;
    let receipt_registry_bytes = files.bytes(&file(mapping, "consumptionRegistry")?)?;
    let receipt_registry: Value =
        serde_json::from_slice(&receipt_registry_bytes).map_err(|e| e.to_string())?;
    check(
        consumption["version"] == "series-asset-consumption-v1"
            && consumption["sourceHash"] == source_sha
            && consumption["bundleSha256"] == bytes_hash(&bundle_bytes)
            && consumption["registrySha256"] == bytes_hash(&receipt_registry_bytes)
            && consumption["id"] == registry["id"]
            && receipt_registry["id"] == registry["id"]
            && receipt_registry["revision"] == consumption["currentRevision"],
        "CONSUMPTION_RECEIPT",
    )?;
    check(
        mapped_path(mapping, s(&consumption, "registrySnapshot"))
            == file(mapping, "consumptionRegistry")?,
        "RECEIPT_PATH_MAPPING",
    )?;
    let direction_path = file(mapping, "direction")?;
    let direction_text = files.text(&direction_path)?;
    check(!direction_text.trim().is_empty(), "DIRECTION_REQUIRED")?;
    let direction_raw: Value =
        serde_json::from_str(&direction_text).unwrap_or_else(|_| json!({"text":direction_text}));
    let direction = json!({"version":"module22-direction-projection/v1","adoptedDirection":{"version":mapping["directionVersion"],"source":direction_raw},"settings":{"visualStyle":mapping["visualStyle"]},"sourcePath":direction_path});
    check(
        mapping["directionVersion"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
            && mapping["visualStyle"]
                .as_str()
                .is_some_and(|v| !v.trim().is_empty()),
        "DIRECTION_MAPPING_REQUIRED",
    )?;
    let mut attachments = Vec::new();
    for a in rows(&registry, "assets")? {
        declared_attachments(&a["design"]["full"], &mut attachments);
    }
    declared_attachments(&direction_raw, &mut attachments);
    for (path, hash) in attachments {
        check(
            bytes_hash(&files.bytes(mapped_path(mapping, &path))?) == hash,
            "DESIGN_ATTACHMENT_HASH",
        )?;
    }
    if let Some(extra) = mapping.get("attachments") {
        for path in extra.as_array().ok_or("MODULE22_ATTACHMENTS")? {
            files.bytes(path.as_str().ok_or("MODULE22_ATTACHMENT_PATH")?)?;
        }
    }
    let adopted = crate::module22_storyboard::verify(
        &bundle,
        &source_lock,
        &registry,
        &catalog,
        &markdown,
        &queue,
        &consumption,
    )?;
    let mut normalized_registry = registry.clone();
    normalized_registry["assets"] = json!(rows(&registry, "assets")?
        .iter()
        .map(design_projection)
        .collect::<Vec<_>>());
    let mut manifest = mapping.clone();
    manifest["files"] = json!(files
        .files
        .iter()
        .map(|(p, b)| json!({"path":p,"bytes":b.len(),"sha256":bytes_hash(b)}))
        .collect::<Vec<_>>());
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
    let fingerprint = bytes_hash(&manifest_bytes);
    let proof = json!({"level":"upstream-module22-package","sourceFormat":module22::FORMAT,"adapterVersion":"module22-native-adapter/v1","manifestSha256":fingerprint,"packageFingerprint":fingerprint,"sourceSha256":source_sha,"registryId":registry["id"],"registryRevision":registry["revision"],"registrySha256":digest(&normalized_registry.to_string()),"originalRegistrySha256":bytes_hash(&current_bytes),"originalCatalogBytesSha256":bytes_hash(&catalog_bytes),"catalogSha256":digest(&catalog.to_string()),"queueSnapshotSha256":bytes_hash(&q_bytes),"queueHash":queue["queueHash"],"productionScopeHash":queue["productionScopeHash"],"directionSha256":digest(&direction.to_string()),"explicitImageCount":rows(&queue,"items")?.len(),"registryAssetCount":rows(&registry,"assets")?.len(),"upstreamModule22QueueVerified":true,"validatorImplementation":"rust-module22/v1","officialValidatorExecuted":false,"adoptionEvidence":"existing-upstream-consumption-receipt","independentDesktopReview":false,"imageApproval":false,"validation":adopted});
    Ok(VerifiedPackage {
        value: json!({"version":"desktop-complete-handoff/v1","proof":proof,"manifest":manifest,"catalog":catalog,"registry":normalized_registry,"upstreamRegistry":registry,"initialRegistry":initial,"direction":direction,"queue":queue,"sourceLock":source_lock,"sourceText":source_text,"markdownText":markdown,"episodes":bundle["episodes"],"upstreamBundle":bundle,"consumptionReceipt":consumption,"module22ImportReceipt":queue_receipt}),
        files: files.files,
        manifest_bytes,
        manifest_name: "module22-map.json".into(),
    })
}
