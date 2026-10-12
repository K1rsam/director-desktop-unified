//! End only a proved halted pre-submit upload; never send or retry a prompt.
use super::*;
pub(super) fn settle(config:&ProviderConfig,client:&Client,turn:&Value,source:&Value,p:&PreparedRequest)->ProviderResult<Option<QueryResult>>{
 if p.provider!="chrome"||p.request.kind!="image"||uuid::Uuid::parse_str(&p.request.request_id).is_err()||turn["id"]!=p.request.request_id||p.wire["id"]!=p.request.request_id
  ||source["url"]!="https://chatgpt.com/"||p.wire["sourceClientId"]!=source["id"]||p.wire["attachments"].as_array().is_none_or(Vec::is_empty){return Ok(None);}
 let id=&p.request.request_id;
 let diagnostic=json_request(config,client,&format!("/diagnostics/request-state?requestId={id}"),None,false)?;
 let mut proof=json!({"version":"chrome-upload-not-submitted/v1","requestId":id,"sourceClientId":source["id"],"preparedSha256":hex_hash(json!(p).to_string().as_bytes()),"diagnosticsAuthoritative":diagnostic["ok"]==true&&diagnostic["authoritative"]==true,"canonicalState":diagnostic["requests"]["state"]});
 if !crate::chrome_terminal::upload_not_submitted_candidate(&proof,&json!(p),id){return Ok(None);}
 let preflight=||json_request(config,client,"/browser/recover-latest",Some(&json!({"sourceClientId":source["id"],"index":1,"timeoutMs":12000,"reconcileConversation":{"mode":"image_preflight"}})),false);
 proof["nativeEmptyBefore"]=preflight()?;
 let empty=&proof["nativeEmptyBefore"];
 if empty["ok"]!=true||empty["result"]["sourceClientId"]!=source["id"]||empty["result"]["url"]!="https://chatgpt.com/"||empty["result"]["reconciliation"]["status"]!="matched_empty"||empty["result"]["reconciliation"]["proofVersion"]!=2{return Ok(None);}
 let settled=if ["failed","cancelled","interrupted"].contains(&s(turn,"status")){turn.clone()}else{
  json_request(config,client,&format!("/turns/{id}/interrupt"),Some(&json!({"reason":"原生空页与原请求执行记录证明参考图上传停住且提示词未发送，结束原等待并保留上传与请求"})),false)?["turn"].clone()
 };
 proof["originalTaskSettlement"]=json!({"id":settled["id"],"status":settled["status"]});proof["nativeEmptyAfter"]=preflight()?;
 if !crate::chrome_terminal::upload_not_submitted_valid(&proof,&json!(p),id){return Err(ProviderError::new("IMAGE_UPLOAD_SETTLEMENT_PENDING","上传停住的原任务尚未完整结算，保留记录，不重新发送"));}
 Ok(Some(QueryResult{task_id:id.clone(),status:"failed".into(),result_urls:vec![],error:Some("参考图上传校验未完成，本次提示词没有发送；原上传及回执已保存，可重新制作。".into()),actual_cost:None,
 raw:json!({"turn":turn,"uploadNotSubmitted":proof,"taskSettlement":{"id":id,"status":settled["status"]},"progress":{"phase":"upload_failed_not_submitted","message":"参考图已上传但校验停住；本次没有发送生成。","checkedAt":now()}})}))
}
