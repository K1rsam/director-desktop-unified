//! One explicitly audited correction of an exhausted legacy supply contract.
//! The original author, supplement requests, raw outputs and two repairs remain
//! immutable. Authorization and every new role slot are persisted before use.
use super::*;

pub(super) const CONTRACT: &str = "series-supply-contract-correction/v1";
const AUTH_REQUEST: &str = "series-supply-correction-authorization/v1";
const AUTH_RECEIPT: &str = "series-supply-correction-authorization-receipt/v1";
const FIELD: &str = "seriesSupplyContractCorrections";
const AUTH_EVENT: &str = "series_supply_contract_correction_authorized";
const SLOT_EVENT: &str = "series_supply_contract_correction_attempt_reserved";
const CORRECTION_HELD: &str = "SERIES_SUPPLY_CORRECTION_HELD: 供料合同纠正的原链、显式审核或资产版本不匹配；保留原稿和预算，不重发";

fn sha(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn job_record(c: &rusqlite::Connection, id: &str) -> Result<Value> {
    c.query_row("SELECT snapshot,result,prepared,receipt,state,error,provider,account_id FROM jobs WHERE id=?1 AND kind='text'", [id], |r| {
        Ok(json!({"jobId":id,"snapshot":r.get::<_,String>(0)?,"result":r.get::<_,Option<String>>(1)?,
            "prepared":r.get::<_,Option<String>>(2)?,"receipt":r.get::<_,Option<String>>(3)?,
            "state":r.get::<_,String>(4)?,"error":r.get::<_,Option<String>>(5)?,
            "provider":r.get::<_,String>(6)?,"accountId":r.get::<_,String>(7)?}))
    }).map_err(|_| CORRECTION_HELD.into())
}
fn step_record(c: &rusqlite::Connection, run: &str, job: &str) -> Result<Value> {
    let mut q=c.prepare("SELECT node_key,stage,request_hash,state,result,error FROM workflow_steps WHERE workflow_id=?1 AND text_job_id=?2").map_err(|e|e.to_string())?;
    let rows=q.query_map(params![run,job],|r|Ok(json!({"nodeKey":r.get::<_,String>(0)?,"stage":r.get::<_,String>(1)?,
        "requestHash":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?,"result":r.get::<_,Option<String>>(4)?,"error":r.get::<_,Option<String>>(5)?})))
        .map_err(|e|e.to_string())?.collect::<std::result::Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if rows.len() != 1 {
        return Err(CORRECTION_HELD.into());
    }
    Ok(rows[0].clone())
}
fn proof_rows(c: &rusqlite::Connection, run: &str, origin: &str) -> Result<Vec<Value>> {
    let mut q=c.prepare("SELECT data FROM events WHERE job_id=?1 AND type=?2 AND json_extract(data,'$.originRequestSha256')=?3 ORDER BY id").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![run, AUTH_EVENT, origin], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|s| serde_json::from_str(&s).map_err(|_| CORRECTION_HELD.into()))
        .collect()
}
fn role_receipt(request: &TextRequest, raw: &Value) -> Result<Value> {
    Ok(
        json!({"role":request.input["role"],"stage":request.stage,"executionRoleId":request.input["executionRoleId"],
        "requestSha256":request_hash(request)?,"rawSha256":hash(raw),"packetSha256":hash(&request.input["packet"]),
        "contextIsolation":"fresh_context_required","request":request,"raw":raw}),
    )
}
fn verify_affected(c: &rusqlite::Connection, context: &Value) -> Result<()> {
    let rows = context["authorization"]["affectedAssets"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or(CORRECTION_HELD)?;
    for target in rows {
        let (data, version): (String, i64) = c
            .query_row(
                "SELECT data,version FROM assets WHERE id=?1",
                [s(target, "assetId")],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| CORRECTION_HELD)?;
        let current: Value = serde_json::from_str(&data).map_err(|_| CORRECTION_HELD)?;
        if crate::store::asset_deleted(&current)
            || current["id"] != target["assetId"]
            || current["variantId"] != target["variantId"]
            || version <= 0
            || current["version"] != version
            || target["baseAssetVersion"] != version
            || current["designRevision"] != target["baseDesignRevision"]
            || hash(&current["adoptedDesign"]) != s(target, "baseDesignSha256")
        {
            return Err(CORRECTION_HELD.into());
        }
    }
    Ok(())
}
fn slot(request: &TextRequest, snapshot: &Value, job: &str, run: &str) -> Value {
    let contract = &request.input["seriesSupplyCorrection"];
    json!({"version":"series-supply-correction-role-reservation/v1","workflowId":run,"originRequestSha256":contract["originRequestSha256"],
        "contextSha256":contract["contextSha256"],"phase":contract["phase"],"attempt":contract["attempt"],
        "nodeKey":step_key(request),"requestSha256":hash(snapshot),"textJobId":job,"maximumAttempts":1,"maximumCorrections":1})
}

impl SavedTextExecutor {
    fn supply_correction_origin(&self, origin_sha: &str) -> Result<Value> {
        let run = self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?;
        let c = self.store.conn()?;
        let mut q=c.prepare("SELECT j.id,j.snapshot FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.stage='seriesAuthor' AND j.kind='text' AND j.provider='codex_sdk'").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([run], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut found = None;
        for row in rows {
            let (id, bytes) = row.map_err(|e| e.to_string())?;
            let request: TextRequest = serde_json::from_str(&bytes).map_err(|_| CORRECTION_HELD)?;
            if request_hash(&request)? == origin_sha && found.replace(id).is_some() {
                return Err(CORRECTION_HELD.into());
            }
        }
        self.sdk_job(found.as_deref().ok_or(CORRECTION_HELD)?)
    }
    fn supply_correction_evidence(&self, job: &Value) -> Result<Value> {
        let run = self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?;
        let id = s(job, "id");
        let c = self.store.conn()?;
        // Callers first verify sdk_schema_owned_raw, including the worker files
        // and UNKNOWN gate. Freeze DB/event evidence without repeating that
        // potentially deep historical validation a second time per receipt.
        Ok(
            json!({"record":job_record(&c,id)?,"step":step_record(&c,run,id)?,"eventsSha256":hash(&events(&c,id)?)}),
        )
    }
    fn supply_correction_legacy_chain(&self, origin: &TextRequest) -> Result<(Value, Value)> {
        let run = self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?;
        let origin_sha = request_hash(origin)?;
        let c = self.store.conn()?;
        let mut q=c.prepare("SELECT j.id FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND j.kind='text' AND json_extract(j.snapshot,'$.input.seriesAssetSupplement.originRequestSha256')=?2 ORDER BY CAST(json_extract(j.snapshot,'$.input.seriesAssetSupplement.attempt') AS INTEGER),CASE w.stage WHEN 'seriesDesign' THEN 0 WHEN 'seriesDesignReview' THEN 1 ELSE 2 END").map_err(|e|e.to_string())?;
        let ids = q
            .query_map(params![run, origin_sha], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if ids.len() != 6 {
            return Err(CORRECTION_HELD.into());
        }
        let mut receipts = Vec::new();
        let mut evidence = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            let job = self.sdk_job(id)?;
            let request: TextRequest =
                serde_json::from_value(job["snapshot"].clone()).map_err(|_| CORRECTION_HELD)?;
            if job["provider"] != "codex_sdk"
                || job["status"] != "completed"
                || job["receipt"]["validation"] != "accepted"
                || request.input["seriesAssetSupplement"]["attempt"] != json!(index / 2)
                || request.stage
                    != if index % 2 == 0 {
                        "seriesDesign"
                    } else {
                        "seriesDesignReview"
                    }
            {
                return Err(CORRECTION_HELD.into());
            }
            let raw = self.sdk_schema_owned_raw(&job)?;
            receipts.push(role_receipt(&request, &raw)?);
            evidence.push(self.supply_correction_evidence(&job)?);
        }
        Ok((json!(receipts), json!(evidence)))
    }
    fn verify_supply_correction_proof(
        &self,
        origin: &TextRequest,
        raw: &Value,
        context: &Value,
        proof: &Value,
    ) -> Result<()> {
        let run = self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?;
        let parent = self.store.job(run)?;
        let origin_job = self.supply_correction_origin(&request_hash(origin)?)?;
        if self.settings["textProvider"] != "codex_sdk"
            || parent["kind"] != "workflow"
            || parent["entityId"] != "storyboard.run"
            || parent["snapshot"]["input"]["settings"] != self.settings
            || parent["snapshot"]["runnerVersion"] != "desktop-runner/v1"
            || origin_job["status"] != "rejected"
            || origin_job["receipt"]
                != json!({"contract":"desktop-runner/v1","validation":"rejected"})
            || self.sdk_schema_owned_raw(&origin_job)? != *raw
            || context["workflowId"] != run
            || proof["version"] != AUTH_RECEIPT
            || proof["workflowId"] != run
            || proof["originRequestSha256"] != request_hash(origin)?
            || proof["parentSnapshotSha256"] != hash(&parent["snapshot"])
            || proof["context"] != *context
            || proof["contextSha256"] != hash(context)
            || proof["originEvidence"] != self.supply_correction_evidence(&origin_job)?
            || proof["generationRequestsSent"] != 0
        {
            return Err(CORRECTION_HELD.into());
        }
        let (receipts, evidence) = self.supply_correction_legacy_chain(origin)?;
        if receipts != context["legacyReceipts"] || evidence != proof["legacyEvidence"] {
            return Err(CORRECTION_HELD.into());
        }
        let auth = &proof["authorizationRequest"];
        if auth["version"] != AUTH_REQUEST
            || auth["acknowledgeAdditionalTextUsage"] != true
            || s(auth, "reason").trim().is_empty()
            || auth["expectedWorkflowSnapshotSha256"] != proof["parentSnapshotSha256"]
            || auth["originRequestSha256"] != context["originRequestSha256"]
            || auth["expectedFinalReviewSha256"] != receipts[5]["rawSha256"]
            || auth["auditSha256"] != context["authorization"]["auditSha256"]
            || auth["affectedAssets"] != context["authorization"]["affectedAssets"]
        {
            return Err(CORRECTION_HELD.into());
        }
        // The immutable domain proof is frequently reached again through an
        // ancestor's SDK continuation. All mutable evidence above is rechecked;
        // only the pure, completely input-bound validation can be reused.
        let inputs = serde_json::to_vec(&(
            "sdk-correction-context-domain/v1",
            context,
            origin,
            raw,
            &parent["snapshot"]["input"],
        ))
        .map_err(|e| e.to_string())?;
        crate::text_executor::sdk_evidence_cache::pure_domain(&inputs, || {
            crate::pipeline::series::validate_supply_correction_context(
                context,
                origin,
                raw,
                &parent["snapshot"]["input"],
            )
        })
    }
    pub(in crate::text_executor) fn saved_series_supply_correction_context(
        &self,
        origin: &TextRequest,
        raw: &Value,
    ) -> Result<Option<Value>> {
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        let Some(run) = self.workflow_id.as_deref() else {
            return Ok(None);
        };
        let origin_sha = request_hash(origin)?;
        let parent = self.store.job(run)?;
        let c = self.store.conn()?;
        let proofs = proof_rows(&c, run, &origin_sha)?;
        let context = parent["prepared"]
            .get(FIELD)
            .and_then(|contexts| contexts.get(&origin_sha));
        let Some(context) = context else {
            if !proofs.is_empty() {
                return Err(CORRECTION_HELD.into());
            }
            return Ok(None);
        };
        if proofs.len() != 1 {
            return Err(CORRECTION_HELD.into());
        }
        self.verify_supply_correction_proof(origin, raw, context, &proofs[0])?;
        Ok(Some(context.clone()))
    }
    /// Public action entry, never invoked by a model's `revise` response. The
    /// caller names precise affected versions and acknowledges extra text usage.
    pub(crate) fn authorize_series_supply_correction(
        &self,
        authorization: &Value,
    ) -> Result<Value> {
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        let run = self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?;
        if self.settings["textProvider"] != "codex_sdk"
            || authorization["version"] != AUTH_REQUEST
            || authorization["acknowledgeAdditionalTextUsage"] != true
            || s(authorization, "reason").trim().is_empty()
            || ![
                "expectedWorkflowSnapshotSha256",
                "originRequestSha256",
                "expectedFinalReviewSha256",
                "auditSha256",
            ]
            .iter()
            .all(|k| sha(&authorization[*k]))
        {
            return Err(CORRECTION_HELD.into());
        }
        self.sdk_parent_binding()?;
        let parent = self.store.job(run)?;
        let parent_sha = hash(&parent["snapshot"]);
        if parent["kind"] != "workflow"
            || parent["entityId"] != "storyboard.run"
            || parent["snapshot"]["runnerVersion"] != "desktop-runner/v1"
            || parent["snapshot"]["input"]["workflowProfile"] != crate::asset_graph::PROFILE
            || authorization["expectedWorkflowSnapshotSha256"] != parent_sha
        {
            return Err(CORRECTION_HELD.into());
        }
        let origin_job = self.supply_correction_origin(s(authorization, "originRequestSha256"))?;
        let origin: TextRequest =
            serde_json::from_value(origin_job["snapshot"].clone()).map_err(|_| CORRECTION_HELD)?;
        let raw = self.sdk_schema_owned_raw(&origin_job)?;
        if let Some(context) = self.saved_series_supply_correction_context(&origin, &raw)? {
            let proofs = proof_rows(
                &self.store.conn()?,
                run,
                s(authorization, "originRequestSha256"),
            )?;
            if proofs[0]["authorizationRequest"] != *authorization {
                return Err(CORRECTION_HELD.into());
            }
            return Ok(
                json!({"version":AUTH_RECEIPT,"contextSha256":hash(&context),"reused":true,"generationRequestsSent":0}),
            );
        }
        if !["failed", "blocked", "partially_blocked", "interrupted"]
            .contains(&s(&parent, "status"))
            || self.store.source()?["sha256"] != parent["snapshot"]["input"]["source"]["sha256"]
            || self.store.source()?["version"] != parent["snapshot"]["input"]["source"]["version"]
        {
            return Err(CORRECTION_HELD.into());
        }
        self.check_sdk_unresolved_hold()?;
        check_cli_unknown_hold(&self.store)?;
        self.sdk_request_policy(&origin)?;
        let (receipts, legacy_evidence) = self.supply_correction_legacy_chain(&origin)?;
        let context = json!({"version":CONTRACT,"workflowId":run,"originRequestSha256":request_hash(&origin)?,"originRawSha256":hash(&raw),
            "legacyFailureCode":"SERIES_SUPPLEMENT_REVIEW_REPAIR_HELD","legacyReceipts":receipts,"legacyReceiptsSha256":hash(&receipts),
            "authorization":{"kind":"controller-verified-supply-contract-gap/v1","classification":"existing_design_revision_unsupported",
                "auditSha256":authorization["auditSha256"],"affectedAssets":authorization["affectedAssets"]},"maximumAttempts":1,"maximumCorrections":1});
        let proof = json!({"version":AUTH_RECEIPT,"workflowId":run,"originRequestSha256":request_hash(&origin)?,"parentSnapshotSha256":parent_sha,
            "authorizationRequest":authorization,"context":context,"contextSha256":hash(&context),
            "originEvidence":self.supply_correction_evidence(&origin_job)?,"legacyEvidence":legacy_evidence,"generationRequestsSent":0});
        self.verify_supply_correction_proof(&origin, &raw, &context, &proof)?;
        let mut c = self.store.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (snapshot, prepared, state): (String, Option<String>, String) = tx
            .query_row(
                "SELECT snapshot,prepared,state FROM jobs WHERE id=?1 AND kind='workflow'",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| CORRECTION_HELD)?;
        let mut saved: Value = prepared
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| CORRECTION_HELD)?
            .unwrap_or_else(|| json!({}));
        if saved.is_null() {
            saved = json!({});
        }
        if !saved.is_object()
            || saved.get(FIELD).is_some_and(|v| !v.is_object())
            || saved[FIELD]
                .get(s(&context, "originRequestSha256"))
                .is_some()
            || serde_json::from_str::<Value>(&snapshot).map_err(|_| CORRECTION_HELD)?
                != parent["snapshot"]
            || state != s(&parent, "status")
            || (!parent["prepared"].is_null() && saved != parent["prepared"])
        {
            return Err(CORRECTION_HELD.into());
        }
        let occupied:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE type=?1 AND json_extract(data,'$.originRequestSha256')=?2)",params![AUTH_EVENT,context["originRequestSha256"].as_str()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if occupied {
            return Err(CORRECTION_HELD.into());
        }
        // Revalidate old jobs/raw/receipts under the same write lock as the CAS.
        self.verify_supply_correction_proof(&origin, &raw, &context, &proof)?;
        verify_affected(&tx, &context)?;
        let current_source = self.store.source()?;
        if current_source["sha256"] != parent["snapshot"]["input"]["source"]["sha256"]
            || current_source["version"] != parent["snapshot"]["input"]["source"]["version"]
        {
            return Err(CORRECTION_HELD.into());
        }
        if saved.get(FIELD).is_none() {
            saved[FIELD] = json!({});
        }
        saved[FIELD][s(&context, "originRequestSha256")] = context.clone();
        let timestamp = now();
        let count=tx.execute("UPDATE jobs SET prepared=?1,updated_at=?2 WHERE id=?3 AND snapshot=?4 AND prepared IS ?5 AND state=?6",
            params![saved.to_string(),timestamp,run,snapshot,prepared,state]).map_err(|e|e.to_string())?;
        if count != 1 {
            return Err(CORRECTION_HELD.into());
        }
        tx.execute(
            "INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",
            params![run, AUTH_EVENT, proof.to_string(), timestamp],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(
            json!({"version":AUTH_RECEIPT,"contextSha256":hash(&context),"reused":false,"generationRequestsSent":0}),
        )
    }
    fn supply_correction_request_context<'a>(&self, request: &'a TextRequest) -> Result<&'a Value> {
        let c = &request.input["seriesSupplyCorrection"];
        if c["version"] != CONTRACT
            || c["maximumAttempts"] != 1
            || c["maximumCorrections"] != 1
            || c["attempt"].as_u64().is_none_or(|n| n > 1)
            || request.input["workflowProfile"] != crate::asset_graph::PROFILE
            || request.input["freshContext"] != true
            || request.input.get("validationRepair").is_some()
            || request.input.get("seriesAssetSupplement").is_some()
            || !matches!(
                (
                    request.stage.as_str(),
                    request.skill.as_str(),
                    s(c, "phase")
                ),
                ("seriesDesign", "series-design", "design")
                    | (
                        "seriesDesignReview",
                        "series-design-review",
                        "design-review"
                    )
                    | ("seriesFinisher", "series-finisher", "finisher")
            )
        {
            return Err(CORRECTION_HELD.into());
        }
        Ok(c)
    }
    pub(super) fn sdk_schema_supply_correction_predecessor(
        &self,
        request: &TextRequest,
    ) -> Result<Option<String>> {
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        let c = self.supply_correction_request_context(request)?;
        let origin = self.sdk_supplement_find_exact(
            s(c, "originExecutionRoleId"),
            s(c, "originRequestSha256"),
        )?;
        let previous = self.sdk_supplement_find_exact(
            s(c, "predecessorExecutionRoleId"),
            s(c, "predecessorRequestSha256"),
        )?;
        let raw = self.sdk_schema_owned_raw(&previous)?;
        self.sdk_schema_supply_correction_bound(request, &previous, &raw)?;
        if origin["snapshot"].get(RETRY).is_none() && origin["snapshot"].get(CONTINUE).is_none() {
            return Ok(None);
        }
        Ok(Some(s(&previous, "id").to_owned()))
    }

    /// Revalidate one exact initial correction candidate. Its complete legacy
    /// chain and controller authorization remain mandatory, including raw files.
    pub(super) fn sdk_supply_fact_revalidation_context(
        &self,
        request: &TextRequest,
        raw: &Value,
    ) -> Result<(Value, Value)> {
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        let contract = self.supply_correction_request_context(request)?;
        if contract["phase"] != "design" || contract["attempt"] != 0 {
            return Err(CORRECTION_HELD.into());
        }
        let origin = self.sdk_supplement_find_exact(
            s(contract, "originExecutionRoleId"),
            s(contract, "originRequestSha256"),
        )?;
        let origin_request: TextRequest =
            serde_json::from_value(origin["snapshot"].clone()).map_err(|_| CORRECTION_HELD)?;
        let origin_raw = self.sdk_schema_owned_raw(&origin)?;
        let context = self
            .saved_series_supply_correction_context(&origin_request, &origin_raw)?
            .ok_or(CORRECTION_HELD)?;
        let previous = self.sdk_supplement_find_exact(
            s(contract, "predecessorExecutionRoleId"),
            s(contract, "predecessorRequestSha256"),
        )?;
        let prior_raw = self.sdk_schema_owned_raw(&previous)?;
        self.sdk_schema_supply_correction_bound(request, &previous, &prior_raw)?;
        let prior: TextRequest =
            serde_json::from_value(previous["snapshot"].clone()).map_err(|_| CORRECTION_HELD)?;
        let run = self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?;
        let parent = self.store.job(run)?;
        crate::pipeline::series::revalidate_supply_correction_source_facts_response(
            request,
            raw,
            &context,
            &origin_request,
            &origin_raw,
            &prior,
            &prior_raw,
            &parent["snapshot"]["input"],
        )?;
        let authorization =
            proof_rows(&self.store.conn()?, run, s(contract, "originRequestSha256"))?;
        Ok((
            origin,
            json!({"context":context,"authorization":authorization,
            "candidateOnly":true,"independentReviewStillRequired":true,"generationRequestsSent":0}),
        ))
    }

    /// Current-state gates apply before the first migration CAS. Receipt replay
    /// later verifies frozen evidence, not mutable post-adoption asset versions.
    pub(super) fn sdk_supply_fact_revalidation_current_guard(
        &self,
        request: &TextRequest,
    ) -> Result<()> {
        self.check_sdk_unresolved_hold()?;
        check_cli_unknown_hold(&self.store)?;
        let contract = self.supply_correction_request_context(request)?;
        let origin = self.sdk_supplement_find_exact(
            s(contract, "originExecutionRoleId"),
            s(contract, "originRequestSha256"),
        )?;
        let origin_request: TextRequest =
            serde_json::from_value(origin["snapshot"].clone()).map_err(|_| CORRECTION_HELD)?;
        let context = self
            .saved_series_supply_correction_context(
                &origin_request,
                &self.sdk_schema_owned_raw(&origin)?,
            )?
            .ok_or(CORRECTION_HELD)?;
        verify_affected(&self.store.conn()?, &context)?;
        let parent = self
            .store
            .job(self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?)?;
        let source = self.store.source()?;
        if source["sha256"] != parent["snapshot"]["input"]["source"]["sha256"]
            || source["version"] != parent["snapshot"]["input"]["source"]["version"]
        {
            return Err(CORRECTION_HELD.into());
        }
        Ok(())
    }
    pub(super) fn sdk_schema_supply_correction_bound(
        &self,
        request: &TextRequest,
        previous: &Value,
        previous_raw: &Value,
    ) -> Result<()> {
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        let c = self.supply_correction_request_context(request)?;
        let origin = self.sdk_supplement_find_exact(
            s(c, "originExecutionRoleId"),
            s(c, "originRequestSha256"),
        )?;
        let exact = self.sdk_supplement_find_exact(
            s(c, "predecessorExecutionRoleId"),
            s(c, "predecessorRequestSha256"),
        )?;
        if exact != *previous
            || previous["status"] != "completed"
            || previous["receipt"]["validation"] != "accepted"
            || hash(previous_raw) != s(c, "predecessorRawSha256")
        {
            return Err(CORRECTION_HELD.into());
        }
        let origin_request: TextRequest =
            serde_json::from_value(origin["snapshot"].clone()).map_err(|_| CORRECTION_HELD)?;
        let prior: TextRequest =
            serde_json::from_value(previous["snapshot"].clone()).map_err(|_| CORRECTION_HELD)?;
        let raw = self.sdk_schema_owned_raw(&origin)?;
        let context = self
            .saved_series_supply_correction_context(&origin_request, &raw)?
            .ok_or(CORRECTION_HELD)?;
        if c["contextSha256"] != hash(&context) || c["originRawSha256"] != hash(&raw) {
            return Err(CORRECTION_HELD.into());
        }
        let parent = self
            .store
            .job(self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?)?;
        let inputs = serde_json::to_vec(&(
            "sdk-correction-transition-domain/v1",
            request,
            &context,
            &origin_request,
            &raw,
            &prior,
            previous_raw,
            &parent["snapshot"]["input"],
        ))
        .map_err(|e| e.to_string())?;
        crate::text_executor::sdk_evidence_cache::pure_domain(&inputs, || {
            crate::pipeline::series::validate_supply_correction_transition(
                request,
                &context,
                &origin_request,
                &raw,
                Some(&prior),
                Some(previous_raw),
                &parent["snapshot"]["input"],
            )
        })
    }
    /// Both queued reservation and the later dispatch CAS recheck this contract.
    /// Recovery reads the immutable context; it cannot reserve an occupied slot.
    pub(in crate::text_executor) fn sdk_supply_correction_dispatch_bound(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        c: &rusqlite::Connection,
        existing: Option<&str>,
    ) -> Result<()> {
        if request.input.get("seriesSupplyCorrection").is_none() {
            return Ok(());
        }
        let _cache = crate::text_executor::sdk_evidence_cache::scope();
        if self.settings["textProvider"] != "codex_sdk" {
            return Err(CORRECTION_HELD.into());
        }
        self.sdk_request_policy(request)?;
        if !crate::text_sdk::schema_rejection_receipt_identity_bound(
            &self.settings["textSdkIdentity"],
            &snapshot["executorIdentity"],
        ) {
            return Err(CORRECTION_HELD.into());
        }
        let validation_repair = request.input.get("validationRepair").is_some();
        let contract = if validation_repair {
            let repair = &request.input["validationRepair"];
            let attempt = repair["attempt"]
                .as_u64()
                .filter(|v| (1..=2).contains(v))
                .ok_or(CORRECTION_HELD)?;
            let suffix = format!("/validation-repair/{attempt}");
            let original_role = s(&request.input, "executionRoleId")
                .strip_suffix(&suffix)
                .ok_or(CORRECTION_HELD)?;
            let previous_role = if attempt == 1 {
                original_role.to_owned()
            } else {
                format!("{original_role}/validation-repair/1")
            };
            let previous =
                self.sdk_supplement_find_exact(&previous_role, s(repair, "previousRequestSha256"))?;
            self.sdk_schema_owned_raw(&previous)?;
            // Existing domain-repair validation owns its original two repairs.
            // A correction design/reviewer can never enter this finisher path.
            self.sdk_schema_domain_repair_bound(request, &previous)?;
            if request.input["seriesSupplyCorrection"]["phase"] != "finisher" {
                return Err(CORRECTION_HELD.into());
            }
            &request.input["seriesSupplyCorrection"]
        } else {
            self.supply_correction_request_context(request)?
        };
        let origin = self.sdk_supplement_find_exact(
            s(contract, "originExecutionRoleId"),
            s(contract, "originRequestSha256"),
        )?;
        let origin_request: TextRequest =
            serde_json::from_value(origin["snapshot"].clone()).map_err(|_| CORRECTION_HELD)?;
        let context = self
            .saved_series_supply_correction_context(
                &origin_request,
                &self.sdk_schema_owned_raw(&origin)?,
            )?
            .ok_or(CORRECTION_HELD)?;
        if contract["contextSha256"] != hash(&context) {
            return Err(CORRECTION_HELD.into());
        }
        if !validation_repair {
            let previous = self.sdk_supplement_find_exact(
                s(contract, "predecessorExecutionRoleId"),
                s(contract, "predecessorRequestSha256"),
            )?;
            self.sdk_schema_supply_correction_bound(
                request,
                &previous,
                &self.sdk_schema_owned_raw(&previous)?,
            )?;
        }
        verify_affected(c, &context)?;
        let run = self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?;
        let parent = self.store.job(run)?;
        let current_source = self.store.source()?;
        if parent["status"] == "completed"
            || current_source["sha256"] != parent["snapshot"]["input"]["source"]["sha256"]
            || current_source["version"] != parent["snapshot"]["input"]["source"]["version"]
        {
            return Err(CORRECTION_HELD.into());
        }
        if validation_repair {
            return Ok(());
        }
        let mut q=c.prepare("SELECT id FROM jobs WHERE kind='text' AND json_extract(snapshot,'$.input.seriesSupplyCorrection.originRequestSha256')=?1 AND (json_extract(snapshot,'$.input.seriesSupplyCorrection.phase')='finisher' OR (json_extract(snapshot,'$.input.seriesSupplyCorrection.phase')=?2 AND json_extract(snapshot,'$.input.seriesSupplyCorrection.attempt')=?3))").map_err(|e|e.to_string())?;
        let ids = q
            .query_map(
                params![
                    s(contract, "originRequestSha256"),
                    s(contract, "phase"),
                    contract["attempt"].as_u64()
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if ids.iter().any(|id| Some(id.as_str()) != existing)
            || (existing.is_some() && ids.len() != 1)
        {
            return Err(CORRECTION_HELD.into());
        }
        let mut q=c.prepare("SELECT data FROM events WHERE job_id=?1 AND type=?2 AND json_extract(data,'$.originRequestSha256')=?3 AND (json_extract(data,'$.phase')='finisher' OR (json_extract(data,'$.phase')=?4 AND json_extract(data,'$.attempt')=?5))").map_err(|e|e.to_string())?;
        let records = q
            .query_map(
                params![
                    run,
                    SLOT_EVENT,
                    s(contract, "originRequestSha256"),
                    s(contract, "phase"),
                    contract["attempt"].as_u64()
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        match existing {
            Some(job)
                if records.len() == 1
                    && records[0] == slot(request, snapshot, job, run).to_string() =>
            {
                Ok(())
            }
            None if records.is_empty() => Ok(()),
            _ => Err(CORRECTION_HELD.into()),
        }
    }
    pub(in crate::text_executor) fn sdk_supply_correction_record_allocation(
        &self,
        request: &TextRequest,
        snapshot: &Value,
        job: &str,
        c: &rusqlite::Connection,
    ) -> Result<()> {
        if request.input.get("seriesSupplyCorrection").is_none()
            || request.input.get("validationRepair").is_some()
        {
            return Ok(());
        }
        let run = self.workflow_id.as_deref().ok_or(CORRECTION_HELD)?;
        c.execute(
            "INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",
            params![
                run,
                SLOT_EVENT,
                slot(request, snapshot, job, run).to_string(),
                now()
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "text_executor_sdk_supply_correction_tests.rs"]
mod tests;
