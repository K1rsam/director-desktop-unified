use super::*;
const TYPES: [&str; 3] = ["SC", "CH", "PR"];
const TOKEN_LABELS: [&str; 3] = ["场景", "本单元道具", "本单元出场人物"];
fn field(label: &str, value: impl Into<String>) -> Value {
    json!([label, value.into()])
}
fn asset_fields(a: &Value) -> Result<Value> {
    let name = string(&a["importName"], "ASSET_IMPORT_NAME")?;
    let kind = string(&a["type"], "ASSET_IMPORT_NAME")?;
    let prefix = match kind {
        "SC" => "场景",
        "CH" => "角色",
        "PR" => "道具",
        _ => return Err("ASSET_IMPORT_NAME: type".into()),
    };
    Ok(
        json!({"name":name,"type":kind,"token":format!("{name}{kind}"),"filename":format!("{prefix}-{name}.png"),"placeholderOrdinal":a["placeholderOrdinal"]}),
    )
}
fn token(a: &Value) -> Result<String> {
    Ok(format!(
        "{}{}",
        string(&a["importName"], "ASSET_IMPORT_NAME")?,
        string(&a["type"], "ASSET_IMPORT_NAME")?
    ))
}
fn find_key<'a>(
    u: &Value,
    assets: &BTreeMap<&'a str, &Value>,
    name: &str,
    kind: &str,
) -> Result<Option<&'a str>> {
    Ok(array(&u["bindings"], "UNIT_BINDINGS")?
        .iter()
        .filter_map(|k| k.as_str())
        .find_map(|k| {
            assets
                .get_key_value(k)
                .filter(|(_, a)| a["name"] == name && a["type"] == kind)
                .map(|(k, _)| *k)
        }))
}
fn append(
    name: &str,
    key: Option<&str>,
    images: &HashSet<&str>,
    inserted: &mut HashSet<String>,
    assets: &BTreeMap<&str, &Value>,
) -> Result<String> {
    if let Some(key) = key {
        if images.contains(key) && inserted.insert(key.into()) {
            return Ok(format!("{name}@[{}]", token(assets[key])?));
        }
    }
    Ok(name.into())
}
fn header(
    e: &Value,
    scene: &Value,
    u: &Value,
    assets: &BTreeMap<&str, &Value>,
) -> Result<Vec<Value>> {
    let image_list = array(&u["imageBindings"], "IMAGE_BINDINGS")?;
    let images: HashSet<_> = image_list.iter().filter_map(Value::as_str).collect();
    let mut inserted = HashSet::new();
    let mut characters = vec![];
    for name in array(&u["participants"], "UNIT_BINDINGS")? {
        let n = string(name, "UNIT_BINDINGS")?;
        characters.push(append(
            n,
            find_key(u, assets, n, "CH")?,
            &images,
            &mut inserted,
            assets,
        )?);
    }
    let empty = empty_array();
    for subject in optional_array(u, "mediaSubjects", &empty)? {
        let domain = match subject["medium"].as_str() {
            Some("screen") => "屏幕中",
            Some("photo") => "照片或画像中",
            Some("flashback") => "历史插入中",
            _ => return Err("MEDIA_SUBJECT: unknown medium".into()),
        };
        let n = string(&subject["name"], "MEDIA_SUBJECT")?;
        characters.push(format!(
            "{domain}：{}",
            append(
                n,
                find_key(u, assets, n, "CH")?,
                &images,
                &mut inserted,
                assets
            )?
        ));
    }
    for subject in optional_array(u, "visualSubjectBindings", &empty)? {
        if let Some(key) = subject["assetKey"].as_str() {
            if images.contains(key) {
                let domain = if subject["domain"] == "subjective" {
                    "主观表现"
                } else {
                    "未来图示"
                };
                characters.push(format!(
                    "{domain}：{}",
                    append(
                        string(&subject["name"], "VISUAL_BINDINGS")?,
                        Some(key),
                        &images,
                        &mut inserted,
                        assets
                    )?
                ));
            }
        }
    }
    let opening = u
        .get("segments")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .unwrap_or(u);
    let mut props = vec![];
    for name in array(&u["props"], "UNIT_BINDINGS")? {
        let n = string(name, "UNIT_BINDINGS")?;
        props.push(append(
            n,
            find_key(u, assets, n, "PR")?,
            &images,
            &mut inserted,
            assets,
        )?);
    }
    let scene_name = string(&u["scene"], "UNIT_BINDINGS")?;
    let mut values = vec![
        field(
            "通用提示词",
            compose_general_prompt(&e["settings"], &u["unitDirection"])?,
        ),
        field(
            "场景",
            format!(
                "{} {}",
                append(
                    scene_name,
                    find_key(u, assets, scene_name, "SC")?,
                    &images,
                    &mut inserted,
                    assets
                )?,
                string(
                    u.get("dayInterior")
                        .filter(|v| !v.is_null())
                        .unwrap_or(&scene["dayInterior"]),
                    "DAY_INTERIOR"
                )?
                .replacen(' ', " / ", 1)
            ),
        ),
        field(
            "本单元道具",
            if props.is_empty() {
                "无".into()
            } else {
                props.join("、")
            },
        ),
        field(
            "本单元出场人物",
            if characters.is_empty() {
                "无".into()
            } else {
                characters.join("、")
            },
        ),
        field("本单元光影", string(&opening["lighting"], "UNIT_FIELDS")?),
        field(
            "人物状态与站位前提",
            string(&opening["initialState"], "UNIT_FIELDS")?,
        ),
    ];
    if !std::ptr::eq(opening, u) {
        if u["lighting"] != opening["lighting"] {
            values.push(field("单元光影", string(&u["lighting"], "UNIT_FIELDS")?));
        }
        if u["initialState"] != opening["initialState"] {
            values.push(field(
                "单元起始状态",
                string(&u["initialState"], "UNIT_FIELDS")?,
            ));
        }
    }
    need(
        inserted.len() == images.len(),
        "IMAGE_BINDINGS",
        "image reference has no visible corresponding asset label",
    )?;
    let all = values
        .iter()
        .filter(|v| TOKEN_LABELS.contains(&v[0].as_str().unwrap()))
        .map(|v| v[1].as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let actual = parse_tokens(&all)?;
    let expected = image_list
        .iter()
        .map(|key| token(assets[string(key, "IMAGE_BINDINGS")?]))
        .collect::<Result<HashSet<_>>>()?;
    need(
        actual.len() == expected.len()
            && actual.iter().collect::<HashSet<_>>().len() == actual.len()
            && actual.iter().all(|v| expected.contains(v)),
        "LINGMO_RESERVED_TOKEN",
        "unexpected/duplicate token in original asset name",
    )?;
    Ok(values)
}
fn parse_tokens(s: &str) -> Result<Vec<String>> {
    let mut rest = s;
    let mut out = vec![];
    while let Some(start) = rest.find("@[") {
        rest = &rest[start + 2..];
        let end = rest
            .find(']')
            .ok_or("LINGMO_RESERVED_TOKEN: unfinished token")?;
        need(end > 0, "LINGMO_RESERVED_TOKEN", "empty token")?;
        out.push(rest[..end].into());
        rest = &rest[end + 1..];
    }
    Ok(out)
}
fn without_tokens(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("@[") {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start + 2..].find(']') {
            rest = &rest[start + 2 + end + 1..];
        } else {
            out.push_str(&rest[start..]);
            return out;
        }
    }
    out.push_str(rest);
    out
}
pub(super) fn project(bundle: &Value, sources: &[Value]) -> Result<Value> {
    let assets: BTreeMap<_, _> = array(&bundle["assets"], "ASSETS")?
        .iter()
        .map(|a| (a["key"].as_str().unwrap(), a))
        .collect();
    let mut episodes = vec![];
    for (ei, e) in bundle["episodes"].as_array().unwrap().iter().enumerate() {
        let quotes: BTreeMap<_, _> = array(&sources[ei]["dialogue"], "SOURCE_BINDING")?
            .iter()
            .map(|q| (q["id"].as_str().unwrap(), q))
            .collect();
        let mut progress: BTreeMap<&str, usize> = BTreeMap::new();
        let mut scenes = vec![];
        for scene in e["scenes"].as_array().unwrap() {
            let id = string(&scene["id"], "SCENE_SOURCE")?;
            need(
                !id.contains("@["),
                "LINGMO_RESERVED_TOKEN",
                "scene ID resembles asset token",
            )?;
            need(
                id.trim() != "**【标准化剧本】**",
                "LINGMO_RESERVED_SCENE",
                "scene ID conflicts with standard marker",
            )?;
            let originals: Vec<_> = e["draft"]["units"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|u| u["sceneId"] == scene["id"])
                .collect();
            let mut image_keys = vec![];
            let mut image_seen = HashSet::new();
            for u in &originals {
                for key in u["imageBindings"].as_array().unwrap() {
                    let k = key.as_str().unwrap();
                    if image_seen.insert(k) {
                        image_keys.push(k);
                    }
                }
            }
            let catalog: Vec<_> = TYPES
                .iter()
                .map(|kind| {
                    let names: Vec<_> = image_keys
                        .iter()
                        .filter(|k| assets[**k]["type"] == *kind)
                        .map(|k| assets[*k]["importName"].as_str().unwrap())
                        .collect();
                    if names.is_empty() {
                        "无".into()
                    } else {
                        names.join("、")
                    }
                })
                .collect();
            let mut units = vec![];
            for (ui, u) in originals.iter().enumerate() {
                let mut cursor = 0i64;
                let mut shots = vec![];
                let empty = empty_array();
                let raw_shots = u["shots"].as_array().unwrap();
                for (si, s) in raw_shots.iter().enumerate() {
                    let start = cursor;
                    let duration = time_ticks(&s["duration"], true, "DURATION")?;
                    cursor += duration;
                    let mut before = vec![];
                    if let Some(segment) = optional_array(u, "segments", &empty)?
                        .iter()
                        .find(|segment| segment["shotIds"][0] == s["id"])
                    {
                        let last = array(&segment["shotIds"], "SEGMENTS")?
                            .last()
                            .ok_or("SEGMENTS: no shot")?;
                        let end = raw_shots
                            .iter()
                            .position(|shot| shot["id"] == *last)
                            .ok_or("SEGMENTS: unknown shot")?
                            + 1;
                        let range = format!("分镜{}—{end}", si + 1);
                        for (label, key) in [
                            ("分段标题", "title"),
                            ("分段演出", "unitDirection"),
                            ("光影承接", "lighting"),
                            ("人物状态", "initialState"),
                            ("分段结束状态", "exitState"),
                        ] {
                            if text(&segment[key]) {
                                before.push(field(
                                    &format!("{label}｜{range}"),
                                    segment[key].as_str().unwrap(),
                                ));
                            }
                        }
                    }
                    let mut fields = vec![
                        field("机位/运镜", s["camera"].as_str().unwrap()),
                        field("画面", s["picture"].as_str().unwrap()),
                        field("光影", s["lighting"].as_str().unwrap()),
                        field("声音", s["sound"].as_str().unwrap()),
                    ];
                    for p in s["dialogueParts"].as_array().unwrap() {
                        let line = string(&p["lineId"], "DIALOGUE_UNKNOWN")?;
                        let q = quotes
                            .get(line)
                            .ok_or("DIALOGUE_UNKNOWN: original quote absent")?;
                        let from = time_text_ticks(
                            start + time_ticks(&p["voiceStart"], false, "TIME_PRECISION")?,
                        );
                        let to = time_text_ticks(
                            start + time_ticks(&p["voiceEnd"], false, "TIME_PRECISION")?,
                        );
                        let spoken = string(&p["text"], "DIALOGUE_TEXT")?;
                        let spoken_start = *progress.get(line).unwrap_or(&0);
                        let spoken_end = spoken_start + spoken.encode_utf16().count();
                        let quote = string(&q["quote"], "SOURCE_BINDING")?;
                        let quote_len = quote.encode_utf16().count();
                        let mut performance = vec![];
                        for cue in optional_array(q, "performanceCues", &empty)? {
                            let offset = cue["quoteOffset"]
                                .as_f64()
                                .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= quote_len as f64)
                                .ok_or("VOICE_CUE_POSITION: invalid cue offset")?
                                as usize;
                            need(text(&cue["text"]), "VOICE_CUE_POSITION", "empty cue")?;
                            if offset < spoken_start
                                || offset > spoken_end
                                || offset == spoken_end && spoken_end < quote_len
                            {
                                continue;
                            }
                            let relative = offset - spoken_start;
                            let place = if relative == 0 {
                                "本段开口前".into()
                            } else if offset == quote_len {
                                "本句收尾后".into()
                            } else {
                                let utf16: Vec<_> = spoken.encode_utf16().take(relative).collect();
                                format!(
                                    "本段第{}个字符后",
                                    String::from_utf16_lossy(&utf16).chars().count()
                                )
                            };
                            if bundle["generationTextVersion"] != "voice-delivery-v1" {
                                performance.push(format!(
                                    "句中原声提示（{place}）：{}",
                                    cue["text"].as_str().unwrap()
                                ));
                            }
                        }
                        progress.insert(line, spoken_end);
                        let kind = match q["kind"].as_str() {
                            Some("speech") => "口语",
                            Some("roar") => "兽吼",
                            Some("scream") => "尖叫",
                            Some("vocalization") => "非词人声",
                            _ => return Err("VOICE_KIND: unknown kind".into()),
                        };
                        let mut cues = vec![
                            string(&q["channel"], "CHANNEL")?.into(),
                            string(&p["presentation"], "CHANNEL")?.into(),
                            format!("原声类别：{kind}"),
                        ];
                        if bundle["generationTextVersion"] != "voice-delivery-v1" && text(&q["cue"])
                        {
                            cues.push(format!("原声提示：{}", q["cue"].as_str().unwrap()));
                        }
                        cues.extend(performance);
                        cues.push(format!(
                            "演法：{}",
                            string(&p["delivery"], "DIALOGUE_DELIVERY")?
                        ));
                        fields.push(field(
                            "台词",
                            format!(
                                "{from}–{to}秒｜{}（{}）：{spoken}",
                                string(&q["speaker"], "SOURCE_BINDING")?,
                                cues.join("｜")
                            ),
                        ));
                    }
                    if s["dialogueParts"].as_array().unwrap().is_empty() {
                        fields.push(field("台词", "无"));
                    }
                    fields.push(field("转场", s["transition"].as_str().unwrap()));
                    shots.push(json!({"number":si+1,"start":start as f64/TICKS as f64,"end":cursor as f64/TICKS as f64,"duration":duration as f64/TICKS as f64,"before":before,"fields":fields}));
                }
                units.push(json!({"number":ui+1,"duration":container_seconds(cursor),"header":header(e,scene,u,&assets)?,"shots":shots,"footer":[field("结束状态",string(&u["exitState"],"UNIT_FIELDS")?)]}));
            }
            scenes.push(json!({"id":id,"assets":catalog,"units":units}));
        }
        episodes.push(json!({"number":e["number"],"scenes":scenes}));
    }
    let out = json!({"profile":"lingmo-v7","episodes":episodes});
    for e in out["episodes"].as_array().unwrap() {
        for scene in e["scenes"].as_array().unwrap() {
            for u in scene["units"].as_array().unwrap() {
                let mut fields: Vec<&Value> = u["header"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .chain(u["footer"].as_array().unwrap())
                    .collect();
                for s in u["shots"].as_array().unwrap() {
                    fields.extend(s["before"].as_array().unwrap());
                    fields.extend(s["fields"].as_array().unwrap());
                }
                for f in fields {
                    if !TOKEN_LABELS.contains(&f[0].as_str().unwrap()) {
                        need(
                            !f[1].as_str().unwrap().contains("@["),
                            "LINGMO_RESERVED_TOKEN",
                            "token-like prose cannot add platform references",
                        )?;
                    }
                }
            }
        }
    }
    Ok(out)
}
fn line(f: &Value, tag: bool) -> Result<String> {
    let label = string(&f[0], "PROJECTION")?;
    let text = string(&f[1], "PROJECTION")?.replace('\n', "\n  ");
    Ok(if tag {
        format!("[{label}] {text}")
    } else {
        format!("{label}：{text}")
    })
}
pub(super) fn render_unit(u: &Value) -> Result<String> {
    let mut lines = vec![];
    for f in array(&u["header"], "PROJECTION")? {
        lines.push(line(f, true)?);
    }
    for s in array(&u["shots"], "PROJECTION")? {
        for f in array(&s["before"], "PROJECTION")? {
            lines.push(line(f, true)?);
        }
        lines.push(format!(
            "分镜{}｜{}-{}s｜{}s",
            s["number"],
            time_text_ticks(time_ticks(&s["start"], false, "TIME_PRECISION")?),
            time_text_ticks(time_ticks(&s["end"], false, "TIME_PRECISION")?),
            time_text_ticks(time_ticks(&s["duration"], true, "TIME_PRECISION")?)
        ));
        for f in array(&s["fields"], "PROJECTION")? {
            lines.push(line(f, false)?);
        }
    }
    for f in array(&u["footer"], "PROJECTION")? {
        lines.push(line(f, true)?);
    }
    Ok(lines.join("\n") + "\n")
}
pub(super) fn render(projected: &Value) -> Result<String> {
    let mut out = "**【标准化剧本】**\n".to_string();
    let mut scenes = 0;
    for e in array(&projected["episodes"], "PROJECTION")? {
        out.push_str(&format!(
            "第{}集\n",
            crate::series_lite_registry::canonical(&e["number"])?
        ));
        for scene in array(&e["scenes"], "PROJECTION")? {
            scenes += 1;
            out.push_str(string(&scene["id"], "PROJECTION")?);
            out.push('\n');
            for (i, label) in ["场景", "人物", "道具"].iter().enumerate() {
                out.push_str(&format!(
                    "{label}：{}\n",
                    string(&scene["assets"][i], "PROJECTION")?
                ));
            }
            for u in array(&scene["units"], "PROJECTION")? {
                out.push_str(&format!("片段{}\n时长：{}秒\n", u["number"], u["duration"]));
                out.push_str(&render_unit(u)?);
            }
        }
    }
    need(
        scenes > 0,
        "LINGMO_SCENE_PROP_BUDGET",
        "missing scene asset triplet",
    )?;
    Ok(out)
}
pub(super) fn catalog(
    bundle: &Value,
    ordered: &Ordered,
    projected: &Value,
    markdown: &str,
    bundle_hash: &str,
) -> Result<Ordered> {
    let assets: BTreeMap<_, _> = bundle["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["key"].as_str().unwrap(), a))
        .collect();
    let mut keys = vec![];
    let mut seen = HashSet::new();
    for e in bundle["episodes"].as_array().unwrap() {
        for u in e["draft"]["units"].as_array().unwrap() {
            for k in u["imageBindings"].as_array().unwrap() {
                let key = k.as_str().unwrap();
                if seen.insert(key) {
                    keys.push(key);
                }
            }
        }
    }
    keys.sort_by_key(|k| {
        TYPES
            .iter()
            .position(|t| assets[k]["type"] == *t)
            .unwrap_or(3)
    });
    let mut mapped = vec![];
    for key in keys {
        let a = assets[key];
        let f = asset_fields(a)?;
        mapped.push(Ordered::object(vec![
            (
                "id",
                a.get("assetId")
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!(format!("fixture:{key}"))),
            ),
            (
                "assetRevision",
                a.get("assetRevision")
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or(json!(1)),
            ),
            ("name", f["name"].clone()),
            ("type", f["type"].clone()),
            ("token", f["token"].clone()),
            ("filename", f["filename"].clone()),
            ("placeholderOrdinal", f["placeholderOrdinal"].clone()),
        ]));
    }
    let mut units = vec![];
    for (ei, e) in bundle["episodes"].as_array().unwrap().iter().enumerate() {
        for (si, scene) in e["scenes"].as_array().unwrap().iter().enumerate() {
            let originals: Vec<_> = e["draft"]["units"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|u| u["sceneId"] == scene["id"])
                .collect();
            for (ui, u) in projected["episodes"][ei]["scenes"][si]["units"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let body = render_unit(u)?;
                let image_tokens = originals[ui]["imageBindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|k| token(assets[k.as_str().unwrap()]))
                    .collect::<Result<Vec<_>>>()?;
                units.push(Ordered::object(vec![
                    ("episode", e["number"].clone()),
                    ("sceneId", scene["id"].clone()),
                    ("unitId", originals[ui]["id"].clone()),
                    ("fragment", u["number"].clone()),
                    (
                        "actualSeconds",
                        u["shots"].as_array().unwrap().last().unwrap()["end"].clone(),
                    ),
                    ("containerSeconds", u["duration"].clone()),
                    ("imageTokens", json!(image_tokens)),
                    ("bodySha256", json!(hash(&body))),
                    (
                        "bodyWithoutTokensSha256",
                        json!(hash(without_tokens(&body))),
                    ),
                ]));
            }
        }
    }
    let mut root = match Ordered::object(vec![
        ("version", json!("lingmo-v7-assets-v1")),
        ("exportProfile", json!("lingmo-v7")),
        ("testFixture", bundle["testFixture"].clone()),
        ("bundleSha256", json!(bundle_hash)),
        ("markdownSha256", json!(hash(markdown))),
    ]) {
        Ordered::Object(f) => f,
        _ => unreachable!(),
    };
    if let Some(scope) = ordered.get("productionScope") {
        root.push(("productionScope".into(), scope.clone()));
        root.push((
            "scopeHash".into(),
            Ordered::from_value(&json!(crate::series_lite_registry::hash_season_scope(
                &bundle["productionScope"]
            )?)),
        ));
        root.push((
            "seasonDelivery".into(),
            Ordered::from_value(bundle.get("seasonDelivery").unwrap_or(&json!("preview"))),
        ));
    }
    root.push(("assets".into(), Ordered::Array(mapped)));
    root.push(("units".into(), Ordered::Array(units)));
    Ok(Ordered::Object(root))
}
