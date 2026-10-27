//! Frozen performance-core-v4 validateDraft + explicit fantasy interval checks.
//! Source normalization is a separate trust boundary; this module never fabricates it.
use super::*;
use regex::Regex;
fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn has(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.trim().is_empty())
}
fn numeric(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}
fn js(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
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
fn issue(code: &str, coordinate: impl Into<String>, detail: impl Into<String>) -> Value {
    json!({"code":code,"coordinate":coordinate.into(),"detail":detail.into()})
}
fn id(v: &Value, fallback: &str) -> String {
    if truth(v) {
        js(v)
    } else {
        fallback.into()
    }
}
fn row_id(row: &Value, source: &Value) -> String {
    if truth(&row["id"]) {
        js(&row["id"])
    } else {
        format!(
            "{}:{}",
            js(&source["key"]),
            if truth(&row["sourceId"]) {
                js(&row["sourceId"])
            } else {
                format!("L{:04}", row["line"].as_i64().unwrap_or(0))
            }
        )
    }
}
pub(super) fn aliases(row: &Value) -> Vec<String> {
    let mut out = vec![];
    for k in ["id", "sourceId"] {
        if let Some(v) = row.get(k) {
            if !v.is_null() && *v != "" {
                out.push(js(v));
            }
        }
    }
    if let Some(n) = numeric(&row["line"]).filter(|n| n.fract() == 0.0) {
        let n = crate::series_lite_registry::canonical(&json!(n)).unwrap();
        if row["numericLineAlias"] != false {
            out.push(n.clone());
        }
        let source = row["sourceId"].as_str().unwrap_or("");
        if !(source.starts_with(['P', 'p'])
            && source.len() > 1
            && source[1..].bytes().all(|b| b.is_ascii_digit()))
        {
            out.push(format!("L{n:0>4}"));
            out.push(format!("L{n}"));
        }
    }
    if let Some(n) = numeric(&row["sourceFileLine"]).filter(|n| n.fract() == 0.0) {
        let n = crate::series_lite_registry::canonical(&json!(n)).unwrap();
        out.push(format!("L{n:0>4}"));
        out.push(format!("L{n}"));
    }
    out.sort();
    out.dedup();
    out
}
pub fn resolve_source_ref<'a>(
    source: &'a Value,
    reference: &Value,
    allow_context: bool,
) -> Result<&'a Value> {
    let key = js(reference);
    let source_key = js(&source["key"]);
    let mut found = None;
    for row in list(&source["sourceRows"]).iter().chain(if allow_context {
        list(&source["contextRows"])
    } else {
        &[]
    }) {
        if aliases(row)
            .iter()
            .any(|a| *a == key || format!("{source_key}:{a}") == key)
        {
            need(found.is_none(), "SOURCE_REF_AMBIGUOUS", &key)?;
            found = Some(row);
        }
    }
    found.ok_or_else(|| format!("SOURCE_REF_UNKNOWN: {key}"))
}
fn resolved_id(source: &Value, reference: &Value, allow_context: bool) -> Option<String> {
    resolve_source_ref(source, reference, allow_context)
        .ok()
        .map(|r| row_id(r, source))
}
pub fn valid_visual_subjects(v: &Value) -> bool {
    let Some(subjects) = v.as_array() else {
        return false;
    };
    let mut seen = HashSet::new();
    subjects.iter().all(|s| {
        s.as_object()
            .is_some_and(|o| o.keys().all(|k| k == "name" || k == "domain"))
            && has(&s["name"])
            && matches!(s["domain"].as_str(), Some("subjective" | "prospective"))
            && seen.insert((s["name"].as_str().unwrap(), s["domain"].as_str().unwrap()))
    })
}
fn fixed(n: f64, digits: u32) -> f64 {
    if !n.is_finite() || n.abs() >= 1e21 {
        return n;
    }
    let sign = if n < 0.0 { -1.0 } else { 1.0 };
    let bits = n.abs().to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let mut significand = (bits & ((1u64 << 52) - 1)) as u128;
    let shift = if exp == 0 {
        -1074
    } else {
        significand += 1u128 << 52;
        exp - 1023 - 52
    };
    let scale = 10u128.pow(digits);
    let product = significand * scale;
    let rounded = if shift >= 0 {
        product.checked_shl(shift as u32).unwrap_or(u128::MAX)
    } else {
        let s = (-shift) as u32;
        if s >= 128 {
            0
        } else {
            let q = product >> s;
            let rem = product - (q << s);
            q + u128::from(s > 0 && rem >= (1u128 << (s - 1)))
        }
    };
    sign * rounded as f64 / scale as f64
}
pub fn fantasy_timing(record: &Value, draft: &Value) -> Option<Value> {
    let spans = record["screenSpans"].as_array()?;
    if record["status"] == "not_realized" {
        return spans
            .is_empty()
            .then(|| json!({"shotIds":[],"screenSeconds":0}));
    }
    let owned = record["unitIds"].as_array()?;
    if record["status"] != "realized" || spans.is_empty() || owned.is_empty() {
        return None;
    }
    let mut shots: BTreeMap<&str, (&str, usize, &Value)> = BTreeMap::new();
    let mut units = HashSet::new();
    let mut index = 0;
    for u in list(&draft["units"]) {
        let uid = u["id"].as_str()?;
        if !units.insert(uid) {
            return None;
        }
        for s in list(&u["shots"]) {
            let sid = s["id"].as_str()?;
            if shots.insert(sid, (uid, index, &s["duration"])).is_some() {
                return None;
            }
            index += 1;
        }
    }
    let mut unit_seen = HashSet::new();
    for id in owned {
        let id = id.as_str()?;
        if !units.contains(id) || !unit_seen.insert(id) {
            return None;
        }
    }
    let mut ids = vec![];
    let mut seconds = 0.0;
    let mut last = None;
    let mut last_end = 0.0;
    for span in spans {
        let sid = span["shotId"].as_str()?;
        let (uid, order, d) = shots.get(sid)?;
        let duration = numeric(d).filter(|n| *n > 0.0)?;
        if !unit_seen.contains(uid) || last.is_some_and(|n| *order < n) {
            return None;
        }
        let (start, end) = if span.get("range") == Some(&Value::Null) {
            (0.0, duration)
        } else {
            let range = span["range"].as_array()?;
            if range.len() != 2 {
                return None;
            }
            let start = numeric(&range[0])?;
            let end = numeric(&range[1])?;
            if start < 0.0 || end <= start || end > duration + 1e-6 {
                return None;
            }
            (start, end)
        };
        if last == Some(*order) && start < last_end {
            return None;
        }
        seconds += end - start;
        if last != Some(*order) {
            ids.push(sid);
        }
        last = Some(*order);
        last_end = end;
    }
    Some(json!({"shotIds":ids,"screenSeconds":fixed(seconds,6)}))
}
pub fn validate_draft(
    draft: &Value,
    source: &Value,
    settings: &Value,
    selection: Option<&Value>,
    plan: &Value,
    unit_direction_version: Option<&str>,
) -> Value {
    let mut errors = vec![];
    let mut warnings = vec![];
    let units = list(&draft["units"]);
    let qs = list(&source["dialogue"]);
    let qmap: BTreeMap<_, _> = qs
        .iter()
        .enumerate()
        .map(|(i, q)| (js(&q["id"]), (i, q)))
        .collect();
    let rows: BTreeMap<_, _> = list(&source["sourceRows"])
        .iter()
        .map(|r| (row_id(r, source), r))
        .collect();
    let mut covered = HashSet::new();
    let mut unit_ids = HashSet::new();
    let mut shot_ids = HashSet::new();
    let mut owner = BTreeMap::new();
    let mut assembled: BTreeMap<String, String> = BTreeMap::new();
    let mut previous = None;
    let mut part_count = 0;
    let mut seconds = 0.0;
    if draft["schemaVersion"] != "performance-core-v4" {
        errors.push(issue("SCHEMA", "schemaVersion", "应为performance-core-v4"));
    }
    if units.is_empty() {
        errors.push(issue("UNITS", "units", "缺少完整单元"));
    }
    if draft.get("unresolved").is_some()
        && (draft["unresolved"].as_array().is_none_or(|a| !a.is_empty()))
    {
        errors.push(issue(
            "UNRESOLVED",
            "unresolved",
            "整编仍有未解决问题，不能作为可交稿",
        ));
    }
    let cap = settings
        .get("maxUnitSeconds")
        .unwrap_or(&json!(30))
        .as_f64()
        .unwrap_or(f64::NAN);
    let effective_cap = if cap == 15.0 || cap == 30.0 {
        cap
    } else {
        30.0
    };
    let event_mode = plan["unitPackingPolicy"] == "event-first-v1";
    let events: BTreeMap<_, _> = list(&plan["events"])
        .iter()
        .map(|e| (js(&e["id"]), e))
        .collect();
    if cap != 15.0 && cap != 30.0 {
        errors.push(issue(
            "SETTINGS",
            "maxUnitSeconds",
            "单元时长上限须为15或30秒",
        ));
    }
    if numeric(&settings["maxUnits"]).is_some_and(|n| units.len() as f64 > n) {
        errors.push(issue("MAX_UNITS", "units", "超过单元数上限"));
    }
    let camera_re=Regex::new(r"(?:先[^。；;]{0,35}再切(?:成|至|到)|同一镜内[^。；;]{0,35}(?:随即|然后)[^。；;]{0,10}切|连续摇焦[^。；;]{0,15}切|(?:随即|然后)切同一镜内)").unwrap();
    for u in units {
        let uid = id(&u["id"], "unit");
        let key = if u.get("id").is_some() {
            js(&u["id"])
        } else {
            "undefined".into()
        };
        if !has(&u["id"]) || unit_ids.contains(&key) {
            errors.push(issue("UNIT_ID", &uid, "单元ID缺失或重复"));
        }
        unit_ids.insert(key);
        let event = events.get(&js(&u["eventId"]));
        if event_mode && event.is_none() {
            errors.push(issue("EVENT_ID", &uid, "单元未归属本次明确完整事件"));
        }
        if event_mode
            && (!matches!(
                u["boundaryBefore"]["kind"].as_str(),
                Some("episode_start" | "continuous" | "capacity" | "new_event" | "time_space_jump")
            ) || !has(&u["boundaryBefore"]["reason"]))
        {
            errors.push(issue("EVENT_BOUNDARY", &uid, "单元边界或依据缺失"));
        }
        for k in ["title", "scene", "lighting", "initialState", "exitState"] {
            if !has(&u[k]) {
                errors.push(issue("FIELD", &uid, format!("单元{k}缺失")));
            }
        }
        if unit_direction_version == Some("unit-direction-v1") {
            if !has(&u["unitDirection"]) {
                errors.push(issue("UNIT_DIRECTION", &uid, "单元专属演出总指令缺失"));
            }
            for (i, s) in list(&u["segments"]).iter().enumerate() {
                if !has(&s["unitDirection"]) {
                    errors.push(issue(
                        "UNIT_DIRECTION",
                        format!("{uid}.segments[{i}]"),
                        "合并前分段的专属演出总指令缺失",
                    ));
                }
            }
        }
        for k in ["props", "participants"] {
            if u[k].as_array().is_none_or(|a| a.iter().any(|v| !has(v))) {
                errors.push(issue("FIELD", &uid, format!("单元{k}格式错误")));
            }
        }
        if u.get("visualSubjects").is_some() && !valid_visual_subjects(&u["visualSubjects"]) {
            errors.push(issue(
                "VISUAL_SUBJECTS",
                &uid,
                "主观或未来图示主体清单格式、类型或唯一性无效；不当作现实出场或觉醒事实",
            ));
        }
        if !["subject", "change", "landing"]
            .iter()
            .all(|k| has(&u["intent"][k]))
            || u["intent"]["retain"]
                .as_array()
                .is_none_or(|a| a.is_empty() || a.iter().any(|v| !has(v)))
        {
            errors.push(issue(
                "INTENT",
                &uid,
                "情绪主体、变化、落点及保留功能不完整",
            ));
        }
        let shots = list(&u["shots"]);
        if shots.is_empty() {
            errors.push(issue("SHOTS", &uid, "缺少镜头"));
            continue;
        }
        let mut duration = 0.0;
        for s in shots {
            let sid = id(&s["id"], &uid);
            let key = if s.get("id").is_some() {
                js(&s["id"])
            } else {
                "undefined".into()
            };
            if !has(&s["id"]) || shot_ids.contains(&key) {
                errors.push(issue("SHOT_ID", &sid, "镜头ID缺失或重复"));
            }
            shot_ids.insert(key.clone());
            owner.insert(key, uid.clone());
            if let Some(n) = numeric(&s["duration"]).filter(|n| *n > 0.0) {
                duration += n;
            } else {
                errors.push(issue("DURATION", &sid, "镜长须为正数"));
            }
            for k in ["camera", "picture", "lighting", "sound", "transition"] {
                if !has(&s[k]) {
                    errors.push(issue("FIELD", &sid, format!("镜头{k}缺失")));
                }
            }
            if s["camera"].as_str().is_some_and(|v| camera_re.is_match(v)) {
                warnings.push(issue(
                    "CAMERA_CUT",
                    &sid,
                    "单条镜头疑似另起硬切，请核对是否需拆镜计时",
                ));
            }
            if let Some(refs) = s["sourceRefs"].as_array() {
                for r in refs {
                    if let Some(id) =
                        resolved_id(source, r, false).filter(|id| rows.contains_key(id))
                    {
                        covered.insert(id.clone());
                        if event_mode
                            && event.is_some_and(|e| !list(&e["sourceRefs"]).contains(&json!(id)))
                        {
                            errors.push(issue("EVENT_SOURCE", &sid, "镜头制作源越出所标完整事件"));
                        }
                    } else {
                        errors.push(issue(
                            "SOURCE_REF",
                            &sid,
                            format!("未知或有歧义的源引用{}", js(r)),
                        ));
                    }
                }
            } else {
                errors.push(issue("SOURCE_REF", &sid, "缺少源引用数组"));
            }
            if let Some(refs) = s.get("contextRefs") {
                if let Some(refs) = refs.as_array() {
                    for r in refs {
                        if resolved_id(source, r, true).is_none() {
                            errors.push(issue(
                                "CONTEXT_REF",
                                &sid,
                                format!("未知或有歧义的前情引用{}", js(r)),
                            ));
                        }
                    }
                } else {
                    errors.push(issue("CONTEXT_REF", &sid, "前情引用须为数组"));
                }
            }
            let Some(parts) = s["dialogueParts"].as_array() else {
                errors.push(issue("FIELD", &sid, "缺少dialogueParts数组"));
                continue;
            };
            let mut voice_end = 0.0;
            for p in parts {
                part_count += 1;
                let qid = js(&p["lineId"]);
                let Some((index, q)) = qmap.get(&qid) else {
                    errors.push(issue("DIALOGUE_REF", &sid, format!("未知原句{qid}")));
                    continue;
                };
                if previous.is_some_and(|i| *index < i) {
                    errors.push(issue("DIALOGUE_ORDER", &sid, format!("原句{qid}倒序")));
                }
                previous = Some(*index);
                if !has(&p["text"]) {
                    errors.push(issue("DIALOGUE_CONTENT", &sid, "台词片段为空"));
                }
                assembled
                    .entry(qid.clone())
                    .or_default()
                    .push_str(p["text"].as_str().unwrap_or(""));
                let start = numeric(&p["voiceStart"]);
                let end = numeric(&p["voiceEnd"]);
                if start.is_none()
                    || end.is_none()
                    || start.unwrap() < 0.0
                    || end.unwrap() <= start.unwrap()
                    || s.get("duration")
                        .and_then(|v| if v.is_null() { Some(0.0) } else { numeric(v) })
                        .is_some_and(|d| end.unwrap() > d + 1e-6)
                    || start.unwrap() < voice_end - 1e-6
                {
                    errors.push(issue(
                        "VOICE_WINDOW",
                        &sid,
                        format!("原句{qid}发声窗越界或叠话未声明"),
                    ));
                }
                voice_end = end.unwrap_or(f64::NAN);
                if !matches!(
                    p["presentation"].as_str(),
                    Some("画内" | "画外" | "VO" | "OS" | "系统音" | "画外转画内" | "画内转画外")
                ) || !has(&p["delivery"])
                {
                    errors.push(issue(
                        "DIALOGUE_PRESENTATION",
                        &sid,
                        format!("原句{qid}呈现或演法缺失"),
                    ));
                }
                if matches!(q["channel"].as_str(), Some("VO" | "OS"))
                    && p["presentation"] != q["channel"]
                {
                    errors.push(issue(
                        "CHANNEL",
                        &sid,
                        format!("原句{qid}的{}声道被改变", js(&q["channel"])),
                    ));
                }
                if q["channel"] == "现场对白"
                    && matches!(p["presentation"].as_str(), Some("VO" | "OS"))
                {
                    errors.push(issue("CHANNEL", &sid, format!("原句{qid}现场声被改变")));
                }
                if q["channel"] == "系统音"
                    && !matches!(p["presentation"].as_str(), Some("画外" | "系统音"))
                {
                    errors.push(issue("CHANNEL", &sid, format!("原句{qid}系统音被改变")));
                }
            }
        }
        seconds += duration;
        if duration > effective_cap + 1e-6 {
            errors.push(issue(
                "UNIT_DURATION",
                &uid,
                format!("单元{}秒超过{}秒", fixed(duration, 3), effective_cap),
            ));
        }
    }
    for q in qs {
        let id = js(&q["id"]);
        if assembled.get(&id).map(String::as_str) != q["quote"].as_str() {
            errors.push(issue(
                "DIALOGUE_CONTENT",
                &id,
                "逐字拼接与原话不一致（含缺漏或重复）",
            ));
        }
    }
    let punctuation = Regex::new(r"^\s*[！!？?。，,.、…；;：:]+\s*$").unwrap();
    let metadata =
        Regex::new(r"^\s*(?:场景[：:]|地点[：:]|时间[：:]|出场人物[：:]|第.{0,12}集)").unwrap();
    for r in list(&source["sourceRows"]) {
        let t = r["text"].as_str().unwrap_or("");
        if has(&r["text"])
            && !matches!(
                r["kind"].as_str(),
                Some("blank" | "punctuation" | "metadata")
            )
            && !punctuation.is_match(t)
            && (truth(&r["kind"]) || !metadata.is_match(t))
            && !covered.contains(&row_id(r, source))
        {
            errors.push(issue(
                "SOURCE_COVERAGE",
                row_id(r, source),
                "源剧情行未关联镜头；关联不证明画面语义正确",
            ));
        }
    }
    for f in list(&source["entryFacts"]) {
        for r in list(&f["sourceRefs"]) {
            if resolved_id(source, r, true).is_none() {
                errors.push(issue(
                    "ENTRY_REF",
                    id(&f["id"], "entryFact"),
                    format!("关键前情出处无效：{}", js(r)),
                ));
            }
        }
    }
    let reality = list(&draft["fantasyRealization"]);
    if !draft["fantasyRealization"].is_array() {
        errors.push(issue(
            "FANTASY_FIELD",
            "fantasyRealization",
            "缺少幻想兑现记录数组",
        ));
    }
    let selected: HashSet<_> = selection
        .map(|s| list(&s["selectedIds"]))
        .unwrap_or(&[])
        .iter()
        .map(js)
        .collect();
    let candidates: HashSet<_> = list(&plan["fantasyCandidates"])
        .iter()
        .map(|c| js(&c["id"]))
        .collect();
    let mut seen = HashSet::new();
    for (index, f) in reality.iter().enumerate() {
        if !f.is_object() {
            errors.push(issue(
                "FANTASY_FIELD",
                format!("fantasyRealization[{index}]"),
                "幻想兑现记录须为对象",
            ));
            continue;
        }
        let fid = id(&f["candidateId"], "fantasy");
        if !seen.insert(fid.clone()) {
            errors.push(issue("FANTASY_DUPLICATE", &fid, "候选兑现记录重复"));
        }
        if !candidates.contains(&fid) {
            errors.push(issue("FANTASY_UNKNOWN", &fid, "未知幻想候选"));
        }
        if selection.is_some() && !selected.contains(&fid) {
            errors.push(issue("FANTASY_UNSELECTED", &fid, "未选候选不得实施"));
        }
        if !matches!(f["status"].as_str(), Some("realized" | "not_realized")) {
            errors.push(issue("FANTASY_STATUS", &fid, "兑现状态无效"));
        }
        if f.get("screenSpans").is_some() {
            let timing = fantasy_timing(f, draft);
            if timing.as_ref().is_none_or(|t| {
                f["timingVersion"] != "fantasy-spans-v1"
                    || !same(&f["shotIds"], &t["shotIds"])
                    || !same(&f["screenSeconds"], &t["screenSeconds"])
            }) {
                errors.push(issue(
                    "FANTASY_SPAN",
                    &fid,
                    "幻想镜内范围、顺序、归属或派生占时不一致；不猜入出或截断镜长",
                ));
            }
        }
        if f["status"] != "realized" {
            continue;
        }
        let owned = list(&f["unitIds"]);
        let unique: HashSet<_> = owned.iter().map(js).collect();
        if owned.is_empty()
            || unique.len() != owned.len()
            || unique.iter().any(|id| !unit_ids.contains(id))
        {
            errors.push(issue("FANTASY_UNIT", &fid, "缺少有效单元归属或归属重复"));
        }
        let all_shots: Vec<_> = units.iter().flat_map(|u| list(&u["shots"])).collect();
        let refs = list(&f["shotIds"]);
        let linked: Vec<_> = refs
            .iter()
            .map(|id| all_shots.iter().find(|s| s["id"] == *id).copied())
            .collect();
        let refs_unique: HashSet<_> = refs.iter().map(js).collect();
        if refs.is_empty()
            || refs_unique.len() != refs.len()
            || refs.iter().enumerate().any(|(i, id)| {
                !shot_ids.contains(&js(id))
                    || owner
                        .get(&js(id))
                        .is_none_or(|owner| !unique.contains(owner))
                    || linked[i].is_none_or(|s| !has(&s["picture"]))
            })
        {
            errors.push(issue(
                "FANTASY_SHOT",
                &fid,
                "缺少有效镜号归属、镜头重复或画面正文",
            ));
        }
        if !linked.is_empty()
            && linked
                .iter()
                .all(|s| s.is_some_and(|s| numeric(&s["duration"]).is_some()))
            && numeric(&f["screenSeconds"]).is_some_and(|n| {
                n > linked
                    .iter()
                    .map(|s| numeric(&s.unwrap()["duration"]).unwrap())
                    .sum::<f64>()
                    + 1e-6
            })
        {
            errors.push(issue(
                "FANTASY_DURATION",
                &fid,
                "幻想屏幕时长超过关联镜头合计镜长",
            ));
        }
        if numeric(&f["screenSeconds"]).is_none_or(|n| n <= 0.0)
            || !["entry", "exit", "domain", "preservedFunction"]
                .iter()
                .all(|k| has(&f[k]))
        {
            errors.push(issue("FANTASY_DETAIL", &fid, "屏幕时长、入出与功能不完整"));
        }
    }
    let mut missing_seen = HashSet::new();
    for selected_id in selection.map(|s| list(&s["selectedIds"])).unwrap_or(&[]) {
        let id = js(selected_id);
        if missing_seen.insert(id.clone())
            && !reality
                .iter()
                .any(|f| f["candidateId"] == *selected_id && f["status"] == "realized")
        {
            errors.push(issue("FANTASY_MISSING", id, "已选幻想尚无兑现记录"));
        }
    }
    json!({"errors":errors,"warnings":warnings,"stats":{"units":units.len(),"shots":shot_ids.len(),"dialogueParts":part_count,"seconds":fixed(seconds,3),"sourceRowsCovered":covered.len(),"sourceRowsTotal":list(&source["sourceRows"]).len(),"selectedFantasy":selected.len(),"realizedFantasy":reality.iter().filter(|f|f["status"]=="realized").count()}})
}

/// Complete checkedEvents gate from storyboard.mjs; it never changes event/unit packing.
pub fn checked_events(plan: &Value, source: &Value) -> Result<Option<Value>> {
    let Some(policy) = plan.get("unitPackingPolicy") else {
        return Ok(None);
    };
    need(
        *policy == "event-first-v1",
        "PLAN_EVENT_POLICY",
        "unknown event policy",
    )?;
    let events = plan["events"]
        .as_array()
        .ok_or("PLAN_EVENTS: event directory missing")?;
    need(!events.is_empty(), "PLAN_EVENTS", "event directory empty")?;
    let punctuation = Regex::new(r"^\s*[！!？?。，,.、…；;：:]+\s*$").unwrap();
    let rows = list(&source["sourceRows"]);
    let required: Vec<_> = rows
        .iter()
        .filter(|r| {
            has(&r["text"])
                && !matches!(
                    r["kind"].as_str(),
                    Some("blank" | "punctuation" | "metadata")
                )
                && !punctuation.is_match(r["text"].as_str().unwrap())
        })
        .map(|r| &r["id"])
        .collect();
    let mut ids = HashSet::new();
    let mut owned = HashSet::new();
    let mut previous_end = None;
    for (i, event) in events.iter().enumerate() {
        need(
            has(&event["id"]) && ids.insert(js(&event["id"])) && has(&event["sceneKey"]),
            "PLAN_EVENT",
            "event ID/scene missing or duplicate",
        )?;
        let boundary = &event["boundaryBefore"];
        need(
            matches!(
                boundary["kind"].as_str(),
                Some("episode_start" | "continuous" | "new_event" | "time_space_jump")
            ) && has(&boundary["reason"])
                && ((i == 0) == (boundary["kind"] == "episode_start")),
            "PLAN_EVENT_BOUNDARY",
            "invalid first/event boundary",
        )?;
        need(
            !(i > 0
                && boundary["kind"] == "continuous"
                && event["sceneKey"] != events[i - 1]["sceneKey"]),
            "PLAN_EVENT_BOUNDARY",
            "continuous event changed scene",
        )?;
        let refs = event["sourceRefs"]
            .as_array()
            .ok_or("PLAN_EVENT_REFS: references missing")?;
        need(!refs.is_empty(), "PLAN_EVENT_REFS", "empty references")?;
        let mut seen = HashSet::new();
        let mut previous_row = None;
        for r in refs {
            let pos = rows
                .iter()
                .position(|row| row["id"] == *r)
                .ok_or("PLAN_EVENT_REFS: unknown reference")?;
            need(
                seen.insert(js(r)) && previous_row.is_none_or(|p| pos > p),
                "PLAN_EVENT_REFS",
                "duplicate/reversed references",
            )?;
            previous_row = Some(pos);
        }
        let indices: Vec<_> = refs
            .iter()
            .filter_map(|r| required.iter().position(|v| **v == *r))
            .collect();
        need(
            !indices.is_empty()
                && previous_end.is_none_or(|end| indices[0] > end)
                && indices.last().unwrap() - indices[0] + 1 == indices.len(),
            "PLAN_EVENT_CONTINUITY",
            "event source must be contiguous and nonoverlapping",
        )?;
        previous_end = indices.last().copied();
        for r in refs {
            if required.contains(&r) {
                need(
                    owned.insert(js(r)),
                    "PLAN_EVENT_OVERLAP",
                    "source row assigned twice",
                )?;
            }
        }
        for q in list(&source["dialogue"]) {
            let quote_refs = list(&q["sourceIds"]);
            if quote_refs.iter().any(|r| refs.contains(r)) {
                need(
                    quote_refs.iter().all(|r| refs.contains(r)),
                    "PLAN_EVENT_REFS",
                    "one quote cannot span separate events",
                )?;
            }
        }
    }
    need(
        required.iter().all(|r| owned.contains(&js(r))),
        "PLAN_EVENT_COVERAGE",
        "required source lacks event assignment",
    )?;
    Ok(Some(Value::Array(events.clone())))
}
