use super::*;

fn assert_original(source: &LockedSource, text: &str) {
    assert_eq!(source.text.as_bytes(), text.as_bytes());
    assert_eq!(source.sha256, digest(text));
    assert_eq!(source.length, text.encode_utf16().count());
    assert_eq!(
        source
            .rows
            .iter()
            .map(|row| row.text.as_str())
            .collect::<String>(),
        text
    );
    for row in &source.rows {
        assert_eq!(slice_utf16(text, &row.range).unwrap(), row.text);
        if let Some(dialogue) = &row.dialogue {
            assert_eq!(slice_utf16(text, &dialogue.range).unwrap(), dialogue.quote);
            for part in dialogue
                .spoken_segments
                .iter()
                .chain(&dialogue.performance_segments)
            {
                assert_eq!(slice_utf16(text, &part.range).unwrap(), part.text);
            }
        }
    }
    validate_source(source).unwrap();
}

fn row<'a>(source: &'a LockedSource, prefix: &str) -> &'a SourceRow {
    source
        .rows
        .iter()
        .find(|row| row.text.starts_with(prefix))
        .unwrap()
}

#[test]
fn complete_multiline_display_fields_are_not_speakers_and_utf16_stays_exact() {
    let text = "第一集\r\n场景：测试室\r\n人物：甲、乙\r\n甲：看屏幕😀。\r\n【任务1：条目e\u{301}。\r\n等级：甲级】\r\n【消息2:条目二。\r\n分数：四】\r\n【选项3：第三项。\r\n备注字段：空】\r\n乙OS：（抬头）收到。\r\n等级：同名角色在框外说话。\r\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.parser_version, MULTILINE_DISPLAY_PARSER_VERSION);
    assert!(source.diagnostics.is_empty());
    for prefix in [
        "【任务1",
        "等级：甲级",
        "【消息2",
        "分数：",
        "【选项3",
        "备注字段：",
    ] {
        let field = row(&source, prefix);
        assert_eq!(field.kind, "body");
        assert!(field.dialogue.is_none(), "{prefix}");
        assert_eq!(field.scene_id, source.rows[3].scene_id);
    }
    let spoken = source
        .rows
        .iter()
        .filter_map(|row| row.dialogue.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        spoken
            .iter()
            .map(|dialogue| (dialogue.speaker.as_str(), dialogue.channel.as_str()))
            .collect::<Vec<_>>(),
        [("甲", "现场"), ("乙", "OS"), ("等级", "现场")]
    );
    assert_eq!(spoken_text(spoken[1]), "收到。");
    assert_original(&source, text);
}

#[test]
fn display_closes_locally_and_recovers_after_an_incomplete_enclosure() {
    let text = "第一集\n场景：测试室\n人物：甲\n【消息1：第一项\n等级：A】\n甲：关闭后说话。\n等级：框外说话。\n【消息2：没有闭合\n甲：这句不能被吞。\n等级：仍是原对白。\n【消息3：另一个完整块\n等级：C】\n甲：最后一句。\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.parser_version, MULTILINE_DISPLAY_PARSER_VERSION);
    for prefix in ["等级：A", "等级：C"] {
        assert!(row(&source, prefix).dialogue.is_none());
    }
    for prefix in [
        "甲：关闭",
        "等级：框外",
        "甲：这句",
        "等级：仍是",
        "甲：最后",
    ] {
        assert!(row(&source, prefix).dialogue.is_some(), "{prefix}");
    }
    assert_original(&source, text);
}

#[test]
fn malformed_cross_scene_or_unclosed_blocks_do_not_exempt_dialogue() {
    for body in [
        "【任务1：未闭合\n等级：A\n",
        "【任务1：括号不匹配（\n等级：A】\n",
        "【任务1：前场开始\n场景：另一间\n等级：A】\n",
        "【任务1：前集开始\n第二集\n场景：另一间\n等级：A】\n",
        "【任务1：前段开始\n【人物关系】\n资料：A】\n第二集\n场景：另一间\n等级：角色原话。\n",
        "【任务1：结束后另有正文\n等级：A】 甲：原话仍在。\n甲：继续。\n",
    ] {
        let text = format!("第一集\n场景：测试室\n{body}");
        let source = prepare_source(&text).unwrap();
        assert_ne!(
            source.parser_version, MULTILINE_DISPLAY_PARSER_VERSION,
            "{body}"
        );
        assert!(row(&source, "等级：").dialogue.is_some(), "{body}");
        assert_original(&source, &text);
    }
}

#[test]
fn nested_wrappers_literal_closing_marks_and_blank_lines_keep_enclosure_boundaries() {
    let text = "第一集\r\n场景：测试室\r\n  【提示一：引号“】”和字母e\u{301}😀\r\n\r\n附注：【内层（说明）】\r\n分数：A】  \r\n甲：正常恢复。\r\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.parser_version, MULTILINE_DISPLAY_PARSER_VERSION);
    assert!(row(&source, "附注：").dialogue.is_none());
    assert!(row(&source, "分数：").dialogue.is_none());
    assert!(row(&source, "甲：").dialogue.is_some());
    assert_original(&source, text);
}

#[test]
fn bracketed_speech_named_cast_and_explicit_delivery_cues_remain_speech() {
    for body in [
        "系统VO：【原文😀。】\n甲（低声）：原话（抬手）。\n",
        "甲：他说“【消息1：保留这些字】”。\n",
        "【甲：括号中的说话\n乙：原话。】\n",
        "【任务1：前置说明\n甲（大喊）：原话不能删。\n等级：A】\n",
        "【消息1：前置说明\n系统VO：【确实发声。】\n等级：B】\n",
        "【消息1：前置说明\n甲：已登记人物原话。\n等级：C】\n",
        "人物：任务1（男）\n【任务1：登记角色的包裹原话\n等级：D】\n",
    ] {
        let text = format!("第一集\n场景：测试室\n人物：甲、乙\n{body}");
        let before = parse_source(&text, FIDELITY_PARSER_VERSION).unwrap();
        let source = prepare_source(&text).unwrap();
        assert_ne!(
            source.parser_version, MULTILINE_DISPLAY_PARSER_VERSION,
            "{body}"
        );
        assert_eq!(
            source
                .rows
                .iter()
                .map(|row| &row.dialogue)
                .collect::<Vec<_>>(),
            before
                .rows
                .iter()
                .map(|row| &row.dialogue)
                .collect::<Vec<_>>(),
            "{body}"
        );
        assert_original(&source, &text);
    }
}

#[test]
fn old_v1_through_v6_remain_frozen_until_explicit_multiline_reparse() {
    let text = "第一集\r\n场景：测试室\r\n【消息1：示例😀\r\n等级：A】\r\n甲：继续。\r\n";
    for version in [
        LEGACY_PARSER_VERSION,
        CONTEXT_PARSER_VERSION,
        CURRENT_PARSER_VERSION,
        STRUCTURAL_PARSER_VERSION,
        DISPLAY_PARSER_VERSION,
        FIDELITY_PARSER_VERSION,
    ] {
        let old = parse_source(text, version).unwrap();
        let saved = serde_json::to_vec(&old).unwrap();
        assert!(old.rows[3].dialogue.is_some());
        assert_eq!(
            serde_json::to_vec(&upgrade_source_dialogues(&old).unwrap()).unwrap(),
            saved
        );
        let reopened: LockedSource = serde_json::from_slice(&saved).unwrap();
        validate_source(&reopened).unwrap();
        let recognized = reparse_source(&reopened).unwrap();
        assert_eq!(recognized.parser_version, MULTILINE_DISPLAY_PARSER_VERSION);
        assert!(recognized.rows[3].dialogue.is_none());
        assert_eq!(
            old.rows
                .iter()
                .map(|row| (
                    &row.id,
                    &row.range,
                    &row.text,
                    &row.kind,
                    &row.episode_id,
                    &row.scene_id
                ))
                .collect::<Vec<_>>(),
            recognized
                .rows
                .iter()
                .map(|row| (
                    &row.id,
                    &row.range,
                    &row.text,
                    &row.kind,
                    &row.episode_id,
                    &row.scene_id
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            serde_json::to_value(&recognized.episodes).unwrap(),
            serde_json::to_value(&old.episodes).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&recognized.scenes).unwrap(),
            serde_json::to_value(&old.scenes).unwrap()
        );
        assert_eq!(serde_json::to_vec(&old).unwrap(), saved);
        assert_eq!(
            serde_json::to_vec(&reparse_source(&recognized).unwrap()).unwrap(),
            serde_json::to_vec(&recognized).unwrap()
        );
        assert_original(&recognized, text);
    }
}

#[test]
fn display_exemption_cannot_hide_real_speech_or_forge_old_metadata() {
    let text = "第一集\n场景：测试室\n【目标1：示例\n等级：A】\n甲：原话。\n";
    let source = prepare_source(text).unwrap();
    let old = parse_source(text, FIDELITY_PARSER_VERSION).unwrap();
    let mut fake = source.clone();
    fake.rows[3].dialogue = old.rows[3].dialogue.clone();
    assert!(validate_source(&fake).is_err());
    let mut silenced = source.clone();
    silenced.rows[4].dialogue = None;
    assert!(validate_source(&silenced).is_err());
    let mut relabelled_old = old.clone();
    relabelled_old.rows[3].dialogue = None;
    assert!(validate_source(&relabelled_old).is_err());
    assert_original(&source, text);
}

#[test]
fn explicit_boundary_decisions_survive_multiline_reparse_and_save_reopen() {
    let text = "【任务1：示例\n等级：A】\n甲：原话。\n";
    let old = parse_source(text, FIDELITY_PARSER_VERSION).unwrap();
    let old = apply_boundary_decisions(
        &old,
        &[BoundaryDecision {
            start: 0,
            end: old.length,
            kind: "body".into(),
            episode_number: Some(1),
            scene_title: Some("已确认测试室".into()),
            reason: "用户明确确认原坐标".into(),
            decision_version: None,
            confirmation: None,
        }],
    )
    .unwrap();
    let recognized = reparse_source(&old).unwrap();
    assert_eq!(recognized.parser_version, MULTILINE_DISPLAY_PARSER_VERSION);
    assert!(recognized.rows[1].dialogue.is_none());
    assert!(recognized.rows[2].dialogue.is_some());
    assert_eq!(
        serde_json::to_value(&recognized.boundary_decisions).unwrap(),
        serde_json::to_value(&old.boundary_decisions).unwrap()
    );
    assert_eq!(recognized.rows[1].scene_id, old.rows[1].scene_id);
    let saved = serde_json::to_vec(&recognized).unwrap();
    let reopened: LockedSource = serde_json::from_slice(&saved).unwrap();
    assert_eq!(
        serde_json::to_vec(&upgrade_source_dialogues(&reopened).unwrap()).unwrap(),
        saved
    );
    assert_original(&reopened, text);
}
