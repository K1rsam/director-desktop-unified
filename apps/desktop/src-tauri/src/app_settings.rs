//! Application-owned channel preferences. Project DBs and frozen requests are
//! deliberately not rewritten when these preferences change.
use crate::store::{atomic_json, id, s, Result};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Read, path::Path};

pub const FILE: &str = "app-settings.json";
pub const SCOPE: &str = "_globalExecution";
pub const FIELDS: &[&str] = &[
    "textProvider",
    "textModel",
    "reasoning",
    "textEndpoint",
    "codexPath",
    "textSdkNodePath",
    "dreaminaPath",
    "dreaminaCanvasPath",
    "bhEndpoint",
    "bhAccountId",
    "chromeEndpoint",
    "chromeAccountId",
    "chromeProfileId",
    "chromeProfileEndpoint",
    "chromeProfileBundleId",
    "providerTimeoutSeconds",
    "textConcurrency",
    "episodeConcurrency",
    "videoConcurrency",
    "chromeConcurrency",
];
pub const SECRETS: &[(&str, &str)] = &[
    ("bhToken", "bh"),
    ("chromeToken", "chrome"),
    ("textApiKey", "text"),
];
pub fn defaults() -> Value {
    json!({"textProvider":"codex","textModel":"gpt-6.1-sol","reasoning":"medium","bhEndpoint":"https://api.7tai.cc/v1","textConcurrency":4,"episodeConcurrency":2,"videoConcurrency":1,"chromeConcurrency":1})
}
fn lock(home: &Path) -> Result<fs::File> {
    fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.join(".app-settings.lock"))
        .map_err(|e| e.to_string())?;
    file.try_lock()
        .map_err(|_| "软件设置正在保存，请稍后重试。")?;
    Ok(file)
}
pub fn load(home: &Path) -> Result<Option<Value>> {
    let path = home.join(FILE);
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if file.metadata().map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("软件设置文件过大，未采用。".into());
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("软件设置文件过大，未采用。".into());
    }
    let doc: Value =
        serde_json::from_slice(&bytes).map_err(|_| "软件设置文件损坏，已保留原文件。")?;
    if doc["schemaVersion"] != "app-settings/v1"
        || !doc["settings"].is_object()
        || doc["revision"].as_u64().is_none()
        || !doc["credentialRefs"].is_object()
    {
        return Err("软件设置格式不受支持，已保留原文件。".into());
    }
    if doc["settings"]
        .as_object()
        .unwrap()
        .keys()
        .any(|k| !FIELDS.contains(&k.as_str()))
    {
        return Err("软件设置包含不支持的字段。".into());
    }
    Ok(Some(doc))
}
fn legacy_row(row: &Value) -> Result<Value> {
    let root = Path::new(s(row, "rootPath"));
    if !root.is_absolute() {
        return Err("旧项目目录不是绝对路径，未迁移。".into());
    }
    let db = rusqlite::Connection::open_with_flags(
        root.join("project.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    let identity: String = db
        .query_row("SELECT id FROM project LIMIT 1", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if identity != s(row, "id") {
        return Err("旧项目身份与目录记录不一致，未迁移。".into());
    }
    let mut q = db
        .prepare("SELECT key,value FROM settings")
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut settings = Map::new();
    for row in rows {
        let (key, value) = row.map_err(|e| e.to_string())?;
        if FIELDS.contains(&key.as_str()) {
            settings.insert(
                key,
                serde_json::from_str(&value).map_err(|e| e.to_string())?,
            );
        }
    }
    Ok(json!({"id":row["id"],"settings":settings}))
}
pub fn initialize(home: &Path, registry: &Value, selected: Option<&str>) -> Result<Value> {
    if let Some(doc) = load(home)? {
        return Ok(doc);
    }
    let _lock = lock(home)?;
    if let Some(doc) = load(home)? {
        return Ok(doc);
    }
    let mut settings = defaults();
    let mut conflicts = Vec::<String>::new();
    let mut refs = json!({});
    let mut sources = Vec::<String>::new();
    let rows: Vec<_> = registry
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["archivedAt"].is_null())
        .collect();
    let selected = selected.filter(|s| !s.is_empty());
    let mut skipped = Vec::<Value>::new();
    let candidates: Vec<_> = if let Some(key) = selected {
        match rows.iter().find(|p| s(p, "id") == key) {
            Some(row) => match legacy_row(row) {
                Ok(value) => vec![value],
                Err(_) => {
                    skipped.push(json!({"projectId":key,"reason":"unreadable"}));
                    vec![]
                }
            },
            None => {
                skipped.push(json!({"projectId":key,"reason":"missing_or_archived"}));
                vec![]
            }
        }
    } else {
        rows.iter()
            .filter_map(|p| match legacy_row(p) {
                Ok(value) => Some(value),
                Err(_) => {
                    skipped.push(json!({"projectId":p["id"],"reason":"unreadable"}));
                    None
                }
            })
            .collect()
    };
    for field in FIELDS {
        let values: Vec<_> = candidates
            .iter()
            .filter_map(|c| {
                c["settings"]
                    .get(*field)
                    .filter(|v| !v.is_null() && v.as_str() != Some(""))
                    .map(|v| (c, v))
            })
            .collect();
        let distinct: BTreeSet<_> = values.iter().map(|(_, v)| v.to_string()).collect();
        if distinct.len() == 1 {
            let (owner, value) = values[0];
            settings[*field] = value.clone();
            sources.push(s(owner, "id").into());
        } else if distinct.len() > 1 {
            conflicts.push((*field).into());
        }
    }
    // Preserve a complete endpoint/account/credential relationship. Never pick
    // a key merely because one project's unrelated model or path is newer.
    for (provider, keys) in [
        ("bh", vec!["bhEndpoint", "bhAccountId"]),
        ("chrome", vec!["chromeEndpoint", "chromeAccountId"]),
        ("text", vec!["textEndpoint"]),
    ] {
        let owners: Vec<_> = candidates
            .iter()
            .filter(|c| {
                keys.iter().all(|key| {
                    c["settings"]
                        .get(*key)
                        .is_some_and(|v| v == &settings[*key] && v.as_str() != Some(""))
                })
            })
            .collect();
        if owners.len() == 1 {
            refs[provider] = json!(format!("{}:{provider}", s(owners[0], "id")));
        } else if owners.len() > 1 {
            conflicts.push(format!("{provider}CredentialSource"));
        }
    }
    // These paths were saved explicitly at software level and win over project
    // copies unless the selected project is the explicit migration source.
    if selected.is_none() || candidates.is_empty() {
        for (key, value) in crate::executable_paths::load(home)?
            .as_object()
            .into_iter()
            .flatten()
        {
            settings[key] = value.clone();
        }
    }
    sources.sort();
    sources.dedup();
    let doc = json!({"schemaVersion":"app-settings/v1","revision":1,"settings":settings,"credentialRefs":refs,"catalogs":{},"migration":{"selectedProjectId":selected,"sourceProjectIds":sources,"conflictingFields":conflicts,"skippedProjects":skipped,"projectDataChanged":false}});
    atomic_json(&home.join(FILE), &doc)?;
    Ok(doc)
}
fn validate_patch(patch: &Value) -> Result<()> {
    let map = patch.as_object().ok_or("软件设置必须为对象。")?;
    for (key, value) in map {
        if !FIELDS.contains(&key.as_str()) && !SECRETS.iter().any(|(k, _)| *k == key) {
            return Err(format!(
                "{key} 属于项目参数或不受支持，不能写入软件渠道设置。"
            ));
        }
        if !value.is_string() && !value.is_number() {
            return Err(format!("{key} 设置格式无效。"));
        }
        if value
            .as_str()
            .is_some_and(|v| v.len() > 32768 || v.chars().any(|c| c == '\0'))
        {
            return Err(format!("{key} 设置过长或无效。"));
        }
    }
    if patch.get("chromeProfileId").is_some() || patch.get("chromeProfileEndpoint").is_some() || patch.get("chromeProfileBundleId").is_some() {
        let complete=["chromeProfileId","chromeProfileEndpoint","chromeProfileBundleId"].iter().all(|key|patch[*key].as_str().is_some());
        if !complete {return Err("Chrome 浏览器绑定必须完整保存，未修改原绑定。".into());}
        if !["chromeProfileId","chromeProfileEndpoint","chromeProfileBundleId"].iter().all(|key|s(patch,key).is_empty()) {
            let mut settings=patch.clone();settings["chromeEndpoint"]=patch["chromeProfileEndpoint"].clone();
            crate::chrome_setup::browser_binding(&settings)?;
        }
    }
    Ok(())
}
pub fn save(home: &Path, patch: &Value, expected: u64) -> Result<Value> {
    validate_patch(patch)?;
    let _lock = lock(home)?;
    let mut doc = load(home)?.ok_or("请刷新软件设置后保存。")?;
    if doc["revision"].as_u64() != Some(expected) {
        return Err("APP_SETTINGS_CONFLICT: 软件设置已在别处更新，请刷新后重试。".into());
    }
    for (field, provider) in SECRETS {
        if let Some(secret) = patch[*field].as_str().filter(|s| !s.is_empty()) {
            let reference = format!("global:{provider}:{}", id());
            crate::platform::write_credential(&reference, secret)
                .map_err(|_| "系统未能安全保存凭据，软件设置未更改。")?;
            doc["credentialRefs"][*provider] = json!(reference);
        }
    }
    for field in FIELDS {
        if let Some(value) = patch.get(*field) {
            doc["settings"][*field] = value.clone();
        }
    }
    doc["revision"] = json!(expected.checked_add(1).ok_or("软件设置版本溢出。")?);
    doc["catalogs"] = json!({});
    atomic_json(&home.join(FILE), &doc)?;
    crate::executable_paths::remember(home, &doc["settings"])?;
    Ok(doc)
}
pub fn effective_from(doc: &Value, project: &Value) -> Value {
    let mut settings = project.clone();
    for field in FIELDS {
        if let Some(value) = doc["settings"].get(*field) {
            settings[*field] = value.clone();
        }
    }
    settings[SCOPE] =
        json!({"version":1,"revision":doc["revision"],"credentialRefs":doc["credentialRefs"]});
    settings
}
pub fn effective(home: &Path, project: &Value) -> Result<Value> {
    Ok(match load(home)? {
        Some(doc) => effective_from(&doc, project),
        None => project.clone(),
    })
}
pub fn reference(settings: &Value, project: &str, provider: &str) -> Result<String> {
    if settings[SCOPE].is_object() {
        if settings[SCOPE]["version"] != 1 {
            return Err("冻结的软件渠道版本无效。".into());
        }
        let reference = settings[SCOPE]["credentialRefs"][provider]
            .as_str()
            .ok_or("请在软件渠道设置中保存此渠道凭据。")?;
        let parts: Vec<_> = reference.split(':').collect();
        let valid = match parts.as_slice() {
            ["global", p, key] => *p == provider && uuid::Uuid::parse_str(key).is_ok(),
            [project, p] => *p == provider && uuid::Uuid::parse_str(project).is_ok(),
            _ => false,
        };
        if !valid {
            return Err("冻结的渠道凭据引用无效，未换用其他账户。".into());
        }
        return Ok(reference.into());
    }
    Ok(format!("{project}:{provider}"))
}
pub fn credential(settings: &Value, project: &str, provider: &str) -> Result<String> {
    let reference = reference(settings, project, provider)?;
    if settings[SCOPE]["credentialHashes"].is_object()
        && !settings[SCOPE]["credentialHashes"][provider].is_string()
    {
        return Err("冻结请求没有此渠道的原凭据证据，未改用当前账户。".into());
    }
    let secret = crate::platform::read_credential(&reference)
        .map_err(|_| "原渠道凭据不可用，请在软件设置中检查连接；未换用其他账户。")?;
    if let Some(expected) = settings[SCOPE]["credentialHashes"][provider].as_str() {
        if expected != format!("{:x}", Sha256::digest(secret.as_bytes())) {
            return Err("原渠道凭据已变化，未改用当前账户；请核对原任务。".into());
        }
    }
    Ok(secret)
}
pub fn freeze_credentials(settings: &mut Value) {
    if !settings[SCOPE].is_object() {
        return;
    }
    if settings[SCOPE]["credentialHashes"].is_object() {
        return;
    }
    let mut hashes = Map::new();
    for provider in ["text", "bh", "chrome"] {
        if let Ok(secret) = credential(settings, "", provider) {
            hashes.insert(
                provider.into(),
                json!(format!("{:x}", Sha256::digest(secret.as_bytes()))),
            );
        }
    }
    settings[SCOPE]["credentialHashes"] = json!(hashes);
}
pub fn cache(
    home: &Path,
    provider: &str,
    revision: u64,
    fingerprint: &str,
    cap: &Value,
) -> Result<()> {
    let _lock = lock(home)?;
    let mut doc = load(home)?.ok_or("请刷新软件设置。")?;
    if doc["revision"].as_u64() != Some(revision) {
        return Err("渠道设置在检查期间已变化，请重新检查。".into());
    }
    doc["catalogs"][provider] = json!({"fingerprint":fingerprint,"capabilities":cap});
    atomic_json(&home.join(FILE), &doc)
}
pub fn public(doc: &Value, account: Value, rules: Value) -> Value {
    let providers: Vec<_> = doc["catalogs"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(_, c)| c["capabilities"].clone())
        .collect();
    let credential_status: Map<_, _> = ["text", "bh", "chrome"]
        .into_iter()
        .map(|p| {
            (
                p.into(),
                json!({"configured":doc["credentialRefs"][p].is_string()}),
            )
        })
        .collect();
    json!({"schemaVersion":"app-settings/v1","revision":doc["revision"],"settings":doc["settings"],"providers":providers,"dreaminaAccount":account,"rules":rules,"credentialStatus":credential_status,"migration":doc["migration"]})
}

#[cfg(test)]
mod tests;
