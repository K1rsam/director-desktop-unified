//! Diagnose one new-profile transport conflict without changing the old
//! (assetId, variantId) uniqueness gate or merging any model-written refs.
use super::*;

pub(super) fn unit_issues(unit: &Value) -> Result<Vec<Value>, String> {
    let mut groups: HashMap<(String, String), Vec<&Value>> = HashMap::new();
    for reference in unit["assetRefs"]
        .as_array()
        .ok_or("单元缺少结构化资产引用")?
    {
        groups
            .entry((
                text(reference, "assetId").into(),
                text(reference, "variantId").into(),
            ))
            .or_default()
            .push(reference);
    }
    let mut issues = Vec::new();
    for ((asset, variant), references) in groups {
        if references.len() == 1 {
            continue;
        }
        let invalid = || crate::text_executor::AUTHOR_ASSET_USE_DUPLICATE_REJECTION.to_string();
        if !super::super::asset_use_binding::enabled(unit)? || references.len() != 2 {
            return Err(invalid());
        }
        let visual = references
            .iter()
            .find(|reference| reference["use"] == "character");
        let audio = references
            .iter()
            .find(|reference| reference["use"] == "audio");
        let (Some(visual), Some(audio)) = (visual, audio) else {
            return Err(invalid());
        };
        let visual_shots = strings(visual, "useShotIds")?;
        let audio_shots = strings(audio, "useShotIds")?;
        let known: HashSet<_> = unit["shots"]
            .as_array()
            .ok_or("单元缺少镜表")?
            .iter()
            .map(|shot| text(shot, "id"))
            .collect();
        if visual_shots.is_empty()
            || audio_shots.is_empty()
            || visual_shots.iter().any(|id| audio_shots.contains(id))
            || visual_shots
                .iter()
                .chain(&audio_shots)
                .any(|id| !known.contains(id.as_str()))
            || visual_shots.iter().collect::<HashSet<_>>().len() != visual_shots.len()
            || audio_shots.iter().collect::<HashSet<_>>().len() != audio_shots.len()
        {
            return Err(invalid());
        }
        issues.push(json!({"code":"ASSET_USE_IDENTITY","unitId":unit["id"],"coordinate":format!("{}.assetRefs",text(unit,"id")),
            "assetId":asset,"variantId":variant,"sourceRefs":unit["sourceRefs"],
            "uses":[{"use":"character","useShotIds":visual_shots},{"use":"audio","useShotIds":audio_shots}],
            "problem":crate::text_executor::AUTHOR_ASSET_USE_DUPLICATE_REJECTION}));
    }
    issues.sort_by_key(|issue| {
        (
            text(issue, "assetId").to_owned(),
            text(issue, "variantId").to_owned(),
        )
    });
    Ok(issues)
}

pub(super) fn candidate(
    source: &LockedSource,
    raw: &Value,
    assets: &[Value],
    caps: &Value,
    refs: &[String],
    style: &str,
    prefix: &str,
    plan: &Value,
    selected: &[Value],
) -> Result<(Value, Vec<Value>), String> {
    if raw["schemaVersion"] != "storyboard-draft/v2" {
        return Err("素材使用冲突缺少原冻结分镜协议".into());
    }
    let mut draft = raw.clone();
    materialize_dialogue(source, &mut draft, false)?;
    materialize_asset_references(&mut draft, assets)?;
    prefix_ids(&mut draft, prefix)?;
    add_delivery_fields(&mut draft, style)?;
    let mut issues = Vec::new();
    validate_storyboard_repair_scope(
        source,
        &draft,
        assets,
        caps,
        Some(refs),
        Some(&mut issues),
        false,
        false,
        false,
        true,
    )?;
    if !issues
        .iter()
        .any(|issue| issue["code"] == "ASSET_USE_IDENTITY")
    {
        return Err("原拒绝没有可证实的不同用途/不相交镜号冲突；不重新整编".into());
    }
    validate_event_bindings(&draft, plan)?;
    validate_fantasy_profile(&draft, selected, true)?;
    materialize_unit_coverage(source, &mut draft)?;
    // This remains an unaccepted diagnostic draft with both original refs.
    // Only the original editor may resolve it, then pass strict export gates.
    Ok((draft, issues))
}
