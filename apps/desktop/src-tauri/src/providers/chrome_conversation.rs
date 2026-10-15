//! Source-bound conversation locator and one durable read-only reopen; never submits.
use super::*;
pub(crate) fn locator_valid(config:&ProviderConfig,p:&PreparedRequest,l:&Value)->bool{
    let Some(binding)=config.chrome_binding.as_ref() else{return false};
    let conversation=s(l,"conversationId");
    config.kind=="chrome"&&p.provider=="chrome"&&p.request.kind=="image"&&l["version"]=="chrome-conversation-locator/v1"
        &&l["requestId"]==p.request.request_id&&l["sourceClientId"]==p.wire["sourceClientId"]
        &&l["browserBinding"]==json!(binding)&&p.wire["browserBinding"]==json!(binding)
        &&l["preparedSha256"]==hex_hash(json!(p).to_string().as_bytes())
        &&l["promptSha256"]==p.wire["metadata"]["promptSha256"]
        &&l["promptSha256"]==hex_hash(s(&p.wire,"message").split_whitespace().collect::<Vec<_>>().join(" ").as_bytes())
        &&uuid::Uuid::parse_str(conversation).is_ok()&&l["url"]==format!("https://chatgpt.com/c/{conversation}")
        &&( !s(l,"userMessageId").is_empty()&&s(l,"userMessageId").len()<=128
            || l["userMessageId"].is_null()&&p.wire["correlationVersion"]!="conversation/v1"&&s(&p.wire,"message").contains(&format!("任务关联码：{}。",p.request.request_id)))
}
pub(super) fn legacy_seed(config:&ProviderConfig,p:&PreparedRequest,source:&Value,observe:&PrepareObserver<'_>)->ProviderResult<Value>{
    let conversation=owned_conversation(source)?;
    let l=json!({"version":"chrome-conversation-locator/v1","requestId":p.request.request_id,"sourceClientId":p.wire["sourceClientId"],"browserBinding":config.chrome_binding,
        "preparedSha256":hex_hash(json!(p).to_string().as_bytes()),"promptSha256":p.wire["metadata"]["promptSha256"],"conversationId":conversation,"url":source["url"],"userMessageId":null});
    if source["id"]==p.wire["sourceClientId"]&&locator_valid(config,p,&l){observe(&json!({"type":"locator","locator":l}))?;Ok(l)}else{Ok(Value::Null)}
}
pub(super) fn pin(config:&ProviderConfig,p:&PreparedRequest,source:&Value,proof:&Value,saved:&Value,observe:&PrepareObserver<'_>)->ProviderResult<Value>{
    if locator_valid(config,p,&saved["locator"]){
        if saved["locator"]["userMessageId"].is_null()&&proof["source"]=="conversation-record"&&proof["proofVersion"]==2&&proof["requestId"]==p.request.request_id
            &&proof["conversationId"]==saved["locator"]["conversationId"]
            &&proof["promptSha256"]==p.wire["metadata"]["promptSha256"]&&proof["userMessageId"].is_string()
            &&(["matched_complete","matched_no_image"].contains(&s(proof,"status"))||proof["status"]=="matched_incomplete"&&proof["backendPromptVerified"]==true){
            let mut l=saved["locator"].clone();l["userMessageId"]=proof["userMessageId"].clone();observe(&json!({"type":"locator","locator":l}))?;return Ok(l);
        }
        if !saved["locator"]["userMessageId"].is_null()&&proof["userMessageId"].is_string()&&proof["userMessageId"]!=saved["locator"]["userMessageId"]{return Err(ProviderError::new("IMAGE_PROVENANCE","原会话的原始消息已变化"));}
        return Ok(saved["locator"].clone());
    }
    if proof["source"]=="conversation-record"&&proof["proofVersion"]==2&&proof["requestId"]==p.request.request_id
        &&proof["promptSha256"]==p.wire["metadata"]["promptSha256"]
        &&(["matched_complete","matched_no_image"].contains(&s(proof,"status"))||proof["status"]=="matched_incomplete"&&proof["backendPromptVerified"]==true)
        &&proof["userMessageId"].is_string()&&source["id"]==p.wire["sourceClientId"] {
        let l=json!({"version":"chrome-conversation-locator/v1","requestId":p.request.request_id,"sourceClientId":p.wire["sourceClientId"],"browserBinding":config.chrome_binding,
            "preparedSha256":hex_hash(json!(p).to_string().as_bytes()),"promptSha256":p.wire["metadata"]["promptSha256"],"conversationId":proof["conversationId"],"url":source["url"],"userMessageId":proof["userMessageId"]});
        if locator_valid(config,p,&l){observe(&json!({"type":"locator","locator":l}))?;return Ok(l);}
    }
    Ok(Value::Null)
}
// An open response is a connection snapshot, not proof that navigation and its
// own tab command have settled. Only the same newly reserved hidden tab may be
// observed again; this helper cannot open, navigate, or submit anything.
const RECOVERY_SETTLE_READS:usize=6;
fn recovery_bound(value:&Value,anchor:&Value,root:&Value,binding:&crate::chrome_setup::BrowserBinding,alias:&str)->bool{
    value["id"]==anchor["id"]&&!s(anchor,"id").is_empty()
        &&anchor["browserTabId"].as_u64().is_some_and(|id|id>0)&&value["browserTabId"]==anchor["browserTabId"]
        &&value["launchToken"]==alias&&same_browser_runtime(value,root,binding)
        &&value["focused"]==false&&value["visibilityState"]=="hidden"
        &&value.get("activeRequest")==Some(&Value::Null)&&value["quarantined"]!=true&&value["compatible"]!=false
        &&value["releasePending"]!=true&&value["releaseState"]!="pending"&&!s(value,"contentEpoch").is_empty()
}
fn recovery_transition(value:&Value,target:&Value)->bool{
    value["url"]=="https://chatgpt.com/"&&value["requestedUrl"]==*target
        ||value["url"]==*target&&crate::chrome_setup::client_command_busy(value)
}
fn settle_new_recovery(config:&ProviderConfig,client:&Client,source:&Value,root:&Value,binding:&crate::chrome_setup::BrowserBinding,alias:&str,target:&Value)->ProviderResult<Value>{
    let held=|reads|{
        let mut error=ProviderError::new("CHROME_LINK_RECOVERY_HELD","原会话恢复页尚未稳定，保留同一预约等待查询；未重新提交");
        error.context=Some(json!({"phase":"recovery_page_settle","launchToken":alias,"sourceClientId":source["id"],"browserTabId":source["browserTabId"],"reads":reads,"submitCalled":false,"additionalOpenCalled":false}));error
    };
    if !recovery_bound(source,source,root,binding,alias)
        ||!(source["url"]==*target&&explicit_idle(source)||recovery_transition(source,target)){return Err(held(0));}
    let mut target_epoch=if source["url"]==*target{Some(source["contentEpoch"].clone())}else{None};
    for read in 1..=RECOVERY_SETTLE_READS{
        // The first read replaces the existing final verification, so a stable
        // open still costs the same five transport calls as before.
        let after=clients(config,client)?;
        let owners:Vec<_>=after.iter().filter(|c|c["launchToken"]==alias).collect();
        let controls:Vec<_>=after.iter().filter(|c|c["launchToken"]==root["launchToken"]).collect();
        if owners.len()!=1||controls.len()!=1||!compatible_profile_clients(&after,root,binding)
            ||!recovery_bound(owners[0],source,root,binding,alias)
            ||!recovery_bound(controls[0],root,root,binding,s(root,"launchToken"))
            ||controls[0]["url"]!=root["url"]||controls[0]["contentEpoch"]!=root["contentEpoch"]{return Err(held(read));}
        let latest=owners[0];let control=controls[0];
        if latest["url"]==*target{
            if target_epoch.as_ref().is_some_and(|epoch|latest["contentEpoch"]!=*epoch){return Err(held(read));}
            if target_epoch.is_none(){target_epoch=Some(latest["contentEpoch"].clone());}
            if explicit_idle(latest)&&explicit_idle(control){return Ok(latest.clone());}
        }else if target_epoch.is_some(){return Err(held(read));}
        let source_wait=recovery_transition(latest,target);
        let control_wait=control["url"]==root["url"]&&crate::chrome_setup::client_command_busy(control);
        if !(source_wait||latest["url"]==*target&&explicit_idle(latest)&&control_wait)
            ||!(explicit_idle(control)||control_wait){return Err(held(read));}
        if read<RECOVERY_SETTLE_READS{std::thread::sleep(std::time::Duration::from_millis(250));}
    }
    Err(held(RECOVERY_SETTLE_READS))
}
pub(super) fn restore_source(config:&ProviderConfig,client:&Client,p:&PreparedRequest,saved:&Value,observe:&PrepareObserver<'_>)->ProviderResult<Value>{
    let held=||ProviderError::new("CHROME_LINK_RECOVERY_HELD","原工作页丢失，保存的会话与后台恢复预约尚未完整核对；未重新提交");
    let loc=&saved["locator"];if !locator_valid(config,p,loc){return Err(held());}
    let binding=config.chrome_binding.as_ref().ok_or_else(held)?;
    let available=clients(config,client)?;let launch=format!("bridge-director-desktop-{}",p.request.request_id);
    // A still-existing original page changed by the user is not an absent page.
    if available.iter().any(|c|c["id"]==p.wire["sourceClientId"]&&c["url"]!=loc["url"]){return Err(held());}
    let roots:Vec<_>=available.iter().filter(|c|c["launchToken"]==control_launch(binding)).collect();
    if roots.len()!=1||!explicit_idle(roots[0])||roots[0]["focused"]!=false||roots[0]["visibilityState"]!="hidden"||!compatible_profile_clients(&available,roots[0],binding){return Err(held());}
    let root=roots[0];let native=execution_proof::prove_idle(config,client,root,binding,&p.request)?;
    let originals:Vec<_>=available.iter().filter(|c|c["launchToken"]==launch).collect();
    let original_proven= originals.is_empty()&&native["status"]=="confirmed_absent"&&native["match"].is_null()
        || originals.len()==1&&native["status"]=="matched"&&native["match"]["tabId"]==originals[0]["browserTabId"]&&native["match"]["url"]==loc["url"]&&native["match"]["connected"]==true;
    if !original_proven{return Err(held());}
    let alias=if saved["recovery"].is_null(){format!("bridge-director-desktop-{}",uuid::Uuid::new_v4())}else{s(&saved["recovery"],"launchToken").to_owned()};
    if !alias.strip_prefix("bridge-director-desktop-").is_some_and(|id|uuid::Uuid::parse_str(id).is_ok())||alias==launch{return Err(held());}
    let owners:Vec<_>=available.iter().filter(|c|c["launchToken"]==alias).collect();
    let mut opened_now=false;
    let mut source=if owners.len()==1&&!saved["recovery"].is_null() {owners[0].clone()}
    else if owners.is_empty()&&saved["recovery"].is_null(){
        observe(&json!({"type":"recovery_intent","requestId":p.request.request_id,"launchToken":alias,"url":loc["url"],"nativeOriginalProof":native,"at":now(),"submitCalled":false}))?;
        let opened=json_request(config,client,"/browser/tabs/open",Some(&json!({"url":loc["url"],"sourceClientId":root["id"],"active":false,"select":false,"launchToken":alias,"timeoutMs":30000,"allowSystemFallback":false})),false)?;
        if opened["ok"]!=true||opened["tabState"]["active"]!=false||opened["tabState"]["discarded"]!=false{return Err(held());}
        opened_now=true;
        opened["client"].clone()
    }else{return Err(held());};
    // A failed website navigation may leave the one reserved recovery page on
    // the homepage. Revisit its saved URL once; never allocate another alias.
    if source["url"]!=loc["url"]&&!saved["recovery"].is_null(){
        if source["url"]!="https://chatgpt.com/"||source["requestedUrl"]!=loc["url"]||!saved["recoveryNavigation"].is_null()
            ||source["launchToken"]!=alias||source["focused"]!=false||source["visibilityState"]!="hidden"
            ||!explicit_idle(&source)||!same_browser_runtime(&source,root,binding){return Err(held());}
        observe(&json!({"type":"recovery_navigation_intent","requestId":p.request.request_id,"launchToken":alias,
            "sourceClientId":source["id"],"browserTabId":source["browserTabId"],"expectedCurrentUrl":source["url"],"url":loc["url"],"at":now(),"submitCalled":false}))?;
        let navigated=json_request(config,client,"/browser/tabs/recover-owned-conversation",Some(&json!({
            "sourceClientId":source["id"],"originalRequestId":p.request.request_id,"expectedLaunchToken":alias,
            "expectedCurrentUrl":source["url"],"targetUrl":loc["url"],"timeoutMs":30000})),false).map_err(|e|{
                let mut error=held();error.context=Some(json!({"phase":"recovery_navigation","cause":e,"submitCalled":false}));error
            })?;
        if navigated["ok"]!=true||navigated["version"]!="director-owned-conversation-navigation/v1"||navigated["status"]!="restored"
            ||navigated["openedNewTab"]!=false||navigated["submitCalled"]!=false||navigated["client"]["id"]!=source["id"]
            ||navigated["client"]["browserTabId"]!=source["browserTabId"]{return Err(held());}
        source=navigated["client"].clone();
    }
    if opened_now{source=settle_new_recovery(config,client,&source,root,binding,&alias,&loc["url"])?;}
    else{
        if source["url"]!=loc["url"]||source["launchToken"]!=alias||source["focused"]!=false||source["visibilityState"]!="hidden"||!explicit_idle(&source)||!same_browser_runtime(&source,root,binding){return Err(held());}
        let after=clients(config,client)?;let verified:Vec<_>=after.iter().filter(|c|c["launchToken"]==alias).collect();
        if verified.len()!=1||!same_idle_owned_source(verified[0],&source,binding)||verified[0]["contentEpoch"]!=source["contentEpoch"]{return Err(held());}
    }
    observe(&json!({"type":"recovery_opened","requestId":p.request.request_id,"launchToken":alias,"client":source,"url":loc["url"],"at":now(),"submitCalled":false}))?;
    Ok(source)
}
