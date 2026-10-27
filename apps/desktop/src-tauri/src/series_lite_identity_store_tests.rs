use super::*;

fn identity(id:&str,variant:&str)->Identity {Identity{desktop_asset_id:id.into(),variant_id:variant.into(),
    identity_id:Some("person-one".into()),base_asset_id:Some("person-one".into()),kind:"CH".into()}}
fn create_assets(c:&Connection) {
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE assets(id TEXT PRIMARY KEY,variant_id TEXT NOT NULL,version INTEGER NOT NULL,data TEXT NOT NULL);").unwrap();
}
fn asset(c:&Connection,id:&Identity,version:u64,extras:Value) {
    let mut data=json!({"id":id.desktop_asset_id,"variantId":id.variant_id,"identityId":id.identity_id,"baseAssetId":id.base_asset_id,
        "kind":id.kind,"version":version,"name":"同名角色","placeholderOrdinal":900});
    for (k,v) in extras.as_object().unwrap(){data[k]=v.clone();}
    c.execute("INSERT INTO assets(id,variant_id,version,data) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET variant_id=excluded.variant_id,version=excluded.version,data=excluded.data",
        params![id.desktop_asset_id,id.variant_id,version,data.to_string()]).unwrap();
}
fn transact<T>(c:&mut Connection,f:impl FnOnce(&mut Transaction<'_>)->Result<T>)->Result<T> {
    let mut tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let result=f(&mut tx);tx.commit().unwrap();result
}
fn fixture()->Connection {
    let mut c=Connection::open_in_memory().unwrap();create_assets(&c);transact(&mut c,|tx|install(tx)).unwrap();c
}
fn reserve(c:&mut Connection,rev:u64,id:&Identity,version:u64)->Result<Reservation> {
    transact(c,|tx|reserve_identity(tx,"registry-one",rev,id,version))
}

#[test]
fn lite_identity_old_database_reads_are_byte_preserving_and_install_is_explicit() {
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("old.sqlite");
    {let c=Connection::open(&path).unwrap();create_assets(&c);asset(&c,&identity("one","base"),1,json!({}));}
    let before=std::fs::read(&path).unwrap();
    {let c=Connection::open_with_flags(&path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert!(read(&c,"registry-one","one").unwrap().is_none());assert!(read_registry(&c,"registry-one").unwrap().is_none());}
    assert_eq!(std::fs::read(&path).unwrap(),before);
    let mut c=Connection::open(&path).unwrap();
    assert!(reserve(&mut c,0,&identity("one","base"),1).unwrap_err().contains("INSTALL_REQUIRED"));
    {let tx=c.transaction().unwrap();install(&tx).unwrap();tx.rollback().unwrap();}
    assert!(read_registry(&c,"registry-one").unwrap().is_none());
}

#[test]
fn lite_identity_same_names_and_owner_with_different_appearances_never_merge_or_sort() {
    let mut c=fixture();let z=identity("z-last","base");let a=identity("a-first","blue");
    asset(&c,&z,1,json!({"placeholderOrdinal":5700}));asset(&c,&a,1,json!({"placeholderOrdinal":0}));
    let first=reserve(&mut c,0,&z,1).unwrap();let second=reserve(&mut c,1,&a,1).unwrap();
    assert_eq!(first.mapping.lite_asset_id,"A0001");assert_eq!(second.mapping.lite_asset_id,"A0002");
    // Reversing lookup order or passing an old map generation does not renumber.
    assert_eq!(reserve(&mut c,0,&a,1).unwrap().mapping,second.mapping);
    assert_eq!(reserve(&mut c,0,&z,1).unwrap().mapping,first.mapping);
    assert_eq!(read_registry(&c,"registry-one").unwrap().unwrap(),RegistryState{revision:2,next_ordinal:3});
    let text:String=c.query_row("SELECT identity_json FROM series_lite_identity_v1 WHERE desktop_asset_id='z-last'",[],|r|r.get(0)).unwrap();
    assert!(!text.contains("name")&&!text.contains("placeholder")&&!text.contains("ADOPTED"));
}

#[test]
fn lite_identity_prompt_media_and_usage_row_versions_preserve_mapping_without_semantic_adoption() {
    let mut c=fixture();let id=identity("one","base");asset(&c,&id,1,json!({}));
    let original=reserve(&mut c,0,&id,1).unwrap();
    for (index,extras) in [json!({"prompt":"手写提示词","imagePrompt":{"origin":"manual"}}),
        json!({"adoptedMediaId":"image-one","imageStatus":"adopted"}),json!({"uses":[{"episode":"ep2","scene":"s2"}],"name":"改了显示名称"})].into_iter().enumerate() {
        let version=index as u64+2;asset(&c,&id,version,extras);
        assert!(reserve(&mut c,0,&id,version-1).unwrap_err().contains("ASSET_CAS_CONFLICT"));
        let replay=reserve(&mut c,0,&id,version).unwrap();assert!(!replay.created);
        assert_eq!(replay.mapping,original.mapping);assert_eq!(replay.registry_revision,1);
    }
    let raw:String=c.query_row("SELECT data FROM assets WHERE id='one'",[],|r|r.get(0)).unwrap();
    assert!(serde_json::from_str::<Value>(&raw).unwrap()["adoptedDesign"].is_null());
    assert_eq!(read_registry(&c,"registry-one").unwrap().unwrap().next_ordinal,2);
}

#[test]
fn lite_identity_changed_variant_owner_or_kind_rejects_even_at_current_row_version() {
    for field in ["variant","identity","base","kind"] {
        let mut c=fixture();let original=identity("one","base");asset(&c,&original,1,json!({}));
        let mapping=reserve(&mut c,0,&original,1).unwrap().mapping;
        let mut changed=original.clone();match field {
            "variant"=>changed.variant_id="new-look".into(),"identity"=>changed.identity_id=Some("another-person".into()),
            "base"=>changed.base_asset_id=Some("another-person".into()),_=>changed.kind="PROP".into(),
        }
        asset(&c,&changed,2,json!({}));
        assert!(reserve(&mut c,1,&original,2).unwrap_err().contains("ASSET_BINDING_CHANGED"));
        assert!(reserve(&mut c,1,&changed,2).unwrap_err().contains("PERMANENT_BINDING_CONFLICT"));
        assert_eq!(read(&c,"registry-one","one").unwrap().unwrap(),mapping);
    }
}

#[test]
fn lite_identity_tombstones_reopen_restore_and_physical_missing_rows_never_recycle() {
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("project.sqlite");
    let id=identity("one","base");
    {let mut c=Connection::open(&path).unwrap();create_assets(&c);transact(&mut c,|tx|install(tx)).unwrap();
        asset(&c,&id,1,json!({}));reserve(&mut c,0,&id,1).unwrap();
        asset(&c,&id,2,json!({"deletedAt":"2026-10-27T00:00:00Z"}));
        let deleted=reserve(&mut c,1,&id,2).unwrap();assert!(deleted.mapping.tombstoned&&deleted.currently_deleted);
        assert_eq!(deleted.mapping.lite_asset_id,"A0001");}
    let mut c=Connection::open(&path).unwrap();let next=identity("two","white");asset(&c,&next,1,json!({}));
    assert_eq!(reserve(&mut c,2,&next,1).unwrap().mapping.lite_asset_id,"A0002");
    asset(&c,&id,3,json!({}));let restored=reserve(&mut c,0,&id,3).unwrap();
    assert!(restored.mapping.tombstoned&&!restored.currently_deleted);assert_eq!(restored.mapping.lite_asset_id,"A0001");
    assert!(c.execute("DELETE FROM series_lite_identity_v1 WHERE desktop_asset_id='one'",[]).is_err());
    assert!(c.execute("UPDATE series_lite_identity_v1 SET tombstoned=0 WHERE desktop_asset_id='one'",[]).is_err());
    c.execute("DELETE FROM assets WHERE id='one'",[]).unwrap();
    assert_eq!(read(&c,"registry-one","one").unwrap().unwrap(),restored.mapping);
    assert!(reserve(&mut c,3,&id,3).unwrap_err().contains("ASSET_MISSING"));
    let third=identity("three","other");asset(&c,&third,1,json!({}));assert_eq!(reserve(&mut c,3,&third,1).unwrap().mapping.lite_asset_id,"A0003");
}

#[test]
fn lite_identity_concurrent_connections_cas_allocate_at_most_one_from_same_generation() {
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("project.sqlite");
    {let mut c=Connection::open(&path).unwrap();create_assets(&c);transact(&mut c,|tx|install(tx)).unwrap();
        asset(&c,&identity("one","base"),1,json!({}));asset(&c,&identity("two","blue"),1,json!({}));}
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles:Vec<_>=[("one","base"),("two","blue")].into_iter().map(|(name,variant)|{
        let path=path.clone();let barrier=barrier.clone();std::thread::spawn(move||{
            let mut c=Connection::open(path).unwrap();c.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
            barrier.wait();reserve(&mut c,0,&identity(name,variant),1)
        })
    }).collect();
    let results:Vec<_>=handles.into_iter().map(|h|h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r|r.is_ok()).count(),1);
    assert!(results.iter().find_map(|r|r.as_ref().err()).unwrap().contains("REGISTRY_CAS_CONFLICT"));
    let c=Connection::open(path).unwrap();assert_eq!(read_registry(&c,"registry-one").unwrap().unwrap(),RegistryState{revision:1,next_ordinal:2});
}

#[test]
fn lite_identity_failed_insert_rolls_back_high_water_and_missing_counters_are_not_guessed() {
    let mut c=fixture();let id=identity("one","base");asset(&c,&id,1,json!({}));
    c.execute_batch("CREATE TRIGGER deny_mapping BEFORE INSERT ON series_lite_identity_v1 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(reserve(&mut c,0,&id,1).is_err());assert!(read_registry(&c,"registry-one").unwrap().is_none());
    c.execute_batch("DROP TRIGGER deny_mapping;").unwrap();reserve(&mut c,0,&id,1).unwrap();
    assert!(c.execute("UPDATE series_lite_identity_v1 SET lite_asset_id='A9999'",[]).is_err());
    c.execute("UPDATE series_lite_identity_registry_v1 SET next_ordinal=1",[]).unwrap();
    assert!(read_registry(&c,"registry-one").unwrap_err().contains("HIGH_WATER_CONFLICT"));
    assert!(reserve(&mut c,1,&id,1).unwrap_err().contains("HIGH_WATER_CONFLICT"));
}

#[test]
fn lite_identity_deleted_unmapped_and_stale_tombstone_request_never_allocate_or_overwrite() {
    let mut c=fixture();let id=identity("one","base");asset(&c,&id,1,json!({"deletedAt":"old"}));
    assert!(reserve(&mut c,0,&id,1).unwrap_err().contains("DELETED_UNMAPPED"));
    assert!(read_registry(&c,"registry-one").unwrap().is_none());
    asset(&c,&id,2,json!({}));let original=reserve(&mut c,0,&id,2).unwrap().mapping;
    asset(&c,&id,3,json!({"deletedAt":"now"}));
    assert!(reserve(&mut c,0,&id,3).unwrap_err().contains("REGISTRY_CAS_CONFLICT"));
    assert_eq!(read(&c,"registry-one","one").unwrap().unwrap(),original);
    assert_eq!(read_registry(&c,"registry-one").unwrap().unwrap(),RegistryState{revision:1,next_ordinal:2});
}
