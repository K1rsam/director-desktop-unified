//! Synthetic in-memory loader/mechanical mapping tests. Explicit verifier doubles
//! exercise the proof boundary; they do NOT prove original production adoption.
use super::*;
use rusqlite::params;
use std::cell::Cell;

struct SourceGate {
    reject: bool,
}
impl SourceVerifier for SourceGate {
    fn verify(&self, _: &Value, _: &Value, _: &[String]) -> Result<()> {
        if self.reject {
            Err("original source verifier rejected".into())
        } else {
            Ok(())
        }
    }
    fn original_episode_number(&self, s: &str) -> Option<u32> {
        match s {
            "二十八" => Some(28),
            _ => s.parse().ok(),
        }
    }
}
fn source_fixture() -> (Value, Value) {
    let text = "第28集\r\n28-1 门口 日 外\r\n△甲抬头。\r\n甲：你好😀。\r\n";
    let source = crate::series_lite_intake::prepare_source(
        &json!(text),
        &json!("desktop-ep"),
        &json!(["甲"]),
    )
    .unwrap();
    let origin = 17u64;
    let mut offset = 0u64;
    let rows:Vec<_>=text.split_inclusive('\n').enumerate().map(|(i,line)|{let start=offset;offset+=line.encode_utf16().count() as u64;json!({"desktopRowId":format!("desktop-row-{i}"),"sceneId":if i==0{Value::Null}else{json!("desktop-scene")},"globalRange":{"start":origin+start,"end":origin+offset},"localRange":{"start":start,"end":offset}})}).collect();
    let locked = json!({"episodes":[{"id":"desktop-ep","number":1,"sourceNumber":"28","sceneIds":["desktop-scene"]}],"scenes":[{"id":"desktop-scene","name":"门口","original":"28-1 门口 日 外"}]});
    let prepared = json!({"source":source,"desktopBinding":{"sourceSha256":"source","lockedSourceSha256":"locked","episodeId":"desktop-ep","sourceRange":{"start":origin,"end":origin+offset},"rows":rows},"backgroundSourceRows":[{"text":"原始背景"}]});
    (locked, prepared)
}
fn mapped(locked: &Value, p: &Value) -> Inspection {
    inspect_sources(&SourceGate { reject: false }, locked, &[p.clone()], &[])
}
#[test]
fn preserves_original_episode_28_not_internal_ordinal_1() {
    let (l, p) = source_fixture();
    let before = (l.clone(), p.clone());
    let out = mapped(&l, &p);
    assert_eq!(out.ready.len(), 1, "{out:?}");
    let e = &out.ready[0];
    assert_eq!(e["episodeNumber"], 28);
    assert_eq!(e["desktopOrdinal"], 1);
    assert_eq!(e["source"], p["source"]);
    assert_eq!(e["backgroundSourceRows"], p["backgroundSourceRows"]);
    assert_eq!(e["semanticReview"], "NOT_RUN");
    assert_eq!((l, p), before);
}
#[test]
fn keeps_unassigned_header_with_structured_gap_instead_of_guessing_first_scene() {
    let (l, p) = source_fixture();
    let out = mapped(&l, &p);
    assert_eq!(out.ready.len(), 1, "{out:?}");
    assert!(!out.ready[0]["unassignedRows"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(out.ready[0]["readyForSceneBaseline"], false);
    assert!(out
        .gaps
        .iter()
        .any(|g| g.code == "LITE_SUPPLY_SCENE_ASSIGNMENT_REQUIRED"));
    assert!(out.ready[0]["rowMappings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["sceneId"].is_null()));
}
#[test]
fn retains_utf16_ranges_and_original_scene_exactly() {
    let (l, p) = source_fixture();
    let out = mapped(&l, &p);
    let e = &out.ready[0];
    assert_eq!(e["sceneMappings"][0]["originalScene"], l["scenes"][0]);
    for r in e["rowMappings"].as_array().unwrap() {
        assert_eq!(
            r["globalRange"]["start"].as_u64().unwrap(),
            17 + r["localRange"]["start"].as_u64().unwrap()
        );
        assert_eq!(
            r["globalRange"]["end"].as_u64().unwrap(),
            17 + r["localRange"]["end"].as_u64().unwrap()
        );
    }
    let end = e["rowMappings"].as_array().unwrap().last().unwrap()["localRange"]["end"]
        .as_u64()
        .unwrap();
    assert_eq!(
        end,
        p["source"]["intakeEvidence"]["sourceText"]
            .as_str()
            .unwrap()
            .encode_utf16()
            .count() as u64
    );
}
#[test]
fn original_chinese_number_uses_explicit_decoder() {
    let (mut l, p) = source_fixture();
    l["episodes"][0]["sourceNumber"] = json!("二十八");
    assert_eq!(mapped(&l, &p).ready[0]["episodeNumber"], 28);
}
#[test]
fn missing_original_integer_never_falls_back_to_ordinal() {
    let (mut l, p) = source_fixture();
    l["episodes"][0]["sourceNumber"] = json!("序章");
    let o = mapped(&l, &p);
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_ORIGINAL_NUMBER_UNRESOLVED");
}
#[test]
fn duplicate_original_integer_reports_all_identities() {
    let (mut l, p) = source_fixture();
    l["episodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"another","sourceNumber":"28"}));
    let o = mapped(&l, &p);
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_ORIGINAL_NUMBER_COLLISION");
    assert_eq!(o.gaps[0].coordinates, vec!["desktop-ep", "another"]);
}
#[test]
fn source_verifier_is_mandatory_and_rejection_stops_mapping() {
    let (l, p) = source_fixture();
    let o = inspect_sources(&SourceGate { reject: true }, &l, &[p], &[]);
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_SOURCE_PROOF");
}
#[test]
fn tampered_standard_source_cannot_bypass_native_standard_gate() {
    let (l, mut p) = source_fixture();
    p["source"]["sourceRows"][0]["text"] = json!("changed");
    let o = mapped(&l, &p);
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_STANDARD_PROOF");
}
#[test]
fn missing_original_scene_is_not_a_null_mapping() {
    let (mut l, p) = source_fixture();
    l["scenes"] = json!([]);
    let o = mapped(&l, &p);
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_SCENE_UNKNOWN");
}
#[test]
fn row_coverage_holes_are_rejected() {
    let (l, mut p) = source_fixture();
    p["desktopBinding"]["rows"][1]["globalRange"]["start"] = json!(999);
    let o = mapped(&l, &p);
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_ROW_COVERAGE");
}
#[test]
fn cross_scene_entry_refuses_guessed_split() {
    let (mut l, mut p) = source_fixture();
    let row = p["desktopBinding"]["rows"][3].clone();
    let start = row["globalRange"]["start"].as_u64().unwrap();
    p["desktopBinding"]["rows"][3]["globalRange"]["end"] = json!(start + 2);
    let mut other = row;
    other["globalRange"]["start"] = json!(start + 2);
    other["sceneId"] = json!("other-scene");
    p["desktopBinding"]["rows"]
        .as_array_mut()
        .unwrap()
        .push(other);
    l["episodes"][0]["sceneIds"]
        .as_array_mut()
        .unwrap()
        .push(json!("other-scene"));
    l["scenes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"other-scene"}));
    let o = mapped(&l, &p);
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_CROSS_SCENE_ENTRY");
}
#[test]
fn repeated_episode_has_one_mapping_and_local_gap() {
    let (l, p) = source_fixture();
    let o = inspect_sources(&SourceGate { reject: false }, &l, &[p.clone(), p], &[]);
    assert_eq!(o.ready.len(), 1);
    assert!(o
        .gaps
        .iter()
        .any(|g| g.code == "LITE_SUPPLY_EPISODE_DUPLICATE"));
}

struct AdoptionGate {
    reject: bool,
    calls: Cell<usize>,
}
impl FrozenSeriesVerifier for AdoptionGate {
    fn verify(&self, _: &Connection, _: &Value, _: &Value, _: &str) -> Result<()> {
        self.calls.set(self.calls.get() + 1);
        if self.reject {
            Err("original accepted job/raw receipt unavailable".into())
        } else {
            Ok(())
        }
    }
}
fn gate() -> AdoptionGate {
    AdoptionGate {
        reject: false,
        calls: Cell::new(0),
    }
}
fn db() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE assets(id TEXT PRIMARY KEY,variant_id TEXT,version INTEGER,data TEXT); CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    c
}
fn asset(c: &Connection, a: &Value) {
    c.execute(
        "INSERT OR REPLACE INTO assets VALUES(?1,?2,?3,?4)",
        params![
            s(a, "id"),
            s(a, "variantId"),
            a["version"].as_u64().unwrap(),
            a.to_string()
        ],
    )
    .unwrap();
}
fn meta(c: &Connection, k: &str, v: &Value) {
    c.execute(
        "INSERT OR REPLACE INTO metadata VALUES(?1,?2)",
        params![k, v.to_string()],
    )
    .unwrap();
}
fn fixture(c: &Connection, id: &str) -> (Value, Value, Value, Value) {
    let raw = json!({"schemaVersion":"series-full-design-candidate/v1","designs":[{"assetId":id,"variantId":"v1","full":"完整设计：人物穿蓝色外套，保留原文限定的伤疤。","brief":"蓝衣人物","sourceRefs":["r1"],"sourceFacts":[{"statement":"原文出现人物","sourceRefs":["r1"]}],"productionChoices":["蓝色外套是制作设计"],"designDependencies":[]}]});
    let review = json!({"schemaVersion":"series-design-review/v1","candidateSha256":hash(&raw),"reviewedAssetIds":[id],"issues":[],"summary":"检查原文与完整设计"});
    let basis = json!({"kind":"delegated-text-review","designRequestSha256":"made","reviewRequestSha256":"checked","candidateSha256":hash(&raw)});
    let d = &raw["designs"][0];
    let adopted = json!({"version":"series-full-design/v1","status":"ADOPTED","assetId":id,"variantId":"v1","designRevision":1,"sourceSha256":"source-hash","sourceRefs":d["sourceRefs"],"sourceFacts":d["sourceFacts"],"productionChoices":d["productionChoices"],"full":d["full"],"brief":d["brief"],"designDependencies":[],"provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT","adoptionBasis":basis});
    let a = json!({"id":id,"variantId":"v1","version":9,"deleted":false,"kind":"CH","designRevision":1,"designStatus":"adopted","adoptedDesign":adopted});
    let input = json!({"workflowProfile":OLD_PROFILE,"source":{"sha256":"source-hash","rows":[{"id":"r1","text":"源文"}]}});
    let delivery = json!({"episodeId":id,"bundle":{"assets":[a]},"adoptionReceipt":{"bundleSha256":"bundle","roleReceipts":[{"stage":"seriesDesign","requestSha256":"made","rawSha256":hash(&raw),"request":{"input":{"workflowProfile":OLD_PROFILE}},"raw":raw},{"stage":"seriesDesignReview","requestSha256":"checked","rawSha256":hash(&review),"request":{"input":{"workflowProfile":OLD_PROFILE}},"raw":review}]},"designAdoptions":[{"assetId":id,"variantId":"v1","baseDesignRevision":0,"designRevision":1,"adoptedDesign":adopted,"designReview":{"requestSha256":"checked","resultSha256":hash(&review),"issues":[]}}]});
    let publication = json!({"bundleSha256":"bundle","workflowId":"wf1"});
    asset(c, &a);
    meta(c, &format!("series.input:{id}"), &input);
    meta(c, &format!("series.delivery:{id}"), &delivery);
    meta(c, &format!("series.publication:{id}"), &publication);
    (a, input, delivery, publication)
}
fn designs(c: &Connection, ids: &[&str], g: &AdoptionGate) -> Inspection {
    inspect_designs(
        c,
        "source-hash",
        &ids.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        g,
    )
}
fn snapshot(c: &Connection) -> Vec<(String, String, String)> {
    let mut out = vec![];
    for table in ["assets", "metadata"] {
        let sql = if table == "assets" {
            "SELECT id,data FROM assets ORDER BY id"
        } else {
            "SELECT key,value FROM metadata ORDER BY key"
        };
        let mut q = c.prepare(sql).unwrap();
        for r in q
            .query_map([], |r| Ok((table.to_string(), r.get(0)?, r.get(1)?)))
            .unwrap()
        {
            out.push(r.unwrap())
        }
    }
    out
}
#[test]
fn exact_ordinary_legacy_evidence_is_not_lite_adoption_and_never_writes() {
    let c = db();
    let (a, _, _, _) = fixture(&c, "a");
    let before = snapshot(&c);
    let g = gate();
    let o = designs(&c, &["a"], &g);
    assert!(o.gaps.is_empty(), "{o:?}");
    assert_eq!(g.calls.get(), 1);
    assert_eq!(o.ready[0]["adoptedDesign"], a["adoptedDesign"]);
    assert_eq!(o.ready[0]["liteRegistryWritten"], false);
    assert_eq!(o.ready[0]["liteDesignAdopted"], false);
    assert_eq!(snapshot(&c), before);
    assert!(!serde_json::to_string(&o).unwrap().contains("TEXT_ADOPTED"));
}
#[test]
fn claimed_adoption_without_original_publication_is_a_gap() {
    let c = db();
    fixture(&c, "a");
    c.execute("DELETE FROM metadata", []).unwrap();
    let o = designs(&c, &["a"], &gate());
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_ADOPTION_EVIDENCE_MISSING");
}
#[test]
fn original_job_receipt_gate_failure_cannot_be_promoted() {
    let c = db();
    fixture(&c, "a");
    let g = AdoptionGate {
        reject: true,
        calls: Cell::new(0),
    };
    let o = designs(&c, &["a"], &g);
    assert!(o.ready.is_empty());
    assert_eq!(g.calls.get(), 1);
    assert!(o.gaps[0].detail.contains("accepted job/raw"));
}
#[test]
fn prompt_media_and_brief_are_not_full_design() {
    for replacement in [
        Value::Null,
        json!({"full":"prompt","brief":"prompt"}),
        json!({"status":"ADOPTED"}),
    ] {
        let c = db();
        let (mut a, _, _, _) = fixture(&c, "a");
        a["adoptedDesign"] = replacement;
        a["imagePrompt"] = json!("a good prompt");
        a["imagePath"] = json!("image.png");
        asset(&c, &a);
        let o = designs(&c, &["a"], &gate());
        assert!(o.ready.is_empty());
        assert_eq!(o.gaps[0].code, "LITE_SUPPLY_DESIGN_UNPROVEN");
    }
}
#[test]
fn stale_deleted_source_variant_revision_and_full_drift_all_refuse() {
    for (pointer, value) in [
        ("/designStatus", json!("stale")),
        ("/deleted", json!(true)),
        ("/adoptedDesign/sourceSha256", json!("other")),
        ("/adoptedDesign/variantId", json!("other")),
        ("/adoptedDesign/designRevision", json!(2)),
        ("/adoptedDesign/full", json!("偷偷修改完整设计")),
    ] {
        let c = db();
        let (mut a, _, _, _) = fixture(&c, "a");
        *a.pointer_mut(pointer)
            .unwrap_or_else(|| panic!("{pointer}")) = value;
        asset(&c, &a);
        assert!(designs(&c, &["a"], &gate()).ready.is_empty(), "{pointer}");
    }
}
#[test]
fn sql_identity_version_disagreement_is_a_gap() {
    let c = db();
    fixture(&c, "a");
    c.execute("UPDATE assets SET version=10 WHERE id='a'", [])
        .unwrap();
    let o = designs(&c, &["a"], &gate());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_ASSET_STORAGE");
}
#[test]
fn om_voice_and_unknown_do_not_acquire_visual_identity() {
    for (kind, code) in [
        ("OM", "LITE_SUPPLY_UNSUPPORTED_OM"),
        ("voice", "LITE_SUPPLY_VOICE_IDENTITY_ONLY"),
        ("new-kind", "LITE_SUPPLY_UNKNOWN_KIND"),
    ] {
        let c = db();
        let (mut a, _, _, _) = fixture(&c, "a");
        a["kind"] = json!(kind);
        asset(&c, &a);
        let o = designs(&c, &["a"], &gate());
        assert!(o.ready.is_empty());
        assert_eq!(o.gaps[0].code, code);
    }
}
#[test]
fn beast_kind_is_preserved_in_evidence() {
    let c = db();
    let (mut a, _, _, _) = fixture(&c, "a");
    a["kind"] = json!("BEAST");
    asset(&c, &a);
    let o = designs(&c, &["a"], &gate());
    assert_eq!(o.ready[0]["upstreamKind"], "BEAST");
}
#[test]
fn publication_profile_and_review_corruption_are_local_refusals() {
    for which in 0..6 {
        let c = db();
        let (_, mut input, mut d, mut publication) = fixture(&c, "a");
        match which {
            0 => input["workflowProfile"] = json!("lite"),
            1 => publication["bundleSha256"] = json!("wrong"),
            2 => d["adoptionReceipt"]["roleReceipts"][1]["raw"]["candidateSha256"] = json!("wrong"),
            3 => d["adoptionReceipt"]["roleReceipts"][1]["raw"]["issues"] = json!(["bad"]),
            4 => {
                d["adoptionReceipt"]["roleReceipts"]
                    .as_array_mut()
                    .unwrap()
                    .remove(0);
            }
            _ => input["source"]["rows"] = json!([]),
        }
        meta(&c, "series.input:a", &input);
        meta(&c, "series.delivery:a", &d);
        meta(&c, "series.publication:a", &publication);
        let g = gate();
        let o = designs(&c, &["a"], &g);
        assert!(o.ready.is_empty(), "case {which}");
        assert_eq!(g.calls.get(), 0, "case {which}");
    }
}
#[test]
fn one_missing_asset_does_not_block_valid_other_evidence() {
    let c = db();
    fixture(&c, "a");
    let o = designs(&c, &["missing", "a", "a"], &gate());
    assert_eq!(o.ready.len(), 1);
    assert_eq!(o.gaps.len(), 2);
    assert_eq!(o.gaps[0].asset_id.as_deref(), Some("missing"));
    assert_eq!(o.gaps[1].code, "LITE_SUPPLY_ASSET_DUPLICATE");
}
#[test]
fn supplemental_or_imported_basis_not_relabelled_ordinary() {
    let c = db();
    let (mut a, _, _, _) = fixture(&c, "a");
    a["adoptedDesign"]["adoptionBasis"]["kind"] = json!("external-import");
    asset(&c, &a);
    let o = designs(&c, &["a"], &gate());
    assert!(o.ready.is_empty());
}
fn with_dependency(c: &Connection, id: &str, parent: &str) {
    let (mut a, _, mut d, _) = fixture(c, id);
    let deps = json!([{"assetId":parent,"designRevision":1}]);
    d["adoptionReceipt"]["roleReceipts"][0]["raw"]["designs"][0]["designDependencies"] =
        deps.clone();
    let candidate_hash = hash(&d["adoptionReceipt"]["roleReceipts"][0]["raw"]);
    d["adoptionReceipt"]["roleReceipts"][0]["rawSha256"] = json!(candidate_hash);
    d["adoptionReceipt"]["roleReceipts"][1]["raw"]["candidateSha256"] = json!(candidate_hash);
    let review_hash = hash(&d["adoptionReceipt"]["roleReceipts"][1]["raw"]);
    d["adoptionReceipt"]["roleReceipts"][1]["rawSha256"] = json!(review_hash);
    a["adoptedDesign"]["designDependencies"] = deps;
    a["adoptedDesign"]["adoptionBasis"]["candidateSha256"] = json!(candidate_hash);
    d["bundle"]["assets"][0] = a.clone();
    d["designAdoptions"][0]["adoptedDesign"] = a["adoptedDesign"].clone();
    d["designAdoptions"][0]["designReview"]["resultSha256"] = json!(review_hash);
    asset(c, &a);
    meta(c, &format!("series.delivery:{id}"), &d);
}
#[test]
fn complete_three_level_design_evidence_is_supplied_without_media_wait() {
    let c = db();
    fixture(&c, "base");
    with_dependency(&c, "suit", "base");
    with_dependency(&c, "hurt", "suit");
    let o = designs(&c, &["hurt", "suit", "base"], &gate());
    assert!(o.gaps.is_empty(), "{o:?}");
    assert_eq!(o.ready.len(), 3);
    assert_eq!(
        o.ready[0]["dependenciesRequireOwnEvidence"][0]["assetId"],
        "suit"
    );
}
#[test]
fn unsupplied_parent_proof_never_auto_expands_scope() {
    let c = db();
    fixture(&c, "base");
    with_dependency(&c, "suit", "base");
    let before = snapshot(&c);
    let o = designs(&c, &["suit"], &gate());
    assert!(o.ready.is_empty());
    assert_eq!(o.gaps[0].code, "LITE_SUPPLY_DESIGN_DEPENDENCY_UNPROVEN");
    assert_eq!(o.gaps[0].coordinates, vec!["suit", "base"]);
    assert_eq!(snapshot(&c), before);
}
#[test]
fn dependency_revision_deleted_or_missing_is_a_local_gap() {
    for variant in 0..3 {
        let c = db();
        let (mut base, _, _, _) = fixture(&c, "base");
        with_dependency(&c, "suit", "base");
        match variant {
            0 => {
                base["adoptedDesign"]["designRevision"] = json!(2);
                asset(&c, &base);
            }
            1 => {
                base["deleted"] = json!(true);
                asset(&c, &base);
            }
            _ => {
                c.execute("DELETE FROM assets WHERE id='base'", []).unwrap();
            }
        }
        let o = designs(&c, &["suit"], &gate());
        assert!(o.ready.is_empty());
        assert_eq!(o.gaps[0].code, "LITE_SUPPLY_DESIGN_DEPENDENCY_STALE");
    }
}
#[test]
fn cycles_report_exact_chain_and_do_not_block_unrelated_asset() {
    let c = db();
    with_dependency(&c, "a", "b");
    with_dependency(&c, "b", "a");
    fixture(&c, "independent");
    let o = designs(&c, &["a", "b", "independent"], &gate());
    assert_eq!(o.ready.len(), 1);
    assert_eq!(o.ready[0]["assetId"], "independent");
    assert!(o
        .gaps
        .iter()
        .any(|g| g.code == "LITE_SUPPLY_DESIGN_DEPENDENCY_CYCLE"
            && g.coordinates == vec!["a", "b", "a"]));
}
#[test]
fn parent_with_fake_adopted_flag_blocks_child_even_when_shape_is_complete() {
    let c = db();
    fixture(&c, "base");
    with_dependency(&c, "suit", "base");
    c.execute("DELETE FROM metadata WHERE key LIKE '%:base'", [])
        .unwrap();
    let o = designs(&c, &["suit", "base"], &gate());
    assert!(o.ready.is_empty());
    assert!(o.gaps.iter().any(|g| g.asset_id.as_deref() == Some("suit")
        && g.code == "LITE_SUPPLY_DESIGN_DEPENDENCY_UNPROVEN"));
}
