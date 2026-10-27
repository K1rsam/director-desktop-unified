use super::*;
fn first(v: &Value) -> Option<usize> {
    v.as_array()?.first().and_then(|p| number(&p["start"]))
}
fn copy(text: &str, parts: &Value) -> IntakeResult<String> {
    arr(parts, "parts")?
        .iter()
        .map(|p| {
            slice(
                text,
                number(&p["start"]).ok_or_else(|| err("INVALID_RANGE", "parts.start"))?,
                number(&p["end"]).ok_or_else(|| err("INVALID_RANGE", "parts.end"))?,
            )
        })
        .collect()
}
fn post_fields(text: &str, child: &Value) -> bool {
    let (Some(a), Some(b)) = (first(&child["speaker"]), first(&child["parts"])) else {
        return false;
    };
    let Ok(header) = slice(text, a, b) else {
        return false;
    };
    re(r"(?i)^[^:：()（）\r\n]+?(?:[（(][^()（）\r\n]*[）)](?:[\t \x{a0}\x{3000}]*(?:OS|VO))?)?[\t ]*[:：]").find(&header).is_some_and(|m|[&child["cue"],&child["channelSource"]].iter().any(|v|first(v).is_some_and(|p|p>=a+utf16_len(m.as_str()))))
}
pub(super) fn entry_post_cue(text: &str, entry: &Value) -> bool {
    if entry["target"] != "BODY" {
        return false;
    }
    if entry["kind"] == "DIALOGUE" {
        post_fields(text, entry)
    } else if entry["kind"] == "DIALOGUE_GROUP" {
        entry["dialogues"]
            .as_array()
            .is_some_and(|a| a.iter().any(|c| post_fields(text, c)))
    } else {
        false
    }
}
pub fn standard_script_plan_version(
    index: &Value,
    entries: &Value,
    options: &Value,
) -> IntakeResult<String> {
    let v = options
        .get("inlinePerformanceVersion")
        .map(st)
        .unwrap_or("inline-performance-v2");
    need(
        matches!(v, "inline-performance-v1" | "inline-performance-v2"),
        "UNSUPPORTED_PERFORMANCE_VERSION",
        "options.inlinePerformanceVersion",
    )?;
    let entries = arr(entries, "entries")?;
    let text = st(&index["text"]);
    if options["inlinePerformance"] != false {
        for e in entries.iter().filter(|e| e["target"] == "BODY") {
            let children: Vec<&Value> = match st(&e["kind"]) {
                "DIALOGUE" => vec![e],
                "DIALOGUE_GROUP" => arr(&e["dialogues"], "entry.dialogues")?.iter().collect(),
                "BODY_GROUP" => arr(&e["children"], "entry.children")?
                    .iter()
                    .filter(|c| c["kind"] == "DIALOGUE")
                    .collect(),
                _ => vec![],
            };
            for c in children {
                if crate::series_lite_bundle_standard::inline_performance_projection(
                    text,
                    &c["parts"],
                    v,
                )?
                .is_some()
                    || v == "inline-performance-v2"
                        && crate::series_lite_bundle_standard::compact_performance_head_projection(
                            text,
                            &c["speaker"],
                            &c["cue"],
                            &c["channelSource"],
                        )?
                        .is_some()
                {
                    return Ok(if v == "inline-performance-v2" {
                        "STANDARD_SCRIPT_PLAN_V5"
                    } else {
                        "STANDARD_SCRIPT_PLAN_V4"
                    }
                    .into());
                }
            }
        }
    }
    Ok(if entries
        .iter()
        .any(|e| e["target"] == "BODY" && e["kind"] == "BODY_GROUP")
    {
        "STANDARD_SCRIPT_PLAN_V3"
    } else if entries
        .iter()
        .any(|e| e["target"] == "BODY" && e["kind"] == "DIALOGUE_GROUP" || entry_post_cue(text, e))
    {
        "STANDARD_SCRIPT_PLAN_V2"
    } else {
        "STANDARD_SCRIPT_PLAN_V1"
    }
    .into())
}
pub fn derive_standard_script_lines(prepared: &Value) -> IntakeResult<Value> {
    let mut by_unit = BTreeMap::new();
    for e in arr(&prepared["plan"]["entries"], "plan.entries")? {
        for id in arr(&e["unitIds"], "entry.unitIds")? {
            by_unit.insert(st(id), e);
        }
    }
    let text = st(&prepared["index"]["text"]);
    let mut lines = vec![];
    for (i, u) in arr(&prepared["index"]["units"], "index.units")?
        .iter()
        .enumerate()
    {
        let e = by_unit.get(st(&u["id"]));
        let mut l = json!({"line":i+1,"unitId":u["id"],"entryId":e.map(|e|e["id"].clone()).unwrap_or(Value::Null),"target":e.map(|e|e["target"].clone()).unwrap_or(json!("UNRESOLVED")),"kind":e.map(|e|if e["target"]=="EXCLUDE"{e["category"].clone()}else{e["kind"].clone()}).unwrap_or(json!("UNRESOLVED")),"text":u["text"]});
        if let Some(e) = e {
            if e["target"] == "BODY" {
                if e["kind"] == "DIALOGUE" {
                    l["speaker"] = json!(copy(text, &e["speaker"])?);
                    l["channel"] = e["channel"].clone();
                } else if e["kind"] == "DIALOGUE_GROUP" {
                    l["speakers"] = json!(arr(&e["dialogues"], "entry.dialogues")?
                        .iter()
                        .map(|c| copy(text, &c["speaker"]))
                        .collect::<IntakeResult<Vec<_>>>()?);
                } else if e["kind"] == "BODY_GROUP" {
                    if let Some(d) = arr(&e["children"], "entry.children")?
                        .iter()
                        .find(|c| c["kind"] == "DIALOGUE")
                    {
                        l["speaker"] = json!(copy(text, &d["speaker"])?);
                        l["channel"] = d["channel"].clone();
                    }
                }
            }
        }
        lines.push(l);
    }
    Ok(json!(lines))
}
pub fn explicit_subtitles(text: &str) -> Value {
    let u: Vec<_> = text.encode_utf16().collect();
    let pairs = |c: u16| match c {
        0x201c => Some(0x201d),
        0x2018 => Some(0x2019),
        0x300c => Some(0x300d),
        0x300e => Some(0x300f),
        34 => Some(34),
        39 => Some(39),
        0x3010 => Some(0x3011),
        91 => Some(93),
        _ => None,
    };
    let mut stack = vec![];
    let mut spans = vec![];
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        if c == 92 {
            i += 2;
            continue;
        }
        let ascii = |x: u16| x <= 127 && (x as u8).is_ascii_alphanumeric();
        if matches!(c, 39 | 0x2019)
            && i > 0
            && i + 1 < u.len()
            && ascii(u[i - 1])
            && ascii(u[i + 1])
        {
            i += 1;
            continue;
        }
        if stack.last() == Some(&c) {
            stack.pop();
            i += 1;
            continue;
        }
        if c == 39
            && !stack.contains(&39)
            && i > 0
            && matches!(u[i - 1], 83 | 115)
            && i + 1 < u.len()
            && char::from_u32(u[i + 1] as u32).is_some_and(ws)
        {
            i += 1;
            continue;
        }
        let rest = String::from_utf16_lossy(&u[i..]);
        if stack.is_empty() && re(r"(?i)^[【\[](?:字幕|SUBTITLE)(?:[：:\t \]】]|$)").is_match(&rest)
        {
            let pattern = re(r"(?i)^【(字幕|SUBTITLE)[：:]([^【】\r\n]+)】");
            let m = pattern.captures(&rest);
            if m.as_ref().is_none_or(|m| trim(&m[2]).is_empty()) {
                return json!({"spans":spans,"invalid":true});
            }
            let m = m.unwrap();
            let body = i + 1 + utf16_len(&m[1]) + 1;
            let end = i + utf16_len(&m[0]);
            spans.push(json!({"start":i,"end":end,"labelStart":i+1,"labelEnd":i+1+utf16_len(&m[1]),"bodyStart":body,"bodyEnd":body+utf16_len(&m[2])}));
            i = end;
            continue;
        }
        if let Some(close) = pairs(c) {
            if !matches!(c, 0x3010 | 91)
                || stack.is_empty()
                || stack.last().is_some_and(|c| matches!(c, 0x3011 | 93))
            {
                stack.push(close);
            }
        }
        i += 1;
    }
    json!({"spans":spans,"invalid":false})
}
