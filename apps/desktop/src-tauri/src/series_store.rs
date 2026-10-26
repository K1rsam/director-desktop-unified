//! Durable same-version series delivery, separate from mutable workspace projections.
use crate::{
    pipeline,
    store::{parse, s, save_asset_tx, Result, Store},
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
#[path = "series_identities.rs"]
mod identities;
#[path = "series_reuse.rs"]
mod reuse;

pub(crate) fn supply_proof(delivery: &Value) -> Result<&Value> {
    let legacy=&delivery["adoptionReceipt"]["assetSupplement"];
    let correction=&delivery["adoptionReceipt"]["supplyContractCorrection"];
    if !legacy.is_null() && !correction.is_null() {
        return Err("原补料与合同纠正不能占用同一次采用".into());
    }
    if correction.is_null() {Ok(legacy)} else {Ok(correction)}
}

pub(crate) fn persist(
    tx: &Transaction<'_>,
    input: &Value,
    output: &Value,
    run: &str,
) -> Result<()> {
    let deliveries = output["seriesDeliveries"]
        .as_array()
        .ok_or("新分镜交付缺少同版资料")?;
    for delivery in deliveries {
        pipeline::series::validate_delivery(input, delivery)?;
        if same_publication(tx, delivery, run)? {
            continue;
        }
        validate_existing_supply(tx, input, delivery)?;
        insert_supplement_assets(tx, delivery, run)?;
        // Adopt only fields from the validated full design, preserving hand-written
        // image prompts, current media, identities and historical versions.
        for designed in delivery["bundle"]["assets"]
            .as_array()
            .ok_or("交付缺少资产设计")?
        {
            let proof = supply_proof(delivery)?;
            if proof["suppliedExistingAssets"]
                .as_array()
                .is_some_and(|a| a.iter().any(|a| a["id"] == designed["id"]))
            {
                let base = proof["identityReservations"]["assetStates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|a| a["assetId"] == designed["id"])
                    .unwrap();
                if base["designRevision"] == designed["designRevision"] {
                    continue;
                }
            }
            let raw: String = tx
                .query_row(
                    "SELECT data FROM assets WHERE id=?1",
                    [s(designed, "id")],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut current = parse(raw)?;
            if current["variantId"] != designed["variantId"] {
                return Err("完整设计与当前资产身份不符".into());
            }
            let before = current.clone();
            for key in [
                "adoptedDesign",
                "designRevision",
                "designStatus",
                "designDependencies",
                "importName",
                "filename",
                "placeholderOrdinal",
            ] {
                if let Some(value) = designed.get(key) {
                    current[key] = value.clone();
                }
            }
            current["workflowProfile"] = json!(pipeline::series::PROFILE);
            if current != before {
                current["expectedVersion"] = before["version"].clone();
                save_asset_tx(tx, current)?;
            }
        }
        let episode = s(delivery, "episodeId");
        for (key, value) in [
            (format!("series.delivery:{episode}"), delivery.clone()),
            (format!("series.input:{episode}"), input.clone()),
            (
                format!("series.publication:{episode}"),
                json!({"workflowId":run,"bundleSha256":delivery["adoptionReceipt"]["bundleSha256"]}),
            ),
        ] {
            tx.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value.to_string()]).map_err(|e|e.to_string())?;
        }
    }
    Ok(())
}

fn same_publication(tx: &Transaction<'_>, delivery: &Value, run: &str) -> Result<bool> {
    let read = |prefix: &str| -> Result<Option<Value>> {
        tx.query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("{prefix}:{}", s(delivery, "episodeId"))],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .map(parse)
        .transpose()
    };
    Ok(
        matches!((read("series.delivery")?,read("series.publication")?),(Some(old),Some(publication)) if old==*delivery && publication["workflowId"]==run && publication["bundleSha256"]==delivery["adoptionReceipt"]["bundleSha256"]),
    )
}

fn validate_existing_supply(tx: &Transaction<'_>, input:&Value, delivery: &Value) -> Result<()> {
    let proof = supply_proof(delivery)?;
    if proof["version"]=="series-asset-supplement/v3" {
        for revised in proof["revisedAssets"].as_array().ok_or("新版供料缺少同身份修订卡")? {
            let before=proof["beforeAssets"].as_array().ok_or("新版供料缺少原完整设计")?.iter().find(|a|a["id"]==revised["id"] && a["variantId"]==revised["variantId"]).ok_or("新版供料修订没有原身份")?;
            let(data,version):(String,i64)=tx.query_row("SELECT data,version FROM assets WHERE id=?1",[s(before,"id")],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"新版供料原资产已不存在")?;
            let current=parse(data)?;
            let frozen=proof["identityReservations"]["assetStates"].as_array().ok_or("新版供料缺少真实冻结资产状态")?.iter().find(|a|a["assetId"]==before["id"]).ok_or("新版供料修订未绑定冻结资产状态")?;
            if crate::store::asset_deleted(&current) || current["version"]!=version || before["version"]!=version || current["variantId"]!=before["variantId"]
                || pipeline::series::supplement_supply_state(&current,s(&proof["identityReservations"],"sourceSha256"))!=*frozen {
                return Err("新版供料修订期间资产版本已变化；成果保留，未覆盖人工改动".into());
            }
            if current["designRevision"]!=before["designRevision"] || current["adoptedDesign"]!=before["adoptedDesign"] {
                // The original complete design may have passed independent review
                // earlier in this same unpublished episode. Prove that exact
                // predecessor; do not pretend it was already written to assets.
                pipeline::series::validate_unpublished_design_predecessor(input,delivery,before)?;
            }
        }
    }
    if proof["version"]=="series-supply-contract-correction/v1" {
        for target in proof["context"]["authorization"]["affectedAssets"].as_array().ok_or("合同纠正缺少原资产版本")? {
            let(data,version):(String,i64)=tx.query_row("SELECT data,version FROM assets WHERE id=?1",[s(target,"assetId")],|r|Ok((r.get(0)?,r.get(1)?)))
                .map_err(|_|"被纠正资产已不存在，未覆盖历史")?;
            let current=parse(data)?;
            if crate::store::asset_deleted(&current) || current["id"]!=target["assetId"] || current["variantId"]!=target["variantId"]
                || current["version"]!=version || target["baseAssetVersion"]!=version || current["designRevision"]!=target["baseDesignRevision"]
                || pipeline::digest(&current["adoptedDesign"].to_string())!=s(target,"baseDesignSha256") {
                return Err("合同纠正期间原资产或完整设计版本已变化；成果保留，未覆盖人工改动".into());
            }
        }
    }
    let Some(supplied) = proof["suppliedExistingAssets"].as_array() else {
        return Ok(());
    };
    for card in supplied {
        let frozen = proof["identityReservations"]["assetStates"]
            .as_array()
            .ok_or("既有供卡缺少冻结版本")?
            .iter()
            .find(|a| a["assetId"] == card["id"])
            .ok_or("既有供卡身份未冻结")?;
        let (data, version): (String, i64) = tx
            .query_row(
                "SELECT data,version FROM assets WHERE id=?1",
                [s(card, "id")],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "既有供卡已不存在，成果保留，未覆盖历史设计".to_string())?;
        let mut current = parse(data)?;
        current["version"] = json!(version);
        // The source hash belongs to the frozen supplement, never to mutable UI state.
        if crate::store::asset_deleted(&current)
            || pipeline::series::supplement_supply_state(
                &current,
                s(&proof["identityReservations"], "sourceSha256"),
            ) != *frozen
        {
            return Err(
                "既有供卡资产版本或完整设计已变化；成果保留，未覆盖人工提示词、图片和历史设计"
                    .into(),
            );
        }
    }
    Ok(())
}

/// Called only inside the already guarded final publication transaction.
/// Old IDs, names, files and placeholder ordinals remain permanently reserved.
fn insert_supplement_assets(tx: &Transaction<'_>, delivery: &Value, run: &str) -> Result<()> {
    let proof = supply_proof(delivery)?;
    if proof.is_null() {
        return Ok(());
    }
    let added = proof["addedAssets"]
        .as_array()
        .ok_or("补料缺少已复核新增资产")?;
    if added.is_empty() {
        return Ok(());
    }
    let saved_delivery: Option<String> = tx
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("series.delivery:{}", s(delivery, "episodeId"))],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let saved_publication: Option<String> = tx
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            [format!("series.publication:{}", s(delivery, "episodeId"))],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let (Some(old), Some(publication)) = (saved_delivery, saved_publication) {
        let publication = parse(publication)?;
        if parse(old)? == *delivery
            && publication["workflowId"] == run
            && publication["bundleSha256"] == delivery["adoptionReceipt"]["bundleSha256"]
        {
            return Ok(());
        }
    }
    let mut query = tx
        .prepare("SELECT data FROM assets UNION ALL SELECT data FROM asset_versions")
        .map_err(|e| e.to_string())?;
    let history = query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(query);
    let history = history.into_iter().map(parse).collect::<Result<Vec<_>>>()?;
    let saved: Option<String> = tx
        .query_row(
            "SELECT value FROM metadata WHERE key='series.placeholder.nextOrdinal'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let mut next = saved
        .map(parse)
        .transpose()?
        .map(|v| {
            v.as_u64()
                .filter(|n| *n <= 5701)
                .ok_or("补料序号高水位无效".to_string())
        })
        .transpose()?
        .unwrap_or(0);
    for old in &history {
        if let Some(n) = old["placeholderOrdinal"].as_u64() {
            next = next.max(n + 1);
        }
    }
    let identities = &proof["identityReservations"];
    if identities["nextOrdinal"] != next {
        return Err("补料期间历史序号预留发生变化；成果保留，未覆盖或回收旧身份".into());
    }
    for reserved in identities["reservedAssets"]
        .as_array()
        .ok_or("补料缺少冻结历史身份")?
    {
        if !history.iter().any(|old| {
            pipeline::series::supplement_identity_reservation(old).as_ref() == Some(reserved)
        }) {
            return Err("补料冻结历史身份与正式版本记录不符，未插入资产".into());
        }
    }
    let mut candidates = added.clone();
    candidates.sort_by_key(|a| a["placeholderOrdinal"].as_u64().unwrap_or(u64::MAX));
    for a in &candidates {
        let ordinal = a["placeholderOrdinal"]
            .as_u64()
            .filter(|n| *n <= 5700)
            .ok_or("补料序号无效")?;
        if ordinal != next {
            return Err("补料期间历史序号预留发生变化；成果保留，未覆盖或回收旧身份".into());
        }
        if history.iter().any(|old| {
            old["id"] == a["id"]
                || (!s(old, "filename").is_empty()
                    && s(old, "filename").eq_ignore_ascii_case(s(a, "filename")))
                || (!s(old, "importName").is_empty()
                    && old["kind"] == a["kind"]
                    && s(old, "importName").eq_ignore_ascii_case(s(a, "importName")))
        }) {
            return Err("补料身份与已有或历史资产冲突；未覆盖或重复生成".into());
        }
        next += 1;
    }
    for a in candidates {
        // The absence check and insert share IMMEDIATE publication lock. The
        // generic saver cannot upsert an unrelated historical identity here.
        save_asset_tx(tx, a)?;
    }
    tx.execute("INSERT INTO metadata(key,value) VALUES('series.placeholder.nextOrdinal',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[next.to_string()]).map_err(|e|e.to_string())?;
    Ok(())
}

impl Store {
    /// Resolve source-backed supply once per unit. Image/media-only revisions
    /// may advance the asset version, but the exact published design, original
    /// applicability and identity must still match their historical version.
    pub(crate) fn series_supplied_unit_scopes(
        &self,
        unit: &Value,
    ) -> Result<HashMap<String, Value>> {
        let mut result = HashMap::new();
        if s(unit, "episodeId").is_empty() || s(unit, "origin") == "imported-storyboard" {
            return Ok(result);
        }
        let mut connection = self.conn()?;
        let tx = connection.transaction().map_err(|e| e.to_string())?;
        let read = |prefix: &str| -> Result<Option<Value>> {
            tx.query_row(
                "SELECT value FROM metadata WHERE key=?1",
                [format!("{prefix}:{}", s(unit, "episodeId"))],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .map(parse)
            .transpose()
        };
        let Some(delivery) = read("series.delivery")? else {
            return Ok(result);
        };
        let proof = supply_proof(&delivery)?;
        if !["series-asset-supplement/v2","series-asset-supplement/v3","series-supply-contract-correction/v1"].contains(&s(proof,"version")) {
            return Ok(result);
        }
        let supplied = proof["suppliedExistingAssets"]
            .as_array()
            .ok_or("补供缺少已采用资产")?;
        let refs: Vec<_> = unit["assetRefs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| supplied.iter().any(|a| a["id"] == r["assetId"]))
            .collect();
        if refs.is_empty() {
            return Ok(result);
        }
        let input = read("series.input")?.ok_or("补供缺少冻结输入")?;
        let publication = read("series.publication")?.ok_or("补供缺少发布回执")?;
        let source = parse(
            tx.query_row(
                "SELECT data FROM sources ORDER BY version DESC LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())?,
        )?;
        if source["sha256"] != input["source"]["sha256"]
            || unit["sourceVersion"] != source["version"]
            || unit["retired"] == true
            || unit["continuityStale"] == true
            || s(&publication, "workflowId").is_empty()
            || publication["bundleSha256"] != delivery["adoptionReceipt"]["bundleSha256"]
        {
            return Err("补供原文、单元版本或发布身份已变化，未扩展资产范围".into());
        }
        let adopted = delivery["bundle"]["draft"]["units"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|u| u["id"] == unit["id"])
            .ok_or("补供单元未绑定已采用稿")?;
        if ["episodeId", "sceneId", "sourceRefs"]
            .iter()
            .any(|key| adopted[*key] != unit[*key])
        {
            return Err("补供单元集场或原文范围已变化，未扩展资产范围".into());
        }
        for r in &refs {
            if !adopted["assetRefs"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|old| {
                    ["assetId", "variantId", "sourceRefs", "use"]
                        .iter()
                        .all(|key| old[*key] == r[*key])
                })
            {
                return Err("补供引用已变化，未按名称或新引用猜测范围".into());
            }
        }
        let projected = pipeline::series::validated_supplied_scope_assets(&input, &delivery)?;
        crate::workflow::verify_series_receipts(
            &tx,
            s(&publication, "workflowId"),
            &json!({"seriesDeliveries":[delivery.clone()]}),
        )?;
        for r in refs {
            let card = supplied.iter().find(|a| a["id"] == r["assetId"]).unwrap();
            let scope = projected
                .iter()
                .find(|a| a["id"] == card["id"])
                .ok_or("补供缺少对应范围证明")?;
            let frozen = proof["identityReservations"]["assetStates"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|a| a["assetId"] == card["id"])
                .ok_or("补供缺少冻结资产版本")?;
            let base_version = frozen["assetVersion"].as_u64().ok_or("补供资产版本无效")?;
            let adopted_version =
                base_version + u64::from(card["designRevision"] != frozen["designRevision"]);
            let (data, version): (String, i64) = tx
                .query_row(
                    "SELECT data,version FROM assets WHERE id=?1",
                    [s(card, "id")],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| e.to_string())?;
            let current = parse(data)?;
            let old = parse(
                tx.query_row(
                    "SELECT data FROM asset_versions WHERE asset_id=?1 AND version=?2",
                    params![s(card, "id"), adopted_version as i64],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|_| "补供已发布资产历史版本缺失".to_string())?,
            )?;
            let same_card = |asset: &Value| {
                let mut value = pipeline::series::supplement_supply_card(asset);
                value["version"] = card["version"].clone();
                value == *card
            };
            if version < adopted_version as i64
                || crate::store::asset_deleted(&current)
                || !same_card(&old)
                || !same_card(&current)
            {
                return Err("补供资产身份、适用声明或采用设计已变化，未套用历史范围".into());
            }
            // This unit must be in an evidence-bearing scene, not just in the
            // same episode. The original global asset remains untouched.
            if scope["applicability"]["scenes"]
                .as_array()
                .is_some_and(|v| v.contains(&unit["sceneId"]))
            {
                result.insert(
                    s(card, "id").to_owned(),
                    json!({"applicability":scope["applicability"],"scope":scope["scope"]}),
                );
            }
        }
        Ok(result)
    }
    pub fn export_series_delivery(&self, request: &Value) -> Result<Value> {
        let source = self.source()?;
        let selected = request["episodeIds"]
            .as_array()
            .filter(|ids| !ids.is_empty())
            .ok_or("请选择要导出的已采用集数")?;
        let mut deliveries = Vec::new();
        let mut frozen_inputs = Vec::new();
        let mut frozen = None;
        let current = self.values("units")?;
        for episode in source["episodes"]
            .as_array()
            .ok_or("当前项目没有锁定剧本")?
        {
            if !selected.contains(&episode["id"]) {
                continue;
            }
            let id = s(episode, "id");
            let c = self.conn()?;
            let read = |prefix: &str| -> Result<Value> {
                let raw: String = c
                    .query_row(
                        "SELECT value FROM metadata WHERE key=?1",
                        [format!("{prefix}:{id}")],
                        |r| r.get(0),
                    )
                    .map_err(|_| format!("{id} 尚无有效的新流程同版交付"))?;
                parse(raw)
            };
            let delivery = read("series.delivery")?;
            let input = read("series.input")?;
            pipeline::series::validate_delivery(&input, &delivery)?;
            if input["source"]["sha256"] != source["sha256"] {
                return Err("原文已更新，请核对采用稿后导出".into());
            }
            let units = delivery["bundle"]["draft"]["units"]
                .as_array()
                .ok_or("交付稿缺少单元")?;
            let actual: Vec<_> = current
                .iter()
                .filter(|u| u["episodeId"] == episode["id"] && u["retired"] != true)
                .collect();
            if actual.len() != units.len()
                || units.iter().any(|u| {
                    !actual.iter().any(|a| {
                        u.as_object().is_some_and(|fields| {
                            fields.iter().all(|(key, value)| a.get(key) == Some(value))
                        })
                    })
                })
            {
                return Err(format!(
                    "{id} 当前稿已修改，独审尚未绑定新版本；保留原采用包，未冒称最新交付"
                ));
            }
            if let Some(first) = &frozen {
                let first: &Value = first;
                if first["productionScope"] != input["productionScope"]
                    || first["seriesRulePackSha256"] != input["seriesRulePackSha256"]
                {
                    return Err("所选集数来自不同冻结制作范围，请分别导出".into());
                }
            } else {
                frozen = Some(input.clone());
            }
            frozen_inputs.push(input);
            deliveries.push(delivery);
        }
        if deliveries.len() != selected.len() {
            return Err("导出集数重复或不在锁定原文中".into());
        }
        let target = Path::new(s(request, "path"));
        if !target.is_absolute() || !target.is_dir() {
            return Err("请选择已有本地导出目录".into());
        }
        let child = target.join(format!(
            "分镜交付-{}-{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            &crate::store::id()[..8]
        ));
        let mut export_input = frozen.ok_or("尚无可导出集稿")?;
        export_input["selection"]["episodeIds"] = json!(deliveries
            .iter()
            .map(|d| d["episodeId"].clone())
            .collect::<Vec<_>>());
        pipeline::series::export_delivery_package_with_inputs(
            &export_input,
            &deliveries,
            &frozen_inputs,
            &child,
        )
    }
}
