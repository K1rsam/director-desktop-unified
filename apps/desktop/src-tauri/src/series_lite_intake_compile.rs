use super::*;
#[derive(Clone)]
struct Span {
    start: usize,
    end: usize,
    unit: String,
    path: String,
}
struct Assembler<'a> {
    text: &'a str,
    units: &'a BTreeMap<String, Value>,
    owned: HashSet<String>,
    path: String,
}
impl Assembler<'_> {
    fn assemble(
        &self,
        parts: &Value,
        field: &str,
        empty: bool,
        sink: &mut Vec<Span>,
        gaps: bool,
    ) -> IntakeResult<Value> {
        let path = format!("{}.{field}", self.path);
        let parts = parts
            .as_array()
            .filter(|p| empty || !p.is_empty())
            .ok_or_else(|| err("EMPTY_OR_INVALID_PARTS", &path))?;
        let layout = field == "layout" || field.ends_with(".layout");
        let mut last = None;
        let mut text = String::new();
        let mut copied = vec![];
        for (i, p) in parts.iter().enumerate() {
            let pp = format!("{path}[{i}]");
            shape(
                p,
                &["kind", "unitId", "start", "end"],
                &["kind", "unitId", "start", "end"],
                &pp,
            )?;
            one_of(
                &p["kind"],
                &["SOURCE_SLICE", "SOURCE_SEPARATOR"],
                &format!("{pp}.kind"),
            )?;
            let uid = st(&p["unitId"]);
            let bound_err = |code: &str| {
                let mut e = err(code, &pp);
                e.source_unit_id = p["unitId"].as_str().map(str::to_string);
                e
            };
            let u = self
                .units
                .get(uid)
                .filter(|_| self.owned.contains(uid))
                .ok_or_else(|| bound_err("UNKNOWN_REFERENCE"))?;
            let a = number(&p["start"]).ok_or_else(|| bound_err("INVALID_RANGE"))?;
            let b = number(&p["end"]).ok_or_else(|| bound_err("INVALID_RANGE"))?;
            if a < number(&u["start"]).unwrap() || b > number(&u["end"]).unwrap() || a >= b {
                return Err(bound_err("INVALID_RANGE"));
            }
            let s = slice(self.text, a, b).map_err(|_| bound_err("UNICODE_BOUNDARY"))?;
            if last.is_some_and(|e| a < e) {
                return Err(bound_err("PART_ORDER_OR_DUPLICATION"));
            }
            if !layout && !gaps && last.is_some_and(|e| a != e) {
                return Err(bound_err("TEXT_GAP"));
            }
            last = Some(b);
            if p["kind"] == "SOURCE_SEPARATOR" && !re(r"^[\t \r\n]+$").is_match(&s) {
                return Err(bound_err("INVALID_SEPARATOR"));
            }
            if layout && !re(r"^[\s\x{feff}:：,，()（）△▲【】\[\]]+$").is_match(&s) {
                return Err(bound_err("NON_LAYOUT_CONTENT"));
            }
            sink.push(Span {
                start: a,
                end: b,
                unit: uid.into(),
                path: pp,
            });
            copied.push(
                json!({"kind":p["kind"],"unitId":p["unitId"],"start":p["start"],"end":p["end"]}),
            );
            text.push_str(&s);
        }
        Ok(json!({"text":text,"parts":copied}))
    }
    fn optional(
        &self,
        c: &Value,
        k: &str,
        prefix: &str,
        sink: &mut Vec<Span>,
    ) -> IntakeResult<Value> {
        if c[k].is_null() {
            Ok(Value::Null)
        } else {
            self.assemble(&c[k], &format!("{prefix}{k}"), false, sink, false)
        }
    }
}
fn first(parts: &Value) -> usize {
    number(&parts[0]["start"]).unwrap()
}
fn last(parts: &Value) -> usize {
    number(&parts.as_array().unwrap().last().unwrap()["end"]).unwrap()
}
fn coverage(spans: &mut [Span], a: usize, b: usize, code: &str, path: &str) -> IntakeResult<()> {
    spans.sort_by_key(|s| s.start);
    let mut edge = a;
    for s in spans {
        if s.start != edge {
            let mut e = err(code, &s.path);
            e.source_unit_id = Some(s.unit.clone());
            return Err(e);
        }
        edge = s.end;
    }
    need(edge == b, code, path)
}
fn delivery(v: &Value) -> String {
    if let Some(p) = v.get("performance") {
        format!(
            "{}\n{}",
            st(&p["spoken"]["text"]),
            p["cues"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| format!(
                    "【行中表演·台词位置{}】{}",
                    c["spokenOffset"],
                    st(&c["text"])
                ))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        st(&v["text"]).into()
    }
}
fn voice(v: &Value) -> String {
    format!(
        "【说者】{}\n【声道】{}\n{}",
        st(&v["speaker"]["text"]),
        st(&v["channel"]),
        if v["cue"].is_null() {
            String::new()
        } else {
            format!("【提示】{}\n", st(&v["cue"]["text"]))
        }
    )
}
fn reading(output: &Value) -> String {
    let label = match st(&output["target"]) {
        "ANNOTATION" => "辅助资料·语义未验收",
        "STRUCTURE" => "结构",
        _ => "正文",
    };
    let kind = st(&output["kind"]);
    if let Some(cs) = output["children"].as_array() {
        format!(
            "【{label}·BODY_GROUP】\n{}",
            cs.iter()
                .map(|c| if c["kind"] == "DIALOGUE" {
                    format!("{}{}", voice(c), delivery(c))
                } else {
                    format!("【画面文字】{}", st(&c["text"]))
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        )
    } else if let Some(ds) = output["dialogues"].as_array() {
        format!(
            "【{label}·{kind}】\n{}",
            ds.iter()
                .map(|d| format!("{}{}", voice(d), delivery(d)))
                .collect::<Vec<_>>()
                .join("\n\n")
        )
    } else {
        format!(
            "【{label}·{kind}】\n{}{}",
            if output.get("speaker").is_some() {
                voice(output)
            } else {
                String::new()
            },
            delivery(output)
        )
    }
}
pub fn compile_standard_script(index: &Value, plan: &Value) -> IntakeResult<Value> {
    need(
        index["text"].is_string()
            && index["sourceId"].is_string()
            && create_source_index(&index["text"], &index["sourceId"]).is_ok_and(|v| v == *index),
        "SOURCE_INDEX_INTEGRITY",
        "sourceIndex",
    )?;
    shape(
        plan,
        &["schemaVersion", "sourceId", "sourceVersion", "entries"],
        &["schemaVersion", "sourceId", "sourceVersion", "entries"],
        "plan",
    )?;
    need(
        matches!(
            st(&plan["schemaVersion"]),
            "STANDARD_SCRIPT_PLAN_V1"
                | "STANDARD_SCRIPT_PLAN_V2"
                | "STANDARD_SCRIPT_PLAN_V3"
                | "STANDARD_SCRIPT_PLAN_V4"
                | "STANDARD_SCRIPT_PLAN_V5"
        ),
        "UNSUPPORTED_SCHEMA",
        "plan.schemaVersion",
    )?;
    need(
        plan["sourceId"] == index["sourceId"] && plan["sourceVersion"] == index["sourceVersion"],
        "SOURCE_BINDING",
        "plan.sourceVersion",
    )?;
    let entries = arr(&plan["entries"], "plan.entries")?;
    let text = st(&index["text"]);
    let grouped = entries
        .iter()
        .any(|e| e["target"] == "BODY" && e["kind"] == "DIALOGUE_GROUP");
    let mixed = entries
        .iter()
        .any(|e| e["target"] == "BODY" && e["kind"] == "BODY_GROUP");
    let post = entries.iter().any(|e| projection::entry_post_cue(text, e));
    let v5 = plan["schemaVersion"] == "STANDARD_SCRIPT_PLAN_V5";
    let performance = v5 || plan["schemaVersion"] == "STANDARD_SCRIPT_PLAN_V4";
    let perfversion = if v5 {
        "inline-performance-v2"
    } else {
        "inline-performance-v1"
    };
    // Rust scalar strings cannot represent JS lone-surrogate slices. Reject such plan
    // boundaries with the compiler's original code before the version probe reads them.
    let source_units: Vec<_> = text.encode_utf16().collect();
    fn unicode_preflight(v: &Value, path: &str, source: &[u16]) -> IntakeResult<()> {
        if let Some(a) = v.as_array() {
            for (i, p) in a.iter().enumerate() {
                unicode_preflight(p, &format!("{path}[{i}]"), source)?;
            }
        } else if let Some(o) = v.as_object() {
            if o.contains_key("start") && o.contains_key("end") {
                for k in ["start", "end"] {
                    if let Some(at) = number(&v[k]) {
                        if at > 0
                            && at < source.len()
                            && (0xd800..=0xdbff).contains(&source[at - 1])
                            && (0xdc00..=0xdfff).contains(&source[at])
                        {
                            let mut e = err("UNICODE_BOUNDARY", path);
                            e.source_unit_id = v["unitId"].as_str().map(str::to_string);
                            return Err(e);
                        }
                    }
                }
            } else {
                for k in [
                    "parts",
                    "layout",
                    "speaker",
                    "cue",
                    "channelSource",
                    "dialogues",
                    "children",
                ] {
                    if let Some(child) = o.get(k) {
                        unicode_preflight(child, &format!("{path}.{k}"), source)?;
                    }
                }
            }
        }
        Ok(())
    }
    for (i, e) in entries.iter().enumerate() {
        unicode_preflight(e, &format!("plan.entries[{i}]"), &source_units)?;
    }
    need(
        standard_script_plan_version(
            index,
            &plan["entries"],
            &json!({"inlinePerformance":performance,"inlinePerformanceVersion":perfversion}),
        )? == st(&plan["schemaVersion"]),
        "UNSUPPORTED_SCHEMA",
        "plan.schemaVersion",
    )?;
    let unit_list = arr(&index["units"], "sourceIndex.units")?;
    let units: BTreeMap<_, _> = unit_list
        .iter()
        .map(|u| (st(&u["id"]).to_string(), u.clone()))
        .collect();
    let mut script = vec![];
    let mut annotations = vec![];
    let mut exclusions = vec![];
    let mut assignments = vec![];
    let mut ids = HashSet::new();
    let mut rendered = vec![];
    let mut next = 0;
    for (position, e) in entries.iter().enumerate() {
        let path = format!("plan.entries[{position}]");
        need(e.is_object() || e.is_array(), "INVALID_ENTRY", &path)?;
        one_of(
            &e["target"],
            &["BODY", "STRUCTURE", "ANNOTATION", "EXCLUDE"],
            &format!("{path}.target"),
        )?;
        let exclude = e["target"] == "EXCLUDE";
        let mut keys = if exclude {
            vec!["id", "target", "unitIds", "category", "reason"]
        } else {
            vec!["id", "target", "unitIds", "kind", "parts", "layout"]
        };
        let required = keys.clone();
        if e["target"] == "BODY" {
            match st(&e["kind"]) {
                "DIALOGUE" => keys.extend(["speaker", "channel", "channelSource", "cue"]),
                "DIALOGUE_GROUP" => keys.push("dialogues"),
                "BODY_GROUP" => keys.push("children"),
                _ => {}
            }
        }
        shape(e, &keys, &required, &path)?;
        nonempty(&e["id"], &format!("{path}.id"))?;
        need(
            ids.insert(st(&e["id"])),
            "DUPLICATE_ID",
            &format!("{path}.id"),
        )?;
        let refs = e["unitIds"]
            .as_array()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| err("INVALID_UNIT_IDS", &format!("{path}.unitIds")))?;
        let mut owned = HashSet::new();
        for uid in refs {
            if !units.contains_key(st(uid)) {
                let mut er = err("UNKNOWN_REFERENCE", &format!("{path}.unitIds"));
                er.source_unit_id = uid.as_str().map(str::to_string);
                return Err(er);
            }
            if unit_list.get(next).map(|u| &u["id"]) != Some(uid) {
                let mut er = err("UNIT_ORDER_OR_DUPLICATION", &format!("{path}.unitIds"));
                er.source_unit_id = uid.as_str().map(str::to_string);
                return Err(er);
            }
            owned.insert(st(uid).to_string());
            next += 1;
        }
        let output_path = if exclude {
            format!("exclusions[{}]", exclusions.len())
        } else if e["target"] == "ANNOTATION" {
            format!("annotations[{}]", annotations.len())
        } else {
            format!("script.entries[{}]", script.len())
        };
        for uid in refs {
            assignments.push(json!({"unitId":uid,"target":e["target"],"targetId":e["id"],"fieldPath":output_path}));
        }
        if exclude {
            one_of(
                &e["category"],
                &["BLANK", "SOURCE_METADATA"],
                &format!("{path}.category"),
            )?;
            nonempty(&e["reason"], &format!("{path}.reason"))?;
            let t = refs
                .iter()
                .map(|id| st(&units[st(id)]["text"]))
                .collect::<String>();
            need(
                e["category"] != "BLANK" || trim(&t).is_empty(),
                "NONBLANK_EXCLUSION",
                &path,
            )?;
            exclusions.push(json!({"id":e["id"],"unitIds":refs,"category":e["category"],"reason":e["reason"],"text":t}));
            continue;
        }
        one_of(
            &e["kind"],
            match st(&e["target"]) {
                "BODY" => &[
                    "DIALOGUE",
                    "DIALOGUE_GROUP",
                    "BODY_GROUP",
                    "ACTION",
                    "SFX",
                    "ONSCREEN_TEXT",
                ],
                "STRUCTURE" => &["EPISODE_HEADING", "SCENE_HEADING", "SCENE_METADATA"],
                _ => &[
                    "CONTINUITY",
                    "EMOTION_CLUE",
                    "RELATIONSHIP_CONTEXT",
                    "KNOWLEDGE_REVEAL",
                    "SPACE_CONSTRAINT",
                    "CREATIVE_GUIDANCE",
                ],
            },
            &format!("{path}.kind"),
        )?;
        let asm = Assembler {
            text,
            units: &units,
            owned,
            path: path.clone(),
        };
        let mut consumed = vec![];
        let body = asm.assemble(&e["parts"], "parts", false, &mut consumed, false)?;
        let layout = asm.assemble(&e["layout"], "layout", true, &mut consumed, false)?;
        let mut out = json!({"id":e["id"],"target":e["target"],"kind":e["kind"],"sourceUnitIds":refs,"text":body["text"],"textAssembly":{"parts":body["parts"]},"layout":layout["parts"]});
        let a = first(&body["parts"]);
        let b = last(&body["parts"]);
        let start = number(&units[st(&refs[0])]["start"]).unwrap();
        let end = number(&units[st(refs.last().unwrap())]["end"]).unwrap();
        if e["target"] == "BODY" && e["kind"] == "DIALOGUE" {
            one_of(
                &e["channel"],
                &["SPEECH", "OS", "VO", "NARRATION", "SYSTEM"],
                &format!("{path}.channel"),
            )?;
            out["speaker"] = asm.assemble(&e["speaker"], "speaker", false, &mut consumed, false)?;
            need(
                !trim(st(&out["speaker"]["text"])).is_empty(),
                "EMPTY_SPEAKER",
                &format!("{path}.speaker"),
            )?;
            out["channel"] = e["channel"].clone();
            out["channelSource"] = asm.optional(e, "channelSource", "", &mut consumed)?;
            out["cue"] = asm.optional(e, "cue", "", &mut consumed)?;
            validation::compact(text, &out, v5, &path)?;
        }
        if e["target"] == "BODY" && e["kind"] == "BODY_GROUP" {
            let cs = e["children"]
                .as_array()
                .filter(|c| {
                    c.len() >= 2
                        && c[0]["kind"] == "DIALOGUE"
                        && c[1..].iter().all(|c| c["kind"] == "ONSCREEN_TEXT")
                })
                .ok_or_else(|| err("BODY_GROUP_SHAPE", &path))?;
            need(
                a == start && b == end && layout["parts"].as_array().unwrap().is_empty(),
                "BODY_GROUP_SHAPE",
                &path,
            )?;
            let mut spans = vec![];
            let mut children = vec![];
            let mut captions = vec![];
            for (i, c) in cs.iter().enumerate() {
                let field = format!("children[{i}]");
                let cp = format!("{path}.{field}");
                let dialogue = c["kind"] == "DIALOGUE";
                let ks = if dialogue {
                    vec![
                        "kind",
                        "parts",
                        "layout",
                        "speaker",
                        "channel",
                        "channelSource",
                        "cue",
                    ]
                } else {
                    vec!["kind", "parts", "layout", "label"]
                };
                let rq = if dialogue {
                    vec!["kind", "parts", "layout", "speaker", "channel"]
                } else {
                    ks.clone()
                };
                shape(c, &ks, &rq, &cp)?;
                let mut own = vec![];
                let bd = asm.assemble(
                    &c["parts"],
                    &format!("{field}.parts"),
                    false,
                    &mut own,
                    dialogue,
                )?;
                let ly = asm.assemble(
                    &c["layout"],
                    &format!("{field}.layout"),
                    true,
                    &mut own,
                    false,
                )?;
                need(!trim(st(&bd["text"])).is_empty(), "BODY_GROUP_EMPTY", &cp)?;
                let mut oc = json!({"kind":c["kind"],"text":bd["text"],"textAssembly":{"parts":bd["parts"]},"layout":ly["parts"]});
                if dialogue {
                    one_of(
                        &c["channel"],
                        &["SPEECH", "OS", "VO", "NARRATION", "SYSTEM"],
                        &format!("{cp}.channel"),
                    )?;
                    oc["speaker"] = asm.assemble(
                        &c["speaker"],
                        &format!("{field}.speaker"),
                        false,
                        &mut own,
                        false,
                    )?;
                    oc["cue"] = asm.optional(c, "cue", &format!("{field}."), &mut own)?;
                    oc["channelSource"] =
                        asm.optional(c, "channelSource", &format!("{field}."), &mut own)?;
                    oc["channel"] = c["channel"].clone();
                    validation::compact(text, &oc, v5, &cp)?;
                } else {
                    oc["label"] = asm.assemble(
                        &c["label"],
                        &format!("{field}.label"),
                        false,
                        &mut own,
                        false,
                    )?;
                    own.sort_by_key(|s| s.start);
                    let ca = own.first().unwrap().start;
                    let cb = own.last().unwrap().end;
                    coverage(&mut own, ca, cb, "BODY_GROUP_CAPTION_COVERAGE", &cp)?;
                    captions.push((ca, cb, oc["label"].clone(), bd));
                }
                spans.extend(own);
                children.push(oc);
            }
            coverage(&mut spans, start, end, "BODY_GROUP_COVERAGE", &path)?;
            let d = &children[0];
            let sa = first(&d["textAssembly"]["parts"]).min(captions[0].0);
            let sb = last(&d["textAssembly"]["parts"]).max(captions.last().unwrap().1);
            let raw = slice(text, sa, sb)?;
            let scan = explicit_subtitles(&raw);
            let markers = scan["spans"].as_array().unwrap();
            need(
                scan["invalid"] == false
                    && markers.len() == captions.len()
                    && trim(&slice(text, sb, end)?).is_empty(),
                "BODY_GROUP_CAPTION",
                &path,
            )?;
            let mut spoken = String::new();
            let mut cursor = 0;
            for (i, m) in markers.iter().enumerate() {
                let (ca, cb, label, bd) = &captions[i];
                let nm = |k| number(&m[k]).unwrap();
                need(
                    *ca == sa + nm("start")
                        && *cb == sa + nm("end")
                        && first(&label["parts"]) == sa + nm("labelStart")
                        && last(&label["parts"]) == sa + nm("labelEnd")
                        && first(&bd["parts"]) == sa + nm("bodyStart")
                        && last(&bd["parts"]) == sa + nm("bodyEnd"),
                    "BODY_GROUP_CAPTION",
                    &path,
                )?;
                spoken.push_str(&slice(&raw, cursor, nm("start"))?);
                cursor = nm("end");
            }
            spoken.push_str(&slice(&raw, cursor, utf16_len(&raw))?);
            need(d["text"] == spoken, "BODY_GROUP_SPOKEN_TEXT", &path)?;
            let mut temp = d.clone();
            temp["textAssembly"]["parts"] = json!([{ "start":sa }]);
            if validation::post(text, &temp) {
                validation::post_header(text, start, sa, d, &path)?;
            } else {
                validation::normal_header(text, start, sa, d, v5, true, &path)?;
            }
            validation::utterance(d, &path)?;
            out["children"] = json!(children);
        }
        if e["target"] == "BODY" && e["kind"] == "DIALOGUE_GROUP" {
            let cs = e["dialogues"]
                .as_array()
                .filter(|c| c.len() >= 2)
                .ok_or_else(|| err("DIALOGUE_GROUP_BOUNDARY", &path))?;
            need(
                refs.len() == 1
                    && a == start
                    && b == end
                    && layout["parts"].as_array().unwrap().is_empty(),
                "DIALOGUE_GROUP_BOUNDARY",
                &path,
            )?;
            let mut edge = start;
            let mut children = vec![];
            for (i, c) in cs.iter().enumerate() {
                let field = format!("dialogues[{i}]");
                let cp = format!("{path}.{field}");
                shape(
                    c,
                    &[
                        "parts",
                        "layout",
                        "speaker",
                        "channel",
                        "channelSource",
                        "cue",
                    ],
                    &["parts", "layout", "speaker", "channel"],
                    &cp,
                )?;
                one_of(
                    &c["channel"],
                    &["SPEECH", "OS", "VO", "NARRATION", "SYSTEM"],
                    &format!("{cp}.channel"),
                )?;
                let mut own = vec![];
                let bd = asm.assemble(
                    &c["parts"],
                    &format!("{field}.parts"),
                    false,
                    &mut own,
                    false,
                )?;
                let speaker = asm.assemble(
                    &c["speaker"],
                    &format!("{field}.speaker"),
                    false,
                    &mut own,
                    false,
                )?;
                let cue = asm.optional(c, "cue", &format!("{field}."), &mut own)?;
                let ch = asm.optional(c, "channelSource", &format!("{field}."), &mut own)?;
                let ly = asm.assemble(
                    &c["layout"],
                    &format!("{field}.layout"),
                    true,
                    &mut own,
                    false,
                )?;
                let child = json!({"text":bd["text"],"textAssembly":{"parts":bd["parts"]},"layout":ly["parts"],"speaker":speaker,"channel":c["channel"],"channelSource":ch,"cue":cue});
                validation::compact(text, &child, v5, &cp)?;
                own.sort_by_key(|s| s.start);
                let ca = edge;
                for span in &own {
                    need(span.start == edge, "DIALOGUE_GROUP_COVERAGE", &span.path)?;
                    edge = span.end;
                }
                let pa = first(&bd["parts"]);
                let pb = last(&bd["parts"]);
                need(
                    !trim(st(&bd["text"])).is_empty()
                        && re(r"^[\t \r\n]*$").is_match(&slice(text, pb, edge)?),
                    "DIALOGUE_GROUP_TEXT_BOUNDARY",
                    &cp,
                )?;
                if validation::post(text, &child) {
                    validation::post_header(text, ca, pa, &child, &cp)?;
                } else {
                    validation::normal_header(text, ca, pa, &child, v5, false, &cp)?;
                }
                if i > 0 {
                    need(
                        re(r#"(?:[。.!！？?;；,，…][”’」』"']?|[ \t])$"#).is_match(&slice(
                            text,
                            start,
                            first(&speaker["parts"]),
                        )?),
                        "DIALOGUE_GROUP_BOUNDARY",
                        &cp,
                    )?;
                }
                validation::utterance(&child, &cp)?;
                children.push(child);
            }
            need(edge == end, "DIALOGUE_GROUP_COVERAGE", &path)?;
            out["dialogues"] = json!(children);
        }
        need(
            re(r"^[\t \r\n]*$").is_match(&slice(text, b, end)?),
            "NON_TRAILING_LAYOUT",
            &format!("{path}.layout"),
        )?;
        if out.get("speaker").is_some() {
            if validation::post(text, &out) {
                validation::post_header(text, start, a, &out, &path)?;
                validation::utterance(&out, &path)?;
            } else {
                let mut fields: Vec<_> = [&out["speaker"], &out["cue"], &out["channelSource"]]
                    .into_iter()
                    .filter(|v| !v.is_null())
                    .collect();
                fields.sort_by_key(|v| first(&v["parts"]));
                let mut edge = start;
                for f in fields {
                    let fa = first(&f["parts"]);
                    need(
                        fa >= edge
                            && re(if edge == start {
                                r"^[\s\x{feff}()（）【】\[\]]*$"
                            } else {
                                r"^[\s\x{feff}()（）【】\[\],，]*$"
                            })
                            .is_match(&slice(text, edge, fa)?),
                        "DIALOGUE_PREFIX_BOUNDARY",
                        &format!("{path}.layout"),
                    )?;
                    edge = last(&f["parts"]);
                }
                need(
                    edge <= a
                        && re(r"^[\t ()（）【】\[\]]*(?:[:：][\t \r\n]*|[\r\n][\t \r\n]*)$")
                            .is_match(&slice(text, edge, a)?),
                    "DIALOGUE_TEXT_BOUNDARY",
                    &format!("{path}.layout"),
                )?;
            }
        } else {
            need(
                re(if e["target"] == "BODY" && e["kind"] == "ACTION" {
                    r"^[\s\x{feff}]*(?:[△▲][\t ]*)?$"
                } else {
                    r"^[\s\x{feff}]*$"
                })
                .is_match(&slice(text, start, a)?),
                "NON_PREFIX_LAYOUT",
                &format!("{path}.layout"),
            )?;
        }
        consumed.sort_by_key(|s| s.start);
        let mut cursor = start;
        for s in &consumed {
            if s.start != cursor {
                let mut er = err("FIELD_COVERAGE_OR_OVERLAP", &s.path);
                er.source_unit_id = Some(s.unit.clone());
                return Err(er);
            }
            cursor = s.end;
        }
        need(cursor == end, "FIELD_COVERAGE", &path)?;
        if performance {
            let add = |v: &mut Value| -> IntakeResult<()> {
                if let Some(p) = crate::series_lite_bundle_standard::inline_performance_projection(
                    text,
                    &v["textAssembly"]["parts"],
                    perfversion,
                )? {
                    v["performance"] = p;
                }
                Ok(())
            };
            if out["kind"] == "DIALOGUE" {
                add(&mut out)?;
            } else if let Some(ds) = out.get_mut("dialogues").and_then(Value::as_array_mut) {
                for d in ds {
                    add(d)?;
                }
            } else if let Some(cs) = out.get_mut("children").and_then(Value::as_array_mut) {
                for c in cs.iter_mut().filter(|c| c["kind"] == "DIALOGUE") {
                    add(c)?;
                }
            }
        }
        rendered.push(reading(&out));
        if e["target"] == "ANNOTATION" {
            annotations.push(out);
        } else {
            script.push(out);
        }
    }
    if next != unit_list.len() {
        let mut e = err("UNIT_COVERAGE", "plan.entries");
        e.source_unit_id = unit_list
            .get(next)
            .and_then(|u| u["id"].as_str())
            .map(str::to_string);
        return Err(e);
    }
    let reading_text = format!("【标准稿基础切片·语义未验收】\n\n{}", rendered.join("\n\n"));
    let mut binding = json!({"sourceId":index["sourceId"],"sourceVersion":index["sourceVersion"],"indexerVersion":index["indexerVersion"],"sourceIndexDigest":hash(index)?});
    let plan_digest = hash(plan)?;
    let mut full_binding = binding.clone();
    full_binding["planDigest"] = json!(plan_digest);
    let v = if v5 {
        5
    } else if performance {
        4
    } else if mixed {
        3
    } else if grouped || post {
        2
    } else {
        1
    };
    let mut payload = json!({"schemaVersion":format!("DIRECTOR_STANDARD_SCRIPT_SLICE_V{v}"),"assemblerVersion":format!("standard-script-compiler-v{v}"),"binding":full_binding,"script":{"entries":script},"annotations":annotations,"exclusions":exclusions,"evidence":unit_list.iter().map(|u|{let mut u=u.clone();u["sourceVersion"]=index["sourceVersion"].clone();u["offsetUnit"]=json!("UTF16");u}).collect::<Vec<_>>(),"fidelity":{"assignments":assignments},"readiness":{"mechanical":"VERIFIED","semantic":"NOT_RUN","acceptedForDispatch":false,"blockers":["SEMANTIC_REVIEW_NOT_RUN","PRODUCTION_INTEGRATION_NOT_IMPLEMENTED"]}});
    let digest = hash(&payload)?;
    let mut checks = json!({"unitCoverage":"VERIFIED","sourceOrder":"VERIFIED","sourceSlices":"VERIFIED","fieldBoundaries":"VERIFIED","exclusionShape":"VERIFIED","noTextPatches":"VERIFIED"});
    for (yes, k) in [
        (grouped, "dialogueGroups"),
        (post, "postColonCues"),
        (mixed, "mixedBodyGroups"),
        (performance, "inlinePerformance"),
    ] {
        if yes {
            checks[k] = json!("VERIFIED");
        }
    }
    binding["schemaVersion"] = json!(format!("STANDARD_SCRIPT_MECHANICAL_RECEIPT_V{v}"));
    binding["status"] = json!("MECHANICALLY_VERIFIED");
    binding["planDigest"] = json!(plan_digest);
    binding["packageDigest"] = json!(digest);
    binding["readingTextDigest"] = json!(hash(&json!(reading_text))?);
    binding["assemblerVersion"] = payload["assemblerVersion"].clone();
    binding["checks"] = checks;
    binding["semantic"] = json!("NOT_RUN");
    binding["acceptedForDispatch"] = json!(false);
    payload["packageId"] = json!(format!("standard:{digest}"));
    payload["fidelity"]["receipt"] = binding.clone();
    Ok(json!({"package":payload,"readingText":reading_text,"receipt":binding}))
}
