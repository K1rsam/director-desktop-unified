use super::*;
#[path = "series_supply_correction_tests.rs"]
pub(crate) mod correction_tests;

fn replace_refs(value: &mut Value, old: &Value, new: &Value) {
    if value == old {
        *value = new.clone();
        return;
    }
    match value {
        Value::Object(map) => {
            for value in map.values_mut() {
                replace_refs(value, old, new)
            }
        }
        Value::Array(values) => {
            for value in values {
                replace_refs(value, old, new)
            }
        }
        _ => {}
    }
}
fn material_input() -> (Value, Value, Value) {
    let (mut input, mut good) = fixture();
    // This fixture is the frozen v1/v2 recovery lineage, not a new workflow.
    input
        .as_object_mut()
        .unwrap()
        .remove(supply_revision::INPUT_FIELD);
    input
        .as_object_mut()
        .unwrap()
        .remove(SOURCE_FACT_CONTRACT_FIELD);
    let old_episode = input["source"]["episodes"][0]["id"].clone();
    let old_scene = input["source"]["scenes"][0]["id"].clone();
    let source = super::super::super::prepare_source(
        "第一集\n场景：门口\n甲手握铜钥匙站在门口。\n甲：来了。\n",
    )
    .unwrap();
    let old_refs = good["draft"]["units"][0]["sourceRefs"].clone();
    let refs = json!(source
        .rows
        .iter()
        .filter(|r| r.kind == "body")
        .map(|r| r.id.clone())
        .collect::<Vec<_>>());
    replace_refs(&mut input, &old_refs, &refs);
    replace_refs(&mut good, &old_refs, &refs);
    for (old, new) in [
        (old_episode, json!(source.episodes[0].id)),
        (old_scene, json!(source.scenes[0].id)),
    ] {
        replace_refs(&mut input, &old, &new);
        replace_refs(&mut good, &old, &new);
    }
    let old_sha = input["source"]["sha256"].clone();
    replace_refs(&mut input, &old_sha, &json!(source.sha256));
    input["source"] = json!(source);
    let quote = source
        .rows
        .iter()
        .find_map(|r| r.dialogue.as_ref())
        .unwrap();
    let part = &mut good["draft"]["units"][0]["shots"][0]["dialogueParts"][0];
    part["lineId"] = json!(quote.id);
    part["quoteStart"] = json!(quote.range.start);
    part["quoteEnd"] = json!(quote.range.end);
    good["draft"]["units"][0]["shots"][0]["picture"] =
        json!("甲手握铜钥匙站在门内，抬眼望向门外，张口说话。");
    let mut author = good.clone();
    author["draft"]["unresolved"] =
        json!([{"coordinate":"U1","problem":"铜钥匙没有完整文字设计，需判断必要图资产或场景细节"}]);
    author["draft"]["units"][0]["shots"][0]["dialogueParts"][0]["quoteStart"] = json!(0);
    author["draft"]["units"][0]["shots"][0]["dialogueParts"][0]["quoteEnd"] =
        json!(quote.quote.encode_utf16().count());
    (input, good, author)
}

#[tokio::test]
async fn supplement_source_fact_summary_is_versioned_and_never_loosens_exact_evidence() {
    for modern in [false, true] {
        let (mut input, good, author) = material_input();
        if modern {
            mark_new_legacy_fixture(&mut input);
        }
        let mut executor = MaterialExecutor::new(good, author);
        if modern {
            executor.supply_assets = Some(input["assets"].as_array().unwrap().clone());
            executor.supply_new_only = true;
        }
        let out = run(&input, &executor, "offline", "medium").await.unwrap();
        assert_eq!(out["status"], "completed", "{}", out["blocked"]);
        let calls = executor.calls.lock().unwrap().clone();
        let (origin, origin_raw) = calls
            .iter()
            .find(|(r, _)| r.stage == "seriesAuthor")
            .unwrap();
        let (request, raw) = calls
            .iter()
            .find(|(r, _)| r.stage == "seriesDesign")
            .unwrap();
        let source = source(&input).unwrap();
        let known = binding(request, "knownAssetCatalog");
        let identities = binding(request, "identityReservations");
        let check = |candidate: &Value| {
            supplement::validate_candidate(
                candidate,
                origin,
                origin_raw,
                known,
                identities,
                &source,
                &source.episodes[0],
            )
        };
        let mut summary = raw.clone();
        summary["proposals"][0]["sourceFacts"][0]["statement"] = json!("甲在门口手持一把铜钥匙。");
        if modern {
            check(&summary).unwrap();
            assert_eq!(
                request.input[SOURCE_FACT_CONTRACT_FIELD],
                SOURCE_FACT_CONTRACT
            );
            assert_eq!(
                request.input["sourceFactFieldSemantics"],
                SOURCE_FACT_INSTRUCTIONS
            );
            assert!(!s(&request.input, "task").contains("sourceFacts必须用原文精确摘句"));
            for fault in ["quote", "empty-fact", "empty-refs", "unknown-row"] {
                let mut bad = summary.clone();
                match fault {
                    "quote" => {
                        bad["proposals"][0]["evidence"][0]["quote"] =
                            json!("甲手握金钥匙站在门口。")
                    }
                    "empty-fact" => bad["proposals"][0]["sourceFacts"][0]["statement"] = json!(" "),
                    "empty-refs" => bad["proposals"][0]["sourceFacts"][0]["sourceRefs"] = json!([]),
                    _ => {
                        bad["proposals"][0]["sourceFacts"][0]["sourceRefs"] =
                            json!(["not-supplied"])
                    }
                }
                assert!(check(&bad).is_err(), "{fault}");
            }
            let mut altered = input.clone();
            altered[SOURCE_FACT_CONTRACT_FIELD] = json!("unrecognized");
            assert_eq!(
                validate_profile(&altered).unwrap_err(),
                "SERIES_SOURCE_FACT_CONTRACT_UNKNOWN"
            );
        } else {
            assert_eq!(
                check(&summary).unwrap_err(),
                "SERIES_SUPPLEMENT_FACT_NOT_SOURCE_QUOTE"
            );
            assert!(request.input.get(SOURCE_FACT_CONTRACT_FIELD).is_none());
            assert!(request.input.get("sourceFactFieldSemantics").is_none());
            assert!(s(&request.input, "task").contains("sourceFacts必须用原文精确摘句"));
            validate_supplement_transition(
                request,
                origin,
                origin_raw,
                Some(origin),
                Some(origin_raw),
                &input,
            )
            .unwrap();
        }
    }
}

#[tokio::test]
async fn existing_supply_complete_facts_follow_new_contract_and_keep_legacy_quotes() {
    for modern in [false, true] {
        let (mut input, good, author) = supply_input(true);
        if modern {
            mark_new_legacy_fixture(&mut input);
        }
        let mut executor = MaterialExecutor::new(good, author);
        executor.supply_assets = Some(input["assets"].as_array().unwrap().clone());
        executor.supply_complete = true;
        let out = run(&input, &executor, "offline", "medium").await.unwrap();
        assert_eq!(out["status"], "completed", "{}", out["blocked"]);
        let calls = executor.calls.lock().unwrap().clone();
        let (origin, origin_raw) = calls
            .iter()
            .find(|(r, _)| r.stage == "seriesAuthor")
            .unwrap();
        let (request, raw) = calls
            .iter()
            .find(|(r, _)| r.stage == "seriesDesign")
            .unwrap();
        let source = source(&input).unwrap();
        let mut summary = raw.clone();
        summary["existingSupply"][0]["sourceFacts"][0]["statement"] =
            json!("甲在门口手持一把铜钥匙。");
        let result = supplement::validate_candidate(
            &summary,
            origin,
            origin_raw,
            binding(request, "knownAssetCatalog"),
            binding(request, "identityReservations"),
            &source,
            &source.episodes[0],
        );
        assert_eq!(result.is_ok(), modern, "{result:?}");
    }
}

#[test]
fn scene_detail_attached_props_require_explicit_current_design_dependency() {
    let (input, _, _) = material_input();
    let source = source(&input).unwrap();
    let mut scene = input["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "s")
        .unwrap()
        .clone();
    let mut prop = scene.clone();
    prop["id"] = json!("attached-prop");
    prop["kind"] = json!("PR");
    prop["adoptedDesign"]["assetId"] = prop["id"].clone();
    scene["adoptedDesign"]["designDependencies"] =
        json!([{"assetId":prop["id"],"designRevision":prop["designRevision"]}]);
    let ids = vec!["s".to_owned(), "attached-prop".to_owned()];
    let before = vec![scene.clone(), prop.clone()];
    assert!(supplement::scene_detail_assets(&ids, &before, &[], &source));
    let revision = |asset: &Value| json!({"assetId":asset["id"],"variantId":asset["variantId"],"baseDesignRevision":asset["designRevision"],"baseDesignSha256":hash(&asset["adoptedDesign"])});
    let mut revisions = vec![revision(&scene), revision(&prop)];
    revisions[0]["designDependencies"] =
        json!([{"assetId":prop["id"],"designRevision":prop["designRevision"].as_u64().unwrap()+1}]);
    assert!(supplement::scene_detail_assets(
        &ids, &before, &revisions, &source
    ));
    for fault in [
        "no-scene",
        "unknown",
        "no-dependency",
        "stale-revision",
        "stale-base",
        "invalid-design",
    ] {
        let mut ids = ids.clone();
        let mut before = before.clone();
        let mut revisions = revisions.clone();
        match fault {
            "no-scene" => {
                ids.remove(0);
            }
            "unknown" => ids.push("foreign-prop".into()),
            "no-dependency" => revisions[0]["designDependencies"] = json!([]),
            "stale-revision" => {
                revisions[0]["designDependencies"][0]["designRevision"] =
                    prop["designRevision"].clone()
            }
            "stale-base" => revisions[1]["baseDesignSha256"] = json!("0".repeat(64)),
            _ => before[1]["designStatus"] = json!("stale"),
        }
        assert!(
            !supplement::scene_detail_assets(&ids, &before, &revisions, &source),
            "{fault}"
        );
    }
    let compact = json!({"id":"selected-scene","kind":"SC"});
    assert!(
        supplement::scene_detail_assets(&["selected-scene".into()], &[compact], &[], &source),
        "legacy existingSupply scene-only selection still loads its complete design later"
    );
}
struct MaterialExecutor {
    normal: Offline,
    good: Value,
    calls: Mutex<Vec<(TextRequest, Value)>>,
    saved: Mutex<HashMap<String, Value>>,
    accepted: Mutex<Vec<String>>,
    rejected: Mutex<Vec<(String, Value)>>,
    revise: bool,
    repeat: bool,
    exhaust: bool,
    normal_revision: bool,
    scene_only: bool,
    broken_evidence: bool,
    unknown: bool,
    boundary_only: bool,
    reservation_extra: Vec<Value>,
    identity_floor: u64,
    extra_fact: bool,
    legacy_review: bool,
    legacy_coverage: bool,
    legacy_design: bool,
    proposal_key: String,
    supply_assets: Option<Vec<Value>>,
    supply_complete: bool,
    supply_new_only: bool,
}
impl MaterialExecutor {
    fn new(good: Value, author: Value) -> Self {
        Self {
            normal: Offline::new(author),
            good,
            calls: Mutex::new(vec![]),
            saved: Mutex::new(HashMap::new()),
            accepted: Mutex::new(vec![]),
            rejected: Mutex::new(vec![]),
            revise: false,
            repeat: false,
            exhaust: false,
            normal_revision: false,
            scene_only: false,
            broken_evidence: false,
            unknown: false,
            boundary_only: false,
            reservation_extra: Vec::new(),
            identity_floor: 0,
            extra_fact: false,
            legacy_review: false,
            legacy_coverage: false,
            legacy_design: false,
            proposal_key: "copper_key".into(),
            supply_assets: None,
            supply_complete: false,
            supply_new_only: false,
        }
    }
    fn candidate(&self, req: &TextRequest) -> Value {
        let row = binding(req, "designSourceRows")
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["text"].as_str().unwrap().contains("手握铜钥匙"))
            .unwrap();
        let evidence = if self.boundary_only {
            json!([])
        } else {
            json!([{"rowId":row["id"],"quote":if self.broken_evidence{"伪造源文"}else{"甲手握铜钥匙站在门口。"}}])
        };
        let mut proposals = if self.scene_only || self.boundary_only {
            json!([])
        } else {
            json!([{"key":self.proposal_key,"name":"铜钥匙","kind":"PR","description":"甲手握并携带的门钥匙","basis":"explicit_source","evidence":evidence,
            "sourceFacts":[{"statement":"甲手握铜钥匙站在门口。","sourceRefs":[row["id"]]}],"productionChoices":["黄铜磨旧质感与单圈匙环为制作补足"],
            "full":"完整道具文字设计：铜钥匙由黄铜匙柄和带齿长杆组成，匙柄为单孔椭圆，挂单圈匙环；单手握住匙柄，齿面向门锁，尺度适合成年人手掌。铜钥匙是源明确物件，齿纹、单圈匙环为制作补足。","brief":"黄铜椭圆柄门钥匙，单圈匙环","designDependencies":[]}])
        };
        if self.extra_fact {
            let other = binding(req, "designSourceRows")
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["text"].as_str().unwrap().contains("来了"))
                .unwrap();
            proposals[0]["sourceFacts"]
                .as_array_mut()
                .unwrap()
                .push(json!({"statement":other["text"],"sourceRefs":[other["id"]]}));
        }
        let mut candidate = json!({"schemaVersion":"series-asset-supplement-candidate/v1","resolutions":[{"unresolvedIndex":0,
            "disposition":if self.boundary_only{"no_visual_asset"}else if self.scene_only{"scene_detail"}else{"new_asset"},"proposalKeys":if self.scene_only||self.boundary_only{json!([])}else{json!([self.proposal_key])},
            "existingAssetIds":if self.scene_only{json!(["s"])}else{json!([])},"evidence":evidence,
            "rationale":if self.scene_only{"合成分支：钥匙只作门锁附近可演出细节，本稿无需特写独立图"}else{"原文在人物手部明确出现，动作需稳定识别的道具"}}],"proposals":proposals});
        if self.supply_assets.is_some() {
            candidate["schemaVersion"] = json!("series-asset-supplement-candidate/v2");
            if self.supply_new_only {
                candidate["existingSupply"] = json!([]);
                return correction_tests::normal_candidate(req, candidate, self.normal_revision);
            }
            let p = &candidate["proposals"][0];
            candidate["existingSupply"] = json!([{"assetId":"existing-key","designMode":if self.supply_complete{"complete"}else{"reuse"},"evidence":evidence,"rationale":"全库已有同一铜钥匙身份，仅本集适用范围漏供", "full":if self.supply_complete{p["full"].clone()}else{json!("")},"brief":if self.supply_complete{p["brief"].clone()}else{json!("")},"sourceFacts":if self.supply_complete{p["sourceFacts"].clone()}else{json!([])},"productionChoices":if self.supply_complete{p["productionChoices"].clone()}else{json!([])},"designDependencies":[]}]);
            candidate["proposals"] = json!([]);
            candidate["resolutions"][0]["disposition"] = json!("existing_asset");
            candidate["resolutions"][0]["proposalKeys"] = json!([]);
            candidate["resolutions"][0]["existingAssetIds"] = json!(["existing-key"]);
        }
        correction_tests::normal_candidate(req, candidate, self.normal_revision)
    }
}
#[async_trait]
impl TextExecutor for MaterialExecutor {
    async fn prepare_transport_request(&self, request: TextRequest) -> Result<TextRequest, String> {
        if self.legacy_design {
            if let Some(legacy) = legacy_supplement_design_request(&request)? {
                return Ok(legacy);
            }
        }
        if self.legacy_review {
            if let Some(legacy) = legacy_supplement_review_request(&request)? {
                return Ok(legacy);
            }
        }
        if self.legacy_coverage {
            if let Some(legacy) = legacy_supplement_coverage_request(&request)? {
                return Ok(legacy);
            }
        }
        Ok(request)
    }
    async fn series_supplement_identity_context(
        &self,
        origin: &TextRequest,
        raw: &Value,
        known: &Value,
        desired_contract: &str,
    ) -> Result<Value, String> {
        let mut reserved: Vec<_> = known
            .as_array()
            .unwrap()
            .iter()
            .filter_map(supplement_identity_reservation)
            .collect();
        reserved.extend(self.reservation_extra.iter().cloned());
        reserved.sort_by(|a, b| s(a, "id").cmp(s(b, "id")));
        let next = reserved
            .iter()
            .filter_map(|a| a["placeholderOrdinal"].as_u64())
            .map(|n| n + 1)
            .max()
            .unwrap_or(0)
            .max(self.identity_floor);
        let mut context = json!({"version":"series-supplement-identities/v1","sourceSha256":origin.input["sourceSha256"],"episodeId":origin.input["episodeId"],
            "originRequestSha256":digest(&serde_json::to_string(origin).unwrap()),"originRawSha256":hash(raw),"knownCatalogSha256":hash(known),"nextOrdinal":next,"reservedAssets":reserved});
        if let Some(assets) = &self.supply_assets {
            let mut states: Vec<_> = assets
                .iter()
                .map(|a| supplement_supply_state(a, s(&origin.input, "sourceSha256")))
                .collect();
            states.sort_by(|a, b| s(a, "assetId").cmp(s(b, "assetId")));
            context["version"] = json!("series-supplement-identities/v2");
            context["contractVersion"] = json!("series-asset-supplement/v2");
            context["assetStates"] = json!(states);
            context["knownCatalogSha256"] = json!(hash(&supplement_supply_catalogue(known)));
            if desired_contract == supply_revision::VERSION {
                context["version"] = json!("series-supplement-identities/v3");
                context["contractVersion"] = json!(supply_revision::VERSION);
            }
        }
        validate_supplement_identity_context(&context, origin, raw, known)?;
        Ok(context)
    }
    async fn execute(&self, req: TextRequest) -> Result<Value, String> {
        let key = hash(&json!(req));
        if let Some(raw) = self.saved.lock().unwrap().get(&key) {
            return Ok(raw.clone());
        }
        let phase = s(&req.input["seriesAssetSupplement"], "phase");
        let raw = match phase {
            "design" => self.candidate(&req),
            "design-review" => {
                if self.unknown {
                    return Err("SUBMISSION_UNKNOWN: original supplement review is retained".into());
                }
                let candidate = binding(&req, "supplementCandidate");
                let round = req.input["seriesAssetSupplement"]["attempt"]
                    .as_u64()
                    .unwrap();
                let revise = self.exhaust || self.repeat || (self.revise && round == 0);
                let mut review = json!({"schemaVersion":"series-asset-supplement-review/v1","candidateSha256":hash(candidate),
                    "reviewedKeys":candidate["proposals"].as_array().unwrap().iter().map(|p|p["key"].clone()).collect::<Vec<_>>(),
                    "reviewedIssueIndexes":[0],"conclusion":if revise{"revise"}else{"pass"},"summary":"synthetic independent review of necessity, source provenance and complete design",
                    "issues":if revise{json!([{"unresolvedIndex":0,"proposalKey":if self.normal_revision||self.scene_only||(self.supply_assets.is_some()&&!self.supply_new_only){""}else{&self.proposal_key},"problem":"需明确钥匙操作轴线","requestedChange":"明确齿面指向锁眼"}])}else{json!([])}});
                if self.exhaust {
                    review["issues"][0]["problem"] =
                        json!(format!("synthetic remaining design use issue {round}"));
                }
                if self.supply_assets.is_some() {
                    review["schemaVersion"] = json!("series-asset-supplement-review/v2");
                    review["reviewedExistingAssetIds"] = if self.supply_new_only {
                        json!([])
                    } else {
                        json!(["existing-key"])
                    };
                }
                if req.input["seriesAssetSupplement"]["version"] == supply_revision::VERSION {
                    review["schemaVersion"] = json!(supply_revision::NORMAL_REVIEW);
                    review["reviewedRevisionAssetIds"] = json!(candidate["revisions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| r["assetId"].clone())
                        .collect::<Vec<_>>());
                }
                review
            }
            "finisher" => {
                let mut raw = self.good.clone();
                if let Some(a) = binding(&req, "assets")
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|a| a["name"] == "铜钥匙")
                {
                    let unit = &mut raw["draft"]["units"][0];
                    unit["props"] = json!(["铜钥匙"]);
                    let refs = unit["sourceRefs"].clone();
                    unit["assetRefs"].as_array_mut().unwrap().push(json!({"assetId":a["id"],"variantId":"base","use":"prop","order":3,"sourceRefs":refs}));
                    unit["imageBindings"]
                        .as_array_mut()
                        .unwrap()
                        .push(a["id"].clone());
                }
                raw
            }
            _ => self.normal.execute(req.clone()).await?,
        };
        self.calls.lock().unwrap().push((req, raw.clone()));
        self.saved.lock().unwrap().insert(key, raw.clone());
        Ok(raw)
    }
    async fn accept(&self, req: &TextRequest, _: &Value) -> Result<(), String> {
        self.accepted
            .lock()
            .unwrap()
            .push(s(&req.input, "executionRoleId").into());
        Ok(())
    }
    async fn reject(&self, req: &TextRequest, raw: &Value, _: &str) -> Result<(), String> {
        let mut saved = self.rejected.lock().unwrap();
        let id = s(&req.input, "executionRoleId").to_owned();
        if let Some((_, prior)) = saved.iter().find(|(old, _)| old == &id) {
            assert_eq!(prior, raw);
        } else {
            saved.push((id, raw.clone()));
        }
        Ok(())
    }
    async fn publish_episode(&self, out: &Value, input: &Value) -> Result<(), String> {
        self.normal.publish_episode(out, input).await
    }
}
pub(crate) async fn series_supplement_fixture() -> (Value, Vec<(TextRequest, Value)>) {
    let (input, good, author) = material_input();
    let mut executor = MaterialExecutor::new(good, author);
    executor.revise = true;
    let out = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let calls = executor.calls.lock().unwrap().clone();
    (input, calls)
}

pub(crate) async fn series_hyphen_supplement_fixture() -> (Value, Vec<(TextRequest, Value)>) {
    let (mut input, good, author) = material_input();
    allocate_asset_identities(input["assets"].as_array_mut().unwrap()).unwrap();
    let mut executor = MaterialExecutor::new(good, author);
    executor.proposal_key = "copper-key".into();
    executor.legacy_design = true;
    executor.supply_assets = Some(input["assets"].as_array().unwrap().clone());
    executor.supply_new_only = true;
    let out = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let calls = executor.calls.lock().unwrap().clone();
    (input, calls)
}

#[tokio::test]
async fn supplement_key_hyphen_and_underscore_keep_exact_links_and_digest_identity() {
    let mut identities = HashMap::new();
    for legacy in [false, true] {
        for key in ["copper_key", "copper-key"] {
            let (input, good, author) = material_input();
            let mut executor = MaterialExecutor::new(good, author);
            executor.proposal_key = key.into();
            executor.legacy_design = legacy;
            let out = run(&input, &executor, "offline", "medium").await.unwrap();
            assert_eq!(out["status"], "completed", "{}", out["blocked"]);
            validate_delivery(&input, &out["seriesDeliveries"][0]).unwrap();
            let calls = executor.calls.lock().unwrap();
            let (origin, _) = &calls[0];
            let (request, candidate) = calls
                .iter()
                .find(|(r, _)| r.stage == "seriesDesign")
                .unwrap();
            assert_eq!(candidate["proposals"][0]["key"], key);
            assert_eq!(candidate["resolutions"][0]["proposalKeys"], json!([key]));
            let (_, review) = calls
                .iter()
                .find(|(r, _)| r.stage == "seriesDesignReview")
                .unwrap();
            assert_eq!(review["reviewedKeys"], json!([key]));
            assert_eq!(review["candidateSha256"], hash(candidate));
            let property = &request.schema["properties"]["proposals"]["items"]["properties"]["key"];
            if legacy {
                assert_eq!(*property, string());
            } else {
                assert_eq!(
                    *property,
                    json!({"type":"string","pattern":"^[a-z][a-z0-9_-]{0,63}$","minLength":1,"maxLength":64})
                );
                let old = legacy_supplement_design_request(request).unwrap().unwrap();
                assert_eq!(old.input, request.input);
                assert_eq!(old.rule_snapshot, request.rule_snapshot);
                assert!(legacy_supplement_design_request(&old).unwrap().is_none());
                let mut invalid = request.clone();
                invalid.schema["properties"]["proposals"]["items"]["properties"]["key"]
                    ["pattern"] = json!(".*");
                assert_eq!(
                    legacy_supplement_design_request(&invalid).unwrap_err(),
                    "SERIES_SUPPLEMENT_KEY_SCHEMA_UNBOUND"
                );
            }
            let added =
                &out["seriesDeliveries"][0]["adoptionReceipt"]["assetSupplement"]["addedAssets"][0];
            let expected = digest(&format!(
                "{}:{}:{}:{}",
                s(&input["source"], "sha256"),
                s(&origin.input, "episodeId"),
                digest(&serde_json::to_string(origin).unwrap()),
                key
            ));
            assert_eq!(added["id"], format!("series-material-{}", &expected[..24]));
            assert_eq!(added["supplementKey"], key);
            assert_eq!(
                added["filename"], "道具-铜钥匙.png",
                "keys never become file paths"
            );
            if let Some(prior) = identities.insert(key.to_owned(), added["id"].clone()) {
                assert_eq!(
                    prior, added["id"],
                    "transport schema must not change opaque-key identity"
                );
            }
        }
    }
    assert_ne!(
        identities["copper-key"], identities["copper_key"],
        "do not normalize or alias distinct keys"
    );
}

#[tokio::test]
async fn supplement_key_rejects_paths_invalid_start_and_overflow_without_review() {
    let invalid = [
        "",
        "9key",
        "a/b",
        "a\\b",
        "a..b",
        "../a",
        "a:b",
        "a b",
        "A-key",
        "a\nkey",
        &"a".repeat(65),
    ];
    for key in invalid {
        let (input, good, author) = material_input();
        let mut executor = MaterialExecutor::new(good, author);
        executor.proposal_key = key.into();
        let out = run(&input, &executor, "offline", "medium").await.unwrap();
        assert!(
            out["blocked"]
                .to_string()
                .contains("SERIES_SUPPLEMENT_IDENTITY_INVALID"),
            "key={key:?}: {}",
            out["blocked"]
        );
        assert!(out["seriesDeliveries"].as_array().unwrap().is_empty());
        let calls = executor.calls.lock().unwrap();
        assert!(!calls.iter().any(|(r, _)| r.stage == "seriesDesignReview"));
        let (_, candidate) = calls
            .iter()
            .find(|(r, _)| r.stage == "seriesDesign")
            .unwrap();
        assert_eq!(
            candidate["proposals"][0]["key"], key,
            "invalid raw is retained without sanitizing"
        );
    }
}

#[tokio::test]
async fn supplement_key_v2_legacy_raw_revalidation_is_narrow_and_still_checks_references() {
    let (input, calls) = series_hyphen_supplement_fixture().await;
    let (origin, origin_raw) = &calls[0];
    let (request, raw) = &calls[1];
    assert_eq!(
        request.schema["properties"]["proposals"]["items"]["properties"]["key"],
        string()
    );
    let before = json!([request, raw, origin, origin_raw, input]);
    revalidate_supplement_key_response(request, raw, origin, origin_raw, &input).unwrap();
    assert_eq!(json!([request, raw, origin, origin_raw, input]), before);
    for (case, expected) in [
        ("references", "SERIES_SUPPLEMENT_RESOLUTION_UNKNOWN_ASSET"),
        ("path", "SERIES_SUPPLEMENT_IDENTITY_INVALID"),
        ("no-hyphen", "SERIES_SUPPLEMENT_KEY_REVALIDATION_SCOPE"),
        ("current-schema", "SERIES_SUPPLEMENT_KEY_REVALIDATION_SCOPE"),
        ("later-attempt", "SERIES_SUPPLEMENT_KEY_REVALIDATION_SCOPE"),
    ] {
        let mut bad_request = request.clone();
        let mut bad_raw = raw.clone();
        match case {
            "references" => bad_raw["resolutions"][0]["proposalKeys"] = json!(["copper_key"]),
            "path" => {
                bad_raw["proposals"][0]["key"] = json!("copper-../key");
                bad_raw["resolutions"][0]["proposalKeys"] = json!(["copper-../key"]);
            }
            "no-hyphen" => {
                bad_raw["proposals"][0]["key"] = json!("copper_key");
                bad_raw["resolutions"][0]["proposalKeys"] = json!(["copper_key"]);
            }
            "current-schema" => {
                bad_request.schema["properties"]["proposals"]["items"]["properties"]["key"] = json!({"type":"string","pattern":"^[a-z][a-z0-9_-]{0,63}$","minLength":1,"maxLength":64})
            }
            "later-attempt" => bad_request.input["seriesAssetSupplement"]["attempt"] = json!(1),
            _ => unreachable!(),
        }
        assert_eq!(
            revalidate_supplement_key_response(&bad_request, &bad_raw, origin, origin_raw, &input)
                .unwrap_err(),
            expected,
            "{case}"
        );
    }
}

fn supply_input(complete: bool) -> (Value, Value, Value) {
    let (mut input, good, author) = material_input();
    let mut asset = input["assets"][0].clone();
    asset["id"] = json!("existing-key");
    asset["name"] = json!("铜钥匙");
    asset["kind"] = json!("PR");
    asset["applicability"] = json!({"episodes":["previous-episode-only"]});
    asset["imagePrompt"] = json!("手写提示词保持原样");
    asset["currentMediaId"] = json!("approved-existing-image");
    asset["adoptedDesign"]["assetId"] = asset["id"].clone();
    asset["adoptedDesign"]["full"]=json!("铜钥匙完整旧设计：黄铜钥匙、椭圆单孔柄与三齿短杆，尺寸可手握；保留原来的完整设计，不能由补料模型重写。");
    asset["adoptedDesign"]["brief"] = json!("旧铜钥匙定位");
    asset["adoptedDesign"]["legacyNote"] = json!("额外旧设计字段也必须保留");
    if complete {
        asset["designStatus"] = json!("needs_review");
    }
    input["assets"].as_array_mut().unwrap().push(asset);
    allocate_asset_identities(input["assets"].as_array_mut().unwrap()).unwrap();
    (input, good, author)
}

pub(crate) async fn series_supply_fixture() -> (Value, Vec<(TextRequest, Value)>) {
    let (input, good, author) = supply_input(false);
    let mut executor = MaterialExecutor::new(good, author);
    executor.supply_assets = Some(input["assets"].as_array().unwrap().clone());
    let out = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let calls = executor.calls.lock().unwrap().clone();
    (input, calls)
}

#[tokio::test]
async fn v2_prior_published_identity_can_be_supplied_beyond_initial_registry_but_requires_frozen_state(
) {
    let (mut input, calls) = series_supply_fixture().await;
    input["assets"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["id"] != "existing-key");
    let (origin, origin_raw) = &calls[0];
    for index in 1..=3 {
        let (req, _) = &calls[index];
        let (prior, prior_raw) = &calls[index - 1];
        validate_supplement_transition(
            req,
            origin,
            origin_raw,
            Some(prior),
            Some(prior_raw),
            &input,
        )
        .unwrap();
    }
    let req = &calls[2].0;
    let known = binding(req, "knownAssetCatalog");
    let context = binding(req, "identityReservations");
    assert!(known
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == "existing-key"));
    let mut changed = context.clone();
    changed["assetStates"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["assetId"] != "existing-key");
    assert!(
        validate_supplement_identity_context(&changed, origin, origin_raw, known)
            .unwrap_err()
            .contains("STATE_SCOPE")
    );
    let mut sources = binding(req, "existingSupplySources").clone();
    sources[0]["adoptedDesign"]["full"] = json!("被换掉的旧设计");
    let source = source(&input).unwrap();
    assert!(
        supply::validate_source_cards(&calls[1].1, &sources, context, &source)
            .unwrap_err()
            .contains("SOURCE_CARD_CHANGED")
    );
    let task = s(&calls[3].0.input, "task");
    assert!(task.contains("必须来自该单元 sourceRefs"));
    assert!(task.contains("不能把其他单元或其他集的设计来源抄入当前单元"));
}

#[tokio::test]
async fn v2_existing_supply_reuses_complete_design_without_rewriting_applicability_or_buying_author(
) {
    let (input, good, author) = supply_input(false);
    let old = input["assets"][2].clone();
    let mut executor = MaterialExecutor::new(good, author.clone());
    executor.supply_assets = Some(input["assets"].as_array().unwrap().clone());
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let delivery = &out["seriesDeliveries"][0];
    let proof = &delivery["adoptionReceipt"]["assetSupplement"];
    assert_eq!(proof["version"], "series-asset-supplement/v2");
    assert_eq!(proof["origin"]["raw"], author);
    assert_eq!(proof["addedAssets"], json!([]));
    assert_eq!(
        proof["suppliedExistingAssets"][0]["adoptedDesign"],
        old["adoptedDesign"]
    );
    assert_eq!(
        proof["suppliedExistingAssets"][0]["applicability"],
        old["applicability"]
    );
    assert!(out["designAdoptions"].as_array().unwrap().is_empty());
    validate_delivery(&input, delivery).unwrap();
    let source = source(&input).unwrap();
    let episode = &source.episodes[0];
    let bundle = &delivery["bundle"];
    let candidate = json!({"schemaVersion":"series-episode-candidate/v1","plan":bundle["plan"],"selection":bundle["selection"],"draft":bundle["draft"]});
    let cards = bundle["assets"].as_array().unwrap();
    assert_eq!(
        validate_candidate(&candidate, &source, episode, cards, &input).unwrap_err(),
        crate::text_executor::AUTHOR_ASSET_SCOPE_REJECTION
    );
    assert_eq!(
        validate_candidate_with_supply(
            &candidate,
            &source,
            episode,
            cards,
            &input,
            Some(&proof["episodeSupply"])
        )
        .unwrap(),
        candidate
    );
    assert_eq!(cards[2]["applicability"], old["applicability"]);
    let mut changed = delivery.clone();
    changed["adoptionReceipt"]["assetSupplement"]["episodeSupply"]["items"][0]["evidence"][0]
        ["quote"] = json!("自行增加的范围");
    assert!(validate_delivery(&input, &changed)
        .unwrap_err()
        .contains("EPISODE_USE_CHANGED"));
    assert!(delivery_inputs_reusable(&input, &input, delivery, &Value::Null).unwrap());
    let calls = executor.calls.lock().unwrap();
    let design = &calls[1].0;
    let review = &calls[2].0;
    let finisher = &calls[3].0;
    let light = binding(design, "knownAssetCatalog").to_string();
    assert!(!light.contains("完整旧设计"));
    assert!(!light.contains("sourceFacts"));
    assert!(!light.contains("preparation"));
    assert_eq!(
        binding(design, "source")["sourceRowsBinding"],
        "designSourceRows"
    );
    assert!(binding(design, "source").get("sourceRows").is_none());
    assert_eq!(
        binding(review, "existingSupplySources")[0]["adoptedDesign"],
        old["adoptedDesign"]
    );
    assert_eq!(
        binding(finisher, "assets")[2]["adoptedDesign"],
        old["adoptedDesign"]
    );
    assert!(!finisher.input["packet"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["kind"] == "existingSupplySources"));
    assert_eq!(
        calls
            .iter()
            .filter(|(r, _)| r.stage == "seriesAuthor")
            .count(),
        1
    );
    assert!(legacy_supplement_review_request(review).unwrap().is_none());
}

#[test]
fn v2_supply_scope_is_limited_to_evidence_scenes_and_rejects_cross_episode_or_forged_evidence() {
    let source = super::super::super::prepare_source(
        "第一集\n场景：门口\n甲拿铜钥匙。\n场景：厨房\n甲端水。\n第二集\n场景：门口\n甲出门。\n",
    )
    .unwrap();
    let episode = &source.episodes[0];
    let row = source
        .rows
        .iter()
        .find(|r| r.text.trim() == "甲拿铜钥匙。")
        .unwrap();
    let original = json!({"id":"key","applicability":{"episodes":["old"]},"scope":{"kind":"SCENES","scenes":[{"episodeId":"old","sceneId":"old-scene"}]}});
    let usage = json!({"version":"series-existing-episode-supply/v1","episodeId":episode.id,"items":[{"assetId":"key","evidence":[{"rowId":row.id,"quote":row.text}]}]});
    let projected =
        supply::scoped_assets(&source, episode, &[original.clone()], Some(&usage)).unwrap();
    assert_eq!(
        projected[0]["applicability"],
        json!({"episodes":[episode.id],"scenes":[row.scene_id]})
    );
    assert_eq!(original["applicability"], json!({"episodes":["old"]}));
    assert_eq!(projected[0]["scope"]["scenes"].as_array().unwrap().len(), 1);
    let mut wrong = usage.clone();
    wrong["episodeId"] = json!(source.episodes[1].id);
    assert!(
        supply::scoped_assets(&source, episode, &[original.clone()], Some(&wrong))
            .unwrap_err()
            .contains("EPISODE_MISMATCH")
    );
    wrong = usage.clone();
    wrong["items"][0]["evidence"][0]["quote"] = json!("不存在的动作");
    assert!(
        supply::scoped_assets(&source, episode, &[original.clone()], Some(&wrong))
            .unwrap_err()
            .contains("QUOTE_CHANGED")
    );
    wrong = usage;
    wrong["items"][0]["evidence"][0]["rowId"] = json!(
        source
            .rows
            .iter()
            .find(|r| r.text.trim() == "甲出门。")
            .unwrap()
            .id
    );
    assert!(
        supply::scoped_assets(&source, episode, &[original], Some(&wrong))
            .unwrap_err()
            .contains("ROW_UNKNOWN")
    );
}

#[tokio::test]
async fn v2_existing_supply_completes_same_identity_with_independent_review_and_next_revision() {
    let (input, good, author) = supply_input(true);
    let mut executor = MaterialExecutor::new(good, author);
    executor.supply_complete = true;
    executor.supply_assets = Some(input["assets"].as_array().unwrap().clone());
    executor.extra_fact = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let delivery = &out["seriesDeliveries"][0];
    let proof = &delivery["adoptionReceipt"]["assetSupplement"];
    let supplied = &proof["suppliedExistingAssets"][0];
    assert_eq!(proof["addedAssets"], json!([]));
    assert_eq!(supplied["id"], "existing-key");
    assert_eq!(supplied["designRevision"], 2);
    assert_eq!(out["designAdoptions"][0]["baseDesignRevision"], 1);
    assert_eq!(
        supplied["adoptedDesign"]["sourceRefs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        supplied["applicability"],
        input["assets"][2]["applicability"]
    );
    let mut current = input.clone();
    current["assets"][2]["adoptedDesign"] = supplied["adoptedDesign"].clone();
    current["assets"][2]["designRevision"] = json!(2);
    current["assets"][2]["designStatus"] = json!("adopted");
    assert!(delivery_inputs_reusable(&current, &input, delivery, &Value::Null).unwrap());
    assert!(!delivery_inputs_reusable(&input, &input, delivery, &Value::Null).unwrap());
}

#[tokio::test]
async fn missing_asset_is_designed_reviewed_and_appended_without_rebuying_or_accepting_author() {
    let (input, good, author) = material_input();
    let executor = MaterialExecutor::new(good, author.clone());
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let delivery = &out["seriesDeliveries"][0];
    let proof = &delivery["adoptionReceipt"]["assetSupplement"];
    assert_eq!(proof["origin"]["raw"], author);
    assert_eq!(proof["origin"]["validation"], "rejected");
    let before = proof["beforeAssets"].as_array().unwrap();
    let after = delivery["bundle"]["assets"].as_array().unwrap();
    assert_eq!(&after[..before.len()], before);
    assert_eq!(after.len(), 3);
    assert_eq!(after[2]["name"], "铜钥匙");
    assert_eq!(after[2]["placeholderOrdinal"], 2);
    assert_eq!(after[2]["filename"], "道具-铜钥匙.png");
    assert!(!executor
        .accepted
        .lock()
        .unwrap()
        .iter()
        .any(|id| id.contains("/author/")));
    let calls = executor.calls.lock().unwrap().clone();
    assert_eq!(
        calls
            .iter()
            .map(|(r, _)| r.stage.as_str())
            .collect::<Vec<_>>(),
        [
            "seriesAuthor",
            "seriesDesign",
            "seriesDesignReview",
            "seriesFinisher",
            "seriesReview"
        ]
    );
    assert_eq!(binding(&calls[3].0, "draft"), &author["draft"]);
    assert_eq!(
        out["units"][0]["shots"][0]["dialogueParts"][0]["text"],
        "来了。"
    );
    let again = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(again["seriesDeliveries"], out["seriesDeliveries"]);
    assert_eq!(executor.calls.lock().unwrap().len(), calls.len());
    validate_delivery(&input, delivery).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("supplement-delivery");
    let exported =
        export_delivery_package(&input, out["seriesDeliveries"].as_array().unwrap(), &target)
            .unwrap();
    assert_eq!(exported["imageCount"], 3);
    let read = |name: &str| -> Value {
        serde_json::from_slice(&std::fs::read(target.join(name)).unwrap()).unwrap()
    };
    assert_eq!(
        read("资料/registry/initial.json")["assets"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let current = read("资料/registry/current.json");
    assert_eq!(current["assets"].as_array().unwrap().len(), 3);
    assert_eq!(
        current["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "铜钥匙")
            .unwrap()["id"],
        after[2]["id"]
    );
    let store = crate::store::Store::open(dir.path().join("imported")).unwrap();
    store.create("补料包独立导入", "supplement-import").unwrap();
    let md = std::fs::read_to_string(target.join("全剧_灵墨v7.md")).unwrap();
    crate::lingmo_import::import(&store,&json!({"path":target.join("全剧_灵墨v7.md").to_string_lossy(),"expectedSha256":digest(&md)})).unwrap();
    assert_eq!(store.values("units").unwrap().len(), 1);
    assert_eq!(store.values("assets").unwrap().len(), 3);
    assert!(store.job_summaries().unwrap().is_empty());
}
#[tokio::test]
async fn scene_detail_resolution_needs_independent_review_but_no_artificial_asset() {
    let (input, good, author) = material_input();
    let mut executor = MaterialExecutor::new(good, author);
    executor.scene_only = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    assert_eq!(
        out["seriesDeliveries"][0]["bundle"]["assets"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        out["seriesDeliveries"][0]["adoptionReceipt"]["assetSupplement"]["addedAssets"],
        json!([])
    );
    assert_eq!(
        executor
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(r, _)| r.stage == "seriesDesignReview")
            .count(),
        1
    );
}

#[tokio::test]
async fn unverified_voice_media_is_a_text_delivery_boundary_not_a_required_generation() {
    let (input, good, mut author) = material_input();
    author["draft"]["unresolved"] = json!([{"coordinate":"draft.units[*].shots[*].dialogueParts","problem":"声窗按完整原句和表演联排文字估算，尚无实际配音或动作演练，不能声明成片自然发声已验收"}]);
    let mut executor = MaterialExecutor::new(good, author);
    executor.boundary_only = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let proof = &out["seriesDeliveries"][0]["adoptionReceipt"]["assetSupplement"];
    assert_eq!(proof["addedAssets"], json!([]));
    let calls = executor.calls.lock().unwrap();
    for (req, _) in calls
        .iter()
        .filter(|(req, _)| !req.input["seriesAssetSupplement"].is_null())
    {
        assert!(s(&req.input, "task").contains("不得要求先生成媒体"));
        assert!(s(&req.input, "task").contains("不得宣称成片自然发声已验收"));
    }
    assert_eq!(
        calls[1].1["resolutions"][0]["disposition"],
        "no_visual_asset"
    );
}

#[tokio::test]
async fn frozen_historical_floor_and_names_are_applied_before_finisher_or_publication() {
    let (input, good, author) = material_input();
    let mut executor = MaterialExecutor::new(good, author);
    executor.identity_floor = 44;
    executor.reservation_extra = vec![
        json!({"id":"historical-deleted-key","variantId":"base","kind":"PR","importName":"铜钥匙","filename":"道具-铜钥匙.png","placeholderOrdinal":40}),
    ];
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let proof = &out["seriesDeliveries"][0]["adoptionReceipt"]["assetSupplement"];
    assert_eq!(proof["identityReservations"]["nextOrdinal"], 44);
    assert_eq!(proof["addedAssets"][0]["placeholderOrdinal"], 44);
    assert_ne!(proof["addedAssets"][0]["filename"], "道具-铜钥匙.png");
    let calls = executor.calls.lock().unwrap();
    for (req, _) in calls
        .iter()
        .filter(|(req, _)| !req.input["seriesAssetSupplement"].is_null())
    {
        assert_eq!(
            binding(req, "identityReservations"),
            &proof["identityReservations"]
        );
    }
}
#[tokio::test]
async fn unknown_supplement_review_retains_origin_and_never_starts_finisher() {
    let (input, good, author) = material_input();
    let mut executor = MaterialExecutor::new(good, author);
    executor.unknown = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert!(out["blocked"].to_string().contains("SUBMISSION_UNKNOWN"));
    assert_eq!(out["seriesDeliveries"], json!([]));
    assert_eq!(executor.calls.lock().unwrap().len(), 2);
    assert!(!executor
        .accepted
        .lock()
        .unwrap()
        .iter()
        .any(|id| id.contains("/author/")));
}
#[tokio::test]
async fn fabricated_source_evidence_cannot_create_a_design_or_hide_the_missing_asset() {
    let (input, good, author) = material_input();
    let mut executor = MaterialExecutor::new(good, author);
    executor.broken_evidence = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert!(out["blocked"].to_string().contains("SOURCE_QUOTE_CHANGED"));
    assert_eq!(executor.calls.lock().unwrap().len(), 2);
    assert_eq!(out["seriesDeliveries"], json!([]));
}

#[tokio::test]
async fn exact_same_episode_fact_can_add_its_own_row_without_duplicating_proposal_evidence() {
    let (input, good, author) = material_input();
    let mut executor = MaterialExecutor::new(good, author);
    executor.extra_fact = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let added = &out["seriesDeliveries"][0]["adoptionReceipt"]["assetSupplement"]["addedAssets"][0];
    assert_eq!(added["sourceRefs"].as_array().unwrap().len(), 2);
    assert_eq!(added["adoptedDesign"]["sourceRefs"], added["sourceRefs"]);
    let calls = executor.calls.lock().unwrap();
    assert_eq!(
        calls[1].1["proposals"][0]["evidence"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn review_digest_is_program_bound_and_frozen_legacy_review_keeps_its_original_shape() {
    for legacy in [false, true] {
        let (input, good, author) = material_input();
        let mut executor = MaterialExecutor::new(good, author);
        executor.legacy_review = legacy;
        let out = run(&input, &executor, "offline", "medium").await.unwrap();
        assert_eq!(out["status"], "completed", "{}", out["blocked"]);
        let calls = executor.calls.lock().unwrap();
        let review = &calls
            .iter()
            .find(|(r, _)| r.stage == "seriesDesignReview")
            .unwrap()
            .0;
        if legacy {
            assert!(review.input.get("reviewDigestBinding").is_none());
            assert!(review.input.get("candidateSha256").is_none());
            assert_eq!(review.schema["properties"]["candidateSha256"], string());
        } else {
            assert_eq!(
                review.input["candidateSha256"],
                review.input["seriesAssetSupplement"]["predecessorRawSha256"]
            );
            assert_eq!(
                review.schema["properties"]["candidateSha256"]["enum"],
                json!([review.input["candidateSha256"]])
            );
            let mut changed = review.clone();
            changed.input["candidateSha256"] = json!("0".repeat(64));
            assert!(legacy_supplement_review_request(&changed).is_err());
            let original = legacy_supplement_review_request(review).unwrap().unwrap();
            assert_eq!(original.input["packet"], review.input["packet"]);
            assert_eq!(original.rule_snapshot, review.rule_snapshot);
        }
        validate_delivery(&input, &out["seriesDeliveries"][0]).unwrap();
    }
}
#[tokio::test]
async fn supplement_review_coverage_preserves_v1_v2_frozen_digest_and_predigest_requests() {
    for (v2, legacy_coverage, legacy_digest) in [
        (false, false, false),
        (false, true, false),
        (false, true, true),
        (true, false, false),
        (true, true, false),
    ] {
        let (input, good, author) = if v2 {
            supply_input(false)
        } else {
            material_input()
        };
        let mut executor = MaterialExecutor::new(good, author);
        executor.legacy_coverage = legacy_coverage;
        executor.legacy_review = legacy_digest;
        if v2 {
            executor.supply_assets = Some(input["assets"].as_array().unwrap().clone());
        }
        let out = run(&input, &executor, "offline", "medium").await.unwrap();
        assert_eq!(out["status"], "completed", "{}", out["blocked"]);
        validate_delivery(&input, &out["seriesDeliveries"][0]).unwrap();
        let calls = executor.calls.lock().unwrap();
        let review = &calls
            .iter()
            .find(|(r, _)| r.stage == "seriesDesignReview")
            .unwrap()
            .0;
        assert_eq!(
            review.input.get("reviewCoverageBinding").is_none(),
            legacy_coverage
        );
        assert_eq!(
            review.input.get("reviewDigestBinding").is_none(),
            legacy_digest
        );
        if !legacy_coverage {
            let expected = &review.input["expectedReviewCoverage"];
            assert_eq!(expected["reviewedIssueIndexes"], json!([0]));
            assert_eq!(
                expected["reviewedKeys"],
                if v2 { json!([]) } else { json!(["copper_key"]) }
            );
            if v2 {
                assert_eq!(
                    expected["reviewedExistingAssetIds"],
                    json!(["existing-key"])
                );
            } else {
                assert!(expected.get("reviewedExistingAssetIds").is_none());
            }
            let old = legacy_supplement_coverage_request(review).unwrap().unwrap();
            assert_eq!(old.input["packet"], review.input["packet"]);
            assert_eq!(old.input["task"], review.input["task"]);
            assert_eq!(old.rule_snapshot, review.rule_snapshot);
            assert_eq!(
                old.input["candidateSha256"],
                review.input["candidateSha256"]
            );
            assert_eq!(old.schema["properties"]["summary"], string());
            assert!(legacy_supplement_coverage_request(&old).unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn supplement_review_coverage_rejects_tampered_guidance_scope_and_schema() {
    let (input, calls) = series_supply_fixture().await;
    let (origin, origin_raw) = &calls[0];
    let (design, candidate) = &calls[1];
    let request = &calls[2].0;
    for mutation in 0..6 {
        let mut changed = request.clone();
        match mutation {
            0 => changed.input["reviewCoverageBinding"] = json!("unknown-version"),
            1 => changed.input["expectedReviewCoverage"]["reviewedExistingAssetIds"] = json!([]),
            2 => changed.input["expectedReviewCoverage"]["reviewedIssueIndexes"] = json!([1]),
            3 => changed.input["reviewResponseContract"] = json!("直接通过"),
            4 => changed.schema["properties"]["summary"] = string(),
            _ => {
                changed
                    .input
                    .as_object_mut()
                    .unwrap()
                    .remove("reviewCoverageBinding");
            }
        }
        assert!(
            legacy_supplement_coverage_request(&changed).is_err(),
            "mutation={mutation}"
        );
        assert!(
            validate_supplement_transition(
                &changed,
                origin,
                origin_raw,
                Some(design),
                Some(candidate),
                &input
            )
            .is_err(),
            "mutation={mutation}"
        );
    }
}
#[tokio::test]
async fn supplemental_review_repair_is_bounded_and_replays_saved_requests() {
    let (input, good, author) = material_input();
    let mut executor = MaterialExecutor::new(good, author);
    executor.revise = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    assert_eq!(
        out["seriesDeliveries"][0]["adoptionReceipt"]["assetSupplement"]["rounds"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(executor.calls.lock().unwrap().len(), 7);
    let again = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(again["seriesDeliveries"], out["seriesDeliveries"]);
    assert_eq!(executor.calls.lock().unwrap().len(), 7);
}
#[tokio::test]
async fn repeated_supplement_issue_stops_before_new_round_or_new_author() {
    let (input, good, author) = material_input();
    let mut executor = MaterialExecutor::new(good, author);
    executor.repeat = true;
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert!(out["blocked"]
        .to_string()
        .contains("SUPPLEMENT_REVIEW_REPAIR_HELD"));
    assert_eq!(executor.calls.lock().unwrap().len(), 5);
    assert_eq!(out["seriesDeliveries"], json!([]));
}
#[tokio::test]
async fn final_proof_rejects_mutated_existing_design_and_promoted_original_author() {
    let (input, good, author) = material_input();
    let executor = MaterialExecutor::new(good, author);
    let out = run(&input, &executor, "offline", "medium").await.unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let delivery = &out["seriesDeliveries"][0];
    let mut changed = delivery.clone();
    changed["adoptionReceipt"]["assetSupplement"]["beforeAssets"][0]["adoptedDesign"]["full"] =
        json!("偷偷换成短卡");
    assert!(validate_delivery(&input, &changed)
        .unwrap_err()
        .contains("ORIGIN_EVIDENCE_CHANGED"));
    let mut changed = delivery.clone();
    changed["adoptionReceipt"]["assetSupplement"]["addedAssets"][0]["placeholderOrdinal"] =
        json!(0);
    assert!(validate_delivery(&input, &changed)
        .unwrap_err()
        .contains("NOT_APPEND_ONLY"));
    let mut changed = delivery.clone();
    changed["adoptionReceipt"]["assetSupplement"]["origin"]["validation"] = json!("accepted");
    assert!(validate_delivery(&input, &changed)
        .unwrap_err()
        .contains("ORIGIN_EVIDENCE_CHANGED"));
}

async fn durable_material_result() -> (
    tempfile::TempDir,
    Value,
    DurableOffline,
    Value,
    Vec<(TextRequest, Value)>,
) {
    durable_material_result_with_supply(None).await
}
async fn durable_material_result_with_supply(
    supply: Option<bool>,
) -> (
    tempfile::TempDir,
    Value,
    DurableOffline,
    Value,
    Vec<(TextRequest, Value)>,
) {
    let (mut input, good, author) = supply.map(supply_input).unwrap_or_else(material_input);
    allocate_asset_identities(input["assets"].as_array_mut().unwrap()).unwrap();
    let (dir, input, persistor) = durable_from(input, author.clone());
    let mut generator = MaterialExecutor::new(good, author);
    if let Some(complete) = supply {
        generator.supply_assets = Some(input["assets"].as_array().unwrap().clone());
        generator.supply_complete = complete;
    }
    let output = run(&input, &generator, "offline", "medium").await.unwrap();
    assert_eq!(output["status"], "completed", "{}", output["blocked"]);
    let publication = generator.normal.published.lock().unwrap()[0].clone();
    let calls = generator.calls.lock().unwrap().clone();
    for (request, raw) in &calls {
        let (snapshot, key) = DurableOffline::transport(request);
        let child = persistor
            .store
            .add_job(
                "text",
                &request.stage,
                "offline-fixture",
                "offline-fixture",
                &snapshot,
                "received",
            )
            .unwrap();
        persistor
            .store
            .update_job(&child, "received", &json!({"result":raw}))
            .unwrap();
        persistor
            .store
            .event(
                &child,
                "text_received_raw",
                &json!({"requestHash":key,"value":raw,"evidence":"synthetic supplement receipt"}),
            )
            .unwrap();
        persistor
            .store
            .checkpoint(
                &persistor.run,
                s(&request.input, "executionRoleId"),
                &request.stage,
                &key,
                "received",
                Some(&child),
                Some(raw),
                None,
            )
            .unwrap();
        if request.stage == "seriesAuthor" {
            let reason = s(
                &publication["seriesDeliveries"][0]["adoptionReceipt"]["assetSupplement"]["origin"],
                "reason",
            );
            persistor.reject(request, raw, reason).await.unwrap();
        } else {
            persistor.accept(request, raw).await.unwrap();
        }
    }
    assert_eq!(
        persistor.store.values("assets").unwrap().len(),
        if supply.is_some() { 3 } else { 2 },
        "no assets inserted before whole-episode publication"
    );
    (dir, input, persistor, publication, calls)
}

#[tokio::test]
async fn v2_existing_supply_publication_preserves_manual_fields_history_and_reopens_without_rebuy()
{
    for complete in [false, true] {
        let (dir, input, persistor, output, _) =
            durable_material_result_with_supply(Some(complete)).await;
        let before = persistor.store.get("assets", "existing-key").unwrap();
        let guard = persistor.store.job(&persistor.run).unwrap()["snapshot"]["guard"].clone();
        persistor
            .store
            .publish_storyboard_episode(&guard, &output, &input, &persistor.run)
            .unwrap();
        let after = persistor.store.get("assets", "existing-key").unwrap();
        assert_eq!(after["designRevision"], if complete { 2 } else { 1 });
        assert_eq!(after["version"], if complete { 2 } else { 1 });
        for field in [
            "id",
            "variantId",
            "imagePrompt",
            "currentMediaId",
            "applicability",
            "importName",
            "filename",
            "placeholderOrdinal",
        ] {
            assert_eq!(before[field], after[field], "{field}");
        }
        if !complete {
            assert_eq!(before["adoptedDesign"], after["adoptedDesign"]);
        }
        let old: String = persistor
            .store
            .conn()
            .unwrap()
            .query_row(
                "SELECT data FROM asset_versions WHERE asset_id='existing-key' AND version=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&old).unwrap(), before);
        let updated =
            persistor.store.job(&persistor.run).unwrap()["prepared"]["publicationGuard"].clone();
        persistor
            .store
            .publish_storyboard_episode(&updated, &output, &input, &persistor.run)
            .unwrap();
        assert_eq!(
            persistor.store.get("assets", "existing-key").unwrap(),
            after
        );
        let reopened = crate::store::Store::open(dir.path().to_owned()).unwrap();
        assert_eq!(reopened.get("assets", "existing-key").unwrap(), after);
        assert_eq!(
            reopened.reusable_series_episode_ids(&input).unwrap(),
            vec![s(&output, "episodeId").to_string()]
        );
        let unit = reopened.get("units", s(&output["units"][0], "id")).unwrap();
        let scopes = reopened.series_supplied_unit_scopes(&unit).unwrap();
        assert_eq!(scopes.len(), 1);
        assert!(scopes.contains_key("existing-key"));
        let snapshot =
            crate::binding::bind(&reopened, &unit, "offline-fixture", "offline-fixture").unwrap();
        assert!(
            !snapshot["problems"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["code"] == "SERIES_SUPPLY_SCOPE_UNVERIFIED"
                    || (p["code"] == "OUT_OF_SCOPE" && p["assetId"] == "existing-key")),
            "{}",
            snapshot["problems"]
        );
        assert!(
            !snapshot["problems"].as_array().unwrap().is_empty(),
            "no fake media readiness"
        );
        assert_eq!(
            reopened.get("assets", "existing-key").unwrap()["applicability"],
            after["applicability"]
        );
        // Image preparation can advance a row version while its published
        // complete design/identity remain exactly the same.
        let mut media_only = reopened.get("assets", "existing-key").unwrap();
        media_only["imagePrompt"] = json!("用户调整的图片提示词");
        media_only["expectedVersion"] = media_only["version"].clone();
        reopened.save_asset(media_only).unwrap();
        assert!(reopened
            .series_supplied_unit_scopes(&unit)
            .unwrap()
            .contains_key("existing-key"));
        let mut wrong_unit = unit.clone();
        wrong_unit["sceneId"] = json!("another-scene");
        assert!(reopened
            .series_supplied_unit_scopes(&wrong_unit)
            .unwrap_err()
            .contains("集场"));
        let mut changed = reopened.get("assets", "existing-key").unwrap();
        changed["adoptedDesign"]["full"] = json!("未经本次采用的另一个设计");
        reopened
            .conn()
            .unwrap()
            .execute(
                "UPDATE assets SET data=?1 WHERE id='existing-key'",
                [changed.to_string()],
            )
            .unwrap();
        assert!(reopened
            .series_supplied_unit_scopes(&unit)
            .unwrap_err()
            .contains("采用设计已变化"));
    }
}

#[tokio::test]
async fn v2_existing_supply_cas_holds_changed_asset_and_design_versions_without_partial_writes() {
    for field in ["assetVersion", "designRevision"] {
        let (_dir, input, persistor, output, _) =
            durable_material_result_with_supply(Some(true)).await;
        let mut changed = persistor.store.get("assets", "existing-key").unwrap();
        if field == "assetVersion" {
            changed["imagePrompt"] = json!("并发人工提示词");
            changed["expectedVersion"] = changed["version"].clone();
            persistor.store.save_asset(changed).unwrap();
        } else {
            changed["designRevision"] = json!(9);
            persistor
                .store
                .conn()
                .unwrap()
                .execute(
                    "UPDATE assets SET data=?1 WHERE id='existing-key'",
                    [changed.to_string()],
                )
                .unwrap();
        }
        let before = persistor.store.values("assets").unwrap();
        let mut connection = persistor.store.conn().unwrap();
        let tx = connection.transaction().unwrap();
        let error = crate::series_store::persist(&tx, &input, &output, &persistor.run).unwrap_err();
        assert!(
            error.contains("既有供卡资产版本或完整设计已变化"),
            "{error}"
        );
        tx.rollback().unwrap();
        assert_eq!(persistor.store.values("assets").unwrap(), before);
        assert!(persistor.store.values("units").unwrap().is_empty());
    }
}
#[tokio::test]
async fn supplement_publication_inserts_once_keeps_author_rejected_and_reopens_same_version() {
    let (dir, input, persistor, output, calls) = durable_material_result().await;
    let guard = persistor.store.job(&persistor.run).unwrap()["snapshot"]["guard"].clone();
    let before = persistor.store.values("assets").unwrap();
    persistor
        .store
        .publish_storyboard_episode(&guard, &output, &input, &persistor.run)
        .unwrap();
    let current = persistor.store.values("assets").unwrap();
    assert_eq!(current.len(), 3);
    for old in &before {
        let after = current.iter().find(|a| a["id"] == old["id"]).unwrap();
        for field in [
            "name",
            "filename",
            "placeholderOrdinal",
            "imagePrompt",
            "adoptedDesign",
        ] {
            assert_eq!(after[field], old[field], "{field}");
        }
    }
    let updated_guard =
        persistor.store.job(&persistor.run).unwrap()["prepared"]["publicationGuard"].clone();
    assert_eq!(updated_guard["assets"].as_object().unwrap().len(), 3);
    persistor
        .store
        .publish_storyboard_episode(&updated_guard, &output, &input, &persistor.run)
        .unwrap();
    assert_eq!(
        persistor.store.values("assets").unwrap(),
        current,
        "whole workflow publish must not insert twice or revise old media"
    );
    let reopened = crate::store::Store::open(dir.path().to_owned()).unwrap();
    assert_eq!(reopened.values("assets").unwrap(), current);
    let original = reopened
        .job(&persistor.child(&calls[0].0).unwrap())
        .unwrap();
    assert_eq!(original["status"], "rejected");
    // Store::job decorates display results with mediaIds. Verify the actual
    // durable raw and its original received event, not that UI projection.
    let original_id = persistor.child(&calls[0].0).unwrap();
    let (saved_raw,event_raw):(String,String)=reopened.conn().unwrap().query_row(
        "SELECT j.result,(SELECT data FROM events WHERE job_id=j.id AND type='text_received_raw' ORDER BY rowid DESC LIMIT 1) FROM jobs j WHERE j.id=?1",
        [&original_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&saved_raw).unwrap(),
        calls[0].1
    );
    assert_eq!(
        serde_json::from_str::<Value>(&event_raw).unwrap()["value"],
        calls[0].1
    );
    let saved: String = reopened
        .conn()
        .unwrap()
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("series.delivery:{}", s(&output, "episodeId"))],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&saved).unwrap(),
        output["seriesDeliveries"][0]
    );
}
#[tokio::test]
async fn supplement_publication_rolls_back_when_deleted_identity_reserves_its_ordinal() {
    let (_dir, input, persistor, output, _) = durable_material_result().await;
    let proposed = output["seriesDeliveries"][0]["adoptionReceipt"]["assetSupplement"]
        ["addedAssets"][0]
        .clone();
    let mut tombstone = proposed;
    tombstone["id"] = json!("old-deleted-key");
    tombstone["name"] = json!("旧钥匙");
    let saved = persistor.store.save_asset(tombstone).unwrap();
    persistor
        .store
        .set_asset_deleted("old-deleted-key", saved["version"].as_i64().unwrap(), true)
        .unwrap();
    let before = persistor.store.values("assets").unwrap();
    let guard = persistor.store.job(&persistor.run).unwrap()["snapshot"]["guard"].clone();
    let error = persistor
        .store
        .publish_storyboard_episode(&guard, &output, &input, &persistor.run)
        .unwrap_err();
    assert!(
        error.contains("历史序号") || error.contains("身份"),
        "{error}"
    );
    assert_eq!(persistor.store.values("assets").unwrap(), before);
    assert!(persistor.store.values("units").unwrap().is_empty());
}
#[tokio::test]
async fn supplement_high_water_conflict_never_recycles_or_writes_partial_adoptions() {
    let (_dir, input, persistor, output, _) = durable_material_result().await;
    persistor
        .store
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO metadata(key,value) VALUES('series.placeholder.nextOrdinal','40')",
            [],
        )
        .unwrap();
    let before = persistor.store.values("assets").unwrap();
    let guard = persistor.store.job(&persistor.run).unwrap()["snapshot"]["guard"].clone();
    let error = persistor
        .store
        .publish_storyboard_episode(&guard, &output, &input, &persistor.run)
        .unwrap_err();
    assert!(error.contains("历史序号"), "{error}");
    assert_eq!(persistor.store.values("assets").unwrap(), before);
    assert!(persistor.store.values("units").unwrap().is_empty());
}
#[tokio::test]
async fn publication_rejects_an_unknown_or_promoted_original_author_even_with_valid_candidate() {
    for state in ["interrupted", "completed"] {
        let (_dir, input, persistor, output, calls) = durable_material_result().await;
        let child = persistor.child(&calls[0].0).unwrap();
        persistor
            .store
            .update_job(&child, state, &json!({}))
            .unwrap();
        let guard = persistor.store.job(&persistor.run).unwrap()["snapshot"]["guard"].clone();
        let error = persistor
            .store
            .publish_storyboard_episode(&guard, &output, &input, &persistor.run)
            .unwrap_err();
        assert!(error.contains("唯一已拒绝原任务"), "{error}");
        assert_eq!(persistor.store.values("assets").unwrap().len(), 2);
        assert!(persistor.store.values("units").unwrap().is_empty());
    }
}
