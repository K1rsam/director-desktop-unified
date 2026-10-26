//! Bounded, exact discovery. Version tags and content hashes resolve files;
//! names, timestamps and a guessed "latest" never select a production version.
use crate::{
    lingmo_package,
    module22::{check, rows},
    module22_json as js,
    module22_package::MAP_VERSION,
    pipeline::digest,
    store::{s, Result},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
struct Candidate {
    path: String,
    bytes: Vec<u8>,
    sha: String,
    value: Value,
}
fn unique<'a>(
    candidates: impl Iterator<Item = &'a Candidate>,
    label: &str,
) -> Result<&'a Candidate> {
    let mut found: Vec<_> = candidates.collect();
    found.sort_by(|a, b| a.path.len().cmp(&b.path.len()).then(a.path.cmp(&b.path)));
    let hashes: HashSet<_> = found.iter().map(|c| c.sha.as_str()).collect();
    check(
        hashes.len() == 1,
        &format!(
            "DISCOVER_{label}:需要唯一同版文件，当前{}个版本",
            hashes.len()
        ),
    )?;
    Ok(found[0])
}
fn safe_relative(root: &Path, path: &Path) -> Result<String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| "MODULE22_SELECTED_FILE_OUTSIDE_PACKAGE")?
        .to_str()
        .ok_or("MODULE22_PATH_UTF8")?
        .replace('\\', "/");
    lingmo_package::relative(root, &relative)?;
    Ok(relative)
}
pub(crate) fn mapping(
    root: &Path,
    markdown: &Path,
    catalog: &Value,
    direction: &str,
) -> Result<Value> {
    let root = lingmo_package::root_path(root)?;
    let md = safe_relative(&root, markdown)?;
    let mut pending = vec![(root.clone(), 0usize)];
    let mut paths = Vec::<PathBuf>::new();
    while let Some((dir, depth)) = pending.pop() {
        check(depth <= 8, "DISCOVER_DEPTH_LIMIT")?;
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    continue;
                }
            }
            if meta.is_dir() {
                pending.push((path, depth + 1));
                continue;
            }
            let extension = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ["json", "md", "txt", "docx"].contains(&extension.as_str())
                && meta.len() <= 64 * 1024 * 1024
            {
                paths.push(path);
                check(paths.len() <= 4096, "DISCOVER_FILE_LIMIT")?;
            }
        }
    }
    paths.sort();
    let mut all = Vec::new();
    let mut total = 0u64;
    for path in paths {
        let bytes = lingmo_package::read(&path)?;
        total += bytes.len() as u64;
        check(total <= lingmo_package::MAX_TOTAL, "DISCOVER_TOTAL_LIMIT")?;
        let value = if path.extension().and_then(|s| s.to_str()) == Some("json") {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        all.push(Candidate {
            path: safe_relative(&root, &path)?,
            sha: format!("{:x}", Sha256::digest(&bytes)),
            bytes,
            value,
        });
    }
    let original_md = all
        .iter()
        .find(|c| c.path == md)
        .ok_or("MODULE22_MD_NOT_IN_PACKAGE")?;
    let cat = unique(
        all.iter().filter(|c| {
            c.value["version"] == "lingmo-v7-assets-v1"
                && c.value["markdownSha256"] == original_md.sha
                && (catalog.is_null() || c.value == *catalog)
        }),
        "CATALOG",
    )?;
    let bundle = unique(
        all.iter().filter(|c| {
            c.sha == cat.value["bundleSha256"]
                && c.value["version"] == "series-storyboard-bundle-v1"
        }),
        "BUNDLE",
    )?;
    let catalog_ids: HashSet<_> = rows(&cat.value, "assets")?
        .iter()
        .map(|a| (s(a, "id"), s(a, "filename")))
        .collect();
    let queue = unique(
        all.iter().filter(|c| {
            c.value["version"] == "series-image-queue-v1"
                && c.value["assetRegistry"]["id"] == bundle.value["assetRegistry"]["id"]
                && c.value["items"].as_array().is_some_and(|items| {
                    items.len() == catalog_ids.len()
                        && items
                            .iter()
                            .all(|a| catalog_ids.contains(&(s(a, "assetId"), s(a, "filename"))))
                })
        }),
        "IMAGE_QUEUE",
    )?;
    let head = unique(
        all.iter().filter(|c| {
            c.path.ends_with("/current.json")
                && c.value["version"] == "series-assets-head-v1"
                && c.value["id"] == queue.value["assetRegistry"]["id"]
        }),
        "REGISTRY_HEAD",
    )?;
    let registry_dir = head.path.strip_suffix("/current.json").unwrap();
    let initial = all
        .iter()
        .find(|c| c.path == format!("{registry_dir}/v000001.json"))
        .ok_or("MODULE22_DISCOVER_INITIAL_REGISTRY")?;
    let source = unique(
        all.iter().filter(|c| c.sha == queue.value["sourceHash"]),
        "SOURCE_ORIGINAL",
    )?;
    let lock = unique(
        all.iter().filter(|c| {
            c.value["version"] == "series-source-lock-v1"
                && js::stringify_raw(&c.bytes)
                    .is_ok_and(|raw| digest(&raw) == bundle.value["sourceLockHash"])
        }),
        "SOURCE_LOCK",
    )?;
    let receipt = unique(
        all.iter().filter(|c| {
            c.value["version"] == "series-asset-consumption-v1"
                && c.value["bundleSha256"] == bundle.sha
                && c.value["id"] == head.value["id"]
        }),
        "CONSUMPTION_RECEIPT",
    )?;
    let receipt_registry = unique(
        all.iter().filter(|c| {
            c.sha == receipt.value["registrySha256"] && c.value["version"] == "series-assets-v1"
        }),
        "CONSUMPTION_REGISTRY",
    )?;
    check(
        !direction.is_empty(),
        "DISCOVER_DIRECTION:原生接续包没有统一方向文件名，请选择包内已采用的制作方向文件",
    )?;
    let direction_path = if Path::new(direction).is_absolute() {
        PathBuf::from(direction)
    } else {
        root.join(direction)
    };
    let direction_relative = safe_relative(&root, &direction_path)?;
    let direction_bytes = lingmo_package::read(&direction_path)?;
    let styles: HashSet<_> = rows(&bundle.value, "episodes")?
        .iter()
        .map(|e| s(&e["settings"], "visualStyle"))
        .collect();
    check(
        styles.len() == 1 && !styles.contains(""),
        "DISCOVER_DIRECTION_STYLE",
    )?;
    let mut relocations = serde_json::Map::new();
    relocations.insert(
        s(&receipt.value, "registrySnapshot").to_owned(),
        json!(receipt_registry.path),
    );
    Ok(
        json!({"version":MAP_VERSION,"discovery":"exact-version-and-content-hash/v1","sourceOriginal":source.path,"registryDirectory":registry_dir,"initialRegistry":initial.path,"imageQueue":queue.path,"bundle":bundle.path,"sourceLock":lock.path,"markdown":md,"catalog":cat.path,"consumptionReceipt":receipt.path,"consumptionRegistry":receipt_registry.path,"direction":direction_relative,"directionVersion":format!("sha256:{:x}",Sha256::digest(direction_bytes)),"visualStyle":styles.into_iter().next().unwrap(),"relocations":relocations}),
    )
}
