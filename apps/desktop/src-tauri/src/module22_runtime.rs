use super::*;
use crate::{lingmo_import, lingmo_package, store::Store};
use std::{fs, io::Write, path::PathBuf, sync::Arc};

const CONTRACT: &str = "module22-image-dispatch/v1";

/// One public queue command owns one context. Nothing is process-global or
/// persisted here. Actual dispatch always constructs a fresh context below.
pub(crate) struct ImageBatchContext {
    root: PathBuf,
    consumers: Option<HashMap<String, Vec<String>>>,
    documents: HashMap<String, Value>,
    packages: HashMap<String, Arc<Value>>,
    package_errors: HashMap<String, String>,
    identities: crate::lingmo_continuation::PackageIdentityContext,
}
impl ImageBatchContext {
    pub(crate) fn new(store: &Store) -> Self {
        Self {
            root: store.root.clone(),
            consumers: None,
            documents: HashMap::new(),
            packages: HashMap::new(),
            package_errors: HashMap::new(),
            identities: crate::lingmo_continuation::PackageIdentityContext::default(),
        }
    }
    fn document_ids(&mut self, store: &Store, asset_id: &str) -> Result<Vec<String>> {
        if self.consumers.is_none() {
            // Build this small identity index once; do not deserialize all shot
            // bodies or repeatedly rescan all consumers for every queue item.
            let connection = store.conn()?;
            let mut query = connection.prepare("SELECT DISTINCT CASE WHEN r.type='object' THEN json_extract(r.value,'$.assetId') ELSE r.value END, json_extract(u.data,'$.importSource.documentId') FROM units AS u, json_each(CASE WHEN json_type(u.data,'$.imageBindings')='array' THEN json_extract(u.data,'$.imageBindings') ELSE json_extract(u.data,'$.assetRefs') END) AS r WHERE json_extract(u.data,'$.origin')='imported-storyboard' AND coalesce(json_extract(u.data,'$.retired'),0)!=1").map_err(|e| e.to_string())?;
            let mut consumers = HashMap::<String, Vec<String>>::new();
            for row in query
                .query_map([], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                    ))
                })
                .map_err(|e| e.to_string())?
            {
                let (asset, doc) = row.map_err(|e| e.to_string())?;
                if let (Some(asset), Some(doc)) = (asset, doc) {
                    consumers.entry(asset).or_default().push(doc);
                }
            }
            self.consumers = Some(consumers);
        }
        Ok(self
            .consumers
            .as_ref()
            .and_then(|index| index.get(asset_id))
            .cloned()
            .unwrap_or_default())
    }
    fn document(&mut self, store: &Store, id: &str) -> Result<Value> {
        if !self.documents.contains_key(id) {
            self.documents
                .insert(id.to_owned(), lingmo_import::document(store, id)?);
        }
        Ok(self.documents[id].clone())
    }
    fn package(&mut self, store: &Store, id: &str) -> Result<Arc<Value>> {
        if let Some(package) = self.packages.get(id) {
            return Ok(Arc::clone(package));
        }
        if let Some(error) = self.package_errors.get(id) {
            return Err(error.clone());
        }
        let verified: Result<Value> = (|| {
            let package =
                lingmo_package::stored(store, id)?.ok_or("MODULE22_COMPLETE_PACKAGE_MISSING")?;
            check(
                package["proof"]["sourceFormat"] == FORMAT,
                "PACKAGE_SOURCE_FORMAT",
            )?;
            check(
                package["module22ImportReceipt"]["ok"] == true
                    && package["module22ImportReceipt"]["validator"] == "rust-module22/v1"
                    && package["module22ImportReceipt"]["queueHash"]
                        == package["queue"]["queueHash"]
                    && package["module22ImportReceipt"]["tasks"].as_u64()
                        == Some(rows(&package["queue"], "items")?.len() as u64),
                "FULL_QUEUE_RECEIPT",
            )?;
            Ok(package)
        })();
        let package = match verified {
            Ok(package) => package,
            Err(error) => {
                // One invalid package must not cause an expensive retry for
                // every sibling in the same command. Next command starts fresh.
                self.package_errors.insert(id.to_owned(), error.clone());
                return Err(error);
            }
        };
        let package = Arc::new(package);
        self.packages.insert(id.to_owned(), Arc::clone(&package));
        Ok(package)
    }
    #[cfg(test)]
    pub(crate) fn validated_package_count(&self) -> usize {
        self.packages.len()
    }
}

fn records(store: &Store, asset: &Value, context: &mut ImageBatchContext) -> Result<Vec<Value>> {
    check(context.root == store.root, "COMMAND_STORE_IDENTITY")?;
    if !s(asset, "id").starts_with("lingmo-")
        && asset["externalId"].is_null()
        && asset["handoffPackage"].is_null()
    {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let doc_ids = context.document_ids(store, s(asset, "id"))?;
    for id in doc_ids {
        let doc = context.document(store, &id)?;
        check(
            !doc["handoff"]["scopeRestriction"].is_string(),
            "RESTRICTED_SOURCE_REQUIRES_COMPLETE_PACKAGE",
        )?;
        if doc["handoff"]["proof"]["sourceFormat"] != FORMAT {
            continue;
        }
        let package = context.package(store, s(&doc, "id"))?;
        let projection = crate::lingmo_continuation::package_asset_projection_with_context(
            store,
            asset,
            &package,
            &mut context.identities,
        )?
        .ok_or("MODULE22_ASSET_IDENTITY_PROOF")?;
        let original = &projection["handoffRegistryAsset"];
        check(
            original["id"] == asset["externalId"]
                && original["assetRevision"] == asset["assetRevision"],
            "CURRENT_ASSET_REVISION",
        )?;
        // A locally adopted design may not silently reuse the old external queue.
        if asset["adoptedDesign"].is_object() {
            check(
                asset["adoptedDesign"]["full"] == original["adoptedDesign"]["full"]
                    && asset["adoptedDesign"]["designRevision"]
                        == original["adoptedDesign"]["designRevision"],
                "LOCAL_DESIGN_CHANGED_REQUIRES_NEW_QUEUE",
            )?;
        }
        let items: Vec<_> = rows(&package["queue"], "items")?
            .iter()
            .filter(|i| {
                i["assetId"] == asset["externalId"] && i["assetRevision"] == asset["assetRevision"]
            })
            .cloned()
            .collect();
        check(items.len() == 1, "ASSET_NOT_IN_EXPLICIT_QUEUE")?;
        let batch = batch(&package["queue"], &items);
        // load/stored has just validated every immutable queue item and its
        // current design. The batch above is an exact nonempty subset, never
        // caller-supplied fields; preserve the same per-task validator receipt.
        let mut receipt = package["module22ImportReceipt"].clone();
        receipt["tasks"] = json!(items.len());
        out.push(json!({"documentId":doc["id"],"packageFingerprint":package["proof"]["packageFingerprint"],"assetId":asset["id"],"externalAssetId":asset["externalId"],"externalTaskId":items[0]["taskId"],"batch":batch,"receipt":receipt}));
    }
    out.sort_by(|a, b| s(a, "documentId").cmp(s(b, "documentId")));
    Ok(out)
}
pub(crate) fn freeze_image_batch(store: &Store, asset: &Value, snapshot: &mut Value) -> Result<()> {
    freeze_image_batch_with_context(store, asset, snapshot, &mut ImageBatchContext::new(store))
}
pub(crate) fn freeze_image_batch_with_context(
    store: &Store,
    asset: &Value,
    snapshot: &mut Value,
    context: &mut ImageBatchContext,
) -> Result<()> {
    let items = records(store, asset, context)?;
    snapshot["module22DispatchContract"] = json!(CONTRACT);
    snapshot["module22Batches"] = json!(items);
    if items.is_empty() {
        return Ok(());
    }
    let request = s(snapshot, "requestId");
    check(
        !request.is_empty()
            && request.len() <= 128
            && request
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "BATCH_REQUEST_ID",
    )?;
    let dir = store.root.join("module22-batches");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    lingmo_package::root_path(&dir)?;
    let path = dir.join(format!("{request}.json"));
    let record = json!({"version":"desktop-module22-frozen-batches/v1","requestId":request,"batches":items,"generationSubmissions":0});
    let bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
    if path.exists() {
        check(
            fs::read(&path).map_err(|e| e.to_string())? == bytes,
            "BATCH_IMMUTABLE_CONFLICT",
        )?;
    } else {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    snapshot["module22BatchReceiptSha256"] = json!(crate::pipeline::digest(
        std::str::from_utf8(&bytes).map_err(|e| e.to_string())?
    ));
    Ok(())
}
pub(crate) fn validate_image_snapshot(store: &Store, snapshot: &Value) -> Result<()> {
    let Some(contract) = snapshot.get("module22DispatchContract") else {
        return Ok(());
    };
    check(contract == CONTRACT, "DISPATCH_CONTRACT")?;
    let asset = store.active_asset(s(snapshot, "assetId"))?;
    // This is an actual-send boundary, not the queue command: reload all files,
    // hashes and current designs even if the same command already validated it.
    let actual = records(store, &asset, &mut ImageBatchContext::new(store))?;
    check(
        snapshot["module22Batches"] == json!(actual),
        "DISPATCH_BASIS_CHANGED",
    )?;
    if actual.is_empty() {
        return Ok(());
    }
    let request = s(snapshot, "requestId");
    check(
        !request.is_empty()
            && request.len() <= 128
            && request
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "BATCH_REQUEST_ID",
    )?;
    let path = store
        .root
        .join("module22-batches")
        .join(format!("{request}.json"));
    let checked = lingmo_package::relative(
        &lingmo_package::root_path(&store.root)?,
        &format!("module22-batches/{request}.json"),
    )?;
    check(
        checked == path || checked.canonicalize().ok() == path.canonicalize().ok(),
        "BATCH_PATH",
    )?;
    let bytes = lingmo_package::read(&checked)?;
    check(
        snapshot["module22BatchReceiptSha256"]
            == crate::pipeline::digest(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?),
        "BATCH_RECEIPT_HASH",
    )?;
    let record: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    check(
        record["requestId"] == request && record["batches"] == json!(actual),
        "BATCH_RECEIPT_BINDING",
    )
}
