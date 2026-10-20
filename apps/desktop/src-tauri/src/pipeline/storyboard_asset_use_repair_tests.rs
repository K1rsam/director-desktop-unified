struct DuplicateUseModel {
    inner: Recorded,
    mode: &'static str,
}
fn duplicate_use_model_wire_unit(unit: &mut Value) {
    unit.as_object_mut()
        .unwrap()
        .remove(super::super::asset_use_binding::FIELD);
    for reference in unit["assetRefs"].as_array_mut().unwrap() {
        for field in ["sourceRefs", "sourceRefsPurpose", "identityEvidence"] {
            reference.as_object_mut().unwrap().remove(field);
        }
    }
}
#[async_trait]
impl TextExecutor for DuplicateUseModel {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        if request.stage != "storyboardFinish" {
            let author = request.stage == "storyboardCreate";
            let mut raw = self.inner.execute(request).await?;
            // Recorded caches the accepted projection; the real executor
            // replays its separately preserved raw event. Keep this fake wire
            // faithful on reopen instead of returning Rust-owned markers.
            if author {
                for unit in raw["units"].as_array_mut().unwrap() {
                    duplicate_use_model_wire_unit(unit);
                }
            }
            return Ok(raw);
        }
        let key = serde_json::to_string(&request).unwrap();
        if let Some(cached) = self.inner.accepted.lock().await.get(&key).cloned() {
            return Ok(cached);
        }
        if let Some(cached) = self.inner.rejected_raw.lock().await.get(&key).cloned() {
            return Ok(cached);
        }
        self.inner.calls.lock().await.push(request.clone());
        let base = request.input["previousDraft"]["units"].as_array().unwrap();
        let mut retained: Vec<_> = base.iter().map(|unit| unit["id"].clone()).collect();
        let order_second=request.input["finishRepair"]["attempt"]==2;
        if request.input["finishRepair"].is_object() && (self.mode == "NO_PROGRESS" || order_second&&self.mode=="GAP_KEEP") {
            return Ok(
                json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":[],"revisionNotes":[],"unresolved":[]}),
            );
        }
        let old = &base[0];
        let mut next = old.clone();
        if request.input["finishRepair"].is_object() {
            next["assetRefs"]
                .as_array_mut()
                .unwrap()
                .retain(|reference| reference["use"] == "character" || self.mode.starts_with("GAP_")&&reference["use"]!="audio");
            if self.mode.starts_with("GAP_") {
                if order_second&&self.mode=="GAP_FIX" {
                    for (i,reference) in next["assetRefs"].as_array_mut().unwrap().iter_mut().enumerate(){reference["order"]=json!(i+1);}
                }
                if order_second&&self.mode=="GAP_LIMIT" {
                    next["assetRefs"][0]["order"]=json!(2);next["assetRefs"][1]["order"]=json!(4);
                }
                if order_second&&self.mode=="GAP_CHANGED" {next["shots"][0]["picture"]=json!("不应改写的正文");}
                if !order_second {
                    match self.mode {
                        "GAP_ZERO"=>next["assetRefs"][0]["order"]=json!(0),
                        "GAP_DUPLICATE"=>next["assetRefs"][1]["order"]=json!(1),
                        "GAP_UNKNOWN"=>next["assetRefs"][1]["assetId"]=json!("unknown"),
                        _=>(),
                    }
                }
            }
        } else {
            let mut voice = next["shots"][0].clone();
            voice["id"] = json!("voice-only");
            next["shots"][0]["dialogueParts"] = json!([]);
            next["shots"][0]["duration"] = json!(1);
            next["shots"].as_array_mut().unwrap().push(voice.clone());
            next["duration"] = json!(1.0 + voice["duration"].as_f64().unwrap());
            let asset = &request.input["assets"][0];
            next["assetRefs"] = json!([
                {"assetId":asset["id"],"variantId":asset["variantId"],"order":1,"use":"character","useShotIds":[next["shots"][0]["id"]]},
                {"assetId":asset["id"],"variantId":asset["variantId"],"order":2,"use":"audio","useShotIds":[voice["id"]]}
            ]);
            if self.mode.starts_with("GAP_") {
                let asset=&request.input["assets"][1];
                let shot=next["shots"][0]["id"].clone();
                next["assetRefs"].as_array_mut().unwrap().push(json!({"assetId":asset["id"],"variantId":asset["variantId"],"order":3,"use":"prop","useShotIds":[shot]}));
            }
        }
        // Model-wire schema selects uses; only Rust may restore these anchors.
        duplicate_use_model_wire_unit(&mut next);
        retained.retain(|id| id != &old["id"]);
        Ok(
            json!({"schemaVersion":"storyboard-delta/v2","baseDraftHash":request.input["baseDraftHash"],"retainedUnitIds":retained,"replacements":[{"replaces":[old["id"]],"units":[next],"fantasyRealization":[]}],"revisionNotes":[],"unresolved":[]}),
        )
    }
    async fn accept(&self, request: &TextRequest, value: &Value) -> Result<(), String> {
        self.inner.accept(request, value).await
    }
    async fn reject(
        &self,
        request: &TextRequest,
        value: &Value,
        reason: &str,
    ) -> Result<(), String> {
        self.inner
            .rejected_raw
            .lock()
            .await
            .insert(serde_json::to_string(request).unwrap(), value.clone());
        self.inner
            .rejected_values
            .lock()
            .await
            .push((request.clone(), value.clone()));
        self.inner.reject(request, value, reason).await
    }
}

#[tokio::test]
async fn duplicate_asset_use_original_finisher_repairs_once_and_reopen_cannot_reset_budget() {
    let source = prepare_source("第一集\n场景：门口\n甲：原话完整保留。\n").unwrap();
    let mut input = input(&source);
    super::super::stage3_pack::mark_input(&mut input).unwrap();
    super::super::asset_use_binding::freeze_new(&mut input).unwrap();
    input["assets"] = json!([{"id":"person","variantId":"base","name":"甲","kind":"CHARACTER","version":1,"sourceRefs":[source.rows[2].id],"applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]},"preparation":{"mode":"REFERENCE"}}]);
    for mode in ["FIX", "NO_PROGRESS"] {
        let model = DuplicateUseModel {
            inner: Recorded::default(),
            mode,
        };
        let output = run_storyboard(&input, &model, "offline", "medium")
            .await
            .unwrap();
        let calls = model.inner.calls.lock().await.clone();
        assert_eq!(
            calls.iter().filter(|r| r.stage == "storyboardPlan").count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|r| r.stage == "storyboardCreate")
                .count(),
            1
        );
        let origin = calls
            .iter()
            .find(|r| r.input["executionBatch"]["version"] == "related-range-finish/v1")
            .unwrap();
        let repairs: Vec<_> = calls
            .iter()
            .filter(|r| r.input["finishRepair"].is_object())
            .collect();
        assert_eq!(repairs.len(), 1, "{}", output["blocked"]);
        assert_eq!(repairs[0].input["finishRepair"]["attempt"], 1);
        assert_eq!(repairs[0].input["finishRepair"]["maximumAttempts"], 2);
        assert_eq!(
            repairs[0].input["validationIssues"][0]["code"],
            "ASSET_USE_IDENTITY"
        );
        assert_eq!(
            repairs[0].rule_snapshot["runtimeContract"]["assetUseIdentityRepairProfile"],
            "single-identity-reference/v1"
        );
        assert_eq!(
            serde_json::to_vec(&creative::prepare_request(repairs[0].clone()).unwrap()).unwrap(),
            serde_json::to_vec(repairs[0]).unwrap()
        );
        let original = model
            .inner
            .rejected_raw
            .lock()
            .await
            .get(&serde_json::to_string(origin).unwrap())
            .unwrap()
            .clone();
        assert_eq!(
            original["replacements"][0]["units"][0]["assetRefs"]
                .as_array()
                .unwrap()
                .len(),
            2,
            "program never merges the original refs"
        );
        assert!(!model
            .inner
            .accepted
            .lock()
            .await
            .contains_key(&serde_json::to_string(origin).unwrap()));
        if mode == "FIX" {
            assert_eq!(output["status"], "completed", "{}", output["blocked"]);
            assert_eq!(output["units"][0]["assetRefs"].as_array().unwrap().len(), 1);
            assert_eq!(output["units"][0]["assetRefs"][0]["use"], "character");
            assert_eq!(
                output["units"][0]["shots"][1]["dialogueParts"][0]["text"],
                "原话完整保留。"
            );
        } else {
            assert!(output["blocked"]
                .to_string()
                .contains("FINISH_SCOPE_REPAIR_NO_PROGRESS"));
            assert!(output["units"].as_array().unwrap().is_empty());
        }
        let again = run_storyboard(&input, &model, "offline", "medium")
            .await
            .unwrap();
        assert_eq!(again["status"], output["status"], "{}", again["blocked"]);
        assert_eq!(again, output);
        assert_eq!(
            model.inner.calls.lock().await.len(),
            calls.len(),
            "reopen must reuse the original rejected raw and exact bounded repair node"
        );
        assert_eq!(
            model
                .inner
                .rejected_raw
                .lock()
                .await
                .get(&serde_json::to_string(origin).unwrap())
                .unwrap(),
            &original
        );
    }
}

#[test]
fn duplicate_asset_use_only_classifies_disjoint_character_audio_and_keeps_voice_only_legal() {
    assert!(crate::text_executor::author_repairable_rejection(
        crate::text_executor::AUTHOR_ASSET_USE_DUPLICATE_REJECTION
    ));
    for reason in [
        "重复资产",
        "单元重复引用同一资产形象；可忽略",
        "分镜引用未知资产/形象，不能按同名匹配",
        "SDK_SUBMISSION_UNRESOLVED",
    ] {
        assert!(!crate::text_executor::author_repairable_rejection(reason));
    }
    let unit = json!({"id":"U",super::super::asset_use_binding::FIELD:super::super::asset_use_binding::PROFILE,"sourceRefs":["r"],"shots":[{"id":"S1"},{"id":"S2"}],"assetRefs":[{"assetId":"A","variantId":"base","use":"character","useShotIds":["S1"]},{"assetId":"A","variantId":"base","use":"audio","useShotIds":["S2"]}]});
    let original = unit.clone();
    assert_eq!(asset_use_repair::unit_issues(&unit).unwrap().len(), 1);
    assert_eq!(unit, original);
    for mode in [
        "same-use",
        "overlap",
        "foreign-shot",
        "empty-use",
        "triple",
        "legacy",
    ] {
        let mut bad = unit.clone();
        match mode {
            "same-use" => bad["assetRefs"][1]["use"] = json!("character"),
            "overlap" => bad["assetRefs"][1]["useShotIds"] = json!(["S1"]),
            "foreign-shot" => bad["assetRefs"][1]["useShotIds"] = json!(["other"]),
            "empty-use" => bad["assetRefs"][1]["useShotIds"] = json!([]),
            "triple" => {
                let third = bad["assetRefs"][0].clone();
                bad["assetRefs"].as_array_mut().unwrap().push(third);
            }
            _ => {
                bad.as_object_mut()
                    .unwrap()
                    .remove(super::super::asset_use_binding::FIELD);
            }
        }
        assert!(
            asset_use_repair::unit_issues(&bad).is_err(),
            "{mode} stays hard"
        );
    }
    let mut audio = unit;
    audio["assetRefs"].as_array_mut().unwrap().remove(0);
    assert!(
        asset_use_repair::unit_issues(&audio).unwrap().is_empty(),
        "whole-unit audio use requires no fake visual/voice asset"
    );
}

#[tokio::test]
async fn post_audio_removal_order_gap_uses_only_second_finisher_attempt_and_reopens_exactly(){
    let source=prepare_source("第一集\n场景：门口\n甲：原话完整保留。\n").unwrap();
    let mut input=input(&source);
    super::super::stage3_pack::mark_input(&mut input).unwrap();
    super::super::asset_use_binding::freeze_new(&mut input).unwrap();
    input["assets"]=json!([{"id":"person","variantId":"base","name":"甲","kind":"CHARACTER","version":1,"sourceRefs":[source.rows[2].id],"applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]},"preparation":{"mode":"REFERENCE"}},
        {"id":"prop","variantId":"base","name":"道具","kind":"PROP","version":1,"sourceRefs":[source.rows[2].id],"applicability":{"episodes":[source.episodes[0].id],"scenes":[source.scenes[0].id]},"preparation":{"mode":"REFERENCE"}}]);
    for mode in ["GAP_FIX","GAP_NO_PROGRESS","GAP_LIMIT","GAP_KEEP","GAP_CHANGED","GAP_ZERO","GAP_DUPLICATE","GAP_UNKNOWN"] {
        let model=DuplicateUseModel{inner:Recorded::default(),mode};
        let output=run_storyboard(&input,&model,"offline","medium").await.unwrap();
        let calls=model.inner.calls.lock().await.clone();
        let repairs=calls.iter().filter(|r|r.input["finishRepair"].is_object()).collect::<Vec<_>>();
        let hard=matches!(mode,"GAP_ZERO"|"GAP_DUPLICATE"|"GAP_UNKNOWN");
        assert_eq!(repairs.len(),if hard{1}else{2},"{mode}: {}",output["blocked"]);
        let first_key=serde_json::to_string(repairs[0]).unwrap();
        let first=model.inner.rejected_raw.lock().await[&first_key].clone();
        let first_hash=digest(&first.to_string());
        assert!(!model.inner.accepted.lock().await.contains_key(&first_key));
        if !hard {
            assert_eq!(first["replacements"][0]["units"][0]["assetRefs"].as_array().unwrap().iter().map(|r|r["order"].clone()).collect::<Vec<_>>(),vec![json!(1),json!(3)]);
            assert_eq!(repairs[1].input["finishRepair"]["attempt"],2);
            assert_eq!(repairs[1].input["validationIssues"][0]["code"],"ASSET_USE_ORDER");
            assert_eq!(repairs[1].rule_snapshot["runtimeContract"]["assetUseOrderRepairProfile"],"post-audio-removal-order/v1");
            assert_eq!(serde_json::to_value(creative::prepare_request(repairs[1].clone()).unwrap()).unwrap(),serde_json::to_value(repairs[1]).unwrap());
        }
        if mode=="GAP_FIX" {
            assert_eq!(output["status"],"completed","{}",output["blocked"]);
            assert_eq!(output["units"][0]["assetRefs"].as_array().unwrap().iter().map(|r|r["order"].clone()).collect::<Vec<_>>(),vec![json!(1),json!(2)]);
            assert_eq!(output["units"][0]["shots"][1]["dialogueParts"][0]["text"],"原话完整保留。");
        } else {
            assert!(!output["blocked"].as_array().unwrap().is_empty(),"{mode}");
            assert!(output["units"].as_array().unwrap().is_empty(),"{mode}");
            if mode=="GAP_LIMIT"{assert!(output["blocked"].to_string().contains("FINISH_SCOPE_REPAIR_LIMIT"));}
            if mode=="GAP_NO_PROGRESS"{assert!(output["blocked"].to_string().contains("FINISH_SCOPE_REPAIR_NO_PROGRESS"));}
        }
        assert_eq!(run_storyboard(&input,&model,"offline","medium").await.unwrap(),output,"{mode}");
        assert_eq!(model.inner.calls.lock().await.len(),calls.len(),"{mode}: no new request on reopen");
        assert_eq!(digest(&model.inner.rejected_raw.lock().await[&first_key].to_string()),first_hash);
    }
}

#[test]
#[ignore = "Requires a preserved private read-only job snapshot; never calls a model or changes its DB"]
fn readonly_rejected_finisher_duplicate_use_fixture() {
    let file = std::env::var("DIRECTOR_DUPLICATE_USE_FIXTURE").unwrap();
    let bytes = std::fs::read(&file).unwrap();
    let packet: Value = serde_json::from_slice(&bytes).unwrap();
    let job = packet["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == "608efa24-e03c-4ca8-a057-1109451a4c79")
        .unwrap();
    assert_eq!(job["state"], "rejected");
    assert_eq!(
        job["error"],
        crate::text_executor::AUTHOR_ASSET_USE_DUPLICATE_REJECTION
    );
    let request: Value = serde_json::from_str(job["snapshot"].as_str().unwrap()).unwrap();
    let raw: Value = serde_json::from_str(job["result"].as_str().unwrap()).unwrap();
    let source: LockedSource =
        serde_json::from_str(packet["source"]["data"].as_str().unwrap()).unwrap();
    let source = upgrade_source_dialogues(&source).unwrap();
    let assets: Vec<Value> = packet["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|asset| serde_json::from_str(asset["data"].as_str().unwrap()).unwrap())
        .collect();
    let input = &request["input"];
    // The actual wire omits only exports; its hash still binds the complete
    // accepted author base. Restore that base from the preserved accepted steps,
    // then prove both its original hash and exact wire projection before replay.
    let wire_base = &input["previousDraft"];
    let authors: Vec<Value> = packet["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| step["stage"] == "storyboardCreate" && step["state"] == "accepted")
        .map(|step| serde_json::from_str(step["result"].as_str().unwrap()).unwrap())
        .collect();
    let mut full_base = wire_base.clone();
    for unit in full_base["units"].as_array_mut().unwrap() {
        let matches: Vec<_> = authors
            .iter()
            .flat_map(|author| author["units"].as_array().unwrap())
            .filter(|original| original["id"] == unit["id"])
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "each frozen base unit has one preserved accepted author"
        );
        *unit = matches[0].clone();
    }
    assert_eq!(
        finish_base_input_projection(&full_base),
        *wire_base,
        "accepted base exactly reproduces the frozen wire projection"
    );
    assert_eq!(
        finish_base_hash(&full_base),
        input["baseDraftHash"],
        "do not rewrite the original model hash to force a diagnostic replay"
    );
    let base = &full_base;
    let materialized = materialize_role_result(input, &raw).unwrap();
    let mut draft = apply_finish_delta(base, &materialized).unwrap();
    let unit_ids: HashSet<_> = base["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| text(u, "id").to_owned())
        .collect();
    let shot_ids: HashSet<_> = base["units"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|u| u["shots"].as_array().unwrap())
        .map(|s| text(s, "id").to_owned())
        .collect();
    let episode = input["episodeId"].as_str().unwrap();
    prefix_ids_preserving(&mut draft, &format!("{episode}:"), &unit_ids, &shot_ids).unwrap();
    let refs: Vec<String> = input["sourceRows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| text(row, "id").into())
        .collect();
    let caps = carrier_validation_caps(&draft, &input["capabilities"], true);
    let selected = input["selectedFantasy"].as_array().unwrap();
    let original = draft.clone();
    let (candidate, issues) = asset_use_repair::candidate(
        &source,
        &draft,
        &assets,
        &caps,
        &refs,
        "原画风",
        &format!("{episode}:"),
        &input["plan"],
        selected,
    )
    .unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["code"], "ASSET_USE_IDENTITY");
    assert_eq!(
        validate_storyboard_scope(&source, &candidate, &assets, &caps, Some(&refs)).unwrap_err(),
        crate::text_executor::AUTHOR_ASSET_USE_DUPLICATE_REJECTION
    );
    for mode in ["unknown-asset", "changed-quote", "foreign-source"] {
        let mut bad = draft.clone();
        let unit = &mut bad["units"][1];
        match mode {
            "unknown-asset" => unit["assetRefs"][0]["assetId"] = json!("unknown"),
            "changed-quote" => {
                let part = unit["shots"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .flat_map(|shot| shot["dialogueParts"].as_array_mut().unwrap())
                    .next()
                    .unwrap();
                part["text"] = json!("非法改写");
            }
            _ => unit["shots"][0]["sourceRefs"] = json!(["unknown-row"]),
        }
        assert!(
            asset_use_repair::candidate(
                &source,
                &bad,
                &assets,
                &caps,
                &refs,
                "原画风",
                &format!("{episode}:"),
                &input["plan"],
                selected
            )
            .is_err(),
            "{mode} must stay hard"
        );
    }
    assert_eq!(draft, original);
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    println!("readonly duplicate-use fixture: original rejected raw retained; one narrowly classified conflict; strict adoption still rejects; three unrelated faults stay hard; modelCalls=0 videoSubmissions=0");
}
