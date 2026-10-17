//! Runs a fixed set of non-generating native regressions outside packaged-host redirection.
use std::{path::PathBuf,process::{Command,Stdio}};
fn main()->Result<(),Box<dyn std::error::Error>>{
    if director_core::windows_runtime::ensure_unvirtualized()?{return Ok(());}
    let args:Vec<_>=std::env::args().filter(|s|s!="--unvirtualized-desktop").collect();if args.len()!=3{return Err("test exe and isolated report directory required".into());}
    let exe=PathBuf::from(&args[1]);let out=PathBuf::from(&args[2]);
    if !exe.is_absolute()||!exe.file_name().unwrap_or_default().to_string_lossy().starts_with("director_core-")||!out.is_absolute()||!out.to_string_lossy().contains("opencli-native-acceptance"){return Err("native regression paths rejected".into());}
    std::fs::create_dir_all(&out)?;let filters=["engine::repair_tests::missing_source_blocks_text_design_but_manual_image_prompt_can_queue_without_text_cli","engine::task_record_tests::task_record_maintenance_is_leased_idempotent_and_never_recovers_another_job","windows_runtime::windows::tests::empty_home_checks_future_writes_without_initializing_a_registry","windows_runtime::windows::tests::shared_registry_does_not_accept_another_filesystem_directory","text_sdk::native::wrapper_tests::sdk_native_wrapper_full_validation_accepts_frozen_normal_and_proven_recovery_identity","text_sdk::native::wrapper_tests::sdk_native_wrapper_full_validation_repair_preserves_v1_and_rejects_broken_lineage","providers::opencli::tests"];
    let mut results=vec![];for(i,filter)in filters.iter().enumerate(){let log=std::fs::File::create(out.join(format!("test-{i}.txt")))?;let clone=log.try_clone()?;let status=Command::new(&exe).arg(filter).arg("--test-threads=1").stdout(Stdio::from(log)).stderr(Stdio::from(clone)).status()?;results.push(serde_json::json!({"filter":filter,"passed":status.success()}));}
    director_core::store::atomic_json(&out.join("result.json"),&serde_json::json!({"packaged":director_core::windows_runtime::is_packaged()?,"results":results}))?;Ok(())
}
