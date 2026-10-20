#[tokio::test]
async fn split_scene_facts_reach_all_three_roles_without_upgrading_old_requests() {
    use super::super::storyboard_scene_facts as scene_facts;
    let source = prepare_source("第七集\n7-1\n测试湖 夜 / 外\n角色甲OS：先取甲。\n").unwrap();
    let mut old = input(&source);
    creative::mark_input(&mut old, true);
    let old_bytes = serde_json::to_vec(&old).unwrap();
    let executor = Recorded::default();
    let old_result = run_storyboard(&old, &executor, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(
        old_result["status"], "completed",
        "{}",
        old_result["blocked"]
    );
    let old_calls = executor.calls.lock().await.clone();
    assert_eq!(old_calls.len(), 3);
    let mut current = old.clone();
    freeze_scene_input_profile(&mut current).unwrap();
    let result = run_storyboard(&current, &executor, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(result["status"], "completed", "{}", result["blocked"]);
    let calls = executor.calls.lock().await.clone();
    assert_eq!(calls.len(), 6);
    for (before, after) in old_calls.iter().zip(&calls[3..]) {
        assert!(before.input.get(scene_facts::PROFILE_FIELD).is_none());
        assert!(before.rule_snapshot["runtimeContract"]
            .get("sceneFactsProfile")
            .is_none());
        assert_eq!(
            after.input[scene_facts::PROFILE_FIELD],
            scene_facts::PROFILE
        );
        assert_eq!(
            after.input[scene_facts::PACKET_FIELD]["scenes"][0]["title"],
            "7-1"
        );
        assert!(
            after.input[scene_facts::PACKET_FIELD]["scenes"][0]["headerRows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["text"] == "测试湖 夜 / 外\n")
        );
        assert_eq!(
            after.rule_snapshot["runtimeContract"]["sceneFactsProfile"],
            scene_facts::PROFILE
        );
        for field in ["sourceRows", "contextRows"] {
            assert_eq!(
                after.input[field], before.input[field],
                "{field}: header never becomes production coverage"
            );
        }
        assert_eq!(
            after.schema, before.schema,
            "allowed source/quote domains remain identical"
        );
        for field in ["primary", "methods"] {
            assert_eq!(
                after.rule_snapshot[field], before.rule_snapshot[field],
                "original 3008 {field} remains intact"
            );
        }
        assert_eq!(
            serde_json::to_vec(&creative::prepare_request(after.clone()).unwrap()).unwrap(),
            serde_json::to_vec(after).unwrap(),
            "trusted wrapper stays idempotent"
        );
    }
    run_storyboard(&old, &executor, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(
        executor.calls.lock().await.len(),
        6,
        "old request identity is reusable"
    );
    assert_eq!(serde_json::to_vec(&old).unwrap(), old_bytes);
    let old_again = executor.calls.lock().await[..3].to_vec();
    assert_eq!(
        serde_json::to_vec(&old_again).unwrap(),
        serde_json::to_vec(&old_calls).unwrap()
    );
    let mut bad = current.clone();
    bad[scene_facts::PACKET_FIELD]["scenes"][0]["title"] = json!("日间");
    assert!(run_storyboard(&bad, &executor, "offline", "medium")
        .await
        .is_err());
    assert_eq!(
        executor.calls.lock().await.len(),
        6,
        "tampering stops before dispatch"
    );
}
