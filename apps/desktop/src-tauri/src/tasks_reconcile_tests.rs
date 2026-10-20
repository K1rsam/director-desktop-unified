use super::*;
struct OnlyReconcile(QueryResult);
impl Channels for OnlyReconcile {
 fn capabilities(&self,_:&ProviderConfig)->ProviderResult<Capabilities>{panic!("not a prepare")}
 fn prepare(&self,_:&ProviderConfig,_:&GenerationRequest)->ProviderResult<PreparedRequest>{panic!("never prepare")}
 fn submit(&self,_:&ProviderConfig,_:&PreparedRequest)->ProviderResult<SubmitReceipt>{panic!("never submit")}
 fn query(&self,_:&ProviderConfig,_:&str)->ProviderResult<QueryResult>{panic!("only reconcile")}
 fn download(&self,_:&ProviderConfig,_:&str,_:&Path)->ProviderResult<DownloadReceipt>{panic!("no download")}
 fn reconcile(&self,_:&ProviderConfig,id:&str)->ProviderResult<QueryResult>{assert_eq!(id,"original");Ok(self.0.clone())}
}
fn fixture()->(tempfile::TempDir,Store,String,ProviderConfig){
 let temp=tempfile::tempdir().unwrap();let store=Store::open(temp.path().join("project")).unwrap();store.create("reconcile","p").unwrap();
 let key=store.add_job("image","asset","chrome","account",&json!({"requestId":"original","immutable":"source"}),"submission_unknown").unwrap();
 let cfg=ProviderConfig{kind:"chrome".into(),endpoint:None,credential_env:None,cli_path:None,bridge_token_env:None,timeout_seconds:5,adapted_models:vec![],chrome_binding:None,browser_transport:None};
 (temp,store,key,cfg)
}
fn answer(status:&str,urls:Vec<String>)->OnlyReconcile{OnlyReconcile(QueryResult{task_id:"original".into(),status:status.into(),result_urls:urls,error:Some("original answer".into()),actual_cost:None,raw:json!({"originalProof":"preserved"})})}
#[test]fn known_failed_reconciliation_settles_without_sending_and_preserves_original(){
 let (_temp,store,key,cfg)=fixture();let before=store.job(&key).unwrap();let channels=answer("failed",vec![]);
 reconcile(&store,&key,&cfg,"account",&channels).unwrap();let after=store.job(&key).unwrap();
 assert_eq!(after["status"],"failed");assert_eq!(after["providerTaskId"],"original");assert_eq!(after["error"],"original answer");assert_eq!(after["result"]["raw"],channels.0.raw);assert_eq!(after["result"]["resultUrls"],json!([]));
 for field in ["snapshot","prepared","receipt"]{assert_eq!(before[field],after[field]);}
}
#[test]fn reconciled_success_without_urls_keeps_query_recovery_instead_of_success(){
 let (_temp,store,key,cfg)=fixture();reconcile(&store,&key,&cfg,"account",&answer("succeeded",vec![])).unwrap();let after=store.job(&key).unwrap();
 assert_eq!(after["status"],"query_failed");assert_eq!(after["providerTaskId"],"original");assert_eq!(after["result"]["status"],"succeeded");
}
#[test]fn reconciled_known_running_and_expired_results_follow_query_contract(){
 for (remote,expected) in [("processing","processing"),("result_expired","result_expired"),("auth_required","auth_required"),("succeeded","succeeded")]{
  let (_temp,store,key,cfg)=fixture();reconcile(&store,&key,&cfg,"account",&answer(remote,vec!["original-result".into()])).unwrap();let after=store.job(&key).unwrap();assert_eq!(after["status"],expected);assert_eq!(after["providerTaskId"],"original");
 }
}
#[test]fn uncertain_reconciliation_keeps_unknown_and_original_bytes(){
 let (_temp,store,key,cfg)=fixture();let before=store.job(&key).unwrap();assert!(reconcile(&store,&key,&cfg,"account",&answer("unknown",vec![])).is_err());assert_eq!(before,store.job(&key).unwrap());
}
#[test]fn different_account_or_saved_task_never_rebinds_unknown(){
 let (_temp,store,key,cfg)=fixture();let before=store.job(&key).unwrap();assert!(reconcile(&store,&key,&cfg,"other",&answer("failed",vec![])).is_err());assert_eq!(before,store.job(&key).unwrap());
 store.update_job(&key,"submission_unknown",&json!({"providerTaskId":"saved-other"})).unwrap();let before=store.job(&key).unwrap();assert!(reconcile(&store,&key,&cfg,"account",&answer("failed",vec![])).is_err());assert_eq!(before,store.job(&key).unwrap());
}
