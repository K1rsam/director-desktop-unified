use super::*;
use crate::pipeline::{digest, slice_utf16, validate_source, LockedSource};
use std::io::Write;

fn source_file(dir: &Path, stem: &str, text: &str) -> PathBuf {
    let path = dir.join(format!("{stem}.txt"));
    fs::write(&path, text.as_bytes()).unwrap();
    path
}
fn exact(source: &LockedSource, text: &str) {
    assert!(source.text == text, "原文逐字保留");
    assert_eq!(source.sha256, digest(text));
    assert_eq!(source.length, text.encode_utf16().count());
    assert!(
        source
            .rows
            .iter()
            .map(|r| r.text.as_str())
            .collect::<String>()
            == text,
        "完整行覆盖原文"
    );
    for row in &source.rows {
        assert!(
            slice_utf16(text, &row.range).unwrap() == row.text,
            "UTF-16范围逐行回读"
        );
    }
    validate_source(source).unwrap();
}
fn only_cover_changed(old: &LockedSource, new: &LockedSource) {
    assert_eq!(old.sha256, new.sha256);
    assert_eq!(old.parser_version, new.parser_version);
    assert_eq!(
        serde_json::to_value(&old.episodes).unwrap(),
        serde_json::to_value(&new.episodes).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&old.scenes).unwrap(),
        serde_json::to_value(&new.scenes).unwrap()
    );
    let title = old
        .rows
        .iter()
        .position(|row| !row.text.trim().is_empty())
        .unwrap();
    for (index, (before, after)) in old.rows.iter().zip(&new.rows).enumerate() {
        assert_eq!(before.id, after.id);
        assert_eq!(before.range, after.range);
        assert_eq!(before.text, after.text);
        if index != title {
            assert_eq!(
                serde_json::to_value(before).unwrap(),
                serde_json::to_value(after).unwrap()
            );
        }
    }
}

#[test]
fn exact_file_stem_and_first_episode_classify_only_the_cover_without_changing_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "合成图鉴：海边归来😀";
    let text=format!("\r\n{stem}\r\n\r\n第一集\r\n场景：码头\r\n甲：我回来了。\r\n第二集\r\n场景：屋内\r\n甲：继续。\r\n");
    let path = source_file(dir.path(), stem, &text);
    let old = crate::pipeline::prepare_source(&text).unwrap();
    let source = prepare_source_import(None, &path).unwrap();
    assert_eq!(source.rows[1].kind, "context");
    assert!(source.diagnostics.is_empty());
    assert_eq!(source.boundary_decisions.len(), 1);
    assert!(source.boundary_decisions[0]
        .reason
        .contains("document-title/file-stem-exact/v1"));
    assert!(source.boundary_decisions[0]
        .reason
        .contains("first-production-episode"));
    only_cover_changed(&old, &source);
    exact(&source, &text);
    assert_eq!(fs::read(&path).unwrap(), text.as_bytes());
}

#[test]
fn existing_zero_episode_context_is_preserved_before_a_true_first_production_episode() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "合成图鉴：开局归来";
    let text=format!("{stem}\n第零集\n故事大纲\n这是已有背景资料。\n人物设定\n甲：合成人物资料。\n第一集\n场景：码头\n甲：回来。\n");
    let path = source_file(dir.path(), stem, &text);
    let old = crate::pipeline::prepare_source(&text).unwrap();
    let source = prepare_source_import(None, &path).unwrap();
    assert_eq!(source.rows[0].kind, "context");
    assert!(source.rows[1..6].iter().all(|row| row.kind == "context"));
    assert_eq!(source.episodes.len(), 1);
    assert!(source.diagnostics.is_empty());
    assert!(source.boundary_decisions[0]
        .reason
        .contains("existing-zero-episode-context"));
    only_cover_changed(&old, &source);
    exact(&source, &text);
}

#[test]
fn unrelated_prefaces_dialogue_non_first_episode_and_inexact_titles_stay_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "合成标题：海边归来";
    for (first, between, heading) in [
        (stem, "甲：这句台词必须保留。\n", "第一集"),
        (stem, "未知前言必须保留。\n", "第一集"),
        (stem, "10集剧本\n", "第一集"),
        (stem, "", "第二集"),
        (stem, "第零集\n故事大纲\n合成资料\n", "第二集"),
        ("合成标题：海边归来，甲推门。", "", "第一集"),
        ("合成标题：海边归来 ", "", "第一集"),
        (" 合成标题：海边归来", "", "第一集"),
        ("甲：回来。", "", "第一集"),
    ] {
        let text = format!("{first}\n{between}{heading}\n场景：码头\n甲：继续。\n");
        let path = source_file(dir.path(), stem, &text);
        let source = prepare_source_import(None, &path).unwrap();
        assert!(source.boundary_decisions.is_empty());
        assert_eq!(source.rows[0].kind, "body");
        assert!(source
            .diagnostics
            .iter()
            .any(|issue| issue.blocking && issue.code == "BOUNDARY_UNCONFIRMED"));
        exact(&source, &text);
    }
}

#[test]
fn pasted_text_cannot_borrow_title_evidence_from_a_supplied_path() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "合成标题：海边归来";
    let text = format!("{stem}\n第一集\n场景：码头\n甲：继续。\n");
    let path = source_file(dir.path(), stem, &text);
    let expected = crate::pipeline::prepare_source(&text).unwrap();
    let pasted = prepare_source_import(Some(&text), &path).unwrap();
    assert_eq!(
        serde_json::to_value(pasted).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert!(prepare_source_import(Some(&text), Path::new("not-an-existing-file.docx")).is_ok());
}

#[test]
fn saved_decision_replays_without_the_original_file_and_old_source_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "合成标题：海边归来";
    let text = format!("{stem}\n第一集\n场景：码头\n甲：继续。\n");
    let path = source_file(dir.path(), stem, &text);
    let old = crate::pipeline::prepare_source(&text).unwrap();
    let frozen = serde_json::to_value(&old).unwrap();
    let imported = prepare_source_import(None, &path).unwrap();
    let saved: LockedSource =
        serde_json::from_value(serde_json::to_value(&imported).unwrap()).unwrap();
    fs::remove_file(&path).unwrap();
    validate_source(&saved).unwrap();
    let reparsed = crate::pipeline::reparse_source(&saved).unwrap();
    assert_eq!(
        serde_json::to_value(&reparsed).unwrap(),
        serde_json::to_value(&saved).unwrap()
    );
    assert_eq!(serde_json::to_value(old).unwrap(), frozen);
    exact(&saved, &text);
}

#[test]
fn docx_import_uses_the_actual_filename_and_keeps_extracted_text_exact() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "合成标题：归来😀";
    let path = dir.path().join(format!("{stem}.docx"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    zip.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    let xml=format!("<w:document xmlns:w=\"urn:synthetic-word\"><w:body><w:p><w:r><w:t>{stem}</w:t></w:r></w:p><w:p><w:r><w:t>第一集</w:t></w:r></w:p><w:p><w:r><w:t>场景：码头</w:t></w:r></w:p><w:p><w:r><w:t>甲：继续。</w:t></w:r></w:p></w:body></w:document>");
    zip.write_all(xml.as_bytes()).unwrap();
    zip.finish().unwrap();
    let bytes = fs::read(&path).unwrap();
    let text = read_source(&path).unwrap();
    let source = prepare_source_import(None, &path).unwrap();
    assert_eq!(source.rows[0].kind, "context");
    assert!(source.diagnostics.is_empty());
    exact(&source, &text);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}
