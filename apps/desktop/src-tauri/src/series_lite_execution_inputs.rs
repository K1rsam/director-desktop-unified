use super::*;
use crate::series_lite_workspace::NativeWorkspaceScopeVerifier;

pub(super) struct Registry {
    pub raw: Vec<u8>,
    pub value: Json,
    pub snapshot: Json,
}
pub(super) struct BoundInputs {
    pub request: Json,
    pub packet: Json,
    pub bundle: Vec<u8>,
    pub registry: Registry,
    pub signature: Json,
    pub fingerprint: String,
}
pub(super) fn registry(job: &Job, path: &str) -> Result<Registry> {
    let dir = job.path(path)?;
    need(dir.is_dir(), "registry must be an existing JOB directory")?;
    let head = job.json(&job.relative(&dir.join("current.json"))?)?;
    let revision = head
        .get("revision")?
        .integer()
        .filter(|n| *n > 0)
        .ok_or_else(|| err("invalid registry revision"))?;
    need(
        s(&head, "version")? == "series-assets-head-v1",
        "invalid registry head",
    )?;
    let snapshot = job.relative(&dir.join(format!("v{revision:06}.json")))?;
    let raw = job.read(&snapshot)?;
    need(
        sha256(&raw) == s(&head, "sha256")?,
        "current registry snapshot changed",
    )?;
    let value = parse(&raw)?;
    need(
        s(&value, "version")? == "series-assets-v1"
            && value.get("id")? == head.get("id")?
            && number_equal(value.get("revision")?, head.get("revision")?),
        "registry identity mismatch",
    )?;
    Ok(Registry {
        raw,
        value,
        snapshot: job.record(&snapshot)?,
    })
}
pub(super) fn relevant(registry: &Json, bundle: &Json) -> Result<Json> {
    let ids: BTreeMap<_, _> = registry
        .get("assets")?
        .array()?
        .iter()
        .map(|a| Ok((s(a, "id")?, a)))
        .collect::<Result<_>>()?;
    let projection: BTreeMap<_, _> = bundle
        .get("assets")?
        .array()?
        .iter()
        .map(|a| Ok((s(a, "key")?, a.object()?.get("assetId"))))
        .collect::<Result<_>>()?;
    fn add<'a>(
        id: &'a str,
        ids: &BTreeMap<&'a str, &'a Json>,
        wanted: &mut BTreeSet<String>,
    ) -> Result<()> {
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            if !wanted.insert(id.to_string()) {
                continue;
            }
            let a = ids.get(id).ok_or_else(|| err("missing declared asset"))?;
            if let Some(deps) = a.get("design")?.object()?.get("dependencies") {
                for dep in deps.array()?.iter().rev() {
                    pending.push(s(dep, "assetId")?);
                }
            }
        }
        Ok(())
    }
    let mut wanted = BTreeSet::new();
    let episodes = bundle.get("episodes")?.array()?;
    for ep in episodes {
        for unit in ep.get("draft")?.get("units")?.array()? {
            for key in unit.get("bindings")?.array()? {
                let id = projection
                    .get(key.string()?)
                    .and_then(|v| *v)
                    .ok_or_else(|| err("missing projected asset"))?
                    .string()?;
                add(id, &ids, &mut wanted)?;
            }
        }
    }
    let requests = registry
        .get("requests")?
        .array()?
        .iter()
        .filter_map(|r| {
            let result = (|| {
                for u in r.get("uses")?.array()? {
                    let used_episode = u.get("episode")?;
                    for episode in episodes {
                        if number_equal(used_episode, episode.get("number")?) {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            })();
            match result {
                Ok(true) => Some(Ok(r.clone())),
                Ok(false) => None,
                Err(e) => Some(Err(e)),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(object([
        ("id", registry.get("id")?.clone()),
        ("sourceHash", registry.get("sourceHash")?.clone()),
        (
            "importProfile",
            registry
                .object()?
                .get("importProfile")
                .cloned()
                .unwrap_or(Json::Null),
        ),
        (
            "productionScope",
            registry
                .object()?
                .get("productionScope")
                .cloned()
                .unwrap_or(Json::Null),
        ),
        (
            "assets",
            Json::Array(
                wanted
                    .iter()
                    .map(|id| (*ids[id.as_str()]).clone())
                    .collect(),
            ),
        ),
        ("requests", Json::Array(requests)),
    ]))
}
pub(super) fn packet_value(packet: &Json, binding: &Json) -> Result<Json> {
    let blob = packet.get("blobs")?.get(s(binding, "blob")?)?;
    if s(blob, "format")? == "text" {
        parse(blob.get("value")?.string()?.as_bytes())
    } else {
        Ok(blob.get("value")?.clone())
    }
}
pub(super) fn inputs(
    job: &Job,
    manifest: &str,
    pack: Option<&VerifiedPack>,
    historical: Option<&Json>,
) -> Result<BoundInputs> {
    let request = job.json(manifest)?;
    fields(
        &request,
        &[
            "version",
            "episode",
            "packet",
            "bundle",
            "sourceLock",
            "registry",
        ],
        &["seasonScope"],
    )?;
    need(
        s(&request, "version")? == REQUEST,
        "unknown execution version",
    )?;
    let packet_path = s(&request, "packet")?;
    let compiled = if let Some(old) = historical {
        let normalized_packet_path = job.relative(&job.path(packet_path)?)?;
        let matching = old
            .get("files")?
            .array()?
            .iter()
            .find(|f| s(f, "path").ok() == Some(normalized_packet_path.as_str()))
            .ok_or_else(|| err("historical packet pin missing"))?;
        crate::series_lite_job_packet::load_frozen(
            &job.root,
            packet_path,
            s(matching, "sha256")?,
            &NativeWorkspaceScopeVerifier,
        )?
    } else {
        crate::series_lite_job_packet::verify(
            &job.root,
            packet_path,
            pack.ok_or_else(|| err("verified rules required"))?,
            &NativeWorkspaceScopeVerifier,
        )?
    };
    let packet = parse(&compiled.bytes)?;
    need(
        number_equal(packet.get("episode")?, request.get("episode")?)
            && matches!(s(&packet, "role")?, "finisher" | "reviewer"),
        "execution needs matching complete finisher/reviewer packet",
    )?;
    let bundle = job.read(s(&request, "bundle")?)?;
    let b = parse(&bundle)?;
    if pack.is_some_and(VerifiedPack::is_current) && historical.is_none() {
        need(
            b.object()?
                .get("generationBoundaryVersion")
                .is_some_and(|v| v.string().ok() == Some("video-prompt-v1")),
            "new lite execution requires video-prompt-v1; historical source unchanged",
        )?;
    }
    need(
        s(&b, "exportProfile")? == "lingmo-v7" && b.get("episodes")?.array()?.len() == 1,
        "execution requires one complete Lingmo v7 episode",
    )?;
    let ep = &b.get("episodes")?.array()?[0];
    need(
        number_equal(ep.get("number")?, request.get("episode")?),
        "bundle episode differs",
    )?;
    for kind in ["source", "plan", "selection"] {
        let values = packet
            .get("bindings")?
            .array()?
            .iter()
            .filter(|v| s(v, "kind").ok() == Some(kind))
            .map(|v| packet_value(&packet, v))
            .collect::<Result<Vec<_>>>()?;
        need(
            values.len() == 1 && canonical(&values[0]) == canonical(ep.get(kind)?),
            "packet and bundle complete values differ; rebuild after amendment",
        )?;
    }
    let baselines = packet
        .get("bindings")?
        .array()?
        .iter()
        .filter(|v| s(v, "kind").ok() == Some("visual-baseline"))
        .map(|v| packet_value(&packet, v))
        .collect::<Result<Vec<_>>>()?;
    if baselines.is_empty() {
        need(
            historical.is_some(),
            "new execution requires complete visual baseline",
        )?;
    } else {
        need(
            baselines.len() == 1
                && canonical(&baselines[0])
                    == canonical(ep.get("settings")?.get("visualBaseline")?)
                && ep.get("settings")?.get("visualStyle")? == baselines[0].get("visualStyle")?,
            "visual baseline or style differs",
        )?;
    }
    let runtime = match historical {
        Some(old) => old.get("runtime")?.clone(),
        None => runtime::current(pack.ok_or_else(|| err("verified pack required"))?)?,
    };
    for (key, hash) in runtime.object()? {
        need(
            !key.is_empty() && hex(hash.string()?),
            "invalid frozen runtime pin",
        )?;
    }
    let mut files = vec![job.record(manifest)?];
    for key in ["packet", "bundle", "sourceLock", "seasonScope"] {
        if let Some(path) = request.object()?.get(key) {
            files.push(job.record(path.string()?)?);
        }
    }
    let registry = registry(job, s(&request, "registry")?)?;
    if historical.is_none() {
        let current = value(&registry.value)?;
        if pack.is_some_and(VerifiedPack::is_current) {
            crate::series_lite_registry::validate_registry_current(&current, false)?;
        } else {
            crate::series_lite_registry::validate_registry(&current, false)?;
        }
    }
    let asset_fingerprint = sha256(&canonical(&relevant(&registry.value, &b)?));
    let signature = object([
        ("files", Json::Array(files)),
        ("inputFingerprint", packet.get("inputFingerprint")?.clone()),
        ("runtime", runtime),
        ("assetFingerprint", text(asset_fingerprint)),
    ]);
    let fingerprint = sha256(&canonical(&signature));
    Ok(BoundInputs {
        request,
        packet,
        bundle,
        registry,
        signature,
        fingerprint,
    })
}
