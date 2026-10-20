use super::*;
use serde_json::json;

fn fixture() -> Value {
    let id = "12345678-1234-4234-8234-123456789abc";
    let pin = include_str!("../../../../web-image-bridge/runtime.sha256").trim();
    let request = json!({"requestId":id,"kind":"image","model":"chatgpt-image","prompt":"synthetic only",
        "duration":null,"ratio":"16:9","resolution":"native","references":[],"options":{"synthetic":true}});
    let hash = "a".repeat(64); // Node's original wire hash, not Rust reserialization.
    let config = json!({"kind":"chrome","browserTransport":{"transport":"opencli/v1","profile":"synthetic-profile","accountKey":"synthetic-account","runtimeSha256":pin}});
    let mut snapshot = request.clone();
    snapshot["providerConfig"] = config;
    snapshot["frozenSettings"] = json!({"synthetic":true});
    let check = json!({"page":"synthetic-page","url":"https://chatgpt.com/","accountKey":"synthetic-account",
        "userCount":0,"assistantCount":0,"composerEmpty":true,"generating":false});
    json!({"provider":"chrome","kind":"image","status":"failed","providerTaskId":id,"snapshot":snapshot,
        "prepared":{"provider":"chrome","request":request,"wire":{"transport":"opencli/v1","requestHash":hash,"profile":"synthetic-profile","accountKey":"synthetic-account"}},
        "result":{"taskId":id,"status":"failed","resultUrls":[],"error":"未提交：原页面两次均为空，保留原失败记录。",
            "raw":{"transport":"opencli/v1","requestHash":hash,
                "opencliNotSubmitted":{"version":"opencli-pre-send-not-submitted/v1","requestId":id,"requestHash":hash,"runtimeSha256":pin,
                    "journalBeforeSha256":"b".repeat(64),"journalAfterSha256":"c".repeat(64),
                    "sendMarkerAbsent":true,"sendConfirmedAbsent":true,"conversationUrlAbsent":true,"page":"synthetic-page","checks":[check.clone(),check]},
                "taskSettlement":{"id":id,"status":"failed","reason":"not_submitted"}}}})
}

fn owned_fixture() -> Value {
    let mut job=fixture();
    let adapter=crate::pipeline::digest(include_str!("../../../../web-image-bridge/composer-transport.mjs"));
    job["prepared"]["wire"]["sendProtocol"]=json!("owned-composer-send/v2");
    job["prepared"]["wire"]["composerAdapterSha256"]=json!(adapter);
    let proof=&mut job["result"]["raw"]["opencliNotSubmitted"];
    proof["version"]=json!("opencli-owned-pre-send-not-submitted/v2");
    proof["sendProtocol"]=json!("owned-composer-send/v2");proof["composerAdapterSha256"]=json!(adapter);
    proof["clickIntentAbsent"]=json!(true);proof["clickCalls"]=json!(0);
    for check in proof["checks"].as_array_mut().unwrap(){check["composerEmpty"]=json!(false);}
    job
}

#[test]
fn opencli_owned_prior_exact_no_click_terminal_survives_upgrade_but_never_converts_an_unknown() {
    for (prior,adapter) in [
        ("f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70","0183767038c20ddebf218ddd297b2cf166aceee4d99bb7c03b7f73d7d3654e65"),
        ("54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0","5c850d49dba9164a61b499565be9eb21833da724aa6aad6f91a67ddbe86b717b"),
        ("afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6","5c850d49dba9164a61b499565be9eb21833da724aa6aad6f91a67ddbe86b717b"),
    ] {
    let mut job=owned_fixture();
    job["snapshot"]["providerConfig"]["browserTransport"]["runtimeSha256"]=json!(prior);
    job["prepared"]["wire"]["composerAdapterSha256"]=json!(adapter);
    job["result"]["raw"]["opencliNotSubmitted"]["runtimeSha256"]=json!(prior);
    job["result"]["raw"]["opencliNotSubmitted"]["composerAdapterSha256"]=json!(adapter);
    assert!(settled_job(&job));
    for (pointer,value) in [
        ("/status",json!("submission_unknown")),("/result/status",json!("unknown")),
        ("/prepared/wire/sendProtocol",json!("legacy")),
        ("/result/raw/opencliNotSubmitted/clickIntentAbsent",json!(false)),
        ("/result/raw/opencliNotSubmitted/clickCalls",json!(1)),
        ("/result/raw/opencliNotSubmitted/composerAdapterSha256",json!("0".repeat(64))),
        ("/snapshot/providerConfig/browserTransport/runtimeSha256",json!("0".repeat(64))),
    ] {let mut changed=job.clone();*changed.pointer_mut(pointer).unwrap()=value;assert!(!settled_job(&changed),"{pointer}");}
    for key in ["sentAt","sendConfirmed","sendIntent","conversationUrl"] {
        let mut changed=job.clone();changed["result"]["raw"][key]=Value::Null;assert!(!settled_job(&changed),"{key}");
    }
    }
}

#[test]
fn opencli_owned_no_click_partial_composer_requires_frozen_protocol_and_exact_adapter() {
    let original=owned_fixture();assert!(settled_job(&original));
    for (pointer,value) in [
        ("/prepared/wire/sendProtocol",json!("legacy")),
        ("/prepared/wire/composerAdapterSha256",json!("0".repeat(64))),
        ("/result/raw/opencliNotSubmitted/composerAdapterSha256",json!("0".repeat(64))),
        ("/result/raw/opencliNotSubmitted/clickIntentAbsent",json!(false)),
        ("/result/raw/opencliNotSubmitted/clickCalls",json!(1)),
        ("/result/raw/opencliNotSubmitted/checks/0/userCount",json!(1)),
        ("/result/raw/opencliNotSubmitted/checks/1/accountKey",json!("other")),
    ] {let mut job=original.clone();*job.pointer_mut(pointer).unwrap()=value;assert!(!settled_job(&job),"{pointer}");}
    let mut legacy=original.clone();legacy["result"]["raw"]["opencliNotSubmitted"]["version"]=json!("opencli-pre-send-not-submitted/v1");assert!(!settled_job(&legacy));
    for key in ["sentAt","sendConfirmed","conversationUrl","sendIntent"] {let mut job=original.clone();job["result"]["raw"][key]=Value::Null;assert!(!settled_job(&job),"{key}");}
}

fn migrated_fixture() -> Value {
    let mut job = fixture();
    let from = "054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2";
    let to = job["result"]["raw"]["opencliNotSubmitted"]["runtimeSha256"].clone();
    job["snapshot"]["providerConfig"]["browserTransport"]["runtimeSha256"] = json!(from);
    job["result"]["raw"]["readerMigration"] = json!({"schemaVersion":"opencli-readonly-reader-migration/v1",
        "operation":"query","bridgeOperation":"query","readOnly":true,"resubmitted":false,"frozenConfigUnchanged":true,
        "fromRuntimeSha256":from,"toRuntimeSha256":to,"fromRunnerSha256":"d".repeat(64),"toRunnerSha256":"e".repeat(64),
        "fromManifestSha256":"f".repeat(64),"toManifestSha256":"1".repeat(64),
        "frozenConfigSha256":crate::pipeline::digest(&job["snapshot"]["providerConfig"].to_string()),
        "generationRequestSha256":crate::pipeline::digest(&job["prepared"]["request"].to_string())});
    job
}

#[test]
fn opencli_saved_historical_reader_audit_retains_terminal_proof_without_new_migration_permission() {
    for prior in ["f35f97fdbfd3f40d9882b275c8cf13310488a162323f7598ab3557377b039e70","54e83e4190239ff9e248082c44008961b0675d9880bb584de026ede24b3c7ec0","afeca4bb4a1f8e59c9850e983882f05593721a2cee019c7a80ca216bf7d817c6"] {
    let mut job=migrated_fixture();
    job["result"]["raw"]["readerMigration"]["toRuntimeSha256"]=json!(prior);
    job["result"]["raw"]["opencliNotSubmitted"]["runtimeSha256"]=json!(prior);
    assert!(settled_job(&job));
    let mut unknown=job.clone();unknown["status"]=json!("submission_unknown");assert!(!settled_job(&unknown));
    let mut send=job.clone();send["result"]["raw"]["sentAt"]=json!(0);assert!(!settled_job(&send));
    let mut changed=job;changed["result"]["raw"]["readerMigration"]["bridgeOperation"]=json!("submit");assert!(!settled_job(&changed));
    }
}

#[test]
fn opencli_terminal_double_empty_settles_same_frozen_image_without_reserializing_node_wire_hash() {
    let job = fixture();
    assert_ne!(job["prepared"]["wire"]["requestHash"], crate::pipeline::digest(&job["prepared"]["request"].to_string()));
    assert!(settled_job(&job));
    let mut new_page = job.clone();
    for check in new_page["result"]["raw"]["opencliNotSubmitted"]["checks"].as_array_mut().unwrap() {
        check["url"] = json!("https://chatgpt.com/new");
    }
    assert!(settled_job(&new_page));
    let mut bridge_message = job.clone();
    bridge_message["result"]["error"] = json!("参考图片准备在发送提示词前中断，已核实没有提交生成；可继续制作，原记录保留。");
    assert!(settled_job(&bridge_message));
    // Extra frozen metadata is not a change to the original GenerationRequest.
    let mut metadata = job;
    metadata["snapshot"]["capabilityVersion"] = json!("synthetic-capability");
    assert!(settled_job(&metadata));
}

#[test]
fn opencli_terminal_request_account_runtime_settlement_or_media_drift_is_held() {
    for (pointer, value) in [
        ("/provider", json!("bh")), ("/kind", json!("video")), ("/status", json!("submission_unknown")),
        ("/providerTaskId", json!("other")), ("/snapshot/requestId", json!("other")), ("/snapshot/prompt", json!("changed")),
        ("/snapshot/ratio", json!("4:3")), ("/snapshot/options/synthetic", json!(false)),
        ("/prepared/provider", json!("other")), ("/prepared/request/requestId", json!("other")),
        ("/prepared/request/kind", json!("video")), ("/prepared/request/model", json!("other")),
        ("/prepared/wire/transport", json!("legacy")), ("/prepared/wire/requestHash", json!("d".repeat(64))),
        ("/prepared/wire/accountKey", json!("other")), ("/prepared/wire/profile", json!("other")),
        ("/snapshot/providerConfig/kind", json!("other")),
        ("/snapshot/providerConfig/browserTransport/transport", json!("legacy")),
        ("/snapshot/providerConfig/browserTransport/accountKey", json!("other")),
        ("/snapshot/providerConfig/browserTransport/runtimeSha256", json!("d".repeat(64))),
        ("/result/taskId", json!("other")), ("/result/status", json!("unknown")),
        ("/result/resultUrls", json!(["synthetic-image"])), ("/result/error", json!("")),
        ("/result/error", json!("task failed")), ("/result/raw/transport", json!("legacy")),
        ("/result/raw/requestHash", json!("d".repeat(64))), ("/result/raw/taskSettlement/id", json!("other")),
        ("/result/raw/taskSettlement/status", json!("cancelled")), ("/result/raw/taskSettlement/reason", json!("unknown")),
    ] {
        let mut job = fixture();
        *job.pointer_mut(pointer).unwrap() = value;
        assert!(!settled_job(&job), "{pointer}");
    }
}

#[test]
fn opencli_terminal_partial_or_nonempty_proof_is_never_enough() {
    let root = "/result/raw/opencliNotSubmitted";
    for (part, value) in [
        ("/version", json!("unknown")), ("/requestId", json!("other")), ("/requestHash", json!("d".repeat(64))),
        ("/runtimeSha256", json!("d".repeat(64))), ("/journalBeforeSha256", json!("")),
        ("/journalAfterSha256", json!("b".repeat(64))), ("/journalAfterSha256", json!("g".repeat(64))),
        ("/sendMarkerAbsent", json!(false)), ("/sendConfirmedAbsent", json!(false)), ("/conversationUrlAbsent", json!(false)),
        ("/page", json!("")), ("/page", json!(" ")), ("/checks", json!([])),
        ("/checks/0/page", json!("other")), ("/checks/1/accountKey", json!("other")),
        ("/checks/0/url", json!("https://chatgpt.com/c/11111111-1111-4111-8111-111111111111")),
        ("/checks/1/url", json!("https://chatgpt.com.evil.test/new")),
        ("/checks/0/userCount", json!(1)), ("/checks/1/assistantCount", json!(1)),
        ("/checks/0/composerEmpty", json!(false)), ("/checks/1/generating", json!(true)),
    ] {
        let mut job = fixture();
        *job.pointer_mut(&format!("{root}{part}")).unwrap() = value;
        assert!(!settled_job(&job), "{part}");
    }
    for key in ["version", "requestId", "requestHash", "runtimeSha256", "journalBeforeSha256", "journalAfterSha256",
        "sendMarkerAbsent", "sendConfirmedAbsent", "conversationUrlAbsent", "page", "checks"] {
        let mut job = fixture();
        job["result"]["raw"]["opencliNotSubmitted"].as_object_mut().unwrap().remove(key);
        assert!(!settled_job(&job), "missing {key}");
    }
    for key in ["page", "url", "accountKey", "userCount", "assistantCount", "composerEmpty", "generating"] {
        for index in [0, 1] {
            let mut job = fixture();
            job["result"]["raw"]["opencliNotSubmitted"]["checks"][index].as_object_mut().unwrap().remove(key);
            assert!(!settled_job(&job), "missing check[{index}] {key}");
        }
    }
    let mut extra = fixture();
    let check = extra["result"]["raw"]["opencliNotSubmitted"]["checks"][0].clone();
    extra["result"]["raw"]["opencliNotSubmitted"]["checks"].as_array_mut().unwrap().push(check);
    assert!(!settled_job(&extra));
}

#[test]
fn opencli_terminal_any_actual_send_field_even_zero_false_or_nested_is_held() {
    for (key, value) in [("sentAt", json!(0)), ("sentAt", Value::Null), ("sendConfirmed", json!(false)),
        ("sendConfirmed", json!(true)), ("conversationUrl", json!("")), ("conversationUrl", json!("https://chatgpt.com/"))] {
        for container in ["/result/raw", "/result/raw/opencliNotSubmitted", "/result/raw/opencliNotSubmitted/checks/0"] {
            let mut job = fixture();
            job.pointer_mut(container).unwrap().as_object_mut().unwrap().insert(key.into(), value.clone());
            assert!(!settled_job(&job), "{container}/{key}");
        }
        let mut nested = fixture();
        nested["result"]["raw"]["diagnostics"] = json!({"records":[{key:value}]});
        assert!(!settled_job(&nested), "nested {key}");
    }
}

#[test]
fn opencli_terminal_complete_query_reader_migration_can_settle_without_editing_frozen_config() {
    let job = migrated_fixture();
    let frozen = job["snapshot"].clone();
    assert!(settled_job(&job));
    assert_eq!(job["snapshot"], frozen);
    let mut second_source = job.clone();
    let previous_reader = json!("48315b72fa8f6ea412c1549bca00c8b3d60ed32d7ceedd9c3d9808dbd5923df3");
    second_source["snapshot"]["providerConfig"]["browserTransport"]["runtimeSha256"] = previous_reader.clone();
    second_source["result"]["raw"]["readerMigration"]["fromRuntimeSha256"] = previous_reader;
    second_source["result"]["raw"]["readerMigration"]["frozenConfigSha256"] = json!(crate::pipeline::digest(&second_source["snapshot"]["providerConfig"].to_string()));
    assert!(settled_job(&second_source));
    let mut reconcile = job;
    reconcile["result"]["raw"]["readerMigration"]["operation"] = json!("reconcile");
    assert!(settled_job(&reconcile));
}

#[test]
fn opencli_terminal_migration_must_match_exact_frozen_config_request_and_current_query_reader() {
    let root = "/result/raw/readerMigration";
    for (part, value) in [
        ("/schemaVersion", json!("unknown")), ("/operation", json!("submit")), ("/bridgeOperation", json!("submit")),
        ("/readOnly", json!(false)), ("/resubmitted", json!(true)), ("/frozenConfigUnchanged", json!(false)),
        ("/fromRuntimeSha256", json!("d".repeat(64))), ("/toRuntimeSha256", json!("d".repeat(64))),
        ("/fromRunnerSha256", json!("")), ("/toManifestSha256", json!("")),
        ("/frozenConfigSha256", json!("d".repeat(64))), ("/generationRequestSha256", json!("d".repeat(64))),
    ] {
        let mut job = migrated_fixture();
        *job.pointer_mut(&format!("{root}{part}")).unwrap() = value;
        assert!(!settled_job(&job), "migration {part}");
    }
    let mut missing = migrated_fixture();
    missing["result"]["raw"].as_object_mut().unwrap().remove("readerMigration");
    assert!(!settled_job(&missing));
    let mut arbitrary = migrated_fixture();
    let fake = json!("9".repeat(64));
    arbitrary["result"]["raw"]["readerMigration"]["toRuntimeSha256"] = fake.clone();
    arbitrary["result"]["raw"]["opencliNotSubmitted"]["runtimeSha256"] = fake;
    assert!(!settled_job(&arbitrary));
    // A self-consistent audit still cannot invent a newly allowed source pin.
    let mut unreviewed_source = migrated_fixture();
    let unknown = json!("9".repeat(64));
    unreviewed_source["snapshot"]["providerConfig"]["browserTransport"]["runtimeSha256"] = unknown.clone();
    unreviewed_source["result"]["raw"]["readerMigration"]["fromRuntimeSha256"] = unknown;
    unreviewed_source["result"]["raw"]["readerMigration"]["frozenConfigSha256"] = json!(crate::pipeline::digest(&unreviewed_source["snapshot"]["providerConfig"].to_string()));
    assert!(!settled_job(&unreviewed_source));
}

#[test]
fn opencli_terminal_never_falls_back_to_legacy_proof_and_legacy_contract_still_works() {
    assert!(settled_job(&super::tests::fixture()));
    assert!(settled_job(&super::tests::upload_fixture()));
    let mut job = super::tests::fixture();
    job["prepared"]["wire"]["transport"] = json!("opencli/v1");
    assert!(!settled_job(&job));
}
