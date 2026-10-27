use super::*;
fn parse(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|e| e.to_string())
}
fn meta(c: &Connection, key: &str) -> Result<Value> {
    let raw: Option<String> = c
        .query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    parse(&raw.ok_or_else(|| format!("missing metadata {key}"))?)
}
fn current(c: &Connection, id: &str) -> Result<Value> {
    let (raw, version, variant): (String, u64, String) = c
        .query_row(
            "SELECT data,version,variant_id FROM assets WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let asset = parse(&raw)?;
    if asset["id"] != id || asset["version"] != version || asset["variantId"] != variant {
        return Err("asset row identity/version differs from stored fields".into());
    }
    Ok(asset)
}
fn design_shape(asset: &Value, source_hash: &str) -> Result<()> {
    let d = &asset["adoptedDesign"];
    if asset["deletedAt"].as_str().is_some_and(|s| !s.is_empty()) || asset["deleted"] == true {
        return Err("asset deleted".into());
    }
    if matches!(
        s(asset, "designStatus"),
        "needs_review" | "stale" | "rejected" | "invalid"
    ) {
        return Err("design review blocked".into());
    }
    if d["version"] != "series-full-design/v1"
        || d["status"] != "ADOPTED"
        || d["assetId"] != asset["id"]
        || d["variantId"] != asset["variantId"]
        || d["sourceSha256"] != source_hash
        || d["provenance"] != "ADOPTED_DESIGN_NOT_SOURCE_FACT"
        || d["designRevision"] != asset["designRevision"]
        || d["designRevision"].as_u64().is_none_or(|n| n == 0)
        || !text(&d["full"])
        || !text(&d["brief"])
        || d["full"] == d["brief"]
    {
        return Err("missing/currently unsupported complete adopted design contract; prompt/media/brief are not substitutes".into());
    }
    if array(&d["sourceRefs"])?.is_empty()
        || !d["sourceFacts"].is_array()
        || !d["productionChoices"].is_array()
        || !d["designDependencies"].is_array()
    {
        return Err("incomplete source/design fields".into());
    }
    Ok(())
}
fn original_design_roles(input: &Value, delivery: &Value, asset: &Value) -> Result<Value> {
    let d = &asset["adoptedDesign"];
    let basis = &d["adoptionBasis"];
    if basis["kind"] != "delegated-text-review" {
        return Err("unsupported adoption basis; cannot reclassify imported/supplement records as ordinary design".into());
    }
    let roles = array(&delivery["adoptionReceipt"]["roleReceipts"])?;
    let find = |key: &str| -> Result<&Value> {
        let matches: Vec<_> = roles
            .iter()
            .filter(|r| r["requestSha256"] == basis[key])
            .collect();
        if matches.len() != 1 {
            return Err(format!(
                "original {key} must identify exactly one role receipt"
            ));
        }
        Ok(matches[0])
    };
    let made = find("designRequestSha256")?;
    let checked = find("reviewRequestSha256")?;
    if made["stage"] != "seriesDesign"
        || checked["stage"] != "seriesDesignReview"
        || made["request"]["input"]["workflowProfile"] != OLD_PROFILE
        || checked["request"]["input"]["workflowProfile"] != OLD_PROFILE
        || made["raw"]["schemaVersion"] != "series-full-design-candidate/v1"
        || checked["raw"]["schemaVersion"] != "series-design-review/v1"
        || checked["raw"]["candidateSha256"] != hash(&made["raw"])
        || basis["candidateSha256"] != hash(&made["raw"])
        || checked["raw"]["issues"] != json!([])
        || !text(&checked["raw"]["summary"])
        || !checked["raw"]["reviewedAssetIds"]
            .as_array()
            .is_some_and(|a| a.contains(&asset["id"]))
    {
        return Err(
            "original design/review is absent, unpassed, or bound to another candidate".into(),
        );
    }
    let authored: Vec<_> = array(&made["raw"]["designs"])?
        .iter()
        .filter(|v| v["assetId"] == asset["id"] && v["variantId"] == asset["variantId"])
        .collect();
    if authored.len() != 1 {
        return Err("original candidate does not contain one exact appearance".into());
    }
    let authored = authored[0];
    let expected = json!({"version":"series-full-design/v1","status":"ADOPTED","assetId":asset["id"],"variantId":asset["variantId"],"designRevision":asset["designRevision"],"sourceSha256":input["source"]["sha256"],"sourceRefs":authored["sourceRefs"],"sourceFacts":authored["sourceFacts"],"productionChoices":authored["productionChoices"],"full":authored["full"],"brief":authored["brief"],"designDependencies":authored["designDependencies"],"provenance":"ADOPTED_DESIGN_NOT_SOURCE_FACT","adoptionBasis":basis});
    if *d != expected {
        return Err("current adopted design differs from original raw fields".into());
    }
    let source_rows = array(&input["source"]["rows"])?;
    if array(&d["sourceRefs"])?
        .iter()
        .any(|r| !source_rows.iter().any(|row| row["id"] == *r))
    {
        return Err("design source anchor is absent from original locked source".into());
    }
    for fact in array(&d["sourceFacts"])? {
        if !text(&fact["statement"])
            || array(&fact["sourceRefs"])?
                .iter()
                .any(|r| !array(&d["sourceRefs"]).unwrap().contains(r))
        {
            return Err("source fact anchor differs from design source scope".into());
        }
    }
    let adoptions = array(&delivery["designAdoptions"])?;
    let matches: Vec<_> = adoptions
        .iter()
        .filter(|a| {
            a["assetId"] == asset["id"]
                && a["designRevision"] == asset["designRevision"]
                && a["adoptedDesign"] == *d
        })
        .collect();
    if matches.len() != 1 {
        return Err("published delivery lacks unique matching design adoption".into());
    }
    let adoption = matches[0];
    if adoption["variantId"] != asset["variantId"]
        || adoption["designReview"]["requestSha256"] != checked["requestSha256"]
        || adoption["designReview"]["resultSha256"] != checked["rawSha256"]
        || adoption["designReview"]["issues"] != json!([])
        || adoption["baseDesignRevision"]
            .as_u64()
            .and_then(|n| n.checked_add(1))
            != asset["designRevision"].as_u64()
    {
        return Err("design adoption version/review differs".into());
    }
    Ok(json!({"designReceipt":made,"reviewReceipt":checked,"designAdoption":adoption}))
}

/// Caller owns a consistent read transaction. This does not migrate any table.
/// A verified design remains source-profile evidence, never a lite adoption.
pub(crate) fn inspect_designs(
    c: &Connection,
    source_hash: &str,
    asset_ids: &[String],
    verifier: &dyn FrozenSeriesVerifier,
) -> Inspection {
    let mut out = Inspection::new();
    let mut seen = BTreeSet::new();
    let mut cache: BTreeMap<String, Result<()>> = BTreeMap::new();
    // Parse each publication once, not once per selected asset. Full deliveries
    // contain substantial source and role packets in a multi-episode project.
    let deliveries: Result<Vec<(String, Result<Value>)>> = (|| {
        let mut q = c
            .prepare(
                "SELECT key,value FROM metadata WHERE key LIKE 'series.delivery:%' ORDER BY key",
            )
            .map_err(|e| e.to_string())?;
        let result = q
            .query_map([], |r| {
                let raw: String = r.get(1)?;
                Ok((r.get(0)?, parse(&raw)))
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        result
    })();
    for id in asset_ids {
        if !seen.insert(id) {
            out.gaps.push(gap(
                "LITE_SUPPLY_ASSET_DUPLICATE",
                None,
                Some(id),
                vec![],
                "asset selected more than once",
            ));
            continue;
        }
        let asset = match current(c, id) {
            Ok(v) => v,
            Err(e) => {
                out.gaps
                    .push(gap("LITE_SUPPLY_ASSET_STORAGE", None, Some(id), vec![], e));
                continue;
            }
        };
        let kind = s(&asset, "kind");
        if matches!(kind, "OM" | "GROUP" | "other" | "群演") {
            out.gaps.push(gap(
                "LITE_SUPPLY_UNSUPPORTED_OM",
                None,
                Some(id),
                vec![],
                "old OM cannot be silently retyped to CH",
            ));
            continue;
        }
        if matches!(kind, "SE" | "audio" | "voice" | "声音")
            || asset["preparation"]["mode"] == "AUDIO"
        {
            out.gaps.push(gap(
                "LITE_SUPPLY_VOICE_IDENTITY_ONLY",
                None,
                Some(id),
                vec![],
                "voice source identity is retained outside visual design registry",
            ));
            continue;
        }
        if !matches!(
            kind,
            "CH" | "character"
                | "人物"
                | "BEAST"
                | "SC"
                | "LOC"
                | "scene"
                | "location"
                | "场景"
                | "PR"
                | "PROP"
                | "prop"
                | "道具"
        ) {
            out.gaps.push(gap(
                "LITE_SUPPLY_UNKNOWN_KIND",
                None,
                Some(id),
                vec![],
                kind,
            ));
            continue;
        }
        if let Err(e) = design_shape(&asset, source_hash) {
            out.gaps.push(gap(
                "LITE_SUPPLY_DESIGN_UNPROVEN",
                None,
                Some(id),
                vec![],
                e,
            ));
            continue;
        }
        let rows = match &deliveries {
            Ok(v) => v,
            Err(e) => {
                out.gaps.push(gap(
                    "LITE_SUPPLY_ADOPTION_STORAGE",
                    None,
                    Some(id),
                    vec![],
                    e,
                ));
                continue;
            }
        };
        let mut failure_reasons = Vec::new();
        let mut evidence = None;
        for (key, parsed) in rows {
            let delivered = match parsed {
                Ok(v) => v,
                Err(e) => {
                    failure_reasons.push(format!("{key}: {e}"));
                    continue;
                }
            };
            if !delivered["bundle"]["assets"]
                .as_array()
                .is_some_and(|assets| {
                    assets.iter().any(|a| {
                        a["id"] == *id
                            && a["variantId"] == asset["variantId"]
                            && a["adoptedDesign"] == asset["adoptedDesign"]
                    })
                })
            {
                continue;
            }
            let episode = &key["series.delivery:".len()..];
            let checked: Result<Value> = (|| {
                let input = meta(c, &format!("series.input:{episode}"))?;
                let publication = meta(c, &format!("series.publication:{episode}"))?;
                if input["workflowProfile"] != OLD_PROFILE
                    || input["source"]["sha256"] != source_hash
                    || delivered["episodeId"] != episode
                    || publication["bundleSha256"] != delivered["adoptionReceipt"]["bundleSha256"]
                    || !text(&publication["workflowId"])
                {
                    return Err("original profile/source/publication binding differs".into());
                }
                let proof = original_design_roles(&input, &delivered, &asset)?;
                let verified = cache.entry(key.clone()).or_insert_with(|| {
                    verifier.verify(c, &input, &delivered, s(&publication, "workflowId"))
                });
                verified.clone()?;
                Ok(
                    json!({"sourceProfile":OLD_PROFILE,"workflowId":publication["workflowId"],"deliveryKey":key,"inputKey":format!("series.input:{episode}"),"publicationKey":format!("series.publication:{episode}"),"frozenInputSha256":hash(&input),"deliverySha256":hash(&delivered),"publicationSha256":hash(&publication),"proof":proof}),
                )
            })();
            match checked {
                Ok(v) => {
                    evidence = Some(v);
                    break;
                }
                Err(e) => failure_reasons.push(format!("{key}: {e}")),
            }
        }
        let Some(evidence) = evidence else {
            out.gaps.push(gap(
                "LITE_SUPPLY_ADOPTION_EVIDENCE_MISSING",
                None,
                Some(id),
                vec![],
                if failure_reasons.is_empty() {
                    "no originating adopted design publication found".into()
                } else {
                    failure_reasons.join("; ")
                },
            ));
            continue;
        };
        let mut dependency_gaps = Vec::new();
        for dep in asset["adoptedDesign"]["designDependencies"]
            .as_array()
            .unwrap()
        {
            let target = s(dep, "assetId");
            match current(c, target) {
                Ok(parent)
                    if parent["adoptedDesign"]["designRevision"] == dep["designRevision"]
                        && target != id
                        && design_shape(&parent, source_hash).is_ok() => {}
                _ => dependency_gaps.push(target.to_owned()),
            }
        }
        if !dependency_gaps.is_empty() {
            out.gaps.push(gap(
                "LITE_SUPPLY_DESIGN_DEPENDENCY_STALE",
                None,
                Some(id),
                dependency_gaps,
                "current dependency identity/design revision missing or changed",
            ));
            continue;
        }
        out.ready.push(json!({"version":VERSION,"kind":"verified-legacy-design-evidence","assetId":asset["id"],"variantId":asset["variantId"],"identityId":asset["identityId"],"baseAssetId":asset["baseAssetId"],"upstreamKind":asset["kind"],"assetStorageVersion":asset["version"],"currentAsset":asset,"assetSnapshotSha256":hash(&asset),"sourceSha256":source_hash,"designRevision":asset["designRevision"],"adoptedDesign":asset["adoptedDesign"],"designSha256":hash(&asset["adoptedDesign"]),"legacyEvidence":evidence,"dependenciesRequireOwnEvidence":asset["adoptedDesign"]["designDependencies"],"liteRegistryWritten":false,"liteDesignAdopted":false}));
    }
    // A parent being present in the database is not adoption evidence. Only the
    // explicitly requested, independently proved closure is eligible; no hidden
    // ancestor expansion, registry writing, or inference of design dependencies.
    let ready = out.ready.clone();
    out.ready.retain(|root| {
        let mut active=Vec::new();let mut done=BTreeSet::new();
        match verify_dependency_closure(s(root,"assetId"),&ready,&mut active,&mut done) {
            Ok(())=>true,
            Err((code,chain))=>{out.gaps.push(gap(code,None,Some(s(root,"assetId")),chain,"explicitly supplied dependency proof is missing or cyclic; no hidden ancestor expansion"));false}
        }
    });
    out
}
fn verify_dependency_closure(
    id: &str,
    ready: &[Value],
    active: &mut Vec<String>,
    done: &mut BTreeSet<String>,
) -> std::result::Result<(), (&'static str, Vec<String>)> {
    if active.iter().any(|s| s == id) {
        let mut chain = active.clone();
        chain.push(id.into());
        return Err(("LITE_SUPPLY_DESIGN_DEPENDENCY_CYCLE", chain));
    }
    if done.contains(id) {
        return Ok(());
    }
    let Some(record) = ready.iter().find(|r| r["assetId"] == id) else {
        let mut chain = active.clone();
        chain.push(id.into());
        return Err(("LITE_SUPPLY_DESIGN_DEPENDENCY_UNPROVEN", chain));
    };
    active.push(id.into());
    for dependency in record["dependenciesRequireOwnEvidence"]
        .as_array()
        .into_iter()
        .flatten()
    {
        verify_dependency_closure(s(dependency, "assetId"), ready, active, done)?;
    }
    active.pop();
    done.insert(id.into());
    Ok(())
}
