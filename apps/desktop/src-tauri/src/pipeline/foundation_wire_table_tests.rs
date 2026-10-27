use super::*;
fn fixture() -> Value {
    json!([
        {"key":"a1","name":"甲｜原文不改","type":"CH","aliases":["a2","同名"],"variants":[{"key":"v1","name":"基础","fromKey":null,"preparation":{"mode":"REFERENCE","note":"原话😀","future":{"retain":[null,1,"一"]}}},{"key":"v2","name":"西装","fromKey":"v1","preparation":{"mode":"DERIVE","fromKey":"v1"}},{"key":"v3","name":"受伤","fromKey":"v2","preparation":null},{"key":"v4","name":"缺省","unknown":{"preparation":17}}],"unknown":[{"key":"a9","text":"不要变成身份"}]},
        {"key":"a2","name":"甲｜原文不改","type":"CH","aliases":[],"variants":[{"key":"v5","name":"独立","preparation":{"mode":"REFERENCE","note":"原话😀","future":{"retain":[null,1,"一"]}}}]},
        {"key":"a3","name":"未知未来形状","variants":null}
    ])
}
#[test]
fn catalogue_table_full_roundtrip_and_exact_order() {
    let f = fixture();
    let t = encode_catalogue(&f).unwrap();
    assert_eq!(decode_catalogue(&t).unwrap(), f);
    assert_eq!(t["preparations"].as_array().unwrap().len(), 3);
    assert_eq!(
        decode_catalogue(&t).unwrap()[0]["variants"][2]["fromKey"],
        "v2"
    );
    assert!(decode_catalogue(&t).unwrap()[0]["variants"][3]
        .get("preparation")
        .is_none());
}
#[test]
fn catalogue_table_markerless_frozen_input_unchanged() {
    let mut input = json!({"registry":fixture(),"source":{"text":"完整源文"}});
    let before = input.to_string();
    assert!(!pack_input(&mut input).unwrap());
    assert_eq!(input.to_string(), before);
    input["foundationCatalogueWire"] = json!("unknown");
    assert!(pack_input(&mut input).is_err());
}
#[test]
fn catalogue_table_detects_corrupt_shapes_values_and_digest() {
    let t = encode_catalogue(&fixture()).unwrap();
    for mode in 0..7 {
        let mut bad = t.clone();
        match mode {
            0 => bad["rows"][0][0] = json!(999),
            1 => {
                bad["rows"][0].as_array_mut().unwrap().pop();
            }
            2 => bad["assetShapes"][0][0] = json!("wrong"),
            3 => bad["preparations"][0] = json!("changed"),
            4 => bad["catalogueSha256"] = json!("bad"),
            5 => bad["extra"] = json!(1),
            _ => bad["interpretation"] = json!("accept all"),
        };
        assert!(decode_catalogue(&bad).is_err(), "{mode}");
    }
}
#[test]
fn catalogue_table_opt_in_after_typed_alias_preserves_complete_values() {
    let prep = json!({"mode":"DERIVE","fromKey":"v1","note":"同一完整制作说明，每个形象仍独立，不能按同名合并。"});
    let f=json!((0..250).map(|n|json!({"key":format!("a{n}"),"name":"人物同名","aliases":[],"type":"CH","variants":[{"key":format!("v{n}"),"name":"同名形象","fromKey":"v1","preparation":prep}]})).collect::<Vec<_>>());
    let mut input = json!({"foundationCatalogueWire":PROFILE,"registry":f,"source":{"text":"原文\r\n😀"},"evidence":{"claim":"CHARACTER_CLAIM"}});
    let original = input.clone();
    assert!(pack_input(&mut input).unwrap());
    assert_eq!(decode_catalogue(&input["registry"]).unwrap(), f);
    assert_eq!(input["source"], original["source"]);
    assert_eq!(input["evidence"], original["evidence"]);
    assert!(input.to_string().len() < original.to_string().len());
    assert!(!pack_input(&mut input).unwrap());
}
