//! Literal source speaker/header recognition from frozen standard-script-plan.mjs.
use super::*;
pub(super) const META: &str =
    r"^(?:出场人物|人物|角色|时间层|时间|地点|场景|天气|内外|道具)[ \t]*[：:]";
pub(super) const CAST: &str = r"^(?:出场人物|人物|角色)[ \t]*[：:]";
const ROLE_NOUN:&str="学生|同学|考生|考官|主考官|路人|保镖|保安|侍卫|士兵|警察|警员|医生|护士|老师|教师|记者|店员|顾客|观众|村民|群众|工作人员|助理|闺蜜|监控员|领导|院长|校长|主任|秘书|负责人|主持人|裁判|门卫|司机|服务员|售货员|军官|指挥官|首领|族长|长老|掌门|弟子|同门|修士|大能";
const ROLE_QUALIFIER:&str="围观|台上|台下|台边|场边|门口|门外|门内|后台|前排|后排|车内|身旁|身边|对面|路边|街边|楼上|楼下|远处|附近|现场|考场|大考|学院|医院|公司|警局|值班|当班|巡逻|随行|某|一名|一位|一个|一群|几名|几位|诸位|众|总|主|副";
const NOTE: &str =
    r"备注|说明|资料|规则|设定|要求|接口|梗概|概要|大纲|指南|免责声明|格式|校验|解析";
const LABEL: &str = r"(?i)(?:备注|说明|提示|事项|处理|要求|规则|设定|notes?|summary|metadata|instructions?|guidance|[说问道答喊叫])$";
pub(super) fn annotation(s: &str) -> Option<&'static str> {
    let r = re(r"^(导演处理|连续性备注|情绪备注)(?:[：:]|$)");
    let c = r.captures(s)?;
    Some(match &c[1] {
        "导演处理" => "CREATIVE_GUIDANCE",
        "连续性备注" => "CONTINUITY",
        _ => "EMOTION_CLUE",
    })
}
pub(super) fn valid_name(s: &str) -> bool {
    re(r"^[\p{L}\p{N}·・ _.-]+$").is_match(s) && !re(r"^\d+$").is_match(s)
}
fn speaker_number(s: &str) -> bool {
    re(r"^(?:[甲乙丙丁戊己庚辛壬癸]|[一二三四五六七八九十百]+|\d+|[A-Z])$").is_match(s)
}
fn personal_name(s: &str) -> bool {
    re(r"^(?:(?:欧阳|司马|上官|诸葛|夏侯|东方|公孙|慕容|宇文|令狐|司徒|南宫|独孤)|[赵钱孙李周吴郑王冯陈沈韩杨朱秦许何吕张曹严华金魏姜谢邹苏潘葛范彭鲁韦马方俞任袁柳史唐薛雷贺倪汤殷罗郝安常于傅齐康伍余顾孟黄穆萧尹姚邵汪毛成戴宋庞熊舒董梁杜阮季贾江童颜郭梅林钟徐邱骆高夏蔡田胡凌霍万卢莫丁邓洪石崔程陆段侯宁刘龙叶黎白乔谭温庄阎向易廖文冷辛曾关])\p{Han}{1,2}$").is_match(s)
}
fn source_role(s: &str) -> bool {
    re(&format!(r"(?i)^(?:{ROLE_QUALIFIER}){{0,2}}(?:男|女)?(?:{ROLE_NOUN})(?:[甲乙丙丁戊己庚辛壬癸]|[一二三四五六七八九十百]+|\d+|[A-Z])?$")).is_match(s)
}
pub(super) fn explicit_speaker(name: &str, known: &[String]) -> bool {
    if let Some(c)=re(&format!(r"(?i)^(\p{{Han}}{{0,10}})(?:{ROLE_NOUN}|观察者|队员|研究员|官员|新闻主持人|副官|老兵|男孩|女孩|网友|手下|府帅|主席|拘使|神将|守将|冥君|兵|君|将|百姓|神王|神兵|守卫|师生|生灵)(?:[甲乙丙丁戊己庚辛壬癸]|\d+|[A-Z])?$")).captures(name){if !re(NOTE).is_match(name)&&(utf16_len(&c[1])<=4||re(r"(?:军府|联盟|高校|大学|学院|医院|公司|新闻|王国|帝国|城|州|界|庭)$").is_match(&c[1])||re(r"^(?:其他|其余)[一二三四五六七八九十百\d]+位?\p{Han}{0,2}$").is_match(&c[1])){return true}}
    if let Some(c) = re(r"^\p{Han}{1,10}的(.+)$").captures(name) {
        if !re(NOTE).is_match(name)
            && (contains(known, &c[1]) || personal_name(&c[1]) || source_role(&c[1]))
        {
            return true;
        }
    }
    if let Some(c) = re(r"(?i)^(?:Q版(.+)|(.+)Q版)$").captures(name) {
        let base = c.get(1).or_else(|| c.get(2)).unwrap().as_str();
        return !re(r"(?i)Q版").is_match(base)
            && !re(LABEL).is_match(base)
            && (contains(known, base) || explicit_speaker(base, known));
    }
    speaker_number(name)||re(r"^(?:(?:围观)?学生|路人|保镖|保安|侍卫|士兵|警察|警员|医生|护士|老师|同学|记者|店员|顾客|观众|考官|考生|村民|群众|工作人员|男|女)(?:[甲乙丙丁戊己庚辛壬癸]|[一二三四五六七八九十百]+|\d+|[A-Z])$").is_match(name)||source_role(name)||re(r"^(?:众人|众学生|众同学|众考生|众士兵|众群众|众工作人员|众修|诸修|[二三四五六七八九十两\d]+人)$").is_match(name)||re(r"^(?:系统|系统窗口|系统提示窗口)$").is_match(name)||personal_name(name)||re(r"^[A-Z][a-z]+(?:[ -][A-Z][a-z]+)*$").is_match(name)||known.iter().any(|g|name.strip_prefix(g).is_some_and(speaker_number))
}
pub(super) fn cast_names(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut start = 0;
    let mut close = None;
    for (i, c) in s.char_indices() {
        if let Some(k) = close {
            if c == k {
                close = None
            }
            continue;
        }
        if matches!(c, '（' | '(') {
            close = Some(if c == '（' { '）' } else { ')' });
            continue;
        }
        if "、,，;；".contains(c) {
            out.push(trim(&s[start..i]).to_string());
            start = i + c.len_utf8()
        }
    }
    out.push(trim(&s[start..]).to_string());
    out
}
fn trim_range(s: &str, start: usize) -> Span {
    Span {
        start: start + utf16_len(s) - utf16_len(s.trim_start_matches(ws)),
        end: start + utf16_len(s.trim_end_matches(ws)),
    }
}
pub(super) fn performance_cue(s: &str) -> bool {
    re(r"[,，、;；]").split(trim(s)).all(|x|re(r"^(?:低声|轻声|小声|大声|压低声音|提高声音|压住颤音|哽咽|哭腔|带哭腔|气喘|喘息|咬牙|大笑|苦笑|冷笑|轻笑|笑着|哭着|怒视|皱眉|挑眉|点头|摇头|抬头|低头|转头|转身|起身|蹲下|抬手|摆手|挥手|握拳|攥拳|摊手|耸肩|抿嘴|闭眼|睁眼|深吸一口气|叹气|掐着(?:手指|指头)|恍然大悟|恍然大悟的样子)$").is_match(trim(x)))
}
fn head_cue(s: &str) -> bool {
    performance_cue(s)||re(r"^(?:(?:微微|轻轻|忽然|突然|忍不住)?(?:憋笑|怒吼|吼道|笑道|听完|听罢|精神传音)|(?:声音|嗓音)(?:微微|略微|不住地)?(?:发抖|颤抖|沙哑)|笑容(?:瞬间|突然|渐渐)?(?:僵住|僵死|消失|凝固)|(?:急|气|笑|哭)得(?:眼圈发红|满脸通红|浑身发抖|直跺脚|说不出话)|(?:这时|此时|突然)?(?:再次)?(?:提示|提醒|通知))$").is_match(trim(s))
}
pub(super) fn voice_head(s: &str) -> Option<(String, String)> {
    let r = re(r"(?i)^(.+?)(OS|VO)$");
    let c = r.captures(s)?;
    if !re(r"(?:\p{Han}|[\t \x{a0}\x{3000}])$").is_match(&c[1]) {
        return None;
    }
    let speaker = c[1].trim_end_matches(ws);
    if speaker.is_empty() {
        None
    } else {
        Some((speaker.to_string(), c[2].to_string()))
    }
}
pub(super) fn utterance_property(speaker: &str, label: &str, cue: &str) -> bool {
    re(r"^[\p{L}\p{N}]{1,16}(?:老样子|如下|很简单|是这样)$").is_match(label)||(re(r"^(?:系统|系统音|系统窗口|系统提示窗口)$").is_match(speaker)&&re(r"^(?:触发(?:终极)?(?:主线|支线)?任务|任务目标|任务奖励|解锁(?:全新)?(?:建筑|技能))$").is_match(label))||(re(r"(?:念出(?:了)?声(?:来)?|读出(?:了)?声(?:来)?|朗读|宣读|(?:大声|高声|低声|轻声|开口)(?:念着|读着))").is_match(cue)&&!re(r"(?:不(?:要|许|能|会|敢|再|想)?|没(?:有)?|(?:尚|还)?未(?:曾)?|莫|别|禁止|默(?:默)?)[^，,；;。.!！？?]{0,12}(?:念出|念着|朗读|宣读|读出|读着)|默读|默念|脑海|心中|内心|心里|无声|不出声|没(?:有)?(?:出声|发出)|声音[^，,；;。.!！？?]{0,8}(?:没|未|不)").is_match(cue)&&re(r"^(?:条款|条目)[一二三四五六七八九十百\d]+$").is_match(label))
}
#[derive(Clone)]
struct Token {
    text: String,
    span: Span,
}
fn tokens(s: &str, start: usize) -> Vec<Token> {
    re(r"[^,， \t]+")
        .find_iter(s)
        .map(|m| Token {
            text: m.as_str().to_string(),
            span: trim_range(m.as_str(), start + utf16_len(&s[..m.start()])),
        })
        .collect()
}
fn channel(s: &str) -> bool {
    re(r"(?i)^(OS|VO)$").is_match(s)
}
fn channelish(s: &str) -> bool {
    re(r"(?i)(?-u:\b)(?:OS|VO)(?-u:\b)|(?-u:\b)(?:O\.S\.?|V\.O\.?)|旁白|画外音|内心独白|系统音")
        .is_match(s)
}
pub(super) fn diagnostic(line: &Line, code: &str, message: &str) -> Value {
    json!({"line":line.number,"code":code,"message":message})
}
pub(super) fn read_head(
    line: &Line,
    known: &[String],
    syntax: &str,
    head_cues: bool,
    whole: &[String],
) -> Option<std::result::Result<Head, Value>> {
    let external_re = re(
        r"(?i)^([^:：()（）]+?)[（(]([^()（）]*)[）)][\t \x{a0}\x{3000}]*(OS|VO)[\t \x{a0}\x{3000}]*(?:([:：])([\s\S]*)|$)$",
    );
    let external = if syntax == "current" {
        external_re.captures(&line.content)
    } else {
        None
    };
    let normal_re = re(r"^([^:：()（）]+?)(?:[（(]([^()（）]*)[）)])?[ \t]*(?:([:：])([\s\S]*)|$)");
    let normal = if external.is_none() {
        normal_re.captures(&line.content)
    } else {
        None
    };
    if external.is_none() && normal.is_none() {
        return None;
    }
    let (name_raw, meta, colon, body) = if let Some(c) = &external {
        (
            c.get(1)?.as_str(),
            c.get(2).map(|m| m.as_str()),
            c.get(4).map(|m| m.as_str()),
            c.get(5).map(|m| m.as_str()),
        )
    } else {
        let c = normal.as_ref()?;
        (
            c.get(1)?.as_str(),
            c.get(2).map(|m| m.as_str()),
            c.get(3).map(|m| m.as_str()),
            c.get(4).map(|m| m.as_str()),
        )
    };
    let literal_end = colon
        .and_then(|c| line.content.find(c))
        .unwrap_or(line.content.len());
    let literal = &line.content[..literal_end];
    let identity = if let Some(c) = &external {
        &line.content[..c.get(3)?.start()]
    } else {
        literal
    };
    let identity_head = trim(identity);
    let compact = if syntax == "current" && colon.is_some() {
        voice_head(trim(name_raw))
    } else {
        None
    };
    let qualified = syntax == "current"
        && compact.is_none()
        && meta.is_some_and(|m| !m.is_empty())
        && !re(r"(?i)^(?:OS|VO|旁白|画外音|内心独白|系统音)$").is_match(trim(meta.unwrap_or("")))
        && contains(known, identity_head);
    let mut name = if qualified {
        identity_head.to_string()
    } else if let Some(c) = &compact {
        c.0.clone()
    } else {
        trim(name_raw).to_string()
    };
    let mut compact_cue = None;
    if head_cues && !contains(whole, &name) {
        for (at, _) in name.char_indices().skip(1) {
            if re(LABEL).is_match(&name[..at]) && performance_cue(&name[at..]) {
                return None;
            }
        }
        let mut candidates: Vec<String> = known
            .iter()
            .filter(|s| {
                name.starts_with(s.as_str()) && name.len() > s.len() && head_cue(&name[s.len()..])
            })
            .cloned()
            .collect();
        if candidates.is_empty() && compact.is_none() {
            for (at, _) in name.char_indices().skip(1) {
                if explicit_speaker(&name[..at], known) && head_cue(&name[at..]) {
                    candidates.push(name[..at].to_string())
                }
            }
        }
        if candidates.iter().any(|s| {
            re(LABEL).is_match(s)
                || re(META).is_match(&format!("{s}："))
                || annotation(&format!("{s}：")).is_some()
        }) {
            return None;
        }
        if candidates.len() == 1 {
            let start = trim_range(name_raw, line.start).start;
            compact_cue = Some(Span {
                start: start + utf16_len(&candidates[0]),
                end: start + utf16_len(compact.as_ref().map(|c| c.0.as_str()).unwrap_or(&name)),
            });
            name = candidates[0].clone();
        }
    }
    let split = re(r"、|以及|及");
    let members: Vec<_> = split.split(&name).collect();
    let joint = syntax == "current"
        && members.len() > 1
        && members.iter().all(|p| {
            valid_name(trim(p)) && (contains(known, trim(p)) || explicit_speaker(trim(p), known))
        });
    if !valid_name(&name) && !qualified && !joint {
        return None;
    }
    if syntax == "current"
        && (re(META).is_match(&format!("{name}：")) || annotation(&format!("{name}：")).is_some())
    {
        return None;
    }
    if syntax == "current"
        && (!contains(known, &name) && re(LABEL).is_match(&name)
            || head_cues && !contains(whole, &name) && re(NOTE).is_match(&name))
    {
        return None;
    }
    let metadata_start = line.start + utf16_len(name_raw) + 1;
    let mut toks = tokens(
        if qualified { "" } else { meta.unwrap_or("") },
        metadata_start,
    );
    if let Some(c) = compact_cue {
        toks.insert(
            0,
            Token {
                text: sub(&line.content, c.start - line.start, c.end - line.start),
                span: c,
            },
        );
    }
    let compact_range = compact.as_ref().map(|_| {
        let end = trim_range(name_raw, line.start).end;
        Span {
            start: end - 2,
            end,
        }
    });
    if let Some(c) = compact_range {
        toks.insert(
            0,
            Token {
                text: compact.as_ref().unwrap().1.clone(),
                span: c,
            },
        );
    }
    if let Some(c) = &external {
        let m = c.get(3)?;
        toks.push(Token {
            text: m.as_str().into(),
            span: Span {
                start: line.start + utf16_len(&line.content[..m.start()]),
                end: line.start + utf16_len(&line.content[..m.end()]),
            },
        });
    }
    let channels: Vec<_> = toks.iter().filter(|t| channel(&t.text)).collect();
    let body_start = if colon.is_some() {
        let b = body.unwrap_or("");
        line.start + utf16_len(&line.content) - utf16_len(b)
            + utf16_len(re(r"^[ \t]*").find(b).unwrap().as_str())
    } else {
        line.start + utf16_len(&line.content)
    };
    let mut post_end = body_start;
    let mut post_tokens = vec![];
    if syntax == "current"
        && colon.is_some()
        && re(r"^[（(]").is_match(&sub(
            &line.content,
            body_start - line.start,
            utf16_len(&line.content),
        ))
    {
        let rest = sub(
            &line.content,
            body_start - line.start,
            utf16_len(&line.content),
        );
        let post_re = re(r"^(?:（([^()（）\r\n]*)）|\(([^()（）\r\n]*)\))[\t ]*");
        let post = post_re.captures(&rest);
        let value = post
            .as_ref()
            .and_then(|p| p.get(1).or_else(|| p.get(2)))
            .map(|m| m.as_str())
            .unwrap_or("");
        if post.is_none()
            || trim(value).is_empty()
            || re(r#"[:：“”‘’「」『』"'△▲]"#).is_match(value)
        {
            return Some(Err(diagnostic(line,"UNSUPPORTED_POST_CUE","后置括号提示为空、嵌套、引述或含说者分隔符；原句保留待核对，不能当台词或悄悄丢弃。")));
        }
        post_end = body_start + utf16_len(&post.as_ref().unwrap()[0]);
        if re(r"^[（(]").is_match(&sub(
            &line.content,
            post_end - line.start,
            utf16_len(&line.content),
        )) {
            return Some(Err(diagnostic(
                line,
                "AMBIGUOUS_POST_CUE",
                "存在连续两段后置提示，须明确归属，不能合并猜测。",
            )));
        }
        // Unlike the pre-colon tokens, the frozen post-colon scanner retains
        // NBSP/ideographic/BOM whitespace inside each source token's range.
        for token in re(r"[^,， \t]+").find_iter(value).map(|m| Token {
            text: m.as_str().into(),
            span: Span {
                start: body_start + 1 + utf16_len(&value[..m.start()]),
                end: body_start + 1 + utf16_len(&value[..m.end()]),
            },
        }) {
            if re(r"(?i)^(?:OS|VO)\p{Han}").is_match(&token.text) {
                post_tokens.push(Token {
                    text: token.text[..2].into(),
                    span: Span {
                        start: token.span.start,
                        end: token.span.start + 2,
                    },
                });
                post_tokens.push(Token {
                    text: token.text[2..].into(),
                    span: Span {
                        start: token.span.start + 2,
                        end: token.span.end,
                    },
                })
            } else {
                post_tokens.push(token)
            }
        }
        if post_tokens
            .iter()
            .any(|t| !channel(&t.text) && channelish(&t.text))
        {
            return Some(Err(diagnostic(
                line,
                "UNSUPPORTED_POST_CHANNEL",
                "后置声道标记未使用可明确定位的OS/VO格式；不静默改为现场对白。",
            )));
        }
    }
    let post_channels: Vec<_> = post_tokens.iter().filter(|t| channel(&t.text)).collect();
    let audio = match name.as_str() {
        "旁白" => Some("NARRATION"),
        "画外音" => Some("VO"),
        "系统音" => Some("SYSTEM"),
        _ => None,
    };
    let explicit =
        syntax == "current" && colon.is_some() && (joint || explicit_speaker(&name, known));
    if !contains(known, &name)
        && channels.is_empty()
        && post_channels.is_empty()
        && audio.is_none()
        && !explicit
    {
        return if !trim(meta.unwrap_or("")).is_empty() || !post_tokens.is_empty() {
            Some(Err(diagnostic(
                line,
                "UNKNOWN_SPEAKER",
                "括号提示不能证明说者身份；请提供原文人物声明或已确认的 knownSpeakers。",
            )))
        } else {
            None
        };
    }
    if syntax == "current"
        && toks
            .iter()
            .any(|t| !channel(&t.text) && channelish(&t.text))
    {
        return Some(Err(diagnostic(
            line,
            "UNSUPPORTED_CHANNEL",
            "声道标记未使用可明确定位的OS/VO格式；原文保留待核对，不静默改为现场对白。",
        )));
    }
    let mut speaker = trim_range(if qualified { identity } else { name_raw }, line.start);
    if compact_range.is_some() || compact_cue.is_some() {
        speaker.end = compact_cue
            .map(|c| c.start)
            .unwrap_or_else(|| speaker.start + utf16_len(&compact.as_ref().unwrap().0));
    }
    let mut head = Head {
        speaker,
        channel: audio.unwrap_or("SPEECH").into(),
        body_start,
        cue: None,
        channel_source: None,
    };
    if (!qualified || external.is_some())
        && (!trim(meta.unwrap_or("")).is_empty()
            || compact_range.is_some()
            || compact_cue.is_some()
            || external.is_some())
    {
        if compact_cue.is_some()
            && toks
                .iter()
                .any(|t| t.span.start >= metadata_start && !channel(&t.text))
        {
            return Some(Err(diagnostic(
                line,
                "UNSUPPORTED_CUE_LAYOUT",
                "裸提示与括号提示分属两段，不能将括号标记混入单一表演字段。",
            )));
        }
        if channels.len() > 1 {
            return Some(Err(diagnostic(
                line,
                "AMBIGUOUS_CHANNEL",
                "同一说者头含多个声道标记，需确认原文声道。",
            )));
        }
        if let Some(c) = channels.first() {
            head.channel = c.text.to_uppercase();
            head.channel_source = Some(c.span);
            let cues: Vec<_> = toks.iter().filter(|t| !std::ptr::eq(*t, *c)).collect();
            if !cues.is_empty()
                && c.span.start > cues[0].span.start
                && c.span.end < cues.last().unwrap().span.end
            {
                return Some(Err(diagnostic(
                    line,
                    "UNSUPPORTED_CUE_LAYOUT",
                    "声道插在两段提示之间，当前连续切片无法安全拆分提示。",
                )));
            }
            if !cues.is_empty() {
                head.cue = Some(Span {
                    start: cues[0].span.start,
                    end: cues.last().unwrap().span.end,
                })
            }
        } else {
            head.cue =
                Some(compact_cue.unwrap_or_else(|| trim_range(meta.unwrap_or(""), metadata_start)));
        }
    }
    if !post_tokens.is_empty() {
        let cues: Vec<_> = post_tokens.iter().filter(|t| !channel(&t.text)).collect();
        if channels.len() + post_channels.len() > 1 {
            return Some(Err(diagnostic(
                line,
                "AMBIGUOUS_CHANNEL",
                "前后提示含多个声道标记，须明确声道。",
            )));
        }
        if head.cue.is_some() && !cues.is_empty() {
            return Some(Err(diagnostic(
                line,
                "AMBIGUOUS_POST_CUE",
                "前置与后置均含表演提示，当前格式不猜测两段提示的组合。",
            )));
        }
        if let Some(c) = post_channels.first() {
            head.channel = c.text.to_uppercase();
            head.channel_source = Some(c.span);
            if !cues.is_empty()
                && c.span.start > cues[0].span.start
                && c.span.end < cues.last().unwrap().span.end
            {
                return Some(Err(diagnostic(
                    line,
                    "UNSUPPORTED_CUE_LAYOUT",
                    "后置声道插在两段提示之间，不能用连续切片吞掉声道。",
                )));
            }
        }
        if !cues.is_empty() {
            head.cue = Some(Span {
                start: cues[0].span.start,
                end: cues.last().unwrap().span.end,
            })
        }
        head.body_start = post_end;
    }
    Some(Ok(head))
}
pub(super) fn quoted(text: &str, brackets: bool) -> (Vec<usize>, bool) {
    let u: Vec<_> = text.encode_utf16().collect();
    let mut colons = vec![];
    let mut stack = vec![];
    let mut i = 0;
    let ascii = |c: u16| c <= 127 && (c as u8).is_ascii_alphanumeric();
    while i < u.len() {
        let c = u[i];
        if c == 92 {
            i += 2;
            continue;
        }
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
        let closer = match c {
            0x201c => Some(0x201d),
            0x2018 => Some(0x2019),
            0x300c => Some(0x300d),
            0x300e => Some(0x300f),
            34 => Some(34),
            39 => Some(39),
            0x3010 if brackets => Some(0x3011),
            91 if brackets => Some(93),
            _ => None,
        };
        if let Some(close) = closer {
            if !matches!(c, 0x3010 | 91)
                || stack.is_empty()
                || stack.last().is_some_and(|c| matches!(c, 0x3011 | 93))
            {
                stack.push(close);
                i += 1;
                continue;
            }
        }
        if stack.is_empty() && matches!(c, 58 | 0xff1a) {
            colons.push(i)
        }
        i += 1
    }
    (colons, !stack.is_empty())
}
