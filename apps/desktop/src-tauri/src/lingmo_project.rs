//! Import a finished document as a new project without an empty-project step.
use super::*;

impl Engine {
    pub(super) fn create_lingmo_project(&self, request: &Value) -> Result<Value> {
        let preview = crate::lingmo_import::validate_project_input(request)?;
        let identity = pipeline::digest(&format!(
            "lingmo-project/v1:{}:{}",
            s(&preview, "markdownSha256"),
            s(&preview, "previewToken")
        ));
        let key = format!("lingmo-{identity}");
        let projects = self.home.join("projects");
        let root = projects.join(&key);
        // This lock also protects against another client process. Do not call
        // register() while holding it: registration is the final atomic write.
        let _registry = self.registry_mutex.lock().map_err(|e| e.to_string())?;
        let _lease = crate::project_library::lock(&self.home)?;
        let mut registry = self.registry()?;
        let rows = registry.as_array().ok_or("项目索引损坏")?;
        if root.exists() {
            let parent = projects.canonicalize().map_err(|e| e.to_string())?;
            let actual = root.canonicalize().map_err(|e| e.to_string())?;
            if actual.parent() != Some(parent.as_path())
                || actual.file_name() != Some(std::ffi::OsStr::new(&key))
            {
                return Err("默认导入位置指向了项目目录外部，未读取或修改".into());
            }
        }
        let marker_path = root.join(".lingmo-create.json");
        let existing_marker = if marker_path.is_file() {
            Some(
                serde_json::from_slice::<Value>(
                    &fs::read(&marker_path).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        if let Some(existing) = rows.iter().find(|p| s(p, "id") == key) {
            if project_archived(existing) {
                return Err("这份分镜已导入的项目已归档，请在项目列表恢复；未重复创建".into());
            }
            let marker = existing_marker
                .as_ref()
                .ok_or("已有项目缺少导入创建回执，未覆盖或重复创建")?;
            if marker["identity"] != identity {
                return Err("项目创建回执身份不匹配，未覆盖已有项目".into());
            }
            let store = registered_store(existing)?;
            let mut report = if !s(request, "imageZipPath").is_empty()
                || !s(request, "imageDirectory").is_empty()
            {
                // A later global import can add optional images to the same
                // document. Text import is idempotent and preserves edits.
                crate::lingmo_import::import(&store, request)?
            } else {
                marker["report"].clone()
            };
            report["duplicate"] = json!(true);
            report["projectCreated"] = json!(false);
            report["projectReused"] = json!(true);
            if !s(request, "imageZipPath").is_empty() || !s(request, "imageDirectory").is_empty() {
                let mut receipt = marker.clone();
                receipt["report"] = report.clone();
                atomic_json(&marker_path, &receipt)?;
            }
            let mut workspace = self.workspace(&store, None)?;
            workspace["lingmoImportReport"] = report;
            return Ok(workspace);
        }
        let created = if root.exists() {
            let marker = existing_marker
                .as_ref()
                .ok_or("默认导入位置已有文件但缺少创建回执，未修改原文件")?;
            if marker["identity"] != identity || marker["state"] != "pending" {
                return Err(
                    "这份分镜的创建记录已结束或项目已删除，未恢复或重复创建；请从项目列表核对"
                        .into(),
                );
            }
            false
        } else {
            fs::create_dir_all(&projects).map_err(|e| e.to_string())?;
            fs::create_dir(&root).map_err(|e| e.to_string())?;
            true
        };
        let result = (|| -> Result<Value> {
            // A pre-existing pending directory is only resumed when its exact
            // document receipt matches. No existing project is ever initialized.
            let mut receipt = existing_marker.unwrap_or_else(||json!({"version":1,"identity":identity,"state":"pending","documentId":preview["markdownSha256"],"previewToken":preview["previewToken"]}));
            atomic_json(&marker_path, &receipt)?;
            let store = Store::open(root.clone())?;
            let count: i64 = store
                .conn()?
                .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if count == 0 {
                let title = s(request, "title").trim();
                let filename = s(&preview, "filename");
                let fallback = Path::new(filename)
                    .file_stem()
                    .and_then(|p| p.to_str())
                    .unwrap_or("导入分镜");
                store.create(if title.is_empty() { fallback } else { title }, &key)?;
            } else if s(&store.project()?, "id") != key {
                return Err("待恢复项目身份不匹配，未覆盖已有内容".into());
            }
            crate::executable_paths::apply(&self.home, &store)?;
            let mut report = crate::lingmo_import::import(&store, request)?;
            report["projectCreated"] = json!(true);
            report["projectReused"] = json!(!created);
            receipt["report"] = report.clone();
            atomic_json(&marker_path, &receipt)?;
            let mut workspace = self.workspace(&store, None)?;
            workspace["lingmoImportReport"] = report;
            let mut project = store.project()?;
            project["archivedAt"] = Value::Null;
            project["lifecycleVersion"] = json!(0);
            registry.as_array_mut().ok_or("项目索引损坏")?.push(project);
            atomic_json(&self.home.join("projects.json"), &registry)?;
            receipt["state"] = json!("committed");
            // Registry is the commit point. A marker update failure must not
            // report failure or delete a successfully registered project.
            let _ = atomic_json(&marker_path, &receipt);
            Ok(workspace)
        })();
        if result.is_err() && created {
            // Delete only this call's new, unregistered directory and only
            // after resolving both paths inside the intended project folder.
            let registered = self
                .registry()
                .ok()
                .and_then(|v| v.as_array().cloned())
                .is_none_or(|rows| rows.iter().any(|p| s(p, "id") == key));
            if !registered {
                if let (Ok(parent), Ok(target)) = (projects.canonicalize(), root.canonicalize()) {
                    if target.parent() == Some(parent.as_path())
                        && target.file_name() == Some(std::ffi::OsStr::new(&key))
                    {
                        let _ = fs::remove_dir_all(&target);
                    }
                }
            }
        }
        result
    }
}

#[cfg(test)]
#[path = "lingmo_project_tests.rs"]
mod tests;
