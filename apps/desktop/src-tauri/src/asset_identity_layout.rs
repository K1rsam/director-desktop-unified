//! An inspected crop belongs to one real adopted image, never a guessed grid.
use crate::store::{mark_changed, parse, s, save_asset_tx, Result, Store};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

impl Store {
    pub fn save_identity_layout(&self, request: &Value) -> Result<Value> {
        if request["confirmed"] != true {
            return Err("请先查看母图并确认上排三脸区域".into());
        }
        let aid = s(request, "assetId");
        let variant = s(request, "variantId");
        let mid = s(request, "mediaId");
        let media = self.get("media", mid)?;
        let path = self.verify_media(&media)?;
        let decoded = image::open(&path).map_err(|e| e.to_string())?.to_rgba8();
        let (width, height) = decoded.dimensions();
        let top = request["topHeight"]
            .as_u64()
            .filter(|v| *v > 0 && *v < u64::from(height))
            .ok_or("请选择位于原图内的上排高度")?;
        if media["kind"] != "image"
            || width != height
            || width == 0
            || media["sha256"] != request["expectedSha256"]
            || decoded
                .pixels()
                .all(|p| p[3] == 0 || p.0[..3].iter().all(|v| *v >= 250))
        {
            return Err("身份布局需要当前真实1:1母图；图片已变化或仍为白色占位".into());
        }
        let mut connection = self.conn()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let raw: String = tx
            .query_row("SELECT data FROM assets WHERE id=?1", [aid], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut asset = parse(raw)?;
        if crate::store::asset_deleted(&asset)
            || asset["variantId"] != variant
            || !["character", "CH"].contains(&s(&asset, "kind"))
        {
            return Err("请选择当前人物形象的身份母版".into());
        }
        let adopted: Option<String> = tx
            .query_row(
                "SELECT media_id FROM adoptions WHERE asset_id=?1 AND variant_id=?2",
                params![aid, variant],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if adopted.as_deref() != Some(mid) {
            return Err("母图采用版本已变化，请重新打开当前图片后登记".into());
        }
        let layout = json!({"version":"asset-identity-layout/v1","mediaId":mid,"sha256":media["sha256"],"width":width,"height":height,
            "identityStrip":{"x":0,"y":0,"width":width,"height":top},"basis":"user-inspected","confirmed":true});
        asset["expectedVersion"] = asset["version"].clone();
        asset["identityLayout"] = layout.clone();
        save_asset_tx(&tx, asset)?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(layout)
    }
}

/// Resolve only while assembling a new send. A frozen request never rebinds.
pub fn freeze(store: &Store, snapshot: &mut Value) -> Result<()> {
    if !crate::asset_composite::requires_composition(snapshot) {
        return Ok(());
    }
    if snapshot["identityComposite"].is_object() {
        return crate::asset_composite::validate_snapshot(snapshot).map(|_| ());
    }
    let refs = snapshot["references"]
        .as_array()
        .ok_or("换装缺少真实母图")?;
    let requirements = snapshot["referenceRequirements"]
        .as_array()
        .ok_or("换装缺少明确身份母图需求")?;
    let identity: Vec<_> = refs
        .iter()
        .filter(|r| {
            requirements.iter().any(|need| {
                need["required"] != false
                    && need["targetAssetId"] == r["assetId"]
                    && ["face", "identity", "identity-faces"].contains(
                        &need["role"]
                            .as_str()
                            .or_else(|| need["kind"].as_str())
                            .unwrap_or(""),
                    )
            })
        })
        .collect();
    if identity.len() != 1 {
        return Err("换装须有且仅有一张明确的身份母图".into());
    }
    let reference = identity[0];
    let mother = store.active_asset(s(reference, "assetId"))?;
    let layout = &mother["identityLayout"];
    if layout["version"] != "asset-identity-layout/v1"
        || layout["confirmed"] != true
        || layout["mediaId"] != reference["mediaId"]
        || layout["sha256"] != reference["sha256"]
    {
        return Err(format!(
            "请在母图「{}」登记当前采用图的锁脸区域；未猜测裁切位置",
            s(&mother, "name")
        ));
    }
    snapshot["identityComposite"] = json!({"version":crate::asset_composite::LOWER_ADAPTIVE_PROFILE,"width":layout["width"],"height":layout["height"],
        "motherMediaId":reference["mediaId"],"motherSha256":reference["sha256"],"identityStrip":layout["identityStrip"],"layoutReceipt":layout});
    snapshot["identityComposite"]["lowerAdaptation"] =
        crate::asset_composite::lower_adaptation_contract(&snapshot["identityComposite"])?;
    crate::asset_composite::validate_snapshot(snapshot)?;
    let instructions=format!("\n原像素合成画布要求：输出1:1新服装板，可使用渠道原生正方形分辨率。顶部按原母图上排高度{}/总高度{}的比例保留空白槽位；只在其下方完整生成四个服装视图，不得裁断衣装或肢体。输出入库时本地先完全丢弃顶部，再仅缩放服装下排到{}×{}母图的下排槽位；母图三脸原像素直接复制、绝不缩放。禁止整板重绘身份母图。",layout["identityStrip"]["height"],layout["height"],layout["width"],layout["height"]);
    snapshot["prompt"] = json!(format!("{}{}", s(snapshot, "prompt"), instructions));
    Ok(())
}

/// Read-only mechanical evidence for prompt authors, never a visual review.
/// An absent/stale mother blocks image dispatch, not clothing description.
pub(crate) fn prompt_supply(store: &Store, asset: &Value) -> Value {
    let checked = (|| -> Result<Value> {
        let layout = &asset["identityLayout"];
        let media_id = s(asset, "adoptedMediaId");
        if media_id.is_empty() {
            return Err("母图尚未采用，实际像素布局在图片派发前登记".into());
        }
        if crate::asset_graph::image_adoption_blocked(asset) {
            return Err("母图采用状态待复核，派发前重新核对".into());
        }
        if layout["version"] != "asset-identity-layout/v1"
            || layout["confirmed"] != true
            || layout["mediaId"] != media_id
        {
            return Err("当前母图尚无同版已确认上排布局，派发前登记".into());
        }
        let media = store.get("media", media_id)?;
        let path = store.verify_media(&media)?;
        if layout["sha256"] != media["sha256"] {
            return Err("登记布局不属于当前母图文件，派发前重新登记".into());
        }
        let mut config = json!({"version":crate::asset_composite::LOWER_ADAPTIVE_PROFILE,
            "width":layout["width"],"height":layout["height"],"identityStrip":layout["identityStrip"],
            "motherMediaId":media_id,"motherSha256":media["sha256"]});
        config["lowerAdaptation"] = crate::asset_composite::lower_adaptation_contract(&config)?;
        let snapshot = json!({"identityComposite":config,"references":[{"mediaId":media_id,
            "assetId":asset["id"],"variantId":asset["variantId"],"kind":"image","path":path,"sha256":media["sha256"]}]});
        crate::asset_composite::validate_snapshot(&snapshot)?;
        Ok(
            json!({"version":"asset-inspected-layout-supply/v1","status":"registered",
            "assetId":asset["id"],"variantId":asset["variantId"],"mediaId":media_id,"sha256":media["sha256"],
            "width":layout["width"],"height":layout["height"],"identityStrip":layout["identityStrip"],
            "identityCopyMode":"full-width-strip-original-pixels","individualFaceSlotsRequired":false,
            "registrationBasis":layout["basis"],"fileBytesAndDimensionsVerified":true,
            "imageEvidence":"NO_PIXELS_READ","visualQualityApproved":false}),
        )
    })();
    checked.unwrap_or_else(|reason| {
        json!({"version":"asset-inspected-layout-supply/v1",
        "status":"deferred-to-image-dispatch","assetId":asset["id"],"variantId":asset["variantId"],
        "reason":reason,"imageEvidence":"NO_PIXELS_READ","visualQualityApproved":false})
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspected_layout_is_saved_through_desktop_dispatch_and_reopened_without_generation() {
        for standard in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let home = directory.path().join("home");
            let engine = if standard {
                crate::Engine::new_standard(home.clone())
            } else {
                crate::Engine::new_offline_verification(home.clone())
            }
            .unwrap();
            let project = engine
                .dispatch(json!({"action":"projects.create","title":"布局本地路由"}))
                .unwrap();
            let pid = s(&project, "id");
            let store = engine.store(pid).unwrap();
            store
                .save_asset(json!({"id":"mother","variantId":"v","kind":"character","name":"母图"}))
                .unwrap();
            let path = directory.path().join("mother.png");
            image::RgbaImage::from_pixel(12, 12, image::Rgba([20, 30, 40, 255]))
                .save(&path)
                .unwrap();
            let media = store
                .import_image_as_current(&path, "mother", Some("v"))
                .unwrap();
            let policy = engine
                .dispatch(json!({"action":"getExecutionPolicy"}))
                .unwrap();
            assert!(policy["allowedActions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "assets.identityLayout.save"));
            engine
                .dispatch(
                    json!({"action":"assets.identityLayout.save","projectId":pid,
                "assetId":"mother","variantId":"v","mediaId":media["id"],
                "expectedSha256":media["sha256"],"topHeight":4,"confirmed":true}),
                )
                .unwrap();
            let reopened = Store::open(store.root.clone()).unwrap();
            assert_eq!(
                reopened.active_asset("mother").unwrap()["identityLayout"]["identityStrip"]
                    ["height"],
                4
            );
            assert!(reopened.jobs().unwrap().is_empty());
        }
    }
    #[test]
    fn layout_binds_inspected_current_pixels_and_rejects_changed_media() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p")).unwrap();
        store.create("布局", "p").unwrap();
        store
            .save_asset(json!({"id":"mother","variantId":"v","kind":"character","name":"母图"}))
            .unwrap();
        let path = dir.path().join("mother.png");
        image::RgbaImage::from_pixel(12, 12, image::Rgba([20, 30, 40, 255]))
            .save(&path)
            .unwrap();
        let media = store
            .import_image_as_current(&path, "mother", Some("v"))
            .unwrap();
        let mut request = json!({"assetId":"mother","variantId":"v","mediaId":media["id"],"expectedSha256":media["sha256"],"topHeight":4,"confirmed":true});
        store.save_identity_layout(&request).unwrap();
        let mut snapshot = json!({"prompt":"下排服装","imagePresentation":{"layoutKind":"costume"},"references":[{"assetId":"mother","variantId":"v","mediaId":media["id"],"kind":"image","path":media["path"],"sha256":media["sha256"]}],"referenceRequirements":[{"targetAssetId":"mother","role":"identity"}]});
        freeze(&store, &mut snapshot).unwrap();
        assert_eq!(snapshot["identityComposite"]["identityStrip"]["height"], 4);
        assert_eq!(
            snapshot["identityComposite"]["version"],
            crate::asset_composite::LOWER_ADAPTIVE_PROFILE
        );
        assert_eq!(
            snapshot["identityComposite"]["lowerAdaptation"]["sourceTop"],
            json!({"rule":"ceil-ratio","numerator":4,"denominator":12})
        );
        assert_eq!(
            snapshot["identityComposite"]["lowerAdaptation"]["order"],
            "crop-then-resize"
        );
        assert!(s(&snapshot, "prompt").contains("渠道原生正方形分辨率"));
        let once = snapshot.clone();
        freeze(&store, &mut snapshot).unwrap();
        assert_eq!(once, snapshot);
        let mut legacy = once.clone();
        legacy["identityComposite"]["version"] = json!(crate::asset_composite::PROFILE);
        legacy["identityComposite"]
            .as_object_mut()
            .unwrap()
            .remove("lowerAdaptation");
        legacy["prompt"] = json!("已冻结旧版提示词不改写");
        let legacy_before = legacy.clone();
        freeze(&store, &mut legacy).unwrap();
        assert_eq!(legacy, legacy_before);
        let mut malformed = once.clone();
        malformed["identityComposite"]
            .as_object_mut()
            .unwrap()
            .remove("lowerAdaptation");
        let malformed_before = malformed.clone();
        assert!(freeze(&store, &mut malformed).is_err());
        assert_eq!(malformed, malformed_before);
        request["expectedSha256"] = json!("changed");
        assert!(store.save_identity_layout(&request).is_err());
        request["expectedSha256"] = media["sha256"].clone();
        request["topHeight"] = json!(12);
        assert!(store.save_identity_layout(&request).is_err());
        request["topHeight"] = json!(4);
        request["confirmed"] = json!(false);
        assert!(store.save_identity_layout(&request).is_err());
        image::RgbaImage::from_pixel(12, 12, image::Rgba([60, 80, 90, 255]))
            .save(&path)
            .unwrap();
        let next = store
            .import_image_as_current(&path, "mother", Some("v"))
            .unwrap();
        assert!(store.save_identity_layout(&json!({"confirmed":true,"topHeight":4,"assetId":"mother","variantId":"v","mediaId":media["id"],"expectedSha256":media["sha256"]})).is_err());
        let mut fresh = once;
        fresh["identityComposite"] = Value::Null;
        fresh["references"][0]["mediaId"] = next["id"].clone();
        fresh["references"][0]["sha256"] = next["sha256"].clone();
        fresh["references"][0]["path"] = next["path"].clone();
        assert!(freeze(&store, &mut fresh).is_err());
    }
}
