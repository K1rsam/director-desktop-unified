use super::*;

pub(crate) fn fixture() -> Value {
    let mut source = serde_json::to_value(pipeline::prepare_source(
        "第1集\n场景：屋内\n甲：来了。\n第2集\n场景：屋内\n甲：走吧。\n第3集\n场景：屋内\n△甲：回来。\n").unwrap()).unwrap();
    source["parserVersion"] = json!("source-parser/v4");
    source["rows"].as_array_mut().unwrap().last_mut().unwrap()["dialogue"] = Value::Null;
    source["version"] = json!(1);
    pipeline::validate_source(&serde_json::from_value(source.clone()).unwrap()).unwrap();
    source
}

pub(crate) fn candidate(old: &Value) -> Value {
    serde_json::to_value(pipeline::reparse_source(&serde_json::from_value(old.clone()).unwrap()).unwrap()).unwrap()
}

#[test]
fn source_migration_same_bytes_marks_dialogue_and_continuity_without_changing_rows() {
    let old=fixture();let next=candidate(&old);let p=plan(&old,&next).unwrap();
    assert_eq!(p.old.text,p.next.text);assert_eq!(p.old.length,p.next.length);
    assert!(p.episode_eligible(&p.old.episodes[0].id));
    assert!(p.episode_eligible(&p.old.episodes[1].id));
    assert!(!p.episode_eligible(&p.old.episodes[2].id));
    let row=&p.old.rows[p.old.rows.len()-1];
    assert!(p.require_unconsumed(&json!({"nested":{"sourceRefs":[row.id]}})).is_err());
    assert!(p.require_unconsumed(&json!({"sourceRows":[p.old.rows[2]]})).is_ok());
    let mut polluted=json!(p.old.rows[2]);polluted["text"]=json!("修改的句子");
    assert!(p.require_unconsumed(&json!({"sourceRows":[polluted]})).is_err());
}

#[test]
fn source_migration_rejects_invented_next_source_and_boundary_drift() {
    let old=fixture();let mut next=candidate(&old);
    next["boundaryDecisions"]=json!([{"start":0,"end":1,"kind":"context","reason":"fake","episodeNumber":null,"sceneTitle":null}]);
    assert!(plan(&old,&next).is_err());
    let mut changed=fixture();changed["text"]=json!("另一个原文");
    assert!(plan(&changed,&candidate(&old)).is_err());
}

#[test]
fn source_migration_plan_uses_domain_source_but_binds_real_historical_version() {
    let old=fixture();let next=candidate(&old);let before=plan(&old,&next).unwrap();
    let mut saved=next;saved["version"]=json!(2);
    let after=plan(&old,&saved).unwrap();
    assert_eq!(after.evidence,before.evidence);
    assert_eq!(after.evidence["oldSourceVersion"],1);
    assert_eq!(after.evidence["oldSourceSha256"],pipeline::digest(&old.to_string()));
    let mut altered=saved;altered["rows"][2]["text"]=json!("篡改正文");
    assert!(plan(&old,&altered).is_err());
}

#[test]
fn source_migration_accepts_exact_one_to_many_but_never_calls_it_unchanged() {
    let text="第1集\n场景：屋内\n乙：在。\n甲：回来。乙：等等。\n";
    let next=pipeline::prepare_source(text).unwrap();
    let mut old=next.clone();old.parser_version="source-parser/v4".into();
    let mut first=old.rows.remove(3);let second=old.rows.remove(3);
    first.text.push_str(&second.text);first.range.end=second.range.end;
    first.id=format!("row:{}:{}:{}",&old.sha256[..12],first.range.start,first.range.end);
    let dialogue=first.dialogue.as_mut().unwrap();
    dialogue.id=format!("dialogue:{}",first.id);dialogue.range.end=second.range.end-1;
    dialogue.quote="回来。乙：等等。".into();
    dialogue.spoken_segments=vec![pipeline::SourceSegment{text:dialogue.quote.clone(),range:dialogue.range.clone()}];
    let a=next.rows[3].id.clone();let b=next.rows[4].id.clone();
    for refs in old.episodes.iter_mut().map(|e|&mut e.row_ids).chain(old.scenes.iter_mut().map(|s|&mut s.row_ids)) {
        refs.retain(|id|id!=&b);for id in refs {if id==&a {*id=first.id.clone();}}
    }
    old.rows.push(first.clone());pipeline::validate_source(&old).unwrap();
    let mut old=serde_json::to_value(old).unwrap();old["version"]=json!(1);
    let p=plan(&old,&serde_json::to_value(next).unwrap()).unwrap();
    let mapping=p.evidence["rowMapping"].as_array().unwrap().iter().find(|m|m["oldRowId"]==first.id).unwrap();
    assert_eq!(mapping["newRowIds"].as_array().unwrap().len(),2);
    assert_eq!(mapping["interpretationUnchanged"],false);
}
