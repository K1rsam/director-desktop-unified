//! v10 only: explicit source labels are lexical boundaries, not guessed speakers.
use super::*;

pub(super) fn speaker_names(source: &LockedSource) -> HashSet<String> {
    let mut names = declared_speaker_names(source);
    // Independent ordinary lines may establish the same exact lexical label.
    // Never learn a speaker from the tail whose boundary is being decided.
    names.extend(
        source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .filter_map(|row| {
                parse_dialogue(
                    &row.text,
                    row.range.start,
                    &row.id,
                    TITLE_METADATA_PARSER_VERSION,
                )
            })
            .map(|dialogue| dialogue.speaker),
    );
    names
}

pub(super) fn marked_dialogue(row: &SourceRow, names: &HashSet<String>) -> Option<SourceDialogue> {
    let leading = row.text.len() - row.text.trim_start().len();
    let after = leading
        + row.text[leading..]
            .strip_prefix('△')
            .map(|_| '△'.len_utf8())?;
    let dialogue = parse_dialogue(
        &row.text[after..],
        row.range.start + row.text[..after].encode_utf16().count(),
        &row.id,
        EXPLICIT_INLINE_PARSER_VERSION,
    )?;
    names.contains(&dialogue.speaker).then_some(dialogue)
}

fn lexical_dialogue(
    text: &str,
    start: usize,
    id: &str,
    names: &HashSet<String>,
) -> Option<SourceDialogue> {
    let row = SourceRow {
        id: id.into(),
        number: 0,
        range: SourceRange {
            start,
            end: start + text.encode_utf16().count(),
        },
        text: text.into(),
        kind: "body".into(),
        episode_id: None,
        scene_id: None,
        dialogue: None,
    };
    marked_dialogue(&row, names)
        .or_else(|| parse_dialogue(text, start, id, EXPLICIT_INLINE_PARSER_VERSION))
        .filter(|dialogue| names.contains(&dialogue.speaker))
}

pub(super) fn outside_enclosure(text: &str, target: usize) -> bool {
    let mut stack = Vec::new();
    let mut escaped = false;
    for (byte, ch) in text.char_indices() {
        if byte >= target {
            break;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if stack.last() == Some(&ch) {
            stack.pop();
            continue;
        }
        if stack
            .last()
            .is_some_and(|close| ['”', '’', '」', '』', '"', '\''].contains(close))
        {
            continue;
        }
        let close = match ch {
            '“' => Some('”'),
            '‘' => Some('’'),
            '「' => Some('」'),
            '『' => Some('』'),
            '"' => Some('"'),
            '\'' => Some('\''),
            '（' => Some('）'),
            '(' => Some(')'),
            '[' => Some(']'),
            '【' => Some('】'),
            '{' => Some('}'),
            '《' => Some('》'),
            _ => None,
        };
        if let Some(close) = close {
            stack.push(close);
        }
    }
    stack.is_empty()
}

/// Returns byte slice boundaries. Coordinates are always derived later from
/// exact UTF-16 prefixes; enclosures and quoted examples cannot open a speaker.
fn split_points(row: &SourceRow, names: &HashSet<String>) -> Vec<usize> {
    if lexical_dialogue(&row.text, row.range.start, &row.id, names).is_none() {
        return vec![];
    }
    let mut stack = Vec::new();
    let mut escaped = false;
    let mut points = Vec::new();
    for (byte, ch) in row.text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if stack.last() == Some(&ch) {
            stack.pop();
            continue;
        }
        // A quotation protects every bracket and colon inside it.
        if stack
            .last()
            .is_some_and(|close| ['”', '’', '」', '』', '"', '\''].contains(close))
        {
            continue;
        }
        let closing = match ch {
            '“' => Some('”'),
            '‘' => Some('’'),
            '「' => Some('」'),
            '『' => Some('』'),
            '"' => Some('"'),
            '\'' => Some('\''),
            '（' => Some('）'),
            '(' => Some(')'),
            '[' => Some(']'),
            '【' => Some('】'),
            '{' => Some('}'),
            '《' => Some('》'),
            _ => None,
        };
        if let Some(close) = closing {
            stack.push(close);
            continue;
        }
        if !stack.is_empty() || !['。', '！', '？', '!', '?', ';', '；'].contains(&ch) {
            continue;
        }
        let boundary = byte + ch.len_utf8();
        if lexical_dialogue(
            &row.text[boundary..],
            row.range.start + row.text[..boundary].encode_utf16().count(),
            &row.id,
            names,
        )
        .is_some()
        {
            points.push(boundary);
        }
    }
    points
}

pub(super) fn changes_dialogue_rows(old: &LockedSource, new: &LockedSource) -> bool {
    old.rows.len() != new.rows.len()
        || old
            .rows
            .iter()
            .zip(&new.rows)
            .any(|(a, b)| a.id != b.id || a.dialogue != b.dialogue)
}

pub(super) fn apply(source: &mut LockedSource) {
    let names = speaker_names(source);
    let display = display_only_row_ids(source);
    let old = std::mem::take(&mut source.rows);
    for row in old {
        if row.kind != "body" || display.contains(&row.id) {
            source.rows.push(row);
            continue;
        }
        let points = split_points(&row, &names);
        if points.is_empty() {
            let mut row = row;
            if let Some(dialogue) = marked_dialogue(&row, &names) {
                row.dialogue = Some(dialogue);
            }
            source.rows.push(row);
            continue;
        }
        let mut from = 0;
        let mut replacement_ids = Vec::new();
        for to in points.into_iter().chain(std::iter::once(row.text.len())) {
            let mut fragment = row.clone();
            fragment.range = SourceRange {
                start: row.range.start + row.text[..from].encode_utf16().count(),
                end: row.range.start + row.text[..to].encode_utf16().count(),
            };
            fragment.id = format!(
                "row:{}:{}:{}",
                &source.sha256[..12],
                fragment.range.start,
                fragment.range.end
            );
            fragment.text = row.text[from..to].into();
            fragment.dialogue =
                lexical_dialogue(&fragment.text, fragment.range.start, &fragment.id, &names);
            replacement_ids.push(fragment.id.clone());
            source.rows.push(fragment);
            from = to;
        }
        for diagnostic in &mut source.diagnostics {
            diagnostic.row_ids = diagnostic
                .row_ids
                .iter()
                .flat_map(|id| {
                    if id == &row.id {
                        replacement_ids.clone()
                    } else {
                        vec![id.clone()]
                    }
                })
                .collect();
        }
    }
    // Rebuild only memberships. Episode/scene identities and raw coordinates stay.
    for episode in &mut source.episodes {
        episode.row_ids = source
            .rows
            .iter()
            .filter(|row| row.episode_id.as_ref() == Some(&episode.id))
            .map(|row| row.id.clone())
            .collect();
    }
    for scene in &mut source.scenes {
        scene.row_ids = source
            .rows
            .iter()
            .filter(|row| row.scene_id.as_ref() == Some(&scene.id))
            .map(|row| row.id.clone())
            .collect();
    }
}

#[cfg(test)]
#[path = "source_explicit_inline_tests.rs"]
mod tests;
