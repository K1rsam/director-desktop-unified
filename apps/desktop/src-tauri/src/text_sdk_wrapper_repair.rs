//! One known supervisor build rejected this frozen recovery identity before
//! run_supervised/Command::spawn. Zero SDK events alone never prove this.
use super::*;
pub(crate) const WRAPPER_FAILED_BRIDGE_SHA: &str = "47de63a6a0cc0a7a1255d593fb428b2a6daac80ddd21dd5d953e16f92347fabb";
pub(crate) const WRAPPER_FAILED_SUPERVISOR_SHA: &str = "48a5b6a608163207297f56240c1f871755d9dd51516ecfad7188543e4a37e146";
const HELD: &str = "SDK_WRAPPER_PREPARATION_REPAIR_HELD: 不能证明已知监督器在启动官方运行时前退出；保留全部意图与原任务";
fn bytes(file: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(file).map_err(|_|HELD)?;
    if !meta.is_file() || meta.is_symlink() || meta.len()>4_194_304 {return Err(HELD.into());}
    std::fs::read(file).map_err(|_|HELD.into())
}
fn read(file: &Path) -> Result<Value> {serde_json::from_slice(&bytes(file)?).map_err(|_|HELD.into())}
fn hash(v:&Value)->String {digest(&v.to_string())}
fn absent(p:&Path)->bool {matches!(std::fs::symlink_metadata(p),Err(e) if e.kind()==std::io::ErrorKind::NotFound)}
// Read-only counterpart to OfficialSdkBridge.initialize's serialized stale-lock
// recovery. The bridge still acquires its guard and rechecks ownership itself.
// Access-denied/unknown PIDs never count as exited, and no lock is deleted here.
fn worker_lock_evidence(worker:&Path)->Result<Value> {
    let file=worker.join("worker.lock");
    if absent(&file){return Ok(json!({"state":"absent"}));}
    let before=bytes(&file)?;let owner:Value=serde_json::from_slice(&before).map_err(|_|HELD)?;
    let pid=owner["pid"].as_u64().filter(|v|*v>0&&*v<=u32::MAX as u64).ok_or(HELD)? as u32;
    let instance=owner["instanceId"].as_str().filter(|v|!v.is_empty()&&v.len()<=128).ok_or(HELD)?;
    #[cfg(windows)]
    let exited=unsafe {
        use std::ffi::c_void;
        #[link(name="kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access:u32,inherit:i32,pid:u32)->*mut c_void;
            fn WaitForSingleObject(handle:*mut c_void,milliseconds:u32)->u32;
            fn CloseHandle(handle:*mut c_void)->i32;
            fn GetLastError()->u32;
        }
        let handle=OpenProcess(0x00100000,0,pid);
        if handle.is_null(){GetLastError()==87}else{let done=WaitForSingleObject(handle,0)==0;CloseHandle(handle);done}
    };
    #[cfg(not(windows))]
    let exited=false;
    if !exited||bytes(&file)?!=before{return Err(HELD.into());}
    Ok(json!({"state":"stale","pid":pid,"instanceId":instance,"sha256":crate::store::hash_file(&file)?.0}))
}
fn inventory(root:&Path)->Result<Value> {
    fn walk(root:&Path, dir:&Path, items:&mut Vec<Value>)->Result<()> {
        let m=std::fs::symlink_metadata(dir).map_err(|_|HELD)?;
        if !m.is_dir()||m.is_symlink(){return Err(HELD.into());}
        for e in std::fs::read_dir(dir).map_err(|_|HELD)? {
            let p=e.map_err(|_|HELD)?.path();let m=std::fs::symlink_metadata(&p).map_err(|_|HELD)?;
            if m.is_symlink(){return Err(HELD.into());}
            if m.is_dir(){walk(root,&p,items)?;}else{
                let body=bytes(&p)?;
                items.push(json!([p.strip_prefix(root).map_err(|_|HELD)?.to_string_lossy().replace('\\',"/"),body.len(),crate::store::hash_file(&p)?.0]));
                if items.len()>8{return Err(HELD.into());}
            }
        }Ok(())
    }
    let mut items=Vec::new();walk(root,root,&mut items)?;items.sort_by(|a,b|a[0].as_str().cmp(&b[0].as_str()));Ok(json!(items))
}
fn failed_identity(identity:&Value)->bool {
    identity["bridgeSha256"]==WRAPPER_FAILED_BRIDGE_SHA && identity["supervisorSha256"]==WRAPPER_FAILED_SUPERVISOR_SHA
}
pub(crate) fn wrapper_preparation_evidence(root:&Path, attempt:&FrozenAttempt, previous:&Value, events:&Value)->Result<Value> {
    historical_receipt(root,attempt)?;
    let dir=root.join("sdk-worker").join(&attempt.attempt_id);let runtime=dir.join("native-runtime");
    let worker=root.join("sdk-worker");
    for p in [&worker,&dir] {let m=std::fs::symlink_metadata(p).map_err(|_|HELD)?;if !m.is_dir()||m.is_symlink(){return Err(HELD.into());}}
    for p in [worker.join("worker.lock.recovery"),dir.join("events.ndjson"),runtime.join("process-receipt.json"),dir.join("wrapper-preparation-repair.json"),dir.join("wrapper-preparation-original.json"),dir.join("native-runtime-wrapper-repair")] {if !absent(&p){return Err(HELD.into());}}
    let worker_lock=worker_lock_evidence(&worker)?;
    let before=bytes(&dir.join("receipt.json"))?;let record:Value=serde_json::from_slice(&before).map_err(|_|HELD)?;
    let identity=&previous["effectiveIdentity"];
    if attempt.state!=crate::text_transport::AttemptState::SubmissionUnknown || attempt.last_sequence>1 || attempt.raw_result.is_some()
        || !local_recovery_identity_bound(&attempt.runtime_identity,identity)
        || !failed_identity(identity) || crate::store::hash_file(Path::new(s(identity,"supervisorExecutable")))?.0!=WRAPPER_FAILED_SUPERVISOR_SHA
        || record["state"]!="dispatching" || !record["error"].is_null() || record.get("wrapperPreparationRepair").is_some()
        || record["preDispatchRecovery"]!=json!({"proof":previous["proof"],"effectiveIdentity":identity})
        || record["lastSequence"]!=0 || !record["threadId"].is_null() || !record["turnId"].is_null() || !record["rawResult"].is_null() || !record["usage"].is_null()
        || read(&dir.join("local-preparation-recovery.json"))?!=previous["proof"]
        || crate::store::hash_file(&dir.join("local-preparation-original.json"))?.0!=s(&previous["evidence"],"originalReceiptSha256")
        || std::fs::read_dir(&dir).map_err(|_|HELD)?.count()!=4 {return Err(HELD.into());}
    let plan=read(&runtime.join("runtime-plan.json"))?;
    // In the pinned failed binary this mismatch is unconditionally returned
    // before argv/stdin validation and before the sole runtime spawn function.
    if plan["version"]!="sdk-native-runtime/v1" || plan["identity"]!=*identity || plan["identity"]==record["attempt"]["runtimeIdentity"]
        || plan["attemptId"]!=attempt.attempt_id || plan["semanticRequestHash"]!=attempt.semantic_request_hash || plan["executionFingerprint"]!=attempt.execution_fingerprint
        || plan["model"]!=record["attempt"]["request"]["model"] || plan["reasoning"]!=record["attempt"]["request"]["reasoning"]
        || plan["compiledPromptSha256"]!=record["compiledPromptSha256"] || plan["compiledSchemaSha256"]!=record["compiledSchemaSha256"] {return Err(HELD.into());}
    let compiled=crate::text_executor::compile_sdk_attempt_input(attempt)?;
    if record["compiledPrompt"]!=compiled.prompt||record["compiledSchema"]!=compiled.schema
        || record["compiledPromptSha256"]!=digest(&compiled.prompt)||record["compiledSchemaSha256"]!=digest(&compiled.schema)
        || std::fs::canonicalize(Path::new(s(&plan,"workingDirectory"))).map_err(|_|HELD)?!=std::fs::canonicalize(runtime.join("work")).map_err(|_|HELD)?
        || std::fs::canonicalize(Path::new(s(&plan,"tempDirectory"))).map_err(|_|HELD)?!=std::fs::canonicalize(runtime.join("tmp")).map_err(|_|HELD)? {return Err(HELD.into());}
    let list=inventory(&runtime)?;let files=list.as_array().ok_or(HELD)?;
    let schema=files.iter().find(|f|f[0].as_str().is_some_and(|p|p.starts_with("tmp/codex-output-schema-")&&p.ends_with("/schema.json"))).ok_or(HELD)?;
    if files.len()!=3 || !files.iter().any(|f|f[0]=="runtime-plan.json")
        || !files.iter().any(|f|f[0]=="work/.director-text-root"&&f[2]==digest("director SDK text execution\n"))
        || schema[2]!=plan["runtimeSchemaSha256"] {return Err(HELD.into());}
    let rows=events.as_array().ok_or(HELD)?;
    let reservations:Vec<_>=rows.iter().enumerate().filter(|(_,e)|e["type"]=="sdk_local_preparation_recovery_reserved").collect();
    if reservations.len()!=1{return Err(HELD.into());}let (n,reserved)=reservations[0];
    if reserved["data"]!=*previous || hash(&json!(&rows[..n]))!=s(&previous["evidence"],"eventsSha256") || rows.len()!=n+5 {return Err(HELD.into());}
    let tail=&rows[n+1..];let ack=&tail[0]["data"];let unknown=&tail[1]["data"];let tree=&tail[2]["data"];
    if tail[0]["type"]!="sdk_dispatch_receipt"||tail[1]["type"]!="sdk_transport_unknown"||tail[2]["type"]!="sdk_process_tree"||tail[3]["type"]!="sdk_stderr"
        || ack["ok"]!=true||ack["attemptId"]!=attempt.attempt_id||ack["result"]["state"]!="dispatching"||ack["result"]["lastSequence"]!=0
        || unknown["attemptId"]!=attempt.attempt_id||unknown["lastSdkSequence"]!=0||tree["processTreeStopProven"]!=true||tree["activeProcesses"]!=0||tree["totalProcesses"]!=3
        || rows.windows(2).any(|w|w[0]["id"].as_i64().unwrap_or(0)>=w[1]["id"].as_i64().unwrap_or(0)) {return Err(HELD.into());}
    for k in ["semanticRequestHash","executionFingerprint","attemptEnvelopeHash"] {if ack["result"][k]!=record["attempt"][k]{return Err(HELD.into());}}
    if bytes(&dir.join("receipt.json"))?!=before||inventory(&runtime)?!=list||worker_lock_evidence(&worker)?!=worker_lock{return Err(HELD.into());}
    Ok(json!({"version":"sdk-wrapper-preparation-evidence/v1","failedReceiptSha256":crate::store::hash_file(&dir.join("receipt.json"))?.0,
        "failedReceiptSeal":record["receiptEnvelopeSha256"],"runtimeInventory":list,"runtimeInventorySha256":hash(&list),
        "eventsSha256":hash(events),"workerLock":worker_lock,"failedSupervisorSha256":WRAPPER_FAILED_SUPERVISOR_SHA,
        "priorRecoveryProofSha256":hash(&previous["proof"]),"sdkEvents":0,"processTreeStopProven":true,"beforeOfficialRuntimeSpawn":true}))
}
pub(crate) fn wrapper_repair_record_bound(dir:&Path, record:&Value, effective:&Value)->Result<bool> {
    let Some(repair)=record.get("wrapperPreparationRepair") else{return Ok(false);};let proof=&repair["proof"];
    if repair["effectiveIdentity"]!=*effective||proof["version"]!="sdk-wrapper-preparation-repair/v1"
        || proof["attemptId"]!=record["attempt"]["attemptId"]||proof["attemptEnvelopeHash"]!=record["attempt"]["attemptEnvelopeHash"]
        || proof["effectiveIdentitySha256"]!=hash(effective)||proof["priorRecoveryProofSha256"]!=hash(&record["preDispatchRecovery"]["proof"])
        || !failed_identity(&record["preDispatchRecovery"]["effectiveIdentity"])
        || !local_recovery_identity_bound(&record["attempt"]["runtimeIdentity"],effective)
        || proof["sdkEvents"]!=0||proof["processTreeStopProven"]!=true||proof["beforeOfficialRuntimeSpawn"]!=true
        || read(&dir.join("wrapper-preparation-repair.json"))?!=*proof
        || crate::store::hash_file(&dir.join("wrapper-preparation-original.json"))?.0!=s(proof,"failedReceiptSha256")
        || hash(&inventory(&dir.join("native-runtime"))?)!=s(proof,"runtimeInventorySha256") {return Ok(false);}
    let old=read(&dir.join("wrapper-preparation-original.json"))?;
    Ok(old["state"]=="dispatching"&&old["error"].is_null()&&old["preDispatchRecovery"]==record["preDispatchRecovery"]
        && ["attempt","frozenAttemptSha256","requestSnapshotSha256","compiledPrompt","compiledSchema","compiledPromptSha256","compiledSchemaSha256"]
            .iter().all(|key|old[key]==record[key]))
}
pub(crate) fn repaired_receipt_bound(root:&Path, attempt:&FrozenAttempt, saved:&Value)->Result<()> {
    let dir=root.join("sdk-worker").join(&attempt.attempt_id);let record=read(&dir.join("receipt.json"))?;
    if record["wrapperPreparationRepair"]!=json!({"proof":saved["proof"],"effectiveIdentity":saved["effectiveIdentity"]})
        || !wrapper_repair_record_bound(&dir,&record,&saved["effectiveIdentity"])? {return Err(HELD.into());}Ok(())
}
#[cfg(test)]
pub(crate) fn test_append_wrapper_repair(dir:&Path,failed:&Value,effective:&Value)->Value {
    let text=failed.to_string();let sealed=format!("{},\"receiptEnvelopeSha256\":\"{}\"}}",&text[..text.len()-1],hash(failed));
    std::fs::write(dir.join("wrapper-preparation-original.json"),&sealed).unwrap();
    let list=inventory(&dir.join("native-runtime")).unwrap();
    let evidence=json!({"failedReceiptSha256":digest(&sealed),"runtimeInventory":list,"runtimeInventorySha256":hash(&list)});
    let proof=json!({"version":"sdk-wrapper-preparation-repair/v1","attemptId":failed["attempt"]["attemptId"],"attemptEnvelopeHash":failed["attempt"]["attemptEnvelopeHash"],
        "priorRecoveryProofSha256":hash(&failed["preDispatchRecovery"]["proof"]),"failedReceiptSha256":evidence["failedReceiptSha256"],"runtimeInventorySha256":evidence["runtimeInventorySha256"],
        "evidenceSha256":hash(&evidence),"effectiveIdentitySha256":hash(effective),"sdkEvents":0,"processTreeStopProven":true,"beforeOfficialRuntimeSpawn":true});
    std::fs::write(dir.join("wrapper-preparation-repair.json"),proof.to_string()).unwrap();
    json!({"proof":proof,"evidence":evidence,"effectiveIdentity":effective})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_wrapper_worker_lock_rejects_live_or_invalid_owner_without_mutation() {
        let temp=tempfile::tempdir().unwrap();let file=temp.path().join("worker.lock");
        assert_eq!(worker_lock_evidence(temp.path()).unwrap(),json!({"state":"absent"}));
        for owner in [json!({"pid":std::process::id(),"instanceId":"live-test"}),json!({"pid":0,"instanceId":"invalid-test"}),json!({"pid":u64::MAX,"instanceId":"overflow-test"}),json!({"pid":10,"instanceId":""})] {
            let body=owner.to_string();std::fs::write(&file,&body).unwrap();
            assert!(worker_lock_evidence(temp.path()).is_err());assert_eq!(std::fs::read_to_string(&file).unwrap(),body);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let mut child=std::process::Command::new(std::env::current_exe().unwrap()).args(["sdk_lock_empty_filter_nonexistent","--exact"]).creation_flags(0x08000000).spawn().unwrap();
            let pid=child.id();assert!(child.wait().unwrap().success());drop(child);
            let body=json!({"pid":pid,"instanceId":"exited-test"}).to_string();std::fs::write(&file,&body).unwrap();
            let evidence=worker_lock_evidence(temp.path()).unwrap();assert_eq!(evidence["state"],"stale");assert_eq!(evidence["sha256"],digest(&body));
            assert_eq!(std::fs::read_to_string(&file).unwrap(),body);
        }
    }
    #[test]
    #[ignore = "explicit captured known wrapper case; read-only original files/SQLite, isolated pollution fixtures, zero account/model/dispatch"]
    fn sdk_wrapper_preparation_captured_readonly_proof_and_pollution_gate() {
        let case=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.local/validation/stability-case-20261014/case.json");
        let case:Value=serde_json::from_slice(&std::fs::read(case).unwrap()).unwrap();let root=PathBuf::from(s(&case,"rootPath"));
        let id="8a7ddf94-dd59-4fbc-85e0-164682daffa6";
        let c=rusqlite::Connection::open_with_flags(root.join("project.sqlite"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let prepared:String=c.query_row("SELECT prepared FROM jobs WHERE id=?1 AND state='submission_unknown'",[id],|r|r.get(0)).unwrap();
        let prepared:Value=serde_json::from_str(&prepared).unwrap();let previous=&prepared["sdkLocalPreparationRecovery"];
        assert!(prepared.get("sdkWrapperPreparationRepair").is_none());
        let mut q=c.prepare("SELECT id,type,data FROM events WHERE job_id=?1 ORDER BY id").unwrap();
        let rows=q.query_map([id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).unwrap();
        let events=json!(rows.map(|row|{let (id,kind,data)=row.unwrap();json!({"id":id,"type":kind,"data":serde_json::from_str::<Value>(&data).unwrap()})}).collect::<Vec<_>>());
        let attempt=FrozenAttempt::load(&root.join("text-runs").join(id).join("sdk-attempt.json")).unwrap();
        let dir=root.join("sdk-worker").join(id);let receipt=std::fs::read(dir.join("receipt.json")).unwrap();let inventory_before=inventory(&dir.join("native-runtime")).unwrap();
        let proof=wrapper_preparation_evidence(&root,&attempt,previous,&events).unwrap();
        assert_eq!(proof["failedReceiptSha256"],"68cd4c38b77d25ff70e3d25c691339900146819434b204227f580a1c78251468");
        fn copy(from:&Path,to:&Path){std::fs::create_dir_all(to).unwrap();for entry in std::fs::read_dir(from).unwrap(){let p=entry.unwrap().path();let m=std::fs::symlink_metadata(&p).unwrap();assert!(!m.is_symlink());let next=to.join(p.file_name().unwrap());if m.is_dir(){copy(&p,&next)}else{std::fs::copy(p,next).unwrap();}}}
        for fault in ["none","runtime-receipt","sdk-event","tree","unknown-wrapper","plan-account","compiled","runtime-file","reservation","live-lock","invalid-lock","recovery-lock"] {
            let temp=tempfile::tempdir().unwrap();let test_dir=temp.path().join("sdk-worker").join(id);copy(&dir,&test_dir);
            let file=test_dir.join("native-runtime/runtime-plan.json");let mut plan=read(&file).unwrap();
            plan["workingDirectory"]=json!(test_dir.join("native-runtime/work"));plan["tempDirectory"]=json!(test_dir.join("native-runtime/tmp"));
            let mut e=events.clone();let mut saved=previous.clone();let mut original=attempt.clone();
            match fault {
                "runtime-receipt"=>std::fs::write(test_dir.join("native-runtime/process-receipt.json"),"{}").unwrap(),
                "sdk-event"=>{e.as_array_mut().unwrap().push(json!({"id":999,"type":"sdk_event","data":{}}));},
                "tree"=>{let row=e.as_array_mut().unwrap().iter_mut().rev().find(|e|e["type"]=="sdk_process_tree").unwrap();row["data"]["activeProcesses"]=json!(1);},
                "unknown-wrapper"=>saved["effectiveIdentity"]["supervisorSha256"]=json!("f".repeat(64)),
                "plan-account"=>plan["identity"]["accountFingerprint"]=json!("f".repeat(64)),
                "compiled"=>original.request.input=json!({"changed":true}),
                "runtime-file"=>std::fs::write(test_dir.join("native-runtime/unexpected"),"{} ").unwrap(),
                "reservation"=>std::fs::write(test_dir.join("wrapper-preparation-repair.json"),"{}").unwrap(),
                "live-lock"=>std::fs::write(temp.path().join("sdk-worker/worker.lock"),json!({"pid":std::process::id(),"instanceId":"live-test"}).to_string()).unwrap(),
                "invalid-lock"=>std::fs::write(temp.path().join("sdk-worker/worker.lock"),json!({"pid":0,"instanceId":"invalid-test"}).to_string()).unwrap(),
                "recovery-lock"=>std::fs::write(temp.path().join("sdk-worker/worker.lock.recovery"),"{}").unwrap(),
                "none"=>(),_=>unreachable!(),
            }
            std::fs::write(&file,plan.to_string()).unwrap();let result=wrapper_preparation_evidence(temp.path(),&original,&saved,&e);
            assert_eq!(result.is_ok(),fault=="none","{fault}: {result:?}");
        }
        assert_eq!(std::fs::read(dir.join("receipt.json")).unwrap(),receipt);assert_eq!(inventory(&dir.join("native-runtime")).unwrap(),inventory_before);
        println!("captured wrapper proof verified; original files unchanged; 11 pollution gates rejected; zero model/account/dispatch");
    }
}
