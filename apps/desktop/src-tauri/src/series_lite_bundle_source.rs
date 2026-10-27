//! Legacy raw-row normalizeSource port. standard-episode-v1 is deliberately refused
//! until its independent package/node/UTF-16 proof verifier is ported in full.
use super::*;
use regex::Regex;
use std::sync::{Mutex, OnceLock};
fn re(pattern: &'static str) -> Regex {
    static CACHE: OnceLock<Mutex<BTreeMap<&'static str, Regex>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap();
    cache.entry(pattern).or_insert_with(||Regex::new(&pattern.replace(r"\s",r"[\x09-\x0d\x20\u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]").replace(r"\d","[0-9]")).unwrap()).clone()
}
fn present(v: Option<&Value>) -> bool {
    v.is_some_and(|v| !v.is_null() && *v != "")
}
fn integer(v: &Value) -> bool {
    v.as_f64()
        .is_some_and(|n| n.is_finite() && n.fract() == 0.0)
}
fn js(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        _ => crate::series_lite_registry::canonical(v).unwrap_or_default(),
    }
}
fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Number(_) => v.as_f64().is_some_and(|n| n != 0.0),
        _ => true,
    }
}
fn js_space(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn trim(s: &str) -> &str {
    s.trim_matches(js_space)
}
fn row_list(raw: &Value, key: &str) -> Result<Vec<Value>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(a)) => Ok(a.clone()),
        _ => Err("SOURCE_ROWS: expected array".into()),
    }
}
fn normalize_rows(key: &str, rows: Vec<Value>) -> Result<Vec<Value>> {
    rows.into_iter()
        .map(|mut r| {
            need(
                r["text"].is_string(),
                "SOURCE_ROW_TEXT",
                "source row text missing",
            )?;
            for field in ["id", "sourceId"] {
                need(
                    !present(r.get(field)) || r[field].is_string(),
                    "SOURCE_ROW_COORDINATE",
                    "invalid coordinate type",
                )?;
            }
            for field in ["line", "sourceFileLine"] {
                need(
                    !present(r.get(field))
                        || (integer(&r[field]) && r[field].as_f64().unwrap() >= 0.0),
                    "SOURCE_ROW_COORDINATE",
                    "invalid coordinate type",
                )?;
            }
            let id = if present(r.get("id")) {
                js(&r["id"])
            } else if present(r.get("sourceId")) {
                format!("{key}:{}", js(&r["sourceId"]))
            } else if integer(&r["line"]) {
                format!("{key}:L{:0>4}", js(&r["line"]))
            } else {
                return Err("SOURCE_ROW_COORDINATE: row lacks stable coordinate".into());
            };
            r["id"] = json!(id);
            Ok(r)
        })
        .collect()
}
fn metadata(text: &str, legacy: bool) -> bool {
    if re(r"^\s*(?:\d+[—–-]\d+(?:\s|[：:])|出场人物[：:]|第.{0,12}集(?:[：:]|\s*$)|[一二三四五六七八九十]+[：:]|(?:标题|场景|时间|地点)[：:]|场景[ \t　]+[^\s：:][^：:\r\n]*$)").is_match(text){return true;}
    if legacy {
        return false;
    }
    if re(r"^\s*(?:人物|角色|道具|天气|内外|时间层)\s*[：:]").is_match(text) {
        return true;
    }
    let numbered = re(r"^\s*(?:场\s*)?\d+\s*([—–－.．-])\s*\d+([^\r\n：:]*)$")
        .captures(text)
        .map(|m| (m[1].to_string(), m[2].to_string()));
    let numbered = numbered.filter(|(_, rest)| {
        !rest.starts_with(['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '.', '．'])
    });
    if numbered.as_ref().is_some_and(|(separator, rest)| {
        trim(rest).is_empty() && separator != "." && separator != "．"
    }) {
        return true;
    }
    let content = numbered.as_ref().map(|(_, s)| s.as_str()).unwrap_or(text);
    let fields: Vec<_> = re(r"[\s，,]+").split(trim(content)).collect();
    let time = |s: &str| matches!(s, "日" | "夜" | "晨" | "午" | "黄昏");
    let io = |s: &str| matches!(s, "内" | "外");
    let pair = |a: &str, b: &str| (time(a) && io(b)) || (io(a) && time(b));
    if numbered.is_some() {
        fields.windows(2).any(|f| pair(f[0], f[1]))
    } else {
        fields.len() >= 3 && pair(fields[0], fields[1])
    }
}
fn production_note(text: &str) -> bool {
    let line = trim(text);
    if !re(r"^【(?:(?:音效|字幕|画面|镜头(?:切换|切)?|转场)[：:]|特写】)").is_match(line)
    {
        return false;
    }
    let mut depth = 0;
    for (i, c) in line.char_indices() {
        if c == '【' {
            depth += 1;
        } else if c == '】' {
            depth -= 1;
        }
        if depth == 0 {
            return i + c.len_utf8() == line.len();
        }
    }
    false
}
fn spoken(text: &str) -> (String, bool, bool) {
    let mut cursor = 0;
    let mut plain = String::new();
    let mut notes = false;
    let marker = re(r"【搭配画面[：:]");
    while let Some(m) = marker.find(&text[cursor..]) {
        let start = cursor + m.start();
        let mut end = cursor + m.end();
        let mut depth = 1;
        for (i, c) in text[end..].char_indices() {
            if c == '【' {
                depth += 1;
            } else if c == '】' {
                depth -= 1;
            }
            if depth == 0 {
                end += i + c.len_utf8();
                break;
            }
        }
        if depth != 0 {
            return (text.into(), true, false);
        }
        plain.push_str(&text[cursor..start]);
        cursor = end;
        notes = true;
    }
    plain.push_str(&text[cursor..]);
    (plain, notes, true)
}
fn channel(d: &Value) -> String {
    let labels = [&d["channel"], &d["cue"]]
        .into_iter()
        .filter(|v| truth(v))
        .map(js)
        .collect::<Vec<_>>()
        .join(" ");
    if labels.contains("画外咆哮") {
        return "现场对白".into();
    }
    if matches!(
        d["channel"].as_str(),
        Some("现场对白" | "VO" | "OS" | "系统音")
    ) {
        return js(&d["channel"]);
    }
    if labels.contains("系统音") || d["speaker"] == "系统音" {
        return "系统音".into();
    }
    if re(r"(?i)(?:^|[^A-Za-z0-9_])VO(?:[^A-Za-z0-9_]|$)").is_match(&labels) {
        return "VO".into();
    }
    if re(r"(?i)(?:^|[^A-Za-z0-9_])OS(?:[^A-Za-z0-9_]|$)").is_match(&labels) {
        return "OS".into();
    }
    if truth(&d["channel"]) {
        js(&d["channel"])
    } else {
        "现场对白".into()
    }
}
fn kind(d: &Value) -> String {
    if truth(&d["kind"]) {
        return js(&d["kind"]);
    }
    let cue = [&d["cue"], &d["channel"]]
        .into_iter()
        .filter(|v| truth(v))
        .map(js)
        .collect::<Vec<_>>()
        .join(" ");
    let quote = d["quote"].as_str().unwrap_or("");
    let pure = re(r"^[啊呀哦喔呃呜吼嗷哇唔哼哈！!？?，,。.…\s]+$").is_match(quote);
    if pure && re("咆哮|兽吼|怒吼").is_match(&cue) {
        "roar".into()
    } else if pure && re("尖叫|惨叫").is_match(&cue) {
        "scream".into()
    } else if pure && re("哼|呜咽|呻吟").is_match(&cue) {
        "vocalization".into()
    } else {
        "speech".into()
    }
}
fn extract(row: &Value, index: usize, legacy: bool) -> Option<Value> {
    let text = row["text"].as_str()?;
    if metadata(text, legacy) {
        return None;
    }
    let (spoken, _, valid) = if legacy {
        (text.into(), false, true)
    } else {
        spoken(text)
    };
    if !valid || !legacy && (re(r"^\s*▲").is_match(&spoken) || metadata(&spoken, false)) {
        return None;
    }
    let expression = re(r"^\s*([^：:△\s][^：:]{0,28})[：:]\s*(.*)$");
    let m = expression.captures(&spoken)?;
    if m[1].encode_utf16().count() > 29 || !legacy && m[1].contains(['【', '】']) {
        return None;
    }
    let mut speaker = trim(&m[1]).to_string();
    let mut rest = m[2].to_string();
    let mut cue = String::new();
    if let Some(pre) = re(r"^(.*?)[（(]([^）)]+)[）)]$").captures(&speaker) {
        let name = trim(&pre[1]).to_string();
        cue = pre[2].into();
        speaker = name;
    }
    if let Some(suffix) = re(r"(?i)^(.*?)(VO|OS)$").captures(&speaker) {
        let name = trim(&suffix[1]).to_string();
        if !cue.is_empty() {
            cue.push('；');
        }
        cue.push_str(&suffix[2].to_uppercase());
        speaker = name;
    }
    if speaker.is_empty() || re(r"\d+[—–-]\d+").is_match(&speaker) {
        return None;
    }
    while rest.starts_with(['（', '(']) {
        let opener = rest.chars().next().unwrap();
        let close = if opener == '（' { '）' } else { ')' };
        let Some(end) = rest.find(close) else {
            break;
        };
        if !cue.is_empty() {
            cue.push('；');
        }
        cue.push_str(&rest[opener.len_utf8()..end]);
        rest = rest[end + close.len_utf8()..]
            .trim_start_matches(js_space)
            .into();
    }
    if trim(&rest).is_empty() {
        return None;
    }
    let mut out = json!({"id":format!("{}:D{index}",js(&row["id"])),"speaker":speaker,"quote":rest,"cue":cue,"kind":kind(&json!({"cue":cue,"quote":rest})),"channel":channel(&json!({"speaker":speaker,"cue":cue})),"rowId":row["id"]});
    if let Some(line) = row.get("line") {
        out["line"] = line.clone();
    }
    Some(out)
}
fn punctuation(text: &str) -> bool {
    re(r"^[！!？?，,。.……]+$").is_match(trim(text))
}
pub fn normalize_legacy_source(raw: &Value) -> Result<Value> {
    need(
        raw.is_object() && present(raw.get("key")),
        "SOURCE_KEY",
        "source key missing",
    )?;
    if let Some(protocol) = raw.get("sourceProtocol") {
        need(
            *protocol == "standard-episode-v1",
            "SOURCE_PROTOCOL",
            "unknown source protocol",
        )?;
        return Err("SOURCE_NATIVE_STANDARD_PENDING: standard package proof verifier is not yet ported; do not downgrade to legacy".into());
    }
    let key = js(&raw["key"]);
    let mut rows = normalize_rows(&key, row_list(raw, "sourceRows")?)?;
    let context = normalize_rows(&key, row_list(raw, "contextRows")?)?;
    let mut aliases = HashSet::new();
    for row in rows.iter().chain(&context) {
        for alias in super::draft::aliases(row) {
            need(
                aliases.insert(alias),
                "SOURCE_ALIAS_AMBIGUOUS",
                "source alias duplicated",
            )?;
        }
    }
    for row in &mut rows {
        let text = row["text"].as_str().unwrap();
        row["kind"] = json!(if trim(text).is_empty() {
            "blank"
        } else if metadata(text, false) {
            "metadata"
        } else if extract(row, 0, false).is_some() || punctuation(text) {
            "dialogue"
        } else {
            "action"
        });
    }
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
    source["sourceRows"] = json!(rows);
    source["contextRows"] = json!(context);
    source["assets"] = raw
        .get("assets")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(json!([]));
    let mut diagnostics = row_list(raw, "diagnostics")?;
    let mut dialogue = if raw.get("dialogue").is_some_and(truth) {
        raw["dialogue"]
            .as_array()
            .cloned()
            .ok_or("SOURCE_DIALOGUE: expected array")?
    } else {
        vec![]
    };
    if !raw.get("dialogue").is_some_and(truth) {
        for (current, row) in rows.iter().enumerate() {
            let extracted = extract(row, dialogue.len() + 1, false);
            if let Some(d) = &extracted {
                dialogue.push(d.clone());
            } else if row["kind"] == "action"
                && !re(r"^\s*[△▲]").is_match(row["text"].as_str().unwrap())
                && !production_note(row["text"].as_str().unwrap())
            {
                let (s, notes, valid) = spoken(row["text"].as_str().unwrap());
                let prev = dialogue.last();
                let prior = prev
                    .and_then(|p| {
                        p["sourceIds"]
                            .as_array()
                            .and_then(|a| a.last())
                            .or_else(|| p.get("rowId"))
                    })
                    .and_then(|r| rows.iter().position(|v| v["id"] == *r));
                let head = prev.and_then(|p| rows.iter().find(|r| r["id"] == p["rowId"]));
                let cue_only = re(r"^\s*[^：:]+[：:]\s*[（(][^）)]+[）)]\s*$").is_match(&s);
                let empty_actor = notes && re(r"^\s*[^：:△▲\s][^：:]{0,28}[：:]\s*$").is_match(&s);
                let boundary =
                    re(r"^\s*[△▲]").is_match(&s) || metadata(&s, false) || cue_only || empty_actor;
                let continuation = valid
                    && !boundary
                    && !re(r"^\s*[【】]").is_match(&s)
                    && notes
                    && !trim(&s).is_empty()
                    && prev.is_some_and(|p| p["channel"] == "VO")
                    && head.is_some_and(|h| spoken(h["text"].as_str().unwrap()).1)
                    && prior.is_some_and(|p| {
                        p < current && rows[p + 1..current].iter().all(|r| r["kind"] == "blank")
                    });
                if continuation {
                    let p = dialogue.last_mut().unwrap();
                    p["quote"] = json!(format!("{}{}", p["quote"].as_str().unwrap(), s));
                    let mut ids = p["sourceIds"]
                        .as_array()
                        .cloned()
                        .unwrap_or_else(|| vec![p["rowId"].clone()]);
                    ids.push(row["id"].clone());
                    p["sourceIds"] = json!(ids);
                } else if !(valid && (boundary || (notes && trim(&s).is_empty()))) {
                    diagnostics.push(json!({"rowId":row["id"],"reason":"无法安全识别为对白"}));
                }
            }
            if extracted.is_none()
                && punctuation(row["text"].as_str().unwrap())
                && !dialogue.is_empty()
            {
                let p = dialogue.last_mut().unwrap();
                let ref_id = p["sourceIds"]
                    .as_array()
                    .and_then(|a| a.last())
                    .unwrap_or(&p["rowId"]);
                if let Some(prior) = rows.iter().position(|r| r["id"] == *ref_id) {
                    if prior < current
                        && rows[prior + 1..current]
                            .iter()
                            .all(|r| r["kind"] == "blank")
                    {
                        p["quote"] = json!(format!(
                            "{}{}",
                            p["quote"].as_str().unwrap(),
                            row["text"].as_str().unwrap()
                        ));
                        let mut ids = p["sourceIds"]
                            .as_array()
                            .cloned()
                            .unwrap_or_else(|| vec![p["rowId"].clone()]);
                        ids.push(row["id"].clone());
                        p["sourceIds"] = json!(ids);
                    }
                }
            }
        }
    }
    let mut normalized = vec![];
    for (i, d) in dialogue.iter().enumerate() {
        let line_matches: Vec<_> = if present(d.get("line")) {
            rows.iter().filter(|r| r["line"] == d["line"]).collect()
        } else {
            vec![]
        };
        let supplied = d["sourceIds"].as_array().filter(|a| !a.is_empty());
        need(
            supplied.is_some()
                || truth(&d["rowId"])
                || truth(&d["sourceId"])
                || line_matches.len() <= 1,
            "DIALOGUE_SOURCE_AMBIGUOUS",
            "ambiguous line",
        )?;
        let refs = if let Some(refs) = supplied {
            refs.clone()
        } else {
            vec![d
                .get("rowId")
                .filter(|v| !v.is_null())
                .or_else(|| d.get("sourceId").filter(|v| !v.is_null()))
                .cloned()
                .or_else(|| line_matches.first().map(|r| r["id"].clone()))
                .unwrap_or(Value::Null)]
        };
        need(
            !refs.is_empty() && truth(&refs[0]),
            "DIALOGUE_SOURCE",
            "quote source missing",
        )?;
        let resolved = refs
            .iter()
            .map(|r| resolve_source_ref(&source, r, false))
            .collect::<Result<Vec<_>>>()?;
        let mut extracted = extract(resolved[0], i + 1, false);
        let mut full = extracted
            .as_ref()
            .map(|q| q["quote"].as_str().unwrap().to_string())
            .unwrap_or_else(|| "undefined".into());
        for r in &resolved[1..] {
            full.push_str(&spoken(r["text"].as_str().unwrap()).0);
        }
        if raw.get("dialogue").is_some_and(truth) {
            if let Some(old) = extract(resolved[0], i + 1, true) {
                let old_quote = format!(
                    "{}{}",
                    old["quote"].as_str().unwrap(),
                    resolved[1..]
                        .iter()
                        .map(|r| r["text"].as_str().unwrap())
                        .collect::<String>()
                );
                let chosen = if present(d.get("channel")) {
                    channel(&json!({"channel":d["channel"]}))
                } else {
                    old["channel"].as_str().unwrap().into()
                };
                if old_quote == d["quote"].as_str().unwrap_or("")
                    && old["speaker"] == d["speaker"]
                    && old["channel"] == chosen
                {
                    extracted = Some(old);
                    full = old_quote;
                }
            }
        }
        let extracted = extracted.ok_or("DIALOGUE_SOURCE: source is not explicit dialogue")?;
        need(
            d["quote"] == full,
            "DIALOGUE_QUOTE",
            "original quote differs",
        )?;
        need(
            extracted["speaker"] == d["speaker"],
            "DIALOGUE_SPEAKER",
            "speaker differs",
        )?;
        let chosen = if present(d.get("channel")) {
            channel(&json!({"channel":d["channel"]}))
        } else {
            extracted["channel"].as_str().unwrap().into()
        };
        need(
            extracted["channel"] == chosen,
            "DIALOGUE_CHANNEL",
            "source channel differs",
        )?;
        need(
            !(truth(&d["kind"])
                && d["kind"] != "speech"
                && kind(&json!({"quote":d["quote"],"cue":d["cue"],"channel":d["channel"]}))
                    == "speech"),
            "DIALOGUE_KIND",
            "nonlexical kind contradicts words",
        )?;
        let cue = d
            .get("cue")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| {
                json!(if d["channel"] == "画外咆哮" {
                    "画外咆哮"
                } else {
                    ""
                })
            });
        let mut n = d.clone();
        n["id"] = if truth(&d["id"]) {
            d["id"].clone()
        } else {
            json!(format!("{}:D{}", js(&resolved[0]["id"]), i + 1))
        };
        n["cue"] = cue.clone();
        n["kind"] = json!(kind(&n));
        n["originalChannel"] = d
            .get("originalChannel")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| {
                if d["channel"] == "画外咆哮"
                    || extracted["cue"].as_str().unwrap_or("").contains("画外咆哮")
                {
                    json!("画外咆哮")
                } else {
                    d.get("channel")
                        .filter(|v| !v.is_null())
                        .cloned()
                        .unwrap_or(extracted["channel"].clone())
                }
            });
        n["channel"] = json!(chosen);
        n["rowId"] = resolved[0]["id"].clone();
        n["sourceIds"] = json!(resolved.iter().map(|r| r["id"].clone()).collect::<Vec<_>>());
        normalized.push(n);
    }
    let mut ids = HashSet::new();
    for q in &normalized {
        need(
            ids.insert(js(&q["id"])),
            "DIALOGUE_ID_DUPLICATE",
            "duplicate quote ID",
        )?;
    }
    let owned: HashSet<_> = normalized
        .iter()
        .flat_map(|q| q["sourceIds"].as_array().unwrap().iter().map(js))
        .collect();
    for row in &mut rows {
        if owned.contains(&js(&row["id"])) {
            row["kind"] = json!("dialogue");
        } else if punctuation(row["text"].as_str().unwrap()) {
            diagnostics.push(json!({"rowId":row["id"],"reason":"孤立标点无法归属对白"}));
            row["kind"] = json!("action");
        }
    }
    source["sourceRows"] = json!(rows);
    source["dialogue"] = json!(normalized);
    let mut facts = vec![];
    let mut fact_ids = HashSet::new();
    for f in row_list(raw, "entryFacts")? {
        let refs = f["sourceRefs"]
            .as_array()
            .ok_or("ENTRY_FACT: references missing")?;
        need(
            truth(&f["id"])
                && truth(&f["subject"])
                && truth(&f["statement"])
                && !refs.is_empty()
                && (truth(&f["evidence"]) || f["evidenceMode"] == "source_rows"),
            "ENTRY_FACT",
            "fact fields missing",
        )?;
        need(
            f.get("evidenceMode").is_none() || f["evidenceMode"] == "source_rows",
            "ENTRY_FACT",
            "unknown evidence mode",
        )?;
        let rows = refs
            .iter()
            .map(|r| resolve_source_ref(&source, r, true))
            .collect::<Result<Vec<_>>>()?;
        need(
            rows.iter().all(|r| r["evidenceRole"] != "guidance"),
            "ENTRY_FACT_GUIDANCE",
            "guidance cannot establish source fact",
        )?;
        let matches = |parts: &[Value]| {
            parts.len() == rows.len()
                && parts.iter().zip(&rows).all(|(p, r)| {
                    p.as_str().is_some_and(|s| {
                        !trim(s).is_empty() && r["text"].as_str().unwrap().contains(s)
                    })
                })
        };
        let mut evidence = f["evidence"].clone();
        if f["evidenceMode"] == "source_rows" {
            let extracted = json!(rows.iter().map(|r| r["text"].clone()).collect::<Vec<_>>());
            need(
                f.get("evidence").is_none() || same(&evidence, &extracted),
                "ENTRY_FACT_EVIDENCE",
                "explicit evidence differs from source rows",
            )?;
            evidence = extracted;
        } else if !evidence.as_str().is_some_and(|s| {
            !trim(s).is_empty()
                && rows
                    .iter()
                    .map(|r| r["text"].as_str().unwrap())
                    .collect::<String>()
                    .contains(s)
        }) && !evidence.as_array().is_some_and(|a| matches(a))
        {
            let mut segments = None;
            if let Some(text) = evidence.as_str() {
                for pattern in [r"\s*／\s*", r"\r?\n", r"\s+\|\s+"] {
                    let parts: Vec<_> = re(pattern).split(text).map(|s| json!(trim(s))).collect();
                    if parts.len() > 1 && matches(&parts) {
                        segments = Some(json!(parts));
                        break;
                    }
                }
            }
            evidence = segments.ok_or("ENTRY_FACT_EVIDENCE: evidence absent from source")?;
        }
        let mut out = f.clone();
        out["evidence"] = evidence;
        out["sourceRefs"] = json!(rows.iter().map(|r| r["id"].clone()).collect::<Vec<_>>());
        need(
            fact_ids.insert(js(&f["id"])),
            "ENTRY_FACT_ID_DUPLICATE",
            "duplicate fact ID",
        )?;
        facts.push(out);
    }
    source["entryFacts"] = json!(facts);
    source["diagnostics"] = json!(diagnostics);
    Ok(source)
}
