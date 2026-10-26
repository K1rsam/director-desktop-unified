//! Reuse adopted episode prefixes before creating a new workflow. This never
//! copies old role receipts into the new workflow or rewrites frozen requests.
use crate::{
    pipeline::series,
    store::{asset_deleted, parse, s, Result, Store},
};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

impl Store {
    pub(crate) fn reusable_series_episode_ids(&self, input: &Value) -> Result<Vec<String>> {
        let selected = series::selected_episode_ids(input)?;
        let mut connection = self.conn()?;
        let tx = connection.transaction().map_err(|e| e.to_string())?;
        let source: String = tx
            .query_row(
                "SELECT data FROM sources ORDER BY version DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if parse(source)? != input["source"] {
            return Err("SERIES_REUSE_SOURCE_CHANGED:原文已变化，请重新读取项目后继续".into());
        }
        let read_values = |table: &str| -> Result<Vec<Value>> {
            // Call sites supply only these two fixed internal table names.
            let sql = match table {
                "units" => "SELECT data FROM units ORDER BY rowid",
                "assets" => "SELECT data FROM assets ORDER BY rowid",
                _ => return Err("SERIES_REUSE_INTERNAL_TABLE".into()),
            };
            let mut q = tx.prepare(sql).map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.map(|r| parse(r.map_err(|e| e.to_string())?)).collect()
        };
        let units = read_values("units")?;
        let mut current = input.clone();
        current["assets"] = json!(read_values("assets")?
            .into_iter()
            .filter(|a| !asset_deleted(a))
            .collect::<Vec<_>>());
        let read = |prefix: &str, episode: &str| -> Result<Option<Value>> {
            tx.query_row(
                "SELECT value FROM metadata WHERE key=?1",
                [format!("{prefix}:{episode}")],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .map(parse)
            .transpose()
        };
        let mut reusable = Vec::new();
        let mut entry = input
            .get("previousBoundary")
            .cloned()
            .unwrap_or(Value::Null);
        for episode in selected {
            let Some(delivery) = read("series.delivery", &episode)? else {
                break;
            };
            let frozen =
                read("series.input", &episode)?.ok_or("SERIES_REUSE_FROZEN_INPUT_MISSING")?;
            let publication =
                read("series.publication", &episode)?.ok_or("SERIES_REUSE_PUBLICATION_MISSING")?;
            if s(&publication, "workflowId").is_empty()
                || publication["bundleSha256"] != delivery["adoptionReceipt"]["bundleSha256"]
            {
                return Err("SERIES_REUSE_PUBLICATION_MISMATCH".into());
            }
            crate::workflow::verify_series_receipts(
                &tx,
                s(&publication, "workflowId"),
                &json!({"seriesDeliveries":[delivery.clone()]}),
            )?;
            let adopted = delivery["bundle"]["draft"]["units"]
                .as_array()
                .ok_or("SERIES_REUSE_ADOPTED_UNITS_MISSING")?;
            let actual: Vec<_> = units
                .iter()
                .filter(|u| u["episodeId"] == episode && u["retired"] != true)
                .collect();
            if actual.len() != adopted.len()
                || adopted.iter().any(|unit| {
                    !actual
                        .iter()
                        .any(|now| series::value_is_projection(unit, now))
                })
            {
                return Err(format!("SERIES_REUSE_CURRENT_DRAFT_CHANGED:{episode}:当前稿已修改，保留人工修改及原采用稿，不能自动覆盖或重复制作"));
            }
            if actual.iter().any(|unit| unit["continuityStale"] == true) {
                break;
            }
            if !series::delivery_inputs_reusable(&current, &frozen, &delivery, &entry)? {
                break;
            }
            entry = adopted.last().cloned().unwrap_or(Value::Null);
            reusable.push(episode);
        }
        Ok(reusable)
    }
}
