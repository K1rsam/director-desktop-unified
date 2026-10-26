//! Explicit owner authorization applies to a newly allocated node only. Old
//! requests keep their original account; UNKNOWN is never a switchable retry.
use super::*;
use std::{cell::RefCell, collections::HashMap, marker::PhantomData, rc::Rc};

const VERSION: &str = "sdk-current-account-authorization/v1";
const AUTHORITY: &str = "explicit-owner-current-connected-account";
const HELD: &str =
    "SDK_CURRENT_ACCOUNT_HANDOFF_HELD: 当前账户交接的原节点或前序证据不一致；未重新提交";
const MAX_LINEAGE: usize = 16_384;

// This is a worklist, not a durable proof cache. Every referenced node has its
// current DB, checkpoint and local files read first. Full ancestor proofs are
// checked once in the same synchronous traversal, before its owner can return.
// Schema-anchor and account-predecessor branches cannot exponentially replay.
// No scope crosses an await or a DB mutation.
struct Lineage {
    owner: String,
    pending: Vec<String>,
    observed: HashMap<String, String>,
}
thread_local! { static LINEAGE: RefCell<Option<Lineage>> = const { RefCell::new(None) }; }
struct Reset(PhantomData<Rc<()>>);
impl Drop for Reset {
    fn drop(&mut self) {
        LINEAGE.with(|state| *state.borrow_mut() = None);
    }
}
fn hash(value: &Value) -> String {
    crate::pipeline::digest(&value.to_string())
}
fn authorization(
    settings: &Value,
    previous: &Value,
    node: &Value,
    current: &Value,
    request: &TextRequest,
    base: &str,
) -> Value {
    json!({"version":VERSION,"authority":AUTHORITY,"scope":"new-unallocated-node",
        "originalAccountFingerprint":settings["textSdkIdentity"]["accountFingerprint"],
        "previousAccountFingerprint":previous["accountFingerprint"],"currentAccountFingerprint":current["accountFingerprint"],
        "previousIdentitySha256":hash(previous),"previousNode":node,"model":request.model,"reasoning":request.reasoning,
        "accountModelChecked":true,"modelCalls":0,"basePolicy":base})
}
pub(super) fn shape_bound(
    proof: &Value,
    settings: &Value,
    current: &Value,
    request: &TextRequest,
    base: &str,
) -> bool {
    proof.as_object().is_some_and(|fields| fields.len() == 13)
        && proof["version"] == VERSION
        && proof["authority"] == AUTHORITY
        && proof["scope"] == "new-unallocated-node"
        && proof["originalAccountFingerprint"] == settings["textSdkIdentity"]["accountFingerprint"]
        && proof["currentAccountFingerprint"] == current["accountFingerprint"]
        && proof["model"] == request.model
        && proof["reasoning"] == request.reasoning
        && proof["accountModelChecked"] == true
        && proof["modelCalls"] == 0
        && proof["basePolicy"] == base
        && (proof["previousAccountFingerprint"] != proof["currentAccountFingerprint"]
            || proof["originalAccountFingerprint"] != proof["currentAccountFingerprint"])
}

impl SavedTextExecutor {
    fn sdk_account_checked<T>(&self, check: impl FnOnce() -> Result<T>) -> Result<T> {
        let owner = hash(&json!([self.store.root, self.workflow_id, self.settings]));
        let nested = LINEAGE.with(|state| -> Result<bool> {
            let mut state = state.borrow_mut();
            if let Some(current) = state.as_ref() {
                return if current.owner == owner {
                    Ok(true)
                } else {
                    Err(HELD.into())
                };
            }
            *state = Some(Lineage {
                owner,
                pending: Vec::new(),
                observed: HashMap::new(),
            });
            Ok(false)
        })?;
        if nested {
            return check();
        }
        let _reset = Reset(PhantomData);
        let _cache = sdk_evidence_cache::scope();
        let output = check()?;
        loop {
            let next = LINEAGE.with(|state| state.borrow_mut().as_mut().unwrap().pending.pop());
            let Some(job) = next else {
                break;
            };
            let value = self.sdk_job(&job)?;
            let same = LINEAGE.with(|state| {
                state.borrow().as_ref().unwrap().observed.get(&job) == Some(&hash(&value))
            });
            if !same {
                return Err(HELD.into());
            }
            self.sdk_raw(&value, &value["snapshot"])?;
        }
        Ok(output)
    }
    fn sdk_account_reference_for(&self, job: &str) -> Result<(Value, Value)> {
        let run = self.workflow_id.as_deref().ok_or(HELD)?;
        let value = self.sdk_job(job)?;
        let request: TextRequest =
            serde_json::from_value(value["snapshot"].clone()).map_err(|_| HELD)?;
        let state = match s(&value, "status") {
            "completed" => "accepted",
            "rejected" => "rejected",
            _ => return Err(HELD.into()),
        };
        let expected = if state == "accepted" {
            &value["prepared"]["sdkDomainResult"]
        } else {
            &value["result"]
        };
        let bound:bool=self.store.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM workflow_steps WHERE workflow_id=?1 AND node_key=?2 AND text_job_id=?3 AND request_hash=?4 AND state=?5 AND result=?6 AND error IS ?7)",
            params![run,step_key(&request),job,hash(&value["snapshot"]),state,expected.to_string(),value["error"].as_str()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !bound
            || value["kind"] != "text"
            || value["provider"] != "codex_sdk"
            || value["prepared"]["sdkOriginWorkflowId"] != run
        {
            return Err(HELD.into());
        }
        // Always reread this node's raw/compiled/attempt/event/receipt evidence.
        let raw = self.sdk_raw_local(&value, &value["snapshot"])?;
        LINEAGE.with(|state| -> Result<()> {
            let mut state = state.borrow_mut();
            let current = state.as_mut().ok_or(HELD)?;
            let digest = hash(&value);
            if let Some(previous) = current.observed.get(job) {
                return if *previous == digest {
                    Ok(())
                } else {
                    Err(HELD.into())
                };
            }
            if current.observed.len() >= MAX_LINEAGE {
                return Err(HELD.into());
            }
            current.observed.insert(job.into(), digest);
            current.pending.push(job.into());
            Ok(())
        })?;
        let proof = json!({"jobId":job,"nodeKey":step_key(&request),"requestHash":hash(&value["snapshot"]),"state":value["status"],
            "preparedSha256":hash(&value["prepared"]),"receiptSha256":hash(&value["receipt"]),"resultSha256":hash(&value["result"]),"rawSha256":hash(&raw)});
        Ok((value["snapshot"]["executorIdentity"].clone(), proof))
    }
    /// Latest terminal identity is distinct from the schema's semantic anchor.
    /// Thus A→B→A records B→A even though the frozen parent still says A.
    pub(super) fn sdk_current_account_reference(&self) -> Result<Option<(Value, Value)>> {
        self.sdk_account_checked(||{
            let Some(run)=self.workflow_id.as_deref() else{return Ok(None);};
            let job:Option<String>=self.store.conn()?.query_row("SELECT j.id FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND ((j.state='completed' AND w.state='accepted') OR (j.state='rejected' AND w.state='rejected')) AND j.result IS NOT NULL ORDER BY j.rowid DESC LIMIT 1",
                [run],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
            job.map(|job|self.sdk_account_reference_for(&job)).transpose()
        })
    }
    pub(super) fn sdk_current_account_authorization(
        &self,
        request: &TextRequest,
        current: &Value,
        reference: Option<&(Value, Value)>,
        base: &str,
    ) -> Option<Value> {
        let previous = reference
            .map(|r| &r.0)
            .unwrap_or(&self.settings["textSdkIdentity"]);
        if previous["accountFingerprint"] == current["accountFingerprint"]
            && self.settings["textSdkIdentity"]["accountFingerprint"]
                == current["accountFingerprint"]
        {
            return None;
        }
        Some(authorization(
            &self.settings,
            previous,
            reference.map(|r| &r.1).unwrap_or(&Value::Null),
            current,
            request,
            base,
        ))
    }
    /// Local historical replay; never checks the currently logged-in account.
    pub(super) fn sdk_current_account_authorization_bound(
        &self,
        snapshot: &Value,
        proof: &Value,
        base: &str,
    ) -> Result<Value> {
        self.sdk_account_checked(||{
            let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
            if !shape_bound(proof,&self.settings,&snapshot["executorIdentity"],&request,base) {return Err(HELD.into());}
            let (previous,node)=if proof["previousNode"].is_null() {
                (self.settings["textSdkIdentity"].clone(),Value::Null)
            }else{
                let prior=s(&proof["previousNode"],"jobId");
                let c=self.store.conn()?;
                let prior_row:i64=c.query_row("SELECT rowid FROM jobs WHERE id=?1",[prior],|r|r.get(0)).map_err(|_|HELD)?;
                let current:Option<i64>=c.query_row("SELECT j.rowid FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND w.node_key=?2",
                    params![self.workflow_id,step_key(&request)],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
                if current.is_some_and(|row|prior_row>=row) {return Err(HELD.into());}
                if current.is_none() {
                    let latest:Option<String>=c.query_row("SELECT j.id FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id WHERE w.workflow_id=?1 AND j.kind='text' AND j.provider='codex_sdk' AND ((j.state='completed' AND w.state='accepted') OR (j.state='rejected' AND w.state='rejected')) AND j.result IS NOT NULL ORDER BY j.rowid DESC LIMIT 1",[self.workflow_id.as_deref().ok_or(HELD)?],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
                    if latest.as_deref()!=Some(prior) {return Err(HELD.into());}
                }
                self.sdk_account_reference_for(prior)?
            };
            if *proof!=authorization(&self.settings,&previous,&node,&snapshot["executorIdentity"],&request,base) {return Err(HELD.into());}
            let mode=if base=="sdk-schema-refusal-continuation/v1" {"schema"}else if previous["bridgeSha256"]==crate::text_sdk::LOCAL_PATH_BRIDGE_SHA {"local"}else{"supervisor"};
            if !crate::text_sdk::current_account_receipt_identity_bound(&previous,&snapshot["executorIdentity"],mode) {return Err(HELD.into());}
            Ok(previous)
        })
    }
}
