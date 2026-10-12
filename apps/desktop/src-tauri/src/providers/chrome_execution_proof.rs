//! A connection hello is historical. Only this exact native read can prove idle.
use super::*;
pub(super) fn owned_root(source:&Value,binding:&crate::chrome_setup::BrowserBinding)->bool {
    let launch=s(source,"launchToken");
    source["url"]=="https://chatgpt.com/" && (launch==control_launch(binding)
        || launch.strip_prefix("bridge-director-desktop-").is_some_and(|id|uuid::Uuid::parse_str(id).is_ok_and(|u|u.get_version_num()==4&&u.to_string()==id)))
}
// Bootstrap uses the ordinary page solely as transport for the extension's
// durable fixed controller reservation. It never reads its body or changes it.
pub(super) fn bootstrap_control(config:&ProviderConfig,client:&Client,source:&Value,binding:&crate::chrome_setup::BrowserBinding)->ProviderResult<Vec<Value>> {
    let held=||ProviderError::new("BRIDGE_BUSY","专属控制页创建尚未证实，保留原预约与用户页面，未准备或提交图片。");
    static BOOTSTRAP:std::sync::OnceLock<std::sync::Mutex<()>>=std::sync::OnceLock::new();
    let _guard=BOOTSTRAP.get_or_init(Default::default).lock().map_err(|_|held())?;
    let launch=control_launch(binding);let before=clients(config,client)?;
    if !compatible_profile_clients(&before,source,binding) || before.iter().filter(|c|c["browserProfileId"]==binding.profile_id).any(|c|!explicit_idle(c)){return Err(held());}
    let controls:Vec<_>=before.iter().filter(|c|c["launchToken"]==launch).collect();
    if !controls.is_empty(){
        if controls.len()!=1 || controls[0]["focused"]!=false || controls[0]["visibilityState"]!="hidden" || !owned_root(controls[0],binding){return Err(held());}
        return Ok(before);
    }
    let originals:Vec<_>=before.iter().filter(|c|c["id"]==source["id"]).collect();
    if originals.len()!=1 || !same_browser_runtime(originals[0],source,binding) || originals[0]["url"]!=source["url"] || originals[0]["contentEpoch"]!=source["contentEpoch"] {return Err(held());}
    let opened=json_request(config,client,"/browser/tabs/open",Some(&json!({"url":"https://chatgpt.com/","sourceClientId":source["id"],"active":false,"select":false,"launchToken":launch,"timeoutMs":15000,"allowSystemFallback":false})),false).map_err(|_|held())?;
    let tab=&opened["client"];
    if opened["ok"]!=true || tab["id"]==source["id"] || s(tab,"id").is_empty() || tab["launchToken"]!=launch || !same_browser_runtime(tab,source,binding) || !explicit_idle(tab)
        || tab["focused"]!=false || tab["visibilityState"]!="hidden" || !owned_root(tab,binding)
        || opened["tabState"]["active"]!=false || opened["tabState"]["autoDiscardable"]!=false || opened["tabState"]["discarded"]!=false {return Err(held());}
    let after=clients(config,client)?;let controls:Vec<_>=after.iter().filter(|c|c["launchToken"]==launch).collect();let originals:Vec<_>=after.iter().filter(|c|c["id"]==source["id"]).collect();
    if controls.len()!=1 || controls[0]["id"]!=tab["id"] || !same_idle_owned_source(controls[0],tab,binding) || controls[0]["contentEpoch"]!=tab["contentEpoch"]
        || originals.len()!=1 || !same_browser_runtime(originals[0],source,binding) || originals[0]["url"]!=source["url"] || originals[0]["contentEpoch"]!=source["contentEpoch"]
        || !compatible_profile_clients(&after,source,binding) || after.iter().filter(|c|c["browserProfileId"]==binding.profile_id).any(|c|!explicit_idle(c)){return Err(held());}
    Ok(after)
}
fn execution_valid(native:&Value,source:&Value)->bool {
    let p=&native["inventory"]["sourceExecutionProof"];let own=&p["ownCommand"];let q=&p["queue"];
    let age=p["observedAt"].as_i64().map(|at|chrono::Utc::now().timestamp_millis()-at);
    let cmd=s(native,"commandId");let rpc=s(native,"rpcRequestId");
    let acks=p["ownAcceptedMessageIds"].as_array();
    !cmd.is_empty() && native["inventoryCommandId"]==cmd && p["inventoryCommandId"]==cmd
        && rpc.starts_with("ext-") && (12..=132).contains(&rpc.len()) && rpc.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-') && p["rpcRequestId"]==rpc
        && p["version"]=="director-source-execution-idle/v1" && p["sourceTabId"]==source["browserTabId"] && p["nativeSamples"]==2
        && age.is_some_and(|v|(0..=5000).contains(&v)) && p.get("lease")==Some(&Value::Null)
        && p["otherUnresolvedCommands"].as_array().is_some_and(Vec::is_empty) && p["otherPendingOutbox"].as_array().is_some_and(Vec::is_empty)
        && own["commandId"]==cmd && own["commandType"]=="browser.tab.identify" && own["scope"]=="standalone" && own["mode"]=="result"
        && own["operation"]=="control" && own["status"]=="dispatched" && own["requestId"]==""
        && acks.is_some_and(|a|a.iter().all(|id|id.as_str().is_some_and(|id|!id.is_empty())))
        && q["version"]=="tab-operation-snapshot/v1" && q["tabId"]==source["browserTabId"] && q["running"]["sequence"].as_u64().is_some_and(|n|n>0)
        && q["running"]["label"]=="content:bridge.tab.open" && q["running"]["meta"]["type"]=="bridge.tab.open" && q["running"]["meta"]["requestId"]==rpc
        && q["queued"].as_array().is_some_and(|items|items.iter().all(|item|s(item,"label").starts_with("server:transport.ack:")
            && item["meta"]["ackMessageId"].as_str().is_some_and(|id|!id.is_empty()) && acks.is_some_and(|a|a.contains(&item["meta"]["ackMessageId"]))))
}
pub(super) fn fresh_idle(config:&ProviderConfig,client:&Client,source:&Value,binding:&crate::chrome_setup::BrowserBinding,request:&GenerationRequest)->ProviderResult<()> {
    prove_idle(config,client,source,binding,request).map(|_|())
}
pub(super) fn prove_idle(config:&ProviderConfig,client:&Client,source:&Value,binding:&crate::chrome_setup::BrowserBinding,request:&GenerationRequest)->ProviderResult<Value> {
    let held=||ProviderError::new("BRIDGE_BUSY","浏览器实时执行队列尚未证实空闲，原任务保持等待；未开页或发送。");
    let mut url=reqwest::Url::parse(&format!("http://localhost/browser/owned-launch/{}",request.request_id)).map_err(|_|held())?;
    url.query_pairs_mut().append_pair("sourceClientId",s(source,"id")).append_pair("profileId",&binding.profile_id).append_pair("bundleId",&binding.bundle_id)
        .append_pair("backgroundEpoch",s(source,"backgroundEpoch")).append_pair("serverInstanceId",s(source,"serverInstanceId"));
    let native=json_request(config,client,&format!("{}?{}",url.path(),url.query().unwrap_or("")),None,false)?;
    let raw=s(&native,"inventoryCanonicalJson");let decoded:Value=serde_json::from_str(raw).map_err(|_|held())?;
    let runtime=&native["runtime"];let inv=&native["inventory"];
    let receipt_age=native["observedAt"].as_i64().map(|at|chrono::Utc::now().timestamp_millis()-at);
    let generation=s(&native,"inventoryGeneration");
    let tabs:Vec<_>=inv["tabs"].as_array().ok_or_else(held)?.iter().filter(|t|t["id"]==source["browserTabId"]).collect();
    if native["ok"]!=true || native["version"]!="director-owned-launch-inventory/v1" || !["matched","confirmed_absent"].contains(&s(&native,"status")) || native["complete"]!=true
        || native["requestId"]!=request.request_id || native["launchToken"]!=format!("bridge-director-desktop-{}",request.request_id)
        || native["componentVersion"]!="6.4.0" || runtime["extensionVersion"]!="2.4.5" || !receipt_age.is_some_and(|age|(0..=30_000).contains(&age))
        || !uuid::Uuid::parse_str(generation).is_ok_and(|id|id.get_version_num()==4 && id.to_string()==generation)
        || native["sourceClientId"]!=source["id"] || runtime["sourceClientId"]!=source["id"] || runtime["sourceTabId"]!=source["browserTabId"]
        || !source["browserTabId"].as_u64().is_some_and(|id|id>0) || runtime["profileId"]!=binding.profile_id || runtime["bundleId"]!=binding.bundle_id
        || runtime["backgroundEpoch"]!=source["backgroundEpoch"] || runtime["serverInstanceId"]!=source["serverInstanceId"] || runtime["contentEpoch"]!=source["contentEpoch"] || s(runtime,"contentEpoch").is_empty()
        || decoded!=*inv || native["inventorySha256"]!=hex_hash(raw.as_bytes()) || inv["sourceTabId"]!=source["browserTabId"]
        || tabs.len()!=1 || tabs[0]["incognito"]!=false || tabs[0]["discarded"]!=false || !inventory_source_background_proven(inv,runtime,source,tabs[0]) || !execution_valid(&native,source) {return Err(held());}
    let after=clients(config,client)?;let matches:Vec<_>=after.iter().filter(|c|c["id"]==source["id"]).collect();
    if matches.len()!=1 || after.iter().filter(|c|c["launchToken"]==source["launchToken"]).count()!=1
        || !same_idle_owned_source(matches[0],source,binding) || matches[0]["browserTabId"]!=source["browserTabId"] || matches[0]["contentEpoch"]!=source["contentEpoch"] || !compatible_profile_clients(&after,source,binding)
        || after.iter().filter(|c|c["browserProfileId"]==binding.profile_id).any(|c|!explicit_idle(c)) {return Err(held());}
    Ok(native)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(Value,Value){
        let source=json!({"browserTabId":10});let proof=json!({"version":"director-source-execution-idle/v1","sourceTabId":10,"inventoryCommandId":"inventory-cmd","rpcRequestId":"ext-native-123456","nativeSamples":2,"observedAt":chrono::Utc::now().timestamp_millis(),"lease":null,
            "ownCommand":{"commandId":"inventory-cmd","commandType":"browser.tab.identify","scope":"standalone","mode":"result","operation":"control","status":"dispatched","requestId":""},"otherUnresolvedCommands":[],"otherPendingOutbox":[],"ownAcceptedMessageIds":["own-ack"],
            "queue":{"version":"tab-operation-snapshot/v1","tabId":10,"running":{"sequence":1,"label":"content:bridge.tab.open","meta":{"type":"bridge.tab.open","requestId":"ext-native-123456"}},"queued":[{"label":"server:transport.ack:own-ack","meta":{"ackMessageId":"own-ack"}}]}});
        (json!({"commandId":"inventory-cmd","inventoryCommandId":"inventory-cmd","rpcRequestId":"ext-native-123456","inventory":{"sourceExecutionProof":proof}}),source)
    }
    #[test] fn chrome_exact_current_inventory_excludes_only_its_own_command_and_ack(){let (v,s)=fixture();assert!(execution_valid(&v,&s));let mut v=v;v["inventory"]["sourceExecutionProof"]["queue"]["queued"]=json!([]);assert!(execution_valid(&v,&s));}
    #[test] fn chrome_stale_foreign_or_missing_execution_evidence_never_proves_idle(){
        for path in ["stale","future","outer_command","outer_rpc","source","samples","lease","command","outbox","own_status","own_request","queue_request","queue_label","foreign_ack","empty_ack","missing_queue","missing_lease"] {
            let (mut v,s)=fixture();let p=&mut v["inventory"]["sourceExecutionProof"];
            match path {"stale"=>p["observedAt"]=json!(chrono::Utc::now().timestamp_millis()-5001),"future"=>p["observedAt"]=json!(chrono::Utc::now().timestamp_millis()+1000),"source"=>p["sourceTabId"]=json!(11),"samples"=>p["nativeSamples"]=json!(1),"lease"=>p["lease"]=json!({"requestId":"other"}),"command"=>p["otherUnresolvedCommands"]=json!([{}]),"outbox"=>p["otherPendingOutbox"]=json!([{}]),"own_status"=>p["ownCommand"]["status"]=json!("completed"),"own_request"=>p["ownCommand"]["requestId"]=json!("other"),"queue_request"=>p["queue"]["running"]["meta"]["requestId"]=json!("ext-other-123456"),"queue_label"=>p["queue"]["running"]["label"]=json!("generation"),"foreign_ack"=>p["queue"]["queued"][0]["meta"]["ackMessageId"]=json!("foreign"),"empty_ack"=>p["ownAcceptedMessageIds"]=json!([""]),"missing_queue"=>{p.as_object_mut().unwrap().remove("queue");},"missing_lease"=>{p.as_object_mut().unwrap().remove("lease");},"outer_command"=>v["commandId"]=json!("different"),_=>v["rpcRequestId"]=json!("ext-other-123456")};
            assert!(!execution_valid(&v,&s),"{path}");
        }
    }
}
