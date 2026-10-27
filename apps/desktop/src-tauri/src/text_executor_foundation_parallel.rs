//! Opt-in foundation trial concurrency. Default SDK dispatch remains serial.
//! Process leases never turn a durable UNKNOWN into an executable request.
use super::*;

pub(super) const PROFILE: &str = "foundation-three-lane-trial/v1";
const HELD: &str = "FOUNDATION_PARALLEL_SDK_HELD: 三路授权、执行租约或原任务证据不匹配；未重复提交";
static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
static ACTIVE: OnceLock<Mutex<Vec<Active>>> = OnceLock::new();

#[derive(Clone)]
struct Active { token:String, root:PathBuf, run:String, authorization:String, lane:u64, job:Option<String> }
pub(super) struct Lease { token:String, _permit:tokio::sync::OwnedSemaphorePermit }
impl Drop for Lease {
    fn drop(&mut self) { if let Ok(mut active)=ACTIVE.get_or_init(||Mutex::new(Vec::new())).lock(){active.retain(|a|a.token!=self.token);} }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(lane:u64)->TextRequest {
        TextRequest{stage:"foundation:F2".into(),skill:"foundation-asset-catalog".into(),model:"synthetic".into(),reasoning:"medium".into(),
            input:json!({"delivery":{"method":"foundation-delivery/v2","scope":["ep2"],"batchIndex":3},
                "foundationParallelTrial":{"version":PROFILE,"concurrency":3,"authorizationSha256":"a".repeat(64),
                    "sourceSha256":"b".repeat(64),"basisSha256":"c".repeat(64),"workflowId":"fixture-run","laneId":lane,"packetId":"f2:3"}}),
            schema:json!({"type":"object"}),rule_snapshot:json!({})}
    }
    #[test]
    fn foundation_parallel_keys_and_worker_roots_preserve_legacy_and_bound_revision() {
        let mut request=request(0);validate_markers(&request).unwrap();let base=step_key(&request);
        let root=std::env::temp_dir().join("synthetic-worker-root");
        let first=crate::text_sdk::worker_root(&root,&request).unwrap();
        request.input["foundationParallelTrial"]["laneId"]=json!(1);
        assert_ne!(first,crate::text_sdk::worker_root(&root,&request).unwrap());assert_ne!(base,step_key(&request));
        request.input["foundationParallelRebase"]=json!({"version":"foundation-parallel-rebase/v1","attempt":1,
            "originalInputSha256":"d".repeat(64),"originalResultSha256":"e".repeat(64),"currentBasisSha256":"f".repeat(64)});
        validate_markers(&request).unwrap();let repaired=step_key(&request);assert_ne!(base,repaired);
        request.input["foundationParallelRebase"]["attempt"]=json!(2);assert!(validate_markers(&request).is_err());
        request.input.as_object_mut().unwrap().remove("foundationParallelRebase");
        request.input.as_object_mut().unwrap().remove("foundationParallelTrial");
        assert_eq!(step_key(&request),base_step_key(&request));
        assert_eq!(crate::text_sdk::worker_root(&root,&request).unwrap(),root.join("sdk-worker"));
    }
    #[test]
    fn foundation_parallel_marker_rejects_unknown_versions_stages_and_unsafe_paths() {
        for field in ["version","authorizationSha256","basisSha256","sourceSha256","laneId","concurrency","packetId"] {
            let mut r=request(0);r.input["foundationParallelTrial"][field]=json!("../bad");assert!(validate_markers(&r).is_err(),"{field}");
        }
        for packet in ["bad/name","bad\\name","bad\nname","bad\0name"] {
            let mut r=request(0);r.input["foundationParallelTrial"]["packetId"]=json!(packet);assert!(validate_markers(&r).is_err());
        }
        let mut r=request(0);r.stage="seriesAuthor".into();assert!(validate_markers(&r).is_err());
        let mut r=request(0);r.input["foundationParallelFinal"]=json!({"version":"foundation-parallel-final/v999"});assert!(validate_markers(&r).is_err());
    }
    #[tokio::test]
    async fn foundation_parallel_input_marker_cannot_mint_authorization() {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("synthetic","synthetic").unwrap();
        store.ensure_workflow_storage().unwrap();
        let exec=SavedTextExecutor{store,settings:json!({}),workflow_id:Some("fixture-run".into())};let r=request(0);
        assert!(exec.foundation_parallel_lease(&r,slot(&r).await.unwrap()).is_err());
    }
    #[tokio::test]
    async fn foundation_parallel_unknown_and_restarted_inflight_are_never_sibling_exemptions() {
        let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("synthetic","synthetic").unwrap();store.ensure_workflow_storage().unwrap();
        let exec=SavedTextExecutor{store,settings:json!({}),workflow_id:Some("fixture-run".into())};
        let r=request(0);let other=request(1);let snapshot=serde_json::to_value(&other).unwrap();
        let job=exec.store.add_job("text","fixture","codex_sdk","account",&snapshot,"running").unwrap();
        exec.store.update_job(&job,"running",&json!({"prepared":{"sdkDispatchStarted":true}})).unwrap();
        let token=id();let owned=Active{token:token.clone(),root:exec.store.root.clone(),run:"fixture-run".into(),authorization:"a".repeat(64),lane:0,job:None};
        ACTIVE.get_or_init(||Mutex::new(Vec::new())).lock().unwrap().push(owned.clone());
        let _lease=Lease{token,_permit:slot(&r).await.unwrap().unwrap()};
        // A durable running record without a process-owned sibling lease is unknown.
        assert!(exec.check_sdk_request_hold(&r).is_err());
        let token=id();let mut sibling=owned;sibling.token=token.clone();sibling.lane=1;sibling.job=Some(job.clone());
        ACTIVE.get().unwrap().lock().unwrap().push(sibling);let _other_lease=Lease{token,_permit:slot(&other).await.unwrap().unwrap()};
        assert!(exec.check_sdk_request_hold(&r).is_ok());
        exec.store.update_job(&job,"submission_unknown",&json!({})).unwrap();assert!(exec.check_sdk_request_hold(&r).is_err());
        assert!(exec.check_sdk_unresolved_hold().is_err(),"legacy entry stays strictly serial");
    }
}
fn sha(value:&Value)->bool { value.as_str().is_some_and(|v|v.len()==64 && v.bytes().all(|b|b.is_ascii_hexdigit())) }
pub(super) fn validate_markers(request:&TextRequest)->Result<()> {
    let input=&request.input;
    let any=["foundationParallelTrial","foundationParallelRebase","foundationParallelFinal","foundationConflictResolution"].iter().any(|k|input.get(*k).is_some());
    if !any{return Ok(());}
    if !matches!((request.stage.as_str(),request.skill.as_str()),("foundation:F2","foundation-asset-catalog")|("foundation:Q1","foundation-finishing-editor"))
        || input["delivery"]["method"]!="foundation-delivery/v2" {return Err(HELD.into());}
    let marker=&input["foundationParallelTrial"];
    if marker["version"]!=PROFILE || marker["concurrency"]!=3 || !sha(&marker["authorizationSha256"])
        || !sha(&marker["basisSha256"]) || !sha(&marker["sourceSha256"])
        || marker["laneId"].as_u64().is_none_or(|n|n>2)
        || s(marker,"workflowId").is_empty() || s(marker,"packetId").is_empty() || s(marker,"packetId").len()>512
        || s(marker,"packetId").chars().any(|c|c.is_control() || c=='/' || c=='\\') {
        return Err(HELD.into());
    }
    for (field,version) in [("foundationParallelRebase","foundation-parallel-rebase/v1"),("foundationParallelFinal","foundation-parallel-final/v1")] {
        if input.get(field).is_some_and(|v|!v.is_object() || v["version"]!=version){return Err(HELD.into());}
    }
    if let Some(rebase)=input.get("foundationParallelRebase") {
        if rebase["attempt"]!=1 || ["originalInputSha256","originalResultSha256","currentBasisSha256"].iter().any(|key|!sha(&rebase[*key])) {
            return Err(HELD.into());
        }
    }
    foundation_conflict::validate_marker(request)?;
    if let Some(final_check)=input.get("foundationParallelFinal") {
        if request.stage!="foundation:Q1" || !sha(&final_check["basisSha256"]) || !sha(&final_check["signalsSha256"])
            || !final_check["scope"].is_array(){return Err(HELD.into());}
    }
    Ok(())
}
pub(super) async fn slot(request:&TextRequest)->Result<Option<tokio::sync::OwnedSemaphorePermit>> {
    validate_markers(request)?;
    if request.input.get("foundationParallelTrial").is_none(){return Ok(None);}
    Ok(Some(SLOTS.get_or_init(||Arc::new(tokio::sync::Semaphore::new(3))).clone().acquire_owned().await.map_err(|_|HELD)?))
}
impl SavedTextExecutor {
    pub(super) fn sdk_parallel_worker_bound(&self,job:&Value,snapshot:&Value)->Result<()> {
        let request:TextRequest=serde_json::from_value(snapshot.clone()).map_err(|_|HELD)?;
        let marker=&request.input["foundationParallelTrial"];
        if marker.is_null(){
            if job["prepared"].get("sdkWorkerScope").is_some(){return Err(HELD.into());}return Ok(());
        }
        validate_markers(&request)?;
        // Zero-dispatch queue reservations have no worker yet.
        if job["prepared"]["sdkDispatchStarted"]!=true{return Ok(());}
        let expected=json!({"version":"foundation-sdk-lane-worker/v1","authorizationSha256":marker["authorizationSha256"],
            "laneId":marker["laneId"],"workerRoot":crate::text_sdk::worker_root(&self.store.root,&request)?});
        if job["prepared"]["sdkWorkerScope"]!=expected{return Err(HELD.into());}Ok(())
    }
    pub(super) fn foundation_parallel_lease(&self,request:&TextRequest,permit:Option<tokio::sync::OwnedSemaphorePermit>)->Result<Option<Lease>> {
        let Some(permit)=permit else{return Ok(None)};
        let marker=&request.input["foundationParallelTrial"];
        let run=self.workflow_id.as_deref().ok_or(HELD)?;
        crate::foundation_trial_store::validate_request(&self.store,run,marker)?;
        let authorization=s(marker,"authorizationSha256");let lane=marker["laneId"].as_u64().ok_or(HELD)?;
        let mut active=ACTIVE.get_or_init(||Mutex::new(Vec::new())).lock().map_err(|_|HELD)?;
        if active.iter().any(|a|a.root==self.store.root && (a.run!=run || a.authorization!=authorization || a.lane==lane)){return Err(HELD.into());}
        let token=id();active.push(Active{token:token.clone(),root:self.store.root.clone(),run:run.into(),authorization:authorization.into(),lane,job:None});
        Ok(Some(Lease{token,_permit:permit}))
    }
    pub(super) fn foundation_parallel_bind(&self,lease:&Option<Lease>,job:&str)->Result<()> {
        let Some(lease)=lease else{return Ok(())};
        let mut active=ACTIVE.get_or_init(||Mutex::new(Vec::new())).lock().map_err(|_|HELD)?;
        let row=active.iter_mut().find(|a|a.token==lease.token).ok_or(HELD)?;
        if row.job.as_deref().is_some_and(|prior|prior!=job){return Err(HELD.into());}row.job=Some(job.into());Ok(())
    }
    pub(super) fn check_sdk_parallel_hold(&self,request:&TextRequest,lease:&Option<Lease>)->Result<()> {
        if lease.is_none(){return self.check_sdk_unresolved_hold();}
        self.check_sdk_request_hold(request)
    }
    pub(super) fn check_sdk_request_hold(&self,request:&TextRequest)->Result<()> {
        if request.input.get("foundationParallelTrial").is_none(){return self.check_sdk_unresolved_hold();}
        let marker=&request.input["foundationParallelTrial"];
        let active=ACTIVE.get_or_init(||Mutex::new(Vec::new())).lock().map_err(|_|HELD)?.clone();
        let owned=active.iter().find(|a|a.root==self.store.root && Some(a.run.as_str())==self.workflow_id.as_deref()
            && a.authorization==s(marker,"authorizationSha256") && Some(a.lane)==marker["laneId"].as_u64()).ok_or(HELD)?;
        let c=self.store.conn()?;
        let mut q=c.prepare("SELECT id,state,snapshot FROM jobs WHERE kind='text' AND provider='codex_sdk' AND state NOT IN ('completed','rejected') AND json_extract(prepared,'$.sdkDispatchStarted')=1").map_err(|e|e.to_string())?;
        let records=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?;
        for row in records {
            let(job,state,text)=row.map_err(|e|e.to_string())?;
            let snapshot:Value=serde_json::from_str(&text).map_err(|_|HELD)?;
            let other=&snapshot["input"]["foundationParallelTrial"];
            if other["authorizationSha256"]!=marker["authorizationSha256"] || other["workflowId"]!=marker["workflowId"]
                || other["laneId"]==marker["laneId"] {return Err(HELD.into());}
            // A returned immutable result may await domain acceptance after its
            // transport lease drops. Revalidate it; never infer success by state.
            if state=="received" {self.sdk_raw(&self.sdk_job(&job)?,&snapshot)?;continue;}
            if !matches!(state.as_str(),"submitting"|"running") || !active.iter().any(|a|a.root==owned.root && a.run==owned.run
                && a.authorization==owned.authorization && a.lane!=owned.lane && Some(a.lane)==other["laneId"].as_u64()
                && a.job.as_deref()==Some(job.as_str())) {return Err(HELD.into());}
        }
        Ok(())
    }
}
