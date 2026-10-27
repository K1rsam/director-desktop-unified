use super::headers::*;
use super::*;
struct Node<'a> {
    source: &'a str,
    entry: &'a Value,
    by_id: &'a BTreeMap<String, Value>,
    start: usize,
    version: usize,
}
impl Node<'_> {
    fn parts(
        &self,
        raw: &Value,
        expected: &Value,
        label: &str,
        empty: bool,
        sink: &mut Vec<(usize, usize)>,
        gaps: bool,
    ) -> Result<Value> {
        let raw = arr(raw, "STANDARD_PARTS")?;
        need(empty || !raw.is_empty(), "STANDARD_PARTS")?;
        let layout = label.ends_with("版式");
        let mut previous = None;
        let mut out = vec![];
        let mut text = String::new();
        for p in raw {
            let unit = self.by_id.get(st(&p["unitId"])).ok_or("STANDARD_PARTS")?;
            let a = n(&p["start"]).ok_or("STANDARD_PARTS")?;
            let b = n(&p["end"]).ok_or("STANDARD_PARTS")?;
            need(
                self.entry["sourceUnitIds"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&p["unitId"]))
                    && a >= n(&unit["start"]).unwrap()
                    && b <= n(&unit["end"]).unwrap()
                    && b > a,
                "STANDARD_PARTS",
            )?;
            let value = slice(self.source, a, b)?;
            need(
                matches!(st(&p["kind"]), "SOURCE_SLICE" | "SOURCE_SEPARATOR")
                    && (p["kind"] != "SOURCE_SEPARATOR" || re(r"^[\t \r\n]+$").is_match(&value)),
                "STANDARD_PARTS",
            )?;
            need(
                previous.is_none_or(|edge| a >= edge && (layout || gaps || a == edge)),
                "STANDARD_PARTS",
            )?;
            previous = Some(b);
            if layout {
                need(
                    re(r"^[\s\x{feff}:：()（）△▲【】\[\],，]+$").is_match(&value),
                    "STANDARD_LAYOUT",
                )?;
            } // slice proves scalar boundaries for all versions.
            text.push_str(&value);
            sink.push((a, b));
            out.push(json!([a - self.start, b - self.start]));
        }
        need(expected.as_str() == Some(&text), "STANDARD_PARTS")?;
        Ok(json!(out))
    }
    fn layout(&self, v: &Value, sink: &mut Vec<(usize, usize)>) -> Result<()> {
        let text = copy_parts(self.source, v)?;
        self.parts(v, &json!(text), "版式", true, sink, false)?;
        Ok(())
    }
    fn field(&self, child: &Value, name: &str, sink: &mut Vec<(usize, usize)>) -> Result<Value> {
        let f = &child[name];
        if f.is_null() {
            return Ok(json!([]));
        }
        self.parts(&f["parts"], &f["text"], name, false, sink, false)
    }
    fn performance(&self, child: &Value, counts: &mut [usize; 2]) -> Result<Option<Value>> {
        if self.version == 5
            && compact_performance_head_projection(
                self.source,
                &child["speaker"]["parts"],
                &child["cue"]["parts"],
                &child["channelSource"]["parts"],
            )?
            .is_some()
        {
            counts[1] += 1;
        }
        let expected = if self.version >= 4 {
            inline_performance_projection(
                self.source,
                &child["textAssembly"]["parts"],
                if self.version == 5 {
                    "inline-performance-v2"
                } else {
                    "inline-performance-v1"
                },
            )?
        } else {
            None
        };
        need(
            match &expected {
                Some(v) => child.get("performance").is_some_and(|p| equal(v, p)),
                None => child.get("performance").is_none(),
            },
            "STANDARD_PERFORMANCE_BINDING",
        )?;
        let Some(e) = expected else { return Ok(None) };
        counts[0] += 1;
        let local = |v: &Value| -> Result<Value> {
            Ok(json!(arr(v, "STANDARD_PERFORMANCE_BINDING")?
                .iter()
                .map(|p| Ok(json!([
                    n(&p["start"])
                        .ok_or("STANDARD_PERFORMANCE_BINDING")?
                        .checked_sub(self.start)
                        .ok_or("STANDARD_PERFORMANCE_BINDING")?,
                    n(&p["end"])
                        .ok_or("STANDARD_PERFORMANCE_BINDING")?
                        .checked_sub(self.start)
                        .ok_or("STANDARD_PERFORMANCE_BINDING")?
                ])))
                .collect::<Result<Vec<_>>>()?))
        };
        let cues=arr(&e["cues"],"STANDARD_PERFORMANCE_BINDING")?.iter().map(|c|Ok(json!({"text":c["text"],"parts":local(&c["parts"] )?,"markerParts":local(&c["markerParts"] )?,"quoteOffset":c["spokenOffset"]}))).collect::<Result<Vec<_>>>()?;
        Ok(Some(
            json!({"spoken":local(&e["spoken"]["parts"] )?,"cues":cues}),
        ))
    }
    fn dialogue(
        &self,
        child: &Value,
        sink: &mut Vec<(usize, usize)>,
        gaps: bool,
        counts: &mut [usize; 2],
    ) -> Result<Value> {
        need(voice(st(&child["channel"])).is_some(), "STANDARD_CHANNEL")?;
        let body = self.parts(
            &child["textAssembly"]["parts"],
            &child["text"],
            "正文",
            false,
            sink,
            gaps,
        )?;
        let speaker = self.parts(
            &child["speaker"]["parts"],
            &child["speaker"]["text"],
            "说者",
            false,
            sink,
            false,
        )?;
        let cue = self.field(child, "cue", sink)?;
        let channel = self.field(child, "channelSource", sink)?;
        compact_voice(self.source, child, self.version == 5)?;
        let mut result = json!({"parts":body,"speaker":speaker,"cue":cue,"channelSource":channel,"channel":voice(st(&child["channel"])).unwrap()});
        if let Some(p) = self.performance(child, counts)? {
            result["performance"] = p;
        }
        Ok(result)
    }
}
fn coverage(spans: &mut [(usize, usize)], start: usize, end: usize, code: &str) -> Result<()> {
    spans.sort_by_key(|x| x.0);
    let mut edge = start;
    for (a, b) in spans {
        need(*a == edge, code)?;
        edge = *b;
    }
    need(edge == end, code)
}
fn shape(v: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    let o = v.as_object().ok_or("STANDARD_BODY_SHAPE")?;
    need(
        o.keys()
            .all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str()))
            && required.iter().all(|k| o.contains_key(*k)),
        "STANDARD_BODY_SHAPE",
    )
}
fn child_shape(c: &Value, performance: bool) -> Result<()> {
    let d = c["kind"] == "DIALOGUE";
    shape(
        c,
        if d {
            &[
                "kind",
                "text",
                "textAssembly",
                "layout",
                "speaker",
                "channel",
                "channelSource",
                "cue",
            ]
        } else {
            &["kind", "text", "textAssembly", "layout", "label"]
        },
        if d && performance {
            &["performance"]
        } else {
            &[]
        },
    )?;
    let parts = |v: &Value| -> Result<()> {
        for p in arr(v, "STANDARD_BODY_SHAPE")? {
            shape(p, &["kind", "unitId", "start", "end"], &[])?;
        }
        Ok(())
    };
    let mapped = |v: &Value| -> Result<()> {
        shape(v, &["text", "parts"], &[])?;
        parts(&v["parts"])
    };
    shape(&c["textAssembly"], &["parts"], &[])?;
    parts(&c["textAssembly"]["parts"])?;
    parts(&c["layout"])?;
    if d {
        mapped(&c["speaker"])?;
        for k in ["cue", "channelSource"] {
            if !c[k].is_null() {
                mapped(&c[k])?;
            }
        }
    } else {
        mapped(&c["label"])?;
    }
    Ok(())
}
/// Input is exactly the frozen compiler package and independently supplied source bytes.
pub fn standard_package_source(pkg: &Value, options: &Value) -> Result<Value> {
    let source = options["sourceText"]
        .as_str()
        .ok_or("STANDARD_SOURCE_BINDING")?;
    let key = st(&options["key"]);
    let version = match st(&pkg["schemaVersion"]) {
        "DIRECTOR_STANDARD_SCRIPT_SLICE_V1" => 1,
        "DIRECTOR_STANDARD_SCRIPT_SLICE_V2" => 2,
        "DIRECTOR_STANDARD_SCRIPT_SLICE_V3" => 3,
        "DIRECTOR_STANDARD_SCRIPT_SLICE_V4" => 4,
        "DIRECTOR_STANDARD_SCRIPT_SLICE_V5" => 5,
        _ => return Err("STANDARD_PACKAGE: unsupported schema".into()),
    };
    let receipt = &pkg["fidelity"]["receipt"];
    need(
        receipt["status"] == "MECHANICALLY_VERIFIED",
        "STANDARD_PACKAGE",
    )?;
    let mut with = pkg.clone();
    with.as_object_mut()
        .ok_or("STANDARD_PACKAGE")?
        .remove("packageId");
    with["fidelity"]
        .as_object_mut()
        .ok_or("STANDARD_PACKAGE")?
        .remove("receipt");
    let digest = standard_fingerprint(&with)?;
    let bind = &pkg["binding"];
    need(
        pkg["packageId"] == format!("standard:{digest}")
            && receipt["packageDigest"] == digest
            && receipt["sourceVersion"] == bind["sourceVersion"]
            && receipt["planDigest"] == bind["planDigest"],
        "STANDARD_RECEIPT",
    )?;
    need(
        bind["sourceVersion"]
            == format!(
                "src:{}",
                standard_fingerprint(&json!([bind["sourceId"], source]))?
            ),
        "STANDARD_SOURCE_BINDING",
    )?;
    need(
        receipt["schemaVersion"] == format!("STANDARD_SCRIPT_MECHANICAL_RECEIPT_V{version}")
            && [
                "sourceId",
                "sourceVersion",
                "sourceIndexDigest",
                "indexerVersion",
                "planDigest",
            ]
            .iter()
            .all(|k| receipt[*k] == bind[*k])
            && receipt["assemblerVersion"] == pkg["assemblerVersion"]
            && pkg["assemblerVersion"] == format!("standard-script-compiler-v{version}")
            && bind["indexerVersion"] == "physical-lines-v1",
        "STANDARD_RECEIPT",
    )?;
    let entries = pkg["script"]["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let grouped = entries.iter().any(|e| e["kind"] == "DIALOGUE_GROUP");
    let mixed = entries.iter().any(|e| e["kind"] == "BODY_GROUP");
    let post = entries.iter().any(|e| {
        if e["kind"] == "DIALOGUE_GROUP" {
            e["dialogues"]
                .as_array()
                .is_some_and(|cs| cs.iter().any(|c| post_fields(source, c)))
        } else {
            e["kind"] == "DIALOGUE" && post_fields(source, e)
        }
    });
    need(
        (version >= 4
            || (mixed == (version == 3)
                && (version == 3 || ((grouped || post) == (version == 2)))))
            && (!post || receipt["checks"]["postColonCues"] == "VERIFIED")
            && (!mixed || receipt["checks"]["mixedBodyGroups"] == "VERIFIED")
            && (version < 4 || receipt["checks"]["inlinePerformance"] == "VERIFIED"),
        "STANDARD_RECEIPT",
    )?;
    let mut physical = vec![];
    for m in re(r"[^\r\n]*(?:\r\n|\r|\n|$)")
        .find_iter(source)
        .filter(|m| !m.as_str().is_empty())
    {
        physical.push(json!({"id":format!("{}:u{}",st(&bind["sourceVersion"]),physical.len()+1),"start":len(&source[..m.start()]),"end":len(&source[..m.end()]),"text":m.as_str()}));
    }
    let index = json!({"schemaVersion":"STANDARD_SCRIPT_SOURCE_INDEX_V1","indexerVersion":"physical-lines-v1","sourceId":bind["sourceId"],"sourceVersion":bind["sourceVersion"],"offsetUnit":"UTF16","text":source,"units":physical});
    need(
        standard_fingerprint(&index)? == bind["sourceIndexDigest"],
        "STANDARD_RECEIPT",
    )?;
    let units = arr(&pkg["evidence"], "STANDARD_EVIDENCE")?;
    need(
        !units.is_empty() && units.len() == physical.len(),
        "STANDARD_EVIDENCE",
    )?;
    let mut by_id = BTreeMap::new();
    let mut cursor = 0;
    for (i, u) in units.iter().enumerate() {
        need(
            ["id", "start", "end", "text"]
                .iter()
                .all(|k| u[*k] == physical[i][*k])
                && u["sourceVersion"] == bind["sourceVersion"]
                && u["offsetUnit"] == "UTF16",
            "STANDARD_EVIDENCE",
        )?;
        let id = u["id"].as_str().ok_or("STANDARD_EVIDENCE")?;
        let end = n(&u["end"]).ok_or("STANDARD_EVIDENCE")?;
        need(
            !by_id.contains_key(id)
                && n(&u["start"]) == Some(cursor)
                && end > cursor
                && u["text"].is_string()
                && end - cursor == len(st(&u["text"])),
            "STANDARD_EVIDENCE",
        )?;
        let mut u = u.clone();
        u["line"] = json!(i + 1);
        by_id.insert(id.to_string(), u);
        cursor = end;
    }
    need(
        units.iter().map(|u| st(&u["text"])).collect::<String>() == source,
        "STANDARD_SOURCE_BINDING",
    )?;
    let mut all = entries;
    all.extend(pkg["annotations"].as_array().cloned().unwrap_or_default());
    for e in pkg["exclusions"].as_array().cloned().unwrap_or_default() {
        let mut e = e.clone();
        e["target"] = json!("EXCLUDE");
        e["sourceUnitIds"] = e["unitIds"].clone();
        all.push(e);
    }
    all.sort_by_key(|e| {
        e["sourceUnitIds"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|id| by_id.get(st(id)))
            .and_then(|u| n(&u["start"]))
            .map(|n| n as i64)
            .unwrap_or(-1)
    });
    let mut owned = HashSet::new();
    let mut ids = HashSet::new();
    let mut rows = vec![];
    let mut notes = vec![];
    let mut annotations = vec![];
    let mut counts = [0, 0];
    for e in &all {
        let eid = e["id"].as_str().ok_or("STANDARD_ENTRY")?;
        let uid = arr(&e["sourceUnitIds"], "STANDARD_ENTRY")?;
        need(ids.insert(eid) && !uid.is_empty(), "STANDARD_ENTRY")?;
        let selected = uid
            .iter()
            .map(|id| {
                by_id
                    .get(st(id))
                    .ok_or_else(|| "STANDARD_ENTRY: unknown source unit".to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        for (i, u) in selected.iter().enumerate() {
            need(
                owned.insert(st(&u["id"])) && (i == 0 || selected[i - 1]["end"] == u["start"]),
                "STANDARD_ENTRY",
            )?;
        }
        let start = n(&selected[0]["start"]).unwrap();
        let end = n(&selected.last().unwrap()["end"]).unwrap();
        let text = slice(source, start, end)?;
        let id = format!("{key}:{eid}");
        if e["target"] == "EXCLUDE" {
            need(
                e["category"] == "BLANK" && trim(&text).is_empty(),
                "STANDARD_EXCLUSION",
            )?;
            rows.push(json!({"id":id,"line":selected[0]["line"],"text":text,"standard":{"target":"EXCLUDE","kind":"BLANK"}}));
            continue;
        }
        need(
            matches!(st(&e["target"]), "BODY" | "STRUCTURE" | "ANNOTATION"),
            "STANDARD_ENTRY",
        )?;
        let node = Node {
            source,
            entry: e,
            by_id: &by_id,
            start,
            version,
        };
        let mut spans = vec![];
        let body = node.parts(
            &e["textAssembly"]["parts"],
            &e["text"],
            "正文",
            false,
            &mut spans,
            false,
        )?;
        let layout = e.get("layout").cloned().unwrap_or(json!([]));
        node.layout(&layout, &mut spans)?;
        let mut standard = json!({"target":e["target"],"kind":e["kind"],"parts":body});
        if e["kind"] == "DIALOGUE" {
            need(
                e["target"] == "BODY" && voice(st(&e["channel"])).is_some(),
                "STANDARD_CHANNEL",
            )?;
            standard["speaker"] = node.parts(
                &e["speaker"]["parts"],
                &e["speaker"]["text"],
                "说者",
                false,
                &mut spans,
                false,
            )?;
            standard["cue"] = node.field(e, "cue", &mut spans)?;
            standard["channelSource"] = node.field(e, "channelSource", &mut spans)?;
            standard["channel"] = json!(voice(st(&e["channel"])).unwrap());
            compact_voice(source, e, version == 5)?;
            if post_fields(source, e) {
                post_header(
                    source,
                    start,
                    part_start(&e["textAssembly"]["parts"]).ok_or("STANDARD_PARTS")?,
                    e,
                )?;
                group_utterance(
                    st(&e["text"]),
                    st(&e["speaker"]["text"]),
                    st(&e["cue"]["text"]),
                )?;
            }
            if let Some(p) = node.performance(e, &mut counts)? {
                standard["performance"] = p;
            }
        }
        if e["kind"] == "DIALOGUE_GROUP" {
            let children = arr(&e["dialogues"], "STANDARD_GROUP")?;
            need(
                e["target"] == "BODY"
                    && selected.len() == 1
                    && e["text"] == text
                    && layout.as_array().is_some_and(Vec::is_empty)
                    && children.len() >= 2,
                "STANDARD_GROUP",
            )?;
            let mut edge = start;
            let mut result = vec![];
            for (i, c) in children.iter().enumerate() {
                let allowed = [
                    "text",
                    "textAssembly",
                    "layout",
                    "speaker",
                    "channel",
                    "channelSource",
                    "cue",
                ];
                need(
                    c.as_object().is_some_and(|o| {
                        o.keys().all(|k| {
                            allowed.contains(&k.as_str()) || (version >= 4 && k == "performance")
                        })
                    }),
                    "STANDARD_GROUP",
                )?;
                let mut own = vec![];
                let r = node.dialogue(c, &mut own, false, &mut counts)?;
                need(c["layout"].is_array(), "STANDARD_GROUP")?;
                node.layout(&c["layout"], &mut own)?;
                let child_start = edge;
                own.sort_by_key(|x| x.0);
                for (a, b) in &own {
                    need(*a == edge, "STANDARD_GROUP_COVERAGE")?;
                    edge = *b;
                }
                let a = part_start(&c["textAssembly"]["parts"]).ok_or("STANDARD_GROUP")?;
                let b = part_end(&c["textAssembly"]["parts"]).ok_or("STANDARD_GROUP")?;
                need(
                    !trim(st(&c["text"])).is_empty()
                        && re(r"^[\t \r\n]*$").is_match(&slice(source, b, edge)?),
                    "STANDARD_GROUP_BOUNDARY",
                )?;
                if post_fields(source, c) {
                    post_header(source, child_start, a, c)?;
                } else {
                    normal_header(source, child_start, a, c, version == 5, false)?;
                }
                if i > 0 {
                    let prefix = slice(
                        source,
                        start,
                        part_start(&c["speaker"]["parts"]).ok_or("STANDARD_GROUP")?,
                    )?;
                    need(
                        re(r#"(?:[。.!！？?;；,，…][”’」』"']?|[ \t])$"#).is_match(&prefix),
                        "STANDARD_GROUP_BOUNDARY",
                    )?;
                }
                group_utterance(
                    st(&c["text"]),
                    st(&c["speaker"]["text"]),
                    st(&c["cue"]["text"]),
                )?;
                result.push(r);
            }
            need(edge == end, "STANDARD_GROUP_COVERAGE")?;
            standard["dialogues"] = json!(result);
        }
        if e["kind"] == "BODY_GROUP" {
            let children = arr(&e["children"], "STANDARD_BODY_GROUP")?;
            need(
                version >= 3
                    && e["target"] == "BODY"
                    && e["text"] == text
                    && layout.as_array().is_some_and(Vec::is_empty)
                    && children.len() >= 2
                    && children[0]["kind"] == "DIALOGUE"
                    && children[1..].iter().all(|c| c["kind"] == "ONSCREEN_TEXT"),
                "STANDARD_BODY_GROUP",
            )?;
            let mut all_spans = vec![];
            let mut captions = vec![];
            let mut result = vec![];
            for (i, c) in children.iter().enumerate() {
                child_shape(c, version >= 4)?;
                let mut own = vec![];
                need(!trim(st(&c["text"])).is_empty(), "STANDARD_BODY_GROUP")?;
                let r = if i == 0 {
                    let mut r = node.dialogue(c, &mut own, true, &mut counts)?;
                    node.layout(&c["layout"], &mut own)?;
                    r["kind"] = json!("DIALOGUE");
                    r
                } else {
                    let body = node.parts(
                        &c["textAssembly"]["parts"],
                        &c["text"],
                        "混合正文",
                        false,
                        &mut own,
                        false,
                    )?;
                    node.layout(&c["layout"], &mut own)?;
                    let label = node.parts(
                        &c["label"]["parts"],
                        &c["label"]["text"],
                        "字幕标记",
                        false,
                        &mut own,
                        false,
                    )?;
                    own.sort_by_key(|x| x.0);
                    let a = own.first().ok_or("STANDARD_BODY_COVERAGE")?.0;
                    let b = own.last().unwrap().1;
                    coverage(&mut own, a, b, "STANDARD_BODY_COVERAGE")?;
                    captions.push((a, b, c));
                    json!({"kind":"ONSCREEN_TEXT","parts":body,"label":label})
                };
                all_spans.extend(own);
                result.push(r);
            }
            coverage(&mut all_spans, start, end, "STANDARD_BODY_COVERAGE")?;
            let d = &children[0];
            let a = part_start(&d["textAssembly"]["parts"])
                .unwrap()
                .min(captions[0].0);
            let b = part_end(&d["textAssembly"]["parts"])
                .unwrap()
                .max(captions.last().unwrap().1);
            let raw = slice(source, a, b)?;
            let markers = subtitle_spans(&raw)?;
            need(
                markers.len() == captions.len() && trim(&slice(source, b, end)?).is_empty(),
                "STANDARD_BODY_CAPTION",
            )?;
            let mut spoken = String::new();
            let mut cursor = 0;
            for (i, m) in markers.iter().enumerate() {
                let (ca, cb, c) = captions[i];
                need(
                    ca == a + m[0]
                        && cb == a + m[1]
                        && part_start(&c["label"]["parts"]) == Some(a + m[2])
                        && part_end(&c["label"]["parts"]) == Some(a + m[3])
                        && part_start(&c["textAssembly"]["parts"]) == Some(a + m[4])
                        && part_end(&c["textAssembly"]["parts"]) == Some(a + m[5]),
                    "STANDARD_BODY_CAPTION",
                )?;
                spoken.push_str(&slice(&raw, cursor, m[0])?);
                cursor = m[1];
            }
            spoken.push_str(&tail(&raw, cursor));
            need(d["text"] == spoken, "STANDARD_BODY_SPEECH")?;
            let mut h = d.clone();
            let mut first = d["textAssembly"]["parts"][0].clone();
            first["start"] = json!(a);
            h["textAssembly"] = json!({"parts":[first]});
            if post_fields(source, &h) {
                post_header(source, start, a, d)?;
            } else {
                normal_header(source, start, a, d, version == 5, true)?;
            }
            group_utterance(
                st(&d["text"]),
                st(&d["speaker"]["text"]),
                st(&d["cue"]["text"]),
            )?;
            standard["children"] = json!(result);
        }
        coverage(&mut spans, start, end, "STANDARD_COVERAGE")?;
        if e["target"] == "ANNOTATION" {
            notes.push(json!({"id":id,"text":text,"evidenceRole":"guidance"}));
            annotations.push(json!({"id":id,"kind":e["kind"]}));
        } else {
            rows.push(json!({"id":id,"line":selected[0]["line"],"text":text,"standard":standard}));
        }
    }
    need(owned.len() == units.len(), "STANDARD_COVERAGE")?;
    need(
        version < 4 || counts[0] > 0 || version == 5 && counts[1] > 0,
        "STANDARD_PERFORMANCE_BINDING",
    )?;
    need(
        rows.iter()
            .any(|r| r["standard"]["target"] == "BODY" && !trim(st(&r["text"])).is_empty()),
        "SOURCE_EMPTY",
    )?;
    let mut context = options
        .get("context")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or(json!({}));
    if !annotations.is_empty() {
        context["annotations"] = json!(annotations);
        context["annotationRule"] = json!(
            "annotations是有来源的辅助说明，不是新增剧情或已证实事实；不得据此虚构entryFacts。"
        );
    }
    let mut context_rows = options["contextRows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    context_rows.extend(notes);
    let mut out = json!({"key":key,"sourceProtocol":"standard-episode-v1","standardPackage":pkg,"context":context,"contextRows":context_rows,"sourceRows":rows});
    if let Some(title) = options.get("title") {
        out["title"] = title.clone();
    }
    Ok(out)
}
