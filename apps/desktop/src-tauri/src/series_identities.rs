//! Persistent image identity high-water, including deleted assets and snapshots.
//! Run only when preparing genuinely new production, never a frozen recovery.
use crate::store::{asset_deleted, parse, s, save_asset_tx, Result, Store};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

const KEY: &str = "series.placeholder.nextOrdinal";
impl Store {
    pub(crate) fn ensure_series_asset_identities(&self) -> Result<()> {
        let mut connection = self.conn()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let saved: Option<String> = tx
            .query_row("SELECT value FROM metadata WHERE key=?1", [KEY], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        let mut next = match saved {
            Some(raw) => parse(raw)?
                .as_u64()
                .filter(|v| *v <= 5701)
                .ok_or("SERIES_PLACEHOLDER_HIGH_WATER_INVALID")?,
            None => 0,
        };
        let mut owners = HashMap::<u64, String>::new();
        let mut historical = HashMap::<String, u64>::new();
        let mut mappings = HashMap::<String, Value>::new();
        // This is deliberately not Store::values("assets"), which hides deletion
        // tombstones. Old snapshots remain authority for occupied ordinals.
        let mut query=tx.prepare("SELECT data FROM asset_versions WHERE json_type(data,'$.placeholderOrdinal') IS NOT NULL UNION ALL SELECT data FROM assets WHERE json_type(data,'$.placeholderOrdinal') IS NOT NULL").map_err(|e|e.to_string())?;
        let records = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(query);
        for raw in records {
            let asset = parse(raw)?;
            if asset["placeholderOrdinal"].is_null() {
                continue;
            }
            let id = s(&asset, "id");
            let ordinal = asset["placeholderOrdinal"]
                .as_u64()
                .filter(|v| *v <= 5700)
                .ok_or("SERIES_HISTORICAL_ORDINAL_INVALID")?;
            if id.is_empty() {
                return Err("SERIES_HISTORICAL_IDENTITY_MISSING".into());
            }
            if owners.get(&ordinal).is_some_and(|owner| owner != id) {
                return Err(format!(
                    "SERIES_HISTORICAL_ORDINAL_COLLISION:{ordinal}:{id}"
                ));
            }
            if historical.get(id).is_some_and(|old| *old != ordinal) {
                return Err(format!("SERIES_HISTORICAL_ORDINAL_CHANGED:{id}"));
            }
            owners.insert(ordinal, id.into());
            historical.insert(id.into(), ordinal);
            next = next.max(ordinal + 1);
            if !s(&asset, "importName").is_empty() && !s(&asset, "filename").is_empty() {
                if let Some(prior) = mappings.get(id) {
                    if prior["importName"] != asset["importName"]
                        || prior["filename"] != asset["filename"]
                    {
                        return Err(format!("SERIES_HISTORICAL_FILENAME_CHANGED:{id}"));
                    }
                }
                mappings.insert(id.into(), asset.clone());
            }
        }
        let mut query = tx
            .prepare("SELECT data FROM assets ORDER BY rowid")
            .map_err(|e| e.to_string())?;
        let records = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(query);
        let all = records.into_iter().map(parse).collect::<Result<Vec<_>>>()?;
        let mut active: Vec<Value> = all.iter().filter(|a| !asset_deleted(a)).cloned().collect();
        for asset in &mut active {
            if asset.get("placeholderOrdinal").is_none_or(Value::is_null) {
                if let Some(ordinal) = historical.get(s(asset, "id")) {
                    asset["placeholderOrdinal"] = json!(ordinal);
                }
            }
            if let Some(mapping) = mappings.get(s(asset, "id")) {
                for key in ["importName", "filename"] {
                    if s(asset, key).is_empty() {
                        asset[key] = mapping[key].clone();
                    } else if asset[key] != mapping[key] {
                        return Err(format!(
                            "SERIES_HISTORICAL_FILENAME_CHANGED:{}",
                            s(asset, "id")
                        ));
                    }
                }
            }
        }
        // Deleted/archived image identities still reserve their import names.
        // Restoring an old asset must never collide with a later new identity.
        let mut reservations = active.clone();
        for (id, mapping) in &mappings {
            if !active.iter().any(|asset| s(asset, "id") == id) {
                reservations.push(mapping.clone());
            }
        }
        next = crate::pipeline::series::allocate_asset_identities_from(&mut reservations, next)?;
        active = reservations
            .into_iter()
            .filter(|a| {
                all.iter()
                    .any(|old| old["id"] == a["id"] && !asset_deleted(old))
            })
            .collect();
        for asset in active {
            if let Some(ordinal) = asset["placeholderOrdinal"].as_u64() {
                if owners
                    .get(&ordinal)
                    .is_some_and(|owner| owner != s(&asset, "id"))
                {
                    return Err("SERIES_RESERVED_ORDINAL_REASSIGNMENT".into());
                }
            }
            let before = all
                .iter()
                .find(|a| a["id"] == asset["id"])
                .ok_or("SERIES_IDENTITY_PREIMAGE_MISSING")?;
            if before != &asset {
                let mut value = asset;
                value["expectedVersion"] = before["version"].clone();
                save_asset_tx(&tx, value)?;
            }
        }
        tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value<>excluded.value",params![KEY,next.to_string()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_owned()).unwrap();
        store.create("序号高水位离线验收", "ordinal-test").unwrap();
        (dir, store)
    }
    fn asset(id: &str) -> Value {
        json!({"id":id,"variantId":"base","name":id,"kind":"prop"})
    }
    #[test]
    fn deleting_highest_asset_does_not_reuse_its_ordinal_after_reopen() {
        let (dir, store) = store();
        store.save_asset(asset("a")).unwrap();
        store.save_asset(asset("b")).unwrap();
        store.ensure_series_asset_identities().unwrap();
        let old = store.get("assets", "b").unwrap();
        assert_eq!(old["placeholderOrdinal"], 1);
        store
            .set_asset_deleted("b", old["version"].as_i64().unwrap(), true)
            .unwrap();
        let reopened = Store::open(dir.path().to_owned()).unwrap();
        reopened.save_asset(asset("c")).unwrap();
        reopened.ensure_series_asset_identities().unwrap();
        assert_eq!(
            reopened.get("assets", "c").unwrap()["placeholderOrdinal"],
            2
        );
        let deleted = reopened.get("assets", "b").unwrap();
        reopened
            .set_asset_deleted("b", deleted["version"].as_i64().unwrap(), false)
            .unwrap();
        reopened.ensure_series_asset_identities().unwrap();
        assert_eq!(
            reopened.get("assets", "b").unwrap()["placeholderOrdinal"],
            1
        );
        let before = reopened.values("assets").unwrap();
        reopened.ensure_series_asset_identities().unwrap();
        assert_eq!(
            before,
            reopened.values("assets").unwrap(),
            "idempotent reservation creates no new asset revisions"
        );
    }
    #[test]
    fn missing_high_water_is_reconstructed_from_history_without_recycling() {
        let (_dir, store) = store();
        let mut historical = asset("a");
        historical["placeholderOrdinal"] = json!(90);
        store.save_asset(historical).unwrap();
        store.save_asset(asset("a")).unwrap();
        store.save_asset(asset("b")).unwrap();
        store.ensure_series_asset_identities().unwrap();
        assert_eq!(store.get("assets", "a").unwrap()["placeholderOrdinal"], 90);
        assert_eq!(store.get("assets", "b").unwrap()["placeholderOrdinal"], 91);
    }
    #[test]
    fn conflicting_historical_owner_rolls_back_new_identity_reservations() {
        let (_dir, store) = store();
        let mut first = asset("a");
        first["placeholderOrdinal"] = json!(3);
        store.save_asset(first).unwrap();
        let mut second = asset("b");
        second["placeholderOrdinal"] = json!(3);
        store.save_asset(second).unwrap();
        let before = store.values("assets").unwrap();
        assert!(store
            .ensure_series_asset_identities()
            .unwrap_err()
            .contains("COLLISION"));
        assert_eq!(before, store.values("assets").unwrap());
    }
    #[test]
    fn deleted_filename_is_reserved_and_missing_mapping_recovers_from_history() {
        let (dir, store) = store();
        let mut first = asset("first");
        first["name"] = json!("同名人物");
        first["kind"] = json!("CH");
        store.save_asset(first).unwrap();
        store.ensure_series_asset_identities().unwrap();
        let original = store.get("assets", "first").unwrap();
        store
            .set_asset_deleted("first", original["version"].as_i64().unwrap(), true)
            .unwrap();
        let mut second = asset("second");
        second["name"] = json!("同名人物");
        second["kind"] = json!("CH");
        store.save_asset(second).unwrap();
        let reopened = Store::open(dir.path().to_owned()).unwrap();
        reopened.ensure_series_asset_identities().unwrap();
        let newer = reopened.get("assets", "second").unwrap();
        assert_ne!(newer["filename"], original["filename"]);
        let deleted = reopened.get("assets", "first").unwrap();
        reopened
            .set_asset_deleted("first", deleted["version"].as_i64().unwrap(), false)
            .unwrap();
        let mut lost = reopened.get("assets", "first").unwrap();
        lost.as_object_mut().unwrap().remove("importName");
        lost.as_object_mut().unwrap().remove("filename");
        reopened.save_asset(lost).unwrap();
        reopened.ensure_series_asset_identities().unwrap();
        let restored = reopened.get("assets", "first").unwrap();
        assert_eq!(restored["filename"], original["filename"]);
        assert_eq!(
            restored["placeholderOrdinal"],
            original["placeholderOrdinal"]
        );
        assert_eq!(
            reopened.get("assets", "second").unwrap()["filename"],
            newer["filename"]
        );
    }
}
