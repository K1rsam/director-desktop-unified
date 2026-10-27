import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { inlinePerformanceProjection, compactPerformanceHeadProjection, sourceVoiceProjection, sourceBarePerformanceProjection, sourceHeadPrefix, sourceUtteranceProperty, sourceInterleavedCueMatches, sourceColonlessVoiceHeader } from '../core/runtime/inline-performance.mjs';
export function explicitSubtitles(text) {
    const spans = [];
    const stack = [], pairs = {
        '“': '”',
        '‘': '’',
        '「': '」',
        '『': '』',
        '"': '"',
        "'": "'",
        '【': '】',
        '[': ']'
    };
    for(let i = 0; i < text.length; i++){
        const c = text[i];
        if (c === '\\') {
            i++;
            continue;
        }
        if ((c === "'" || c === '’') && /[A-Za-z0-9]/.test(text[i - 1] ?? '') && /[A-Za-z0-9]/.test(text[i + 1] ?? '')) continue;
        if (stack.at(-1) === c) {
            stack.pop();
            continue;
        }
        if (c === "'" && !stack.includes("'") && /[sS]/.test(text[i - 1] ?? '') && /\s/.test(text[i + 1] ?? '')) continue;
        if (!stack.length && /^[【\[](?:字幕|SUBTITLE)(?=[：:\t \]】]|$)/iu.test(text.slice(i))) {
            const m = /^【(字幕|SUBTITLE)[：:]([^【】\r\n]+)】/iu.exec(text.slice(i));
            if (!m || !m[2].trim()) return {
                spans,
                invalid: true
            };
            const bodyStart = i + 1 + m[1].length + 1;
            spans.push({
                start: i,
                end: i + m[0].length,
                labelStart: i + 1,
                labelEnd: i + 1 + m[1].length,
                bodyStart,
                bodyEnd: bodyStart + m[2].length
            });
            i += m[0].length - 1;
            continue;
        }
        if (pairs[c] && (!/[【\[]/u.test(c) || !stack.length || /[】\]]/u.test(stack.at(-1)))) stack.push(pairs[c]);
    }
    return {
        spans,
        invalid: false
    };
}
export class StandardScriptError extends Error {
    code;
    fieldPath;
    sourceUnitId;
    constructor(code, fieldPath, sourceUnitId){
        super(`${code}: ${fieldPath}${sourceUnitId ? ` (${sourceUnitId})` : ''}`), this.code = code, this.fieldPath = fieldPath, this.sourceUnitId = sourceUnitId;
        this.name = 'StandardScriptError';
    }
}
function fail(code, path, unitId) {
    throw new StandardScriptError(code, path, unitId);
}
const nonempty = (value, path)=>{
    if (typeof value !== 'string' || !value.trim()) fail('REQUIRED_STRING', path);
};
const oneOf = (value, values, path)=>{
    if (!values.includes(value)) fail('UNKNOWN_KIND', path);
};
function shape(value, keys, required, path) {
    if (!value || typeof value !== 'object' || Array.isArray(value) || ![
        Object.prototype,
        null
    ].includes(Object.getPrototypeOf(value))) fail('INVALID_OBJECT', path);
    for (const key of Object.keys(value))if (!keys.includes(key)) fail('UNKNOWN_FIELD', `${path}.${key}`);
    for (const key of required)if (!Object.hasOwn(value, key)) fail('MISSING_FIELD', `${path}.${key}`);
}
function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).sort(([a], [b])=>a < b ? -1 : a > b ? 1 : 0).map(([key, item])=>[
            key,
            canonical(item)
        ]));
    return value;
}
const digest = (value)=>createHash('sha256').update(JSON.stringify(canonical(value)), 'utf16le').digest('hex');
const splitPair = (text, offset)=>offset > 0 && offset < text.length && /[\uD800-\uDBFF]/.test(text[offset - 1]) && /[\uDC00-\uDFFF]/.test(text[offset]);
const declaredSpeakerForm = (text, name)=>text.split(/\r\n|\r|\n/).some((line)=>/^\s*(?:出场人物|人物|角色)[ \t]*[:：]/u.test(line) && line.replace(/^\s*(?:出场人物|人物|角色)[ \t]*[:：]/u, '').split(/[、,，;；]/u).some((item)=>item.trim() === name));
function compactSpeakerVoice(text, speaker) {
    return sourceVoiceProjection(text, speaker);
}
function assertCompactVoice(text, speaker, channelSource, channel, path, cue, v5 = false) {
    const token = (v5 ? compactPerformanceHeadProjection(text, speaker.parts, cue?.parts, channelSource?.parts) : null) ?? compactSpeakerVoice(text, speaker);
    if (token && (channel !== token.text.toUpperCase() || channelSource?.text !== token.text || channelSource.parts[0].start !== token.start || channelSource.parts.at(-1).end !== token.end)) fail('COMPACT_VOICE_FIELDS', path);
    return token;
}
function postColonFields(text, speaker, cue, channelSource, body) {
    const start = speaker?.[0]?.start, end = body?.[0]?.start;
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end)) return false;
    const head = /^[^:：()（）\r\n]+?(?:[（(][^()（）\r\n]*[）)])?(?:(?<=[）)])[\t \u00a0\u3000]*(?:OS|VO))?[\t ]*[:：]/iu.exec(text.slice(start, end));
    return Boolean(head && [
        cue?.[0]?.start,
        channelSource?.[0]?.start
    ].some((at)=>Number.isSafeInteger(at) && at >= start + head[0].length));
}
function entryPostCue(text, entry) {
    if (entry?.target !== 'BODY') return false;
    const children = entry.kind === 'DIALOGUE_GROUP' ? Array.isArray(entry.dialogues) ? entry.dialogues : [] : entry.kind === 'DIALOGUE' ? [
        entry
    ] : [];
    return Array.from(children).some((child)=>Boolean(child && postColonFields(text, child.speaker, child.cue, child.channelSource, child.parts)));
}
export function standardScriptPlanVersion(index, entries, options = {}) {
    const version = options.inlinePerformanceVersion === undefined ? 'inline-performance-v2' : options.inlinePerformanceVersion;
    if (![
        'inline-performance-v1',
        'inline-performance-v2'
    ].includes(version)) fail('UNSUPPORTED_PERFORMANCE_VERSION', 'options.inlinePerformanceVersion');
    if (options.inlinePerformance !== false && entries.some((entry)=>entry?.target === 'BODY' && (entry.kind === 'DIALOGUE' ? [
            entry
        ] : entry.kind === 'DIALOGUE_GROUP' ? entry.dialogues : entry.kind === 'BODY_GROUP' ? entry.children.filter((c)=>c.kind === 'DIALOGUE') : []).some((child)=>inlinePerformanceProjection(index.text, child.parts, version) || version === 'inline-performance-v2' && compactPerformanceHeadProjection(index.text, child.speaker, child.cue, child.channelSource)))) return version === 'inline-performance-v2' ? 'STANDARD_SCRIPT_PLAN_V5' : 'STANDARD_SCRIPT_PLAN_V4';
    if (entries.some((entry)=>entry?.target === 'BODY' && entry.kind === 'BODY_GROUP')) return 'STANDARD_SCRIPT_PLAN_V3';
    return entries.some((entry)=>entry?.target === 'BODY' && entry.kind === 'DIALOGUE_GROUP' || entryPostCue(index.text, entry)) ? 'STANDARD_SCRIPT_PLAN_V2' : 'STANDARD_SCRIPT_PLAN_V1';
}
function assertPostColonHeader(text, start, bodyStart, speaker, cue, channelSource, channel, path) {
    const match = /^[\t \uFEFF]*([^:：()（）\r\n]+?)(?:（([^()（）\r\n]*)）|\(([^()（）\r\n]*)\))?(?:(?<=[）)])[\t \u00a0\u3000]*(?:OS|VO))?[\t ]*[:：][\t ]*(?:（([^()（）\r\n]*)）|\(([^()（）\r\n]*)\))[\t \r\n]*$/diu.exec(text.slice(start, bodyStart));
    if (!match || /^[（(]/.test(text.slice(bodyStart))) fail('POST_CUE_HEADER', path);
    const post = match[4] ?? match[5];
    if (!post.trim() || /[:：“”‘’「」『』"'△▲]/u.test(post)) fail('POST_CUE_CONTENT', path);
    const bounds = match.indices;
    const range = (value, offset)=>({
            start: offset + value.length - value.trimStart().length,
            end: offset + value.trimEnd().length
        });
    const name = range(match[1], start + bounds[1][0]);
    const bare = sourceBarePerformanceProjection(text, speaker, cue);
    if (bare && bare.end <= name.end) name.end = speaker.parts.at(-1).end;
    const compact = compactSpeakerVoice(text, speaker);
    if (compact && name.end === compact.end) name.end = name.start + text.slice(name.start, compact.start).trimEnd().length;
    const preSlot = match[2] === undefined ? 3 : 2;
    const qualifiedEnd = bounds[preSlot] ? start + bounds[preSlot][1] + 1 : name.end;
    const qualified = (!compact || compact.start >= qualifiedEnd) && qualifiedEnd > name.end && speaker.text === text.slice(name.start, qualifiedEnd) && !/^(?:OS|VO|旁白|画外音|内心独白|系统音)$/iu.test(match[preSlot].trim()) && declaredSpeakerForm(text, speaker.text);
    if (qualified) name.end = qualifiedEnd;
    const exact = (field, expected)=>expected ? Boolean(field && field.parts[0].start === expected.start && field.parts.at(-1).end === expected.end && field.text === text.slice(expected.start, expected.end)) : field == null;
    if (!exact(speaker, name) || !/^[\p{L}\p{N}·・ _.-]+(?:、[\p{L}\p{N}·・ _.-]+)*$/u.test(qualified ? match[1].trim() : speaker.text) || /^\d+$/.test(speaker.text)) fail('POST_CUE_SPEAKER', path);
    const tokens = (qualified ? [
        4,
        5
    ] : [
        2,
        3,
        4,
        5
    ]).flatMap((slot)=>match[slot] === undefined ? [] : [
            ...match[slot].matchAll(/[^,， \t]+/g)
        ].flatMap((token)=>{
            const from = start + bounds[slot][0] + token.index, end = from + token[0].length;
            return slot >= 4 && /^(?:OS|VO)(?=\p{Script=Han})/iu.test(token[0]) ? [
                {
                    slot,
                    text: token[0].slice(0, 2),
                    start: from,
                    end: from + 2
                },
                {
                    slot,
                    text: token[0].slice(2),
                    start: from + 2,
                    end
                }
            ] : [
                {
                    slot,
                    text: token[0],
                    start: from,
                    end
                }
            ];
        }));
    if (compact) tokens.unshift({
        slot: 1,
        ...compact
    });
    if (bare) tokens.unshift({
        slot: 1,
        text: cue.text,
        start: bare.start,
        end: bare.end
    });
    if (tokens.some((token)=>token.slot >= 4 && !/^(OS|VO)$/i.test(token.text) && /\b(?:OS|VO)\b|\b(?:O\.S\.?|V\.O\.?)|旁白|画外音|内心独白|系统音/i.test(token.text))) fail('POST_CUE_CHANNEL', path);
    const channels = tokens.filter((token)=>/^(OS|VO)$/i.test(token.text)), cues = tokens.filter((token)=>!channels.includes(token));
    if (channels.length > 1 || new Set(cues.map((token)=>token.slot)).size > 1) fail('POST_CUE_AMBIGUOUS', path);
    const cueRange = cues.length ? {
        start: cues[0].start,
        end: cues.at(-1).end
    } : null;
    const expectedChannel = channels[0]?.text.toUpperCase() ?? ({
        旁白: 'NARRATION',
        画外音: 'VO',
        系统音: 'SYSTEM'
    })[speaker.text] ?? 'SPEECH';
    if (!exact(cue, cueRange) || !exact(channelSource, channels[0] ?? null) || channel !== expectedChannel) fail('POST_CUE_FIELDS', path);
}
function assertGroupUtterance(text, path, speaker = '', cue = '') {
    const closers = [], pairs = {
        '“': '”',
        '‘': '’',
        '「': '」',
        '『': '』',
        '"': '"',
        "'": "'",
        '【': '】',
        '[': ']'
    };
    for(let i = 0; i < text.length; i++){
        const c = text[i];
        if (c === '\\') {
            i++;
            continue;
        }
        if ((c === "'" || c === '’') && /[A-Za-z0-9]/.test(text[i - 1] ?? '') && /[A-Za-z0-9]/.test(text[i + 1] ?? '')) continue;
        if (closers.at(-1) === c) {
            closers.pop();
            continue;
        }
        if (c === "'" && !closers.includes("'") && /[sS]/.test(text[i - 1] ?? '') && /\s/.test(text[i + 1] ?? '')) continue;
        if (pairs[c] && (!/[【\[]/u.test(c) || !closers.length || /[】\]]/u.test(closers.at(-1)))) {
            closers.push(pairs[c]);
            continue;
        }
        if (/[”’」』]/.test(c)) fail('DIALOGUE_GROUP_QUOTE', path);
        if (closers.length) continue;
        if (/[△▲]/.test(c)) fail('DIALOGUE_GROUP_ACTION', path);
        if (/[:：]/.test(c)) {
            const prefix = text.slice(0, i), label = /(?:^|[。.!！？?;；,， \t\r\n])([^。.!！？?;；,，:：()（） \t\r\n]+(?:[（(][^()（）]*[）)])?)[ \t]*$/u.exec(prefix)?.[1]?.trim();
            if (!label || sourceUtteranceProperty(speaker, label, cue) || /^(?:时间层|时间|地点|场景|天气|内外|道具)$/.test(label) || /[说问道答喊叫]$/.test(label) && /^[“‘「『"']/.test(text.slice(i + 1).trimStart())) continue;
            if (/^[\p{L}\p{N}·・ _.-]+(?:[（(][^()（）]*[）)])?$/u.test(label)) fail('DIALOGUE_GROUP_NESTED_HEAD', path);
        }
    }
    if (closers.length) fail('DIALOGUE_GROUP_QUOTE', path);
}
export function createSourceIndex(text, sourceId) {
    if (typeof text !== 'string') fail('INVALID_SOURCE_TEXT', 'source.text');
    nonempty(sourceId, 'source.sourceId');
    const sourceVersion = `src:${digest([
        sourceId,
        text
    ])}`;
    const units = [];
    for (const match of text.matchAll(/[^\r\n]*(?:\r\n|\r|\n|$)/g)){
        if (!match[0].length) continue;
        const start = match.index;
        units.push({
            id: `${sourceVersion}:u${units.length + 1}`,
            start,
            end: start + match[0].length,
            text: match[0]
        });
    }
    return {
        schemaVersion: 'STANDARD_SCRIPT_SOURCE_INDEX_V1',
        indexerVersion: 'physical-lines-v1',
        sourceId,
        sourceVersion,
        offsetUnit: 'UTF16',
        text,
        units
    };
}
export function compileStandardScript(index, plan) {
    if (!index || typeof index.text !== 'string' || typeof index.sourceId !== 'string' || !isDeepStrictEqual(index, createSourceIndex(index.text, index.sourceId))) fail('SOURCE_INDEX_INTEGRITY', 'sourceIndex');
    shape(plan, [
        'schemaVersion',
        'sourceId',
        'sourceVersion',
        'entries'
    ], [
        'schemaVersion',
        'sourceId',
        'sourceVersion',
        'entries'
    ], 'plan');
    if (![
        'STANDARD_SCRIPT_PLAN_V1',
        'STANDARD_SCRIPT_PLAN_V2',
        'STANDARD_SCRIPT_PLAN_V3',
        'STANDARD_SCRIPT_PLAN_V4',
        'STANDARD_SCRIPT_PLAN_V5'
    ].includes(plan.schemaVersion)) fail('UNSUPPORTED_SCHEMA', 'plan.schemaVersion');
    if (plan.sourceId !== index.sourceId || plan.sourceVersion !== index.sourceVersion) fail('SOURCE_BINDING', 'plan.sourceVersion');
    if (!Array.isArray(plan.entries)) fail('INVALID_ARRAY', 'plan.entries');
    const grouped = plan.entries.some((entry)=>entry?.target === 'BODY' && entry.kind === 'DIALOGUE_GROUP');
    const mixed = plan.entries.some((entry)=>entry?.target === 'BODY' && entry.kind === 'BODY_GROUP');
    const postCues = plan.entries.some((entry)=>entryPostCue(index.text, entry)), v2 = grouped || postCues;
    const v5 = plan.schemaVersion === 'STANDARD_SCRIPT_PLAN_V5', performance = v5 || plan.schemaVersion === 'STANDARD_SCRIPT_PLAN_V4';
    const performanceVersion = v5 ? 'inline-performance-v2' : 'inline-performance-v1';
    if (standardScriptPlanVersion(index, plan.entries, {
        inlinePerformance: performance,
        inlinePerformanceVersion: performanceVersion
    }) !== plan.schemaVersion) fail('UNSUPPORTED_SCHEMA', 'plan.schemaVersion');
    const units = new Map(index.units.map((unit)=>[
            unit.id,
            unit
        ]));
    const script = {
        entries: []
    };
    const annotations = [];
    const exclusions = [];
    const assignments = [];
    const ids = new Set();
    const rendered = [];
    let nextUnit = 0;
    for (const [position, entry] of plan.entries.entries()){
        const path = `plan.entries[${position}]`;
        if (!entry || typeof entry !== 'object') fail('INVALID_ENTRY', path);
        oneOf(entry.target, [
            'BODY',
            'STRUCTURE',
            'ANNOTATION',
            'EXCLUDE'
        ], `${path}.target`);
        const base = [
            'id',
            'target',
            'unitIds'
        ];
        const keys = entry.target === 'EXCLUDE' ? [
            ...base,
            'category',
            'reason'
        ] : [
            ...base,
            'kind',
            'parts',
            'layout'
        ];
        if (entry.target === 'BODY' && entry.kind === 'DIALOGUE') keys.push('speaker', 'channel', 'channelSource', 'cue');
        if (entry.target === 'BODY' && entry.kind === 'DIALOGUE_GROUP') keys.push('dialogues');
        if (entry.target === 'BODY' && entry.kind === 'BODY_GROUP') keys.push('children');
        shape(entry, keys, entry.target === 'EXCLUDE' ? keys : [
            ...base,
            'kind',
            'parts',
            'layout'
        ], path);
        nonempty(entry.id, `${path}.id`);
        if (ids.has(entry.id)) fail('DUPLICATE_ID', `${path}.id`);
        ids.add(entry.id);
        if (!Array.isArray(entry.unitIds) || !entry.unitIds.length) fail('INVALID_UNIT_IDS', `${path}.unitIds`);
        const owned = new Set();
        for (const unitId of entry.unitIds){
            if (!units.has(unitId)) fail('UNKNOWN_REFERENCE', `${path}.unitIds`, unitId);
            if (index.units[nextUnit]?.id !== unitId) fail('UNIT_ORDER_OR_DUPLICATION', `${path}.unitIds`, unitId);
            owned.add(unitId);
            nextUnit++;
        }
        const outputPath = entry.target === 'EXCLUDE' ? `exclusions[${exclusions.length}]` : entry.target === 'ANNOTATION' ? `annotations[${annotations.length}]` : `script.entries[${script.entries.length}]`;
        for (const unitId of entry.unitIds)assignments.push({
            unitId,
            target: entry.target,
            targetId: entry.id,
            fieldPath: outputPath
        });
        if (entry.target === 'EXCLUDE') {
            oneOf(entry.category, [
                'BLANK',
                'SOURCE_METADATA'
            ], `${path}.category`);
            nonempty(entry.reason, `${path}.reason`);
            const text = entry.unitIds.map((id)=>units.get(id).text).join('');
            if (entry.category === 'BLANK' && !/^\s*$/.test(text)) fail('NONBLANK_EXCLUSION', path);
            exclusions.push({
                id: entry.id,
                unitIds: [
                    ...entry.unitIds
                ],
                category: entry.category,
                reason: entry.reason,
                text
            });
            continue;
        }
        if (entry.target === 'BODY') oneOf(entry.kind, [
            'DIALOGUE',
            'DIALOGUE_GROUP',
            'BODY_GROUP',
            'ACTION',
            'SFX',
            'ONSCREEN_TEXT'
        ], `${path}.kind`);
        else if (entry.target === 'STRUCTURE') oneOf(entry.kind, [
            'EPISODE_HEADING',
            'SCENE_HEADING',
            'SCENE_METADATA'
        ], `${path}.kind`);
        else oneOf(entry.kind, [
            'CONTINUITY',
            'EMOTION_CLUE',
            'RELATIONSHIP_CONTEXT',
            'KNOWLEDGE_REVEAL',
            'SPACE_CONSTRAINT',
            'CREATIVE_GUIDANCE'
        ], `${path}.kind`);
        const consumed = [];
        function assemble(parts, field, allowEmpty = false, sink = consumed, allowCaptionGaps = false) {
            if (!Array.isArray(parts) || !allowEmpty && !parts.length) fail('EMPTY_OR_INVALID_PARTS', `${path}.${field}`);
            const isLayout = field === 'layout' || field.endsWith('.layout');
            const childCue = /^(children|dialogues)\[(\d+)\]\.cue$/u.exec(field);
            const cueOwner = field === 'cue' ? entry : childCue ? entry[childCue[1]]?.[Number(childCue[2])] : null;
            const headerCueGaps = cueOwner && sourceInterleavedCueMatches(index.text, cueOwner.speaker, cueOwner.cue, cueOwner.channelSource);
            let end = -1;
            const copied = Array.from(parts, (part, i)=>{
                const fieldPath = `${path}.${field}[${i}]`;
                shape(part, [
                    'kind',
                    'unitId',
                    'start',
                    'end'
                ], [
                    'kind',
                    'unitId',
                    'start',
                    'end'
                ], fieldPath);
                oneOf(part.kind, [
                    'SOURCE_SLICE',
                    'SOURCE_SEPARATOR'
                ], `${fieldPath}.kind`);
                const unit = units.get(part.unitId);
                if (!unit || !owned.has(part.unitId)) fail('UNKNOWN_REFERENCE', fieldPath, part.unitId);
                if (!Number.isInteger(part.start) || !Number.isInteger(part.end) || part.start < unit.start || part.end > unit.end || part.start >= part.end) fail('INVALID_RANGE', fieldPath, part.unitId);
                if (splitPair(index.text, part.start) || splitPair(index.text, part.end)) fail('UNICODE_BOUNDARY', fieldPath, part.unitId);
                if (part.start < end) fail('PART_ORDER_OR_DUPLICATION', fieldPath, part.unitId);
                if (!isLayout && !allowCaptionGaps && !headerCueGaps && i > 0 && part.start !== end) fail('TEXT_GAP', fieldPath, part.unitId);
                end = part.end;
                const text = index.text.slice(part.start, part.end);
                if (part.kind === 'SOURCE_SEPARATOR' && !/^[\t \r\n]+$/.test(text)) fail('INVALID_SEPARATOR', fieldPath, part.unitId);
                if (isLayout && !/^[\s\uFEFF:：,，()（）△▲【】\[\]]+$/.test(text)) fail('NON_LAYOUT_CONTENT', fieldPath, part.unitId);
                sink.push({
                    start: part.start,
                    end: part.end,
                    unitId: part.unitId,
                    fieldPath
                });
                return {
                    kind: part.kind,
                    unitId: part.unitId,
                    start: part.start,
                    end: part.end
                };
            });
            return {
                text: copied.map((part)=>index.text.slice(part.start, part.end)).join(''),
                parts: copied
            };
        }
        const text = assemble(entry.parts, 'parts');
        const layout = assemble(entry.layout, 'layout', true);
        const output = {
            id: entry.id,
            target: entry.target,
            kind: entry.kind,
            sourceUnitIds: [
                ...entry.unitIds
            ],
            text: text.text,
            textAssembly: {
                parts: text.parts
            },
            layout: layout.parts
        };
        if (entry.target === 'BODY' && entry.kind === 'DIALOGUE') {
            oneOf(entry.channel, [
                'SPEECH',
                'OS',
                'VO',
                'NARRATION',
                'SYSTEM'
            ], `${path}.channel`);
            output.speaker = assemble(entry.speaker, 'speaker');
            if (!output.speaker.text.trim()) fail('EMPTY_SPEAKER', `${path}.speaker`);
            output.channel = entry.channel;
            output.channelSource = entry.channelSource == null ? null : assemble(entry.channelSource, 'channelSource');
            output.cue = entry.cue == null ? null : assemble(entry.cue, 'cue');
            assertCompactVoice(index.text, output.speaker, output.channelSource, output.channel, path, output.cue, v5);
        }
        const textStart = text.parts[0].start, textEnd = text.parts[text.parts.length - 1].end;
        const unitStart = units.get(entry.unitIds[0]).start, unitEnd = units.get(entry.unitIds[entry.unitIds.length - 1]).end;
        if (entry.target === 'BODY' && entry.kind === 'BODY_GROUP') {
            if (textStart !== unitStart || textEnd !== unitEnd || layout.parts.length || !Array.isArray(entry.children) || entry.children.length < 2 || entry.children[0]?.kind !== 'DIALOGUE' || entry.children.slice(1).some((c)=>c?.kind !== 'ONSCREEN_TEXT')) fail('BODY_GROUP_SHAPE', path);
            const spans = [], captions = [];
            const children = entry.children.map((child, i)=>{
                const field = `children[${i}]`, childPath = `${path}.${field}`;
                const keys = child.kind === 'DIALOGUE' ? [
                    'kind',
                    'parts',
                    'layout',
                    'speaker',
                    'channel',
                    'channelSource',
                    'cue'
                ] : [
                    'kind',
                    'parts',
                    'layout',
                    'label'
                ];
                shape(child, keys, child.kind === 'DIALOGUE' ? [
                    'kind',
                    'parts',
                    'layout',
                    'speaker',
                    'channel'
                ] : keys, childPath);
                const childSpans = [];
                const body = assemble(child.parts, `${field}.parts`, false, childSpans, child.kind === 'DIALOGUE');
                const childLayout = assemble(child.layout, `${field}.layout`, true, childSpans);
                if (!body.text.trim()) fail('BODY_GROUP_EMPTY', childPath);
                let outputChild;
                if (child.kind === 'DIALOGUE') {
                    oneOf(child.channel, [
                        'SPEECH',
                        'OS',
                        'VO',
                        'NARRATION',
                        'SYSTEM'
                    ], `${childPath}.channel`);
                    const speaker = assemble(child.speaker, `${field}.speaker`, false, childSpans);
                    const cue = child.cue == null ? null : assemble(child.cue, `${field}.cue`, false, childSpans);
                    const channelSource = child.channelSource == null ? null : assemble(child.channelSource, `${field}.channelSource`, false, childSpans);
                    assertCompactVoice(index.text, speaker, channelSource, child.channel, childPath, cue, v5);
                    outputChild = {
                        kind: 'DIALOGUE',
                        text: body.text,
                        textAssembly: {
                            parts: body.parts
                        },
                        layout: childLayout.parts,
                        speaker,
                        cue,
                        channelSource,
                        channel: child.channel
                    };
                } else {
                    const label = assemble(child.label, `${field}.label`, false, childSpans);
                    childSpans.sort((a, b)=>a.start - b.start);
                    let edge = childSpans[0].start;
                    for (const span of childSpans){
                        if (span.start !== edge) fail('BODY_GROUP_CAPTION_COVERAGE', childPath);
                        edge = span.end;
                    }
                    captions.push({
                        start: childSpans[0].start,
                        end: edge,
                        label,
                        body
                    });
                    outputChild = {
                        kind: 'ONSCREEN_TEXT',
                        text: body.text,
                        textAssembly: {
                            parts: body.parts
                        },
                        label,
                        layout: childLayout.parts
                    };
                }
                spans.push(...childSpans);
                return outputChild;
            });
            spans.sort((a, b)=>a.start - b.start);
            let edge = unitStart;
            for (const span of spans){
                if (span.start !== edge) fail('BODY_GROUP_COVERAGE', span.fieldPath);
                edge = span.end;
            }
            if (edge !== unitEnd) fail('BODY_GROUP_COVERAGE', path);
            const dialogue = children[0];
            const a = Math.min(dialogue.textAssembly.parts[0].start, captions[0].start), b = Math.max(dialogue.textAssembly.parts.at(-1).end, captions.at(-1).end);
            const raw = index.text.slice(a, b), scan = explicitSubtitles(raw);
            if (scan.invalid || scan.spans.length !== captions.length || !/^\s*$/.test(index.text.slice(b, unitEnd))) fail('BODY_GROUP_CAPTION', path);
            let spoken = '', cursor = 0;
            for (const [i, marker] of scan.spans.entries()){
                const caption = captions[i], exact = (value, from, to)=>value.parts[0].start === a + from && value.parts.at(-1).end === a + to;
                if (caption.start !== a + marker.start || caption.end !== a + marker.end || !exact(caption.label, marker.labelStart, marker.labelEnd) || !exact(caption.body, marker.bodyStart, marker.bodyEnd)) fail('BODY_GROUP_CAPTION', path);
                spoken += raw.slice(cursor, marker.start);
                cursor = marker.end;
            }
            spoken += raw.slice(cursor);
            if (spoken !== dialogue.text) fail('BODY_GROUP_SPOKEN_TEXT', path);
            if (postColonFields(index.text, dialogue.speaker.parts, dialogue.cue?.parts, dialogue.channelSource?.parts, [
                {
                    ...dialogue.textAssembly.parts[0],
                    start: a
                }
            ])) assertPostColonHeader(index.text, unitStart, a, dialogue.speaker, dialogue.cue, dialogue.channelSource, dialogue.channel, path);
            else {
                const compact = assertCompactVoice(index.text, dialogue.speaker, dialogue.channelSource, dialogue.channel, path, dialogue.cue, v5);
                const header = /^\s*([^:：()（）]+?)(?:[（(]([^()（）]*)[）)])?(?:(?<=[）)])[\t \u00a0\u3000]*(OS|VO))?[ \t]*[:：][ \t\r\n]*$/iu.exec(index.text.slice(unitStart, a));
                const qualified = Boolean(header?.[2] && !/^(?:OS|VO|旁白|画外音|内心独白|系统音)$/iu.test(header[2].trim()) && index.text.slice(unitStart, a).replace(/(?<=[）)])[\t \u00a0\u3000]*(?:OS|VO)[\t \u00a0\u3000]*(?=[:：])/iu, '').replace(/[:：][\s]*$/u, '').trim() === dialogue.speaker.text && declaredSpeakerForm(index.text, dialogue.speaker.text));
                if (!header || !qualified && header[1].trim() !== sourceHeadPrefix(index.text, dialogue.speaker, dialogue.cue, dialogue.channelSource)) fail('BODY_GROUP_HEADER', path);
                const fields = [
                    dialogue.speaker,
                    dialogue.cue,
                    dialogue.channelSource
                ].filter((f)=>Boolean(f)).flatMap(f=>f.parts.map(part=>({parts:[part]}))).sort((x, y)=>x.parts[0].start - y.parts[0].start);
                let edge = unitStart;
                for (const f of fields){
                    if (f.parts[0].start < edge || !(edge === unitStart ? /^[\s\uFEFF]*$/ : /^[\s()（）,，]*$/).test(index.text.slice(edge, f.parts[0].start))) fail('BODY_GROUP_HEADER', path);
                    edge = f.parts.at(-1).end;
                }
                if (edge > a || !/^[\t ()（）]*[:：][\t \r\n]*$/.test(index.text.slice(edge, a))) fail('BODY_GROUP_HEADER', path);
                const tokens = [
                    ...compact && !header[3] ? [
                        compact.text
                    ] : [],
                    ...header[3] ? [
                        header[3]
                    ] : [],
                    ...(qualified ? '' : header[2] ?? '').split(/[,， \t]+/).filter(Boolean)
                ], channels = tokens.filter((t)=>/^(OS|VO)$/i.test(t));
                const expected = channels[0]?.toUpperCase() ?? ({
                    旁白: 'NARRATION',
                    画外音: 'VO',
                    系统音: 'SYSTEM'
                })[dialogue.speaker.text] ?? 'SPEECH';
                if (channels.length > 1 || dialogue.channel !== expected || (channels.length ? dialogue.channelSource?.text !== channels[0] : Boolean(dialogue.channelSource))) fail('BODY_GROUP_CHANNEL', path);
            }
            assertGroupUtterance(dialogue.text, path, dialogue.speaker.text, dialogue.cue?.text);
            output.children = children;
        }
        if (entry.target === 'BODY' && entry.kind === 'DIALOGUE_GROUP') {
            if (entry.unitIds.length !== 1 || textStart !== unitStart || textEnd !== unitEnd || layout.parts.length || !Array.isArray(entry.dialogues) || entry.dialogues.length < 2) fail('DIALOGUE_GROUP_BOUNDARY', path);
            let groupEdge = unitStart;
            output.dialogues = Array.from(entry.dialogues, (child, i)=>{
                const field = `dialogues[${i}]`, childPath = `${path}.${field}`, spans = [];
                shape(child, [
                    'parts',
                    'layout',
                    'speaker',
                    'channel',
                    'channelSource',
                    'cue'
                ], [
                    'parts',
                    'layout',
                    'speaker',
                    'channel'
                ], childPath);
                oneOf(child.channel, [
                    'SPEECH',
                    'OS',
                    'VO',
                    'NARRATION',
                    'SYSTEM'
                ], `${childPath}.channel`);
                const body = assemble(child.parts, `${field}.parts`, false, spans), speaker = assemble(child.speaker, `${field}.speaker`, false, spans);
                const cue = child.cue == null ? null : assemble(child.cue, `${field}.cue`, false, spans);
                const channelSource = child.channelSource == null ? null : assemble(child.channelSource, `${field}.channelSource`, false, spans);
                const compact = assertCompactVoice(index.text, speaker, channelSource, child.channel, childPath, cue, v5);
                const childLayout = assemble(child.layout, `${field}.layout`, true, spans);
                spans.sort((a, b)=>a.start - b.start);
                const childStart = groupEdge;
                for (const span of spans){
                    if (span.start !== groupEdge) fail('DIALOGUE_GROUP_COVERAGE', span.fieldPath);
                    groupEdge = span.end;
                }
                const a = body.parts[0].start, b = body.parts.at(-1).end;
                if (!body.text.trim() || !/^[\t \r\n]*$/.test(index.text.slice(b, groupEdge))) fail('DIALOGUE_GROUP_TEXT_BOUNDARY', childPath);
                if (postColonFields(index.text, speaker.parts, cue?.parts, channelSource?.parts, body.parts)) {
                    assertPostColonHeader(index.text, childStart, a, speaker, cue, channelSource, child.channel, childPath);
                } else {
                    const header = /^\s*([^:：()（）]+?)(?:[（(]([^()（）]*)[）)])?(?:(?<=[）)])[\t \u00a0\u3000]*(OS|VO))?[ \t]*[:：][ \t]*$/iu.exec(index.text.slice(childStart, a));
                    const qualified = Boolean(header?.[2] && !/^(?:OS|VO|旁白|画外音|内心独白|系统音)$/iu.test(header[2].trim()) && index.text.slice(childStart, a).replace(/(?<=[）)])[\t \u00a0\u3000]*(?:OS|VO)[\t \u00a0\u3000]*(?=[:：])/iu, '').replace(/[:：][ \t]*$/u, '').trim() === speaker.text && declaredSpeakerForm(index.text, speaker.text));
                    if (!header || !qualified && header[1].trim() !== sourceHeadPrefix(index.text, speaker, cue, channelSource) || !/^[\p{L}\p{N}·・ _.-]+(?:、[\p{L}\p{N}·・ _.-]+)*$/u.test(qualified ? header[1].trim() : speaker.text)) fail('DIALOGUE_GROUP_HEADER', childPath);
                    const prefixFields = [
                        speaker,
                        cue,
                        channelSource
                    ].filter((f)=>Boolean(f)).flatMap(f=>f.parts.map(part=>({parts:[part]}))).sort((x, y)=>x.parts[0].start - y.parts[0].start);
                    let edge = childStart;
                    for (const f of prefixFields){
                        if (f.parts[0].start < edge || !(edge === childStart ? /^[\s\uFEFF()（）【】\[\]]*$/ : /^[\s\uFEFF()（）【】\[\],，]*$/).test(index.text.slice(edge, f.parts[0].start))) fail('DIALOGUE_GROUP_HEADER', childPath);
                        edge = f.parts.at(-1).end;
                    }
                    if (edge > a || !/^[\t ()（）【】\[\]]*[:：][\t ]*$/.test(index.text.slice(edge, a))) fail('DIALOGUE_GROUP_HEADER', childPath);
                    const tokens = [
                        ...compact && !header[3] ? [
                            compact.text
                        ] : [],
                        ...header[3] ? [
                            header[3]
                        ] : [],
                        ...(qualified ? '' : header[2] ?? '').split(/[,， \t]+/).filter(Boolean)
                    ], channels = tokens.filter((token)=>/^(?:OS|VO)$/i.test(token));
                    const expected = channels.length ? channels[0].toUpperCase() : ({
                        旁白: 'NARRATION',
                        画外音: 'VO',
                        系统音: 'SYSTEM'
                    })[speaker.text] ?? 'SPEECH';
                    if (channels.length > 1 || child.channel !== expected || (channels.length ? channelSource?.text !== channels[0] : Boolean(channelSource))) fail('DIALOGUE_GROUP_CHANNEL', childPath);
                }
                if (i && !/(?:[。.!！？?;；,，…][”’」』"']?|[ \t])$/u.test(index.text.slice(unitStart, speaker.parts[0].start))) fail('DIALOGUE_GROUP_BOUNDARY', childPath);
                assertGroupUtterance(body.text, childPath, speaker.text, cue?.text);
                return {
                    text: body.text,
                    textAssembly: {
                        parts: body.parts
                    },
                    layout: childLayout.parts,
                    speaker,
                    channel: child.channel,
                    channelSource,
                    cue
                };
            });
            if (groupEdge !== unitEnd) fail('DIALOGUE_GROUP_COVERAGE', path);
        }
        if (!/^[\t \r\n]*$/.test(index.text.slice(textEnd, unitEnd))) fail('NON_TRAILING_LAYOUT', `${path}.layout`);
        if (output.speaker) {
            if (postColonFields(index.text, output.speaker.parts, output.cue?.parts, output.channelSource?.parts, text.parts)) {
                assertPostColonHeader(index.text, unitStart, textStart, output.speaker, output.cue, output.channelSource, output.channel, path);
                assertGroupUtterance(output.text, path, output.speaker.text, output.cue?.text);
            } else {
                const fields = [
                    output.speaker,
                    output.cue,
                    output.channelSource
                ].filter((field)=>Boolean(field)).flatMap(f=>f.parts.map(part=>({parts:[part]}))).sort((a, b)=>a.parts[0].start - b.parts[0].start);
                let edge = unitStart;
                for (const field of fields){
                    const gap = index.text.slice(edge, field.parts[0].start);
                    const allowed = edge === unitStart ? /^[\s\uFEFF()（）【】\[\]]*$/ : /^[\s\uFEFF()（）【】\[\],，]*$/;
                    if (field.parts[0].start < edge || !allowed.test(gap)) fail('DIALOGUE_PREFIX_BOUNDARY', `${path}.layout`);
                    edge = field.parts[field.parts.length - 1].end;
                }
                const separator = index.text.slice(edge, textStart);
                const colonlessVoice = sourceColonlessVoiceHeader(index.text, output.speaker, output.channelSource, output.channel, textStart, output.cue);
                if (edge > textStart || !colonlessVoice && !/^[\t ()（）【】\[\]]*(?:[:：][\t \r\n]*|[\r\n][\t \r\n]*)$/.test(separator)) fail('DIALOGUE_TEXT_BOUNDARY', `${path}.layout`);
            }
        } else {
            const prefix = index.text.slice(unitStart, textStart);
            const allowed = entry.target === 'BODY' && entry.kind === 'ACTION' ? /^[\s\uFEFF]*(?:[△▲][\t ]*)?$/ : /^[\s\uFEFF]*$/;
            if (!allowed.test(prefix)) fail('NON_PREFIX_LAYOUT', `${path}.layout`);
        }
        consumed.sort((a, b)=>a.start - b.start);
        let cursor = units.get(entry.unitIds[0]).start;
        for (const span of consumed){
            if (span.start !== cursor) fail('FIELD_COVERAGE_OR_OVERLAP', span.fieldPath, span.unitId);
            cursor = span.end;
        }
        if (cursor !== units.get(entry.unitIds[entry.unitIds.length - 1]).end) fail('FIELD_COVERAGE', path);
        if (performance) {
            const voices = output.kind === 'DIALOGUE' ? [
                output
            ] : output.dialogues ?? output.children?.filter((c)=>c.kind === 'DIALOGUE') ?? [];
            for (const voice of voices){
                const projected = inlinePerformanceProjection(index.text, voice.textAssembly.parts, performanceVersion);
                if (projected) voice.performance = projected;
            }
        }
        (entry.target === 'ANNOTATION' ? annotations : script.entries).push(output);
        const label = entry.target === 'ANNOTATION' ? '辅助资料·语义未验收' : entry.target === 'STRUCTURE' ? '结构' : '正文';
        const voice = output.speaker ? `【说者】${output.speaker.text}\n【声道】${output.channel}\n${output.cue ? `【提示】${output.cue.text}\n` : ''}` : '';
        const delivery = (d)=>d.performance ? `${d.performance.spoken.text}\n${d.performance.cues.map((c)=>`【行中表演·台词位置${c.spokenOffset}】${c.text}`).join('\n')}` : d.text;
        rendered.push(output.children ? `【${label}·BODY_GROUP】\n${output.children.map((d)=>d.kind === 'DIALOGUE' ? `【说者】${d.speaker.text}\n【声道】${d.channel}\n${d.cue ? `【提示】${d.cue.text}\n` : ''}${delivery(d)}` : `【画面文字】${d.text}`).join('\n\n')}` : output.dialogues ? `【${label}·${entry.kind}】\n${output.dialogues.map((d)=>`【说者】${d.speaker.text}\n【声道】${d.channel}\n${d.cue ? `【提示】${d.cue.text}\n` : ''}${delivery(d)}`).join('\n\n')}` : `【${label}·${entry.kind}】\n${voice}${delivery(output)}`);
    }
    if (nextUnit !== index.units.length) fail('UNIT_COVERAGE', 'plan.entries', index.units[nextUnit]?.id);
    const readingText = `【标准稿基础切片·语义未验收】\n\n${rendered.join('\n\n')}`;
    const sourceBinding = {
        sourceId: index.sourceId,
        sourceVersion: index.sourceVersion,
        indexerVersion: index.indexerVersion,
        sourceIndexDigest: digest(index)
    };
    const payload = {
        schemaVersion: v5 ? 'DIRECTOR_STANDARD_SCRIPT_SLICE_V5' : performance ? 'DIRECTOR_STANDARD_SCRIPT_SLICE_V4' : mixed ? 'DIRECTOR_STANDARD_SCRIPT_SLICE_V3' : v2 ? 'DIRECTOR_STANDARD_SCRIPT_SLICE_V2' : 'DIRECTOR_STANDARD_SCRIPT_SLICE_V1',
        assemblerVersion: v5 ? 'standard-script-compiler-v5' : performance ? 'standard-script-compiler-v4' : mixed ? 'standard-script-compiler-v3' : v2 ? 'standard-script-compiler-v2' : 'standard-script-compiler-v1',
        binding: {
            ...sourceBinding,
            planDigest: digest(plan)
        },
        script,
        annotations,
        exclusions,
        evidence: index.units.map((unit)=>({
                ...unit,
                sourceVersion: index.sourceVersion,
                offsetUnit: 'UTF16'
            })),
        fidelity: {
            assignments
        },
        readiness: {
            mechanical: 'VERIFIED',
            semantic: 'NOT_RUN',
            acceptedForDispatch: false,
            blockers: [
                'SEMANTIC_REVIEW_NOT_RUN',
                'PRODUCTION_INTEGRATION_NOT_IMPLEMENTED'
            ]
        }
    };
    const packageDigest = digest(payload);
    const receipt = {
        schemaVersion: v5 ? 'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V5' : performance ? 'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V4' : mixed ? 'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V3' : v2 ? 'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V2' : 'STANDARD_SCRIPT_MECHANICAL_RECEIPT_V1',
        status: 'MECHANICALLY_VERIFIED',
        ...sourceBinding,
        planDigest: payload.binding.planDigest,
        packageDigest,
        readingTextDigest: digest(readingText),
        assemblerVersion: payload.assemblerVersion,
        checks: {
            unitCoverage: 'VERIFIED',
            sourceOrder: 'VERIFIED',
            sourceSlices: 'VERIFIED',
            fieldBoundaries: 'VERIFIED',
            exclusionShape: 'VERIFIED',
            noTextPatches: 'VERIFIED',
            ...grouped ? {
                dialogueGroups: 'VERIFIED'
            } : {},
            ...postCues ? {
                postColonCues: 'VERIFIED'
            } : {},
            ...mixed ? {
                mixedBodyGroups: 'VERIFIED'
            } : {},
            ...performance ? {
                inlinePerformance: 'VERIFIED'
            } : {}
        },
        semantic: 'NOT_RUN',
        acceptedForDispatch: false
    };
    return {
        package: {
            ...payload,
            packageId: `standard:${packageDigest}`,
            fidelity: {
                ...payload.fidelity,
                receipt
            }
        },
        readingText,
        receipt
    };
}
