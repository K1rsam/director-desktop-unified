#[tokio::test]
async fn sdk_series_supplement_identity_context_uses_owned_raw_for_both_versions() {
    for contract in [CONTRACT,CONTRACT_V2] {
        let(_temp,executor,chain,job)=fixture_without_identity_context().await;
        let(origin,raw)=&chain[0];let known=packet(&chain[1].0,"knownAssetCatalog");
        let author=executor.sdk_job(&job).unwrap();let snapshot=&author["snapshot"];
        assert!(executor.exact_workflow_v1_raw(origin,snapshot,&hash(snapshot)).unwrap().is_none(),"SDK must keep using the full owned-evidence verifier");
        let original_files=file_state(&executor,&job);
        let original_events=events(&executor.store.conn().unwrap(),&job).unwrap();
        let count=executor.store.jobs().unwrap().len();
        let context=executor.freeze_series_supplement_identity_context(origin,raw,known,contract).unwrap();
        let reopened=SavedTextExecutor{store:Store::open(executor.store.root.clone()).unwrap(),settings:executor.settings.clone(),workflow_id:executor.workflow_id.clone()};
        assert_eq!(reopened.freeze_series_supplement_identity_context(origin,raw,known,contract).unwrap(),context);
        assert_eq!(reopened.verified_series_supplement_identity_context(origin,raw,known,&author).unwrap(),context);
        assert_eq!(context["version"],if contract==CONTRACT {"series-supplement-identities/v1"}else{"series-supplement-identities/v2"});
        assert_eq!(reopened.sdk_job(&job).unwrap(),author,"freezing supply context cannot adopt or rewrite the rejected author");
        assert_eq!(events(&reopened.store.conn().unwrap(),&job).unwrap(),original_events);
        assert_eq!(file_state(&reopened,&job),original_files);
        assert_eq!(reopened.store.jobs().unwrap().len(),count);
        assert_eq!(std::fs::read_dir(reopened.store.root.join("sdk-worker")).unwrap().count(),1,"only the synthetic refusal evidence, no new worker");
    }
}

#[tokio::test]
async fn sdk_series_supplement_identity_context_tampered_owned_evidence_writes_nothing() {
    for frozen in [false,true] {
        for fault in ["raw","attempt","compiled","receipt","raw-event"] {
            let(_temp,executor,chain,job)=fixture_without_identity_context().await;
            let(origin,raw)=&chain[0];let known=packet(&chain[1].0,"knownAssetCatalog");
            if frozen {executor.freeze_series_supplement_identity_context(origin,raw,known,CONTRACT_V2).unwrap();}
            let directory=executor.store.root.join("text-runs").join(&job);
            match fault {
                "raw"=>std::fs::write(directory.join("result.json"),"{}").unwrap(),
                "attempt"=>std::fs::write(directory.join("sdk-attempt.json"),"{}").unwrap(),
                "compiled"=>std::fs::write(directory.join("sdk-compiled.json"),"{}").unwrap(),
                "receipt"=>executor.store.update_job(&job,"rejected",&json!({"receipt":{"contract":"desktop-runner/v1","validation":"rejected","unexpected":true}})).unwrap(),
                "raw-event"=>{executor.store.conn().unwrap().execute("UPDATE events SET data='{}' WHERE job_id=?1 AND type='text_received_raw'",[&job]).unwrap();},
                _=>unreachable!(),
            }
            let jobs=executor.store.jobs().unwrap();
            let origin_events=events(&executor.store.conn().unwrap(),&job).unwrap();
            let parent_events=events(&executor.store.conn().unwrap(),executor.workflow_id.as_deref().unwrap()).unwrap();
            let files=file_state(&executor,&job);
            assert!(executor.freeze_series_supplement_identity_context(origin,raw,known,CONTRACT_V2).is_err(),"frozen={frozen}, fault={fault}");
            assert_eq!(executor.store.jobs().unwrap(),jobs,"frozen={frozen}, fault={fault}");
            assert_eq!(events(&executor.store.conn().unwrap(),&job).unwrap(),origin_events);
            assert_eq!(events(&executor.store.conn().unwrap(),executor.workflow_id.as_deref().unwrap()).unwrap(),parent_events);
            assert_eq!(file_state(&executor,&job),files);
            assert_eq!(std::fs::read_dir(executor.store.root.join("sdk-worker")).unwrap().count(),1);
        }
    }
}

#[tokio::test]
async fn sdk_series_supplement_audio_prop_freezes_and_reopens_without_image_reservation() {
    for mode in ["AUDIO", "IMAGE"] {
        let (_temp, executor, chain, job) = fixture_without_identity_context().await;
        let (origin, raw) = &chain[0];
        let mut known = packet(&chain[1].0, "knownAssetCatalog").clone();
        let asset = executor.store.save_asset(json!({
            "id":"system-voice", "variantId":"base", "kind":"PROP", "name":"系统声音",
            "preparation":{"mode":mode, "note":"实际源记录决定媒介类型"}
        })).unwrap();
        known.as_array_mut().unwrap().push(asset);
        let assets = || {
            let c = executor.store.conn().unwrap();
            json!([
                "SELECT data FROM assets ORDER BY id",
                "SELECT data FROM asset_versions ORDER BY rowid",
                "SELECT value FROM metadata WHERE key='series.placeholder.nextOrdinal'"
            ].iter().map(|sql| {
                let mut q = c.prepare(sql).unwrap();
                let rows = q.query_map([], |r| r.get::<_, String>(0)).unwrap()
                    .collect::<std::result::Result<Vec<_>, _>>().unwrap();
                rows
            }).collect::<Vec<_>>())
        };
        let before_assets = assets();
        let before_jobs = executor.store.jobs().unwrap();
        let author = executor.sdk_job(&job).unwrap();
        let files = file_state(&executor, &job);
        let origin_events = events(&executor.store.conn().unwrap(), &job).unwrap();
        let run = executor.workflow_id.as_deref().unwrap();
        let parent_events = events(&executor.store.conn().unwrap(), run).unwrap();
        let result = executor.freeze_series_supplement_identity_context(origin, raw, &known, CONTRACT_V2);
        if mode == "AUDIO" {
            let context = result.unwrap();
            let compact = crate::pipeline::series::supplement_supply_catalogue(&known);
            let voice = compact.as_array().unwrap().iter().find(|a| a["id"] == "system-voice").unwrap();
            assert_eq!(voice["preparation"], json!({"mode":"AUDIO"}));
            assert!(voice.get("filename").is_none());
            assert!(voice.get("placeholderOrdinal").is_none());
            assert!(context["reservedAssets"].as_array().unwrap().iter().all(|a| a["id"] != "system-voice"));
            assert!(context["assetStates"].as_array().unwrap().iter().any(|a| a["assetId"] == "system-voice"));
            let reopened = SavedTextExecutor {
                store:Store::open(executor.store.root.clone()).unwrap(),
                settings:executor.settings.clone(), workflow_id:executor.workflow_id.clone()
            };
            assert_eq!(reopened.freeze_series_supplement_identity_context(origin, raw, &known, CONTRACT_V2).unwrap(), context);
            assert_eq!(reopened.verified_series_supplement_identity_context(origin, raw, &compact, &author).unwrap(), context);
            assert_eq!(events(&reopened.store.conn().unwrap(), run).unwrap().as_array().unwrap().iter()
                .filter(|e| e["type"] == "series_supplement_identities_frozen").count(), 1);
        } else {
            assert_eq!(result.unwrap_err(), "SERIES_SUPPLEMENT_ACTIVE_IDENTITY_INCOMPLETE");
            assert_eq!(executor.store.jobs().unwrap(), before_jobs);
            assert_eq!(events(&executor.store.conn().unwrap(), run).unwrap(), parent_events);
        }
        assert_eq!(assets(), before_assets, "no image identity, ordinal or asset version is written");
        assert_eq!(executor.sdk_job(&job).unwrap(), author);
        assert_eq!(events(&executor.store.conn().unwrap(), &job).unwrap(), origin_events);
        assert_eq!(file_state(&executor, &job), files);
        assert_eq!(executor.store.jobs().unwrap().len(), before_jobs.len());
        assert_eq!(std::fs::read_dir(executor.store.root.join("sdk-worker")).unwrap().count(), 1,
            "only the original synthetic refusal; no model worker or generation");
    }
}
