use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

pub fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceDialogue {
    pub id: String,
    pub speaker: String,
    /// Explicit joint-label members; absent on every historical source dialect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speakers: Vec<SourceSegment>,
    pub channel: String,
    pub quote: String,
    pub range: SourceRange,
    #[serde(default)]
    pub spoken_segments: Vec<SourceSegment>,
    #[serde(default)]
    pub performance_segments: Vec<SourceSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSegment {
    pub text: String,
    pub range: SourceRange,
}

pub fn dialogue_speaker_names(dialogue: &SourceDialogue) -> Vec<&str> {
    if dialogue.speakers.is_empty() { vec![dialogue.speaker.as_str()] }
    else { dialogue.speakers.iter().map(|member| member.text.as_str()).collect() }
}

pub fn spoken_text(dialogue: &SourceDialogue) -> String {
    dialogue
        .spoken_segments
        .iter()
        .map(|part| part.text.as_str())
        .collect()
}

// Bracketed directions outside literal quotation are separate source evidence,
// never invented text. Nested brackets remain one complete outer segment.
fn bracket_segments(text: &str, origin: usize) -> Vec<SourceSegment> {
    let mut parts = Vec::new();
    let mut stack = Vec::new();
    let mut begin = 0;
    let mut quotation = None;
    for (byte, ch) in text.char_indices() {
        if stack.is_empty() {
            match ch {
                '“' | '「' | '『' if quotation.is_none() => {
                    quotation = Some(match ch {
                        '“' => '”',
                        '「' => '」',
                        _ => '』',
                    });
                    continue;
                }
                '"' if quotation.is_none() => {
                    quotation = Some('"');
                    continue;
                }
                _ if quotation == Some(ch) => {
                    quotation = None;
                    continue;
                }
                _ if quotation.is_some() => continue,
                _ => {}
            }
        }
        let close = match ch {
            '（' => Some('）'),
            '(' => Some(')'),
            '[' => Some(']'),
            '【' => Some('】'),
            _ => None,
        };
        if let Some(close) = close {
            if stack.is_empty() {
                begin = byte;
            }
            stack.push(close);
        } else if stack.last() == Some(&ch) {
            stack.pop();
            if stack.is_empty() {
                let end = byte + ch.len_utf8();
                parts.push(SourceSegment {
                    text: text[begin..end].into(),
                    range: SourceRange {
                        start: origin + text[..begin].encode_utf16().count(),
                        end: origin + text[..end].encode_utf16().count(),
                    },
                });
            }
        }
    }
    parts
}

fn dialogue_segments(
    line: &str,
    start: usize,
    quote_start: usize,
    quote_range: &SourceRange,
) -> (Vec<SourceSegment>, Vec<SourceSegment>) {
    let mut performance = bracket_segments(&line[..quote_start], start);
    let inline = bracket_segments(&line[quote_start..], quote_range.start);
    let mut spoken = Vec::new();
    let mut cursor = quote_range.start;
    for cue in &inline {
        if cursor < cue.range.start {
            let range = SourceRange {
                start: cursor,
                end: cue.range.start,
            };
            spoken.push(SourceSegment {
                text: slice_utf16(
                    line,
                    &SourceRange {
                        start: range.start - start,
                        end: range.end - start,
                    },
                )
                .unwrap(),
                range,
            });
        }
        cursor = cue.range.end;
    }
    if cursor < quote_range.end {
        let range = SourceRange {
            start: cursor,
            end: quote_range.end,
        };
        spoken.push(SourceSegment {
            text: slice_utf16(
                line,
                &SourceRange {
                    start: range.start - start,
                    end: range.end - start,
                },
            )
            .unwrap(),
            range,
        });
    }
    performance.extend(inline);
    (spoken, performance)
}

// System announcements commonly wrap the actual spoken message in 【】.
// That typography is not an acting cue. Keep the original characters and
// coordinates; only change the classification of this unambiguous wrapper.
fn system_message_segments(dialogue: &mut SourceDialogue) {
    if !["系统", "系统音"].contains(&dialogue.speaker.as_str())
        || !["VO", "系统音"].contains(&dialogue.channel.as_str())
        || !dialogue.quote.starts_with('【')
        || !dialogue.quote.ends_with('】')
        || dialogue
            .performance_segments
            .iter()
            .filter(|part| part.range.start >= dialogue.range.start)
            .count()
            != 1
        || !dialogue.spoken_segments.is_empty()
    {
        return;
    }
    dialogue
        .performance_segments
        .retain(|part| part.range.end <= dialogue.range.start);
    dialogue.spoken_segments.push(SourceSegment {
        text: dialogue.quote.clone(),
        range: dialogue.range.clone(),
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRow {
    pub id: String,
    pub number: usize,
    pub range: SourceRange,
    pub text: String,
    pub kind: String,
    pub episode_id: Option<String>,
    pub scene_id: Option<String>,
    pub dialogue: Option<SourceDialogue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEpisode {
    pub id: String,
    pub number: u32,
    pub source_number: String,
    pub title: String,
    pub row_ids: Vec<String>,
    pub scene_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceScene {
    pub id: String,
    pub episode_id: String,
    pub title: String,
    pub row_ids: Vec<String>,
    #[serde(default, flatten)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDiagnostic {
    pub code: String,
    pub row_ids: Vec<String>,
    pub message: String,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedSource {
    pub schema_version: String,
    /// Missing on earlier saved sources: their original classification contract
    /// stays valid until the user explicitly requests recognition again.
    #[serde(default = "legacy_parser_version")]
    pub parser_version: String,
    pub text: String,
    pub sha256: String,
    pub coordinate_system: String,
    pub length: usize,
    pub rows: Vec<SourceRow>,
    pub episodes: Vec<SourceEpisode>,
    pub scenes: Vec<SourceScene>,
    pub diagnostics: Vec<SourceDiagnostic>,
    #[serde(default)]
    pub boundary_decisions: Vec<BoundaryDecision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_edits: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryDecision {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub episode_number: Option<u32>,
    pub scene_title: Option<String>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<BoundaryConfirmation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryConfirmation {
    pub source_version: i64,
    pub source_sha256: String,
    pub parser_version: String,
    pub heading_row_id: String,
    pub diagnostic_code: String,
}

const LEGACY_PARSER_VERSION: &str = "source-parser/v1";
const CONTEXT_PARSER_VERSION: &str = "source-parser/v2";
const CURRENT_PARSER_VERSION: &str = "source-parser/v3";
const STRUCTURAL_PARSER_VERSION: &str = "source-parser/v4";
const DISPLAY_PARSER_VERSION: &str = "source-parser/v5";
const FIDELITY_PARSER_VERSION: &str = "source-parser/v6";
const MULTILINE_DISPLAY_PARSER_VERSION: &str = "source-parser/v7";
const JOINT_SPEAKER_PARSER_VERSION: &str = "source-parser/v8";
const TITLE_METADATA_PARSER_VERSION: &str = "source-parser/v9";
const EXPLICIT_INLINE_PARSER_VERSION: &str = "source-parser/v10";
const FORMAT_PARSER_VERSION: &str = "source-parser/v11";

#[path = "source_explicit_inline.rs"]
mod explicit_inline;

fn uses_joint_speaker_rules(version: &str) -> bool {
    [JOINT_SPEAKER_PARSER_VERSION, TITLE_METADATA_PARSER_VERSION, EXPLICIT_INLINE_PARSER_VERSION, FORMAT_PARSER_VERSION].contains(&version)
}

fn uses_fidelity_rules(version: &str) -> bool {
    [FIDELITY_PARSER_VERSION, MULTILINE_DISPLAY_PARSER_VERSION, JOINT_SPEAKER_PARSER_VERSION, TITLE_METADATA_PARSER_VERSION, EXPLICIT_INLINE_PARSER_VERSION, FORMAT_PARSER_VERSION].contains(&version)
}

fn display_block_header(text: &str) -> bool {
    static HEADER: OnceLock<Regex> = OnceLock::new();
    HEADER.get_or_init(|| Regex::new(r"^[△▲]?\s*(?:面板|屏幕|界面|投影|显示屏)(?:上的?|中)?(?:字幕|文字|内容|显示内容)\s*[：:]\s*$").unwrap()).is_match(text.trim())
}

fn numbered_display_field_re() -> &'static Regex {
    static FIELD: OnceLock<Regex> = OnceLock::new();
    FIELD.get_or_init(|| {
        Regex::new(
            r"^(?:情报|任务|消息|提示|目标|奖励|内容|选项)[零〇一二三四五六七八九十百千\d]+$",
        )
        .unwrap()
    })
}

fn declared_speaker_names(source: &LockedSource) -> HashSet<String> {
    static CAST: OnceLock<Regex> = OnceLock::new();
    let cast = CAST.get_or_init(|| {
        Regex::new(r"^\s*(?:人物|出场人物|出场角色|角色)\s*[：:]\s*(.*)$").unwrap()
    });
    source
        .rows
        .iter()
        .filter_map(|r| cast.captures(r.text.trim()))
        .flat_map(|c| {
            c[1].split(['、', '，', ',', '；', ';'])
                .map(|name| {
                    name.trim()
                        .split(['（', '('])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn display_enclosure_field(line: &str) -> Option<&str> {
    let inner = line.trim_start().strip_prefix('【')?;
    let (label, _) = inner.split_once(['：', ':'])?;
    let label = label.trim();
    numbered_display_field_re().is_match(label).then_some(label)
}

/// A complete, row-started 【numbered field: ...】 enclosure establishes a
/// display structure across lines. Its interior field names are not a blacklist
/// of speakers: the same labels outside that exact enclosure remain dialogue.
/// Incomplete enclosures and explicit speech cues never grant an exemption.
fn multiline_display_row_ids(source: &LockedSource, named: &HashSet<String>) -> HashSet<String> {
    if ![MULTILINE_DISPLAY_PARSER_VERSION, JOINT_SPEAKER_PARSER_VERSION, TITLE_METADATA_PARSER_VERSION, EXPLICIT_INLINE_PARSER_VERSION, FORMAT_PARSER_VERSION].contains(&source.parser_version.as_str()) {
        return HashSet::new();
    }
    let mut ids = HashSet::new();
    for (index, first) in source.rows.iter().enumerate() {
        if first.kind != "body" || ids.contains(&first.id) {
            continue;
        }
        let Some(label) = display_enclosure_field(&first.text) else {
            continue;
        };
        if named.contains(label) {
            continue;
        }
        let mut stack = Vec::new();
        let mut quotation = None;
        let mut covered = Vec::new();
        'candidate: for (relative, row) in source.rows[index..].iter().enumerate() {
            if row.episode_id != first.episode_id
                || row.scene_id != first.scene_id
                || row.kind != "body" && !row.text.trim().is_empty()
            {
                break;
            }
            if relative > 0 {
                if let Some(dialogue) =
                    row_dialogue(&row.text, row.range.start, &row.id, &source.parser_version)
                {
                    if dialogue.channel != "现场"
                        || named.contains(&dialogue.speaker)
                        || dialogue
                            .performance_segments
                            .iter()
                            .any(|part| part.range.end <= dialogue.range.start)
                    {
                        break;
                    }
                }
            }
            covered.push(row.id.clone());
            let line = if relative == 0 {
                row.text.trim_start()
            } else {
                row.text.as_str()
            };
            for (byte, ch) in line.char_indices() {
                if quotation == Some(ch) {
                    quotation = None;
                    continue;
                }
                if quotation.is_some() {
                    continue;
                }
                if let Some(close) = match ch {
                    '“' => Some('”'),
                    '「' => Some('」'),
                    '『' => Some('』'),
                    '"' => Some('"'),
                    _ => None,
                } {
                    quotation = Some(close);
                    continue;
                }
                if let Some(close) = match ch {
                    '【' => Some('】'),
                    '[' => Some(']'),
                    '（' => Some('）'),
                    '(' => Some(')'),
                    _ => None,
                } {
                    stack.push(close);
                } else if ['】', ']', '）', ')'].contains(&ch) {
                    if stack.pop() != Some(ch) {
                        break 'candidate;
                    }
                    if stack.is_empty() {
                        if relative > 0 && line[byte + ch.len_utf8()..].trim().is_empty() {
                            ids.extend(covered);
                        }
                        break 'candidate;
                    }
                }
            }
        }
    }
    ids
}

/// Only an explicit display header or the versioned complete enclosure grants
/// a non-spoken role. Ordinary dialogue and voice labels outside it are intact.
fn display_only_row_ids(source: &LockedSource) -> HashSet<String> {
    if ![
        DISPLAY_PARSER_VERSION,
        FIDELITY_PARSER_VERSION,
        MULTILINE_DISPLAY_PARSER_VERSION,
        JOINT_SPEAKER_PARSER_VERSION,
        TITLE_METADATA_PARSER_VERSION,
        EXPLICIT_INLINE_PARSER_VERSION,
        FORMAT_PARSER_VERSION,
    ]
    .contains(&source.parser_version.as_str())
    {
        return HashSet::new();
    }
    let field = numbered_display_field_re();
    let named = declared_speaker_names(source);
    let mut scope = None;
    let mut ids = multiline_display_row_ids(source, &named);
    for row in &source.rows {
        if display_block_header(&row.text) && row.kind == "body" {
            scope = Some((&row.episode_id, &row.scene_id));
            continue;
        }
        if row.text.trim().is_empty() {
            continue;
        }
        if scope
            .is_some_and(|(episode, scene)| episode == &row.episode_id && scene == &row.scene_id)
            && row.kind == "body"
            && row
                .text
                .split(['：', ':'])
                .next()
                .is_some_and(|head| field.is_match(head.trim()))
            && parse_dialogue(&row.text, row.range.start, &row.id, &source.parser_version)
                .is_some_and(|q| q.channel == "现场" && !named.contains(&q.speaker))
        {
            ids.insert(row.id.clone());
        } else {
            scope = None;
        }
    }
    ids
}

fn episode_heading_re() -> &'static Regex {
    static EPISODE_HEADING: OnceLock<Regex> = OnceLock::new();
    EPISODE_HEADING.get_or_init(|| {
        Regex::new(r"^(?:#{1,6}\s*)?第\s*([零〇一二三四五六七八九十百千两\d]+)\s*集(?:\s|[:：｜|—-]|$)(.*)$").unwrap()
    })
}

fn tight_scene_heading_re() -> &'static Regex {
    static TIGHT_SCENE_HEADING: OnceLock<Regex> = OnceLock::new();
    TIGHT_SCENE_HEADING.get_or_init(|| {
        // A missing separator is safe only before an explicit metadata token.
        // Numeric prose such as 3.14元 or 3-1日记 is not a scene heading.
        Regex::new(r"^(?:#{1,6}\s*)?(?:场\s*)?(\d+)\s*[-－.．]\s*(\d+)((?:日|夜|昼|晨|黄昏|清晨|凌晨|傍晚|白天|夜晚|内|外|内景|外景|INT\.?|EXT\.?)(?:[\s/／|｜].*)?)$").unwrap()
    })
}

// Only opt fresh parsing into the changed boundary contract when it is needed.
// Existing v1-v3 sources always replay their original parsing and coordinates.
fn prior_fresh_parser_version(text: &str) -> &'static str {
    if raw_lines(text)
        .iter()
        .any(|line| display_block_header(line))
    {
        return DISPLAY_PARSER_VERSION;
    }
    let mut previous = None;
    for raw in raw_lines(text) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if tight_scene_heading_re().is_match(line) {
            return STRUCTURAL_PARSER_VERSION;
        }
        let number = episode_heading_re()
            .captures(line)
            .and_then(|c| number_value(c.get(1)?.as_str()));
        if number == Some(0) || number.is_some() && number == previous {
            return STRUCTURAL_PARSER_VERSION;
        }
        previous = number;
    }
    CURRENT_PARSER_VERSION
}

// Compare complete deterministic projections so unaffected inputs retain their
// established contract. Saved sources never pass through this opt-in selector.
fn prior_display_parser_version(text: &str) -> &'static str {
    let mut prior = prior_fresh_parser_version(text);
    if let (Ok(old), Ok(new)) = (
        parse_source(text, prior),
        parse_source(text, FIDELITY_PARSER_VERSION),
    ) {
        let mut old = serde_json::to_value(old).unwrap();
        let mut new = serde_json::to_value(new).unwrap();
        old.as_object_mut().unwrap().remove("parserVersion");
        new.as_object_mut().unwrap().remove("parserVersion");
        if old != new {
            prior = FIDELITY_PARSER_VERSION;
        }
    }
    if !raw_lines(text)
        .iter()
        .any(|line| display_enclosure_field(line).is_some())
    {
        return prior;
    }
    if let (Ok(old), Ok(new)) = (
        parse_source(text, prior),
        parse_source(text, MULTILINE_DISPLAY_PARSER_VERSION),
    ) {
        let mut old = serde_json::to_value(old).unwrap();
        let mut new = serde_json::to_value(new).unwrap();
        old.as_object_mut().unwrap().remove("parserVersion");
        new.as_object_mut().unwrap().remove("parserVersion");
        if old != new {
            return MULTILINE_DISPLAY_PARSER_VERSION;
        }
    }
    prior
}

// Joint speech is a new import / explicit re-recognition contract. Never
// reinterpret an already frozen v1-v7 source during load or recovery.
fn prior_joint_parser_version(text: &str) -> &'static str {
    let prior = prior_display_parser_version(text);
    static JOINT_HEAD: OnceLock<Regex> = OnceLock::new();
    let possible = JOINT_HEAD.get_or_init(|| Regex::new(r"^\s*[\p{L}\p{M}\p{N}·・_]{1,24}\s*[&＆、与和][^:：\r\n]*[：:]").unwrap());
    if !raw_lines(text).iter().any(|line| possible.is_match(line)) { return prior; }
    if let (Ok(old), Ok(new)) = (parse_source(text, prior), parse_source(text, JOINT_SPEAKER_PARSER_VERSION)) {
        let mut old = serde_json::to_value(old).unwrap();
        let mut new = serde_json::to_value(new).unwrap();
        old.as_object_mut().unwrap().remove("parserVersion");
        new.as_object_mut().unwrap().remove("parserVersion");
        if old != new { return JOINT_SPEAKER_PARSER_VERSION; }
    }
    prior
}

// A cover title is metadata only with a completely explicit, local shape:
// independent book-title line, optional edition/episode-count line, then an
// actual positive episode heading. No arbitrary intervening preface is hidden.
// Book references inside a scene, title-plus-action lines, and unknown prose
// still require a source-bound decision. Original rows/bytes are never removed.
fn title_metadata_row_indices(text: &str) -> HashSet<usize> {
    let lines = raw_lines(text);
    let mut nonempty = lines.iter().enumerate().filter(|(_, line)| !line.trim().is_empty());
    let Some((title_index, title)) = nonempty.next() else { return HashSet::new(); };
    let Some(title) = title.trim().strip_prefix('《').and_then(|value| value.strip_suffix('》')) else { return HashSet::new(); };
    if title.trim().is_empty() || title.chars().count() > 80
        || title.contains(['《', '》', '：', ':', '。', '！', '？', '!', '?', '△', '▲', '“', '”'])
    { return HashSet::new(); }
    let Some((next_index, next)) = nonempty.next() else { return HashSet::new(); };
    static FORMAT: OnceLock<Regex> = OnceLock::new();
    let format = FORMAT.get_or_init(|| Regex::new(r"^(?:共|全)?\s*([零〇一二三四五六七八九十百千\d]+)\s*集\s*(?:短剧漫剧|动画短剧|动画漫剧|短剧|漫剧|动画)?\s*剧本$").unwrap());
    let format_row = format.captures(next.trim())
        .and_then(|capture| number_value(capture.get(1)?.as_str()))
        .is_some_and(|number| number > 0);
    let episode_line = if format_row {
        let Some((_, episode)) = nonempty.next() else { return HashSet::new(); };
        episode.trim()
    } else { next.trim() };
    if !episode_heading_re().captures(episode_line)
        .and_then(|capture| number_value(capture.get(1)?.as_str()))
        .is_some_and(|number| number > 0)
    { return HashSet::new(); }
    let mut indices = HashSet::from([title_index]);
    if format_row { indices.insert(next_index); }
    indices
}

fn prior_title_parser_version(text: &str) -> &'static str {
    let prior = prior_joint_parser_version(text);
    if title_metadata_row_indices(text).is_empty() { return prior; }
    if let (Ok(old), Ok(new)) = (parse_source(text, prior), parse_source(text, TITLE_METADATA_PARSER_VERSION)) {
        let mut old = serde_json::to_value(old).unwrap();
        let mut new = serde_json::to_value(new).unwrap();
        old.as_object_mut().unwrap().remove("parserVersion");
        new.as_object_mut().unwrap().remove("parserVersion");
        if old != new { return TITLE_METADATA_PARSER_VERSION; }
    }
    prior
}

fn prior_inline_parser_version(text: &str) -> &'static str {
    let prior = prior_title_parser_version(text);
    if let (Ok(old), Ok(new)) = (parse_source(text, prior), parse_source(text, EXPLICIT_INLINE_PARSER_VERSION)) {
        // Only an actual explicit-inline change opts new imports into v10.
        if explicit_inline::changes_dialogue_rows(&old, &new) { return EXPLICIT_INLINE_PARSER_VERSION; }
    }
    prior
}

// Only explicit recognition / new imports choose a newer dialect. Replaying
// saved sources always uses their recorded version and original decisions.
fn fresh_parser_version(text: &str) -> &'static str {
    let prior = prior_inline_parser_version(text);
    static ZERO_SCENE: OnceLock<Regex> = OnceLock::new();
    let zero_scene = ZERO_SCENE.get_or_init(|| Regex::new(
        r"^(?:#{1,6}\s*)?(?:第\s*[零〇一二三四五六七八九十百千两\d]+\s*集.*?场\s*|场\s*)?[1-9]\d*\s*[-－.．]\s*0(?:\s|[:：]|日|夜|昼|晨|黄昏|清晨|凌晨|傍晚|白天|夜晚|内|外|INT|EXT|$)"
    ).unwrap());
    if biography_preface_rows(text).is_empty()
        && !raw_lines(text).iter().any(|line| zero_scene.is_match(line.trim()))
    { return prior; }
    if let (Ok(old), Ok(new)) = (parse_source(text, prior), parse_source(text, FORMAT_PARSER_VERSION)) {
        let mut old = serde_json::to_value(old).unwrap();
        let mut new = serde_json::to_value(new).unwrap();
        old.as_object_mut().unwrap().remove("parserVersion");
        new.as_object_mut().unwrap().remove("parserVersion");
        if old != new { return FORMAT_PARSER_VERSION; }
    }
    prior
}

fn uses_explicit_inline_rules(version: &str) -> bool {
    [EXPLICIT_INLINE_PARSER_VERSION, FORMAT_PARSER_VERSION].contains(&version)
}

// Strip only balanced presentation wrappers for evidence matching; the actual
// source row, UTF-16 coordinates and identifiers always retain original bytes.
fn format_heading_text(line: &str) -> &str {
    let mut value = line.trim();
    let hashes = value.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) { value = value[hashes..].trim(); }
    if let Some(inner) = value.strip_prefix("**").and_then(|v| v.strip_suffix("**")) {
        value = inner.trim();
    }
    for (open, close) in [('【', '】'), ('[', ']'), ('（', '）'), ('(', ')')] {
        if let Some(inner) = value.strip_prefix(open).and_then(|v| v.strip_suffix(close)) {
            value = inner.trim();
            break;
        }
    }
    value
}

fn biography_heading(line: &str) -> bool {
    static HEADING: OnceLock<Regex> = OnceLock::new();
    HEADING.get_or_init(|| Regex::new(
        r"^(?:(?:主要)?(?:人物|角色)(?:小传|档案)(?:\s*[：:]\s*[\p{L}\p{M}\p{N}·・]{1,24})?|(?:(?:主角|男主角|女主角|男主|女主|角色|人物)\s*[：:]\s*)?[\p{L}\p{M}\p{N}·・]{1,24}\s+(?:人物|角色)(?:小传|档案))$"
    ).unwrap()).is_match(format_heading_text(line))
}

fn biography_field(line: &str) -> Option<String> {
    let value = format_heading_text(line);
    let (name, content) = value.split_once(['：', ':'])?;
    let name = name.trim().trim_matches('*').trim();
    (!content.trim().is_empty() && ["姓名", "名字", "身份", "年龄", "性别", "出身", "职业", "背景", "性格", "外貌", "所属", "角色定位"].contains(&name))
        .then(|| name.to_owned())
}

fn biography_cover(line: &str) -> bool {
    let value = format_heading_text(line);
    if value.is_empty() || value.chars().count() > 80
        || value.contains(['。', '！', '？', '!', '?', '△', '▲', '：', ':', '“', '”'])
    { return false; }
    if value.strip_prefix('《').and_then(|v| v.strip_suffix('》'))
        .is_some_and(|v| !v.trim().is_empty() && !v.contains(['《', '》']))
    { return true; }
    static COVER: OnceLock<Regex> = OnceLock::new();
    COVER.get_or_init(|| Regex::new(
        r"^.+?(?:共|全)?\s*([零〇一二三四五六七八九十百千\d]+)\s*集\s*(?:短剧漫剧|动画短剧|动画漫剧|短剧|漫剧|动画)?\s*剧本$"
    ).unwrap()).captures(value)
        .and_then(|c| number_value(c.get(1)?.as_str())).is_some_and(|number| number > 0)
}

fn biography_production_boundary(line: &str) -> bool {
    let value = format_heading_text(line);
    static BARE_SCENE: OnceLock<Regex> = OnceLock::new();
    episode_heading_re().is_match(value)
        || value.starts_with(['△', '▲'])
        || ["正文", "剧本正文", "序章", "序幕"].iter()
            .any(|word| value == *word || value.starts_with(&format!("{word}：")) || value.starts_with(&format!("{word} ")))
        || value.starts_with("场景：") || value.starts_with("场景:")
        || BARE_SCENE.get_or_init(|| Regex::new(r"^(?:场\s*)?\d+\s*[-－]\s*\d+\s*[:：]?$").unwrap()).is_match(value)
        || fidelity_pair_re().captures(value).is_some_and(|c| fidelity_pair_valid(&c))
        || value.starts_with("第") && value.contains("场")
}

fn biography_preface_rows(text: &str) -> HashSet<usize> {
    let lines = raw_lines(text);
    let nonempty: Vec<_> = lines.iter().enumerate().filter(|(_, line)| !line.trim().is_empty()).collect();
    let Some(&(first, first_line)) = nonempty.first() else { return HashSet::new(); };
    let heading_position = if biography_heading(first_line) { 0 }
        else if biography_cover(first_line) && nonempty.get(1).is_some_and(|(_, line)| biography_heading(line)) { 1 }
        else { return HashSet::new(); };
    let heading_index = nonempty[heading_position].0;
    let end = lines.iter().enumerate().skip(heading_index + 1)
        .find(|(_, line)| biography_production_boundary(line)).map(|(index, _)| index).unwrap_or(lines.len());
    let anchors: Vec<_> = nonempty.iter().skip(heading_position + 1)
        .take_while(|(index, _)| *index < end).take(3).map(|(_, line)| *line).collect();
    let Some(anchor) = anchors.first() else { return HashSet::new(); };
    let evidence = if ["基本资料", "基本信息", "基础资料", "基础信息", "人物资料", "角色资料", "角色信息"].contains(&format_heading_text(anchor)) {
        anchors.get(1).is_some_and(|line| biography_field(line).is_some())
    } else {
        biography_field(anchor).zip(anchors.get(1).and_then(|line| biography_field(line)))
            .is_some_and(|(first, second)| first != second)
    };
    if !evidence { return HashSet::new(); }
    (first..end).filter(|index| !lines[*index].trim().is_empty()).collect()
}

fn valid_numbered_scene(value: &str, parser_version: &str) -> bool {
    value.parse::<u32>().is_ok_and(|number| number > 0 || parser_version == FORMAT_PARSER_VERSION)
}

fn scene_separator(ch: char) -> bool {
    ch.is_whitespace() || [',', '，', '、', '/', '／', '|', '｜'].contains(&ch)
}

fn fidelity_pair_re() -> &'static Regex {
    static PAIR: OnceLock<Regex> = OnceLock::new();
    PAIR.get_or_init(|| {
        Regex::new(r"^(?:#{1,6}\s*)?(?:场\s*)?(\d+)\s*[-－.．]\s*(\d+)[\s,，、]*(.+)$").unwrap()
    })
}

fn metadata_token(value: &str) -> bool {
    [
        "日", "夜", "昼", "晨", "黄昏", "清晨", "凌晨", "傍晚", "白天", "夜晚", "内", "外", "内景",
        "外景", "INT", "EXT", "INT.", "EXT.",
    ]
    .contains(&value)
}

fn fidelity_pair_valid(captures: &regex::Captures<'_>) -> bool {
    let payload = captures.get(3).unwrap();
    // Preserve established location-first dashed headings. Decimal-looking
    // prefixes require metadata first so a duration cannot become a scene.
    !captures.get(0).unwrap().as_str()[..payload.start()].contains(['.', '．'])
        || fidelity_scene_payload(payload.as_str())
}

fn fidelity_scene_payload(payload: &str) -> bool {
    // A numeric pair must be followed immediately by a standalone scene token.
    // Finding 日/内 later in a sentence does not turn 1.2 秒后 into a scene.
    payload
        .split(scene_separator)
        .find(|part| !part.is_empty())
        .is_some_and(metadata_token)
}

// Byte positions only select slices of the ORIGINAL line. Every returned source
// coordinate is computed from the exact prefix's UTF-16 length, never byte math.
fn inline_scene_dialogue(text: &str, start: usize, row_id: &str) -> Option<SourceDialogue> {
    let line = text.trim_end_matches(['\r', '\n']);
    let leading = line.len() - line.trim_start().len();
    let captures = fidelity_pair_re().captures(&line[leading..])?;
    let payload = captures.get(3)?;
    if !fidelity_scene_payload(payload.as_str()) {
        return None;
    }
    let payload_start = leading + payload.start();
    for (index, ch) in line[payload_start..].char_indices() {
        if !scene_separator(ch) {
            continue;
        }
        let next = payload_start + index + ch.len_utf8();
        let prefix = &line[payload_start..next];
        if prefix.contains([':', '：']) {
            return None;
        }
        // There must be a separate place label before the speaker. Without it
        // the scene/location and speaker boundary is not uniquely established.
        if !prefix
            .split(scene_separator)
            .any(|token| !token.is_empty() && !metadata_token(token))
        {
            continue;
        }
        if let Some(dialogue) = parse_dialogue(
            &line[next..],
            start + line[..next].encode_utf16().count(),
            row_id,
            FIDELITY_PARSER_VERSION,
        ) {
            return Some(dialogue);
        }
    }
    None
}

fn row_dialogue(text: &str, start: usize, row_id: &str, version: &str) -> Option<SourceDialogue> {
    parse_dialogue(text, start, row_id, version).or_else(|| {
        uses_fidelity_rules(version)
            .then(|| inline_scene_dialogue(text, start, row_id))
            .flatten()
    })
}

fn legacy_parser_version() -> String {
    LEGACY_PARSER_VERSION.into()
}

fn context_heading(line: &str, inline_allowed: bool) -> bool {
    let line = line.trim();
    let hashes = line.chars().take_while(|c| *c == '#').count();
    let line = if (1..=6).contains(&hashes) {
        line[hashes..].trim_start()
    } else {
        line
    };
    let (title, suffix) = line.split_once([':', '：']).unwrap_or((line, ""));
    if !suffix.trim().is_empty() && !inline_allowed {
        return false;
    }
    let title = title.trim();
    let title = [('【', '】'), ('[', ']'), ('（', '）'), ('(', ')')]
        .iter()
        .find_map(|(open, close)| title.strip_prefix(*open)?.strip_suffix(*close))
        .unwrap_or(title)
        .trim();
    [
        "作者资料",
        "故事大纲",
        "故事简介",
        "剧情简介",
        "内容简介",
        "剧本简介",
        "人物关系",
        "人物小传",
        "人物设定",
        "角色小传",
        "角色设定",
        "角色关系",
        "世界观",
        "世界观设定",
        "背景资料",
        "背景设定",
        "段纲",
        "分集大纲",
        "附录",
    ]
    .contains(&title)
}

pub fn slice_utf16(text: &str, range: &SourceRange) -> Result<String, String> {
    if range.end < range.start {
        return Err("来源区间结束早于开始".into());
    }
    let mut offset = 0;
    let mut start = None;
    let mut end = None;
    for (byte, ch) in text.char_indices() {
        if offset == range.start {
            start = Some(byte);
        }
        if offset == range.end {
            end = Some(byte);
        }
        offset += ch.len_utf16();
    }
    if offset == range.start {
        start = Some(text.len());
    }
    if offset == range.end {
        end = Some(text.len());
    }
    match (start, end) {
        (Some(a), Some(b)) => Ok(text[a..b].to_string()),
        _ => Err("来源区间越界或切入 UTF-16 代理对".into()),
    }
}

fn raw_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let (mut cursor, mut start) = (0, 0);
    let mut rows = Vec::new();
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\r' => {
                cursor += 1;
                if bytes.get(cursor) == Some(&b'\n') {
                    cursor += 1;
                }
                rows.push(&text[start..cursor]);
                start = cursor;
            }
            b'\n' => {
                cursor += 1;
                rows.push(&text[start..cursor]);
                start = cursor;
            }
            _ => cursor += 1,
        }
    }
    if start < text.len() {
        rows.push(&text[start..]);
    }
    rows
}

fn number_value(value: &str) -> Option<u32> {
    if let Ok(number) = value.parse() {
        return Some(number);
    }
    let (mut total, mut digit) = (0_u32, 0_u32);
    for ch in value.chars() {
        match ch {
            '零' | '〇' => digit = 0,
            '一' => digit = 1,
            '二' | '两' => digit = 2,
            '三' => digit = 3,
            '四' => digit = 4,
            '五' => digit = 5,
            '六' => digit = 6,
            '七' => digit = 7,
            '八' => digit = 8,
            '九' => digit = 9,
            '十' => {
                total += digit.max(1) * 10;
                digit = 0;
            }
            '百' => {
                total += digit.max(1) * 100;
                digit = 0;
            }
            '千' => {
                total += digit.max(1) * 1000;
                digit = 0;
            }
            _ => return None,
        }
    }
    Some(total + digit)
}

fn parse_dialogue(
    text: &str,
    start: usize,
    row_id: &str,
    parser_version: &str,
) -> Option<SourceDialogue> {
    parse_dialogue_with_label(text, start, row_id, parser_version, None)
}

fn parse_dialogue_with_label(text: &str, start: usize, row_id: &str, parser_version: &str, joint_label: Option<&str>) -> Option<SourceDialogue> {
    let line = text.trim_end_matches(['\r', '\n']);
    // Only an explicit leading label is a lexical speaker; no colon inference inside a sentence.
    let mut depth = 0_i32;
    let mut colon = None;
    for (byte, ch) in line.char_indices() {
        match ch {
            '（' | '(' | '[' | '【' => depth += 1,
            '）' | ')' | ']' | '】' => depth -= 1,
            ':' | '：' if depth == 0 => {
                colon = Some(byte);
                break;
            }
            _ => {}
        }
        if depth < 0 || byte > 256 {
            return None;
        }
    }
    let colon = colon?;
    let head = line[..colon].trim();
    static HEAD: OnceLock<Regex> = OnceLock::new();
    let re = HEAD.get_or_init(|| Regex::new(r"^([\p{L}\p{N}·・_]{1,24})(.*)$").unwrap());
    static FIDELITY_HEAD: OnceLock<Regex> = OnceLock::new();
    let re = if uses_fidelity_rules(parser_version) {
        FIDELITY_HEAD.get_or_init(|| Regex::new(r"^([\p{L}\p{M}\p{N}·・_]{1,24})(.*)$").unwrap())
    } else {
        re
    };
    let captures;
    let (label, tail) = if let Some(label) = joint_label {
        (label, head.strip_prefix(label)?)
    } else {
        captures = re.captures(head)?;
        (captures.get(1)?.as_str(), captures.get(2).map(|m| m.as_str()).unwrap_or(""))
    };
    if !label.chars().any(char::is_alphabetic) {
        return None;
    }
    if [
        "场景",
        "地点",
        "时间",
        "时刻",
        "人物",
        "出场人物",
        "道具",
        "旁注",
        "备注",
        "画面",
        "声音",
    ]
    .contains(&label)
    {
        return None;
    }
    let mut hint = String::new();
    let mut channel_text = String::new();
    let mut hint_depth = 0_i32;
    for ch in tail.chars() {
        match ch {
            '（' | '(' | '[' | '【' => hint_depth += 1,
            '）' | ')' | ']' | '】' => hint_depth -= 1,
            _ if hint_depth > 0 => hint.push(ch),
            _ if !ch.is_whitespace() => channel_text.push(ch),
            _ => {}
        }
    }
    // Existing frozen sources keep their original lexical metadata. Only the
    // new parser treats explicit ASCII voice labels case-insensitively, as the
    // original runtime did; source text and the speaker's own case stay intact.
    let normalized_channel;
    let normalized_label;
    let current_voice_rules = [
        CURRENT_PARSER_VERSION,
        STRUCTURAL_PARSER_VERSION,
        DISPLAY_PARSER_VERSION,
        FIDELITY_PARSER_VERSION,
        MULTILINE_DISPLAY_PARSER_VERSION,
        JOINT_SPEAKER_PARSER_VERSION,
        TITLE_METADATA_PARSER_VERSION,
        EXPLICIT_INLINE_PARSER_VERSION,
        FORMAT_PARSER_VERSION,
    ]
    .contains(&parser_version);
    let (channel_text, channel_label) = if current_voice_rules {
        normalized_channel = channel_text.to_ascii_uppercase();
        normalized_label = label.to_ascii_uppercase();
        (normalized_channel.as_str(), normalized_label.as_str())
    } else {
        (channel_text.as_str(), label)
    };
    let (hint_os, hint_vo) = if current_voice_rules {
        static OS_CUE: OnceLock<Regex> = OnceLock::new();
        static VO_CUE: OnceLock<Regex> = OnceLock::new();
        let os = OS_CUE.get_or_init(|| {
            Regex::new(r"(?i)(?:^|[^A-Za-z0-9_])(?:OS|O\.S\.)(?:$|[^A-Za-z0-9_])").unwrap()
        });
        let vo = VO_CUE.get_or_init(|| {
            Regex::new(r"(?i)(?:^|[^A-Za-z0-9_])(?:VO|V\.O\.)(?:$|[^A-Za-z0-9_])").unwrap()
        });
        (os.is_match(&hint), vo.is_match(&hint))
    } else {
        (hint.contains("OS"), hint.contains("VO"))
    };
    if hint_depth != 0 || !["", "OS", "VO", "O.S.", "V.O."].contains(&channel_text) {
        return None;
    }
    let suffix = channel_text;
    static HAN: OnceLock<Regex> = OnceLock::new();
    let attached_cue = joint_label.is_none() && (!uses_fidelity_rules(parser_version)
        || label.char_indices().rev().nth(2).is_some_and(|(_, ch)| {
            HAN.get_or_init(|| Regex::new(r"^\p{Han}$").unwrap())
                .is_match(&ch.to_string())
        }));
    let (speaker, channel) =
        if attached_cue && channel_label.ends_with("OS") && label.chars().count() > 2 {
            (&label[..label.len() - 2], "OS")
        } else if attached_cue && channel_label.ends_with("VO") && label.chars().count() > 2 {
            (&label[..label.len() - 2], "VO")
        } else if label == "旁白" {
            (label, "VO")
        } else if ["系统", "系统音"].contains(&label) {
            (label, "系统音")
        } else if suffix.contains('O') && !suffix.starts_with('V') || hint_os {
            (label, "OS")
        } else if suffix.starts_with('V') || hint_vo {
            (label, "VO")
        } else if hint.contains("画外") {
            (label, "画外")
        } else {
            (label, "现场")
        };
    let after_colon = colon + line[colon..].chars().next()?.len_utf8();
    let quote_start =
        after_colon + line[after_colon..].len() - line[after_colon..].trim_start().len();
    let quote = &line[quote_start..];
    if quote.is_empty() {
        return None;
    }
    let a = start + line[..quote_start].encode_utf16().count();
    let range = SourceRange {
        start: a,
        end: a + quote.encode_utf16().count(),
    };
    let (spoken_segments, performance_segments) =
        dialogue_segments(line, start, quote_start, &range);
    let mut dialogue = SourceDialogue {
        id: format!("dialogue:{row_id}"),
        speaker: speaker.into(),
        channel: channel.into(),
        quote: quote.into(),
        range,
        spoken_segments,
        performance_segments,
        speakers: vec![],
    };
    system_message_segments(&mut dialogue);
    Some(dialogue)
}

// Exact explicit cast declarations provide lexical
// name evidence. A conjunction inside narrative prose is not a new speaker.
struct JointSpeakerGrammar { names: HashSet<String>, grammar: Option<Regex>, member: Option<Regex> }

fn joint_speaker_grammar(source: &LockedSource) -> JointSpeakerGrammar {
    let names = declared_speaker_names(source);
    let mut ordered: Vec<_> = names.iter().filter(|name| !name.is_empty() && name.chars().count() <= 24).collect();
    ordered.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    let atom = ordered.iter().map(|name| regex::escape(name)).collect::<Vec<_>>().join("|");
    let grammar = if ordered.is_empty() || ordered.len() > 1024 { None } else {
        Regex::new(&format!(r"^((?:{})(?:\s*[&＆、与和]\s*(?:{})){{1,7}})(.*)$", atom, atom)).ok()
    };
    let member = grammar.as_ref().and_then(|_| Regex::new(&format!(r"^(?:{})", atom)).ok());
    JointSpeakerGrammar { names, grammar, member }
}

// Some(None) marks an explicit but unproved joint-label shape: do not fall
// back to the older permissive single-label rule and read narrative as speech.
fn joint_row_dialogue(row: &SourceRow, evidence: &JointSpeakerGrammar) -> Option<Option<SourceDialogue>> {
    let names = &evidence.names;
    let line = row.text.trim_end_matches(['\r', '\n']);
    let trimmed = line.trim_start();
    let mut depth = 0_i32;
    let mut colon = None;
    for (byte, ch) in trimmed.char_indices() {
        match ch {
            '（' | '(' | '[' | '【' => depth += 1,
            '）' | ')' | ']' | '】' => depth -= 1,
            ':' | '：' if depth == 0 => { colon = Some(byte); break; }
            _ => {}
        }
        if depth < 0 || byte > 256 { return None; }
    }
    let head = trimmed[..colon?].trim();
    if names.contains(head) { return None; }
    let explicit = head.contains(['&', '＆', '、']);
    let conjunction = head.contains(['与', '和']);
    if !explicit && !conjunction { return None; }
    // Only a head starting with an evidenced name and a join operator can
    // opt into the joint-speech grammar. A delta/action prefix stays narrative.
    let starts_joint = names.iter().any(|name| head.strip_prefix(name).is_some_and(|rest|
        rest.trim_start().starts_with(['&', '＆', '、', '与', '和'])));
    if !starts_joint { return None; }
    let Some(grammar) = &evidence.grammar else { return Some(None); };
    let Some(captures) = grammar.captures(head) else { return Some(None); };
    let label = captures.get(1)?.as_str();
    // Existing grammar parses the remaining whole-group channel and performance cues.
    let Some(mut dialogue) = parse_dialogue_with_label(line, row.range.start, &row.id, JOINT_SPEAKER_PARSER_VERSION, Some(label)) else { return Some(None); };
    let member = evidence.member.as_ref()?;
    let label_origin = line.len() - trimmed.len();
    let mut cursor = 0;
    let mut seen = HashSet::new();
    while cursor < label.len() {
        let Some(found) = member.find(&label[cursor..]) else { return Some(None); };
        let name = found.as_str();
        if !seen.insert(name) { return Some(None); }
        let start = label_origin + cursor;
        let end = start + name.len();
        dialogue.speakers.push(SourceSegment { text: name.into(), range: SourceRange {
            start: row.range.start + line[..start].encode_utf16().count(),
            end: row.range.start + line[..end].encode_utf16().count(),
        }});
        cursor += name.len();
        if cursor == label.len() { break; }
        let rest = &label[cursor..]; cursor += rest.len() - rest.trim_start().len();
        let Some(separator) = label[cursor..].chars().next() else { return Some(None); };
        if !['&', '＆', '、', '与', '和'].contains(&separator) { return Some(None); }
        cursor += separator.len_utf8();
        let rest = &label[cursor..]; cursor += rest.len() - rest.trim_start().len();
    }
    if dialogue.speakers.len() < 2 { return Some(None); }
    Some(Some(dialogue))
}

fn apply_joint_dialogues(source: &mut LockedSource) {
    if !uses_joint_speaker_rules(&source.parser_version) { return; }
    let names = joint_speaker_grammar(source);
    for row in source.rows.iter_mut().filter(|row| row.kind == "body") {
        if let Some(dialogue) = joint_row_dialogue(row, &names) { row.dialogue = dialogue; }
    }
}

pub fn prepare_source(text: &str) -> Result<LockedSource, String> {
    let source = parse_source(text, fresh_parser_version(text))?;
    validate_integrity(&source)?;
    Ok(source)
}

fn parse_source(text: &str, parser_version: &str) -> Result<LockedSource, String> {
    if text.is_empty() {
        return Err("原文不能为空".into());
    }
    if ![
        LEGACY_PARSER_VERSION,
        CONTEXT_PARSER_VERSION,
        CURRENT_PARSER_VERSION,
        STRUCTURAL_PARSER_VERSION,
        DISPLAY_PARSER_VERSION,
        FIDELITY_PARSER_VERSION,
        MULTILINE_DISPLAY_PARSER_VERSION,
        JOINT_SPEAKER_PARSER_VERSION,
        TITLE_METADATA_PARSER_VERSION,
        EXPLICIT_INLINE_PARSER_VERSION,
        FORMAT_PARSER_VERSION,
    ]
    .contains(&parser_version)
    {
        return Err("原文解析规则版本不受支持，请升级客户端".into());
    }
    let structural = [
        STRUCTURAL_PARSER_VERSION,
        DISPLAY_PARSER_VERSION,
        FIDELITY_PARSER_VERSION,
        MULTILINE_DISPLAY_PARSER_VERSION,
        JOINT_SPEAKER_PARSER_VERSION,
        TITLE_METADATA_PARSER_VERSION,
        EXPLICIT_INLINE_PARSER_VERSION,
        FORMAT_PARSER_VERSION,
    ]
    .contains(&parser_version);
    let fidelity = uses_fidelity_rules(parser_version);
    let sha256 = digest(text);
    let episode_re = episode_heading_re();
    let context_re = Regex::new(r"^(?:#{1,6}\s*)?(?:【)?(?:作者资料|故事大纲|人物小传|人物设定|世界观设定|背景资料|附录)(?:】)?(?:\s|[:：｜|]|$)").unwrap();
    let is_context = |line: &str, inline_allowed: bool| {
        if parser_version == LEGACY_PARSER_VERSION {
            context_re.is_match(line)
        } else {
            context_heading(line, inline_allowed)
        }
    };
    let scene_re = Regex::new(r"^(?:#{1,6}\s*)?(?:第\s*([零〇一二三四五六七八九十百千两\d]+)\s*场(?:\s|[:：｜|]|$)|场景(?:\s*[:：]\s*|\s+)(\S.*))").unwrap();
    let pair_re = Regex::new(r"^(?:#{1,6}\s*)?(?:场\s*)?(\d+)\s*[-－.．]\s*(\d+)\s+(.+)$").unwrap();
    let bare_pair_re =
        Regex::new(r"^(?:#{1,6}\s*)?(?:场\s*)?(\d+)\s*[-－]\s*(\d+)\s*[:：]?$").unwrap();
    let combined_re = Regex::new(r"^(.*?)\s*场\s*(\d+)\s*[-－.．]\s*(\d+)\s+(.+)$").unwrap();
    let metadata_re = Regex::new(r"(?:^|[\s/／|｜])(?:日|夜|昼|晨|黄昏|清晨|凌晨|傍晚|白天|夜晚|内|外|内景|外景|INT\.?|EXT\.?)(?:$|[\s/／|｜])").unwrap();
    let time_re = Regex::new(
        r"(?:^|[\s/／|｜])(?:日|夜|昼|晨|黄昏|清晨|凌晨|傍晚|白天|夜晚)(?:$|[\s/／|｜])",
    )
    .unwrap();
    let location_re =
        Regex::new(r"(?:^|[\s/／|｜])(?:内|外|内景|外景|INT\.?|EXT\.?)(?:$|[\s/／|｜])").unwrap();
    let metadata_re = if fidelity {
        Regex::new(r"(?:^|[\s/／|｜,，、])(?:日|夜|昼|晨|黄昏|清晨|凌晨|傍晚|白天|夜晚|内|外|内景|外景|INT\.?|EXT\.?)(?:$|[\s/／|｜,，、])").unwrap()
    } else {
        metadata_re
    };
    let time_re = if fidelity {
        Regex::new(r"(?:^|[\s/／|｜,，、])(?:日|夜|昼|晨|黄昏|清晨|凌晨|傍晚|白天|夜晚)(?:$|[\s/／|｜,，、])").unwrap()
    } else {
        time_re
    };
    let location_re = if fidelity {
        Regex::new(r"(?:^|[\s/／|｜,，、])(?:内|外|内景|外景|INT\.?|EXT\.?)(?:$|[\s/／|｜,，、])")
            .unwrap()
    } else {
        location_re
    };
    let mut source = LockedSource {
        schema_version: "locked-source/v1".into(),
        parser_version: parser_version.into(),
        text: text.into(),
        sha256: sha256.clone(),
        coordinate_system: "utf16".into(),
        length: text.encode_utf16().count(),
        rows: Vec::new(),
        episodes: Vec::new(),
        scenes: Vec::new(),
        diagnostics: Vec::new(),
        boundary_decisions: Vec::new(),
        scene_edits: Vec::new(),
    };
    let (mut offset, mut episode_id, mut scene_id, mut context, mut source_number) =
        (0, None, None, false, None);
    let mut seen = HashSet::new();
    let mut continued_metadata = HashSet::new();
    let mut previous_heading_number = None;
    let lines = raw_lines(text);
    let title_metadata = if [TITLE_METADATA_PARSER_VERSION, EXPLICIT_INLINE_PARSER_VERSION, FORMAT_PARSER_VERSION].contains(&parser_version) {
        title_metadata_row_indices(text)
    } else { HashSet::new() };
    let biography_metadata = if parser_version == FORMAT_PARSER_VERSION { biography_preface_rows(text) } else { HashSet::new() };
    let first_nonempty = lines.iter().position(|line| !line.trim().is_empty());
    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if !line.is_empty() && !episode_re.is_match(line) {
            previous_heading_number = None;
        }
        let end = offset + raw.encode_utf16().count();
        let id = format!("row:{}:{offset}:{end}", &sha256[..12]);
        let mut kind = if line.is_empty() {
            "format"
        } else if context {
            "context"
        } else {
            "body"
        };
        let title_before_context = Some(index) == first_nonempty
            && line.chars().count() <= 60
            && !line.contains(['。', '！', '？', '!', '?', '△', '▲', '：', ':', '“', '”'])
            && lines
                .iter()
                .skip(index + 1)
                .find(|s| !s.trim().is_empty())
                .map(|next| {
                    is_context(next.trim(), true)
                        || structural
                            && episode_re
                                .captures(next.trim())
                                .and_then(|c| number_value(c.get(1)?.as_str()))
                                == Some(0)
                })
                .unwrap_or(false);
        if biography_metadata.contains(&index) {
            kind = "context";
        } else if let Some(c) = episode_re.captures(line) {
            let number = number_value(c.get(1).unwrap().as_str()).ok_or("无法识别原集号")?;
            let title = c.get(2).map(|m| m.as_str()).unwrap_or("");
            let heading_has_scene = combined_re.captures(title).is_some_and(|c| {
                c.get(1).unwrap().as_str().chars().count() <= 80
                    && !c
                        .get(1)
                        .unwrap()
                        .as_str()
                        .contains(['。', '！', '？', '△', '▲', ':', '：'])
                    && valid_numbered_scene(c.get(3).unwrap().as_str(), parser_version)
                    && metadata_re.is_match(c.get(4).unwrap().as_str())
            });
            let supplemental_heading = structural
                && previous_heading_number == Some(number)
                && episode_id.is_some()
                && scene_id.is_none()
                && !heading_has_scene;
            let next_is_context = lines
                .iter()
                .skip(index + 1)
                .find(|s| !s.trim().is_empty())
                .map(|s| is_context(s.trim(), true))
                .unwrap_or(false);
            context = is_context(title.trim(), true)
                || number == 0 && title.trim().is_empty() && next_is_context
                // The original production framework supplies all episode-0
                // material to the full-script foundation, never to a single
                // production episode. Retain even its scene/dialogue text here.
                || structural && number == 0;
            scene_id = None;
            source_number = Some(number);
            if context {
                episode_id = None;
                kind = "context";
            } else if supplemental_heading {
                // Two adjacent labels describe one boundary; both original rows
                // belong to that episode without rewriting either label.
                kind = "format";
            } else {
                let eid = format!("episode:{}:{offset}", &sha256[..12]);
                if !seen.insert(number) {
                    source.diagnostics.push(SourceDiagnostic {
                        code: "DUPLICATE_EPISODE_NUMBER".into(),
                        row_ids: vec![id.clone()],
                        message: "原集号重复，保留两处来源，需明确制作范围".into(),
                        blocking: true,
                    });
                }
                if number == 0 && title.trim().is_empty() {
                    source.diagnostics.push(SourceDiagnostic {
                        code: "ZERO_EPISODE_UNCONFIRMED".into(),
                        row_ids: vec![id.clone()],
                        message: "第0集未声明资料或序章剧情，需确认".into(),
                        blocking: true,
                    });
                }
                source.episodes.push(SourceEpisode {
                    id: eid.clone(),
                    number: source.episodes.len() as u32 + 1,
                    source_number: c.get(1).unwrap().as_str().into(),
                    title: line.into(),
                    row_ids: vec![],
                    scene_ids: vec![],
                });
                episode_id = Some(eid);
                kind = "format";
                if let Some(combined) = combined_re.captures(title).filter(|c| {
                    c.get(1).unwrap().as_str().chars().count() <= 80
                        && !c
                            .get(1)
                            .unwrap()
                            .as_str()
                            .contains(['。', '！', '？', '△', '▲', ':', '：'])
                        && valid_numbered_scene(c.get(3).unwrap().as_str(), parser_version)
                        && metadata_re.is_match(c.get(4).unwrap().as_str())
                }) {
                    if combined.get(2).unwrap().as_str().parse::<u32>().ok() == Some(number) {
                        let sid = format!("scene:{}:{offset}", &sha256[..12]);
                        let eid = episode_id.as_ref().unwrap();
                        source.scenes.push(SourceScene {
                            id: sid.clone(),
                            episode_id: eid.clone(),
                            title: combined.get(0).unwrap().as_str().into(),
                            row_ids: vec![],
                            metadata: Default::default(),
                        });
                        source
                            .episodes
                            .last_mut()
                            .unwrap()
                            .scene_ids
                            .push(sid.clone());
                        scene_id = Some(sid);
                    } else {
                        source.diagnostics.push(SourceDiagnostic {
                            code: "SCENE_EPISODE_MISMATCH".into(),
                            row_ids: vec![id.clone()],
                            message: "同行场号与集号不一致；原行完整保留".into(),
                            blocking: true,
                        });
                    }
                }
            }
            previous_heading_number = (!context && !heading_has_scene).then_some(number);
        } else if is_context(line, episode_id.is_none() || context) {
            context = true;
            episode_id = None;
            scene_id = None;
            kind = "context";
        } else if ["序章", "序幕"].iter().any(|p| {
            line == *p || line.starts_with(&format!("{p}：")) || line.starts_with(&format!("{p} "))
        }) {
            let eid = format!("episode:{}:{offset}", &sha256[..12]);
            source.episodes.push(SourceEpisode {
                id: eid.clone(),
                number: source.episodes.len() as u32 + 1,
                source_number: "序章".into(),
                title: line.into(),
                row_ids: vec![],
                scene_ids: vec![],
            });
            context = false;
            episode_id = Some(eid);
            scene_id = None;
            kind = "format";
        } else if !context {
            let scene = scene_re.captures(line);
            let explicit_location = scene.as_ref().and_then(|c| c.get(2)).map(|m| m.as_str());
            let valid_location = explicit_location
                .map(|s| {
                    s.chars().count() <= 160
                        && !s.contains(['：', ':', '△', '▲', '。', '！', '？', '“', '”'])
                })
                .unwrap_or(true);
            let pair = if fidelity {
                fidelity_pair_re().captures(line)
            } else {
                pair_re.captures(line).or_else(|| {
                    structural
                        .then(|| tight_scene_heading_re().captures(line))
                        .flatten()
                })
            };
            let bare = bare_pair_re.captures(line);
            let following = if bare.is_some() {
                lines
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, s)| !s.trim().is_empty())
            } else {
                None
            };
            let separate_metadata = bare
                .as_ref()
                .map(|c| valid_numbered_scene(c.get(2).unwrap().as_str(), parser_version))
                .unwrap_or(false)
                && following
                    .map(|(_, s)| {
                        s.trim().chars().count() <= 160
                            && !s.contains(['：', ':', '△', '▲', '。', '！', '？', '“', '”'])
                            && time_re.is_match(s.trim())
                            && location_re.is_match(s.trim())
                            && !episode_re.is_match(s.trim())
                    })
                    .unwrap_or(false);
            let separate_matches = separate_metadata
                && bare
                    .as_ref()
                    .unwrap()
                    .get(1)
                    .unwrap()
                    .as_str()
                    .parse::<u32>()
                    .ok()
                    == source_number;
            let has_metadata = pair
                .as_ref()
                .map(|c| {
                    valid_numbered_scene(c.get(2).unwrap().as_str(), parser_version)
                        && metadata_re.is_match(c.get(3).unwrap().as_str())
                        && (!fidelity || fidelity_pair_valid(c))
                })
                .unwrap_or(false);
            let mismatch = pair
                .as_ref()
                .filter(|_| has_metadata)
                .map(|c| c.get(1).unwrap().as_str().parse::<u32>().ok() != source_number)
                .unwrap_or(false)
                || separate_metadata && !separate_matches;
            if mismatch {
                source.diagnostics.push(SourceDiagnostic {
                    code: "SCENE_EPISODE_MISMATCH".into(),
                    row_ids: vec![id.clone()],
                    message: "场号所属集与当前集不同，未自动归属".into(),
                    blocking: true,
                });
                scene_id = None;
            } else if scene.is_some() && valid_location || has_metadata || separate_matches {
                if let Some(eid) = &episode_id {
                    let sid = format!("scene:{}:{offset}", &sha256[..12]);
                    source.scenes.push(SourceScene {
                        id: sid.clone(),
                        episode_id: eid.clone(),
                        title: line.into(),
                        row_ids: vec![],
                        metadata: Default::default(),
                    });
                    source
                        .episodes
                        .iter_mut()
                        .find(|e| &e.id == eid)
                        .unwrap()
                        .scene_ids
                        .push(sid.clone());
                    scene_id = Some(sid);
                    if separate_matches {
                        continued_metadata.insert(following.unwrap().0);
                    }
                    // Inline source action remains body; otherwise scene metadata is format only.
                    kind = if fidelity && inline_scene_dialogue(raw, offset, &id).is_some()
                        || line.contains(['△', '▲'])
                        || parse_dialogue(
                            line.split_once(['△', '▲']).map(|(_, t)| t).unwrap_or(""),
                            0,
                            "",
                            parser_version,
                        )
                        .is_some()
                    {
                        "body"
                    } else {
                        "format"
                    };
                }
            }
            if fidelity
                && has_metadata
                && line.contains([':', '：'])
                && inline_scene_dialogue(raw, offset, &id).is_none()
            {
                kind = "body";
                source.diagnostics.push(SourceDiagnostic {
                    code: "SCENE_PAYLOAD_UNCONFIRMED".into(),
                    row_ids: vec![id.clone()],
                    message: "同行场景与正文边界无法唯一确定，请按原坐标确认；原行完整保留".into(),
                    blocking: true,
                });
            }
        }
        if continued_metadata.contains(&index) && !context {
            kind = "format";
        }
        if title_before_context || title_metadata.contains(&index) {
            kind = "context";
        }
        if kind == "body" && (episode_id.is_none() || scene_id.is_none()) {
            let code = if episode_id.is_some() {
                "SCENE_BOUNDARY_UNCONFIRMED"
            } else {
                "BOUNDARY_UNCONFIRMED"
            };
            if let Some(last) = source.diagnostics.last_mut().filter(|d| d.code == code) {
                last.row_ids.push(id.clone());
            } else {
                source.diagnostics.push(SourceDiagnostic {
                    code: code.into(),
                    row_ids: vec![id.clone()],
                    message: "原文已完整保留；请对待定正文给出明确集场范围".into(),
                    blocking: true,
                });
            }
        }
        let dialogue = if kind == "body" {
            row_dialogue(raw, offset, &id, parser_version)
        } else {
            None
        };
        if let Some(eid) = &episode_id {
            if let Some(e) = source.episodes.iter_mut().find(|e| &e.id == eid) {
                e.row_ids.push(id.clone());
            }
        }
        if let Some(sid) = &scene_id {
            if let Some(s) = source.scenes.iter_mut().find(|s| &s.id == sid) {
                s.row_ids.push(id.clone());
            }
        }
        source.rows.push(SourceRow {
            id,
            number: index + 1,
            range: SourceRange { start: offset, end },
            text: (*raw).into(),
            kind: kind.into(),
            episode_id: episode_id.clone(),
            scene_id: scene_id.clone(),
            dialogue,
        });
        offset = end;
    }
    apply_joint_dialogues(&mut source);
    if uses_explicit_inline_rules(parser_version) {
        explicit_inline::apply(&mut source);
    }
    let display_ids = display_only_row_ids(&source);
    for row in &mut source.rows {
        if display_ids.contains(&row.id) {
            row.dialogue = None;
        }
    }
    let mut speaker_names: HashSet<_> = source
        .rows
        .iter()
        .filter_map(|r| r.dialogue.as_ref().map(|q| q.speaker.clone()))
        .collect();
    if uses_explicit_inline_rules(parser_version) { speaker_names.extend(declared_speaker_names(&source)); }
    let inline_re = Regex::new(
        r"(?:^|[\s；;])([\p{L}][\p{L}\p{N}·・_]{0,23})(?:\s*[（(][^\r\n:：()（）]*[）)])?\s*[:：]",
    )
    .unwrap();
    for row in &source.rows {
        if let Some(q) = &row.dialogue {
            if inline_re
                .captures_iter(&q.quote)
                .any(|c| ((!uses_explicit_inline_rules(parser_version) && fidelity) || speaker_names.contains(c.get(1).unwrap().as_str()))
                    && (!uses_explicit_inline_rules(parser_version) || explicit_inline::outside_enclosure(&q.quote,c.get(1).unwrap().start())))
            {
                source.diagnostics.push(SourceDiagnostic{code:"DIALOGUE_MULTI_SPEAKER_UNCONFIRMED".into(),row_ids:vec![row.id.clone()],message:"同一原行含多个明确说者，保留原行；需按原坐标明确对白分段，不能把后段读给首位说者".into(),blocking:true});
            }
        }
    }
    Ok(source)
}

pub(super) fn source_before_scene_edits(source: &LockedSource) -> Result<LockedSource, String> {
    let canonical = parse_source(&source.text, &source.parser_version)?;
    if source.boundary_decisions.is_empty() {
        Ok(canonical)
    } else {
        apply_decisions_inner(&canonical, &source.boundary_decisions)
    }
}

pub fn validate_source(source: &LockedSource) -> Result<(), String> {
    // Source validation is deterministic for the running parser binary. Cache
    // only successful checks and bind ALL serialized fields, not merely the
    // text SHA: altered rows, classifications, decisions and scene edits must
    // still be rejected. No source prose or trust survives process restart.
    static VERIFIED: OnceLock<std::sync::Mutex<std::collections::VecDeque<[u8; 32]>>> = OnceLock::new();
    let encoded = serde_json::to_vec(source).map_err(|e| e.to_string())?;
    let key: [u8; 32] = Sha256::digest(&encoded).into();
    let memo = VERIFIED.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::new()));
    if memo.lock().is_ok_and(|entries| entries.contains(&key)) { return Ok(()); }
    validate_source_uncached(source)?;
    if let Ok(mut entries) = memo.lock() {
        if !entries.contains(&key) {
            if entries.len() == 4 { entries.pop_front(); }
            entries.push_back(key);
        }
    }
    Ok(())
}

fn validate_source_uncached(source: &LockedSource) -> Result<(), String> {
    validate_integrity(source)?;
    let canonical = parse_source(&source.text, &source.parser_version)?;
    let expected = if source.boundary_decisions.is_empty() {
        canonical
    } else {
        apply_decisions_inner(&canonical, &source.boundary_decisions)?
    };
    let expected = super::scene_edits::replay_scene_edits(expected, &source.scene_edits)?;
    let mut rows = source.rows.clone();
    for (row, expected) in rows.iter_mut().zip(&expected.rows) {
        if let (Some(actual), Some(expected)) = (&mut row.dialogue, &expected.dialogue) {
            if actual.spoken_segments.is_empty() && actual.performance_segments.is_empty() {
                actual.spoken_segments = expected.spoken_segments.clone();
                actual.performance_segments = expected.performance_segments.clone();
            }
            // Integrity already proved any legacy segments are the exact old
            // parser projection. Normalize only the known system wrapper here.
            system_message_segments(actual);
        }
    }
    if serde_json::to_value(&rows).unwrap() != serde_json::to_value(&expected.rows).unwrap()
        || serde_json::to_value(&source.episodes).unwrap()
            != serde_json::to_value(&expected.episodes).unwrap()
        || serde_json::to_value(&source.scenes).unwrap()
            != serde_json::to_value(&expected.scenes).unwrap()
        || serde_json::to_value(&source.diagnostics).unwrap()
            != serde_json::to_value(&expected.diagnostics).unwrap()
    {
        return Err("来源分类、说者、集场或疑点不匹配程序解析与已保存显式决定".into());
    }
    Ok(())
}

/// Old saved sources can omit the new segments. Validate their original contract
/// first, then materialize every new field from the same locked text/decisions.
pub fn upgrade_source_dialogues(source: &LockedSource) -> Result<LockedSource, String> {
    validate_source(source)?;
    let canonical = parse_source(&source.text, &source.parser_version)?;
    let out = if source.boundary_decisions.is_empty() {
        Ok(canonical)
    } else {
        apply_decisions_inner(&canonical, &source.boundary_decisions)
    }?;
    super::scene_edits::replay_scene_edits(out, &source.scene_edits)
}

/// Recognition changes only deterministic classifications. Raw text, source
/// coordinates, and all explicit user decisions retain their original identity.
pub fn reparse_source(source: &LockedSource) -> Result<LockedSource, String> {
    validate_source(source)?;
    let canonical = parse_source(&source.text, fresh_parser_version(&source.text))?;
    let out = if source.boundary_decisions.is_empty() {
        canonical
    } else {
        apply_decisions_inner(&canonical, &source.boundary_decisions)?
    };
    prove_boundary_reparse(source, &out)?;
    let out = migrate_scene_edits(source, out)?;
    validate_source(&out)?;
    Ok(out)
}

// Episode headings may coalesce on recognition. Only those format rows may
// differ: every substantive row must still belong to the same unique episode.
fn episode_evidence(source: &LockedSource, episode_id: &str) -> Vec<String> {
    source
        .rows
        .iter()
        .filter(|row| {
            row.episode_id.as_deref() == Some(episode_id)
                && !row.text.trim().is_empty()
                && !(row.kind == "format"
                    && row.scene_id.is_none()
                    && episode_heading_re().is_match(row.text.trim()))
        })
        .map(|row| row.id.clone())
        .collect()
}

fn prove_boundary_reparse(previous: &LockedSource, next: &LockedSource) -> Result<(), String> {
    if previous.boundary_decisions.is_empty() {
        return Ok(());
    }
    let old = source_before_scene_edits(previous)?;
    for decision in &previous.boundary_decisions {
        let select = |source: &LockedSource| {
            source
                .rows
                .iter()
                .filter(|row| row.range.start >= decision.start && row.range.end <= decision.end)
                .map(|row| {
                    (
                        row.id.clone(),
                        row.kind.clone(),
                        row.episode_id.clone(),
                        row.scene_id.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let before = select(&old);
        let after = select(next);
        if before.len() != after.len()
            || before
                .iter()
                .zip(&after)
                .any(|(a, b)| a.0 != b.0 || a.1 != b.1)
        {
            return Err("重新识别无法证明原范围决定的完整行与分类，保留旧决定".into());
        }
        for (before, after) in before.iter().zip(&after) {
            match (&before.2, &after.2) {
                (Some(a), Some(b)) if episode_evidence(&old, a) == episode_evidence(next, b) => {}
                (None, None) => {}
                _ => {
                    return Err(
                        "重新识别改变了已确认范围的集成员，须重新确认，旧决定保持不变".into(),
                    )
                }
            }
            match (&before.3, &after.3) {
                (Some(a), Some(b)) => {
                    let a = old
                        .scenes
                        .iter()
                        .find(|scene| &scene.id == a)
                        .ok_or("旧范围场身份失效")?;
                    let b = next
                        .scenes
                        .iter()
                        .find(|scene| &scene.id == b)
                        .ok_or("新范围场身份失效")?;
                    if a.row_ids != b.row_ids {
                        return Err("重新识别改变了已确认范围的场成员，旧决定保持不变".into());
                    }
                }
                (None, None) => {}
                _ => return Err("重新识别改变了已确认范围的场归属，旧决定保持不变".into()),
            }
        }
    }
    Ok(())
}

fn migrate_scene_edits(
    previous: &LockedSource,
    mut next: LockedSource,
) -> Result<LockedSource, String> {
    if previous.scene_edits.is_empty() {
        return Ok(next);
    }
    let mut old = source_before_scene_edits(previous)?;
    let mut migrated = Vec::new();
    for edit in &previous.scene_edits {
        let plan = &edit["plan"];
        let sid = plan["sceneId"].as_str().ok_or("旧场次修订缺少场身份")?;
        let eid = plan["episodeId"].as_str().ok_or("旧场次修订缺少集身份")?;
        let scene = old
            .scenes
            .iter()
            .find(|scene| scene.id == sid && scene.episode_id == eid)
            .ok_or("旧场次修订身份无效")?;
        let evidence = episode_evidence(&old, eid);
        let candidates: Vec<_> = next
            .scenes
            .iter()
            .filter(|candidate| {
                candidate.row_ids == scene.row_ids
                    && candidate.metadata == scene.metadata
                    && !evidence.is_empty()
                    && episode_evidence(&next, &candidate.episode_id) == evidence
            })
            .collect();
        if candidates.len() != 1 {
            return Err(
                "重新识别后无法唯一证明旧场次修订的原范围及集场成员，已保留旧原文与编辑".into(),
            );
        }
        let mut mapped = edit.clone();
        mapped["plan"]["sceneId"] = candidates[0].id.clone().into();
        mapped["plan"]["episodeId"] = candidates[0].episode_id.clone().into();
        // Replay both histories one step at a time. Later metadata can refer to
        // a split-derived ID only after the same exact cut was proven on both.
        old = super::scene_edits::replay_scene_edits(old, std::slice::from_ref(edit))?;
        next = super::scene_edits::replay_scene_edits(next, std::slice::from_ref(&mapped))?;
        migrated.push(mapped);
    }
    next.scene_edits = migrated;
    Ok(next)
}

fn validate_integrity(source: &LockedSource) -> Result<(), String> {
    let display_ids = display_only_row_ids(source);
    let joint_names = uses_joint_speaker_rules(&source.parser_version).then(|| joint_speaker_grammar(source));
    let inline_names = (uses_explicit_inline_rules(&source.parser_version)).then(|| explicit_inline::speaker_names(source));
    if source.schema_version != "locked-source/v1"
        || source.coordinate_system != "utf16"
        || source.sha256 != digest(&source.text)
        || source.length != source.text.encode_utf16().count()
    {
        return Err("原文或来源元数据已变更".into());
    }
    let mut cursor = 0;
    let mut ids = HashSet::new();
    // Check UTF-16 lengths while reconstructing once. Re-slicing the complete
    // script for every row scans every preceding character again (quadratic
    // for long scripts); the final byte equality proves the same row contents.
    let mut reconstructed = String::new();
    for row in &source.rows {
        if row.range.start != cursor
            || row.range.end <= cursor
            || !ids.insert(&row.id)
            || row.text.encode_utf16().count() != row.range.end - row.range.start
        {
            return Err("来源行存在遗漏、重叠、错序或回读不一致".into());
        }
        let expected_dialogue = if row.kind == "body" && !display_ids.contains(&row.id) {
            inline_names.as_ref().and_then(|names| explicit_inline::marked_dialogue(row, names))
                .or_else(|| joint_names.as_ref().and_then(|names| joint_row_dialogue(row, names)).unwrap_or_else(|| row_dialogue(&row.text, row.range.start, &row.id, &source.parser_version)))
        } else {
            None
        };
        let mut actual_dialogue = row.dialogue.clone();
        if let (Some(actual), Some(expected)) = (&mut actual_dialogue, &expected_dialogue) {
            if actual.spoken_segments.is_empty() && actual.performance_segments.is_empty() {
                actual.spoken_segments = expected.spoken_segments.clone();
                actual.performance_segments = expected.performance_segments.clone();
            }
            // Previously saved parser metadata may contain the exact legacy
            // bracket classification. Upgrade that one known parser defect
            // from the locked source, never accept arbitrary changed segments.
            if actual != expected {
                let colon_end = expected.range.start - row.range.start;
                let prefix = slice_utf16(
                    &row.text,
                    &SourceRange {
                        start: 0,
                        end: colon_end,
                    },
                )?;
                let legacy =
                    dialogue_segments(&row.text, row.range.start, prefix.len(), &expected.range);
                if actual.spoken_segments == legacy.0 && actual.performance_segments == legacy.1 {
                    system_message_segments(actual);
                }
            }
        }
        if actual_dialogue != expected_dialogue {
            return Err("对白说者、声道、原片段或来源范围已变更".into());
        }
        if let Some(q) = &row.dialogue {
            if q.range.start < row.range.start
                || q.range.end < q.range.start
                || q.range.end > row.range.end
                || slice_utf16(&row.text, &SourceRange {
                    start: q.range.start - row.range.start,
                    end: q.range.end - row.range.start,
                })? != q.quote
            {
                return Err("对白引用与原文不一致".into());
            }
        }
        if !["body", "context", "format"].contains(&row.kind.as_str()) {
            return Err("未知原文分类".into());
        }
        if row.kind == "body"
            && row.episode_id.is_some()
            && row.scene_id.is_some()
            && (!source
                .episodes
                .iter()
                .any(|e| Some(&e.id) == row.episode_id.as_ref() && e.row_ids.contains(&row.id))
                || !source.scenes.iter().any(|s| {
                    Some(&s.id) == row.scene_id.as_ref()
                        && Some(&s.episode_id) == row.episode_id.as_ref()
                        && s.row_ids.contains(&row.id)
                }))
        {
            return Err("原文集场身份或成员关联已失效".into());
        }
        reconstructed.push_str(&row.text);
        cursor = row.range.end;
    }
    if cursor != source.length || reconstructed != source.text {
        return Err("来源未完整覆盖到结尾".into());
    }
    Ok(())
}

pub fn apply_boundary_decisions(
    source: &LockedSource,
    decisions: &[BoundaryDecision],
) -> Result<LockedSource, String> {
    validate_source(source)?;
    let mut all = source.boundary_decisions.clone();
    for decision in decisions {
        if decision.kind == "independent_episode" {
            let proof = decision
                .confirmation
                .as_ref()
                .ok_or("独立同号集确认缺少当前原文证明")?;
            if proof.parser_version != source.parser_version || proof.source_sha256 != source.sha256
            {
                return Err("独立同号集确认已过期，请刷新当前原文".into());
            }
            if proof.source_version <= 0
                || proof.diagnostic_code != "DUPLICATE_EPISODE_NUMBER"
                || decision.reason.trim().is_empty()
                || decision.scene_title.is_some()
            {
                return Err("独立同号集确认来源证明无效".into());
            }
            // A repeated current-version acknowledgement keeps the first proof,
            // rather than manufacturing another source revision or new identity.
            if all.iter().any(|old| {
                old.kind == decision.kind
                    && old.start == decision.start
                    && old.end == decision.end
                    && old.episode_number == decision.episode_number
                    && old.confirmation.as_ref().is_some_and(|old_proof| {
                        old_proof.source_sha256 == proof.source_sha256
                            && old_proof.heading_row_id == proof.heading_row_id
                            && old_proof.diagnostic_code == proof.diagnostic_code
                    })
            }) {
                continue;
            }
        }
        if decision.kind != "independent_episode"
            && source.diagnostics.iter().any(|issue| {
                issue.code == "DUPLICATE_EPISODE_NUMBER"
                    && issue.row_ids.iter().any(|id| {
                        source.rows.iter().any(|row| {
                            &row.id == id
                                && row.range.start < decision.end
                                && decision.start < row.range.end
                        })
                    })
            })
        {
            return Err(
                "同号集须使用保留为独立制作段的明确确认，不能通过正文或资料决定清除".into(),
            );
        }
        // Replacing a previous whole decision is explicit and stored as a new source version by Store.
        all.retain(|old| !(old.start == decision.start && old.end == decision.end));
        let mut decision = decision.clone();
        decision.decision_version = Some("boundary-decision/v2".into());
        all.push(decision);
    }
    let mut out =
        apply_decisions_inner(&parse_source(&source.text, &source.parser_version)?, &all)?;
    out.boundary_decisions = all;
    let out = super::scene_edits::replay_scene_edits(out, &source.scene_edits)?;
    validate_source(&out)?;
    Ok(out)
}

fn apply_decisions_inner(
    source: &LockedSource,
    decisions: &[BoundaryDecision],
) -> Result<LockedSource, String> {
    let boundaries: HashSet<_> = source
        .rows
        .iter()
        .flat_map(|r| [r.range.start, r.range.end])
        .collect();
    for (i, d) in decisions.iter().enumerate() {
        if !boundaries.contains(&d.start)
            || !boundaries.contains(&d.end)
            || d.start >= d.end
            || d.reason.trim().is_empty()
            || !["body", "context", "independent_episode"].contains(&d.kind.as_str())
            || d.decision_version
                .as_deref()
                .is_some_and(|v| v != "boundary-decision/v2")
            || d.kind != "independent_episode" && d.confirmation.is_some()
            || decisions[..i]
                .iter()
                .any(|p| p.start < d.end && d.start < p.end)
        {
            return Err("范围决定必须完整行、无重叠且有明确原因".into());
        }
        if d.kind == "body"
            && (d.episode_number.is_none()
                || d.scene_title
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true))
        {
            return Err("正文决定必须指定集号和场景".into());
        }
    }
    let mut out = source.clone();
    for d in decisions {
        if d.kind == "independent_episode" {
            let proof = d
                .confirmation
                .as_ref()
                .ok_or("独立同号集确认缺少精确标题证明")?;
            if proof.source_version <= 0
                || proof.source_sha256 != source.sha256
                || proof.diagnostic_code != "DUPLICATE_EPISODE_NUMBER"
                || d.decision_version.as_deref() != Some("boundary-decision/v2")
                || d.scene_title.is_some()
            {
                return Err("独立同号集确认来源证明无效".into());
            }
            let original = parse_source(&source.text, &proof.parser_version)?;
            let heading = original
                .rows
                .iter()
                .find(|row| {
                    row.id == proof.heading_row_id
                        && row.range.start == d.start
                        && row.range.end == d.end
                })
                .ok_or("独立同号集确认必须精确绑定一个原标题行")?;
            let number = episode_heading_re()
                .captures(heading.text.trim())
                .and_then(|c| number_value(c.get(1)?.as_str()))
                .ok_or("独立同号集确认必须指向集标题")?;
            if d.episode_number != Some(number)
                || !original.diagnostics.iter().any(|issue| {
                    issue.blocking
                        && issue.code == proof.diagnostic_code
                        && issue.row_ids == [heading.id.clone()]
                })
                || !out.diagnostics.iter().any(|issue| {
                    issue.blocking
                        && issue.code == proof.diagnostic_code
                        && issue.row_ids == [heading.id.clone()]
                })
                || !out
                    .rows
                    .iter()
                    .any(|row| row.id == heading.id && row.episode_id.is_some())
            {
                return Err("独立同号集确认不对应当前重复标题疑点".into());
            }
            out.diagnostics.retain(|issue| {
                !(issue.code == proof.diagnostic_code && issue.row_ids == [heading.id.clone()])
            });
            continue;
        }
        let selected: Vec<_> = out
            .rows
            .iter()
            .filter(|r| r.range.start >= d.start && r.range.end <= d.end)
            .map(|r| r.id.clone())
            .collect();
        if d.decision_version.as_deref() == Some("boundary-decision/v2")
            && out.diagnostics.iter().any(|issue| {
                issue.code == "DUPLICATE_EPISODE_NUMBER"
                    && issue.row_ids.iter().any(|id| selected.contains(id))
            })
        {
            return Err("同号集需独立确认，范围决定不能清除此疑点".into());
        }
        let eid = if d.kind == "body" {
            let candidates: Vec<_> = out
                .episodes
                .iter()
                .filter(|e| {
                    number_value(&e.source_number) == d.episode_number
                        && out.rows.iter().any(|r| {
                            selected.contains(&r.id) && r.episode_id.as_ref() == Some(&e.id)
                        })
                })
                .collect();
            if d.decision_version.as_deref() == Some("boundary-decision/v2") && candidates.len() > 1
            {
                return Err("范围跨越多个制作段，不能仅凭同号集自动合并".into());
            }
            Some(if candidates.len() == 1 {
                candidates[0].id.clone()
            } else {
                format!(
                    "episode:{}:decision:{}",
                    &out.sha256[..12],
                    d.episode_number.unwrap()
                )
            })
        } else {
            None
        };
        let sid = eid
            .as_ref()
            .map(|_| format!("scene:{}:decision:{}:{}", &out.sha256[..12], d.start, d.end));
        for row in out.rows.iter_mut().filter(|r| selected.contains(&r.id)) {
            row.kind = d.kind.clone();
            row.episode_id = eid.clone();
            row.scene_id = sid.clone();
            row.dialogue = if d.kind == "body" {
                row_dialogue(&row.text, row.range.start, &row.id, &out.parser_version)
            } else {
                None
            };
        }
        out.diagnostics.retain_mut(|issue| {
            if [
                "BOUNDARY_UNCONFIRMED",
                "SCENE_BOUNDARY_UNCONFIRMED",
                "ZERO_EPISODE_UNCONFIRMED",
                "SCENE_EPISODE_MISMATCH",
                "DUPLICATE_EPISODE_NUMBER",
            ]
            .contains(&issue.code.as_str())
            {
                issue.row_ids.retain(|id| !selected.contains(id));
            }
            !issue.row_ids.is_empty()
        });
        if let (Some(eid), Some(sid)) = (eid, sid) {
            if !out.episodes.iter().any(|e| e.id == eid) {
                out.episodes.push(SourceEpisode {
                    id: eid.clone(),
                    number: d.episode_number.unwrap(),
                    source_number: d.episode_number.unwrap().to_string(),
                    title: format!("第{}集", d.episode_number.unwrap()),
                    row_ids: vec![],
                    scene_ids: vec![],
                });
            }
            out.scenes.push(SourceScene {
                id: sid,
                episode_id: eid,
                title: d.scene_title.clone().unwrap(),
                row_ids: vec![],
                metadata: Default::default(),
            });
        }
    }
    // v11: an explicit decision can move every actual row of an automatic
    // scene while leaving its trailing blank separator outside the decision.
    // That separator is still source evidence, but cannot preserve a phantom
    // scene. Do not expand the user's range or touch scenes retaining a title
    // or body; historical parsers retain their original replay byte contract.
    if out.parser_version == FORMAT_PARSER_VERSION && !decisions.is_empty() {
        let substantive_scenes: HashSet<_> = out.rows.iter()
            .filter(|row| !row.text.trim().is_empty())
            .filter_map(|row| row.scene_id.as_deref()).collect();
        let displaced_scenes: HashSet<String> = source.rows.iter()
            .filter(|row| !row.text.trim().is_empty()
                && decisions.iter().any(|decision| decision.kind != "independent_episode"
                    && row.range.start >= decision.start && row.range.end <= decision.end))
            .filter_map(|row| row.scene_id.as_ref())
            .filter(|scene_id| !substantive_scenes.contains(scene_id.as_str()))
            .cloned().collect();
        for row in &mut out.rows {
            if row.text.trim().is_empty()
                && row.scene_id.as_ref().is_some_and(|id| displaced_scenes.contains(id))
            {
                row.scene_id = None;
            }
        }
    }
    // Rebuild memberships from current row assignments; never retain previous stale members.
    let row_groups: BTreeMap<String, Vec<String>> = out
        .rows
        .iter()
        .filter_map(|r| r.episode_id.as_ref().map(|id| (id.clone(), r.id.clone())))
        .fold(BTreeMap::new(), |mut m, (id, row)| {
            m.entry(id).or_default().push(row);
            m
        });
    for e in &mut out.episodes {
        e.row_ids = row_groups.get(&e.id).cloned().unwrap_or_default();
        e.scene_ids = out
            .scenes
            .iter()
            .filter(|s| {
                s.episode_id == e.id && out.rows.iter().any(|r| r.scene_id.as_ref() == Some(&s.id))
            })
            .map(|s| s.id.clone())
            .collect();
    }
    for s in &mut out.scenes {
        s.row_ids = out
            .rows
            .iter()
            .filter(|r| r.scene_id.as_ref() == Some(&s.id))
            .map(|r| r.id.clone())
            .collect();
    }
    out.episodes.retain(|e| !e.row_ids.is_empty());
    out.scenes.retain(|s| !s.row_ids.is_empty());
    if uses_fidelity_rules(&out.parser_version)
        || decisions
            .iter()
            .any(|d| d.decision_version.as_deref() == Some("boundary-decision/v2"))
    {
        let positions: BTreeMap<_, _> = out
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.clone(), index))
            .collect();
        out.episodes.sort_by_key(|episode| {
            episode
                .row_ids
                .first()
                .and_then(|id| positions.get(id))
                .copied()
                .unwrap_or(usize::MAX)
        });
        out.scenes.sort_by_key(|scene| {
            scene
                .row_ids
                .first()
                .and_then(|id| positions.get(id))
                .copied()
                .unwrap_or(usize::MAX)
        });
        for (index, episode) in out.episodes.iter_mut().enumerate() {
            episode.number = index as u32 + 1;
            episode.scene_ids = out
                .scenes
                .iter()
                .filter(|scene| scene.episode_id == episode.id)
                .map(|scene| scene.id.clone())
                .collect();
        }
    }
    let display_ids = display_only_row_ids(&out);
    for row in &mut out.rows {
        if display_ids.contains(&row.id) {
            row.dialogue = None;
        }
    }
    out.boundary_decisions = decisions.to_vec();
    apply_joint_dialogues(&mut out);
    validate_integrity(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_display_fields_remain_picture_text_and_keep_explicit_voices() {
        let text="第一集\n场景：房间\n甲：查看今日情报。\n△面板字幕：\n情报1：原样显示😀。\n情报2：第二条消息。\n系统VO：【请查收！】\n情报3：没有字幕前置时仍按原对白规则。\n△甲抬头。\n甲：走吧。\n第二集\n场景：房间\n情报1：另一场的角色台词。\n";
        let fresh = prepare_source(text).unwrap();
        assert_eq!(fresh.parser_version, DISPLAY_PARSER_VERSION);
        for label in ["情报1：原样显示", "情报2：第二条"] {
            assert!(fresh
                .rows
                .iter()
                .find(|r| r.text.starts_with(label))
                .unwrap()
                .dialogue
                .is_none());
        }
        for label in [
            "甲：查看",
            "系统VO：",
            "情报3：",
            "甲：走吧",
            "情报1：另一场",
        ] {
            assert!(
                fresh
                    .rows
                    .iter()
                    .find(|r| r.text.starts_with(label))
                    .unwrap()
                    .dialogue
                    .is_some(),
                "{label}"
            );
        }
        validate_source(&fresh).unwrap();
        let old = parse_source(text, STRUCTURAL_PARSER_VERSION).unwrap();
        assert!(old
            .rows
            .iter()
            .find(|r| r.text.starts_with("情报1：原样显示"))
            .unwrap()
            .dialogue
            .is_some());
        assert_eq!(
            json_value(&upgrade_source_dialogues(&old).unwrap()),
            json_value(&old),
            "saved parsing is never silently reclassified"
        );
        let reparsed = reparse_source(&old).unwrap();
        assert_eq!(reparsed.sha256, old.sha256);
        assert_eq!(reparsed.text, old.text);
        assert_eq!(reparsed.length, old.length);
        assert_eq!(
            reparsed
                .rows
                .iter()
                .map(|r| (&r.id, &r.range, &r.text))
                .collect::<Vec<_>>(),
            old.rows
                .iter()
                .map(|r| (&r.id, &r.range, &r.text))
                .collect::<Vec<_>>()
        );
        let mut polluted = fresh.clone();
        polluted
            .rows
            .iter_mut()
            .find(|r| r.text.starts_with("甲：查看"))
            .unwrap()
            .dialogue = None;
        assert!(
            validate_source(&polluted).is_err(),
            "a real spoken line cannot be deleted under a display exemption"
        );
    }

    #[test]
    fn numbered_character_with_delivery_cue_is_not_silenced_by_a_display_header() {
        let source = prepare_source(
            "第一集\n场景：房间\n△屏幕文字：\n情报1（大喊）：快走！\n情报2：我是继续说话的角色。\n",
        )
        .unwrap();
        assert!(source
            .rows
            .iter()
            .find(|r| r.text.starts_with("情报1（"))
            .unwrap()
            .dialogue
            .is_some());
        assert!(source
            .rows
            .iter()
            .find(|r| r.text.starts_with("情报2："))
            .unwrap()
            .dialogue
            .is_some());
        let cast = prepare_source(
            "第一集\n场景：房间\n人物：情报1（男）、甲\n△面板字幕：\n情报1：这是登记角色的原话。\n",
        )
        .unwrap();
        assert!(cast
            .rows
            .iter()
            .find(|r| r.text.starts_with("情报1："))
            .unwrap()
            .dialogue
            .is_some());
    }

    fn json_value(source: &LockedSource) -> serde_json::Value {
        serde_json::to_value(source).unwrap()
    }

    #[test]
    fn system_message_wrapper_is_spoken_and_legacy_metadata_upgrades_without_source_changes() {
        let source=prepare_source("第一集\r\n场景：海边\r\n系统VO：【叮！今日有青蟹😀，请查看位置。】\r\n甲（低声）：别动（抬手），回来。\r\n").unwrap();
        let system = source.rows[2].dialogue.as_ref().unwrap();
        assert_eq!(system.speaker, "系统");
        assert_eq!(system.channel, "VO");
        assert_eq!(spoken_text(system), "【叮！今日有青蟹😀，请查看位置。】");
        assert!(system.performance_segments.is_empty());
        let mut legacy = source.clone();
        let row = &mut legacy.rows[2];
        let dialogue = row.dialogue.as_mut().unwrap();
        let prefix = slice_utf16(
            &row.text,
            &SourceRange {
                start: 0,
                end: dialogue.range.start - row.range.start,
            },
        )
        .unwrap();
        let segments = dialogue_segments(&row.text, row.range.start, prefix.len(), &dialogue.range);
        dialogue.spoken_segments = segments.0;
        dialogue.performance_segments = segments.1;
        assert!(dialogue.spoken_segments.is_empty());
        validate_source(&legacy).unwrap();
        let upgraded = upgrade_source_dialogues(&legacy).unwrap();
        assert_eq!(
            serde_json::to_value(&upgraded).unwrap(),
            serde_json::to_value(&source).unwrap()
        );
        assert_eq!(upgraded.sha256, legacy.sha256);
        assert_eq!(upgraded.text, legacy.text);
        assert_eq!(
            spoken_text(upgraded.rows[3].dialogue.as_ref().unwrap()),
            "别动，回来。"
        );
        legacy.rows[2]
            .dialogue
            .as_mut()
            .unwrap()
            .performance_segments[0]
            .range
            .start += 1;
        assert!(validate_source(&legacy).is_err());
        let ordinary =
            prepare_source("第一集\n甲VO：【压低声音】\n系统VO：请看（抬手）位置。\n").unwrap();
        assert!(ordinary.rows[1]
            .dialogue
            .as_ref()
            .unwrap()
            .spoken_segments
            .is_empty());
        assert_eq!(
            spoken_text(ordinary.rows[2].dialogue.as_ref().unwrap()),
            "请看位置。"
        );
    }

    #[test]
    fn source_structuring_adjacent_equal_headings_share_one_exact_boundary() {
        let text = "第 1 集\r\n　\r\n第一集\r\n1-1 日 外 门口\r\n甲os：回来😀。\r\n第 2 集\r\n第二集 重逢\r\n场景：大厅\r\n乙：到了。\r\n";
        let source = prepare_source(text).unwrap();
        assert_eq!(source.parser_version, STRUCTURAL_PARSER_VERSION);
        assert_eq!(source.episodes.len(), 2);
        assert_eq!(source.scenes.len(), 2);
        assert!(source.diagnostics.is_empty());
        assert_eq!(source.episodes[0].source_number, "1");
        assert_eq!(source.episodes[1].source_number, "2");
        assert_eq!(source.rows[0].episode_id, source.rows[2].episode_id);
        assert_eq!(source.rows[5].episode_id, source.rows[6].episode_id);
        assert_eq!(source.rows[4].dialogue.as_ref().unwrap().speaker, "甲");
        assert_eq!(source.rows[4].dialogue.as_ref().unwrap().channel, "OS");
        assert_eq!(source.text, text);
        assert_eq!(source.sha256, digest(text));
        assert_eq!(source.length, text.encode_utf16().count());
        assert_eq!(
            source
                .rows
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            text
        );
        for row in &source.rows {
            assert_eq!(slice_utf16(text, &row.range).unwrap(), row.text);
        }
        validate_source(&source).unwrap();
        assert_eq!(
            serde_json::to_vec(&source).unwrap(),
            serde_json::to_vec(&reparse_source(&source).unwrap()).unwrap()
        );
    }

    #[test]
    fn source_structuring_body_and_scene_separate_real_duplicate_numbers() {
        for text in [
            "第 1 集\n第一集\n场景：门口\n甲：走。\n第一集\n场景：大厅\n乙：来。\n",
            "第 1 集\n第一集\n场景：门口\n第一集\n场景：大厅\n",
            "第1集 重逢 场1-1 日 外 门口\n第一集\n场景：大厅\n",
        ] {
            let source = prepare_source(text).unwrap();
            assert_eq!(source.episodes.len(), 2);
            assert!(source
                .diagnostics
                .iter()
                .any(|d| d.code == "DUPLICATE_EPISODE_NUMBER" && d.blocking));
            assert_eq!(
                source
                    .rows
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>(),
                text
            );
            validate_source(&source).unwrap();
        }
    }

    #[test]
    fn source_structuring_zero_is_complete_auxiliary_not_an_extra_production_episode() {
        let text = "示例剧名\r\n第 0 集\r\n第零集\r\n资料说明。\r\n0-1 日 内 旧屋\r\n甲：第零集的原句也保留😀。\r\n▲甲抬头。\r\n第 1 集\r\n第一集\r\n1-1 日 外 门口\r\n甲：走。\r\n";
        let source = prepare_source(text).unwrap();
        assert_eq!(source.parser_version, STRUCTURAL_PARSER_VERSION);
        assert_eq!(source.episodes.len(), 1);
        assert_eq!(source.episodes[0].number, 1);
        assert_eq!(source.episodes[0].source_number, "1");
        assert_eq!(source.scenes.len(), 1);
        assert!(source.rows[..7]
            .iter()
            .all(|r| r.kind == "context" && r.episode_id.is_none() && r.scene_id.is_none()));
        assert!(source.diagnostics.is_empty());
        assert_eq!(source.text, text);
        assert_eq!(source.sha256, digest(text));
        assert_eq!(
            source
                .rows
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            text
        );
        validate_source(&source).unwrap();
    }

    #[test]
    fn source_structuring_tight_scene_metadata_is_versioned_and_rejects_numeric_prose() {
        let text = "第三集\n3-1日 外 门口\n甲：走。\n3-2夜 内 大厅\n乙：到了。\n";
        let source = prepare_source(text).unwrap();
        assert_eq!(source.parser_version, STRUCTURAL_PARSER_VERSION);
        assert_eq!(source.episodes.len(), 1);
        assert_eq!(source.scenes.len(), 2);
        assert!(source.diagnostics.is_empty());
        assert_eq!(
            source
                .rows
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            text
        );
        validate_source(&source).unwrap();
        for version in [
            LEGACY_PARSER_VERSION,
            CONTEXT_PARSER_VERSION,
            CURRENT_PARSER_VERSION,
        ] {
            let old = parse_source(text, version).unwrap();
            assert!(old.scenes.is_empty());
            validate_source(&old).unwrap();
            assert_eq!(
                serde_json::to_vec(&old).unwrap(),
                serde_json::to_vec(&upgrade_source_dialogues(&old).unwrap()).unwrap()
            );
        }
        let numeric_prose = "第一集\n场景：大厅\n3.14元可以买什么\n3-1日记原文\n3-1外套是黑色\n";
        let source = parse_source(numeric_prose, STRUCTURAL_PARSER_VERSION).unwrap();
        assert_eq!(source.scenes.len(), 1);
        assert!(source.rows[2..].iter().all(|r| r.kind == "body"));
        assert!(source.diagnostics.is_empty());
        assert_eq!(source.text, numeric_prose);
        validate_source(&source).unwrap();
    }

    #[test]
    fn source_structuring_frozen_v1_v2_v3_replay_without_boundary_migration() {
        let text = "第 0 集\n第零集\n辅助资料。\n第 1 集\n第一集\n场景：门口\n甲os：回来😀。\n";
        for version in [
            LEGACY_PARSER_VERSION,
            CONTEXT_PARSER_VERSION,
            CURRENT_PARSER_VERSION,
        ] {
            let source = parse_source(text, version).unwrap();
            let before = serde_json::to_vec(&source).unwrap();
            assert_eq!(source.episodes.len(), 4);
            assert!(source
                .diagnostics
                .iter()
                .any(|d| d.code == "DUPLICATE_EPISODE_NUMBER"));
            validate_source(&source).unwrap();
            assert_eq!(
                serde_json::to_vec(&upgrade_source_dialogues(&source).unwrap()).unwrap(),
                before
            );
            let recognized = reparse_source(&source).unwrap();
            assert_eq!(recognized.parser_version, STRUCTURAL_PARSER_VERSION);
            assert_eq!(recognized.episodes.len(), 1);
            for (old, new) in source.rows.iter().zip(&recognized.rows) {
                assert_eq!(old.id, new.id);
                assert_eq!(old.text, new.text);
                assert_eq!(old.range, new.range);
                assert_eq!(old.number, new.number);
            }
            assert_eq!(serde_json::to_vec(&source).unwrap(), before);
            validate_source(&recognized).unwrap();
        }
    }

    #[test]
    #[ignore = "requires private DIRECTOR_SOURCE_STRUCTURING_FIXTURE; reads only, no model or database"]
    fn source_structuring_private_first10_docx_prefix_has_exact_ten_production_episodes() {
        let path = std::env::var("DIRECTOR_SOURCE_STRUCTURING_FIXTURE")
            .expect("DIRECTOR_SOURCE_STRUCTURING_FIXTURE is required");
        let text = std::fs::read_to_string(path).unwrap();
        let source = prepare_source(&text).unwrap();
        assert_eq!(source.parser_version, STRUCTURAL_PARSER_VERSION);
        assert_eq!(source.episodes.len(), 10);
        assert_eq!(source.scenes.len(), 23);
        assert_eq!(
            source
                .episodes
                .iter()
                .map(|e| number_value(&e.source_number).unwrap())
                .collect::<Vec<_>>(),
            (1..=10).collect::<Vec<_>>()
        );
        assert_eq!(source.text, text);
        assert_eq!(source.sha256, digest(&text));
        assert_eq!(
            source
                .rows
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            text
        );
        assert!(source.rows.iter().any(|r| r.kind == "context"));
        assert!(
            source.diagnostics.iter().all(|d| !d.blocking),
            "blocking diagnostic codes: {:?}",
            source
                .diagnostics
                .iter()
                .filter(|d| d.blocking)
                .map(|d| d.code.as_str())
                .collect::<Vec<_>>()
        );
        assert!(!source.diagnostics.iter().any(|d| [
            "DUPLICATE_EPISODE_NUMBER",
            "ZERO_EPISODE_UNCONFIRMED",
            "SCENE_EPISODE_MISMATCH"
        ]
        .contains(&d.code.as_str())));
        validate_source(&source).unwrap();
    }

    #[test]
    fn case_insensitive_explicit_voice_labels_preserve_locked_text_and_speaker_case() {
        let text = "第一集\r\n场景：大厅\r\n小王os：我听见了😀。\r\n小李Vo：继续（压低声音）说。\r\nTom（内心oS）：原话里的OS与VO不变。\r\n孙OSkar：现场原话。\r\n小陈（低声） o.s.：不要动。\r\n小周 v.o.（轻快）：到了。\r\n";
        let source = prepare_source(text).unwrap();
        let actual: Vec<_> = source
            .rows
            .iter()
            .filter_map(|row| row.dialogue.as_ref())
            .map(|quote| (quote.speaker.as_str(), quote.channel.as_str()))
            .collect();
        assert_eq!(
            actual,
            vec![
                ("小王", "OS"),
                ("小李", "VO"),
                ("Tom", "OS"),
                ("孙OSkar", "现场"),
                ("小陈", "OS"),
                ("小周", "VO"),
            ]
        );
        assert_eq!(source.parser_version, "source-parser/v3");
        assert_eq!(source.text, text);
        assert_eq!(source.sha256, digest(text));
        assert_eq!(source.length, text.encode_utf16().count());
        assert_eq!(
            source
                .rows
                .iter()
                .map(|row| row.text.as_str())
                .collect::<String>(),
            text
        );
        for row in &source.rows {
            assert_eq!(slice_utf16(text, &row.range).unwrap(), row.text);
            if let Some(quote) = &row.dialogue {
                assert_eq!(slice_utf16(text, &quote.range).unwrap(), quote.quote);
                for part in quote
                    .spoken_segments
                    .iter()
                    .chain(&quote.performance_segments)
                {
                    assert_eq!(slice_utf16(text, &part.range).unwrap(), part.text);
                }
            }
        }
        let quoted = source.rows[4].dialogue.as_ref().unwrap();
        assert_eq!(quoted.quote, "原话里的OS与VO不变。");
        let directed = source.rows[3].dialogue.as_ref().unwrap();
        assert_eq!(spoken_text(directed), "继续说。");
        assert_eq!(directed.performance_segments[0].text, "（压低声音）");
        let literal_cues = prepare_source("第一集\n场景：大厅\nTom（pose保持，voice放松）：原话有OS和VO。\n小王（内心oS，低声）：不要动。\n小李（v.o.）：到了。\n").unwrap();
        assert_eq!(
            literal_cues.rows[2].dialogue.as_ref().unwrap().speaker,
            "Tom"
        );
        assert_eq!(
            literal_cues.rows[2].dialogue.as_ref().unwrap().channel,
            "现场"
        );
        assert_eq!(
            literal_cues.rows[2].dialogue.as_ref().unwrap().quote,
            "原话有OS和VO。"
        );
        assert_eq!(
            literal_cues.rows[3].dialogue.as_ref().unwrap().channel,
            "OS"
        );
        assert_eq!(
            literal_cues.rows[4].dialogue.as_ref().unwrap().channel,
            "VO"
        );
        validate_source(&literal_cues).unwrap();
        validate_source(&source).unwrap();
    }

    #[test]
    fn voice_label_upgrade_requires_explicit_reparse_and_keeps_all_source_identities() {
        let text = "【故事简介】\n背景😀。\n第一集\n场景：大厅\n小王os：我听见了（抬手）😀。\n小李 Vo：继续。\n";
        let old = parse_source(text, CONTEXT_PARSER_VERSION).unwrap();
        validate_source(&old).unwrap();
        assert_eq!(old.rows[4].dialogue.as_ref().unwrap().speaker, "小王os");
        assert_eq!(old.rows[4].dialogue.as_ref().unwrap().channel, "现场");
        assert!(old.rows[5].dialogue.is_none());
        let old_bytes = serde_json::to_vec(&old).unwrap();
        assert_eq!(
            serde_json::to_vec(&upgrade_source_dialogues(&old).unwrap()).unwrap(),
            old_bytes
        );

        let new = reparse_source(&old).unwrap();
        assert_eq!(new.parser_version, CURRENT_PARSER_VERSION);
        assert_eq!(new.text, old.text);
        assert_eq!(new.sha256, old.sha256);
        assert_eq!(new.length, old.length);
        assert_eq!(
            serde_json::to_value(&new.episodes).unwrap(),
            serde_json::to_value(&old.episodes).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&new.scenes).unwrap(),
            serde_json::to_value(&old.scenes).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&new.boundary_decisions).unwrap(),
            serde_json::to_value(&old.boundary_decisions).unwrap()
        );
        assert_eq!(new.scene_edits, old.scene_edits);
        for (before, after) in old.rows.iter().zip(&new.rows) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.text, after.text);
            assert_eq!(before.number, after.number);
            assert_eq!(before.range, after.range);
            assert_eq!(before.kind, after.kind);
            assert_eq!(before.episode_id, after.episode_id);
            assert_eq!(before.scene_id, after.scene_id);
        }
        let before = old.rows[4].dialogue.as_ref().unwrap();
        let after = new.rows[4].dialogue.as_ref().unwrap();
        assert_eq!(after.speaker, "小王");
        assert_eq!(after.channel, "OS");
        assert_eq!(after.id, before.id);
        assert_eq!(after.quote, before.quote);
        assert_eq!(after.range, before.range);
        assert_eq!(after.spoken_segments, before.spoken_segments);
        assert_eq!(after.performance_segments, before.performance_segments);
        assert_eq!(new.rows[5].dialogue.as_ref().unwrap().speaker, "小李");
        assert_eq!(new.rows[5].dialogue.as_ref().unwrap().channel, "VO");
        validate_source(&new).unwrap();
        assert_eq!(
            serde_json::to_vec(&new).unwrap(),
            serde_json::to_vec(&reparse_source(&new).unwrap()).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&new).unwrap(),
            serde_json::to_vec(&upgrade_source_dialogues(&new).unwrap()).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            old_bytes,
            "new parsing cannot mutate a frozen source"
        );
    }

    #[test]
    fn old_voice_metadata_cannot_be_silently_relabelled_under_either_parser_contract() {
        for version in [LEGACY_PARSER_VERSION, CONTEXT_PARSER_VERSION] {
            let old = parse_source("第一集\n场景：大厅\n小王os：不要动。\n", version).unwrap();
            validate_source(&old).unwrap();
            assert_eq!(
                upgrade_source_dialogues(&old).unwrap().rows[2].dialogue,
                old.rows[2].dialogue
            );
            let new = reparse_source(&old).unwrap();
            let mut forged_old = old.clone();
            forged_old.rows[2].dialogue = new.rows[2].dialogue.clone();
            assert!(validate_source(&forged_old).is_err());
            let mut forged_new = new.clone();
            forged_new.rows[2].dialogue = old.rows[2].dialogue.clone();
            assert!(validate_source(&forged_new).is_err());
        }
    }

    #[test]
    fn explicit_front_matter_is_context_and_existing_episode_scene_ids_stay_stable() {
        let text = "示例剧名\r\n【故事简介】\r\n主角来到新世界😀。\r\n【人物关系】\r\n姓名：苏远\r\n关系：男主\r\n【段纲】\r\n第一集发生重逢。\r\n\r\n第一集 重逢\r\n场景：门口\r\n苏远（低声）：回来😀。\r\n第二集\r\n2-1 夜 内 大厅\r\n苏远：门关了。\r\n";
        let legacy = parse_source(text, LEGACY_PARSER_VERSION).unwrap();
        let source = prepare_source(text).unwrap();
        assert_eq!(source.parser_version, CURRENT_PARSER_VERSION);
        assert!(source.diagnostics.is_empty());
        assert_eq!(source.episodes.len(), 2);
        assert_eq!(source.scenes.len(), 2);
        assert!(source.rows[..8].iter().all(|r| r.kind == "context"
            && r.episode_id.is_none()
            && r.scene_id.is_none()
            && r.dialogue.is_none()));
        assert_eq!(
            source
                .rows
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            text
        );
        assert_eq!(source.length, text.encode_utf16().count());
        assert_eq!(source.sha256, digest(text));
        assert_eq!(
            source.episodes.iter().map(|e| &e.id).collect::<Vec<_>>(),
            legacy.episodes.iter().map(|e| &e.id).collect::<Vec<_>>()
        );
        assert_eq!(
            source.scenes.iter().map(|s| &s.id).collect::<Vec<_>>(),
            legacy.scenes.iter().map(|s| &s.id).collect::<Vec<_>>()
        );
        for row in &source.rows {
            assert_eq!(slice_utf16(text, &row.range).unwrap(), row.text);
        }
        let dialogue = source.rows[11].dialogue.as_ref().unwrap();
        assert_eq!(dialogue.speaker, "苏远");
        assert_eq!(dialogue.quote, "回来😀。");
        validate_source(&source).unwrap();
    }

    #[test]
    fn background_titles_are_explicit_not_nearby_body_wording() {
        for heading in [
            "故事简介",
            "【剧情简介】",
            "[人物关系]",
            "（作者资料）",
            "(人物小传)",
            "## 世界观",
            "【段纲】：",
        ] {
            assert!(context_heading(heading, false), "{heading}");
        }
        for body in [
            "故事简介中的主角走来。",
            "人物关系已经改变。",
            "【故事简介】不像实际剧情",
            "【人物关系)",
            "苏远：故事简介。",
            "故事大纲 不是一句标题。",
            "人物关系：先别说。",
        ] {
            assert!(!context_heading(body, false), "{body}");
        }
        let source = prepare_source("第一集\n场景：大厅\n故事简介中的人物走来。\n人物关系：先别说。\n苏远：他说【故事简介】没有更新。\n").unwrap();
        assert!(source.diagnostics.is_empty());
        assert!(source.rows[2..].iter().all(|r| r.kind == "body"));
    }

    #[test]
    fn unknown_prefix_stays_a_local_issue_without_inventing_an_episode() {
        let source =
            prepare_source("未注明用途的原文。\n第一集\n场景：门口\n小王：走。\n").unwrap();
        assert_eq!(source.episodes.len(), 1);
        assert!(source.rows[0].episode_id.is_none());
        assert_eq!(source.diagnostics.len(), 1);
        assert_eq!(
            source.diagnostics[0].row_ids,
            vec![source.rows[0].id.clone()]
        );
    }

    #[test]
    fn unversioned_legacy_sources_keep_their_original_contract_until_explicit_reparse() {
        let text = "【故事简介】\r\n资料文字😀。\r\n第一集\r\n场景：门口\r\n小王（低声）：别动（抬手），回来。\r\n";
        let legacy = parse_source(text, LEGACY_PARSER_VERSION).unwrap();
        let mut saved = serde_json::to_value(&legacy).unwrap();
        saved.as_object_mut().unwrap().remove("parserVersion");
        for row in saved["rows"].as_array_mut().unwrap() {
            if let Some(dialogue) = row["dialogue"].as_object_mut() {
                dialogue.remove("spokenSegments");
                dialogue.remove("performanceSegments");
            }
        }
        let saved: LockedSource = serde_json::from_value(saved).unwrap();
        assert_eq!(saved.parser_version, LEGACY_PARSER_VERSION);
        validate_source(&saved).unwrap();
        let upgraded = upgrade_source_dialogues(&saved).unwrap();
        assert_eq!(upgraded.parser_version, LEGACY_PARSER_VERSION);
        assert_eq!(upgraded.rows[0].kind, "body");
        assert!(!upgraded.rows[4]
            .dialogue
            .as_ref()
            .unwrap()
            .spoken_segments
            .is_empty());
        let recognized = reparse_source(&saved).unwrap();
        assert_eq!(recognized.parser_version, CURRENT_PARSER_VERSION);
        assert_eq!(recognized.rows[0].kind, "context");
        assert!(recognized.diagnostics.is_empty());
        assert_eq!(recognized.text, text);
        assert_eq!(recognized.sha256, saved.sha256);
        assert_eq!(recognized.rows[4].dialogue, upgraded.rows[4].dialogue);
        let unchanged = reparse_source(&recognized).unwrap();
        assert_eq!(
            serde_json::to_value(&recognized).unwrap(),
            serde_json::to_value(unchanged).unwrap()
        );
    }

    #[test]
    fn reparse_keeps_all_confirmed_boundary_decisions_and_exact_source_coordinates() {
        let legacy = parse_source(
            "【故事简介】\r\n资料。\r\n第一集\r\n小王：走😀。\r\n",
            LEGACY_PARSER_VERSION,
        )
        .unwrap();
        let decided = apply_boundary_decisions(
            &legacy,
            &[BoundaryDecision {
                start: legacy.rows[3].range.start,
                end: legacy.length,
                kind: "body".into(),
                episode_number: Some(1),
                scene_title: Some("已确认门口".into()),
                reason: "用户明确这一句属于门口场".into(),
                decision_version: None,
                confirmation: None,
            }],
        )
        .unwrap();
        let recognized = reparse_source(&decided).unwrap();
        assert_eq!(
            serde_json::to_value(&decided.boundary_decisions).unwrap(),
            serde_json::to_value(&recognized.boundary_decisions).unwrap()
        );
        assert_eq!(recognized.rows[3].scene_id, decided.rows[3].scene_id);
        assert_eq!(recognized.rows[3].dialogue, decided.rows[3].dialogue);
        assert_eq!(recognized.text, decided.text);
        assert_eq!(recognized.sha256, decided.sha256);
        assert_eq!(recognized.length, decided.length);
        assert!(recognized.diagnostics.is_empty());
        validate_source(&recognized).unwrap();
    }

    #[test]
    fn utf16_crlf_exact_coverage_and_quotes() {
        let text="作者资料\r\n姓名：张三\r\n第一集 重逢\r\n场景：门口\r\n张三（压低声音）OS：\"回来😀。\"\r\n李四：10:30到了。";
        let s = prepare_source(text).unwrap();
        assert_eq!(
            s.rows.iter().map(|r| r.text.as_str()).collect::<String>(),
            text
        );
        assert_eq!(s.length, text.encode_utf16().count());
        assert_eq!(s.episodes.len(), 1);
        assert_eq!(s.scenes.len(), 1);
        assert!(s.diagnostics.is_empty());
        let q = s.rows.iter().find_map(|r| r.dialogue.as_ref()).unwrap();
        assert_eq!(q.speaker, "张三");
        assert_eq!(q.channel, "OS");
        assert_eq!(q.quote, "\"回来😀。\"");
        assert!(slice_utf16("😀", &SourceRange { start: 0, end: 1 }).is_err());
    }
    #[test]
    fn unknown_boundaries_require_decision_without_changing_text() {
        let source = prepare_source("小王：走。\n△他推门。\n").unwrap();
        assert!(!source.diagnostics.is_empty());
        let ready = apply_boundary_decisions(
            &source,
            &[BoundaryDecision {
                start: 0,
                end: source.length,
                kind: "body".into(),
                episode_number: Some(1),
                scene_title: Some("门口".into()),
                reason: "用户明确此范围为第一集门口场".into(),
                decision_version: None,
                confirmation: None,
            }],
        )
        .unwrap();
        assert_eq!(ready.text, source.text);
        assert_eq!(ready.sha256, source.sha256);
        assert!(ready.diagnostics.is_empty());
        assert_eq!(ready.scenes.len(), 1);
    }
    #[test]
    fn altered_or_reordered_source_rejected() {
        let mut source = prepare_source("第一集\n场景：门口\n小王：走。\n").unwrap();
        source.rows.swap(1, 2);
        assert!(validate_source(&source).is_err());
    }
    #[test]
    fn source_classification_cannot_be_tampered_and_decision_keeps_existing_episode() {
        let mut s = prepare_source("第一集\n小王：走。\n").unwrap();
        validate_source(&s).unwrap();
        let mut counterfeit = s.clone();
        counterfeit.rows[1].kind = "context".into();
        counterfeit.rows[1].dialogue = None;
        counterfeit.diagnostics.clear();
        assert!(validate_source(&counterfeit).is_err());
        s = apply_boundary_decisions(
            &s,
            &[BoundaryDecision {
                start: s.rows[1].range.start,
                end: s.length,
                kind: "body".into(),
                episode_number: Some(1),
                scene_title: Some("门口".into()),
                reason: "用户明确整集一场".into(),
                decision_version: None,
                confirmation: None,
            }],
        )
        .unwrap();
        assert_eq!(s.episodes.len(), 1);
        assert_eq!(s.episodes[0].number, 1);
        validate_source(&s).unwrap();
    }
    #[test]
    fn source_validation_memo_binds_projection_not_only_text_hash() {
        let source = prepare_source("第1集\n场景：大厅\n小王：原话不改。\n").unwrap();
        validate_source(&source).unwrap();
        validate_source(&source).unwrap();
        for field in ["speaker", "kind", "range", "episode", "scene", "parser"] {
            let mut changed = source.clone();
            match field {
                "speaker" => changed.rows[2].dialogue.as_mut().unwrap().speaker = "另一人".into(),
                "kind" => changed.rows[2].kind = "context".into(),
                "range" => changed.rows[2].range.end -= 1,
                "episode" => changed.episodes[0].number = 99,
                "scene" => changed.scenes[0].title = "另一地点".into(),
                _ => changed.parser_version = "unknown-parser".into(),
            }
            assert_eq!(changed.sha256, source.sha256);
            assert_eq!(changed.text, source.text);
            let expected = validate_source_uncached(&changed);
            assert!(expected.is_err(), "{field}");
            assert_eq!(validate_source(&changed), expected, "{field}");
            assert_eq!(validate_source(&changed), expected, "invalid is never admitted");
        }
        validate_source(&source).unwrap();
    }
    #[test]
    fn scene_mentions_scores_and_utc_colon_are_body() {
        let s = prepare_source("第一集\n场景：大厅\n小王：场景：门口，我们10:30走。\n10-20 分\n")
            .unwrap();
        assert_eq!(s.scenes.len(), 1);
        assert_eq!(s.rows[2].kind, "body");
        assert_eq!(s.rows[3].kind, "body");
    }
    #[test]
    fn numbered_labels_post_channel_hints_and_parenthetical_colons() {
        let s=prepare_source("第一集\n场景：大厅\n1号工人 VO（语气：轻快）：到齐了。\n10:30\n小王（内心：急切）OS：快些。\n").unwrap();
        let q = s.rows[2].dialogue.as_ref().unwrap();
        assert_eq!(q.speaker, "1号工人");
        assert_eq!(q.channel, "VO");
        assert_eq!(q.quote, "到齐了。");
        assert!(s.rows[3].dialogue.is_none());
        assert_eq!(s.rows[4].dialogue.as_ref().unwrap().channel, "OS");
    }
    #[test]
    fn explicit_bare_and_combined_scene_headers_without_rewriting() {
        let text = "第一集 场1-1 日 内 大厅\r\n小王：到了。\r\n1-2\r\n夜 外 门口\r\n小王：走。\r\n";
        let s = prepare_source(text).unwrap();
        assert!(s.diagnostics.is_empty());
        assert_eq!(s.scenes.len(), 2);
        assert_eq!(s.rows[3].kind, "format");
        assert_eq!(
            s.rows.iter().map(|r| r.text.as_str()).collect::<String>(),
            text
        );
    }
}

#[cfg(test)]
#[path = "source_m2_tests.rs"]
mod m2_tests;

#[cfg(test)]
#[path = "source_display_tests.rs"]
mod display_tests;

#[cfg(test)]
#[path="source_joint_tests.rs"]
mod joint_tests;

#[cfg(test)]
#[path = "source_title_tests.rs"]
mod title_tests;

#[cfg(test)]
#[path = "source_format_tests.rs"]
mod format_tests;
