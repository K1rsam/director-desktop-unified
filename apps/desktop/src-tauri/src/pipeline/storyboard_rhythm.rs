//! Source-bound timing coordinates, never a creative verdict or automatic repair.
//! Opt-in is frozen per workflow so an older request keeps its original identity.
use crate::pipeline::{digest, slice_utf16, SourceDialogue, SourceRange, SourceRow, TextRequest};
use serde_json::{json, Value};

const ENTRY_LIMIT: usize = 24;
const BRIDGE: &str = "声画定位合同 source-bound-rhythm-locator/v1：capabilities 的15/30秒等容量约束作用于完整生成单元，不是内部单镜长度；unit、工作包与shot不是一一对应。capacityBrief.rhythm.reference 的2–5秒和每30秒8–12镜仅为原生产规则的常用参考，不是上下限、配额或领域验收门槛。每个shots条目是一段连续取景，真实硬切另列条目；原句可按连续UTF16片段不断声跨镜或跨单元。audit只定位现稿的长镜、声窗、接话及已提供范围尾段，不认定空等、缺陷或质量通过。先读对应完整声画收益，再在本次主创/整编或用户选择的精修中局部联修；保留有收益的长镜、短爆点、必要气口、完整动作和幻想，不自动拆镜、压秒、裁词、clamp声窗或另购审稿调用。";

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn effective_characters(value: &str) -> usize {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count()
}

fn source_part_characters(line: &SourceDialogue, part: &Value) -> Option<usize> {
    let start = usize::try_from(part["quoteStart"].as_u64()?).ok()?;
    let end = usize::try_from(part["quoteEnd"].as_u64()?).ok()?;
    if end <= start {
        return None;
    }
    let segment = line
        .spoken_segments
        .iter()
        .find(|segment| segment.range.start <= start && end <= segment.range.end)?;
    let locked = slice_utf16(
        &segment.text,
        &SourceRange {
            start: start - segment.range.start,
            end: end - segment.range.start,
        },
    )
    .ok()?;
    Some(effective_characters(&locked))
}

#[derive(Clone)]
struct Window {
    coordinate: String,
    line_id: String,
    start: f64,
    end: f64,
    characters: Option<usize>,
    quote_start: Value,
    quote_end: Value,
}

fn limited(entries: &[Value]) -> (Vec<Value>, usize) {
    (
        entries.iter().take(ENTRY_LIMIT).cloned().collect(),
        entries.len().saturating_sub(ENTRY_LIMIT),
    )
}

fn reference() -> Value {
    json!({"commonShotSecondsRange":[2,5],"shotsPerThirtySecondsRange":[8,12],
        "naturalChineseCharactersPerMinuteRange":[300,360],"emotionalChineseCharactersPerMinuteRange":[240,420],
        "referenceOnly":true,"hardShotDurationBounds":false,"fixedShotCount":false,
        "sourceCharacterMethod":"locked_spoken_segments_unicode_letters_and_digits",
        "performedTimingVerified":false,"qualityVerdict":null})
}

/// The range clock concatenates only the supplied units. It never claims elapsed
/// story time, an entire episode, a performed voice or an empty tail.
fn locate(input: &Value, draft: Option<&Value>, rows: &[SourceRow], source_status: &str) -> Value {
    let mut result = json!({"version":crate::pipeline::stage3_pack::RHYTHM_PROFILE,
        "reference":reference(),"sourceRowsStatus":source_status,
        "sourceRowsSha256":digest(&input["sourceRows"].to_string()),
        "providedDraftSha256":draft.map(|draft|digest(&draft.to_string())),
        "clock":"playback_seconds_of_supplied_units_only","assessment":"mechanical_locator_only",
        "entryLimit":ENTRY_LIMIT});
    let Some(units) = draft.and_then(|draft| draft["units"].as_array()) else {
        result["audit"] = json!({"available":false,"reason":"no_supplied_shot_timeline",
            "viewingReview":{"longShots":[],"units":[]},"speech":{"lineTimings":[],"continuityGaps":[],"turnGaps":[],"fragmentSkews":[],"tailIntervals":[]}});
        return result;
    };
    let mut invalid = Vec::new();
    let mut total = 0.0;
    for (unit_index, unit) in units.iter().enumerate() {
        let Some(shots) = unit["shots"].as_array().filter(|shots| !shots.is_empty()) else {
            invalid.push(json!({"coordinate":format!("units[{unit_index}].shots"),"unitId":unit["id"],"problem":"missing_shot_timeline"}));
            continue;
        };
        for (shot_index, shot) in shots.iter().enumerate() {
            match shot["duration"].as_f64().filter(|duration|duration.is_finite() && *duration>0.0) {
                Some(duration) => total += duration,
                None => invalid.push(json!({"coordinate":format!("units[{unit_index}].shots[{shot_index}].duration"),"unitId":unit["id"],"shotId":shot["id"],"originalValue":shot["duration"],"problem":"unknown_or_non_positive_shot_duration"})),
            }
        }
    }
    if !total.is_finite() {
        invalid.push(json!({"coordinate":"provided_range","problem":"non_finite_playback_sum"}));
    }
    if !invalid.is_empty() || units.is_empty() {
        let (entries, omitted) = limited(&invalid);
        result["audit"] = json!({"available":false,"reason":"unreliable_shot_timeline",
            "totalSeconds":null,"invalidTimings":entries,"omittedInvalidTimings":omitted,
            "viewingReview":{"longShots":[],"units":[]},"speech":{"lineTimings":[],"continuityGaps":[],"turnGaps":[],"fragmentSkews":[],"tailIntervals":[]}});
        return result;
    }
    let lines: Vec<_> = rows
        .iter()
        .filter_map(|row| row.dialogue.as_ref())
        .collect();
    let mut unit_reviews = Vec::new();
    let mut long_shots = Vec::new();
    let mut windows = Vec::new();
    let mut invalid_voice = Vec::new();
    let mut unknown_source = Vec::new();
    let mut offset = 0.0;
    let mut shot_count = 0usize;
    let mut common_count = 0usize;
    let mut short_count = 0usize;
    let mut max_shot: f64 = 0.0;
    for (unit_index, unit) in units.iter().enumerate() {
        let shots = unit["shots"].as_array().unwrap();
        let mut local = 0.0;
        let mut unit_long_count = 0usize;
        for (shot_index, shot) in shots.iter().enumerate() {
            let duration = shot["duration"].as_f64().unwrap();
            let coordinate = format!("units[{unit_index}].shots[{shot_index}]");
            shot_count += 1;
            max_shot = max_shot.max(duration);
            if duration > 5.0 {
                unit_long_count += 1;
                long_shots.push(
                    json!({"coordinate":coordinate,"unitId":unit["id"],"shotId":shot["id"],
                    "duration":duration,"unitStart":local,"unitEnd":local+duration,
                    "rangeStart":offset+local,"rangeEnd":offset+local+duration}),
                );
            } else if duration < 2.0 {
                short_count += 1;
            } else {
                common_count += 1;
            }
            for (part_index, part) in shot["dialogueParts"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let part_coordinate = format!("{coordinate}.dialogueParts[{part_index}]");
                let matches: Vec<_> = lines
                    .iter()
                    .filter(|line| line.id == text(part, "lineId"))
                    .collect();
                let characters = if matches.len() == 1 {
                    source_part_characters(matches[0], part)
                } else {
                    None
                };
                if characters.is_none() {
                    unknown_source.push(json!({"coordinate":part_coordinate,"lineId":part["lineId"],"quoteStart":part["quoteStart"],"quoteEnd":part["quoteEnd"]}));
                }
                let Some((start, end)) = part["voiceStart"]
                    .as_f64()
                    .zip(part["voiceEnd"].as_f64())
                    .filter(|(start, end)| {
                        start.is_finite()
                            && end.is_finite()
                            && *start >= 0.0
                            && *end > *start
                            && *end <= duration
                    })
                else {
                    invalid_voice.push(json!({"coordinate":part_coordinate,"lineId":part["lineId"],"voiceStart":part["voiceStart"],"voiceEnd":part["voiceEnd"],"shotSeconds":duration}));
                    continue;
                };
                windows.push(Window {
                    coordinate: part_coordinate,
                    line_id: text(part, "lineId").into(),
                    start: offset + local + start,
                    end: offset + local + end,
                    characters,
                    quote_start: part["quoteStart"].clone(),
                    quote_end: part["quoteEnd"].clone(),
                });
            }
            local += duration;
        }
        unit_reviews.push(json!({"unitId":unit["id"],"seconds":local,"shotCount":shots.len(),
            "averageShotSeconds":local/shots.len() as f64,"shotsPerThirtySeconds":shots.len() as f64*30.0/local,
            "singleShotUnit":shots.len()==1,"longerThanCommonReferenceCount":unit_long_count,
            "rangeStart":offset,"rangeEnd":offset+local}));
        offset += local;
    }
    windows.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then_with(|| a.end.total_cmp(&b.end))
            .then_with(|| a.coordinate.cmp(&b.coordinate))
    });
    let mut turn_gaps = Vec::new();
    for pair in windows.windows(2) {
        if pair[0].line_id != pair[1].line_id && (pair[1].start - pair[0].end).abs() > 1e-6 {
            turn_gaps.push(json!({"fromLineId":pair[0].line_id,"toLineId":pair[1].line_id,
                "fromCoordinate":pair[0].coordinate,"toCoordinate":pair[1].coordinate,
                "rangeStart":pair[0].end,"rangeEnd":pair[1].start,"seconds":pair[1].start-pair[0].end}));
        }
    }
    let mut continuity_gaps = Vec::new();
    let mut fragment_skews = Vec::new();
    let mut line_timings = Vec::new();
    let mut fragment_details_remaining = ENTRY_LIMIT;
    for line in &lines {
        let parts: Vec<_> = windows
            .iter()
            .filter(|window| window.line_id == line.id)
            .collect();
        if parts.is_empty() {
            continue;
        }
        let window_seconds: f64 = parts.iter().map(|part| part.end - part.start).sum();
        let characters: Option<usize> = parts.iter().map(|part| part.characters).sum();
        let start = parts[0].start;
        let end = parts.iter().map(|part| part.end).fold(start, f64::max);
        let full_source_characters: usize = line
            .spoken_segments
            .iter()
            .map(|segment| effective_characters(&segment.text))
            .sum();
        line_timings.push(json!({"lineId":line.id,"speaker":line.speaker,"channel":line.channel,
            "partCount":parts.len(),"sourceEffectiveCharacters":full_source_characters,"providedEffectiveCharacters":characters,
            "rangeStart":start,"rangeEnd":end,"elapsedSeconds":end-start,"voiceWindowSeconds":window_seconds,
            "voiceWindowCharactersPerMinute":characters.map(|count|count as f64*60.0/window_seconds),
            "elapsedCharactersPerMinute":characters.map(|count|count as f64*60.0/(end-start)),
            "firstCoordinate":parts[0].coordinate,"lastCoordinate":parts.last().unwrap().coordinate}));
        for pair in parts.windows(2) {
            if (pair[1].start - pair[0].end).abs() > 1e-6 {
                continuity_gaps.push(json!({"lineId":line.id,"fromCoordinate":pair[0].coordinate,"toCoordinate":pair[1].coordinate,
                    "rangeStart":pair[0].end,"rangeEnd":pair[1].start,"seconds":pair[1].start-pair[0].end,
                    "sourceIntervalsAdjacent":pair[0].quote_end==pair[1].quote_start}));
            }
        }
        if parts.len() > 1 {
            // Bound all nested fragment details together, rather than allowing
            // ENTRY_LIMIT lines each to expand into ENTRY_LIMIT extra entries.
            let fragments:Vec<_>=parts.iter().take(fragment_details_remaining).map(|part|json!({"coordinate":part.coordinate,
                "sourceEffectiveCharacters":part.characters,"voiceSeconds":part.end-part.start,
                "charactersPerMinute":part.characters.map(|count|count as f64*60.0/(part.end-part.start))})).collect();
            fragment_details_remaining -= fragments.len();
            fragment_skews.push(json!({"lineId":line.id,"fragments":fragments,
                "omittedFragments":parts.len()-fragments.len(),"uniformRateRequired":false}));
        }
    }
    let last_voice = windows.iter().map(|window| window.end).fold(0.0, f64::max);
    let voice_intervals_known = invalid_voice.is_empty();
    let tail = if voice_intervals_known && last_voice < total {
        vec![
            json!({"rangeStart":last_voice,"rangeEnd":total,"seconds":total-last_voice,
        "boundary":"provided_range_end_not_proven_episode_or_scene_end","noSpeechWindows":windows.is_empty(),"isEmptyTime":null}),
        ]
    } else {
        vec![]
    };
    if !voice_intervals_known {
        // Skipping an unknown window cannot establish adjacent turns, a whole
        // source line's elapsed timing or a speech-free tail. Keep its exact
        // coordinate for the existing author/editor, without inventing silence.
        turn_gaps.clear();
        continuity_gaps.clear();
        fragment_skews.clear();
        line_timings.clear();
    }
    let (long_entries, long_omitted) = limited(&long_shots);
    let (unit_entries, unit_omitted) = limited(&unit_reviews);
    let (line_entries, line_omitted) = limited(&line_timings);
    let (continuity_entries, continuity_omitted) = limited(&continuity_gaps);
    let (turn_entries, turn_omitted) = limited(&turn_gaps);
    let (skew_entries, skew_omitted) = limited(&fragment_skews);
    let (voice_entries, voice_omitted) = limited(&invalid_voice);
    let (source_entries, source_omitted) = limited(&unknown_source);
    result["audit"] = json!({"available":true,"totalSeconds":total,"providedUnitCount":units.len(),
        "viewingReview":{"statistics":{"shotCount":shot_count,"averageShotSeconds":total/shot_count as f64,
            "maxShotSeconds":max_shot,"withinCommonReferenceCount":common_count,"belowCommonReferenceCount":short_count,
            "aboveCommonReferenceCount":long_shots.len()},"longShots":long_entries,"omittedLongShots":long_omitted,
            "units":unit_entries,"omittedUnits":unit_omitted},
        "speech":{"providedVoiceWindowsKnown":voice_intervals_known,
            "providedSourceIntervalsKnown":source_status=="present" && unknown_source.is_empty(),
            "intervalDerivationsAvailable":voice_intervals_known,
            "lineTimings":line_entries,"omittedLineTimings":line_omitted,
            "continuityGaps":continuity_entries,"omittedContinuityGaps":continuity_omitted,
            "turnGaps":turn_entries,"omittedTurnGaps":turn_omitted,"fragmentSkews":skew_entries,"omittedFragmentSkews":skew_omitted,
            "tailIntervals":tail,"invalidVoiceWindows":voice_entries,"omittedInvalidVoiceWindows":voice_omitted,
            "unknownSourceIntervals":source_entries,"omittedUnknownSourceIntervals":source_omitted}});
    result
}

fn annotate_schema(schema: &mut Value) {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        if properties.contains_key("camera") && properties.contains_key("picture") {
            if let Some(duration) = properties
                .get_mut("duration")
                .filter(|duration| duration.is_object())
            {
                duration["description"]=json!("本条连续取景的播放秒数，不是整个15/30秒生成单元容量。按实际观看收益定长；2–5秒仅为常用参考，允许有效长镜及短爆点。真实硬切另列shots，原句按连续源片段跨切发声，不改字、不裁声窗。");
            }
        }
        if let Some(range) = properties
            .get_mut("secondsRange")
            .filter(|range| range.is_object())
        {
            range["description"]=json!("初排生成单元的声画容量估计，不是内部镜头长度或交稿下限。容量为上限，自然短尾不拖长，不靠催读或删源凑档。");
        }
        if let Some(shots) = properties
            .get_mut("shots")
            .filter(|shots| shots.is_object())
        {
            shots["description"]=json!("完整生成单元内的实际镜表；unit不等于shot。每次独立硬切分别列条目，连续原话与幻想可跨镜保持，不按固定镜数或秒数机械拆合。");
        }
    }
    match schema {
        Value::Object(map) => {
            for child in map.values_mut() {
                annotate_schema(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                annotate_schema(child);
            }
        }
        _ => (),
    }
}

pub(in crate::pipeline) fn attach(request: &mut TextRequest) -> Result<(), String> {
    let source_status = if request.input.get("sourceRows").is_none() {
        "missing"
    } else {
        "unreadable"
    };
    let parsed = serde_json::from_value::<Vec<SourceRow>>(request.input["sourceRows"].clone());
    let source_status = if parsed.is_ok() {
        "present"
    } else {
        source_status
    };
    let rows = parsed.unwrap_or_default();
    let draft = request
        .input
        .get("previousDraft")
        .filter(|draft| draft["units"].is_array())
        .or_else(|| {
            request
                .input
                .get("draft")
                .filter(|draft| draft["units"].is_array())
        })
        .cloned()
        .or_else(|| {
            request.input["revisionTargetUnits"]
                .as_array()
                .map(|units| json!({"units":units}))
        });
    let locator = locate(&request.input, draft.as_ref(), &rows, source_status);
    if !request.input["capacityBrief"].is_object() {
        request.input["capacityBrief"] = super::capacity_brief(
            &request.input["capabilities"],
            draft.as_ref(),
            request.input.get("voiceReference"),
            &rows,
        );
    }
    // These old fill targets contradict the frozen Mac primary's upper-bound
    // policy. Older requests without the opt-in marker are never altered here.
    request.input["capacityBrief"]["preferredRange"] = Value::Null;
    request.input["capacityBrief"]["unitCapacityMeaning"] =
        json!("complete_generated_unit_not_internal_shot");
    request.input["capacityBrief"]["rhythm"] = locator;
    let runtime = request
        .rule_snapshot
        .get_mut("runtimeContract")
        .filter(|runtime| runtime.is_object())
        .ok_or("声画定位缺少可信运输合同；未派发新请求")?;
    let original = text(runtime, "content");
    let content = if original.ends_with(BRIDGE) {
        original.to_string()
    } else {
        format!("{original}\n\n{BRIDGE}")
    };
    runtime["rhythmProfile"] = json!(crate::pipeline::stage3_pack::RHYTHM_PROFILE);
    runtime["sha256"] = json!(digest(&content));
    runtime["content"] = json!(content);
    annotate_schema(&mut request.schema);
    Ok(())
}

#[cfg(test)]
mod rhythm_locator_tests {
    use super::*;

    fn fixture() -> (Value, Value, Vec<SourceRow>) {
        let source = crate::pipeline::prepare_source(
            "第一集\n场景：测试室\n小王（压低声）：甲乙😀丙丁。\n小李：收到。\n",
        )
        .unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .cloned()
            .collect();
        let first = rows[0].dialogue.as_ref().unwrap();
        let second = rows[1].dialogue.as_ref().unwrap();
        let split = first.range.start + "甲乙😀".encode_utf16().count();
        let part = |line: &SourceDialogue, a, b, start, end| {
            json!({"lineId":line.id,"quoteStart":a,"quoteEnd":b,
            "voiceStart":start,"voiceEnd":end,"text":"不得信任这段模型文字统计字数"})
        };
        let shot = |id: &str, seconds, parts: Vec<Value>| json!({"id":id,"duration":seconds,"sourceRefs":[],"dialogueParts":parts});
        let draft = json!({"units":[{"id":"u1","shots":[
            shot("s1",6.0,vec![part(first,first.range.start,split,0.5,2.5)]),
            shot("s2",4.0,vec![part(first,split,first.range.end,0.0,1.0)])]},
            {"id":"u2","shots":[shot("s3",5.0,vec![part(second,second.range.start,second.range.end,0.25,2.25)])]}]});
        (
            json!({"sourceRows":rows,"capabilities":{"maxDuration":15}}),
            draft,
            rows,
        )
    }

    #[test]
    fn source_bound_windows_cross_shots_and_units_without_timing_edits() {
        let (input, draft, rows) = fixture();
        let before = draft.clone();
        let report = locate(&input, Some(&draft), &rows, "present");
        assert_eq!(draft, before);
        assert_eq!(report["audit"]["totalSeconds"], 15.0);
        assert_eq!(
            report["audit"]["viewingReview"]["statistics"]["shotCount"],
            3
        );
        assert_eq!(
            report["audit"]["viewingReview"]["longShots"][0]["duration"],
            6.0
        );
        let speech = &report["audit"]["speech"];
        assert_eq!(speech["lineTimings"][0]["sourceEffectiveCharacters"], 4);
        assert_eq!(speech["lineTimings"][0]["providedEffectiveCharacters"], 4);
        assert_eq!(speech["continuityGaps"][0]["seconds"], 3.5);
        assert_eq!(speech["continuityGaps"][0]["sourceIntervalsAdjacent"], true);
        assert_eq!(speech["turnGaps"][0]["seconds"], 3.25);
        assert_eq!(speech["tailIntervals"][0]["seconds"], 2.75);
        assert_eq!(speech["tailIntervals"][0]["isEmptyTime"], Value::Null);
        assert_eq!(speech["fragmentSkews"][0]["uniformRateRequired"], false);
    }

    #[test]
    fn long_and_silent_shots_are_coordinates_without_quality_or_clamping() {
        let (input, mut draft, rows) = fixture();
        draft["units"][0]["shots"][0]["duration"] = json!(12.0);
        draft["units"][0]["shots"][0]["dialogueParts"] = json!([]);
        let before = draft.clone();
        let report = locate(&input, Some(&draft), &rows, "present");
        assert_eq!(draft, before);
        assert_eq!(
            report["audit"]["viewingReview"]["longShots"][0]["duration"],
            12.0
        );
        assert_eq!(report["reference"]["hardShotDurationBounds"], false);
        assert_eq!(report["reference"]["qualityVerdict"], Value::Null);
        assert_eq!(report["reference"]["performedTimingVerified"], false);
        assert_eq!(report["audit"]["totalSeconds"], 21.0);
    }

    #[test]
    fn zero_duration_does_not_invent_a_clock_and_invalid_source_does_not_trust_model_text() {
        let (input, mut draft, rows) = fixture();
        draft["units"][0]["shots"][0]["duration"] = json!(0.0);
        let report = locate(&input, Some(&draft), &rows, "present");
        assert_eq!(report["audit"]["available"], false);
        assert_eq!(report["audit"]["totalSeconds"], Value::Null);
        draft["units"][0]["shots"][0]["duration"] = json!(6.0);
        let first = rows[0].dialogue.as_ref().unwrap();
        draft["units"][0]["shots"][0]["dialogueParts"][0]["quoteStart"] =
            json!(first.range.start + 3);
        let report = locate(&input, Some(&draft), &rows, "present");
        assert_eq!(
            report["audit"]["speech"]["providedSourceIntervalsKnown"],
            false
        );
        assert_eq!(
            report["audit"]["speech"]["lineTimings"][0]["providedEffectiveCharacters"],
            Value::Null
        );
    }

    #[test]
    fn unknown_voice_window_never_becomes_a_gap_or_silent_tail() {
        let (input, mut draft, rows) = fixture();
        draft["units"][0]["shots"][1]["dialogueParts"][0]["voiceEnd"] = json!(50.0);
        let before = draft.clone();
        let report = locate(&input, Some(&draft), &rows, "present");
        assert_eq!(draft, before);
        assert_eq!(report["audit"]["available"], true);
        assert_eq!(report["audit"]["totalSeconds"], 15.0);
        let speech = &report["audit"]["speech"];
        assert_eq!(speech["providedVoiceWindowsKnown"], false);
        assert_eq!(speech["providedSourceIntervalsKnown"], true);
        assert_eq!(speech["intervalDerivationsAvailable"], false);
        assert_eq!(speech["invalidVoiceWindows"][0]["voiceEnd"], 50.0);
        assert_eq!(
            speech["invalidVoiceWindows"][0]["coordinate"],
            "units[0].shots[1].dialogueParts[0]"
        );
        for field in [
            "lineTimings",
            "continuityGaps",
            "turnGaps",
            "fragmentSkews",
            "tailIntervals",
        ] {
            assert_eq!(
                speech[field],
                json!([]),
                "unknown speech must not imply {field}"
            );
        }
    }

    #[test]
    fn coordinates_are_bounded_with_explicit_omission_counts() {
        let input = json!({"sourceRows":[]});
        let units:Vec<_>=(0..30).map(|i|json!({"id":format!("u{i}"),"shots":[{"id":format!("s{i}"),"duration":6.0,"dialogueParts":[]}]})).collect();
        let report = locate(&input, Some(&json!({"units":units})), &[], "present");
        assert_eq!(
            report["audit"]["viewingReview"]["statistics"]["aboveCommonReferenceCount"],
            30
        );
        assert_eq!(
            report["audit"]["viewingReview"]["longShots"]
                .as_array()
                .unwrap()
                .len(),
            ENTRY_LIMIT
        );
        assert_eq!(report["audit"]["viewingReview"]["omittedLongShots"], 6);
        assert_eq!(report["audit"]["totalSeconds"], 180.0);

        let source = crate::pipeline::prepare_source(&format!(
            "第一集\n场景：测试室\n小王：{}\n小李：{}\n",
            "甲".repeat(40),
            "乙".repeat(40)
        ))
        .unwrap();
        let rows: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == "body")
            .cloned()
            .collect();
        let mut units = Vec::new();
        for row in &rows {
            let line = row.dialogue.as_ref().unwrap();
            for i in 0..40 {
                units.push(json!({"id":format!("{}-{i}",line.id),"shots":[{"id":format!("s{}-{i}",line.id),
                    "duration":2.0,"dialogueParts":[{"lineId":line.id,"quoteStart":line.range.start+i,
                        "quoteEnd":line.range.start+i+1,"voiceStart":0.1,"voiceEnd":1.1}]}]}));
            }
        }
        let report = locate(
            &json!({"sourceRows":rows}),
            Some(&json!({"units":units})),
            &rows,
            "present",
        );
        let skews = report["audit"]["speech"]["fragmentSkews"]
            .as_array()
            .unwrap();
        assert_eq!(skews.len(), 2);
        assert_eq!(
            skews
                .iter()
                .map(|line| line["fragments"].as_array().unwrap().len())
                .sum::<usize>(),
            ENTRY_LIMIT
        );
        assert_eq!(
            skews
                .iter()
                .map(|line| line["omittedFragments"].as_u64().unwrap())
                .sum::<u64>(),
            80 - ENTRY_LIMIT as u64
        );
        assert_eq!(
            report["audit"]["viewingReview"]["statistics"]["shotCount"],
            80
        );
    }
}
