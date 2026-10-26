//! Exact, versioned external identities survive MD byte changes. No name matching.
use crate::{
    pipeline::digest,
    store::{parse, s, Result},
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MAP_PREFIX: &str = "lingmo.asset-identities:";

fn namespace(catalog: &Value) -> Option<String> {
    for key in ["assetRegistryId", "sourceSha256"] {
        if let Some(value) = catalog[key].as_str().filter(|v| !v.is_empty()) {
            return Some(format!("{key}:{value}"));
        }
    }
    None
}

fn package_namespace(catalog: &Value, proof: &Value) -> Option<String> {
    if proof["sourceFormat"] == crate::module22::FORMAT {
        // Native identity comes from the verified immutable registry, not an
        // optional catalog extension. Keep all catalog bytes as evidence.
        return (proof["catalogSha256"] == digest(&catalog.to_string())
            && !s(proof, "registryId").is_empty())
        .then(|| format!("assetRegistryId:{}", s(proof, "registryId")));
    }
    namespace(catalog)
}

fn revision(asset: &Value) -> Value {
    asset.get("assetRevision").cloned().unwrap_or(json!(1))
}

fn catalog_asset<'a>(catalog: &'a Value, asset: &Value) -> Result<&'a Value> {
    let matches: Vec<_> = catalog["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["id"] == asset["externalId"] && revision(a) == revision(asset))
        .collect();
    if matches.len() != 1 {
        return Err("接续目录资产身份或版本不唯一；未猜测继承".into());
    }
    Ok(matches[0])
}

fn metadata(tx: &Transaction<'_>, key: &str) -> Result<Option<Value>> {
    let raw: Option<String> = tx
        .query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(parse).transpose()
}

struct OriginalEvidence {
    catalog: Value,
    assets: Vec<Value>,
}

/// A single queue command may share parsed immutable source evidence. Registry
/// namespaces remain separate even when two packages have identical MD bytes.
#[derive(Default)]
pub(crate) struct PackageIdentityContext {
    registries: HashMap<String, HashMap<String, OriginalEvidence>>,
}

/// The old importer did not persist handoffRegistry. Its immutable original and
/// the parser's exact local ID/variant must jointly prove the catalog identity.
/// Editable display names, filenames and handoff fields are never that proof.
fn original_handoff(
    tx: &Transaction<'_>,
    old: &Value,
    registry: &str,
    originals: &mut HashMap<String, OriginalEvidence>,
) -> Result<Option<Value>> {
    let document = s(old, "id")
        .strip_prefix("lingmo-")
        .and_then(|id| id.split_once("-asset-").map(|(sha, _)| sha))
        .filter(|sha| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()));
    let Some(document) = document else {
        return Ok(None);
    };
    if !originals.contains_key(document) {
        let Some(original) = metadata(tx, &format!("lingmo.original:{document}"))? else {
            return Ok(None);
        };
        let pointer=metadata(tx,&format!("lingmo.package:{document}"))?.unwrap_or(Value::Null);
        if package_namespace(&original["catalog"],&pointer["proof"]).as_deref() != Some(registry) {
            return Ok(None);
        }
        if original["sha256"] != document
            || digest(s(&original, "text")) != document
            || original["catalog"]["markdownSha256"] != document
        {
            return Err("已有资产的不可变原件或同版目录摘要不符，未猜测接续".into());
        }
        let parsed = crate::lingmo_parser::parse(s(&original, "text"), Some(&original["catalog"]))?;
        if parsed["catalogMatched"] != true {
            return Err("已有资产的原件目录不能精确核验，未猜测接续".into());
        }
        originals.insert(
            document.to_owned(),
            OriginalEvidence {
                catalog: original["catalog"].clone(),
                assets: parsed["assets"].as_array().cloned().unwrap_or_default(),
            },
        );
    }
    let evidence = &originals[document];
    let matches: Vec<_> = evidence
        .assets
        .iter()
        .filter(|a| {
            a["id"] == old["id"]
                && a["variantId"] == old["variantId"]
                && a["externalId"] == old["externalId"]
                && revision(a) == revision(old)
        })
        .collect();
    if matches.len() != 1 {
        return Err("已有资产身份或形象版本与不可变目录不符，未猜测接续".into());
    }
    let entry = catalog_asset(&evidence.catalog, matches[0])?;
    if old["handoffRegistry"]
        .as_str()
        .is_some_and(|r| r != registry)
        || (old["handoffAsset"].is_object() && old["handoffAsset"] != *entry)
    {
        return Err("已有资产的接续标记与不可变目录冲突，未覆盖历史结果".into());
    }
    Ok(Some(entry.clone()))
}

fn replace_scalar(value: &mut Value, map: &HashMap<String, Value>) {
    match value {
        Value::String(id) => {
            if let Some(replacement) = map.get(id) {
                *value = replacement.clone();
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_scalar(item, map);
            }
        }
        _ => {}
    }
}

/// Traverse wrappers as well as direct refs, but replace only identity fields.
/// Prose, exact tokens, filenames, source positions and immutable evidence stay
/// byte-for-byte unchanged even when a string happens to equal an old ID.
fn replace_ids(
    value: &mut Value,
    assets: &HashMap<String, Value>,
    variants: &HashMap<String, Value>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                replace_ids(item, assets, variants);
            }
        }
        Value::Object(fields) => {
            for (key, field) in fields {
                if ["importSource", "sourceRefs", "handoffAsset", "raw"].contains(&key.as_str()) {
                    continue;
                }
                if [
                    "assetId",
                    "assetIds",
                    "targetAssetId",
                    "parentAssetId",
                    "baseAssetId",
                    "assetRefs",
                    "audioAssetRefs",
                    "imageBindings",
                    "resolvedAssets",
                ]
                .contains(&key.as_str())
                {
                    replace_scalar(field, assets);
                }
                if [
                    "variantId",
                    "variantIds",
                    "targetVariantId",
                    "parentVariantId",
                ]
                .contains(&key.as_str())
                {
                    replace_scalar(field, variants);
                }
                replace_ids(field, assets, variants);
            }
        }
        _ => {}
    }
}

fn map_sections(
    parsed: &mut Value,
    assets: &HashMap<String, Value>,
    variants: &HashMap<String, Value>,
) {
    for section in ["scenes", "units"] {
        for item in parsed[section].as_array_mut().into_iter().flatten() {
            replace_ids(item, assets, variants);
        }
    }
}

fn resolve<'a>(
    identities: &'a [Value],
    external: &Value,
    asset_revision: Option<&Value>,
    design_revision: Option<&Value>,
) -> Result<Option<&'a Value>> {
    if external.is_null() || external.as_str() == Some("") {
        return Ok(None);
    }
    let found: Vec<_> = identities
        .iter()
        .filter(|a| {
            a["externalId"] == *external && asset_revision.is_none_or(|r| r == &revision(a))
        })
        .collect();
    if found.len() != 1 {
        return Err(format!("接续依赖资产身份或版本无法唯一匹配：{external}"));
    }
    let asset = found[0];
    if let Some(expected) = design_revision {
        // Design and asset revisions are independent domains. Never compare a
        // design revision with assetRevision or with the editable row version.
        let actual = asset["adoptedDesign"]
            .get("designRevision")
            .or_else(|| asset.get("designRevision"))
            .or_else(|| asset["handoffAsset"]["adoptedDesign"].get("designRevision"));
        if actual != Some(expected) {
            return Err(format!("接续依赖设计版本不符：{external}，需要 {expected}"));
        }
    }
    Ok(Some(asset))
}

pub(crate) fn continue_assets(
    tx: &Transaction<'_>,
    parsed: &mut Value,
    catalog: &Value,
) -> Result<HashSet<String>> {
    let mut preserved = HashSet::new();
    if parsed["catalogMatched"] != true {
        return Ok(preserved);
    }
    let mut q = tx
        .prepare("SELECT data FROM assets")
        .map_err(|e| e.to_string())?;
    let current = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .map(|r| parse(r.map_err(|e| e.to_string())?))
        .collect::<Result<Vec<_>>>()?;
    drop(q);
    let ns = package_namespace(catalog,&parsed["handoff"]["proof"]);
    let mut originals = HashMap::new();
    let mut mapping = HashMap::new();
    let mut variants = HashMap::new();
    let mut bindings = Vec::new();
    let mut seen = HashSet::new();
    for asset in parsed["assets"].as_array_mut().ok_or("导入资产数组缺失")? {
        let source_id = s(asset, "id").to_owned();
        let source_variant = s(asset, "variantId").to_owned();
        if !seen.insert(source_id.clone()) {
            return Err("同一目录包含重复本地资产身份，未覆盖已有版本".into());
        }
        if s(asset, "externalId").is_empty() {
            continue;
        }
        let handoff = catalog_asset(catalog, asset)?.clone();
        asset["handoffAsset"] = handoff.clone();
        asset["handoffRegistry"] = json!(ns);
        if let Some(registry) = &ns {
            let mut existing = Vec::new();
            for old in current.iter().filter(|old| {
                old["externalId"] == asset["externalId"] && revision(old) == revision(asset)
            }) {
                if let Some(original) = original_handoff(tx, old, registry, &mut originals)? {
                    if original != handoff {
                        return Err("相同资产版本的目录内容已变化，请提高明确资产版本；原图片和手工编辑保留".into());
                    }
                    existing.push(old);
                }
            }
            if existing.len() > 1 {
                return Err("已有接续身份重复，未猜测覆盖".into());
            }
            if let Some(old) = existing.first() {
                mapping.insert(source_id.clone(), old["id"].clone());
                variants.insert(source_variant.clone(), old["variantId"].clone());
                preserved.insert(s(old, "id").to_owned());
                // No save/version bump: even fields edited after the old import
                // remain intact. The mapping receipt supplies immutable proof.
                *asset = (*old).clone();
                // This transient projection is used only for this MD package's
                // image backfill; the preserved asset row is never saved.
                asset["filename"] = handoff["filename"].clone();
            }
        }
        bindings.push(
            json!({"sourceAssetId":source_id,"sourceVariantId":source_variant,
            "assetId":asset["id"],"variantId":asset["variantId"],
            "externalId":asset["externalId"],"assetRevision":revision(asset),
            "catalogEntrySha256":digest(&handoff.to_string())}),
        );
    }
    let identities = parsed["assets"].as_array().unwrap().clone();
    let registry_namespace = ns
        .clone()
        .unwrap_or_else(|| s(parsed, "markdownSha256").to_owned());
    for asset in parsed["assets"].as_array_mut().unwrap() {
        if preserved.contains(s(asset, "id")) {
            continue;
        }
        let handoff = asset["handoffAsset"].clone();
        if handoff.is_null() {
            continue;
        }
        let mut issues = Vec::new();
        for key in ["importName", "purpose", "imageRequired", "imageTask"] {
            if let Some(v) = handoff.get(key) {
                asset[key] = v.clone();
            }
        }
        if let Some(identity) = handoff["identityId"].as_str().filter(|v| !v.is_empty()) {
            asset["identityId"] = json!(format!(
                "imported-{}",
                digest(&format!("{registry_namespace}:{identity}"))
            ));
        }
        for key in ["baseAssetId", "parentAssetId"] {
            if let Some(v) = handoff.get(key) {
                match resolve(&identities, v, None, None) {
                    Ok(target) => {
                        asset[key] = target.map(|a| a["id"].clone()).unwrap_or(Value::Null)
                    }
                    Err(e) => issues.push(e),
                }
            }
        }
        let mut design = handoff["adoptedDesign"].clone();
        if design.is_object() {
            design["assetId"] = asset["id"].clone();
            design["variantId"] = asset["variantId"].clone();
            asset["designRevision"] = design["designRevision"].clone();
        }
        for key in ["designDependencies", "referenceRequirements"] {
            let original = handoff.get(key).or_else(|| design.get(key));
            if let Some(values) = original.and_then(Value::as_array) {
                let mut mapped = Vec::new();
                for original in values {
                    let mut item = original.clone();
                    let is_design = key == "designDependencies";
                    let field = if is_design {
                        "assetId"
                    } else {
                        "targetAssetId"
                    };
                    let asset_revision = item.get(if is_design {
                        "assetRevision"
                    } else {
                        "targetAssetRevision"
                    });
                    let design_revision = if is_design {
                        item.get("designRevision")
                    } else {
                        None
                    };
                    match resolve(&identities, &item[field], asset_revision, design_revision) {
                        Ok(Some(target)) => {
                            item[field] = target["id"].clone();
                            if !is_design {
                                item["targetVariantId"] = target["variantId"].clone();
                            } else if item.get("variantId").is_some() {
                                item["variantId"] = target["variantId"].clone();
                            }
                        }
                        Ok(None) => {
                            item[field] = Value::Null;
                        }
                        Err(e) => issues.push(e),
                    }
                    mapped.push(item);
                }
                asset[key] = json!(mapped);
                if design.is_object() && (key == "designDependencies" || design.get(key).is_some())
                {
                    design[key] = asset[key].clone();
                }
            }
        }
        if design.is_object() {
            asset["adoptedDesign"] = design;
        }
        asset["handoffValidation"] = json!({"kind":"matched-md-catalog-exact-identity","issues":issues,
            "originalPreserved":"handoffAsset","completePackageVerified":false});
        if !issues.is_empty() {
            asset["promptStatus"] = json!("needs_review");
        }
    }
    map_sections(parsed, &mapping, &variants);
    // Frozen per-document aliases also make duplicate-import image backfill
    // target the retained identity, instead of the new MD hash's parser IDs.
    tx.execute(
        "INSERT INTO metadata(key,value) VALUES(?1,?2)",
        params![format!("{MAP_PREFIX}{}", s(parsed, "markdownSha256")),
            json!({"schemaVersion":"lingmo-asset-identities/v1","documentSha256":parsed["markdownSha256"],
                "catalogSha256":digest(&catalog.to_string()),"registry":ns,"bindings":bindings}).to_string()],
    ).map_err(|e| e.to_string())?;
    if !preserved.is_empty() {
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES('','lingmo_asset_identity_continued',?1,?2)",
            params![json!({"documentSha256":parsed["markdownSha256"],"assetIds":preserved,"registry":ns}).to_string(),crate::store::now()])
            .map_err(|e|e.to_string())?;
    }
    Ok(preserved)
}

/// Duplicate imports are read-only for text/assets. Use the original alias
/// receipt for optional new media; never rerun continuation or recreate rows.
pub(crate) fn restore_assets(
    tx: &Transaction<'_>,
    parsed: &mut Value,
    catalog: &Value,
) -> Result<()> {
    if parsed["catalogMatched"] != true {
        return Ok(());
    }
    let Some(receipt) = metadata(tx, &format!("{MAP_PREFIX}{}", s(parsed, "markdownSha256")))?
    else {
        // Imports predating continuation used exactly the parser identities.
        return Ok(());
    };
    if receipt["schemaVersion"] != "lingmo-asset-identities/v1"
        || receipt["documentSha256"] != parsed["markdownSha256"]
        || receipt["catalogSha256"] != digest(&catalog.to_string())
    {
        return Err("导入资产接续回执与同版目录不符，未猜测补图".into());
    }
    let mut mapping = HashMap::new();
    let mut variants = HashMap::new();
    for asset in parsed["assets"].as_array_mut().ok_or("导入资产数组缺失")? {
        if s(asset, "externalId").is_empty() {
            continue;
        }
        let entry_sha = digest(&catalog_asset(catalog, asset)?.to_string());
        let matches: Vec<_> = receipt["bindings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|binding| {
                binding["sourceAssetId"] == asset["id"]
                    && binding["sourceVariantId"] == asset["variantId"]
                    && binding["externalId"] == asset["externalId"]
                    && binding["assetRevision"] == revision(asset)
                    && binding["catalogEntrySha256"] == entry_sha
            })
            .collect();
        if matches.len() != 1 {
            return Err("导入资产接续回执缺失或重复，未猜测补图".into());
        }
        let binding = matches[0];
        let raw: Option<String> = tx
            .query_row(
                "SELECT data FROM assets WHERE id=?1",
                [s(binding, "assetId")],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let current = raw
            .map(parse)
            .transpose()?
            .ok_or("接续资产已缺失，未重建或覆盖原身份")?;
        if current["variantId"] != binding["variantId"]
            || current["externalId"] != binding["externalId"]
            || revision(&current) != binding["assetRevision"]
        {
            return Err("接续资产身份或形象版本已变化，未猜测补图".into());
        }
        mapping.insert(s(asset, "id").to_owned(), binding["assetId"].clone());
        variants.insert(
            s(asset, "variantId").to_owned(),
            binding["variantId"].clone(),
        );
        // Keep exact catalog filenames; a manual asset label is not a filename
        // matching rule for the original delivery package.
        asset["id"] = binding["assetId"].clone();
        asset["variantId"] = binding["variantId"].clone();
    }
    map_sections(parsed, &mapping, &variants);
    Ok(())
}

/// Add package proof only to newly created cards. The full registry stays in
/// immutable package storage; its extra identities never become image tasks.
pub(crate) fn attach_package(
    tx: &Transaction<'_>,
    parsed: &mut Value,
    catalog: &Value,
    package: &Value,
    preserved: &HashSet<String>,
) -> Result<()> {
    let registry = package["registry"]["assets"]
        .as_array()
        .ok_or("完整包总库缺失")?;
    let mut current = Vec::new();
    let mut query = tx
        .prepare("SELECT data FROM assets")
        .map_err(|e| e.to_string())?;
    for raw in query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
    {
        current.push(parse(raw.map_err(|e| e.to_string())?)?);
    }
    drop(query);
    let ns = package_namespace(catalog,&package["proof"]).ok_or("完整包稳定身份命名空间缺失")?;
    let mut originals = HashMap::new();
    let local = parsed["assets"].as_array().ok_or("导入资产缺失")?.clone();
    for asset in parsed["assets"].as_array_mut().unwrap() {
        if preserved.contains(s(asset, "id")) {
            continue;
        }
        let original = registry
            .iter()
            .find(|a| a["id"] == asset["externalId"])
            .ok_or("图片资产不属于完整总库")?;
        asset["handoffRegistryAsset"] = original.clone();
        if package["proof"]["sourceFormat"]==crate::module22::FORMAT {
            let mut design=original["adoptedDesign"].clone();
            design["assetId"]=asset["id"].clone();design["variantId"]=asset["variantId"].clone();
            asset["adoptedDesign"]=design;
            asset["designRevision"]=original["revision"].clone();
            asset["sourceFacts"]=original["sourceFacts"].clone();
            asset["handoffRegistry"]=json!(ns);
        }
        asset["handoffPackage"] = package["proof"].clone();
        asset["handoffValidation"]["completePackageVerified"] = json!(true);
        asset["handoffValidation"]["packageProof"] = package["proof"].clone();
        asset["handoffValidation"]["kind"] = json!(if package["proof"]["sourceFormat"]==crate::module22::FORMAT {"upstream-module22-exact-identity"}else{"desktop-complete-package-exact-identity"});
        let mut issues = Vec::new();
        for (key, saved_key, field, variant_field) in [
            (
                "designDependencies",
                "packageDesignDependencies",
                "assetId",
                "variantId",
            ),
            (
                "referenceRequirements",
                "packageReferenceRequirements",
                "targetAssetId",
                "targetVariantId",
            ),
        ] {
            let refs = original
                .get(key)
                .or_else(|| original["adoptedDesign"].get(key));
            let Some(refs) = refs.and_then(Value::as_array) else {
                continue;
            };
            asset[saved_key] = json!(refs);
            let mut mapped = Vec::new();
            for reference in refs {
                let mut item = reference.clone();
                let external = &reference[field];
                let mut found: Vec<Value> = local
                    .iter()
                    .filter(|a| a["externalId"] == *external)
                    .cloned()
                    .collect();
                if found.is_empty() {
                    for candidate in current.iter().filter(|a| a["externalId"] == *external) {
                        if let Some(entry) = original_handoff(tx, candidate, &ns, &mut originals)? {
                            let stable = registry.iter().find(|a| a["id"] == *external);
                            if stable.is_some_and(|r| {
                                r["variantId"] == entry["variantId"]
                                    && r["adoptedDesign"] == entry["adoptedDesign"]
                            }) {
                                found.push(candidate.clone());
                            }
                        }
                    }
                }
                let expected_revision = reference.get(if key == "designDependencies" {
                    "assetRevision"
                } else {
                    "targetAssetRevision"
                });
                found.retain(|a| {
                    expected_revision.is_none_or(|r| revision(a) == *r)
                        && reference
                            .get(variant_field)
                            .is_none_or(|v| a["handoffAsset"]["variantId"] == *v)
                        && reference
                            .get("designRevision")
                            .is_none_or(|r| a["adoptedDesign"]["designRevision"] == *r)
                });
                if found.len() == 1 {
                    item[field] = found[0]["id"].clone();
                    item[variant_field] = found[0]["variantId"].clone();
                } else {
                    // This explicit registry binding can supply text, but not
                    // substitute a missing real image or broaden queue scope.
                    let matches: Vec<_> =
                        registry.iter().filter(|a| a["id"] == *external).collect();
                    if matches.len() != 1 {
                        issues.push(format!("接续包依赖未找到唯一稳定资产：{external}"));
                    } else {
                        issues.push(format!(
                            "范围外{}依赖 {external} 保留在完整总库，尚未绑定本地{}",
                            if key == "designDependencies" {
                                "文字设计"
                            } else {
                                "媒体"
                            },
                            if key == "designDependencies" {
                                "设计"
                            } else {
                                "采用图"
                            }
                        ));
                    }
                    item["externalPackageBinding"] = json!({"registrySha256":package["proof"]["registrySha256"],"assetId":external,"sourceSha256":package["proof"]["sourceSha256"]});
                }
                mapped.push(item);
            }
            asset[key] = json!(mapped);
            if key == "designDependencies" && asset["adoptedDesign"].is_object() {
                asset["adoptedDesign"][key] = asset[key].clone();
            }
        }
        // Existing exact catalog-only mapping diagnostics remain authoritative;
        // appending package diagnostics cannot silently mark them reviewed.
        let mut combined = asset["handoffValidation"]["issues"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for issue in issues {
            if !combined.contains(&json!(issue)) {
                combined.push(json!(issue));
            }
        }
        asset["handoffValidation"]["issues"] = json!(combined);
    }
    Ok(())
}

/// Upgrade only the current input projection after an exact catalog-only import
/// later receives its full package. Immutable originals prove identity; edited
/// labels or handoff fields cannot authorize inheritance from another package.
pub(crate) fn package_asset_projection(
    store: &crate::store::Store,
    asset: &Value,
    package: &Value,
) -> Result<Option<Value>> {
    package_asset_projection_with_context(store, asset, package, &mut PackageIdentityContext::default())
}

pub(crate) fn package_asset_projection_with_context(
    store: &crate::store::Store,
    asset: &Value,
    package: &Value,
    context: &mut PackageIdentityContext,
) -> Result<Option<Value>> {
    let catalog = &package["catalog"];
    let Some(registry) = package_namespace(catalog,&package["proof"]) else {
        return Ok(None);
    };
    let matches: Vec<_> = catalog["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["id"] == asset["externalId"] && revision(a) == revision(asset))
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() != 1 {
        return Err("完整包补充资产身份或版本不唯一".into());
    }
    let mut connection = store.conn()?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    let originals = context.registries.entry(registry.clone()).or_default();
    let Some(original) = original_handoff(&tx, asset, &registry, originals)? else {
        return Ok(None);
    };
    if original != *matches[0] {
        return Err("补充完整包与原不可变资产目录不同版；未改写已有资产".into());
    }
    let originals: Vec<_> = package["registry"]["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["id"] == asset["externalId"] && (package["proof"]["sourceFormat"]==crate::module22::FORMAT || a["variantId"] == matches[0]["variantId"]))
        .collect();
    if originals.len() != 1 {
        return Err("完整包总库身份不唯一，未猜测接续".into());
    }
    let mut projection=json!({"handoffPackage":package["proof"],"handoffRegistryAsset":originals[0]});
    if package["proof"]["sourceFormat"]==crate::module22::FORMAT && !asset["adoptedDesign"].is_object(){
        let mut design=originals[0]["adoptedDesign"].clone();design["assetId"]=asset["id"].clone();design["variantId"]=asset["variantId"].clone();
        projection["adoptedDesign"]=design;projection["designRevision"]=originals[0]["revision"].clone();
    }
    Ok(Some(projection))
}

#[cfg(test)]
#[path = "lingmo_continuation_tests.rs"]
mod tests;
