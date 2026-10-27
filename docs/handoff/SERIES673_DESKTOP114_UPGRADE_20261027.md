# series v6.7.3 / desktop v1.1.4 桌面升级

## 来源与边界

基线提交 `06fed53`，集成分支 `feat/series673-desktop114-20261027`。开始时工作区干净。

上轮桌面分镜加载 series v6.7.2；本轮以用户维护的 Soullnk 本机规则为准，保留完整副本及独立哈希清单：

| 规则 | 冻结位置 | 源文件数 | 清单 SHA-256 |
| --- | --- | ---: | --- |
| series v6.7.3 | `rules/series/20261026-v6.7.3` | 134 | `3a5d08ba6fcc5657cda31283de13737566c65a266cd3bc9074df3219d93db890` |
| desktop v1.1.4 | `rules/desktop/20261026-v1.1.4` | 147 | `c8838adb0ddffdc30505547e5857ec195e66dba169ede1e2150a48e60ea71817` |

上游旧 SHA256SUMS/provenance 原样保留，不冒称与旧发布版完全相同；本次 `desktop-freeze.json` 绑定复制字节。旧 series v6.7.2、资产 V1.1.2 原件未修改。desktop v1.1.4 是桌面分镜 Skill，不是资产提示词版本。

## 采用内容

1. 新任务按冻结版本分派规则；series v6.7.3 提供整体合同及视觉基准，desktop v1.1.4 提供公共合同、情绪统筹、完整主创、整编、精修及声画节奏六份适用正文。运行快照记录每份实际来源、原始及有效正文摘要、适配项。
2. 项目设置提供自然 3D 国漫、3D 三渲二国漫、2D 动漫、真人电影写实四种明确候选与完整自定义。采用需要实际点击确认；打开设置不自动改变旧方向。自定义保留原文，六项依据须引用说明中的原文。更改正文、依据或画幅后重新确认。
3. 新任务必须有 `series-visual-baseline-v1` 采用对象。完整文字进入设计、设计审核、主创、整编、独审及修订岗位的唯一 `visual-baseline` 绑定；基准正文逐字传递，不能用短卡或“同上”替代。
4. 每个片段的提示词保留完整基准及原有本段演出。Rust 派生基准身份/版本/摘要，不要求模型填写派生字段。采用稿保存基准；独立导出及再导入核对方向、岗位包、采用稿与片段绑定。
5. 保留原文绝对 UTF-16 坐标、源锁、声道、原话、真实切点与精确小数时长。吸收长句跨镜不断声、完整读稿、精准供料、仅修必要范围等创作经验。

## 有意保留的桌面实现

- Rust 运行器、四阶段及稳定 ID 不改；CLI/SDK、全局渠道、媒体候选和母图依赖保持。
- 旧冻结任务仍读取 v6.7.2 原规则；标记函数不能覆盖已有版本或为损坏的半冻结输入补造身份。旧 12 种岗位组合以修改前黄金 SHA 验证。
- 本机 Rust 字段是 `dialogueParts.quoteStart/quoteEnd` 的完整源绝对 UTF-16 区间；不迁入便携脚本的 quote 相对坐标。
- 继续使用当前完整候选输出和有界局部修订，不冒充已实现 portable `desktop-field-patch-v1`、`desktop-draft-delta-v1` 或 `soundEventDecisions` 豁免。
- 不要求模型调用 `assets.mjs`、`inspect-source`、`inspect_editing/cut_pairs` 等外部脚本，等价写入/校验/导出由当前 Rust 链负责。
- 规则升级不自动改旧稿、采用图片、重发 UNKNOWN 或开始生产。

## 验证记录

| 检查 | 实际结果 |
| --- | --- |
| `python scripts/freeze-workflow-rules.py --verify` | 旧新 4 包全部通过；原件未变 |
| `npm run test:frontend` | 最终 342 项全部通过 |
| `npm run build` | TypeScript 与 Vite 通过；保留既有 bundle 大小提示 |
| `cargo test --lib series -- --test-threads=4` | 首轮 150 通过、2 个引擎 fixture 版本问题；已修正测试来源，没有放宽生产校验 |
| `cargo test --lib engine::series_start_tests` | 修正后 4/4 通过 |
| `cargo test --lib visual_baseline_tests -- --nocapture` | 最终 4/4 通过，含各岗位传递、同版采用、局部返修、UNKNOWN、恢复不重发、拒绝混版导出 |
| `cargo test --lib lingmo_package::tests -- --nocapture` | 8/8 通过，含完整导出再导入、保存重开、原源移走后可用、拒绝重算文件 hash 的基准漂移 |
| `cargo test --lib pipeline::series::rules -- --nocapture` | 最终 6/6 通过，含旧 12 岗位组合黄金 SHA 与新规则适配 |

上述数字有范围重叠，不相加冒充独立测试总数。SDK 回放使用离线合成响应；未进行本轮真实模型质量验证。证据目录 `.local/series673-desktop114-20261027/`，不提交私人运行目录。

所有合成测试、原生导入均不提交模型、图片或视频生成；本轮不开始新剧本生产。旧新规则的稿件不能伪装为一套共同创作依据混合导出；沿用原冻结恢复或分别导出，不自动重签旧稿。

本轮仅验证工程合同及 Windows 包装运行。实际新剧本的创作质量、风格充分性、真实图片与视频效果不由这些测试证明。
