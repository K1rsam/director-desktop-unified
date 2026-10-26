//! Keep Win32 process cwd short; evidence and logical SDK input stay immutable.
use super::*;
use std::io::Write;

const BINDING: &str = "visual-execution-directory/v1";
const MARKER: &[u8] = b"isolated visual review\n";

fn directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("VISUAL_EXECUTION_DIRECTORY_LINK".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("VISUAL_EXECUTION_DIRECTORY_REPARSE".into());
        }
    }
    Ok(())
}

fn ordinary(path: &Path) -> Result<PathBuf> {
    crate::text_sdk::node_path_argument(&fs::canonicalize(path).map_err(|e| e.to_string())?)
}

fn expected(root: &Path, plan: &Value, sha: &str) -> Result<Value> {
    if sha.len() != 64
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
        || uuid::Uuid::parse_str(s(plan, "attemptId")).is_err()
        || hash_file(&root.join("manifest.json"))?.0 != sha
    {
        return Err("VISUAL_EXECUTION_DIRECTORY_IDENTITY".into());
    }
    directory(&root.join("work"))?;
    let logical = ordinary(&root.join("work"))?;
    let home = ordinary(Path::new(s(&plan["baseIdentity"], "credentialHome")))?;
    directory(&home)?;
    let physical = home.join(".director-visual-runtime").join(sha).join("work");
    if !physical.is_absolute() || physical.to_string_lossy().encode_utf16().count() >= 248 {
        return Err(
            "VISUAL_EXECUTION_DIRECTORY_TOO_LONG: 本机账户目录不能提供短隔离执行路径".into(),
        );
    }
    Ok(
        json!({"version":BINDING,"attemptId":plan["attemptId"],"manifestSha256":sha,
        "logicalDirectory":logical,"executionDirectory":physical}),
    )
}

fn create_record(file: &Path, value: &Value) -> Result<()> {
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .map_err(|_| "VISUAL_EXECUTION_DIRECTORY_RECORD_EXISTS")?;
    out.write_all(value.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    out.sync_all().map_err(|e| e.to_string())
}

fn verify_record(file: &Path, value: &Value) -> Result<()> {
    let meta = fs::symlink_metadata(file).map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() > 16384
        || read(file)? != *value
    {
        return Err("VISUAL_EXECUTION_DIRECTORY_BINDING_CHANGED".into());
    }
    Ok(())
}

pub(super) fn verify(root: &Path, plan: &Value, sha: &str) -> Result<PathBuf> {
    let value = expected(root, plan, sha)?;
    let work = PathBuf::from(s(&value, "executionDirectory"));
    let base = work.parent().ok_or("VISUAL_EXECUTION_DIRECTORY")?;
    directory(base.parent().ok_or("VISUAL_EXECUTION_DIRECTORY")?)?;
    directory(base)?;
    directory(&work)?;
    if ordinary(&work)? != work {
        return Err("VISUAL_EXECUTION_DIRECTORY_IDENTITY".into());
    }
    verify_record(&root.join("execution-directory.json"), &value)?;
    verify_record(&base.join("owner.json"), &value)?;
    let marker = work.join(".director-text-root");
    let meta = fs::symlink_metadata(&marker).map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || fs::read(marker).map_err(|e| e.to_string())? != MARKER
    {
        return Err("VISUAL_EXECUTION_DIRECTORY_MARKER_CHANGED".into());
    }
    Ok(work)
}

pub(super) fn prepare(root: &Path, plan: &Value, sha: &str) -> Result<PathBuf> {
    let value = expected(root, plan, sha)?;
    if root.join("execution-directory.json").exists() {
        return verify(root, plan, sha);
    }
    let work = PathBuf::from(s(&value, "executionDirectory"));
    let base = work.parent().ok_or("VISUAL_EXECUTION_DIRECTORY")?;
    let parent = base.parent().ok_or("VISUAL_EXECUTION_DIRECTORY")?;
    if !parent.exists() {
        fs::create_dir(parent).map_err(|e| e.to_string())?;
    }
    directory(parent)?;
    if base.exists() {
        // A crash after writing the owner but before writing the sidecar can
        // finish this same binding. No unowned directory is adopted or cleared.
        directory(base)?;
        verify_record(&base.join("owner.json"), &value)?;
        directory(&work)?;
    } else {
        fs::create_dir(base).map_err(|e| e.to_string())?;
        fs::create_dir(&work).map_err(|e| e.to_string())?;
        fs::write(work.join(".director-text-root"), MARKER).map_err(|e| e.to_string())?;
        create_record(&base.join("owner.json"), &value)?;
    }
    create_record(&root.join("execution-directory.json"), &value)?;
    verify(root, plan, sha)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf, Value, String) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir(&home).unwrap();
        let root = temp
            .path()
            .join("a".repeat(100))
            .join("b".repeat(120))
            .join("evidence");
        fs::create_dir_all(root.join("work")).unwrap();
        let plan = json!({"attemptId":id(),"baseIdentity":{"credentialHome":home}});
        fs::write(root.join("manifest.json"), plan.to_string()).unwrap();
        let sha = hash_file(&root.join("manifest.json")).unwrap().0;
        (temp, root, plan, sha)
    }
    #[test]
    fn short_execution_directory_is_bound_to_manifest_and_rejects_changed_owner() {
        let (_temp, root, plan, sha) = fixture();
        let before = fs::read(root.join("manifest.json")).unwrap();
        let work = prepare(&root, &plan, &sha).unwrap();
        assert_eq!(verify(&root, &plan, &sha).unwrap(), work);
        assert_eq!(prepare(&root, &plan, &sha).unwrap(), work);
        assert_eq!(fs::read(root.join("manifest.json")).unwrap(), before);
        let marker = work.parent().unwrap().join("owner.json");
        fs::write(marker, "{}").unwrap();
        assert!(verify(&root, &plan, &sha).is_err());
        assert!(prepare(&root, &plan, &sha).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn windows_long_evidence_directory_launches_hidden_process_only_from_bound_short_cwd() {
        use std::os::windows::process::CommandExt;
        let (_temp, root, plan, sha) = fixture();
        let logical = ordinary(&root.join("work")).unwrap();
        assert!(logical.to_string_lossy().encode_utf16().count() >= 260);
        let executable = std::env::current_exe().unwrap();
        // Rust's test-list mode exits before running tests and cannot call a
        // model, SDK, browser or network. It exercises real CreateProcessW.
        let fail = std::process::Command::new(&executable)
            .arg("--list")
            .current_dir(&logical)
            .creation_flags(0x08000000)
            .output()
            .unwrap_err();
        assert_eq!(fail.raw_os_error(), Some(267));
        let work = prepare(&root, &plan, &sha).unwrap();
        let success = std::process::Command::new(executable)
            .arg("--list")
            .current_dir(&work)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(success.status.success());
        assert!(!success.stdout.is_empty());
        assert_eq!(verify(&root, &plan, &sha).unwrap(), work);
    }
}
