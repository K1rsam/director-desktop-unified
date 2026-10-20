//! Lossless scene headers are evidence, never new production rows or prose.
//! Only new workflows opt in; unmarked frozen requests remain byte identical.
use super::*;

pub(crate) const PROFILE_FIELD: &str = "storyboardSceneFactsProfile";
pub(crate) const PACKET_FIELD: &str = "storyboardSceneFacts";
pub(crate) const PROFILE: &str = "source-scene-facts/v1";
const REQUIREMENTS: &str = "场景原文证据 source-scene-facts/v1：storyboardSceneFacts 按本岗实际制作源场提供锁定原场标题及首段场头原行；分行标题中的地点、时段与内外信息也属于原文事实。structuredSourceMetadata 逐值保全已保存的分场结构资料（包括正式分场修订保存的位置、时段和人物），不是补写的场头原文；它不覆盖原场头或正文，headerRows 为空时也不推断或补造丢失场头。结合 sourceRows 的实际动作和有依据的内部时空变化逐单元核对 scene、lighting 与逐镜声画，不能因简短场号、公共资产图或前稿错误而丢失或反转场头事实。正文后段若明确发生时间变化，应保留该局部变化，不用场头把整场锁成一个时段。合法闪回、幻想及明确跳转仍依原生产规则处理，不把整包强制写成同一场。headerRows 仅为原场事实证据，不扩大可制作 sourceRefs、对白区间或剧情许可；已在 sourceRows 内的行仍按原权限使用。整编及精修在保留单元前也须核对；实际修订通过正式差量回包，不由程序改写作品。";

fn hash(value: &Value) -> String {
    digest(&value.to_string())
}

pub(crate) fn enabled(input: &Value) -> Result<bool, String> {
    match (input.get(PROFILE_FIELD), input.get(PACKET_FIELD)) {
        (None, None) => Ok(false),
        (Some(Value::String(profile)), Some(packet)) if profile == PROFILE => {
            validate_packet(packet)?;
            Ok(true)
        }
        _ => Err("场景原文证据版本或冻结包不完整；原请求保留，未调用模型".into()),
    }
}

fn validate_packet(packet: &Value) -> Result<(), String> {
    let cards = packet["scenes"].as_array().ok_or("场景原文证据缺少场卡")?;
    if packet["version"] != PROFILE
        || packet["sourceSha256"]
            .as_str()
            .is_none_or(|s| s.len() != 64)
        || packet["catalogSha256"]
            .as_str()
            .is_none_or(|s| s.len() != 64)
        || packet["coordinateSystem"] != "utf16"
        || packet["parserVersion"].as_str().is_none_or(str::is_empty)
    {
        return Err("场景原文证据身份不完整".into());
    }
    let mut identities = HashSet::new();
    for card in cards {
        let scene = card["sceneId"].as_str().ok_or("场卡缺少源场身份")?;
        if scene.is_empty()
            || !identities.insert(scene)
            || card["episodeId"].as_str().is_none_or(str::is_empty)
            || !card["title"].is_string()
            || !card["structuredSourceMetadata"].is_object()
        {
            return Err("场景原文证据场身份为空或重复".into());
        }
        for row in card["headerRows"].as_array().ok_or("场卡缺少场头原行")? {
            let parsed: SourceRow =
                serde_json::from_value(row.clone()).map_err(|e| e.to_string())?;
            if parsed.scene_id.as_deref() != Some(scene)
                || parsed.episode_id.as_deref() != card["episodeId"].as_str()
                || row["textSha256"] != digest(&parsed.text)
                || parsed.range.end < parsed.range.start
                || parsed.range.end - parsed.range.start != parsed.text.encode_utf16().count()
            {
                return Err("场景原文证据的场头身份、区间或摘要不成立".into());
            }
        }
    }
    let mut unsigned = packet.clone();
    unsigned
        .as_object_mut()
        .ok_or("场景原文证据不是对象")?
        .remove("packetSha256");
    if packet["packetSha256"] != hash(&unsigned) {
        return Err("场景原文证据冻结摘要不匹配".into());
    }
    Ok(())
}

fn header_row(row: &SourceRow) -> Value {
    // Dialogue normalization is a separate versioned contract. Header evidence
    // binds the original row coordinates/text without copying derived speech.
    json!({"id":row.id,"number":row.number,"range":row.range,"text":row.text,
        "kind":row.kind,"episodeId":row.episode_id,"sceneId":row.scene_id,
        "textSha256":digest(&row.text)})
}

fn cards(source: &LockedSource) -> Vec<Value> {
    source.scenes.iter().enumerate().map(|(order, scene)| {
        let mut header = Vec::new();
        for row in source.rows.iter().filter(|row| row.scene_id.as_deref() == Some(&scene.id)) {
            if row.kind == "body" {
                // Inline headers retain their original body permission; the copy
                // below is just evidence and does not create another sourceRef.
                if header.is_empty() && row.text.trim_end() == scene.title.trim_end() {
                    header.push(header_row(row));
                }
                break;
            }
            if row.kind == "format" {
                header.push(header_row(row));
            }
        }
        json!({"episodeId":scene.episode_id,"sceneId":scene.id,"sourceOrder":order,"title":scene.title,
            "headerRows":header,"structuredSourceMetadata":scene.metadata})
    }).collect()
}

fn sign(mut packet: Value) -> Value {
    packet.as_object_mut().unwrap().remove("packetSha256");
    packet["packetSha256"] = json!(hash(&packet));
    packet
}

fn source_packet(source: &LockedSource, raw_source: &Value) -> Value {
    let scenes = cards(source);
    let mut packet = json!({"version":PROFILE,"sourceSha256":source.sha256,
        "parserVersion":source.parser_version,"coordinateSystem":source.coordinate_system,
        "catalogSha256":hash(&json!(scenes)),"scenes":scenes});
    if let Some(version) = raw_source.get("version") {
        packet["sourceVersion"] = version.clone();
    }
    sign(packet)
}

pub(crate) fn freeze_new(input: &mut Value) -> Result<(), String> {
    if enabled(input)? {
        return validate_source(
            input,
            &serde_json::from_value(input["source"].clone()).map_err(|e| e.to_string())?,
        );
    }
    let raw = input.get("source").ok_or("场景原文证据缺少锁定源文")?;
    let source: LockedSource = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
    super::validate_source(&source)?;
    let packet = source_packet(&source, raw);
    validate_packet(&packet)?;
    input[PROFILE_FIELD] = json!(PROFILE);
    input[PACKET_FIELD] = packet;
    Ok(())
}

pub(crate) fn validate_source(input: &Value, source: &LockedSource) -> Result<(), String> {
    if enabled(input)? && input[PACKET_FIELD] != source_packet(source, &input["source"]) {
        return Err("冻结场景原文证据与当前锁定源文不一致；未派发请求".into());
    }
    Ok(())
}

fn required_scenes(input: &Value) -> Result<HashSet<String>, String> {
    input["sourceRows"]
        .as_array()
        .ok_or("场景原文证据缺少制作源行")?
        .iter()
        .filter(|row| row["kind"] == "body")
        .map(|row| {
            row["sceneId"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| "制作源行缺少真实场身份".into())
        })
        .collect()
}

pub(crate) fn validate_request(input: &Value) -> Result<bool, String> {
    if !enabled(input)? {
        return Ok(false);
    }
    let required = required_scenes(input)?;
    let actual: HashSet<_> = input[PACKET_FIELD]["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["sceneId"].as_str().unwrap().to_owned())
        .collect();
    if required.is_empty() || required != actual {
        return Err("场景原文证据缺失或越过本岗制作源场；未派发请求".into());
    }
    for row in input["sourceRows"].as_array().unwrap() {
        if row["kind"] != "body" {
            continue;
        }
        let card = input[PACKET_FIELD]["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["sceneId"] == row["sceneId"])
            .unwrap();
        if card["episodeId"] != row["episodeId"] {
            return Err("场景原文证据与制作行集身份冲突".into());
        }
    }
    Ok(true)
}

pub(crate) fn attach_scope(
    target: &mut Value,
    frozen: &Value,
    source: &LockedSource,
) -> Result<(), String> {
    validate_source(frozen, source)?;
    if !enabled(frozen)? {
        return Ok(());
    }
    let required = required_scenes(target)?;
    let mut packet = frozen[PACKET_FIELD].clone();
    packet["scenes"] = json!(packet["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|card| required.contains(card["sceneId"].as_str().unwrap()))
        .cloned()
        .collect::<Vec<_>>());
    let mut proposed = target.clone();
    proposed[PROFILE_FIELD] = json!(PROFILE);
    proposed[PACKET_FIELD] = sign(packet);
    validate_request(&proposed)?;
    *target = proposed;
    Ok(())
}

pub(crate) fn merge_scopes(target: &mut Value, inputs: &[&Value]) -> Result<(), String> {
    let states = inputs
        .iter()
        .map(|input| validate_request(input))
        .collect::<Result<Vec<_>, _>>()?;
    if states.iter().all(|value| !value) {
        return Ok(());
    }
    if states.iter().any(|value| !value) {
        return Err("整编场景原文证据不能混用不同冻结合同".into());
    }
    let first = inputs.first().ok_or("整编缺少作者场景原文证据")?[PACKET_FIELD].clone();
    let mut found = std::collections::BTreeMap::new();
    for input in inputs {
        let packet = &input[PACKET_FIELD];
        for field in [
            "version",
            "sourceSha256",
            "sourceVersion",
            "parserVersion",
            "coordinateSystem",
            "catalogSha256",
        ] {
            if packet.get(field) != first.get(field) {
                return Err("整编场景原文证据并非同一冻结源目录".into());
            }
        }
        for card in packet["scenes"].as_array().unwrap() {
            let id = card["sceneId"].as_str().unwrap().to_owned();
            if let Some(previous) = found.insert(id, card.clone()) {
                if previous != *card {
                    return Err("整编作者场景原文证据冲突".into());
                }
            }
        }
    }
    let required = required_scenes(target)?;
    let mut packet = first;
    // Preserve original source order rather than map/hash iteration order.
    let mut scenes: Vec<_> = found
        .into_values()
        .filter(|card| required.contains(card["sceneId"].as_str().unwrap()))
        .collect();
    scenes.sort_by_key(|card| card["sourceOrder"].as_u64().unwrap_or(u64::MAX));
    packet["scenes"] = json!(scenes);
    let mut proposed = target.clone();
    proposed[PROFILE_FIELD] = json!(PROFILE);
    proposed[PACKET_FIELD] = sign(packet);
    validate_request(&proposed)?;
    *target = proposed;
    Ok(())
}

pub(crate) fn inherit(target: &mut Value, frozen: &Value) -> Result<(), String> {
    let active = enabled(frozen)?;
    for field in [PROFILE_FIELD, PACKET_FIELD] {
        if target.get(field).is_some() && (!active || target.get(field) != frozen.get(field)) {
            return Err("精修采用不能新增或变更原冻结场景原文证据".into());
        }
    }
    if active {
        let mut proposed = target.clone();
        proposed[PROFILE_FIELD] = frozen[PROFILE_FIELD].clone();
        proposed[PACKET_FIELD] = frozen[PACKET_FIELD].clone();
        if proposed.get("source").is_some() {
            let source: LockedSource =
                serde_json::from_value(proposed["source"].clone()).map_err(|e| e.to_string())?;
            validate_source(&proposed, &source)?;
        }
        *target = proposed;
    }
    Ok(())
}

pub(crate) fn attach_contract(contract: &mut Value, enabled: bool) {
    if enabled {
        let content = format!(
            "{}\n\n{REQUIREMENTS}",
            contract["content"].as_str().unwrap_or("")
        );
        contract["sceneFactsProfile"] = json!(PROFILE);
        contract["content"] = json!(content);
        contract["sha256"] = json!(digest(contract["content"].as_str().unwrap()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (LockedSource, Value) {
        let source = prepare_source("第七集\n7-1\n测试湖 夜 / 外\n人物：角色甲\n\n角色甲OS：先取甲。\n7-2\n洞口 日 / 内\n△他收起龟甲。\n第八集\n8-1\n森林 晨 / 外\n△他醒来。\n").unwrap();
        let mut input = json!({"source":source});
        input["source"]["version"] = json!(3);
        freeze_new(&mut input).unwrap();
        (source, input)
    }
    fn scoped(source: &LockedSource, frozen: &Value, scene: usize) -> Value {
        let mut input = json!({"sourceRows":source.rows.iter().filter(|row|row.kind=="body" && row.scene_id.as_deref()==Some(&source.scenes[scene].id)).collect::<Vec<_>>(),
            "contextRows":source.rows.iter().filter(|row|row.kind=="body" && row.scene_id.as_deref()!=Some(&source.scenes[scene].id)).collect::<Vec<_>>()});
        attach_scope(&mut input, frozen, source).unwrap();
        input
    }

    #[test]
    fn split_scene_header_retains_exact_source_rows_and_body_permissions() {
        let (source, frozen) = fixture();
        assert_eq!(source.scenes.len(), 3);
        assert_eq!(source.scenes[0].title, "7-1");
        let original = serde_json::to_value(&source).unwrap();
        let request = scoped(&source, &frozen, 0);
        let card = &request[PACKET_FIELD]["scenes"][0];
        let header = card["headerRows"].as_array().unwrap();
        assert!(header.iter().any(|row| row["text"] == "7-1\n"));
        let line = header
            .iter()
            .find(|row| row["text"] == "测试湖 夜 / 外\n")
            .unwrap();
        let row = source
            .rows
            .iter()
            .find(|row| row.text == "测试湖 夜 / 外\n")
            .unwrap();
        for (field, value) in header_row(row).as_object().unwrap() {
            assert_eq!(line[field], *value);
        }
        assert_eq!(line["kind"], "format");
        assert_eq!(request[PACKET_FIELD]["sourceVersion"], 3);
        assert_eq!(
            request["sourceRows"],
            json!(source
                .rows
                .iter()
                .filter(|row| row.kind == "body"
                    && row.scene_id
                        == source
                            .rows
                            .iter()
                            .find(|row| row.text == "测试湖 夜 / 外\n")
                            .unwrap()
                            .scene_id)
                .collect::<Vec<_>>())
        );
        assert!(request["sourceRows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["kind"] == "body"));
        assert_eq!(serde_json::to_value(&source).unwrap(), original);
        assert_eq!(
            request[PACKET_FIELD]["scenes"].as_array().unwrap().len(),
            1,
            "context scenes never leak into production cards"
        );
        validate_request(&request).unwrap();
    }

    #[test]
    fn finish_merges_same_source_cards_and_rejects_missing_conflicting_or_mixed_packets() {
        let (source, frozen) = fixture();
        let one = scoped(&source, &frozen, 0);
        let two = scoped(&source, &frozen, 1);
        let mut target = json!({"sourceRows":source.rows.iter().filter(|row|row.kind=="body" && row.episode_id.as_deref()==Some(&source.episodes[0].id)).collect::<Vec<_>>()});
        merge_scopes(&mut target, &[&two, &one]).unwrap();
        assert_eq!(target[PACKET_FIELD]["scenes"].as_array().unwrap().len(), 2);
        assert_eq!(
            target[PACKET_FIELD]["scenes"][0],
            one[PACKET_FIELD]["scenes"][0]
        );
        let before = target.clone();
        assert!(merge_scopes(&mut target, &[&one]).is_err());
        assert_eq!(target, before);
        let mut changed = one.clone();
        changed[PACKET_FIELD]["scenes"][0]["title"] = json!("虚构日间");
        changed[PACKET_FIELD] = sign(changed[PACKET_FIELD].clone());
        assert!(merge_scopes(&mut target, &[&one, &changed, &two]).is_err());
        assert_eq!(target, before);
        let mut other = two.clone();
        other[PACKET_FIELD]["sourceVersion"] = json!(4);
        other[PACKET_FIELD] = sign(other[PACKET_FIELD].clone());
        assert!(merge_scopes(&mut target, &[&one, &other]).is_err());
        assert_eq!(target, before);
        let mut absent = one.clone();
        absent[PACKET_FIELD]
            .as_object_mut()
            .unwrap()
            .remove("sourceVersion");
        absent[PACKET_FIELD] = sign(absent[PACKET_FIELD].clone());
        let mut null = two.clone();
        null[PACKET_FIELD]["sourceVersion"] = Value::Null;
        null[PACKET_FIELD] = sign(null[PACKET_FIELD].clone());
        assert!(
            merge_scopes(&mut target, &[&absent, &null]).is_err(),
            "absent source version is not a null source version"
        );
        assert_eq!(target, before);
        let mut old = two.clone();
        old.as_object_mut().unwrap().remove(PROFILE_FIELD);
        old.as_object_mut().unwrap().remove(PACKET_FIELD);
        assert!(merge_scopes(&mut target, &[&one, &old]).is_err());
        assert_eq!(target, before);
    }

    #[test]
    fn frozen_source_rejects_tampering_even_with_recomputed_packet_digest() {
        let (source, frozen) = fixture();
        for mode in ["source", "scene", "row", "range", "order", "catalog"] {
            let mut changed = frozen.clone();
            match mode {
                "source" => changed[PACKET_FIELD]["sourceSha256"] = json!("0".repeat(64)),
                "scene" => changed[PACKET_FIELD]["scenes"][0]["title"] = json!("新标题"),
                "row" => {
                    changed[PACKET_FIELD]["scenes"][0]["headerRows"][0]["text"] = json!("7-9\n");
                    changed[PACKET_FIELD]["scenes"][0]["headerRows"][0]["textSha256"] =
                        json!(digest("7-9\n"));
                }
                "range" => {
                    changed[PACKET_FIELD]["scenes"][0]["headerRows"][0]["range"]["start"] = json!(0)
                }
                "order" => changed[PACKET_FIELD]["scenes"][0]["sourceOrder"] = json!(99),
                _ => changed[PACKET_FIELD]["catalogSha256"] = json!("0".repeat(64)),
            }
            changed[PACKET_FIELD] = sign(changed[PACKET_FIELD].clone());
            assert!(validate_source(&changed, &source).is_err(), "{mode}");
        }
        for value in [Value::Null, json!(false), json!("unknown/v99")] {
            let mut bad = frozen.clone();
            bad[PROFILE_FIELD] = value;
            assert!(enabled(&bad).is_err());
        }
        for field in [PROFILE_FIELD, PACKET_FIELD] {
            let mut bad = frozen.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(enabled(&bad).is_err());
        }
    }

    #[test]
    fn inheritance_and_old_absence_are_atomic_and_do_not_upgrade_old_candidates() {
        let (source, frozen) = fixture();
        let original = json!({"source":frozen["source"],"unrelated":"unchanged"});
        let mut target = original.clone();
        inherit(&mut target, &json!({})).unwrap();
        assert_eq!(target, original);
        inherit(&mut target, &frozen).unwrap();
        assert_eq!(target[PACKET_FIELD], frozen[PACKET_FIELD]);
        let before = target.clone();
        assert!(inherit(&mut target, &json!({})).is_err());
        assert_eq!(target, before);
        let mut target = original.clone();
        target["source"]["version"] = json!(4);
        let before = target.clone();
        assert!(inherit(&mut target, &frozen).is_err());
        assert_eq!(target, before);
        let mut old = json!({"sourceRows":[],"content":"old"});
        let before = old.clone();
        attach_scope(&mut old, &json!({}), &source).unwrap();
        assert_eq!(old, before);
        let mut contract = json!({"content":"original","sha256":"old"});
        let before = contract.clone();
        attach_contract(&mut contract, false);
        assert_eq!(contract, before);
    }

    #[test]
    fn inline_body_and_uninterpreted_titles_keep_their_original_roles() {
        let source = prepare_source(
            "第一集 场1-1 日 内 大厅\r\n小王：到了。\r\n1-2\r\n夜 外 门口\r\n小王：走。\r\n",
        )
        .unwrap();
        let raw = json!({"source":source});
        let mut frozen = raw.clone();
        freeze_new(&mut frozen).unwrap();
        validate_source(&frozen, &source).unwrap();
        assert_eq!(frozen["source"], raw["source"]);
        let source = prepare_source("第一集\n场景：无时段洞穴\n△他站定。\n").unwrap();
        let mut input = json!({"source":source});
        freeze_new(&mut input).unwrap();
        assert_eq!(
            input[PACKET_FIELD]["scenes"][0]["title"],
            source.scenes[0].title
        );
        let text = input[PACKET_FIELD].to_string();
        assert!(!text.contains("dayTime"));
        assert!(!text.contains("nightTime"));
    }

    #[test]
    fn same_scene_header_does_not_replace_a_later_explicit_body_time_change() {
        let source=prepare_source("第八集\n8-1\n测试湖 日 / 外\n△角色甲游向湖心。\n角色甲OS：大晚上还不安静。\n△夜色中泛起水光。\n").unwrap();
        assert_eq!(source.scenes.len(), 1);
        let mut frozen = json!({"source":source});
        freeze_new(&mut frozen).unwrap();
        let request = scoped(&source, &frozen, 0);
        assert!(request[PACKET_FIELD]["scenes"][0]["headerRows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["text"] == "测试湖 日 / 外\n"));
        assert_eq!(
            request["sourceRows"],
            json!(source
                .rows
                .iter()
                .filter(|row| row.kind == "body")
                .collect::<Vec<_>>())
        );
        assert!(request["sourceRows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["text"] == "△夜色中泛起水光。\n"));
        let mut contract = json!({"content":"原3008规则"});
        attach_contract(&mut contract, true);
        assert!(contract["content"]
            .as_str()
            .unwrap()
            .contains("不用场头把整场锁成一个时段"));
        assert!(request.get("dayTime").is_none());
        assert!(request.get("nightTime").is_none());
    }

    #[test]
    fn approved_child_scene_keeps_structured_metadata_without_inventing_a_header() {
        let source=prepare_source("第九集\n9-1\n测试湖 夜 / 外\n△两人在湖岸交谈。\n△两名黑衣人一起入水。\n甲：目标在湖底。\n").unwrap();
        let cut = source
            .rows
            .iter()
            .find(|row| row.text.starts_with("△两名黑衣人一起入水。"))
            .unwrap();
        let child = format!("{}__L{}", cut.scene_id.as_ref().unwrap(), cut.number);
        let edits = json!([
            {"kind":"sceneSplit","sourceSha256":source.sha256,"plan":{"episodeId":cut.episode_id,"sceneId":cut.scene_id,"cuts":[{"rowId":cut.id,"anchor":cut.text.trim_end_matches(['\r','\n'])}]}},
            {"kind":"sceneMetadata","sourceSha256":source.sha256,"plan":{"episodeId":cut.episode_id,"sceneId":child,"sourceRefs":[cut.id],"location":"测试湖｜湖底","timeOfDay":"夜","interiorExterior":"外","characters":["黑衣人甲","黑衣人乙"]}}
        ]);
        let revised = super::super::scene_edits::preview_foundation_scene_edits(
            &source,
            edits.as_array().unwrap(),
            &[source.episodes[0].id.clone()],
        )
        .unwrap();
        super::super::validate_source(&revised).unwrap();
        let saved = serde_json::to_value(&revised).unwrap();
        let mut frozen = json!({"source":revised});
        freeze_new(&mut frozen).unwrap();
        let index = revised
            .scenes
            .iter()
            .position(|scene| scene.id == child)
            .unwrap();
        let request = scoped(&revised, &frozen, index);
        let card = &request[PACKET_FIELD]["scenes"][0];
        assert_eq!(card["sceneId"], child);
        assert_eq!(
            card["headerRows"],
            json!([]),
            "a body-starting child has no newly invented header"
        );
        assert_eq!(
            card["structuredSourceMetadata"],
            json!(revised.scenes[index].metadata)
        );
        assert_eq!(card["structuredSourceMetadata"]["location"], "测试湖｜湖底");
        assert_eq!(card["structuredSourceMetadata"]["timeOfDay"], "夜");
        assert_eq!(
            frozen["source"], saved,
            "freezing never changes saved split, metadata, body or speech"
        );
        assert_eq!(
            request["sourceRows"],
            json!(revised
                .rows
                .iter()
                .filter(|row| row.kind == "body" && row.scene_id.as_deref() == Some(child.as_str()))
                .collect::<Vec<_>>())
        );
        let mut tampered = frozen.clone();
        tampered[PACKET_FIELD]["scenes"][index]["structuredSourceMetadata"]["location"] =
            json!("虚构场所");
        tampered[PACKET_FIELD] = sign(tampered[PACKET_FIELD].clone());
        assert!(
            super::validate_source(&tampered, &revised).is_err(),
            "saved metadata remains source-bound despite a recomputed transport hash"
        );
        for invalid in [Value::Null, json!([]), json!("missing")] {
            let mut bad = request.clone();
            bad[PACKET_FIELD]["scenes"][0]["structuredSourceMetadata"] = invalid;
            bad[PACKET_FIELD] = sign(bad[PACKET_FIELD].clone());
            assert!(validate_request(&bad).is_err());
        }
    }
}
