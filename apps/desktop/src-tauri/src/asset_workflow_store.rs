//! Typed storyboard-to-image supply. Imported MD never becomes locked source.
use crate::{
    asset_graph, lingmo_import,
    pipeline::{asset_workflow, digest},
    store::{mark_changed, now, parse, s, save_asset_tx, Result, Store},
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, io::Write, path::Path};

fn rows(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}

#[cfg(test)]
#[path = "asset_workflow_store_tests.rs"]
mod tests;
pub(crate) fn unit_asset_ids(unit: &Value) -> Vec<Value> {
    let mut ids = Vec::new();
    // Native series binds image use explicitly. Imported MD's lossless parser
    // exposes the same visual-only fact as assetRefs; audio stays separate.
    let key = if unit["imageBindings"].is_array() {
        "imageBindings"
    } else if unit["origin"] == "imported-storyboard" {
        "assetRefs"
    } else {
        return ids;
    };
    for key in [key] {
        for item in rows(unit, key) {
            let id = item.get("assetId").unwrap_or(&item);
            if id.is_string() && !ids.contains(id) {
                ids.push(id.clone());
            }
        }
    }
    ids
}

/// Ignore storage-only unit revisions, but freeze the exact authored body,
/// image identities/uses, location and external provenance actually consumed.
pub(crate) fn source_unit_binding(unit: &Value) -> Value {
    json!({"unitId":unit["id"],"assetIds":unit_asset_ids(unit),
        "promptSha256":digest(s(unit,"prompt")),"origin":unit["origin"],
        "episodeId":unit["episodeId"],"sceneId":unit["sceneId"],
        "imageBindings":if unit["imageBindings"].is_array(){&unit["imageBindings"]}else{&unit["assetRefs"]},
        "importSource":unit["importSource"]})
}

pub(crate) fn validate_prompt_source_tx(tx: &rusqlite::Transaction<'_>, prompt: &Value) -> Result<()> {
    match prompt.get("sourceBindingContract") {
        None | Some(Value::Null) => return Ok(()), // Historic frozen contracts remain unchanged.
        Some(value) if value == asset_workflow::SOURCE_BINDING_V1 => {},
        Some(_) => return Err("未知提示词来源绑定合同，未派发旧请求".into()),
    }
    let bindings = prompt["sourceUnitBindings"].as_array().ok_or("提示词缺少冻结单元来源")?;
    let mut seen = HashSet::new();
    for binding in bindings {
        let id = s(binding,"unitId");
        if id.is_empty() || !seen.insert(id) { return Err("提示词单元来源身份缺失或重复".into()); }
        let raw:Option<String> = tx.query_row("SELECT data FROM units WHERE id=?1",[id],|r|r.get(0))
            .optional().map_err(|e|e.to_string())?;
        let unit = raw.map(parse).transpose()?.ok_or("提示词来源分镜已不存在，请核对当前依据")?;
        if unit["retired"]==true || source_unit_binding(&unit)!=*binding {
            return Err(format!("提示词来源单元 {id} 已变化，请核对当前依据"));
        }
    }
    if prompt["sourceKind"]=="original-script" {
        let raw:Option<String> = tx.query_row("SELECT data FROM sources ORDER BY version DESC LIMIT 1",[],|r|r.get(0))
            .optional().map_err(|e|e.to_string())?;
        let source=raw.map(parse).transpose()?.unwrap_or(Value::Null);
        if source["sha256"]!=prompt["sourceSha256"] {return Err("提示词原剧本依据已经变化，请核对当前依据".into());}
    }
    Ok(())
}

pub(crate) fn validate_prompt_source(store:&Store,prompt:&Value)->Result<()> {
    let mut connection=store.conn()?;
    let tx=connection.transaction().map_err(|e|e.to_string())?;
    validate_prompt_source_tx(&tx,prompt)
}

fn reference_filename(asset: &Value) -> String {
    format!("reference-{}.png", &digest(&format!("{}:{}", s(asset,"id"), s(asset,"variantId")))[..20])
}
fn registered_reference(asset: &Value, scope: &[Value], assets: &[Value]) -> bool {
    fn reaches_formal(asset: &Value, scope: &[Value], assets: &[Value], path: &mut HashSet<String>) -> bool {
        let registration = &asset["referenceRegistration"];
        let id = s(asset, "id");
        if crate::store::asset_deleted(asset) || id.is_empty()
            || registration["version"] != "desktop-reference-image/v1"
            || s(registration, "reason").trim().is_empty()
            // The original v1 filename also froze this exact identity. Keep
            // those registrations usable, but never carry them to a new variant.
            || registration["internalFilename"] != reference_filename(asset)
            || registration.get("assetId").is_some_and(|v| v != &asset["id"])
            || registration.get("variantId").is_some_and(|v| v != &asset["variantId"])
            || !path.insert(id.to_owned()) {
            return false;
        }
        let valid = rows(registration, "serves").iter().any(|served| {
            assets.iter().any(|child| {
                child["id"] == served["assetId"] && child["variantId"] == served["variantId"]
                    && !crate::store::asset_deleted(child)
                    && asset_graph::dependencies(assets, child).is_ok_and(|ids| ids.iter().any(|v| v == id))
                    && (scope.contains(&child["id"]) || reaches_formal(child, scope, assets, path))
            })
        });
        path.remove(id);
        valid
    }
    reaches_formal(asset, scope, assets, &mut HashSet::new())
}
fn filename(asset: &Value, mapped: Option<&Value>) -> Result<String> {
    let name = asset["filename"]
        .as_str()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            mapped
                .and_then(|m| m["filename"].as_str())
                .filter(|v| !v.is_empty())
        })
        .or_else(|| {
            asset["referenceRegistration"]["internalFilename"]
                .as_str()
                .filter(|v| !v.is_empty())
        })
        .map(str::to_owned)
        .or_else(|| {
            asset["token"]
                .as_str()
                .and_then(|v| v.strip_prefix("@[")?.strip_suffix(']'))
                .map(|v| format!("{v}.png"))
        })
        .ok_or_else(|| format!("{} 缺少同版精确PNG文件名；未根据名字猜写", s(asset, "id")))?;
    if !name.to_ascii_lowercase().ends_with(".png")
        || name.contains(['/', '\\', ':'])
        || name == ".png"
        || name.trim() != name
    {
        return Err(format!("无效的同版PNG文件名：{name}"));
    }
    Ok(name)
}

impl Store {
    pub fn series_deliveries(&self) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let mut q = c
            .prepare("SELECT value FROM metadata WHERE key LIKE 'series.delivery:%' ORDER BY rowid")
            .map_err(|e| e.to_string())?;
        let result = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .map(|r| parse(r.map_err(|e| e.to_string())?))
            .collect();
        result
    }
    pub fn image_production_scope(&self) -> Result<Value> {
        let assets = self.values("assets")?;
        let deliveries = self.series_deliveries()?;
        let units = self.values("units")?;
        let mut used = HashSet::new();
        let mut used_order = Vec::new();
        for delivery in &deliveries {
            for unit in rows(&delivery["bundle"]["draft"], "units") {
                for id in unit_asset_ids(&unit) {
                    if let Some(id) = id.as_str() {
                        if used.insert(id.to_owned()) {
                            used_order.push(id.to_owned());
                        }
                    }
                }
            }
        }
        for unit in units.iter().filter(|u| u["retired"] != true) {
            for id in unit_asset_ids(unit) {
                if let Some(id) = id.as_str() {
                    if used.insert(id.to_owned()) {
                        used_order.push(id.to_owned());
                    }
                }
            }
        }
        let actual_scope = !deliveries.is_empty() || units.iter().any(|u| u["retired"] != true);
        let mut ids: Vec<_> = assets
            .iter()
            .filter(|a| {
                asset_graph::image_required(a) && (!actual_scope || used.contains(s(a, "id")))
            })
            .map(|a| a["id"].clone())
            .collect();
        if actual_scope {
            ids.sort_by_key(|id| {
                used_order
                    .iter()
                    .position(|v| id == v)
                    .unwrap_or(usize::MAX)
            });
        }
        let references: Vec<_> = assets
            .iter()
            .filter(|a| !ids.contains(&a["id"]) && registered_reference(a, &ids, &assets))
            .map(|a| a["id"].clone())
            .collect();
        Ok(
            json!({"version":"desktop-image-scope/v1","assetIds":ids,"basis":if actual_scope{"adopted-or-imported-visual-use"}else{"legacy-explicit-assets"},"referenceOnlyIds":references}),
        )
    }
    /// A named UI action confirms one support image; no ancestor closure or
    /// media submission happens here, and narrative purpose remains unchanged.
    pub fn register_image_reference(&self, request: &Value) -> Result<Value> {
        if request["confirmed"] != true || s(request, "reason").trim().is_empty() {
            return Err("请明确登记参考母版用途及所服务的图片项".into());
        }
        let mut asset = self.active_asset(s(request, "assetId"))?;
        if request["expectedVersion"] != asset["version"] {
            return Err("参考母版版本已变化，请刷新后登记".into());
        }
        if ["audio", "voice", "SE"].contains(&s(&asset, "kind"))
            || matches!(
                asset.pointer("/preparation/mode").and_then(Value::as_str),
                Some("AUDIO" | "INLINE" | "UNRESOLVED")
            )
        {
            return Err("该项不是可登记的独立参考图片".into());
        }
        let scope = self.image_production_scope()?;
        let ids = request["servesAssetIds"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or("请指定本次正式范围或已登记参考链内所服务的子项")?;
        let assets = self.values("assets")?;
        let mut seen = HashSet::new();
        let mut serves = Vec::new();
        for id in ids {
            let key = id.as_str().ok_or("所服务图片身份无效")?;
            if !seen.insert(key) || (!rows(&scope, "assetIds").contains(id)
                && !rows(&scope, "referenceOnlyIds").contains(id)) {
                return Err("所服务图片不在当前正式范围或已登记参考链内，或身份重复".into());
            }
            let child = assets
                .iter()
                .find(|a| a["id"] == *id)
                .ok_or("所服务图片不存在")?;
            if !asset_graph::dependencies(&assets, child)?.contains(&s(&asset, "id").to_owned()) {
                return Err("子项没有登记对该母版的真实媒体需求，不按名字猜依赖".into());
            }
            serves.push(json!({"assetId":id,"variantId":child["variantId"],"uses":child["uses"]}));
        }
        let (_, invalid) = asset_graph::explicit_order(&assets, ids);
        if !invalid.is_empty() {
            return Err(format!("参考链尚未有效：{}", invalid.iter().map(|v| s(v,"reason")).collect::<Vec<_>>().join("；")));
        }
        let already_visible = rows(&scope, "assetIds").contains(&asset["id"]);
        let already_registered = rows(&scope, "referenceOnlyIds").contains(&asset["id"]);
        let reused = self
            .adopted(s(&asset, "id"), s(&asset, "variantId"))?
            .filter(|mid| {
                !asset_graph::image_adoption_blocked(&asset)
                    && !asset_graph::design_review_blocked(&asset)
                    && self.get("media", mid).is_ok_and(|m| {
                    m["placeholder"] != true
                        && m["kind"] == "image"
                        && self.verify_media(&m).is_ok()
                })
            });
        if !already_visible {
            let version = asset["version"].clone();
            let registration = json!({"version":"desktop-reference-image/v1","assetId":asset["id"],"variantId":asset["variantId"],"reason":s(request,"reason").trim(),"serves":serves,"confirmation":"explicit-user-selection",
                "internalFilename":reference_filename(&asset)});
            if asset["referenceRegistration"] != registration || asset["productionPurpose"] != "reference-only" {
                asset["referenceRegistration"] = registration;
                asset["productionPurpose"] = json!("reference-only");
                asset["expectedVersion"] = version;
                asset = self.save_asset(asset)?;
            }
        }
        Ok(
            json!({"asset":asset,"assetId":asset["id"],"alreadyInFormalScope":already_visible,"alreadyRegistered":already_registered,"reusableMediaId":reused,"adoptedMediaId":reused,"reusedAdoptedMedia":reused.is_some(),
            "addedImageCount":usize::from(!already_visible&&!already_registered&&reused.is_none()),"generationSubmissions":0,"scope":self.image_production_scope()?}),
        )
    }
    pub fn image_workflow_input(&self, request: &Value, settings: &Value) -> Result<Value> {
        let mut assets = self.values("assets")?;
        let scope = self.image_production_scope()?;
        let selection = request
            .get("assetIds")
            .cloned()
            .unwrap_or_else(|| scope["assetIds"].clone());
        let ids = selection.as_array().ok_or("图片范围必须为资产 ID 数组")?;
        if ids.is_empty() {
            return Err("当前没有需要编写提示词的图片资产".into());
        }
        let mut seen = HashSet::new();
        for id in ids {
            let key = id
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("图片范围包含无效 ID")?;
            if !seen.insert(key) || !assets.iter().any(|a| a["id"] == *id) {
                return Err("图片范围重复或资产不存在".into());
            }
            if !rows(&scope, "assetIds").contains(id)
                && !rows(&scope, "referenceOnlyIds").contains(id)
            {
                return Err(format!("资产 {key} 不在当前正式图片或已登记参考范围"));
            }
        }
        let source = self.source()?;
        let deliveries = self.series_deliveries()?;
        let foundation = self.foundation()?;
        let units = self.values("units")?;
        let mut related_ids = ids.clone();
        let mut related_index = 0;
        while related_index < related_ids.len() {
            let current_id = related_ids[related_index].clone();
            related_index += 1;
            for a in assets.iter().filter(|a| a["id"] == current_id
                && registered_reference(a, &rows(&scope,"assetIds"), &assets)) {
                for served in rows(&a["referenceRegistration"], "serves") {
                    let related = assets.iter().any(|child| child["id"] == served["assetId"]
                        && child["variantId"] == served["variantId"]
                        && (rows(&scope,"assetIds").contains(&child["id"]) || rows(&scope,"referenceOnlyIds").contains(&child["id"]))
                        && asset_graph::dependencies(&assets,child).is_ok_and(|deps| deps.contains(&s(a,"id").to_owned())));
                    if related && !related_ids.contains(&served["assetId"]) {
                        related_ids.push(served["assetId"].clone());
                    }
                }
            }
        }
        let imported_selection = units.iter().any(|u| {
            u["retired"] != true
                && u["origin"] == "imported-storyboard"
                && unit_asset_ids(u).iter().any(|id| related_ids.contains(id))
        });
        let mut image_source;
        let mut external_packages=Vec::new();
        if !s(request, "documentId").is_empty()
            || imported_selection
            || (deliveries.is_empty() && source.is_null())
        {
            let documents = lingmo_import::documents(self)?;
            let mut selected_docs = Vec::new();
            for doc in &documents {
                let doc_id = s(doc, "id");
                if !s(request, "documentId").is_empty() {
                    if request["documentId"] != doc["id"] {
                        continue;
                    }
                } else if !units.iter().any(|u| {
                    u["retired"] != true
                        && u["importSource"]["documentId"] == doc_id
                        && unit_asset_ids(u).iter().any(|id| related_ids.contains(id))
                }) {
                    continue;
                }
                selected_docs.push(doc.clone());
            }
            if selected_docs.is_empty() {
                return Err("所选图片没有可核对的导入MD来源".into());
            }
            let mut texts = Vec::new();
            let mut evidence = Vec::new();
            let mut provenance = Vec::new();
            for doc in &selected_docs {
                let doc_id = s(doc, "id");
                if doc["handoff"]["scopeRestriction"].is_string() {
                    return Err("此接续稿已声明制作范围但缺少同版完整资料；可浏览原稿，补齐原生队列与范围前不能降级写词或生图".into());
                }
                if let Some(mut package)=crate::lingmo_package::stored(self,doc_id)? {
                    package["documentId"]=json!(doc_id);
                    // The package verifier owns complete provenance; the text
                    // compiler only needs direction, registry and explicit queue.
                    if let Some(fields)=package.as_object_mut(){for key in ["sourceText","sourceLock","episodes","initialRegistry","upstreamBundle","upstreamRegistry","markdownText","consumptionReceipt"]{fields.remove(key);}}
                    external_packages.push(package);
                }
                let original = lingmo_import::original(self, doc_id, None)?;
                texts.push(s(&original, "text").to_owned());
                provenance.push(json!({"documentId":doc_id,"originalSha256":original["sha256"]}));
                for unit in units
                    .iter()
                    .filter(|u| u["retired"] != true && u["importSource"]["documentId"] == doc_id)
                {
                    lingmo_import::validate_unit(self, unit)?;
                    let raw = lingmo_import::original(self, doc_id, Some(s(unit, "id")))?;
                    let current = s(unit, "prompt");
                    if current.trim().is_empty() {
                        continue;
                    }
                    let edited = current != s(&raw, "text");
                    // Preserve immutable originals. Current explicit desktop edits
                    // are separate, named evidence, never a rewritten original MD.
                    if edited {
                        texts.push(format!(
                            "\n[桌面已保存单元编辑：{}]\n{current}",
                            s(unit, "id")
                        ));
                    }
                    evidence.push(json!({"id":unit["id"],"text":current,"assetIds":unit_asset_ids(unit),
                        "documentId":doc_id,"episodeId":unit["episodeId"],"sceneId":unit["sceneId"],
                        "unitVersion":unit["version"],"originalUnitSha256":raw["sha256"],"textSha256":digest(current),
                        "evidenceOrigin":if edited{"saved-desktop-unit-edit"}else{"imported-original-md"}}));
                }
            }
            let text = texts.join("\n\n");
            image_source = json!({"sourceKind":if external_packages.len()==selected_docs.len(){"external-package"}else{"imported-md"},"sha256":digest(&text),"documentId":if selected_docs.len()==1{selected_docs[0]["id"].clone()}else{json!(format!("collection:{}",digest(&json!(provenance).to_string())))},"text":text,"documents":provenance,"evidence":evidence});
            if !external_packages.is_empty(){image_source["completePackages"]=json!(external_packages.iter().map(|p|json!({"documentId":p["documentId"],"proof":p["proof"]})).collect::<Vec<_>>());}
        } else if !deliveries.is_empty() {
            let mut texts = deliveries
                .iter()
                .map(|d| s(d, "markdown").to_owned())
                .collect::<Vec<_>>();
            let mut evidence=Vec::new();
            for unit in units.iter().filter(|u| u["retired"]!=true && unit_asset_ids(u).iter().any(|id|related_ids.contains(id))) {
                let current=s(unit,"prompt");
                if current.trim().is_empty(){continue;}
                // Current desktop edits are named evidence alongside the adopted
                // delivery; they never overwrite its original Markdown or proof.
                texts.push(format!("\n[桌面当前分镜单元：{}]\n{current}",s(unit,"id")));
                evidence.push(json!({"id":unit["id"],"text":current,"assetIds":unit_asset_ids(unit),
                    "episodeId":unit["episodeId"],"sceneId":unit["sceneId"],"unitVersion":unit["version"],
                    "textSha256":digest(current),"evidenceOrigin":"saved-desktop-storyboard"}));
            }
            let text=texts.join("\n\n");
            // Asset image/prompt edits bump storage version, not the adopted
            // design registry. Re-running must not buy all text again.
            let revision = deliveries
                .iter()
                .flat_map(|d| rows(&d["assetIndex"], "assets"))
                .filter_map(|a| a["assetRevision"].as_u64())
                .sum::<u64>()
                .max(1);
            let registry = json!({"assetRegistryId":self.project()?["id"],"assetRegistryRevision":revision,"deliveries":deliveries});
            image_source = json!({"sourceKind":"adopted-storyboard","sha256":digest(&text),"text":text,"handoffManifest":registry,"handoffSha256":digest(&registry.to_string()),"evidence":evidence});
        } else if !source.is_null() {
            image_source = json!({"sourceKind":"original-script","sha256":source["sha256"],"lockedSource":source,"text":source["text"],"evidence":rows(&source,"rows").into_iter().filter(|r|!s(r,"text").trim().is_empty()).collect::<Vec<_>>()});
        } else {
            return Err("没有可用于写词的剧本、采用分镜或导入MD".into());
        }
        // Bind the exact input, without putting credentials in the source evidence.
        image_source["sourceDescriptorVersion"] = json!("image-source/v1");
        crate::pipeline::asset_workflow::validate_source(&image_source)?;
        let items:Vec<_>=ids.iter().map(|id|assets.iter().find(|a|a["id"]==*id).ok_or_else(||"图片范围资产不存在".to_owned()))
          .collect::<Result<Vec<_>>>()?.into_iter().map(|a|{
            let mapped=deliveries.iter().flat_map(|d|rows(&d["assetIndex"],"assets")).find(|v|v["id"]==a["id"]);
            let filename=filename(a,mapped.as_ref())?;
            let uses=if a["uses"].is_array(){a["uses"].clone()}else{json!(units.iter().filter(|u|u["retired"]!=true&&unit_asset_ids(u).contains(&a["id"])).map(|u|json!({"episode":u["episodeId"],"scene":u["sceneId"],"unit":u["id"],"purpose":"visible"})).collect::<Vec<_>>())};
            Ok(json!({"assetId":a["id"],"variantId":a["variantId"],"filename":filename,"uses":uses,"purpose":a["purpose"]}))
        }).collect::<Result<_>>()?;
        let unit_bindings:Vec<_>=units.iter().filter(|u|u["retired"]!=true&&unit_asset_ids(u).iter().any(|id|related_ids.contains(id)))
            .map(source_unit_binding).collect();
        for a in &mut assets {
            if a["handoffPackage"].is_null() {
                for package in &external_packages {
                    if let Some(projection)=crate::lingmo_continuation::package_asset_projection(self,a,package)? {
                        for (key,value) in projection.as_object().ok_or("完整包临时供料无效")? {a[key]=value.clone();}
                        break;
                    }
                }
            }
            a["adoptedMediaId"] = json!(self.adopted(s(a, "id"), s(a, "variantId"))?);
            // Supply registered geometry as metadata, without making image
            // availability a prerequisite for authoring clothing prompts.
            a["identityLayoutSupply"] = crate::asset_identity_layout::prompt_supply(self, a);
        }
        Ok(
            json!({"workflowProfile":asset_graph::PROFILE,"assetPromptRequestVersion":asset_workflow::REQUEST_V2,"assetPromptSourceContract":asset_workflow::SOURCE_BINDING_V1,"costumePromptLayoutContract":asset_workflow::COSTUME_PROMPT_V1,"imageSource":image_source,"externalPackages":external_packages,"assets":assets,"settings":settings,"foundation":foundation,"visualDirection":foundation["productionDirection"],"selection":{"assetIds":selection},"imageQueue":{"version":"desktop-image-queue/v1","items":items},"sourceVersion":source["version"],"sourceUnitBindings":unit_bindings}),
        )
    }

    /// An explicit edit/review keeps the exact manual text and records which
    /// current source/design it was checked against. It calls no model/channel.
    pub fn save_image_prompt(&self, request:&Value, settings:&Value)->Result<Value> {
        let asset=self.active_asset(s(request,"assetId"))?;
        if request["expectedVersion"]!=asset["version"] {return Err("资产已更新，请刷新后再保存提示词".into());}
        let text=s(request,"text");
        if text.trim().is_empty(){return Err("图片提示词不能为空".into());}
        let ratio=s(request,"ratio");
        if !ratio.is_empty() && !regex::Regex::new(r"^[1-9][0-9]?:[1-9][0-9]?$").unwrap().is_match(ratio) {
            return Err("图片画幅无效".into());
        }
        let context=self.image_workflow_input(&json!({"assetIds":[asset["id"]]}),settings);
        let basis=context.as_ref().map_err(Clone::clone).and_then(|input|asset_workflow::manual_prompt_basis(input,&asset));
        let prior_invalid=matches!(s(&asset,"promptStatus"),"needs_review"|"stale"|"rejected"|"invalid")
            || validate_prompt_source(self,&asset["imagePrompt"]).is_err();
        let explicitly_reviewed=request["reviewCurrentBasis"]==true;
        if explicitly_reviewed && basis.is_err(){return Err(basis.unwrap_err());}
        let can_accept=basis.is_ok() && (!prior_invalid||explicitly_reviewed);
        let mut next=asset.clone();
        if !next["imagePrompt"].is_object(){next["imagePrompt"]=json!({"version":asset_workflow::PROMPT_VERSION});}
        next["imagePrompt"]["text"]=json!(text);
        next["imagePrompt"]["origin"]=json!("manual");
        next["imagePrompt"]["imageEvidence"]=json!("NO_PIXELS_READ");
        if !ratio.is_empty(){next["imagePrompt"]["ratio"]=json!(ratio);}
        next["promptStatus"]=json!(if can_accept{"ready"}else{"needs_review"});
        next["imagePrompt"]["status"]=next["promptStatus"].clone();
        if can_accept {
            let basis=basis.unwrap();
            for (key,value) in basis.as_object().unwrap(){next["imagePrompt"][key]=value.clone();}
            next["imagePrompt"]["manualReview"]=json!({"version":"asset-prompt-manual-review/v1",
                "kind":if explicitly_reviewed{"explicit-current-basis-confirmation"}else{"manual-edit-current-basis"},
                "sourceBasisSha256":basis["sourceBasisSha256"],"promptSha256":digest(text),
                "previousPromptSha256":digest(s(&asset["imagePrompt"],"text")),"reviewedAt":now(),
                "scope":"saved-text-and-current-source-design;not-model-or-visual-quality-approval"});
        }
        let mut connection=self.conn()?;
        let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        let raw:String=tx.query_row("SELECT data FROM assets WHERE id=?1",[s(&asset,"id")],|r|r.get(0)).map_err(|e|e.to_string())?;
        if parse(raw)?["version"]!=asset["version"]{return Err("保存期间资产已变化，请刷新后重试".into());}
        if can_accept {validate_prompt_source_tx(&tx,&next["imagePrompt"])?;}
        next["expectedVersion"]=asset["version"].clone();
        save_asset_tx(&tx,next)?;
        tx.commit().map_err(|e|e.to_string())?;
        self.active_asset(s(&asset,"id"))
    }

    /// Publish independently successful words even when an unrelated asset is blocked.
    pub fn finish_image_prompts(&self, input: &Value, output: &Value, run: &str) -> Result<Value> {
        if input["workflowProfile"] != asset_graph::PROFILE
            || output["workflowProfile"] != asset_graph::PROFILE
            || output["schemaVersion"] != "asset-prompt-batch/v1"
            || output["sourceSha256"] != input["imageSource"]["sha256"]
            || output["sourceKind"] != input["imageSource"]["sourceKind"]
            || output["imageGenerationSubmissions"] != 0
            || !output["assets"].is_array()
            || !output["blocked"].is_array()
        {
            return Err("图片提示词流程合同不匹配".into());
        }
        let mut c = self.conn()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (state, snapshot): (String, String) = tx
            .query_row(
                "SELECT state,snapshot FROM jobs WHERE id=?1 AND kind='workflow'",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let snapshot = parse(snapshot)?;
        if state != "running" || snapshot["stage2Context"] != *input {
            return Err("提示词原流程状态或冻结输入变化，成果保留未覆盖".into());
        }
        if input["imageSource"]["sourceKind"] == "original-script"
            && self.source()?["sha256"] != input["imageSource"]["sha256"]
        {
            return Err("原文变化，旧提示词结果保留待核".into());
        }
        let mut changed_source = HashSet::new();
        for frozen in rows(input, "sourceUnitBindings") {
            let current: Option<String> = tx
                .query_row(
                    "SELECT data FROM units WHERE id=?1",
                    [s(&frozen, "unitId")],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let unchanged = current.map(parse).transpose()?.is_some_and(|u| {
                if input["assetPromptSourceContract"]==asset_workflow::SOURCE_BINDING_V1 {
                    return u["retired"]!=true && source_unit_binding(&u)==frozen;
                }
                u["retired"] != true
                    && digest(s(&u, "prompt")) == s(&frozen, "promptSha256")
                    && json!(unit_asset_ids(&u)) == frozen["assetIds"]
            });
            if !unchanged {
                for id in rows(&frozen, "assetIds") {
                    if let Some(id) = id.as_str() {
                        changed_source.insert(id.to_owned());
                    }
                }
            }
        }
        let mut result = output.clone();
        let mut blocked = rows(output, "blocked");
        let mut saved = Vec::new();
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        for proposal in rows(output, "assets") {
            if !seen.insert(s(&proposal, "id").to_owned()) {
                return Err("提示词结果包含重复资产身份".into());
            }
            let frozen = rows(input, "assets")
                .into_iter()
                .find(|a| a["id"] == proposal["id"])
                .ok_or("结果资产不属于原冻结范围")?;
            if !rows(&input["selection"], "assetIds").contains(&proposal["id"])
                || proposal["variantId"] != frozen["variantId"]
            {
                return Err("提示词输出超出显式资产/形象范围".into());
            }
            if proposal["adoptedDesign"] != frozen["adoptedDesign"]
                || proposal["prompt"] != frozen["prompt"]
                || proposal["promptOrigin"] != frozen["promptOrigin"]
            {
                return Err("图片写词不得改写完整采用设计或旧手写提示词".into());
            }
            if proposal["mediaPlan"].is_object() {
                let plan = &proposal["mediaPlan"];
                if plan["version"] != "asset-media-plan/v1"
                    || plan["referenceRequirements"] != proposal["referenceRequirements"]
                    || plan["imageTask"] != proposal["imageTask"]
                    || plan["sourceSha256"] != input["imageSource"]["sha256"]
                    || plan["requestFingerprint"] != proposal["imagePrompt"]["inputFingerprint"]
                {
                    return Err("媒体计划与已冻结提示词、来源或引用不一致".into());
                }
                let mut drifted = false;
                for parent in rows(plan, "parentEvidence") {
                    let raw: Option<String> = tx
                        .query_row(
                            "SELECT data FROM assets WHERE id=?1",
                            [s(&parent, "assetId")],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(|e| e.to_string())?;
                    let matches = raw.map(parse).transpose()?.is_some_and(|a| {
                        a["variantId"] == parent["variantId"]
                            && asset_workflow::media_identity_fingerprint(&a)
                                == s(&parent, "fingerprint")
                    });
                    if !matches {
                        drifted = true;
                        break;
                    }
                }
                if drifted {
                    blocked.push(json!({"assetId":proposal["id"],"reason":"写词期间参考资产身份或采用设计改变；媒体计划候选保留，未覆盖当前计划"}));
                    candidates.push(proposal);
                    continue;
                }
            }
            let current: Option<String> = tx
                .query_row(
                    "SELECT data FROM assets WHERE id=?1",
                    [s(&proposal, "id")],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let Some(current) = current.map(parse).transpose()? else {
                blocked.push(json!({"assetId":proposal["id"],"reason":"资产已不存在"}));
                candidates.push(proposal);
                continue;
            };
            if changed_source.contains(s(&proposal, "id"))
                || crate::store::asset_deleted(&current)
                || asset_graph::design_review_blocked(&current)
            {
                blocked.push(json!({"assetId":proposal["id"],"reason":"编写期间来源分镜或采用设计依据已变更，或资产已删除；候选保留待核"}));
                candidates.push(proposal);
                continue;
            }
            if current["version"] != frozen["version"] {
                if current["imagePrompt"] == proposal["imagePrompt"]
                    && current["adoptedDesign"] == frozen["adoptedDesign"]
                    && current["prompt"] == frozen["prompt"]
                    && current["variantId"] == frozen["variantId"]
                {
                    saved.push(current);
                    continue;
                }
                blocked.push(json!({"assetId":proposal["id"],"reason":"编写期间资产已编辑，保留当前版本及新候选"}));
                candidates.push(proposal);
                continue;
            }
            let mut next = current;
            for key in [
                "imagePrompt",
                "imagePresentation",
                "referenceRequirements",
                "mediaPlan",
                "imageTask",
                "designCandidate",
                "promptStatus",
            ] {
                if let Some(v) = proposal.get(key) {
                    next[key] = v.clone();
                }
            }
            next["workflowProfile"] = json!(asset_graph::PROFILE);
            next["expectedVersion"] = frozen["version"].clone();
            saved.push(save_asset_tx(&tx, next)?);
        }
        result["promptCount"] = json!(saved.len());
        result["unpublishedCandidates"] = json!(candidates);
        result["imagePlan"] = json!(rows(output, "imagePlan")
            .into_iter()
            .filter(|p| saved.iter().any(|a| a["id"] == p["assetId"]))
            .collect::<Vec<_>>());
        result["assets"] = json!(saved);
        result["blocked"] = json!(blocked);
        let state = if blocked.is_empty() {
            "completed"
        } else {
            "partially_blocked"
        };
        result["status"] = json!(state);
        result["publication"] = json!({"version":"image-prompt-publication/v1","workflowId":run,"successfulItemsPublished":true});
        tx.execute(
            "UPDATE jobs SET state=?1,result=?2,error=NULL,updated_at=?3 WHERE id=?4",
            params![state, result.to_string(), now(), run],
        )
        .map_err(|e| e.to_string())?;
        mark_changed(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }

    pub fn export_image_prompts(&self, request: &Value) -> Result<Value> {
        let target = Path::new(s(request, "path"));
        if !target.is_absolute() || !target.is_dir() {
            return Err("请选择已有本地导出文件夹".into());
        }
        let scope = self.image_production_scope()?;
        let selected = request
            .get("assetIds")
            .unwrap_or(&scope["assetIds"])
            .as_array()
            .ok_or("导出选择无效")?;
        let assets = self.values("assets")?;
        let mut sections = Vec::new();
        let mut missing = Vec::new();
        for id in selected {
            let a = assets
                .iter()
                .find(|a| a["id"] == *id)
                .ok_or("导出资产不存在")?;
            let text = crate::pipeline::asset_workflow::prompt_text(a);
            if text.trim().is_empty() {
                missing.push(id.clone());
                continue;
            }
            sections.push(format!(
                "## {}\n\n{}\n\n参考需求：{}\n",
                s(a, "name"),
                text,
                a["referenceRequirements"]
            ));
        }
        if sections.is_empty() {
            return Err("所选资产尚无可导出的提示词".into());
        }
        let text = format!("# 资产图片提示词\n\n{}", sections.join("\n"));
        let path = target.join(format!("资产提示词-{}.md", &digest(&text)[..12]));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if fs::read(&path).map_err(|e| e.to_string())? != text.as_bytes() {
                    return Err("目标同名文件已有不同内容，未覆盖".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
        Ok(
            json!({"path":path,"count":sections.len(),"missingAssetIds":missing,"generationSubmissions":0}),
        )
    }
}
