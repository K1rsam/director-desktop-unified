//! Permanent desktop appearance -> lite A-ID map only. No design adoption,
//! semantic revision, export naming, media, or task execution lives here.
//! Existing databases are not migrated by reads. Install explicitly in a caller
//! transaction, then reserve alongside the caller's ordinary asset operation.
use crate::pipeline::digest;
use crate::store::{asset_deleted,Result};
use rusqlite::{params,Connection,OptionalExtension,Transaction};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};

pub(crate) const VERSION:&str="series-lite-permanent-identity/v1";
const MAX_SAFE:u64=9_007_199_254_740_991;
fn sql(e:rusqlite::Error)->String {format!("LITE_IDENTITY_STORAGE:{e}")}
fn text(s:&str)->bool {!s.trim().is_empty()&&!s.chars().any(char::is_control)}
fn stringify<T:Serialize>(value:&T)->Result<String>{serde_json::to_string(value).map_err(|e|e.to_string())}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
pub(crate) struct Identity {
    pub desktop_asset_id:String,
    pub variant_id:String,
    pub identity_id:Option<String>,
    pub base_asset_id:Option<String>,
    pub kind:String,
}
impl Identity {
    fn validate(&self)->Result<()> {
        if !text(&self.desktop_asset_id)||!text(&self.variant_id)||!text(&self.kind)
            ||self.identity_id.as_deref().is_some_and(|s|!text(s))
            ||self.base_asset_id.as_deref().is_some_and(|s|!text(s)) {
            return Err("LITE_IDENTITY_INVALID".into());
        }
        Ok(())
    }
    fn from_asset(data:&Value)->Result<Self> {
        let required=|key|data[key].as_str().map(str::to_owned).ok_or("LITE_IDENTITY_ASSET_FIELD_MISSING".to_owned());
        let optional=|key|match data.get(key) {None|Some(Value::Null)=>Ok(None),Some(Value::String(s))=>Ok(Some(s.clone())),_=>Err("LITE_IDENTITY_ASSET_FIELD_INVALID".to_owned())};
        let identity=Self{desktop_asset_id:required("id")?,variant_id:required("variantId")?,
            identity_id:optional("identityId")?,base_asset_id:optional("baseAssetId")?,kind:required("kind")?};
        identity.validate()?;Ok(identity)
    }
}

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
pub(crate) struct Mapping {
    pub registry_id:String,
    pub lite_asset_id:String,
    pub ordinal:u64,
    pub identity:Identity,
    /// Storage CAS evidence only; never a lite assetRevision/designRevision.
    pub first_asset_version:u64,
    /// Ever observed deleted. Restoration never clears or recycles this marker.
    pub tombstoned:bool,
}
impl Mapping {
    fn frozen(&self)->Value {json!({"version":VERSION,"registryId":self.registry_id,"liteAssetId":self.lite_asset_id,
        "ordinal":self.ordinal,"identity":self.identity,"firstAssetVersion":self.first_asset_version})}
    fn validate(&self)->Result<()> {
        self.identity.validate()?;
        if !text(&self.registry_id)||self.ordinal==0||self.ordinal>MAX_SAFE||self.first_asset_version==0
            ||self.lite_asset_id!=format!("A{:04}",self.ordinal) {return Err("LITE_IDENTITY_MAPPING_INVALID".into());}
        Ok(())
    }
}
#[derive(Clone,Debug,PartialEq,Eq)]
/// Identity-map CAS generation only, not the semantic series registry revision.
pub(crate) struct RegistryState {pub revision:u64,pub next_ordinal:u64}
#[derive(Clone,Debug,PartialEq,Eq)]
pub(crate) struct Reservation {
    pub mapping:Mapping,pub registry_revision:u64,pub created:bool,pub asset_version:u64,pub currently_deleted:bool,
}

pub(crate) fn install(tx:&Transaction<'_>)->Result<()> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS series_lite_identity_registry_v1(
        registry_id TEXT PRIMARY KEY, revision INTEGER NOT NULL CHECK(revision>=0), next_ordinal INTEGER NOT NULL CHECK(next_ordinal>=1));
        CREATE TABLE IF NOT EXISTS series_lite_identity_v1(
        registry_id TEXT NOT NULL REFERENCES series_lite_identity_registry_v1(registry_id), desktop_asset_id TEXT NOT NULL,
        lite_asset_id TEXT NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal>=1), identity_json TEXT NOT NULL,
        first_asset_version INTEGER NOT NULL CHECK(first_asset_version>=1), binding_sha256 TEXT NOT NULL,
        tombstoned INTEGER NOT NULL DEFAULT 0 CHECK(tombstoned IN (0,1)),
        PRIMARY KEY(registry_id,desktop_asset_id), UNIQUE(registry_id,lite_asset_id), UNIQUE(registry_id,ordinal));
        CREATE TRIGGER IF NOT EXISTS series_lite_identity_binding_immutable_v1 BEFORE UPDATE OF registry_id,desktop_asset_id,lite_asset_id,ordinal,identity_json,first_asset_version,binding_sha256 ON series_lite_identity_v1
        BEGIN SELECT RAISE(ABORT,'LITE_IDENTITY_BINDING_IMMUTABLE'); END;
        CREATE TRIGGER IF NOT EXISTS series_lite_identity_no_delete_v1 BEFORE DELETE ON series_lite_identity_v1
        BEGIN SELECT RAISE(ABORT,'LITE_IDENTITY_NO_RECYCLE'); END;
        CREATE TRIGGER IF NOT EXISTS series_lite_identity_tombstone_monotonic_v1 BEFORE UPDATE OF tombstoned ON series_lite_identity_v1 WHEN NEW.tombstoned<OLD.tombstoned
        BEGIN SELECT RAISE(ABORT,'LITE_IDENTITY_TOMBSTONE_IMMUTABLE'); END;") .map_err(sql)
}
fn installed(c:&Connection)->Result<bool> {
    let count:i64=c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('series_lite_identity_registry_v1','series_lite_identity_v1')",[],|r|r.get(0)).map_err(sql)?;
    match count {0=>Ok(false),2=>Ok(true),_=>Err("LITE_IDENTITY_SCHEMA_INCOMPLETE".into())}
}
/// Reads persisted mappings, including tombstones, without writing or adopting.
/// Current asset identity/version is rechecked by reserve_identity, not inferred.
pub(crate) fn read(c:&Connection,registry_id:&str,desktop_asset_id:&str)->Result<Option<Mapping>> {
    if !installed(c)? {return Ok(None);}
    let row:Option<(String,u64,String,u64,String,bool)>=c.query_row(
        "SELECT lite_asset_id,ordinal,identity_json,first_asset_version,binding_sha256,tombstoned FROM series_lite_identity_v1 WHERE registry_id=?1 AND desktop_asset_id=?2",
        params![registry_id,desktop_asset_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(sql)?;
    row.map(|(lite_asset_id,ordinal,body,first_asset_version,sha256,tombstoned)| {
        let mapping=Mapping{registry_id:registry_id.into(),lite_asset_id,ordinal,identity:serde_json::from_str(&body).map_err(|e|e.to_string())?,first_asset_version,tombstoned};
        mapping.validate()?;
        if mapping.identity.desktop_asset_id!=desktop_asset_id||digest(&mapping.frozen().to_string())!=sha256 {
            return Err("LITE_IDENTITY_BINDING_CHANGED".into());
        }
        Ok(mapping)
    }).transpose()
}
pub(crate) fn read_registry(c:&Connection,registry_id:&str)->Result<Option<RegistryState>> {
    if !installed(c)? {return Ok(None);}
    let state:Option<RegistryState>=c.query_row("SELECT revision,next_ordinal FROM series_lite_identity_registry_v1 WHERE registry_id=?1",[registry_id],
        |r|Ok(RegistryState{revision:r.get(0)?,next_ordinal:r.get(1)?})).optional().map_err(sql)?;
    let maximum:u64=c.query_row("SELECT COALESCE(MAX(ordinal),0) FROM series_lite_identity_v1 WHERE registry_id=?1",[registry_id],|r|r.get(0)).map_err(sql)?;
    if state.as_ref().is_some_and(|s|s.next_ordinal<=maximum||s.next_ordinal>MAX_SAFE+1||s.revision>MAX_SAFE)
        ||state.is_none()&&maximum!=0 {return Err("LITE_IDENTITY_HIGH_WATER_CONFLICT".into());}
    Ok(state)
}

fn current_asset(c:&Connection,identity:&Identity,expected:u64)->Result<bool> {
    let row:Option<(String,u64,String)>=c.query_row("SELECT variant_id,version,data FROM assets WHERE id=?1",[&identity.desktop_asset_id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
    let (variant,version,body)=row.ok_or("LITE_IDENTITY_ASSET_MISSING")?;
    let data:Value=serde_json::from_str(&body).map_err(|e|e.to_string())?;
    if expected==0||version!=expected||data["version"]!=version {return Err("LITE_IDENTITY_ASSET_CAS_CONFLICT".into());}
    if Identity::from_asset(&data)?!=*identity||variant!=identity.variant_id {return Err("LITE_IDENTITY_ASSET_BINDING_CHANGED".into());}
    Ok(asset_deleted(&data))
}

/// A-ID allocation is independent of names, sorting, and placeholderOrdinal.
/// Existing mappings survive prompt/media/use row-version changes; changing a
/// variant, owner identity, or type requires an explicit migration, never reuse.
pub(crate) fn reserve_identity(tx:&mut Transaction<'_>,registry_id:&str,expected_registry_revision:u64,
    identity:&Identity,expected_asset_version:u64)->Result<Reservation> {
    identity.validate()?;
    if !text(registry_id){return Err("LITE_IDENTITY_REGISTRY_INVALID".into());}
    let save=tx.savepoint().map_err(sql)?;
    if !installed(&save)? {return Err("LITE_IDENTITY_INSTALL_REQUIRED".into());}
    let deleted=current_asset(&save,identity,expected_asset_version)?;
    let state=read_registry(&save,registry_id)?;
    if let Some(mut mapping)=read(&save,registry_id,&identity.desktop_asset_id)? {
        if mapping.identity!=*identity {return Err("LITE_IDENTITY_PERMANENT_BINDING_CONFLICT".into());}
        let state=state.ok_or("LITE_IDENTITY_REGISTRY_MISSING")?;let mut revision=state.revision;
        if deleted&&!mapping.tombstoned {
            if revision!=expected_registry_revision||revision>=MAX_SAFE {return Err("LITE_IDENTITY_REGISTRY_CAS_CONFLICT".into());}
            let n=save.execute("UPDATE series_lite_identity_registry_v1 SET revision=revision+1 WHERE registry_id=?1 AND revision=?2",params![registry_id,revision]).map_err(sql)?;
            if n!=1{return Err("LITE_IDENTITY_REGISTRY_CAS_CONFLICT".into());}
            save.execute("UPDATE series_lite_identity_v1 SET tombstoned=1 WHERE registry_id=?1 AND desktop_asset_id=?2 AND tombstoned=0",params![registry_id,identity.desktop_asset_id]).map_err(sql)?;
            mapping.tombstoned=true;revision+=1;
        }
        save.commit().map_err(sql)?;
        return Ok(Reservation{mapping,registry_revision:revision,created:false,asset_version:expected_asset_version,currently_deleted:deleted});
    }
    if deleted{return Err("LITE_IDENTITY_DELETED_UNMAPPED".into());}
    let state=state.unwrap_or(RegistryState{revision:0,next_ordinal:1});
    if state.revision!=expected_registry_revision||state.revision>=MAX_SAFE||state.next_ordinal>MAX_SAFE {
        return Err("LITE_IDENTITY_REGISTRY_CAS_CONFLICT".into());
    }
    save.execute("INSERT INTO series_lite_identity_registry_v1(registry_id,revision,next_ordinal) VALUES(?1,0,1) ON CONFLICT(registry_id) DO NOTHING",[registry_id]).map_err(sql)?;
    let n=save.execute("UPDATE series_lite_identity_registry_v1 SET revision=revision+1,next_ordinal=next_ordinal+1 WHERE registry_id=?1 AND revision=?2 AND next_ordinal=?3",
        params![registry_id,state.revision,state.next_ordinal]).map_err(sql)?;
    if n!=1{return Err("LITE_IDENTITY_REGISTRY_CAS_CONFLICT".into());}
    let mapping=Mapping{registry_id:registry_id.into(),lite_asset_id:format!("A{:04}",state.next_ordinal),ordinal:state.next_ordinal,
        identity:identity.clone(),first_asset_version:expected_asset_version,tombstoned:false};
    save.execute("INSERT INTO series_lite_identity_v1(registry_id,desktop_asset_id,lite_asset_id,ordinal,identity_json,first_asset_version,binding_sha256) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![registry_id,identity.desktop_asset_id,mapping.lite_asset_id,mapping.ordinal,stringify(identity)?,expected_asset_version,digest(&mapping.frozen().to_string())]).map_err(sql)?;
    save.commit().map_err(sql)?;
    Ok(Reservation{mapping,registry_revision:state.revision+1,created:true,asset_version:expected_asset_version,currently_deleted:false})
}

#[cfg(test)]
#[path="series_lite_identity_store_tests.rs"]
mod tests;
