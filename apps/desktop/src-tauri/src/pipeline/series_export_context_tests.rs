//! Independent synthetic export tests. No real executor, account or media call.
use super::*;
use async_trait::async_trait;

struct ReplayCandidates {
    originals: Vec<Value>,
}
fn packet_part<'a>(request: &'a TextRequest, kind: &str) -> &'a Value {
    let packet = &request.input["packet"];
    let b = packet["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == kind)
        .unwrap();
    &packet["blobs"][b["blob"].as_str().unwrap()]["value"]
}
#[async_trait]
impl TextExecutor for ReplayCandidates {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        match request.stage.as_str() {
            "seriesAuthor" => Ok(self
                .originals
                .iter()
                .find(|d| d["episodeId"] == request.input["episodeId"])
                .unwrap()["adoptionReceipt"]["roleReceipts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["role"] == "author")
                .unwrap()["raw"]
                .clone()),
            "seriesFinisher" => Ok(json!({"schemaVersion":"series-episode-candidate/v1",
                "plan":packet_part(&request,"plan"),"selection":packet_part(&request,"selection"),"draft":packet_part(&request,"draft")})),
            "seriesReview" => Ok(
                json!({"schemaVersion":"series-independent-review/v1","candidateSha256":request.input["candidateSha256"],
                "reviewedUnitIds":packet_part(&request,"draft")["units"].as_array().unwrap().iter().map(|u|u["id"].clone()).collect::<Vec<_>>(),
                "sourceRowIds":packet_part(&request,"source")["sourceRows"].as_array().unwrap().iter().map(|r|r["id"].clone()).collect::<Vec<_>>(),
                "conclusion":"pass","summary":"synthetic frozen-context review","issues":[]}),
            ),
            other => Err(format!("unexpected export fixture role: {other}")),
        }
    }
    async fn publish_episode(&self, output: &Value, input: &Value) -> Result<(), String> {
        super::super::validate_episode_delivery(input, output)
    }
}

async fn mixed_context_fixture() -> (
    tempfile::TempDir,
    crate::store::Store,
    Value,
    Vec<Value>,
    Vec<Value>,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(directory.path().join("project")).unwrap();
    store
        .create("合成逐集冻结导出", "synthetic-export-context")
        .unwrap();
    let input = super::super::seed_engine_series_fixture(&store, 2).await;
    let originals = store.series_deliveries().unwrap();
    let executor = ReplayCandidates { originals };
    let episodes = input["productionScope"]["episodeIds"].clone();
    let mut first = input.clone();
    first
        .as_object_mut()
        .unwrap()
        .remove("seriesRuleMaterialization");
    first["selection"]["episodeIds"] = json!([episodes[0]]);
    let first_out = super::super::run(&first, &executor, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(first_out["status"], "completed", "{}", first_out["blocked"]);
    let first_delivery = first_out["seriesDeliveries"][0].clone();
    let mut second = input;
    second["selection"]["episodeIds"] = json!([episodes[1]]);
    second["previousBoundary"] = first_delivery["bundle"]["draft"]["units"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    let second_out = super::super::run(&second, &executor, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(
        second_out["status"], "completed",
        "{}",
        second_out["blocked"]
    );
    let mut export_input = first.clone();
    export_input["selection"]["episodeIds"] = episodes;
    (
        directory,
        store,
        export_input,
        vec![first, second],
        vec![first_delivery, second_out["seriesDeliveries"][0].clone()],
    )
}

#[tokio::test]
async fn frozen_export_combines_legacy_first_and_pooled_later_without_rewriting_inputs() {
    let (directory, store, input, frozen, deliveries) = mixed_context_fixture().await;
    let original_inputs = serde_json::to_vec(&frozen).unwrap();
    assert!(frozen[0].get("seriesRuleMaterialization").is_none());
    assert_eq!(frozen[1]["seriesRuleMaterialization"], rules::CONTENT_ONCE);
    let rejected = directory.path().join("wrong-shared-context");
    assert!(write(&input, &deliveries, &rejected)
        .unwrap_err()
        .contains("RULE_MATERIALIZATION_INPUT_MISMATCH"));
    assert!(!rejected.exists());
    let a = directory.path().join("combined-a");
    let b = directory.path().join("combined-b");
    let result = write_with_inputs(&input, &deliveries, &frozen, &a).unwrap();
    assert_eq!(result["count"], 2);
    assert_eq!(result["scopeComplete"], true);
    write_with_inputs(&input, &deliveries, &frozen, &b).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(a.join("manifest.json")).unwrap()).unwrap();
    for file in manifest["files"].as_array().unwrap() {
        let path = s(file, "path");
        assert_eq!(
            fs::read(a.join(path)).unwrap(),
            fs::read(b.join(path)).unwrap(),
            "{path}"
        );
    }
    assert_eq!(
        fs::read(a.join("manifest.json")).unwrap(),
        fs::read(b.join("manifest.json")).unwrap()
    );
    assert!(write_with_inputs(&input, &deliveries, &frozen, &a)
        .unwrap_err()
        .contains("NOT_OVERWRITTEN"));
    assert_eq!(serde_json::to_vec(&frozen).unwrap(), original_inputs);
    // Public Store export retains the existing same-context case, and uses
    // its saved per-episode inputs rather than mutating the project metadata.
    let before = store.series_deliveries().unwrap();
    let public = store
        .export_series_delivery(
            &json!({"episodeIds":input["selection"]["episodeIds"],"path":directory.path()}),
        )
        .unwrap();
    assert_eq!(public["count"], 2);
    assert_eq!(store.series_deliveries().unwrap(), before);
    assert!(store
        .jobs()
        .unwrap()
        .iter()
        .all(|job| job["kind"] != "image" && job["kind"] != "video"));
}

#[tokio::test]
async fn frozen_export_rejects_source_direction_scope_rules_or_pairing_drift_before_writing() {
    let (directory, _store, input, frozen, deliveries) = mixed_context_fixture().await;
    for kind in ["source", "direction", "scope", "rules", "pairing", "count"] {
        let mut bad = frozen.clone();
        match kind {
            "source" => bad[1]["source"]["text"] = json!("不是同一锁定原文"),
            "direction" => {
                bad[1]["settings"]["productionDirection"] = json!("另一制作方向");
                bad[1]["settings"]["confirmedDirection"] = json!("另一制作方向");
            }
            "scope" => bad[1]["productionScope"]["basis"] = json!("another-production-scope"),
            "rules" => bad[1]["seriesRulePackSha256"] = json!("another-rule-pack"),
            "pairing" => bad.swap(0, 1),
            "count" => {
                bad.pop();
            }
            _ => unreachable!(),
        }
        let target = directory.path().join(kind);
        let error = write_with_inputs(&input, &deliveries, &bad, &target).unwrap_err();
        assert!(error.starts_with("SERIES_"), "{kind}: {error}");
        assert!(
            !target.exists(),
            "{kind} must fail before a partial package is created"
        );
    }
    let mut narrowed = input;
    narrowed["selection"]["episodeIds"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    let target = directory.path().join("outside-selection");
    assert!(write_with_inputs(&narrowed, &deliveries, &frozen, &target)
        .unwrap_err()
        .contains("OUTSIDE_SELECTION"));
    assert!(!target.exists());
}
