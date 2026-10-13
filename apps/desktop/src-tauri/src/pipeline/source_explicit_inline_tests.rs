use super::*;

fn exact(source: &LockedSource, text: &str) {
    assert_eq!(source.text, text);
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

#[test]
fn explicit_inline_speakers_split_exact_utf16_without_inserting_newline() {
    let text =
        "第一集\r\n场景：码头\r\n人物：甲、乙、丙\r\n甲：回来😀！乙：等我。 丙（低声）：走。\r\n";
    let old = parse_source(text, TITLE_METADATA_PARSER_VERSION).unwrap();
    let new = prepare_source(text).unwrap();
    assert_eq!(new.parser_version, EXPLICIT_INLINE_PARSER_VERSION);
    assert_eq!(new.rows.len(), old.rows.len() + 2);
    let dialogue: Vec<_> = new
        .rows
        .iter()
        .filter_map(|row| row.dialogue.as_ref())
        .collect();
    assert_eq!(
        dialogue
            .iter()
            .map(|d| d.speaker.as_str())
            .collect::<Vec<_>>(),
        vec!["甲", "乙", "丙"]
    );
    assert_eq!(
        dialogue
            .iter()
            .map(|d| d.quote.as_str())
            .collect::<Vec<_>>(),
        vec!["回来😀！", "等我。", "走。"]
    );
    assert!(new.rows[3..].iter().all(|row| row.number == 4));
    assert!(new.diagnostics.is_empty());
    assert_eq!(new.episodes[0].id, old.episodes[0].id);
    assert_eq!(new.scenes[0].id, old.scenes[0].id);
    exact(&new, text);
}

#[test]
fn marked_known_speaker_is_dialogue_but_narrative_and_missing_colon_are_not_guessed() {
    let text="第一集\n场景：屋内\n人物：甲、乙\n△甲：那是什么？\n甲早让你别用你不听，现在坏了。\n△甲看向乙：他没有开口。\n△物件：一盏灯。\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.parser_version, EXPLICIT_INLINE_PARSER_VERSION);
    assert_eq!(source.rows[3].dialogue.as_ref().unwrap().speaker, "甲");
    assert!(source.rows[4..].iter().all(|row| row.dialogue.is_none()));
    exact(&source, text);
}

#[test]
fn quoted_reported_speech_brackets_and_terms_never_split() {
    for text in [
        "甲：他说“回来！乙：我不回。”我不信。",
        "甲：他说\"回来！乙：我不回。\"我不信。",
        "甲：他说‘回来！乙：我不回。’我不信。",
        "甲：他说「回来！乙：我不回。」我不信。",
        "甲：（他大喊！乙：这一段是动作说明。）我听见了。",
        "甲：名称：树。功效：遮阴。比例：1:2。",
        "甲：他说【提醒！乙：别走。】的纸条还在。",
    ] {
        let text = format!("第一集\n场景：屋内\n人物：甲、乙\n△甲：开始。\n{text}\n");
        let source = prepare_source(&text).unwrap();
        assert_eq!(source.rows.len(), 5);
        assert_eq!(source.rows[4].dialogue.as_ref().unwrap().speaker, "甲");
        assert!(source.diagnostics.is_empty(), "{:?}", source.diagnostics);
        exact(&source, &text);
    }
}

#[test]
fn only_existing_exact_labels_and_sentence_boundaries_can_split() {
    let text="第一集\n场景：屋内\n人物：甲、乙\n△甲：开始。\n甲：回来！陌生人：别走。\n甲：我记得乙：曾这样说过。\n甲：警告！名称：说明。\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.rows.len(), 7);
    assert!(source.rows[4..]
        .iter()
        .all(|row| row.dialogue.as_ref().unwrap().speaker == "甲"));
    exact(&source, text);
}

#[test]
fn v9_frozen_rows_and_upgrade_stay_unchanged_until_explicit_reparse() {
    let text = "第一集\n场景：屋内\n人物：甲、乙\n甲：回来！乙：等我。\n△甲：开始。\n";
    let old = parse_source(text, TITLE_METADATA_PARSER_VERSION).unwrap();
    let bytes = serde_json::to_vec(&old).unwrap();
    assert_eq!(
        old.rows[3].dialogue.as_ref().unwrap().quote,
        "回来！乙：等我。"
    );
    assert!(old.rows[4].dialogue.is_none());
    validate_source(&old).unwrap();
    assert_eq!(
        serde_json::to_vec(&upgrade_source_dialogues(&old).unwrap()).unwrap(),
        bytes
    );
    let new = reparse_source(&old).unwrap();
    assert_eq!(new.parser_version, EXPLICIT_INLINE_PARSER_VERSION);
    assert_eq!(serde_json::to_vec(&old).unwrap(), bytes);
    exact(&new, text);
    assert_eq!(
        serde_json::to_vec(&reparse_source(&new).unwrap()).unwrap(),
        serde_json::to_vec(&new).unwrap()
    );
    let mut forged = old.clone();
    forged.rows[4].dialogue = new.rows.last().unwrap().dialogue.clone();
    assert!(validate_source(&forged).is_err());
}

#[test]
fn confirmed_boundary_decision_is_not_silently_rebound_to_new_row_ids() {
    let text = "第一集\n场景：屋内\n人物：甲、乙\n甲：回来！乙：等我。\n";
    let old = parse_source(text, TITLE_METADATA_PARSER_VERSION).unwrap();
    let body = &old.rows[3];
    let decided = apply_boundary_decisions(
        &old,
        &[BoundaryDecision {
            start: body.range.start,
            end: body.range.end,
            kind: "body".into(),
            episode_number: Some(1),
            scene_title: Some("屋内".into()),
            reason: "用户已确认原行范围".into(),
            decision_version: None,
            confirmation: None,
        }],
    )
    .unwrap();
    let before = serde_json::to_vec(&decided).unwrap();
    assert!(reparse_source(&decided).unwrap_err().contains("原范围决定"));
    assert_eq!(serde_json::to_vec(&decided).unwrap(), before);
}

#[test]
fn unrelated_import_does_not_upgrade_and_display_enclosure_stays_non_spoken() {
    let text = "第一集\n场景：屋内\n人物：甲、乙\n甲：走吧。\n乙：好的。\n";
    let source = prepare_source(text).unwrap();
    assert_ne!(source.parser_version, EXPLICIT_INLINE_PARSER_VERSION);
    exact(&source, text);
    let text =
        "第一集\n场景：屋内\n人物：甲、乙\n△甲：开始。\n屏幕内容：\n【甲：走！乙：别走。】\n";
    let source = prepare_source(text).unwrap();
    assert!(source.rows.last().unwrap().dialogue.is_none());
    exact(&source, text);
}
