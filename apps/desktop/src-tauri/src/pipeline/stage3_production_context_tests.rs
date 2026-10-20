use super::*;

fn request(stage: &str, fresh: bool) -> TextRequest {
    let skill = match stage {
        "storyboardCreate" => "unit-performance-director",
        "storyboardFinish" => "episode-finishing-editor",
        _ => "episode-emotion-planner",
    };
    let mut input = json!({PROFILE_FIELD:PROFILE,"storyboardRulePackSha256":pack_sha256().unwrap(),
        "sourceRows":[{"id":"r1","text":"△甲走到镜面前，包落在地上。"}],
        "assets":[{"id":"room","variantId":"base","kind":"LOC","sourceRefs":["r1"]}],
        "facts":[{"id":"mirror","claim":"SOURCE","value":"室内有镜面","sourceRefs":["r1"]}]});
    if fresh {
        mark_input(&mut input).unwrap();
        // Isolate this method-loading contract from the independently tested
        // rhythm/source/dialogue schemas. Real new workflows retain rhythm.
        input.as_object_mut().unwrap().remove(RHYTHM_PROFILE_FIELD);
    }
    let mut rule_snapshot = snapshot(skill,&["storyboard-common"]).unwrap();
    rule_snapshot["runtimeContract"] = runtime_contract(stage,stage=="storyboardFinish").unwrap();
    TextRequest { stage:stage.into(),skill:skill.into(),model:"offline".into(),reasoning:"medium".into(),
        input,schema:json!({"type":"object","properties":{}}),rule_snapshot }
}

#[test]
fn ordinary_context_new_card_binds_original_rule_bytes_without_changing_pack_identity() {
    let previous_pack = pack_sha256().unwrap();
    let mut input = json!({});
    mark_input(&mut input).unwrap();
    assert!(production_context_enabled(&input).unwrap());
    let card = &input[PRODUCTION_CONTEXT_FIELD];
    assert_eq!(card["version"],"ordinary-production-context/v1");
    assert_eq!(card["ruleId"],"storyboard-source-context");
    assert_eq!(card["ruleSha256"],"f563acc7ef27d08e60fd596778e511d7132d4e62144d4cb75f6fee63a4a9d291");
    assert_eq!(input["storyboardRulePackSha256"],previous_pack);
    assert_eq!(rule_snapshots().unwrap().len(),22,"no new role or rewritten original rule");
}

#[test]
fn ordinary_context_old_author_and_finisher_replay_without_a_new_rule_or_footer() {
    for stage in ["storyboardCreate","storyboardFinish"] {
        let old = request(stage,false);
        let before = serde_json::to_value(&old).unwrap();
        let actual = prepare_request(old).unwrap();
        assert_eq!(serde_json::to_value(actual).unwrap(),before,"old frozen request is not upgraded by installed code");
    }
}

#[test]
fn ordinary_context_new_author_and_finisher_load_same_original_method_once() {
    let original = rule_snapshots().unwrap().into_iter().find(|rule|rule.id==SOURCE_CONTEXT_RULE).unwrap();
    for stage in ["storyboardCreate","storyboardFinish"] {
        let new = request(stage,true);
        let frozen_input = new.input.clone();
        let schema = new.schema.clone();
        let prepared = prepare_request(new).unwrap();
        assert_eq!(prepared.input,frozen_input);
        assert_eq!(prepared.schema,schema,"ordinary production does not create new facts/assets or output fields");
        let methods = prepared.rule_snapshot["methods"].as_array().unwrap();
        let source:Vec<_> = methods.iter().filter(|method|method["id"]==SOURCE_CONTEXT_RULE).collect();
        assert_eq!(source.len(),1);
        assert_eq!(source[0]["content"],original.content);
        assert_eq!(source[0]["sha256"],original.sha256);
        assert!(original.content.contains("未登记独立资产不等于不能入画"));
        let loaded:Vec<_> = prepared.rule_snapshot["creativePack"]["loadedFiles"].as_array().unwrap().iter()
            .filter(|method|method["id"]==SOURCE_CONTEXT_RULE).collect();
        assert_eq!(loaded.len(),1,"loaded-file receipt includes the actual extra method");
        let runtime = &prepared.rule_snapshot["runtimeContract"];
        assert_eq!(runtime[PRODUCTION_CONTEXT_FIELD],frozen_input[PRODUCTION_CONTEXT_FIELD]);
        assert_eq!(runtime["sha256"],digest(runtime["content"].as_str().unwrap()));
        assert!(runtime["content"].as_str().unwrap().contains("可沿主LOC引用"));
        let twice = prepare_request(prepared.clone()).unwrap();
        assert_eq!(serde_json::to_value(twice).unwrap(),serde_json::to_value(prepared).unwrap(),"reconstruction does not duplicate the card or method");
    }
}

#[test]
fn ordinary_context_inheritance_restores_only_the_saved_workflow_card() {
    let fresh = request("storyboardCreate",true).input;
    let old = request("storyboardCreate",false).input;
    let mut projected = fresh.clone();
    attach_profile(&mut projected,&old).unwrap();
    assert!(projected.get(PRODUCTION_CONTEXT_FIELD).is_none(),"fresh caller cannot upgrade old saved workflow");
    attach_profile(&mut projected,&fresh).unwrap();
    assert_eq!(projected[PRODUCTION_CONTEXT_FIELD],fresh[PRODUCTION_CONTEXT_FIELD]);
}

#[test]
fn ordinary_context_unknown_or_tampered_card_is_held_before_rule_loading() {
    for (field,value) in [
        ("version",json!("ordinary-production-context/future")),
        ("ruleId",json!("made-up-rule")),
        ("ruleVersion",json!("new-unaccepted-version")),
        ("ruleSha256",json!("changed")),
        ("scopeInstructionsSha256",json!("changed")),
    ] {
        let mut changed = request("storyboardCreate",true);
        changed.input[PRODUCTION_CONTEXT_FIELD][field] = value;
        assert!(prepare_request(changed.clone()).is_err(),"{field}");
        let mut input = changed.input;
        let before = input.clone();
        assert!(mark_input(&mut input).is_err(),"mark_input cannot overwrite an unknown existing card");
        assert_eq!(input,before);
    }
    let mut unsupported = request("storyboardCreate",true);
    unsupported.input.as_object_mut().unwrap().remove(PROFILE_FIELD);
    assert!(prepare_request(unsupported).is_err());
}

#[test]
fn ordinary_context_is_an_integrated_method_and_does_not_add_a_preparation_stage() {
    let plan = request("storyboardPlan",true);
    let methods = plan.rule_snapshot["methods"].clone();
    let prepared = prepare_request(plan).unwrap();
    assert_eq!(prepared.stage,"storyboardPlan");
    assert_eq!(prepared.skill,"episode-emotion-planner");
    assert_eq!(prepared.rule_snapshot["methods"],methods);
    assert!(prepared.rule_snapshot["runtimeContract"].get(PRODUCTION_CONTEXT_FIELD).is_none());
    for stage in ["storyboardCreate","storyboardFinish"] {
        assert_eq!(prepare_request(request(stage,true)).unwrap().stage,stage);
    }
}
