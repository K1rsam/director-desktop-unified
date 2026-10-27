//! Versioned issue identity and bounded revision ledger, not a task executor.
//! Integration must validate source/provider evidence before calling this module.
//! Reads never install tables. Call install explicitly inside a write transaction.
use crate::pipeline::digest;
use crate::store::Result;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const VERSION:&str="workflow-issues/v1";

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub(crate) enum Repair { Program, AiRevision, Manual }

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct Evidence {
    pub kind:String,
    pub reference:String,
    pub sha256:String,
}

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct Issue {
    pub version:String,
    pub source_sha256:String,
    /// Immutable original draft, not the next repaired draft or runtime account.
    pub origin_sha256:String,
    pub stage:String,
    pub code:String,
    pub object:String,
    pub location:String,
    pub evidence:Vec<Evidence>,
    pub impact:Vec<String>,
    pub repair:Repair,
    /// Presentation only. Never affects cause, replay eligibility, or budget.
    pub message:String,
}

fn sha(s:&str)->bool {s.len()==64 && s.bytes().all(|v|v.is_ascii_digit()||(b'a'..=b'f').contains(&v))}
fn encoded<T:Serialize>(value:&T)->Result<String> {serde_json::to_string(value).map_err(|e|e.to_string())}
fn sql(error:rusqlite::Error)->String {format!("WORKFLOW_ISSUE_STORAGE:{error}")}
fn valid_code(s:&str)->bool {!s.is_empty() && s.bytes().all(|c|c.is_ascii_uppercase()||c.is_ascii_digit()||c==b'_')}

impl Issue {
    fn frozen(&self)->Result<Value> {
        if self.version!=VERSION || !sha(&self.source_sha256)||!sha(&self.origin_sha256)
            || !valid_code(&self.code)||self.stage.is_empty()||self.object.is_empty()||!self.location.starts_with('/')
            || self.evidence.is_empty()||self.evidence.iter().any(|e|e.kind.is_empty()||e.reference.is_empty()||!sha(&e.sha256))
            || self.impact.is_empty()||self.impact.iter().any(String::is_empty) {
            return Err("WORKFLOW_ISSUE_INVALID".into());
        }
        let mut value=serde_json::to_value(self).map_err(|e|e.to_string())?;
        value.as_object_mut().ok_or("WORKFLOW_ISSUE_INVALID")?.remove("message");
        Ok(value)
    }
    pub(crate) fn cause(&self)->Result<String> {
        self.frozen()?;
        // Evidence/repair cannot be changed to buy another budget: they are
        // frozen against this cause by register. A genuinely new origin is new.
        Ok(digest(&json!({"version":VERSION,"sourceSha256":self.source_sha256,
            "originSha256":self.origin_sha256,"stage":self.stage,"code":self.code,
            "object":self.object,"location":self.location}).to_string()))
    }
}

pub(crate) fn install(tx:&Transaction<'_>)->Result<()> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS workflow_issue_v1(
        cause TEXT PRIMARY KEY, issue_sha256 TEXT NOT NULL, issue_json TEXT NOT NULL,
        original_message TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('pending','resolved','held','exhausted')),
        revision INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 2));
        CREATE TABLE IF NOT EXISTS workflow_issue_attempt_v1(
        cause TEXT NOT NULL REFERENCES workflow_issue_v1(cause), ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 2),
        attempt_key TEXT NOT NULL UNIQUE, request_json TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('reserved','unknown','accepted','rejected')),
        outcome_json TEXT, unknown_sha256 TEXT, reconciliation_json TEXT,
        PRIMARY KEY(cause,ordinal));") .map_err(sql)
}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq)]
#[serde(rename_all="camelCase")]
pub(crate) struct Ledger {
    pub cause:String,
    pub issue:Value,
    pub state:String,
    pub revision:u64,
    pub attempts:u64,
}

/// Old databases are observed without DDL or opportunistic migration.
pub(crate) fn read(c:&Connection,cause:&str)->Result<Option<Ledger>> {
    let installed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_issue_v1')",[],|r|r.get(0)).map_err(sql)?;
    if !installed{return Ok(None);}
    let row:Option<(String,String,String,u64,u64)>=c.query_row(
        "SELECT issue_json,issue_sha256,state,revision,attempts FROM workflow_issue_v1 WHERE cause=?1",[cause],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(sql)?;
    row.map(|(body,hash,state,revision,attempts)|{
        if digest(&body)!=hash{return Err("WORKFLOW_ISSUE_EVIDENCE_CHANGED".into());}
        let issue=serde_json::from_str(&body).map_err(|e|format!("WORKFLOW_ISSUE_INVALID:{e}"))?;
        Ok(Ledger{cause:cause.into(),issue,state,revision,attempts})
    }).transpose()
}

pub(crate) fn register(tx:&mut Transaction<'_>,issue:&Issue)->Result<Ledger> {
    let cause=issue.cause()?;let frozen=issue.frozen()?;let body=frozen.to_string();
    let save=tx.savepoint().map_err(sql)?;
    if let Some(old)=read(&save,&cause)? {
        if old.issue!=frozen{return Err("WORKFLOW_ISSUE_ORIGINAL_CONFLICT".into());}
        save.commit().map_err(sql)?;return Ok(old);
    }
    // Missing install is an explicit integration error, never hidden DDL.
    save.execute("INSERT INTO workflow_issue_v1(cause,issue_sha256,issue_json,original_message,state) VALUES(?1,?2,?3,?4,'pending')",
        params![cause,digest(&body),body,issue.message]).map_err(sql)?;
    let result=read(&save,&cause)?.ok_or("WORKFLOW_ISSUE_MISSING")?;
    save.commit().map_err(sql)?;Ok(result)
}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
pub(crate) struct Attempt {
    pub attempt_key:String,
    pub request_sha256:String,
    pub base_sha256:String,
    pub workflow_id:String,
    pub account:String,
    pub provider:String,
    pub repair:Repair,
    /// Deterministic program action or versioned AI revision contract identifier.
    pub action:String,
}
impl Attempt {
    fn validate(&self)->Result<()> {
        if self.attempt_key.is_empty()||!sha(&self.request_sha256)||!sha(&self.base_sha256)
            ||self.workflow_id.is_empty()||self.action.is_empty()||self.repair==Repair::Manual
            || (self.repair==Repair::Program && (self.provider!="local"||!self.account.is_empty()))
            || (self.repair==Repair::AiRevision && (self.provider.is_empty()||self.provider=="local"||self.account.is_empty())) {
            return Err("WORKFLOW_ISSUE_ATTEMPT_INVALID".into());
        }
        Ok(())
    }
}

#[derive(Debug,PartialEq)]
pub(crate) struct Reservation {pub ordinal:u64,pub revision:u64,pub fresh:bool,pub state:String}

/// Caller must allocate its normal job in this same transaction. This ledger
/// has no process/network API and a reservation alone is not dispatch authority.
pub(crate) fn reserve(tx:&mut Transaction<'_>,cause:&str,expected_revision:u64,attempt:&Attempt)->Result<Reservation> {
    attempt.validate()?;let body=encoded(attempt)?;
    let save=tx.savepoint().map_err(sql)?;
    let ledger=read(&save,cause)?.ok_or("WORKFLOW_ISSUE_MISSING")?;
    let existing:Option<(String,u64,String,String)>=save.query_row("SELECT cause,ordinal,request_json,state FROM workflow_issue_attempt_v1 WHERE attempt_key=?1",
        [&attempt.attempt_key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
    if let Some((original,ordinal,request,state))=existing {
        if original!=cause||request!=body{return Err("WORKFLOW_ISSUE_ATTEMPT_CONFLICT".into());}
        if state=="unknown" {return Err("WORKFLOW_ISSUE_UNKNOWN_RECONCILE_FIRST".into());}
        save.commit().map_err(sql)?;
        return Ok(Reservation{ordinal,revision:ledger.revision,fresh:false,state});
    }
    let active:bool=save.query_row("SELECT EXISTS(SELECT 1 FROM workflow_issue_attempt_v1 WHERE cause=?1 AND state IN ('reserved','unknown'))",[cause],|r|r.get(0)).map_err(sql)?;
    if active{return Err("WORKFLOW_ISSUE_UNRESOLVED_RECONCILE_FIRST".into());}
    if ledger.revision!=expected_revision{return Err("WORKFLOW_ISSUE_CAS_CONFLICT".into());}
    if ledger.state!="pending"||ledger.attempts>=2{return Err("WORKFLOW_ISSUE_REVISION_HELD".into());}
    if ledger.issue["repair"]!=serde_json::to_value(&attempt.repair).map_err(|e|e.to_string())? {
        return Err("WORKFLOW_ISSUE_REPAIR_CLASS_CHANGED".into());
    }
    let ordinal=ledger.attempts+1;
    // First repair starts from the frozen origin. Later repairs bind the latest
    // returned result, rather than quietly substituting an unrelated draft.
    let expected_base=if ordinal==1 {ledger.issue["originSha256"].as_str().ok_or("WORKFLOW_ISSUE_INVALID")?.to_owned()}
        else {
            let text:String=save.query_row("SELECT outcome_json FROM workflow_issue_attempt_v1 WHERE cause=?1 AND ordinal=?2 AND state='rejected'",
                params![cause,ordinal-1],|r|r.get(0)).map_err(sql)?;
            let value:Outcome=serde_json::from_str(&text).map_err(|e|e.to_string())?;value.result_sha256
        };
    if attempt.base_sha256!=expected_base{return Err("WORKFLOW_ISSUE_BASE_CHANGED".into());}
    let n=save.execute("UPDATE workflow_issue_v1 SET revision=revision+1,attempts=attempts+1 WHERE cause=?1 AND revision=?2 AND state='pending' AND attempts=?3",
        params![cause,expected_revision,ordinal-1]).map_err(sql)?;
    if n!=1{return Err("WORKFLOW_ISSUE_CAS_CONFLICT".into());}
    save.execute("INSERT INTO workflow_issue_attempt_v1(cause,ordinal,attempt_key,request_json,state) VALUES(?1,?2,?3,?4,'reserved')",
        params![cause,ordinal,attempt.attempt_key,body]).map_err(sql)?;
    save.commit().map_err(sql)?;
    Ok(Reservation{ordinal,revision:expected_revision+1,fresh:true,state:"reserved".into()})
}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
pub(crate) struct Outcome {
    pub accepted:bool,
    pub result_sha256:String,
    pub progress_sha256:String,
    /// Stable validator code; localized message is never a recovery signal.
    pub code:String,
    pub finding_sha256:String,
}
impl Outcome {
    fn validate(&self)->Result<()> {
        if !sha(&self.result_sha256)||!sha(&self.progress_sha256)||!sha(&self.finding_sha256)||!valid_code(&self.code) {
            return Err("WORKFLOW_ISSUE_OUTCOME_INVALID".into());
        }
        Ok(())
    }
}

pub(crate) fn mark_unknown(tx:&mut Transaction<'_>,cause:&str,ordinal:u64,expected_revision:u64,evidence_sha256:&str)->Result<Ledger> {
    if !sha(evidence_sha256){return Err("WORKFLOW_ISSUE_UNKNOWN_EVIDENCE_INVALID".into());}
    let save=tx.savepoint().map_err(sql)?;let ledger=read(&save,cause)?.ok_or("WORKFLOW_ISSUE_MISSING")?;
    let row:Option<(String,Option<String>)>=save.query_row("SELECT state,unknown_sha256 FROM workflow_issue_attempt_v1 WHERE cause=?1 AND ordinal=?2",params![cause,ordinal],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
    if row==Some(("unknown".into(),Some(evidence_sha256.into()))){save.commit().map_err(sql)?;return Ok(ledger);}
    if ledger.revision!=expected_revision||row.as_ref().is_none_or(|r|r.0!="reserved") {return Err("WORKFLOW_ISSUE_CAS_CONFLICT".into());}
    let n=save.execute("UPDATE workflow_issue_v1 SET revision=revision+1 WHERE cause=?1 AND revision=?2",params![cause,expected_revision]).map_err(sql)?;
    if n!=1{return Err("WORKFLOW_ISSUE_CAS_CONFLICT".into());}
    save.execute("UPDATE workflow_issue_attempt_v1 SET state='unknown',unknown_sha256=?3 WHERE cause=?1 AND ordinal=?2 AND state='reserved'",params![cause,ordinal,evidence_sha256]).map_err(sql)?;
    let result=read(&save,cause)?.ok_or("WORKFLOW_ISSUE_MISSING")?;save.commit().map_err(sql)?;Ok(result)
}

/// Supplied only after the existing provider adapter has verified the original
/// immutable receipt. This module binds that proof; it cannot verify a service.
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct Reconciliation {pub attempt_key:String,pub request_sha256:String,pub receipt_sha256:String,pub result_sha256:String}

pub(crate) fn finish(tx:&mut Transaction<'_>,cause:&str,ordinal:u64,expected_revision:u64,outcome:&Outcome,reconciliation:Option<&Reconciliation>)->Result<Ledger> {
    outcome.validate()?;let body=encoded(outcome)?;
    let proof=reconciliation.map(encoded).transpose()?;
    let save=tx.savepoint().map_err(sql)?;let ledger=read(&save,cause)?.ok_or("WORKFLOW_ISSUE_MISSING")?;
    let (state,request,old_outcome,old_proof):(String,String,Option<String>,Option<String>)=save.query_row(
        "SELECT state,request_json,outcome_json,reconciliation_json FROM workflow_issue_attempt_v1 WHERE cause=?1 AND ordinal=?2",params![cause,ordinal],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(sql)?;
    if let Some(old)=old_outcome {
        if old!=body||old_proof!=proof{return Err("WORKFLOW_ISSUE_TERMINAL_CONFLICT".into());}
        save.commit().map_err(sql)?;return Ok(ledger);
    }
    if ledger.revision!=expected_revision{return Err("WORKFLOW_ISSUE_CAS_CONFLICT".into());}
    if state=="unknown" {
        let proof=reconciliation.ok_or("WORKFLOW_ISSUE_UNKNOWN_RECONCILE_FIRST")?;
        let attempt:Attempt=serde_json::from_str(&request).map_err(|e|e.to_string())?;
        if proof.attempt_key!=attempt.attempt_key||proof.request_sha256!=attempt.request_sha256
            ||!sha(&proof.receipt_sha256)||proof.result_sha256!=outcome.result_sha256 {
            return Err("WORKFLOW_ISSUE_RECONCILIATION_UNBOUND".into());
        }
    } else if state!="reserved"||reconciliation.is_some() {return Err("WORKFLOW_ISSUE_TERMINAL_CONFLICT".into());}
    let no_progress=if !outcome.accepted && ordinal>1 {
        let prior:String=save.query_row("SELECT outcome_json FROM workflow_issue_attempt_v1 WHERE cause=?1 AND ordinal=?2",params![cause,ordinal-1],|r|r.get(0)).map_err(sql)?;
        let previous:Outcome=serde_json::from_str(&prior).map_err(|e|e.to_string())?;
        !previous.accepted && previous.code==outcome.code && previous.finding_sha256==outcome.finding_sha256
            && previous.progress_sha256==outcome.progress_sha256
    } else {false};
    let next=if outcome.accepted {"resolved"}else if no_progress {"held"}else if ledger.attempts>=2 {"exhausted"}else{"pending"};
    let n=save.execute("UPDATE workflow_issue_v1 SET state=?3,revision=revision+1 WHERE cause=?1 AND revision=?2 AND state='pending'",params![cause,expected_revision,next]).map_err(sql)?;
    if n!=1{return Err("WORKFLOW_ISSUE_CAS_CONFLICT".into());}
    save.execute("UPDATE workflow_issue_attempt_v1 SET state=?3,outcome_json=?4,reconciliation_json=?5 WHERE cause=?1 AND ordinal=?2 AND outcome_json IS NULL",
        params![cause,ordinal,if outcome.accepted{"accepted"}else{"rejected"},body,proof]).map_err(sql)?;
    let result=read(&save,cause)?.ok_or("WORKFLOW_ISSUE_MISSING")?;save.commit().map_err(sql)?;Ok(result)
}

#[cfg(test)]
#[path="workflow_issues_tests.rs"]
mod tests;
