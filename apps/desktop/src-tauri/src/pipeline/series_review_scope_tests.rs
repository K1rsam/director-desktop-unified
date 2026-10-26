use super::*;

#[tokio::test]
async fn series_review_entry_scope_preserves_raw_and_keeps_current_episode_write_boundary() {
    let (input, request, raw) = super::super::series_review_entry_replay_fixture().await;
    let before = raw.clone();
    let proof = revalidate_review_entry_scope_response(&request, &raw, &input).unwrap();
    assert_eq!(raw, before);
    assert_eq!(proof["originalRawChanged"], false);
    assert_eq!(proof["conclusion"], "revise");
    assert_eq!(proof["rawSha256"], hash(&before));
    assert!(!proof["allowedEntrySourceRefs"]
        .as_array()
        .unwrap()
        .is_empty());
    for fault in [
        "source-coverage",
        "unit-scope",
        "candidate-hash",
        "unknown-source",
        "future-source",
    ] {
        let mut changed = raw.clone();
        match fault {
            "source-coverage" => changed["sourceRowIds"]
                .as_array_mut()
                .unwrap()
                .push(raw["issues"][0]["sourceRefs"][1].clone()),
            "unit-scope" => {
                changed["issues"][0]["unitIds"] =
                    json!([packet_value(&request.input["packet"], "entry").unwrap()["id"]])
            }
            "candidate-hash" => changed["candidateSha256"] = json!("changed"),
            "unknown-source" => changed["issues"][0]["sourceRefs"] = json!(["row:unknown"]),
            "future-source" => changed["issues"][0]["sourceRefs"] = json!(["row:future:0:1"]),
            _ => unreachable!(),
        }
        assert!(
            revalidate_review_entry_scope_response(&request, &changed, &input).is_err(),
            "{fault}"
        );
    }
}

#[tokio::test]
async fn series_review_entry_scope_rejects_unbound_entry_or_changed_packet() {
    let (input, request, raw) = super::super::series_review_entry_replay_fixture().await;
    let source = source(&input).unwrap();
    let episode = &source.episodes[1];
    let entry = packet_value(&request.input["packet"], "entry").unwrap();
    for fault in [
        "absent",
        "current-episode",
        "retired",
        "stale",
        "duplicate-row",
        "foreign-row",
        "empty-id",
    ] {
        let mut changed = entry.clone();
        match fault {
            "absent" => changed = Value::Null,
            "current-episode" => changed["episodeId"] = json!(episode.id),
            "retired" => changed["retired"] = json!(true),
            "stale" => changed["continuityStale"] = json!(true),
            "duplicate-row" => changed["sourceRefs"]
                .as_array_mut()
                .unwrap()
                .push(entry["sourceRefs"][0].clone()),
            "foreign-row" => changed["sourceRefs"][0] = json!("row:foreign"),
            "empty-id" => changed["id"] = json!(""),
            _ => unreachable!(),
        }
        let candidate = json!({"schemaVersion":"series-episode-candidate/v1",
            "plan":packet_value(&request.input["packet"],"plan").unwrap(),
            "selection":packet_value(&request.input["packet"],"selection").unwrap(),
            "draft":packet_value(&request.input["packet"],"draft").unwrap()});
        assert!(
            validate_review(&raw, &candidate, &source, episode, &changed).is_err(),
            "{fault}"
        );
    }
    let mut changed = request.clone();
    let key = changed.input["packet"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == "entry")
        .unwrap()["blob"]
        .as_str()
        .unwrap()
        .to_owned();
    changed.input["packet"]["blobs"][&key]["value"]["exitState"] = json!("tampered");
    assert!(revalidate_review_entry_scope_response(&changed, &raw, &input).is_err());
    let mut same_episode_only = raw.clone();
    same_episode_only["issues"][0]["sourceRefs"] = json!([raw["issues"][0]["sourceRefs"][0]]);
    assert!(
        revalidate_review_entry_scope_response(&request, &same_episode_only, &input).is_err(),
        "only actual old scope false rejection can migrate"
    );
}
