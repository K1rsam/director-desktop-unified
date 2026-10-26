use super::*;
use std::os::windows::ffi::OsStrExt;

fn root_with_database_length(base: &Path, tag: &str, length: usize) -> PathBuf {
    let mut root = base.join(tag);
    loop {
        let current = root.join("project.sqlite").as_os_str().encode_wide().count();
        if current == length {
            return root;
        }
        assert!(current + 1 < length, "fixture requires room for a path component");
        let remaining = length - current;
        root.push("x".repeat(if remaining > 122 { 120 } else { remaining - 1 }));
    }
}

#[test]
fn windows_long_project_paths_preserve_wal_readonly_backup_and_reopen() {
    let temp = tempfile::tempdir().unwrap();
    // 253 reproduces a normal main filename whose initial -journal reaches
    // 261. 1100 also exceeds the default SQLite Win32 VFS's UTF-8 path buffer.
    for length in [253, 1100] {
        let root = root_with_database_length(temp.path(), &format!("原路径-{length}"), length);
        let store = Store::open(root.clone()).expect("initialize at the original long path");
        assert_eq!(store.root, root, "public path must not be relocated or rewritten");
        store.create("保留项目", "long-path-project").unwrap();
        let asset = store.save_asset(json!({"id":"original-asset","name":"角色","variantId":"base"})).unwrap();
        let c = store.conn().unwrap();
        assert_eq!(c.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0)).unwrap(), "wal");
        assert_eq!(c.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(c.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0)).unwrap(), 5000);
        c.execute("INSERT INTO metadata(key,value) VALUES('long-path-evidence','original-raw')", []).unwrap();
        assert!(root.join("project.sqlite-wal").is_file());
        assert!(root.join("project.sqlite-shm").is_file());
        let reader = open_database(&root.join("project.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(reader.query_row("SELECT value FROM metadata WHERE key='long-path-evidence'", [], |r| r.get::<_, String>(0)).unwrap(), "original-raw");
        assert!(reader.execute("DELETE FROM metadata", []).is_err(), "read-only flags must survive the path adapter");
        assert_eq!(Store::chrome_image_blocker_scoped(&root, "long-path-project", "", true).unwrap(), None);

        let backup_root = root_with_database_length(temp.path(), &format!("备份原路径-{length}"), length);
        store.backup_db(&backup_root.join("project.sqlite")).unwrap();
        drop(reader);
        drop(c);
        drop(store);
        for reopened_root in [root, backup_root] {
            let reopened = Store::open(reopened_root.clone()).expect("reopen the same saved database");
            assert_eq!(reopened.root, reopened_root);
            assert_eq!(reopened.project().unwrap()["id"], "long-path-project");
            assert_eq!(reopened.get("assets", "original-asset").unwrap(), asset);
            let db = reopened.conn().unwrap();
            assert_eq!(db.query_row("SELECT value FROM metadata WHERE key='long-path-evidence'", [], |r| r.get::<_, String>(0)).unwrap(), "original-raw");
            assert_eq!(db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(), "ok");
            assert!(reopened.jobs().unwrap().is_empty());
        }
        let missing = temp.path().join(format!("missing-{length}"));
        fs::create_dir_all(&missing).unwrap();
        assert!(Store::chrome_image_blocker_scoped(&missing, "missing", "", true).is_err());
        assert!(!missing.join("project.sqlite").exists(), "read-only check must never create a database");
    }
}
