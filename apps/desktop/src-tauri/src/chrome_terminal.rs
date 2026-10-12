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
        &&proof_valid(&result["raw"]["nativeProof"],&job["prepared"],task)
        &&result["raw"]["taskSettlement"]["id"]==task
        &&["failed","cancelled","interrupted","completed_without_artifact"].contains(&s(&result["raw"]["taskSettlement"],"status"))
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
 #[test]fn chrome_terminal_unknown_wrong_request_partial_branches_or_media_always_hold(){
  for (pointer,value) in [("/status",json!("submission_unknown")),("/snapshot/requestId",json!("other")),("/result/taskId",json!("other")),("/result/resultUrls",json!(["image"])),("/result/raw/taskSettlement/status",json!("running")),("/result/raw/taskSettlement/id",json!("other")),("/result/raw/nativeProof/status",json!("matched_incomplete")),("/result/raw/nativeProof/backendImageCount",json!(1)),("/result/raw/nativeProof/bindings",json!([{}])),("/result/raw/nativeProof/promptSha256",json!("a".repeat(64))),("/result/raw/nativeProof/recordRecheckSha256",json!("")),("/result/raw/nativeProof/endTurn",json!(false)),("/result/raw/nativeProof/branchIdentity/0/id",json!("other")),("/result/raw/nativeProof/branchIdentity/1/role",json!("tool")),("/result/raw/nativeProof/conversationId",json!("invalid"))]{
   let mut job=fixture();*job.pointer_mut(pointer).unwrap()=value;assert!(!settled_job(&job),"{pointer}");
  }
 }
}
