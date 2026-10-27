use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
#[ignore = "local locked-source JSON supplied explicitly; no model calls"]
async fn private_locked_source_format_acceptance() {
    let path = std::env::var("DIRECTOR_SOURCE_FORMAT_SAMPLE")
        .expect("explicit local sample path required");
    let raw = std::fs::read_to_string(path).unwrap();
    let source: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}')).unwrap();
    let input = json!({"source":source,"sourceVersion":source["version"]});
    assert!(!requires_model(&input).unwrap());
    let e = Executor::new("unknown");
    let output = run(&input, &e, "unused", "medium").await.unwrap();
    assert_eq!(output["status"], "completed");
    assert_eq!(output["source"]["text"], source["text"]);
    assert_eq!(
        output["source"]["boundaryDecisions"],
        source["boundaryDecisions"]
    );
    assert_eq!(e.calls.load(Ordering::SeqCst), 0);
    println!(
        "{}",
        json!({"sourceSha256":source["sha256"],"parserVersion":output["source"]["parserVersion"],"rows":output["source"]["rows"].as_array().unwrap().len(),"episodes":output["source"]["episodes"].as_array().unwrap().len(),"scenes":output["source"]["scenes"].as_array().unwrap().len(),"blockingAfter":0,"sourceTextPreserved":true,"manualDecisionsPreserved":true,"modelCalls":0})
    );
}

struct Executor {
    calls: AtomicUsize,
    accepts: AtomicUsize,
    rejects: AtomicUsize,
    mode: &'static str,
}
impl Executor {
    fn new(mode: &'static str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            accepts: AtomicUsize::new(0),
            rejects: AtomicUsize::new(0),
            mode,
        }
    }
}
#[async_trait]
impl TextExecutor for Executor {
    async fn execute(&self, request: TextRequest) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.stage, "sourceStructure");
        assert_eq!(request.input["sourceStructureProfile"], PROFILE);
        assert_eq!(request.input["sourceVersion"], 1);
        if self.mode == "unknown" {
            return Err("TEXT_SUBMISSION_UNKNOWN: original task preserved".into());
        }
        let ids = request.input["editableRowIds"].clone();
        let first = ids[0].clone();
        let mut output = json!({"groups":[{"rowIds":ids,"kind":"body","episodeNumber":1,"sceneTitle":"工坊","evidenceRowIds":[first],"reason":"本集原文地点字段明确为工坊"}]});
        if self.mode == "unresolved" {
            output["groups"][0]["kind"] = json!("unresolved");
            output["groups"][0]["episodeNumber"] = Value::Null;
            output["groups"][0]["sceneTitle"] = Value::Null;
        }
        if self.mode == "rewrite" {
            output["text"] = json!("改写的全文");
        }
        Ok(output)
    }
    async fn accept(&self, _: &TextRequest, _: &Value) -> Result<(), String> {
        self.accepts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn reject(&self, _: &TextRequest, _: &Value, _: &str) -> Result<(), String> {
        self.rejects.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn input(text: &str) -> Value {
    json!({"source":super::super::prepare_source(text).unwrap(),"sourceVersion":1})
}
const BODY: &str = "第1集\n地点：工坊\n△林川开门。\n林川：我回来了。\n";

#[tokio::test]
async fn local_explicit_formats_need_zero_model_requests() {
    let i=input("主角：林川 人物小传\n【基本资料】\n姓名：林川\n身份：修理员\n第1集\n1-0 工坊 日 内\n林川：我回来了。\n");
    let e = Executor::new("unknown");
    let out = run(&i, &e, "test", "medium").await.unwrap();
    assert_eq!(out["status"], "completed");
    assert_eq!(e.calls.load(Ordering::SeqCst), 0);
    assert_eq!(out["source"]["text"], i["source"]["text"]);
    assert_eq!(out["usedModel"], false);
}

#[tokio::test]
async fn ai_body_mapping_preserves_text_and_accepts_only_validated_response() {
    let i = input(BODY);
    let e = Executor::new("body");
    let out = run(&i, &e, "test", "medium").await.unwrap();
    assert_eq!(out["status"], "completed");
    assert_eq!(out["source"]["text"], BODY);
    assert_eq!(out["source"]["sha256"], i["source"]["sha256"]);
    assert_eq!(e.accepts.load(Ordering::SeqCst), 1);
    assert_eq!(out["source"]["scenes"][0]["title"], "工坊");
}

#[tokio::test]
async fn unknown_propagates_without_retry_or_acceptance() {
    let e = Executor::new("unknown");
    let err = run(&input(BODY), &e, "test", "medium").await.unwrap_err();
    assert!(err.contains("TEXT_SUBMISSION_UNKNOWN"));
    assert_eq!(e.calls.load(Ordering::SeqCst), 1);
    assert_eq!(e.accepts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unresolved_is_a_persistable_partial_result_not_a_fake_success() {
    let i = input(BODY);
    let e = Executor::new("unresolved");
    let out = run(&i, &e, "test", "medium").await.unwrap();
    assert_eq!(out["status"], "partially_blocked");
    assert_eq!(out["decisions"], json!([]));
    assert_eq!(e.accepts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rewriting_is_rejected_without_retry() {
    let e = Executor::new("rewrite");
    assert!(run(&input(BODY), &e, "test", "medium").await.is_err());
    assert_eq!(e.rejects.load(Ordering::SeqCst), 1);
    assert_eq!(e.calls.load(Ordering::SeqCst), 1);
    assert_eq!(e.accepts.load(Ordering::SeqCst), 0);
}

fn fixture() -> (LockedSource, BTreeSet<usize>, BTreeSet<usize>, Value) {
    let source = super::super::prepare_source(BODY).unwrap();
    let editable = editable_indices(&source);
    assert!(!editable.is_empty());
    let visible = (0..source.rows.len()).collect();
    let ids: Vec<_> = editable
        .iter()
        .map(|i| source.rows[*i].id.clone())
        .collect();
    let raw = json!({"groups":[{"rowIds":ids,"kind":"body","episodeNumber":1,"sceneTitle":"工坊","evidenceRowIds":[ids[0]],"reason":"原文地点字段"}]});
    (source, editable, visible, raw)
}

#[test]
fn rejects_outside_duplicate_missing_reordered_and_invented_rows() {
    for mode in 0..5 {
        let (s, e, v, mut raw) = fixture();
        let ids = raw["groups"][0]["rowIds"].as_array_mut().unwrap();
        match mode {
            0 => ids.insert(0, json!(s.rows[0].id)),
            1 => ids.push(ids[0].clone()),
            2 => {
                ids.pop();
            }
            3 => ids.reverse(),
            _ => ids[0] = json!("invented-row"),
        };
        assert!(validate_response(&s, &e, &v, &raw).is_err(), "mode {mode}");
    }
}

#[test]
fn context_cannot_hide_action_and_body_cannot_invent_scene_or_episode() {
    for mode in 0..3 {
        let (s, e, v, mut raw) = fixture();
        match mode {
            0 => {
                raw["groups"][0]["kind"] = json!("context");
                raw["groups"][0]["episodeNumber"] = Value::Null;
                raw["groups"][0]["sceneTitle"] = Value::Null;
            }
            1 => raw["groups"][0]["sceneTitle"] = json!("不存在的月球"),
            _ => raw["groups"][0]["episodeNumber"] = json!(2),
        };
        assert!(validate_response(&s, &e, &v, &raw).is_err());
    }
}

#[test]
fn explicit_unsupported_profile_heading_can_be_classified_but_narrative_mention_cannot() {
    let text = "角色背景\n从小在工坊长大，是机械师。\n第1集\n1-1 工坊 日 内\n林川：回来啦。\n";
    let source = super::super::prepare_source(text).unwrap();
    let editable = editable_indices(&source);
    assert!(!editable.is_empty());
    let visible = (0..source.rows.len()).collect();
    let ids: Vec<_> = editable.iter().map(|i| &source.rows[*i].id).collect();
    let raw = json!({"groups":[{"rowIds":ids,"kind":"context","episodeNumber":null,"sceneTitle":null,"evidenceRowIds":[ids[0]],"reason":"独立角色背景标题下的设定资料"}]});
    let (out, _, _) = validate_response(&source, &editable, &visible, &raw).unwrap();
    assert!(!out.diagnostics.iter().any(|d| d.blocking));
    assert!(!evidence_context_title("她说这是人物小传。"));
    assert!(!evidence_context_title("林川：你的人物小传"));
}

#[tokio::test]
async fn saved_manual_decisions_are_unchanged_by_local_first_analysis() {
    let mut i = input(BODY);
    let (s, e, _, _) = fixture();
    let a = *e.first().unwrap();
    let b = *e.last().unwrap();
    let mapped = apply_boundary_decisions(
        &s,
        &[BoundaryDecision {
            start: s.rows[a].range.start,
            end: s.rows[b].range.end,
            kind: "body".into(),
            episode_number: Some(1),
            scene_title: Some("用户工坊".into()),
            reason: "用户明确决定".into(),
            decision_version: None,
            confirmation: None,
        }],
    )
    .unwrap();
    i["source"] = json!(mapped);
    let e = Executor::new("unknown");
    let out = run(&i, &e, "test", "medium").await.unwrap();
    assert_eq!(
        out["source"]["boundaryDecisions"],
        i["source"]["boundaryDecisions"]
    );
    assert_eq!(e.calls.load(Ordering::SeqCst), 0);
}
