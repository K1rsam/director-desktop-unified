//! Versioned publication guard for lite work. Storage versions for pictures,
//! prompts and unrelated catalogue entries are not text-design dependencies.
use crate::pipeline::digest;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, String>;
pub(crate) const VERSION: &str = "desktop-lite-consumption-guard/v1";
fn held(detail: &str) -> String { format!("LITE_INPUT_CHANGED: {detail}；原回包和候选保留") }
pub(crate) fn design_pin(asset: &Value) -> String {
    // A use-list append and a media/prompt edit are deliberately absent. A
    // direct design dependency is part of the adopted design itself.
    digest(&json!({"id":asset["id"],"variantId":asset["variantId"],
        "identityId":asset["identityId"],"baseAssetId":asset["baseAssetId"],
        "kind":asset["kind"],"deletedAt":asset["deletedAt"],
        "designRevision":asset["designRevision"],"adoptedDesign":asset["adoptedDesign"],
        "designDependencies":asset["designDependencies"]}).to_string())
}
/// Parent guard protects source, scope and writable units. The role reservation
/// adds only the full designs it really consumes, after any required supplement.
pub(crate) fn from_input(input: &Value) -> Value {
    let scope: BTreeSet<_> = input["selection"]["episodeIds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut units: BTreeMap<_,_> = input["existingUnits"].as_array().into_iter().flatten()
        .filter(|u| scope.contains(u["episodeId"].as_str().unwrap_or("")))
        .map(|u|(u["id"].as_str().unwrap_or("").to_owned(),u["version"].clone())).collect();
    if let Some(id)=input["previousBoundary"]["id"].as_str() {
        units.insert(id.into(),input["previousBoundary"]["version"].clone());
    }
    json!({"version":VERSION,"sourceVersion":input["source"]["version"],
        "sourceSha256":input["source"]["sha256"],"episodeIds":input["selection"]["episodeIds"],
        "units":units,"designs":{}})
}
pub(crate) fn with_designs(mut guard: Value, assets: &[Value]) -> Result<Value> {
    if guard["version"]!=VERSION {return Err(held("未知守卫版本"));}
    let mut pins=BTreeMap::new();
    for a in assets {
        let id=a["id"].as_str().filter(|s|!s.is_empty()).ok_or_else(||held("缺资产身份"))?;
        if pins.insert(id.to_owned(),design_pin(a)).is_some() {return Err(held("重复资产身份"));}
    }
    guard["designs"]=json!(pins);Ok(guard)
}
fn verify_connection(c:&Connection,g:&Value)->Result<()> {
    if g["version"]!=VERSION {return Err(held("未知守卫版本"));}
    let source:Option<(i64,String)>=c.query_row("SELECT version,sha256 FROM sources ORDER BY version DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    if !source.is_some_and(|(v,h)|g["sourceVersion"]==v&&g["sourceSha256"]==h) {return Err(held("原文版本变化"));}
    let scope=g["episodeIds"].as_array().ok_or_else(||held("缺冻结集范围"))?;
    let expected=g["units"].as_object().ok_or_else(||held("缺单元版本"))?;
    let mut actual=BTreeMap::new();
    let mut q=c.prepare("SELECT id,episode_id,version FROM units WHERE COALESCE(json_extract(data,'$.retired'),0)<>1").map_err(|e|e.to_string())?;
    let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?))).map_err(|e|e.to_string())?;
    for row in rows {
        let(id,ep,v)=row.map_err(|e|e.to_string())?;
        if scope.iter().any(|e|e==&ep)||expected.contains_key(&id) {actual.insert(id,json!(v));}
    }
    if json!(actual)!=g["units"] {return Err(held("实际集稿或人工编辑变化"));}
    for (id,pin) in g["designs"].as_object().ok_or_else(||held("缺消费设计清单"))? {
        let raw:Option<String>=c.query_row("SELECT data FROM assets WHERE id=?1",[id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let asset:Value=serde_json::from_str(&raw.ok_or_else(||held(&format!("消费资产已删除：{id}")))?).map_err(|e|e.to_string())?;
        if pin!=&json!(design_pin(&asset)) {return Err(held(&format!("消费设计已变化：{id}")));}
    }
    Ok(())
}
pub(crate) fn verify(tx:&Transaction<'_>,guard:&Value)->Result<()> {verify_connection(tx,guard)}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn only_consumed_designs_and_written_units_can_invalidate_text() {
        let mut c=Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE sources(version INTEGER,sha256 TEXT); CREATE TABLE units(id TEXT,episode_id TEXT,version INTEGER,data TEXT); CREATE TABLE assets(id TEXT,data TEXT);").unwrap();
        c.execute("INSERT INTO sources VALUES(1,?1)",["a".repeat(64)]).unwrap();
        let asset=json!({"id":"A","variantId":"V","version":1,"designRevision":1,"adoptedDesign":{"full":"真实完整设计"}});
        c.execute("INSERT INTO assets VALUES('A',?1)",[asset.to_string()]).unwrap();
        let input=json!({"source":{"version":1,"sha256":"a".repeat(64)},"selection":{"episodeIds":["E1"]},"existingUnits":[]});
        let g=with_designs(from_input(&input),&[asset.clone()]).unwrap();
        verify_connection(&c,&g).unwrap();
        let mut changed=asset.clone();changed["version"]=json!(7);changed["imagePrompt"]=json!("新制图词");changed["adoptedMediaId"]=json!("M2");changed["sourceRefs"]=json!(["later-use"]);
        c.execute("UPDATE assets SET data=?1 WHERE id='A'",[changed.to_string()]).unwrap();
        c.execute("INSERT INTO assets VALUES('unrelated','{}')",[]).unwrap();
        c.execute("INSERT INTO units VALUES('other','E2',1,'{}')",[]).unwrap();
        verify_connection(&c,&g).unwrap();
        changed["adoptedDesign"]["full"]=json!("实际变更");
        c.execute("UPDATE assets SET data=?1 WHERE id='A'",[changed.to_string()]).unwrap();
        assert!(verify_connection(&c,&g).unwrap_err().contains("消费设计已变化"));
        c.execute("UPDATE assets SET data=?1 WHERE id='A'",[asset.to_string()]).unwrap();
        c.execute("INSERT INTO units VALUES('manual','E1',1,'{}')",[]).unwrap();
        assert!(verify_connection(&c,&g).unwrap_err().contains("人工编辑变化"));
        let tx=c.transaction().unwrap();assert!(verify(&tx,&g).is_err());
    }
}
