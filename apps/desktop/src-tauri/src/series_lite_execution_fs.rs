use super::*;
use std::io::Write;
use std::path::Component;
use std::sync::atomic::{AtomicU64, Ordering};
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct Job {
    pub root: PathBuf,
}
impl Job {
    pub fn new(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root).map_err(|_| err("workdir missing"))?;
        need(root.is_dir(), "workdir must be directory")?;
        Ok(Self { root })
    }
    pub fn path(&self, value: &str) -> Result<PathBuf> {
        need(
            !value.trim().is_empty()
                && !value.contains(['\0', ':'])
                && !value.starts_with(['/', '\\'])
                && !value.split(['/', '\\']).any(|p| p == "..")
                && Path::new(value)
                    .components()
                    .all(|p| matches!(p, Component::Normal(_) | Component::CurDir)),
            "invalid JOB relative path",
        )?;
        let path = self.root.join(value);
        let mut cursor = path.as_path();
        let mut tail = vec![];
        loop {
            match fs::symlink_metadata(cursor) {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tail.push(
                        cursor
                            .file_name()
                            .ok_or_else(|| err("invalid path"))?
                            .to_owned(),
                    );
                    cursor = cursor.parent().ok_or_else(|| err("invalid path"))?;
                }
                Err(_) => return Err(err("path inaccessible")),
            }
        }
        let mut resolved = fs::canonicalize(cursor).map_err(|_| err("unresolved link"))?;
        for part in tail.iter().rev() {
            resolved.push(part);
        }
        need(
            resolved != self.root && resolved.starts_with(&self.root),
            "path escapes JOB",
        )?;
        Ok(resolved)
    }
    pub fn relative(&self, path: &Path) -> Result<String> {
        path.strip_prefix(&self.root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .map_err(|_| err("path escapes JOB"))
    }
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let path = self.path(path)?;
        need(path.is_file(), "input is not regular file")?;
        fs::read(path).map_err(|_| err("file read failed"))
    }
    pub fn json(&self, path: &str) -> Result<Json> {
        parse(&self.read(path)?).map_err(|_| err("invalid exact JSON"))
    }
    pub fn record(&self, path: &str) -> Result<Json> {
        Ok(object([
            ("path", text(self.relative(&self.path(path)?)?)),
            ("sha256", text(sha256(&self.read(path)?))),
        ]))
    }
    pub fn record_matches(&self, record: &Json) -> Result<()> {
        fields(record, &["path", "sha256"], &[])?;
        need(
            self.record(s(record, "path")?)? == *record,
            "record file changed",
        )
    }
    pub fn ensure_dir(&self, path: &Path) -> Result<()> {
        let rel = self.relative(path)?;
        need(self.path(&rel)? == path, "directory redirected")?;
        fs::create_dir_all(path).map_err(|_| err("directory creation failed"))?;
        need(self.path(&rel)? == path, "created directory redirected")
    }
    pub fn atomic(&self, path: &Path, value: &Json) -> Result<()> {
        let parent = path.parent().ok_or_else(|| err("missing parent"))?;
        self.ensure_dir(parent)?;
        let rel = self.relative(path)?;
        need(self.path(&rel)? == path, "output redirected")?;
        let temp = parent.join(format!(
            ".execution-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|_| err("atomic temporary file unavailable"))?;
        let mut bytes = canonical(value);
        bytes.push(b'\n');
        let result = (|| {
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| err("atomic write failed"))?;
            drop(file);
            need(self.path(&rel)? == path, "output redirected before commit")?;
            fs::rename(&temp, path).map_err(|_| err("atomic replace failed"))
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    pub fn exclusive(&self, path: &Path, raw: &[u8]) -> Result<()> {
        self.ensure_dir(path.parent().ok_or_else(|| err("missing parent"))?)?;
        need(
            self.path(&self.relative(path)?)? == path,
            "output redirected",
        )?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|_| err("exclusive output exists or unavailable"))?;
        file.write_all(raw)
            .and_then(|_| file.sync_all())
            .map_err(|_| err("exclusive write failed"))
    }
    /// Publish immutable receipt bytes only after a complete flushed write.
    /// A same-directory hard link is atomic and fails if destination exists;
    /// unlike replacing rename it cannot clobber an earlier receipt. Unsupported
    /// filesystems fail closed instead of falling back to a partial final file.
    pub fn atomic_new(&self, path: &Path, raw: &[u8]) -> Result<()> {
        let parent = path.parent().ok_or_else(|| err("missing parent"))?;
        self.ensure_dir(parent)?;
        let relative = self.relative(path)?;
        need(self.path(&relative)? == path, "receipt redirected")?;
        let temp = parent.join(format!(
            ".execution-receipt-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        self.exclusive(&temp, raw)?;
        let result = (|| {
            need(
                self.path(&relative)? == path,
                "receipt redirected before publication",
            )?;
            fs::hard_link(&temp, path).map_err(|_| {
                err("atomic immutable receipt publication failed; original output retained")
            })
        })();
        let _ = fs::remove_file(temp);
        result
    }
    pub fn snapshot_absolute(&self, raw: &str) -> Result<PathBuf> {
        let path = fs::canonicalize(raw).map_err(|_| err("interrupted snapshot missing"))?;
        need(
            path.starts_with(&self.root) && path.is_file(),
            "interrupted snapshot escapes JOB",
        )?;
        Ok(path)
    }
    pub fn lock(&self, path: &Path) -> Result<fs::File> {
        need(self.path(&self.relative(path)?)? == path, "lock redirected")?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|_| err("execution lock unavailable"))?;
        file.try_lock()
            .map_err(|_| err("execution busy; original task retained"))?;
        Ok(file)
    }
}
