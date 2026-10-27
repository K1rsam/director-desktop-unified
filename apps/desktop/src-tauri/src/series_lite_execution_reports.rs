use super::*;
fn js_space(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn names(raw: &str) -> Vec<String> {
    if matches!(raw, "无" | "待确认") {
        return vec![];
    }
    let mut chars = raw.chars().peekable();
    let mut names = vec![];
    let mut item = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(|n| matches!(n, '\\' | '、')) {
            item.push(chars.next().unwrap());
        } else if c == '、' {
            names.push(item.trim_matches(js_space).to_string());
            item.clear();
        } else {
            item.push(c);
        }
    }
    names.push(item.trim_matches(js_space).to_string());
    names
}
pub(super) fn field_budget(md: &str) -> Value {
    let lines: Vec<_> = md
        .strip_prefix('\u{feff}')
        .unwrap_or(md)
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut active = false;
    let mut episode = "";
    let mut rows = vec![];
    let episode_re = regex::Regex::new(r"^第[0-9零〇一二两三四五六七八九十百千万]+集$").unwrap();
    let heading = regex::Regex::new(r"^\*\*【.+】\*\*$").unwrap();
    for (i, line) in lines.iter().enumerate() {
        if *line == "**【标准化剧本】**" {
            active = true;
            continue;
        }
        if active && heading.is_match(line) {
            active = false;
            continue;
        }
        if !active {
            continue;
        }
        if episode_re.is_match(line) {
            episode = line;
        }
        if !line.starts_with("场景：")
            || !lines.get(i + 1).is_some_and(|l| l.starts_with("人物："))
            || !lines.get(i + 2).is_some_and(|l| l.starts_with("道具："))
        {
            continue;
        }
        let raw = lines[i + 2].strip_prefix("道具：").unwrap();
        let names = names(raw);
        let metrics = [
            ("characters", raw.chars().count(), 81),
            ("utf8Bytes", raw.len(), 243),
            (
                "jsonArrayCharacters",
                serde_json::to_string(&names).unwrap().chars().count(),
                101,
            ),
        ];
        let mut row = json!({"episode":episode,"scene":if i>0{Value::String(lines[i-1].into())}else{Value::Null},"line":i+3,"count":names.len(),"exceeded":metrics.iter().filter(|(_,n,limit)|n>limit).map(|(k,_,_)|*k).collect::<Vec<_>>()});
        for (k, n, _) in metrics {
            row[k] = json!(n);
        }
        rows.push(row);
    }
    let warnings: Vec<_> = rows
        .iter()
        .filter(|r| !r["exceeded"].as_array().unwrap().is_empty())
        .cloned()
        .collect();
    let maxima: serde_json::Map<_, _> = ["characters", "utf8Bytes", "jsonArrayCharacters"]
        .into_iter()
        .map(|k| {
            (
                k.into(),
                json!(rows.iter().filter_map(|r| r[k].as_u64()).max().unwrap_or(0)),
            )
        })
        .collect();
    json!({"ok":!rows.is_empty(),"reviewRequired":!warnings.is_empty(),"profile":"lingmo-scene-prop-observation-v2","databaseColumnLimit":"UNKNOWN","budget":{"characters":81,"utf8Bytes":243,"jsonArrayCharacters":101},"scenes":rows.len(),"maxima":maxima,"warnings":warnings,"proof":"Observed sample envelope only; exceeding it is not proof of rejection. Actual backend limit UNKNOWN."})
}
pub(super) fn export_report(
    check: &Value,
    md: &str,
    preview: &Path,
    catalog: &Path,
    verify: bool,
    image_assets: usize,
) -> Value {
    let mut report = json!({"ok":true,"fieldCapacity":field_budget(md),"testFixture":check["testFixture"],"sourceFidelity":check["sourceFidelity"],"visualBaseline":check["visualBaseline"],"assetsIndex":catalog.to_string_lossy()});
    if verify {
        report["exportProfile"] = json!("lingmo-v7");
        report["imageAssets"] = json!(image_assets);
        if let Some(season) = check.get("season") {
            report["season"] = season.clone();
        }
        report["proof"]=json!("独立反解全量可见字段一致；冻结源证据见sourceFidelity；不代表内容审核、平台导入或视频效果");
    } else {
        report["file"] = json!(preview.to_string_lossy());
        report["proof"] =
            json!("结构导出及独立回读一致；冻结源证据见sourceFidelity；v7目录由同一映射生成");
    }
    report
}
