use super::*;

const VERSIONS: [&str; 10] = [
    LEGACY_PARSER_VERSION, CONTEXT_PARSER_VERSION, CURRENT_PARSER_VERSION,
    STRUCTURAL_PARSER_VERSION, DISPLAY_PARSER_VERSION, FIDELITY_PARSER_VERSION,
    MULTILINE_DISPLAY_PARSER_VERSION, JOINT_SPEAKER_PARSER_VERSION,
    TITLE_METADATA_PARSER_VERSION, EXPLICIT_INLINE_PARSER_VERSION,
];

fn exact(source: &LockedSource, text: &str) {
    assert_eq!(source.text, text);
    assert_eq!(source.sha256, digest(text));
    assert_eq!(source.length, text.encode_utf16().count());
    assert_eq!(source.rows.iter().map(|row| row.text.as_str()).collect::<String>(), text);
    for row in &source.rows {
        assert_eq!(slice_utf16(text, &row.range).unwrap(), row.text);
    }
    validate_source(source).unwrap();
}

#[test]
fn explicit_biography_prefaces_need_heading_and_local_record_evidence() {
    for prefix in [
        "合成角色测试150集剧本\r\n主角：林川 人物小传\r\n【基本资料】\r\n姓名：林川 成年\r\n身份：修理员\r\n【外貌】\r\n黑发，戴旧帽😀。\r\n",
        "# 《合成角色测试》\r\n## 主角：林川 角色小传\r\n### 基本信息\r\n姓名：林川\r\n身份：修理员\r\n",
        "**角色档案：林川**\r\n姓名：林川\r\n职业：修理员\r\n",
        "【人物档案】\r\n【基础资料】\r\n姓名：林川\r\n身份：修理员\r\n",
        "林川 人物小传\r\n姓名：林川\r\n身份：修理员\r\n",
    ] {
        let text = format!("{prefix}\r\n第1集\r\n1-1 日 内 工坊\r\n林川：我回来了😀。\r\n");
        let old = parse_source(&text, EXPLICIT_INLINE_PARSER_VERSION).unwrap();
        let current = prepare_source(&text).unwrap();
        assert_eq!(current.parser_version, FORMAT_PARSER_VERSION, "{prefix}");
        assert!(current.diagnostics.is_empty(), "{:?}", current.diagnostics);
        for row in current.rows.iter().take(prefix.lines().count()) {
            assert_eq!(row.kind, "context");
            assert!(row.episode_id.is_none() && row.scene_id.is_none() && row.dialogue.is_none());
        }
        for (before, after) in old.rows.iter().zip(&current.rows) {
            assert_eq!((before.id.as_str(), &before.range, before.text.as_str(), before.number),
                (after.id.as_str(), &after.range, after.text.as_str(), after.number));
        }
        assert_eq!(serde_json::to_value(&old.episodes).unwrap(), serde_json::to_value(&current.episodes).unwrap());
        assert_eq!(serde_json::to_value(&old.scenes).unwrap(), serde_json::to_value(&current.scenes).unwrap());
        exact(&current, &text);
    }
}

#[test]
fn keywords_without_metadata_anchors_do_not_hide_narrative() {
    for prefix in [
        "主角：林川 人物小传\n林川走进门。\n",
        "主角：林川 人物小传\n【基本资料】\n林川：我回来。\n",
        "主角：林川 人物小传\n姓名：林川\n林川：回来。\n",
        "主角：林川 人物小传\n姓名：林川\n姓名：张海\n",
        "他说主角：林川 人物小传\n【基本资料】\n姓名：林川\n",
        "林川看见了人物小传。\n【基本资料】\n姓名：林川\n",
        "林川推门进来。\n主角：林川 人物小传\n【基本资料】\n姓名：林川\n",
    ] {
        let text = format!("{prefix}第1集\n1-1 日 内 工坊\n林川：回来。\n");
        assert!(biography_preface_rows(&text).is_empty(), "{prefix}");
        let source = prepare_source(&text).unwrap();
        assert_ne!(source.parser_version, FORMAT_PARSER_VERSION, "{prefix}");
        assert!(source.diagnostics.iter().any(|d| d.code == "BOUNDARY_UNCONFIRMED" && d.blocking));
        exact(&source, &text);
    }
}

#[test]
fn production_markers_end_biography_instead_of_absorbing_unassigned_story() {
    for marker in ["△林川推门。", "场景：工坊", "1-1 日 内 工坊", "1-0", "正文"] {
        let text = format!("主角：林川 人物小传\n【基本资料】\n姓名：林川\n{marker}\n林川：开门。\n第1集\n1-1 日 内 工坊\n林川：回来。\n");
        let source = prepare_source(&text).unwrap();
        assert_eq!(source.parser_version, FORMAT_PARSER_VERSION);
        assert!(source.rows[..3].iter().all(|row| row.kind == "context"));
        assert_eq!(source.rows[3].kind, "body", "{marker}");
        assert_eq!(source.rows[4].kind, "body", "{marker}");
        assert!(source.diagnostics.iter().any(|d| d.code == "BOUNDARY_UNCONFIRMED" && d.blocking));
        exact(&source, &text);
    }
}

#[test]
fn biography_words_and_record_shaped_dialogue_inside_story_remain_body() {
    let text = "第1集\n1-0 日 内 工坊\n林川：他说人物小传还没写完。\n主角：林川 人物小传\n姓名：林川\n身份：修理员\n△林川翻开角色档案。\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.parser_version, FORMAT_PARSER_VERSION);
    assert!(source.rows[2..].iter().all(|row| row.kind == "body" && row.scene_id.is_some()));
    assert!(source.diagnostics.is_empty());
    exact(&source, text);
}

#[test]
fn zero_scene_number_is_legal_with_matching_positive_episode_and_metadata() {
    for opening in [
        "第1集\n1-0 工坊 日 内\n",
        "第1集\n1-0 日 内 工坊\n",
        "第1集\n1-0日 内 工坊\n",
        "第1集\n1-0\n日 内 工坊\n",
        "第1集\n### 1-0 日 内 工坊\n",
        "第1集 开场 场1-0 日 内 工坊\n",
    ] {
        let text = format!("{opening}林川：开始😀。\n1-1 夜 外 门口\n林川：结束。\n");
        let old = parse_source(&text, EXPLICIT_INLINE_PARSER_VERSION).unwrap();
        let current = prepare_source(&text).unwrap();
        assert_eq!(current.parser_version, FORMAT_PARSER_VERSION, "{opening}");
        assert!(current.diagnostics.is_empty(), "{opening}: {:?}", current.diagnostics);
        assert_eq!(current.episodes.len(), 1);
        assert_eq!(current.scenes.len(), 2);
        assert_eq!(current.episodes[0].id, old.episodes[0].id);
        assert_eq!(current.scenes[1].id, old.scenes.last().unwrap().id);
        assert_eq!(current.scenes[1].row_ids, old.scenes.last().unwrap().row_ids);
        for (before, after) in old.rows.iter().zip(&current.rows) {
            assert_eq!((&before.id, &before.range, &before.text, before.number),
                (&after.id, &after.range, &after.text, after.number));
        }
        exact(&current, &text);
    }
}

#[test]
fn zero_scene_does_not_license_episode_zero_mismatch_or_numeric_prose() {
    let zero = prepare_source("第0集\n0-0 日 内 工坊\n林川：资料。\n").unwrap();
    assert!(zero.episodes.is_empty() && zero.scenes.is_empty());
    let mismatch = prepare_source("第1集\n2-0 日 内 工坊\n林川：回来。\n").unwrap();
    assert!(mismatch.scenes.is_empty());
    assert!(mismatch.diagnostics.iter().any(|d| d.code == "SCENE_EPISODE_MISMATCH"));
    for line in ["1.0 秒后，屋内响起声音。", "1-0日记写完了。", "0-0 日 内 工坊"] {
        let text = format!("第1集\n1-1 日 内 工坊\n{line}\n林川：回来。\n");
        let source = prepare_source(&text).unwrap();
        assert_eq!(source.scenes.len(), 1, "{line}");
        assert_eq!(source.rows[2].kind, "body");
        exact(&source, &text);
    }
}

#[test]
fn frozen_v1_through_v10_replay_exactly_and_only_explicit_reparse_upgrades() {
    let text = "合成测试150集剧本\r\n主角：林川 人物小传\r\n【基本资料】\r\n姓名：林川\r\n身份：修理员\r\n第1集\r\n1-0 日 内 工坊\r\n林川：回来😀。\r\n1-1 夜 外 门口\r\n林川：结束。\r\n";
    for version in VERSIONS {
        let old = parse_source(text, version).unwrap();
        let frozen = serde_json::to_vec(&old).unwrap();
        assert_eq!(old.scenes.len(), 1, "{version}");
        assert_eq!(old.rows[0].kind, "body");
        assert_eq!(serde_json::to_vec(&upgrade_source_dialogues(&old).unwrap()).unwrap(), frozen);
        let current = reparse_source(&old).unwrap();
        assert_eq!(current.parser_version, FORMAT_PARSER_VERSION);
        assert!(current.diagnostics.is_empty());
        assert_eq!(current.scenes.len(), 2);
        assert_eq!(serde_json::to_vec(&old).unwrap(), frozen);
        assert_eq!(serde_json::to_vec(&current).unwrap(), serde_json::to_vec(&reparse_source(&current).unwrap()).unwrap());
        exact(&old, text);
        exact(&current, text);
    }
}

#[test]
fn explicit_reparse_preserves_boundary_decisions_scene_identity_and_source() {
    let text = "主角：林川 人物小传\n【基本资料】\n姓名：林川\n第1集\n1-0 日 内 工坊\n林川：回来😀。\n1-1 夜 外 门口\n林川：结束。\n";
    let old = parse_source(text, EXPLICIT_INLINE_PARSER_VERSION).unwrap();
    let decided = apply_boundary_decisions(&old, &[
        BoundaryDecision { start: 0, end: old.rows[3].range.start, kind: "context".into(),
            episode_number: None, scene_title: None, reason: "明确人物资料".into(), decision_version: None, confirmation: None },
        BoundaryDecision { start: old.rows[4].range.start, end: old.rows[6].range.start, kind: "body".into(),
            episode_number: Some(1), scene_title: Some("人工确认开场工坊".into()), reason: "明确开场场次".into(), decision_version: None, confirmation: None },
    ]).unwrap();
    let current = reparse_source(&decided).unwrap();
    assert_eq!(current.parser_version, FORMAT_PARSER_VERSION);
    assert_eq!(serde_json::to_value(&current.boundary_decisions).unwrap(), serde_json::to_value(&decided.boundary_decisions).unwrap());
    for (before, after) in decided.rows.iter().zip(&current.rows) {
        assert_eq!((&before.id, &before.range, &before.text, &before.episode_id, &before.scene_id, &before.dialogue),
            (&after.id, &after.range, &after.text, &after.episode_id, &after.scene_id, &after.dialogue));
    }
    exact(&current, text);
}

#[test]
fn unrelated_inputs_keep_their_existing_selected_parser() {
    for text in [
        "第1集\n场景：工坊\n林川：回来。\n",
        "《合成测试》\n第1集\n场景：工坊\n林川：回来。\n",
        "第1集\n1-1 日 内 工坊\n△林川看向人物小传。\n林川：回来。\n",
    ] {
        assert_eq!(fresh_parser_version(text), prior_inline_parser_version(text));
    }
}

#[test]
fn v11_decisions_detach_only_blank_remnants_of_displaced_automatic_scenes() {
    let text = "第1集\r\n1-0 日 内 工坊\r\n林川：回来😀。\r\n \t\r\n1-1 夜 外 门口\r\n林川：继续。\r\n";
    let old = parse_source(text, EXPLICIT_INLINE_PARSER_VERSION).unwrap();
    let decided = apply_boundary_decisions(&old, &[
        BoundaryDecision { start: old.rows[1].range.start, end: old.rows[2].range.end,
            kind: "body".into(), episode_number: Some(1), scene_title: Some("人工确认工坊".into()),
            reason: "用户确认完整开场正文，未包含空白分隔行".into(), decision_version: None, confirmation: None },
    ]).unwrap();
    assert_eq!(decided.scenes.len(), 2);
    assert!(decided.rows[3].scene_id.is_none());
    let current = reparse_source(&decided).unwrap();
    assert_eq!(current.parser_version, FORMAT_PARSER_VERSION);
    assert_eq!(current.scenes.len(), 2);
    assert_eq!(current.episodes[0].scene_ids.len(), 2);
    assert_eq!(current.rows[1].scene_id, decided.rows[1].scene_id);
    assert_eq!(current.rows[2].scene_id, decided.rows[2].scene_id);
    assert_eq!(current.rows[3].episode_id, decided.rows[3].episode_id);
    assert!(current.rows[3].scene_id.is_none());
    assert_eq!(serde_json::to_value(&current.boundary_decisions).unwrap(), serde_json::to_value(&decided.boundary_decisions).unwrap());
    assert_eq!(serde_json::to_value(&current.scenes).unwrap(), serde_json::to_value(&decided.scenes).unwrap());
    assert!(current.scenes.iter().all(|scene| current.rows.iter().any(|row|
        row.scene_id.as_ref() == Some(&scene.id) && !row.text.trim().is_empty())));
    exact(&current, text);
    assert_eq!(serde_json::to_vec(&current).unwrap(), serde_json::to_vec(&reparse_source(&current).unwrap()).unwrap());
}

#[test]
fn blank_scene_cleanup_is_versioned_and_never_removes_a_remaining_heading() {
    let text = "第1集\n1-1 日 内 工坊\n林川：回来。\n\n1-2 夜 外 门口\n林川：继续。\n";
    for version in VERSIONS.into_iter().chain([FORMAT_PARSER_VERSION]) {
        let source = parse_source(text, version).unwrap();
        let decision = |start| BoundaryDecision { start, end: source.rows[2].range.end,
            kind: "body".into(), episode_number: Some(1), scene_title: Some("人工场".into()),
            reason: "精确人工范围".into(), decision_version: None, confirmation: None };
        let all_actual = apply_boundary_decisions(&source, &[decision(source.rows[1].range.start)]).unwrap();
        if version == FORMAT_PARSER_VERSION {
            assert_eq!(all_actual.scenes.len(), 2);
            assert!(all_actual.rows[3].scene_id.is_none());
        } else {
            assert_eq!(all_actual.scenes.len(), 3, "{version}");
            assert_eq!(all_actual.rows[3].scene_id, source.rows[3].scene_id);
            assert_eq!(serde_json::to_vec(&all_actual).unwrap(), serde_json::to_vec(&upgrade_source_dialogues(&all_actual).unwrap()).unwrap());
        }
        let body_only = apply_boundary_decisions(&source, &[decision(source.rows[2].range.start)]).unwrap();
        assert_eq!(body_only.scenes.len(), 3, "{version}");
        assert_eq!(body_only.rows[1].scene_id, source.rows[1].scene_id);
        assert_eq!(body_only.rows[3].scene_id, source.rows[3].scene_id);
        exact(&all_actual, text);
        exact(&body_only, text);
    }
}

#[test]
fn linear_integrity_check_rejects_row_text_utf16_and_partition_forgery() {
    let text = "主角：林川 人物小传\r\n【基本资料】\r\n姓名：林川😀\r\n第1集\r\n1-0 日 内 工坊\r\n林川：回来😀。\r\n";
    let source = prepare_source(text).unwrap();
    exact(&source, text);
    for kind in ["same_length_text", "utf16_length", "shifted_start", "reversed_row", "duplicate_id", "missing_row", "reordered_rows"] {
        let mut forged = source.clone();
        match kind {
            "same_length_text" => forged.rows[2].text = forged.rows[2].text.replace("林川", "张海"),
            "utf16_length" => forged.rows[2].text = forged.rows[2].text.replace('😀', "甲"),
            "shifted_start" => forged.rows[2].range.start += 1,
            "reversed_row" => forged.rows[2].range.end = forged.rows[2].range.start - 1,
            "duplicate_id" => forged.rows[2].id = forged.rows[1].id.clone(),
            "missing_row" => { forged.rows.remove(2); }
            "reordered_rows" => forged.rows.swap(1, 2),
            _ => unreachable!(),
        }
        let checked = std::panic::catch_unwind(|| validate_integrity(&forged));
        assert!(checked.is_ok(), "{kind} must reject without panic");
        assert!(checked.unwrap().is_err(), "{kind}");
    }
}

#[test]
fn row_local_quote_validation_rejects_reversed_outside_and_surrogate_ranges() {
    let text = "第1集\r\n1-0 日 内 工坊\r\n林川：回来😀。\r\n";
    let source = prepare_source(text).unwrap();
    for kind in ["reversed", "before_row", "after_row", "surrogate_interior", "quote_text"] {
        let mut forged = source.clone();
        let row = &mut forged.rows[2];
        let dialogue = row.dialogue.as_mut().unwrap();
        match kind {
            "reversed" => dialogue.range.end = dialogue.range.start - 1,
            "before_row" => dialogue.range.start = row.range.start - 1,
            "after_row" => dialogue.range.end = row.range.end + 1,
            "surrogate_interior" => dialogue.range.start = row.range.start + row.text.split('😀').next().unwrap().encode_utf16().count() + 1,
            "quote_text" => dialogue.quote = dialogue.quote.replace("回来", "离开"),
            _ => unreachable!(),
        }
        let checked = std::panic::catch_unwind(|| validate_integrity(&forged));
        assert!(checked.is_ok(), "{kind} must reject without panic");
        assert!(checked.unwrap().is_err(), "{kind}");
    }
}
