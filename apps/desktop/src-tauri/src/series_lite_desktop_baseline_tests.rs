use super::*;
use crate::series_lite_identity_store::Identity;

struct Fixture {
    locked: LockedSource,
    designs: Vec<DesignInput>,
    mappings: Vec<Mapping>,
    bindings: Vec<SceneBinding>,
    settings: Value,
    baseline: Value,
}
struct SyntheticAdoption;
impl DesignEvidenceVerifier for SyntheticAdoption {
    fn verify(&self, input: &DesignInput, source_hash: &str) -> Result<()> {
        let DesignOrigin::Lite { adoption_evidence } = &input.origin else {
            return Err("wrong evidence origin".into());
        };
        if adoption_evidence["fixtureOnly"] != true
            || adoption_evidence["designSha256"] != exact_hash(&input.design)?
            || input.design["sourceSha256"] != source_hash
        {
            return Err("synthetic adoption drift".into());
        }
        Ok(())
    }
}
fn design(
    locked: &LockedSource,
    id: &str,
    name: &str,
    kind: &str,
    parent: Option<&str>,
    ordinal: u64,
) -> (DesignInput, Mapping) {
    let identity = Identity {
        desktop_asset_id: id.into(),
        variant_id: format!("v-{id}"),
        identity_id: None,
        base_asset_id: parent.map(str::to_owned),
        kind: kind.into(),
    };
    let asset = json!({"id":id,"name":name,"kind":kind,"variantId":identity.variant_id,"version":9,
        "designStatus":"adopted","baseAssetId":parent,"appearance":format!("{name}当前形态")});
    let row = &locked
        .rows
        .iter()
        .find(|r| {
            r.episode_id.as_deref() == Some(locked.episodes[0].id.as_str()) && r.kind == "action"
        })
        .unwrap_or(&locked.rows[0])
        .id;
    let value = json!({"version":"series-full-design/v1","assetId":id,"variantId":identity.variant_id,"designRevision":1,
        "status":"ADOPTED","sourceSha256":locked.sha256,"brief":format!("{name}定位卡"),
        "full":format!("完整必要设计：{name}具有明确轮廓、动作支点、空间或接触关系；保留源中身份与使用边界。"),
        "sourceRefs":[row],"sourceFacts":[{"statement":"按合成源冻结","sourceRefs":[row]}],
        "productionChoices":["测试中的补足选择，不是源事实"],
        "designDependencies":parent.map(|p|vec![json!({"assetId":p,"designRevision":1,"purpose":"继承直接父造型"})]).unwrap_or_default()});
    let origin = DesignOrigin::Lite {
        adoption_evidence: json!({"fixtureOnly":true,"designSha256":exact_hash(&value).unwrap()}),
    };
    let mapping = Mapping {
        registry_id: "synthetic-registry".into(),
        lite_asset_id: format!("A{ordinal:04}"),
        ordinal,
        identity,
        first_asset_version: 9,
        tombstoned: false,
    };
    (
        DesignInput {
            desktop_asset: asset,
            design: value,
            origin,
        },
        mapping,
    )
}
fn fixture() -> Fixture {
    source_fixture("【故事简介】\r\n这是全局资料，不属于本集演出。\r\n第150集\r\n150-1 阳台与餐厅 日 内\r\n△甲在阳台扶住椅背，随后走进餐厅。\r\n甲：钥匙在这里😀。\r\n第151集\r\n151-1 院内 夜 外\r\n乙：下一集原话不能进入前集。\r\n")
}
fn source_fixture(text: &str) -> Fixture {
    let locked = crate::pipeline::prepare_source(text).unwrap();
    assert_eq!(locked.episodes.len(), 2);
    let mut designs = Vec::new();
    let mut mappings = Vec::new();
    for (id, name, kind, parent, ordinal) in [
        ("normal", "甲正常形态", "CH", None, 1),
        ("suit", "甲西装形态", "CH", Some("normal"), 2),
        ("injured", "甲西装受伤形态", "CH", Some("suit"), 3),
        ("balcony", "阳台", "SC", None, 4),
        ("dining", "餐厅", "SC", None, 5),
        ("future", "下集院落", "SC", None, 6),
    ] {
        let (d, m) = design(&locked, id, name, kind, parent, ordinal);
        designs.push(d);
        mappings.push(m);
    }
    let bindings = vec![SceneBinding {
        source_scene_id: locked.episodes[0].scene_ids[0].clone(),
        desktop_asset_ids: vec!["balcony".into(), "dining".into(), "injured".into()],
    }];
    Fixture {
        locked,
        designs,
        mappings,
        bindings,
        settings: json!({"maxUnitSeconds":30,"fantasyMode":"off"}),
        baseline: serde_json::from_str(include_str!(
            "../../../../rules/series-lite/20261028-v0.4.1/tests/fixtures/visual-baseline.json"
        ))
        .unwrap(),
    }
}
fn run(f: &Fixture) -> Result<BuildOutcome> {
    build(
        BuildInput {
            locked: &f.locked,
            episode_id: &f.locked.episodes[0].id,
            known_speakers: &["甲".into(), "乙".into()],
            title: "",
            registry_id: "synthetic-registry",
            registry_revision: 7,
            mappings: &f.mappings,
            designs: &f.designs,
            scene_bindings: &f.bindings,
            settings: &f.settings,
            visual_baseline: &f.baseline,
            production_direction: "沿用已定方向",
        },
        &SyntheticAdoption,
    )
}
fn value(raw: &[u8]) -> Value {
    serde_json::from_slice(raw).unwrap()
}

#[test]
fn baseline_preserves_original_episode_source_order_and_keeps_global_and_future_source_out() {
    let f = fixture();
    let before = serde_json::to_vec(&f.locked).unwrap();
    let outcome = run(&f).unwrap();
    assert!(outcome.gaps.is_empty(), "{outcome:?}");
    let ready = outcome.ready.unwrap();
    let bundle = value(&ready.bundle_utf8);
    let source = &bundle["episodes"][0]["source"];
    assert_eq!(bundle["episodes"][0]["number"], 150);
    assert_eq!(f.locked.episodes[0].number, 1);
    let prepared = crate::series_lite_desktop_source::prepare(
        &f.locked,
        &f.locked.episodes[0].id,
        &["甲".into(), "乙".into()],
    )
    .unwrap();
    assert_eq!(*source, prepared["source"]);
    assert!(source["intakeEvidence"]["sourceText"]
        .as_str()
        .unwrap()
        .contains("甲：钥匙在这里😀。\r\n"));
    assert!(!source.to_string().contains("这是全局资料"));
    assert!(!source.to_string().contains("下一集原话"));
    assert!(!ready.evidence["sourceMap"]["backgroundSourceRows"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        value(&ready.source_lock_utf8)["episodes"][0]["source"],
        *source
    );
    assert_eq!(
        bundle["sourceLockHash"],
        crate::series_lite_bundle::source_lock_hash(&ready.source_lock_utf8).unwrap()
    );
    assert_eq!(bundle["generationBoundaryVersion"], "video-prompt-v1");
    let heading = source["sourceRows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["standard"]["kind"] == "EPISODE_HEADING")
        .unwrap();
    assert_eq!(
        bundle["episodes"][0]["scenes"][0]["sourceRefs"][0],
        heading["id"]
    );
    assert_eq!(
        ready.evidence["structuralHeaderOwnership"][0]["standardRowId"],
        heading["id"]
    );
    assert!(
        ready.evidence["sourceMap"]["rowMappings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["standardRowId"] == heading["id"])
            .unwrap()["sceneId"]
            .is_null(),
        "original map stays unchanged; structural ownership is separately explicit"
    );
    for key in ["plan", "selection", "draft"] {
        assert_eq!(bundle["episodes"][0][key], json!({}));
    }
    assert_eq!(
        bundle["episodes"][0]["settings"]["visualBaseline"],
        f.baseline
    );
    assert_eq!(bundle["episodes"][0]["settings"]["maxUnitSeconds"], 30);
    assert_eq!(
        bundle["episodes"][0]["settings"]["prohibitions"],
        DEFAULT_PROHIBITIONS
    );
    assert_eq!(serde_json::to_vec(&f.locked).unwrap(), before);
    let again = run(&f).unwrap().ready.unwrap();
    assert_eq!(ready.bundle_utf8, again.bundle_utf8);
    assert_eq!(ready.registry_snapshot_utf8, again.registry_snapshot_utf8);
}

#[test]
fn baseline_one_original_scene_contains_two_location_assets_and_parent_designs_are_material_only() {
    let mut f = fixture();
    f.designs[4].desktop_asset["imageRequired"] = json!(false);
    let ready = run(&f).unwrap().ready.unwrap();
    let bundle = value(&ready.bundle_utf8);
    let registry = value(&ready.registry_snapshot_utf8);
    assert_eq!(bundle["episodes"][0]["scenes"].as_array().unwrap().len(), 1);
    assert_eq!(bundle["episodes"][0]["scenes"][0]["id"], "150-1");
    assert_eq!(bundle["episodes"][0]["scenes"][0]["dayInterior"], "日 内");
    let allowed = &ready.scene_supply.scenes[0].allowed_assets;
    assert_eq!(allowed, &["阳台SC", "餐厅SC", "甲西装受伤形态CH"]);
    assert_eq!(registry["assets"].as_array().unwrap().len(), 5);
    assert!(!registry.to_string().contains("下集院落"));
    for id in ["A0001", "A0002"] {
        let parent = ready
            .scene_supply
            .assets
            .iter()
            .find(|a| a.asset_id == id)
            .unwrap();
        assert!(parent.material_source_only);
        assert!(!parent.image_allowed);
        assert!(!allowed.contains(&parent.key));
    }
    let child = registry["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "A0003")
        .unwrap();
    assert_eq!(child["design"]["dependencies"][0]["assetId"], "A0002");
    assert_eq!(child["design"]["detailLevel"], "storyboard");
    assert_eq!(child["placeholderOrdinal"], 2);
    let no_image = registry["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "A0005")
        .unwrap();
    assert_eq!(no_image["imageRequired"], false);
    assert!(
        !ready
            .scene_supply
            .assets
            .iter()
            .find(|a| a.asset_id == "A0005")
            .unwrap()
            .image_allowed
    );
    crate::series_lite_registry::validate_registry_current(&registry, false).unwrap();
}

#[test]
fn baseline_missing_or_unverified_designs_have_specific_local_gaps_without_adopting_prompts_or_images(
) {
    for missing in ["full", "adoption", "parent", "mapping"] {
        let mut f = fixture();
        match missing {
            "full" => {
                f.designs[2].design["full"] = f.designs[2].design["brief"].clone();
                f.designs[2].desktop_asset["imagePrompt"] = json!("导入词不替代完整设计");
            }
            "adoption" => {
                f.designs[2].origin = DesignOrigin::Lite {
                    adoption_evidence: json!({"fixtureOnly":true,"designSha256":"changed"}),
                }
            }
            "parent" => {
                f.designs.remove(1);
            }
            "mapping" => f.mappings[2].tombstoned = true,
            _ => unreachable!(),
        }
        let before = serde_json::to_vec(&f.designs).unwrap();
        let out = run(&f).unwrap();
        assert!(out.ready.is_none(), "{missing}");
        assert!(
            out.gaps
                .iter()
                .any(|g| g.asset_id.as_deref() == Some("injured")
                    && g.episode_id.as_deref() == Some(f.locked.episodes[0].id.as_str())),
            "{missing}: {out:?}"
        );
        assert_eq!(serde_json::to_vec(&f.designs).unwrap(), before);
    }
    assert!(
        run(&fixture()).unwrap().ready.is_some(),
        "another independent ready episode is not globally marked failed"
    );
}

#[test]
fn baseline_dependency_cycle_and_stale_revision_report_actual_chain() {
    for cycle in [false, true] {
        let mut f = fixture();
        if cycle {
            f.designs[0].design["designDependencies"] =
                json!([{"assetId":"injured","designRevision":1,"purpose":"合成循环"}]);
            f.designs[0].origin = DesignOrigin::Lite {
                adoption_evidence: json!({"fixtureOnly":true,"designSha256":exact_hash(&f.designs[0].design).unwrap()}),
            };
        } else {
            f.designs[2].design["designDependencies"][0]["designRevision"] = json!(2);
            f.designs[2].origin = DesignOrigin::Lite {
                adoption_evidence: json!({"fixtureOnly":true,"designSha256":exact_hash(&f.designs[2].design).unwrap()}),
            };
        }
        let out = run(&f).unwrap();
        assert!(out.ready.is_none());
        let detail = &out.gaps[0].detail;
        assert!(
            detail.contains(if cycle { "循环" } else { "版本已变化" }),
            "{detail}"
        );
        assert!(detail.contains("suit"), "{detail}");
    }
}

#[test]
fn legacy_and_lite_design_evidence_do_not_impersonate_each_other() {
    let f = fixture();
    let d = &f.designs[2];
    let record = json!({"kind":"verified-legacy-design-evidence","sourceSha256":f.locked.sha256,"currentAsset":d.desktop_asset,"adoptedDesign":d.design});
    let inspected = Inspection {
        version: crate::series_lite_desktop_supply::VERSION.into(),
        ready: vec![record.clone()],
        gaps: vec![],
    };
    let verifier = InspectedLegacyDesigns(&inspected);
    assert!(verifier.verify(d, &f.locked.sha256).is_err());
    let mut legacy = d.clone();
    legacy.origin = DesignOrigin::Legacy {
        verified_record: record,
    };
    verifier.verify(&legacy, &f.locked.sha256).unwrap();
    legacy.design["brief"] = json!("改变后的卡");
    assert!(verifier.verify(&legacy, &f.locked.sha256).is_err());
}

#[test]
fn historical_image_ordinal_is_independent_of_permanent_a_id_allocation() {
    let mut f = fixture();
    f.mappings[2].ordinal = 7000;
    f.mappings[2].lite_asset_id = "A7000".into();
    f.designs[2].desktop_asset["placeholderOrdinal"] = json!(37);
    let before = f.designs[2].desktop_asset.clone();
    let ready = run(&f).unwrap().ready.unwrap();
    let registry = value(&ready.registry_snapshot_utf8);
    let child = registry["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "A7000")
        .unwrap();
    assert_eq!(child["placeholderOrdinal"], 37);
    let bundle = value(&ready.bundle_utf8);
    assert_eq!(
        bundle["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["assetId"] == "A7000")
            .unwrap()["placeholderOrdinal"],
        37
    );
    assert_eq!(f.designs[2].desktop_asset, before);
    f.designs[2].desktop_asset["placeholderOrdinal"] = json!(6000);
    let refused = run(&f).unwrap();
    assert!(refused.ready.is_none());
    assert!(
        refused
            .gaps
            .iter()
            .any(|g| g.asset_id.as_deref() == Some("injured")
                && g.detail.contains("已有图片序号非法"))
    );
}

#[test]
fn unplaced_body_is_not_promoted_by_the_episode_heading_container_rule() {
    let f=source_fixture("第150集\r\n△甲做了尚未归场的动作。\r\n150-1 阳台 日 内\r\n△甲扶住椅背。\r\n甲：钥匙在这里😀。\r\n第151集\r\n151-1 院内 夜 外\r\n乙：下一集原话。\r\n");
    let result = run(&f).unwrap();
    assert!(result.ready.is_none());
    assert!(
        result
            .gaps
            .iter()
            .any(|g| g.code == "LITE_SUPPLY_SCENE_ASSIGNMENT_REQUIRED"
                && g.coordinates.iter().any(|id| id.ends_with(":line-2"))),
        "{result:?}"
    );
}
