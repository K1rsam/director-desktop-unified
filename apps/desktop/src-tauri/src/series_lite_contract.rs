//! Frozen rule identity and byte verification, not an enabled production route.
//! Original Node/Python files are checked as provenance; none are executed.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

type Result<T> = std::result::Result<T, String>;
pub const PROFILE: &str = "series-lite-rules-contract/v1";
pub const CREATIVE_VERSION: &str = "0.1.2";
pub const RELEASE: &str = "20261027-v0.1.2-usage-reuse";
pub const SOURCE_COMMIT: &str = "6dea8d6c57dde38f04a4d71f05f4c039004dd4a9";
pub const MANIFEST_SHA256: &str =
    "4d6e37e88da4c9f6a01bb817993e93581957827a5ebda8acdd9327df149d6f56";
pub const ADAPTER_VERSION: &str = "series-lite-rust-adapter/v0.1.0-unaccepted";
const ACTIVE: bool = false;
const FILE_COUNT: usize = 90;
const MANIFEST: &[u8] =
    include_bytes!("../../../../rules/series-lite/20261027-v0.1.2/SHA256SUMS.txt");
#[path = "series_lite_contract_embedded.rs"]
mod embedded;
#[path = "series_lite_contract_041_embedded.rs"]
mod embedded_041;

pub const CURRENT_RELEASE: &str = "20261028-v0.4.1-3d-fantasy";
pub const CURRENT_SOURCE_COMMIT: &str = "f855227c8406b38093d03c777fe24a0b0b4f87dc";
pub const CURRENT_ADAPTER_VERSION: &str = "series-lite-rust-adapter/v0.4.1-candidate.1";
pub const CURRENT_MANIFEST_SHA256: &str =
    "25e213960a03eddb59928108ed7e95f4743b674074736129deefe803b972c538";
const CURRENT_MANIFEST: &[u8] =
    include_bytes!("../../../../rules/series-lite/20261028-v0.4.1/SHA256SUMS.txt");

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn error(detail: &str) -> String {
    format!("SERIES_LITE_CONTRACT: {detail}")
}
fn manifest(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    let current = hash(bytes) == CURRENT_MANIFEST_SHA256;
    if !current && hash(bytes) != MANIFEST_SHA256 {
        return Err(error("冻结清单字节已漂移"));
    }
    let mut rows = BTreeMap::new();
    for line in std::str::from_utf8(bytes)
        .map_err(|_| error("清单不是UTF-8"))?
        .lines()
    {
        let (sha, name) = line.split_once("  ").ok_or_else(|| error("清单行无效"))?;
        if sha.len() != 64
            || !sha.bytes().all(|b| b.is_ascii_hexdigit())
            || name.is_empty()
            || name.contains(['\\', ':'])
            || Path::new(name)
                .components()
                .any(|p| !matches!(p, Component::Normal(_)))
            || rows.insert(name.into(), sha.into()).is_some()
        {
            return Err(error("清单含重复或越界路径"));
        }
    }
    if rows.len() != if current { 106 } else { FILE_COUNT } {
        return Err(error("冻结清单文件数量不符"));
    }
    Ok(rows)
}

fn identity() -> Value {
    json!({"schemaVersion":PROFILE,"skill":"series-storyboard-director-lite",
        "creativeVersion":CREATIVE_VERSION,"releaseVersion":RELEASE,
        "sourceRepository":"K1rsam/series-storyboard-director","sourceCommit":SOURCE_COMMIT,
        "checksumFileSha256":MANIFEST_SHA256,"rustAdapterVersion":ADAPTER_VERSION,"active":ACTIVE,
        "dataContracts":{"bundle":"series-storyboard-bundle-v1","plan":"performance-core-v4-plan",
            "draft":"performance-core-v4","generationText":"voice-delivery-v1","exportProfile":"lingmo-v7"}})
}

/// Immutable in-memory bytes: a later on-disk edit cannot change the inspected
/// rules underneath a compiled packet. No public unchecked constructor exists.
pub struct VerifiedPack {
    files: BTreeMap<String, Vec<u8>>,
    current: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExistingContract {
    LegacyUnchanged,
    VerifiedFrozenRulesOnly,
}

impl VerifiedPack {
    /// Installed clients carry the exact frozen originals. No source checkout,
    /// mutable global Skill directory, Python or Node is required to read them.
    pub fn embedded() -> Result<Self> {
        Self::verify_files(MANIFEST, embedded::files())
    }
    /// Explicit selection for NEW work. Historical callers retain embedded().
    pub fn current_embedded() -> Result<Self> {
        Self::verify_files(CURRENT_MANIFEST, embedded_041::files())
    }
    pub fn is_current(&self) -> bool { self.current }
    pub fn release(&self) -> &'static str {
        if self.current { CURRENT_RELEASE } else { RELEASE }
    }
    pub fn manifest_sha256(&self) -> &'static str {
        if self.current { CURRENT_MANIFEST_SHA256 } else { MANIFEST_SHA256 }
    }
    pub fn compiler_version(&self) -> &'static str {
        if self.current { "lingmo-lite-role-rules-v2" } else { "lingmo-lite-role-rules-v1" }
    }
    pub fn file_bytes(&self, path: &str) -> Result<&[u8]> {
        self.files.get(path).map(Vec::as_slice).ok_or_else(|| error("冻结规则文件缺失"))
    }
    pub fn contract_identity(&self) -> Value {
        let mut result = identity();
        if self.current {
            result["creativeVersion"] = json!("0.4.1");
            result["releaseVersion"] = json!(CURRENT_RELEASE);
            result["sourceCommit"] = json!(CURRENT_SOURCE_COMMIT);
            result["checksumFileSha256"] = json!(CURRENT_MANIFEST_SHA256);
            result["rustAdapterVersion"] = json!(CURRENT_ADAPTER_VERSION);
            result["active"] = json!(true);
            result["activationScope"] = json!("new-explicit-desktop-lite-tasks/v1");
            result["productionAcceptance"] = json!("candidate-real-model-review-pending");
            result["dataContracts"]["generationBoundary"] = json!("video-prompt-v1");
            result["dataContracts"]["imageQueue"] = json!("series-image-queue-v2");
        }
        result
    }
    /// Load every file from the pinned release manifest. This checks provenance
    /// only; successful loading never grants model dispatch or adoption authority.
    pub fn load(directory: &Path) -> Result<Self> {
        let root = directory
            .canonicalize()
            .map_err(|e| error(&format!("规则目录不可读：{e}")))?;
        let bytes =
            std::fs::read(root.join("SHA256SUMS.txt")).map_err(|e| error(&e.to_string()))?;
        let entries = manifest(&bytes)?;
        let mut files = BTreeMap::new();
        for name in entries.keys() {
            let path = root
                .join(name)
                .canonicalize()
                .map_err(|e| error(&format!("缺少{name}：{e}")))?;
            if !path.starts_with(&root) || !path.is_file() {
                return Err(error("规则文件越出冻结目录"));
            }
            files.insert(
                name.clone(),
                std::fs::read(path).map_err(|e| error(&e.to_string()))?,
            );
        }
        Self::verify_files(&bytes, files)
    }
    fn verify_files(checksums: &[u8], files: BTreeMap<String, Vec<u8>>) -> Result<Self> {
        let entries = manifest(checksums)?;
        let current = hash(checksums) == CURRENT_MANIFEST_SHA256;
        if files.len() != entries.len() {
            return Err(error("规则集合缺失或包含未知文件"));
        }
        for (name, sha) in entries {
            if files.get(&name).is_none_or(|bytes| hash(bytes) != sha) {
                return Err(error(&format!("完整规则字节校验失败：{name}")));
            }
        }
        if files
            .get("VERSION")
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(str::trim)
            != Some(if current { CURRENT_RELEASE } else { RELEASE })
        {
            return Err(error("创作版本与冻结版本不符"));
        }
        Ok(Self { files, current })
    }
    pub fn inspection(&self) -> Value {
        if self.current {
            return json!({"contract":self.contract_identity(),"filesVerified":self.files.len(),"active":true,
                "implementedCapabilities":["pinned_release_byte_validation","complete_role_rule_bytes","episode_local_prepare","derived_creative_compose","expected_value_patch","three_role_saved_controller","bounded_revisions","native_execution_and_adoption"],
                "productionCompatibility":"desktop_candidate_real_model_acceptance_pending","modelDispatchAllowed":true,
                "activationScope":"new-explicit-desktop-lite-tasks/v1","standalonePackDispatchAllowed":false,
                "threeRoleRuntimeAccepted":false,"historicalResigningAllowed":false,"runtimeScriptExecutionRequired":false,
                "boundary":"已接候选运行器；工程与安装证据不能替代新剧本真实生成、独审与用户验收"});
        }
        json!({"contract":self.contract_identity(),"filesVerified":self.files.len(),"active":false,
            "implementedCapabilities":["pinned_release_byte_validation","complete_role_rule_bytes","frozen_rule_identity_check"],
            "productionCompatibility":"not_accepted","modelDispatchAllowed":false,
            "fullRustGateAccepted":false,"threeRoleRuntimeAccepted":false,"historicalResigningAllowed":false,
            "runtimeScriptExecutionRequired":false,
            "boundary":"规则验证不等于完整供料、内容独审、采用、导出或恢复验收"})
    }
    fn paths(role: &str, fantasy: bool) -> Result<Vec<&'static str>> {
        Self::selected_paths(
            role,
            if fantasy {
                &["fantasy-expression-methods"]
            } else {
                &[]
            },
            &[],
        )
    }
    /// Frozen job_packet.py selects whole rule files and deduplicates paths in
    /// first-use order. A shared method does not add a truncated rule summary.
    fn selected_paths(
        role: &str,
        methods: &[&str],
        contracts: &[&str],
    ) -> Result<Vec<&'static str>> {
        let specific = match role {
            "author" => "references/author.md",
            "finisher" => "references/finisher.md",
            "reviewer" => "references/reviewer.md",
            _ => return Err(error("未知岗位，不猜测兼容")),
        };
        let mut paths = vec![
            "references/creative-core.md",
            "references/schema.md",
            specific,
        ];
        let mut seen_methods = std::collections::BTreeSet::new();
        for method in methods {
            if !seen_methods.insert(*method) {
                return Err(error("重复创作方法"));
            }
            let path = match *method {
                "acting-blocking-methods"
                | "camera-emotion-methods"
                | "sound-rhythm-methods"
                | "lighting-space-methods"
                | "director-refinement" => "references/creative-core.md",
                "fantasy-expression-methods" => "references/fantasy.md",
                _ => return Err(error("未知创作方法")),
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        let mut seen_contracts = std::collections::BTreeSet::new();
        for contract in contracts {
            if !seen_contracts.insert(*contract) {
                return Err(error("重复岗位合同"));
            }
            let path = match *contract {
                "system-presentation" => "references/system.md",
                "asset-registry-contract" => "references/schema.md",
                "shot-handoffs" => "references/creative-core.md",
                _ => return Err(error("未知岗位合同")),
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        Ok(paths)
    }
    pub fn selected_role_rules(
        &self,
        role: &str,
        methods: &[&str],
        contracts: &[&str],
    ) -> Result<BTreeMap<String, Vec<u8>>> {
        Self::selected_paths(role, methods, contracts)?
            .into_iter()
            .map(|path| {
                self.files
                    .get(path)
                    .cloned()
                    .map(|bytes| (path.into(), bytes))
                    .ok_or_else(|| error("冻结岗位规则缺失"))
            })
            .collect()
    }
    /// Inspection/conformance material only. The future runtime must call the
    /// closed production gate before allocating any new model task.
    pub fn inspection_role_rules(
        &self,
        role: &str,
        fantasy: bool,
    ) -> Result<BTreeMap<String, Vec<u8>>> {
        Self::paths(role, fantasy)?
            .into_iter()
            .map(|path| {
                self.files
                    .get(path)
                    .cloned()
                    .map(|bytes| (path.into(), bytes))
                    .ok_or_else(|| error("冻结岗位规则缺失"))
            })
            .collect()
    }
    /// Check the ACTUAL complete bytes included by a packet compiler, not a
    /// caller-supplied digest or a claim that a file was read.
    pub fn verify_runtime_rules(
        &self,
        role: &str,
        fantasy: bool,
        actual: &BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        let expected = self.inspection_role_rules(role, fantasy)?;
        if actual != &expected {
            return Err(error("运行岗位规则缺失、额外或字节漂移"));
        }
        Ok(())
    }
    /// Existing unrelated frozen requests remain owned by their original reader.
    /// This never fills a missing marker or upgrades old approvals to this release.
    pub fn verify_existing(
        &self,
        snapshot: &Value,
        role: &str,
        fantasy: bool,
        actual: &BTreeMap<String, Vec<u8>>,
    ) -> Result<ExistingContract> {
        let Some(saved) = snapshot.get("seriesLiteContract") else {
            return Ok(ExistingContract::LegacyUnchanged);
        };
        if saved != &self.contract_identity() {
            return Err(error(
                "未知或漂移的创作/数据/适配器合同；保留原任务，不重签",
            ));
        }
        self.verify_runtime_rules(role, fantasy, actual)?;
        Ok(ExistingContract::VerifiedFrozenRulesOnly)
    }
    /// Deliberately unconditionally closed. Editing an external receipt's active
    /// flag or presenting a valid rule hash cannot enable the unfinished route.
    pub fn require_new_production(&self) -> Result<()> {
        Err(
            "SERIES_LITE_INACTIVE: 完整Rust gate、三岗执行、采用与恢复链尚未验收；不分配新任务"
                .into(),
        )
    }
}

#[cfg(test)]
#[path = "series_lite_contract_tests.rs"]
mod tests;
