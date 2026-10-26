//! Original-attempt reconciliation. This module cannot dispatch SDK calls.
use super::*;
use std::io::Write;

pub(super) fn ensure_original_available(store: &Store, media_id: &str) -> Result<()> {
    if pointer(store, Some(media_id))?.is_some() {
        return Ok(());
    }
    let media = store.get("media", media_id)?;
    let previous = &media["visualReview"];
    if previous["evidenceUnavailable"] == true
        || matches!(
            s(previous, "status"),
            "reviewing" | "unknown" | "passed" | "needs_repair"
        )
        || previous["attemptDirectory"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
    {
        return Err("VISUAL_ORIGINAL_EVIDENCE_MISSING: 此候选存在原审核记录，但本项目没有原请求回执；可能来自跨机导入。原摘要保留，不创建新的 SDK 调用，请人工核对图片。".into());
    }
    Ok(())
}
pub(super) fn preserve_missing_evidence(store: &Store, media_id: &str, error: &str) -> Result<()> {
    let media = store.get("media", media_id)?;
    let mut value = media["visualReview"].clone();
    if !value.is_object() {
        value = json!({});
    }
    if value["originalReview"].is_null() {
        value["originalReview"] = media["visualReview"].clone();
    }
    value["status"] = json!("manual_required");
    value["reason"] = json!(error);
    value["evidenceUnavailable"] = json!(true);
    value["modelCalls"] = json!(0);
    value["mediaId"] = json!(media_id);
    store.record_visual_review(media_id, &value)
}

fn pointer_file(store: &Store, media_id: Option<&str>) -> PathBuf {
    root(store).join("pointers").join(match media_id {
        Some(id) => format!("candidate-{}.json", digest(id)),
        None => "capability.json".into(),
    })
}
pub(super) fn pointer(store: &Store, media_id: Option<&str>) -> Result<Option<Value>> {
    let file = pointer_file(store, media_id);
    if !file.exists() {
        return Ok(None);
    }
    let mut value = read(&file)?;
    // At most the two distinct, proven local preparation defects can precede
    // a real call. Follow durable claims after a crash, never reserve anew.
    for _ in 0..4 {
        let original_path = pointer_path(store, &value)?;
        if value["purpose"] == "capability" {
            if let Some(next) = restart::successor(store, &value)? {
                value = next;
                continue;
            }
        }
        let claim = original_path.join("local-preparation-successor.json");
        if !claim.exists() {
            return Ok(Some(value));
        }
        let claim = read(&claim)?;
        let next = &claim["successor"];
        if claim["version"] != "visual-preparation-successor/v1"
            || claim["originalAttemptId"] != value["attemptId"]
            || claim["originalManifestSha256"] != value["manifestSha256"]
            || next["previous"] != value
        {
            return Err("VISUAL_PREPARATION_SUCCESSOR_CHANGED".into());
        }
        preparation_repair::validate_successor(store, &value, next)?;
        value = next.clone();
    }
    Err("VISUAL_PREPARATION_SUCCESSOR_LIMIT: 恢复链超出两种前置修复及一次显式新验证，保留原记录".into())
}
pub(super) fn pointer_path(store: &Store, pointer: &Value) -> Result<PathBuf> {
    let path = PathBuf::from(s(pointer, "attemptDirectory"));
    if pointer["version"] != "visual-attempt-pointer/v1"
        || path.parent() != Some(root(store).as_path())
        || !path.file_name().is_some_and(|v| {
            v.to_string_lossy()
                .starts_with(if pointer["purpose"] == "capability" {
                    "probe-"
                } else {
                    "candidate-"
                })
        })
        || hash_file(&path.join("manifest.json"))?.0 != s(pointer, "manifestSha256")
    {
        return Err(
            "VISUAL_ORIGINAL_ATTEMPT_CHANGED: 原审核指针或冻结材料不一致，不派发新请求".into(),
        );
    }
    let plan = read(&path.join("manifest.json"))?;
    if plan["attemptId"] != pointer["attemptId"]
        || plan["purpose"] != pointer["purpose"]
        || plan["basis"]["mediaId"] != pointer["mediaId"]
    {
        return Err("VISUAL_ORIGINAL_ATTEMPT_IDENTITY".into());
    }
    Ok(path)
}
pub(super) fn bind_pointer(
    store: &Store,
    path: &Path,
    replace_completed_probe: bool,
) -> Result<Value> {
    let plan = read(&path.join("manifest.json"))?;
    let media = if plan["purpose"] == "candidate" {
        Some(s(&plan["basis"], "mediaId"))
    } else {
        None
    };
    let previous = pointer(store, media)?;
    let mut preparation_recovery = false;
    if let Some(previous) = &previous {
        if previous["attemptId"] == plan["attemptId"] {
            return Ok(previous.clone());
        }
        if !replace_completed_probe || media.is_some() {
            return Err(
                "VISUAL_ORIGINAL_ATTEMPT_EXISTS: 先核对原审核结果，不能替换为新的调用".into(),
            );
        }
        if terminal(&pointer_path(store, previous)?).is_err() {
            if preparation_repair::verified_closure(store, previous)?.is_none() {
                return Err("VISUAL_REVIEW_UNKNOWN: 原验证尚未结案，不通过更换组件绕开".into());
            }
            preparation_recovery = true;
        }
    }
    let value = json!({"version":"visual-attempt-pointer/v1","purpose":plan["purpose"],"attemptId":plan["attemptId"],"attemptDirectory":path,
        "manifestSha256":hash_file(&path.join("manifest.json"))?.0,"mediaId":plan["basis"]["mediaId"],"assetId":plan["basis"]["assetId"],"scopeSha256":scope(&plan["baseIdentity"]),"createdAt":now(),"previous":previous});
    let file = pointer_file(store, media);
    fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    // This copy never changes, even when a completed capability is upgraded.
    atomic_json(&path.join("pointer.json"), &value)?;
    if preparation_recovery {
        let original = previous.as_ref().unwrap();
        preparation_repair::validate_successor(store, original, &value)?;
        let claim = pointer_path(store, original)?.join("local-preparation-successor.json");
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(claim)
            .map_err(|_| "VISUAL_PREPARATION_RECOVERY_ALREADY_RESERVED")?;
        output.write_all(json!({"version":"visual-preparation-successor/v1","originalAttemptId":original["attemptId"],"originalManifestSha256":original["manifestSha256"],"successor":value,"maximumSuccessors":1}).to_string().as_bytes()).map_err(|e|e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
    }
    if previous.is_some() {
        atomic_json(&file, &value)?;
    } else {
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&file)
            .map_err(|_| "VISUAL_ORIGINAL_ATTEMPT_EXISTS")?;
        output
            .write_all(value.to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
    }
    store.dirty()?;
    Ok(value)
}
pub fn reconcile(store: &Store, settings: &Value, request: &Value) -> Result<Value> {
    let media_id = request["mediaId"].as_str().filter(|v| !v.is_empty());
    let pointer = pointer(store, media_id)?.ok_or("没有可核对的原视觉审核，未创建新调用")?;
    let path = pointer_path(store, &pointer)?;
    let mut result = if media_id.is_some() {
        finish_candidate(store, &path)
    } else {
        finish_capability(store, settings, &path)
    };
    if result.is_err() && media_id.is_none() {
        if let Ok(closure) = preparation_repair::settle(store, &pointer) {
            result = Ok(closure);
        }
    }
    let mut value = match result {
        Ok(v) => v,
        Err(error) => {
            json!({"version":"visual-reconciliation/v1","status":if error.starts_with("VISUAL_REVIEW_UNKNOWN")||error.starts_with("VISUAL_IMAGE_"){"unknown"}else{"manual_required"},"reason":error,"assetId":pointer["assetId"],"mediaId":pointer["mediaId"],"attemptDirectory":path,"manifestSha256":pointer["manifestSha256"],"adopted":false})
        }
    };
    value["modelCalls"] = json!(0);
    value["reconciled"] = json!(true);
    if let Some(mid) = media_id {
        store.record_visual_review(mid, &value)?;
    } else {
        set_probe_status(
            store,
            if value["verified"] == true {
                "verified"
            } else if value["status"] == "unknown" {
                "unknown"
            } else if value["status"] == "not_submitted" {
                "not_submitted"
            } else {
                "manual_required"
            },
            s(&value, "reason"),
        )?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_original_pointer_cannot_be_changed_by_new_scope() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("p")).unwrap();
        store.create("recovery", "p").unwrap();
        let a = allocate(&store, "probe-old").unwrap();
        atomic_json(
            &a.join("manifest.json"),
            &json!({"attemptId":"original","purpose":"capability","baseIdentity":{},"basis":{}}),
        )
        .unwrap();
        let pointer = bind_pointer(&store, &a, false).unwrap();
        let b = allocate(&store, "probe-new").unwrap();
        atomic_json(&b.join("manifest.json"),&json!({"attemptId":"new","purpose":"capability","baseIdentity":{"supervisorSha256":"new"},"basis":{}})).unwrap();
        assert!(bind_pointer(&store, &b, true).is_err());
        assert_eq!(super::pointer(&store, None).unwrap().unwrap(), pointer);
        assert!(reconcile(&store, &json!({}), &json!({})).is_ok());
        assert!(!b.join("dispatch-intent.json").exists());
    }
    #[test]
    fn pointer_hash_change_is_not_repaired_or_redispatched() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("p")).unwrap();
        store.create("recovery", "p").unwrap();
        let a = allocate(&store, "candidate-original").unwrap();
        atomic_json(&a.join("manifest.json"),&json!({"attemptId":"original","purpose":"candidate","baseIdentity":{},"basis":{"mediaId":"m","assetId":"a"}})).unwrap();
        bind_pointer(&store, &a, false).unwrap();
        atomic_json(&a.join("manifest.json"), &json!({"attemptId":"new"})).unwrap();
        assert!(pointer(&store, Some("m")).is_err());
        assert!(!a.join("dispatch-intent.json").exists());
    }
}
