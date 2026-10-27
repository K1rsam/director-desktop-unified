# 可恢复工作流重构与验收记录

用户于 2026-10-27 确认实施。保留客户端、Rust 底座、已有项目、四阶段界面及渠道；先完成原 150 集制作基础，再验收蒸馏版 0.1.2 和新项目全流程。新增功能暂缓。原冻结请求、原回包、采用版本、UNKNOWN、源锁与已有媒体不得覆盖。

## 冻结决定

- 从 `ea6b165e3613fa29d1c6b676827a3a99e7a97a3a` 建立 `feat/recoverable-workflow-core-20261027`；原两份前端未提交状态已独立保存，不混入重构提交。
- 资产身份、设计版本、用途、真实媒体依赖分开；别名相同仅触发核对，不证明身份相同。
- AI 修订只交绑定基稿、目标及预期旧值的操作；程序合成完整候选，不默认重交整个批次。
- 不确定身份保留独立并标记待核对；其他任务继续，相关图片不自动生成，待核对不计为最终通过。
- 每问题最多两轮 AI 局部修订，同因无进展提前停止；旧预算跨恢复、账户切换和重启保留。
- 统一版本化成果、结构化问题、采用记录与消费依赖；增量迁移，不整库重建。
- 全部运行工具移植 Rust；原版脚本仅作开发期对照，不成为客户端运行依赖。
- lite 初始基线：`K1rsam/series-storyboard-director` 提交 `6dea8d6c57dde38f04a4d71f05f4c039004dd4a9`，`20261027-v0.1.2-usage-reuse`。改变工具行为的后续更新必须同步验收适配器，不仅替换规则文本。
- SDK 沿用已授权额度，图片只用已确认免费的渠道，不提交视频；不操作或自行关闭用户前台，真实生产从原客户端续接。

## 实施与验收顺序

1. 当前 150 集：冲突清单、显式身份/变体/用途决议、程序应用、有界恢复及旧原稿复用；核对与统一收口。
2. 统一合同：原回包、候选、验证、采用和下游消费分离；局部失败不阻塞无关分支；人工修改形成新候选。
3. lite Rust 适配：preflight/compose/patch、record-uses、sync/check/export/verify；保留完整主创、新上下文整编、非作者独审。
4. 小本一集→同本前十集→多来源/多格式/恢复/规则升级→同一 Windows 安装候选验收。

工程通过、真实模型调用、内容审阅、图片审核、平台导入和安装版验证分别记录，不相互替代。任何未决项如实报告，不以完成标志隐藏。

## 当前状态

实施中。原项目从已保存 74 集续跑至 **83 集**；旧受限重基已通过新的局部决议恢复。当前新阻断为完整 F2 请求超出应用容量，尚未完成 150 集核对与收口，尚未启用 lite 新生产。保护证据位于本机 `.local/workflow-core-refactor-20261027-163123/`，不提交私人原文、数据库、模型回包或凭据。

## 第一实施批：原任务恢复合同（恢复候选已安装，真实续跑待原客户端启动）

- 新增冻结冲突清单与显式决议，资产身份、已有变体用途和设计变更分开应用；原请求、原成功／拒绝回包保留。
- SDK 新决议沿用原任务证据及两轮上限。新增被动 UNKNOWN 对账只读取原尝试，不能重新执行；CLI/API 历史采用可读，新决议暂不开放这两入口。
- 未决身份进入非阻塞待核对列表，相关资产及实际母图消费者不得新发图片；无关分支继续。普通 Q1 不能仅用 resolvedIssues 清除程序保留项。
- 新决议回调在 jobs 与 workflow_steps CAS 的同一事务保存原回包、候选和结构验证；不补签旧任务，不冒充内容独审或正式采用。
- 统一成果和问题账本为显式增量表；旧库纯读不执行 DDL。成果核心已在新决议窄域接线，通用问题账本尚未接生产。
- 源验证成功缓存仅绑定完整 LockedSource 序列化摘要，最多四项且不持久化；相同原文 hash 下改变说者、分类、坐标、集场或解析器仍拒绝。未宣称真实运行提速。

### 本机阶段证据

证据根：`.local/workflow-core-refactor-20261027-163123/`（私有，不入 Git）。

| 检查 | 结果与边界 |
| --- | --- |
| compile-08 / compile-release-08 | 含 UNKNOWN 精确对账与新决议 invalid_json 有界恢复的测试目标编译成功 |
| frontend-03 / frontend-foundation-review-04 | 359 项完整前端测试及新增待核对显示检查通过；不代表生产完成 |
| saved-wave-05、saved-wave-06 | 保护副本六批和旧采用回放通过；已保存行不变，零模型调用，只留下一个新决议请求（九项冲突）；旧拒绝消耗一次，下一次为第二次 |
| 新决议实际供料 | canonical 287732 → wire 168108 字符，低于 176000；完整必要资料保留，无静默裁剪 |
| core-06 | 900 秒限时停止，未完整通过；三项旧测试缺新版视觉采用前置条件，一项本地异步测试在负载下超短超时。已补正确前置条件和非性能测试等待期限，生产门禁不变；后续按模块分组复验 |
| live-context-06 | 逻辑对账/计数通过，但整体测试因 SQLite 创建零字节 WAL 而失败；数据库 hash 未变。后续证明明确区分空 WAL 生命周期与任何数据页变化，不沿用本次为通过证据 |
| live-context-08 | 真实原 SDK 证据只读核验通过：零 dispatch、预算不变、DB 和 WAL 数据页未变；没有复用失败的 06 为通过证据 |
| core-through-10 | debug08 与 optimized release10 按精确单例补齐覆盖：2,127 通过、69 忽略、0 未覆盖。原组超时、7 项失败后补测通过和重型单例 debug 超时记录均保留；这是跨两份二进制逐项证据汇总，不是一次全组 PASS。release10 SHA256 `084f755ae60a6f3dcedf709cb22e4df68f01033b2f0be8ba424eba66fc58f1dd` |
| startup-08/report1、report2 | 两轮真实候选隐藏窗口验收均 ok；第二轮保存重开及工作页恢复通过，generationRequestsSent=0 |
| 安装版恢复候选 08 | 已通过原 update-client.ps1 PublishOnly 更新原快捷方式同名 EXE；不是最终发行版。当前 SHA256 `572d0d892b0582d91fd00183d2232ffa38fbf13a7f1f0abff413ed7fcefd582a`；上一版 `aa2afbaaeac70f0d388a84a5bb6d0281c7e939ddbd64b23bb3ca99c826751d36` 保留。发布前无原客户端进程，未自行关闭窗口 |
| 原项目生产 | 用户已从原快捷方式恢复。SDK 第二次局部决议接受：九事项中六项字段处理、三项身份暂缓；原九个受保护任务的冻结字段摘要全部未变。随后三路推进至 83 个唯一已接受集范围，F2 全请求容量门禁停止新分派；150 集 Q1/收口未完成 |

## 第 83 集后的容量恢复（候选 12 已验，待安装）

- 实测完整目录重复字段是主要膨胀来源；不缩减共同 1–27 集历史、当前集原文、身份、别名或制作说明，不提高 200k 总预算及其输出预留。
- 新增可反解、可校验的 `foundation-catalogue-table/v1`，只在 typed ID 替换之后压缩字段名和重复 preparation 值；canonical 冻结请求仍保留完整目录。缺省与 null、未知字段、顺序和同名不同身份均保留，输出仍按原 schema 交本批差异。
- **必须先完整运行原容量规划。** 原规划成功时保持原 scope、batchCount、packetId 和请求字节。只有内部类型化容量失败才从未消耗的原 pending 重试表化；不以中文展示错误判断恢复，不修改旧请求或原回包。
- `compile-release-11` 与 `lite-wire-11`：88 项相关测试通过。只读真实回放 11 首先被测试路径斜杠与原 workerRoot 不一致拒绝；使用原根路径的 11b 证明提前表化确会改变旧批范围，测试正确失败，DB、WAL 数据页及保存记录不变。该失败是采用“原规划优先”修复的依据，不能作为恢复通过证据。
- `compile-release-12` / `lite-foundation-12`：92 项通过，包含原规划优先、失败保留队列、最小批容量 fallback；测试二进制 SHA256 `26e1795f11f6760fd57ec317c05f48a6d0ac9f1c846f2387791265eaea811534`。
- `current-f2-replay-12`：原 16 个 accepted 节点（覆盖 1–83 集及决议）全部精确复用并复验；拒绝记录保留。DB、WAL 数据页和全部 jobs/workflow_steps/events 不变，模型调用与 dispatch 均为 0。下一波为 11、10、7 集三个新请求，含完整规则与 schema 的保守输入预算分别 175841、173229、164801，均低于 176000；另保留 24000 输出预算。仅证明可以安全准备下一波，不代表模型已经完成剩余集或 150 集验收。
- `build-candidate-12` / `startup-12`：候选 SHA256 `acffcd065e7d320aebffc51408bfe3871255b213a591d1976b01659cae3e5dcd`，两轮隐藏原生启动、IPC、保存重开和工作页恢复通过，生成请求为 0。初次验证脚本只等待启动器退出，先于实际客户端报告落盘；已改为同时等待真实报告，未重复第一轮。原客户端仍在运行，未自行关闭、未替换安装版。

## lite 0.1.2 冻结与适配边界

### 主工程接收、供料与本地执行（2026-10-27，仍未启用三岗生产）

- `compile-release-15` 因新场级投影误克隆 slice 而编译失败；已改为独立 Vec，保留失败日志。`compile-release-16` 成功，二进制 SHA256 `7918b68f10f78b38d821942897a6bf95293f211bd549693d282f0ffc9333a9b6`。
- `lite-exact-execution-16b`：19/19 通过（exact 7、场级投影 5、本地执行 7）。第一次测试启动使用相对路径未产生 runner 回执，不能算通过；16b 改用绝对路径，实收 Windows Job Object 回执及完整测试输出。
- exact SDK/CLI 分流保留原响应 UTF-8 与精确小数；复用原 jobs、workflow_steps、账户回执和 UNKNOWN 对账。只证明原文本来源，不能代替领域验证、独审或采用。API、无 workflow 独立执行、CLI 自动 transport retry 尚未开放。
- 场级投影只从实际 units 的 bindings 推导同场稳定并集，要求外置场级供料 pin；同一编号场次允许阳台与餐厅两种场景资产，不能拆成两个场次。未供资产、跨场/纯参考父卡越权和 voice-only 入图拒绝；源、场次其他字段、资产版本和精确时间不改。供料 pin 的来源真实性仍由 controller 核验，投影本身不授予设计采用。
- Rust 本地 `sync → check → export → verify` 接入真实 crate。20 个冻结原版执行/恢复场景、8 个依赖投影（含十一层）、7 个字段容量对照包含于上述执行测试；最终 MD 与目录字节一致。同版恢复需独立持久 receipt pin，禁止覆盖回执；`contentReview=NOT_PROVEN` 始终保留。旧非 v7 跨季入口仍明确不支持。
- `compile-release-17` 成功，二进制 SHA256 `76d44ea6a58768d618e6d16ac33c2d46e5d2fb1de8d870560cd18092767b4dc6`；`lite-supply-17`：30/30 通过。含 28 项供料测试及 2 项真实 Desktop parser/adapter 包装器测试。第 28 集不能被内部序号改为第 1 集；保留 UTF16 与原场映射。旧完整设计需原发布、原作者/独审回执和当前版本一致，显式依赖闭包逐项核证；未知类型及证据不足返回局部 gap，不伪造 lite TEXT_ADOPTED。
- 本批未进入安装候选 12，未调用真实模型、图片或视频。岗位持久账本、完整三岗调度、原生采用/第四阶段投影及真实内容验收仍在实施，不以这 49 项新增工程通过宣称最终发行通过。

### 岗位持久账本及独审响应合同（尚未接 Engine 生产）

- 新增 `series_lite_controller_store`：显式增量建表，旧库读取不写 DDL；派发前保存完整请求、精确基稿、外置岗位/上下文 pin、共同依赖及前岗候选。首版限定主创→整编→独审三槽，前岗原任务接受且候选持久保存后才可取得后岗上下文。独审原回包与候选共用源/设计/基准依赖，岗位独有证据留在 provenance，避免合法独审因输入摘要不同被拒。
- 通过原 `VerifiedRawText`、jobs、workflow_steps 和接收事件核验后保存 Raw/Candidate/ReviewRaw；重入幂等，不覆盖已有成果。原文本文件摘要与成果封套摘要分别保存。未知提交仍返回原槽，不因换账户更换创作上下文。
- 审查发现初版跨 workflow 扫描仅挡未决传输，仍允许 received/queued/rejected 等状态绕行。已收紧初始链：同源同集已存在其他 workflow 岗位时不再建立新初创槽，避免丢掉已返结果、双排队或重置拒绝预算。明确重新创作版本及有预算局部修订尚未接入此模块，不能把这个初始链当完整修订控制器。
- 新增 `series_lite_review`：绑定完整候选原字节、实际源行/单元阅读范围、问题坐标与影响、旧问题保留或当前稿关闭依据，以及确实提供的上集末单元→本集首单元。不提供上集稿时保留边界待核；不会把单集报告 PASS 当整季交付。`declared_content_pass` 仅是报告结论，不是独立内容验收。
- 计划上下文不同不等于 SDK/CLI 真实会话不同。账本明确保留 `actualContextIndependence=NOT_VERIFIED`、`contentReview=NOT_PROVEN`、`adoptionGranted=false`；真实会话排除、领域验证、正式采用与既有单元/第四阶段原子投影仍须在 controller/发布适配中落实。
- `compile-release-18` / `lite-controller-review-18`：12/12 通过；二进制 SHA256 `76d877ff674c51ac4ef5608e87b9edaecb420f4a44ad687dc7de6b56e364cb5a`。补跨 workflow 各状态回归后，`compile-release-19` / `lite-controller-review-19`：13/13 通过，二进制 SHA256 `0b498895fe729a054ac37e8964ef17231f216b6b5e45f96fb1999605fbfabe20`。测试使用合成 CLI 原始证据和临时库，没有真实模型；包含原执行器精确读取/接受/拒绝、保存重读和幂等，而不是实际三岗内容创作验收。
- 19:54 只读现场复核：原 workflow 仍为 83 集后容量失败，events 最大 ID 276；原客户端 PID 361108 仍占用当前 EXE，未自行关闭或替换。候选 12 SHA 再核一致。已有关闭客户端的请求仍待完成；真实 150 集/Q1/收口、三项身份未决、lite 新项目一集→十集、最终安装发行门均未完成。

冻结包位于 `rules/series-lite/20261027-v0.1.2/`，来自指定提交 `6dea8d6c57dde38f04a4d71f05f4c039004dd4a9`。90 份文件逐项符合原 SHA256SUMS；原脚本仅开发期等价对照，普通用户运行依赖仍为 Rust。旁置 receipt 的 active=false，未替换已生效旧规则。

已迁入 compose/patch/preflight、同版用途登记、registry/season 校验、精确时间、源锁、灵墨导出／独立反解／文件 hash、sync 消费回执。完整源正规化、创作门禁及实际三岗链仍在接续，不能将纯函数对照通过写成新项目验收。

### 后续原生接线批（尚未进入安装候选 08）

- 提交 `d60143a` 新增完整岗位材料包编译、冻结读回和漂移拒绝。44 个冻结 Python oracle 中 13 个成功包字节一致、31 个拒绝一致；实际 crate 的 job_packet 11 项已包含于上述 88/92 项通过。精确数字留在原字节中，不经 `Value` 浮点往返。workspace/续季验证和精确 transport 另行注册；三岗实际调度及采用仍未启用。
- `compile-release-13` / `lite-registered-13`：实际 crate 的 lite 相关 **83 项通过**。新增 workspace 原生验源、首季及 v7 闭季接续；68 个原版 oracle 中 8 个支持的成功、59 个拒绝一致，另 1 个原版合法旧单区接续明确未支持。新增岗位 UTF-8 载体保存完整包、外置身份/上下文/迭代 pin 与原数字；13 个成功 golden 原字节保存重开一致。均未进入候选 12，也未启用三岗生产；精确模型响应接收、执行/采用及真实内容验收尚在接线。

- 已在真实 Rust crate 注册 standard-source 完整证明、normalizeDraft/event packing、永久 A-ID 映射。标准包在三个 BundleGate 入口统一分流，历史 raw-row 仍保留独立读取器，不能降级绕过标准证明。
- `compile-release-09` 成功，真实 crate 中 lite 相关 **45 项通过、0 失败**（含永久身份映射的重开、双连接 CAS、墓碑与零写入读取）。证据 `lite-registered-09/`。这不是三岗生产、真实内容审核或安装版 lite 验收。
- 进一步新增标准包整链 MD/manifest 字节对照回归、随安装包内置原件及 system-presentation 完整规则选择；这些是 09 编译之后的改动，必须在下一稳定候选验证，不能沿用 45 项结果冒充通过。
- 冻结原版工具继续仅作开发期对照；生产开关仍关闭。intake planner/compiler、完整创作供料、采用与下游投影正在实现。

- `compile-release-10` 编译成功，真实 crate **59 项 lite 测试通过**，包括已注册 intake、标准源整链、内置规则、Desktop 原文范围/背景/重开绑定。intake 独立对照含 468 个接口案例与 251 个 planner 案例；原文 UTF16 和有效完整结果对照，不以成功退出代替比较。完整三岗生产仍未接入。
- 供料包 `series_lite_job_packet*` 已完成独立移植，44 个 Python 原工具对照及 11 项独立 Rust 回归通过，尚待真实 crate 注册。完整 file-workspace/season verifier 尚缺，默认拒绝含未验证 state.json 的工作区；不以哈希/字段存在冒充前季与来源链验收。

### 83 集容量阻断调查

最后普通单集 F2 冻结输入中，registry 约 97,450 字符、历史候选约 65,016 字符，当前集 source 约 4,686 字符。历史仍限定共同 1–27 集 seed；主要增长来自全目录。减小集数不能继续降低这一固定开销。正在验证有显式版本标记、能完整反解的运输表表示；不提高预算、不裁原文、不按名字合并资产、不重写旧请求。

工程补跑改用优化构建补齐未覆盖单例；原 debug 分组超时/失败日志保留，跨二进制单例覆盖不会表述为“一次全组通过”。

### lite 接线调查（不是已接入声明）

现有 Desktop series 的候选是 `series-episode-candidate/v1`、selection 数组和绝对 UTF16 对白坐标；lite 是 `performance-core-v4`、`selection.selectedIds`、lineId 与镜内声窗。后续采用独立 `pipeline::series_lite`，不能改名后互认。

- 真正新任务在 engine 的 `production_input_for_new_workflow_with_capabilities` 分流；旧任务继续原 snapshot。
- storyboard 运行/交付验证先识别新 profile；text executor 的父合同与 `executionRoleId` 独立绑定。
- 新原稿、源锁、审核和采用保留原字节与 hash，同事务投影到既有 units/assetRefs；不反向二次导入 MD 复制整套资产。
- 新生产必须有完整 standard-episode-v1 证明、独立源锁、registry、完整视觉基准；不能删除协议后借历史 raw-row gate 放行。
- 每个 Desktop 稳定资产形象（assets.id + variantId）映成一个永久 A 编号。独立映射高水位/墓碑、设计版本与 registry revision，不借用随图片/提示词变化的 assets.version。
- 原 `series_export` 的 designRevision→version fallback 不进入新 lite 账本。已有图和手写词不能冒充 TEXT_ADOPTED；同名不同身份不合并。
- 身份关系、文字设计依赖与实际媒体依赖分别投影；同版用途用 record-uses 追加，不改 assetRevision。
- 规则角色需完整 creative-core/schema/岗位；系统类项目还须按冻结 job-packet 协议加载 system-presentation，不能漏供。
