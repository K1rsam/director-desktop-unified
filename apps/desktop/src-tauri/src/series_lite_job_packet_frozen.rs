use super::*;

/// Historical validation does not consult today's Skill. The adoption receipt's
/// packet file hash is mandatory: a self-checksum alone cannot identify adopted
/// bytes when a writer replaces both the body and checksums. This returns only
/// verified supplied evidence, never an independent-review or adoption verdict.
pub fn load_frozen(
    root: &Path,
    packet_path: &str,
    adopted_file_sha256: &str,
    scope_gate: &dyn WorkspaceScopeVerifier,
) -> Result<CompiledPacket> {
    let root = root_path(root)?;
    digest(&text(adopted_file_sha256))?;
    let raw = read_file(&root, packet_path)?;
    need(
        sha256(&raw) == adopted_file_sha256,
        "adopted packet file hash mismatch",
    )?;
    let packet = parse(&raw)?;
    let mut keys = INPUT_KEYS.to_vec();
    keys.extend([
        "version",
        "maxBytes",
        "blobs",
        "inputFingerprint",
        "task",
        "evidenceBoundary",
        "packetSha256",
    ]);
    fields(&packet, &keys, &[])?;
    need(
        packet.get("version")?.string()? == PACKET_VERSION,
        "unsupported packet version",
    )?;
    digest(packet.get("packetSha256")?)?;
    digest(packet.get("inputFingerprint")?)?;
    let mut body = packet.object()?.clone();
    body.remove("packetSha256");
    need(
        packet.get("packetSha256")?.string()? == sha256(&canonical(&Json::Object(body))),
        "frozen self-checksum mismatch",
    )?;
    need(raw == canonical(&packet), "noncanonical frozen bytes")?;
    let budget = packet
        .get("maxBytes")?
        .integer()
        .and_then(|v| usize::try_from(v).ok());
    need(
        budget.is_some_and(|v| v > 0 && raw.len() <= v),
        "invalid byte budget",
    )?;
    let episode = packet.get("episode")?;
    let role = packet.get("role")?.string()?;
    need(
        episode.is_integer() && ["author", "finisher", "reviewer"].contains(&role),
        "invalid episode or role",
    )?;
    for name in ["skill", "compiler"] {
        let record = packet.get(name)?;
        fields(record, &["version", "sha256"], &[])?;
        nonempty(record.get("version")?)?;
        digest(record.get("sha256")?)?;
    }
    fields(packet.get("task")?, &["role", "contract"], &[])?;
    for value in packet.get("task")?.object()?.values() {
        nonempty(value)?;
    }
    nonempty(packet.get("evidenceBoundary")?)?;
    let origin = packet.get("requestOrigin")?;
    fields(origin, &["path", "sha256"], &[])?;
    digest(origin.get("sha256")?)?;
    let manifest_raw = read_file(&root, origin.get("path")?.string()?)?;
    need(
        sha256(&manifest_raw) == origin.get("sha256")?.string()?,
        "frozen manifest changed",
    )?;
    let request = parse(&manifest_raw)?;
    validate_request(
        &request,
        packet.get("compiler")?.get("version")?.string()? != LEGACY_COMPILER_VERSION,
        packet.get("compiler")?.get("version")?.string()? == CURRENT_COMPILER_VERSION,
    )?;
    need(
        request.get("episode")? == episode && request.get("role")?.string()? == role,
        "manifest episode or role mismatch",
    )?;

    let mut actual_blobs = BTreeMap::new();
    let bindings = request
        .get("inputs")?
        .array()?
        .iter()
        .map(|item| bind(&root, item, &mut actual_blobs, None))
        .collect::<Result<Vec<_>>>()?;
    let review = if let Some(item) = request.object()?.get("review") {
        bind(&root, item, &mut actual_blobs, Some("review"))?
    } else {
        Json::Null
    };
    for record in packet
        .get("bindings")?
        .array()?
        .iter()
        .chain((packet.get("review")? != &Json::Null).then_some(packet.get("review")?))
    {
        fields(
            record,
            &["kind", "path", "sha256", "originBytes", "blob"],
            &["pointer"],
        )?;
        need(
            record.get("originBytes")?.integer().is_some_and(|n| n >= 0),
            "invalid original input bytes",
        )?;
        digest(record.get("sha256")?)?;
        digest(record.get("blob")?)?;
    }
    need(
        packet.get("bindings")? == &Json::Array(bindings),
        "JOB input binding changed",
    )?;
    need(packet.get("review")? == &review, "review record changed")?;
    if review != Json::Null {
        review_nonempty(actual_blobs[review.get("blob")?.string()?].get("value")?)?;
    }
    validate_scope(packet.get("scope")?, episode)?;
    need(
        packet.get("scope")? == &scope(&root, episode, scope_gate)?,
        "workspace scope changed",
    )?;

    let blobs = packet.get("blobs")?.object()?;
    need(!blobs.is_empty(), "invalid blob pool")?;
    for (key, body) in blobs {
        digest(&text(key))?;
        fields(body, &["format", "value"], &[])?;
        let format = body.get("format")?.string()?;
        need(["text", "json"].contains(&format), "invalid blob format")?;
        if format == "text" {
            body.get("value")?.string()?;
        }
        need(
            sha256(&canonical(body)) == *key,
            "blob content hash mismatch",
        )?;
    }
    for (key, body) in &actual_blobs {
        need(
            blobs
                .get(key)
                .is_some_and(|saved| canonical(saved) == canonical(body)),
            "JOB input blob mismatch",
        )?;
    }
    let rules = packet.get("rules")?.array()?;
    need(!rules.is_empty(), "empty rules")?;
    let mut used: BTreeSet<String> = actual_blobs.keys().cloned().collect();
    let mut paths = BTreeSet::new();
    for rule in rules {
        fields(
            rule,
            &[
                "kind",
                "path",
                "sha256",
                "originBytes",
                "adjustments",
                "blob",
            ],
            &[],
        )?;
        need(rule.get("kind")?.string()? == "rules", "invalid rule kind")?;
        let path = rule.get("path")?.string()?;
        relative_path(path)?; // lexical provenance only; no current Skill lookup
        need(paths.insert(path), "duplicate rule path")?;
        digest(rule.get("sha256")?)?;
        digest(rule.get("blob")?)?;
        need(
            rule.get("originBytes")?.integer().is_some_and(|n| n >= 0),
            "invalid original rule bytes",
        )?;
        for adjustment in rule.get("adjustments")?.array()? {
            nonempty(adjustment)?;
        }
        let key = rule.get("blob")?.string()?;
        let body = blobs
            .get(key)
            .ok_or("JOB_PACKET: missing effective rule text")?;
        need(
            body.get("format")?.string()? == "text",
            "missing effective rule text",
        )?;
        used.insert(key.to_owned());
    }
    need(
        used == blobs.keys().cloned().collect(),
        "unreferenced or missing blob",
    )?;
    let inputs = Json::Object(
        INPUT_KEYS
            .iter()
            .map(|k| Ok(((*k).to_owned(), packet.get(k)?.clone())))
            .collect::<Result<_>>()?,
    );
    need(
        packet.get("inputFingerprint")?.string()? == sha256(&canonical(&inputs)),
        "input fingerprint mismatch",
    )?;
    result(&packet, raw)
}
