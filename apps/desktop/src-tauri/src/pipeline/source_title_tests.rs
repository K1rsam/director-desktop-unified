use super::*;

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
fn explicit_cover_title_and_edition_are_context_without_changing_source() {
    for edition in ["80集短剧漫剧剧本", "全80集漫剧剧本", "共八十集短剧剧本", "10集剧本"] {
        let text = format!("\r\n《合成测试，海边归来😀》\r\n\r\n{edition}\r\n第一集 归来\r\n场景：码头\r\n林川：我回来了😀。\r\n第二集\r\n场景：屋内\r\n林川：继续。\r\n");
        let old = parse_source(&text, JOINT_SPEAKER_PARSER_VERSION).unwrap();
        let source = prepare_source(&text).unwrap();
        assert_eq!(source.parser_version, TITLE_METADATA_PARSER_VERSION);
        assert!(source.diagnostics.is_empty());
        assert_eq!(source.rows[1].kind, "context");
        assert_eq!(source.rows[3].kind, "context");
        for row in [&source.rows[1], &source.rows[3]] {
            assert!(row.episode_id.is_none() && row.scene_id.is_none() && row.dialogue.is_none());
        }
        assert_eq!(serde_json::to_value(&old.episodes).unwrap(), serde_json::to_value(&source.episodes).unwrap());
        assert_eq!(serde_json::to_value(&old.scenes).unwrap(), serde_json::to_value(&source.scenes).unwrap());
        for (before, after) in old.rows.iter().zip(&source.rows) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.range, after.range);
            assert_eq!(before.text, after.text);
            assert_eq!(before.number, after.number);
        }
        exact(&source, &text);
    }
}

#[test]
fn standalone_title_before_explicit_episode_needs_no_edition() {
    let text = "《合成测试》\n第一集\n场景：码头\n林川：回来。\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.parser_version, TITLE_METADATA_PARSER_VERSION);
    assert!(source.diagnostics.is_empty());
    assert_eq!(source.rows[0].kind, "context");
    exact(&source, text);
}

#[test]
fn unknown_story_and_mixed_prefaces_remain_blocking() {
    for prefix in [
        "《合成测试》\n林川推门走入。\n",
        "《合成测试》\n80集短剧漫剧剧本\n林川：我来了。\n",
        "《合成测试》他推门。\n80集短剧漫剧剧本\n",
        "《合成测试》\n80集短剧漫剧剧本，林川是主角。\n",
        "《合成测试》\n0集漫剧剧本\n",
        "合成测试\n80集短剧漫剧剧本\n",
        "80集短剧漫剧剧本\n",
        "林川看到《合成测试》\n",
        "《合成测试》\n制作说明：剧情如下。\n",
    ] {
        let text = format!("{prefix}第一集\n场景：码头\n林川：回来。\n");
        let source = prepare_source(&text).unwrap();
        assert_ne!(source.parser_version, TITLE_METADATA_PARSER_VERSION, "{prefix}");
        assert!(source.diagnostics.iter().any(|diagnostic| diagnostic.blocking && diagnostic.code == "BOUNDARY_UNCONFIRMED"), "{prefix}");
        assert!(source.rows[..prefix.lines().count()].iter().all(|row| row.kind == "body"), "{prefix}");
        exact(&source, &text);
    }
}

#[test]
fn title_and_edition_inside_episode_remain_body() {
    let text = "第一集\n场景：码头\n《合成测试》\n80集短剧漫剧剧本\n林川：这是书名。\n";
    let source = prepare_source(text).unwrap();
    assert_ne!(source.parser_version, TITLE_METADATA_PARSER_VERSION);
    assert!(source.rows[2..].iter().all(|row| row.kind == "body"));
    exact(&source, text);
}

#[test]
fn old_parser_sources_replay_unchanged_and_upgrade_only_on_explicit_reparse() {
    let text = "《合成测试😀》\r\n80集短剧漫剧剧本\r\n第一集\r\n场景：码头\r\n林川：回来。\r\n";
    for version in [LEGACY_PARSER_VERSION, CONTEXT_PARSER_VERSION, CURRENT_PARSER_VERSION,
        STRUCTURAL_PARSER_VERSION, DISPLAY_PARSER_VERSION, FIDELITY_PARSER_VERSION,
        MULTILINE_DISPLAY_PARSER_VERSION, JOINT_SPEAKER_PARSER_VERSION] {
        let old = parse_source(text, version).unwrap();
        let frozen = serde_json::to_vec(&old).unwrap();
        assert!(old.diagnostics.iter().any(|diagnostic| diagnostic.blocking));
        assert_eq!(serde_json::to_vec(&upgrade_source_dialogues(&old).unwrap()).unwrap(), frozen);
        let new = reparse_source(&old).unwrap();
        assert_eq!(new.parser_version, TITLE_METADATA_PARSER_VERSION);
        assert!(new.diagnostics.is_empty());
        assert_eq!(new.text, old.text);
        assert_eq!(new.sha256, old.sha256);
        assert_eq!(serde_json::to_vec(&old).unwrap(), frozen);
        exact(&old, text);
        exact(&new, text);
        assert_eq!(serde_json::to_vec(&new).unwrap(), serde_json::to_vec(&reparse_source(&new).unwrap()).unwrap());
        let mut forged_old = old.clone();
        forged_old.rows[0].kind = "context".into();
        forged_old.rows[1].kind = "context".into();
        forged_old.diagnostics.clear();
        assert!(validate_source(&forged_old).is_err());
    }
}

#[test]
fn title_version_inherits_joint_speech_multiline_display_and_fidelity() {
    let text = "《合成测试》\n80集短剧漫剧剧本\n第一集\n场景：码头\n人物：林川、周航\n林川&周航OS：回来😀。\n【任务一：寻找船只\n奖励一：安全到达】\n系统VO：【准备出发。】\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.parser_version, TITLE_METADATA_PARSER_VERSION);
    assert!(source.diagnostics.is_empty());
    assert_eq!(dialogue_speaker_names(source.rows[5].dialogue.as_ref().unwrap()), vec!["林川", "周航"]);
    assert_eq!(source.rows[5].dialogue.as_ref().unwrap().channel, "OS");
    assert!(source.rows[6].dialogue.is_none() && source.rows[7].dialogue.is_none());
    assert_eq!(spoken_text(source.rows[8].dialogue.as_ref().unwrap()), "【准备出发。】");
    exact(&source, text);
}

#[test]
fn title_metadata_does_not_swallow_unscoped_story_inside_episode() {
    let text = "《合成测试》\n80集短剧漫剧剧本\n第一集\n林川推门走入。\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.parser_version, TITLE_METADATA_PARSER_VERSION);
    assert_eq!(source.rows[3].kind, "body");
    assert!(source.diagnostics.iter().any(|diagnostic| diagnostic.blocking && diagnostic.code == "SCENE_BOUNDARY_UNCONFIRMED"));
    exact(&source, text);
}
