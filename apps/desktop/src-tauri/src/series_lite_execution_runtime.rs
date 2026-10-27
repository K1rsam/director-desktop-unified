use super::*;
const MANIFEST: &[u8] =
    include_bytes!("../../../../rules/series-lite/20261027-v0.1.2/SHA256SUMS.txt");
pub(super) fn current() -> Result<Json> {
    need(
        sha256(MANIFEST) == crate::series_lite_contract::MANIFEST_SHA256,
        "runtime manifest drift",
    )?;
    let mut runtime = BTreeMap::new();
    for line in std::str::from_utf8(MANIFEST)
        .map_err(|_| err("runtime manifest UTF-8"))?
        .lines()
    {
        let (hash, path) = line
            .split_once("  ")
            .ok_or_else(|| err("runtime manifest row"))?;
        if let Some(path) = path.strip_prefix("scripts/") {
            if path.ends_with(".py") || path.ends_with(".mjs") {
                runtime.insert(path.to_string(), text(hash));
            }
        }
    }
    let native: &[(&str, &[u8])] = &[
        (
            "_native/series_lite_asset_uses.rs",
            include_bytes!("series_lite_asset_uses.rs"),
        ),
        (
            "_native/series_lite_bundle_draft.rs",
            include_bytes!("series_lite_bundle_draft.rs"),
        ),
        (
            "_native/series_lite_bundle_gate_check.rs",
            include_bytes!("series_lite_bundle_gate_check.rs"),
        ),
        (
            "_native/series_lite_bundle_gate.rs",
            include_bytes!("series_lite_bundle_gate.rs"),
        ),
        (
            "_native/series_lite_bundle_json.rs",
            include_bytes!("series_lite_bundle_json.rs"),
        ),
        (
            "_native/series_lite_bundle_parse.rs",
            include_bytes!("series_lite_bundle_parse.rs"),
        ),
        (
            "_native/series_lite_bundle_project.rs",
            include_bytes!("series_lite_bundle_project.rs"),
        ),
        (
            "_native/series_lite_bundle_source.rs",
            include_bytes!("series_lite_bundle_source.rs"),
        ),
        (
            "_native/series_lite_bundle_standard_headers.rs",
            include_bytes!("series_lite_bundle_standard_headers.rs"),
        ),
        (
            "_native/series_lite_bundle_standard_inline.rs",
            include_bytes!("series_lite_bundle_standard_inline.rs"),
        ),
        (
            "_native/series_lite_bundle_standard_package.rs",
            include_bytes!("series_lite_bundle_standard_package.rs"),
        ),
        (
            "_native/series_lite_bundle_standard_rows.rs",
            include_bytes!("series_lite_bundle_standard_rows.rs"),
        ),
        (
            "_native/series_lite_bundle_standard.rs",
            include_bytes!("series_lite_bundle_standard.rs"),
        ),
        (
            "_native/series_lite_bundle.rs",
            include_bytes!("series_lite_bundle.rs"),
        ),
        (
            "_native/series_lite_contract_embedded.rs",
            include_bytes!("series_lite_contract_embedded.rs"),
        ),
        (
            "_native/series_lite_contract.rs",
            include_bytes!("series_lite_contract.rs"),
        ),
        (
            "_native/series_lite_edit.rs",
            include_bytes!("series_lite_edit.rs"),
        ),
        (
            "_native/series_lite_execution_fs.rs",
            include_bytes!("series_lite_execution_fs.rs"),
        ),
        (
            "_native/series_lite_execution_inputs.rs",
            include_bytes!("series_lite_execution_inputs.rs"),
        ),
        (
            "_native/series_lite_execution_reports.rs",
            include_bytes!("series_lite_execution_reports.rs"),
        ),
        (
            "_native/series_lite_execution.rs",
            include_bytes!("series_lite_execution.rs"),
        ),
        (
            "_native/series_lite_job_packet_frozen.rs",
            include_bytes!("series_lite_job_packet_frozen.rs"),
        ),
        (
            "_native/series_lite_job_packet.rs",
            include_bytes!("series_lite_job_packet.rs"),
        ),
        (
            "_native/series_lite_registry_season.rs",
            include_bytes!("series_lite_registry_season.rs"),
        ),
        (
            "_native/series_lite_registry_unicode.rs",
            include_bytes!("series_lite_registry_unicode.rs"),
        ),
        (
            "_native/series_lite_registry_unicode17.json",
            include_bytes!("series_lite_registry_unicode17.json"),
        ),
        (
            "_native/series_lite_registry.rs",
            include_bytes!("series_lite_registry.rs"),
        ),
        (
            "_native/series_lite_workspace_handoff.rs",
            include_bytes!("series_lite_workspace_handoff.rs"),
        ),
        (
            "_native/series_lite_workspace_visible.rs",
            include_bytes!("series_lite_workspace_visible.rs"),
        ),
        (
            "_native/series_lite_workspace.rs",
            include_bytes!("series_lite_workspace.rs"),
        ),
        (
            "_native/series_lite_execution_runtime.rs",
            include_bytes!("series_lite_execution_runtime.rs"),
        ),
    ];
    for (path, raw) in native {
        runtime.insert((*path).into(), text(sha256(raw)));
    }
    Ok(Json::Object(runtime))
}
