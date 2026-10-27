use super::*;
use crate::series_lite_job_packet::{assemble, RejectUnverifiedWorkspace};
use std::fs;

fn cases() -> Vec<Value> {
    let corpus: Value =
        serde_json::from_str(include_str!("series_lite_job_packet_golden.json")).unwrap();
    corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["ok"] == true)
        .cloned()
        .collect()
}
fn compile_case(case: &Value) -> CompiledPacket {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in case["files"].as_object().unwrap() {
        let target = dir.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, text.as_str().unwrap().as_bytes()).unwrap();
    }
    assemble(
        dir.path(),
        "request.json",
        case["maxBytes"].as_u64().unwrap() as usize,
        &VerifiedPack::embedded().unwrap(),
        &RejectUnverifiedWorkspace,
    )
    .unwrap()
}
fn example(name: &str) -> CompiledPacket {
    compile_case(&cases().into_iter().find(|v| v["name"] == name).unwrap())
}
fn context(packet: &CompiledPacket) -> RoleContext {
    let value = parse(&packet.bytes).unwrap();
    let role = Role::parse(value.get("role").unwrap().string().unwrap()).unwrap();
    RoleContext {
        episode: String::from_utf8(canonical(value.get("episode").unwrap())).unwrap(),
        role,
        iteration_id: format!("frozen-{}-iteration-1", role.as_str()),
        context_id: format!("frozen-{}-context", role.as_str()),
        prior_contributors: match role {
            Role::Author => vec![],
            Role::Finisher => vec![Contributor {
                role: Role::Author,
                context_id: "frozen-author-context".into(),
            }],
            Role::Reviewer => vec![
                Contributor {
                    role: Role::Author,
                    context_id: "frozen-author-context".into(),
                },
                Contributor {
                    role: Role::Finisher,
                    context_id: "frozen-finisher-context".into(),
                },
            ],
        },
    }
}
fn freeze(packet: &CompiledPacket) -> (FrozenCarrier, RoleContext) {
    let context = context(packet);
    (
        freeze_verified(packet, &context, &VerifiedPack::embedded().unwrap()).unwrap(),
        context,
    )
}
// Test-only self-signing simulates a modified artifact: the immutable outer
// attempt pin must still reject it even when its own checksums are consistent.
fn reseal(packet: &mut BTreeMap<String, Json>) -> Vec<u8> {
    let bound = Json::Object(
        INPUT_KEYS
            .iter()
            .map(|k| (k.to_string(), packet[*k].clone()))
            .collect(),
    );
    packet.insert(
        "inputFingerprint".into(),
        Json::String(sha256(&canonical(&bound))),
    );
    packet.remove("packetSha256");
    let hash = sha256(&canonical(&Json::Object(packet.clone())));
    packet.insert("packetSha256".into(), Json::String(hash));
    canonical(&Json::Object(packet.clone()))
}

#[test]
fn all_13_success_oracles_survive_string_transport_and_save_reopen_byte_exact() {
    let corpus = cases();
    assert_eq!(corpus.len(), 13);
    let mut roles = BTreeSet::new();
    for case in corpus {
        let packet = compile_case(&case);
        assert_eq!(packet.bytes, case["encoded"].as_str().unwrap().as_bytes());
        let (carrier, context) = freeze(&packet);
        roles.insert(context.role.as_str());
        // Emulate TextRequest.input's JSON serialization without converting
        // packetUtf8 itself into a numeric Value tree.
        let wire = serde_json::to_vec(&json!({"input": carrier.input})).unwrap();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("request.json"), wire).unwrap();
        fs::write(
            dir.path().join("attempt-pin.json"),
            serde_json::to_vec(&carrier.identity).unwrap(),
        )
        .unwrap();
        let request: Value =
            serde_json::from_slice(&fs::read(dir.path().join("request.json")).unwrap()).unwrap();
        let pin: TransportIdentity =
            serde_json::from_slice(&fs::read(dir.path().join("attempt-pin.json")).unwrap())
                .unwrap();
        let result = restore(&request["input"], &pin, &context).unwrap();
        assert_eq!(result.bytes, packet.bytes, "{}", case["name"]);
        assert_eq!(result.identity, carrier.identity);
        assert_eq!(result.identity.packet_file_sha256, packet.sha256);
    }
    assert_eq!(
        roles,
        ["author", "finisher", "reviewer"].into_iter().collect()
    );
}

#[test]
fn payload_object_and_float_roundtrip_are_rejected_even_if_outer_sha_is_recomputed() {
    // Some valid exact decimals exceed f64 altogether; their string transport
    // still succeeds in the complete oracle test above.
    assert!(serde_json::from_slice::<Value>(
        &example("decimals_pointer_and_duplicate_projection").bytes
    )
    .is_err());
    let packet = example("large_integer_episode");
    let (carrier, context) = freeze(&packet);
    let parsed: Value = serde_json::from_slice(&packet.bytes).unwrap();
    let mut bad = carrier.input.clone();
    bad["packetUtf8"] = parsed.clone();
    assert!(restore(&bad, &carrier.identity, &context).is_err());
    let lossy = serde_json::to_string(&parsed).unwrap();
    assert_ne!(lossy.as_bytes(), packet.bytes);
    bad["packetUtf8"] = json!(lossy);
    assert!(restore(&bad, &carrier.identity, &context).is_err());
    let mut forged_pin = carrier.identity.clone();
    forged_pin.packet_file_sha256 = sha256(lossy.as_bytes());
    forged_pin.packet_bytes = lossy.len();
    bad["identity"] = serde_json::to_value(&forged_pin).unwrap();
    assert!(
        restore(&bad, &forged_pin, &context).is_err(),
        "internal self-hash must catch rounded content"
    );
}

#[test]
fn separately_pinned_identity_rejects_every_identity_field_change() {
    let packet = example("author_complete");
    let (carrier, context) = freeze(&packet);
    let replacements = [
        ("profile", json!("future-profile")),
        ("packetFileSha256", json!("a".repeat(64))),
        ("packetBytes", json!(1)),
        ("inputFingerprint", json!("a".repeat(64))),
        ("episode", json!("999")),
        ("role", json!("reviewer")),
        ("ruleIdentitySha256", json!("a".repeat(64))),
        ("roleContextSha256", json!("a".repeat(64))),
    ];
    for (field, replacement) in replacements {
        let mut bad = carrier.input.clone();
        bad["identity"][field] = replacement;
        assert!(
            restore(&bad, &carrier.identity, &context).is_err(),
            "{field}"
        );
        let changed_expected: TransportIdentity =
            serde_json::from_value(bad["identity"].clone()).unwrap();
        assert!(
            restore(&bad, &changed_expected, &context).is_err(),
            "recomputed identity must reject {field} even when both outer metadata copies changed"
        );
    }
    let mut bad = carrier.input.clone();
    bad["packetUtf8"] = json!(format!("{} ", bad["packetUtf8"].as_str().unwrap()));
    assert!(restore(&bad, &carrier.identity, &context).is_err());
    let mut bad = carrier.input.clone();
    bad["extra"] = json!(true);
    assert!(restore(&bad, &carrier.identity, &context).is_err());
    let mut bad = carrier.input.clone();
    bad["roleContext"]["contextId"] = json!("another-context");
    assert!(restore(&bad, &carrier.identity, &context).is_err());
    let mut other_iteration = context.clone();
    other_iteration.iteration_id = "another-iteration".into();
    assert!(restore(&carrier.input, &carrier.identity, &other_iteration).is_err());
    let mut bad = carrier.input.clone();
    bad["roleContext"]["iterationId"] = json!("another-iteration");
    assert!(restore(&bad, &carrier.identity, &context).is_err());
    let mut bad = carrier.input.clone();
    bad["transportRule"] = json!("summarize rules");
    assert!(restore(&bad, &carrier.identity, &context).is_err());
}

#[test]
fn self_consistent_replacement_packet_cannot_replace_frozen_attempt() {
    let packet = example("author_complete");
    let (carrier, context) = freeze(&packet);
    let mut changed = parse(&packet.bytes).unwrap().object().unwrap().clone();
    changed.insert("episode".into(), parse(b"99").unwrap());
    let raw = reseal(&mut changed);
    assert!(
        checked(&raw).is_ok(),
        "replacement should be internally consistent"
    );
    let c = checked(&raw).unwrap();
    let mut other_context = context.clone();
    other_context.episode = "99".into();
    let other_pin = identity(&raw, &c, &other_context).unwrap();
    let mut bad = carrier.input.clone();
    bad["packetUtf8"] = json!(String::from_utf8(raw).unwrap());
    bad["identity"] = serde_json::to_value(other_pin).unwrap();
    bad["roleContext"] = serde_json::to_value(other_context).unwrap();
    assert!(restore(&bad, &carrier.identity, &context).is_err());
}

#[test]
fn roles_have_distinct_contexts_and_non_authors_require_actual_contributor_ledger() {
    let pack = VerifiedPack::embedded().unwrap();
    for (name, stage) in [
        ("author_complete", "series-lite:author"),
        ("finisher_complete", "series-lite:finisher"),
        ("reviewer_complete", "series-lite:reviewer"),
    ] {
        let packet = example(name);
        let (carrier, ctx) = freeze(&packet);
        assert_eq!(ctx.role.stage(), stage);
        assert!(restore(&carrier.input, &carrier.identity, &ctx).is_ok());
        assert!(
            carrier.input["roleContext"].get("accountId").is_none(),
            "SDK account switching is not a creative context identity"
        );
        if ctx.role == Role::Author {
            continue;
        }
        let mut bad = ctx.clone();
        bad.prior_contributors.clear();
        assert!(freeze_verified(&packet, &bad, &pack).is_err());
        let mut bad = ctx.clone();
        bad.prior_contributors.retain(|c| c.role != Role::Author);
        assert!(
            freeze_verified(&packet, &bad, &pack).is_err(),
            "finisher-only ledger cannot stand in for author history"
        );
        for c in &ctx.prior_contributors {
            let mut bad = ctx.clone();
            bad.context_id = c.context_id.clone();
            assert!(freeze_verified(&packet, &bad, &pack).is_err());
        }
        let mut bad = ctx.clone();
        bad.prior_contributors
            .push(bad.prior_contributors[0].clone());
        assert!(freeze_verified(&packet, &bad, &pack).is_err());
        let mut bad = ctx.clone();
        bad.prior_contributors[0].role = Role::Reviewer;
        assert!(freeze_verified(&packet, &bad, &pack).is_err());
    }
}

#[test]
fn non_author_dependencies_and_complete_rule_bodies_cannot_be_removed() {
    for name in ["finisher_complete", "reviewer_complete"] {
        let packet = example(name);
        for missing in ["draft", "plan", "selection"] {
            let mut p = parse(&packet.bytes).unwrap().object().unwrap().clone();
            let bindings = p["bindings"]
                .array()
                .unwrap()
                .iter()
                .filter(|b| b.get("kind").unwrap().string().unwrap() != missing)
                .cloned()
                .collect();
            p.insert("bindings".into(), Json::Array(bindings));
            assert!(checked(&reseal(&mut p)).is_err(), "{name}: {missing}");
        }
    }
    let mut packet = example("author_complete");
    let ctx = context(&packet);
    let mut p = parse(&packet.bytes).unwrap().object().unwrap().clone();
    // Replace a rule with a self-consistent abbreviated body and references.
    let mut rules = p["rules"].array().unwrap().to_vec();
    let mut rule = rules[0].object().unwrap().clone();
    let old_key = rule["blob"].string().unwrap().to_string();
    let body = Json::Object(
        [
            ("format".into(), Json::String("text".into())),
            ("value".into(), Json::String("abbreviated".into())),
        ]
        .into_iter()
        .collect(),
    );
    let key = sha256(&canonical(&body));
    let mut blobs = p["blobs"].object().unwrap().clone();
    blobs.remove(&old_key);
    blobs.insert(key.clone(), body);
    rule.insert("blob".into(), Json::String(key));
    rule.insert("sha256".into(), Json::String(sha256(b"abbreviated")));
    rule.insert("originBytes".into(), parse(b"11").unwrap());
    rules[0] = Json::Object(rule);
    p.insert("rules".into(), Json::Array(rules));
    p.insert("blobs".into(), Json::Object(blobs));
    packet.bytes = reseal(&mut p);
    packet.sha256 = sha256(&packet.bytes);
    packet.input_fingerprint = p["inputFingerprint"].string().unwrap().into();
    assert!(checked(&packet.bytes).is_ok());
    assert!(
        freeze_verified(&packet, &ctx, &VerifiedPack::embedded().unwrap()).is_err(),
        "current full rule comparison must reject self-signed summary"
    );
}

#[test]
fn compiled_metadata_and_context_slot_are_rechecked_without_exposing_source() {
    let pack = VerifiedPack::embedded().unwrap();
    for field in 0..5 {
        let mut packet = example("author_complete");
        let ctx = context(&packet);
        match field {
            0 => packet.sha256 = "a".repeat(64),
            1 => packet.input_fingerprint = "a".repeat(64),
            2 => packet.inputs += 1,
            3 => packet.rules += 1,
            _ => packet.unique_bodies += 1,
        }
        assert!(freeze_verified(&packet, &ctx, &pack).is_err());
    }
    let packet = example("author_complete");
    for bad_ctx in [
        RoleContext {
            role: Role::Reviewer,
            ..context(&packet)
        },
        RoleContext {
            episode: "2".into(),
            ..context(&packet)
        },
        RoleContext {
            context_id: "  ".into(),
            ..context(&packet)
        },
    ] {
        assert!(freeze_verified(&packet, &bad_ctx, &pack).is_err());
    }
    let secret = b"PRIVATE_SOURCE_SENTINEL_not_for_errors";
    let bad = CompiledPacket {
        bytes: secret.to_vec(),
        sha256: sha256(secret),
        input_fingerprint: "a".repeat(64),
        inputs: 0,
        rules: 0,
        unique_bodies: 0,
    };
    let err = freeze_verified(&bad, &context(&packet), &pack).unwrap_err();
    assert!(!err.contains("PRIVATE_SOURCE_SENTINEL"));
}
