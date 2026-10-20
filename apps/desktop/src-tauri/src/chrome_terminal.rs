//! Native final text is a failed image attempt, never successful media or unknown submission.
use serde_json::Value;
use crate::store::s;
fn sha(v:&Value)->bool{v.as_str().is_some_and(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit()))}
fn presentation_tail_valid(proof:&Value,branch:&[Value])->bool{
    let tail=&proof["presentationTail"];
    let (Some(current),Some(final_node))=(branch.first(),branch.get(1)) else{return false};
    tail.is_object()&&tail["version"]==1&&tail["kind"]=="reasoning_recap"&&tail["contentValueType"]=="string"
        &&tail["contentSha256"].as_str().is_some_and(|value|value.len()==64&&value.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)))
        &&tail["hasImagePointers"]==false&&tail["hideInlineActions"]==true
        &&tail["recipient"]=="all"&&tail.get("channel")==Some(&Value::Null)&&tail["finalRecipient"]=="all"&&tail["finalChannel"]=="final"
        &&!s(tail,"messageId").is_empty()
        &&current["nodeKey"]==proof["currentNode"]&&current["nodeKey"]==tail["nodeKey"]&&current["id"]==tail["messageId"]
        &&current["role"]=="assistant"&&current["status"]=="finished_successfully"&&current["endTurn"]==false
        &&current["parent"]==final_node["nodeKey"]&&tail["parentNodeKey"]==final_node["nodeKey"]
        &&final_node["nodeKey"]==tail["finalNodeKey"]&&final_node["id"]==tail["finalMessageId"]
        &&branch.get(2).is_some_and(|node|node["role"]=="user"&&node["id"]==proof["userMessageId"]&&final_node["parent"]==node["nodeKey"])
        &&branch.iter().filter(|node|node["role"]=="assistant"&&node["endTurn"]==true).count()==1
        &&branch.iter().enumerate().all(|(index,node)|!s(node,"nodeKey").is_empty()
            &&if index+1==branch.len(){node.get("parent")==Some(&Value::Null)}else{node["parent"]==branch[index+1]["nodeKey"]})
        &&branch.iter().map(|node|s(node,"nodeKey")).collect::<std::collections::BTreeSet<_>>().len()==branch.len()
}
pub(crate) fn proof_valid(proof:&Value,prepared:&Value,task:&str)->bool{
    let body=s(&prepared["wire"],"message").split_whitespace().collect::<Vec<_>>().join(" ");
    let hash=crate::pipeline::digest(&body);
    let Some(branch)=proof["branchIdentity"].as_array() else{return false};
    let has_tail=proof.get("presentationTail").is_some();
    let Some(first)=branch.get(if has_tail{1}else{0}) else{return false};
    prepared["provider"]=="chrome"&&prepared["request"]["kind"]=="image"&&prepared["request"]["model"]=="chatgpt-image"
        &&proof["source"]=="conversation-record"&&proof["version"]==1&&proof["proofVersion"]==2&&proof["status"]=="matched_no_image"
        &&proof["requestId"]==task&&prepared["request"]["requestId"]==task&&prepared["wire"]["id"]==task
        &&proof["promptSha256"]==hash&&prepared["wire"]["metadata"]["promptSha256"]==hash
        &&uuid::Uuid::parse_str(s(proof,"conversationId")).is_ok()
        &&proof["backendStatus"]=="finished_successfully"&&proof["endTurn"]==true&&proof["backendImageCount"]==0
        &&proof["bindings"].as_array().is_some_and(Vec::is_empty)&&sha(&proof["recordSha256"])&&sha(&proof["recordRecheckSha256"])&&sha(&proof["assistantTextSha256"])
        &&!s(proof,"userMessageId").is_empty()&&!s(proof,"assistantMessageId").is_empty()
        &&(if has_tail{presentation_tail_valid(proof,branch)}else{first["nodeKey"]==proof["currentNode"]})
        &&first["id"]==proof["assistantMessageId"]&&first["role"]=="assistant"&&first["endTurn"]==true&&first["status"]=="finished_successfully"
        &&branch.iter().filter(|n|n["role"]=="assistant").count()==if has_tail{2}else{1}
        &&branch.iter().filter(|n|n["role"]=="user").count()==1
        &&branch.iter().any(|n|n["role"]=="user"&&n["id"]==proof["userMessageId"])
        &&branch.iter().all(|n|["","user","assistant"].contains(&s(n,"role")))
}
pub(crate) fn settled_job(job:&Value)->bool{
    let task=s(job,"providerTaskId");let result=&job["result"];
    if !(job["provider"]=="chrome"&&job["kind"]=="image"&&job["status"]=="failed"&&!task.is_empty()
        &&job["snapshot"]["requestId"]==task&&result["taskId"]==task&&result["status"]=="failed"&&result["resultUrls"].as_array().is_some_and(Vec::is_empty)) {
        return false;
    }
    // OpenCLI has its own frozen wire and journal contract. A malformed new
    // proof must never obtain settlement from an unrelated legacy proof.
    if job["prepared"]["wire"]["transport"]=="opencli/v1"
        || job["snapshot"]["providerConfig"]["browserTransport"]["transport"]=="opencli/v1" {
        return opencli_not_submitted_valid(job,task);
    }
    (proof_valid(&result["raw"]["nativeProof"],&job["prepared"],task)||upload_not_submitted_valid(&result["raw"]["uploadNotSubmitted"],&job["prepared"],task))
        &&result["raw"]["taskSettlement"]["id"]==task
        &&["failed","cancelled","interrupted","completed_without_artifact"].contains(&s(&result["raw"]["taskSettlement"],"status"))
}

fn opencli_sha(value:&Value)->bool {
    value.as_str().is_some_and(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)))
}

fn no_opencli_send_markers(value:&Value,depth:usize)->bool {
    if depth>32 {return false;}
    match value {
        Value::Object(fields)=>fields.iter().all(|(key,value)|
            !["sentAt","sendConfirmed","conversationUrl","sendIntent"].contains(&key.as_str())
                &&no_opencli_send_markers(value,depth+1)),
        Value::Array(values)=>values.iter().all(|value|no_opencli_send_markers(value,depth+1)),
        _=>true,
    }
}

fn opencli_reader_bound(job:&Value,proof:&Value)->bool {
    let raw=&job["result"]["raw"];
    let config=&job["snapshot"]["providerConfig"];
    let frozen=&config["browserTransport"]["runtimeSha256"];
    if !opencli_sha(frozen)||!opencli_sha(&proof["runtimeSha256"]) {return false;}
    let Some(audit)=raw.get("readerMigration") else{return proof["runtimeSha256"]==*frozen;};
    // This is the existing query-only provider audit, not a new execution
    // identity. Its destination must be an exact reviewed reader generation.
    audit["schemaVersion"]=="opencli-readonly-reader-migration/v1"
        &&["query","reconcile"].contains(&s(audit,"operation"))&&audit["bridgeOperation"]=="query"
        &&audit["readOnly"]==true&&audit["resubmitted"]==false&&audit["frozenConfigUnchanged"]==true
        &&audit["fromRuntimeSha256"]==*frozen&&audit["toRuntimeSha256"]==proof["runtimeSha256"]
        &&audit["fromRuntimeSha256"]!=audit["toRuntimeSha256"]
        &&crate::providers::opencli::read_only_reader_audit_allowed(s(audit,"fromRuntimeSha256"),s(audit,"toRuntimeSha256"))
        &&["fromRunnerSha256","toRunnerSha256","fromManifestSha256","toManifestSha256"].iter().all(|key|opencli_sha(&audit[key]))
        &&audit["frozenConfigSha256"]==crate::pipeline::digest(&config.to_string())
        &&audit["generationRequestSha256"]==crate::pipeline::digest(&job["prepared"]["request"].to_string())
}

fn opencli_owned_adapter_bound(proof:&Value)->bool {
    let pin=s(proof,"runtimeSha256");let adapter=s(proof,"composerAdapterSha256");
    (pin==include_str!("../../../../web-image-bridge/runtime.sha256").trim()
        &&adapter==crate::pipeline::digest(include_str!("../../../../web-image-bridge/composer-transport.mjs")))
        // Preserve only this exact prior owned no-click proof. Legacy unknowns
        // have neither this protocol nor a valid absent-click proof.
        ||(pin=="f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70"
            &&adapter=="0183767038c20ddebf218ddd297b2cf166aceee4d99bb7c03b7f73d7d3654e65")
        ||(pin=="54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0"
            &&adapter=="5c850d49dba9164a61b499565be9eb21833da724aa6aad6f91a67ddbe86b717b")
        ||(pin=="afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6"
            &&adapter=="5c850d49dba9164a61b499565be9eb21833da724aa6aad6f91a67ddbe86b717b")
}

fn opencli_not_submitted_valid(job:&Value,task:&str)->bool {
    let prepared=&job["prepared"];let wire=&prepared["wire"];let result=&job["result"];let raw=&result["raw"];
    let proof=&raw["opencliNotSubmitted"];let config=&job["snapshot"]["providerConfig"];let frozen=&config["browserTransport"];
    // Compare the typed original request, not metadata added to the snapshot.
    // In particular, do not recompute Node's wire hash with Rust's key order.
    let Ok(original)=serde_json::from_value::<crate::providers::GenerationRequest>(job["snapshot"].clone()) else{return false;};
    let Ok(original)=serde_json::to_value(original) else{return false;};
    let Ok(request)=serde_json::from_value::<crate::providers::GenerationRequest>(prepared["request"].clone()) else{return false;};
    let Ok(request)=serde_json::to_value(request) else{return false;};
    let Some(checks)=proof["checks"].as_array().filter(|checks|checks.len()==2) else{return false;};
    let page=s(proof,"page");let account=s(wire,"accountKey");let profile=s(wire,"profile");
    let error=s(result,"error");
    let owned=proof["version"]=="opencli-owned-pre-send-not-submitted/v2";
    let proof_kind=proof["version"]=="opencli-pre-send-not-submitted/v1" || (owned
        &&wire["sendProtocol"]=="owned-composer-send/v2"&&proof["sendProtocol"]==wire["sendProtocol"]
        &&proof["composerAdapterSha256"]==wire["composerAdapterSha256"]
        &&opencli_owned_adapter_bound(proof)
        &&proof["clickIntentAbsent"]==true&&proof["clickCalls"].as_u64()==Some(0));
    prepared["provider"]=="chrome"&&request==prepared["request"]&&original==request
        &&request["requestId"]==task&&request["kind"]=="image"&&request["model"]=="chatgpt-image"
        &&uuid::Uuid::parse_str(task).is_ok()
        &&wire["transport"]=="opencli/v1"&&config["kind"]=="chrome"&&frozen["transport"]=="opencli/v1"
        &&!account.trim().is_empty()&&frozen["accountKey"]==account&&!profile.trim().is_empty()&&frozen["profile"]==profile
        &&raw["transport"]=="opencli/v1"&&opencli_sha(&wire["requestHash"])&&raw["requestHash"]==wire["requestHash"]
        &&proof_kind&&proof["requestId"]==task&&proof["requestHash"]==wire["requestHash"]
        &&opencli_reader_bound(job,proof)
        &&opencli_sha(&proof["journalBeforeSha256"])&&opencli_sha(&proof["journalAfterSha256"])
        &&proof["journalBeforeSha256"]!=proof["journalAfterSha256"]
        &&proof["sendMarkerAbsent"]==true&&proof["sendConfirmedAbsent"]==true&&proof["conversationUrlAbsent"]==true
        &&!page.trim().is_empty()&&page.len()<=256&&no_opencli_send_markers(raw,0)
        &&checks.iter().all(|check|check["page"]==page&&check["accountKey"]==account
            &&["https://chatgpt.com/","https://chatgpt.com/new"].contains(&s(check,"url"))
            &&check["userCount"].as_u64()==Some(0)&&check["assistantCount"].as_u64()==Some(0)
            &&(check["composerEmpty"]==true||(owned&&check["composerEmpty"].is_boolean()))&&check["generating"]==false)
        &&raw["taskSettlement"]["id"]==task&&raw["taskSettlement"]["status"]=="failed"&&raw["taskSettlement"]["reason"]=="not_submitted"
        &&(error.contains("未提交")||error.contains("未发送")||error.contains("没有提交")
            ||error.to_ascii_lowercase().contains("not_submitted")||error.to_ascii_lowercase().contains("not submitted"))
}
/// A halted upload effect plus fresh native empty-page proof does not grant a
/// retry until the original bridge task has been cancelled and rechecked empty.
pub(crate) fn upload_not_submitted_candidate(p:&Value,prepared:&Value,task:&str)->bool{
 let state=&p["canonicalState"];let source=&state["source"];let effect=&state["effect"]["browser"]["lastResult"];
 let snapshot=&state["effect"]["coordinator"]["lastResult"]["data"]["result"];let active=&snapshot["activeRequest"];
 let wire=&prepared["wire"];let upload_ids:Vec<_>=effect["data"]["preconditions"]["attachments"].as_array().into_iter().flatten().map(|a|a["id"].clone()).collect();
 p["version"]=="chrome-upload-not-submitted/v1"&&p["requestId"]==task&&p["sourceClientId"]==wire["sourceClientId"]
 &&p["preparedSha256"]==crate::pipeline::digest(&prepared.to_string())&&p["diagnosticsAuthoritative"]==true
 &&prepared["provider"]=="chrome"&&prepared["request"]["kind"]=="image"&&prepared["request"]["model"]=="chatgpt-image"&&prepared["request"]["requestId"]==task&&wire["id"]==task
 &&uuid::Uuid::parse_str(task).is_ok()&&state["requestId"]==task&&["preparing","failed"].contains(&s(state,"lifecycle"))&&state["submission"]=="accepted"&&state["blocker"]=="recovery"
 &&source["clientId"]==wire["sourceClientId"]&&source["url"]=="https://chatgpt.com/"&&source["conversationId"]=="new"
 &&state["effect"]["browser"].get("activeId")==Some(&Value::Null)&&state["effect"]["browser"].get("activeType")==Some(&Value::Null)
 &&effect["type"]=="effect.uncertain"&&effect["data"]["effectType"]=="attachments.upload"&&effect["data"]["preconditions"]["requestId"]==task
 &&effect["data"]["preconditions"]["leaseId"]==source["leaseId"]&&!s(source,"leaseId").is_empty()
 &&effect["data"]["preconditions"]["ownerServerInstanceId"]==source["ownerServerInstanceId"]&&!s(source,"ownerServerInstanceId").is_empty()
 &&!upload_ids.is_empty()&&wire["attachments"]==Value::Array(upload_ids)
 &&snapshot["requestId"]==task&&snapshot["sourceClientId"]==wire["sourceClientId"]&&snapshot["phase"]=="attachments_uploading"&&snapshot["active"]==true&&snapshot["generating"]==false
 &&snapshot["url"]=="https://chatgpt.com/"&&snapshot["answer"]==""&&snapshot["artifacts"].as_array().is_some_and(Vec::is_empty)
 &&active["requestId"]==task&&active["leaseId"]==source["leaseId"]&&active["ownerServerInstanceId"]==source["ownerServerInstanceId"]
 &&active["sentAt"]==0&&active["responseEpoch"]==0&&active["submittedUserTurnKey"]==""&&active["assistantTurnKey"]==""
}
fn empty_receipt(v:&Value,source:&Value)->bool{
 v["ok"]==true&&v["result"]["sourceClientId"]==*source&&v["result"]["url"]=="https://chatgpt.com/"
 &&v["result"]["reconciliation"]["source"]=="browser-empty-preflight"&&v["result"]["reconciliation"]["version"]==1
 &&v["result"]["reconciliation"]["proofVersion"]==2&&v["result"]["reconciliation"]["status"]=="matched_empty"
}
pub(crate) fn upload_not_submitted_valid(p:&Value,prepared:&Value,task:&str)->bool{
 upload_not_submitted_candidate(p,prepared,task)&&empty_receipt(&p["nativeEmptyBefore"],&p["sourceClientId"])
 &&empty_receipt(&p["nativeEmptyAfter"],&p["sourceClientId"])&&p["originalTaskSettlement"]["id"]==task
 &&["failed","cancelled","interrupted"].contains(&s(&p["originalTaskSettlement"],"status"))
}

#[cfg(test)]
#[path = "chrome_terminal_opencli_not_submitted_tests.rs"]
mod opencli_not_submitted_tests;

#[cfg(test)]pub(crate) mod tests{
 use super::*;use serde_json::json;
 pub(crate) fn fixture()->Value{
  let task="12345678-1234-4234-8234-123456789abc";let hash=crate::pipeline::digest("synthetic image");
  json!({"provider":"chrome","kind":"image","status":"failed","providerTaskId":task,"snapshot":{"requestId":task},
   "prepared":{"provider":"chrome","request":{"requestId":task,"kind":"image","model":"chatgpt-image"},"wire":{"id":task,"message":"synthetic image","metadata":{"promptSha256":hash}}},
   "result":{"taskId":task,"status":"failed","resultUrls":[],"raw":{"taskSettlement":{"id":task,"status":"failed"},"nativeProof":{
    "source":"conversation-record","version":1,"proofVersion":2,"status":"matched_no_image","requestId":task,"promptSha256":hash,"conversationId":"11111111-1111-4111-8111-111111111111",
    "backendStatus":"finished_successfully","endTurn":true,"backendImageCount":0,"bindings":[],"recordSha256":"a".repeat(64),"recordRecheckSha256":"b".repeat(64),"assistantTextSha256":"c".repeat(64),
    "userMessageId":"user","assistantMessageId":"assistant","currentNode":"final","branchIdentity":[{"nodeKey":"final","id":"assistant","role":"assistant","endTurn":true,"status":"finished_successfully"},{"nodeKey":"user-node","id":"user","role":"user"},{"nodeKey":"root","id":"","role":""}]}}}})
 }
 #[test]fn chrome_terminal_only_proved_no_image_and_settled_original_stops_blocking(){assert!(settled_job(&fixture()));}
 fn presentation_fixture()->Value{
  let mut job=fixture();let proof=&mut job["result"]["raw"]["nativeProof"];
  let branch=proof["branchIdentity"].as_array_mut().unwrap();
  for index in 0..branch.len(){branch[index]["parent"]=if index+1==branch.len(){Value::Null}else{branch[index+1]["nodeKey"].clone()};}
  branch.insert(0,json!({"nodeKey":"recap","parent":"final","id":"recap-message","role":"assistant","status":"finished_successfully","endTurn":false}));
  proof["currentNode"]=json!("recap");
  proof["presentationTail"]=json!({"version":1,"kind":"reasoning_recap","nodeKey":"recap","messageId":"recap-message","parentNodeKey":"final",
    "finalNodeKey":"final","finalMessageId":"assistant","contentSha256":"d".repeat(64),"contentValueType":"string","hasImagePointers":false,
    "recipient":"all","channel":null,"hideInlineActions":true,"finalRecipient":"all","finalChannel":"final"});job
 }
 #[test]fn chrome_terminal_precise_presentation_tail_keeps_current_and_only_settles_no_image(){
  let job=presentation_fixture();assert!(settled_job(&job));
  assert_eq!(job["result"]["raw"]["nativeProof"]["currentNode"],"recap");
  assert_eq!(job["result"]["raw"]["nativeProof"]["branchIdentity"][0]["endTurn"],false);
  let mut image=job.clone();image["result"]["raw"]["nativeProof"]["backendImageCount"]=json!(1);assert!(!settled_job(&image));
 }
 #[test]fn chrome_terminal_presentation_tail_rejects_missing_unknown_changed_branch_and_incomplete_evidence(){
  for (part,value) in [("/presentationTail",Value::Null),("/presentationTail/version",json!(2)),("/presentationTail/kind",json!("unknown")),
    ("/presentationTail/contentValueType",json!("object")),("/presentationTail/contentSha256",json!("")),("/presentationTail/hasImagePointers",json!(true)),
    ("/presentationTail/hideInlineActions",json!(false)),("/presentationTail/recipient",json!("tool")),("/presentationTail/channel",json!("analysis")),
    ("/presentationTail/finalRecipient",json!("tool")),("/presentationTail/finalChannel",json!("analysis")),("/presentationTail/nodeKey",json!("other")),
    ("/presentationTail/messageId",json!("other")),("/presentationTail/parentNodeKey",json!("other")),("/presentationTail/finalNodeKey",json!("other")),
    ("/presentationTail/finalMessageId",json!("other")),("/branchIdentity/0/endTurn",json!(true)),("/branchIdentity/0/status",json!("in_progress")),
    ("/branchIdentity/0/role",json!("tool")),("/branchIdentity/0/parent",json!("user-node")),("/branchIdentity/1/id",json!("other")),
    ("/branchIdentity/1/parent",json!("missing")),("/branchIdentity/3/parent",json!("cycle")),("/currentNode",json!("final"))]{
   let mut job=presentation_fixture();*job.pointer_mut(&format!("/result/raw/nativeProof{part}")).unwrap()=value;assert!(!settled_job(&job),"{part}");
  }
  let mut absent=presentation_fixture();absent["result"]["raw"]["nativeProof"].as_object_mut().unwrap().remove("presentationTail");assert!(!settled_job(&absent));
  let mut empty_id=presentation_fixture();empty_id["result"]["raw"]["nativeProof"]["presentationTail"]["messageId"]=json!("");empty_id["result"]["raw"]["nativeProof"]["branchIdentity"][0]["id"]=json!("");assert!(!settled_job(&empty_id));
  let mut extra=presentation_fixture();let proof=&mut extra["result"]["raw"]["nativeProof"];proof["branchIdentity"][1]["parent"]=json!("extra");
  proof["branchIdentity"].as_array_mut().unwrap().insert(2,json!({"nodeKey":"extra","parent":"user-node","id":"","role":""}));assert!(!settled_job(&extra));
  for role in ["assistant","tool","user"]{
   let mut job=presentation_fixture();let proof=&mut job["result"]["raw"]["nativeProof"];
   proof["branchIdentity"].as_array_mut().unwrap().insert(0,json!({"nodeKey":"later","parent":"recap","id":"later-message","role":role,"status":"finished_successfully","endTurn":role=="assistant"}));
   proof["currentNode"]=json!("later");assert!(!settled_job(&job),"later {role}");
  }
 }
 pub(crate) fn upload_fixture()->Value{
  let mut job=fixture();let task=job["providerTaskId"].clone();let prepared=&mut job["prepared"];prepared["wire"]["sourceClientId"]=serde_json::json!("worker");prepared["wire"]["attachments"]=serde_json::json!(["file-parent"]);
  let preconditions=serde_json::json!({"requestId":task,"leaseId":"lease","ownerServerInstanceId":"server","attachments":[{"id":"file-parent"}]});
  let snapshot=serde_json::json!({"requestId":task,"sourceClientId":"worker","phase":"attachments_uploading","active":true,"generating":false,"url":"https://chatgpt.com/","answer":"","artifacts":[],"activeRequest":{"requestId":task,"leaseId":"lease","ownerServerInstanceId":"server","sentAt":0,"responseEpoch":0,"submittedUserTurnKey":"","assistantTurnKey":""}});
  let empty=serde_json::json!({"ok":true,"result":{"sourceClientId":"worker","url":"https://chatgpt.com/","reconciliation":{"source":"browser-empty-preflight","version":1,"proofVersion":2,"status":"matched_empty"}}});
  let proof=serde_json::json!({"version":"chrome-upload-not-submitted/v1","requestId":task,"sourceClientId":"worker","preparedSha256":crate::pipeline::digest(&prepared.to_string()),"diagnosticsAuthoritative":true,
    "canonicalState":{"requestId":task,"lifecycle":"preparing","submission":"accepted","blocker":"recovery","source":{"clientId":"worker","url":"https://chatgpt.com/","conversationId":"new","leaseId":"lease","ownerServerInstanceId":"server"},"effect":{"browser":{"activeId":null,"activeType":null,"lastResult":{"type":"effect.uncertain","data":{"effectType":"attachments.upload","preconditions":preconditions}}},"coordinator":{"lastResult":{"data":{"result":snapshot}}}}},"nativeEmptyBefore":empty,"nativeEmptyAfter":empty,"originalTaskSettlement":{"id":task,"status":"cancelled"}});
  job["result"]["raw"].as_object_mut().unwrap().remove("nativeProof");job["result"]["raw"]["uploadNotSubmitted"]=proof;job["result"]["raw"]["taskSettlement"]["status"]=serde_json::json!("cancelled");job
 }
 #[test]fn chrome_terminal_only_halted_upload_with_prepost_empty_and_original_cancellation_releases(){
  let job=upload_fixture();assert!(settled_job(&job));let root="/result/raw/uploadNotSubmitted";
  for (part,value) in [("/diagnosticsAuthoritative",serde_json::json!(false)),("/preparedSha256",serde_json::json!("other")),("/canonicalState/requestId",serde_json::json!("other")),("/canonicalState/source/clientId",serde_json::json!("other")),("/canonicalState/effect/browser/activeId",serde_json::json!("running")),("/canonicalState/effect/browser/lastResult/data/effectType",serde_json::json!("prompt.submit")),("/canonicalState/effect/coordinator/lastResult/data/result/activeRequest/sentAt",serde_json::json!(1)),("/canonicalState/effect/coordinator/lastResult/data/result/activeRequest/submittedUserTurnKey",serde_json::json!("user")),("/nativeEmptyBefore/result/reconciliation/status",serde_json::json!("mismatch")),("/nativeEmptyAfter/result/reconciliation/status",serde_json::json!("mismatch")),("/originalTaskSettlement/id",serde_json::json!("other")),("/originalTaskSettlement/status",serde_json::json!("running"))]{let mut bad=job.clone();*bad.pointer_mut(&format!("{root}{part}")).unwrap()=value;assert!(!settled_job(&bad),"{part}");}
 }
 #[test]fn chrome_terminal_deadline_failed_upload_still_requires_accepted_not_submitted_and_native_double_empty(){
  let mut job=upload_fixture();job["result"]["raw"]["uploadNotSubmitted"]["canonicalState"]["lifecycle"]=serde_json::json!("failed");assert!(settled_job(&job));
  job["result"]["raw"]["uploadNotSubmitted"]["canonicalState"]["submission"]=serde_json::json!("submitted");assert!(!settled_job(&job));
 }
 #[test]fn chrome_terminal_unknown_wrong_request_partial_branches_or_media_always_hold(){
  for (pointer,value) in [("/status",json!("submission_unknown")),("/snapshot/requestId",json!("other")),("/result/taskId",json!("other")),("/result/resultUrls",json!(["image"])),("/result/raw/taskSettlement/status",json!("running")),("/result/raw/taskSettlement/id",json!("other")),("/result/raw/nativeProof/status",json!("matched_incomplete")),("/result/raw/nativeProof/backendImageCount",json!(1)),("/result/raw/nativeProof/bindings",json!([{}])),("/result/raw/nativeProof/promptSha256",json!("a".repeat(64))),("/result/raw/nativeProof/recordRecheckSha256",json!("")),("/result/raw/nativeProof/endTurn",json!(false)),("/result/raw/nativeProof/branchIdentity/0/id",json!("other")),("/result/raw/nativeProof/branchIdentity/1/role",json!("tool")),("/result/raw/nativeProof/conversationId",json!("invalid"))]{
   let mut job=fixture();*job.pointer_mut(pointer).unwrap()=value;assert!(!settled_job(&job),"{pointer}");
  }
 }
}
