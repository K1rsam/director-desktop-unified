//! Opt-in v0.4.1 generation-boundary.mjs port. Never scans or edits source,
//! dialogue text, asset names or cues; only explicit generated performance fields.
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

fn list(v: &Value) -> &[Value] { v.as_array().map(Vec::as_slice).unwrap_or(&[]) }
fn instruction(text: &str) -> bool {
    static QUOTES: OnceLock<Regex> = OnceLock::new();
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    static PREFIX: OnceLock<Regex> = OnceLock::new();
    static EDIT: OnceLock<Regex> = OnceLock::new();
    let quotes = QUOTES.get_or_init(|| Regex::new("“[^”]*”|「[^」]*」|『[^』]*』|\"[^\"\\n]*\"").unwrap());
    let split = SPLIT.get_or_init(|| Regex::new("[。！？；;\\n，,]").unwrap());
    let prefix = PREFIX.get_or_init(|| Regex::new("^(?:请|需要|需|须|应|建议)\\s*").unwrap());
    let edit = EDIT.get_or_init(|| Regex::new(concat!(
        "^(?:后期(?:时间卡|字幕|叠字|标题卡)\\s*(?:〔|放|置|位于|在|采用|使用|：|:)",
        "|后期(?:添加|加上|加入|叠加|制作|插入)(?:[^。！？；;\\n]{0,24})(?:字幕|时间卡|标题卡|叠字|文字))",
        "|^(?:字幕(?:的)?(?:字号|字体|排版|字距|行距)|(?:调整|设置|修改)字幕(?:的)?(?:字号|字体|排版|字距|行距))"
    )).unwrap());
    let unquoted = quotes.replace_all(text, "〔原样文字〕");
    split.split(&unquoted).any(|part| edit.is_match(&prefix.replace(part.trim(), "")))
}
pub(super) fn check(bundle: &Value) -> Result<(), String> {
    let Some(version) = bundle.get("generationBoundaryVersion") else { return Ok(()); };
    if version != "video-prompt-v1" { return Err("GENERATION_BOUNDARY_VERSION: 未知生成正文边界策略".into()); }
    let check = |value: &Value, path: &str| -> Result<(), String> {
        if value.as_str().is_some_and(instruction) {
            return Err(format!("GENERATION_BOUNDARY: {path} 含后期字幕/时间卡或文字排版指令；编辑说明应留独立后期资料，原文和台词保留"));
        }
        Ok(())
    };
    for (ei, episode) in list(&bundle["episodes"]).iter().enumerate() {
        let ep = format!("episodes[{ei}]（第{}集）", episode["number"]);
        for key in ["visualStyle", "prohibitions"] { check(&episode["settings"][key], &format!("{ep}.settings.{key}"))?; }
        for (ui, unit) in list(&episode["draft"]["units"]).iter().enumerate() {
            let up = format!("{ep}.draft.units[{ui}]（{}）", unit["id"]);
            for key in ["unitDirection", "lighting", "initialState", "exitState"] {
                check(&unit[key], &format!("{up}.{key}"))?;
                for (si, segment) in list(&unit["segments"]).iter().enumerate() {
                    check(&segment[key], &format!("{up}.segments[{si}].{key}"))?;
                }
            }
            for (si, shot) in list(&unit["shots"]).iter().enumerate() {
                let sp = format!("{up}.shots[{si}]（{}）", shot["id"]);
                for key in ["camera", "picture", "lighting", "sound", "transition"] { check(&shot[key], &format!("{sp}.{key}"))?; }
                for (pi, part) in list(&shot["dialogueParts"]).iter().enumerate() {
                    check(&part["delivery"], &format!("{sp}.dialogueParts[{pi}].delivery"))?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn exact_editorial_fields_only_and_source_is_immutable() {
        for text in ["他询问后期制作进度。", "后期字幕组来电，他接起电话。", "牌匾写着“后期添加字幕”。", "屏幕显示：后期添加字幕。", "系统界面显示“字幕字号排版设置”。", "禁止后期添加字幕；不要修改字幕字号。", "无额外字幕、配乐、水印。"] {
            assert!(!instruction(text), "{text}");
        }
        for text in ["后期时间卡“同日夜·雨停”在左上空窗区，米白细字。", "后期添加白色字幕", "字幕字号设为24，字幕排版居中", "请后期添加字幕", "设置字幕字号为24"] { assert!(instruction(text), "{text}"); }
        let mut b = json!({"episodes":[{"number":1,"source":{"text":"后期添加字幕"},"draft":{"units":[{"id":"U1","shots":[{"id":"S1","picture":"后期添加字幕","dialogueParts":[{"text":"字幕字号排版"}]}]}]}}]});
        assert!(check(&b).is_ok()); // original frozen legacy contract
        b["generationBoundaryVersion"] = json!("video-prompt-v1");
        let before = b.clone();
        let err = check(&b).unwrap_err();
        assert!(err.contains("第1集") && err.contains("U1") && err.contains("S1") && err.contains("picture"));
        assert_eq!(b, before);
        b["episodes"][0]["draft"]["units"][0]["shots"][0]["picture"] = json!("牌匾写着“后期添加字幕”。");
        assert!(check(&b).is_ok());
        b["generationBoundaryVersion"] = json!("unknown");
        assert!(check(&b).unwrap_err().starts_with("GENERATION_BOUNDARY_VERSION:"));
    }
}
