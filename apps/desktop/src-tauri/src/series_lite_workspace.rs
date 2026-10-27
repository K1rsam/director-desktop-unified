//! Read-only frozen 0.1.2 workspace/load_state verification. Never adopts output.
//! Caller must hold its workspace lease across verification and packet freezing.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;
#[path = "series_lite_workspace_handoff.rs"]
mod handoff;

fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("WORKSPACE: {message}"))
    }
}
fn hash(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}
fn parse(raw: &[u8]) -> Result<Value> {
    serde_json::from_slice(raw).map_err(|e| format!("WORKSPACE_JSON: {e}"))
}
fn array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().ok_or("WORKSPACE: array required".into())
}
fn string(v: &Value) -> Result<&str> {
    v.as_str().ok_or("WORKSPACE: string required".into())
}
fn same(a: &Value, b: &Value) -> bool {
    matches!((crate::series_lite_registry::canonical(a),crate::series_lite_registry::canonical(b)),(Ok(a),Ok(b)) if a==b)
}
fn text(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim_matches(js_space).is_empty() && !s.contains(['\0', '\r', '\n']))
}
fn js_space(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn fields(v: &Value, allowed: &[&str]) -> Result<()> {
    need(
        v.as_object()
            .is_some_and(|o| o.keys().all(|k| allowed.contains(&k.as_str()))),
        "unknown fields or invalid object",
    )
}
fn record(v: &Value) -> Result<()> {
    need(
        v.is_object()
            && matches!(v["kind"].as_str(), Some("user" | "delegated"))
            && text(&v["basis"]),
        "adoption kind/basis missing",
    )
}
/// Records exact bytes, then rechecks all inputs before returning. This detects
/// ordinary concurrent drift; it does not replace the owner's exclusive lease.
struct Reads {
    root: PathBuf,
    files: BTreeMap<PathBuf, Vec<u8>>,
    aliases: BTreeMap<PathBuf, PathBuf>,
}
impl Reads {
    fn new(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root).map_err(|e| format!("WORKSPACE_PATH: {e}"))?;
        need(root.is_dir(), "workspace root is not a directory")?;
        Ok(Self {
            root,
            files: BTreeMap::new(),
            aliases: BTreeMap::new(),
        })
    }
    fn path(&self, path: &Path) -> Result<PathBuf> {
        let path = fs::canonicalize(path).map_err(|e| format!("WORKSPACE_PATH: {e}"))?;
        need(
            path.starts_with(&self.root) && path.is_file(),
            "file escapes workspace or is not regular",
        )?;
        Ok(path)
    }
    fn read(&mut self, path: &Path) -> Result<Vec<u8>> {
        let requested = path.to_path_buf();
        let path = self.path(path)?;
        if let Some(old) = self.aliases.get(&requested) {
            need(*old == path, "file path redirected within verification")?;
        }
        self.aliases.insert(requested, path.clone());
        let raw = fs::read(&path).map_err(|e| format!("WORKSPACE_READ: {e}"))?;
        if let Some(before) = self.files.get(&path) {
            need(*before == raw, "file changed within verification")?;
        }
        self.files.insert(path, raw.clone());
        Ok(raw)
    }
    fn local(&mut self, name: &str) -> Result<Vec<u8>> {
        self.read(&self.root.join(name))
    }
    fn finish(&self) -> Result<()> {
        for (requested, resolved) in &self.aliases {
            need(
                self.path(requested)? == *resolved,
                "file path redirected during verification",
            )?;
        }
        for (path, raw) in &self.files {
            need(
                self.path(path)? == *path && fs::read(path).map_err(|e| e.to_string())? == *raw,
                "file drift during verification",
            )?;
        }
        Ok(())
    }
}

/// Exact native equivalent of project_index(index_path) for embedded scope.
pub fn project_index(index: &Value) -> Result<Value> {
    let Some(scope) = index.get("productionScope").filter(|v| !v.is_null()) else {
        need(
            index.get("scopeHash").is_none() && index.get("readingScope").is_none(),
            "partial season credentials cannot downgrade to legacy",
        )?;
        return Ok(index.clone());
    };
    crate::series_lite_registry::validate_season_scope(scope, None, Some(index), true)?;
    let selected = array(&scope["episodes"])?;
    let mut result = index.clone();
    result["episodes"] = json!(array(&index["episodes"])?
        .iter()
        .filter(|e| selected.iter().any(|s| s["number"] == e["number"]))
        .cloned()
        .collect::<Vec<_>>());
    result["readingScope"] = scope["readingScope"].clone();
    result["productionScope"] = scope.clone();
    result["scopeHash"] = json!(crate::series_lite_registry::hash_season_scope(scope)?);
    Ok(result)
}
pub fn check_state_scope(state: &Value, index: &Value) -> Result<()> {
    let Some(scope) = index.get("productionScope").filter(|v| !v.is_null()) else {
        return need(
            state.get("productionScope").is_none() && state.get("scopeHash").is_none(),
            "season state differs from frozen index",
        );
    };
    need(
        same(&state["productionScope"], scope) && state["scopeHash"] == index["scopeHash"],
        "season state/hash differs from frozen index",
    )?;
    let active = array(&state["scope"])?;
    let selected: Vec<_> = array(&scope["episodes"])?
        .iter()
        .map(|e| e["number"].clone())
        .filter(|n| active.contains(n))
        .collect();
    need(
        !active.is_empty() && *active == selected,
        "scope outside locked season or source order changed",
    )?;
    let mut groups = Vec::new();
    for g in array(&state["groups"])? {
        groups.extend(array(&g["episodes"])?.iter().cloned());
    }
    let episodes = state["episodes"]
        .as_object()
        .ok_or("WORKSPACE: episode status map missing")?;
    let keys: std::collections::BTreeSet<_> = active
        .iter()
        .map(|n| {
            n.as_i64()
                .map(|n| n.to_string())
                .ok_or("WORKSPACE: episode number must be integer")
        })
        .collect::<std::result::Result<_, _>>()?;
    need(
        groups == *active
            && episodes
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                == keys,
        "queue differs from locked season",
    )
}
/// Returns original state.json bytes only after all load_state dependencies pass.
pub fn verify(root: &Path) -> Result<Vec<u8>> {
    let mut reads = Reads::new(root)?;
    let extraction = parse(&reads.local("extraction.json")?)?;
    let source = reads.local(string(&extraction["sourceCopy"])?)?;
    need(
        hash(&source) == extraction["sha256"],
        "original source copy changed",
    )?;
    need(
        hash(&reads.local("source-body.txt")?) == extraction["bodyHash"],
        "extracted body changed",
    )?;
    let raw = reads.local("state.json")?;
    let state = parse(&raw)?;
    need(
        state["version"] == "series-workspace-v1",
        "unknown workspace version",
    )?;
    need(
        state["sourceHash"] == extraction["sha256"],
        "source version changed",
    )?;
    let index = reads.local("source-index.json")?;
    need(
        hash(&index) == state["sourceIndexHash"],
        "source index changed",
    )?;
    check_state_scope(&state, &project_index(&parse(&index)?)?)?;
    let actual = handoff::previous(&mut reads, &state)?;
    need(
        same(
            state.get("continuationEvidence").unwrap_or(&Value::Null),
            &actual,
        ),
        "previous season evidence differs from current continuation",
    )?;
    reads.finish()?;
    Ok(raw)
}
pub struct NativeWorkspaceScopeVerifier;
impl crate::series_lite_job_packet::WorkspaceScopeVerifier for NativeWorkspaceScopeVerifier {
    fn verify_workspace(&self, root: &Path) -> Result<Vec<u8>> {
        verify(root)
    }
}

#[cfg(test)]
#[path = "series_lite_workspace_tests.rs"]
mod tests;
