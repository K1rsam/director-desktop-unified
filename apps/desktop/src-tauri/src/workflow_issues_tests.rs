use super::*;

fn issue()->Issue {Issue{version:VERSION.into(),source_sha256:"a".repeat(64),origin_sha256:"b".repeat(64),
    stage:"foundation:F2".into(),code:"IDENTITY_CONFLICT".into(),object:"asset:one".into(),location:"/assets/asset:one/identity".into(),
    evidence:vec![Evidence{kind:"source_row".into(),reference:"row:source:1:20".into(),sha256:"c".repeat(64)}],
    impact:vec!["episode:1".into()],repair:Repair::AiRevision,message:"首次中文说明".into()}}
fn attempt(key:&str)->Attempt {Attempt{attempt_key:key.into(),request_sha256:"d".repeat(64),base_sha256:"b".repeat(64),
    workflow_id:"run-one".into(),account:"account-one".into(),provider:"codex_sdk".into(),repair:Repair::AiRevision,action:"conflict-decision/v1".into()}}
fn outcome(accepted:bool)->Outcome {Outcome{accepted,result_sha256:"e".repeat(64),progress_sha256:"f".repeat(64),code:if accepted{"VALIDATED"}else{"IDENTITY_UNRESOLVED"}.into(),finding_sha256:"1".repeat(64)}}
fn transact<T>(c:&mut Connection,f:impl FnOnce(&mut Transaction<'_>)->Result<T>)->Result<T> {
    let mut tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let result=f(&mut tx);
    // Commit even an error, proving each API's savepoint rolls itself back.
    tx.commit().unwrap();result
}
fn fixture()->(Connection,String) {
    let mut c=Connection::open_in_memory().unwrap();
    let ledger=transact(&mut c,|tx|{install(tx)?;register(tx,&issue())}).unwrap();(c,ledger.cause)
}
fn counts(c:&Connection)->(i64,i64) {
    (c.query_row("SELECT COUNT(*) FROM workflow_issue_v1",[],|r|r.get(0)).unwrap(),
     c.query_row("SELECT COUNT(*) FROM workflow_issue_attempt_v1",[],|r|r.get(0)).unwrap())
}

#[test]
fn workflow_issues_legacy_read_is_zero_ddl_and_install_is_explicit_transactional() {
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("old.sqlite");
    {let c=Connection::open(&path).unwrap();c.execute_batch("CREATE TABLE original(id TEXT PRIMARY KEY); INSERT INTO original VALUES('preserved');").unwrap();}
    let bytes=std::fs::read(&path).unwrap();
    let read_only=Connection::open_with_flags(&path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(read(&read_only,&issue().cause().unwrap()).unwrap().is_none());drop(read_only);
    assert_eq!(std::fs::read(&path).unwrap(),bytes);
    let mut c=Connection::open(&path).unwrap();
    {let tx=c.transaction().unwrap();install(&tx).unwrap();tx.rollback().unwrap();}
    assert!(read(&c,&issue().cause().unwrap()).unwrap().is_none());
    assert!(transact(&mut c,|tx|register(tx,&issue())).is_err());
    assert_eq!(c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'workflow_issue%'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(c.query_row("SELECT id FROM original",[],|r|r.get::<_,String>(0)).unwrap(),"preserved");
}

#[test]
fn workflow_issues_cause_excludes_message_and_freezes_original_evidence_and_repair_class() {
    let (mut c,cause)=fixture();let before=read(&c,&cause).unwrap();
    let mut changed=issue();changed.message="完全不同的新说明，允许重试".into();
    assert_eq!(changed.cause().unwrap(),cause);
    assert_eq!(Some(transact(&mut c,|tx|register(tx,&changed)).unwrap()),before);
    assert_eq!(c.query_row("SELECT original_message FROM workflow_issue_v1 WHERE cause=?1",[&cause],|r|r.get::<_,String>(0)).unwrap(),"首次中文说明");
    changed.evidence[0].sha256="2".repeat(64);
    assert_eq!(changed.cause().unwrap(),cause);
    assert!(transact(&mut c,|tx|register(tx,&changed)).unwrap_err().contains("ORIGINAL_CONFLICT"));
    let mut changed=issue();changed.repair=Repair::Program;
    assert_eq!(changed.cause().unwrap(),cause);
    assert!(transact(&mut c,|tx|register(tx,&changed)).unwrap_err().contains("ORIGINAL_CONFLICT"));
    assert_eq!(read(&c,&cause).unwrap(),before);assert_eq!(counts(&c),(1,0));
    let mut alternate=issue();alternate.source_sha256=alternate.source_sha256.to_uppercase();
    assert!(alternate.cause().is_err(),"alternate hash casing cannot create a new budget");
}

#[test]
fn workflow_issues_allocation_and_completion_are_idempotent_and_cas_bound() {
    let (mut c,cause)=fixture();let a=attempt("one");
    let first=transact(&mut c,|tx|reserve(tx,&cause,0,&a)).unwrap();assert!(first.fresh);assert_eq!(first.revision,1);
    let replay=transact(&mut c,|tx|reserve(tx,&cause,0,&a)).unwrap();assert!(!replay.fresh);assert_eq!(replay.ordinal,1);
    let mut changed=a.clone();changed.request_sha256="3".repeat(64);
    assert!(transact(&mut c,|tx|reserve(tx,&cause,1,&changed)).unwrap_err().contains("ATTEMPT_CONFLICT"));
    assert!(transact(&mut c,|tx|finish(tx,&cause,1,0,&outcome(false),None)).unwrap_err().contains("CAS_CONFLICT"));
    let done=transact(&mut c,|tx|finish(tx,&cause,1,1,&outcome(false),None)).unwrap();assert_eq!(done.revision,2);
    assert_eq!(transact(&mut c,|tx|finish(tx,&cause,1,1,&outcome(false),None)).unwrap(),done);
    assert!(transact(&mut c,|tx|finish(tx,&cause,1,2,&outcome(true),None)).unwrap_err().contains("TERMINAL_CONFLICT"));
    let mut second=attempt("two");second.base_sha256=outcome(false).result_sha256;
    assert!(transact(&mut c,|tx|reserve(tx,&cause,1,&second)).unwrap_err().contains("CAS_CONFLICT"));
    second.base_sha256="b".repeat(64);
    assert!(transact(&mut c,|tx|reserve(tx,&cause,2,&second)).unwrap_err().contains("BASE_CHANGED"));
    assert_eq!(read(&c,&cause).unwrap().unwrap(),done);assert_eq!(counts(&c),(1,1));
}

#[test]
fn workflow_issues_budget_survives_new_workflow_account_and_database_reopen() {
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("project.sqlite");let cause=issue().cause().unwrap();
    {
        let mut c=Connection::open(&path).unwrap();
        transact(&mut c,|tx|{install(tx)?;register(tx,&issue())?;reserve(tx,&cause,0,&attempt("one"))}).unwrap();
        transact(&mut c,|tx|finish(tx,&cause,1,1,&outcome(false),None)).unwrap();
    }
    let mut c=Connection::open(&path).unwrap();let mut second=attempt("two");
    second.base_sha256=outcome(false).result_sha256;second.workflow_id="different-run".into();second.account="backup-account".into();second.provider="api".into();
    let second=transact(&mut c,|tx|reserve(tx,&cause,2,&second)).unwrap();assert_eq!(second.ordinal,2);
    let mut result=outcome(false);result.finding_sha256="2".repeat(64);result.progress_sha256="3".repeat(64);
    let done=transact(&mut c,|tx|finish(tx,&cause,2,3,&result,None)).unwrap();assert_eq!(done.state,"exhausted");
    drop(c);let mut c=Connection::open(&path).unwrap();
    let mut third=attempt("three");third.base_sha256=result.result_sha256;third.workflow_id="third-run".into();third.account="third-account".into();
    assert!(transact(&mut c,|tx|reserve(tx,&cause,4,&third)).unwrap_err().contains("REVISION_HELD"));
    assert_eq!(counts(&c),(1,2));assert_eq!(read(&c,&cause).unwrap().unwrap(),done);
}

#[test]
fn workflow_issues_same_finding_without_progress_stops_and_message_cannot_unlock() {
    let (mut c,cause)=fixture();transact(&mut c,|tx|reserve(tx,&cause,0,&attempt("one"))).unwrap();
    transact(&mut c,|tx|finish(tx,&cause,1,1,&outcome(false),None)).unwrap();
    let mut second=attempt("two");second.base_sha256=outcome(false).result_sha256;
    transact(&mut c,|tx|reserve(tx,&cause,2,&second)).unwrap();
    let done=transact(&mut c,|tx|finish(tx,&cause,2,3,&outcome(false),None)).unwrap();assert_eq!(done.state,"held");
    let mut changed=issue();changed.message="已修复，现在重新生成".into();
    assert_eq!(transact(&mut c,|tx|register(tx,&changed)).unwrap(),done);
    assert!(transact(&mut c,|tx|reserve(tx,&cause,4,&attempt("third"))).is_err());assert_eq!(counts(&c),(1,2));
}

#[test]
fn workflow_issues_unknown_requires_bound_receipt_before_any_terminal_or_new_attempt() {
    let (mut c,cause)=fixture();let a=attempt("one");transact(&mut c,|tx|reserve(tx,&cause,0,&a)).unwrap();
    let unknown=transact(&mut c,|tx|mark_unknown(tx,&cause,1,1,&"4".repeat(64))).unwrap();assert_eq!(unknown.revision,2);
    assert_eq!(transact(&mut c,|tx|mark_unknown(tx,&cause,1,1,&"4".repeat(64))).unwrap(),unknown);
    for next in [a.clone(),attempt("replacement")] {
        assert!(transact(&mut c,|tx|reserve(tx,&cause,2,&next)).unwrap_err().contains("RECONCILE_FIRST"));
    }
    assert!(transact(&mut c,|tx|finish(tx,&cause,1,2,&outcome(false),None)).unwrap_err().contains("UNKNOWN_RECONCILE_FIRST"));
    let mut proof=Reconciliation{attempt_key:a.attempt_key,request_sha256:"5".repeat(64),receipt_sha256:"6".repeat(64),result_sha256:outcome(true).result_sha256};
    assert!(transact(&mut c,|tx|finish(tx,&cause,1,2,&outcome(true),Some(&proof))).unwrap_err().contains("RECONCILIATION_UNBOUND"));
    assert_eq!(read(&c,&cause).unwrap().unwrap(),unknown);assert_eq!(counts(&c),(1,1));
    proof.request_sha256=a.request_sha256;
    let done=transact(&mut c,|tx|finish(tx,&cause,1,2,&outcome(true),Some(&proof))).unwrap();assert_eq!(done.state,"resolved");
    assert_eq!(transact(&mut c,|tx|finish(tx,&cause,1,2,&outcome(true),Some(&proof))).unwrap(),done);
    assert_eq!(c.query_row("SELECT unknown_sha256 FROM workflow_issue_attempt_v1 WHERE cause=?1",[&cause],|r|r.get::<_,String>(0)).unwrap(),"4".repeat(64));
}

#[test]
fn workflow_issues_program_repairs_are_typed_local_actions_not_ai_dispatches() {
    let mut c=Connection::open_in_memory().unwrap();let mut original=issue();original.repair=Repair::Program;
    let ledger=transact(&mut c,|tx|{install(tx)?;register(tx,&original)}).unwrap();let cause=ledger.cause;
    assert!(transact(&mut c,|tx|reserve(tx,&cause,0,&attempt("wrong-ai"))).unwrap_err().contains("REPAIR_CLASS_CHANGED"));
    let mut local=attempt("program-one");local.repair=Repair::Program;local.provider="local".into();local.account.clear();local.action="canonical-id-restore/v1".into();
    let mut forged=local.clone();forged.provider="codex_sdk".into();
    assert!(transact(&mut c,|tx|reserve(tx,&cause,0,&forged)).unwrap_err().contains("ATTEMPT_INVALID"));
    transact(&mut c,|tx|reserve(tx,&cause,0,&local)).unwrap();
    assert_eq!(transact(&mut c,|tx|finish(tx,&cause,1,1,&outcome(true),None)).unwrap().state,"resolved");
    assert_eq!(counts(&c),(1,1));
}

#[test]
fn workflow_issues_savepoint_rollback_preserves_counter_when_insert_fails() {
    let (mut c,cause)=fixture();
    c.execute_batch("CREATE TRIGGER deny_attempt BEFORE INSERT ON workflow_issue_attempt_v1 BEGIN SELECT RAISE(ABORT,'synthetic insert failure'); END;").unwrap();
    let before=read(&c,&cause).unwrap();
    assert!(transact(&mut c,|tx|reserve(tx,&cause,0,&attempt("one"))).is_err());
    assert_eq!(read(&c,&cause).unwrap(),before);assert_eq!(counts(&c),(1,0));
    c.execute_batch("DROP TRIGGER deny_attempt; UPDATE workflow_issue_v1 SET issue_json='{}';").unwrap();
    assert!(read(&c,&cause).unwrap_err().contains("EVIDENCE_CHANGED"));
}
