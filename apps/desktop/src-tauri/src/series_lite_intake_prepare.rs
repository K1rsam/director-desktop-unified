//! Original prepare-source.mjs boundary, heading projection and intake evidence.
use super::*;
use sha2::{Digest, Sha256};
fn reject(code: &str, message: &str, diagnostics: Vec<Value>) -> IntakeError {
    IntakeError {
        code: code.into(),
        message: Some(message.into()),
        field_path: None,
        source_unit_id: None,
        diagnostics,
    }
}
fn map_headings(prepared: &mut Value) -> IntakeResult<Value> {
    let units = arr(&prepared["index"]["units"], "index.units")?.clone();
    let mut by_id = BTreeMap::new();
    for (i, u) in units.iter().enumerate() {
        let mut u = u.clone();
        u["line"] = json!(i + 1);
        by_id.insert(st(&u["id"]).to_string(), u);
    }
    let mut mappings = vec![];
    for u in &units {
        let raw = st(&u["text"]);
        let no_end = re(r"(?:\r\n|\r|\n)$").replace(raw, "");
        let no_indent = re(r"^[\t \x{feff}\x{a0}\x{3000}]*").replace(&no_end, "");
        let value = re(r"^#{1,6}[\t ]+").replace(&no_indent, "");
        let kind=if re(r"^第[〇零一二三四五六七八九十百千万两\d]+集(?:（[^()（）\r\n]+）|\([^()（）\r\n]+\))?(?:[ \t:：]|$)").is_match(&value){Some("EPISODE_HEADING")}else if re(r"^\d+[ \t]*-[ \t]*\d+(?:[ \t:：]|$)").is_match(&value)||re(r"^\d+[ \t]*\.[ \t]*\d+[\t \x{a0}]*(?:日|夜|晨|昏|清晨|傍晚|凌晨|上午|下午|中午|深夜)[\t \x{a0},，、]+(?:内外|内|外)[\t \x{a0},，、]+\S").is_match(&value)||re(r"(?i)^(?:场景|SCENE)[\t :：]+[^\r\n:：]+$").is_match(&value){Some("SCENE_HEADING")}else{None};
        let Some(kind) = kind else { continue };
        let pos = arr(&prepared["plan"]["entries"], "plan.entries")?
            .iter()
            .position(|e| {
                e["unitIds"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&u["id"]))
            });
        if let Some(i) = pos {
            let e = &prepared["plan"]["entries"][i];
            if e["target"] != "BODY"
                || e["kind"] != "ACTION"
                || e["unitIds"].as_array().is_none_or(|a| a.len() != 1)
            {
                continue;
            }
        }
        let line = by_id[st(&u["id"])]["line"].clone();
        if pos.is_none()
            && !arr(&prepared["diagnostics"], "diagnostics")?
                .iter()
                .any(|d| {
                    d["line"] == line
                        && matches!(
                            st(&d["code"]),
                            "UNKNOWN_COLON_LINE" | "AMBIGUOUS_CONTINUATION"
                        )
                })
        {
            continue;
        }
        if let Some(i) = pos {
            prepared["plan"]["entries"][i]["target"] = json!("STRUCTURE");
            prepared["plan"]["entries"][i]["kind"] = json!(kind);
        } else {
            prepared["plan"]["entries"].as_array_mut().unwrap().push(json!({"id":format!("line-{}",line),"unitIds":[u["id"]],"target":"STRUCTURE","kind":kind,"parts":[{"kind":"SOURCE_SLICE","unitId":u["id"],"start":u["start"],"end":u["end"]}],"layout":[]}));
        }
        mappings.push(
            json!({"line":line,"unitId":u["id"],"kind":kind,"basis":"explicit-heading-layout-v1"}),
        );
    }
    if !mappings.is_empty() {
        let lines: HashSet<_> = mappings
            .iter()
            .map(|m| number(&m["line"]).unwrap())
            .collect();
        prepared["diagnostics"].as_array_mut().unwrap().retain(|d| {
            !number(&d["line"]).is_some_and(|l| lines.contains(&l))
                || !matches!(
                    st(&d["code"]),
                    "UNKNOWN_COLON_LINE" | "AMBIGUOUS_CONTINUATION"
                )
        });
        prepared["plan"]["entries"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|e| number(&by_id[st(&e["unitIds"][0])]["start"]).unwrap());
        if prepared["diagnostics"].as_array().unwrap().is_empty() {
            prepared["compiled"] = compile_standard_script(&prepared["index"], &prepared["plan"])?;
        }
    }
    let headings: Vec<_> = arr(&prepared["plan"]["entries"], "plan.entries")?
        .iter()
        .filter(|e| e["target"] == "STRUCTURE" && e["kind"] == "EPISODE_HEADING")
        .collect();
    if headings.len() > 1 {
        return Err(reject("INTAKE_MULTIPLE_EPISODES","单集入口检测到多个集标题；按源分集，重复或同号标题先核对归属",headings[1..].iter().map(|e|json!({"line":by_id[st(&e["unitIds"][0])]["line"],"code":"INTAKE_MULTIPLE_EPISODES","message":"保留原文；不能把多集全文绑定到一个单集 key。"})).collect()));
    }
    Ok(json!(mappings))
}
/// Does not dispatch or adopt. Source typing remains mechanical/semantic NOT_RUN.
pub fn prepare_source(text: &Value, key: &Value, known_speakers: &Value) -> IntakeResult<Value> {
    let invalid = |v: &Value| {
        v.as_str()
            .is_none_or(|s| trim(s).is_empty() || s.contains(['\r', '\n', '\0']))
    };
    if !text.is_string() || invalid(key) {
        return Err(reject("INTAKE_INPUT", "text/key 输入无效", vec![]));
    }
    let names = known_speakers
        .as_array()
        .filter(|a| a.iter().all(|n| !invalid(n)))
        .ok_or_else(|| {
            reject(
                "KNOWN_SPEAKERS",
                "known-speakers 须是明确说者的非空字符串数组，可用 []，不可将说明标头编成角色",
                vec![],
            )
        })?;
    let options = json!({"knownSpeakers":names,"syntax":"current","postColonCue":true,"inlinePerformance":true,"inlinePerformanceVersion":"inline-performance-v2"});
    let mut prepared = super::plan::prepare_standard_episode(text, key, &options)?;
    let mappings = map_headings(&mut prepared)?;
    let diagnostics = arr(&prepared["diagnostics"], "diagnostics")?;
    if !diagnostics.is_empty() {
        return Err(reject(
            "INTAKE_DIAGNOSTICS",
            "来源分型存在未解决歧义，未写 source",
            diagnostics.clone(),
        ));
    }
    let pkg = &prepared["compiled"]["package"];
    if !pkg.is_object() {
        return Err(reject(
            "INTAKE_NOT_COMPILED",
            "没有完整编译包，未写 source",
            vec![],
        ));
    }
    if !pkg["script"]["entries"]
        .as_array()
        .is_some_and(|es| es.iter().any(|e| e["target"] == "BODY"))
    {
        return Err(reject(
            "INTAKE_NO_BODY",
            "本集只有结构或说明，没有可制作正文",
            vec![
                json!({"line":1,"code":"INTAKE_NO_BODY","message":"原文保留；不能将说明编成剧情或对白。"}),
            ],
        ));
    }
    let raw = crate::series_lite_bundle_standard::standard_package_source(
        pkg,
        &json!({"key":key,"sourceText":text}),
    )?;
    let mut source = crate::series_lite_bundle_standard::normalize_standard_source(&raw)?;
    if let Some(ds) = source["diagnostics"].as_array().filter(|d| !d.is_empty()) {
        return Err(reject(
            "NORMALIZED_DIAGNOSTICS",
            "标准 source 仍有未决歧义，未写 source",
            ds.clone(),
        ));
    }
    source["intakeEvidence"] = json!({"version":"series-intake-evidence-v1","sourceText":text,"sourceSha256":format!("{:x}",Sha256::digest(st(text).as_bytes())),"sourceVersion":prepared["index"]["sourceVersion"],"options":options,"structureMappings":mappings,"plan":prepared["plan"],"lines":derive_standard_script_lines(&prepared)?,"receipt":prepared["compiled"]["receipt"],"proof":"机械回执仅证明原文/UTF16坐标、完整归属与分型映射；不证明语义正确、内容审核或用户验收。","semantic":"NOT_RUN"});
    Ok(source)
}
