import { compileStandardScript, createSourceIndex, explicitSubtitles, StandardScriptError, standardScriptPlanVersion } from './standard-script.mjs';
import { explicitPerformanceCue, sourceHeadPerformanceCue, sourceVoiceHead, sourceUtteranceProperty } from '../core/runtime/inline-performance.mjs';
const legacyEpisodeHeading = /^第[〇零一二三四五六七八九十百千万两\d]+集(?:[ \t:：]|$)/u;
const legacySceneHeading = /^\d+[ \t]*-[ \t]*\d+(?:[ \t]|$)/u;
const castHeading = /^(?:出场人物|人物|角色)[ \t]*[：:]/u;
const sceneMetadataHeading = /^(?:出场人物|人物|角色|时间层|时间|地点|场景|天气|内外|道具)[ \t]*[：:]/u;
const typedMarker = /^[【[]/u;
const decimalSceneHeading = /^\d+[ \t]*\.[ \t]*\d+[\t \u00a0]*(?:日|夜|晨|昏|清晨|傍晚|凌晨|上午|下午|中午|深夜)[\t \u00a0,，、]+(?:内外|内|外)[\t \u00a0,，、]+\S/u;
const productionGuidance = (text)=>/^【(?:镜头切|[一二三四五六七八九十\d]+卡)】[ \t]*$/u.test(text) || Boolean(/^【镜头切[：:]([^【】\r\n]+)】[ \t]*$/u.exec(text)?.[1].trim());
const annotationKinds = {
    导演处理: 'CREATIVE_GUIDANCE',
    连续性备注: 'CONTINUITY',
    情绪备注: 'EMOTION_CLUE'
};
const audioRoles = {
    旁白: 'NARRATION',
    画外音: 'VO',
    系统音: 'SYSTEM'
};
const annotationKind = (text)=>annotationKinds[/^(导演处理|连续性备注|情绪备注)(?:[：:]|$)/u.exec(text)?.[1] ?? ''];
const validName = (text)=>/^[\p{L}\p{N}·・ _.-]+$/u.test(text) && !/^\d+$/.test(text);
const trimRange = (text, start)=>({
        start: start + text.length - text.trimStart().length,
        end: start + text.trimEnd().length
    });
const speakerNumber = /^(?:[甲乙丙丁戊己庚辛壬癸]|[一二三四五六七八九十百]+|\d+|[A-Z])$/u;
const numberedRole = /^(?:(?:围观)?学生|路人|保镖|保安|侍卫|士兵|警察|警员|医生|护士|老师|同学|记者|店员|顾客|观众|考官|考生|村民|群众|工作人员|男|女)(?:[甲乙丙丁戊己庚辛壬癸]|[一二三四五六七八九十百]+|\d+|[A-Z])$/u;
const sourceRoleNoun = '学生|同学|考生|考官|主考官|路人|保镖|保安|侍卫|士兵|警察|警员|医生|护士|老师|教师|记者|店员|顾客|观众|村民|群众|工作人员|助理|闺蜜|监控员|领导|院长|校长|主任|秘书|负责人|主持人|裁判|门卫|司机|服务员|售货员|军官|指挥官|首领|族长|长老|掌门|弟子|同门|修士|大能';
const sourceRoleQualifier = '围观|台上|台下|台边|场边|门口|门外|门内|后台|前排|后排|车内|身旁|身边|对面|路边|街边|楼上|楼下|远处|附近|现场|考场|大考|学院|医院|公司|警局|值班|当班|巡逻|随行|某|一名|一位|一个|一群|几名|几位|诸位|众|总|主|副';
const sourceRole = new RegExp(`^(?:${sourceRoleQualifier}){0,2}(?:男|女)?(?:${sourceRoleNoun})(?:[甲乙丙丁戊己庚辛壬癸]|[一二三四五六七八九十百]+|\\d+|[A-Z])?$`, 'iu');
const collectiveRole = /^(?:众人|众学生|众同学|众考生|众士兵|众群众|众工作人员|众修|诸修|[二三四五六七八九十两\d]+人)$/u;
const incidentalRole = new RegExp(`^(\\p{Script=Han}{0,10})(?:${sourceRoleNoun}|观察者|队员|研究员|官员|新闻主持人|副官|老兵|男孩|女孩|网友|手下|府帅|主席|拘使|神将|守将|冥君|兵|君|将|百姓|神王|神兵|守卫|师生|生灵)(?:[甲乙丙丁戊己庚辛壬癸]|\\d+|[A-Z])?$`, 'iu');
const noteQualifier = /备注|说明|资料|规则|设定|要求|接口|梗概|概要|大纲|指南|免责声明|格式|校验|解析/u;
const screenTextHeading = /^(?:弹幕|字幕|屏幕文字)(?:（[^()（）\r\n]+）|\([^()（）\r\n]+\))?[\t \u00a0\u3000]*[:：]/u;
const omissionMarker = /^(?:\.{3,}|．{3,}|…{2,})[\t ]*$/u;
const sourceSystemRole = /^(?:系统|系统窗口|系统提示窗口)$/u;
const personalName = /^(?:(?:欧阳|司马|上官|诸葛|夏侯|东方|公孙|慕容|宇文|令狐|司徒|南宫|独孤)|[赵钱孙李周吴郑王冯陈沈韩杨朱秦许何吕张曹严华金魏姜谢邹苏潘葛范彭鲁韦马方俞任袁柳史唐薛雷贺倪汤殷罗郝安常于傅齐康伍余顾孟黄穆萧尹姚邵汪毛成戴宋庞熊舒董梁杜阮季贾江童颜郭梅林钟徐邱骆高夏蔡田胡凌霍万卢莫丁邓洪石崔程陆段侯宁刘龙叶黎白乔谭温庄阎向易廖文冷辛曾关])\p{Script=Han}{1,2}$/u;
const sourceLabel = /(?:备注|说明|提示|事项|处理|要求|规则|设定|notes?|summary|metadata|instructions?|guidance|[说问道答喊叫])$/iu;
function explicitSpeaker(name, knownSpeakers) {
    const role = incidentalRole.exec(name);
    if (role && !noteQualifier.test(name) && (role[1].length <= 4 || /(?:军府|联盟|高校|大学|学院|医院|公司|新闻|王国|帝国|城|州|界|庭)$/u.test(role[1]) || /^(?:其他|其余)[一二三四五六七八九十百\d]+位?\p{Script=Han}{0,2}$/u.test(role[1]))) return true;
    const qualified = /^\p{Script=Han}{1,10}的(.+)$/u.exec(name);
    if (qualified && !noteQualifier.test(name) && (knownSpeakers.has(qualified[1]) || personalName.test(qualified[1]) || sourceRole.test(qualified[1]))) return true;
    const comic = /^(?:Q版(.+)|(.+)Q版)$/iu.exec(name), base = comic?.[1] ?? comic?.[2];
    if (base) return !/Q版/iu.test(base) && !sourceLabel.test(base) && (knownSpeakers.has(base) || explicitSpeaker(base, knownSpeakers));
    return speakerNumber.test(name) || numberedRole.test(name) || sourceRole.test(name) || collectiveRole.test(name) || sourceSystemRole.test(name) || personalName.test(name) || /^[A-Z][a-z]+(?:[ -][A-Z][a-z]+)*$/.test(name) || [
        ...knownSpeakers
    ].some((group)=>name.startsWith(group) && speakerNumber.test(name.slice(group.length)));
}
function castNames(text) {
    const names = [];
    let start = 0, close = '';
    for(let i = 0; i < text.length; i++){
        if (close) {
            if (text[i] === close) close = '';
            continue;
        }
        if (text[i] === '（' || text[i] === '(') {
            close = text[i] === '（' ? '）' : ')';
            continue;
        }
        if (/[、,，;；]/u.test(text[i])) {
            names.push(text.slice(start, i).trim());
            start = i + 1;
        }
    }
    names.push(text.slice(start).trim());
    return names;
}
function readHead(line, knownSpeakers, syntax, headCues = false, wholeSpeakers = knownSpeakers) {
    const external = syntax === 'current' ? /^([^:：()（）]+?)[（(]([^()（）]*)[）)][\t \u00a0\u3000]*(OS|VO)[\t \u00a0\u3000]*(?:([:：])([\s\S]*)|$)$/diu.exec(line.content) : null;
    const match = external ? [
        external[0],
        external[1],
        external[2],
        external[4],
        external[5]
    ] : /^([^:：()（）]+?)(?:[（(]([^()（）]*)[）)])?[ \t]*(?:([:：])([\s\S]*)|$)$/u.exec(line.content);
    if (!match) return null;
    const literalHeadRaw = line.content.slice(0, match[3] ? line.content.indexOf(match[3]) : line.content.length);
    const identityRaw = external ? line.content.slice(0, external.indices[3][0]) : literalHeadRaw;
    const identityHead = identityRaw.trim();
    const compact = syntax === 'current' && match[3] ? sourceVoiceHead(match[1].trim()) : null;
    const qualified = syntax === 'current' && !compact && Boolean(match[2]) && !/^(?:OS|VO|旁白|画外音|内心独白|系统音)$/iu.test(match[2].trim()) && knownSpeakers.has(identityHead);
    let name = qualified ? identityHead : compact?.speaker ?? match[1].trim();
    let compactCue = null;
    if (headCues && !wholeSpeakers.has(name)) {
        for(let at = 1; at < name.length; at++)if (sourceLabel.test(name.slice(0, at)) && explicitPerformanceCue(name.slice(at))) return null;
        const candidates = [
            ...knownSpeakers
        ].filter((speaker)=>name.startsWith(speaker) && name.length > speaker.length && sourceHeadPerformanceCue(name.slice(speaker.length)));
        if (!candidates.length && !compact) {
            for(let at = 1; at < name.length; at++)if (explicitSpeaker(name.slice(0, at), knownSpeakers) && sourceHeadPerformanceCue(name.slice(at))) candidates.push(name.slice(0, at));
        }
        if (candidates.some((speaker)=>sourceLabel.test(speaker) || sceneMetadataHeading.test(`${speaker}：`) || annotationKind(`${speaker}：`))) return null;
        if (candidates.length === 1) {
            const from = trimRange(match[1], line.start).start + candidates[0].length;
            compactCue = {
                start: from,
                end: trimRange(match[1], line.start).start + (compact?.speaker ?? name).length
            };
            name = candidates[0];
        }
    }
    const members = name.split(/、|以及|及/u);
    const joint = syntax === 'current' && members.length > 1 && members.every((part)=>validName(part.trim()) && (knownSpeakers.has(part.trim()) || explicitSpeaker(part.trim(), knownSpeakers)));
    if (!validName(name) && !qualified && !joint) return null;
    if (syntax === 'current' && (sceneMetadataHeading.test(`${name}：`) || annotationKind(`${name}：`))) return null;
    if (syntax === 'current' && (!knownSpeakers.has(name) && sourceLabel.test(name) || headCues && !wholeSpeakers.has(name) && noteQualifier.test(name))) return null;
    const metadataStart = line.start + match[1].length + 1;
    const tokens = [
        ...(qualified ? '' : match[2] ?? '').matchAll(/[^,， \t]+/g)
    ].map((token)=>({
            text: token[0],
            ...trimRange(token[0], metadataStart + token.index)
        }));
    if (compactCue) tokens.unshift({
        text: line.content.slice(compactCue.start - line.start, compactCue.end - line.start),
        ...compactCue
    });
    const compactRange = compact ? {
        start: trimRange(match[1], line.start).end - 2,
        end: trimRange(match[1], line.start).end
    } : null;
    if (compactRange) tokens.unshift({
        text: compact.text,
        ...compactRange
    });
    if (external) tokens.push({
        text: external[3],
        start: line.start + external.indices[3][0],
        end: line.start + external.indices[3][1]
    });
    const channels = tokens.filter((token)=>/^(OS|VO)$/i.test(token.text));
    const bodyStart = match[3] ? line.start + line.content.length - (match[4]?.length ?? 0) + (/^[ \t]*/.exec(match[4] ?? '')?.[0].length ?? 0) : line.start + line.content.length;
    let postEnd = bodyStart, postTokens = [];
    if (syntax === 'current' && match[3] && /^[（(]/.test(line.content.slice(bodyStart - line.start))) {
        const post = /^(?:（([^()（）\r\n]*)）|\(([^()（）\r\n]*)\))[\t ]*/u.exec(line.content.slice(bodyStart - line.start));
        if (!post || !(post[1] ?? post[2]).trim() || /[:：“”‘’「」『』"'△▲]/u.test(post[1] ?? post[2])) return {
            line: line.number,
            code: 'UNSUPPORTED_POST_CUE',
            message: '后置括号提示为空、嵌套、引述或含说者分隔符；原句保留待核对，不能当台词或悄悄丢弃。'
        };
        postEnd = bodyStart + post[0].length;
        if (/^[（(]/.test(line.content.slice(postEnd - line.start))) return {
            line: line.number,
            code: 'AMBIGUOUS_POST_CUE',
            message: '存在连续两段后置提示，须明确归属，不能合并猜测。'
        };
        postTokens = [
            ...(post[1] ?? post[2]).matchAll(/[^,， \t]+/g)
        ].flatMap((token)=>{
            const start = bodyStart + 1 + token.index, end = start + token[0].length;
            return /^(?:OS|VO)(?=\p{Script=Han})/iu.test(token[0]) ? [
                {
                    text: token[0].slice(0, 2),
                    start,
                    end: start + 2
                },
                {
                    text: token[0].slice(2),
                    start: start + 2,
                    end
                }
            ] : [
                {
                    text: token[0],
                    start,
                    end
                }
            ];
        });
        if (postTokens.some((token)=>!/^(OS|VO)$/i.test(token.text) && /\b(?:OS|VO)\b|\b(?:O\.S\.?|V\.O\.?)|旁白|画外音|内心独白|系统音/i.test(token.text))) return {
            line: line.number,
            code: 'UNSUPPORTED_POST_CHANNEL',
            message: '后置声道标记未使用可明确定位的OS/VO格式；不静默改为现场对白。'
        };
    }
    const postChannels = postTokens.filter((token)=>/^(OS|VO)$/i.test(token.text));
    const audioRole = Object.hasOwn(audioRoles, name) ? audioRoles[name] : undefined;
    const explicit = syntax === 'current' && Boolean(match[3]) && (joint || explicitSpeaker(name, knownSpeakers));
    if (!knownSpeakers.has(name) && !channels.length && !postChannels.length && !audioRole && !explicit) return match[2]?.trim() || postTokens.length ? {
        line: line.number,
        code: 'UNKNOWN_SPEAKER',
        message: '括号提示不能证明说者身份；请提供原文人物声明或已确认的 knownSpeakers。'
    } : null;
    if (syntax === 'current' && tokens.some((token)=>!/^(OS|VO)$/i.test(token.text) && /\b(?:OS|VO)\b|\b(?:O\.S\.?|V\.O\.?)|旁白|画外音|内心独白|系统音/i.test(token.text))) return {
        line: line.number,
        code: 'UNSUPPORTED_CHANNEL',
        message: '声道标记未使用可明确定位的OS/VO格式；原文保留待核对，不静默改为现场对白。'
    };
    const speaker = trimRange(qualified ? identityRaw : match[1], line.start);
    if (compactRange || compactCue) speaker.end = compactCue?.start ?? speaker.start + compact.speaker.length;
    const head = {
        speaker,
        channel: audioRole ?? 'SPEECH',
        bodyStart
    };
    if ((!qualified || external) && (match[2]?.trim() || compactRange || compactCue || external)) {
        if (compactCue && tokens.some((token)=>token.start >= metadataStart && !/^(OS|VO)$/i.test(token.text))) return {
            line: line.number,
            code: 'UNSUPPORTED_CUE_LAYOUT',
            message: '裸提示与括号提示分属两段，不能将括号标记混入单一表演字段。'
        };
        if (channels.length > 1) return {
            line: line.number,
            code: 'AMBIGUOUS_CHANNEL',
            message: '同一说者头含多个声道标记，需确认原文声道。'
        };
        if (channels.length) {
            const channel = channels[0];
            head.channel = channel.text.toUpperCase();
            head.channelSource = {
                start: channel.start,
                end: channel.end
            };
            const cues = tokens.filter((token)=>token !== channel);
            if (cues.length && channel.start > cues[0].start && channel.end < cues.at(-1).end) return {
                line: line.number,
                code: 'UNSUPPORTED_CUE_LAYOUT',
                message: '声道插在两段提示之间，当前连续切片无法安全拆分提示。'
            };
            if (cues.length) head.cue = {
                start: cues[0].start,
                end: cues.at(-1).end
            };
        } else head.cue = compactCue ?? trimRange(match[2], metadataStart);
    }
    if (postTokens.length) {
        const cues = postTokens.filter((token)=>!postChannels.includes(token));
        if (channels.length + postChannels.length > 1) return {
            line: line.number,
            code: 'AMBIGUOUS_CHANNEL',
            message: '前后提示含多个声道标记，须明确声道。'
        };
        if (head.cue && cues.length) return {
            line: line.number,
            code: 'AMBIGUOUS_POST_CUE',
            message: '前置与后置均含表演提示，当前格式不猜测两段提示的组合。'
        };
        if (postChannels.length) {
            const channel = postChannels[0];
            head.channel = channel.text.toUpperCase();
            head.channelSource = {
                start: channel.start,
                end: channel.end
            };
            if (cues.length && channel.start > cues[0].start && channel.end < cues.at(-1).end) return {
                line: line.number,
                code: 'UNSUPPORTED_CUE_LAYOUT',
                message: '后置声道插在两段提示之间，不能用连续切片吞掉声道。'
            };
        }
        if (cues.length) head.cue = {
            start: cues[0].start,
            end: cues.at(-1).end
        };
        head.bodyStart = postEnd;
    }
    return head;
}
function scanQuotedText(text, brackets = false) {
    const result = [], closers = [];
    const pairs = {
        '“': '”',
        '‘': '’',
        '「': '」',
        '『': '』',
        '"': '"',
        "'": "'",
        ...brackets ? {
            '【': '】',
            '[': ']'
        } : {}
    };
    for(let i = 0; i < text.length; i++){
        const char = text[i];
        if (char === '\\') {
            i++;
            continue;
        }
        if ((char === "'" || char === '’') && /[A-Za-z0-9]/.test(text[i - 1] ?? '') && /[A-Za-z0-9]/.test(text[i + 1] ?? '')) continue;
        if (closers.at(-1) === char) {
            closers.pop();
            continue;
        }
        if (char === "'" && !closers.includes("'") && /[sS]/.test(text[i - 1] ?? '') && /\s/.test(text[i + 1] ?? '')) continue;
        if (pairs[char] && (!/[【\[]/u.test(char) || !closers.length || /[】\]]/u.test(closers.at(-1)))) {
            closers.push(pairs[char]);
            continue;
        }
        if (!closers.length && /[:：]/.test(char)) result.push(i);
    }
    return {
        colons: result,
        open: closers.length > 0
    };
}
export function prepareStandardEpisode(text, sourceId, options = {}) {
    const index = createSourceIndex(text, sourceId);
    const plan = {
        schemaVersion: 'STANDARD_SCRIPT_PLAN_V1',
        sourceId: index.sourceId,
        sourceVersion: index.sourceVersion,
        entries: []
    };
    const diagnostics = [];
    const ambiguousLines = new Set();
    const syntax = options.syntax ?? (options.postColonCue === false ? 'legacy' : 'current');
    const performanceVersion = options.inlinePerformanceVersion === undefined ? 'inline-performance-v2' : options.inlinePerformanceVersion;
    if (![
        'inline-performance-v1',
        'inline-performance-v2'
    ].includes(performanceVersion)) throw new StandardScriptError('UNSUPPORTED_PERFORMANCE_VERSION', 'options.inlinePerformanceVersion');
    const headCues = syntax === 'current' && options.inlinePerformance !== false && performanceVersion === 'inline-performance-v2';
    const scanQuotes = (value)=>scanQuotedText(value, syntax === 'current');
    const unquotedColons = (value)=>scanQuotes(value).colons;
    const episodeHeading = syntax === 'legacy' ? legacyEpisodeHeading : /^第[〇零一二三四五六七八九十百千万两\d]+集(?:（[^()（）\r\n]+）|\([^()（）\r\n]+\))?(?:[ \t:：]|$)/u;
    const sceneHeading = syntax === 'legacy' ? legacySceneHeading : {
        test: (value)=>/^\d+[ \t]*-[ \t]*\d+(?:[ \t:：]|$)/u.test(value) || decimalSceneHeading.test(value) || /^(?:场景|SCENE)[\t :：]+[^\r\n:：]+$/iu.test(value)
    };
    const endMarker = (line)=>syntax === 'current' && /^【全剧终】[ \t]*$/u.test(line.content);
    const lines = index.units.map((unit, i)=>{
        const raw = unit.text.replace(/(?:\r\n|\r|\n)$/, ''), indent = /^[\t \uFEFF]*/.exec(raw)[0].length;
        return {
            number: i + 1,
            start: unit.start + indent,
            content: raw.slice(indent)
        };
    });
    const knownSpeakers = new Set((options.knownSpeakers ?? []).filter((name)=>typeof name === 'string' && name.trim()).map((name)=>name.trim()));
    const declaredSpeakers = new Set(knownSpeakers);
    for (const line of lines){
        if (castHeading.test(line.content)) {
            const names = syntax === 'legacy' ? line.content.replace(castHeading, '').split(/[、,，;；]/).map((name)=>name.trim()) : castNames(line.content.replace(castHeading, ''));
            for (const name of names){
                if (validName(name)) {
                    knownSpeakers.add(name);
                    declaredSpeakers.add(name);
                    continue;
                }
                if (syntax === 'legacy') continue;
                const form = /^([^()（）]+)(?:（([^()（）]+)）|\(([^()（）]+)\))$/u.exec(name);
                if (!form || !validName(form[1].trim())) continue;
                knownSpeakers.add(form[1].trim());
                declaredSpeakers.add(form[1].trim());
                const qualifier = (form[2] ?? form[3]).trim();
                if (validName(qualifier) && !/^(?:OS|VO|旁白|画外音|内心独白|系统音)$/iu.test(qualifier)) {
                    knownSpeakers.add(name);
                    declaredSpeakers.add(name);
                    knownSpeakers.add(form[1].trim() + qualifier);
                }
            }
        }
        const head = readHead(line, new Set(), syntax);
        if (head && 'speaker' in head) knownSpeakers.add(text.slice(head.speaker.start, head.speaker.end));
    }
    const wholeSpeakers = new Set(declaredSpeakers);
    if (headCues) for (const line of lines){
        const head = readHead(line, new Set(), syntax);
        if (head && 'speaker' in head && !head.channelSource) wholeSpeakers.add(text.slice(head.speaker.start, head.speaker.end));
    }
    const read = (line, names = knownSpeakers)=>readHead(line, names, syntax, headCues, wholeSpeakers);
    const actorAction = (line)=>syntax === 'current' && [
            ...knownSpeakers
        ].some((name)=>line.content.startsWith(name) && /^(?:(?:震惊|惊讶|惊愕|错愕|惊恐|疑惑|慌忙|微微|轻轻|猛然|猛地|忽然|突然|缓缓|立刻|随即|再次|不由得?|下意识)地?){0,2}(?:抬头|低头|转头|转身|起身|蹲下|抬手|摆手|挥手|握拳|攥拳|攥紧|摊手|耸肩|点头|摇头|闭眼|睁眼|后退|躬身|跪下)[^:：?？!！“”‘’「」『』"'（）()\r\n]*。$/u.test(line.content.slice(name.length)) && !/(?:说道|问道|喊道|答道|开口|表示|声称|告诉|认为|觉得|[吗呢吧啊]。)/u.test(line.content.slice(name.length)));
    const parts = (range)=>index.units.flatMap((unit)=>{
            const start = Math.max(range.start, unit.start), end = Math.min(range.end, unit.end);
            return start < end ? [
                {
                    kind: /^[\t \r\n]+$/.test(text.slice(start, end)) ? 'SOURCE_SEPARATOR' : 'SOURCE_SLICE',
                    unitId: unit.id,
                    start,
                    end
                }
            ] : [];
        });
    const mapped = (first, last, body, fields = [])=>{
        const ranges = [
            ...fields,
            body
        ].sort((a, b)=>a.start - b.start), layout = [];
        let cursor = index.units[first].start;
        for (const range of ranges){
            if (cursor < range.start) layout.push(...parts({
                start: cursor,
                end: range.start
            }));
            cursor = range.end;
        }
        if (cursor < index.units[last].end) layout.push(...parts({
            start: cursor,
            end: index.units[last].end
        }));
        return {
            id: `line-${first + 1}`,
            unitIds: index.units.slice(first, last + 1).map((unit)=>unit.id),
            parts: parts(body),
            layout
        };
    };
    const bodyEnd = (last)=>index.units[last].end - /[\t \r\n]*$/.exec(index.units[last].text)[0].length;
    const mixedBody = (first, last, start, end, head)=>{
        const scan = explicitSubtitles(text.slice(start, end));
        if (!scan.spans.length || scan.invalid) return null;
        const spoken = [];
        let cursor = start;
        const captions = scan.spans.map((s)=>{
            if (cursor < start + s.start) spoken.push(...parts({
                start: cursor,
                end: start + s.start
            }));
            cursor = start + s.end;
            return {
                kind: 'ONSCREEN_TEXT',
                parts: parts({
                    start: start + s.bodyStart,
                    end: start + s.bodyEnd
                }),
                label: parts({
                    start: start + s.labelStart,
                    end: start + s.labelEnd
                }),
                layout: [
                    ...parts({
                        start: start + s.start,
                        end: start + s.labelStart
                    }),
                    ...parts({
                        start: start + s.labelEnd,
                        end: start + s.bodyStart
                    }),
                    ...parts({
                        start: start + s.bodyEnd,
                        end: start + s.end
                    })
                ]
            };
        });
        if (cursor < end) spoken.push(...parts({
            start: cursor,
            end
        }));
        const fields = [
            head.speaker,
            head.cue,
            head.channelSource
        ].filter((r)=>Boolean(r));
        const original = mapped(first, last, {
            start,
            end
        }, fields);
        return {
            id: original.id,
            unitIds: original.unitIds,
            target: 'BODY',
            kind: 'BODY_GROUP',
            parts: parts({
                start: index.units[first].start,
                end: index.units[last].end
            }),
            layout: [],
            children: [
                {
                    kind: 'DIALOGUE',
                    parts: spoken,
                    layout: original.layout,
                    speaker: parts(head.speaker),
                    channel: head.channel,
                    cue: head.cue ? parts(head.cue) : null,
                    channelSource: head.channelSource ? parts(head.channelSource) : null
                },
                ...captions
            ]
        };
    };
    const structural = (line)=>episodeHeading.test(line.content) || sceneHeading.test(line.content) || sceneMetadataHeading.test(line.content) || endMarker(line) || Boolean(annotationKind(line.content)) || typedMarker.test(line.content) || syntax === 'current' && (screenTextHeading.test(line.content) || omissionMarker.test(line.content)) || /^[△▲]/.test(line.content);
    const error = (line, code, message)=>diagnostics.push({
            line: line.number,
            code,
            message
        });
    const inlineLabel = (prefix)=>(syntax === 'current' ? /(?:^|[。.!！？?;；,，…][”’」』"']?|[ \t])((?:[A-Z][a-z]+(?:[ -][A-Z][a-z]+)+|[^。.!！？?;；,，:：()（） \t\u00a0\u3000]+)(?:[\t \u00a0\u3000]+(?:OS|VO))?(?:[（(][^()（）]*[）)])?(?:[\t \u00a0\u3000]*(?:OS|VO))?)[ \t]*$/iu : /(?:^|[。.!！？?;；,， \t])([^。.!！？?;；,，:：()（） \t]+(?:[（(][^()（）]*[）)])?)[ \t]*$/u).exec(prefix)?.[1];
    const multipleSpeakers = (line, head)=>unquotedColons(line.content).some((colon)=>{
            const bodyStart = head.bodyStart;
            if (line.start + colon < bodyStart) return false;
            const prefix = line.content.slice(0, colon);
            if ([
                ...knownSpeakers
            ].some((name)=>{
                const at = prefix.lastIndexOf(name);
                if (at < 0 || line.start + at < bodyStart) return false;
                const candidate = read({
                    ...line,
                    start: line.start + at,
                    content: line.content.slice(at)
                });
                return Boolean(candidate && 'speaker' in candidate);
            })) return true;
            const tail = prefix.slice(bodyStart - line.start);
            if (syntax === 'legacy' && !/[。.!！？?;；,， \t]/u.test(tail)) return false;
            const afterBoundary = inlineLabel(tail);
            if (!afterBoundary) return false;
            const name = afterBoundary.replace(/[（(][^()（）]*[）)]$/u, '').trim();
            if (sourceUtteranceProperty(text.slice(head.speaker.start, head.speaker.end), name, head.cue ? text.slice(head.cue.start, head.cue.end) : '')) return false;
            if (/[说问道答喊叫]$/.test(name) && /^[“‘「『"']/.test(line.content.slice(colon + 1).trimStart())) return false;
            if (syntax === 'current' && annotationKind(`${name}：`)) return true;
            return validName(name) && !sceneMetadataHeading.test(`${name}：`) && !annotationKind(`${name}：`);
        });
    const dialogueGroup = (line, first, row, end)=>{
        if (syntax === 'legacy' && !declaredSpeakers.has(text.slice(first.speaker.start, first.speaker.end)) || scanQuotes(line.content).open) return null;
        const heads = [
            first
        ];
        for (const colon of unquotedColons(line.content)){
            if (line.start + colon < first.bodyStart) continue;
            const found = [];
            const label = syntax === 'current' ? inlineLabel(line.content.slice(0, colon)) : undefined;
            const rawName = label?.replace(/[（(][^()（）]*[）)]$/u, '').trim();
            const names = syntax === 'current' ? rawName ? [
                sourceVoiceHead(rawName)?.speaker ?? rawName
            ] : [] : declaredSpeakers;
            for (const name of names){
                const at = line.content.slice(0, colon).lastIndexOf(name);
                if (at < 0 || line.start + at < first.bodyStart) continue;
                const head = read({
                    ...line,
                    start: line.start + at,
                    content: line.content.slice(at)
                }, syntax === 'current' ? knownSpeakers : declaredSpeakers);
                if (head && 'speaker' in head && (text.slice(head.speaker.start, head.speaker.end) === name || syntax === 'current' && (text.slice(head.speaker.start, head.speaker.end) === label || headCues && Boolean(head.cue))) && line.start + colon < head.bodyStart) {
                    const afterColon = text.slice(line.start + colon + 1, head.bodyStart);
                    if (!afterColon.trim() || syntax === 'current' && /^[\t ]*(?:（[^()（）\r\n]*）|\([^()（）\r\n]*\))[\t ]*$/u.test(afterColon)) found.push(head);
                }
            }
            if (!found.length) continue;
            const eligible = found.filter((head)=>/(?:[。.!！？?;；,，…][”’」』"']?|[ \t])$/u.test(text.slice(line.start, head.speaker.start)));
            if (eligible.length !== 1) return null;
            if (!heads.some((head)=>head.speaker.start === eligible[0].speaker.start)) heads.push(eligible[0]);
        }
        if (heads.length < 2) return null;
        heads.sort((a, b)=>a.speaker.start - b.speaker.start);
        const dialogues = heads.map((head, i)=>{
            const start = i ? head.speaker.start : index.units[row].start, finish = heads[i + 1]?.speaker.start ?? index.units[row].end;
            const bodyFinish = finish - /[\t \r\n]*$/.exec(text.slice(start, finish))[0].length;
            if (head.bodyStart >= bodyFinish || head.bodyStart > end) return null;
            const ranges = [
                head.speaker,
                head.cue,
                head.channelSource,
                {
                    start: head.bodyStart,
                    end: bodyFinish
                }
            ].filter((r)=>Boolean(r)).sort((a, b)=>a.start - b.start);
            const layout = [];
            let cursor = start;
            for (const range of ranges){
                if (cursor < range.start) layout.push(...parts({
                    start: cursor,
                    end: range.start
                }));
                cursor = range.end;
            }
            if (cursor < finish) layout.push(...parts({
                start: cursor,
                end: finish
            }));
            return {
                parts: parts({
                    start: head.bodyStart,
                    end: bodyFinish
                }),
                layout,
                speaker: parts(head.speaker),
                channel: head.channel,
                channelSource: head.channelSource ? parts(head.channelSource) : null,
                cue: head.cue ? parts(head.cue) : null
            };
        });
        if (dialogues.some((d)=>!d)) return null;
        return {
            id: `line-${row + 1}`,
            unitIds: [
                index.units[row].id
            ],
            target: 'BODY',
            kind: 'DIALOGUE_GROUP',
            parts: parts({
                start: index.units[row].start,
                end: index.units[row].end
            }),
            layout: [],
            dialogues: dialogues.filter((d)=>Boolean(d))
        };
    };
    for(let i = 0; i < lines.length;){
        const line = lines[i], end = bodyEnd(i);
        if (ambiguousLines.has(line.number)) {
            i++;
            continue;
        }
        if (!line.content.trim()) {
            plan.entries.push({
                id: `line-${i + 1}`,
                target: 'EXCLUDE',
                category: 'BLANK',
                reason: '原文空白行',
                unitIds: [
                    index.units[i].id
                ]
            });
            i++;
            continue;
        }
        if (/^【作者资料】[ \t]*$/.test(line.content)) {
            let last = i;
            while(last + 1 < lines.length && !episodeHeading.test(lines[last + 1].content) && !sceneHeading.test(lines[last + 1].content))last++;
            plan.entries.push({
                target: 'ANNOTATION',
                kind: 'CREATIVE_GUIDANCE',
                ...mapped(i, last, {
                    start: line.start,
                    end: bodyEnd(last)
                })
            });
            i = last + 1;
            continue;
        }
        if (syntax === 'current' && productionGuidance(line.content)) {
            plan.entries.push({
                target: 'ANNOTATION',
                kind: 'CREATIVE_GUIDANCE',
                ...mapped(i, i, {
                    start: line.start,
                    end
                })
            });
            i++;
            continue;
        }
        if (typedMarker.test(line.content) && !endMarker(line)) {
            error(line, 'UNKNOWN_TYPED_LINE', '未识别的显式类型标记，不能默认归为动作或对白。');
            i++;
            continue;
        }
        const structureKind = episodeHeading.test(line.content) ? 'EPISODE_HEADING' : sceneHeading.test(line.content) ? 'SCENE_HEADING' : sceneMetadataHeading.test(line.content) || endMarker(line) ? 'SCENE_METADATA' : null;
        if (structureKind) {
            plan.entries.push({
                target: 'STRUCTURE',
                kind: structureKind,
                ...mapped(i, i, {
                    start: line.start,
                    end
                })
            });
            i++;
            continue;
        }
        const annotation = annotationKind(line.content);
        if (annotation) {
            plan.entries.push({
                target: 'ANNOTATION',
                kind: annotation,
                ...mapped(i, i, {
                    start: line.start,
                    end
                })
            });
            i++;
            continue;
        }
        if (syntax === 'current' && omissionMarker.test(line.content)) {
            plan.entries.push({
                target: 'ANNOTATION',
                kind: 'CREATIVE_GUIDANCE',
                ...mapped(i, i, {
                    start: line.start,
                    end
                })
            });
            i++;
            continue;
        }
        if (syntax === 'current' && screenTextHeading.test(line.content)) {
            if (!line.content.replace(screenTextHeading, '').trim()) error(line, 'EMPTY_ONSCREEN_TEXT', '显式画面文字标记后没有内容。');
            else plan.entries.push({
                target: 'BODY',
                kind: 'ONSCREEN_TEXT',
                ...mapped(i, i, {
                    start: line.start,
                    end
                })
            });
            i++;
            continue;
        }
        const marker = /^[△▲][ \t]*/.exec(line.content);
        if (marker) {
            if (line.start + marker[0].length >= end) error(line, 'EMPTY_ACTION', '动作标记后没有正文。');
            else plan.entries.push({
                target: 'BODY',
                kind: 'ACTION',
                ...mapped(i, i, {
                    start: line.start + marker[0].length,
                    end
                })
            });
            i++;
            continue;
        }
        if (/^(?:[:：]|[（(][^）)]*[）)][ \t]*[:：])/.test(line.content)) {
            error(line, 'MISSING_SPEAKER', '对白头缺少明确说者。');
            i++;
            continue;
        }
        const head = read(line);
        if (head && 'code' in head) {
            diagnostics.push(head);
            i++;
            continue;
        }
        if (head) {
            const inlineCaptions = syntax === 'current' ? explicitSubtitles(text.slice(head.bodyStart, end)) : null;
            if (inlineCaptions?.invalid) {
                error(line, 'MIXED_DIALOGUE_ONSCREEN_TEXT', '显式字幕为空、嵌套、跨行或未闭合；当前程序不能安全分型，原文保留且不作对白发出。');
                i++;
                continue;
            }
            const inline = head.bodyStart < end;
            let last = inline ? i : -1, start = head.bodyStart, next = i + 1;
            const blocked = multipleSpeakers(line, head);
            if (blocked) {
                if (inlineCaptions?.spans.length) {
                    error(line, 'MIXED_DIALOGUE_ONSCREEN_TEXT', '同一行同时含多说者与字幕，当前混合正文只支持一个明确说者；原文保留且不作整段对白发出。');
                    i++;
                    continue;
                }
                const group = dialogueGroup(line, head, i, end);
                if (group) {
                    plan.schemaVersion = 'STANDARD_SCRIPT_PLAN_V2';
                    plan.entries.push(group);
                    i++;
                    continue;
                }
                error(line, 'MULTIPLE_SPEAKERS_ON_LINE', '同一物理行的说者、引号或分隔边界仍不明确；原文保留，不能猜测拆分。');
            }
            while(!blocked && next < lines.length){
                const continuation = lines[next];
                const previousText = last < 0 ? '' : text.slice(start, bodyEnd(last)), inQuote = scanQuotes(previousText).open;
                if (!inQuote && (structural(continuation) || read(continuation) || /^(?:[:：]|[（(][^）)]*[）)][ \t]*[:：])/.test(continuation.content))) break;
                if (!inQuote && actorAction(continuation)) break;
                if (continuation.content.trim()) {
                    const startsWithName = [
                        ...knownSpeakers
                    ].some((name)=>continuation.content.startsWith(name) && continuation.content.length > name.length);
                    const previousCaptions = syntax === 'current' ? explicitSubtitles(previousText).spans : [];
                    let spokenPrevious = previousText;
                    for (const span of [
                        ...previousCaptions
                    ].reverse())spokenPrevious = spokenPrevious.slice(0, span.start) + spokenPrevious.slice(span.end);
                    const finishedInline = inline && /[。.!！？?…][”’」』"']?[ \t]*$/u.test(spokenPrevious);
                    const wholeQuoted = /^[“‘「『"']/u.test(previousText.trimStart()) && /[”’」』"']$/u.test(previousText.trimEnd());
                    if (!inQuote && (startsWithName || finishedInline || wholeQuoted)) {
                        error(continuation, 'AMBIGUOUS_CONTINUATION', '未标记新行可能是动作或对白续句，请明确说者续写或动作标记；不自动归入上一段对白。');
                        ambiguousLines.add(continuation.number);
                        break;
                    }
                    const colons = inQuote ? [] : unquotedColons(continuation.content);
                    if (colons.some((colon)=>!/[说问道答喊叫]$/.test(continuation.content.slice(0, colon)) || !/^[“「『"']/.test(continuation.content.slice(colon + 1).trimStart()))) break;
                    if (last < 0) start = continuation.start;
                    last = next;
                }
                next++;
            }
            if (last < 0) {
                if (!blocked && syntax === 'current' && head.cue && head.channel === 'SPEECH' && !head.channelSource && line.content.search(/[:：]/) >= 0 && head.cue.start > line.start + line.content.search(/[:：]/)) {
                    plan.entries.push({
                        target: 'BODY',
                        kind: 'ACTION',
                        ...mapped(i, i, {
                            start: line.start,
                            end
                        })
                    });
                } else if (!blocked) error(line, 'EMPTY_DIALOGUE', '说者头之后没有可归属的对白正文。');
                i++;
                continue;
            }
            if (!blocked) {
                if (scanQuotes(text.slice(start, bodyEnd(last))).open) {
                    error(line, 'UNCLOSED_DIALOGUE_QUOTE', '对白引号未闭合或撇号用途不明确；不能据此把后续说者归入同一段，原文保留待核对。');
                    i = last + 1;
                    continue;
                }
                const subtitles = syntax === 'current' ? explicitSubtitles(text.slice(start, bodyEnd(last))) : null;
                if (subtitles?.invalid) {
                    error(line, 'MIXED_DIALOGUE_ONSCREEN_TEXT', '显式字幕形状不受支持；原文保留且不作对白发出。');
                    i = last + 1;
                    continue;
                }
                if (subtitles?.spans.length) {
                    const group = mixedBody(i, last, start, bodyEnd(last), head);
                    if (group) plan.entries.push(group);
                    else error(line, 'MIXED_DIALOGUE_ONSCREEN_TEXT', '混合正文无法保真分型；原文保留。');
                    i = last + 1;
                    continue;
                }
                const fields = [
                    head.speaker,
                    head.cue,
                    head.channelSource
                ].filter((range)=>Boolean(range));
                plan.entries.push({
                    target: 'BODY',
                    kind: 'DIALOGUE',
                    channel: head.channel,
                    speaker: parts(head.speaker),
                    cue: head.cue ? parts(head.cue) : null,
                    channelSource: head.channelSource ? parts(head.channelSource) : null,
                    ...mapped(i, last, {
                        start,
                        end: bodyEnd(last)
                    }, fields)
                });
            }
            i = last + 1;
            continue;
        }
        if (unquotedColons(line.content).length) {
            error(line, 'UNKNOWN_COLON_LINE', '冒号行缺少明确说者或类型；请补充人物名单，或明确该行是对白还是资料。');
            i++;
            continue;
        }
        plan.entries.push({
            target: 'BODY',
            kind: 'ACTION',
            ...mapped(i, i, {
                start: line.start,
                end
            })
        });
        i++;
    }
    if (!text.trim()) diagnostics.push({
        line: 1,
        code: 'EMPTY_SOURCE',
        message: '没有可接收的剧本正文。'
    });
    plan.schemaVersion = standardScriptPlanVersion(index, plan.entries, {
        inlinePerformance: syntax === 'current' && options.inlinePerformance !== false,
        inlinePerformanceVersion: performanceVersion
    });
    let compiled = null;
    if (!diagnostics.length) {
        try {
            compiled = compileStandardScript(index, plan);
        } catch (cause) {
            if (!(cause instanceof StandardScriptError)) throw cause;
            const entryNumber = /plan\.entries\[(\d+)\]/.exec(cause.fieldPath)?.[1];
            const entry = entryNumber === undefined ? undefined : plan.entries[Number(entryNumber)];
            const unitId = cause.sourceUnitId ?? entry?.unitIds[0];
            const mixed = entry?.target === 'BODY' && entry.kind === 'BODY_GROUP';
            diagnostics.push({
                line: Math.max(1, index.units.findIndex((unit)=>unit.id === unitId) + 1),
                code: mixed ? 'MIXED_DIALOGUE_ONSCREEN_TEXT' : cause.code,
                message: `${mixed ? '混合字幕边界暂不支持，原文保留且不作整段对白发出；' : ''}源片段编译未通过：${cause.fieldPath}（${cause.code}）`
            });
        }
    }
    return {
        index,
        plan,
        compiled,
        diagnostics
    };
}
export function deriveStandardScriptLines(prepared) {
    const byUnit = new Map(prepared.plan.entries.flatMap((entry)=>entry.unitIds.map((id)=>[
                id,
                entry
            ])));
    return prepared.index.units.map((unit, i)=>{
        const entry = byUnit.get(unit.id);
        const line = {
            line: i + 1,
            unitId: unit.id,
            entryId: entry?.id ?? null,
            target: entry?.target ?? 'UNRESOLVED',
            kind: entry ? entry.target === 'EXCLUDE' ? entry.category : entry.kind : 'UNRESOLVED',
            text: unit.text
        };
        if (entry?.target === 'BODY' && entry.kind === 'DIALOGUE') {
            line.speaker = entry.speaker.map((part)=>prepared.index.text.slice(part.start, part.end)).join('');
            line.channel = entry.channel;
        }
        if (entry?.target === 'BODY' && entry.kind === 'DIALOGUE_GROUP') line.speakers = entry.dialogues.map((d)=>d.speaker.map((part)=>prepared.index.text.slice(part.start, part.end)).join(''));
        if (entry?.target === 'BODY' && entry.kind === 'BODY_GROUP') {
            const dialogue = entry.children.find((child)=>child.kind === 'DIALOGUE');
            if (dialogue?.kind === 'DIALOGUE') {
                line.speaker = dialogue.speaker.map((part)=>prepared.index.text.slice(part.start, part.end)).join('');
                line.channel = dialogue.channel;
            }
        }
        return line;
    });
}
