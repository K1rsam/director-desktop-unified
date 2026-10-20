use super::*;

#[test]
fn images_are_copied_before_restore_projects_thumbnail_metadata_and_survive_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let engine = Engine::new_offline_verification(home.clone()).unwrap();
    let project = engine.dispatch(json!({"action":"projects.create","title":"synthetic backup image"})).unwrap();
    let project_id = s(&project, "id");
    let store = engine.store(project_id).unwrap();
    store.save_asset(json!({"id":"synthetic-prop","variantId":"base","name":"synthetic prop","kind":"PROP"})).unwrap();
    let image = temp.path().join("synthetic.png");
    image::RgbaImage::from_pixel(3, 2, image::Rgba([23, 57, 91, 255])).save(&image).unwrap();
    let original = store.import_media(&image, "image", Some("synthetic-prop"), Some("base")).unwrap();
    store.adopt("synthetic-prop", "base", s(&original, "id")).unwrap();
    let original_hash = crate::store::hash_file(&store.verify_media(&original).unwrap()).unwrap();
    let backup = engine.dispatch(json!({"action":"backup.create","projectId":project_id})).unwrap();
    let workspace = engine.dispatch(json!({"action":"backup.restore","projectId":project_id,"path":backup["path"]})).unwrap();
    let restored_id = s(&workspace["project"], "id").to_owned();
    assert_ne!(restored_id, project_id);
    let restored = engine.store(&restored_id).unwrap();
    let media = restored.values("media").unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0]["id"], original["id"]);
    assert_eq!(crate::store::hash_file(&restored.verify_media(&media[0]).unwrap()).unwrap(), original_hash);
    assert!(Path::new(s(&media[0], "thumbnailPath")).is_file());
    assert!(Path::new(s(&media[0], "path")).starts_with(&restored.root));
    let adopted:String = restored.conn().unwrap().query_row("SELECT media_id FROM adoptions WHERE asset_id='synthetic-prop' AND variant_id='base'", [], |row| row.get(0)).unwrap();
    assert_eq!(adopted, s(&original, "id"));
    assert_eq!(crate::store::hash_file(&store.verify_media(&original).unwrap()).unwrap(), original_hash);
    drop(restored);
    drop(store);
    drop(engine);
    let reopened = Engine::new_offline_verification(home).unwrap();
    let restored = reopened.store(&restored_id).unwrap();
    assert_eq!(restored.values("media").unwrap()[0]["sha256"], original["sha256"]);
    assert_eq!(restored.conn().unwrap().query_row("SELECT media_id FROM adoptions WHERE asset_id='synthetic-prop' AND variant_id='base'", [], |row| row.get::<_,String>(0)).unwrap(), adopted);
}
