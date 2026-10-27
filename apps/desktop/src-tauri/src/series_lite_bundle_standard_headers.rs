use super::inline::*;
use super::*;
pub(super) fn declared(source: &str, name: &str) -> bool {
    re(r"\r\n|\r|\n").split(source).any(|line| {
        let p = re(r"^\s*(?:出场人物|人物|角色)[ \t]*[:：]");
        p.find(line).is_some_and(|m| {
            line[m.end()..]
                .split(['、', ',', '，', ';', '；'])
                .any(|s| trim(s) == name)
        })
    })
}
pub(super) fn compact_voice(source: &str, child: &Value, v5: bool) -> Result<Option<Value>> {
    let token = if v5 {
        compact_performance_head_projection(
            source,
            &child["speaker"]["parts"],
            &child["cue"]["parts"],
            &child["channelSource"]["parts"],
        )?
    } else {
        None
    }
    .or_else(|| voice_projection(source, &child["speaker"]));
    if let Some(t) = &token {
        let f = &child["channelSource"];
        need(
            st(&child["channel"]) == st(&t["text"]).to_uppercase()
                && f["text"] == t["text"]
                && part_start(&f["parts"]) == n(&t["start"])
                && part_end(&f["parts"]) == n(&t["end"]),
            "STANDARD_COMPACT_VOICE",
        )?;
    }
    Ok(token)
}
pub(super) fn post_fields(source: &str, child: &Value) -> bool {
    let (Some(a), Some(b)) = (
        part_start(&child["speaker"]["parts"]),
        part_start(&child["textAssembly"]["parts"]),
    ) else {
        return false;
    };
    let Ok(text) = slice(source, a, b) else {
        return false;
    };
    let p = re(
        r"(?i)^[^:：()（）\r\n]+?(?:[（(][^()（）\r\n]*[）)](?:[\t \x{a0}\x{3000}]*(?:OS|VO))?)?[\t ]*[:：]",
    );
    p.find(&text).is_some_and(|m| {
        [&child["cue"], &child["channelSource"]]
            .iter()
            .any(|f| part_start(&f["parts"]).is_some_and(|x| x >= a + len(m.as_str())))
    })
}
fn exact(source: &str, f: &Value, range: Option<(usize, usize)>) -> bool {
    match range {
        None => f.is_null(),
        Some((a, b)) => {
            part_start(&f["parts"]) == Some(a)
                && part_end(&f["parts"]) == Some(b)
                && slice(source, a, b).ok().as_deref() == f["text"].as_str()
        }
    }
}
fn default_channel(speaker: &str) -> &'static str {
    match speaker {
        "旁白" => "NARRATION",
        "画外音" => "VO",
        "系统音" => "SYSTEM",
        _ => "SPEECH",
    }
}
fn actor_name(s: &str, plural: bool) -> bool {
    if plural {
        re(r"^[\p{L}\p{N}·・ _.-]+(?:、[\p{L}\p{N}·・ _.-]+)*$").is_match(s)
    } else {
        re(r"^[\p{L}\p{N}·・ _.-]+(?:[（(][^()（）]*[）)])?$").is_match(s)
    }
}
pub(super) fn post_header(source: &str, start: usize, body: usize, child: &Value) -> Result<()> {
    let text = slice(source, start, body)?;
    let p = re(
        r"(?i)^[\t \x{feff}]*([^:：()（）\r\n]+?)(?:(?:（([^()（）\r\n]*)）|\(([^()（）\r\n]*)\))([\t \x{a0}\x{3000}]*(?:OS|VO))?)?[\t ]*[:：][\t ]*(?:（([^()（）\r\n]*)）|\(([^()（）\r\n]*)\))[\t \r\n]*$",
    );
    let c = p.captures(&text).ok_or("STANDARD_POST_CUE_HEADER")?;
    need(
        (c.get(4).is_none() || c.get(2).is_some() || c.get(3).is_some())
            && !re(r"^[（(]").is_match(&tail(source, body)),
        "STANDARD_POST_CUE_HEADER",
    )?;
    let post = c.get(5).or_else(|| c.get(6)).unwrap();
    need(
        !trim(post.as_str()).is_empty() && !re(r#"[:：“”‘’「」『』"'△▲]"#).is_match(post.as_str()),
        "STANDARD_POST_CUE_CONTENT",
    )?;
    let m = c.get(1).unwrap();
    let a =
        start + len(&text[..m.start()]) + len(m.as_str()) - len(m.as_str().trim_start_matches(ws));
    let mut b = start + len(&text[..m.start()]) + len(m.as_str().trim_end_matches(ws));
    let bare = bare(source, &child["speaker"], &child["cue"]);
    if bare.is_some_and(|(_, end)| end <= b) {
        b = part_end(&child["speaker"]["parts"]).ok_or("STANDARD_POST_CUE_SPEAKER")?;
    }
    let compact = compact_voice(source, child, false)?;
    if let Some(t) = &compact {
        if Some(b) == n(&t["end"]) {
            b = a + len(slice(source, a, n(&t["start"]).unwrap())?.trim_end_matches(ws));
        }
    }
    let pre = c.get(2).or_else(|| c.get(3));
    let qualified_end = pre.map(|m| start + len(&text[..m.end()]) + 1).unwrap_or(b);
    let qualified = compact
        .as_ref()
        .is_none_or(|t| n(&t["start"]).is_some_and(|x| x >= qualified_end))
        && qualified_end > b
        && slice(source, a, qualified_end)? == st(&child["speaker"]["text"])
        && pre.is_some_and(|p| {
            !re(r"(?i)^(?:OS|VO|旁白|画外音|内心独白|系统音)$").is_match(trim(p.as_str()))
        })
        && declared(source, st(&child["speaker"]["text"]));
    if qualified {
        b = qualified_end;
    }
    need(
        exact(source, &child["speaker"], Some((a, b)))
            && actor_name(
                if qualified {
                    trim(m.as_str())
                } else {
                    st(&child["speaker"]["text"])
                },
                true,
            )
            && !re(r"^\d+$").is_match(st(&child["speaker"]["text"])),
        "STANDARD_POST_CUE_SPEAKER",
    )?;
    let mut tokens: Vec<(usize, String, usize, usize)> = vec![];
    for slot in if qualified {
        vec![5, 6]
    } else {
        vec![2, 3, 5, 6]
    } {
        if let Some(m) = c.get(slot) {
            for t in re(r"[^,， \t]+").find_iter(m.as_str()) {
                let from = start + len(&text[..m.start()]) + len(&m.as_str()[..t.start()]);
                let end = from + len(t.as_str());
                if slot >= 5 && re(r"(?i)^(?:OS|VO)\p{Han}").is_match(t.as_str()) {
                    tokens.push((slot, t.as_str()[..2].into(), from, from + 2));
                    tokens.push((slot, t.as_str()[2..].into(), from + 2, end));
                } else {
                    tokens.push((slot, t.as_str().into(), from, end));
                }
            }
        }
    }
    if let Some(t) = &compact {
        tokens.insert(
            0,
            (
                1,
                st(&t["text"]).into(),
                n(&t["start"]).unwrap(),
                n(&t["end"]).unwrap(),
            ),
        );
    }
    if let Some((a, b)) = bare {
        tokens.insert(0, (1, st(&child["cue"]["text"]).into(), a, b));
    }
    need(!tokens.iter().any(|(slot,t,_,_)|*slot>=5&&!re(r"(?i)^(OS|VO)$").is_match(t)&&re(r"(?i)(?-u:\b)(?:OS|VO)(?-u:\b)|(?-u:\b)(?:O\.S\.?|V\.O\.?)|旁白|画外音|内心独白|系统音").is_match(t)),"STANDARD_POST_CUE_CHANNEL")?;
    let channels: Vec<_> = tokens
        .iter()
        .filter(|(_, t, _, _)| re(r"(?i)^(OS|VO)$").is_match(t))
        .collect();
    let cues: Vec<_> = tokens
        .iter()
        .filter(|(_, t, _, _)| !re(r"(?i)^(OS|VO)$").is_match(t))
        .collect();
    need(
        channels.len() <= 1 && cues.iter().map(|x| x.0).collect::<HashSet<_>>().len() <= 1,
        "STANDARD_POST_CUE_AMBIGUOUS",
    )?;
    let cue = cues.first().map(|x| (x.2, cues.last().unwrap().3));
    let chan = channels.first().map(|x| (x.2, x.3));
    let expected = channels
        .first()
        .map(|x| x.1.to_uppercase())
        .unwrap_or_else(|| default_channel(st(&child["speaker"]["text"])).into());
    need(
        exact(source, &child["cue"], cue)
            && exact(source, &child["channelSource"], chan)
            && child["channel"] == expected,
        "STANDARD_POST_CUE_FIELDS",
    )
}
pub(super) fn group_utterance(text: &str, speaker: &str, cue: &str) -> Result<()> {
    let u: Vec<_> = text.encode_utf16().collect();
    let mut stack = vec![];
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        if c == 92 {
            i += 2;
            continue;
        }
        if apostrophe(&u, i, &stack) {
            i += 1;
            continue;
        }
        if stack.last() == Some(&c) {
            stack.pop();
            i += 1;
            continue;
        }
        if let Some(close) = closer(c) {
            if !matches!(c, 0x3010 | 91)
                || stack.is_empty()
                || stack.last().is_some_and(|c| matches!(c, 0x3011 | 93))
            {
                stack.push(close);
                i += 1;
                continue;
            }
        }
        need(
            !matches!(c, 0x201d | 0x2019 | 0x300d | 0x300f),
            "STANDARD_GROUP_QUOTE",
        )?;
        if !stack.is_empty() {
            i += 1;
            continue;
        }
        need(!matches!(c, 0x25b3 | 0x25b2), "STANDARD_GROUP_ACTION")?;
        if matches!(c, 58 | 0xff1a) {
            let prefix = slice(text, 0, i)?;
            let p = re(
                r"(?:^|[。.!！？?;；,， \t\r\n])([^。.!！？?;；,，:：()（） \t\r\n]+(?:[（(][^()（）]*[）)])?)[ \t]*$",
            );
            if let Some(caps) = p.captures(&prefix) {
                let label = trim(&caps[1]);
                if !utterance_property(speaker, label, cue)
                    && !re(r"^(?:时间层|时间|地点|场景|天气|内外|道具)$").is_match(label)
                    && !(re(r"[说问道答喊叫]$").is_match(label)
                        && re(r#"^[“‘「『"']"#).is_match(tail(text, i + 1).trim_start_matches(ws)))
                {
                    need(!actor_name(label, false), "STANDARD_GROUP_NESTED_HEAD")?;
                }
            }
        }
        i += 1;
    }
    need(stack.is_empty(), "STANDARD_GROUP_QUOTE")
}
pub(super) fn subtitle_spans(text: &str) -> Result<Vec<[usize; 6]>> {
    let u: Vec<_> = text.encode_utf16().collect();
    let mut stack = vec![];
    let mut out = vec![];
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        if c == 92 {
            i += 2;
            continue;
        }
        if apostrophe(&u, i, &stack) {
            i += 1;
            continue;
        }
        if stack.last() == Some(&c) {
            stack.pop();
            i += 1;
            continue;
        }
        let rest = tail(text, i);
        if stack.is_empty() && re(r"(?i)^[【\[](?:字幕|SUBTITLE)(?:[：:\t \]】]|$)").is_match(&rest)
        {
            let p = re(r"(?i)^【(字幕|SUBTITLE)[：:]([^【】\r\n]+)】");
            let m = p.captures(&rest).ok_or("STANDARD_BODY_CAPTION")?;
            need(!trim(&m[2]).is_empty(), "STANDARD_BODY_CAPTION")?;
            let body = i + 1 + len(&m[1]) + 1;
            let end = i + len(&m[0]);
            out.push([i, end, i + 1, i + 1 + len(&m[1]), body, body + len(&m[2])]);
            i = end;
            continue;
        }
        if let Some(close) = closer(c) {
            if !matches!(c, 0x3010 | 91)
                || stack.is_empty()
                || stack.last().is_some_and(|c| matches!(c, 0x3011 | 93))
            {
                stack.push(close);
            }
        }
        i += 1;
    }
    Ok(out)
}
pub(super) fn normal_header(
    source: &str,
    start: usize,
    body: usize,
    child: &Value,
    v5: bool,
    mixed: bool,
) -> Result<()> {
    let text = slice(source, start, body)?;
    let pattern = if mixed {
        r"(?i)^\s*([^:：()（）]+?)(?:[（(]([^()（）]*)[）)]([\t \x{a0}\x{3000}]*(OS|VO))?)?[ \t]*[:：][ \t\r\n]*$"
    } else {
        r"(?i)^\s*([^:：()（）]+?)(?:[（(]([^()（）]*)[）)]([\t \x{a0}\x{3000}]*(OS|VO))?)?[ \t]*[:：][ \t]*$"
    };
    let code = if mixed {
        "STANDARD_BODY_HEADER"
    } else {
        "STANDARD_GROUP_HEADER"
    };
    let p = re(pattern);
    let cap = p.captures(&text).ok_or(code)?;
    need(cap.get(3).is_none() || cap.get(2).is_some(), code)?;
    let compact = compact_voice(source, child, v5)?;
    let before_colon = text.rsplit_once([':', '：']).unwrap().0;
    let qualified_text = if let Some(m) = cap.get(3) {
        format!("{}{}", &before_colon[..m.start()], &before_colon[m.end()..])
    } else {
        before_colon.into()
    };
    let qualified = cap.get(2).is_some_and(|m| {
        !re(r"(?i)^(?:OS|VO|旁白|画外音|内心独白|系统音)$").is_match(trim(m.as_str()))
    }) && trim(&qualified_text) == st(&child["speaker"]["text"])
        && declared(source, st(&child["speaker"]["text"]));
    need(
        (qualified
            || trim(&cap[1])
                == head_prefix(
                    source,
                    &child["speaker"],
                    &child["cue"],
                    &child["channelSource"],
                )?)
            && (mixed
                || actor_name(
                    if qualified {
                        trim(&cap[1])
                    } else {
                        st(&child["speaker"]["text"])
                    },
                    true,
                )),
        code,
    )?;
    let mut fields: Vec<_> = [&child["speaker"], &child["cue"], &child["channelSource"]]
        .into_iter()
        .filter(|f| !f.is_null())
        .collect();
    fields.sort_by_key(|f| part_start(&f["parts"]));
    let mut edge = start;
    for f in fields {
        let a = part_start(&f["parts"]).ok_or(code)?;
        let b = part_end(&f["parts"]).ok_or(code)?;
        need(a >= edge, code)?;
        let pattern = if mixed {
            if edge == start {
                r"^[\s\x{feff}]*$"
            } else {
                r"^[\s()（）,，]*$"
            }
        } else if edge == start {
            r"^[\s\x{feff}()（）【】\[\]]*$"
        } else {
            r"^[\s\x{feff}()（）【】\[\],，]*$"
        };
        need(re(pattern).is_match(&slice(source, edge, a)?), code)?;
        edge = b;
    }
    need(
        edge <= body
            && re(if mixed {
                r"^[\t ()（）]*[:：][\t \r\n]*$"
            } else {
                r"^[\t ()（）【】\[\]]*[:：][\t ]*$"
            })
            .is_match(&slice(source, edge, body)?),
        code,
    )?;
    let mut tokens = vec![];
    if cap.get(4).is_none() {
        if let Some(t) = compact {
            tokens.push(st(&t["text"]).to_string());
        }
    }
    if let Some(t) = cap.get(4) {
        tokens.push(t.as_str().into());
    }
    if !qualified {
        tokens.extend(
            re(r"[,， \t]+")
                .split(cap.get(2).map(|m| m.as_str()).unwrap_or(""))
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }
    let channels: Vec<_> = tokens
        .iter()
        .filter(|s| re(r"(?i)^(OS|VO)$").is_match(s))
        .collect();
    let expected = channels
        .first()
        .map(|s| s.to_uppercase())
        .unwrap_or_else(|| default_channel(st(&child["speaker"]["text"])).into());
    need(
        channels.len() <= 1
            && child["channel"] == expected
            && channels
                .first()
                .map(|s| child["channelSource"]["text"] == **s)
                .unwrap_or_else(|| child["channelSource"].is_null()),
        if mixed {
            "STANDARD_BODY_CHANNEL"
        } else {
            "STANDARD_GROUP_CHANNEL"
        },
    )
}
