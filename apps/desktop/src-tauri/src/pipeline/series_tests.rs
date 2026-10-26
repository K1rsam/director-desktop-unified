use super::*;
use async_trait::async_trait;
use std::sync::Mutex;
#[path = "series_frame_render_tests.rs"]
pub(crate) mod frame_render_tests;
#[path = "series_repair_plan_tests.rs"]
pub(crate) mod repair_plan_tests;
#[path = "series_supplement_tests.rs"]
pub(crate) mod supplement_tests;

#[test]
fn dialogue_diagnostics_use_absolute_source_ranges_and_never_rewrite_the_raw() {
    let source = super::super::prepare_source(
        "第一集\n场景：门口\n甲：来了。（转头）走吧。\n第二集\n场景：门口\n甲：停。\n",
    )
    .unwrap();
    let episode = &source.episodes[0];
    let quote = source
        .rows
        .iter()
        .filter(|row| row.episode_id.as_deref() == Some(&episode.id))
        .find_map(|row| row.dialogue.as_ref())
        .unwrap();
    let raw = json!({"draft":{"units":[{"shots":[{"dialogueParts":[
        {"lineId":quote.id,"quoteStart":0,"quoteEnd":3},
        {"lineId":"unknown","quoteStart":0,"quoteEnd":3}]}]}]}});
    let before = raw.clone();
    let report = source_coordinate_diagnostics(&source, episode, &raw);
    assert_eq!(raw, before);
    assert_eq!(report["originalRawChanged"], false);
    assert_eq!(report["sourceSha256"], source.sha256);
    assert_eq!(report["lines"].as_array().unwrap().len(), 1);
    assert_eq!(report["lines"][0]["lineId"], quote.id);
    let ranges = report["lines"][0]["allowedAbsoluteRanges"]
        .as_array()
        .unwrap();
    assert_eq!(ranges.len(), 2);
    for (actual, segment) in ranges.iter().zip(&quote.spoken_segments) {
        assert_eq!(actual["quoteStart"], segment.range.start);
        assert_eq!(actual["quoteEnd"], segment.range.end);
        assert!(actual["quoteStart"].as_u64().unwrap() > 0);
    }
    assert!(ranges[0]["quoteEnd"].as_u64() < ranges[1]["quoteStart"].as_u64());
}

#[tokio::test]
async fn new_dialogue_coordinate_guidance_does_not_rewrite_frozen_legacy_requests() {
    let (input, authored) = fixture();
    let executor = Offline::new(authored.clone());
    run(&input, &executor, "offline", "medium").await.unwrap();
    let requests = executor.requests.lock().unwrap();
    let author = requests.iter().find(|r| r.stage == "seriesAuthor").unwrap();
    assert_eq!(
        author.input["seriesDialogueCoordinates"],
        ABSOLUTE_DIALOGUE_COORDINATES
    );
    assert!(s(&author.input, "transportContract").contains("不能从句首0开始"));
    drop(requests);
    let mut legacy = input.clone();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("seriesDialogueCoordinates");
    let executor = Offline::new(authored);
    let delivered = run(&legacy, &executor, "offline", "medium").await.unwrap();
    assert_eq!(delivered["status"], "completed");
    let requests = executor.requests.lock().unwrap();
    let author = requests.iter().find(|r| r.stage == "seriesAuthor").unwrap();
    assert!(author.input.get("seriesDialogueCoordinates").is_none());
    assert!(!s(&author.input, "transportContract").contains("不能从句首0开始"));
}

fn binding<'a>(request: &'a TextRequest, kind: &str) -> &'a Value {
    let packet = &request.input["packet"];
    let entry = packet["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == kind)
        .unwrap();
    &packet["blobs"][entry["blob"].as_str().unwrap()]["value"]
}

#[tokio::test]
async fn pooled_rules_domain_repair_preserves_source_full_design_rejected_candidate_and_legacy_budget(
) {
    struct RepairFixture {
        good: Value,
        calls: Mutex<Vec<TextRequest>>,
        rejected: Mutex<Vec<Value>>,
    }
    #[async_trait]
    impl TextExecutor for RepairFixture {
        async fn execute(&self, request: TextRequest) -> Result<Value, String> {
            self.calls.lock().unwrap().push(request.clone());
            let mut raw = self.good.clone();
            if request.stage == "seriesFinisher" {
                raw["draft"]["unresolved"] =
                    json!([{"coordinate":"U1","problem":"合成返修定位，不能删减原源或设计"}]);
            }
            Ok(raw)
        }
        async fn reject(&self, _: &TextRequest, raw: &Value, _: &str) -> Result<(), String> {
            self.rejected.lock().unwrap().push(raw.clone());
            Ok(())
        }
    }
    for pooled in [false, true] {
        let (mut input, good) = fixture();
        if !pooled {
            input
                .as_object_mut()
                .unwrap()
                .remove("seriesRuleMaterialization");
        }
        let source = source(&input).unwrap();
        let episode = &source.episodes[0];
        let cards = design::cards(&source, episode, input["assets"].as_array().unwrap()).unwrap();
        let mut values = base_values(&input, &source, episode, &cards, &Value::Null);
        values.extend([
            ("plan", good["plan"].clone()),
            ("selection", good["selection"].clone()),
            ("draft", good["draft"].clone()),
        ]);
        let original = request(
            "finisher",
            "seriesFinisher",
            &episode.id,
            "0",
            &input,
            values,
            candidate_schema(&source, episode, &cards, "off"),
            "offline",
            "medium",
        )
        .unwrap();
        let executor = RepairFixture {
            good: good.clone(),
            calls: Mutex::new(vec![]),
            rejected: Mutex::new(vec![]),
        };
        let mut receipts = vec![];
        create_candidate(
            &executor,
            original.clone(),
            &source,
            episode,
            &cards,
            &input,
            &mut receipts,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let calls = executor.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let repair = &calls[1];
        validate_role_packet(repair, &input, &source, episode).unwrap();
        assert_eq!(repair.input["packet"], original.input["packet"]);
        assert_eq!(repair.schema, original.schema);
        assert_eq!(binding(repair, "source"), binding(&original, "source"));
        assert_eq!(binding(repair, "assets"), binding(&original, "assets"));
        assert_eq!(binding(repair, "plan"), &good["plan"]);
        assert_eq!(binding(repair, "selection"), &good["selection"]);
        assert_eq!(binding(repair, "draft"), &good["draft"]);
        assert_eq!(
            repair.input["validationRepair"]["candidate"],
            executor.rejected.lock().unwrap()[0]
        );
        assert_eq!(repair.input["validationRepair"]["attempt"], 1);
        assert_eq!(
            repair.input["validationRepair"]["maximumAttempts"],
            MAX_REPAIRS
        );
        let legacy = repair_rules("finisher", false).unwrap();
        assert_eq!(
            restore_repair_rule_snapshot(&repair.rule_snapshot).unwrap(),
            legacy
        );
        if pooled {
            assert_eq!(repair.rule_snapshot["version"], rules::CONTENT_ONCE);
            let mut bad = repair.clone();
            bad.rule_snapshot["roleSets"]["repair"]["methods"]
                .as_array_mut()
                .unwrap()
                .swap(0, 1);
            assert_eq!(
                validate_role_packet(&bad, &input, &source, episode).unwrap_err(),
                "SERIES_ROLE_RULE_CONTENT_MISMATCH"
            );
        } else {
            assert!(repair.input.get("seriesRuleMaterialization").is_none());
            assert_eq!(
                serde_json::to_vec(&repair.rule_snapshot).unwrap(),
                serde_json::to_vec(&legacy).unwrap()
            );
        }
    }
}

#[tokio::test]
async fn pooled_rules_keep_regular_review_repairs_plain_and_reject_unknown_marker() {
    let (input, good) = fixture();
    assert_eq!(
        repair_rules_for(&input, "repair", false).unwrap(),
        rules::role("repair", false).unwrap()
    );
    let mut executor = Offline::new(good);
    executor.review_rounds = 1;
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["status"], "completed", "{}", output["blocked"]);
    let calls = executor.requests.lock().unwrap();
    let repair = calls.iter().find(|r| r.stage == "seriesRepair").unwrap();
    assert_eq!(repair.rule_snapshot, rules::role("repair", false).unwrap());
    let mut unknown = input;
    unknown["seriesRuleMaterialization"] = json!("series-rule-content-once/future");
    assert_eq!(
        repair_rules_for(&unknown, "finisher", false).unwrap_err(),
        "SERIES_RULE_MATERIALIZATION_VERSION"
    );
}
fn fixture() -> (Value, Value) {
    let source = super::super::prepare_source("第一集\n场景：门口\n甲：来了。\n").unwrap();
    let eid = &source.episodes[0].id;
    let sid = &source.scenes[0].id;
    let rows: Vec<_> = source.rows.iter().filter(|r| r.kind == "body").collect();
    let refs: Vec<_> = rows.iter().map(|r| r.id.clone()).collect();
    let q = rows.iter().find_map(|r| r.dialogue.as_ref()).unwrap();
    let assets:Vec<_>=[("a","甲","character"),("s","门口","scene")].into_iter().map(|(id,name,kind)|json!({"id":id,"variantId":"base","name":name,"kind":kind,"version":1,"designRevision":1,"applicability":{"episodes":[eid],"scenes":[sid]},"adoptedDesign":{"version":"series-full-design/v1","status":"ADOPTED","assetId":id,"variantId":"base","designRevision":1,"sourceSha256":source.sha256,"sourceRefs":refs,"full":format!("{name}完整文字设计：正式身份稳定，适用本集。入口在画面左方，固定门框位于后方。人物保留黑短发、普通棉衣，门外侧光，动作通道净宽两人。"),"brief":format!("{name}定位短卡"),"designDependencies":[],"provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT","adoptionBasis":{"kind":"delegated-text-review","evidence":"synthetic fixture"}},"imagePrompt":"DO_NOT_FEED_IMAGE_PROMPT","history":["DO_NOT_FEED_HISTORY"]})).collect();
    let emotion = json!({"subject":"甲","contextTone":"等待","audienceEffect":"看见到来","change":"等待变确认","landing":"目光落向门外","retain":["原话"]});
    let plan = json!({"schemaVersion":"storyboard-plan/v2","unitPackingPolicy":"event-first-v1","emotionArc":"等待转为确认","events":[{"id":"E1","sceneId":sid,"sourceRefs":refs,"boundaryBefore":{"kind":"episode_start","reason":"本集开始"},"emotionContract":emotion}],"workPackages":[{"id":"W1","eventId":"E1","sourceRefs":refs,"contextRefs":[],"entryFactIds":[],"dependsOn":[],"scenarioKeys":[],"emotionContract":emotion}],"provisionalUnits":[{"id":"P1","packageId":"W1","sourceRefs":refs,"secondsRange":[3.2,3.2]}],"entryFacts":[],"fantasyCandidates":[]});
    let draft = json!({"schemaVersion":"storyboard-draft/v2","units":[{"id":"U1","episodeId":eid,"eventId":"E1","sceneId":sid,"boundaryBefore":{"kind":"episode_start","reason":"本集开始"},"title":"门外到来","unitDirection":"甲由等待转为确认，目光先动再说。","intent":{"subject":"甲","change":"等待到确认","landing":"门外","retain":["原话"]},"scene":"门口","participants":["甲"],"props":[],"lighting":"门外侧光","initialState":"甲在门内左侧，面向走廊。","exitState":"甲目光落在门外，身体仍在门内。","sourceRefs":refs,"assetRefs":[{"assetId":"a","variantId":"base","use":"character","order":1,"sourceRefs":refs},{"assetId":"s","variantId":"base","use":"scene","order":2,"sourceRefs":refs}],"imageBindings":["a","s"],"shots":[{"id":"S1","duration":3.2,"camera":"中近景固定机位，保留门框","picture":"甲抬眼望向门外，张口说话。","lighting":"侧光照脸，门框留阴影","sound":"轻脚步由远及近","dialogueParts":[{"lineId":q.id,"quoteStart":q.range.start,"quoteEnd":q.range.end,"presentation":"画内","delivery":"轻声确认","voiceStart":0.2,"voiceEnd":1.3}],"transition":"自然切出","sourceRefs":refs,"contextRefs":[]}]}],"fantasyRealization":[],"revisionNotes":[],"unresolved":[]});
    let mut input = json!({"source":source,"assets":assets,"foundation":{"direction":{"style":"3D国漫"},"facts":[]},"settings":{"visualStyle":"3D国漫，16:9","confirmedDirection":"3D国漫，情绪充分，保留源文，不加额外幻想。","fantasyMode":"off","maxUnitSeconds":30},"capabilities":{"minDuration":0,"maxDuration":30,"imageLimit":30},"selection":{"episodeIds":[eid]},"productionScope":{"version":"desktop-production-scope/v1","status":"locked","sourceSha256":source.sha256,"episodeIds":[eid]}});
    mark_input(&mut input).unwrap();
    (
        input,
        json!({"schemaVersion":"series-episode-candidate/v1","plan":plan,"selection":[],"draft":draft}),
    )
}
struct Offline {
    authored: Value,
    requests: Mutex<Vec<TextRequest>>,
    cache: Mutex<HashMap<String, Value>>,
    unknown: bool,
    published: Mutex<Vec<Value>>,
    stop_episode: Option<String>,
    review_rounds: usize,
    repeat_issue: bool,
    unknown_repair: bool,
}
impl Offline {
    fn new(authored: Value) -> Self {
        Self {
            authored,
            requests: Mutex::new(vec![]),
            cache: Mutex::new(HashMap::new()),
            unknown: false,
            published: Mutex::new(vec![]),
            stop_episode: None,
            review_rounds: 0,
            repeat_issue: false,
            unknown_repair: false,
        }
    }
}
#[async_trait]
impl TextExecutor for Offline {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let identity = digest(&serde_json::to_string(&request).unwrap());
        if let Some(saved) = self.cache.lock().unwrap().get(&identity) {
            return Ok(saved.clone());
        }
        self.requests.lock().unwrap().push(request.clone());
        if request.stage == "seriesAuthor"
            && self.stop_episode.as_deref() == request.input["episodeId"].as_str()
        {
            return Err("LOCAL_PREPARATION_STOPPED: synthetic before-submit failure".into());
        }
        if self.unknown {
            return Err("SUBMISSION_UNKNOWN: retained original attempt".into());
        }
        if self.unknown_repair && request.stage == "seriesRepair" {
            return Err("SUBMISSION_UNKNOWN: preserve original repair attempt".into());
        }
        let raw = match request.stage.as_str() {
            "seriesAuthor" => self
                .authored
                .get(s(&request.input, "episodeId"))
                .unwrap_or(&self.authored)
                .clone(),
            "seriesFinisher" => {
                json!({"schemaVersion":"series-episode-candidate/v1","plan":binding(&request,"plan"),"selection":binding(&request,"selection"),"draft":binding(&request,"draft")})
            }
            "seriesReview" => {
                let mut review = json!({"schemaVersion":"series-independent-review/v1","candidateSha256":request.input["candidateSha256"],"reviewedUnitIds":binding(&request,"draft")["units"].as_array().unwrap().iter().map(|u|u["id"].clone()).collect::<Vec<_>>(),"sourceRowIds":binding(&request,"source")["sourceRows"].as_array().unwrap().iter().map(|r|r["id"].clone()).collect::<Vec<_>>(),"conclusion":"pass","summary":"synthetic independent role checked exact source, full design, timing and intended image references","issues":[]});
                let round = s(&request.input, "executionRoleId")
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                if round < self.review_rounds {
                    review["conclusion"] = json!("revise");
                    review["issues"] = json!([{"id":"visibility","severity":"must_fix","unitIds":[binding(&request,"draft")["units"][0]["id"]],"sourceRefs":[],"problem":format!("synthetic camera visibility {}",if self.repeat_issue{0}else{round}),"requestedChange":"仅明确本镜机位与门框关系，不改变原话或其它单元"}]);
                }
                review
            }
            "seriesRepair" => {
                let mut candidate = json!({"schemaVersion":"series-episode-candidate/v1","plan":binding(&request,"plan"),"selection":binding(&request,"selection"),"draft":binding(&request,"draft")});
                candidate["draft"]["units"][0]["shots"][0]["camera"] =
                    json!("门内中近景固定，门框居右，保留甲的目光与开口动作");
                repair::schema_projection(&candidate, &request.schema)?
            }
            "seriesDesign" => {
                json!({"schemaVersion":"series-full-design-candidate/v1","designs":binding(&request,"designTargets").as_array().unwrap().iter().map(|t|json!({"assetId":t["asset"]["id"],"variantId":t["asset"]["variantId"],"full":"完整角色设计：短黑发圆脸，棉衣的领口闭合，鞋底厚实。源事实为门口的甲；衣服材质为已确认3D风格下的制作补足。动作只沿原文。","brief":"黑短发棉衣角色短卡","sourceRefs":binding(&request,"designSourceRows").as_array().unwrap().iter().map(|r|r["id"].clone()).collect::<Vec<_>>(),"sourceFacts":[],"productionChoices":["棉衣材质"],"designDependencies":[]})).collect::<Vec<_>>()})
            }
            "seriesDesignReview" => {
                json!({"schemaVersion":"series-design-review/v1","candidateSha256":request.input["candidateSha256"],"reviewedAssetIds":binding(&request,"designTargets").as_array().unwrap().iter().map(|t|t["asset"]["id"].clone()).collect::<Vec<_>>(),"summary":"synthetic separate design reviewer verified identity and source scope","issues":[]})
            }
            other => return Err(format!("unexpected offline role: {other}")),
        };
        self.cache.lock().unwrap().insert(identity, raw.clone());
        Ok(raw)
    }
    async fn publish_episode(&self, output: &Value, input: &Value) -> Result<(), String> {
        validate_episode_delivery(input, output)?;
        self.published.lock().unwrap().push(output.clone());
        Ok(())
    }
}
#[tokio::test]
async fn full_episode_uses_complete_design_author_finish_review_and_same_version_md() {
    let (input, authored) = fixture();
    let executor = Offline::new(authored);
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["blocked"], json!([]), "{}", output["blocked"]);
    let requests = executor.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|r| r.stage.as_str())
            .collect::<Vec<_>>(),
        ["seriesAuthor", "seriesFinisher", "seriesReview"]
    );
    for r in requests.iter() {
        assert!(r.input["freshContext"] == true);
        assert!(binding(r, "assets")[0]["adoptedDesign"]["full"]
            .as_str()
            .unwrap()
            .contains("完整文字设计"));
        assert!(!r.input.to_string().contains("DO_NOT_FEED"));
    }
    assert_eq!(
        binding(&requests[1], "source"),
        binding(&requests[0], "source")
    );
    assert_ne!(
        requests[0].input["executionRoleId"],
        requests[1].input["executionRoleId"]
    );
    assert_eq!(output["units"][0]["duration"], 3.2);
    assert_eq!(output["units"][0]["containerSeconds"], 4);
    assert_eq!(
        output["units"][0]["shots"][0]["dialogueParts"][0]["text"],
        "来了。"
    );
    let d = &output["seriesDeliveries"][0];
    assert!(s(d, "markdown").contains("@[甲CH]"));
    assert!(s(d, "markdown").contains("分镜1｜0-3.2s｜3.2s"));
    assert_eq!(d["assetIndex"]["assets"][0]["filename"], "角色-甲.png");
    validate_delivery(&input, d).unwrap();
}
#[tokio::test]
async fn missing_complete_design_is_created_and_reviewed_without_image_account() {
    let (mut input, authored) = fixture();
    input["assets"][0]
        .as_object_mut()
        .unwrap()
        .remove("adoptedDesign");
    let executor = Offline::new(authored);
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["blocked"], json!([]), "{}", out["blocked"]);
    let requests = executor.requests.lock().unwrap();
    assert_eq!(requests[0].stage, "seriesDesign");
    assert_eq!(requests[1].stage, "seriesDesignReview");
    assert_eq!(out["designAdoptions"].as_array().unwrap().len(), 1);
    assert_eq!(
        out["designAdoptions"][0]["adoptedDesign"]["status"],
        "ADOPTED"
    );
    assert_eq!(
        input["assets"][0]["imagePrompt"],
        "DO_NOT_FEED_IMAGE_PROMPT"
    );
}

#[tokio::test]
async fn compact_design_packet_keeps_full_material_once_and_legacy_frozen_replay() {
    // The explicit-use contract adds an earlier, exact source-selection guard.
    // Exercise both frozen generations: the new guard must reject tampering,
    // while inputs created before it keep their original packet/cache contract.
    for explicit_source_uses in [false, true] {
        let (mut input, authored) = fixture();
        if !explicit_source_uses {
            input.as_object_mut().unwrap().remove(design_source::FIELD);
            input
                .as_object_mut()
                .unwrap()
                .remove(supply_revision::INPUT_FIELD);
        }
        input["assets"][0]
            .as_object_mut()
            .unwrap()
            .remove("adoptedDesign");
        let mut legacy = input.clone();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("seriesTransportVersion");
        let old = Offline::new(authored.clone());
        let old_out = run(&legacy, &old, "offline", "medium").await.unwrap();
        assert_eq!(old_out["blocked"], json!([]));
        let old_requests = old.requests.lock().unwrap().clone();
        let current = Offline::new(authored);
        let out = run(&input, &current, "offline", "medium").await.unwrap();
        assert_eq!(out["blocked"], json!([]));
        let requests = current.requests.lock().unwrap();
        let locked = source(&input).unwrap();
        let episode = &locked.episodes[0];
        for index in 0..2 {
            let a = &old_requests[index];
            let b = &requests[index];
            assert_eq!(
                binding(b, "source")["sourceRowsBinding"],
                "designSourceRows"
            );
            assert!(binding(b, "source").get("sourceRows").is_none());
            assert_eq!(
                binding(a, "designSourceRows"),
                binding(b, "designSourceRows")
            );
            assert_eq!(binding(a, "assets"), binding(b, "assets"));
            assert_eq!(
                binding(b, "designTargets")[0]["asset"]
                    .as_object()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(
                binding(a, "designTargets")[0]["asset"]["id"],
                binding(b, "designTargets")[0]["asset"]["id"]
            );
            assert!(b.input.to_string().len() < a.input.to_string().len());
            assert_eq!(a.rule_snapshot, b.rule_snapshot);
            assert!(a.input.get("seriesTransportVersion").is_none());
            assert_eq!(
                a.input.get(design_source::FIELD).is_some(),
                explicit_source_uses
            );
            validate_role_packet(a, &legacy, &locked, episode).unwrap();
            validate_role_packet(b, &input, &locked, episode).unwrap();
            // Re-signing a compact packet must not make changed source material
            // authoritative; validate the content against the frozen screenplay.
            let mut tampered = b.clone();
            let mut values: Vec<_> = b.input["packet"]["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| {
                    let kind = s(item, "kind");
                    (kind, binding(b, kind).clone())
                })
                .collect();
            let rows = values
                .iter_mut()
                .find(|(kind, _)| *kind == "designSourceRows")
                .unwrap();
            rows.1[0]["text"] = json!("不能通过重签摘要替换冻结源文");
            tampered.input["packet"] =
                packet(s(&b.input, "role"), &episode.id, &input, values).unwrap();
            assert_eq!(
                validate_role_packet(&tampered, &input, &locked, episode).unwrap_err(),
                if explicit_source_uses {
                    "SERIES_DESIGN_SOURCE_USES_CHANGED"
                } else {
                    "SERIES_DESIGN_SOURCE_ROWS_CHANGED"
                }
            );
        }
        drop(requests);
        let repeated = run(&legacy, &old, "offline", "medium").await.unwrap();
        assert_eq!(repeated["blocked"], json!([]));
        assert_eq!(
            old.requests.lock().unwrap().len(),
            old_requests.len(),
            "old frozen calls retain their cache keys"
        );
    }
}

#[tokio::test]
async fn stale_design_requires_new_review_while_unaffected_design_is_reused() {
    let (mut input, authored) = fixture();
    input["assets"][0]["designStatus"] = json!("needs_review");
    let original = input["assets"][0]["adoptedDesign"].clone();
    let executor = Offline::new(authored);
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["blocked"], json!([]), "{}", out["blocked"]);
    let adoptions = out["designAdoptions"].as_array().unwrap();
    assert_eq!(adoptions.len(), 1);
    assert_eq!(adoptions[0]["assetId"], "a");
    assert_eq!(adoptions[0]["baseDesignRevision"], 1);
    assert_eq!(adoptions[0]["designRevision"], 2);
    assert_eq!(
        input["assets"][0]["adoptedDesign"], original,
        "history is not rewritten in place"
    );
    assert_eq!(
        out["seriesDeliveries"][0]["bundle"]["assets"][0]["designStatus"],
        "adopted"
    );
}
#[tokio::test]
async fn replay_same_frozen_inputs_reuses_all_paid_roles() {
    let (input, authored) = fixture();
    let executor = Offline::new(authored);
    let first = run(&input, &executor, "offline", "medium").await.unwrap();
    let count = executor.requests.lock().unwrap().len();
    let second = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(first, second);
    assert_eq!(executor.requests.lock().unwrap().len(), count);
}
#[tokio::test]
async fn unknown_does_not_trigger_domain_repair_or_other_roles() {
    let (input, authored) = fixture();
    let mut executor = Offline::new(authored);
    executor.unknown = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "partially_blocked");
    assert_eq!(executor.requests.lock().unwrap().len(), 1);
    assert!(executor.published.lock().unwrap().is_empty());
}
#[tokio::test]
async fn adopted_receipt_rejects_forged_review_or_changed_markdown() {
    let (input, authored) = fixture();
    let executor = Offline::new(authored);
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["blocked"], json!([]), "{}", out["blocked"]);
    let mut d = out["seriesDeliveries"][0].clone();
    d["adoptionReceipt"]["review"]["candidateSha256"] = json!("other");
    assert!(validate_delivery(&input, &d).is_err());
    let mut d = out["seriesDeliveries"][0].clone();
    d["markdown"] = json!(s(&d, "markdown").replace("来了。", "走了。"));
    assert!(validate_delivery(&input, &d).is_err());
}
#[test]
fn scope_and_legacy_profiles_never_silently_migrate() {
    let (mut input, _) = fixture();
    input["storyboardCarrierProfile"] = json!("integer-video-units/v1");
    assert!(validate_profile(&input).is_err());
    let mut old = json!({"source":input["source"]});
    assert!(!uses_profile(&old).unwrap());
    old[PROFILE_FIELD] = json!("desktop-series-unknown/v99");
    assert!(uses_profile(&old).is_err());
}

#[tokio::test]
async fn portable_delivery_is_exclusive_complete_and_contains_no_channel_credentials() {
    let (mut input, authored) = fixture();
    input["settings"]["apiKey"] = json!("SECRET_NOT_IN_DELIVERY");
    input["settings"]["providers"] = json!({"private":{"token":"SECRET_NOT_IN_DELIVERY"}});
    let executor = Offline::new(authored);
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["blocked"], json!([]), "{}", output["blocked"]);
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("delivery");
    let result = export_delivery_package(
        &input,
        output["seriesDeliveries"].as_array().unwrap(),
        &target,
    )
    .unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["imageCount"], 2);
    assert_eq!(result["scopeComplete"], true);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(target.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["mediaGenerated"], false);
    assert_eq!(manifest["platformImportVerified"], false);
    for file in manifest["files"].as_array().unwrap() {
        let bytes = std::fs::read(target.join(file["path"].as_str().unwrap())).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("SECRET_NOT_IN_DELIVERY"));
    }
    let files: Vec<_> = std::fs::read_dir(target.join("白图"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 2);
    let proof: Value = serde_json::from_slice(
        &std::fs::read(target.join("资料/placeholder-verification.json")).unwrap(),
    )
    .unwrap();
    let md5s: HashSet<_> = proof["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["md5"].as_str().unwrap())
        .collect();
    assert_eq!(md5s.len(), 2);
    assert!(md5s.iter().all(|v| v.len() == 32));
    let mut dimensions = HashSet::new();
    for file in files {
        assert_eq!(file.extension().unwrap(), "png");
        let pixels = image::open(file).unwrap().to_rgb8();
        assert!(pixels.pixels().all(|p| p.0 == [255, 255, 255]));
        assert!(dimensions.insert(pixels.dimensions()));
    }
    let before = std::fs::read(target.join("manifest.json")).unwrap();
    assert!(export_delivery_package(
        &input,
        output["seriesDeliveries"].as_array().unwrap(),
        &target
    )
    .unwrap_err()
    .contains("NOT_OVERWRITTEN"));
    assert_eq!(before, std::fs::read(target.join("manifest.json")).unwrap());
}

#[test]
fn registry_identity_allocation_is_persistent_and_frozen_collisions_fail_closed() {
    let (input, _) = fixture();
    let mut assets = input["assets"].as_array().unwrap().clone();
    allocate_asset_identities(&mut assets).unwrap();
    let original = assets.clone();
    assets.reverse();
    assets.push(json!({"id":"p","variantId":"base","name":"杯","kind":"prop"}));
    allocate_asset_identities(&mut assets).unwrap();
    assert_eq!(
        assets[0]["placeholderOrdinal"],
        original[1]["placeholderOrdinal"]
    );
    assert_eq!(assets[2]["placeholderOrdinal"], 2);
    let mut collision = vec![
        json!({"id":"1","name":"Ａ","kind":"prop","importName":"Ａ"}),
        json!({"id":"2","name":"a","kind":"prop","importName":"a"}),
    ];
    assert!(allocate_asset_identities(&mut collision)
        .unwrap_err()
        .contains("COLLISION"));
    let mut collision = vec![
        json!({"id":"1","name":"Straße","kind":"prop","importName":"Straße"}),
        json!({"id":"2","name":"STRASSE","kind":"prop","importName":"STRASSE"}),
    ];
    assert!(allocate_asset_identities(&mut collision)
        .unwrap_err()
        .contains("COLLISION"));
}

#[test]
fn known_legacy_kinds_preserve_source_identity_and_voice_only_never_allocates_images() {
    let kinds = [
        ("CH", "CH"),
        ("BEAST", "CH"),
        ("GROUP", "OM"),
        ("LOC", "SC"),
        ("PROP", "PR"),
    ];
    let mut assets:Vec<_> = kinds.iter().enumerate().map(|(i,(kind,_))|
        json!({"id":format!("a{i}"),"variantId":"base","name":format!("资产{i}"),"kind":kind})).collect();
    assets.extend([
        json!({"id":"voice","name":"旁白","kind":"SE"}),
        json!({"id":"ch-voice","name":"画外人物","kind":"CH","preparation":{"mode":"AUDIO"}}),
    ]);
    allocate_asset_identities(&mut assets).unwrap();
    let used: HashSet<_> = kinds
        .iter()
        .enumerate()
        .map(|(i, _)| format!("a{i}"))
        .collect();
    let rows = export::index_assets(&assets, &used).unwrap();
    for (i, (original, expected)) in kinds.iter().enumerate() {
        assert_eq!(assets[i]["kind"], *original);
        assert_eq!(rows[i]["type"], *expected);
    }
    for voice in &assets[5..] {
        assert!(voice.get("placeholderOrdinal").is_none());
        assert!(voice.get("filename").is_none());
        assert!(
            export::index_assets(&assets, &HashSet::from([s(voice, "id").to_owned()]))
                .unwrap_err()
                .contains("VOICE_ONLY")
        );
    }
}

#[test]
fn new_colliding_names_are_stable_by_identity_not_order_or_human_name() {
    let initial = vec![
        json!({"id":"person-a/base","identityId":"person-a","variantId":"base","name":"小王","kind":"CH"}),
        json!({"id":"person-b/base","identityId":"person-b","variantId":"base","name":"小王","kind":"CH"}),
        json!({"id":"person-a/injury","identityId":"person-a","variantId":"injury","name":"小王","kind":"CH"}),
        json!({"id":"unicode-a","variantId":"base","name":"Ａ","kind":"PROP"}),
        json!({"id":"unicode-b","variantId":"base","name":"a","kind":"prop"}),
        json!({"id":"unicode-c","variantId":"base","name":"Straße","kind":"prop"}),
        json!({"id":"unicode-d","variantId":"base","name":"STRASSE","kind":"prop"}),
    ];
    let mut a = initial.clone();
    let mut b = initial.clone();
    b.reverse();
    allocate_asset_identities(&mut a).unwrap();
    allocate_asset_identities(&mut b).unwrap();
    for asset in &a {
        assert_eq!(
            asset,
            b.iter().find(|other| other["id"] == asset["id"]).unwrap()
        );
        assert_eq!(
            asset["name"],
            initial
                .iter()
                .find(|other| other["id"] == asset["id"])
                .unwrap()["name"]
        );
    }
    let before = a.clone();
    a.push(json!({"id":"person-c/base","identityId":"person-c","variantId":"base","name":"小王","kind":"CH"}));
    allocate_asset_identities(&mut a).unwrap();
    assert_eq!(&a[..before.len()], before.as_slice());
    let used = a.iter().map(|asset| s(asset, "id").to_owned()).collect();
    assert_eq!(export::index_assets(&a, &used).unwrap().len(), a.len());
}

#[tokio::test]
async fn role_rules_and_packet_content_cannot_be_replaced_by_resigning_receipt() {
    let (input, authored) = fixture();
    let executor = Offline::new(authored);
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["blocked"], json!([]), "{}", out["blocked"]);
    let mut delivery = out["seriesDeliveries"][0].clone();
    let receipt = &mut delivery["adoptionReceipt"]["roleReceipts"][0];
    let mut req: TextRequest = serde_json::from_value(receipt["request"].clone()).unwrap();
    req.rule_snapshot["primary"]["content"] =
        json!("Only a short summary, not the frozen creative rules.");
    *receipt = role_receipt(&req, &receipt["raw"]).unwrap();
    assert_eq!(
        validate_delivery(&input, &delivery).unwrap_err(),
        "SERIES_ROLE_RULE_CONTENT_MISMATCH"
    );
}

// Offline transport with the same persisted request/raw/acceptance shape as the
// real runner. It never calls an account, network, image or video provider.
struct DurableOffline {
    generator: Offline,
    store: crate::store::Store,
    run: String,
}
impl DurableOffline {
    fn transport(request: &TextRequest) -> (Value, String) {
        let snapshot = json!(request);
        let key = digest(&snapshot.to_string());
        (snapshot, key)
    }
    fn child(&self, request: &TextRequest) -> Result<String, String> {
        self.store
            .conn()?
            .query_row(
                "SELECT text_job_id FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2",
                rusqlite::params![self.run, s(&request.input, "executionRoleId")],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }
}
#[async_trait]
impl TextExecutor for DurableOffline {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let (snapshot, key) = Self::transport(&request);
        if let Some(raw) = self.store.accepted_step(&self.run, &key)? {
            return Ok(raw);
        }
        let raw = self.generator.execute(request.clone()).await?;
        let child = self.store.add_job(
            "text",
            &request.stage,
            "offline-fixture",
            "offline-fixture",
            &snapshot,
            "received",
        )?;
        self.store
            .update_job(&child, "received", &json!({"result":raw}))?;
        self.store.event(
            &child,
            "text_received_raw",
            &json!({"requestHash":key,"value":raw,"evidence":"offline transport fixture"}),
        )?;
        self.store.checkpoint(
            &self.run,
            s(&request.input, "executionRoleId"),
            &request.stage,
            &key,
            "received",
            Some(&child),
            Some(&raw),
            None,
        )?;
        Ok(raw)
    }
    async fn accept(&self, request: &TextRequest, raw: &Value) -> Result<(), String> {
        let (_, key) = Self::transport(request);
        if self.store.accepted_step(&self.run, &key)?.is_some() {
            return Ok(());
        }
        let child = self.child(request)?;
        self.store.update_job(&child,"completed",&json!({"result":raw,"receipt":{"contract":"desktop-runner/v1","validation":"accepted","acceptedRequestHash":key,"evidence":"offline transport fixture"}}))?;
        self.store.checkpoint(
            &self.run,
            s(&request.input, "executionRoleId"),
            &request.stage,
            &key,
            "accepted",
            Some(&child),
            Some(raw),
            None,
        )
    }
    async fn reject(&self, request: &TextRequest, raw: &Value, reason: &str) -> Result<(), String> {
        let (_, key) = Self::transport(request);
        let child = self.child(request)?;
        self.store.update_job(&child,"rejected",&json!({"result":raw,"error":reason,"receipt":{"contract":"desktop-runner/v1","validation":"rejected"}}))?;
        self.store.checkpoint(
            &self.run,
            s(&request.input, "executionRoleId"),
            &request.stage,
            &key,
            "rejected",
            Some(&child),
            Some(raw),
            Some(reason),
        )
    }
    async fn publish_episode(&self, output: &Value, input: &Value) -> Result<(), String> {
        let record = self.store.job(&self.run)?;
        let guard = record["prepared"]
            .get("publicationGuard")
            .unwrap_or(&record["snapshot"]["guard"]);
        self.store
            .publish_storyboard_episode(guard, output, input, &self.run)
    }
}
fn durable_fixture() -> (tempfile::TempDir, Value, DurableOffline) {
    let (input, authored) = fixture();
    durable_from(input, authored)
}
fn durable_from(mut input: Value, authored: Value) -> (tempfile::TempDir, Value, DurableOffline) {
    let temporary = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(temporary.path().to_owned()).unwrap();
    store
        .create("新版分镜离线持久化验收", "synthetic-series")
        .unwrap();
    input["source"] = store.save_source(input["source"].clone()).unwrap();
    let source_hash = input["source"]["sha256"].clone();
    input["foundation"]["sourceSha256"] = source_hash;
    input["foundation"] = store.save_foundation(input["foundation"].clone()).unwrap();
    for asset in input["assets"].as_array().unwrap() {
        store.save_asset(asset.clone()).unwrap();
    }
    input["assets"] = json!(store.values("assets").unwrap());
    input["existingUnits"] = json!([]);
    store.ensure_workflow_storage().unwrap();
    let guard = crate::workflow::guard_from_input(&input);
    let run=store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","guard":guard,"input":input}),"running").unwrap();
    (
        temporary,
        input,
        DurableOffline {
            generator: Offline::new(authored),
            store,
            run,
        },
    )
}
fn two_episode_fixture() -> (Value, Value) {
    let (mut input, authored) = fixture();
    let old: LockedSource = serde_json::from_value(input["source"].clone()).unwrap();
    let source = super::super::prepare_source(
        "第一集\n场景：门口\n甲：来了。\n第二集\n场景：门口\n甲：走了。\n",
    )
    .unwrap();
    let old_rows: Vec<_> = old.rows.iter().filter(|r| r.kind == "body").collect();
    let old_q = old_rows.iter().find_map(|r| r.dialogue.as_ref()).unwrap();
    fn replace(value: &mut Value, mapping: &HashMap<String, String>) {
        match value {
            Value::String(v) => {
                if let Some(next) = mapping.get(v) {
                    *v = next.clone();
                }
            }
            Value::Array(a) => {
                for v in a {
                    replace(v, mapping);
                }
            }
            Value::Object(o) => {
                for v in o.values_mut() {
                    replace(v, mapping);
                }
            }
            _ => {}
        }
    }
    let mut candidates = Map::new();
    for episode in &source.episodes {
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|r| r.kind == "body" && r.episode_id.as_deref() == Some(&episode.id))
            .collect();
        let q = rows.iter().find_map(|r| r.dialogue.as_ref()).unwrap();
        let scene = source
            .scenes
            .iter()
            .find(|s| s.episode_id == episode.id)
            .unwrap();
        let mut mapping = HashMap::from([
            (old.episodes[0].id.clone(), episode.id.clone()),
            (old.scenes[0].id.clone(), scene.id.clone()),
            (old_q.id.clone(), q.id.clone()),
        ]);
        for (before, after) in old_rows.iter().zip(&rows) {
            mapping.insert(before.id.clone(), after.id.clone());
        }
        let mut next = authored.clone();
        replace(&mut next, &mapping);
        next["draft"]["units"][0]["shots"][0]["dialogueParts"][0]["quoteStart"] =
            json!(q.range.start);
        next["draft"]["units"][0]["shots"][0]["dialogueParts"][0]["quoteEnd"] = json!(q.range.end);
        candidates.insert(episode.id.clone(), next);
    }
    let episodes: Vec<_> = source.episodes.iter().map(|e| e.id.clone()).collect();
    let scenes: Vec<_> = source.scenes.iter().map(|s| s.id.clone()).collect();
    for asset in input["assets"].as_array_mut().unwrap() {
        asset["applicability"] = json!({"episodes":episodes,"scenes":scenes});
        asset["adoptedDesign"]["sourceSha256"] = json!(source.sha256);
    }
    input["source"] = json!(source);
    input["selection"]["episodeIds"] = json!(episodes);
    input["productionScope"]["episodeIds"] = json!(episodes);
    input["productionScope"]["sourceSha256"] = json!(source.sha256);
    (input, Value::Object(candidates))
}
pub(crate) async fn series_review_entry_replay_fixture() -> (Value, TextRequest, Value) {
    let (input, candidates) = two_episode_fixture();
    let executor = Offline::new(candidates);
    let result = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    assert_eq!(result["blocked"], json!([]));
    let request = executor
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|r| {
            r.stage == "seriesReview" && r.input["episodeId"] == input["selection"]["episodeIds"][1]
        })
        .unwrap()
        .clone();
    let key = digest(&serde_json::to_string(&request).unwrap());
    let mut raw = executor.cache.lock().unwrap()[&key].clone();
    raw["conclusion"] = json!("revise");
    raw["issues"] = json!([{"id":"entry-continuity","severity":"must_fix",
        "unitIds":[binding(&request,"draft")["units"][0]["id"]],
        "sourceRefs":[binding(&request,"source")["sourceRows"][0]["id"],binding(&request,"entry")["sourceRefs"][0]],
        "problem":"当前单元的入口应核对准确前集出口，不修改前集。",
        "requestedChange":"只补当前单元与已供入口的衔接。"}]);
    (input, request, raw)
}
#[tokio::test]
async fn two_episode_resume_keeps_first_adoption_and_only_executes_unfinished_episode() {
    let (input, candidates) = two_episode_fixture();
    let (temporary, input, mut executor) = durable_from(input, candidates.clone());
    executor.generator.stop_episode = input["selection"]["episodeIds"][1]
        .as_str()
        .map(str::to_owned);
    let first = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(
        first["seriesDeliveries"].as_array().unwrap().len(),
        1,
        "{}",
        first["blocked"]
    );
    assert_eq!(first["status"], "partially_blocked");
    let first_id = first["units"][0]["id"].as_str().unwrap();
    let saved = executor.store.get("units", first_id).unwrap();
    let reopened = crate::store::Store::open(temporary.path().to_owned()).unwrap();
    let resumed = DurableOffline {
        generator: Offline::new(candidates),
        store: reopened,
        run: executor.run.clone(),
    };
    let completed = run(&input, &resumed, "offline", "medium").await.unwrap();
    assert_eq!(completed["blocked"], json!([]), "{}", completed["blocked"]);
    assert_eq!(completed["seriesDeliveries"].as_array().unwrap().len(), 2);
    assert_eq!(resumed.store.get("units", first_id).unwrap(), saved);
    let requests = resumed.generator.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests
        .iter()
        .all(|r| r.input["episodeId"] == input["selection"]["episodeIds"][1]));
    let output = tempfile::tempdir().unwrap();
    let exported=resumed.store.export_series_delivery(&json!({"path":output.path().to_string_lossy(),"episodeIds":input["selection"]["episodeIds"]})).unwrap();
    assert_eq!(exported["count"], 2);
    assert_eq!(exported["scopeComplete"], true);
}
#[tokio::test]
async fn durable_publication_reopen_resume_export_and_user_edit_guard() {
    let (temporary, input, executor) = durable_fixture();
    let first = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(first["blocked"], json!([]), "{}", first["blocked"]);
    let uid = first["units"][0]["id"].as_str().unwrap();
    let before = executor.store.get("units", uid).unwrap();
    let jobs = executor.store.jobs().unwrap().len();
    let reopened = crate::store::Store::open(temporary.path().to_owned()).unwrap();
    let resumed = DurableOffline {
        generator: Offline::new(executor.generator.authored.clone()),
        store: reopened.clone(),
        run: executor.run.clone(),
    };
    let second = run(&input, &resumed, "offline", "medium").await.unwrap();
    assert_eq!(first, second);
    assert_eq!(resumed.store.jobs().unwrap().len(), jobs);
    assert!(resumed.generator.requests.lock().unwrap().is_empty());
    assert_eq!(resumed.store.get("units", uid).unwrap(), before);
    let events: i64 = resumed
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE job_id=?1 AND type='episode_published'",
            [&resumed.run],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
    let export_parent = tempfile::tempdir().unwrap();
    let request = json!({"path":export_parent.path().to_string_lossy(),"episodeIds":input["selection"]["episodeIds"]});
    let result = resumed.store.export_series_delivery(&request).unwrap();
    assert_eq!(result["count"], 1);
    assert!(std::path::Path::new(result["manifestPath"].as_str().unwrap()).is_file());
    let mut edited = resumed.store.get("units", uid).unwrap();
    edited["shots"][0]["picture"] = json!("人工新改的镜头，尚未再次独审");
    resumed.store.save_unit(edited).unwrap();
    assert!(resumed
        .store
        .export_series_delivery(&request)
        .unwrap_err()
        .contains("已修改"));
    let remains = resumed.store.get("units", uid).unwrap();
    assert_eq!(
        remains["shots"][0]["picture"],
        "人工新改的镜头，尚未再次独审"
    );
    let saved: Option<String> = resumed
        .store
        .conn()
        .unwrap()
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!(
                "series.delivery:{}",
                input["selection"]["episodeIds"][0].as_str().unwrap()
            )],
            |r| r.get(0),
        )
        .ok();
    assert!(saved.is_some());
}
#[tokio::test]
async fn valid_memory_result_without_saved_raw_cannot_be_adopted() {
    let (_temporary, input, executor) = durable_fixture();
    let memory = Offline::new(executor.generator.authored.clone());
    let out = run(&input, &memory, "offline", "medium").await.unwrap();
    assert_eq!(out["blocked"], json!([]));
    let publication = memory.published.lock().unwrap()[0].clone();
    let guard = crate::workflow::guard_from_input(&input);
    let error = executor
        .store
        .publish_storyboard_episode(&guard, &publication, &input, &executor.run)
        .unwrap_err();
    assert!(error.contains("唯一已接收"));
    assert!(executor.store.values("units").unwrap().is_empty());
}

#[tokio::test]
async fn expanded_scope_reuses_published_prefix_without_copying_role_receipts() {
    let (mut input, candidates) = two_episode_fixture();
    let all = input["selection"]["episodeIds"].clone();
    input["selection"]["episodeIds"] = json!([all[0]]);
    input["productionScope"]["episodeIds"] = json!([all[0]]);
    let (directory, input, executor) = durable_from(input, candidates);
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["blocked"], json!([]), "{}", output["blocked"]);
    let jobs = executor.store.jobs().unwrap();
    let reopened = crate::store::Store::open(directory.path().to_owned()).unwrap();
    let mut expanded = input.clone();
    expanded["selection"]["episodeIds"] = all.clone();
    expanded["productionScope"]["episodeIds"] = all;
    expanded["assets"] = json!(reopened.values("assets").unwrap());
    let reused = reopened.reusable_series_episode_ids(&expanded).unwrap();
    assert_eq!(json!(reused), input["selection"]["episodeIds"]);
    assert_eq!(
        reopened.jobs().unwrap(),
        jobs,
        "read-only reuse allocates no workflow or text job"
    );
    let saved: String = reopened
        .conn()
        .unwrap()
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("series.publication:{}", reused[0])],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&saved).unwrap()["workflowId"],
        executor.run
    );
    for (key, value) in [
        ("confirmedDirection", json!("新的已确认制作方向")),
        ("visualStyle", json!("新画风")),
        ("fantasyMode", json!("all")),
        ("maxUnitSeconds", json!(15)),
    ] {
        let mut changed = expanded.clone();
        changed["settings"][key] = value;
        assert!(
            reopened
                .reusable_series_episode_ids(&changed)
                .unwrap()
                .is_empty(),
            "{key}"
        );
    }
    // A separately started continuation receives the persisted predecessor,
    // which carries DB versions/status absent from an in-run predecessor.
    let mut later = expanded.clone();
    later["selection"]["episodeIds"] = json!([expanded["selection"]["episodeIds"][1]]);
    later["existingUnits"] = json!(reopened.values("units").unwrap());
    later["previousBoundary"] = later["existingUnits"][0].clone();
    later["assets"] = json!(reopened.values("assets").unwrap());
    let guard = crate::workflow::guard_from_input(&later);
    let run_id = reopened.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","guard":guard,"input":later}),"running").unwrap();
    let continuation = DurableOffline {
        generator: Offline::new(executor.generator.authored.clone()),
        store: reopened.clone(),
        run: run_id,
    };
    let second = run(&later, &continuation, "offline", "medium")
        .await
        .unwrap();
    assert_eq!(second["blocked"], json!([]), "{}", second["blocked"]);
    assert_eq!(
        reopened
            .reusable_series_episode_ids(&expanded)
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn reuse_ignores_image_only_versions_but_rejects_stale_design_dependencies() {
    let (mut input, authored) = fixture();
    let mut parent = input["assets"][0].clone();
    parent["id"] = json!("mother");
    parent["name"] = json!("母设计");
    parent["adoptedDesign"]["assetId"] = json!("mother");
    parent["applicability"] = json!({"episodes":["other-episode"]});
    input["assets"][0]["adoptedDesign"]["designDependencies"] =
        json!([{"assetId":"mother","designRevision":1}]);
    input["assets"].as_array_mut().unwrap().push(parent);
    let (_directory, input, executor) = durable_from(input, authored);
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["blocked"], json!([]), "{}", output["blocked"]);
    let mut image_update = executor.store.get("assets", "a").unwrap();
    image_update["prompt"] = json!("新的独立图片提示词，不改完整文字设计");
    executor.store.save_asset(image_update).unwrap();
    assert_eq!(
        executor
            .store
            .reusable_series_episode_ids(&input)
            .unwrap()
            .len(),
        1
    );
    let mut stale = executor.store.get("assets", "mother").unwrap();
    stale["designStatus"] = json!("stale");
    executor.store.save_asset(stale).unwrap();
    assert!(executor
        .store
        .reusable_series_episode_ids(&input)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn reuse_protects_user_edits_and_original_raw_provenance() {
    let (_directory, input, executor) = durable_fixture();
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["blocked"], json!([]));
    let uid = s(&output["units"][0], "id");
    let mut edited = executor.store.get("units", uid).unwrap();
    edited["shots"][0]["picture"] = json!("手动新改的表演");
    executor.store.save_unit(edited).unwrap();
    assert!(executor
        .store
        .reusable_series_episode_ids(&input)
        .unwrap_err()
        .contains("CURRENT_DRAFT_CHANGED"));
    executor
        .store
        .conn()
        .unwrap()
        .execute("DELETE FROM events WHERE type='text_received_raw'", [])
        .unwrap();
    assert!(
        executor.store.reusable_series_episode_ids(&input).is_err(),
        "in-memory bundled receipts cannot replace original received raw"
    );
}

#[tokio::test]
async fn project_style_ratio_and_confirmed_direction_reach_roles_md_and_reuse_guard() {
    let (mut input, authored) = fixture();
    input["settings"]["projectVisualStyle"] = json!("3D国漫，写实材质");
    input["settings"]["visualStyle"] = json!("旧废弃画风");
    input["settings"]["ratio"] = json!("9:16");
    input["settings"]["productionDirection"] = json!("自然冷色电影光");
    input["settings"]["productionDirectionConfirmed"] = json!(true);
    let (_directory, input, executor) = durable_from(input, authored);
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(output["blocked"], json!([]), "{}", output["blocked"]);
    for request in executor.generator.requests.lock().unwrap().iter() {
        let direction = binding(request, "direction");
        assert_eq!(direction["visualStyle"], "3D国漫，写实材质");
        assert_eq!(direction["ratio"], "9:16");
        assert_eq!(direction["productionDirection"], "自然冷色电影光");
    }
    assert!(s(&output["seriesDeliveries"][0], "markdown").contains("3D国漫，写实材质"));
    assert!(!s(&output["seriesDeliveries"][0], "markdown").contains("旧废弃画风"));
    assert_eq!(
        executor
            .store
            .reusable_series_episode_ids(&input)
            .unwrap()
            .len(),
        1
    );
    for (key, value) in [
        ("projectVisualStyle", json!("2D水彩")),
        ("ratio", json!("16:9")),
        ("productionDirection", json!("暖色喜剧")),
    ] {
        let mut changed = input.clone();
        changed["settings"][key] = value;
        assert!(
            executor
                .store
                .reusable_series_episode_ids(&changed)
                .unwrap()
                .is_empty(),
            "{key}"
        );
    }
}

/// Shared only with engine start/CAS regression tests. Every receipt here is a
/// synthetic transport record persisted through the actual publication path.
pub(crate) async fn seed_engine_series_fixture(
    store: &crate::store::Store,
    completed: usize,
) -> Value {
    assert!((1..=2).contains(&completed));
    let (mut input, authored) = two_episode_fixture();
    input["settings"]["productionDirection"] = input["settings"]["confirmedDirection"].clone();
    input["settings"]["productionDirectionConfirmed"] = json!(true);
    input["settings"]["textProvider"] = json!("forbidden-in-offline-test");
    input["foundation"]["productionDirection"] = input["settings"]["confirmedDirection"].clone();
    input["source"] = store.save_source(input["source"].clone()).unwrap();
    input["foundation"]["sourceSha256"] = input["source"]["sha256"].clone();
    input["foundation"] = store.save_foundation(input["foundation"].clone()).unwrap();
    store.save_settings(&input["settings"]).unwrap();
    // Match the real engine's frozen settings, including the stored default
    // 16:9 ratio. The raw model fixture deliberately omits application defaults.
    input["settings"] = store.settings().unwrap();
    for asset in input["assets"].as_array().unwrap() {
        store.save_asset(asset.clone()).unwrap();
    }
    store.ensure_series_asset_identities().unwrap();
    input["assets"] = json!(store.values("assets").unwrap());
    input["existingUnits"] = json!([]);
    input["selection"]["episodeIds"]
        .as_array_mut()
        .unwrap()
        .truncate(completed);
    store.ensure_workflow_storage().unwrap();
    let guard = crate::workflow::guard_from_input(&input);
    let run_id=store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","sourceVersion":input["source"]["version"],"guard":guard,"input":input}),"running").unwrap();
    let executor = DurableOffline {
        generator: Offline::new(authored),
        store: store.clone(),
        run: run_id.clone(),
    };
    let result = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(result["blocked"], json!([]), "{}", result["blocked"]);
    store
        .update_job(&run_id, "completed", &json!({"result":result}))
        .unwrap();
    input
}

#[test]
fn local_repair_preserves_unaffected_units_plan_and_selected_fantasy() {
    let before = json!({"plan":{"id":"adopted-plan"},"selection":["fantasy-a"],"draft":{"units":[{"id":"u1","picture":"需要调整的机位"},{"id":"u2","picture":"保持的表演"}]}});
    let issues = json!([{"severity":"must_fix","unitIds":["u1"]}]);
    let mut allowed = before.clone();
    allowed["draft"]["units"][0]["picture"] = json!("具体机位已调整");
    assert_local_repair(&before, &allowed, &issues).unwrap();
    let mut unrelated = allowed.clone();
    unrelated["draft"]["units"][1]["picture"] = json!("擅改无关单元");
    assert!(assert_local_repair(&before, &unrelated, &issues)
        .unwrap_err()
        .contains("UNAFFECTED"));
    let mut removed = allowed.clone();
    removed["draft"]["units"].as_array_mut().unwrap().pop();
    assert!(assert_local_repair(&before, &removed, &issues).is_err());
    for key in ["plan", "selection"] {
        let mut changed = allowed.clone();
        changed[key] = json!([]);
        assert!(assert_local_repair(&before, &changed, &issues)
            .unwrap_err()
            .contains("PLAN_CHANGED"));
    }
}

#[tokio::test]
async fn repaired_episode_is_independently_reviewed_and_replay_never_buys_another_author() {
    let (_directory, input, mut executor) = durable_fixture();
    executor.generator.review_rounds = 1;
    let first = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(first["blocked"], json!([]), "{}", first["blocked"]);
    let requests = executor.generator.requests.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .map(|r| r.stage.as_str())
            .collect::<Vec<_>>(),
        [
            "seriesAuthor",
            "seriesFinisher",
            "seriesReview",
            "seriesRepair",
            "seriesReview"
        ]
    );
    let final_review = requests.last().unwrap();
    assert_eq!(
        binding(final_review, "draft"),
        &first["seriesDeliveries"][0]["bundle"]["draft"]
    );
    assert_eq!(final_review.input["freshContext"], true);
    assert_ne!(
        final_review.input["executionRoleId"],
        requests[3].input["executionRoleId"]
    );
    let jobs = executor.store.jobs().unwrap().len();
    let resumed = DurableOffline {
        generator: Offline::new(executor.generator.authored.clone()),
        store: executor.store.clone(),
        run: executor.run.clone(),
    };
    let second = run(&input, &resumed, "offline", "medium").await.unwrap();
    assert_eq!(first, second);
    assert!(resumed.generator.requests.lock().unwrap().is_empty());
    assert_eq!(resumed.store.jobs().unwrap().len(), jobs);
}

#[tokio::test]
async fn local_repair_budget_and_no_progress_hold_survive_original_workflow_replay() {
    for repeat_issue in [false, true] {
        let (_directory, input, mut executor) = durable_fixture();
        executor.generator.review_rounds = MAX_REPAIRS + 3;
        executor.generator.repeat_issue = repeat_issue;
        let first = run(&input, &executor, "offline", "medium").await.unwrap();
        assert!(first["blocked"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("REVIEW_REPAIR_HELD"));
        assert!(executor.store.values("units").unwrap().is_empty());
        let count = executor
            .generator
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.stage == "seriesRepair")
            .count();
        assert_eq!(count, if repeat_issue { 1 } else { MAX_REPAIRS });
        let jobs = executor.store.jobs().unwrap().len();
        let resumed = DurableOffline {
            generator: Offline::new(executor.generator.authored.clone()),
            store: executor.store.clone(),
            run: executor.run.clone(),
        };
        let second = run(&input, &resumed, "offline", "medium").await.unwrap();
        assert_eq!(first, second);
        assert!(resumed.generator.requests.lock().unwrap().is_empty());
        assert_eq!(resumed.store.jobs().unwrap().len(), jobs);
    }
}

#[tokio::test]
async fn unknown_local_repair_never_falls_back_to_new_author_or_publishes_unreviewed_text() {
    let (input, authored) = fixture();
    let mut executor = Offline::new(authored);
    executor.review_rounds = 1;
    executor.unknown_repair = true;
    let output = run(&input, &executor, "offline", "medium").await.unwrap();
    assert!(output["blocked"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("SUBMISSION_UNKNOWN"));
    assert!(executor.published.lock().unwrap().is_empty());
    assert_eq!(
        executor
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.stage.as_str())
            .collect::<Vec<_>>(),
        [
            "seriesAuthor",
            "seriesFinisher",
            "seriesReview",
            "seriesRepair"
        ]
    );
}
