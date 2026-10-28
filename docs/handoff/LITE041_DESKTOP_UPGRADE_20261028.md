# lite 0.4.1 桌面接线与验证

本轮在 `feat/recoverable-workflow-core-20261027` 上继续，起点 `832dfb14d9afd00f0b0e546925b584598aaf0657`。保留 Windows 已有渠道、资产与父图调度、灵墨导入、媒体、历史采用和恢复合同；不整目录覆盖，不改用户原有两份 `foundationCoverage` 未提交文件。

## 规则与运行版本

- 冻结来源：`K1rsam/series-storyboard-director`，提交 `f855227c8406b38093d03c777fe24a0b0b4f87dc`。
- Skill：`series-storyboard-director-lite 20261028-v0.4.1-3d-fantasy`。本机可变入口后来已更新为 0.4.2，本轮没有混入。
- 冻结目录：`rules/series-lite/20261028-v0.4.1/`；106 份原件，清单 SHA256 `25e213960a03eddb59928108ed7e95f4743b674074736129deefe803b972c538`。
- Rust 适配器：`series-lite-rust-adapter/v0.4.1-candidate.1`；新桌面路由 `desktop-series-lite/v1`；岗位账本 `series-lite-runtime/v2`。
- 激活范围仅限用户明确启动的新 lite 桌面任务。旧三槽、旧 0.1.2、series673、原 150 集冻结请求与 UNKNOWN 不重签。原版 Python/Node 脚本只作开发对照，客户端运行全部采用 Rust。

## 本轮行为

1. 新项目默认从首集开始，可选前 3 集、前 10 集或全部。保存已有方向后直接进入分镜创作；不再先默认整理完整 150 集或等待全库 F2。旧制作基础任务仍从原范围恢复，项目占用和 UNKNOWN 保护保留。文字创作不以图片账户或视频组件就绪为前提。
2. 按本集真实源、方向、必要完整设计和显式父依赖取料。已有可信设计与明确场绑定时本地准备，零模型补料；确实缺设计时只补当前必要项。不靠同名猜身份或场绑定。
3. 完整主创同次统筹，只交 plan/selection/draft；程序合入源、设置及引用，反算 workPackages/provisionalUnits。整编通过绑定基稿摘要、目标和预期旧值的补丁合成，独审获得完整当前源与稿。
4. 新岗位、回包、候选和拒绝均追加保存。格式或具体内容问题最多两轮局部修订；相同稿和相同问题无进展时提前挂起本集。预算跨重开和账户切换继承，网络/UNKNOWN 留在原槽对账，不借修订重发。其他集继续。
5. 独审采用验证实际 CLI/SDK 线程、完整创作参与历史、原请求/响应文件和运行回执。模型 PASS、机械通过或下载成功均不能单独授予 ContentReview。
6. 正式采用后在同一事务投影稳定单元，依据精确替换凭证退休旧 ID，保留原版本与媒体。待核候选不覆盖当前正式稿。媒体准备不使未改文字失效；人工改稿保留并进入待核。
7. 相邻前稿按实际采用证据登记消费者。前稿在创作中或采用后被改，会在真实消费者的采用/配图/导出核验中被发现，不改写无关正文。前集晚到时冻结新的接缝审阅节点，复用后集正文和原三岗回包；不只翻完成标志。
8. 导出 MD、同名 `.md.assets.json`、精确 native bundle、source-lock、完整 registry、候选、采用和执行凭证以及逐文件哈希索引。已有灵墨 MD 导入接续为 `md-and-catalog`，不冒充 native 完整包恢复。
9. 制图设计与分镜设计分别校验。保留身份父、设计父、实际媒体父的区别；显式媒体父转入现有 `referenceRequirements`，不把所有设计依赖都变成等待母图。

## 已记录的验证与失败

证据保存在本机 `.local/lite041-upgrade-20261028/`，私人数据库、源文和真实回包不提交。

| 项目 | 实际结果与边界 |
|---|---|
| Rust 原工具对照 | derive-plan 45 例、0.4.1 packet 52 例、prepare 16 例、image design 62 例；成功原字节与拒绝结果分别对照。旧 packet 44 例保留 |
| 本地执行回执 | 独立 harness 11 项通过，含冻结执行/恢复案例及 SQL intent→FS receipt 崩溃窗口、篡改和幂等 |
| Rust compile-03 | 失败：导出把解析 JSON 误传给原 source-lock 字节接口，另一本地 gate 返回类型不符；已修，失败日志保留 |
| Rust compile-04 / lite-04 | 编译通过；185 通过、28 失败。失败集中在新增 fixture 的集场归属、registry 和独审绑定，按真实合同修 fixture，没有放宽生产门禁；不是最终通过证据 |
| 前端 final-02 | 365 通过、1 失败；旧离线测试仍要求新版第一阶段渲染全剧 F2 按钮。按新入口及旧 API 边界分别调整后须复跑 |
| SDK bridge | 本轮离线 191 项通过；不等于新剧本真实 SDK 内容验收 |
| Rust compile-05 / lite-05 | 编译通过；217 通过、5 失败。Windows fixture 未 canonicalize 目录和混用媒体初导入/重读投影已修正；生产包含检查、媒体字节保护未放宽 |
| Rust compile-06 / rust-final-06 | 303/303 相关测试通过：lite 223、exact 7、issues 8、artifacts 7、显式路由 4、单元局部配图阻断 1、资产工作流 48、SDK 进度/守卫 5。每组均有 Windows Job Object 回执和进程树清空证明，无超时，不运行 ignored。二进制 SHA256 `b6ce7c055f71f35e085c12d3e56a19a77598ace78a5557fe6d209e55252461ee`；不是全 crate 的 2382 项全部运行 |
| 前端 final-03 | 366/366 通过；typecheck 通过。含 150 集新项目不先跑全剧 F2、旧恢复及离线 API 边界 |
| 账户本地状态 | 本轮 `codex login status` 返回 `Logged in using ChatGPT`；不据此声称远端模型、额度或新稿质量已验收 |
| Windows 安装候选 | 构建通过；相同候选两轮隐藏启动/IPC/保存重开/工作页恢复通过，四阶段菜单与本地素材可读，均无生成请求。另从最终安装路径隐藏启动一次通过，验证设置界面、素材和工作页持久化 |

## 尚不能宣称的结果

- 本轮尚未从原客户端执行新剧本一集→十集的真实模型与逐集读稿验收，也未做图片视觉审核。禁止提交视频；没有为本轮发起图片或视频生成。
- 原“精神小妹”150 集、Q1 和统一收口不因 Skill 升级自动完成；最近保存覆盖为 83 集，完整范围仍待原任务续接。已有容量修复随当前源码保留，不能用新工作流抹去旧任务。
- 当前 native 完整包的自动恢复导入器尚未接入；独立 MD＋catalog 导入保留真实来源和版本，不伪造原剧本。
- macOS 原生、真实平台实导和成片质量不在本机工程证据内。
- 上级 `D:/EDY/Documents/CL` 的 AOCI 收尾返回 `scope_change_required`：索引 26 条、当前策略发现约 5.15 万源码对象，未签发语义候选，无 Recovery 或第三方冲突。本轮没有擅自扩大/缩减治理范围、改写正式索引或把旧认知称为已对齐；此工作区治理缺口与桌面候选工程验收分别记录。

## 安装与回退

已使用现有 `update-client.ps1 -BuildOnly` 构建，验收后通过 `-PublishOnly` 更新同名程序。未操作或关闭用户前台。后端与冻结包提交为 `c6a40ff`，前端与本报告随后独立提交；完整最终提交号以本分支 Git 历史为准。

- 原快捷方式：`D:/EDY/Desktop/director-desktop.lnk`，仍指向 `apps/desktop/.local/client/director-desktop-next.exe`。
- 当前版和验收候选 SHA256 同为 `2e3bfb84095b870cb6d82b1f3301cef525e9e84a45e99a8c49a05de8a4623907`。
- 上一版 `director-desktop-next-previous.exe` SHA256 `572d0d892b0582d91fd00183d2232ffa38fbf13a7f1f0abff413ed7fcefd582a`，资源按各自 EXE SHA 隔离。
- 回退可直接打开同目录的上一版；没有对真实项目自动重签或批量升级数据。首次产生的新 v2 账本是增量表，旧版不会替新候选作采用判定。
- 私人证据：`startup-final/summary.json`、`startup-installed/report.json`、`publish-final.log`。安装验证是零模型/图片/视频请求；新剧本真实一集与十集内容仍需从原客户端实测。

用户原有未提交文件本轮末次 SHA 核对一致：`foundationCoverage.ts` 为 `839e206702860c5b0bdc5b8c2d21ba742be8a6d06c8b3c6d89e187b8ab9ba15a`，对应测试为 `57f8f32b0a953dae0dabe2fc95f3eec24edce7d40462e1cd5be2c2216edf0d7a`；没有混入本轮提交。
