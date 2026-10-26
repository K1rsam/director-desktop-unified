use super::*;

fn with_dependent_proposal(candidate: &Value, parent_revision: u64) -> Value {
    let mut mixed = candidate.clone();
    let revised = &candidate["revisions"][0];
    mixed["proposals"] = json!([{"key":"copper_key","name":"铜钥匙","kind":"PR","description":"适配门口新锁眼的钥匙",
        "basis":"explicit_source","evidence":revised["evidence"],"sourceFacts":revised["sourceFacts"],"productionChoices":["椭圆匙柄为制作选择"],
        "full":"铜钥匙完整设计：椭圆匙柄和长杆，齿面对应门口锁眼，沿当前采用门口设计操作。","brief":"椭圆柄铜钥匙",
        "designDependencies":[{"assetId":revised["assetId"],"designRevision":parent_revision}]}]);
    mixed["resolutions"][0]["disposition"] = json!("new_asset");
    mixed["resolutions"][0]["proposalKeys"] = json!(["copper_key"]);
    mixed
}

pub(super) fn normal_candidate(request: &TextRequest, mut candidate: Value, revise: bool) -> Value {
    if request.input["seriesAssetSupplement"]["version"] != supply_revision::VERSION {
        return candidate;
    }
    candidate["schemaVersion"] = json!(supply_revision::NORMAL_CANDIDATE);
    candidate["revisions"] = json!([]);
    if !revise {
        return candidate;
    }
    let card = binding(request, "assets")
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "s")
        .unwrap();
    let row = binding(request, "designSourceRows")
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["text"].as_str().unwrap().contains("手握铜钥匙"))
        .unwrap();
    candidate["proposals"] = json!([]);
    candidate["existingSupply"] = json!([]);
    candidate["resolutions"][0]["disposition"] = json!("scene_detail");
    candidate["resolutions"][0]["proposalKeys"] = json!([]);
    candidate["resolutions"][0]["existingAssetIds"] = json!(["s"]);
    candidate["revisions"] = json!([{"assetId":"s","variantId":"base","baseAssetVersion":card["version"],
        "baseDesignRevision":card["designRevision"],"baseDesignSha256":hash(&card["adoptedDesign"]),"unresolvedIndexes":[0],
        "sourceRefs":[row["id"]],"evidence":[{"rowId":row["id"],"quote":"甲手握铜钥匙站在门口。"}],
        "rationale":"本集钥匙动作需要旧门口设计补足操作位置，不另造身份。","changeSummary":"补清锁眼位置和手部动作空间。",
        "preservedConstraints":"保留既有门框、通道、光线和左右关系。",
        "full":format!("{} 门框锁眼位于自然手高，手持铜钥匙时齿面朝锁眼；门边留足操作空间。",s(&card["adoptedDesign"],"full")),
        "brief":"原门口同身份，锁眼与握匙空间明确。","sourceFacts":[{"statement":"甲手握铜钥匙站在门口。","sourceRefs":[row["id"]]}],
        "productionChoices":["锁眼位置与操作留白为制作补足"],"designDependencies":[]}]);
    candidate
}

pub(crate) async fn series_normal_revision_fixture() -> (Value, Vec<(TextRequest, Value)>, Value) {
    normal_revision_fixture(false).await
}
pub(crate) async fn series_unpublished_revision_fixture(
) -> (Value, Vec<(TextRequest, Value)>, Value) {
    normal_revision_fixture(true).await
}
async fn normal_revision_fixture(pending: bool) -> (Value, Vec<(TextRequest, Value)>, Value) {
    let (mut input, good, author) = material_input();
    mark_input(&mut input).unwrap();
    input["source"]["version"] = json!(1);
    allocate_asset_identities(input["assets"].as_array_mut().unwrap()).unwrap();
    if pending {
        let scene = input["assets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|a| a["id"] == "s")
            .unwrap();
        scene.as_object_mut().unwrap().remove("adoptedDesign");
        scene["designRevision"] = json!(0);
    }
    let mut executor = MaterialExecutor::new(good, author);
    executor.supply_assets = Some(input["assets"].as_array().unwrap().clone());
    executor.supply_new_only = true;
    executor.normal_revision = true;
    executor.revise = true;
    let out = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let calls = executor.calls.lock().unwrap().clone();
    (input, calls, out["seriesDeliveries"][0].clone())
}

#[tokio::test]
async fn normal_v3_pending_first_design_then_revision_preserves_both_receipts_and_contiguous_adoptions(
) {
    let (input, _calls, delivery) = series_unpublished_revision_fixture().await;
    validate_delivery(&input, &delivery).unwrap();
    let proof = &delivery["adoptionReceipt"]["assetSupplement"];
    let before = proof["beforeAssets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "s")
        .unwrap();
    validate_unpublished_design_predecessor(&input, &delivery, before).unwrap();
    assert_eq!(before["designRevision"], 1);
    assert_eq!(proof["revisedAssets"][0]["designRevision"], 2);
    let state = proof["identityReservations"]["assetStates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["assetId"] == "s")
        .unwrap();
    assert_eq!(
        state["designRevision"], 0,
        "freeze the actual persisted baseline, not pending design1"
    );
    let adoptions: Vec<_> = delivery["designAdoptions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["assetId"] == "s")
        .map(|a| json!([a["baseDesignRevision"], a["designRevision"]]))
        .collect();
    assert_eq!(adoptions, vec![json!([0, 1]), json!([1, 2])]);
    for mutation in ["old-full", "remove-prior", "skip-revision", "duplicate"] {
        let mut changed = delivery.clone();
        let list = changed["designAdoptions"].as_array_mut().unwrap();
        let index = list
            .iter()
            .position(|a| a["assetId"] == "s" && a["designRevision"] == 1)
            .unwrap();
        match mutation {
            "old-full" => list[index]["adoptedDesign"]["full"] = json!("forged predecessor"),
            "remove-prior" => {
                list.remove(index);
            }
            "skip-revision" => list[index]["baseDesignRevision"] = json!(9),
            _ => {
                let copy = list[index].clone();
                list.push(copy);
            }
        }
        assert!(validate_delivery(&input, &changed).is_err(), "{mutation}");
    }
}

#[tokio::test]
async fn normal_v3_revises_existing_design_in_original_budget_without_correction_authorization() {
    let (input, calls, delivery) = series_normal_revision_fixture().await;
    validate_delivery(&input, &delivery).unwrap();
    let proof = &delivery["adoptionReceipt"]["assetSupplement"];
    assert_eq!(proof["version"], supply_revision::VERSION);
    assert_eq!(
        proof["identityReservations"]["version"],
        "series-supplement-identities/v3"
    );
    assert!(delivery["adoptionReceipt"]["supplyContractCorrection"].is_null());
    assert_eq!(proof["addedAssets"], json!([]));
    assert_eq!(proof["revisedAssets"][0]["id"], "s");
    assert_eq!(proof["revisedAssets"][0]["designRevision"], 2);
    assert_eq!(proof["rounds"].as_array().unwrap().len(), 2);
    assert!(calls
        .iter()
        .all(|(r, _)| r.input.get("seriesSupplyCorrection").is_none()));
    let (origin, origin_raw) = calls
        .iter()
        .find(|(r, _)| r.stage == "seriesAuthor")
        .unwrap();
    let (design, candidate) = calls
        .iter()
        .find(|(r, _)| r.stage == "seriesDesign")
        .unwrap();
    let identities = binding(design, "identityReservations");
    let known = binding(design, "knownAssetCatalog");
    let source = source(&input).unwrap();
    let episode = &source.episodes[0];
    let mixed = with_dependent_proposal(candidate, 2);
    let added = supplement::validate_candidate(
        &mixed, origin, origin_raw, known, identities, &source, episode,
    )
    .unwrap();
    let revised =
        supply_revision::adopted(&mixed, origin, &source, &json!("design"), &json!("review"))
            .unwrap();
    let mut projected = supply_revision::revised_before(origin, &revised).unwrap();
    projected.extend(supplement::adopted_added(
        added,
        &mixed,
        &source,
        &json!("design"),
        &json!("review"),
    ));
    design::cards(&source, episode, &projected).unwrap();
    assert!(
        supplement::validate_candidate(
            &with_dependent_proposal(candidate, 1),
            origin,
            origin_raw,
            known,
            identities,
            &source,
            episode
        )
        .is_err(),
        "new child must pin the newly reviewed parent revision"
    );
    for mutation in ["version", "identity", "fake-source", "no-use", "empty-full"] {
        let mut changed = candidate.clone();
        match mutation {
            "version" => changed["revisions"][0]["baseDesignRevision"] = json!(7),
            "identity" => changed["revisions"][0]["assetId"] = json!("unknown"),
            "fake-source" => {
                changed["revisions"][0]["evidence"][0]["quote"] = json!("原文没有这句")
            }
            "no-use" => changed["resolutions"][0]["existingAssetIds"] = json!(["a"]),
            _ => changed["revisions"][0]["full"] = json!(" "),
        }
        assert!(
            supplement::validate_candidate(
                &changed, origin, origin_raw, known, identities, &source, episode
            )
            .is_err(),
            "{mutation}"
        );
    }
    let (review, review_raw) = calls
        .iter()
        .find(|(r, _)| r.stage == "seriesDesignReview")
        .unwrap();
    assert_eq!(
        review.input["expectedReviewCoverage"]["reviewedRevisionAssetIds"],
        json!(["s"])
    );
    let mut changed = review_raw.clone();
    changed["reviewedRevisionAssetIds"] = json!([]);
    assert!(supplement::validate_review(&changed, candidate, 1).is_err());
    let mut changed = review.clone();
    changed.input["seriesAssetSupplement"]["attempt"] = json!(3);
    assert!(validate_supplement_transition(
        &changed,
        origin,
        origin_raw,
        Some(design),
        Some(candidate),
        &input
    )
    .is_err());
    let mut changed = delivery.clone();
    changed["adoptionReceipt"]["assetSupplement"]["revisedAssets"][0]["filename"] =
        json!("renamed.png");
    assert!(validate_delivery(&input, &changed).is_err());
}

struct CorrectionExecutor {
    prior: MaterialExecutor,
    context: Value,
    correction_calls: Mutex<Vec<(TextRequest, Value)>>,
    revise_once: bool,
    reject_twice: bool,
    unknown: bool,
    summarize_fact: bool,
}
impl CorrectionExecutor {
    fn candidate(&self, request: &TextRequest) -> Value {
        let mut candidate = self.prior.candidate(request);
        candidate["schemaVersion"] = json!(supply_revision::CANDIDATE);
        candidate["proposals"] = json!([]);
        candidate["resolutions"][0]["disposition"] = json!("scene_detail");
        candidate["resolutions"][0]["proposalKeys"] = json!([]);
        candidate["resolutions"][0]["existingAssetIds"] = json!(["s"]);
        candidate["resolutions"][0]["rationale"] = json!(
            "本镜钥匙作为既有门口设计的小物细节，无需另建资产；同身份新版补充握持与锁眼关系。"
        );
        let binding = &self.context["authorization"]["affectedAssets"][0];
        let old = binding_for(request, "assets")
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "s")
            .unwrap();
        let row = binding_for(request, "designSourceRows")
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["text"].as_str().unwrap().contains("手握铜钥匙"))
            .unwrap();
        let mut revision = binding.clone();
        for key in ["basisReviewRequestSha256", "basisReviewIssueIndex"] {
            revision.as_object_mut().unwrap().remove(key);
        }
        revision["evidence"] = json!([{"rowId":row["id"],"quote":"甲手握铜钥匙站在门口。"}]);
        revision["rationale"] =
            json!("原文需要握匙动作，已采用设计尚未说明锁眼和手部空间，沿门口原身份修订。");
        revision["changeSummary"] = json!("补入锁眼与手部动作空间，门框、方位、光线不变。");
        revision["preservedConstraints"] = json!("门框位置、门内外关系、人物通行和既有尺度保留。");
        revision["full"] = json!(format!(
            "{} 门框锁眼在人物自然手高，铜钥匙作为门边小物被握持；齿面朝锁眼，手部动作不挡通行。",
            s(&old["adoptedDesign"], "full")
        ));
        revision["brief"] = json!("既有门口同身份，锁眼与手部空间明确。");
        revision["sourceFacts"] =
            json!([{"statement":"甲手握铜钥匙站在门口。","sourceRefs":[row["id"]]}]);
        if self.summarize_fact {
            revision["sourceFacts"][0]["statement"] = json!("甲在门口手持一把铜钥匙。");
        }
        revision["productionChoices"] = json!(["锁眼高度与留白为制作选择，不添加源事件。"]);
        revision["designDependencies"] = json!([]);
        candidate["revisions"] = json!([revision]);
        candidate
    }
}
fn binding_for<'a>(request: &'a TextRequest, kind: &str) -> &'a Value {
    binding(request, kind)
}
#[async_trait]
impl TextExecutor for CorrectionExecutor {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        let phase = s(&request.input["seriesSupplyCorrection"], "phase");
        let raw = match phase {
            "design" => self.candidate(&request),
            "design-review" => {
                if self.unknown {
                    return Err("SUBMISSION_UNKNOWN: correction review held".into());
                }
                let attempt = request.input["seriesSupplyCorrection"]["attempt"]
                    .as_u64()
                    .unwrap();
                let revise = self.reject_twice || (self.revise_once && attempt == 0);
                let mut value = request.input["expectedReviewCoverage"].clone();
                value["schemaVersion"] = json!(supply_revision::REVIEW);
                value["candidateSha256"] = request.input["candidateSha256"].clone();
                value["conclusion"] = json!(if revise { "revise" } else { "pass" });
                value["summary"] = json!("synthetic independent same-identity revision review");
                value["issues"] = if revise {
                    json!([{"unresolvedIndex":0,"proposalKey":"","assetId":"s",
                    "problem":format!("synthetic remaining local issue {attempt}"),"requestedChange":"补清手部动作路线，不改原话。"}])
                } else {
                    json!([])
                };
                value
            }
            "finisher" => self.prior.good.clone(),
            _ => self.prior.execute(request.clone()).await?,
        };
        self.correction_calls
            .lock()
            .unwrap()
            .push((request, raw.clone()));
        Ok(raw)
    }
    async fn series_supply_correction_context(
        &self,
        _: &TextRequest,
        _: &Value,
    ) -> Result<Option<Value>, String> {
        Ok(Some(self.context.clone()))
    }
    async fn accept(&self, r: &TextRequest, v: &Value) -> Result<(), String> {
        self.prior.accept(r, v).await
    }
    async fn reject(&self, r: &TextRequest, v: &Value, e: &str) -> Result<(), String> {
        self.prior.reject(r, v, e).await
    }
    async fn publish_episode(&self, out: &Value, input: &Value) -> Result<(), String> {
        self.prior.publish_episode(out, input).await
    }
}

async fn setup(workflow_id: &str) -> (Value, Vec<(TextRequest, Value)>, CorrectionExecutor) {
    let (mut input, good, author) = material_input();
    // Fixture simulates a real old frozen workflow; new marker must not be
    // retroactively written to it by correction or request reconstruction.
    input.as_object_mut().unwrap().remove(design_source::FIELD);
    input["source"]["version"] = json!(1);
    let mut prior = MaterialExecutor::new(good, author);
    prior.exhaust = true;
    let out = run(&input, &prior, "gpt-6.1-sol", "medium").await.unwrap();
    assert_eq!(out["status"], "partially_blocked", "{out}");
    assert!(out["blocked"]
        .to_string()
        .contains("SERIES_SUPPLEMENT_REVIEW_REPAIR_HELD"));
    let calls = prior.calls.lock().unwrap().clone();
    let (origin, raw) = calls
        .iter()
        .find(|(r, _)| r.stage == "seriesAuthor")
        .unwrap();
    let receipts: Vec<_> = calls
        .iter()
        .filter(|(r, _)| r.input.get("seriesAssetSupplement").is_some())
        .map(|(r, v)| role_receipt(r, v).unwrap())
        .collect();
    assert_eq!(receipts.len(), 6);
    let card = supplement::before(origin)
        .unwrap()
        .iter()
        .find(|a| a["id"] == "s")
        .unwrap();
    let source = source(&input).unwrap();
    let row = source
        .rows
        .iter()
        .find(|r| r.text.contains("手握铜钥匙"))
        .unwrap();
    let context = json!({"version":supply_correction::VERSION,"workflowId":workflow_id,
        "originRequestSha256":supplement::request_sha(origin).unwrap(),"originRawSha256":hash(raw),
        "legacyFailureCode":"SERIES_SUPPLEMENT_REVIEW_REPAIR_HELD","legacyReceipts":receipts,
        "legacyReceiptsSha256":hash(&json!(receipts)),"maximumAttempts":1,"maximumCorrections":1,
        "authorization":{"kind":"controller-verified-supply-contract-gap/v1","classification":"existing_design_revision_unsupported",
            "auditSha256":digest("synthetic independent root cause audit"),"affectedAssets":[{"assetId":"s","variantId":"base",
            "baseAssetVersion":card["version"],"baseDesignRevision":card["designRevision"],"baseDesignSha256":hash(&card["adoptedDesign"]),
            "unresolvedIndexes":[0],"sourceRefs":[row.id],"basisReviewRequestSha256":receipts[5]["requestSha256"],"basisReviewIssueIndex":0}]}});
    validate_supply_correction_context(&context, origin, raw, &input).unwrap();
    (
        input,
        calls,
        CorrectionExecutor {
            prior,
            context,
            correction_calls: Mutex::new(vec![]),
            revise_once: true,
            reject_twice: false,
            unknown: false,
            summarize_fact: false,
        },
    )
}

/// Synthetic chain for the durable SDK/CAS tests. No production text or account.
pub(crate) async fn series_supply_correction_fixture(
    workflow_id: &str,
) -> (
    Value,
    Vec<(TextRequest, Value)>,
    Value,
    Vec<(TextRequest, Value)>,
    Value,
) {
    correction_fixture_with_fact_summary(workflow_id, false).await
}

pub(crate) async fn series_supply_correction_fact_fixture(
    workflow_id: &str,
) -> (
    Value,
    Vec<(TextRequest, Value)>,
    Value,
    Vec<(TextRequest, Value)>,
    Value,
) {
    correction_fixture_with_fact_summary(workflow_id, true).await
}

async fn correction_fixture_with_fact_summary(
    workflow_id: &str,
    summarize: bool,
) -> (
    Value,
    Vec<(TextRequest, Value)>,
    Value,
    Vec<(TextRequest, Value)>,
    Value,
) {
    let started = std::time::Instant::now();
    let (input, legacy, mut executor) = setup(workflow_id).await;
    executor.summarize_fact = summarize;
    let legacy_ms = started.elapsed().as_millis();
    let out = run(&input, &executor, "gpt-6.1-sol", "medium")
        .await
        .unwrap();
    assert_eq!(out["status"], "completed", "{}", out["blocked"]);
    let publication = executor
        .prior
        .normal
        .published
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    let delivery = publication["seriesDeliveries"][0].clone();
    let calls = executor.correction_calls.lock().unwrap().clone();
    eprintln!("[series-correction-domain-timing] legacy_ms={legacy_ms} correction_and_delivery_ms={} total_ms={}",started.elapsed().as_millis()-legacy_ms,started.elapsed().as_millis());
    (input, legacy, executor.context.clone(), calls, delivery)
}

#[tokio::test]
async fn supply_correction_preserves_old_chain_and_publishes_same_identity_with_new_design() {
    let (input, legacy, context, calls, delivery) =
        series_supply_correction_fixture("synthetic-workflow").await;
    assert!(input.get(design_source::FIELD).is_none());
    assert_eq!(context["legacyReceipts"].as_array().unwrap().len(), 6);
    validate_delivery(&input, &delivery).unwrap();
    assert!(delivery["adoptionReceipt"]["assetSupplement"].is_null());
    let proof = &delivery["adoptionReceipt"]["supplyContractCorrection"];
    let old = proof["beforeAssets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "s")
        .unwrap();
    let revised = &proof["revisedAssets"][0];
    for field in [
        "id",
        "variantId",
        "importName",
        "filename",
        "placeholderOrdinal",
        "version",
        "scope",
        "applicability",
    ] {
        assert_eq!(revised[field], old[field], "permanent field {field}");
    }
    assert_eq!(revised["designRevision"], 2);
    assert_ne!(
        revised["adoptedDesign"]["full"],
        old["adoptedDesign"]["full"]
    );
    assert_eq!(proof["addedAssets"], json!([]));
    assert_eq!(proof["rounds"].as_array().unwrap().len(), 2);
    assert!(calls
        .iter()
        .filter(|(r, _)| r.input.get("seriesSupplyCorrection").is_some())
        .all(|(r, _)| !r.input.to_string().contains("legacyReceipts\"")
            && r.input[design_source::FIELD] == design_source::VERSION));
    let (origin, raw) = legacy
        .iter()
        .find(|(r, _)| r.stage == "seriesAuthor")
        .unwrap();
    let (created, candidate) = calls
        .iter()
        .find(|(r, _)| r.input["seriesSupplyCorrection"]["phase"] == "design")
        .unwrap();
    let last = context["legacyReceipts"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    let predecessor: TextRequest = serde_json::from_value(last["request"].clone()).unwrap();
    let mut altered_context = context.clone();
    altered_context["legacyReceipts"][0]["raw"]["proposals"][0]["full"] =
        json!("tampered old raw after an earlier valid run");
    altered_context["legacyReceiptsSha256"] = json!(hash(&altered_context["legacyReceipts"]));
    assert!(validate_supply_correction_transition(created,&altered_context,origin,raw,Some(&predecessor),Some(&last["raw"]),&input).is_err(),"a new public entry revalidates the complete old chain and cannot reuse a previous local proof");
    let mut altered_request = created.clone();
    altered_request.input["seriesSupplyCorrection"]["attempt"] = json!(2);
    assert!(
        validate_supply_correction_transition(
            &altered_request,
            &context,
            origin,
            raw,
            Some(&predecessor),
            Some(&last["raw"]),
            &input
        )
        .is_err(),
        "local context deduplication never expands the correction budget"
    );
    let source = source(&input).unwrap();
    let episode = &source.episodes[0];
    supply_revision::validate(
        &with_dependent_proposal(candidate, 2),
        &context,
        origin,
        raw,
        binding(created, "knownAssetCatalog"),
        binding(created, "identityReservations"),
        &source,
        episode,
        &input,
    )
    .unwrap();
    let old_shape = supply_revision::legacy_candidate(
        &with_dependent_proposal(candidate, 2),
        binding(created, "identityReservations"),
    );
    assert!(
        supplement::validate_candidate(
            &old_shape,
            origin,
            raw,
            binding(created, "knownAssetCatalog"),
            binding(created, "identityReservations"),
            &source,
            episode
        )
        .is_err(),
        "plain historical v1/v2 never gains permission to change its parent version"
    );
    assert!(
        supply_revision::validate(
            &with_dependent_proposal(candidate, 1),
            &context,
            origin,
            raw,
            binding(created, "knownAssetCatalog"),
            binding(created, "identityReservations"),
            &source,
            episode,
            &input
        )
        .is_err(),
        "correction also pins its candidate graph without rewriting the old v1 proof"
    );
    assert_eq!(proof["origin"]["raw"], *raw);
    for (field, value) in [
        ("maximumAttempts", json!(2)),
        ("maximumCorrections", json!(2)),
        ("originRawSha256", json!("0".repeat(64))),
    ] {
        let mut changed = context.clone();
        changed[field] = value;
        assert!(
            validate_supply_correction_context(&changed, origin, raw, &input).is_err(),
            "{field}"
        );
    }
    for mutation in ["base", "source", "basis", "budget", "raw"] {
        let mut changed = context.clone();
        match mutation {
            "base" => {
                changed["authorization"]["affectedAssets"][0]["baseDesignRevision"] = json!(99)
            }
            "source" => {
                changed["authorization"]["affectedAssets"][0]["sourceRefs"] = json!(["foreign-row"])
            }
            "basis" => {
                changed["authorization"]["affectedAssets"][0]["basisReviewIssueIndex"] = json!(99)
            }
            "budget" => {
                changed["legacyReceipts"].as_array_mut().unwrap().pop();
            }
            _ => changed["legacyReceipts"][5]["raw"]["conclusion"] = json!("pass"),
        }
        changed["legacyReceiptsSha256"] = json!(hash(&changed["legacyReceipts"]));
        assert!(
            validate_supply_correction_context(&changed, origin, raw, &input).is_err(),
            "{mutation}"
        );
    }
    let mut changed = delivery.clone();
    changed["adoptionReceipt"]["supplyContractCorrection"]["revisedAssets"][0]["filename"] =
        json!("changed.png");
    assert!(validate_delivery(&input, &changed).is_err());
}

#[tokio::test]
async fn supply_correction_repair_budget_and_unknown_never_restart_old_attempts() {
    for unknown in [false, true] {
        let (input, _, mut executor) = setup("budget-workflow").await;
        executor.reject_twice = !unknown;
        executor.unknown = unknown;
        let out = run(&input, &executor, "gpt-6.1-sol", "medium")
            .await
            .unwrap();
        assert_eq!(out["status"], "partially_blocked");
        assert!(out["blocked"].to_string().contains(if unknown {
            "SUBMISSION_UNKNOWN"
        } else {
            "SERIES_SUPPLY_CORRECTION_REPAIR_HELD"
        }));
        let calls = executor.correction_calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|(r, _)| r.input["seriesSupplyCorrection"]["phase"] == "design")
                .count(),
            if unknown { 1 } else { 2 }
        );
        assert!(executor.prior.normal.published.lock().unwrap().is_empty());
    }
}

#[test]
fn explicit_design_source_uses_expand_headers_in_locked_scope_without_name_guessing() {
    let source=super::super::super::super::prepare_source("第一集\n场景：房间\n甲第一次进入房间。\n第二集\n场景：房间\n甲从桌边爬回缸里。\n第三集\n场景：房间\n甲换了窗帘。\n").unwrap();
    let e = &source.episodes[0];
    let mut input = json!({"productionScope":{"status":"locked","sourceSha256":source.sha256,"episodeIds":[e.id,source.episodes[1].id]},"selection":{"episodeIds":[e.id]}});
    let heading = source
        .rows
        .iter()
        .find(|r| r.kind != "body" && r.scene_id.as_deref() == Some(&source.scenes[1].id))
        .unwrap();
    let target = json!({"id":"a","name":"房间","sourceRefs":[heading.id],"scope":{"scenes":[{"episodeId":source.episodes[2].id,"sceneId":source.scenes[2].id}]}});
    let rows = design_source::rows(&input, &source, e, &[target]).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[1].text.contains("爬回"));
    let rows = design_source::rows(&input, &source, e, &[json!({"id":"a","name":"房间"})]).unwrap();
    assert_eq!(rows.len(), 1, "same names never grant cross-episode input");
    input["productionScope"]["episodeIds"] =
        json!([e.id, source.episodes[1].id, source.episodes[2].id]);
    let rows=design_source::rows(&input,&source,e,&[json!({"id":"a","applicability":{"scenes":[source.scenes[1].id]},"scope":{"scenes":[[source.episodes[1].id,source.scenes[1].id]]}})]).unwrap();
    assert_eq!(rows.len(), 2, "source order and deduplication");
}

#[tokio::test]
async fn supply_correction_source_fact_summary_revalidation_requires_complete_original_contract() {
    let (input, legacy, context, calls, _) =
        series_supply_correction_fixture("summary-revalidation").await;
    let (origin, origin_raw) = legacy
        .iter()
        .find(|(r, _)| r.stage == "seriesAuthor")
        .unwrap();
    let (request, raw) = calls
        .iter()
        .find(|(r, _)| r.input["seriesSupplyCorrection"]["phase"] == "design")
        .unwrap();
    let prior = context["legacyReceipts"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    let prior_request: TextRequest = serde_json::from_value(prior["request"].clone()).unwrap();
    let input_sha = hash(&input);
    let request_sha = supplement::request_sha(request).unwrap();
    assert!(input.get(SOURCE_FACT_CONTRACT_FIELD).is_none());
    assert!(request.input.get(SOURCE_FACT_CONTRACT_FIELD).is_none());
    assert!(!s(&request.input, "task").contains("精确摘句"));
    let mut summary = with_dependent_proposal(raw, 2);
    for field in ["proposals", "revisions"] {
        summary[field][0]["sourceFacts"][0]["statement"] = json!("甲在门口手持一把铜钥匙。");
    }
    let check = |r: &TextRequest, candidate: &Value| {
        revalidate_supply_correction_source_facts_response(
            r,
            candidate,
            &context,
            origin,
            origin_raw,
            &prior_request,
            &prior["raw"],
            &input,
        )
    };
    check(request, &summary).unwrap();
    assert_eq!(
        check(request, raw).unwrap_err(),
        "SERIES_SUPPLY_FACT_REVALIDATION_NOT_APPLICABLE"
    );
    for fault in [
        "quote",
        "empty",
        "unsupplied-fact",
        "revision-ref",
        "dependency-version",
        "attempt",
        "mixed",
    ] {
        let mut changed = summary.clone();
        let mut altered = request.clone();
        match fault {
            "quote" => {
                changed["proposals"][0]["evidence"][0]["quote"] = json!("甲手握金钥匙站在门口。")
            }
            "empty" => changed["proposals"][0]["sourceFacts"][0]["statement"] = json!(" "),
            "unsupplied-fact" => {
                changed["proposals"][0]["sourceFacts"][0]["sourceRefs"] = json!(["foreign"])
            }
            "revision-ref" => {
                changed["revisions"][0]["sourceFacts"][0]["sourceRefs"] = json!(["foreign"])
            }
            "dependency-version" => {
                changed["proposals"][0]["designDependencies"][0]["designRevision"] = json!(1)
            }
            "attempt" => altered.input["seriesSupplyCorrection"]["attempt"] = json!(1),
            _ => {
                altered.input["seriesAssetSupplement"] =
                    json!({"version":"series-asset-supplement/v3"})
            }
        }
        assert!(check(&altered, &changed).is_err(), "{fault}");
    }
    assert_eq!(hash(&input), input_sha);
    assert_eq!(supplement::request_sha(request).unwrap(), request_sha);
}
