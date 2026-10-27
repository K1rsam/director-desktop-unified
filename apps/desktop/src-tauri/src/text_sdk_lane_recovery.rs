//! The pinned first three-lane supervisor rejected its directory before reading
//! stdin or spawning Codex. EOF/empty journals alone never authorize recovery.
use super::*;
use crate::text_transport::AttemptState;
pub(crate) const LANE_FAILED_SUPERVISOR_SHA:&str="80349b58f08959b2461c3e24e51caa64869913f3c73b5428c9328e9a4fb96db7";
pub(crate) const LANE_FAILED_BRIDGE_SHA:&str="acf04b60785355135627fe791ba35e771c8ad59937dc9df0cfe365190c43e7f2";
pub(crate) const LANE_RECOVERY_VERSION:&str="sdk-lane-preparation-recovery/v1";
pub(crate) const LANE_RECOVERY_ROOT:&str="native-runtime-lane-recovery";
const HELD:&str="SDK_LANE_PREPARATION_RECOVERY_HELD: 三路原任务零派发证据或一次性恢复链不完整；保留 UNKNOWN";
fn hash(v:&Value)->String{digest(&v.to_string())}
fn absent(p:&Path)->bool{matches!(std::fs::symlink_metadata(p),Err(e) if e.kind()==std::io::ErrorKind::NotFound)}
fn bytes(p:&Path)->Result<Vec<u8>>{let m=std::fs::symlink_metadata(p).map_err(|_|HELD)?;if !m.is_file()||m.is_symlink()||m.len()>4_194_304{return Err(HELD.into());}std::fs::read(p).map_err(|_|HELD.into())}
fn read(p:&Path)->Result<Value>{serde_json::from_slice(&bytes(p)?).map_err(|_|HELD.into())}
fn directory(p:&Path,names:&[&str])->Result<()>{let m=std::fs::symlink_metadata(p).map_err(|_|HELD)?;if !m.is_dir()||m.is_symlink(){return Err(HELD.into());}let mut actual=std::fs::read_dir(p).map_err(|_|HELD)?.map(|r|r.map(|e|e.file_name().to_string_lossy().into_owned()).map_err(|_|HELD.to_string())).collect::<Result<Vec<_>>>()?;actual.sort();let mut expected:Vec<_>=names.iter().map(|n|n.to_string()).collect();expected.sort();if actual!=expected{return Err(HELD.into());}Ok(())}
pub(crate) fn lane_runtime_inventory(root:&Path)->Result<Value>{
    directory(root,&["runtime-plan.json","tmp","work"])?;directory(&root.join("work"),&[".director-text-root"])?;
    let tmp=root.join("tmp");let entries=std::fs::read_dir(&tmp).map_err(|_|HELD)?.collect::<std::result::Result<Vec<_>,_>>().map_err(|_|HELD)?;
    if entries.len()!=1{return Err(HELD.into());}let name=entries[0].file_name().to_string_lossy().into_owned();
    if !name.starts_with("codex-output-schema-")||name.len()>80{return Err(HELD.into());}directory(&tmp,&[&name])?;directory(&tmp.join(&name),&["schema.json"])?;
    let mut out=vec![];for name in ["runtime-plan.json".to_string(),format!("tmp/{name}/schema.json"),"work/.director-text-root".into()]{let body=bytes(&root.join(&name))?;if name=="work/.director-text-root"&&body!=b"director SDK text execution\n"{return Err(HELD.into());}out.push(json!([name,body.len(),digest(std::str::from_utf8(&body).map_err(|_|HELD)?)]));}Ok(json!(out))
}
fn lock_stopped(worker:&Path)->Result<()> {
    if !absent(&worker.join("worker.lock.recovery")){return Err(HELD.into());}let file=worker.join("worker.lock");if absent(&file){return Ok(());}
    let original=bytes(&file)?;let value:Value=serde_json::from_slice(&original).map_err(|_|HELD)?;let pid=value["pid"].as_u64().filter(|n|*n>0&&*n<=u32::MAX as u64).ok_or(HELD)? as u32;
    if value["instanceId"].as_str().is_none_or(|s|s.is_empty()){return Err(HELD.into());}
    #[cfg(windows)]
    let stopped=unsafe {use std::ffi::c_void;#[link(name="kernel32")]unsafe extern "system"{fn OpenProcess(access:u32,inherit:i32,pid:u32)->*mut c_void;fn WaitForSingleObject(h:*mut c_void,ms:u32)->u32;fn CloseHandle(h:*mut c_void)->i32;fn GetLastError()->u32;}let h=OpenProcess(0x00100000,0,pid);if h.is_null(){GetLastError()==87}else{let done=WaitForSingleObject(h,0)==0;CloseHandle(h);done}};
    #[cfg(not(windows))]let stopped={let _=pid;false};
    if !stopped||bytes(&file)?!=original{return Err(HELD.into());}Ok(())
}
pub(crate) fn lane_recovery_identity_bound(old:&Value,new:&Value)->bool{
    old["supervisorSha256"]==LANE_FAILED_SUPERVISOR_SHA&&old["bridgeSha256"]==LANE_FAILED_BRIDGE_SHA
        && new["supervisorSha256"]!=old["supervisorSha256"]&&supervisor_handoff_identity_bound(old,new)
}
fn original_record_bound(dir:&Path,original:&Value)->Result<()> {
    if original.as_object().is_none_or(|m|m.len()!=19)||original["state"]!="dispatching"||!original["error"].is_null()||original["lastSequence"]!=0
        ||["threadId","turnId","rawResult","rawResultSha256","usage"].iter().any(|k|!original[k].is_null())||original["journalSha256"]!=digest(""){return Err(HELD.into());}
    let attempt:FrozenAttempt=serde_json::from_value(original["attempt"].clone()).map_err(|_|HELD)?;attempt.validate_identity()?;
    if attempt.runtime_identity["supervisorSha256"]!=LANE_FAILED_SUPERVISOR_SHA||attempt.runtime_identity["bridgeSha256"]!=LANE_FAILED_BRIDGE_SHA{return Err(HELD.into());}
    let marker=&attempt.request.input["foundationParallelTrial"];
    if marker["workflowId"]!=attempt.workflow_id||marker["version"]!="foundation-three-lane-trial/v1"{return Err(HELD.into());}
    let worker=dir.parent().ok_or(HELD)?;let project=worker.parent().and_then(Path::parent).and_then(Path::parent).ok_or(HELD)?;
    if std::fs::canonicalize(worker).map_err(|_|HELD)?!=std::fs::canonicalize(worker_root(project,&attempt.request)?).map_err(|_|HELD)?||dir.file_name().and_then(|n|n.to_str())!=Some(attempt.attempt_id.as_str()){return Err(HELD.into());}
    let runtime=dir.join("native-runtime");let list=lane_runtime_inventory(&runtime)?;let plan=read(&runtime.join("runtime-plan.json"))?;
    if plan.as_object().is_none_or(|m|m.len()!=12)||plan["version"]!="sdk-native-runtime/v1"||plan["identity"]!=attempt.runtime_identity||plan["attemptId"]!=attempt.attempt_id
        ||plan["model"]!=attempt.request.model||plan["reasoning"]!=attempt.request.reasoning||plan["semanticRequestHash"]!=attempt.semantic_request_hash||plan["executionFingerprint"]!=attempt.execution_fingerprint
        ||["compiledPromptSha256","compiledSchemaSha256"].iter().any(|k|plan[k]!=original[k])||list[1][2]!=plan["runtimeSchemaSha256"]
        ||std::fs::canonicalize(Path::new(s(&plan,"workingDirectory"))).map_err(|_|HELD)?!=std::fs::canonicalize(runtime.join("work")).map_err(|_|HELD)?
        ||std::fs::canonicalize(Path::new(s(&plan,"tempDirectory"))).map_err(|_|HELD)?!=std::fs::canonicalize(runtime.join("tmp")).map_err(|_|HELD)?{return Err(HELD.into());}
    let compiled=crate::text_executor::compile_sdk_attempt_input(&attempt)?;
    if original["compiledPrompt"]!=compiled.prompt||original["compiledSchema"]!=compiled.schema||original["compiledPromptSha256"]!=digest(&compiled.prompt)||original["compiledSchemaSha256"]!=digest(&compiled.schema){return Err(HELD.into());}
    Ok(())
}
fn events_bound(record:&Value,events:&Value)->bool{
    let Some(rows)=events.as_array()else{return false;};let mut tail=vec![];let mut last=0;
    for row in rows{let Some(id)=row["id"].as_i64().filter(|n|*n>last)else{return false;};last=id;
        match s(row,"type"){"state"=>(),"sdk_supervisor_handoff_frozen" if tail.is_empty()&&row["data"]["modelCalls"]==0=>(),"sdk_pre_dispatch_queued" if tail.is_empty()&&row["data"]["modelCalls"]==0=>(),"sdk_dispatch_receipt"|"sdk_transport_unknown"|"sdk_process_tree"|"sdk_stderr"=>tail.push(row),_=>return false}}
    if tail.len()!=4||tail.iter().zip(["sdk_dispatch_receipt","sdk_transport_unknown","sdk_process_tree","sdk_stderr"]).any(|(r,t)|r["type"]!=t){return false;}
    let(a,u,t,e)=(&tail[0]["data"],&tail[1]["data"],&tail[2]["data"],&tail[3]["data"]);
    a["ok"]==true&&a["protocolVersion"]==PROTOCOL&&a["attemptId"]==record["attempt"]["attemptId"]&&a["result"]["state"]=="dispatching"&&a["result"]["lastSequence"]==0
        &&["semanticRequestHash","executionFingerprint","attemptEnvelopeHash"].iter().all(|k|a["result"][k]==record["attempt"][k])
        &&["compiledPromptSha256","compiledSchemaSha256"].iter().all(|k|a["result"][k]==record[k])
        &&u["attemptId"]==a["attemptId"]&&u["lastSdkSequence"]==0&&u["error"]=="SDK_STDERR_CLOSED"
        &&t["processTreeStopProven"]==true&&t["activeProcesses"]==0&&t["totalProcesses"]==3&&e["truncated"]==false&&e["bytes"]==e["retainedBytes"]&&e["bytes"].as_u64().is_some_and(|n|n>0&&n<16384)
}
pub(crate) fn lane_preparation_evidence(root:&Path,attempt:&FrozenAttempt,events:&Value)->Result<Value>{
    let historical=historical_receipt(root,attempt)?;
    if attempt.state!=AttemptState::SubmissionUnknown||attempt.last_sequence>1||attempt.thread_id.is_some()||attempt.turn_id.is_some()||attempt.raw_result.is_some(){return Err(HELD.into());}
    let worker=worker_root(root,&attempt.request)?;lock_stopped(&worker)?;let dir=worker.join(&attempt.attempt_id);directory(&dir,&["receipt.json","native-runtime"])?;
    let original=bytes(&dir.join("receipt.json"))?;let record:Value=serde_json::from_slice(&original).map_err(|_|HELD)?;original_record_bound(&dir,&record)?;
    if !events_bound(&record,events){return Err(HELD.into());}
    // The installed current file is rotated to previous on update. Either exact
    // binary proves the fixed pre-spawn predicate; never accept an arbitrary EOF.
    let exe=Path::new(s(&attempt.runtime_identity,"supervisorExecutable"));let previous=exe.with_file_name("director-desktop-next-previous.exe");
    let pinned=[exe,previous.as_path()].iter().any(|p|std::fs::symlink_metadata(p).is_ok_and(|m|m.is_file()&&!m.is_symlink())&&crate::store::hash_file(p).is_ok_and(|h|h.0==LANE_FAILED_SUPERVISOR_SHA));
    if !pinned{return Err(HELD.into());}
    let list=lane_runtime_inventory(&dir.join("native-runtime"))?;if bytes(&dir.join("receipt.json"))?!=original{return Err(HELD.into());}lock_stopped(&worker)?;
    Ok(json!({"version":"sdk-lane-preparation-evidence/v1","attemptId":attempt.attempt_id,"attemptEnvelopeHash":attempt.attempt_envelope_hash,
        "originalReceiptSha256":crate::store::hash_file(&dir.join("receipt.json"))?.0,"originalReceiptSeal":historical["receiptEnvelopeSha256"],
        "runtimeInventory":list,"runtimeInventorySha256":hash(&list),"events":events,"eventsSha256":hash(events),"failedSupervisorSha256":LANE_FAILED_SUPERVISOR_SHA,
        "sdkImplementationSha256":SDK_IMPLEMENTATION_SHA,"sdkEvents":0,"processTreeStopProven":true,"beforeOfficialRuntimeSpawn":true}))
}
pub(crate) fn lane_recovery_record_bound(dir:&Path,record:&Value,effective:&Value)->Result<bool>{
    let Some(recovery)=record.get("lanePreparationRecovery")else{return Ok(false);};let proof=&recovery["proof"];
    if recovery.as_object().is_none_or(|m|m.len()!=2)||recovery["effectiveIdentity"]!=*effective||!lane_recovery_identity_bound(&record["attempt"]["runtimeIdentity"],effective)
        ||["preDispatchRecovery","wrapperPreparationRepair","schemaPreparationRecovery"].iter().any(|k|record.get(k).is_some())
        ||proof["version"]!=LANE_RECOVERY_VERSION||proof["controllerSha256"]!=lane_controller_sha256()||proof["attemptId"]!=record["attempt"]["attemptId"]||proof["attemptEnvelopeHash"]!=record["attempt"]["attemptEnvelopeHash"]
        ||proof["effectiveIdentitySha256"]!=hash(effective)||proof["sdkEvents"]!=0||proof["processTreeStopProven"]!=true||proof["beforeOfficialRuntimeSpawn"]!=true{return Ok(false);}
    let original_bytes=bytes(&dir.join("lane-preparation-original.json"))?;let original:Value=serde_json::from_slice(&original_bytes).map_err(|_|HELD)?;original_record_bound(dir,&original)?;
    let evidence=read(&dir.join("lane-preparation-evidence.json"))?;
    if read(&dir.join("lane-preparation-recovery.json"))?!=*proof||proof["evidenceSha256"]!=hash(&evidence)||proof["originalReceiptSha256"]!=digest(std::str::from_utf8(&original_bytes).map_err(|_|HELD)?)
        ||proof["originalReceiptSeal"]!=original["receiptEnvelopeSha256"]||proof["runtimeInventorySha256"]!=hash(&lane_runtime_inventory(&dir.join("native-runtime"))?)||evidence["eventsSha256"]!=hash(&evidence["events"])||!events_bound(&original,&evidence["events"]){return Ok(false);}
    Ok(["protocolVersion","attempt","frozenAttemptSha256","requestSnapshotSha256","compiledPrompt","compiledSchema","compiledPromptSha256","compiledSchemaSha256","domainAccepted"].iter().all(|k|original[k]==record[k]))
}
pub(crate) fn lane_recovered_receipt_bound(root:&Path,attempt:&FrozenAttempt,saved:&Value)->Result<()>{historical_receipt(root,attempt)?;let dir=worker_root(root,&attempt.request)?.join(&attempt.attempt_id);let record=read(&dir.join("receipt.json"))?;if record["lanePreparationRecovery"]!=json!({"proof":saved["proof"],"effectiveIdentity":saved["effectiveIdentity"]})||!lane_recovery_record_bound(&dir,&record,&saved["effectiveIdentity"])?{return Err(HELD.into());}Ok(())}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_lane_preparation_events_require_exact_stopped_pre_runtime_sequence(){
        let record=json!({"attempt":{"attemptId":"lane-a","semanticRequestHash":"a","executionFingerprint":"b","attemptEnvelopeHash":"c"},"compiledPromptSha256":"p","compiledSchemaSha256":"s"});
        let events=json!([
            {"id":1,"type":"sdk_dispatch_receipt","data":{"ok":true,"protocolVersion":PROTOCOL,"attemptId":"lane-a","result":{"state":"dispatching","lastSequence":0,"semanticRequestHash":"a","executionFingerprint":"b","attemptEnvelopeHash":"c","compiledPromptSha256":"p","compiledSchemaSha256":"s"}}},
            {"id":2,"type":"sdk_transport_unknown","data":{"attemptId":"lane-a","lastSdkSequence":0,"error":"SDK_STDERR_CLOSED"}},
            {"id":3,"type":"sdk_process_tree","data":{"processTreeStopProven":true,"activeProcesses":0,"totalProcesses":3}},
            {"id":4,"type":"sdk_stderr","data":{"truncated":false,"bytes":511,"retainedBytes":511}}]);
        assert!(events_bound(&record,&events));
        for fault in ["sequence","runtime-child","active","stop","sdk-event","order","hash","unknown","truncated"] {
            let mut changed=events.clone();match fault {
                "sequence"=>changed[1]["data"]["lastSdkSequence"]=json!(1),"runtime-child"=>changed[2]["data"]["totalProcesses"]=json!(4),
                "active"=>changed[2]["data"]["activeProcesses"]=json!(1),"stop"=>changed[2]["data"]["processTreeStopProven"]=json!(false),
                "sdk-event"=>changed.as_array_mut().unwrap().push(json!({"id":5,"type":"sdk_event","data":{}})),"order"=>changed[2]["id"]=json!(1),
                "hash"=>changed[0]["data"]["result"]["compiledPromptSha256"]=json!("other"),"unknown"=>changed[1]["data"]["error"]=json!("TLS"),
                "truncated"=>changed[3]["data"]["truncated"]=json!(true),_=>unreachable!()}
            assert!(!events_bound(&record,&changed),"{fault}");
        }
    }
    #[test]
    #[ignore="Explicit original private frozen attempt/events; read-only, zero account/model/runtime calls"]
    fn sdk_lane_preparation_actual_original_is_evidence_only(){
        let root=PathBuf::from(std::env::var_os("DIRECTOR_LANE_PREPARATION_ROOT").expect("private root"));
        let id=std::env::var("DIRECTOR_LANE_PREPARATION_ATTEMPT").expect("private attempt");
        let events=read(Path::new(&std::env::var("DIRECTOR_LANE_PREPARATION_EVENTS").expect("private events"))).unwrap();
        let attempt=FrozenAttempt::load(&root.join("text-runs").join(&id).join("sdk-attempt.json")).unwrap();
        let receipt=worker_root(&root,&attempt.request).unwrap().join(id).join("receipt.json");let before=bytes(&receipt).unwrap();
        let evidence=lane_preparation_evidence(&root,&attempt,&events).unwrap();assert_eq!(evidence["beforeOfficialRuntimeSpawn"],true);assert_eq!(evidence["runtimeInventory"].as_array().unwrap().len(),3);
        assert_eq!(bytes(&receipt).unwrap(),before);assert_eq!(attempt.state,AttemptState::SubmissionUnknown);
    }
}
