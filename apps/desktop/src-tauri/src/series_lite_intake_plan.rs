//! Complete deterministic planner from frozen lite 0.1.2 standard-script-plan.mjs.
//! No invented speaker, model inference, fallback rewrite, or source mutation.
use super::*;
#[path = "series_lite_intake_plan_headers.rs"]
mod headers;
use headers::*;
#[cfg(test)]
#[path = "series_lite_intake_plan_tests.rs"]
mod tests;
#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}
#[derive(Clone)]
struct Line {
    number: usize,
    start: usize,
    content: String,
}
#[derive(Clone)]
struct Head {
    speaker: Span,
    channel: String,
    body_start: usize,
    cue: Option<Span>,
    channel_source: Option<Span>,
}
fn contains(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n == name)
}
fn add(names: &mut Vec<String>, name: String) {
    if !contains(names, &name) {
        names.push(name)
    }
}
fn sub(s: &str, a: usize, b: usize) -> String {
    let n = utf16_len(s);
    slice(s, a.min(n), b.min(n).max(a.min(n)))
        .expect("planner source ranges preserve UTF16 boundaries")
}
fn units(index: &Value) -> &[Value] {
    index["units"].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn offset(v: &Value, k: &str) -> usize {
    number(&v[k]).expect("source index offset")
}
struct Planner<'a> {
    text: &'a str,
    index: &'a Value,
    lines: Vec<Line>,
    known: Vec<String>,
    declared: Vec<String>,
    whole: Vec<String>,
    syntax: String,
    head_cues: bool,
}
impl Planner<'_> {
    fn current(&self) -> bool {
        self.syntax == "current"
    }
    fn episode(&self, s: &str) -> bool {
        re(if self.syntax=="legacy"{r"^第[〇零一二三四五六七八九十百千万两\d]+集(?:[ \t:：]|$)"}else{r"^第[〇零一二三四五六七八九十百千万两\d]+集(?:（[^()（）\r\n]+）|\([^()（）\r\n]+\))?(?:[ \t:：]|$)"}).is_match(s)
    }
    fn scene(&self, s: &str) -> bool {
        if self.syntax == "legacy" {
            re(r"^\d+[ \t]*-[ \t]*\d+(?:[ \t]|$)").is_match(s)
        } else {
            re(r"^\d+[ \t]*-[ \t]*\d+(?:[ \t:：]|$)").is_match(s)||re(r"^\d+[ \t]*\.[ \t]*\d+[\t \x{a0}]*(?:日|夜|晨|昏|清晨|傍晚|凌晨|上午|下午|中午|深夜)[\t \x{a0},，、]+(?:内外|内|外)[\t \x{a0},，、]+\S").is_match(s)||re(r"(?i)^(?:场景|SCENE)[\t :：]+[^\r\n:：]+$").is_match(s)
        }
    }
    fn end_marker(&self, l: &Line) -> bool {
        self.current() && re(r"^【全剧终】[ \t]*$").is_match(&l.content)
    }
    fn guidance(&self, s: &str) -> bool {
        re(r"^【(?:镜头切|[一二三四五六七八九十\d]+卡)】[ \t]*$").is_match(s)
            || re(r"^【镜头切[：:]([^【】\r\n]+)】[ \t]*$")
                .captures(s)
                .is_some_and(|c| !trim(&c[1]).is_empty())
    }
    fn screen(&self, s: &str) -> bool {
        re(r"^(?:弹幕|字幕|屏幕文字)(?:（[^()（）\r\n]+）|\([^()（）\r\n]+\))?[\t \x{a0}\x{3000}]*[:：]").is_match(s)
    }
    fn omission(&self, s: &str) -> bool {
        re(r"^(?:\.{3,}|．{3,}|…{2,})[\t ]*$").is_match(s)
    }
    fn typed(&self, s: &str) -> bool {
        re(r"^[【\[]").is_match(s)
    }
    fn missing_speaker(&self, s: &str) -> bool {
        re(r"^(?:[:：]|[（(][^）)]*[）)][ \t]*[:：])").is_match(s)
    }
    fn read(&self, l: &Line, names: Option<&[String]>) -> Option<std::result::Result<Head, Value>> {
        read_head(
            l,
            names.unwrap_or(&self.known),
            &self.syntax,
            self.head_cues,
            &self.whole,
        )
    }
    fn scan(&self, s: &str) -> (Vec<usize>, bool) {
        quoted(s, self.current())
    }
    fn structural(&self, l: &Line) -> bool {
        self.episode(&l.content)
            || self.scene(&l.content)
            || re(META).is_match(&l.content)
            || self.end_marker(l)
            || annotation(&l.content).is_some()
            || self.typed(&l.content)
            || self.current() && (self.screen(&l.content) || self.omission(&l.content))
            || re(r"^[△▲]").is_match(&l.content)
    }
    fn actor_action(&self, l: &Line) -> bool {
        self.current()&&self.known.iter().any(|n|l.content.strip_prefix(n).is_some_and(|tail|re(r#"^(?:(?:震惊|惊讶|惊愕|错愕|惊恐|疑惑|慌忙|微微|轻轻|猛然|猛地|忽然|突然|缓缓|立刻|随即|再次|不由得?|下意识)地?){0,2}(?:抬头|低头|转头|转身|起身|蹲下|抬手|摆手|挥手|握拳|攥拳|攥紧|摊手|耸肩|点头|摇头|闭眼|睁眼|后退|躬身|跪下)[^:：?？!！“”‘’「」『』"'（）()\r\n]*。$"#).is_match(tail)&&!re(r"(?:说道|问道|喊道|答道|开口|表示|声称|告诉|认为|觉得|[吗呢吧啊]。)").is_match(tail)))
    }
    fn parts(&self, r: Span) -> Vec<Value> {
        units(self.index).iter().filter_map(|u|{let a=r.start.max(offset(u,"start"));let b=r.end.min(offset(u,"end"));(a<b).then(||json!({"kind":if re(r"^[\t \r\n]+$").is_match(&sub(self.text,a,b)){"SOURCE_SEPARATOR"}else{"SOURCE_SLICE"},"unitId":u["id"],"start":a,"end":b}))}).collect()
    }
    fn nullable_parts(&self, r: Option<Span>) -> Value {
        r.map(|r| json!(self.parts(r))).unwrap_or(Value::Null)
    }
    fn layout(&self, start: usize, end: usize, mut ranges: Vec<Span>) -> Vec<Value> {
        ranges.sort_by_key(|r| r.start);
        let mut cursor = start;
        let mut layout = vec![];
        for r in ranges {
            if cursor < r.start {
                layout.extend(self.parts(Span {
                    start: cursor,
                    end: r.start,
                }))
            }
            cursor = r.end
        }
        if cursor < end {
            layout.extend(self.parts(Span { start: cursor, end }))
        }
        layout
    }
    fn mapped(&self, first: usize, last: usize, body: Span, mut fields: Vec<Span>) -> Value {
        fields.push(body);
        json!({"id":format!("line-{}",first+1),"unitIds":units(self.index)[first..=last].iter().map(|u|u["id"].clone()).collect::<Vec<_>>(),"parts":self.parts(body),"layout":self.layout(offset(&units(self.index)[first],"start"),offset(&units(self.index)[last],"end"),fields)})
    }
    fn body_end(&self, row: usize) -> usize {
        let u = &units(self.index)[row];
        offset(u, "end") - utf16_len(re(r"[\t \r\n]*$").find(st(&u["text"])).unwrap().as_str())
    }
    fn entry(&self, target: &str, kind: &str, first: usize, last: usize, body: Span) -> Value {
        let mut out = self.mapped(first, last, body, vec![]);
        out["target"] = json!(target);
        out["kind"] = json!(kind);
        out
    }
    fn fields(&self, h: &Head) -> Vec<Span> {
        let mut fields = vec![h.speaker];
        fields.extend(h.cue);
        fields.extend(h.channel_source);
        fields
    }
    fn mixed(
        &self,
        first: usize,
        last: usize,
        start: usize,
        end: usize,
        h: &Head,
    ) -> Option<Value> {
        let scan = explicit_subtitles(&sub(self.text, start, end));
        let spans = scan["spans"].as_array()?;
        if spans.is_empty() || scan["invalid"] == true {
            return None;
        }
        let mut spoken = vec![];
        let mut cursor = start;
        let mut captions = vec![];
        for s in spans {
            let span_start = start + offset(s, "start");
            if cursor < span_start {
                spoken.extend(self.parts(Span {
                    start: cursor,
                    end: span_start,
                }))
            }
            cursor = start + offset(s, "end");
            let mut layout = self.parts(Span {
                start: span_start,
                end: start + offset(s, "labelStart"),
            });
            layout.extend(self.parts(Span {
                start: start + offset(s, "labelEnd"),
                end: start + offset(s, "bodyStart"),
            }));
            layout.extend(self.parts(Span {
                start: start + offset(s, "bodyEnd"),
                end: start + offset(s, "end"),
            }));
            captions.push(json!({"kind":"ONSCREEN_TEXT","parts":self.parts(Span{start:start+offset(s,"bodyStart"),end:start+offset(s,"bodyEnd")}),"label":self.parts(Span{start:start+offset(s,"labelStart"),end:start+offset(s,"labelEnd")}),"layout":layout}));
        }
        if cursor < end {
            spoken.extend(self.parts(Span { start: cursor, end }))
        }
        let original = self.mapped(first, last, Span { start, end }, self.fields(h));
        let mut children = vec![
            json!({"kind":"DIALOGUE","parts":spoken,"layout":original["layout"],"speaker":self.parts(h.speaker),"channel":h.channel,"cue":self.nullable_parts(h.cue),"channelSource":self.nullable_parts(h.channel_source)}),
        ];
        children.extend(captions);
        Some(
            json!({"id":original["id"],"unitIds":original["unitIds"],"target":"BODY","kind":"BODY_GROUP","parts":self.parts(Span{start:offset(&units(self.index)[first],"start"),end:offset(&units(self.index)[last],"end")}),"layout":[],"children":children}),
        )
    }
    fn inline_label(&self, prefix: &str) -> Option<String> {
        let regex = re(if self.current() {
            r#"(?i)(?:^|[。.!！？?;；,，…][”’」』"']?|[ \t])((?:[A-Z][a-z]+(?:[ -][A-Z][a-z]+)+|[^。.!！？?;；,，:：()（） \t\x{a0}\x{3000}]+)(?:[\t \x{a0}\x{3000}]+(?:OS|VO))?(?:[（(][^()（）]*[）)])?(?:[\t \x{a0}\x{3000}]*(?:OS|VO))?)[ \t]*$"#
        } else {
            r"(?:^|[。.!！？?;；,， \t])([^。.!！？?;；,，:：()（） \t]+(?:[（(][^()（）]*[）)])?)[ \t]*$"
        });
        regex.captures(prefix).map(|c| c[1].to_string())
    }
    fn strip_meta(&self, s: &str) -> String {
        trim(&re(r"[（(][^()（）]*[）)]$").replace(s, "")).to_string()
    }
    fn multiple(&self, line: &Line, h: &Head) -> bool {
        self.scan(&line.content).0.into_iter().any(|colon| {
            if line.start + colon < h.body_start {
                return false;
            }
            let prefix = sub(&line.content, 0, colon);
            if self.known.iter().any(|name| {
                let Some(at) = prefix.rfind(name) else {
                    return false;
                };
                let at = utf16_len(&prefix[..at]);
                if line.start + at < h.body_start {
                    return false;
                }
                matches!(
                    self.read(
                        &Line {
                            number: line.number,
                            start: line.start + at,
                            content: sub(&line.content, at, utf16_len(&line.content))
                        },
                        None
                    ),
                    Some(Ok(_))
                )
            }) {
                return true;
            }
            let tail = sub(&prefix, h.body_start - line.start, utf16_len(&prefix));
            if self.syntax == "legacy" && !re(r"[。.!！？?;；,， \t]").is_match(&tail) {
                return false;
            }
            let Some(label) = self.inline_label(&tail) else {
                return false;
            };
            let name = self.strip_meta(&label);
            if utterance_property(
                &sub(self.text, h.speaker.start, h.speaker.end),
                &name,
                &h.cue
                    .map(|c| sub(self.text, c.start, c.end))
                    .unwrap_or_default(),
            ) {
                return false;
            }
            if re(r"[说问道答喊叫]$").is_match(&name)
                && re(r#"^[“‘「『"']"#).is_match(
                    sub(&line.content, colon + 1, utf16_len(&line.content)).trim_start_matches(ws),
                )
            {
                return false;
            }
            if self.current() && annotation(&format!("{name}：")).is_some() {
                return true;
            }
            valid_name(&name)
                && !re(META).is_match(&format!("{name}："))
                && annotation(&format!("{name}：")).is_none()
        })
    }
    fn dialogue_group(&self, line: &Line, first: &Head, row: usize, end: usize) -> Option<Value> {
        if self.syntax == "legacy"
            && !contains(
                &self.declared,
                &sub(self.text, first.speaker.start, first.speaker.end),
            )
            || self.scan(&line.content).1
        {
            return None;
        }
        let mut heads = vec![first.clone()];
        for colon in self.scan(&line.content).0 {
            if line.start + colon < first.body_start {
                continue;
            }
            let prefix = sub(&line.content, 0, colon);
            let label = if self.current() {
                self.inline_label(&prefix)
            } else {
                None
            };
            let raw = label.as_ref().map(|s| self.strip_meta(s));
            let names = if self.current() {
                raw.filter(|s| !s.is_empty())
                    .map(|s| vec![voice_head(&s).map(|v| v.0).unwrap_or(s)])
                    .unwrap_or_default()
            } else {
                self.declared.clone()
            };
            let mut found = vec![];
            for name in names {
                let Some(at) = prefix.rfind(&name) else {
                    continue;
                };
                let at = utf16_len(&prefix[..at]);
                if line.start + at < first.body_start {
                    continue;
                }
                let candidate = Line {
                    number: line.number,
                    start: line.start + at,
                    content: sub(&line.content, at, utf16_len(&line.content)),
                };
                if let Some(Ok(head)) = self.read(
                    &candidate,
                    if self.current() {
                        None
                    } else {
                        Some(&self.declared)
                    },
                ) {
                    let actual = sub(self.text, head.speaker.start, head.speaker.end);
                    if (actual == name
                        || self.current()
                            && (label.as_ref() == Some(&actual)
                                || self.head_cues && head.cue.is_some()))
                        && line.start + colon < head.body_start
                    {
                        let after = sub(self.text, line.start + colon + 1, head.body_start);
                        if trim(&after).is_empty()
                            || self.current()
                                && re(r"^[\t ]*(?:（[^()（）\r\n]*）|\([^()（）\r\n]*\))[\t ]*$")
                                    .is_match(&after)
                        {
                            found.push(head)
                        }
                    }
                }
            }
            if found.is_empty() {
                continue;
            }
            let eligible: Vec<_> = found
                .into_iter()
                .filter(|h| {
                    re(r#"(?:[。.!！？?;；,，…][”’」』"']?|[ \t])$"#).is_match(&sub(
                        self.text,
                        line.start,
                        h.speaker.start,
                    ))
                })
                .collect();
            if eligible.len() != 1 {
                return None;
            }
            if !heads
                .iter()
                .any(|h| h.speaker.start == eligible[0].speaker.start)
            {
                heads.push(eligible[0].clone())
            }
        }
        if heads.len() < 2 {
            return None;
        }
        heads.sort_by_key(|h| h.speaker.start);
        let mut dialogues = vec![];
        for (i, h) in heads.iter().enumerate() {
            let start = if i == 0 {
                offset(&units(self.index)[row], "start")
            } else {
                h.speaker.start
            };
            let finish = heads
                .get(i + 1)
                .map(|h| h.speaker.start)
                .unwrap_or(offset(&units(self.index)[row], "end"));
            let body_finish = finish
                - utf16_len(
                    re(r"[\t \r\n]*$")
                        .find(&sub(self.text, start, finish))
                        .unwrap()
                        .as_str(),
                );
            if h.body_start >= body_finish || h.body_start > end {
                return None;
            }
            let mut fields = self.fields(h);
            fields.push(Span {
                start: h.body_start,
                end: body_finish,
            });
            dialogues.push(json!({"parts":self.parts(Span{start:h.body_start,end:body_finish}),"layout":self.layout(start,finish,fields),"speaker":self.parts(h.speaker),"channel":h.channel,"channelSource":self.nullable_parts(h.channel_source),"cue":self.nullable_parts(h.cue)}));
        }
        Some(
            json!({"id":format!("line-{}",row+1),"unitIds":[units(self.index)[row]["id"]],"target":"BODY","kind":"DIALOGUE_GROUP","parts":self.parts(Span{start:offset(&units(self.index)[row],"start"),end:offset(&units(self.index)[row],"end")}),"layout":[],"dialogues":dialogues}),
        )
    }
}

pub fn prepare_standard_episode(
    text: &Value,
    source_id: &Value,
    options: &Value,
) -> IntakeResult<Value> {
    let index = create_source_index(text, source_id)?;
    let text = st(text);
    let syntax = options
        .get("syntax")
        .filter(|v| !v.is_null())
        .map(st)
        .unwrap_or(if options["postColonCue"] == false {
            "legacy"
        } else {
            "current"
        })
        .to_string();
    let performance = options
        .get("inlinePerformanceVersion")
        .map(st)
        .unwrap_or("inline-performance-v2");
    need(
        matches!(
            performance,
            "inline-performance-v1" | "inline-performance-v2"
        ),
        "UNSUPPORTED_PERFORMANCE_VERSION",
        "options.inlinePerformanceVersion",
    )?;
    let head_cues = syntax == "current"
        && options["inlinePerformance"] != false
        && performance == "inline-performance-v2";
    let mut lines = vec![];
    for (i, u) in units(&index).iter().enumerate() {
        let raw = re(r"(?:\r\n|\r|\n)$")
            .replace(st(&u["text"]), "")
            .to_string();
        let indent = re(r"^[\t \x{feff}]*").find(&raw).unwrap().end();
        lines.push(Line {
            number: i + 1,
            start: offset(u, "start") + utf16_len(&raw[..indent]),
            content: raw[indent..].to_string(),
        });
    }
    let mut known = vec![];
    if let Some(names) = options.get("knownSpeakers").filter(|v| !v.is_null()) {
        for name in arr(names, "options.knownSpeakers")? {
            if let Some(s) = name.as_str().filter(|s| !trim(s).is_empty()) {
                add(&mut known, trim(s).to_string())
            }
        }
    }
    let mut declared = known.clone();
    for line in &lines {
        if re(CAST).is_match(&line.content) {
            let tail = re(CAST).replace(&line.content, "").to_string();
            let names = if syntax == "legacy" {
                re(r"[、,，;；]")
                    .split(&tail)
                    .map(|s| trim(s).to_string())
                    .collect()
            } else {
                cast_names(&tail)
            };
            for name in names {
                if valid_name(&name) {
                    add(&mut known, name.clone());
                    add(&mut declared, name);
                    continue;
                }
                if syntax == "legacy" {
                    continue;
                }
                if let Some(c) =
                    re(r"^([^()（）]+)(?:（([^()（）]+)）|\(([^()（）]+)\))$").captures(&name)
                {
                    let base = trim(&c[1]);
                    if !valid_name(base) {
                        continue;
                    }
                    add(&mut known, base.to_string());
                    add(&mut declared, base.to_string());
                    let qualifier = trim(c.get(2).or_else(|| c.get(3)).unwrap().as_str());
                    if valid_name(qualifier)
                        && !re(r"(?i)^(?:OS|VO|旁白|画外音|内心独白|系统音)$").is_match(qualifier)
                    {
                        add(&mut known, name.clone());
                        add(&mut declared, name.clone());
                        add(&mut known, format!("{base}{qualifier}"));
                    }
                }
            }
        }
        if let Some(Ok(head)) = read_head(line, &[], &syntax, false, &[]) {
            add(&mut known, sub(text, head.speaker.start, head.speaker.end));
        }
    }
    let mut whole = declared.clone();
    if head_cues {
        for line in &lines {
            if let Some(Ok(head)) = read_head(line, &[], &syntax, false, &[]) {
                if head.channel_source.is_none() {
                    add(&mut whole, sub(text, head.speaker.start, head.speaker.end));
                }
            }
        }
    }
    let p = Planner {
        text,
        index: &index,
        lines,
        known,
        declared,
        whole,
        syntax,
        head_cues,
    };
    let mut entries = vec![];
    let mut diagnostics = vec![];
    let mut ambiguous = HashSet::new();
    let mut i = 0;
    while i < p.lines.len() {
        let line = &p.lines[i];
        let end = p.body_end(i);
        if ambiguous.contains(&line.number) {
            i += 1;
            continue;
        }
        if trim(&line.content).is_empty() {
            entries.push(json!({"id":format!("line-{}",i+1),"target":"EXCLUDE","category":"BLANK","reason":"原文空白行","unitIds":[units(&index)[i]["id"]]}));
            i += 1;
            continue;
        }
        if re(r"^【作者资料】[ \t]*$").is_match(&line.content) {
            let mut last = i;
            while last + 1 < p.lines.len()
                && !p.episode(&p.lines[last + 1].content)
                && !p.scene(&p.lines[last + 1].content)
            {
                last += 1
            }
            entries.push(p.entry(
                "ANNOTATION",
                "CREATIVE_GUIDANCE",
                i,
                last,
                Span {
                    start: line.start,
                    end: p.body_end(last),
                },
            ));
            i = last + 1;
            continue;
        }
        if p.current() && p.guidance(&line.content) {
            entries.push(p.entry(
                "ANNOTATION",
                "CREATIVE_GUIDANCE",
                i,
                i,
                Span {
                    start: line.start,
                    end,
                },
            ));
            i += 1;
            continue;
        }
        if p.typed(&line.content) && !p.end_marker(line) {
            diagnostics.push(diagnostic(
                line,
                "UNKNOWN_TYPED_LINE",
                "未识别的显式类型标记，不能默认归为动作或对白。",
            ));
            i += 1;
            continue;
        }
        let structure = if p.episode(&line.content) {
            Some("EPISODE_HEADING")
        } else if p.scene(&line.content) {
            Some("SCENE_HEADING")
        } else if re(META).is_match(&line.content) || p.end_marker(line) {
            Some("SCENE_METADATA")
        } else {
            None
        };
        if let Some(kind) = structure {
            entries.push(p.entry(
                "STRUCTURE",
                kind,
                i,
                i,
                Span {
                    start: line.start,
                    end,
                },
            ));
            i += 1;
            continue;
        }
        if let Some(kind) = annotation(&line.content) {
            entries.push(p.entry(
                "ANNOTATION",
                kind,
                i,
                i,
                Span {
                    start: line.start,
                    end,
                },
            ));
            i += 1;
            continue;
        }
        if p.current() && p.omission(&line.content) {
            entries.push(p.entry(
                "ANNOTATION",
                "CREATIVE_GUIDANCE",
                i,
                i,
                Span {
                    start: line.start,
                    end,
                },
            ));
            i += 1;
            continue;
        }
        if p.current() && p.screen(&line.content) {
            let tail=re(r"^(?:弹幕|字幕|屏幕文字)(?:（[^()（）\r\n]+）|\([^()（）\r\n]+\))?[\t \x{a0}\x{3000}]*[:：]").replace(&line.content,"").to_string();
            if trim(&tail).is_empty() {
                diagnostics.push(diagnostic(
                    line,
                    "EMPTY_ONSCREEN_TEXT",
                    "显式画面文字标记后没有内容。",
                ))
            } else {
                entries.push(p.entry(
                    "BODY",
                    "ONSCREEN_TEXT",
                    i,
                    i,
                    Span {
                        start: line.start,
                        end,
                    },
                ))
            }
            i += 1;
            continue;
        }
        if let Some(marker) = re(r"^[△▲][ \t]*").find(&line.content) {
            let start = line.start + utf16_len(marker.as_str());
            if start >= end {
                diagnostics.push(diagnostic(line, "EMPTY_ACTION", "动作标记后没有正文。"))
            } else {
                entries.push(p.entry("BODY", "ACTION", i, i, Span { start, end }))
            }
            i += 1;
            continue;
        }
        if p.missing_speaker(&line.content) {
            diagnostics.push(diagnostic(line, "MISSING_SPEAKER", "对白头缺少明确说者。"));
            i += 1;
            continue;
        }
        let read = p.read(line, None);
        if let Some(Err(d)) = &read {
            diagnostics.push(d.clone());
            i += 1;
            continue;
        }
        if let Some(Ok(head)) = read {
            let inline_captions = if p.current() {
                explicit_subtitles(&sub(text, head.body_start, end))
            } else {
                json!({"spans":[],"invalid":false})
            };
            if inline_captions["invalid"] == true {
                diagnostics.push(diagnostic(line,"MIXED_DIALOGUE_ONSCREEN_TEXT","显式字幕为空、嵌套、跨行或未闭合；当前程序不能安全分型，原文保留且不作对白发出。"));
                i += 1;
                continue;
            }
            let inline = head.body_start < end;
            let mut last = if inline { Some(i) } else { None };
            let mut start = head.body_start;
            let mut next = i + 1;
            let blocked = p.multiple(line, &head);
            if blocked {
                if !inline_captions["spans"].as_array().unwrap().is_empty() {
                    diagnostics.push(diagnostic(line,"MIXED_DIALOGUE_ONSCREEN_TEXT","同一行同时含多说者与字幕，当前混合正文只支持一个明确说者；原文保留且不作整段对白发出。"));
                    i += 1;
                    continue;
                }
                if let Some(group) = p.dialogue_group(line, &head, i, end) {
                    entries.push(group);
                    i += 1;
                    continue;
                }
                diagnostics.push(diagnostic(
                    line,
                    "MULTIPLE_SPEAKERS_ON_LINE",
                    "同一物理行的说者、引号或分隔边界仍不明确；原文保留，不能猜测拆分。",
                ));
            }
            while !blocked && next < p.lines.len() {
                let continuation = &p.lines[next];
                let previous = last
                    .map(|last| sub(text, start, p.body_end(last)))
                    .unwrap_or_default();
                let in_quote = p.scan(&previous).1;
                if !in_quote
                    && (p.structural(continuation)
                        || p.read(continuation, None).is_some()
                        || p.missing_speaker(&continuation.content))
                {
                    break;
                }
                if !in_quote && p.actor_action(continuation) {
                    break;
                }
                if !trim(&continuation.content).is_empty() {
                    let starts_name = p.known.iter().any(|n| {
                        continuation.content.starts_with(n) && continuation.content.len() > n.len()
                    });
                    let previous_captions = if p.current() {
                        explicit_subtitles(&previous)
                    } else {
                        json!({"spans":[]})
                    };
                    let mut spoken = previous.clone();
                    for span in previous_captions["spans"].as_array().unwrap().iter().rev() {
                        spoken = format!(
                            "{}{}",
                            sub(&spoken, 0, offset(span, "start")),
                            sub(&spoken, offset(span, "end"), utf16_len(&spoken))
                        );
                    }
                    let finished =
                        inline && re(r#"[。.!！？?…][”’」』"']?[ \t]*$"#).is_match(&spoken);
                    let whole_quoted = re(r#"^[“‘「『"']"#)
                        .is_match(previous.trim_start_matches(ws))
                        && re(r#"[”’」』"']$"#).is_match(previous.trim_end_matches(ws));
                    if !in_quote && (starts_name || finished || whole_quoted) {
                        diagnostics.push(diagnostic(continuation,"AMBIGUOUS_CONTINUATION","未标记新行可能是动作或对白续句，请明确说者续写或动作标记；不自动归入上一段对白。"));
                        ambiguous.insert(continuation.number);
                        break;
                    }
                    let colons = if in_quote {
                        vec![]
                    } else {
                        p.scan(&continuation.content).0
                    };
                    if colons.into_iter().any(|colon| {
                        !re(r"[说问道答喊叫]$").is_match(&sub(&continuation.content, 0, colon))
                            || !re(r#"^[“「『"']"#).is_match(
                                sub(
                                    &continuation.content,
                                    colon + 1,
                                    utf16_len(&continuation.content),
                                )
                                .trim_start_matches(ws),
                            )
                    }) {
                        break;
                    }
                    if last.is_none() {
                        start = continuation.start
                    }
                    last = Some(next);
                }
                next += 1;
            }
            let Some(last) = last else {
                if !blocked
                    && p.current()
                    && head.cue.is_some()
                    && head.channel == "SPEECH"
                    && head.channel_source.is_none()
                    && re(r"[:：]").find(&line.content).is_some_and(|colon| {
                        head.cue.unwrap().start
                            > line.start + utf16_len(&line.content[..colon.start()])
                    })
                {
                    entries.push(p.entry(
                        "BODY",
                        "ACTION",
                        i,
                        i,
                        Span {
                            start: line.start,
                            end,
                        },
                    ));
                } else if !blocked {
                    diagnostics.push(diagnostic(
                        line,
                        "EMPTY_DIALOGUE",
                        "说者头之后没有可归属的对白正文。",
                    ));
                }
                i += 1;
                continue;
            };
            if !blocked {
                let body = sub(text, start, p.body_end(last));
                if p.scan(&body).1 {
                    diagnostics.push(diagnostic(line,"UNCLOSED_DIALOGUE_QUOTE","对白引号未闭合或撇号用途不明确；不能据此把后续说者归入同一段，原文保留待核对。"));
                    i = last + 1;
                    continue;
                }
                let subtitles = if p.current() {
                    explicit_subtitles(&body)
                } else {
                    json!({"spans":[],"invalid":false})
                };
                if subtitles["invalid"] == true {
                    diagnostics.push(diagnostic(
                        line,
                        "MIXED_DIALOGUE_ONSCREEN_TEXT",
                        "显式字幕形状不受支持；原文保留且不作对白发出。",
                    ));
                    i = last + 1;
                    continue;
                }
                if !subtitles["spans"].as_array().unwrap().is_empty() {
                    if let Some(group) = p.mixed(i, last, start, p.body_end(last), &head) {
                        entries.push(group)
                    } else {
                        diagnostics.push(diagnostic(
                            line,
                            "MIXED_DIALOGUE_ONSCREEN_TEXT",
                            "混合正文无法保真分型；原文保留。",
                        ))
                    }
                    i = last + 1;
                    continue;
                }
                let mut entry = p.mapped(
                    i,
                    last,
                    Span {
                        start,
                        end: p.body_end(last),
                    },
                    p.fields(&head),
                );
                entry["target"] = json!("BODY");
                entry["kind"] = json!("DIALOGUE");
                entry["channel"] = json!(head.channel);
                entry["speaker"] = json!(p.parts(head.speaker));
                entry["cue"] = p.nullable_parts(head.cue);
                entry["channelSource"] = p.nullable_parts(head.channel_source);
                entries.push(entry);
            }
            i = last + 1;
            continue;
        }
        if !p.scan(&line.content).0.is_empty() {
            diagnostics.push(diagnostic(
                line,
                "UNKNOWN_COLON_LINE",
                "冒号行缺少明确说者或类型；请补充人物名单，或明确该行是对白还是资料。",
            ));
            i += 1;
            continue;
        }
        entries.push(p.entry(
            "BODY",
            "ACTION",
            i,
            i,
            Span {
                start: line.start,
                end,
            },
        ));
        i += 1;
    }
    if trim(text).is_empty() {
        diagnostics
            .push(json!({"line":1,"code":"EMPTY_SOURCE","message":"没有可接收的剧本正文。"}));
    }
    let entries = json!(entries);
    let version = standard_script_plan_version(
        &index,
        &entries,
        &json!({"inlinePerformance":p.current()&&options["inlinePerformance"]!=false,"inlinePerformanceVersion":performance}),
    )?;
    let plan = json!({"schemaVersion":version,"sourceId":index["sourceId"],"sourceVersion":index["sourceVersion"],"entries":entries});
    let mut compiled = Value::Null;
    if diagnostics.is_empty() {
        match compile_standard_script(&index, &plan) {
            Ok(c) => compiled = c,
            Err(cause) => {
                let path = cause.field_path.as_deref().unwrap_or("undefined");
                let entry_number = re(r"plan\.entries\[(\d+)\]")
                    .captures(path)
                    .and_then(|c| c[1].parse::<usize>().ok());
                let entry = entry_number.and_then(|i| plan["entries"].get(i));
                let unit = cause.source_unit_id.as_deref().or_else(|| {
                    entry
                        .and_then(|e| e["unitIds"].get(0))
                        .and_then(Value::as_str)
                });
                let mixed =
                    entry.is_some_and(|e| e["target"] == "BODY" && e["kind"] == "BODY_GROUP");
                let line = unit
                    .and_then(|id| units(&index).iter().position(|u| st(&u["id"]) == id))
                    .map(|i| i + 1)
                    .unwrap_or(1);
                diagnostics.push(json!({"line":line,"code":if mixed{"MIXED_DIALOGUE_ONSCREEN_TEXT"}else{&cause.code},"message":format!("{}源片段编译未通过：{path}（{}）",if mixed{"混合字幕边界暂不支持，原文保留且不作整段对白发出；"}else{""},cause.code)}));
            }
        }
    }
    Ok(json!({"index":index,"plan":plan,"compiled":compiled,"diagnostics":diagnostics}))
}
