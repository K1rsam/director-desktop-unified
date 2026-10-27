use super::*;

fn database() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT); INSERT INTO metadata VALUES('schema_version','2'); CREATE TABLE jobs(id TEXT PRIMARY KEY,state TEXT); INSERT INTO jobs VALUES('old','completed');").unwrap();
    let tx = c.transaction().unwrap();
    install(&tx).unwrap();
    tx.commit().unwrap();
    c
}
fn base(id: &str, kind: Kind) -> Artifact {
    Artifact {
        id: id.into(),
        identity: "episode:1/storyboard".into(),
        kind,
        contract_version: "storyboard/v3".into(),
        source_sha256: "a".repeat(64),
        inputs: BTreeMap::from([("source".into(), "a".repeat(64)), ("rules".into(), "b".repeat(64))]),
        producer: "synthetic-test".into(),
        author_attempt: "author-1".into(),
        parents: vec![],
        payload: json!({"text":"源句完整保留。"}),
    }
}
fn candidate(tx: &Transaction<'_>) -> StoredArtifact {
    put(tx, &base("raw", Kind::Raw)).unwrap();
    let mut a = base("candidate", Kind::Candidate);
    a.parents = vec!["raw".into()];
    put(tx, &a).unwrap()
}
struct FixtureVerifier(Capability);
impl Verifier for FixtureVerifier {
    fn id(&self) -> &str {
        match self.0 {
            Capability::Structure => "test-json-shape/v1",
            Capability::SourceCoverage => "test-exact-source/v1",
            Capability::ContentReview => "test-reviewed-quote/v1",
            Capability::VisualReview => "test-native-read-receipt/v1",
        }
    }
    fn capability(&self) -> Capability {
        self.0
    }
    fn verify(&self, c: &StoredArtifact, e: &StoredArtifact) -> Result<Verdict> {
        let accepted = match self.0 {
            Capability::Structure => c.artifact.payload["text"].as_str().is_some_and(|s| !s.is_empty()),
            Capability::SourceCoverage => c.artifact.payload["text"] == e.artifact.payload["text"],
            Capability::ContentReview => {
                e.artifact.payload["reviewedQuote"] == c.artifact.payload["text"]
                    && e.artifact.payload["verdict"] == "approved"
            }
            Capability::VisualReview => e.artifact.payload["fixtureNativeReadReceipt"] == "verified-test-only",
        };
        Ok(Verdict { accepted, findings: json!({"syntheticOnly":true,"capability":self.0}) })
    }
}
fn review(tx: &Transaction<'_>, candidate: &StoredArtifact, id: &str) -> StoredArtifact {
    let mut a = base(id, Kind::ReviewRaw);
    a.inputs = candidate.artifact.inputs.clone();
    a.author_attempt = "reviewer-1".into();
    a.parents = vec![candidate.artifact.id.clone()];
    a.payload = json!({"candidateSha256":candidate.payload_sha256,"inputSha256":candidate.input_sha256,"authorAttempt":candidate.artifact.author_attempt,"reviewerAttempt":a.author_attempt,"reviewedQuote":candidate.artifact.payload["text"],"verdict":"approved"});
    put(tx, &a).unwrap()
}
fn adopt_one(
    tx: &Transaction<'_>,
    candidate: &StoredArtifact,
    id: &str,
    previous: u64,
    validation_ids: &[String],
    required: &[Capability],
) -> Result<Adoption> {
    adopt(
        tx,
        Adopt {
            id,
            candidate_id: &candidate.artifact.id,
            identity: &candidate.artifact.identity,
            expected_previous: previous,
            source_sha256: &candidate.artifact.source_sha256,
            contract_version: &candidate.artifact.contract_version,
            current_inputs: &candidate.artifact.inputs,
            required,
            validation_ids,
        },
    )
}
fn consume_one(
    tx: &Transaction<'_>,
    candidate: &StoredArtifact,
    id: &str,
    version: u64,
    inputs: &Inputs,
    required: &[Capability],
) -> Result<Value> {
    consume(
        tx,
        Consume {
            id,
            consumer: "stage-3",
            identity: &candidate.artifact.identity,
            adopted_version: version,
            expected_payload_sha256: &candidate.payload_sha256,
            source_sha256: &candidate.artifact.source_sha256,
            contract_version: &candidate.artifact.contract_version,
            current_inputs: inputs,
            required,
        },
    )
}
fn count(c: &Connection, table: &str) -> i64 {
    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

#[test]
fn additive_install_preserves_legacy_version_and_readonly_old_database_does_no_ddl() {
    let c = database();
    assert_eq!(
        c.query_row("SELECT value FROM metadata WHERE key='schema_version'", [], |r| r.get::<_, String>(0)).unwrap(),
        "2"
    );
    assert_eq!(
        c.query_row("SELECT state FROM jobs WHERE id='old'", [], |r| r.get::<_, String>(0)).unwrap(),
        "completed"
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.sqlite");
    let old = Connection::open(&path).unwrap();
    old.execute_batch(
        "CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT); INSERT INTO metadata VALUES('schema_version','2');",
    )
    .unwrap();
    drop(old);
    let bytes = std::fs::read(&path).unwrap();
    let ro = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(artifact(&ro, "unknown").unwrap().is_none());
    assert!(adoption(&ro, "identity", 1).unwrap().is_none());
    assert_eq!(
        ro.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get::<_, i64>(0)).unwrap(),
        1
    );
    drop(ro);
    assert_eq!(bytes, std::fs::read(path).unwrap());
}

#[test]
fn immutable_payload_provenance_and_idempotency_reject_collisions_without_writes() {
    let mut c = database();
    let tx = c.transaction().unwrap();
    let a = candidate(&tx);
    assert_eq!(put(&tx, &a.artifact).unwrap(), a);
    let before = count(&tx, "workflow_artifacts_v1");
    let mut changed = a.artifact.clone();
    changed.payload["text"] = json!("改写");
    assert!(put(&tx, &changed).is_err());
    assert_eq!(before, count(&tx, "workflow_artifacts_v1"));
    assert!(tx.execute("UPDATE workflow_artifacts_v1 SET source_sha256='tampered' WHERE id='candidate'", []).is_err());
    assert!(tx.execute("DELETE FROM workflow_artifacts_v1 WHERE id='raw'", []).is_err());
    let mut no_origin = base("orphan", Kind::Candidate);
    assert!(put(&tx, &no_origin).is_err());
    no_origin.parents = vec!["missing".into()];
    assert!(put(&tx, &no_origin).is_err());
    assert_eq!(artifact(&tx, "candidate").unwrap().unwrap(), a);
}

#[test]
fn draft_and_pass_text_cannot_claim_adoption_or_independent_review() {
    let mut c = database();
    let tx = c.transaction().unwrap();
    let a = candidate(&tx);
    let draft = put(&tx, &base("draft", Kind::ManualDraft)).unwrap();
    assert!(adopt_one(&tx, &draft, "draft-adoption", 0, &[], &[Capability::Structure]).is_err());
    let mut pass = base("fake-review", Kind::ReviewRaw);
    pass.payload = json!({"status":"PASS"});
    put(&tx, &pass).unwrap();
    assert!(validate(&tx, "fake", "candidate", "fake-review", &FixtureVerifier(Capability::ContentReview)).is_err());
    let mut authored = base("self-review", Kind::ReviewRaw);
    authored.payload = json!({"candidateSha256":a.payload_sha256,"inputSha256":a.input_sha256,"authorAttempt":"author-1","reviewerAttempt":"author-1","reviewedQuote":a.artifact.payload["text"],"verdict":"approved"});
    put(&tx, &authored).unwrap();
    assert!(validate(&tx, "self", "candidate", "self-review", &FixtureVerifier(Capability::ContentReview)).is_err());
    assert_eq!(count(&tx, "workflow_validations_v1"), 0);
}

#[test]
fn capabilities_cas_and_downstream_input_binding_form_real_adoption_chain() {
    let mut c = database();
    let tx = c.transaction().unwrap();
    let a = candidate(&tx);
    let structure = validate(&tx, "structure", "candidate", "raw", &FixtureVerifier(Capability::Structure)).unwrap();
    assert!(structure.accepted);
    assert!(adopt_one(&tx, &a, "early", 0, &["structure".into()], &[Capability::Structure, Capability::ContentReview])
        .is_err());
    assert!(consume_one(&tx, &a, "before", 1, &a.artifact.inputs, &[Capability::Structure]).is_err());
    review(&tx, &a, "review");
    validate(&tx, "content", "candidate", "review", &FixtureVerifier(Capability::ContentReview)).unwrap();
    let ids = vec!["structure".into(), "content".into()];
    let required = [Capability::Structure, Capability::ContentReview];
    let first = adopt_one(&tx, &a, "adopt", 0, &ids, &required).unwrap();
    assert_eq!(first.version, 1);
    assert_eq!(adopt_one(&tx, &a, "adopt", 0, &ids, &required).unwrap(), first);
    assert!(adopt_one(&tx, &a, "racing-adopt", 0, &ids, &required).is_err());
    assert_eq!(count(&tx, "workflow_adoptions_v1"), 1);
    assert_eq!(adoption(&tx, &a.artifact.identity, 1).unwrap(), Some(first));
    let receipt = consume_one(&tx, &a, "consume", 1, &a.artifact.inputs, &required).unwrap();
    assert_eq!(receipt["adoptedVersion"], 1);
    assert_eq!(consume_one(&tx, &a, "consume", 1, &a.artifact.inputs, &required).unwrap(), receipt);
    let mut stale = a.artifact.inputs.clone();
    stale.insert("rules".into(), "c".repeat(64));
    assert!(consume_one(&tx, &a, "stale", 1, &stale, &required).is_err());
    assert!(consume_one(&tx, &a, "missing-version", 2, &a.artifact.inputs, &required).is_err());
    assert_eq!(count(&tx, "workflow_consumptions_v1"), 1);
}

#[test]
fn validation_negative_is_retained_and_never_satisfies_content_or_visual_capability() {
    let mut c = database();
    let tx = c.transaction().unwrap();
    let a = candidate(&tx);
    validate(&tx, "structure", "candidate", "raw", &FixtureVerifier(Capability::Structure)).unwrap();
    let mut evidence = review(&tx, &a, "good-envelope").artifact;
    evidence.id = "negative-review".into();
    evidence.payload["reviewedQuote"] = json!("没有读当前稿");
    put(&tx, &evidence).unwrap();
    let negative =
        validate(&tx, "negative", "candidate", "negative-review", &FixtureVerifier(Capability::ContentReview)).unwrap();
    assert!(!negative.accepted);
    assert!(adopt_one(
        &tx,
        &a,
        "reject",
        0,
        &["structure".into(), "negative".into()],
        &[Capability::Structure, Capability::ContentReview]
    )
    .is_err());
    assert!(validate(&tx, "visual", "candidate", "good-envelope", &FixtureVerifier(Capability::VisualReview)).is_err());
    assert_eq!(count(&tx, "workflow_adoptions_v1"), 0);
}

#[test]
fn usage_revisions_are_independent_of_design_and_cannot_switch_adoption() {
    let mut c = database();
    let tx = c.transaction().unwrap();
    let a = candidate(&tx);
    validate(&tx, "structure", "candidate", "raw", &FixtureVerifier(Capability::Structure)).unwrap();
    let adopted = adopt_one(&tx, &a, "adopt", 0, &["structure".into()], &[Capability::Structure]).unwrap();
    let before = artifact(&tx, "candidate").unwrap();
    let first =
        record_use(&tx, "use-1", &adopted.identity, 1, &adopted.source_sha256, &json!({"sceneIds":["scene-1"]}), None)
            .unwrap();
    let second = record_use(
        &tx,
        "use-2",
        &adopted.identity,
        1,
        &adopted.source_sha256,
        &json!({"sceneIds":["scene-1","scene-2"]}),
        Some("use-1"),
    )
    .unwrap();
    assert_eq!(first["candidateSha256"], second["candidateSha256"]);
    assert_eq!(before, artifact(&tx, "candidate").unwrap());
    assert_eq!(count(&tx, "workflow_adoptions_v1"), 1);
    assert_eq!(count(&tx, "workflow_uses_v1"), 2);
    assert!(record_use(
        &tx,
        "use-2",
        &adopted.identity,
        1,
        &adopted.source_sha256,
        &json!({"sceneIds":["other"]}),
        Some("use-1")
    )
    .is_err());
    assert_eq!(count(&tx, "workflow_uses_v1"), 2);
}

#[test]
fn failed_publication_rolls_back_its_own_rows_without_touching_prior_transaction_work() {
    let mut c = database();
    let tx = c.transaction().unwrap();
    let a = candidate(&tx);
    validate(&tx, "structure", "candidate", "raw", &FixtureVerifier(Capability::Structure)).unwrap();
    tx.execute_batch("CREATE TEMP TRIGGER reject_artifact_head BEFORE INSERT ON workflow_adoption_heads_v1 BEGIN SELECT RAISE(ABORT,'injected CAS storage failure'); END;").unwrap();
    assert!(adopt_one(&tx, &a, "adopt", 0, &["structure".into()], &[Capability::Structure]).is_err());
    assert_eq!(count(&tx, "workflow_adoptions_v1"), 0);
    assert_eq!(count(&tx, "workflow_adoption_heads_v1"), 0);
    assert_eq!(count(&tx, "workflow_artifacts_v1"), 2);
    assert_eq!(count(&tx, "workflow_validations_v1"), 1);
}
