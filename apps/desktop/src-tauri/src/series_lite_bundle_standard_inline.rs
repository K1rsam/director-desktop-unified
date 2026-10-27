use super::*;
pub(super) fn explicit(value: &str, v2: bool) -> bool {
    trim(value).split([',','，','、',';','；']).all(|s|re(r"^(?:低声|轻声|小声|大声|压低声音|提高声音|压住颤音|哽咽|哭腔|带哭腔|气喘|喘息|咬牙|大笑|苦笑|冷笑|轻笑|笑着|哭着|怒视|皱眉|挑眉|点头|摇头|抬头|低头|转头|转身|起身|蹲下|抬手|摆手|挥手|握拳|攥拳|摊手|耸肩|抿嘴|闭眼|睁眼|深吸一口气|叹气|掐着(?:手指|指头))$").is_match(trim(s))||v2&&matches!(trim(s),"恍然大悟"|"恍然大悟的样子"))
}
pub(super) fn head_cue(value: &str) -> bool {
    explicit(value,true)||re(r"^(?:(?:微微|轻轻|忽然|突然|忍不住)?(?:憋笑|怒吼|吼道|笑道|听完|听罢|精神传音)|(?:声音|嗓音)(?:微微|略微|不住地)?(?:发抖|颤抖|沙哑)|笑容(?:瞬间|突然|渐渐)?(?:僵住|僵死|消失|凝固)|(?:急|气|笑|哭)得(?:眼圈发红|满脸通红|浑身发抖|直跺脚|说不出话)|(?:这时|此时|突然)?(?:再次)?(?:提示|提醒|通知))$").is_match(trim(value))
}
pub(super) fn bare(source: &str, speaker: &Value, cue: &Value) -> Option<(usize, usize)> {
    let a = part_end(&speaker["parts"])?;
    let b = part_start(&cue["parts"])?;
    if !head_cue(st(&cue["text"]))
        || b < a
        || !re(r"^[\t \x{a0}\x{3000}]*$").is_match(&slice(source, a, b).ok()?)
    {
        return None;
    }
    Some((b, part_end(&cue["parts"])?))
}
pub(super) fn voice_projection(source: &str, speaker: &Value) -> Option<Value> {
    let end = part_end(&speaker["parts"])?;
    let rest = tail(source, end);
    let patterns = [
        r"(?i)^([\t \x{a0}\x{3000}]*)(OS|VO)[\t \x{a0}\x{3000}]*[（(:：]",
        r"(?i)^([\t \x{a0}\x{3000}]*[（(][^()（）\r\n]*[）)][\t \x{a0}\x{3000}]*)(OS|VO)[\t \x{a0}\x{3000}]*[:：]",
    ];
    for p in patterns {
        if let Some(c) = re(p).captures(&rest) {
            if c[1].is_empty() && !re(r"(?:\p{Han}|[）)])$").is_match(st(&speaker["text"])) {
                return None;
            }
            let a = end + len(&c[1]);
            return Some(json!({"text":&c[2],"start":a,"end":a+2}));
        }
    }
    None
}
pub(super) fn head_prefix(
    source: &str,
    speaker: &Value,
    cue: &Value,
    channel: &Value,
) -> Result<String> {
    let mut end = part_end(&speaker["parts"]).ok_or("STANDARD_PARTS")?;
    if let Some((_, b)) = bare(source, speaker, cue) {
        end = b;
    }
    if let (Some(a), Some(b)) = (part_start(&channel["parts"]), part_end(&channel["parts"])) {
        if a >= end
            && re(r"^[\t \x{a0}\x{3000}]*$").is_match(&slice(source, end, a)?)
            && re(r"(?i)^(OS|VO)$").is_match(st(&channel["text"]))
        {
            end = b;
        }
    }
    slice(
        source,
        part_start(&speaker["parts"]).ok_or("STANDARD_PARTS")?,
        end,
    )
}
pub fn compact_performance_head_projection(
    source: &str,
    speaker: &Value,
    cue: &Value,
    channel: &Value,
) -> Result<Option<Value>> {
    let (Some(sa), Some(se), Some(ca), Some(ce), Some(a), Some(b)) = (
        part_start(speaker),
        part_end(speaker),
        part_start(cue),
        part_end(cue),
        part_start(channel),
        part_end(channel),
    ) else {
        return Ok(None);
    };
    let speaker_text = copy_parts(source, speaker)?;
    let cue_text = copy_parts(source, cue)?;
    let text = copy_parts(source, channel)?;
    let _ = sa;
    if !re(r"\p{Han}$").is_match(&speaker_text)
        || se != ca
        || ce > a
        || !re(r"^[\t \x{a0}\x{3000}]*$").is_match(&slice(source, ce, a)?)
        || !head_cue(&cue_text)
        || !re(r"(?i)^(OS|VO)$").is_match(&text)
        || b - a != 2
        || !re(r"^[\t ]*[（(:：]").is_match(&tail(source, b))
    {
        return Ok(None);
    }
    Ok(Some(
        json!({"text":text,"start":a,"end":b,"cueText":cue_text}),
    ))
}
pub(super) fn utterance_property(speaker: &str, label: &str, cue: &str) -> bool {
    re(r"^[\p{L}\p{N}]{1,16}(?:老样子|如下|很简单|是这样)$").is_match(label)||re(r"^(?:系统|系统音|系统窗口|系统提示窗口)$").is_match(speaker)&&re(r"^(?:触发(?:终极)?(?:主线|支线)?任务|任务目标|任务奖励|解锁(?:全新)?(?:建筑|技能))$").is_match(label)||re(r"(?:念出(?:了)?声(?:来)?|读出(?:了)?声(?:来)?|朗读|宣读|(?:大声|高声|低声|轻声|开口)(?:念着|读着))").is_match(cue)&&!re(r"(?:不(?:要|许|能|会|敢|再|想)?|没(?:有)?|(?:尚|还)?未(?:曾)?|莫|别|禁止|默(?:默)?)[^，,；;。.!！？?]{0,12}(?:念出|念着|朗读|宣读|读出|读着)|默读|默念|脑海|心中|内心|心里|无声|不出声|没(?:有)?(?:出声|发出)|声音[^，,；;。.!！？?]{0,8}(?:没|未|不)").is_match(cue)&&re(r"^(?:条款|条目)[一二三四五六七八九十百\d]+$").is_match(label)
}
pub(super) fn closer(c: u16) -> Option<u16> {
    Some(match c {
        0x201c => 0x201d,
        0x2018 => 0x2019,
        0x300c => 0x300d,
        0x300e => 0x300f,
        34 => 34,
        39 => 39,
        0x3010 => 0x3011,
        91 => 93,
        _ => return None,
    })
}
pub(super) fn apostrophe(u: &[u16], i: usize, stack: &[u16]) -> bool {
    let c = u[i];
    let ascii = |x: u16| x <= 127 && (x as u8).is_ascii_alphanumeric();
    (c == 39 || c == 0x2019) && i > 0 && i + 1 < u.len() && ascii(u[i - 1]) && ascii(u[i + 1])
        || c == 39
            && !stack.contains(&39)
            && i > 0
            && matches!(u[i - 1], 83 | 115)
            && i + 1 < u.len()
            && char::from_u32(u[i + 1] as u32).is_some_and(ws)
}
pub fn inline_performance_projection(
    source: &str,
    parts: &Value,
    version: &str,
) -> Result<Option<Value>> {
    need(
        matches!(version, "inline-performance-v1" | "inline-performance-v2"),
        "STANDARD_PERFORMANCE_VERSION",
    )?;
    let Some(parts) = parts.as_array().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let mut size = 0;
    let mut mapped = vec![];
    for p in parts {
        let a = n(&p["start"]).ok_or("STANDARD_PARTS")?;
        let b = n(&p["end"]).ok_or("STANDARD_PARTS")?;
        need(a <= b, "STANDARD_PARTS")?;
        mapped.push((p, size, size + b - a));
        size += b - a;
    }
    let text = copy_parts(source, &json!(parts))?;
    let u: Vec<_> = text.encode_utf16().collect();
    let cut = |a: usize, b: usize| -> Vec<Value> {
        mapped
            .iter()
            .filter_map(|(p, from, to)| {
                let x = a.max(*from);
                let y = b.min(*to);
                if x >= y {
                    return None;
                }
                let mut p = (*p).clone();
                let base = n(&p["start"]).unwrap();
                p["start"] = json!(base + x - from);
                p["end"] = json!(base + y - from);
                Some(p)
            })
            .collect()
    };
    let mut stack = vec![];
    let mut markers = vec![];
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
            stack.push(close);
            i += 1;
            continue;
        }
        if !stack.is_empty() || !matches!(c, 40 | 0xff08) {
            i += 1;
            continue;
        }
        let mut brackets = vec![if c == 40 { 41 } else { 0xff09 }];
        let mut end = i + 1;
        while end < u.len() && !brackets.is_empty() {
            let ch = u[end];
            if matches!(ch, 40 | 0xff08) {
                brackets.push(if ch == 40 { 41 } else { 0xff09 });
            } else if matches!(ch, 41 | 0xff09) {
                if brackets.last() != Some(&ch) {
                    break;
                }
                brackets.pop();
            }
            end += 1;
        }
        if !brackets.is_empty() {
            break;
        }
        let value = slice(&text, i + 1, end - 1)?;
        let marker = cut(i, end);
        let contiguous = marker.windows(2).all(|p| p[0]["end"] == p[1]["start"]);
        let literal = re(r"(?:叫|名为|称作|写着|写成|写的是|读作|读成|念作|念成|字样[：:]?)\s*$")
            .is_match(&slice(&text, 0, i)?)
            || re(r"^\s*(?:这(?:个|两个|几个)?字|这些字|这个词|这个字样|一词|二字|两个字|三个字)")
                .is_match(&tail(&text, end));
        if contiguous && explicit(&value, version == "inline-performance-v2") && !literal {
            markers.push((i, end, value, marker));
        }
        i = end;
    }
    if markers.is_empty() {
        return Ok(None);
    }
    let mut spoken_parts = vec![];
    let mut cues = vec![];
    let mut cursor = 0;
    let mut offset = 0;
    for (a, b, value, marker) in markers {
        spoken_parts.extend(cut(cursor, a));
        offset += a - cursor;
        let x = a + 1 + len(&value) - len(value.trim_start_matches(ws));
        let y = b - 1 - (len(&value) - len(value.trim_end_matches(ws)));
        cues.push(json!({"text":trim(&value),"parts":cut(x,y),"markerParts":marker,"spokenOffset":offset}));
        cursor = b;
    }
    spoken_parts.extend(cut(cursor, u.len()));
    let spoken = copy_parts(source, &json!(spoken_parts))?;
    if trim(&spoken).is_empty() {
        return Ok(None);
    }
    Ok(Some(
        json!({"version":version,"spoken":{"text":spoken,"parts":spoken_parts},"cues":cues}),
    ))
}
