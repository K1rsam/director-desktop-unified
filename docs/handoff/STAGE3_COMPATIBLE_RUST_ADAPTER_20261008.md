# 第三阶段：冻结生产规则的 Rust 兼容适配

本次集成实际启用 Mac 3008 的 22 份冻结生产创作文本，以 Windows 的 Rust 三岗、源绑定、差量采用和保存恢复链承载。实现身份为 `mac-3008-compatible-rust/v1`；这不是原 Mac 运行器移植，也不是仅修改 `active`。

## 来源与边界

- 本轮 Mac 代码为 `05b64c920384200ce45be51ac9962c39714178cd`，完整指南为 `8b07ca276a82fe6d91cc1c3b3c0db58ed76e24af`。此前历史指南来自独立兄弟分支 `cb4f728e1f22dbf20b88e0eae0cf0a11c6c85f13`；审查建议、选择性迁入和各轮实现不混为同一版本。
- 原生产文本对应 `K1rsam/director-web@00093c632c45369106c7ef79a47b684fd5e77526`。`rules/stage3/manifest.json` 保留全部 22 个原件路径、字节数和 SHA-256；加载器逐项验证真实 `include_str!` 内容。原件不修改，换行必须保持 LF。
- `MAC_RUNTIME_TRANSFER_REQUIREMENTS.json` 仅移交 41 份 MJS 静态依赖和 1 份 TS 业务绑定的元数据，42 项均 `bodyTransferred=false`，不能证明原 Mac 调用器、DTO、取料器或精修实现已移植。
- Windows 采用受保护工作树的实际后续有效实现，包括已进入 Windows 后续提交链的源坐标修复、零秒无对白镜有界修订、原 Finisher 差量恢复等。保护报告中的已跟踪差异仅为 `.gitignore`；本地新增资料另行保全，不能将上述代码功能误称为未提交更新。
- `reference-skills` 的 72 份快照与历史分支的 65 份归档为不同统计。它们不自动成为运行规则；旧桌面注册 29 份与新生产包 22 份按请求 profile 分流。

## 实际注册、取料与调用

`pipeline/stage3_pack.rs` 是独立加载器。规则包哈希绑定 `packId`、`mac-3008-rust-adapter/v1` 及全部原件路径、字节数和哈希；`source-bound-rust-selector/v1` 另在实际请求的 `stage3-loaded-rules/v1` 回执中记录并受请求身份保护。manifest 的状态、日期和发布说明不改变已冻结请求身份。每个实际请求记录主岗、去重后的方法子集、原件路径、版本、字节数、哈希。未知 profile、未知方法、重复方法或冻结包哈希不符均在调用前拒绝。

正常顺序仍是 `episode-emotion-planner` → `unit-performance-director` → `episode-finishing-editor`。供料整理是可信 Rust 取料和主岗按需加载 `source-context-preparer`，不新增一个隐式收费岗位。创作文本由原件主岗及方法提供；兼容合同只解释运输字段、源引用、时间基准、差量和采用条件，不引入额外镜数、镜长、幻想比例或新岗位。

第二、三岗按当前源范围、上下文证据和正式集场身份选择资产卡。直接身份/变体保持正式 ID，显式依赖父资产按精确关系闭包保留为 `MATERIAL_SOURCE_ONLY`；未来形象不因同名或邻文进入当场制作。未知导入范围保留原字面值，不推断适用范围。领域验收仍核对完整正式资产库；缩小模型卡片不放宽源事实或身份校验。

运输中对白采用独立的 `locked-utf16-reference/v1`：模型选择源 `lineId`、`quoteStart`、`quoteEnd`，Rust 从锁定半开 UTF-16 区间逐字物化原话、说者和声道。它与 Mac 原件中要求的文字保全一致，但不冒充未移交的原 DTO 实现。表演标记不变成发声文字，声窗、顺序、一次覆盖和完整源范围继续严格验收。

未携带 `storyboardRulePackProfile` 的旧任务继续使用原 29 份注册规则、原 settings/schema/合同和原请求缓存。已携带 Mac 生产 profile 的任务保留其原 22 份冻结规则和请求身份。新任务冻结明确的新 profile 与包哈希；保存重开不会把旧请求改造成新配方。

## 声画节奏的实际定位

十集验收预检发现，原 Rust `capacityBrief` 没有生成 Mac 规则引用的 `rhythm.audit`，且仍输出 13–15/26–30 秒填满优选区间，与原生产规则的容量上限原则冲突。新建请求另冻结 `storyboardRhythmProfile=source-bound-rhythm-locator/v1`；只有携带该 marker 的请求补齐定位并清除填满偏好。已有 legacy 或未携带该 marker 的 Mac 请求继续保留原请求字节和缓存，不在恢复时静默升级。

三岗和独立精修均收到 `capacityBrief.rhythm`：15/30 秒等渠道限制属于完整生成单元；2–5 秒和每 30 秒 8–12 镜是原规则的常用参考，没有设置 5 秒硬上限或固定镜数。schema 注释明确 unit 与内部 shots 的区别、实际硬切另列镜条目、原句按连续 UTF-16 片段跨镜发声，未增加数值裁切约束。

`pipeline/storyboard_rhythm.rs` 只读已提供范围的真实镜长和相对声窗，形成长镜坐标、实际镜数、完整原句声窗/历时、跨镜与跨单元接话、片段语速和范围尾段定位。字数从锁定 `spokenSegments` 与精确 UTF-16 区间取得，不信任回包里的改写 `text`，不朗读演法。源区间或代理对无效时保留未知标志；声窗无效时不跳过未知片段后推断停顿或无声尾段，零镜长不造一个假时钟。逐类坐标限制为 24 条，嵌套片段明细合计也最多 24 条，并明确记录省略数；统计总数仍覆盖全部给定稿。摘要不复制原文、对白、源引用列表或画面正文，不增加模型岗位或调用，新增字段仍受实际出站 envelope 预算限制。

这些坐标不判空等、不判创作质量、不决定自动重试。已有主创/整编在本次调用内按具体声画收益局部联修；额外精修仍需用户选择及显式采用。有效长镜、短爆点、必要气口、完整动作、原话和幻想均保留，不机械拆镜、压秒或裁声窗。自然语速参考和文字估窗也不等于真实配音或视频验收。

## 独立导演精修

`pipeline/storyboard_refinement.rs` 使用实际 Mac `director-refinement` 原件，并作为用户可选的独立一次候选操作运行，不附加到普通三岗自动调用。

- 输入必须是完整、实际已采用的集稿。完整单元体、已核验原 Planner、源范围、正式资产、生产方向、容量和用户制作要求一起供料；局部选择仅限制允许替换的范围，不缩成只读一个小单元。
- 返回 `storyboard-refinement-candidate/v1`，差量、原回包、工程验收稿、问题组、反馈、输入绑定、规则回执和请求身份同时保留。`no_improvement`、标题或说明变化、未解决问题、改原话或越范围均不能自动成为可采用成果。
- 每个替换组必须对应完整连续旧单元，保留单元由程序逐值复制。散落的多个问题组可分别替换，组间保留段不被快捷依赖绕过。幻想关联以完整合法组验收。
- 只保存候选，`automaticAdoption=false`、`creativeQualityAccepted=false`。失败不自动重发；只有真实已停止的运输尝试可在原来源和一次预算内恢复，未知、已发出、收到原回包或仍运行尝试不获得新调用权。

## 原子采用与后续来源链

`Store::publish_storyboard_refinement(run, expected_candidate_sha, current_input)` 在同一个 SQLite `Immediate` 事务中读取原完成候选、原验收 text checkpoint 和原 `text_received_raw`，验证唯一请求、原规则、候选自哈希、源/资产/单元 guard、完整当前集稿和项目保存 settings 指纹，然后重做完整差量领域验收。

单位发布、组间精确依赖重接、后续末态影响标记以及 `storyboard-refinement-adoption/v1` 回执共同提交。并发人工稿、来源变化、设置变化或原回包损坏导致整笔回滚。相同已采用候选重复采用幂等，不增版本；候选 `jobs.result` 原始字节不更新。任务展示层补入的 `mediaIds` 另作投影核对，不能被误当成原回包变化。

采用回执保存原已采用 Planner 的完整来源、采用后实际单元版本和内容指纹。`engine.rs` 的来源辅助函数识别原 Delta Finisher 的 `materializedDraft`，并逐步回绑独立精修的原验收请求、原回包和上游 Planner。采用后可继续精修或局部重做；缺少原回包或伪造来源失败关闭，不用名字或较旧相同正文兜底。

## Windows 初版兼容适配的离线证据

日期：2026-10-08。下列为初版兼容适配记录，基于当时集成工作树的同一 Windows MSVC 测试二进制 `director_core-439d45a8f08a86f8.exe`，不使用 Mac 测试统计代替 Windows 验证，也不将这些旧记录当作本次声画定位增量已通过的证据。声画定位增量的独立回归记录见下节。

1. `cargo check --lib --manifest-path apps/desktop/src-tauri/Cargo.toml` 通过。
2. 最终编译与真实保存执行器定向：`cargo test --lib --manifest-path apps/desktop/src-tauri/Cargo.toml native_saved_executor_reopens_the_exact_refinement_checkpoint_without_dispatch -- --nocapture`：1 通过。隔离临时库种入工程验收的合成 checkpoint，保存重开后由真实 `SavedTextExecutor` 复用，新增 text job 为 0；采用前后原候选 SQL 结果字节一致。
3. 编译结束后复用同一 exe，`storyboard --test-threads=4 --nocapture`：147 通过、0 失败、7 忽略，36.38 秒。覆盖 Windows 原三岗/差量/源坐标/载体/资产/缓存，以及集成的新规则、取料、精修和恢复。
4. 同一 exe，`refinement --nocapture`：7 通过、0 失败，9.57 秒。包括三岗实际 Mac 规则加载、整稿阅读局部变化、未解决/零改动/标题变化拒绝、源与范围保护、保存重开、真实执行器缓存、原子采用/幂等/人工稿与设置 CAS，以及采用后继续精修和局部重做的原 Planner 来源链。
5. 同一 exe，`stage3_pack --nocapture`：2 通过、0 失败，0.02 秒。22/22 原件实际哈希及按需规则子集一致，未知/重复规则和冻结 hash 漂移被拒绝。上述取料身份回归同时包含在第 3 项中，不重复累计测试总数。

日志保存于工作区外的 `_task-backups`，没有写入真实生产库。对应最终日志 SHA-256：

| 日志 | SHA-256 |
| --- | --- |
| `stage3-windows-storyboard-20261008.log` | `dfaea0f981f3517d4b1a7d7a72086f67bd4f552742d04a450f4c64ce602eaf5b` |
| `stage3-windows-pack-20261008.log` | `e18292cf15dbc2324805b52f3cbb0c6d67c8ebbdc9cb6802efc73e493e724944` |
| `stage3-windows-refinement-20261008.log` | `65133dac9753595501c75eb92807c12363344ca34617079d7ccf23a4968cec21` |

编译曾因另一个已授权离线测试占用同一 exe 出现 `LNK1104`，等待该测试结束后同一源码编译通过。首次新增测试把展示投影和原结果混比，修正为原 SQL 字节和单独的展示字段验收后通过；不把这两次中间失败记成最终通过证据。

这些回归使用合成源、临时 SQLite、内存模型或已种入的完整本地缓存，没有调用真实 CLI/API、读取真实凭据、触碰用户生产库或发生付费生成。7 项忽略是原有显式私有证据测试；本轮没有运行，不能声称已验收其实际项目内容。

## 声画定位增量的实际离线回归

2026-10-08，在基线 `0175c0d` 后的集成工作树中，复用主控统一 release 构建的 `director_core-865dfb0c8f9ffef6.exe`，通过解包原生 runner 顺序执行以下过滤器。实际命令为 `windows-native-test-runner-job.exe --timeout-seconds 300 <新的绝对证据目录> -- <实际 release lib exe> <filter> --test-threads=1 --nocapture`，没有逐项重新链接。

| filter | 通过 | 本机证据目录 |
| --- | ---: | --- |
| `rhythm_locator_tests` | 5 | `_task-backups/stage3-rhythm-native-01-20261008` |
| `stage3_pack` | 4 | `_task-backups/stage3-rhythm-native-02-20261008` |
| `actual_mac_rules_cover_three_roles_and_exact_legacy_replay_stays_old` | 1 | `_task-backups/stage3-rhythm-native-03-20261008` |
| `refinement_feeds_the_full_adopted_episode_and_changes_only_permitted_group` | 1 | `_task-backups/stage3-rhythm-native-04-20261008` |
| `native_saved_executor_reopens_the_exact_refinement_checkpoint_without_dispatch` | 1 | `_task-backups/stage3-rhythm-native-05-20261008` |

12 通过、0 失败、0 忽略；每份独立 `receipt.json` 均为 `success=true`、`exitCode=0`、`processTreeEmptyVerified=true`，并记录启动时 Windows Job Object 进程树包含。每个目录另有完整 `stdout.log` 与 `stderr.log`。聚合 `stage3-rhythm-incremental-evidence-20261008.json` 记录实际命令、四份 Rust 源码指纹、测试 exe 指纹和全部日志/回执哈希，SHA-256 为 `b107c3cf3cfe024bd74d2105e9e60cb5d5d74485e8a42f3333e24e1b1308e024`。

新回归实际核对跨镜/跨单元连续源片段、锁定源字数、无效代理对和零镜长、未知声窗不误判停顿或尾段、坐标及嵌套明细边界；实际三岗与精修输入收到定位，未携带 marker 的请求保持不变，真实保存执行器重开后不新增派发。这些仍是合成输入和隔离库的工程证据，不证明十集实稿质量、真实配音或视频效果；本节与初版测试存在重叠，不能相加为唯一测试总数。

## 仍未验收

原 Mac 42 份运行正文及原精修 schema/loader 未移交；已移交的本次 Rust 源码和历史追溯资料不能代替这些原运行实现，兼容适配不声称完全相同。真实模型执行、具体账户 scope、Canvas 图片桥、Windows 真实渠道和用户已有项目原生验收由总集成报告分别记录。本文件只证明第三阶段离线工程链。

第 1、2 集新旧成稿的情绪、动作、镜头、幻想及最终生产质量尚未对比；合成测试只能证明身份、源保全、顺序、采用和恢复，不能证明创作质量。真实生成必须核对账户、冻结输入及既有费用授权。最终集成分支、提交和全量/启动验收见总集成报告。
