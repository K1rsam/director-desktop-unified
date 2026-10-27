use super::*;
fn present(v: &Value) -> bool {
    !v.is_null() && v != ""
}
fn js(v: &Value) -> String {
    v.as_str()
        .map(str::to_string)
        .unwrap_or_else(|| canonical(v).unwrap_or_default())
}
pub fn verify_standard_source(source: &Value) -> Result<()> {
    let pkg = &source["standardPackage"];
    let ev = arr(&pkg["evidence"], "STANDARD_RECEIPT")?;
    let text = ev.iter().map(|u| st(&u["text"])).collect::<String>();
    let mut opts = json!({"key":js(&source["key"]),"context":{},"sourceText":text});
    if let Some(t) = source.get("title") {
        opts["title"] = t.clone();
    }
    let original = standard_package_source(pkg, &opts)?;
    let rows = arr(&original["sourceRows"], "STANDARD_RECEIPT")?;
    let notes = arr(&original["contextRows"], "STANDARD_RECEIPT")?;
    let expected: BTreeMap<_, _> = rows
        .iter()
        .chain(notes)
        .map(|r| (st(&r["id"]), r))
        .collect();
    let all: Vec<_> = rows.iter().map(|r| r["id"].clone()).collect();
    let scope = source
        .get("standardScope")
        .filter(|v| !v.is_null() && **v != false);
    let selected = if let Some(s) = scope {
        let ids = arr(&s["sourceIds"], "STANDARD_SCOPE")?;
        let unique: HashSet<_> = ids.iter().map(js).collect();
        need(
            s["version"] == "standard-scope-v1"
                && unique.len() == ids.len()
                && ids.iter().all(|id| all.contains(id)),
            "STANDARD_SCOPE",
        )?;
        all.into_iter()
            .filter(|id| ids.contains(id))
            .collect::<Vec<_>>()
    } else {
        all
    };
    let actual = source["sourceRows"].as_array().cloned().unwrap_or_default();
    let context = source["contextRows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    need(
        actual.iter().map(|r| r["id"].clone()).collect::<Vec<_>>() == selected,
        "STANDARD_SOURCE_COVERAGE",
    )?;
    need(
        notes
            .iter()
            .all(|n| context.iter().any(|r| r["id"] == n["id"])),
        "STANDARD_NOTE_COVERAGE",
    )?;
    for r in actual.iter().chain(&context) {
        if let Some(b) = expected.get(st(&r["id"])) {
            need(
                r.get("text") == b.get("text")
                    && r.get("line") == b.get("line")
                    && equal(&r["standard"], &b["standard"])
                    && r.get("evidenceRole") == b.get("evidenceRole"),
                "STANDARD_ROW_BINDING",
            )?;
        } else {
            need(
                r.get("standard").is_none_or(|s| s.is_null() || s == false),
                "STANDARD_ROW_BINDING",
            )?;
        }
    }
    need(
        actual
            .iter()
            .all(|r| expected.contains_key(st(&r["id"])) && r["standard"].is_object()),
        "STANDARD_ROW_BINDING",
    )
}
pub fn standard_row_kind(row: &Value) -> Result<&'static str> {
    let s = &row["standard"];
    need(
        matches!(st(&s["target"]), "BODY" | "STRUCTURE" | "EXCLUDE"),
        "STANDARD_ROW",
    )?;
    if s["target"] == "EXCLUDE" {
        need(trim(st(&row["text"])).is_empty(), "STANDARD_EXCLUSION")?;
        return Ok("blank");
    }
    if s["target"] == "STRUCTURE" {
        return Ok("metadata");
    }
    need(
        matches!(
            st(&s["kind"]),
            "ACTION" | "DIALOGUE" | "DIALOGUE_GROUP" | "BODY_GROUP" | "SFX" | "ONSCREEN_TEXT"
        ),
        "STANDARD_ROW",
    )?;
    Ok(
        if matches!(st(&s["kind"]), "DIALOGUE" | "DIALOGUE_GROUP" | "BODY_GROUP") {
            "dialogue"
        } else {
            "action"
        },
    )
}
fn copy_local(text: &str, parts: &Value) -> Result<String> {
    let mut out = String::new();
    let mut edge = 0;
    for p in arr(parts, "STANDARD_ROW")? {
        let pair = arr(p, "STANDARD_ROW")?;
        need(pair.len() == 2, "STANDARD_ROW")?;
        let a = n(&pair[0]).ok_or("STANDARD_ROW")?;
        let b = n(&pair[1]).ok_or("STANDARD_ROW")?;
        need(a >= edge && b > a && b <= len(text), "STANDARD_ROW")?;
        out.push_str(&slice(text, a, b)?);
        edge = b;
    }
    Ok(out)
}
pub fn standard_dialogues(row: &Value) -> Result<Vec<Value>> {
    if standard_row_kind(row)? != "dialogue" {
        return Ok(vec![]);
    }
    let s = &row["standard"];
    let children: Vec<&Value> = match st(&s["kind"]) {
        "BODY_GROUP" => arr(&s["children"], "STANDARD_ROW")?
            .iter()
            .filter(|c| c["kind"] == "DIALOGUE")
            .collect(),
        "DIALOGUE_GROUP" => arr(&s["dialogues"], "STANDARD_ROW")?.iter().collect(),
        _ => vec![s],
    };
    need(!children.is_empty(), "STANDARD_ROW")?;
    let text = st(&row["text"]);
    children.iter().enumerate().map(|(i,s)|{let speaker=copy_local(text,&s["speaker"])?;let quote=copy_local(text,s.get("performance").filter(|v|!v.is_null()).map(|p|&p["spoken"]).unwrap_or(&s["parts"]))?;let cue=[copy_local(text,s.get("channelSource").unwrap_or(&json!([])))?,copy_local(text,s.get("cue").unwrap_or(&json!([])))?].into_iter().filter(|s|!s.is_empty()).collect::<Vec<_>>().join("；");need(!trim(&speaker).is_empty()&&!trim(&quote).is_empty()&&matches!(st(&s["channel"]),"现场对白"|"OS"|"VO"|"系统音"),"STANDARD_ROW")?;let mut d=json!({"id":format!("{}:D{}",st(&row["id"]),i+1),"speaker":speaker,"quote":quote,"cue":cue,"channel":s["channel"],"rowId":row["id"]});if let Some(line)=row.get("line"){d["line"]=line.clone();}if let Some(p)=s.get("performance").filter(|v|!v.is_null()){d["performanceCues"]=p["cues"].clone();}Ok(d)}).collect()
}
pub fn standard_dialogue(row: &Value, index: usize) -> Result<Option<Value>> {
    if row["standard"]["kind"] == "DIALOGUE_GROUP" && index == 0 {
        return Ok(None);
    }
    let items = standard_dialogues(row)?;
    Ok(items
        .get(if row["standard"]["kind"] == "DIALOGUE_GROUP" {
            index.saturating_sub(1)
        } else {
            0
        })
        .cloned())
}
pub fn standard_onscreen_texts(row: &Value) -> Result<Vec<Value>> {
    let s = &row["standard"];
    let text = st(&row["text"]);
    if s["kind"] == "ONSCREEN_TEXT" {
        let parts = arr(&s["parts"], "STANDARD_ROW")?;
        let raw = copy_local(text, &s["parts"])?;
        let prefix = re(r"^(?:弹幕|字幕|屏幕文字)[\t \x{a0}\x{3000}]*[:：][\t \x{a0}\x{3000}]*")
            .find(&raw)
            .map(|m| len(m.as_str()))
            .unwrap_or(0);
        let mut edge = 0;
        let mut out = vec![];
        for p in parts {
            let a = n(&p[0]).ok_or("STANDARD_ROW")?;
            let b = n(&p[1]).ok_or("STANDARD_ROW")?;
            let skip = prefix.saturating_sub(edge);
            edge += b - a;
            if a + skip < b {
                out.push(json!([a + skip, b]));
            }
        }
        let mut result = json!({"id":format!("{}:T1",st(&row["id"])),"rowId":row["id"],"kind":"ONSCREEN_TEXT","text":tail(&raw,prefix),"parts":out,"sourceIds":[row["id"]]});
        if let Some(l) = row.get("line") {
            result["line"] = l.clone();
        }
        return Ok(vec![result]);
    }
    if s["kind"] != "BODY_GROUP" {
        return Ok(vec![]);
    }
    arr(&s["children"],"STANDARD_ROW")?.iter().filter(|c|c["kind"]=="ONSCREEN_TEXT").enumerate().map(|(i,c)|{let prefix=slice(text,0,n(&c["parts"][0][0]).ok_or("STANDARD_ROW")?)?;let lines=re(r"\r\n|\r|\n").find_iter(&prefix).count();Ok(json!({"id":format!("{}:T{}",st(&row["id"]),i+1),"rowId":row["id"],"line":n(&row["line"]).map(|l|l+lines),"kind":"ONSCREEN_TEXT","text":copy_local(text,&c["parts"] )?,"parts":c["parts"],"sourceIds":[row["id"]]}))}).collect()
}
fn normalize_rows(key: &str, v: &Value) -> Result<Vec<Value>> {
    if v.is_null() {
        return Ok(vec![]);
    }
    arr(v, "SOURCE_ROWS")?
        .iter()
        .map(|r| {
            need(r["text"].is_string(), "SOURCE_ROW_TEXT")?;
            for k in ["id", "sourceId"] {
                need(!present(&r[k]) || r[k].is_string(), "SOURCE_ROW_COORDINATE")?;
            }
            for k in ["line", "sourceFileLine"] {
                need(
                    !present(&r[k]) || n(&r[k]).is_some(),
                    "SOURCE_ROW_COORDINATE",
                )?;
            }
            let id = if present(&r["id"]) {
                js(&r["id"])
            } else if present(&r["sourceId"]) {
                format!("{key}:{}", js(&r["sourceId"]))
            } else if let Some(n) = n(&r["line"]) {
                format!("{key}:L{n:04}")
            } else {
                return Err("SOURCE_ROW_COORDINATE".into());
            };
            let mut r = r.clone();
            r["id"] = json!(id);
            Ok(r)
        })
        .collect()
}
fn kind(d: &Value) -> &'static str {
    let cue = format!("{} {}", st(&d["cue"]), st(&d["channel"]));
    let pure = re(r"^[啊呀哦喔呃呜吼嗷哇唔哼哈！!？?，,。.…\s]+$").is_match(st(&d["quote"]));
    if pure && re("咆哮|兽吼|怒吼").is_match(&cue) {
        "roar"
    } else if pure && re("尖叫|惨叫").is_match(&cue) {
        "scream"
    } else if pure && re("哼|呜咽|呻吟").is_match(&cue) {
        "vocalization"
    } else {
        "speech"
    }
}
pub fn normalize_standard_source(raw: &Value) -> Result<Value> {
    need(raw.is_object() && present(&raw["key"]), "SOURCE_KEY")?;
    need(
        raw["sourceProtocol"] == "standard-episode-v1",
        "SOURCE_PROTOCOL",
    )?;
    verify_standard_source(raw)?;
    let key = js(&raw["key"]);
    let mut rows = normalize_rows(&key, &raw["sourceRows"])?;
    let context = normalize_rows(&key, &raw["contextRows"])?;
    let mut source = raw.clone();
    source["key"] = json!(key);
    source["title"] = raw
        .get("title")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(json!(""));
    source["context"] = raw
        .get("context")
        .filter(|v| !v.is_null())
        .or_else(|| raw.get("episodeContext").filter(|v| !v.is_null()))
        .cloned()
        .unwrap_or(json!(""));
    source["assets"] = raw
        .get("assets")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(json!([]));
    source["sourceRows"] = json!(rows);
    source["contextRows"] = json!(context);
    // Resolve every alias against the complete set, preserving the frozen ambiguity guard.
    let mut aliases = HashSet::new();
    for r in rows.iter().chain(&context) {
        let mut set = HashSet::new();
        for k in ["id", "sourceId"] {
            if present(&r[k]) {
                set.insert(js(&r[k]));
            }
        }
        if let Some(num) = n(&r["line"]) {
            if r["numericLineAlias"] != false {
                set.insert(num.to_string());
            }
            if !re(r"(?i)^P\d+$").is_match(st(&r["sourceId"])) {
                set.extend([format!("L{num}"), format!("L{num:04}")]);
            }
        }
        if let Some(num) = n(&r["sourceFileLine"]) {
            set.extend([format!("L{num}"), format!("L{num:04}")]);
        }
        for a in set {
            need(aliases.insert(a), "SOURCE_ALIAS_AMBIGUOUS")?;
        }
    }
    let mut captions = vec![];
    let mut dialogue = vec![];
    for r in &mut rows {
        r["kind"] = json!(standard_row_kind(r)?);
        captions.extend(standard_onscreen_texts(r)?);
        dialogue.extend(standard_dialogues(r)?);
    }
    if let Some(actual) = raw.get("onscreenTexts") {
        let actual = arr(actual, "STANDARD_ONSCREEN_COVERAGE")?;
        need(actual.len() == captions.len(), "STANDARD_ONSCREEN_COVERAGE")?;
        for (a, b) in actual.iter().zip(&captions) {
            need(
                a.as_object()
                    .is_some_and(|o| o.keys().all(|k| b.get(k).is_some()))
                    && ["id", "rowId", "line", "kind", "text", "parts", "sourceIds"]
                        .iter()
                        .all(|k| a.get(*k) == b.get(*k)),
                "STANDARD_ONSCREEN_BINDING",
            )?;
        }
    }
    if !captions.is_empty()
        || matches!(
            st(&raw["standardPackage"]["schemaVersion"]),
            "DIRECTOR_STANDARD_SCRIPT_SLICE_V3"
                | "DIRECTOR_STANDARD_SCRIPT_SLICE_V4"
                | "DIRECTOR_STANDARD_SCRIPT_SLICE_V5"
        )
    {
        source["onscreenTexts"] = json!(captions);
    }
    if let Some(actual) = raw.get("dialogue") {
        let actual = arr(actual, "STANDARD_DIALOGUE_COVERAGE")?;
        need(actual.len() == dialogue.len(), "STANDARD_DIALOGUE_COVERAGE")?;
        for (a, b) in actual.iter().zip(&dialogue) {
            need(
                [
                    "id",
                    "rowId",
                    "speaker",
                    "quote",
                    "cue",
                    "channel",
                    "performanceCues",
                ]
                .iter()
                .all(|k| a.get(*k) == b.get(*k))
                    && (!present(&a["kind"]) || a["kind"] == kind(b))
                    && a["sourceIds"] == json!([b["rowId"]]),
                "STANDARD_DIALOGUE_BINDING",
            )?;
        }
    }
    for d in &mut dialogue {
        need(
            !st(&d["cue"]).contains("画外咆哮") || d["channel"] == "现场对白",
            "DIALOGUE_CHANNEL",
        )?;
        d["kind"] = json!(kind(d));
        d["originalChannel"] = if st(&d["cue"]).contains("画外咆哮") {
            json!("画外咆哮")
        } else {
            d["channel"].clone()
        };
        d["sourceIds"] = json!([d["rowId"]]);
    }
    let mut diagnostics = raw["diagnostics"].as_array().cloned().unwrap_or_default();
    for r in &mut rows {
        if re(r"^[！!？?，,。.……]+$").is_match(trim(st(&r["text"])))
            && !dialogue.iter().any(|d| d["rowId"] == r["id"])
        {
            diagnostics.push(json!({"rowId":r["id"],"reason":"孤立标点无法归属对白"}));
            r["kind"] = json!("action");
        }
    }
    source["sourceRows"] = json!(rows);
    source["dialogue"] = json!(dialogue);
    let mut facts = vec![];
    let mut ids = HashSet::new();
    for f in raw["entryFacts"].as_array().cloned().unwrap_or_default() {
        let refs = arr(&f["sourceRefs"], "ENTRY_FACT")?;
        need(
            present(&f["id"])
                && present(&f["subject"])
                && present(&f["statement"])
                && !refs.is_empty()
                && (present(&f["evidence"]) || f["evidenceMode"] == "source_rows")
                && (f.get("evidenceMode").is_none() || f["evidenceMode"] == "source_rows"),
            "ENTRY_FACT",
        )?;
        let bound = refs
            .iter()
            .map(|r| crate::series_lite_bundle_gate::resolve_source_ref(&source, r, true))
            .collect::<Result<Vec<_>>>()?;
        need(
            bound.iter().all(|r| r["evidenceRole"] != "guidance"),
            "ENTRY_FACT_GUIDANCE",
        )?;
        let matching = |v: &Value| {
            v.as_array().is_some_and(|a| {
                a.len() == bound.len()
                    && a.iter().zip(&bound).all(|(p, r)| {
                        p.as_str()
                            .is_some_and(|s| !trim(s).is_empty() && st(&r["text"]).contains(s))
                    })
            })
        };
        let mut ev = f["evidence"].clone();
        if f["evidenceMode"] == "source_rows" {
            let derived = json!(bound.iter().map(|r| r["text"].clone()).collect::<Vec<_>>());
            need(
                f.get("evidence").is_none() || equal(&ev, &derived),
                "ENTRY_FACT_EVIDENCE",
            )?;
            ev = derived;
        } else if !(ev.as_str().is_some_and(|s| {
            !trim(s).is_empty()
                && bound
                    .iter()
                    .map(|r| st(&r["text"]))
                    .collect::<String>()
                    .contains(s)
        }) || matching(&ev))
        {
            let mut found = None;
            if let Some(s) = ev.as_str() {
                for pat in [r"\s*／\s*", r"\r?\n", r"\s+\|\s+"] {
                    let parts: Vec<_> = re(pat).split(s).map(|p| json!(trim(p))).collect();
                    let v = json!(parts);
                    if parts.len() > 1 && matching(&v) {
                        found = Some(v);
                        break;
                    }
                }
            }
            ev = found.ok_or("ENTRY_FACT_EVIDENCE")?;
        }
        need(ids.insert(js(&f["id"])), "ENTRY_FACT_ID_DUPLICATE")?;
        let mut f = f.clone();
        f["evidence"] = ev;
        f["sourceRefs"] = json!(bound.iter().map(|r| r["id"].clone()).collect::<Vec<_>>());
        facts.push(f);
    }
    source["entryFacts"] = json!(facts);
    source["diagnostics"] = json!(diagnostics);
    Ok(source)
}
