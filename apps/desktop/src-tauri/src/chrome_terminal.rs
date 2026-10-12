//! Native final text is a failed image attempt, never successful media or unknown submission.
use serde_json::Value;
use crate::store::s;
fn sha(v:&Value)->bool{v.as_str().is_some_and(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit()))}
pub(crate) fn proof_valid(proof:&Value,prepared:&Value,task:&str)->bool{
    let body=s(&prepared["wire"],"message").split_whitespace().collect::<Vec<_>>().join(" ");
    let hash=crate::pipeline::digest(&body);
    let Some(branch)=proof["branchIdentity"].as_array() else{return false};
    let Some(first)=branch.first() else{return false};
    prepared["provider"]=="chrome"&&prepared["request"]["kind"]=="image"&&prepared["request"]["model"]=="chatgpt-image"
        &&proof["source"]=="conversation-record"&&proof["version"]==1&&proof["proofVersion"]==2&&proof["status"]=="matched_no_image"
        &&proof["requestId"]==task&&prepared["request"]["requestId"]==task&&prepared["wire"]["id"]==task
        &&proof["promptSha256"]==hash&&prepared["wire"]["metadata"]["promptSha256"]==hash
        &&uuid::Uuid::parse_str(s(proof,"conversationId")).is_ok()
        &&proof["backendStatus"]=="finished_successfully"&&proof["endTurn"]==true&&proof["backendImageCount"]==0
        &&proof["bindings"].as_array().is_some_and(Vec::is_empty)&&sha(&proof["recordSha256"])&&sha(&proof["recordRecheckSha256"])&&sha(&proof["assistantTextSha256"])
        &&!s(proof,"userMessageId").is_empty()&&!s(proof,"assistantMessageId").is_empty()
        &&first["nodeKey"]==proof["currentNode"]&&first["id"]==proof["assistantMessageId"]&&first["role"]=="assistant"&&first["endTurn"]==true&&first["status"]=="finished_successfully"
        &&branch.iter().filter(|n|n["role"]=="assistant").count()==1&&branch.iter().filter(|n|n["role"]=="user").count()==1
        &&branch.iter().any(|n|n["role"]=="user"&&n["id"]==proof["userMessageId"])
        &&branch.iter().all(|n|["","user","assistant"].contains(&s(n,"role")))
}
pub(crate) fn settled_job(job:&Value)->bool{
    let task=s(job,"providerTaskId");let result=&job["result"];
    job["provider"]=="chrome"&&job["kind"]=="image"&&job["status"]=="failed"&&!task.is_empty()
        &&job["snapshot"]["requestId"]==task&&result["taskId"]==task&&result["status"]=="failed"&&result["resultUrls"].as_array().is_some_and(Vec::is_empty)
        &&(proof_valid(&result["raw"]["nativeProof"],&job["prepared"],task)||upload_not_submitted_valid(&result["raw"]["uploadNotSubmitted"],&job["prepared"],task))
        &&result["raw"]["taskSettlement"]["id"]==task
        &&["failed","cancelled","interrupted","completed_without_artifact"].contains(&s(&result["raw"]["taskSettlement"],"status"))
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
 &&uuid::Uuid::parse_str(task).is_ok()&&state["requestId"]==task&&state["lifecycle"]=="preparing"&&state["blocker"]=="recovery"
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
 pub(crate) fn upload_fixture()->Value{
  let mut job=fixture();let task=job["providerTaskId"].clone();let prepared=&mut job["prepared"];prepared["wire"]["sourceClientId"]=serde_json::json!("worker");prepared["wire"]["attachments"]=serde_json::json!(["file-parent"]);
  let preconditions=serde_json::json!({"requestId":task,"leaseId":"lease","ownerServerInstanceId":"server","attachments":[{"id":"file-parent"}]});
  let snapshot=serde_json::json!({"requestId":task,"sourceClientId":"worker","phase":"attachments_uploading","active":true,"generating":false,"url":"https://chatgpt.com/","answer":"","artifacts":[],"activeRequest":{"requestId":task,"leaseId":"lease","ownerServerInstanceId":"server","sentAt":0,"responseEpoch":0,"submittedUserTurnKey":"","assistantTurnKey":""}});
  let empty=serde_json::json!({"ok":true,"result":{"sourceClientId":"worker","url":"https://chatgpt.com/","reconciliation":{"source":"browser-empty-preflight","version":1,"proofVersion":2,"status":"matched_empty"}}});
  let proof=serde_json::json!({"version":"chrome-upload-not-submitted/v1","requestId":task,"sourceClientId":"worker","preparedSha256":crate::pipeline::digest(&prepared.to_string()),"diagnosticsAuthoritative":true,
    "canonicalState":{"requestId":task,"lifecycle":"preparing","blocker":"recovery","source":{"clientId":"worker","url":"https://chatgpt.com/","conversationId":"new","leaseId":"lease","ownerServerInstanceId":"server"},"effect":{"browser":{"activeId":null,"activeType":null,"lastResult":{"type":"effect.uncertain","data":{"effectType":"attachments.upload","preconditions":preconditions}}},"coordinator":{"lastResult":{"data":{"result":snapshot}}}}},"nativeEmptyBefore":empty,"nativeEmptyAfter":empty,"originalTaskSettlement":{"id":task,"status":"cancelled"}});
  job["result"]["raw"].as_object_mut().unwrap().remove("nativeProof");job["result"]["raw"]["uploadNotSubmitted"]=proof;job["result"]["raw"]["taskSettlement"]["status"]=serde_json::json!("cancelled");job
 }
 #[test]fn chrome_terminal_only_halted_upload_with_prepost_empty_and_original_cancellation_releases(){
  let job=upload_fixture();assert!(settled_job(&job));let root="/result/raw/uploadNotSubmitted";
  for (part,value) in [("/diagnosticsAuthoritative",serde_json::json!(false)),("/preparedSha256",serde_json::json!("other")),("/canonicalState/requestId",serde_json::json!("other")),("/canonicalState/source/clientId",serde_json::json!("other")),("/canonicalState/effect/browser/activeId",serde_json::json!("running")),("/canonicalState/effect/browser/lastResult/data/effectType",serde_json::json!("prompt.submit")),("/canonicalState/effect/coordinator/lastResult/data/result/activeRequest/sentAt",serde_json::json!(1)),("/canonicalState/effect/coordinator/lastResult/data/result/activeRequest/submittedUserTurnKey",serde_json::json!("user")),("/nativeEmptyBefore/result/reconciliation/status",serde_json::json!("mismatch")),("/nativeEmptyAfter/result/reconciliation/status",serde_json::json!("mismatch")),("/originalTaskSettlement/id",serde_json::json!("other")),("/originalTaskSettlement/status",serde_json::json!("running"))]{let mut bad=job.clone();*bad.pointer_mut(&format!("{root}{part}")).unwrap()=value;assert!(!settled_job(&bad),"{part}");}
 }
 #[test]fn chrome_terminal_unknown_wrong_request_partial_branches_or_media_always_hold(){
  for (pointer,value) in [("/status",json!("submission_unknown")),("/snapshot/requestId",json!("other")),("/result/taskId",json!("other")),("/result/resultUrls",json!(["image"])),("/result/raw/taskSettlement/status",json!("running")),("/result/raw/taskSettlement/id",json!("other")),("/result/raw/nativeProof/status",json!("matched_incomplete")),("/result/raw/nativeProof/backendImageCount",json!(1)),("/result/raw/nativeProof/bindings",json!([{}])),("/result/raw/nativeProof/promptSha256",json!("a".repeat(64))),("/result/raw/nativeProof/recordRecheckSha256",json!("")),("/result/raw/nativeProof/endTurn",json!(false)),("/result/raw/nativeProof/branchIdentity/0/id",json!("other")),("/result/raw/nativeProof/branchIdentity/1/role",json!("tool")),("/result/raw/nativeProof/conversationId",json!("invalid"))]{
   let mut job=fixture();*job.pointer_mut(pointer).unwrap()=value;assert!(!settled_job(&job),"{pointer}");
  }
 }
}
