use super::*;

fn exact(source: &LockedSource, text: &str) {
    assert_eq!(source.text, text);
    assert_eq!(source.sha256, digest(text));
    assert_eq!(source.length, text.encode_utf16().count());
    assert_eq!(source.rows.iter().map(|r| r.text.as_str()).collect::<String>(), text);
    for row in &source.rows {
        assert_eq!(slice_utf16(text, &row.range).unwrap(), row.text);
        if let Some(q) = &row.dialogue {
            assert_eq!(slice_utf16(text, &q.range).unwrap(), q.quote);
            for part in q.spoken_segments.iter().chain(&q.performance_segments) {
                assert_eq!(slice_utf16(text, &part.range).unwrap(), part.text);
            }
        }
    }
    validate_source(source).unwrap();
}

#[test]
fn explicit_joint_labels_keep_exact_quote_performance_and_utf16() {
    for separator in ["&", "＆", "、", "和", "与", " & ", " 与 "] {
        let group = format!("林川{separator}周航");
        let text = format!("第一集😀\r\n场景：码头\r\n人物：林川、周航\r\n  {group}：（二人惊讶对视）真的😀来了！！！！\r\n");
        let source = prepare_source(&text).unwrap();
        let q = source.rows[3].dialogue.as_ref().unwrap();
        assert_eq!(q.speaker, group);
        assert_eq!(dialogue_speaker_names(q), vec!["林川", "周航"]);
        for member in &q.speakers { assert_eq!(slice_utf16(&text, &member.range).unwrap(), member.text); }
        assert_eq!(source.parser_version, JOINT_SPEAKER_PARSER_VERSION);
        assert_eq!(q.channel, "现场");
        assert_eq!(q.quote, "（二人惊讶对视）真的😀来了！！！！");
        assert_eq!(spoken_text(q), "真的😀来了！！！！");
        assert_eq!(q.performance_segments.len(), 1);
        assert_eq!(q.performance_segments[0].text, "（二人惊讶对视）");
        assert_eq!(q.spoken_segments[0].range.start, text[..text.find("真的").unwrap()].encode_utf16().count());
        assert_eq!(q.range.start, text[..text.find("（二人").unwrap()].encode_utf16().count());
        exact(&source, &text);
    }
}

#[test]
fn joint_speakers_use_explicit_cast_evidence_and_whole_group_voice_cues() {
    for cue in [" OS", "os", "（VO）", " [O.S.]", "（画外）"] {
        let text = format!("第一集\n场景：门口\n人物：林川、周航\n林川：我在。\n周航：听见了。\n林川&周航{cue}：原话（抬手）后半句。\n");
        let source = prepare_source(&text).unwrap();
        let q = source.rows[5].dialogue.as_ref().unwrap();
        assert_eq!(q.speaker, "林川&周航");
        assert_eq!(q.channel, if cue.contains("VO") { "VO" } else if cue.contains("画外") { "画外" } else { "OS" });
        assert_eq!(spoken_text(q), "原话后半句。");
        exact(&source, &text);
    }
}

#[test]
fn narrative_and_unproved_joint_heads_are_not_new_dialogue() {
    for line in [
        "△林川&周航：二人向门外跑去。",
        "林川&周航走向门口：四周无人。",
        "林川和周航走向门口：四周无人。",
        "林川与周航喊道：停下。",
        "林川&陌生名称：不要猜测角色。",
        "林川&：缺少另一位。",
        "林川&&周航：重复连接符。",
        "林川&周航，转身：后面是叙述。",
        "旁注：林川&周航：这里是注释。",
    ] {
        let text = format!("第一集\n场景：门口\n人物：林川、周航\n林川&周航：明确联合原话。\n{line}\n");
        let source = prepare_source(&text).unwrap();
        assert_eq!(source.parser_version, JOINT_SPEAKER_PARSER_VERSION);
        assert!(source.rows[4].dialogue.is_none(), "{line}");
        exact(&source, &text);
    }
    let text = "第一集\n场景：门口\n人物：林川、周航、林和明\n林川&周航：明确联合原话。\n林和明：我的名字不能拆开。\n林川：他和周航：是原话中的标点。\n";
    let source = prepare_source(text).unwrap();
    assert_eq!(source.rows[4].dialogue.as_ref().unwrap().speaker, "林和明");
    assert_eq!(source.rows[5].dialogue.as_ref().unwrap().quote, "他和周航：是原话中的标点。");
    exact(&source, text);
}

#[test]
fn old_sources_are_frozen_and_only_explicit_reparse_adopts_joint_metadata() {
    let text = "第一集\r\n场景：门口\r\n人物：林川、周航\r\n林川&周航：（惊讶）原话！！！！\r\n";
    for version in [LEGACY_PARSER_VERSION, CONTEXT_PARSER_VERSION, CURRENT_PARSER_VERSION, STRUCTURAL_PARSER_VERSION, DISPLAY_PARSER_VERSION, FIDELITY_PARSER_VERSION, MULTILINE_DISPLAY_PARSER_VERSION] {
        let old = parse_source(text, version).unwrap();
        assert!(old.rows[3].dialogue.is_none());
        let old_bytes = serde_json::to_vec(&old).unwrap();
        assert_eq!(serde_json::to_vec(&upgrade_source_dialogues(&old).unwrap()).unwrap(), old_bytes);
        let new = reparse_source(&old).unwrap();
        assert_eq!(new.parser_version, JOINT_SPEAKER_PARSER_VERSION);
        assert_eq!(new.rows[3].dialogue.as_ref().unwrap().speaker, "林川&周航");
        assert_eq!(new.sha256, old.sha256);
        assert_eq!(new.rows.iter().map(|r| (&r.id, &r.range, &r.text)).collect::<Vec<_>>(), old.rows.iter().map(|r| (&r.id, &r.range, &r.text)).collect::<Vec<_>>());
        let mut forged = old.clone(); forged.rows[3].dialogue = new.rows[3].dialogue.clone();
        assert!(validate_source(&forged).is_err());
        assert_eq!(serde_json::to_vec(&old).unwrap(), old_bytes);
        assert_eq!(serde_json::to_vec(&reparse_source(&new).unwrap()).unwrap(), serde_json::to_vec(&new).unwrap());
        exact(&new, text);
    }
}

#[test]
fn unrelated_sources_keep_their_parser_and_display_blocks_remain_nonspoken() {
    assert_eq!(prepare_source("第一集\n场景：门口\n林川：别动。\n").unwrap().parser_version, CURRENT_PARSER_VERSION);
    let text = "第一集\n场景：门口\n人物：林川、周航\n林川&周航：看屏幕。\n【情报1：原样显示\n情报2：保持不发声】\n";
    let source = prepare_source(text).unwrap();
    assert!(source.rows[4].dialogue.is_none()); assert!(source.rows[5].dialogue.is_none());
    exact(&source, text);
}

#[test]
fn evidenced_latin_names_do_not_consume_attached_voice_cue_as_name() {
    let text = "第一集\n场景：门口\n人物：Alice、Bob\nAlice&BobOS：原话。\n";
    let source = prepare_source(text).unwrap();
    let q = source.rows[3].dialogue.as_ref().unwrap();
    assert_eq!(q.speaker, "Alice&Bob");
    assert_eq!(q.channel, "OS");
    exact(&source, text);
}
#[test]
fn joint_members_require_explicit_cast_and_preserve_conjunction_in_real_names() {
    let no_cast = prepare_source("第一集\n场景：门口\n林川：在。\n周航：在。\n林川&周航：同声。\n").unwrap();
    assert!(no_cast.rows[4].dialogue.is_none());
    let text="第一集\n场景：门口\n人物：林、和明、林和明、周航\n林和明与周航：同声。\n林和明：我的名字。\n";
    let source=prepare_source(text).unwrap();
    assert_eq!(dialogue_speaker_names(source.rows[3].dialogue.as_ref().unwrap()),vec!["林和明","周航"]);
    assert!(source.rows[4].dialogue.as_ref().unwrap().speakers.is_empty());
    let saved=serde_json::to_vec(&source).unwrap();
    let reopened:LockedSource=serde_json::from_slice(&saved).unwrap();validate_source(&reopened).unwrap();
    assert_eq!(serde_json::to_vec(&upgrade_source_dialogues(&reopened).unwrap()).unwrap(),saved);
    let mut forged=source.clone();forged.rows[3].dialogue.as_mut().unwrap().speakers[0].range.start+=1;
    assert!(validate_source(&forged).is_err());
    let mut forged=source.clone();forged.rows[3].dialogue.as_mut().unwrap().speakers.reverse();
    assert!(validate_source(&forged).is_err());
    exact(&source,text);
}

#[test]
#[ignore = "requires explicit private source JSON copy and output proof path; read-only source, no store or generation"]
fn joint_source_private_copy_preserves_old_frozen_source_and_reports_only_metadata() {
    let input=std::env::var("DIRECTOR_JOINT_SOURCE_COPY").unwrap();
    let out=std::env::var("DIRECTOR_JOINT_SOURCE_PROOF").unwrap();
    let bytes=std::fs::read(input).unwrap();let old:LockedSource=serde_json::from_slice(&bytes).unwrap();
    validate_source(&old).unwrap();
    let canonical=serde_json::to_vec(&old).unwrap();
    assert_eq!(serde_json::to_vec(&upgrade_source_dialogues(&old).unwrap()).unwrap(),canonical);
    let next=reparse_source(&old).unwrap();validate_source(&next).unwrap();
    assert_eq!(old.text,next.text);assert_eq!(old.sha256,next.sha256);assert_eq!(old.length,next.length);
    assert_eq!(old.rows.iter().map(|r|(&r.id,&r.range,&r.text,&r.episode_id,&r.scene_id)).collect::<Vec<_>>(),next.rows.iter().map(|r|(&r.id,&r.range,&r.text,&r.episode_id,&r.scene_id)).collect::<Vec<_>>());
    let changed:Vec<_>=old.rows.iter().zip(&next.rows).filter(|(a,b)|a.dialogue!=b.dialogue).map(|(a,b)|serde_json::json!({"row":a.number,"wasDialogue":a.dialogue.is_some(),"nowDialogue":b.dialogue.is_some(),"members":b.dialogue.as_ref().map(|q|q.speakers.len()),"memberSourceRangesExact":b.dialogue.as_ref().is_some_and(|q|q.speakers.iter().all(|m|slice_utf16(&next.text,&m.range).as_deref()==Ok(m.text.as_str())))})).collect();
    let proof=serde_json::json!({"sourceSha256":old.sha256,"oldParser":old.parser_version,"newParser":next.parser_version,"oldReplayByteEqual":true,"sourceAndAllRowIdentitiesEqual":true,"changedRows":changed,"productionDatabaseWrites":0,"generationCalls":0});
    std::fs::write(out,serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
}

#[test]
fn joint_member_name_with_channel_suffix_is_not_rewritten() {
    let text="第一集\n场景：门口\n人物：林川、周航OS\n林川&周航OS：原话。\n";
    let source=prepare_source(text).unwrap();let q=source.rows[3].dialogue.as_ref().unwrap();
    assert_eq!(q.speaker,"林川&周航OS");assert_eq!(q.channel,"现场");
    assert_eq!(dialogue_speaker_names(q),vec!["林川","周航OS"]);
    exact(&source,text);
}
