use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, String>;
pub fn now() -> String {
    Utc::now().to_rfc3339()
}
pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
pub fn asset_deleted(value: &Value) -> bool {
    !s(value, "deletedAt").is_empty()
}
pub fn parse(v: String) -> Result<Value> {
    serde_json::from_str(&v).map_err(|e| e.to_string())
}
pub fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut n = 0;
    let mut b = [0u8; 65536];
    loop {
        let c = f.read(&mut b).map_err(|e| e.to_string())?;
        if c == 0 {
            break;
        }
        digest.update(&b[..c]);
        n += c as u64;
    }
    Ok((format!("{:x}", digest.finalize()), n))
}

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}
impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let this = Self { root };
        let mut c = this.conn()?;
        let tables:i64=c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        let version = if tables == 0 {
            None
        } else {
            let has_metadata:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='metadata')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !has_metadata {
                return Err("已有数据库没有项目schema_version；未进行初始化或覆盖".into());
            }
            Some(
                c.query_row(
                    "SELECT value FROM metadata WHERE key='schema_version'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or("已有数据库缺少schema_version；未进行初始化或覆盖")?,
            )
        };
        if let Some(version) = &version {
            if !["1", "2"].contains(&version.as_str()) {
                return Err(format!(
                    "不支持项目数据库schema_version={version}；未改写数据库"
                ));
            }
        }
        if version.as_deref() == Some("2") {
            let mut q = c
                .prepare("PRAGMA table_info(jobs)")
                .map_err(|e| e.to_string())?;
            let columns = q
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(|e| e.to_string())?
                .collect::<std::result::Result<HashSet<_>, _>>()
                .map_err(|e| e.to_string())?;
            if ["next_attempt_at", "query_attempts", "download_attempts"]
                .iter()
                .any(|column| !columns.contains(*column))
            {
                return Err("schema_version=2与实际任务结构不一致，未自动修改".into());
            }
            // Reopening v2 is read-only: no DDL, IMMEDIATE transaction, WAL setting, or version write.
        } else {
            c.execute_batch(include_str!("schema.sql"))
                .map_err(|e| format!("项目存储初始化失败（请保留原数据库并核对活动任务）: {e}"))?;
            let tx = c
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let mut q = tx
                .prepare("PRAGMA table_info(jobs)")
                .map_err(|e| e.to_string())?;
            let columns = q
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(|e| e.to_string())?
                .collect::<std::result::Result<HashSet<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(q);
            for column in ["next_attempt_at", "query_attempts", "download_attempts"] {
                if !columns.contains(column) {
                    tx.execute(
                        &format!("ALTER TABLE jobs ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0"),
                        [],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            tx.execute(
                "CREATE INDEX IF NOT EXISTS job_due_idx ON jobs(state,next_attempt_at,updated_at)",
                [],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE metadata SET value='2' WHERE key='schema_version'",
                [],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        fs::create_dir_all(this.root.join("media")).map_err(|e| e.to_string())?;
        fs::create_dir_all(this.root.join("thumbnails")).map_err(|e| e.to_string())?;
        Ok(this)
    }
    pub fn conn(&self) -> Result<Connection> {
        let c = Connection::open(self.root.join("project.sqlite")).map_err(|e| e.to_string())?;
        c.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        c.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;
        Ok(c)
    }
    pub fn create(&self, title: &str, project_id: &str) -> Result<()> {
        self.conn()?
            .execute(
                "INSERT INTO project VALUES(?1,?2,?3)",
                params![project_id, title, now()],
            )
            .map_err(|e| e.to_string())?;
        self.dirty()?;
        Ok(())
    }
    pub fn dirty(&self) -> Result<()> {
        self.conn()?.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now()]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn project(&self) -> Result<Value> {
        self.conn()?.query_row("SELECT id,title,created_at FROM project LIMIT 1",[],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"createdAt":r.get::<_,String>(2)?,"rootPath":self.root}))).map_err(|e|e.to_string())
    }
    pub fn values(&self, table: &str) -> Result<Vec<Value>> {
        if !["assets", "units", "media"].contains(&table) {
            return Err("Unsupported projection".into());
        }
        let c = self.conn()?;
        let mut q = c
            .prepare(&format!("SELECT data FROM {table} ORDER BY rowid"))
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            let v = parse(r.map_err(|e| e.to_string())?)?;
            if table == "media" {
                self.relocate_media(&v)
            } else {
                Ok(v)
            }
        })
        .collect::<Result<Vec<_>>>()
        .map(|values| {
            values
                .into_iter()
                .filter(|v| {
                    (table != "units" || v["retired"] != true)
                        && (table != "assets" || !asset_deleted(v))
                })
                .collect()
        })
    }
    pub fn deleted_assets(&self) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let mut query = c.prepare("SELECT data FROM assets WHERE COALESCE(json_extract(data,'$.deletedAt'),'')<>'' ORDER BY rowid DESC").map_err(|e|e.to_string())?;
        let values = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .map(|row| parse(row.map_err(|e| e.to_string())?))
            .collect();
        values
    }
    /// A deleted record remains readable for task snapshots and media history.
    /// Production uses this explicit guard instead of silently reviving it.
    pub fn active_asset(&self, key: &str) -> Result<Value> {
        let value = self.get("assets", key)?;
        if asset_deleted(&value) {
            return Err("资产已删除，请先恢复或在单元配图中替换资产".into());
        }
        Ok(value)
    }
    pub fn asset_usage(&self, key: &str) -> Result<Value> {
        let c = self.conn()?;
        asset_usage_conn(&c, key)
    }
    /// Hide an asset from new production while retaining every historical file,
    /// appearance version and cloud receipt. Restore is a new appearance version.
    pub fn set_asset_deleted(&self, key: &str, expected: i64, deleted: bool) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let usage = asset_usage_conn(&tx, key)?;
        if usage["version"].as_i64() != Some(expected) {
            return Err("资产已更新，请刷新后再操作".into());
        }
        let mut data = parse(
            tx.query_row("SELECT data FROM assets WHERE id=?1", [key], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?,
        )?;
        if asset_deleted(&data) == deleted {
            return Ok(data);
        }
        if deleted {
            data["deletedAt"] = json!(now());
        } else {
            data.as_object_mut()
                .ok_or("资产必须为对象")?
                .remove("deletedAt");
        }
        let version = expected + 1;
        data["version"] = json!(version);
        tx.execute(
            "UPDATE assets SET version=?2,data=?3 WHERE id=?1",
            params![key, version, data.to_string()],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO asset_versions VALUES(?1,?2,?3)",
            params![key, version, data.to_string()],
        )
        .map_err(|e| e.to_string())?;
        let mut affected = vec![key.to_owned()];
        affected.extend(
            usage["derivedAssets"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| s(a, "id").to_owned()),
        );
        let changed_at = now();
        for asset in affected {
            // Never touch a known task ID or an uncertain submission: recovery is
            // always querying that original cloud identity, even after deletion.
            tx.execute("UPDATE jobs SET state='stale',error=?3,updated_at=?2,next_attempt_at=0 WHERE (task_id IS NULL OR task_id='') AND state IN ('ready','queued','preparing','prepared','waiting_dependency') AND kind IN ('image','video') AND ((kind='image' AND entity_id=?1) OR EXISTS(SELECT 1 FROM json_each(jobs.snapshot,'$.references') r WHERE json_extract(r.value,'$.assetId')=?1) OR (kind='video' AND entity_id IN (SELECT unit_id FROM unit_refs WHERE asset_id=?1)))",params![asset,changed_at,if deleted {"引用资产已删除，请调整配图后重新准备"} else {"引用资产已恢复，请按当前版本重新准备"}]).map_err(|e|e.to_string())?;
        }
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(NULL,?1,?2,?3)",params![if deleted {"asset_deleted"} else {"asset_restored"},json!({"assetId":key,"version":version,"impact":usage,"historicalMediaPreserved":true,"submittedSnapshotsPreserved":true}).to_string(),changed_at]).map_err(|e|e.to_string())?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(data)
    }
    pub fn get(&self, table: &str, key: &str) -> Result<Value> {
        if !["assets", "units", "media"].contains(&table) {
            return Err("Unsupported object".into());
        }
        let raw = self
            .conn()?
            .query_row(
                &format!("SELECT data FROM {table} WHERE id=?1"),
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let value = raw
            .map(parse)
            .transpose()?
            .ok_or_else(|| format!("对象不存在: {key}"))?;
        if table == "media" {
            self.relocate_media(&value)
        } else {
            Ok(value)
        }
    }
    pub fn unit_summaries(&self) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let mut q=c.prepare("SELECT json_remove(data,'$.shots','$.plan','$.emotionPlan','$.performance','$.ruleSnapshots') FROM units WHERE COALESCE(json_extract(data,'$.retired'),0)<>1 ORDER BY rowid").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| parse(r.map_err(|e| e.to_string())?)).collect()
    }
    pub fn source(&self) -> Result<Value> {
        self.latest("sources")
    }
    pub fn foundation(&self) -> Result<Value> {
        self.latest("foundations")
    }
    /// Carry provenance across a proven parser-only migration. Every complete
    /// affected episode is revalidated against the current source and capacity;
    /// creative data and submitted snapshots are never edited in place.
    pub(crate) fn rebind_verified_source_projection(&self, unit_ids: &[Value], caps: &Value) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        let latest_workflow:Option<String>=tx.query_row("SELECT state FROM jobs WHERE kind='workflow' AND json_extract(snapshot,'$.stage')='storyboard.run' ORDER BY rowid DESC LIMIT 1",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if latest_workflow.as_deref().is_some_and(|state|state!="completed" && state!="failed" && state!="cancelled") {
            // The current coordinator still owns immutable guards. Its normal
            // completion/recovery must precede a provenance-only migration.
            return Ok(json!({"rebound":[],"blocked":[],"deferred":"storyboard_workflow_active"}));
        }
        let current: Option<String> = tx.query_row("SELECT data FROM sources ORDER BY version DESC LIMIT 1",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let Some(current) = current else { return Ok(json!({"rebound":[],"blocked":[]})); };
        let current = parse(current)?;
        let read_values = |sql: &str| -> Result<Vec<Value>> {
            let mut q=tx.prepare(sql).map_err(|e|e.to_string())?;
            let rows=q.query_map([],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?;
            rows.map(|r|parse(r.map_err(|e|e.to_string())?)).collect()
        };
        let selected: HashSet<_>=unit_ids.iter().filter_map(Value::as_str).collect();
        if selected.is_empty(){return Ok(json!({"rebound":[],"blocked":[]}));}
        let placeholders=vec!["?";selected.len()].join(",");
        let mut q=tx.prepare(&format!("SELECT data FROM units WHERE episode_id IN (SELECT episode_id FROM units WHERE id IN ({placeholders})) AND COALESCE(json_extract(data,'$.retired'),0)<>1 ORDER BY rowid")).map_err(|e|e.to_string())?;
        let units:Vec<Value>=q.query_map(rusqlite::params_from_iter(selected.iter()),|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.map(|r|parse(r.map_err(|e|e.to_string())?)).collect::<Result<_>>()?;
        drop(q);
        let foundation:Option<(i64,i64,String)>=tx.query_row("SELECT version,source_version,data FROM foundations ORDER BY version DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
        let mut old_versions:HashSet<i64>=units.iter().filter_map(|u|u["sourceVersion"].as_i64()).filter(|v|Some(*v)!=current["version"].as_i64()).collect();
        if let Some((_,v,_))=&foundation {if Some(*v)!=current["version"].as_i64(){old_versions.insert(*v);}}
        if old_versions.is_empty(){return Ok(json!({"rebound":[],"blocked":[],"foundationRebound":false}));}
        let source: crate::pipeline::LockedSource = serde_json::from_value(current.clone()).map_err(|e|e.to_string())?;
        crate::pipeline::validate_source(&source)?;
        let assets=read_values("SELECT data FROM assets WHERE COALESCE(json_extract(data,'$.deletedAt'),'')='' ORDER BY rowid")?;
        let mut q=tx.prepare(&format!("SELECT data FROM sources WHERE version IN ({}) ORDER BY version",vec!["?";old_versions.len()].join(","))).map_err(|e|e.to_string())?;
        let sources:Vec<Value>=q.query_map(rusqlite::params_from_iter(old_versions.iter()),|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.map(|r|parse(r.map_err(|e|e.to_string())?)).collect::<Result<_>>()?;
        drop(q);
        // Identical old versions share one canonical proof across all units.
        let compatible:HashSet<i64>=sources.iter().filter(|old|source_channel_metadata_only(old,&current)).filter_map(|old|old["version"].as_i64()).collect();
        let episodes: HashSet<_>=units.iter().filter(|u|selected.contains(s(u,"id"))).map(|u|s(u,"episodeId")).collect();
        let mut rebound=Vec::new();
        let mut blocked=Vec::new();
        for episode in episodes {
            let episode_units:Vec<_>=units.iter().filter(|u|s(u,"episodeId")==episode).cloned().collect();
            let stale:Vec<_>=episode_units.iter().filter(|u|u["sourceVersion"].as_i64().is_some_and(|v|Some(v)!=current["version"].as_i64())).collect();
            if stale.is_empty(){continue;}
            if stale.iter().any(|u|u["sourceVersion"].as_i64().is_none_or(|v|!compatible.contains(&v))) {
                blocked.push(json!({"episodeId":episode,"code":"SOURCE_REVISION_REQUIRED","message":"原文或集场已经变化，保留旧分镜，须修订关联集。"}));
                continue;
            }
            let draft=json!({"schemaVersion":"storyboard-draft/v2","units":episode_units});
            if let Err(error)=crate::pipeline::validate_storyboard(&source,&draft,&assets,caps){
                blocked.push(json!({"episodeId":episode,"code":"SOURCE_REVISION_REQUIRED","message":error}));
                continue;
            }
            let proof_hash=crate::pipeline::digest(&draft.to_string());
            for unit in stale {
                let old=sources.iter().find(|old|old["version"]==unit["sourceVersion"]).unwrap();
                let mut carried=unit.clone();
                carried["expectedVersion"]=unit["version"].clone();
                carried["sourceVersion"]=current["version"].clone();
                carried["sourceMetadataRebind"]=json!({"version":"validated-storyboard-source-projection/v1","previousSourceVersion":unit["sourceVersion"],"sourceVersion":current["version"],"sourceSha256":current["sha256"],"fromParser":old["parserVersion"],"toParser":current["parserVersion"],"validatedEpisodeDraftSha256":proof_hash,"creativeContentUnchanged":true});
                let carried=save_unit_tx(&tx,carried)?;
                rebound.push(json!({"unitId":carried["id"],"unitVersion":carried["version"],"sourceVersion":carried["sourceVersion"]}));
            }
        }
        let mut foundation_rebound=false;
        if let Some((version,source_version,raw))=foundation {
            let mut data=parse(raw)?;
            if let Some(old)=sources.iter().find(|old|old["version"]==source_version && compatible.contains(&source_version)) {
                if s(&data,"sourceSha256")==s(&current,"sha256") {
                    data["version"]=json!(version+1);
                    data["sourceVersion"]=current["version"].clone();
                    data["sourceMetadataRebind"]=json!({"version":"source-channel-metadata-rebind/v1","previousSourceVersion":source_version,"sourceVersion":current["version"],"sourceSha256":current["sha256"],"fromParser":old["parserVersion"],"toParser":current["parserVersion"],"rawTextAndCoordinatesUnchanged":true,"assetAndMediaVersionsUnchanged":true});
                    tx.execute("INSERT INTO foundations VALUES(?1,?2,?3,?4)",params![version+1,current["version"].as_i64(),data.to_string(),now()]).map_err(|e|e.to_string())?;
                    foundation_rebound=true;
                }
            }
        }
        let report=json!({"version":"validated-source-projection-rebind/v1","rebound":rebound,"blocked":blocked,"foundationRebound":foundation_rebound});
        if !rebound.is_empty() || foundation_rebound {
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES('','source_projection_rebound',?1,?2)",params![report.to_string(),now()]).map_err(|e|e.to_string())?;
            mark_changed(&tx)?;
        }
        tx.commit().map_err(|e|e.to_string())?;
        Ok(report)
    }
    pub fn foundation_choices(&self) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        let mut view = foundation_choices_conn(&tx)?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='userReviewRequirements'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let requirements = raw.map(parse).transpose()?.unwrap_or(json!([]));
        for contract in view["contracts"].as_array_mut().into_iter().flatten() {
            contract["requiresUserReview"] = json!(
                crate::pipeline::foundation_delivery::choice_requires_user_review(
                    contract,
                    &requirements
                )
            );
        }
        Ok(view)
    }
    pub fn foundation_automatic_decisions(
        &self,
        user_review_requirements: &Value,
    ) -> Result<Vec<Value>> {
        crate::pipeline::foundation_delivery::automatic_decisions(
            &self.foundation_choices()?,
            user_review_requirements,
        )
    }
    /// Ordinary AUTHOR interpretations are one frozen Q1 operation, not a series
    /// of invented user approvals. CAS also preserves the user's explicit list.
    pub fn reserve_foundation_auto_workflow(&self, snapshot: &Value) -> Result<String> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        assert_no_local_production(&tx)?;
        crate::workflow::verify_guard(&tx, &snapshot["guard"])?;
        let view = foundation_choices_conn(&tx)?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='userReviewRequirements'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let requirements = raw.map(parse).transpose()?.unwrap_or(json!([]));
        let current =
            crate::pipeline::foundation_delivery::automatic_decisions(&view, &requirements)?;
        if current.is_empty()
            || json!(current) != snapshot["input"]["automaticDecisions"]
            || snapshot["input"]["foundationPhase"] != "AUTO_INTERPRET"
            || snapshot["input"]["foundationDecisionPolicy"]
                != crate::pipeline::foundation_delivery::AUTO_INTERPRET_POLICY
            || snapshot["input"]["userReviewRequirements"] != requirements
            || snapshot["input"]["projectId"] != view["projectId"]
            || snapshot["input"]["sourceVersion"] != view["sourceVersion"]
        {
            return Err("自动制作解释或用户预先确认要求已更新；未发起模型".into());
        }
        let key = id();
        tx.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,created_at,updated_at) VALUES(?1,'workflow','foundation.run','local','queued','local',?2,?3,?3)",params![key,snapshot.to_string(),now()]).map_err(|e|e.to_string())?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(key)
    }
    /// AUTHOR confirmation is a short source/version/revision CAS. It grants no
    /// source mutation and does not start a model per click.
    pub fn confirm_foundation_choice(&self, request: &Value) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        assert_no_local_production(&tx)?;
        let view = foundation_choices_conn(&tx)?;
        check_choice_view_versions(&view, request)?;
        let record = view["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["id"] == request["contractId"])
            .ok_or("选择契约已更新，请刷新")?;
        if record["status"] != "PENDING" || record["revision"] != request["revision"] {
            return Err("选择已确认或契约版本已变化，请刷新".into());
        }
        let option = request.get("optionIndex");
        let custom = request.get("customAnswer");
        if option.is_some() == custom.is_some() {
            return Err("请选择一个方案或填写一个自定义制作解释".into());
        }
        let answer = if let Some(option) = option {
            let index = option
                .as_u64()
                .or_else(|| {
                    option
                        .as_str()
                        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
                        .and_then(|s| s.parse::<u64>().ok())
                })
                .ok_or("方案序号无效")?;
            record["options"]
                .as_array()
                .and_then(|options| usize::try_from(index).ok().and_then(|i| options.get(i)))
                .and_then(Value::as_str)
                .ok_or("方案不属于当前契约")?
                .to_owned()
        } else {
            custom
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("自定义制作解释不能为空")?
                .to_owned()
        };
        let mut confirmed = record.clone();
        confirmed["status"] = json!("CONFIRMED");
        confirmed["answer"] = json!(answer);
        confirmed["decidedAt"] = json!(now());
        let mut saved = read_foundation_choice_records(&tx)?;
        saved.retain(|c| c["id"] != confirmed["id"]);
        saved.push(confirmed.clone());
        tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![crate::pipeline::foundation_delivery::AUTHOR_CHOICES_VERSION,json!(saved).to_string()]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES('', 'foundation_author_confirmed',?1,?2)",params![confirmed.to_string(),now()]).map_err(|e|e.to_string())?;
        mark_changed(&tx)?;
        let result = foundation_choices_conn(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
    /// Reserve exactly the confirmed decision set with the publication guard in
    /// one transaction. A second apply/confirmation cannot race the queued Q1.
    pub fn reserve_foundation_choice_workflow(&self, snapshot: &Value) -> Result<String> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        assert_no_local_production(&tx)?;
        crate::workflow::verify_guard(&tx, &snapshot["guard"])?;
        let view = foundation_choices_conn(&tx)?;
        let current: Vec<_> = view["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["status"] == "CONFIRMED")
            .cloned()
            .collect();
        if current.is_empty()
            || json!(current) != snapshot["input"]["confirmedDecisions"]
            || snapshot["input"]["foundationPhase"] != "APPLY_DECISION"
        {
            return Err("已确认选择已更新，请刷新后采用；未发起模型".into());
        }
        let key = id();
        tx.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,created_at,updated_at) VALUES(?1,'workflow','foundation.run','local','queued','local',?2,?3,?3)",params![key,snapshot.to_string(),now()]).map_err(|e|e.to_string())?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(key)
    }
    fn latest(&self, table: &str) -> Result<Value> {
        let raw: Option<String> = self
            .conn()?
            .query_row(
                &format!("SELECT data FROM {table} ORDER BY version DESC LIMIT 1"),
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        raw.map(parse).transpose().map(|v| v.unwrap_or(Value::Null))
    }
    pub fn save_source(&self, mut source: Value) -> Result<Value> {
        self.save_source_inner(&mut source, None, false)
    }
    /// Compare the displayed source version in the same short write transaction
    /// that publishes recognition/decisions. An unchanged result is a no-op.
    pub fn save_source_if_current(&self, mut source: Value, expected: i64) -> Result<Value> {
        self.save_source_inner(&mut source, Some(expected), false)
    }
    pub fn save_reparsed_source_if_current(
        &self,
        mut source: Value,
        expected: i64,
    ) -> Result<Value> {
        self.save_source_inner(&mut source, Some(expected), true)
    }
    fn save_source_inner(
        &self,
        source: &mut Value,
        expected: Option<i64>,
        preserve_metadata_only: bool,
    ) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut previous_source = None;
        if let Some(expected) = expected {
            let current: Option<(i64, String)> = tx
                .query_row(
                    "SELECT version,data FROM sources ORDER BY version DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let (version, raw) = current.ok_or("请先导入原文")?;
            if expected != version {
                return Err("原文范围已更新，请刷新后再确认".into());
            }
            let latest = parse(raw)?;
            let mut candidate = source.clone();
            candidate["version"] = json!(version);
            if latest == candidate {
                return Ok(latest);
            }
            previous_source = Some(latest);
        }
        let version: i64 = tx
            .query_row("SELECT COALESCE(MAX(version),0)+1 FROM sources", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        source["version"] = json!(version);
        tx.execute(
            "INSERT INTO sources VALUES(?1,?2,?3,?4,?5)",
            params![
                version,
                s(&source, "text"),
                s(&source, "sha256"),
                source.to_string(),
                now()
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("UPDATE jobs SET state='stale',error='原文已更新',updated_at=?1,next_attempt_at=0 WHERE task_id IS NULL AND state IN ('ready','queued','preparing','waiting_dependency') AND kind IN ('video','image','text')",[now()]).map_err(|e|e.to_string())?;
        if preserve_metadata_only {
            if let Some(previous) = previous_source
                .as_ref()
                .filter(|previous| source_channel_metadata_only(previous, source))
            {
                let foundation: Option<(i64, i64, String)> = tx.query_row(
                    "SELECT version,source_version,data FROM foundations ORDER BY version DESC LIMIT 1", [],
                    |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|e|e.to_string())?;
                if let Some((foundation_version, source_version, raw)) = foundation {
                    let mut foundation = parse(raw)?;
                    if source_version == previous["version"].as_i64().unwrap_or(-1)
                        && s(&foundation, "sourceSha256") == s(source, "sha256")
                    {
                        foundation["version"] = json!(foundation_version + 1);
                        foundation["sourceVersion"] = json!(version);
                        foundation["sourceMetadataRebind"] = json!({"version":"source-channel-metadata-rebind/v1",
                            "previousSourceVersion":source_version,"sourceVersion":version,"sourceSha256":source["sha256"],
                            "fromParser":previous["parserVersion"],"toParser":source["parserVersion"],
                            "rawTextAndCoordinatesUnchanged":true,"assetAndMediaVersionsUnchanged":true});
                        tx.execute(
                            "INSERT INTO foundations VALUES(?1,?2,?3,?4)",
                            params![
                                foundation_version + 1,
                                version,
                                foundation.to_string(),
                                now()
                            ],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()?;
        Ok(source.clone())
    }
    pub fn save_foundation(&self, mut data: Value) -> Result<Value> {
        let source = self.source()?;
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let version: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(version),0)+1 FROM foundations",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        data["version"] = json!(version);
        data["sourceVersion"] = source["version"].clone();
        tx.execute(
            "INSERT INTO foundations VALUES(?1,?2,?3,?4)",
            params![
                version,
                source["version"].as_i64().ok_or("请先导入原文")?,
                data.to_string(),
                now()
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()?;
        Ok(data)
    }
    pub fn save_asset(&self, data: Value) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        let data = save_asset_tx(&tx, data)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(data)
    }
    pub fn save_asset_user(&self, mut data: Value) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let old: Option<String> = tx
            .query_row(
                "SELECT data FROM assets WHERE id=?1",
                [s(&data, "id")],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let old = old.map(parse).transpose()?;
        if let Some(old) = &old {
            if data["expectedVersion"].as_i64() != old["version"].as_i64() {
                return Err("资产已更新，请刷新后再保存编辑".into());
            }
            for key in ["variantId", "baseAssetId"] {
                if !s(old, key).is_empty() && old[key] != data[key] {
                    return Err(
                        "资产与形象身份不能直接改写；请新增正确形象并在单元配图中替换".into(),
                    );
                }
            }
            if old["sourceAssetId"] != data["sourceAssetId"]
                || old["sourceVariantId"] != data["sourceVariantId"]
            {
                // Old generated reference lists cannot survive a manual parent
                // correction. Their original version is retained in history.
                let fields = data.as_object_mut().ok_or("资产必须为对象")?;
                fields.remove("design");
                fields.remove("referenceAssetIds");
                fields.remove("referenceVariantIds");
                fields.insert("designStatus".into(), json!("needsDesign"));
            }
        }
        validate_user_asset(&tx, &data)?;
        let result = save_asset_tx(&tx, data)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
    pub fn save_unit(&self, data: Value) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        let data = save_unit_tx(&tx, data)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(data)
    }
    pub fn import_media(
        &self,
        path: &Path,
        kind: &str,
        asset_id: Option<&str>,
        variant: Option<&str>,
    ) -> Result<Value> {
        if !["image", "video", "audio"].contains(&kind) {
            return Err("未知媒体类型".into());
        }
        if !path.is_file() {
            return Err("媒体文件不存在".into());
        }
        let (hash, size) = hash_file(path)?;
        if size == 0 {
            return Err("媒体为空".into());
        }
        let existing: Option<String> = self
            .conn()?
            .query_row(
                "SELECT data FROM media WHERE sha256=?1 AND kind=?2 LIMIT 1",
                params![hash, kind],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(v) = existing {
            let m = parse(v)?;
            self.verify_media(&m)?;
            if let Some(a) = asset_id {
                self.link_media(s(&m, "id"), a, variant.unwrap_or("base"))?;
            }
            return self.relocate_media(&m);
        }
        let guessed = if kind == "image" {
            let reader = image::ImageReader::open(path)
                .map_err(|e| e.to_string())?
                .with_guessed_format()
                .map_err(|e| e.to_string())?;
            let format = reader.format().ok_or("图片格式无法识别")?;
            let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
            if (w as u64) * (h as u64) > 100_000_000 {
                return Err("图片超过1亿像素导入上限".into());
            }
            Some(format)
        } else {
            None
        };
        let ext = if let Some(format) = guessed {
            format.extensions_str()[0]
        } else {
            path.extension()
                .and_then(|e| e.to_str())
                .filter(|e| e.len() <= 10 && e.chars().all(|c| c.is_ascii_alphanumeric()))
                .unwrap_or("bin")
        };
        let rel = format!("media/{hash}.{ext}");
        let dst = self.root.join(&rel);
        if !dst.exists() {
            let temp = dst.with_extension(format!("{ext}.{}.partial", id()));
            fs::copy(path, &temp).map_err(|e| e.to_string())?;
            let (h, _) = hash_file(&temp)?;
            if h != hash {
                let _ = fs::remove_file(temp);
                return Err("媒体导入期间内容发生变化".into());
            }
            match fs::rename(&temp, &dst) {
                Ok(()) => {}
                Err(e) => {
                    if dst.is_file() && hash_file(&dst)?.0 == hash {
                        let _ = fs::remove_file(temp);
                    } else {
                        return Err(e.to_string());
                    }
                }
            }
        }
        if hash_file(&dst)?.0 != hash {
            return Err("项目媒体文件已损坏，未覆盖原文件".into());
        }
        let thumb = if kind == "image" {
            let t = self.root.join("thumbnails").join(format!("{hash}.webp"));
            if !t.exists() {
                image::open(&dst)
                    .map_err(|e| e.to_string())?
                    .thumbnail(640, 640)
                    .save(&t)
                    .map_err(|e| e.to_string())?;
            }
            Some(t)
        } else {
            None
        };
        let mut media = json!({"id":id(),"kind":kind,"relativePath":rel,"path":dst,"thumbnailRelativePath":if kind=="image"{Some(format!("thumbnails/{hash}.webp"))}else{None},"thumbnailPath":thumb,"sha256":hash,"size":size,"name":path.file_name().and_then(|v|v.to_str()),"assetId":asset_id,"variantId":variant,"createdAt":now()});
        if kind == "image" {
            let (w, h) = image::image_dimensions(&dst).map_err(|e| e.to_string())?;
            media["width"] = json!(w);
            media["height"] = json!(h);
            media["metadataVerified"] = json!(true);
            media["metadataSource"] = json!("image-container");
        } else {
            match crate::media_probe::probe(&dst, &self.settings()?) {
                Ok(metadata) => {
                    let valid_kind = metadata["streams"]
                        .as_array()
                        .map(|streams| {
                            streams.iter().any(|stream| {
                                if kind == "video" {
                                    s(stream, "kind") == "video"
                                        && stream["attachedPicture"] != true
                                        && metadata["width"]
                                            .as_u64()
                                            .map(|n| n > 0)
                                            .unwrap_or(false)
                                        && metadata["height"]
                                            .as_u64()
                                            .map(|n| n > 0)
                                            .unwrap_or(false)
                                } else {
                                    s(stream, "kind") == "audio"
                                }
                            })
                        })
                        .unwrap_or(false);
                    if valid_kind {
                        for (k, v) in metadata.as_object().ok_or("媒体探测没有返回对象")?
                        {
                            media[k] = v.clone();
                        }
                    } else {
                        media["metadataVerified"] = json!(false);
                        media["metadataError"] =
                            json!("实际媒体流与声明的音频/视频类型不匹配；原文件已保留");
                    }
                }
                Err(error) => {
                    media["metadataVerified"] = json!(false);
                    media["metadataError"] = json!(error);
                }
            }
        }
        self.conn()?
            .execute(
                "INSERT INTO media VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    s(&media, "id"),
                    kind,
                    rel,
                    hash,
                    size,
                    media.to_string(),
                    now()
                ],
            )
            .map_err(|e| e.to_string())?;
        if let Some(a) = asset_id {
            self.link_media(s(&media, "id"), a, variant.unwrap_or("base"))?;
        }
        self.dirty()?;
        Ok(media)
    }
    pub fn link_media(&self, mid: &str, aid: &str, variant: &str) -> Result<()> {
        self.conn()?
            .execute(
                "INSERT OR IGNORE INTO media_assets VALUES(?1,?2,?3)",
                params![mid, aid, variant],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Explicit user import: keep the immutable image in history and select it only
    /// if the exact appearance inspected before file import is still current.
    pub fn import_image_as_current(
        &self,
        path: &Path,
        asset_id: &str,
        variant: Option<&str>,
    ) -> Result<Value> {
        let asset = self.active_asset(asset_id)?;
        let variant = variant.unwrap_or_else(|| s(&asset, "variantId"));
        if variant != s(&asset, "variantId") {
            return Err("导入图片的形象与当前资产不一致".into());
        }
        // The image is not a backfill candidate until its user selection is committed.
        let media = self.import_media(path, "image", None, None)?;
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        set_current_image(
            &tx,
            asset_id,
            variant,
            s(&media, "id"),
            asset["version"].as_i64(),
            false,
        )?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(media)
    }
    pub fn adopt(&self, asset_id: &str, variant: &str, media_id: &str) -> Result<()> {
        self.adopt_batch(&[json!({"assetId":asset_id,"variantId":variant,"mediaId":media_id})])
    }
    pub fn adopt_batch(&self, selections: &[Value]) -> Result<()> {
        let mut seen = HashSet::new();
        for item in selections {
            if !seen.insert((s(item, "assetId"), s(item, "variantId"))) {
                return Err("同一形象不能在一个批次重复采用".into());
            }
            let a = self.active_asset(s(item, "assetId"))?;
            if s(&a, "variantId") != s(item, "variantId") {
                return Err("采用形象与资产不一致".into());
            }
            let m = self.get("media", s(item, "mediaId"))?;
            if s(&m, "kind") != "image" {
                return Err("资产只能采用图片".into());
            }
            self.verify_media(&m)?;
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut changed = false;
        for item in selections {
            let asset_id = s(item, "assetId");
            let variant = s(item, "variantId");
            let media_id = s(item, "mediaId");
            changed |= set_current_image(&tx, asset_id, variant, media_id, None, false)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        if changed {
            self.dirty()?;
        }
        Ok(())
    }
    /// Finalize all downloaded outputs and image selection atomically. Partial
    /// downloads never enter this method. The event marker makes selection once-only,
    /// so a recovered query cannot undo a later manual choice from history.
    pub fn complete_download(
        &self,
        key: &str,
        result: &Value,
        ordered_media: &[String],
    ) -> Result<()> {
        for mid in ordered_media {
            self.verify_media(&self.get("media", mid)?)?;
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (kind, entity, snapshot, task): (String, String, String, Option<String>) = tx
            .query_row(
                "SELECT kind,entity_id,snapshot,task_id FROM jobs WHERE id=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|e| e.to_string())?;
        if task.as_deref() != result["taskId"].as_str() {
            return Err("下载完成回执的任务身份与原任务不一致".into());
        }
        for mid in ordered_media {
            let linked: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM job_media WHERE job_id=?1 AND media_id=?2)",
                    params![key, mid],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !linked {
                return Err("下载完成结果没有可靠关联到原任务".into());
            }
            let media_kind: String = tx
                .query_row("SELECT kind FROM media WHERE id=?1", [mid], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if media_kind != kind {
                return Err("下载完成结果的媒体类型与原任务不一致".into());
            }
        }
        if ordered_media.is_empty() {
            return Err("下载完成结果不能为空".into());
        }
        if result["resultUrls"].as_array().map(Vec::len) != Some(ordered_media.len()) {
            return Err("下载完成结果数量与渠道结果不一致".into());
        }
        let snapshot = parse(snapshot)?;
        let processed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE job_id=?1 AND type='image_current_processed')",[key],|r|r.get(0)).map_err(|e|e.to_string())?;
        if kind == "image" && !processed {
            let asset_id = s(&snapshot, "assetId");
            let variant = s(&snapshot, "variantId");
            let version = snapshot["assetVersion"].as_i64();
            let current: Option<(String, i64, String)> = tx
                .query_row(
                    "SELECT variant_id,version,data FROM assets WHERE id=?1",
                    [asset_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let valid_identity = !asset_id.is_empty()
                && asset_id == entity
                && !variant.is_empty()
                && version.is_some();
            let valid_version = current
                .as_ref()
                .map(|(v, n, raw)| {
                    v == variant
                        && Some(*n) == version
                        && parse(raw.clone()).is_ok_and(|asset| !asset_deleted(&asset))
                })
                .unwrap_or(false);
            let linked: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM media_assets ma JOIN media m ON m.id=ma.media_id WHERE ma.media_id=?1 AND ma.asset_id=?2 AND ma.variant_id=?3 AND m.kind='image')",params![ordered_media[0],asset_id,variant],|r|r.get(0)).map_err(|e|e.to_string())?;
            let applied = valid_identity && valid_version && linked;
            let reason = if !valid_identity {
                "frozen_identity_missing_or_mismatched"
            } else if !valid_version {
                "appearance_version_changed"
            } else if !linked {
                "image_not_linked_to_frozen_appearance"
            } else {
                "all_outputs_downloaded"
            };
            if applied {
                set_current_image(&tx, asset_id, variant, &ordered_media[0], version, false)?;
            }
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'image_current_processed',?2,?3)",params![key,json!({"applied":applied,"reason":reason,"assetId":asset_id,"variantId":variant,"assetVersion":version,"mediaId":ordered_media[0],"historyPreserved":true}).to_string(),now()]).map_err(|e|e.to_string())?;
        }
        let fields = json!({"result":result});
        if update_state(&tx, key, "downloaded", &fields, None)? != 1 {
            return Err("原任务不存在".into());
        }
        write_job_fields(&tx, key, &fields)?;
        tx.execute(
            "INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'state',?2,?3)",
            params![
                key,
                json!({"state":"downloaded","allOutputsDownloaded":true}).to_string(),
                now()
            ],
        )
        .map_err(|e| e.to_string())?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    /// Backfill only missing current images. Existing choices always win. Results
    /// from incomplete jobs or a different frozen appearance version are ineligible.
    pub fn ensure_current_images(&self) -> Result<Value> {
        let missing: Vec<(String, String, i64)> = {
            let c = self.conn()?;
            let mut q=c.prepare("SELECT a.id,a.variant_id,a.version FROM assets a WHERE COALESCE(json_extract(a.data,'$.deletedAt'),'')='' AND NOT EXISTS(SELECT 1 FROM adoptions ad WHERE ad.asset_id=a.id AND ad.variant_id=a.variant_id) ORDER BY a.rowid").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(|e| e.to_string())?
                .collect::<std::result::Result<_, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        let mut prepared = Vec::new();
        let mut skipped = Vec::new();
        for (asset, variant, version) in missing {
            let candidates: Vec<Value> = {
                let c = self.conn()?;
                let mut q=c.prepare("SELECT m.data FROM media_assets ma JOIN media m ON m.id=ma.media_id WHERE ma.asset_id=?1 AND ma.variant_id=?2 AND m.kind='image' AND NOT EXISTS(SELECT 1 FROM job_media jm JOIN jobs j ON j.id=jm.job_id WHERE jm.media_id=m.id AND j.kind='image' AND (j.state!='downloaded' OR COALESCE(json_extract(j.snapshot,'$.assetId'),'')!=?1 OR COALESCE(json_extract(j.snapshot,'$.variantId'),'')!=?2 OR COALESCE(json_extract(j.snapshot,'$.assetVersion'),-1)!=?3 OR jm.media_id!=COALESCE(json_extract(j.result,'$.mediaIds[0]'),(SELECT media_id FROM job_media WHERE job_id=j.id ORDER BY rowid LIMIT 1)))) ORDER BY m.created_at DESC,ma.rowid DESC").map_err(|e|e.to_string())?;
                let rows = q
                    .query_map(params![asset, variant, version], |r| r.get::<_, String>(0))
                    .map_err(|e| e.to_string())?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                rows.into_iter().map(parse).collect::<Result<_>>()?
            };
            if let Some(media) = candidates
                .into_iter()
                .find(|m| self.verify_media(m).is_ok())
            {
                prepared.push((asset, variant, version, s(&media, "id").to_owned()));
            } else {
                skipped.push(json!({"assetId":asset,"variantId":variant,"reason":"no_valid_complete_history"}));
            }
        }
        if prepared.is_empty() {
            return Ok(json!({"selectedCount":0,"selections":[],"skipped":skipped}));
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut selected = Vec::new();
        for (asset, variant, version, mid) in prepared {
            let actual: Option<(String, i64)> = tx
                .query_row(
                    "SELECT variant_id,version FROM assets WHERE id=?1",
                    [&asset],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if actual.as_ref() != Some(&(variant.clone(), version)) {
                skipped.push(json!({"assetId":asset,"variantId":variant,"reason":"appearance_changed_during_backfill"}));
                continue;
            }
            if set_current_image(&tx, &asset, &variant, &mid, Some(version), true)? {
                selected.push(json!({"assetId":asset,"variantId":variant,"mediaId":mid}));
            }
        }
        if !selected.is_empty() {
            tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(NULL,'missing_current_images_selected',?1,?2)",params![json!({"selections":selected,"onlyMissing":true}).to_string(),now()]).map_err(|e|e.to_string())?;
            mark_changed(&tx)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(json!({"selectedCount":selected.len(),"selections":selected,"skipped":skipped}))
    }
    pub fn adopted(&self, asset: &str, variant: &str) -> Result<Option<String>> {
        self.conn()?
            .query_row(
                "SELECT media_id FROM adoptions WHERE asset_id=?1 AND variant_id=?2",
                params![asset, variant],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn settings(&self) -> Result<Value> {
        let mut data = json!({"textProvider":"codex","textModel":"gpt-6.1-sol","reasoning":"medium","ratio":"16:9","videoModel":"","imageModel":"","bhEndpoint":"https://api.7tai.cc/v1","textConcurrency":4,"episodeConcurrency":2,"videoConcurrency":1,"chromeConcurrency":1});
        let c = self.conn()?;
        let mut q = c
            .prepare("SELECT key,value FROM settings")
            .map_err(|e| e.to_string())?;
        for row in q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
        {
            let (k, v) = row.map_err(|e| e.to_string())?;
            data[k] = parse(v)?;
        }
        Ok(data)
    }
    pub fn save_settings(&self, data: &Value) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        for (k, v) in data.as_object().ok_or("设置必须为对象")? {
            if k.to_lowercase().contains("token") || k.to_lowercase().contains("key") {
                continue;
            }
            tx.execute("INSERT INTO settings VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![k,v.to_string()]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.dirty()?;
        Ok(())
    }
    pub fn add_job(
        &self,
        kind: &str,
        entity: &str,
        provider: &str,
        account: &str,
        snapshot: &Value,
        state: &str,
    ) -> Result<String> {
        let key = id();
        self.conn()?.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![key,kind,entity,provider,state,account,snapshot.to_string(),now()]).map_err(|e|e.to_string())?;
        self.dirty()?;
        Ok(key)
    }
    pub fn job(&self, key: &str) -> Result<Value> {
        let query = format!("SELECT {JOB_COLUMNS} FROM jobs AS j WHERE j.id=?1");
        let raw = self
            .conn()?
            .query_row(&query, [key], read_job)
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("任务不存在")?;
        finish_job(raw)
    }
    pub fn jobs(&self) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let query=format!("SELECT {JOB_COLUMNS} FROM jobs AS j ORDER BY j.created_at DESC,j.rowid DESC LIMIT 2000");
        let mut q = c.prepare(&query).map_err(|e| e.to_string())?;
        let rows = q.query_map([], read_job).map_err(|e| e.to_string())?;
        rows.map(|r| finish_job(r.map_err(|e| e.to_string())?))
            .collect()
    }
    /// Production task history stays independently inspectable; large local workflow history is bounded.
    pub fn job_summaries(&self) -> Result<Vec<Value>> {
        let c = self.conn()?;
        // Legacy projects may not have created checkpoint storage yet. Reading
        // summaries must not create tables or alter an original task record.
        let has_steps: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_steps')",[],|row|row.get(0)).map_err(|e|e.to_string())?;
        // Historical CLI failures remain untouched on disk. Project the same
        // evidence-based hold used before dispatch, including old hidden rows.
        let mut cli_unknown: std::collections::BTreeMap<String, String> =
            crate::text_executor::cli_unknown_jobs(self)?.into_iter().collect();
        if has_steps && !cli_unknown.is_empty() {
            let ids = json!(cli_unknown.keys().collect::<Vec<_>>()).to_string();
            let mut parents = c.prepare("SELECT workflow_id,text_job_id FROM workflow_steps WHERE text_job_id IN (SELECT value FROM json_each(?1)) ORDER BY rowid").map_err(|e|e.to_string())?;
            let links = parents.query_map([ids], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).map_err(|e|e.to_string())?;
            for link in links {
                let (parent, child) = link.map_err(|e|e.to_string())?;
                if let Some(reason) = cli_unknown.get(&child).cloned() {
                    cli_unknown.entry(parent).or_insert(reason);
                }
            }
        }
        let cli_ids = json!(cli_unknown.keys().collect::<Vec<_>>()).to_string();
        let unknown = if has_steps { JOB_SUMMARY_UNKNOWN_COLUMN } else { "j.state='submission_unknown'" };
        let query=format!("SELECT {JOB_SUMMARY_COLUMNS},{unknown} AS summary_submission_unknown FROM jobs AS j WHERE j.kind IN ('image','video') OR (j.kind IN ('workflow','text') AND (j.state IN ('queued','running','preparing','auth_required') OR ({unknown}) OR j.id IN (SELECT value FROM json_each(?1)) OR j.id IN (SELECT id FROM jobs WHERE kind IN ('workflow','text') ORDER BY created_at DESC,rowid DESC LIMIT 100))) ORDER BY j.created_at DESC,j.rowid DESC");
        let mut q = c.prepare(&query).map_err(|e| e.to_string())?;
        let rows = q.query_map([cli_ids], |row| Ok((read_job(row)?,row.get::<_,bool>(18)?))).map_err(|e| e.to_string())?;
        rows.map(|row| {
            let (raw,unknown) = row.map_err(|e|e.to_string())?;
            let mut job = finish_job(raw)?;
            job["submissionUnknown"] = json!(unknown);
            if let Some(reason) = cli_unknown.get(s(&job, "id")) {
                job["submissionUnknown"] = json!(true);
                job["cliSubmissionUnknown"] = json!(true);
                job["submissionUnknownReason"] = json!(reason);
            }
            Ok(job)
        }).collect()
    }
    /// The scheduler has no history cap; old active cloud identities remain recoverable.
    pub fn pending_jobs(&self) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let mut q=c.prepare("SELECT id,kind,entity_id,provider,state,account_id,task_id,next_attempt_at,updated_at,json_object('autoSubmit',json(CASE json_extract(snapshot,'$.autoSubmit') WHEN 1 THEN 'true' WHEN 0 THEN 'false' ELSE 'null' END),'assetVersion',json_extract(snapshot,'$.assetVersion')) FROM jobs WHERE kind IN ('image','video') AND state IN ('queued','ready','submitted','processing','succeeded','query_failed','download_failed','waiting_dependency') ORDER BY next_attempt_at,updated_at,rowid").map_err(|e|e.to_string())?;
        let rows=q.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"entityId":r.get::<_,String>(2)?,"provider":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?,"accountId":r.get::<_,String>(5)?,"providerTaskId":r.get::<_,Option<String>>(6)?,"nextAttemptAt":r.get::<_,i64>(7)?,"updatedAt":r.get::<_,String>(8)?,"snapshot":r.get::<_,String>(9)?}))).map_err(|e|e.to_string())?;
        rows.map(|r| {
            let mut job = r.map_err(|e| e.to_string())?;
            job["snapshot"] =
                serde_json::from_str(s(&job, "snapshot")).map_err(|e| e.to_string())?;
            Ok(job)
        })
        .collect()
    }
    pub fn update_job(&self, key: &str, state: &str, fields: &Value) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if update_state(&tx, key, state, fields, None)? == 0 {
            return Err("任务不存在".into());
        }
        write_job_fields(&tx, key, fields)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.event(
            key,
            "state",
            &json!({"state":state,"error":fields.get("error")}),
        )?;
        self.dirty()?;
        Ok(())
    }
    /// Atomically owns a state transition; a stale/uploading race cannot overwrite a newer state.
    pub fn claim_job(&self, key: &str, from: &str, to: &str) -> Result<bool> {
        self.claim_job_fields(key, from, to, &json!({}))
    }
    pub fn claim_job_fields(
        &self,
        key: &str,
        from: &str,
        to: &str,
        fields: &Value,
    ) -> Result<bool> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if update_state(&tx, key, to, fields, Some(from))? == 0 {
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(false);
        }
        write_job_fields(&tx, key, fields)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.event(key, "state_claim", &json!({"from":from,"state":to}))?;
        self.dirty()?;
        Ok(true)
    }
    /// Revalidate one frozen text response and its new workflow checkpoint in one
    /// transaction. The original model result and request columns are never written.
    pub(crate) fn commit_text_replay(
        &self,
        audit: &Value,
        fields: &Value,
        validated_result: &Value,
    ) -> Result<bool> {
        let receipt = &fields["receipt"];
        let accepted = match s(receipt, "validation") {
            "accepted" => true,
            "rejected" => false,
            _ => return Err("回放提交缺少明确的校验状态".into()),
        };
        let author_shot_replay =
            s(audit, "policy") == crate::text_executor::STORYBOARD_AUTHOR_SHOT_REPLAY_POLICY;
        let storyboard_repair = author_shot_replay
            || s(audit, "policy") == crate::text_executor::STORYBOARD_FINISH_COVERAGE_POLICY;
        if s(receipt, "contract") != "desktop-runner/v1"
            || s(audit, "policy").is_empty()
            || !(matches!(s(audit, "stage"), "foundation:F2" | "foundation:Q1")
                || storyboard_repair && matches!(s(audit, "stage"),"storyboardFinish"|"storyboardCreate"))
            || receipt["migrationPolicy"] != audit["policy"]
            || receipt["migrationAudit"] != *audit
            || accepted && receipt["acceptedRequestHash"] != audit["newRequestHash"]
            || [
                "originalJobId",
                "workflowId",
                "nodeKey",
                "previousError",
                "originalSnapshotJson",
                "originalResultJson",
                "oldRequestHash",
                "newRequestHash",
            ]
            .iter()
            .any(|key| s(audit, key).is_empty())
        {
            return Err("回放提交的冻结记录或审计不完整".into());
        }
        // Only this named Finisher regression can cross the foundation-only
        // transaction boundary. Its canonical input and original raw response
        // cannot be changed or inferred from a new validator result.
        let parent_entity = if author_shot_replay {
            // Independent policy: no extra errors or stages are granted to the
            // older source-coverage replay policy above.
            if !crate::text_executor::scoped_author_replay_audit_valid(audit)
                || !fields.as_object().is_some_and(|fields| fields.len() == 2
                    && fields.contains_key("error") && fields.contains_key("receipt"))
                || !receipt.as_object().is_some_and(|fields| fields.len() == if accepted { 8 } else { 7 })
                || receipt["revalidatedFrom"] != "rejected"
                || receipt["previousValidationError"] != audit["previousError"]
                || receipt["validatedResultSha256"] != crate::pipeline::digest(&validated_result.to_string())
                || accepted && !fields["error"].is_null()
                || !accepted && (s(fields, "error").is_empty()
                    || serde_json::from_str::<Value>(s(audit, "originalResultJson")).ok().as_ref() != Some(validated_result))
            {
                return Err("主创镜号回放的独立政策、原包证据或规范化结果绑定不匹配".into());
            }
            "storyboard.run"
        } else if storyboard_repair {
            let snapshot: Value = serde_json::from_str(s(audit, "originalSnapshotJson"))
                .map_err(|_| "整编回放的原执行快照无效")?;
            let original: Value = serde_json::from_str(s(audit, "originalResultJson"))
                .map_err(|_| "整编回放的原返回无效")?;
            let old_receipt: Value = serde_json::from_str(s(audit, "originalReceiptJson"))
                .map_err(|_| "整编回放的原回执无效")?;
            let raw_event: Value = serde_json::from_str(s(audit, "originalRawEventDataJson"))
                .map_err(|_| "整编回放的原始接收事件无效")?;
            if s(audit,"stage") != s(&snapshot,"stage")
                || !crate::text_executor::source_coverage_replay_stage(s(&snapshot,"stage"),s(&snapshot,"skill"),&snapshot["input"]["storyboardCreativeProfile"])
                || snapshot["executorIdentity"]["provider"] != "codex"
                || ![
                    crate::text_executor::STORYBOARD_FINISH_COVERAGE_REJECTION,
                    crate::text_executor::AUTHOR_SOURCE_COVERAGE_REJECTION,
                ]
                .contains(&s(audit, "previousError"))
                || s(audit, "oldRequestHash") != s(audit, "newRequestHash")
                || crate::pipeline::digest(&snapshot.to_string()) != s(audit, "oldRequestHash")
                || snapshot.to_string() != s(audit, "originalSnapshotJson")
                || crate::pipeline::digest(s(audit, "originalResultJson"))
                    != s(audit, "originalResultSha256")
                || !crate::text_executor::source_coverage_replay_receipt_valid(&old_receipt,s(audit,"originalSnapshotJson"),s(audit,"originalResultJson"),audit["originalRawEventId"].as_i64().unwrap_or(0),s(audit,"oldRequestHash"),s(audit,"workflowId"),s(audit,"nodeKey"))
                || audit["originalRawEventId"].as_i64().unwrap_or(0) <= 0
                || raw_event["requestHash"] != audit["oldRequestHash"]
                || raw_event["value"] != original
                || audit["generationRequestsSent"] != 0
                || audit["acceptedAutomatically"] != false
            {
                return Err("整编回放的政策、身份、原包哈希或接收证据不匹配".into());
            }
            "storyboard.run"
        } else {
            "foundation.run"
        };
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let eligible: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_steps w JOIN jobs j ON j.id=w.text_job_id JOIN jobs parent ON parent.id=w.workflow_id WHERE j.id=?1 AND w.workflow_id=?2 AND w.node_key=?3 AND w.stage=?8 AND w.state='rejected' AND w.request_hash=?4 AND w.error=?5 AND w.result=?6 AND parent.kind='workflow' AND parent.entity_id=?9 AND j.kind='text' AND j.provider='codex' AND j.state='rejected' AND j.error=?5 AND j.result=?6 AND j.snapshot=?7 AND json_extract(j.receipt,'$.validation')='rejected' AND json_extract(j.receipt,'$.contract')='desktop-runner/v1' AND (?10 IS NULL OR (j.receipt=?10 AND EXISTS(SELECT 1 FROM events e WHERE e.id=?11 AND e.job_id=j.id AND e.type='text_received_raw' AND e.data=?12))))",
            params![s(audit,"originalJobId"),s(audit,"workflowId"),s(audit,"nodeKey"),s(audit,"oldRequestHash"),s(audit,"previousError"),s(audit,"originalResultJson"),s(audit,"originalSnapshotJson"),s(audit,"stage"),parent_entity,storyboard_repair.then(||s(audit,"originalReceiptJson")),audit["originalRawEventId"].as_i64(),storyboard_repair.then(||s(audit,"originalRawEventDataJson"))],
            |row| row.get(0),
        ).map_err(|e|e.to_string())?;
        if !eligible {
            return Ok(false);
        }
        if author_shot_replay {
            // A newer raw event or workflow snapshot invalidates this proof even
            // when the previously audited event still exists in the database.
            let latest_bound: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM jobs parent JOIN events e ON e.job_id=?1 WHERE parent.id=?2 AND parent.snapshot=?3 AND e.id=?4 AND e.id=(SELECT MAX(x.id) FROM events x WHERE x.job_id=?1 AND x.type='text_received_raw') AND e.type='text_received_raw' AND e.data=?5)",
                params![s(audit,"originalJobId"),s(audit,"workflowId"),s(audit,"originalWorkflowSnapshotJson"),audit["originalRawEventId"].as_i64(),s(audit,"originalRawEventDataJson")], |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            if !latest_bound { return Ok(false); }
        }
        let job = s(audit, "originalJobId");
        let state = if accepted { "completed" } else { "rejected" };
        if update_state(&tx, job, state, fields, Some("rejected"))? != 1 {
            return Err("原文字任务状态已变化，回放事务未提交".into());
        }
        // A caller cannot replace raw result, snapshot, uploads or prepared data here.
        write_job_fields(&tx, job, &json!({"receipt":receipt}))?;
        let count=tx.execute(
            "UPDATE workflow_steps SET request_hash=?1,state=?2,result=?3,error=?4,updated_at=?5 WHERE workflow_id=?6 AND node_key=?7 AND text_job_id=?8 AND state='rejected' AND request_hash=?9",
            params![s(audit,"newRequestHash"),if accepted{"accepted"}else{"rejected"},validated_result.to_string(),fields.get("error").and_then(Value::as_str),now(),s(audit,"workflowId"),s(audit,"nodeKey"),job,s(audit,"oldRequestHash")],
        ).map_err(|e|e.to_string())?;
        if count != 1 {
            return Err("流程节点状态已变化，回放事务未提交".into());
        }
        let event = if author_shot_replay {
            if accepted { "storyboard_author_shot_validation_replay_accepted" }
            else { "storyboard_author_shot_validation_replay_rejected" }
        } else { match (s(audit, "stage"), accepted) {
            ("storyboardFinish", true) => "storyboard_finish_validation_replay_accepted",
            ("storyboardFinish", false) => "storyboard_finish_validation_replay_rejected",
            ("foundation:Q1", true) => "q1_validation_replay_accepted",
            ("foundation:Q1", false) => "q1_validation_replay_rejected",
            (_, true) => "catalogue_migration_accepted",
            (_, false) => "catalogue_migration_rejected",
        } };
        let mut event_data = json!({"requestHash":audit["newRequestHash"],"error":fields.get("error"),"validatedResult":validated_result,"originalEvidencePreserved":true});
        if storyboard_repair {
            event_data["policy"] = audit["policy"].clone();
            event_data["workflowId"] = audit["workflowId"].clone();
            event_data["nodeKey"] = audit["nodeKey"].clone();
            event_data["generationRequestsSent"] = json!(0);
        }
        tx.execute(
            "INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",
            params![job, event, event_data.to_string(), now()],
        )
        .map_err(|e| e.to_string())?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }
    /// A user declaration closes only an unresolved submission with no cloud task identity.
    /// Original snapshot, prepared request, receipt, result and error stay unchanged.
    pub fn close_unknown_job(&self, key: &str, confirmed: bool, reason: &str) -> Result<()> {
        let reason = reason.trim();
        if !confirmed || reason.is_empty() {
            return Err("请先核对渠道确实没有受理，并填写关闭原因".into());
        }
        if reason.chars().count() > 2000 {
            return Err("关闭原因最多2000字".into());
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let closed_at = now();
        let count = tx.execute("UPDATE jobs SET state='closed_by_user',updated_at=?2,next_attempt_at=0 WHERE id=?1 AND state='submission_unknown' AND (task_id IS NULL OR task_id='')",params![key,closed_at]).map_err(|e| e.to_string())?;
        if count != 1 {
            return Err("只能关闭尚未取得渠道任务ID的未知提交；状态可能已变化".into());
        }
        let declaration = json!({"confirmedNotAccepted":true,"reason":reason,"declaredBy":"user","closedAt":closed_at,"channelOperationPerformed":false,"originalEvidencePreserved":true});
        tx.execute("INSERT INTO events(job_id,type,data,created_at) VALUES(?1,'unknown_submission_closed_by_user',?2,?3)",params![key,declaration.to_string(),closed_at]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[closed_at]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Stop repeated recovery failures after five attempts; manual recovery never submits.
    pub fn retry_job(&self, key: &str, operation: &str, message: &str) -> Result<()> {
        let (column, state, base, cap) = match operation {
            "query" => ("query_attempts", "query_failed", 5_i64, 300_i64),
            "download" => ("download_attempts", "download_failed", 10, 600),
            _ => return Err("重试种类仅允许query或download；生成提交不能自动重试".into()),
        };
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let attempts: i64 = tx
            .query_row(
                &format!("SELECT {column} FROM jobs WHERE id=?1"),
                [key],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if attempts >= 4 {
            let message =
                format!("{message} 自动恢复已连续失败5次；请检查连接后在任务记录恢复原任务。");
            tx.execute(&format!("UPDATE jobs SET state='recovery_paused',error=?1,updated_at=?2,next_attempt_at=0,{column}={column}+1 WHERE id=?3"),params![message,now(),key]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            self.event(
                key,
                "retry_budget_exhausted",
                &json!({"operation":operation,"attempts":attempts+1,"error":message}),
            )?;
            self.dirty()?;
            return Ok(());
        }
        let delay = base
            .saturating_mul(
                1_i64
                    .checked_shl(attempts.max(0).min(30) as u32)
                    .unwrap_or(i64::MAX),
            )
            .min(cap);
        let next = Utc::now().timestamp().saturating_add(delay);
        tx.execute(&format!("UPDATE jobs SET state=?1,error=?2,updated_at=?3,next_attempt_at=?4,{column}={column}+1 WHERE id=?5"),params![state,message,now(),next,key]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.event(key,"retry_scheduled",&json!({"operation":operation,"delaySeconds":delay,"nextAttemptAt":next,"error":message}))?;
        self.dirty()?;
        Ok(())
    }
    /// Moving a project invalidates local prepared paths, never already-submitted cloud identities.
    pub fn invalidate_unsubmitted(&self, reason: &str) -> Result<()> {
        self.conn()?.execute("UPDATE jobs SET state='stale',prepared=NULL,error=?1,updated_at=?2,next_attempt_at=0 WHERE (task_id IS NULL OR task_id='') AND (receipt IS NULL OR receipt IN ('','null','{}')) AND kind IN ('image','video') AND state IN ('queued','preparing','ready','prepared','waiting_dependency')",params![reason,now()]).map_err(|e|e.to_string())?;
        self.dirty()?;
        Ok(())
    }
    /// Settings affect new Stage2 requests. Existing frozen image submissions keep
    /// their original parameters; moving a project still uses invalidate_unsubmitted.
    pub fn invalidate_unsubmitted_settings(&self, reason: &str) -> Result<()> {
        self.conn()?.execute("UPDATE jobs SET state='stale',prepared=NULL,error=?1,updated_at=?2,next_attempt_at=0 WHERE (task_id IS NULL OR task_id='') AND (receipt IS NULL OR receipt IN ('','null','{}')) AND (kind='video' OR (kind='image' AND json_type(snapshot,'$.frozenSettings') IS NULL)) AND state IN ('queued','preparing','ready','prepared','waiting_dependency')",params![reason,now()]).map_err(|e|e.to_string())?;
        self.dirty()?;
        Ok(())
    }
    pub fn event(&self, key: &str, kind: &str, data: &Value) -> Result<()> {
        self.conn()?
            .execute(
                "INSERT INTO events(job_id,type,data,created_at) VALUES(?1,?2,?3,?4)",
                params![key, kind, data.to_string(), now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn attach_result(&self, job: &str, media: &str) -> Result<()> {
        self.conn()?
            .execute(
                "INSERT OR IGNORE INTO job_media VALUES(?1,?2)",
                params![job, media],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Publish generated history and its owning job together. A backfill reader
    /// must never see a partial generated image masquerading as an unowned import.
    pub fn attach_download_result(&self, job: &str, media: &str) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (kind, asset, snapshot): (String, String, String) = tx
            .query_row(
                "SELECT kind,entity_id,snapshot FROM jobs WHERE id=?1",
                [job],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())?;
        let actual: String = tx
            .query_row("SELECT kind FROM media WHERE id=?1", [media], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if kind != actual {
            return Err("下载媒体类型与原任务不一致".into());
        }
        tx.execute(
            "INSERT OR IGNORE INTO job_media VALUES(?1,?2)",
            params![job, media],
        )
        .map_err(|e| e.to_string())?;
        if kind == "image" {
            let snapshot = parse(snapshot)?;
            let variant = if s(&snapshot, "variantId").is_empty() {
                "base"
            } else {
                s(&snapshot, "variantId")
            };
            tx.execute(
                "INSERT OR IGNORE INTO media_assets VALUES(?1,?2,?3)",
                params![media, asset, variant],
            )
            .map_err(|e| e.to_string())?;
        }
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn backup_db(&self, target: &Path) -> Result<()> {
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let source = self.conn()?;
        let mut dest = Connection::open(target).map_err(|e| e.to_string())?;
        rusqlite::backup::Backup::new(&source, &mut dest)
            .map_err(|e| e.to_string())?
            .run_to_completion(32, Duration::from_millis(5), None)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn recover_interrupted(&self) -> Result<()> {
        let c = self.conn()?;
        c.execute("UPDATE jobs SET state='interrupted',error='文字步骤中断或尚未校验；继续原流程时重新执行，原回包保留' WHERE kind='text' AND state IN ('submitting','received')",[]).map_err(|e|e.to_string())?;
        let steps:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_steps')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if steps {
            c.execute("UPDATE workflow_steps SET state='interrupted',error='文字步骤尚未完成校验',updated_at=?1 WHERE state IN ('running','received')",[now()]).map_err(|e|e.to_string())?;
        }
        c.execute("UPDATE jobs SET state='submission_unknown',error='提交期间客户端退出；禁止自动重发' WHERE state='submitting' AND task_id IS NULL",[]).map_err(|e|e.to_string())?;
        c.execute("UPDATE jobs SET state='queued',error='本地准备中断；可安全继续准备' WHERE state='preparing'",[]).map_err(|e|e.to_string())?;
        c.execute("UPDATE jobs SET state='interrupted',error='本地阶段流程已中断；已完成文本结果可复用，需明确继续' WHERE kind='workflow' AND state IN ('queued','running')",[]).map_err(|e|e.to_string())?;
        c.execute("UPDATE jobs SET state='submitted',error=NULL WHERE task_id IS NOT NULL AND state IN ('submitting','downloading')",[]).map_err(|e|e.to_string())?;
        // A known cloud task interrupted by local CLI shutdown did not lose
        // authentication. Recover query-only without touching submission or
        // any real authentication failure / unknown submission.
        c.execute("UPDATE jobs SET state='submitted',error=NULL,next_attempt_at=0,updated_at=?1 WHERE kind IN ('image','video') AND state='auth_required' AND task_id IS NOT NULL AND task_id<>'' AND error='客户端已停止本地 CLI 查询。'",[now()]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn absolute_media(&self, media: &Value) -> Result<PathBuf> {
        let rel = Path::new(s(media, "relativePath"));
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err("非法媒体路径".into());
        }
        Ok(self.root.join(rel))
    }
    pub fn verify_media(&self, media: &Value) -> Result<PathBuf> {
        let p = self.absolute_media(media)?;
        if hash_file(&p)?.0 != s(media, "sha256") {
            return Err(format!("媒体哈希不一致: {}", s(media, "id")));
        }
        Ok(p)
    }
    /// Absolute paths are a machine-local projection. The media identity and relative path stay fixed.
    pub fn relocate_media(&self, media: &Value) -> Result<Value> {
        let mut projected = media.clone();
        projected["path"] = json!(self.absolute_media(media)?);
        if s(media, "kind") == "image" {
            let hash = s(media, "sha256");
            if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("图片媒体摘要无效".into());
            }
            let relative = format!("thumbnails/{hash}.webp");
            let path = self.root.join(&relative);
            if !path.is_file() {
                let source = self.verify_media(media)?;
                let (w, h) = image::image_dimensions(&source).map_err(|e| e.to_string())?;
                if (w as u64) * (h as u64) > 100_000_000 {
                    return Err("缩略图源图片超过1亿像素上限".into());
                }
                fs::create_dir_all(self.root.join("thumbnails")).map_err(|e| e.to_string())?;
                let temp = self
                    .root
                    .join("thumbnails")
                    .join(format!("{hash}-{}.webp", id()));
                image::open(&source)
                    .map_err(|e| e.to_string())?
                    .thumbnail(640, 640)
                    .save(&temp)
                    .map_err(|e| e.to_string())?;
                if let Err(error) = fs::rename(&temp, &path) {
                    if path.is_file() {
                        let _ = fs::remove_file(temp);
                    } else {
                        return Err(error.to_string());
                    }
                }
            }
            projected["thumbnailRelativePath"] = json!(relative);
            projected["thumbnailPath"] = json!(path);
        } else {
            projected["thumbnailRelativePath"] = Value::Null;
            projected["thumbnailPath"] = Value::Null;
        }
        Ok(projected)
    }
    pub fn refresh_media_paths(&self) -> Result<()> {
        let media = self.values("media")?;
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        for value in media {
            tx.execute(
                "UPDATE media SET data=?1 WHERE id=?2",
                params![value.to_string(), s(&value, "id")],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn source_channel_metadata_only(previous: &Value, current: &Value) -> bool {
    let channel = s(previous,"parserVersion")=="source-parser/v2" && s(current,"parserVersion")=="source-parser/v3";
    let display = s(previous,"parserVersion")=="source-parser/v4" && s(current,"parserVersion")=="source-parser/v5";
    if (!channel && !display)
        || s(previous, "text") != s(current, "text")
        || s(previous, "sha256") != s(current, "sha256")
    {
        return false;
    }
    for source in [previous, current] {
        let Ok(source) = serde_json::from_value::<crate::pipeline::LockedSource>(source.clone())
        else {
            return false;
        };
        if crate::pipeline::validate_source(&source).is_err() {
            return false;
        }
    }
    let normalize = |source: &Value| {
        let mut source = source.clone();
        if let Some(map) = source.as_object_mut() {
            map.remove("version");
            map.remove("parserVersion");
        }
        for row in source["rows"].as_array_mut().into_iter().flatten() {
            if display {
                if let Some(row)=row.as_object_mut(){row.remove("dialogue");}
            } else if let Some(dialogue) = row["dialogue"].as_object_mut() {
                dialogue.remove("speaker");
                dialogue.remove("channel");
            }
        }
        source
    };
    normalize(previous) == normalize(current)
}

const JOB_COLUMNS:&str="j.id,j.kind,j.entity_id,j.provider,j.state,j.task_id,j.account_id,j.snapshot,j.prepared,j.receipt,j.result,j.error,j.created_at,j.updated_at,j.next_attempt_at,j.query_attempts,j.download_attempts,(SELECT json_group_array(media_id) FROM (SELECT media_id FROM job_media WHERE job_id=j.id ORDER BY rowid))";
const JOB_SUMMARY_UNKNOWN_COLUMN: &str = "j.state='submission_unknown' OR (j.kind='workflow' AND EXISTS(SELECT 1 FROM workflow_steps w JOIN jobs t ON t.id=w.text_job_id WHERE w.workflow_id=j.id AND t.kind='text' AND t.state='submission_unknown'))";
const JOB_SUMMARY_COLUMNS: &str = r#"j.id,j.kind,j.entity_id,j.provider,j.state,j.task_id,j.account_id,
CASE WHEN j.kind='workflow' THEN json_object('runnerVersion',json_extract(j.snapshot,'$.runnerVersion'),'stage',json_extract(j.snapshot,'$.stage'),'selection',json(COALESCE(json_extract(j.snapshot,'$.input.selection'),json_extract(j.snapshot,'$.selection'),'null')),'sourceVersion',json_extract(j.snapshot,'$.sourceVersion'),'input',json_object('source',json_object('version',json_extract(j.snapshot,'$.input.source.version'),'sha256',json_extract(j.snapshot,'$.input.source.sha256'))))
WHEN j.state='ready' THEN json_object('model',json_extract(j.snapshot,'$.model'),'unitVersion',json_extract(j.snapshot,'$.unitVersion'),'problems',json_extract(j.snapshot,'$.problems'),'provider',json_extract(j.snapshot,'$.provider'),'ratio',json_extract(j.snapshot,'$.ratio'),'resolution',json_extract(j.snapshot,'$.resolution'),'duration',json_extract(j.snapshot,'$.duration'),'prompt',json_extract(j.snapshot,'$.prompt'),'references',json_extract(j.snapshot,'$.references'))
ELSE json_object('model',json_extract(j.snapshot,'$.model'),'unitVersion',json_extract(j.snapshot,'$.unitVersion'),'problems',json_extract(j.snapshot,'$.problems')) END,
CASE WHEN j.prepared IS NULL THEN NULL ELSE json_object('estimatedCost',json_extract(j.prepared,'$.estimatedCost'),'capabilityVersion',json_extract(j.prepared,'$.capabilityVersion')) END,
NULL,
CASE WHEN j.result IS NULL THEN NULL WHEN j.kind='workflow' THEN json_object('schemaVersion',json_extract(j.result,'$.schemaVersion'),'status',json_extract(j.result,'$.status'),'blocked',json_extract(j.result,'$.blocked'),'imagePlan',(SELECT json_group_array(json_object('assetId',json_extract(item.value,'$.assetId'),'variantId',json_extract(item.value,'$.variantId'),'dependencies',json_extract(item.value,'$.dependencies'),'status',json_extract(item.value,'$.status'),'missingAdoption',json_extract(item.value,'$.missingAdoption'),'candidateCount',json_extract(item.value,'$.candidateCount'))) FROM json_each(j.result,'$.imagePlan') item))
ELSE json_object('status',json_extract(j.result,'$.status'),'actualCost',json_extract(j.result,'$.actualCost'),'resultUrls',json_extract(j.result,'$.resultUrls')) END,
j.error,j.created_at,j.updated_at,j.next_attempt_at,j.query_attempts,j.download_attempts,
(SELECT json_group_array(media_id) FROM (SELECT media_id FROM job_media WHERE job_id=j.id ORDER BY rowid))"#;
fn read_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let optional = |i| row.get::<_, Option<String>>(i);
    Ok(
        json!({"id":row.get::<_,String>(0)?,"kind":row.get::<_,String>(1)?,"entityId":row.get::<_,String>(2)?,"provider":row.get::<_,String>(3)?,"status":row.get::<_,String>(4)?,"providerTaskId":optional(5)?,"accountId":row.get::<_,String>(6)?,"snapshot":row.get::<_,String>(7)?,"prepared":optional(8)?,"receipt":optional(9)?,"result":optional(10)?,"error":optional(11)?,"createdAt":row.get::<_,String>(12)?,"updatedAt":row.get::<_,String>(13)?,"nextAttemptAt":row.get::<_,i64>(14)?,"queryAttempts":row.get::<_,i64>(15)?,"downloadAttempts":row.get::<_,i64>(16)?,"resultMediaIds":row.get::<_,String>(17)?}),
    )
}
fn finish_job(mut job: Value) -> Result<Value> {
    for key in [
        "snapshot",
        "prepared",
        "receipt",
        "result",
        "resultMediaIds",
    ] {
        if let Some(raw) = job[key].as_str() {
            job[key] =
                serde_json::from_str(raw).map_err(|e| format!("任务 {key} 数据损坏: {e}"))?;
        }
    }
    job["submissionUnknown"] = json!(s(&job, "status") == "submission_unknown");
    if job["result"].is_object()
        || !job["resultMediaIds"]
            .as_array()
            .map(|ids| ids.is_empty())
            .unwrap_or(true)
    {
        if !job["result"].is_object() {
            job["result"] = json!({});
        }
        job["result"]["mediaIds"] = job["resultMediaIds"].clone();
    }
    Ok(job)
}
pub(crate) fn mark_changed(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute("INSERT INTO metadata VALUES('changed_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now()]).map_err(|e|e.to_string())?;
    Ok(())
}
fn validate_user_asset(c: &Connection, data: &Value) -> Result<()> {
    if s(data, "name").trim().is_empty() {
        return Err("请填写资产名称".into());
    }
    let source: Option<String> = c
        .query_row(
            "SELECT data FROM sources ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let source = source.map(parse).transpose()?.unwrap_or(Value::Null);
    let strings = |value: &Value| -> Result<Vec<String>> {
        if value.is_null() {
            return Ok(vec![]);
        }
        value
            .as_array()
            .ok_or("适用范围必须是列表")?
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .ok_or("适用范围含有无效身份")
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(str::to_owned)
    };
    let episodes = strings(&data["applicability"]["episodes"])?;
    let scenes = strings(&data["applicability"]["scenes"])?;
    let source_scenes = source["scenes"].as_array().cloned().unwrap_or_default();
    if episodes.iter().any(|id| {
        !source["episodes"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| s(e, "id") == id)
    }) {
        return Err("资产适用集不属于当前原文，请刷新后重新选择".into());
    }
    if scenes.iter().any(|id| {
        !source_scenes.iter().any(|scene| {
            s(scene, "id") == id
                && (episodes.is_empty() || episodes.iter().any(|ep| ep == s(scene, "episodeId")))
        })
    }) {
        return Err("资产适用场不属于已选集，请重新选择".into());
    }
    match s(&data["scope"], "kind") {
        "GLOBAL"
            if !episodes.is_empty()
                || !scenes.is_empty()
                || data["scope"]["scenes"]
                    .as_array()
                    .is_some_and(|scenes| !scenes.is_empty()) =>
        {
            return Err("全部集场与指定适用范围不一致，请重新选择".into());
        }
        "SCENES" => {
            let scope = data["scope"]["scenes"]
                .as_array()
                .ok_or("指定集场缺少范围")?;
            if scope.is_empty() {
                return Err("请至少选择一个适用集场".into());
            }
            let mut actual = HashSet::new();
            for reference in scope {
                if !source_scenes.iter().any(|scene| {
                    scene["id"] == reference["sceneId"]
                        && scene["episodeId"] == reference["episodeId"]
                }) {
                    return Err("资产范围引用的集场已失效，请重新选择".into());
                }
                actual.insert(s(reference, "sceneId").to_owned());
            }
            let expected: HashSet<_> = if scenes.is_empty() {
                source_scenes
                    .iter()
                    .filter(|scene| episodes.iter().any(|ep| ep == s(scene, "episodeId")))
                    .map(|scene| s(scene, "id").to_owned())
                    .collect()
            } else {
                scenes.into_iter().collect()
            };
            if actual != expected {
                return Err("资产集场范围与适用范围不一致，请重新选择".into());
            }
        }
        _ => {}
    }
    let mut q = c.prepare("SELECT data FROM assets WHERE COALESCE(json_extract(data,'$.deletedAt'),'')='' ORDER BY rowid").map_err(|e|e.to_string())?;
    let mut assets = q
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .map(|row| parse(row.map_err(|e| e.to_string())?))
        .collect::<Result<Vec<_>>>()?;
    assets.retain(|asset| s(asset, "id") != s(data, "id"));
    assets.push(data.clone());
    let mut done = HashSet::new();
    let mut visiting = HashSet::new();
    let mut stack = vec![(s(data, "id").to_owned(), false)];
    while let Some((key, expanded)) = stack.pop() {
        if expanded {
            visiting.remove(&key);
            done.insert(key);
            continue;
        }
        if done.contains(&key) {
            continue;
        }
        if !visiting.insert(key.clone()) {
            return Err("图片来源形成循环，请选择正确来源或清除派生来源".into());
        }
        let value = assets
            .iter()
            .find(|a| s(a, "id") == key)
            .ok_or("来源资产已删除或不存在，请重新选择来源")?;
        let parent = crate::pipeline::resolve_asset_dependency(&assets, value)?;
        let mut dependencies = value["design"]["referenceAssetIds"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_str().ok_or("设计参考缺少稳定资产身份"))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(str::to_owned)?;
        if let Some(parent) = &parent {
            dependencies.push(parent);
        }
        stack.push((key, true));
        for dependency in dependencies {
            stack.push((dependency.to_owned(), false));
        }
    }
    Ok(())
}
fn asset_usage_conn(c: &Connection, key: &str) -> Result<Value> {
    let raw: String = c
        .query_row("SELECT data FROM assets WHERE id=?1", [key], |row| {
            row.get(0)
        })
        .map_err(|_| "资产不存在，请刷新目录")?;
    let asset = parse(raw)?;
    let mut q = c.prepare("SELECT data FROM assets WHERE COALESCE(json_extract(data,'$.deletedAt'),'')='' ORDER BY rowid").map_err(|e|e.to_string())?;
    let mut assets = q
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .map(|row| parse(row.map_err(|e| e.to_string())?))
        .collect::<Result<Vec<_>>>()?;
    if !assets.iter().any(|value| s(value, "id") == key) {
        assets.push(asset.clone());
    }
    let mut affected: HashSet<String> = HashSet::from([key.to_owned()]);
    loop {
        let before = affected.len();
        for value in &assets {
            let parent = crate::pipeline::resolve_asset_dependency(&assets, value)
                .ok()
                .flatten();
            let depends = parent
                .as_ref()
                .is_some_and(|parent| affected.contains(parent))
                || value["design"]["referenceAssetIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .any(|id| affected.contains(id));
            if depends {
                affected.insert(s(value, "id").to_owned());
            }
        }
        if affected.len() == before {
            break;
        }
    }
    let derived: Vec<_> = assets
        .iter()
        .filter(|a| s(a, "id") != key && affected.contains(s(a, "id")))
        .map(|a| json!({"id":a["id"],"name":a["name"],"version":a["version"]}))
        .collect();
    let affected_ids = json!(affected.iter().collect::<Vec<_>>()).to_string();
    let mut q = c.prepare("SELECT data FROM units WHERE COALESCE(json_extract(data,'$.retired'),0)<>1 AND EXISTS(SELECT 1 FROM unit_refs WHERE unit_id=units.id AND asset_id IN (SELECT value FROM json_each(?1))) ORDER BY rowid").map_err(|e|e.to_string())?;
    let units = q.query_map([affected_ids],|row|row.get::<_,String>(0)).map_err(|e|e.to_string())?.map(|row| {
        let unit = parse(row.map_err(|e|e.to_string())?)?;
        Ok(json!({"id":unit["id"],"title":unit["title"],"episodeId":unit["episodeId"],"sceneId":unit["sceneId"],"version":unit["version"]}))
    }).collect::<Result<Vec<_>>>()?;
    let mut q = c.prepare("SELECT id,kind,entity_id,state,task_id,snapshot FROM jobs WHERE kind IN ('image','video') ORDER BY rowid DESC").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut active = vec![];
    let mut submitted = vec![];
    for row in rows {
        let (id, kind, entity, state, task, snapshot) = row.map_err(|e| e.to_string())?;
        let snapshot = parse(snapshot)?;
        let matches = (kind == "image" && affected.contains(&entity))
            || units.iter().any(|unit| s(unit, "id") == entity)
            || snapshot["references"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|r| affected.contains(s(r, "assetId")));
        if !matches {
            continue;
        }
        let record = json!({"id":id,"kind":kind,"status":state,"providerTaskId":task});
        if task.as_ref().is_some_and(|task| !task.is_empty()) || state == "submission_unknown" {
            submitted.push(record);
        } else if [
            "ready",
            "queued",
            "preparing",
            "prepared",
            "waiting_dependency",
        ]
        .contains(&state.as_str())
        {
            active.push(record);
        }
    }
    let image_count: i64 = c.query_row("SELECT COUNT(DISTINCT ma.media_id) FROM media_assets ma JOIN media m ON m.id=ma.media_id WHERE ma.asset_id=?1 AND m.kind='image'",[key],|row|row.get(0)).map_err(|e|e.to_string())?;
    Ok(
        json!({"assetId":key,"version":asset["version"],"name":asset["name"],"deletedAt":asset["deletedAt"],"units":units,"derivedAssets":derived,"activeJobs":active,"submittedJobs":submitted,"imageCount":image_count}),
    )
}
pub(crate) fn save_asset_tx(tx: &rusqlite::Transaction<'_>, mut data: Value) -> Result<Value> {
    if s(&data, "id").is_empty() {
        data["id"] = json!(id());
    }
    if s(&data, "variantId").is_empty() {
        data["variantId"] = json!("base");
    }
    let raw: Option<String> = tx
        .query_row(
            "SELECT data FROM assets WHERE id=?1",
            [s(&data, "id")],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let old = raw.map(parse).transpose()?;
    let version = old.as_ref().and_then(|a| a["version"].as_i64());
    if old.as_ref().is_some_and(asset_deleted) {
        return Err("资产已删除，请先恢复后再编辑；原图片和任务记录仍保留".into());
    }
    if asset_deleted(&data) {
        return Err("请使用删除资产操作，不要通过编辑改写删除状态".into());
    }
    if let Some(expected) = data.get("expectedVersion").and_then(Value::as_i64) {
        if version != Some(expected) {
            return Err("资产已被更新，请刷新后再保存".into());
        }
    }
    data.as_object_mut()
        .ok_or("资产必须为对象")?
        .remove("expectedVersion");
    if old.as_ref().is_some_and(|v| same_content(v, &data)) {
        return Ok(old.unwrap());
    }
    let v = version.unwrap_or(0) + 1;
    data["version"] = json!(v);
    tx.execute("INSERT INTO assets VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET variant_id=excluded.variant_id,version=excluded.version,data=excluded.data",params![s(&data,"id"),s(&data,"variantId"),v,data.to_string()]).map_err(|e|e.to_string())?;
    tx.execute(
        "INSERT INTO asset_versions VALUES(?1,?2,?3)",
        params![s(&data, "id"), v, data.to_string()],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("UPDATE jobs SET state='stale',error='资产版本已更新',updated_at=?2,next_attempt_at=0 WHERE task_id IS NULL AND state IN ('ready','queued','preparing','waiting_dependency') AND kind IN ('image','video') AND ((kind='image' AND entity_id=?1) OR EXISTS(SELECT 1 FROM json_each(jobs.snapshot,'$.references') AS r WHERE json_extract(r.value,'$.assetId')=?1 OR EXISTS(SELECT 1 FROM json_each(r.value,'$.semanticBindings') b WHERE json_extract(b.value,'$.assetId')=?1)) OR EXISTS(SELECT 1 FROM json_each(jobs.snapshot,'$.inlineReferences') r WHERE json_extract(r.value,'$.assetId')=?1))",params![s(&data,"id"),now()]).map_err(|e|e.to_string())?;
    mark_changed(tx)?;
    Ok(data)
}
pub(crate) fn save_unit_tx(tx: &rusqlite::Transaction<'_>, mut data: Value) -> Result<Value> {
    if s(&data, "id").is_empty() {
        data["id"] = json!(id());
    }
    if s(&data, "episodeId").is_empty() {
        return Err("分镜需要明确episodeId".into());
    }
    let raw: Option<String> = tx
        .query_row(
            "SELECT data FROM units WHERE id=?1",
            [s(&data, "id")],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let old = raw.map(parse).transpose()?;
    let version = old.as_ref().and_then(|u| u["version"].as_i64());
    for reference in data["assetRefs"].as_array().into_iter().flatten() {
        let asset_id = s(reference, "assetId");
        // An unchanged invalid reference may remain visible while the user fixes
        // another binding. It must still fail preparation, never silently vanish.
        let unchanged = old
            .as_ref()
            .and_then(|unit| unit["assetRefs"].as_array())
            .is_some_and(|refs| refs.contains(reference));
        if unchanged {
            continue;
        }
        let raw: Option<String> = tx
            .query_row("SELECT data FROM assets WHERE id=?1", [asset_id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        let asset = raw
            .map(parse)
            .transpose()?
            .ok_or("新配图引用的资产不存在，请重新选择")?;
        if asset_deleted(&asset) {
            return Err("新配图不能引用已删除资产，请恢复或选择正确资产".into());
        }
        if !s(reference, "variantId").is_empty()
            && s(reference, "variantId") != s(&asset, "variantId")
        {
            return Err("新配图的形象身份与资产不一致，请重新选择".into());
        }
    }
    if let Some(expected) = data.get("expectedVersion").and_then(Value::as_i64) {
        if version != Some(expected) {
            return Err("分镜已被更新，请刷新后再保存".into());
        }
    }
    let fields = data.as_object_mut().ok_or("单元必须为对象")?;
    fields.remove("expectedVersion");
    fields.remove("preparedPackage");
    if old.as_ref().is_some_and(|v| same_content(v, &data)) {
        return Ok(old.unwrap());
    }
    let v = version.unwrap_or(0) + 1;
    data["version"] = json!(v);
    tx.execute("INSERT INTO units VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET episode_id=excluded.episode_id,version=excluded.version,data=excluded.data",params![s(&data,"id"),s(&data,"episodeId"),v,data.to_string()]).map_err(|e|e.to_string())?;
    tx.execute(
        "INSERT INTO unit_versions VALUES(?1,?2,?3)",
        params![s(&data, "id"), v, data.to_string()],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM unit_refs WHERE unit_id=?1", [s(&data, "id")])
        .map_err(|e| e.to_string())?;
    if let Some(refs) = data["assetRefs"].as_array() {
        for (i, r) in refs.iter().enumerate() {
            tx.execute(
                "INSERT INTO unit_refs VALUES(?1,?2,?3,?4,?5)",
                params![
                    s(&data, "id"),
                    s(r, "assetId"),
                    s(r, "variantId"),
                    i as i64,
                    r.to_string()
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    tx.execute("UPDATE jobs SET state='stale',error='单元版本已更新',updated_at=?2,next_attempt_at=0 WHERE task_id IS NULL AND entity_id=?1 AND kind='video' AND state IN ('ready','queued','preparing','waiting_dependency')",params![s(&data,"id"),now()]).map_err(|e|e.to_string())?;
    mark_changed(tx)?;
    Ok(data)
}
fn read_foundation_choice_records(c: &Connection) -> Result<Vec<Value>> {
    let raw: Option<String> = c
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [crate::pipeline::foundation_delivery::AUTHOR_CHOICES_VERSION],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|e| format!("已确认制作选择记录无效，未自动覆盖：{e}"))
    })
    .transpose()
    .map(|v| v.unwrap_or_default())
}
fn foundation_choices_conn(c: &Connection) -> Result<Value> {
    let project: String = c
        .query_row("SELECT id FROM project LIMIT 1", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let source: Option<String> = c
        .query_row(
            "SELECT data FROM sources ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let foundation: Option<String> = c
        .query_row(
            "SELECT data FROM foundations ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let source = source.map(parse).transpose()?.unwrap_or(Value::Null);
    let foundation = foundation.map(parse).transpose()?.unwrap_or(Value::Null);
    if source.is_null() || foundation.is_null() {
        return Ok(
            json!({"projectId":project,"sourceVersion":source["version"].as_i64().unwrap_or(0),"sourceSha256":source["sha256"],"foundationVersion":foundation["version"].as_i64().unwrap_or(0),"contracts":[]}),
        );
    }
    crate::pipeline::foundation_delivery::choice_contracts(
        &project,
        &source,
        &foundation,
        &read_foundation_choice_records(c)?,
    )
}
fn assert_no_local_production(c: &Connection) -> Result<()> {
    let active:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE (kind='workflow' AND state IN ('queued','running')) OR (kind='text' AND state IN ('queued','running','submitting','received')))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if active {
        Err("本项目的文字制作流程正在执行，请完成或中断后确认制作选择".into())
    } else {
        Ok(())
    }
}
fn check_choice_view_versions(view: &Value, request: &Value) -> Result<()> {
    if request["projectId"] != view["projectId"]
        || request["expectedSourceVersion"].as_i64() != view["sourceVersion"].as_i64()
        || request["expectedFoundationVersion"].as_i64() != view["foundationVersion"].as_i64()
    {
        return Err("项目、原文或制作基础版本已变化，请刷新选择后再确认".into());
    }
    Ok(())
}
pub(crate) fn same_content(a: &Value, b: &Value) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    for value in [&mut a, &mut b] {
        if let Some(fields) = value.as_object_mut() {
            fields.remove("version");
            fields.remove("expectedVersion");
            fields.remove("preparedPackage");
        }
    }
    a == b
}
/// One selection primitive serves manual history, user import, backfill and
/// completed image jobs; every path has the same precise preparation invalidation.
fn set_current_image(
    tx: &rusqlite::Transaction<'_>,
    asset: &str,
    variant: &str,
    media: &str,
    expected_version: Option<i64>,
    only_missing: bool,
) -> Result<bool> {
    let (actual_variant, actual_version, raw): (String, i64, String) = tx
        .query_row(
            "SELECT variant_id,version,data FROM assets WHERE id=?1",
            [asset],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    if asset_deleted(&parse(raw)?) {
        return Err("资产已删除；原图已保留在历史，恢复后才可切换当前图片".into());
    }
    if actual_variant != variant
        || expected_version
            .map(|n| n != actual_version)
            .unwrap_or(false)
    {
        return Err("切换图片期间形象版本已经变化，请刷新；原图已保留在历史".into());
    }
    let kind: String = tx
        .query_row("SELECT kind FROM media WHERE id=?1", [media], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if kind != "image" {
        return Err("当前图片只能选择图片媒体".into());
    }
    let current: Option<String> = tx
        .query_row(
            "SELECT media_id FROM adoptions WHERE asset_id=?1 AND variant_id=?2",
            params![asset, variant],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if current.as_deref() == Some(media) || (only_missing && current.is_some()) {
        return Ok(false);
    }
    let time = now();
    tx.execute(
        "INSERT OR IGNORE INTO media_assets VALUES(?1,?2,?3)",
        params![media, asset, variant],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO adoptions VALUES(?1,?2,?3,?4) ON CONFLICT(asset_id,variant_id) DO UPDATE SET media_id=excluded.media_id,created_at=excluded.created_at",params![asset,variant,media,time]).map_err(|e|e.to_string())?;
    // First adoption satisfies unfrozen dependencies. Only prepared references to
    // the replaced asset/variant/media expire; submitted snapshots stay immutable.
    tx.execute("UPDATE jobs SET state='stale',error='当前图片已替换',updated_at=?4,next_attempt_at=0 WHERE task_id IS NULL AND state IN ('ready','queued','preparing') AND kind IN ('image','video') AND (EXISTS(SELECT 1 FROM json_each(jobs.snapshot,'$.references') AS r WHERE json_extract(r.value,'$.assetId')=?1 AND COALESCE(json_extract(r.value,'$.variantId'),?2)=?2 AND COALESCE(json_extract(r.value,'$.mediaId'),'')!=?3) OR EXISTS(SELECT 1 FROM json_each(jobs.snapshot,'$.inlineReferences') r WHERE json_extract(r.value,'$.assetId')=?1 AND json_extract(r.value,'$.variantId')=?2))",params![asset,variant,media,time]).map_err(|e|e.to_string())?;
    Ok(true)
}
fn update_state(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    state: &str,
    fields: &Value,
    from: Option<&str>,
) -> Result<usize> {
    let count=tx.execute("UPDATE jobs SET state=?1,updated_at=?2,error=?3,next_attempt_at=CASE WHEN ?1 IN ('query_failed','download_failed') THEN next_attempt_at ELSE 0 END,query_attempts=CASE WHEN ?1 IN ('processing','submitted','succeeded','downloaded') THEN 0 ELSE query_attempts END,download_attempts=CASE WHEN ?1='downloaded' THEN 0 ELSE download_attempts END WHERE id=?4 AND (?5 IS NULL OR state=?5)",params![state,now(),fields.get("error").and_then(Value::as_str),key,from]).map_err(|e|e.to_string())?;
    Ok(count)
}
fn write_job_fields(tx: &rusqlite::Transaction<'_>, key: &str, fields: &Value) -> Result<()> {
    for (name, column) in [
        ("providerTaskId", "task_id"),
        ("prepared", "prepared"),
        ("receipt", "receipt"),
        ("result", "result"),
    ] {
        if let Some(value) = fields.get(name) {
            let content = if name == "providerTaskId" {
                let task = value
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("渠道任务ID不能为空")?;
                let old: Option<String> = tx
                    .query_row("SELECT task_id FROM jobs WHERE id=?1", [key], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                if old
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .map(|id| id != task)
                    .unwrap_or(false)
                {
                    return Err("已有云端任务ID不可替换；重新生成必须新建任务".into());
                }
                task.to_string()
            } else if name == "result" {
                let mut merged = value.clone();
                let old: Option<String> = tx
                    .query_row("SELECT result FROM jobs WHERE id=?1", [key], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                if let Some(old) = old.and_then(|v| serde_json::from_str::<Value>(&v).ok()) {
                    if merged.is_object()
                        && merged["actualCost"].is_null()
                        && !old["actualCost"].is_null()
                    {
                        merged["actualCost"] = old["actualCost"].clone();
                    }
                }
                merged.to_string()
            } else {
                value.to_string()
            };
            tx.execute(
                &format!("UPDATE jobs SET {column}=?1 WHERE id=?2"),
                params![content, key],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    for (name, column) in [
        ("nextAttemptAt", "next_attempt_at"),
        ("queryAttempts", "query_attempts"),
        ("downloadAttempts", "download_attempts"),
    ] {
        if let Some(value) = fields.get(name) {
            let n = value
                .as_i64()
                .filter(|n| *n >= 0)
                .ok_or("任务重试时间及计数必须是非负整数")?;
            tx.execute(
                &format!("UPDATE jobs SET {column}=?1 WHERE id=?2"),
                params![n, key],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!("json.{}.partial", id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(
        serde_json::to_string_pretty(value)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
        }
        let from: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // Same-directory atomic replacement: never remove the old registry before the rename.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(tmp, path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            fs::File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    fn fixture() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path().join("isolated-project")).unwrap();
        store.create("store contract", "p").unwrap();
        store
            .save_settings(&json!({"ffprobePath":store.root.join("missing-test-ffprobe.exe")}))
            .unwrap();
        (tmp, store)
    }
    #[test]
    fn explicit_channel_reparse_rebinds_foundation_without_reauthoring_or_changing_assets() {
        let (_temp, store) = fixture();
        let current =
            crate::pipeline::prepare_source("第1集\n场景：门口\n小王os：我听见了😀。\n").unwrap();
        let mut old = current.clone();
        old.parser_version = "source-parser/v2".into();
        let dialogue = old
            .rows
            .iter_mut()
            .find_map(|row| row.dialogue.as_mut())
            .unwrap();
        dialogue.speaker = "小王os".into();
        dialogue.channel = "现场".into();
        crate::pipeline::validate_source(&old).unwrap();
        let old = store.save_source(json!(old)).unwrap();
        let asset = store
            .save_asset(json!({"id":"hero","variantId":"base","name":"小王"}))
            .unwrap();
        let foundation = store.save_foundation(json!({"sourceSha256":current.sha256,"facts":[],"episodes":[],"overview":"原有制作基础"})).unwrap();
        let revised = store
            .save_reparsed_source_if_current(json!(current), 1)
            .unwrap();
        assert_eq!(revised["version"], 2);
        assert_eq!(revised["text"], old["text"]);
        assert_eq!(revised["sha256"], old["sha256"]);
        let carried = store.foundation().unwrap();
        assert_eq!(
            carried["version"],
            foundation["version"].as_i64().unwrap() + 1
        );
        assert_eq!(carried["sourceVersion"], 2);
        assert_eq!(carried["overview"], foundation["overview"]);
        assert_eq!(
            carried["sourceMetadataRebind"]["rawTextAndCoordinatesUnchanged"],
            true
        );
        assert_eq!(store.get("assets", "hero").unwrap(), asset);
        assert!(store.jobs().unwrap().is_empty());
        assert!(store.save_reparsed_source_if_current(revised, 1).is_err());
    }
    #[test]
    fn channel_reparse_cannot_carry_foundation_over_changed_source_or_forged_metadata() {
        let (_temp, store) = fixture();
        let current = crate::pipeline::prepare_source("第1集\n场景：门口\n小王os：走。\n").unwrap();
        let mut old = current.clone();
        old.parser_version = "source-parser/v2".into();
        let dialogue = old
            .rows
            .iter_mut()
            .find_map(|row| row.dialogue.as_mut())
            .unwrap();
        dialogue.speaker = "小王os".into();
        dialogue.channel = "现场".into();
        let old = store.save_source(json!(old)).unwrap();
        store
            .save_foundation(json!({"sourceSha256":current.sha256,"overview":"原有基础"}))
            .unwrap();
        let mut forged = json!(current);
        forged["rows"][2]["dialogue"]["speaker"] = json!("其他人物");
        assert!(!source_channel_metadata_only(&old, &forged));
        let changed =
            crate::pipeline::prepare_source("第1集\n场景：门口\n小王os：快走。\n").unwrap();
        store
            .save_reparsed_source_if_current(json!(changed), 1)
            .unwrap();
        assert_eq!(store.foundation().unwrap()["sourceVersion"], 1);
    }
    #[test]
    fn source_projection_rebind_validates_whole_episode_preserves_creative_history_and_submissions() {
        for invalid in [false,true] {
            let (_temp,store)=fixture();
            let current=crate::pipeline::prepare_source("第1集\n场景：门口\n小王os：我听见了😀。\n").unwrap();
            let mut old=current.clone();old.parser_version="source-parser/v2".into();
            let old_dialogue=old.rows.iter_mut().find_map(|row|row.dialogue.as_mut()).unwrap();
            old_dialogue.speaker="小王os".into();old_dialogue.channel="现场".into();
            store.save_source(json!(old)).unwrap();
            store.save_foundation(json!({"sourceSha256":current.sha256,"overview":"原制作基础"})).unwrap();
            let row=&current.rows[2];let quote=row.dialogue.as_ref().unwrap();
            let unit=store.save_unit(json!({"id":"U","episodeId":row.episode_id,"sceneId":row.scene_id,"sourceVersion":1,"sourceSha256":current.sha256,"prompt":"原完整分镜提示词","title":"听见","eventId":"E","unitDirection":"人物在门口停步。","scene":"门口","initialState":"人物停步。","exitState":"人物抬头。","lighting":"侧光","duration":3,"sourceRefs":[row.id],"assetRefs":[],"shots":[{"id":"S","duration":3,"camera":"近景","picture":"人物抬头。","lighting":"侧光","sound":"原声音","transition":"切","sourceRefs":[row.id],"contextRefs":[],"dialogueParts":[{"lineId":quote.id,"text":if invalid{"改过的原话"}else{&quote.quote},"speaker":quote.speaker,"channel":quote.channel,"presentation":quote.channel,"voiceStart":0,"voiceEnd":2.9,"delivery":"低声"}]}]})).unwrap();
            let task=store.add_job("video","U","dreamina","account",&json!({"unit":unit,"sourceVersion":1}),"submitted").unwrap();
            store.conn().unwrap().execute("UPDATE jobs SET task_id='original-cloud-task' WHERE id=?1",[&task]).unwrap();
            let before=store.job(&task).unwrap();
            store.save_reparsed_source_if_current(json!(current),1).unwrap();
            let report=store.rebind_verified_source_projection(&[json!("U")],&json!({"minDuration":0,"maxDuration":15,"imageLimit":0})).unwrap();
            let carried=store.get("units","U").unwrap();
            assert_eq!(store.job(&task).unwrap(),before);
            assert_eq!(carried["shots"],unit["shots"]);assert_eq!(carried["prompt"],unit["prompt"]);
            if invalid {
                assert!(report["rebound"].as_array().unwrap().is_empty());
                assert_eq!(report["blocked"].as_array().unwrap().len(),1);
                assert_eq!(carried["sourceVersion"],1);
            } else {
                assert_eq!(report["rebound"].as_array().unwrap().len(),1);
                assert_eq!(carried["sourceVersion"],2);assert_eq!(carried["version"],2);
                let original:String=store.conn().unwrap().query_row("SELECT data FROM unit_versions WHERE unit_id='U' AND version=1",[],|r|r.get(0)).unwrap();
                assert_eq!(parse(original).unwrap(),unit);
                let repeated=store.rebind_verified_source_projection(&[json!("U")],&json!({"minDuration":0,"maxDuration":15,"imageLimit":0})).unwrap();
                assert!(repeated["rebound"].as_array().unwrap().is_empty());
                assert_eq!(store.get("units","U").unwrap(),carried);
            }
        }
    }
    #[test]
    #[ignore="explicit consistent private copy only; production DB is never writable in this regression"]
    fn actual_source_projection_rebind_preserves_all_twenty_four_units_and_all_cloud_receipts() {
        let root=std::env::var("DIRECTOR_PROJECTION_ISOLATED_ROOT").expect("explicit isolated root");
        assert!(root.replace('\\',"/").contains("/reports/desktop-real-e2e-20261005/boundary-regression-projection-"));
        let store=Store::open(PathBuf::from(&root)).unwrap();
        let source=store.source().unwrap();assert_eq!(source["sha256"],"39d7551dcd8802cf25d597e2e8932b8b6edfe93517ec8a181343458169960b17");
        let units:Vec<_>=store.values("units").unwrap().into_iter().filter(|u|u["retired"]!=true && u["sourceVersion"]==1).collect();assert_eq!(units.len(),24);
        let ids:Vec<_>=units.iter().map(|u|u["id"].clone()).collect();
        let raw:String=store.conn().unwrap().query_row("SELECT snapshot FROM jobs WHERE id='36b872ee-72d4-4a60-8bc0-15625a6fe318'",[],|r|r.get(0)).unwrap();
        let caps=parse(raw).unwrap()["input"]["capabilities"].clone();
        assert!(caps["maxDuration"].as_f64().is_some_and(|v|v>0.));
        assert_eq!(store.rebind_verified_source_projection(&ids,&caps).unwrap()["deferred"],"storyboard_workflow_active");
        // Simulate coordinator completion exclusively in the disposable copy.
        store.conn().unwrap().execute("UPDATE jobs SET state='completed' WHERE id='36b872ee-72d4-4a60-8bc0-15625a6fe318' AND kind='workflow'",[]).unwrap();
        let read_media = || {
            let c=store.conn().unwrap();let mut q=c.prepare("SELECT data FROM media ORDER BY rowid").unwrap();
            q.query_map([],|r|r.get::<_,String>(0)).unwrap().map(|r|r.unwrap()).collect::<Vec<_>>()
        };
        let assets=store.values("assets").unwrap();let media=read_media();
        let known:Vec<_>=store.jobs().unwrap().into_iter().filter(|j|!s(j,"providerTaskId").is_empty()).collect();
        let foundation=store.foundation().unwrap();
        let report=store.rebind_verified_source_projection(&ids,&caps).unwrap();
        assert_eq!(report["rebound"].as_array().unwrap().len(),24,"{report}");assert!(report["blocked"].as_array().unwrap().is_empty());assert_eq!(report["foundationRebound"],true);
        for unit in units {
            let mut carried=store.get("units",s(&unit,"id")).unwrap();let mut original=unit.clone();
            assert_eq!(carried["sourceVersion"],2);assert_eq!(carried["sourceMetadataRebind"]["creativeContentUnchanged"],true);
            for key in ["version","sourceVersion","sourceMetadataRebind"] {carried.as_object_mut().unwrap().remove(key);original.as_object_mut().unwrap().remove(key);}
            assert_eq!(carried,original);
        }
        let mut carried=store.foundation().unwrap();let mut original=foundation;
        for key in ["version","sourceVersion","sourceMetadataRebind"] {carried.as_object_mut().unwrap().remove(key);original.as_object_mut().unwrap().remove(key);}
        assert_eq!(carried,original);assert_eq!(store.values("assets").unwrap(),assets);assert_eq!(read_media(),media);
        let after:Vec<_>=store.jobs().unwrap().into_iter().filter(|j|!s(j,"providerTaskId").is_empty()).collect();assert_eq!(after,known);
        assert!(store.rebind_verified_source_projection(&ids,&caps).unwrap()["rebound"].as_array().unwrap().is_empty());
    }
    fn author_store_fixture() -> (tempfile::TempDir, Store, Value) {
        let (tmp, store) = fixture();
        let source =
            crate::pipeline::prepare_source("第1集\n场景：大厅\n小王：开始。\n△小王穿白衣进入。\n")
                .unwrap();
        store.save_source(json!(source)).unwrap();
        let refs = source
            .rows
            .iter()
            .take(2)
            .map(|r| r.id.clone())
            .collect::<Vec<_>>();
        let choice = json!({"decisionKey":"identity-conflict","kind":"SOURCE_CONFLICT","question":"制作时采用哪种身份解释？","reason":"原文声明不同","options":["保留标注","单独解释"],"recommended":"单独解释","recommendationReason":"逐字保留原文","evidence":"两处原文","episodes":[source.episodes[0].id],"sourceRefs":refs,"changeTargets":{"assetKeys":[],"assetFields":[],"sceneFields":["speaker"]}});
        store.save_foundation(json!({"schemaVersion":"foundation/v1","sourceSha256":source.sha256,"overview":"","direction":"","episodes":[],"assets":[],"facts":[],"issues":[{"coordinate":"choice:identity-conflict","problem":"身份解释","sourceRefs":refs,"blocking":true,"episodeIds":[source.episodes[0].id],"details":{"owner":"AUTHOR","kind":"HUMAN_CHOICE","choice":choice,"recommendedIsAdopted":false}}]})).unwrap();
        let view = store.foundation_choices().unwrap();
        let request = json!({"projectId":"p","contractId":view["contracts"][0]["id"],"revision":view["contracts"][0]["revision"],"expectedSourceVersion":view["sourceVersion"],"expectedFoundationVersion":view["foundationVersion"],"customAnswer":"逐字保留说者、台词和动作；后续制作单独解释"});
        (tmp, store, request)
    }
    #[test]
    fn author_confirmation_cas_preserves_source_and_rejects_stale_or_repeated_answer() {
        let (_tmp, store, request) = author_store_fixture();
        let source = store.source().unwrap();
        let basis = store.foundation().unwrap();
        let mut wrong = request.clone();
        wrong["revision"] = json!("stale");
        assert!(store.confirm_foundation_choice(&wrong).is_err());
        wrong = request.clone();
        wrong["expectedSourceVersion"] = json!(0);
        assert!(store.confirm_foundation_choice(&wrong).is_err());
        wrong = request.clone();
        wrong["expectedFoundationVersion"] = json!(0);
        assert!(store.confirm_foundation_choice(&wrong).is_err());
        wrong = request.clone();
        wrong["optionIndex"] = json!(0);
        assert!(store.confirm_foundation_choice(&wrong).is_err());
        wrong = request.clone();
        wrong.as_object_mut().unwrap().remove("customAnswer");
        wrong["optionIndex"] = json!(9);
        assert!(store.confirm_foundation_choice(&wrong).is_err());
        let confirmed = store.confirm_foundation_choice(&request).unwrap();
        assert_eq!(confirmed["contracts"][0]["status"], "CONFIRMED");
        assert_eq!(store.source().unwrap(), source);
        assert_eq!(store.foundation().unwrap(), basis);
        assert!(store.confirm_foundation_choice(&request).is_err());
        assert!(
            store.jobs().unwrap().is_empty(),
            "confirming does not launch any model"
        );
    }
    #[test]
    fn author_confirmation_is_atomic_under_concurrent_submissions() {
        let (_tmp, store, request) = author_store_fixture();
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let request = request.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.confirm_foundation_choice(&request)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            store.foundation_choices().unwrap()["contracts"][0]["status"],
            "CONFIRMED"
        );
    }
    #[test]
    fn author_apply_reservation_guards_confirmed_set_and_retains_f2_checkpoint() {
        let (_tmp, store, request) = author_store_fixture();
        store.ensure_workflow_storage().unwrap();
        let old = store
            .add_job(
                "workflow",
                "foundation.run",
                "local",
                "local",
                &json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run"}),
                "completed",
            )
            .unwrap();
        let f2 = store
            .add_job(
                "text",
                "foundation:F2",
                "codex",
                "local",
                &json!({"stage":"foundation:F2","input":{"frozen":"original"}}),
                "accepted",
            )
            .unwrap();
        store
            .checkpoint(
                &old,
                "F2-original",
                "foundation:F2",
                "old-hash",
                "accepted",
                Some(&f2),
                Some(&json!({"kept":true})),
                None,
            )
            .unwrap();
        let original = store.job(&f2).unwrap();
        let view = store.confirm_foundation_choice(&request).unwrap();
        let snapshot = json!({"runnerVersion":"desktop-runner/v1","foundationMethod":crate::pipeline::foundation_delivery::METHOD_VERSION,"stage":"foundation.run","foundationPhase":"APPLY_DECISION","guard":{"sourceVersion":1,"foundationVersion":1,"assets":{},"units":{},"episodeIds":view["contracts"][0]["episodes"]},"input":{"foundationPhase":"APPLY_DECISION","confirmedDecisions":view["contracts"]}});
        let mut invalid = snapshot.clone();
        invalid["input"]["confirmedDecisions"][0]["answer"] = json!("篡改冻结答案");
        assert!(store.reserve_foundation_choice_workflow(&invalid).is_err());
        let run = store.reserve_foundation_choice_workflow(&snapshot).unwrap();
        assert_eq!(store.job(&run).unwrap()["snapshot"], snapshot);
        assert!(store.reserve_foundation_choice_workflow(&snapshot).is_err());
        assert_eq!(store.job(&f2).unwrap(), original);
        assert_eq!(
            store.accepted_step(&old, "old-hash").unwrap(),
            Some(json!({"kept":true}))
        );
    }
    #[test]
    fn author_confirmation_waits_for_active_local_text_pipeline() {
        let (_tmp, store, request) = author_store_fixture();
        store
            .add_job(
                "workflow",
                "images.design",
                "local",
                "local",
                &json!({}),
                "running",
            )
            .unwrap();
        assert!(store.confirm_foundation_choice(&request).is_err());
        assert_eq!(
            store.foundation_choices().unwrap()["contracts"][0]["status"],
            "PENDING"
        );
    }
    #[test]
    fn auto_author_reservation_is_atomic_without_manual_confirmation() {
        let (_tmp, store, _) = author_store_fixture();
        store.ensure_workflow_storage().unwrap();
        let source = store.source().unwrap();
        let foundation = store.foundation().unwrap();
        let decisions = store.foundation_automatic_decisions(&json!([])).unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0]["status"], "MODEL_PENDING");
        let snapshot = json!({"runnerVersion":"desktop-runner/v1","foundationMethod":crate::pipeline::foundation_delivery::METHOD_VERSION,"stage":"foundation.run","guard":{"sourceVersion":1,"foundationVersion":1,"assets":{},"units":{},"episodeIds":decisions[0]["episodes"]},"input":{"projectId":"p","sourceVersion":1,"foundationPhase":"AUTO_INTERPRET","foundationDecisionPolicy":crate::pipeline::foundation_delivery::AUTO_INTERPRET_POLICY,"automaticDecisions":decisions,"userReviewRequirements":[]}});
        let mut invalid = snapshot.clone();
        invalid["input"]["automaticDecisions"][0]["sourceSha256"] = json!("stale");
        assert!(store.reserve_foundation_auto_workflow(&invalid).is_err());
        let id = store.reserve_foundation_auto_workflow(&snapshot).unwrap();
        assert_eq!(store.job(&id).unwrap()["snapshot"], snapshot);
        assert!(store.reserve_foundation_auto_workflow(&snapshot).is_err());
        assert_eq!(store.source().unwrap(), source);
        assert_eq!(store.foundation().unwrap(), foundation);
    }
    #[test]
    fn auto_author_preserves_old_answer_as_reference_and_honors_user_requirement_cas() {
        let (_tmp, store, request) = author_store_fixture();
        store.confirm_foundation_choice(&request).unwrap();
        let decisions = store.foundation_automatic_decisions(&json!([])).unwrap();
        assert_eq!(decisions[0]["priorAnswer"], request["customAnswer"]);
        assert!(decisions[0]["answer"].is_null());
        let required = json!([{"decisionKey":"identity-conflict","needsUserConfirmation":true}]);
        store
            .save_settings(&json!({"userReviewRequirements":required}))
            .unwrap();
        assert!(store
            .foundation_automatic_decisions(&required)
            .unwrap()
            .is_empty());
        assert_eq!(
            store.foundation_choices().unwrap()["contracts"][0]["requiresUserReview"],
            true
        );
        let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run","guard":{"sourceVersion":1,"foundationVersion":1,"assets":{},"units":{},"episodeIds":decisions[0]["episodes"]},"input":{"projectId":"p","sourceVersion":1,"foundationPhase":"AUTO_INTERPRET","foundationDecisionPolicy":crate::pipeline::foundation_delivery::AUTO_INTERPRET_POLICY,"automaticDecisions":decisions,"userReviewRequirements":[]}});
        assert!(store.reserve_foundation_auto_workflow(&snapshot).is_err());
        assert!(store.jobs().unwrap().is_empty());
    }
    #[test]
    fn recovery_cli_closing_known_media_tasks_is_query_only_and_exact() {
        let (_tmp, store, _) = author_store_fixture();
        let mut cases = vec![];
        for (kind, state, task, error, recover) in [
            (
                "image",
                "auth_required",
                Some("image-original"),
                "客户端已停止本地 CLI 查询。",
                true,
            ),
            (
                "video",
                "auth_required",
                Some("video-original"),
                "客户端已停止本地 CLI 查询。",
                true,
            ),
            (
                "image",
                "auth_required",
                Some("real-auth"),
                "登录已失效",
                false,
            ),
            (
                "image",
                "auth_required",
                None,
                "客户端已停止本地 CLI 查询。",
                false,
            ),
            (
                "video",
                "auth_required",
                Some(""),
                "客户端已停止本地 CLI 查询。",
                false,
            ),
            (
                "image",
                "submission_unknown",
                None,
                "客户端已停止本地 CLI 查询。",
                false,
            ),
            (
                "video",
                "paused",
                Some("budget-paused"),
                "客户端已停止本地 CLI 查询。",
                false,
            ),
            (
                "workflow",
                "auth_required",
                Some("other-kind"),
                "客户端已停止本地 CLI 查询。",
                false,
            ),
            (
                "image",
                "auth_required",
                Some("similar-error"),
                "客户端已停止本地 CLI 查询。 额外原因",
                false,
            ),
        ] {
            let key = store
                .add_job(
                    kind,
                    &format!("fixture-{}", cases.len()),
                    "dreamina",
                    "fixture-account",
                    &json!({"originalSubmission":"immutable","estimatedCost":{"amount":9}}),
                    state,
                )
                .unwrap();
            let mut fields = json!({"error":error,"nextAttemptAt":123456,"queryAttempts":3,"downloadAttempts":2,"prepared":{"uploadReceipts":[{"id":"old-upload"}]},"receipt":{"taskId":task,"actualCost":{"amount":8}},"result":{"completedOriginalResult":true}});
            if let Some(task) = task.filter(|task| !task.is_empty()) {
                fields["providerTaskId"] = json!(task);
            }
            store.update_job(&key, state, &fields).unwrap();
            if task == Some("") {
                store
                    .conn()
                    .unwrap()
                    .execute("UPDATE jobs SET task_id='' WHERE id=?1", [&key])
                    .unwrap();
            }
            let original = store.job(&key).unwrap();
            cases.push((key, original, recover));
        }
        store.recover_interrupted().unwrap();
        assert_eq!(store.jobs().unwrap().len(), cases.len());
        for (key, original, recover) in &cases {
            let updated = store.job(key).unwrap();
            if *recover {
                assert_eq!(updated["status"], "submitted");
                assert_eq!(updated["nextAttemptAt"], 0);
                assert!(updated["error"].is_null());
            } else {
                assert_eq!(updated, *original);
            }
            for field in [
                "providerTaskId",
                "accountId",
                "snapshot",
                "prepared",
                "receipt",
                "result",
                "queryAttempts",
                "downloadAttempts",
            ] {
                assert_eq!(
                    updated[field], original[field],
                    "recovery must preserve {field}"
                );
            }
        }
        let once = store.jobs().unwrap();
        store.recover_interrupted().unwrap();
        assert_eq!(
            store.jobs().unwrap(),
            once,
            "idempotent startup cannot create a new generation or reset budgets"
        );
    }
    fn image(store: &Store, label: &str, color: u8, asset: Option<&str>) -> Value {
        let file = store.root.join(format!("{label}.png"));
        image::RgbImage::from_pixel(2, 2, image::Rgb([color, 7, 13]))
            .save(&file)
            .unwrap();
        store
            .import_media(&file, "image", asset, asset.map(|_| "base"))
            .unwrap()
    }
    #[test]
    fn cas_prepared_receipt_is_atomic_and_cannot_resurrect_stale_work() {
        let (_tmp, store) = fixture();
        let key = store
            .add_job(
                "video",
                "u",
                "mock",
                "account",
                &json!({"prompt":"frozen"}),
                "queued",
            )
            .unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|winner| {
                let store = store.clone();
                let key = key.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    (
                        winner,
                        store
                            .claim_job_fields(
                                &key,
                                "queued",
                                "ready",
                                &json!({"prepared":{"winner":winner}}),
                            )
                            .unwrap(),
                    )
                })
            })
            .collect();
        let winners: Vec<_> = workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .filter(|(_, won)| *won)
            .collect();
        assert_eq!(winners.len(), 1);
        assert_eq!(store.job(&key).unwrap()["prepared"]["winner"], winners[0].0);
        assert!(store.claim_job(&key, "ready", "preparing").unwrap());
        store
            .update_job(
                &key,
                "stale",
                &json!({"error":"reference replaced during upload"}),
            )
            .unwrap();
        assert!(!store
            .claim_job_fields(
                &key,
                "preparing",
                "ready",
                &json!({"prepared":{"winner":"late upload"}})
            )
            .unwrap());
        assert_eq!(s(&store.job(&key).unwrap(), "status"), "stale");
        assert_eq!(store.job(&key).unwrap()["prepared"]["winner"], winners[0].0);
    }
    #[test]
    fn query_and_download_backoff_are_independent_and_capped() {
        let (_tmp, store) = fixture();
        let key = store
            .add_job("video", "u", "mock", "account", &json!({}), "submitted")
            .unwrap();
        store
            .update_job(
                &key,
                "submitted",
                &json!({"providerTaskId":"remote-original"}),
            )
            .unwrap();
        let started = Utc::now().timestamp();
        store
            .retry_job(&key, "query", "temporary query failure")
            .unwrap();
        let job = store.job(&key).unwrap();
        assert_eq!(job["queryAttempts"], 1);
        assert_eq!(job["downloadAttempts"], 0);
        assert!(
            job["nextAttemptAt"].as_i64().unwrap() >= started + 5
                && job["nextAttemptAt"].as_i64().unwrap() <= Utc::now().timestamp() + 5
        );
        store
            .retry_job(&key, "download", "download failure")
            .unwrap();
        let job = store.job(&key).unwrap();
        assert_eq!(job["queryAttempts"], 1);
        assert_eq!(job["downloadAttempts"], 1);
        assert!(job["nextAttemptAt"].as_i64().unwrap() >= Utc::now().timestamp() + 9);
        for _ in 0..12 {
            store
                .retry_job(&key, "query", "same task remains unavailable")
                .unwrap();
        }
        let job = store.job(&key).unwrap();
        assert!(job["nextAttemptAt"].as_i64().unwrap() <= Utc::now().timestamp() + 300);
        assert_eq!(job["downloadAttempts"], 1);
        for _ in 0..12 {
            store
                .retry_job(&key, "download", "URL download still fails")
                .unwrap();
        }
        assert!(
            store.job(&key).unwrap()["nextAttemptAt"].as_i64().unwrap()
                <= Utc::now().timestamp() + 600
        );
        assert!(store
            .retry_job(&key, "submit", "must never resubmit")
            .is_err());
        assert!(store
            .update_job(
                &key,
                "submitted",
                &json!({"providerTaskId":"different-task"})
            )
            .is_err());
        assert_eq!(
            s(&store.job(&key).unwrap(), "providerTaskId"),
            "remote-original"
        );
        store.update_job(&key, "processing", &json!({})).unwrap();
        assert_eq!(store.job(&key).unwrap()["queryAttempts"], 0);
        store.update_job(&key, "downloaded", &json!({})).unwrap();
        assert_eq!(store.job(&key).unwrap()["downloadAttempts"], 0);
    }
    #[test]
    fn unresolved_active_states_prevent_a_second_generation_identity() {
        let (_tmp, store) = fixture();
        for state in [
            "submission_unknown",
            "query_failed",
            "download_failed",
            "recovery_paused",
            "waiting_dependency",
            "succeeded",
        ] {
            let entity = format!("unit-{state}");
            store
                .add_job("video", &entity, "mock", "account", &json!({}), state)
                .unwrap();
            assert!(
                store
                    .add_job(
                        "video",
                        &entity,
                        "other-channel",
                        "account",
                        &json!({}),
                        "queued"
                    )
                    .is_err(),
                "{state} must still own the unresolved generation identity"
            );
        }
    }
    #[test]
    fn replacing_adoption_expires_only_frozen_unsubmitted_references() {
        let (_tmp, store) = fixture();
        store
            .save_asset(json!({"id":"parent","variantId":"base","name":"parent"}))
            .unwrap();
        let one = image(&store, "one", 20, Some("parent"));
        let two = image(&store, "two", 40, Some("parent"));
        store.adopt("parent", "base", s(&one, "id")).unwrap();
        let frozen =
            json!({"references":[{"assetId":"parent","variantId":"base","mediaId":one["id"]}]});
        let ready = store
            .add_job("image", "child-ready", "mock", "account", &frozen, "ready")
            .unwrap();
        let upload = store
            .add_job(
                "image",
                "child-upload",
                "mock",
                "account",
                &frozen,
                "preparing",
            )
            .unwrap();
        let waiting = store
            .add_job(
                "image",
                "child-wait",
                "mock",
                "account",
                &json!({"references":[],"dependencies":["parent"]}),
                "waiting_dependency",
            )
            .unwrap();
        let submitted = store
            .add_job(
                "video",
                "submitted",
                "mock",
                "account",
                &frozen,
                "submitted",
            )
            .unwrap();
        store
            .update_job(
                &submitted,
                "submitted",
                &json!({"providerTaskId":"original"}),
            )
            .unwrap();
        let unrelated = store
            .add_job(
                "video",
                "unrelated",
                "mock",
                "account",
                &json!({"references":[{"assetId":"other","mediaId":one["id"]}]}),
                "ready",
            )
            .unwrap();
        let before: String = store
            .conn()
            .unwrap()
            .query_row(
                "SELECT value FROM metadata WHERE key='changed_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store.adopt("parent", "base", s(&one, "id")).unwrap();
        let after: String = store
            .conn()
            .unwrap()
            .query_row(
                "SELECT value FROM metadata WHERE key='changed_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after, "same adoption is a no-op");
        store.adopt("parent", "base", s(&two, "id")).unwrap();
        assert_eq!(s(&store.job(&ready).unwrap(), "status"), "stale");
        assert_eq!(s(&store.job(&upload).unwrap(), "status"), "stale");
        assert_eq!(
            s(&store.job(&waiting).unwrap(), "status"),
            "waiting_dependency"
        );
        assert_eq!(s(&store.job(&submitted).unwrap(), "status"), "submitted");
        assert_eq!(store.job(&submitted).unwrap()["snapshot"], frozen);
        assert_eq!(s(&store.job(&unrelated).unwrap(), "status"), "ready");
        assert!(!store.claim_job(&upload, "preparing", "ready").unwrap());
    }
    #[test]
    fn downloaded_media_projection_survives_future_query_results() {
        let (_tmp, store) = fixture();
        let media = image(&store, "result", 11, None);
        let key = store
            .add_job("video", "u", "mock", "account", &json!({}), "submitted")
            .unwrap();
        store.attach_result(&key, s(&media, "id")).unwrap();
        store
            .update_job(
                &key,
                "downloaded",
                &json!({"result":{"mediaIds":[media["id"]],"actualCost":1.25}}),
            )
            .unwrap();
        store
            .update_job(
                &key,
                "succeeded",
                &json!({"result":{"taskId":"same-remote","resultUrls":["refreshed-url"]}}),
            )
            .unwrap();
        let job = store.job(&key).unwrap();
        assert_eq!(job["resultMediaIds"], json!([media["id"]]));
        assert_eq!(job["result"]["mediaIds"], json!([media["id"]]));
        assert_eq!(job["result"]["actualCost"], 1.25);
    }
    #[test]
    fn relocation_rebuilds_thumbnail_and_preserves_submitted_and_unknown_snapshot_bytes() {
        let (tmp, store) = fixture();
        let media = image(&store, "relocate", 9, None);
        let snapshot = json!({"references":[{"path":media["path"],"relativePath":media["relativePath"],"sha256":media["sha256"]}]});
        let ready = store
            .add_job("video", "ready", "mock", "account", &snapshot, "ready")
            .unwrap();
        store
            .update_job(
                &ready,
                "ready",
                &json!({"prepared":{"wire":{"args":["old-machine-path"]}}}),
            )
            .unwrap();
        let submitted = store
            .add_job(
                "video",
                "submitted",
                "mock",
                "account",
                &snapshot,
                "submitted",
            )
            .unwrap();
        store
            .update_job(
                &submitted,
                "submitted",
                &json!({"providerTaskId":"known-original","receipt":{"taskId":"known-original"}}),
            )
            .unwrap();
        let unknown = store
            .add_job(
                "video",
                "unknown",
                "mock",
                "account",
                &snapshot,
                "submission_unknown",
            )
            .unwrap();
        let bytes = |st: &Store, key: &str| -> String {
            st.conn()
                .unwrap()
                .query_row("SELECT snapshot FROM jobs WHERE id=?1", [key], |r| r.get(0))
                .unwrap()
        };
        let submitted_bytes = bytes(&store, &submitted);
        let unknown_bytes = bytes(&store, &unknown);
        let root = tmp.path().join("other-machine");
        store.backup_db(&root.join("project.sqlite")).unwrap();
        let moved = Store::open(root.clone()).unwrap();
        let destination = moved.absolute_media(&media).unwrap();
        fs::copy(store.verify_media(&media).unwrap(), destination).unwrap();
        moved
            .invalidate_unsubmitted("machine-local prepared paths changed")
            .unwrap();
        moved.refresh_media_paths().unwrap();
        let remapped = moved.get("media", s(&media, "id")).unwrap();
        assert!(Path::new(s(&remapped, "path")).starts_with(&root));
        assert!(Path::new(s(&remapped, "thumbnailPath")).starts_with(&root));
        assert!(Path::new(s(&remapped, "thumbnailPath")).is_file());
        assert_eq!(remapped["sha256"], media["sha256"]);
        assert_eq!(s(&moved.job(&ready).unwrap(), "status"), "stale");
        assert!(moved.job(&ready).unwrap()["prepared"].is_null());
        assert_eq!(bytes(&moved, &submitted), submitted_bytes);
        assert_eq!(bytes(&moved, &unknown), unknown_bytes);
    }
    #[test]
    fn invalid_av_metadata_preserves_original_and_records_the_real_failure() {
        let (_tmp, store) = fixture();
        let path = store.root.join("broken.mp4");
        fs::write(&path, b"not a video container").unwrap();
        let media = store.import_media(&path, "video", None, None).unwrap();
        assert_eq!(media["metadataVerified"], false);
        assert!(!s(&media, "metadataError").is_empty());
        assert!(path.is_file());
        assert!(store.verify_media(&media).unwrap().is_file());
        assert!(media["durationSeconds"].is_null());
    }
    #[test]
    fn schema_gate_refuses_unknown_versions_and_reopening_v2_does_not_rewrite_the_db() {
        let (_tmp, store) = fixture();
        let before = hash_file(&store.root.join("project.sqlite")).unwrap().0;
        for _ in 0..3 {
            Store::open(store.root.clone()).unwrap();
        }
        assert_eq!(
            hash_file(&store.root.join("project.sqlite")).unwrap().0,
            before
        );
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE metadata SET value='99' WHERE key='schema_version'",
                [],
            )
            .unwrap();
        let before = hash_file(&store.root.join("project.sqlite")).unwrap().0;
        assert!(Store::open(store.root.clone()).is_err());
        assert_eq!(
            hash_file(&store.root.join("project.sqlite")).unwrap().0,
            before
        );
        let actual: String = store
            .conn()
            .unwrap()
            .query_row(
                "SELECT value FROM metadata WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(actual, "99");
    }
    #[test]
    fn legacy_desktop_schema_one_migrates_retry_fields_without_losing_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("v1-project");
        fs::create_dir_all(&root).unwrap();
        let db = Connection::open(root.join("project.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);INSERT INTO metadata VALUES('schema_version','1');CREATE TABLE jobs(id TEXT PRIMARY KEY,kind TEXT NOT NULL,entity_id TEXT NOT NULL,provider TEXT NOT NULL,state TEXT NOT NULL,task_id TEXT,account_id TEXT NOT NULL,snapshot TEXT NOT NULL,prepared TEXT,receipt TEXT,result TEXT,error TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);INSERT INTO jobs VALUES('existing','video','unit','mock','submitted','original-id','account','{\"prompt\":\"frozen\"}',NULL,NULL,NULL,NULL,'old','old');").unwrap();
        drop(db);
        let store = Store::open(root).unwrap();
        let job = store.job("existing").unwrap();
        assert_eq!(s(&job, "providerTaskId"), "original-id");
        assert_eq!(job["snapshot"]["prompt"], "frozen");
        assert_eq!(job["nextAttemptAt"], 0);
        assert_eq!(job["queryAttempts"], 0);
        assert_eq!(job["downloadAttempts"], 0);
        let version: String = store
            .conn()
            .unwrap()
            .query_row(
                "SELECT value FROM metadata WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, "2");
    }
    #[test]
    fn atomic_json_replaces_the_existing_registry_with_a_complete_document() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("项目索引.json");
        atomic_json(&path, &json!([{"id":"first"}])).unwrap();
        for n in 0..10 {
            let expected =
                json!([{"id":"first"},{"id":format!("project-{n}"),"title":"完整新内容😀"}]);
            atomic_json(&path, &expected).unwrap();
            let actual: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(actual, expected);
        }
        assert_eq!(
            fs::read_dir(tmp.path()).unwrap().count(),
            1,
            "successful replacement leaves one complete registry"
        );
    }
    #[test]
    fn workflow_summary_propagates_child_unknown_without_mutating_parent_or_exposing_payloads() {
        let (_temp,store) = fixture();
        store.ensure_workflow_storage().unwrap();
        for stage in ["foundation.run","storyboard.run"] {
            let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":stage,"sourceVersion":1,
                "selection":{"episodeIds":["e1"]},"input":{"source":{"version":1,"sha256":"a".repeat(64),"text":"SYNTHETIC_PRIVATE_SOURCE"},
                    "settings":{"textApiKey":"SYNTHETIC_PRIVATE_SETTING"}},"rules":[{"content":"SYNTHETIC_PRIVATE_RULE"}]});
            let parent = store.add_job("workflow",stage,"local","local",&snapshot,"failed").unwrap();
            let child = store.add_job("text","frozen-node","codex_sdk","local",&json!({"private":"SYNTHETIC_PRIVATE_REQUEST"}),"submission_unknown").unwrap();
            store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,updated_at) VALUES(?1,'frozen-node',?2,'synthetic','failed',?3,?4)",params![parent,stage,child,now()]).unwrap();
            let before = store.job(&parent).unwrap();
            let summary = store.job_summaries().unwrap().into_iter().find(|job|job["id"]==parent).unwrap();
            assert_eq!(summary["status"],"failed");
            assert_eq!(summary["submissionUnknown"],true,"child state is authoritative even when the checkpoint still says failed");
            assert!(!summary.to_string().contains("SYNTHETIC_PRIVATE_"));
            assert_eq!(store.job(&parent).unwrap(),before,"summary does not change the parent record");
            assert_eq!(store.job(&child).unwrap()["status"],"submission_unknown");
            for state in ["failed","rejected","completed"] {
                store.update_job(&child,state,&json!({})).unwrap();
                let summary = store.job_summaries().unwrap().into_iter().find(|job|job["id"]==parent).unwrap();
                assert_eq!(summary["submissionUnknown"],false,"known terminal child {state} is not an unknown submission");
                assert_eq!(store.job(&parent).unwrap(),before);
            }
        }
    }
    #[test]
    fn old_workflow_with_unknown_child_remains_in_bounded_summary_history() {
        let (_temp,store) = fixture();store.ensure_workflow_storage().unwrap();
        let parent = store.add_job("workflow","foundation.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run","sourceVersion":1,"selection":{"episodeIds":["e1"]}}),"completed").unwrap();
        let child = store.add_job("text","frozen-node","codex_sdk","local",&json!({}),"submission_unknown").unwrap();
        store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,updated_at) VALUES(?1,'frozen-node','foundation:F2','synthetic','failed',?2,?3)",params![parent,child,now()]).unwrap();
        for _ in 0..105 {store.add_job("text","unrelated-history","local","local",&json!({}),"completed").unwrap();}
        let before_parent = store.job(&parent).unwrap();let before_child = store.job(&child).unwrap();
        let summaries = store.job_summaries().unwrap();
        let summary = summaries.iter().find(|job|job["id"]==parent).unwrap();
        assert_eq!(summary["submissionUnknown"],true,"completed parent projection cannot hide an unknown child");
        assert!(summaries.iter().any(|job|job["id"]==child));
        assert_eq!(summaries.len(),102,"only the two unresolved identities extend the 100-record text/workflow window");
        assert!(summaries.iter().filter(|job|job["id"]!=parent && job["id"]!=child).all(|job|job["submissionUnknown"]==false));
        assert_eq!(store.job(&parent).unwrap(),before_parent);assert_eq!(store.job(&child).unwrap(),before_child);
    }
    #[test]
    fn cli_unknown_summary_projects_legacy_failure_without_mutating_or_hiding_original_rows() {
        let (_temp,store) = fixture();store.ensure_workflow_storage().unwrap();
        let parent = store.add_job("workflow","storyboard.run","local","local",&json!({"runnerVersion":"desktop-runner/v1","stage":"storyboard.run","selection":{"episodeIds":["e1"]}}),"partially_blocked").unwrap();
        let child = store.add_job("text","frozen-node","codex","local",&json!({"private":"SYNTHETIC_PRIVATE_REQUEST"}),"failed").unwrap();
        store.update_job(&child,"failed",&json!({"error":"original timeout","result":{"retained":"SYNTHETIC_PRIVATE_RAW"},"prepared":{"budget":1},"receipt":{"original":true}})).unwrap();
        store.event(&child,"codex_event",&json!({"type":"turn.started"})).unwrap();
        store.conn().unwrap().execute("INSERT INTO workflow_steps(workflow_id,node_key,stage,request_hash,state,text_job_id,updated_at) VALUES(?1,'frozen-node','storyboardCreate','synthetic','failed',?2,?3)",params![parent,child,now()]).unwrap();
        for _ in 0..105 {store.add_job("text","unrelated-history","local","local",&json!({}),"completed").unwrap();}
        let before_parent = store.job(&parent).unwrap();let before_child = store.job(&child).unwrap();
        let count = || store.conn().unwrap().query_row("SELECT COUNT(*) FROM events",[],|row|row.get::<_,i64>(0)).unwrap();
        let events_before = count();
        let stored_bytes = || {
            let c = store.conn().unwrap();
            let mut q = c.prepare("SELECT json_array(id,state,snapshot,prepared,receipt,result,error,updated_at) FROM jobs WHERE id IN (?1,?2) ORDER BY id").unwrap();
            let rows = q.query_map(params![parent,child],|row|row.get::<_,String>(0)).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();
            rows
        };
        let bytes_before = stored_bytes();
        let (_other_temp,other) = fixture();
        let unrelated = other.add_job("workflow","storyboard.run","local","local",&json!({}),"failed").unwrap();
        for _ in 0..2 {
            let summaries = store.job_summaries().unwrap();
            assert_eq!(summaries.len(),102,"old unknown CLI and its workflow survive the history cap");
            for (key, status) in [(&parent,"partially_blocked"),(&child,"failed")] {
                let summary = summaries.iter().find(|job|job["id"]==*key).unwrap();
                assert_eq!(summary["status"],status,"stored historical status remains readable");
                assert_eq!(summary["submissionUnknown"],true);
                assert_eq!(summary["cliSubmissionUnknown"],true);
                assert!(!s(summary,"submissionUnknownReason").is_empty());
                assert!(!summary.to_string().contains("SYNTHETIC_PRIVATE_"));
            }
            assert!(summaries.iter().filter(|job|job["id"]!=parent && job["id"]!=child).all(|job|job["submissionUnknown"]==false && job["cliSubmissionUnknown"].is_null()));
            assert_eq!(store.job(&parent).unwrap(),before_parent);assert_eq!(store.job(&child).unwrap(),before_child);
            assert_eq!(stored_bytes(),bytes_before,"raw snapshot/result/receipt serialization and timestamps stay byte-identical");
            assert_eq!(count(),events_before,"projection writes no events or recovery state");
            let other_summary = other.job_summaries().unwrap().into_iter().find(|job|job["id"]==unrelated).unwrap();
            assert_eq!(other_summary["submissionUnknown"],false,"another project's production remains independent");
            assert!(other_summary["cliSubmissionUnknown"].is_null());
        }
        // A real terminal event removes only the projection; no fake state migration.
        store.event(&child,"codex_event",&json!({"type":"turn.completed"})).unwrap();
        let summaries = store.job_summaries().unwrap();
        assert_eq!(summaries.len(),100);
        assert!(summaries.iter().all(|job|job["submissionUnknown"]==false));
        assert_eq!(store.job(&parent).unwrap(),before_parent);assert_eq!(store.job(&child).unwrap(),before_child);
    }
    #[test]
    fn workflow_summary_keeps_only_frozen_identity_and_effective_scope() {
        let (_tmp, store) = fixture();
        for stage in ["foundation.run", "storyboard.run"] {
            let source_sha = "a".repeat(64);
            let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":stage,
                "sourceVersion":9,"selection":{"episodeIds":["old-top-level-scope"]},
                "planningProfile":{"private":"SYNTHETIC_PRIVATE_PROFILE"},
                "input":{"source":{"version":9,"sha256":source_sha,"text":"SYNTHETIC_PRIVATE_SOURCE","rows":[{"private":"SYNTHETIC_PRIVATE_ROW"}]},
                    "selection":{"episodeIds":["e1","e2"],"unitIds":["u1"]},
                    "settings":{"textApiKey":"SYNTHETIC_PRIVATE_SETTING"}},
                "rules":[{"content":"SYNTHETIC_PRIVATE_RULE"}]});
            let key = store.add_job("workflow",stage,"local","local",&snapshot,"failed").unwrap();
            let summary = store.job_summaries().unwrap().into_iter().find(|job|job["id"]==key).unwrap();
            assert_eq!(summary["snapshot"],json!({"runnerVersion":"desktop-runner/v1","stage":stage,
                "sourceVersion":9,"selection":{"episodeIds":["e1","e2"],"unitIds":["u1"]},
                "input":{"source":{"version":9,"sha256":"a".repeat(64)}}}));
            assert!(!summary.to_string().contains("SYNTHETIC_PRIVATE_"));
            assert_eq!(store.job(&key).unwrap()["snapshot"],snapshot,"projection never rewrites the frozen workflow");
        }
        let snapshot = json!({"runnerVersion":"desktop-runner/v1","stage":"foundation.run","sourceVersion":7,"selection":{"episodeIds":["legacy-e"]}});
        let key = store.add_job("workflow","foundation.run","local","local",&snapshot,"failed").unwrap();
        let summary = store.job_summaries().unwrap().into_iter().find(|job|job["id"]==key).unwrap();
        assert_eq!(summary["snapshot"]["selection"],snapshot["selection"]);
        assert_eq!(summary["snapshot"]["sourceVersion"],7);
        assert!(summary["snapshot"]["input"]["source"]["version"].is_null());
        assert!(summary["snapshot"]["input"]["source"]["sha256"].is_null());
    }
    #[test]
    fn summaries_are_compact_but_old_pending_identities_are_not_capped_by_history() {
        let (_tmp, store) = fixture();
        let old=store.add_job("video","old-unit","mock","account",&json!({"autoSubmit":true,"assetVersion":3,"model":"m","unitVersion":1,"prompt":"material review body","references":[],"ruleSnapshot":{"content":"full rule retained only in detail"},"problems":[]}),"ready").unwrap();
        store.update_job(&old,"ready",&json!({"prepared":{"estimatedCost":2.0,"capabilityVersion":"1","wire":{"payload":"large wire detail"}}})).unwrap();
        let mut c = store.conn().unwrap();
        let tx = c.transaction().unwrap();
        for n in 0..2050 {
            tx.execute("INSERT INTO jobs(id,kind,entity_id,provider,state,account_id,snapshot,result,created_at,updated_at) VALUES(?1,'text',?1,'local','completed','local',?2,?3,?4,?4)",params![format!("historical-text-{n}"),json!({"model":"m","input":{"wholeSource":"large source body"},"ruleSnapshot":{"content":"large rules"}}).to_string(),json!({"raw":"complete historical response","actualCost":0.5}).to_string(),now()]).unwrap();
        }
        tx.commit().unwrap();
        assert!(!store.jobs().unwrap().iter().any(|j| s(j, "id") == old));
        assert_eq!(
            s(&store.job(&old).unwrap(), "id"),
            old,
            "direct identity lookup survives beyond latest 2000 history"
        );
        let pending = store.pending_jobs().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["snapshot"]["autoSubmit"], true);
        assert!(pending[0]["snapshot"].get("prompt").is_none());
        let summaries = store.job_summaries().unwrap();
        let selected = summaries.iter().find(|j| s(j, "id") == old).unwrap();
        assert_eq!(selected["snapshot"]["prompt"], "material review body");
        assert!(selected["snapshot"]["references"].is_array());
        assert!(selected["snapshot"].get("ruleSnapshot").is_none());
        assert!(selected["prepared"].get("wire").is_none());
        assert_eq!(summaries.len(), 101);
        assert!(summaries.iter().all(|j| j["result"].get("raw").is_none()));
        store.save_unit(json!({"id":"u","episodeId":"episode-1","prompt":"display prompt","shots":[{"picture":"full shot"}],"performance":{"large":"body"},"ruleSnapshots":["large"],"assetRefs":[]})).unwrap();
        let summary = store.unit_summaries().unwrap();
        assert_eq!(summary[0]["prompt"], "display prompt");
        assert!(summary[0].get("shots").is_none());
        assert!(summary[0].get("performance").is_none());
        assert!(store.get("units", "u").unwrap()["shots"].is_array());
    }
}
