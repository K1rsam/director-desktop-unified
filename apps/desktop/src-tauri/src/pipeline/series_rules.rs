//! Rust counterpart of series v6.7 job_packet.py's effective role view.
//! Original files stay byte-for-byte frozen; adaptations fail closed on drift.
use super::super::{digest, RuleSnapshot};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub const LEGACY_VERSION: &str = "20261025-v6.7.2-readable-image-bindings";
pub const VERSION: &str = "20261026-v6.7.3-visual-baseline";
pub const DESKTOP_VERSION: &str = "20261026-v1.1.4-visual-baseline";
const VISUAL_BASELINE: &str = "references/visual-baseline.md";
pub(super) const CONTENT_ONCE: &str = "series-rule-content-once/v1";
const CONTENT_READ: &str = "完整规则正文均在 bodies 中，按 roleSets.repair 的 primary 再 methods 原顺序读取；roleSets.origin 保留原岗位的 primary/methods 顺序与职责关联。每个描述符的 contentRef 与 sha256 指向同一正文，正文只存一份但两组规则全部有效；不能以引用、哈希或短卡代替读取正文。";
const COMMON: &str = "references/creative/references/common-contract.md";
const SOURCE: &str = "references/creative/skills/source-context-preparer/SKILL.md";
const AUTHOR: &str = "references/creative/skills/unit-performance-director/SKILL.md";
const FINISHER: &str = "references/creative/skills/episode-finishing-editor/SKILL.md";
const REVIEWER: &str = "stages/review-delivery/SKILL.md";
macro_rules! rule {
    ($path:literal) => {
        (
            $path,
            include_str!(concat!(
                "../../../../../rules/series/20261025-v6.7.2/",
                $path
            )),
        )
    };
}
macro_rules! current_rule {
    ($path:literal) => {
        (
            $path,
            include_str!(concat!(
                "../../../../../rules/series/20261026-v6.7.3/",
                $path
            )),
        )
    };
}
macro_rules! desktop_rule {
    ($path:literal) => {
        (
            $path,
            include_str!(concat!(
                "../../../../../rules/desktop/20261026-v1.1.4/",
                $path
            )),
        )
    };
}
fn files_for(version: &str) -> Result<Vec<(&'static str, &'static str)>, String> {
    match version {
        LEGACY_VERSION => Ok(legacy_files()),
        VERSION => Ok(current_files()),
        _ => Err("SERIES_RULE_VERSION_UNSUPPORTED".into()),
    }
}
fn source_identity<'a>(version: &'a str, path: &str) -> (&'static str, &'a str) {
    if version == VERSION
        && matches!(
            path,
            COMMON
                | AUTHOR
                | FINISHER
                | "references/creative/skills/episode-emotion-planner/SKILL.md"
                | "references/creative/skills/sound-rhythm-methods/SKILL.md"
                | "references/creative/skills/director-refinement/SKILL.md"
        )
    {
        ("desktop", DESKTOP_VERSION)
    } else {
        ("series", version)
    }
}
fn legacy_files() -> Vec<(&'static str, &'static str)> {
    vec![
        rule!("references/creative/references/common-contract.md"),
        rule!("references/production-contract.md"),
        rule!("references/data-contract.md"),
        rule!("references/lingmo-v7-contract.md"),
        rule!("references/expression-and-screen-text.md"),
        rule!("references/creative/skills/unit-performance-director/SKILL.md"),
        rule!("references/creative/skills/episode-finishing-editor/SKILL.md"),
        rule!("stages/review-delivery/SKILL.md"),
        rule!("references/creative/skills/source-context-preparer/SKILL.md"),
        rule!("references/creative/skills/episode-emotion-planner/SKILL.md"),
        rule!("references/creative/skills/acting-blocking-methods/SKILL.md"),
        rule!("references/creative/skills/camera-emotion-methods/SKILL.md"),
        rule!("references/creative/skills/sound-rhythm-methods/SKILL.md"),
        rule!("references/creative/skills/lighting-space-methods/SKILL.md"),
        rule!("references/creative/skills/fantasy-expression-methods/SKILL.md"),
        rule!("references/creative/skills/director-refinement/SKILL.md"),
        rule!("references/shot-handoffs.md"),
        rule!("references/system-presentation.md"),
        rule!("references/asset-registry-contract.md"),
        rule!("stages/asset-design/SKILL.md"),
        rule!("references/preproduction-contract.md"),
        rule!("references/asset-handoff.md"),
        rule!("references/lingmo-field-budget.md"),
    ]
}
fn current_files() -> Vec<(&'static str, &'static str)> {
    vec![
        current_rule!("references/visual-baseline.md"),
        desktop_rule!("references/creative/references/common-contract.md"),
        current_rule!("references/production-contract.md"),
        current_rule!("references/data-contract.md"),
        current_rule!("references/lingmo-v7-contract.md"),
        current_rule!("references/expression-and-screen-text.md"),
        desktop_rule!("references/creative/skills/unit-performance-director/SKILL.md"),
        desktop_rule!("references/creative/skills/episode-finishing-editor/SKILL.md"),
        current_rule!("stages/review-delivery/SKILL.md"),
        current_rule!("references/creative/skills/source-context-preparer/SKILL.md"),
        desktop_rule!("references/creative/skills/episode-emotion-planner/SKILL.md"),
        current_rule!("references/creative/skills/acting-blocking-methods/SKILL.md"),
        current_rule!("references/creative/skills/camera-emotion-methods/SKILL.md"),
        desktop_rule!("references/creative/skills/sound-rhythm-methods/SKILL.md"),
        current_rule!("references/creative/skills/lighting-space-methods/SKILL.md"),
        current_rule!("references/creative/skills/fantasy-expression-methods/SKILL.md"),
        desktop_rule!("references/creative/skills/director-refinement/SKILL.md"),
        current_rule!("references/shot-handoffs.md"),
        current_rule!("references/system-presentation.md"),
        current_rule!("references/asset-registry-contract.md"),
        current_rule!("stages/asset-design/SKILL.md"),
        current_rule!("references/preproduction-contract.md"),
        current_rule!("references/asset-handoff.md"),
        current_rule!("references/lingmo-field-budget.md"),
    ]
}
pub fn snapshots() -> Result<Vec<RuleSnapshot>, String> {
    snapshots_for(VERSION)
}
pub fn snapshots_for(version: &str) -> Result<Vec<RuleSnapshot>, String> {
    let installed = match version {
        LEGACY_VERSION => include_str!("../../../../../rules/series/20261025-v6.7.2/VERSION"),
        VERSION => include_str!("../../../../../rules/series/20261026-v6.7.3/VERSION"),
        _ => return Err("SERIES_RULE_VERSION_UNSUPPORTED".into()),
    };
    if installed.trim() != version
        || (version == VERSION
            && include_str!("../../../../../rules/desktop/20261026-v1.1.4/VERSION").trim()
                != DESKTOP_VERSION)
    {
        return Err("SERIES_RULE_VERSION_MISMATCH".into());
    }
    Ok(files_for(version)?
        .into_iter()
        .map(|(path, body)| {
            let (origin, source_version) = source_identity(version, path);
            RuleSnapshot {
                id: format!("{origin}/{path}"),
                version: source_version.into(),
                sha256: digest(body),
                content: body.into(),
            }
        })
        .collect())
}
pub fn pack_hash() -> Result<String, String> {
    pack_hash_for(VERSION)
}
pub fn pack_hash_for(version: &str) -> Result<String, String> {
    let manifest: Vec<_> = snapshots_for(version)?
        .into_iter()
        .map(|r| json!({"id":r.id,"sha256":r.sha256,"version":r.version}))
        .collect();
    Ok(digest(&json!(manifest).to_string()))
}
fn replace_once(
    body: &mut String,
    before: &str,
    after: &str,
    id: &str,
    changes: &mut Vec<String>,
) -> Result<(), String> {
    if body.matches(before).count() != 1 {
        return Err(format!("SERIES_RULE_ADAPTER_DRIFT:{id}"));
    }
    *body = body.replacen(before, after, 1);
    changes.push(id.into());
    Ok(())
}
fn sections(body: &str, wanted: &[&str]) -> Result<String, String> {
    let mut headings = std::collections::BTreeMap::<String, String>::new();
    let mut current: Option<String> = None;
    for line in body.split_inclusive('\n') {
        if let Some(heading) = line.strip_prefix("## ") {
            let name = heading.trim().to_owned();
            if headings.insert(name.clone(), String::new()).is_some() {
                return Err("SERIES_DUPLICATE_RULE_SECTION".into());
            }
            current = Some(name);
        }
        if let Some(name) = &current {
            headings.get_mut(name).unwrap().push_str(line);
        }
    }
    wanted
        .iter()
        .map(|name| {
            headings
                .get(*name)
                .cloned()
                .ok_or_else(|| format!("SERIES_RULE_SECTION_MISSING:{name}"))
        })
        .collect()
}
fn effective(path: &str, raw: &str) -> Result<(String, Vec<String>), String> {
    let mut body = raw.to_owned();
    let mut changes = Vec::new();
    if path == SOURCE {
        replace_once(&mut body,
            "上游标准结构、全剧方向、连续性、目录与已采用美术是可选辅助，有则核对来源版本、对象和适用范围后择需使用；没有则依据本集原文和实际提供的前情自行准备必要事实与最小一致文字设计，不等待资料齐套。",
            "本岗位包已提供本集受核源、有效方向、时点事实、完整必要采用设计与实际入口，核对来源版本、对象和适用范围后使用；发现真正缺失的必要依赖时标明具体缺口交主控补齐，不自行改造已采用设计。无关辅助资料无需等待。",
            "required-input-context", &mut changes)?;
    }
    if path == COMMON {
        replace_once(&mut body,
            "每个单元的镜长合计落在整数秒，单镜仍可小数；用成立的切点和相位排定，不四舍五入显示掩盖实际小数，也不催读、拖动作或补空镜凑整。",
            "v7 内部镜长与单元实际总长保留精确小数；外层容器取 max(4,ceil(实际总长))，内部实际总长和外层容器均不得超过项目15/30秒上限。不改成立的切点与相位，不催读、拖动作或补空镜凑整。",
            "common-unit-end", &mut changes)?;
    }
    if path == FINISHER {
        replace_once(&mut body,
            "整数终点只在相关单元内选成立的切点/相位，单镜可小数；不改已好的邻段凑整，不以新的暂停或付费返修处理小数格式。若整数终点仍与真实声画冲突，明确保留待核坐标，不能假称整数交付。",
            "v7 内部保留精确小数终点，外层容器取 max(4,ceil(实际总长))；内部与容器均守项目15/30秒上限。不改已好的邻段凑整，不以新的暂停或付费返修处理合法小数。",
            "finisher-unit-end", &mut changes)?;
        replace_once(
            &mut body,
            "最终按成立的相位排整数终点，已收束则不填到26秒",
            "最终保留成立相位的精确小数终点，由外层容器取整，已收束则不填到26秒",
            "finisher-example-end",
            &mut changes,
        )?;
    }
    if path == "references/creative/skills/director-refinement/SKILL.md" {
        replace_once(&mut body, "单元整数终点由真实动作/声画调整，内镜允许小数秒，不靠等待/吞字凑整。", "v7 保留内部镜长和单元实际总长的精确小数，外层容器取 max(4,ceil(实际总长))；二者均守项目15/30秒上限，不靠等待/吞字凑整。", "refinement-unit-end", &mut changes)?;
    }
    let wanted: &[&str] = match path {
        "references/production-contract.md" => &[
            "新任务的完整计划",
            "幻想候选、模式与选择",
            "整编、精修与采用",
        ],
        "references/data-contract.md" => {
            &["Bundle 根与原文", "后期文字交接", "场、单元、镜头与人声"]
        }
        "references/lingmo-v7-contract.md" => &[
            "1. 创作范围与兼容边界",
            "2. 内部身份、图片引用与安全名",
            "3. 正文结构与字段保留",
            "4. 精确时间与外层容器",
        ],
        REVIEWER => &["内容双向复核"],
        _ => &[],
    };
    if !wanted.is_empty() {
        body = sections(&body, wanted)?;
        changes.extend(wanted.iter().map(|s| format!("section:{s}")));
    }
    Ok((body, changes))
}
/// Adapt only the new portable desktop source to this executable's contracts.
/// Legacy adaptations and serialized role packages must remain byte-for-byte stable.
fn effective_for(version: &str, path: &str, raw: &str) -> Result<(String, Vec<String>), String> {
    let (mut body, mut changes) = effective(path, raw)?;
    if version == LEGACY_VERSION {
        return Ok((body, changes));
    }
    if version != VERSION {
        return Err("SERIES_RULE_VERSION_UNSUPPORTED".into());
    }
    if path == VISUAL_BASELINE {
        body = sections(&body, &["前期采用", "岗位与输出", "恢复与已有稿"])?;
        changes.push("visual-baseline-creative-sections".into());
        body.push_str("\n本 Rust 执行器已在岗位输入提供采用基准；按当前岗位 outputSchema 交付，不新增该岗位未声明的 settings 或外部工具指令。正文完整保留基准，由运行器写入设置并核对；不能自行采用、替换基准或修改旧冻结任务。\n");
        changes.push("rust-output-schema-authority".into());
    }
    if path == COMMON {
        replace_once(&mut body,
            "桌面源绑定声音事件仅将经核源的VO非人声描述演绎到sound，原quote/声道/区间完整留结构化稿与源锁追溯；不朗读事件描述，也不能把旁白、OS、系统音、兽吼或尖叫等实际发声改成事件省略。",
            "声音演绎保留源声型与实际发声；本运行器未启用独立声音事件豁免，不能通过改成声音事件绕过原声覆盖。源标注有歧义时保留原句、说者、声道及源坐标，交当前审核流程核对，不自创声音事件字段或删减原声。",
            "rust-no-sound-event-exemption", &mut changes)?;
    }
    if path == AUTHOR {
        replace_once(&mut body,
            "新原声按规范化quote相对UTF-16区间交materialize-dialogue取回逐字text；长句跨镜保持同一声流，不把切镜写成气口。源VO中的明确非人声事件经核源后按soundEventDecisions合同演绎，保留源句/区间而不朗读描述；真实人声不得改成事件。源/事实问题与普通演法选择分开处理，不因工具缺字段越过原文。",
            "本 Rust 运行器的原声坐标使用 source-absolute-utf16/v1：dialogueParts 的 lineId 对应 sourceRows，quoteStart/quoteEnd 指向冻结完整源的绝对 UTF-16 半开区间，直接从该行 spokenSegments.range 选取连续子区间，不跨越不发声演法；逐字 text、说者和声道由原声物化器恢复。不使用 quote 相对坐标，不另调用外部物化工具。长句跨镜保持同一声流，不把切镜写成气口。实际发声保持原声覆盖，不以未启用的声音事件字段省略源句；源/事实疑义交既有审核处理，普通演法自行完成。",
            "rust-absolute-dialogue-coordinates", &mut changes)?;
    }
    if path == FINISHER {
        replace_once(
            &mut body,
            "新生成文本按 `generationTextVersion:\"voice-delivery-v1\"` 执行：",
            "本岗按现有 outputSchema 保留原声与演绎：",
            "rust-finisher-output-shape",
            &mut changes,
        )?;
        replace_once(&mut body,
            "先完整读稿再看editingSignals的原句跨镜断口、声窗外段、长镜与尾段坐标；信号不判断空等、不设镜长配额，不另开评分岗。",
            "先完整读稿，再核对输入中实际提供的原句跨镜断口、声窗外段、长镜与尾段坐标；诊断只定位，不判断空等、不设镜长配额，不另开评分岗，也不假设存在未提供的信号字段。",
            "rust-finisher-evidence", &mut changes)?;
        replace_once(&mut body,
            "默认只提交实际修改：既有picture/camera/sound/声窗等字段用desktop-field-patch-v1，结构拆合用desktop-draft-delta-v1完整关联组，由apply-delta生成完整候选并复制正确邻镜。完整阅读不要求整集重抄，工具成功也不等于内容采用；联修实际受影响的原句、幻想、接触与出口。有效零改动可结束，整编已修好的内容不自动再做一轮精修。接口见[本地适配](../../../desktop-adapter.md#差量合并)。",
            "只改变有实际改善的内容，保留正确邻镜；提交本次 outputSchema 要求的完整候选，不使用便携脚本的 field-patch/delta 或 apply-delta。联修实际受影响的原句、幻想、接触与出口，工具通过不等于内容采用。完整阅读后确无有效修改可保留原稿，已修好的内容不自动再做一轮精修。",
            "rust-finisher-complete-candidate", &mut changes)?;
        replace_once(
            &mut body,
            "先对当前保存稿运行inspect_editing，读cut_pairs的实际两镜，",
            "从岗位包中的当前完整稿按镜序配对读取实际切出的镜头与下一切入镜头，",
            "rust-finisher-direct-shot-read",
            &mut changes,
        )?;
    }
    if path == "references/creative/skills/sound-rhythm-methods/SKILL.md" {
        replace_once(&mut body,
            "本段delivery写法针对实际发声；已核源的非人声声音事件按桌面soundEventDecisions合同写事件演绎与发生窗，导出到声音栏，不套人声语速或把描述读成台词。",
            "delivery针对实际发声，环境与动作声按本岗声音栏表达；本运行器未启用独立声音事件豁免，不添加 soundEventDecisions 或用声音栏替代必须保留的原声覆盖。",
            "rust-sound-schema", &mut changes)?;
    }
    if path == "references/creative/skills/director-refinement/SKILL.md" {
        replace_once(&mut body,
            "桌面版默认使用[本地差量工具](../../../desktop-adapter.md#差量合并)：少量既有演出/声窗字段用desktop-field-patch-v1；拆合镜头或单元用desktop-draft-delta-v1完整关联组。基稿hash与before匹配后由程序复制未改内容，模型不重抄正确邻镜；不是原桌面数据库协议。超出局部字段能力的计划/选择更新走完整候选合同，不能放宽源、身份或许可。已验证的旧任务交换格式可继续，不为换规则重跑旧稿。仅交差量不算采用，仍核完整结果。",
            "本 Rust 岗位按冻结基稿和当前 outputSchema 交完整修订候选，仅改变真实关联范围并保留未改正文；不输出便携脚本的 field-patch/delta，不放宽源、身份或许可。旧任务继续使用原冻结规则与交换格式，不为升级规则重跑旧稿。仍审核完整结果，机械通过不算内容采用。",
            "rust-repair-complete-candidate", &mut changes)?;
    }
    if path == REVIEWER {
        replace_once(
            &mut body,
            "读取当前采用版inspect_editing的cut_pairs（同哈希可复用），",
            "从绑定当前稿哈希的完整候选中按镜序读取每对切出与切入镜头，",
            "rust-review-direct-shot-read",
            &mut changes,
        )?;
    }
    if path == "references/shot-handoffs.md" {
        replace_once(&mut body,
            "对保存的当前draft或bundle运行[inspect_editing](tools.md)；按input_sha256绑定版本，直接读cut_pairs内的两镜原文及跨单元头。",
            "从本岗已绑定当前稿哈希的完整 draft 中按镜序读取实际相邻两镜原文及跨单元头；不要求额外运行便携脚本或提供未实现的取证字段。",
            "rust-shot-handoff-direct-read", &mut changes)?;
    }
    if path == "references/data-contract.md" {
        replace_once(&mut body,
            "- `generationTextVersion:\"voice-delivery-v1\"`：新生产必填，原始cue目录只供核源；演员必要提示由作者写入已有delivery/picture/sound，后期说明独立交付。",
            "- 原始演法提示供核源；演员必要提示由作者写入当前 schema 的 delivery/picture/sound。便携版的 generationTextVersion 根字段由对应导出器管理，本 Rust 岗位不额外输出 schema 未声明的字段。",
            "rust-generation-text-schema", &mut changes)?;
        replace_once(&mut body,
            "- `sourceLockHash`：创作前freeze-source返回的hash；有锁的新稿必须同时声明上述生成文本策略。",
            "- 冻结源及其 hash 由本 Rust 接入方供给，模型不得重写源、重新冻结或自造源锁；返回形状以当前 outputSchema 为准。",
            "rust-source-lock-ownership", &mut changes)?;
        replace_once(&mut body,
            "构稿前先把单集source对象另存为SOURCE.json，执行 `node scripts/storyboard.mjs inspect-source --input /absolute/SOURCE.json`；stdout直接返回原版normalizeSource的完整结果，含sourceRows、dialogue、diagnostics及每句id、speaker、quote、channel、cue、kind。此命令无需Bundle或draft，不创作、不写文件，诊断项原样返回供接入补证据。自动D编号是本集整份对白目录的顺序号，不是每原文行都从D1开始：例如第一句E01:R4:D1、第二句E01:R6:D2。实际id必须从inspect结果复制，不能按行号猜；标准协议或已有显式目录以返回结果为准。对白始终输出source里的原说者、原声道、逐字文本。资产正式状态名不同于原说者时使用下面的显式voiceBindings。",
            "本 Rust 岗位已收到冻结源及 sourceRows、原声区间和相关诊断。lineId 必须复制当前供料的真实 id，不重编号、不猜便携版 D 编号，也不调用外部 inspect-source。dialogueParts 依当前运输合同选择 quoteStart/quoteEnd 的完整源绝对 UTF-16 半开区间；Rust 恢复原说者、原声道和逐字文本。资产状态名不能替代源说者，必要声音身份绑定按本岗 schema 提供。",
            "rust-source-read-and-coordinate-contract", &mut changes)?;
    }
    if path == "references/production-contract.md" {
        replace_once(
            &mut body,
            "可用inspect_editing定位完全相同镜级字段与跨镜长句",
            "直接对照当前完整稿定位完全相同镜级字段与跨镜长句",
            "rust-production-direct-read",
            &mut changes,
        )?;
    }
    if path == "references/asset-registry-contract.md" {
        replace_once(&mut body,
            "仅用包内 `scripts/assets.mjs` 写总库，不手改 `current.json` 或 `v*.json`。初始输入：",
            "本 Rust 岗位不直接写总库，按当前 outputSchema 返回设计或补项，由运行器核对身份、版本与采用记录后持久化；不调用便携脚本，也不手改数据库或历史文件。以下便携记录例子说明设计和身份关系，不能替代当前岗位 schema：",
            "rust-asset-registry-ownership", &mut changes)?;
        for (command, explanation, id) in [
            ("```sh\nnode scripts/assets.mjs prepare-import --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/asset-import_v1.json\n```",
             "安全导入映射由 Rust 导出流程管理；岗位只按当前输入核对真实映射，不执行独立迁移命令。", "rust-asset-import-command"),
            ("```sh\nnode scripts/assets.mjs init --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/assets-baseline.json\nnode scripts/assets.mjs apply --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/asset-request.json\nnode scripts/assets.mjs sync --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/bundle_v2.json --output /absolute/JOB/ep1/asset-consumption_v2.json\n```",
             "资产初始化、补项采用和消费核对由 Rust 运行器负责；本岗仅提交当前 schema 的内容候选，不能自行声称已持久化或已采用。", "rust-asset-mutation-commands"),
            ("```sh\nnode scripts/assets.mjs amend --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/asset-request-amend.json\n```",
             "实际使用范围的变动交当前 Rust 校验与采用链处理，保留旧使用记录，不调用独立 amend 命令。", "rust-asset-amend-command"),
        ] {
            replace_once(&mut body, command, explanation, id, &mut changes)?;
        }
    }
    if path == "references/lingmo-field-budget.md" {
        replace_once(
            &mut body,
            "运行 `node scripts/lingmo-field-budget.mjs FINAL.md`，记录",
            "对当前实际导出文件核对字段观测记录，不在模型岗位调用外部脚本；观测需覆盖",
            "rust-field-budget-no-external-command",
            &mut changes,
        )?;
    }
    Ok((body, changes))
}
pub fn role(role: &str, fantasy: bool) -> Result<Value, String> {
    role_for(VERSION, role, fantasy)
}
pub fn role_for(version: &str, role: &str, fantasy: bool) -> Result<Value, String> {
    let mut paths = vec![
        COMMON,
        "references/production-contract.md",
        "references/data-contract.md",
        "references/lingmo-v7-contract.md",
        "references/expression-and-screen-text.md",
    ];
    let primary = match role {
        "author" => AUTHOR,
        "finisher" | "repair" => FINISHER,
        "reviewer" => REVIEWER,
        "design" | "design-review" => "stages/asset-design/SKILL.md",
        _ => return Err("SERIES_ROLE_UNKNOWN".into()),
    };
    if matches!(role, "design" | "design-review") {
        paths = vec![
            primary,
            "references/asset-registry-contract.md",
            "references/preproduction-contract.md",
            "references/system-presentation.md",
            "references/asset-handoff.md",
        ];
    } else {
        paths.push(primary);
        if role == "author" {
            paths.extend([
                SOURCE,
                "references/creative/skills/episode-emotion-planner/SKILL.md",
            ]);
        }
        if matches!(role, "finisher" | "repair" | "reviewer") {
            paths.push("references/shot-handoffs.md");
        }
        paths.extend([
            "references/creative/skills/acting-blocking-methods/SKILL.md",
            "references/creative/skills/camera-emotion-methods/SKILL.md",
            "references/creative/skills/sound-rhythm-methods/SKILL.md",
            "references/creative/skills/lighting-space-methods/SKILL.md",
            "references/system-presentation.md",
            "references/asset-registry-contract.md",
            "references/lingmo-field-budget.md",
        ]);
        if fantasy {
            paths.push("references/creative/skills/fantasy-expression-methods/SKILL.md");
        }
        if role == "repair" {
            paths.push("references/creative/skills/director-refinement/SKILL.md");
        }
    }
    if version == VERSION {
        paths.push(VISUAL_BASELINE);
    }
    let all = files_for(version)?;
    let mut seen = HashSet::new();
    let mut rules = Vec::new();
    for path in paths {
        if !seen.insert(path) {
            continue;
        }
        let raw = all
            .iter()
            .find(|(p, _)| *p == path)
            .ok_or("SERIES_RULE_MISSING")?
            .1;
        let (content, adjustments) = effective_for(version, path, raw)?;
        let (origin, source_version) = source_identity(version, path);
        rules.push(json!({"id":format!("{origin}/{path}"),"path":path,"version":source_version,"sourceSha256":digest(raw),"sha256":digest(&content),"adjustments":adjustments,"content":content}));
    }
    let p = rules
        .iter()
        .position(|r| r["path"] == primary)
        .ok_or("SERIES_PRIMARY_MISSING")?;
    let primary_rule = rules.remove(p);
    let mut result = json!({"primary":primary_rule,"methods":rules,"creativePack":{"version":"desktop-series-role-rules/v1","skillVersion":version,"packSha256":pack_hash_for(version)?,"role":role,"adapterVersion":if version == LEGACY_VERSION { "series672-rust-role-rules/v1" } else { "series673-desktop114-rust-role-rules/v1" }}});
    if version == VERSION {
        result["creativePack"]["desktopSkillVersion"] = json!(DESKTOP_VERSION);
    }
    Ok(result)
}

/// Only a newly frozen workflow opts into this representation. Every complete
/// rule body remains present; role descriptions and method order stay separate.
pub(super) fn materialize_repair(original: &Value) -> Result<Value, String> {
    fn one(rule: &Value, bodies: &mut Map<String, Value>) -> Result<Value, String> {
        let mut descriptor = rule
            .as_object()
            .cloned()
            .ok_or("SERIES_RULE_DESCRIPTOR_REQUIRED")?;
        let body = descriptor
            .remove("content")
            .ok_or("SERIES_RULE_BODY_REQUIRED")?;
        let sha = descriptor
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or("SERIES_RULE_DIGEST_REQUIRED")?
            .to_owned();
        if descriptor.contains_key("contentRef")
            || body.as_str().is_none_or(|text| digest(text) != sha)
        {
            return Err("SERIES_RULE_BODY_DIGEST_MISMATCH".into());
        }
        if bodies.get(&sha).is_some_and(|old| old != &body) {
            return Err("SERIES_RULE_BODY_COLLISION".into());
        }
        bodies.insert(sha.clone(), body);
        descriptor.insert("contentRef".into(), json!(sha));
        Ok(Value::Object(descriptor))
    }
    fn role_set(value: &Value, bodies: &mut Map<String, Value>) -> Result<Value, String> {
        let mut value = value.clone();
        value["primary"] = one(&value["primary"], bodies)?;
        let methods = value["methods"]
            .as_array()
            .ok_or("SERIES_RULE_METHODS_REQUIRED")?
            .iter()
            .map(|r| one(r, bodies))
            .collect::<Result<Vec<_>, _>>()?;
        value["methods"] = json!(methods);
        Ok(value)
    }
    let mut base = original.clone();
    let origin = base
        .as_object_mut()
        .ok_or("SERIES_RULE_SET_REQUIRED")?
        .remove("originRules");
    let mut bodies = Map::new();
    let mut role_sets = Map::new();
    role_sets.insert("repair".into(), role_set(&base, &mut bodies)?);
    if let Some(origin) = origin {
        role_sets.insert("origin".into(), role_set(&origin, &mut bodies)?);
    }
    let materialized = json!({"version":CONTENT_ONCE,"readContract":CONTENT_READ,"bodies":bodies,"roleSets":role_sets});
    if restore_repair(&materialized)? != *original {
        return Err("SERIES_RULE_MATERIALIZATION_ROUNDTRIP".into());
    }
    Ok(materialized)
}
pub(super) fn restore_repair(value: &Value) -> Result<Value, String> {
    if value["version"] != CONTENT_ONCE {
        if value.get("bodies").is_some() || value.get("roleSets").is_some() {
            return Err("SERIES_RULE_MATERIALIZATION_VERSION".into());
        }
        return Ok(value.clone());
    }
    if value.as_object().is_none_or(|v| v.len() != 4) || value["readContract"] != CONTENT_READ {
        return Err("SERIES_RULE_MATERIALIZATION_SHAPE".into());
    }
    let bodies = value["bodies"]
        .as_object()
        .ok_or("SERIES_RULE_BODIES_REQUIRED")?;
    let sets = value["roleSets"]
        .as_object()
        .ok_or("SERIES_RULE_SETS_REQUIRED")?;
    if !sets.contains_key("repair")
        || sets
            .keys()
            .any(|k| !matches!(k.as_str(), "repair" | "origin"))
    {
        return Err("SERIES_RULE_ROLE_SET_CHANGED".into());
    }
    fn one(
        rule: &Value,
        bodies: &Map<String, Value>,
        used: &mut HashSet<String>,
    ) -> Result<Value, String> {
        let mut descriptor = rule
            .as_object()
            .cloned()
            .ok_or("SERIES_RULE_DESCRIPTOR_REQUIRED")?;
        let reference = descriptor
            .remove("contentRef")
            .ok_or("SERIES_RULE_REFERENCE_REQUIRED")?;
        let sha = reference.as_str().ok_or("SERIES_RULE_REFERENCE_INVALID")?;
        let body = bodies.get(sha).ok_or("SERIES_RULE_BODY_MISSING")?;
        if descriptor.contains_key("content")
            || descriptor.get("sha256") != Some(&reference)
            || body.as_str().is_none_or(|text| digest(text) != sha)
        {
            return Err("SERIES_RULE_BODY_DIGEST_MISMATCH".into());
        }
        used.insert(sha.to_owned());
        descriptor.insert("content".into(), body.clone());
        Ok(Value::Object(descriptor))
    }
    fn role_set(
        value: &Value,
        bodies: &Map<String, Value>,
        used: &mut HashSet<String>,
    ) -> Result<Value, String> {
        let mut value = value.clone();
        value["primary"] = one(&value["primary"], bodies, used)?;
        value["methods"] = json!(value["methods"]
            .as_array()
            .ok_or("SERIES_RULE_METHODS_REQUIRED")?
            .iter()
            .map(|r| one(r, bodies, used))
            .collect::<Result<Vec<_>, _>>()?);
        Ok(value)
    }
    let mut used = HashSet::new();
    let mut restored = role_set(&sets["repair"], bodies, &mut used)?;
    if let Some(origin) = sets.get("origin") {
        restored["originRules"] = role_set(origin, bodies, &mut used)?;
    }
    if used.len() != bodies.len() {
        return Err("SERIES_RULE_UNREFERENCED_BODY".into());
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_roles_and_pack_remain_byte_exact_after_upgrade() {
        assert_eq!(
            pack_hash_for(LEGACY_VERSION).unwrap(),
            "4296ed0ef02940e2873291781b78f77e12a0adcb23e25d7e6e3a9fa54d1ab4ee"
        );
        for (name, fantasy, expected) in [
            (
                "author",
                false,
                "a527248b92ecc5bd1216e1668ab08a39ae676170ff57e60c283a9bd3774d6613",
            ),
            (
                "author",
                true,
                "92e6287036e15abed3ed5a158df7473534dd7cbaaf134e0bac3505b8f129cee5",
            ),
            (
                "design-review",
                false,
                "0d02552c7b6e7d8291d12de42830bc89beed0bd65f3353798925a5289bcbfefe",
            ),
            (
                "design-review",
                true,
                "0d02552c7b6e7d8291d12de42830bc89beed0bd65f3353798925a5289bcbfefe",
            ),
            (
                "design",
                false,
                "cf6c1b6a4088efd7959a85d7b17add01b58432be555ba40780dbd2a328719ee6",
            ),
            (
                "design",
                true,
                "cf6c1b6a4088efd7959a85d7b17add01b58432be555ba40780dbd2a328719ee6",
            ),
            (
                "finisher",
                false,
                "c1f871fcff40b181bbab1cfa501a8c75cf1d113fd460ce0b2877244ac5ac9f73",
            ),
            (
                "finisher",
                true,
                "b155221c40bf4eea14e51213350eeadbac32549256eaa444641cdc3d3a4b7051",
            ),
            (
                "repair",
                false,
                "c189145ba2a87bc5b708355023226cd12d3f09e8925a523659f68b6c1974ce82",
            ),
            (
                "repair",
                true,
                "85d12088073ff54e7ada8337ed0e70595f91d8f52bc11d4f6a7d73eecc2fe33b",
            ),
            (
                "reviewer",
                false,
                "917936bcb9f6ce6d026147a7c4f099188521a60cfcea5b631607d4f88cb9e870",
            ),
            (
                "reviewer",
                true,
                "1322ff7f4f5629367828ce5411eb1c20655ddf181df5d40f8de42af1ee6e3ba0",
            ),
        ] {
            let value = role_for(LEGACY_VERSION, name, fantasy).unwrap();
            assert_eq!(digest(&value.to_string()), expected, "{name}/{fantasy}");
            assert!(!value.to_string().contains("series-visual-baseline-v1"));
        }
    }
    #[test]
    fn latest_roles_bind_actual_sources_baseline_and_rust_contracts() {
        assert_ne!(
            pack_hash_for(VERSION).unwrap(),
            pack_hash_for(LEGACY_VERSION).unwrap()
        );
        for name in [
            "author",
            "finisher",
            "repair",
            "reviewer",
            "design",
            "design-review",
        ] {
            for fantasy in [false, true] {
                let value = role_for(VERSION, name, fantasy).unwrap();
                assert_eq!(
                    value["creativePack"]["desktopSkillVersion"],
                    DESKTOP_VERSION
                );
                let text = value.to_string();
                assert!(text.contains("series-visual-baseline-v1"), "{name}");
                assert!(!text.contains("desktop-field-patch-v1"));
                assert!(!text.contains("desktop-draft-delta-v1"));
                assert!(!text.contains("规范化quote相对UTF-16"));
                assert!(!text.contains("sourceStart/sourceEnd"));
                assert!(!text.contains("运行inspect_editing"));
                assert!(!text.contains("inspect_editing的cut_pairs"));
                assert!(!text.contains("node scripts/"));
                assert!(!text.contains("inspect_editing"));
                assert!(!text.contains("每个单元的镜长合计落在整数秒"));
                for rule in
                    std::iter::once(&value["primary"]).chain(value["methods"].as_array().unwrap())
                {
                    let snapshot = snapshots()
                        .unwrap()
                        .into_iter()
                        .find(|r| r.id == rule["id"])
                        .unwrap();
                    assert_eq!(snapshot.version, rule["version"]);
                    assert_eq!(snapshot.sha256, rule["sourceSha256"]);
                    assert_eq!(digest(rule["content"].as_str().unwrap()), rule["sha256"]);
                }
            }
        }
        let author = role("author", false).unwrap();
        assert_eq!(author["primary"]["version"], DESKTOP_VERSION);
        assert!(author["primary"]["content"]
            .as_str()
            .unwrap()
            .contains("source-absolute-utf16/v1"));
        assert!(author["primary"]["content"]
            .as_str()
            .unwrap()
            .contains("quoteStart/quoteEnd"));
        assert!(role_for("unknown", "author", false).is_err());
        assert!(snapshots_for("unknown").is_err());
        assert!(pack_hash_for("unknown").is_err());
    }
    #[test]
    fn effective_roles_preserve_full_rules_and_replace_only_known_v7_clauses() {
        for name in [
            "author",
            "finisher",
            "reviewer",
            "design",
            "design-review",
            "repair",
        ] {
            let loaded = role(name, true).unwrap();
            assert_eq!(loaded["creativePack"]["skillVersion"], VERSION);
            let all = loaded.to_string();
            assert!(!all.contains("mac-3008-compatible-rust"));
            assert!(!all.contains("每个单元的镜长合计落在整数秒"));
        }
        let a = role("author", true).unwrap();
        assert!(a.to_string().contains("完整必要采用设计"));
        assert!(a.to_string().contains("episode-emotion-planner"));
        let reviewer = role("reviewer", false).unwrap();
        assert!(reviewer["primary"]["content"]
            .as_str()
            .unwrap()
            .starts_with("## 内容双向复核"));
        assert!(snapshots().unwrap().len() > 20);
    }
    #[test]
    fn source_adapter_drift_fails_closed() {
        assert!(effective(SOURCE, "changed upstream clause").is_err());
    }
    #[test]
    fn pooled_rules_preserve_every_descriptor_role_order_creative_pack_and_full_body() {
        for origin in ["author", "finisher", "reviewer", "design", "design-review"] {
            for fantasy in [false, true] {
                let original = super::super::repair_rules(origin, fantasy).unwrap();
                let pooled = materialize_repair(&original).unwrap();
                assert_eq!(restore_repair(&pooled).unwrap(), original);
                let mut expected = Map::new();
                let mut occurrences = 0;
                for set in [&original, &original["originRules"]] {
                    for rule in
                        std::iter::once(&set["primary"]).chain(set["methods"].as_array().unwrap())
                    {
                        occurrences += 1;
                        expected.insert(
                            rule["sha256"].as_str().unwrap().to_owned(),
                            rule["content"].clone(),
                        );
                    }
                }
                assert_eq!(pooled["bodies"], Value::Object(expected));
                assert!(pooled["bodies"].as_object().unwrap().len() < occurrences);
                assert_eq!(
                    pooled["roleSets"]["repair"]["creativePack"],
                    original["creativePack"]
                );
                assert_eq!(
                    pooled["roleSets"]["origin"]["creativePack"],
                    original["originRules"]["creativePack"]
                );
                if origin == "finisher" {
                    assert!(pooled.to_string().len() < original.to_string().len());
                }
            }
        }
    }
    #[test]
    fn pooled_rules_reject_missing_changed_ambiguous_or_unused_bodies() {
        let original = super::super::repair_rules("finisher", false).unwrap();
        let pooled = materialize_repair(&original).unwrap();
        let primary = pooled["roleSets"]["repair"]["primary"]["contentRef"]
            .as_str()
            .unwrap()
            .to_owned();
        for kind in [
            "missing",
            "changed",
            "reference",
            "inline",
            "unused",
            "role",
            "version",
        ] {
            let mut bad = pooled.clone();
            match kind {
                "missing" => {
                    bad["bodies"].as_object_mut().unwrap().remove(&primary);
                }
                "changed" => bad["bodies"][&primary] = json!("不能删减规则正文"),
                "reference" => {
                    bad["roleSets"]["repair"]["primary"]["contentRef"] = json!("0".repeat(64))
                }
                "inline" => {
                    bad["roleSets"]["repair"]["primary"]["content"] = json!("第二份不受绑定的正文")
                }
                "unused" => {
                    let text = "unused complete rule";
                    bad["bodies"][digest(text)] = json!(text);
                }
                "role" => bad["roleSets"]["unknown"] = original.clone(),
                "version" => bad["version"] = json!("series-rule-content-once/future"),
                _ => unreachable!(),
            }
            assert!(restore_repair(&bad).is_err(), "{kind}");
        }
        assert_eq!(restore_repair(&original).unwrap(), original);
    }
}
