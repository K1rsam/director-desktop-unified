# Director Desktop：CLI + 官方 Codex SDK 双入口并存实施方案

- 文档日期：2026-10-06
- 代码基线：`K1rsam/director-web`，提交 `5c10d315289862e8c452cb283e6df97616af57e5`，范围 `director-desktop/`
- 交付用途：供本地 AI 在获得实施授权后逐项执行的 Markdown 方案
- 当前状态：只读代码审查与方案编写；没有实施迁移、修改产品代码、安装运行时、启动模型生成、消耗用户额度、修改生产数据或发布程序

## 1. 结论先说

用户最新要求是：**CLI 不移除，新增 SDK 入口，两者可选并存，实际测试后再决定以后保留谁。** 因此，本次实施目标不是替换或删除 CLI，而是在保留 Rust + Tauri + React + SQLite 业务核心的前提下，增加一个真正调用官方 Codex SDK 的薄 sidecar，并让两条入口复用同一套任务、校验、缓存、恢复和发布契约。

建议产品设置明确显示“Codex CLI”和“官方 Codex SDK”两个入口。既有项目仍默认 CLI；用户可以为新工作流选择 SDK。每个 job 永久记录实际 backend，不因用户后来切换设置而改道。用户将来决定删掉哪条路线，不构成本次删除任何入口的授权。

官方目前同时提供 TypeScript `@openai/codex-sdk` 与 Python `openai-codex`。Python SDK 的 app-server 路径和固定 Codex runtime 依赖在当前官方文档中较清楚，因此可作为原型首选候选；但必须先验证锁定版本的结构化输出、取消、恢复、隔离及 Windows 打包能力。若团队更希望复用 TypeScript，则按同一门槛采用 TS sidecar，不能凭过去记忆假定它当前的底层 transport。**SDK 不等于不需要 Codex runtime，也不等于获得额外配额或把 80 集自动并发完成。**[O1]

Rust 自写 app-server adapter 在技术栈上更直接，但它不是官方 SDK，不能改一个标签充当用户要求的 SDK 入口。本方案保留它为明确列出的可选工程路线；除非用户随后选择，不能用它替代本次官方 SDK 双入口交付。

截至本次核查，官方虽提供 app-server 的稳定 API 子集，但仍把 app-server 命令标为 Experimental，未承诺生产工作负载支持。Python SDK 包的稳定发布与底层命令成熟度是不同层次，风险仍须披露。因此 SDK 通道先显式启用、固定版本并验收，CLI 同期保持可用；不能接受相关风险时继续使用 CLI。[O2][O3]

对本项目，更大的潜在收益在于：

1. 让任务的开始、进度、完成、取消、恢复有清楚的协议与数据库映射
2. 消除“只是等待结果，却又发出一遍生成”的重复消费风险
3. 把真正独立的工作放进有上限的队列，同时保留同集和跨集连续性依赖
4. 让 UI 显示真实排队与阻塞原因，而不是把没有 stdout 的长推理误判为卡死

不能承诺本次迁移会让模型推理变快、降低单次 token 消耗、提高账号限制，或让 80 集达到某个完成时间。需要在相同模型、推理强度、输入、缓存状态和账号条件下测量。

## 2. 名称与能力边界

| 方案 | 实际含义 | 对本项目的意义 | 不能混淆之处 |
|---|---|---|---|
| Codex CLI / `codex exec` | Codex 本地运行时的非交互任务入口 | 目前实际生成通道，已有 JSON 事件和输出 Schema | 已经能被 Rust 直接调用，不需要终端 UI |
| Codex TypeScript SDK | 在 Node 服务端程序中控制本地 Codex 任务的官方库 | 可做独立 sidecar，Node ≥18 | 不能在 Tauri WebView 里当普通浏览器 SDK 用；底层实现须按选定版本核实 |
| Codex Python SDK | Python ≥3.10 的官方库；控制本地 app-server，发布包带固定 runtime | 可做 Python sidecar，有同步/异步入口 | “稳定版 SDK 包”不自动消除底层协议、平台打包和恢复边界 |
| Codex app-server | 本地 Codex 的双向客户端协议；线程、轮次、事件、登录、审批等 | Rust 可直接通过 stdio JSONL 对接，无须语言桥接 | 它是 Codex 协议，并非 MCP；稳定 API 子集不等同产品生产支持 |
| MCP | 向 agent 暴露工具或资源的协议 | 以后有明确工具需求时才考虑 | 不是文本并发加速器；旧 `codex mcp-server` 与独立二进制已被官方移除 |
| OpenAI API SDK | 直接调用 OpenAI API 的语言客户端 | 是另一个 provider/backend 方案 | 不是 Codex SDK；认证、计费、会话与 agent 执行循环不同，不能透明替换 |
| OpenAI Agents SDK | 构建 agent 编排的框架 | 本次不是必要依赖 | 不等于本机 Codex 的客户端 SDK，也不替代本项目的领域工作流 |

这里的“迁移”限定为 Codex 文本执行通道。现有图片、视频供应商不因此迁移。OpenAI API 直连若以后单独采用，另立计费、密钥与契约迁移任务，不能作为 Codex 登录失效时的无提示兜底。[O1][O3][O7]

## 3. 已核实的工程现状

下表是本次读取基线源文件得到的事实，不是待实现功能。源码链接均固定到上述提交。

| 位置 | 当前行为 | 迁移影响 |
|---|---|---|
| `src-tauri/src/text_cli.rs` | 查找原生 Codex 程序，显式路径无效时报错；将 npm wrapper 尽量解析为原生 binary | 保留路径解析、可见版本、不得静默换程序的约束 |
| 同文件 `RpcClient` | 已启动 `app-server --stdio`，执行初始化，读取账户、模型、额度，并执行官方网页登录 | 本工程已经有 app-server 控制面；无需从零设计登录 |
| 同文件 `read_status` / `resolve_settings` | 检查 exec 所需 flags，45 秒路径绑定缓存；固定实际模型、推理强度、程序与版本 | 新通道必须有独立的兼容性预检，不能沿用“exec flags 全部存在”作为 app-server 可用证据 |
| `text_executor.rs::cli_arguments` | `exec --ignore-user-config --ignore-rules --ephemeral --sandbox read-only --skip-git-repo-check --json`；传模型、Schema、结果文件及 stdin prompt | 现在不是交互式 CLI；每个请求是隔离的短生命周期进程；不持久保存 Codex 对话 |
| `CLI_CONFIG` | 关闭 shell、web、apps、plugins、hooks、memories、多 agent、图像及其他工具发现；使用受限执行目录 | 新协议必须证明隔离等价，不能只改启动参数 |
| `supervise_cli` | 同时处理 stdin/stdout/stderr；整体 1800 秒，stdin 60 秒，排空/清理各 5 秒，心跳 30 秒，安静 120 秒仅作诊断 | 迁移不能退回容易死锁的“先写完 stdin，再读 stdout” |
| Windows I/O | 自有 OVERLAPPED named pipes、受控清理和子进程登记 | 这些是已经修复的生命周期保障，不能被简单 sidecar 示例丢掉 |
| `SavedTextExecutor` | 冻结请求、计算 hash、保存 text job、事件与结果；领域层之后才 accept/reject | 该层应继续掌管持久化和验收，transport 只提供执行事实 |
| `workflow_steps` | 保存 workflow、node、stage、request hash、text job 和 checkpoint | 优先扩展，不另建一套互不相通的任务系统 |
| `store.rs::recover_interrupted` | text 的 submitting/received 被转为 interrupted；普通继续可能重新执行，另有部分特定回包重放保护 | 新通道必须区分“未发出”“已发出但未知”“已收到待校验”，不能统统失败后重发 |
| 全局文本槽 | `TEXT_SLOTS` 实际硬编码 `Semaphore::new(4)` | `textConcurrency` 不能只存设置，必须成为真实统一预算 |
| 并发设置 | `store.rs` 默认含 `textConcurrency:4`、`episodeConcurrency:2`；`text_cli.rs` 回传常量；本次检索未发现阶段 1/3 用这些值调度 | 不能把设置面上的数字写成“已实现两集并行” |
| 阶段 1 | F2 批次顺序合并到共享 foundation；之后 Q1 继续按批修改 | 每一批输入可能依赖前批输出，不可直接并行 |
| 阶段 2 | 图片提示词设计已有按直接父形象依赖的 in-flight 调度，仍受全局文本槽约束 | 保留父结果校验通过后才放行子项的规则 |
| 阶段 3 | 分集顺序推进；使用前集尾态、主创批次依赖、已接受连续状态；整编在相关主创完成后进行 | 80 个 episode 不天然构成 80 个独立任务 |
| `engine.rs` | `pipeline:{projectId}` 排他锁；后台执行拥有独立 runtime；恢复入口仍冻结原工作流快照 | 不要删除排他锁后直接并发写项目；需在协调器内部有限并行 |
| `workflow.rs` | 发布前核对 source/foundation/assets/units 版本；整集发布和运行回执同事务 | 必须保留程序主导发布与版本保护 |
| React | `src/api.ts` 通过 Tauri IPC 访问 Rust，`Workspace.jobs` 与 `workflowProgress` 驱动 UI | 当前 desktop 范围未找到 `jobsStore.ts`；不要按旧 Web 后端假设搬运一个 jobsStore |
| Windows 分发 | 目前 resources 包含 dreamina 与 ffprobe，未列 Codex；日常更新固定当前 EXE 与一个 previous EXE | runtime 分发与 app 更新要新增明确配套，不应要求用户手装 Node/Python 才能打开桌面应用 |

定位入口：[C1][C2][C3][C4][C5][C6][C7][C8][C9][C10][C11]

## 4. 双入口架构与明确的路线选择

### 4.1 主交付：现有 Rust 核心 + CLI + 官方 SDK 薄 sidecar

```text
React：Codex CLI / 官方 Codex SDK 两个可选入口
  -> Tauri desktop_request / 受控状态事件
  -> Engine / Workflow coordinator
  -> SavedTextExecutor（快照、缓存、持久化、验收回执）
  -> TextTransport
       -> CliExecTransport -> 原生 codex exec（保留）
       -> OfficialSdkTransport -> 私有 stdio RPC
            -> 官方 SDK sidecar（Python 或 TypeScript 二选一）
            -> 所选 SDK 的固定/明确定位 Codex runtime
       -> ExistingApiTransport（已有独立 provider，保持现状）

SQLite + text-runs 是业务事实来源
Codex thread/turn 是执行证据和恢复来源
模型和 sidecar 不直接写生产数据库、项目成果或媒体目录
```

这是增加一个执行桥，不是把业务后端迁成 Python/Node。sidecar 不接管集场解析、规则选择、业务验收、计费决定、任务唯一性或成果发布。它只将固定版本官方 SDK 的真实能力转换为本项目的传输契约。

第一版可采用有上限的暖 worker 池，每个 worker 同时只拥有一个活动生成 turn，worker 数受全局文本预算限制。若 SDK 的实际运行方式不是常驻 app-server，则如实记录每次启动成本，不宣称有暖进程收益。

跨 worker 可同时执行独立 thread；每个 thread 同时最多一个活动 job。取消失败时只清理 Director 拥有的对应 worker/子进程，避免一个共享进程被杀导致其他任务同时失联。以后是否一进程多活动 thread 是独立优化，需要完整路由、取消隔离与崩溃影响面验收。

### 4.2 SDK 语言选择与 Rust 可选路线

| 选择 | 长处 | 新增负担 | 本方案地位 |
|---|---|---|---|
| Python SDK + Python sidecar | 当前官方稳定包、异步入口、明确使用 app-server、发布包固定 runtime | Python 打包、Windows 架构与 helper 分发；Rust↔Python IPC 与双层进程监督 | 原型首选候选，完整能力/分发门槛通过后才能锁定 |
| TypeScript SDK + Node sidecar | 与现有前端语言一致，官方程序接口，Node ≥18 | Node + SDK + runtime；须验证所选版本底层 transport、取消/恢复和配置隔离 | 若团队明确优先 TS 或 Python 能力不满足，选此路线；不是静默运行时 fallback |
| Rust app-server adapter | 延续 Rust，无额外语言 runtime，已有控制面可复用 | 自维护协议与兼容性，承担 Experimental 风险 | 可选第三路线；必须明确标“app-server 原生适配”，不能称官方 SDK |
| 现有 CLI exec | 最少变化，保留已验证隔离与测试资产 | 每次进程启动，会话级恢复较弱 | 一直保留为用户可选择入口，不只是隐藏的应急开关 |

不要同时实现两种语言 sidecar 来增加维护面。P0 完成一次有证据的 ADR，锁定一种 SDK 包、版本、runtime、目标平台和接口覆盖；另一种是文档备选。若两种官方 SDK 都不能满足关键隔离/恢复要求，就报告 SDK 入口被阻断，保留 CLI，不悄悄改用自写 Rust 适配器冒充完成。

两个入口统一显示模型、推理强度、账号状态和任务结果，但能力差异如实暴露。CLI 生成失败或 SDK 失联时，**不自动跨入口重发**。只允许对尚未派发的新工作流切换，或在确认原执行未被受理/已明确终止且用户批准重试后创建新 attempt。

## 5. 不可破坏的契约

1. 保留 `TextExecutor::execute/accept/reject/publish_episode/continuity_changed` 的职责，不让 SDK 的“完成”直接变成业务成功
2. JSON Schema 通过只是运输格式成立；原文忠实、对白归属、资产身份、范围、覆盖、连续性、时长和完整整集交付仍由 Rust 领域校验
3. 原始回包、标准化值、领域采用值分开保存，禁止为了让校验通过修改原始证据
4. `foundation-compact`、storyboard transport、strict optional 字段还原及规则快照继续使用原有逻辑
5. 模型、推理强度和 provider 不静默替换；换账号、换 transport、换 runtime 也必须可见并记录
6. 不把已拒绝结果视为普遍可复用缓存；现有特定修复回放规则必须独立回归
7. 不把新 transport 的完成状态覆盖旧 job 的历史；不重写旧快照与 accepted hash
8. 图片/视频提交、未知提交保护与恢复查询保持原有语义
9. 任何外部程序都不得直接改生产成果；只有 Rust 在版本验证后事务发布
10. 不因迁移而修改原文解析、对白拆分或创作质量标准；这些若要调整，单独验收

## 6. 新的应用内接口与身份模型

以下是**建议的应用内部契约**，不是可直接照抄的官方 JSON-RPC 类型。实现时以锁定 Codex runtime 导出的 Schema 建立 wire DTO，不假定文档上的全部字段都存在于用户机器。

```rust
// 示意；沿用项目现有 Result 和异步风格
trait TextTransport: Send + Sync {
    async fn inspect(&self, policy: &RuntimePolicy) -> Result<TransportCapabilities>;
    async fn execute(&self, attempt: &FrozenAttempt, events: EventSink)
        -> Result<TransportOutcome>;
    async fn reconcile(&self, attempt: &PersistedAttempt)
        -> Result<RecoveryObservation>;
    async fn cancel(&self, attempt: &PersistedAttempt)
        -> Result<CancelObservation>;
}
```

### 6.1 必须分开的五类 ID

| ID | 含义 | 生成/保存时机 |
|---|---|---|
| `workflowId` | 用户启动的阶段/批量制作流程 | 沿用 jobs 中 workflow 主记录 |
| `logicalNodeId` | 某阶段、集、scope、batch 的逻辑工作 | 由冻结业务身份确定，不能只用集号 |
| `attemptId` | 某次具体执行尝试 | 真正准备执行前事务创建 |
| `threadId` / `turnId` | Codex runtime 的会话及轮次身份 | 收到响应或相关事件后立即持久化 |
| `rpcRequestId` | 当前连接内的请求关联号 | 每连接单调递增；不当成服务端幂等键 |

建议第一阶段 **每个逻辑节点的执行 attempt 使用独立、可持久恢复的 thread（显式不采用 ephemeral）**。F2、Q1、Plan、Create、Finish 和 repair 不混用一个全剧长对话。这样保持现有自包含请求语义，避免前一岗位/前一集的隐藏对话内容绕过冻结输入和 hash。需要恢复时连接原 thread 查证；不能用一句“继续”隐式增加一轮生成。对应 runtime、CODEX_HOME 和会话存储必须持续保留并可重新定位；SDK 未暴露所需操作或只支持 ephemeral 时，状态明确标为不可持久恢复，不得通过营销标签宣称支持。

### 6.2 冻结请求与版本

新记录至少包含：

- `projectId`、`workflowId`、`logicalNodeId`、`stage`、`skill`、`episodeIds`、`scopeHash`、`batchId`
- `sourceVersion`、`sourceSha256`、`foundationVersion`、相关 asset/unit 版本
- 前置节点的 accepted artifact hash、连续边界 hash、决定/选择版本
- 规则内容 hash、业务 schema 版本、运输 profile 版本、validator 版本
- 原始 `TextRequest`、规范化 prompt/schema 的 hash
- `transportKind`、adapter version、runtime version/hash、protocol schema hash、有效权限策略 hash
- 模型、推理强度、非秘密 account identity/认证模式指纹

保留旧 `requestHash` 计算与旧 cache lookup 的解释，不在本次顺便换全局 canonical JSON 算法。新契约用独立版本名，如 `text-attempt/v2`。若以后统一规范化 hash，应做单独有证据的迁移。

`semanticRequestHash`（业务输入）与 `executionFingerprint`（实际模型/权限/运行时）分开。接受结果的复用需要明确兼容规则；不能仅凭业务 hash 相同，就把不同模型或权限下的结果跨通道自动采用。旧 workflow 默认保持其原 transport，只有显式允许的已验收回放可零调用复用。

## 7. 官方 SDK 桥接与底层 app-server 契约

### 7.0 SDK sidecar 必须先做的工作

- 新建独立 `sdk-bridge/` 目录，依赖官方 SDK 包并固定版本；不把 SDK 加到 React 浏览器 bundle
- Python 候选使用官方 `openai-codex`，异步入口采用实际版本的 `AsyncCodex`；TS 候选使用 `@openai/codex-sdk`。本方案不虚构未核验的 cancel/stream 方法名
- 定义应用自己的 bridge protocol v1：`hello`、`inspect`、`execute`、`observe`、`cancel`、`shutdown`；这是 Director 协议，不是官方 SDK 方法
- 每条消息带 protocolVersion、requestId、attemptId；事件带 threadId/turnId（SDK 可提供时）、类型和无秘密的结构化数据
- `hello` 返回 bridge/SDK/runtime/protocol 版本和能力；关键能力缺失时 UI 显示 SDK 不可用，不自动借 CLI 完成
- SDK 的输出事件、最终结果、异常、usage 映射到统一 TextTransport outcome；最终业务 JSON 仍由 Rust 验证
- SDK 自带自动重试若涉及可能重新发起生成，必须核实并限制；无法证明幂等的断流/超时不得由 bridge 直接再 run
- 原始输出可通过有界 IPC 返回 Rust，由 Rust 写入 attempt 目录；sidecar 只拥有专用运行目录，不能写项目数据库
- 需要 SDK 下层协议能力时，应先寻找该版本官方暴露入口。若必须自己实现大量 app-server 客户端，暂停并说明已偏离“薄官方 SDK 桥”的范围，不能假称全部由 SDK 提供
- Rust→sidecar 与 sidecar→Codex 两层进程都要有 ownership、超时与退出回收；重启桥后先对账原 attempt，不创建新生成

以下 app-server 部分描述 Python SDK 已确认采用的底层语义，以及可选 Rust 原生路线的实现依据。采用 TS 时必须核实其真实底层，不强行套用不适用的方法。能否通过 SDK 暴露相应功能，也是 P0/P2 的能力门槛。

### 7.1 版本锁定与握手

1. 本次官方 changelog 可见 CLI `0.160.1`；仓库历史文档记载的真实试验是 `0.160.0`。两者都不能证明用户电脑当前版本，也不能把某个版本直接宣布为本项目兼容版本。[O4][C12]
2. 实施第一步只做无生成的版本/帮助/Schema 采集，记录平台、架构、binary SHA-256 和协议快照
3. 从所选 runtime 生成协议 JSON Schema/类型资料，固定到版本化目录；用 fixture 测试所需字段
4. 原有 `app-server --stdio` 与当前文档示例 `--listen stdio://` 的支持情况按实际目标版本确定，不混抄
5. `initialize` 成功后再 `initialized`；未完成握手不得发送业务调用；默认不启用 experimental API 字段；SDK 是否允许配置该能力须由目标版本核实
6. 新未知 SDK/runtime 不自动覆盖已验证版本；返回兼容性阻断和用户可理解的提示

### 7.2 必需协议能力

最低能力集合：账户与模型只读检查、线程创建/读取/恢复、轮次开始/中断、结构化输出、轮次与 item 事件。协议边界应把以下方法与通知纳入版本化 fixture：

- `initialize`、`initialized`
- `account/read`、`model/list`、`account/rateLimits/read`
- `account/login/start`、`account/login/cancel`、登录完成通知
- `thread/start`、`thread/read`、`thread/resume`
- `turn/start`、`turn/interrupt`
- `turn/started`、`turn/completed`、相关 item 完成及 agent message 事件

官方接口存在这些概念，但准确字段/是否稳定必须由目标 Schema 验证。`outputSchema` 应逐 turn 显式传递，不能认为它像部分设置那样持续到后续 turn。[O2]

### 7.3 I/O 与事件路由

- 一条专用 stdout reader 持续读取；writer 通过有界队列串行写入；stderr 独立持续排空
- 区分 response、notification、server request。不能只等“下一个 id 匹配的响应”而丢弃中间事件
- `connectionEpoch + rpcRequestId` 关联响应；`threadId + turnId + itemId` 关联生成事件
- 迟到的旧连接响应不写入新 attempt；未知 thread 的事件隔离为诊断，不猜归属
- 设置单帧、总输出、事件队列与日志磁盘上限。暂建议单帧 8 MiB、单 attempt 原始输出 64 MiB，超限为可见错误且保留已落盘证据；这只是可调初始工程值，不是 Codex 官方限制
- 持久化关键状态后再发布 UI 事件。token delta 可合并显示，不必每个 token 做 SQLite 事务
- 展示用事件采用白名单投影。原始内容只进入受控本地证据，不把 prompt、登录链接、token、文件绝对路径或完整剧本塞进心跳
- 新增 `text_runtime_event` 时兼容当前 `workflow_progress` 对 `codex_heartbeat` 的查询；不得切换后使运行状态全部消失

### 7.4 输出处理

1. 按 thread/turn 收集准确的最终 assistant 结果，不能把所有 commentary/delta 直接拼成业务 JSON
2. 仅 `turn/completed` 且其 status 为 completed，才进入“运输完成”分支；failed/interrupted 不是成功
3. 保存原始最终文本与完成证据，再解析 JSON、恢复运输别名/optional 字段
4. 调用现有领域 validator，成功后才 accept；再由发布事务核对版本并采用
5. 先收到最终文本但没有终态时，保存为待对账。先收到终态但缺少完整最终文本时，查询原 thread；不自动追加生成
6. SDK `run()` 返回或 CLI 退出码 0 同样不能绕过领域验收

## 8. 权限、登录、配置与账号额度

### 8.1 最重要的阻断项：配置隔离等价

现有 exec 依赖 `--ignore-user-config` 和 `--ignore-rules`。这些不能假定是 app-server 参数。不要将 CLI_CONFIG 不加验证地复制为一串 `-c` 后声称完成隔离。

实施时优先验证以下方案：

- Director 管理的独立、持久 `CODEX_HOME`，不读取/复制用户已有 auth 文件
- 由官方登录流程在该运行时中完成认证；若该平台使用系统凭据存储，其具体账户作用域需要实测
- 只加载版本固定、明确允许的配置；禁止继承用户的 MCP、plugins、hooks、skills、memories、自定义 provider 与项目规则
- 每个请求固定独立 cwd；模型无项目写权限，工具和读文件能力按最小范围限制
- runtime 可正常进行所需模型网络通信，不要误把“工具网络关闭”实现成无法连接模型服务
- 纯文本通道意外出现 approval、permission、MCP elicitation 或 tool request 时 fail closed：记录 `policy_violation`，按已验证的 SDK decline/cancel 路径拒绝并结束/隔离该任务，不自动批准、不接受 acceptForSession、不默默等死；无法确认结束则按 unknown。`approval_policy=never` 不是授予全文件/全工具权限
- 系统/组织强制策略不能绕过；无法满足时停止新通道并显示原因

`--strict-config` 若目标版本支持，可用于发现无效配置键，但它不能证明忽略了其他配置来源。`read-only` 也不能单独证明没有敏感文件读取或外部工具调用。fake/污染 fixture 只能证明本应用的路由与配置构造，不能证明真实 runtime 没有加载其他配置。另需在固定的真实 runtime 上，以合成独立 HOME/配置、无用户凭据、断网且不发送模型 prompt 的方式检查配置来源、实际加载能力和隔离效果；只能验证到的范围要记录证据。若相关能力无法在不推理时充分验证，则保持 SDK 通道受阻，或在用户明确授权的受控验收中补齐，不能将 fake 通过宣称为隔离已证明。权限等价证明失败时不启用 SDK 生成。

### 8.2 登录与秘密

- 复用官方账户和浏览器授权流程；密码、token、auth 文件不经过 React、日志、Git 或迁移文档
- CLI 与 SDK 的状态/登录/模型/额度必须针对各自实际 pinned runtime 和实际 CODEX_HOME 检查；不能把外部 CLI 已登录直接显示为 SDK 已登录
- 只显示需要的账户摘要、登录状态与额度窗口，不扫描认证目录来推断身份
- 引入独立运行时可能需要重新登录，应明确告诉用户，不能后台复制凭据
- 只取消 Director 自己发起的登录等待，不注销或清理用户其他 Codex 会话
- 账号切换时暂停新派发；旧 attempt 必须按原账号/认证模式对账，不跨账号恢复
- 非 ephemeral SDK 会话会新增持久 prompt/回复历史，可能包含剧本文本；启用时明确告知存储位置类别、恢复用途和保留策略。默认只在本机受控目录保存，不自动上传遥测，不扫描或复制其他 Codex 会话
- 历史保留/清理作为显式产品操作，不把删除会话当作退出清理；未知执行和可恢复记录在处理完前不自动清除，删除行为遵循用户授权
- 本次迁移不默认引入自己的 OAuth client、token exchange 或 Sign in with ChatGPT 集成；若要做，那是新增认证边界和产品合规任务

### 8.3 额度与限流

- 模型目录是目录，不是执行权保证；保留“额度未知”的状态，不将未知显示为 0，也不推算能跑多少集
- 全局并发应按账号/认证模式共享预算，不让不同项目各自启动四项绕过总上限
- 429/额度不足：停止新派发或降速，展示返回的限制/恢复时间；不循环创建新 thread
- 用户明确选定模型不可因限流改模型或改为 API 付费
- 配额检查不得通过隐藏生成请求完成；真正验证模型权限属于明确授权的 live test
- 从服务得到的 usage 若可用则按 turn 记录；价格、额度金额未知时保留未知，不自行承诺成本

## 9. 持久化、恢复与避免重复执行

### 9.1 建议新增的存储

继续以现有 SQLite 为事实来源，可新增 `text_attempts` 表而不改变旧 jobs 的历史字段含义：

```text
text_attempts
  id, text_job_id, workflow_id, logical_node_id, attempt_no
  request_hash, execution_fingerprint, transport_kind
  runtime_identity_json, account_fingerprint, connection_epoch
  thread_id, turn_id, rpc_request_id
  state, dispatch_intent_at, dispatched_at, acknowledged_at
  terminal_at, cancel_requested_at, cancel_generation, last_observed_at
  raw_result_path, raw_result_sha256, validation_state
  error_code, retry_after, created_at, updated_at
```

另用版本化 `text_attempt_events` 保存关键事件序号/摘要；或扩展现有 events 并建立相应索引。`next_attempt_at` 已存在，但文本恢复不要冒用图片/视频 task_id 的语义。

为同一 workflow/node/request/attempt 建唯一约束；并发点击通过事务 CAS 领取，不依赖 UI disabled 或随机 entity_id。兼容旧数据库采用前向、可重入迁移；迁移前用 SQLite backup API 做一致备份，不能直接复制正在使用的 WAL 数据库文件。

### 9.2 状态机

```text
queued -> preparing -> ready_to_dispatch
  -> dispatching -> running -> output_received
  -> validating -> accepted -> published

旁路终态/等待态：
failed_before_dispatch / execution_unknown / auth_required / rate_limited
cancel_requested / cancelled_confirmed / transport_failed
validation_rejected / stale_input / waiting_dependencies / paused_by_user
```

`accepted`、`published` 是业务状态，不能与 runtime 的 completed 混为一谈。为兼容既有 Job.status，可以将细粒度状态放 attempt 中，再明确投影到 UI，不要未经统一迁移把新状态散落到各处正则判断里。

### 9.3 发送的正确顺序

1. 事务冻结请求、创建 attempt，写 `ready_to_dispatch`
2. 创建并持久化 thread identity；失败但未发 turn，可安全重做本地准备
3. 事务写 `dispatch_intent`、连接 epoch 与请求关联号，再向 stdin 写 `turn/start`
4. 一旦可能写出任何字节，就不能默认“没有执行”
5. 收到 response 或可归属的 turn-started 事件，立即保存 turn ID 和 running
6. 原始结果与 terminal 证据完整落盘后进入 output_received
7. 离线领域校验和发布可以安全重试，不需重新调用模型

本地数据库与远端模型执行之间没有跨系统原子事务。`rpcRequestId` 不是官方推理幂等键。某些协议版本中的 `clientUserMessageId` 也不能当作已有 exactly-once 保证；没有官方明确语义时最多作相关性标识。因此只承诺“防止已知重复、对不明结果保守处理”，不宣称 exactly-once 模型执行。

### 9.4 恢复决策表

| 崩溃/中断位置 | 恢复动作 | 可否自动新发生成 |
|---|---|---|
| 未形成 dispatch intent | 重做本地准备 | 可按原用户任务继续 |
| 已有 thread，尚未发送 turn | 读取并验证该 thread 无冲突后继续 | 仅有确切未发送证据时可 |
| 已发/部分写出 turn，尚无 turn ID | 按保存 thread 查询历史和运行状态；若无法唯一绑定，execution_unknown | 不可 |
| 已有 turn ID，进程连接断开 | 重建连接，读取/恢复原 thread，查原 turn 状态和结果 | 不可先创建新 turn |
| 有完整 raw result，绑定原 turn 的 completed 终态证据，尚未领域验收 | 对原回包重新做确定性校验 | 不可为此新发 |
| 有 raw result，但没有可信 completed 终态证据 | 先对账原 turn；不能仅凭文件完整标 accepted | 不可 |
| 有 accepted，尚未发布 | 重新核对版本 guard 后事务发布 | 不可 |
| 原 turn 明确 failed/interrupted | 保留证据，按错误和用户授权选择重试或停止 | 不能仅因失败默认无限重试 |
| thread 无法读取、历史缺失或身份不符 | 显示需要处理，保留记录 | 不可 |
| 用户明确要求重新生成 | 建新的 attempt，绑定前次失败/未知说明，保留旧记录 | 仅在重复/额度风险已清楚时 |

`thread/resume` 是恢复会话上下文，不是“自动接着完成原推理”的保证；随后再 `turn/start` 是新生成。不能写一个名为 resume 的函数内部无条件调用 SDK run("继续")。

已知业务修复预算，例如现有最多两次 repair，必须持久化到原节点链，重启不清零，不用 transport retry 隐式增加创作次数。

### 9.5 重试与退避

- 只读账户/模型/线程查询可做有界指数退避，加 jitter，并尊重服务返回的 retry 时间
- schema 错误、权限阻断、模型不存在不做盲重试
- 明确未受理的提交才进入安全重试队列；不明提交先对账
- 传输层网络重试与业务创作修复分别计数，UI 分开解释
- 建议初始退避基数 1 秒，上限 60 秒；这是本地策略，不是官方配额承诺
- 达到策略阈值进入 paused/needs attention；没有服务端取消或终态证据时仍保留 unknown

## 10. 80 集调度设计：先证明独立，再并发

### 10.1 三个预算

- `globalTextConcurrency`：本应用同一账户的总活动生成上限，初始保留 4
- `episodeConcurrency`：同时处于执行中的独立 episode lane 上限，建议从 2 开始，但依赖链可能使实际并行为 1
- `perEpisodeConcurrency`：同一集内可证明独立的 ready 节点上限，第一版默认 1

设置必须经过合法范围校验并记录在 workflow snapshot；运行中调低只停止补充新任务，不任意杀死已有 turn。使用有界队列和公平轮转，不为 80 集一次创建成百上千个运行时/future/完整 prompt 副本。

两入口共用“已知活动 + 未知可能仍活动”的保守预算。`execution_unknown` / `cancel_pending` 不能因本地 pipe 关闭就当服务端槽已释放；保留未知占用，必要时暂停该账号新派发，直到终态对账或用户理解可能重叠执行/消费风险后明确解锁。释放本地 worker 句柄与释放服务端执行预算是两个不同动作，不能另一入口马上补满四项。

只在节点真正 ready、依赖 accepted、请求已冻结时占用文本槽。等待依赖、等待登录、等待人工决定、磁盘发布时不长占模型槽。禁止嵌套 semaphore 造成父节点拿着槽等子节点的死锁。

### 10.2 分阶段依赖

**阶段 1：F2/Q1**

当前 F2 会更新共享资产身份目录，下一批使用更新后的目录；Q1 也会连续修改同一 foundation。第一版保留顺序执行，不能按集拆成 80 条独立 F2。若以后要并发，必须单独设计“冻结基础目录上的候选提取 + 确定性身份归并 + 冲突处理 + 单写发布”，并证明与现有语义等价；这不是替换 SDK 顺便就能完成的改动。

**阶段 2：资产文字设计**

沿用已存在的父子形象 DAG。不同根资产或已满足依赖的兄弟节点可并发；父提示词必须已验收才可交给子形象。INLINE/DERIVE/AUDIO/UNRESOLVED 等现有语义不变，图片生成渠道保持独立队列。

**阶段 3：分镜文字**

每集至少保留：Plan → 相关 Create 批次 → Finish 分组 → 有界 repair → 整集验证 → 发布。

- `workPackages.dependsOn`、`dependencyExits`、`acceptedContinuity`、前一批尾态都视为真实依赖
- 连续前集最终尾态 → 后集 Plan 的边不能删除。上一集 repair 改变尾态时，后续受影响链要按现有 continuity revision 失效规则处理
- 同集所有作者完成后才能执行依赖它们的整编；不能因为某个 future 快先发布半集
- 独立性不是“episodeId 不同”。只有输入已冻结、不依赖尚未完成前集、无共享可变资源的节点才可同时 ready
- 若整部剧是一条连续链，跨集生成实际可能仍串行；UI 应显示“等待第 N 集已验收末态”，而不是伪报正在并行
- 要新增显式独立集/片段模式，属于新的业务规则与验收任务，不能用调度器默认假设替代

**阶段 4：视频**

本方案不改视频生成并发和提交恢复。分镜成果必须完整、版本匹配，才能进入现有素材与视频链。

### 10.3 写入隔离

- 原始证据按 `text-runs/<textJobId>/<attemptId>/` 保存，含 schema、冻结 request、raw final、事件摘要和验收回执
- runtime cwd 使用 Director 创建的 attempt 独立目录，不以仓库/项目根为 cwd；不并发覆盖一个 result.json
- 不用每个 episode 一个 Git worktree：本任务是生成结构化文本，模型无权改 repo，worktree 不能替代业务写入隔离
- SQLite 写事务短小；业务发布由单写协调器排序，模型等待不持有事务
- 保留项目级协调器排他锁，先在一次 workflow 内调度。不要直接允许多个 foundation/storyboard workflow 同时改同一项目
- 当前 publication guard 涉及全局 source/foundation/assets 与所选 units；并发发布必须串行读取最新运行回执中的 publicationGuard，在一个事务里验证和推进
- 外部用户编辑导致版本不符时，候选保留为 stale，不自动覆盖。不同集的并发发布不能相互错误地覆盖 prepared/result 的聚合状态

## 11. 超时、取消与退出

维持三种不同信号：

1. 本地进程/传输是否活着
2. 最近是否收到执行事件或结果
3. 服务端是否确认完成/失败/中断

没有文本输出不等于失败；本地 PID 存在也不等于模型仍健康。心跳记录应继续是无正文摘要。

- 等待槽位、握手、发送、模型执行、结果落盘分别计时；总 deadline 可保留当前 1800 秒作为初始值，但不把它解释为平台保证
- 点击取消后立即停止该 workflow 的新节点派发，并持久化 `cancel_requested` 和递增的 cancel generation
- 取消记录同时是 accept/publish 屏障。取消与 completed 竞态时，迟到成功只留候选和证据，不自动采用/发布；事务提交前校验目标 workflow/attempt 的 cancel generation。已在取消前完成的发布不撤销，取消后的候选只有用户明确恢复/采用才可重新验收提交
- 对已知 thread/turn 发送 `turn/interrupt`，等待明确 interrupted/其他终态；RPC 返回 `{}` 不足以伪造业务取消完成
- 确认不了终态时，关闭该 worker 的本地执行并记录 unknown/cancel pending；不能表示服务端从未产生消费
- worker 进程挂死才执行受控终止；Windows 只针对自有进程句柄/树，不用进程名全局杀 Codex
- 若采用共享多 turn 进程，强杀之前必须明确标记所有受影响 attempt，不能只取消一个 UI 行
- 应用退出先停止新派发，处理已有 turn 中断/持久化，再排空数据库写入与自有 I/O；沿用现有 shutdown guard 的竞态保护
- 父进程退出后的 orphan、PID 重用、stderr 洪泛、后代持有 pipe、磁盘满都要进入离线回归

## 12. 前端与任务状态

继续保持 Rust 数据库为源。不要新增一个只在 React 内存里拥有排队事实的 jobsStore。如果要抽取前端 selector/store，它只能缓存后端投影并通过事件序号/版本重读对账。

新增可见信息：

- 运行通道：Codex CLI / 官方 Codex SDK（注明 Python 或 TypeScript、版本）；原生 app-server 仅在用户另选该路线时独立显示；文本 API 保持原入口
- runtime 版本、兼容性结果、有效并发上限
- 排队、等待前置集、等待登录、受限流、正在生成、待核对、已验收、已发布、结果未知
- 本次已提交/已验收/缓存复用/失败/未知数量；总集数和节点数分开
- “继续原流程”“查询原执行”“取消等待/取消执行”“重新生成”分成不同操作
- 字段未知显示未知，不能把没有收到 usage 算成零成本

兼容现有 `Stage1Panel`、`Stage3Panel`、`WorkflowProgress`、TasksPanel、`jobActive` 与 `generationState`。新增状态必须一并更新 active/blocking 判定，避免“unknown 被当作空闲可重复点击”。

建议设置新增 `codexExecutionBackend: "cli" | "sdk"`，在 workflow 和 text job 中冻结，旧项目缺省为 cli。API provider 单独保留，不混入该二选一。切换设置只影响新任务，已有任务卡必须展示其实际 backend。新的 IPC action 使用 `textRuntime.status`、`textAttempts.reconcile`、`textAttempts.cancel`、`workflow.pause` 等名称，但保留 `textCli.*` 兼容委托一个版本。UI 不能直接调用 app-server 或拿到秘密。取消按钮只在后端能验证该用户操作目标属于当前 workflow 时生效。

## 13. 逐文件改动清单

以下均为未来实施建议，本次未修改这些文件。

| 文件/目录 | 计划变化 | 验收重点 |
|---|---|---|
| `src-tauri/src/text_executor.rs` | 保留 SavedTextExecutor 与历史 replay；抽出 CLI 传输/请求编译；接入 attempt store | 旧 accepted/rejected/repaired 回放不新增模型调用 |
| 新 `src-tauri/src/text_transport/mod.rs` | TextTransport、内部 outcome/capability/error 类型 | 明确提交前失败与结果未知 |
| 新 `text_transport/cli_exec.rs` | 移入现有 exec 监督实现，行为保持 | 所有管道、超时、退出用例回归 |
| 新 `text_transport/sdk_bridge.rs` | Rust 侧私有 bridge IPC、SDK 能力预检和 outcome 映射 | 真正使用官方 SDK；不偷偷改走 CLI |
| 新 `sdk-bridge/` | 选定 Python/TS 的官方 SDK 包、锁文件、协议实现和打包 | 干净 Windows 可运行，版本清单完整 |
| 可选 `text_transport/app_server.rs` | 仅用户另选原生路线时实现，不能代替 SDK | 准确标注自写 adapter |
| 新 `text_transport/protocol.rs` | Director bridge DTO 与 SDK 映射边界；可选原生 wire DTO | 未知字段可容忍，必要字段缺失阻断，不伪成功 |
| 新 `text_transport/runtime.rs` | CLI/SDK 两类进程、全局预算、worker ownership | 不泄漏、不越界杀进程、不绕过账号预算 |
| 新 `text_transport/policy.rs` | 冻结隔离配置、能力探测、配置指纹 | 与现有 text-only 权限等价 |
| `src-tauri/src/text_cli.rs` | 复用发现/登录，分开 exec 与 app-server 能力检查 | 状态查询和登录不发生成请求 |
| 新 `src-tauri/src/text_attempts.rs` | schema migration、CAS、恢复状态机和对账 | 崩溃后不盲重发 |
| `src-tauri/src/schema.sql` / `store.rs` | 前向迁移、attempt 元数据、摘要投影、恢复分流 | 老项目可读，旧历史不改写 |
| `src-tauri/src/workflow.rs` | progress 状态投影；协调器单写与 publication guard 更新 | 整集原子发布、旧版本不覆盖 |
| `src-tauri/src/engine.rs` | 统一 runtime 所有权、调度入口、取消/对账 IPC | 不在短暂 Tokio runtime 中启动需长期存活的服务 |
| `src-tauri/src/pipeline/mod.rs` | 尽量不变；必要时增加明确 execution context | 不把 transport 类型传播成全领域耦合 |
| `pipeline/foundation_delivery.rs` | 第一阶段迁移只换 transport，不改 F2/Q1 串行语义 | 共享身份目录一致 |
| `pipeline/assets.rs` | 现有 DAG 对接统一预算，减少过量 in-flight | 父验收前不派子项 |
| `pipeline/storyboard.rs` / `storyboard_dependencies.rs` | 后续独立阶段提取 ready DAG，保持 continuity 边 | 失败节点只阻塞真实后继 |
| `src-tauri/src/lib.rs` / `main.rs` | 注册模块/管理器，接入退出清理 | release 仍内嵌前端，自有 worker 排空 |
| `src/types.ts` / `api.ts` | DTO、兼容 action、投影序号 | 秘密不出后端 |
| `TextExecutionSettings.tsx` | CLI + 官方 SDK 选择、各自版本/能力/风险、真实预算与登录 | 不把预检当权限成功 |
| `WorkflowProgress.tsx` / `workflowProgressModel.ts` / `components.tsx` / `Panels.tsx` | 排队、unknown、cancel 与恢复入口 | 新状态不掉出阻塞集合 |
| `src-tauri/Cargo.toml` / `Cargo.lock` | 仅为 bridge 和调度增加确有必要的 Rust 依赖 | 不重写业务栈 |
| `package.json` / `package-lock.json` | 前端不依赖本地 SDK；若选 TS，在 sdk-bridge 独立 package/lock 固定依赖 | 不把 Node 模块打进浏览器产物 |
| `tauri.windows.conf.json` / `portable.rs` | 可选 runtime 资源发现与版本清单 | EXE/运行时版本成套、架构匹配 |
| `scripts/update-client.ps1` / `launch-client.ps1` | 原子配套版本选择、回滚 manifest | 不强关用户客户端、入口名不变 |
| 新 `tests/fixtures/codex/<version>/` | 协议与故障 fixture，不含真实秘密/剧本 | 可完全离线重放 |
| 新 `docs/CODEX_TRANSPORT.md` / `docs/verification/codex-transport/` | 兼容矩阵、测试报告、回滚操作 | 准确标注 tested / not tested |

拆分超大 `text_executor.rs` 时先机械移动并保持测试通过，再改变行为，避免一次同时重写缓存、运输和创作流程。

## 14. Windows runtime 分发与更新

### 14.1 两种明确模式

- **外部 runtime 模式**：用户选择现有原生 Codex 程序；校验版本/能力，不修改其安装，不在后台自动升级。实现成本低，但环境漂移风险较大
- **应用管理模式**：发行物携带固定版本官方 SDK、必要语言运行环境及 Codex runtime/helper 完整分发集，以及来源/哈希/许可证记录；不要求终端 npm/pip 安装。需先核实目标平台发行物及允许再分发的条件

新版本 manifest 至少记录：应用版本、transport adapter 版本、Codex runtime 版本与哈希、目标 OS/arch、协议 schema hash、配置 profile hash、下载来源和适用许可证。使用官方发行渠道与校验结果，不自动接受新的法律协议。不能假定只复制 codex.exe 就足够；平台 helper、runtime 依赖与许可证清单须按锁定 SDK 的发行物验证。当前官方 Windows standalone 安装路径与原代码发现路径也存在差异，见 [O8]，需扩展受控发现而不是扫描认证秘密。

### 14.2 交付约束

遵守仓库 AGENTS.md：外部入口固定 `director-desktop.exe`，只保留一个 `director-desktop-previous.exe`；候选在内部 deps；未被要求不要制作安装包；文件占用时等待用户自然关闭，不强杀客户端，不清理项目/数据库/媒体/凭据/CLI。

如果二进制不内嵌，单独轮换 EXE 不足以回滚 runtime。应增加当前/上一版配套 manifest，或采用版本化只读 runtime 目录并由每个 EXE 的兼容清单选择。旧 runtime 在没有使用中的 attempt 且仍有可回滚版本前不可删除。

针对 x64/arm64 分别验证。测试中文/空格/长路径、非管理员账号、无 Node/Python PATH、杀毒拦截、缺失/损坏 runtime、运行中更新、旧版程序重新打开新版数据库。不能用 Linux 测试代替 Windows 实机验收。

若选 TS/Python sidecar，要额外证明打包产物能在干净 Windows 上运行，并定位到携带的 Codex runtime；禁止首次启动偷偷联网安装依赖或自动获取持久权限。

## 15. 实施分期与退出标准

### P0：冻结基线与可复现证据

- 记录本地实际 commit、工作区改动、已有工具链；不得覆盖用户未提交改动
- 确认代码基线差异，读取仓库当前 AGENTS 和规则
- 收集现有离线测试基线；当前文档没有声称在本轮重跑这些测试
- 写 ADR：CLI + 官方 SDK 双入口，选定一种 SDK sidecar；CLI 保留默认，Experimental 风险及能力阻断条件

退出标准：源码与测试基线明确；只有方案/fixture 工作，不调用模型。

### P1：先抽象，不改变业务

- 将 exec 变为 `CliExecTransport`，原逻辑原样保留
- `SavedTextExecutor` 仍负责缓存、raw、accept/reject 和历史修复
- 引入版本化 attempt 记录，但默认仍走旧 transport

退出标准：旧离线测试全部通过；固定 fixture 输出/缓存/错误行为不变；无真实生成。

### P2：官方 SDK bridge 与离线协议原型

- fake bridge/fake app-server 子进程覆盖 handshake、事件、取消与失败；官方 SDK 包接口以所选版本验证
- 两入口共用测试契约，UI 切换仅改变新任务 backend；不能以一个 exec wrapper 冒充 SDK
- 加入生产版本固定与 schema/capability 检查
- 实现新配置隔离；分开 fake 配置构造测试与固定真实 runtime 的合成 HOME/断网/无凭据/无模型调用加载检查，不互相冒充
- 实现 Windows 等价 I/O ownership 与进程收尾

退出标准：无秘密读取、无模型请求的条件下，必要协议与隔离测试通过。任一隔离要求无法满足，停止推进默认切换。

### P3：持久恢复与 UI

- 完成 dispatch intent、unknown、raw/accepted/published 状态分离
- 所有崩溃窗口可离线回放；UI 明确展示不可自动重发的原因
- 新 action 与旧入口兼容；重启后无需 React 内存恢复事实

退出标准：发送不明状态不产生新的 turn/start；accepted 结果恢复零调用；取消与重启不重置 repair 预算。

### P4：有界调度，先不改领域语义

- 全局文本预算真正读取配置，公平队列与 bounded memory
- 复用阶段 2 DAG；阶段 1保持顺序；阶段 3先保留连续依赖
- 用 80 集离线 fixture 验证每集/阶段/版本映射及状态展示
- 在证明独立的节点上才启用 episode lane，初始上限 2

退出标准：最大活动 turn 不超预算；同集和跨集依赖无越序；整集发布不产生交叉覆盖；队列不通过“删边”获取速度。

### P5：用户明确授权的小规模真实验收

授权应明确：隔离测试项目、所选模型和推理强度、最多请求/预算、是否会发送剧本文本、是否允许必要登录或安装。首次使用合成短输入；通过后再用用户批准的单集样本。

先并发 1，再 2；确认恢复与质量后才测试 4。只比较相同业务输入和条件。不得一次用 80 集试运行，也不能因失败自动扩大预算。每次记录实际调用数、usage、通过率、P50/P95、首次结果与总时长。

退出标准：实测数据和限制公开；业务质量不退化；用户对两个入口分别实测，再选择新任务默认入口；不删除任一路线。新通道失败时不自动重投 exec。

### P6：受控发布与回滚验收

- 新项目/新 workflow 可以显式选择 CLI 或 SDK；旧 workflow 保留原 transport
- 验收 UI 中两条入口均可选择，结果明确标 backend；本阶段不移除 CLI 或 SDK
- 用固定当前 EXE 入口发布，保留上版 EXE + 配套 runtime/manifest
- 回滚开关只影响未来任务，运行中 attempt 先对账
- 发布不授权删除任一入口；之后仅在用户明确选定并授权删除时，另做历史恢复与分发兼容评估

退出标准：Windows 真机启动与回滚通过；旧项目、旧 accepted checkpoint、旧媒体任务仍可用。

## 16. 离线验收矩阵

| 测试组 | 必测场景 | 通过标准 |
|---|---|---|
| 协议 | initialize 失败、晚响应、通知穿插、未知字段、缺字段、UTF-8 分片、大帧 | 不串任务、不挂死、错误可解释 |
| 完成 | failed/interrupted 也有 final text；final text 先到；完成先到；重复通知 | 只接受完整成功终态且领域通过的结果 |
| 并发 | 80 集 fixture、4 槽、2 lane、连续链、独立枝、父失败、循环依赖 | 不超预算；ready 判定准确；无饥饿/死锁 |
| 映射 | 相同 episode 的不同 stage/batch/revision，两个项目同名集号 | 文件/数据库/thread 无碰撞 |
| 恢复 | 在 intent 前、写一半后、ack 前、raw 后、accept 后、publish 后强制结束 fake process | 每个窗口均符合恢复表，不发生隐式新生成 |
| 缓存 | 同请求重放、模型/规则/源版本变化、runtime 变化、既有修复政策 | 兼容复用有证据，变更不误命中 |
| 取消 | 排队取消、running 中断、拒绝中断、超时、completed 与取消竞态、其他任务仍运行 | 迟到成功不越过取消屏障发布；无误杀；无自动重投 |
| 未知占用 | 一个入口失联或中断未确认，另一个入口继续排队 | 未知占用计入共同预算，不自动补满或越过暂停 |
| 退出 | stdin 堵塞、stdout/stderr 洪泛、子孙持有 pipe、PID 复用、磁盘满 | 有界清理，证据保留，无 orphan 写回 |
| 权限 | fake 配置构造/意外 approval、permission、MCP elicitation、tool request；另做真实 runtime 合成 HOME 加载检查 | fail closed、policy_violation 可见，不默默等待；分别记录证明范围，未证明隔离则阻断 |
| 身份 | 账号切换、模型不存在、额度未知、429、登录取消 | 不换模型、不换计费、不伪配额 |
| 领域 | F2/Q1 身份冲突、范围漂移、对白遗漏、重复 source refs、repair 预算 | 仍由原 validator 拒绝，transport 不掩盖问题 |
| 发布 | source/asset/unit 被用户改、两集同时完成、重复完成回调 | stale 不覆盖；同一成果只采用一次 |
| 数据库 | 旧库升级、重复迁移、WAL 备份、只读/磁盘满、降级打开 | 不损坏旧历史，失败可恢复 |
| UI | 断线重连、旧投影乱序、unknown/cancel 状态、刷新后恢复 | 后端为准、无重复按钮行为 |
| Windows | 干净用户、无开发服务器、无语言 PATH、中文路径、runtime 缺失、更新占用 | 入口正常；缺失报明确原因；不强关用户 |

所有 fake/fixture 测试默认断网、无用户凭据、无真实 provider。真实 runtime 的离线加载检查单独报告，不能用假进程测试替代。测试统计必须分离“离线通过”“官方 runtime 本机协议通过”“真实模型通过”“Windows 实机通过”；任何一类缺失都要如实写未验证。

建议保留并扩展已有前端 `npm run test:frontend`、typecheck/build、Rust core 套件及 `verify-text-cli-ui.mjs`、`verify-stage-workbenches.mjs`。由本地 AI 依据真实工具链执行；本方案不要求为验证文档而安装工具链，更不允许绕过已拒绝的安装操作。

## 17. 性能测量与成功定义

先记录现状再优化：

- 排队等待、进程启动/握手、prompt 编译、首事件、首可见文本、模型总执行、校验、发布耗时
- 每 logical node 与每 episode 的 token/usage、实际请求次数、命中缓存次数、修复次数
- P50/P95、失败率、unknown 比例、取消确认时间、重启恢复率、峰值内存/进程数
- 同一剧本的原文覆盖、对白准确率、身份/连续性错误和人工返工数量

暖进程可能节省启动成本；协议化可能改善恢复与可观察性；DAG 队列可能提高独立节点吞吐。它们分别测量，不把网络、模型负载或缓存偶然差异都归功于 SDK。

可接受的首版成功：不退化质量与安全、可恢复、可解释、无已知重复发起，且在代表性样本上得到可重复的数据。没有测量就不写“提升 X 倍”。

## 18. 回滚与实施红线

### 回滚

1. 将新工作流默认入口改回 CLI（或临时关闭 SDK 新派发），保留两入口代码和历史
2. 现有 running/unknown attempt 继续按原 transport 对账，不能自动复制成 exec 新任务
3. accepted 结果按原 request/validator 证据复用；旧数据不降级重写
4. 如需程序回滚，使用 previous EXE 对应的 runtime 与 manifest；保持新增表可忽略、旧字段含义不变
5. 若旧程序无法安全理解新 schema，则使用迁移前一致备份与明确恢复流程，不能带着正在运行任务直接替换数据库

### 交给本地 AI 的强制约束

- 先读本机当前 AGENTS 和当前源码；本文件是方案，不是盲目覆盖更新代码的脚本
- 实施前确认授权范围；没有授权不改代码、安装依赖、启动生成、提交 Git 或替换 EXE
- 不读取、复制、导出或打印认证秘密；不把数据库、剧本、prompt、auth 或 token 上传到调试服务
- 不清理用户项目、数据库、媒体、凭据或现有 Codex 工具
- 不把 MCP 或 SDK 当作额外额度来源，不自动换 provider/账号/模型
- 不删除领域 validator、连续性依赖、未知提交保护来让测试变绿
- 不使用未验证的参数或虚构 SDK 方法；准确 API 来自固定版本 Schema/官方实现
- 不在同一改动里迁移文本传输、重写原文解析、调整创作合同和扩大媒体提交权限
- 不把本文视为优化 Web 底层架构的授权；如果任务扩大到该范围，按用户规则在行动前另行确认

## 19. 实施前必须回答的决策

1. 官方 SDK sidecar 最终选 Python 还是 TypeScript？建议先对 Python 做能力与 Windows 分发核验；偏好 TS 时按同样门槛验证，不重写 Rust 业务层
2. 使用用户已有原生 runtime，还是应用管理固定 runtime？建议开发原型先显式路径，正式分发再决策
3. 独立 CODEX_HOME 的隔离和官方登录是否在目标 Windows 版本下可用？未通过则不得迁移默认
4. 能否接受所选 SDK/底层 runtime 的成熟度与持久化隐私成本？不能接受时保持 CLI 可用；自写 Rust app-server 只有用户另选时才实施
5. 哪些 episode 实际无连续性依赖？没有证据的边保留，不预设 80 集可全并行
6. 真实验收允许用哪个隔离项目、模型、请求数/预算与文本范围？未授权前只做离线验收

## 20. 证据与来源

### 官方来源（本次访问：2026-10-06；实施时需重新核查）

- [O1] [Codex SDK](https://learn.chatgpt.com/docs/codex-sdk)：TypeScript/Python SDK、运行环境要求、Python 固定 runtime、线程续接接口概念
- [O2] [Codex App Server](https://learn.chatgpt.com/docs/app-server)：stdio JSONL、版本对应 Schema、线程/轮次、输出 schema、审批、取消与命令成熟度
- [O3] [Codex MCP server migration](https://learn.chatgpt.com/docs/mcp-server)：旧 MCP server 命令移除、app-server 迁移与 Experimental 提示
- [O4] [Codex changelog](https://learn.chatgpt.com/docs/changelog)：本次可见 2026-10-05 CLI 0.160.1；不代表本项目或用户机器已使用
- [O5] [Codex as a platform](https://developers.openai.com/blog/codex-as-a-platform)：exec、SDK、app-server 的产品集成定位
- [O6] [Codex CLI reference](https://learn.chatgpt.com/docs/cli/reference)：具体命令与选项，须配合实际目标版本帮助与 Schema
- [O7] [OpenAI SDK](https://developers.openai.com/api/docs/libraries)：OpenAI API 客户端属于独立 API 集成路线
- [O8] [Codex environment variables](https://learn.chatgpt.com/docs/config-file/environment-variables)：Windows standalone 运行环境与路径，需按固定发行物核对完整分发集

以上官方资料可能持续更新。本方案没有固定一个“自动安装的最新版”，实施时应保存实际采用的 runtime 版本、哈希、Schema 和来源回执。参数/能力在该版本不存在时阻断或保留旧通道，不凭文档更新静默改行为。

### 固定代码来源

- [C1] [text_cli.rs：程序发现、状态、账户和登录](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/text_cli.rs)
- [C2] [text_executor.rs：运输、隔离、超时、持久化与回放](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/text_executor.rs)
- [C3] [engine.rs：IPC、工作流执行与恢复入口](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/engine.rs)
- [C4] [store.rs：settings、jobs、恢复](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/store.rs)
- [C5] [workflow.rs：checkpoint、进度与事务发布](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/workflow.rs)
- [C6] [foundation_delivery.rs：F2/Q1 逐批合并](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/pipeline/foundation_delivery.rs)
- [C7] [storyboard.rs：跨集连续性、作者、整编与修复](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/pipeline/storyboard.rs)
- [C8] [assets.rs：父子形象文字设计调度](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/pipeline/assets.rs)
- [C9] [schema.sql：现有 jobs/events/schema](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/src/schema.sql)
- [C10] [Windows resources](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src-tauri/tauri.windows.conf.json) 与 [更新脚本](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/scripts/update-client.ps1)
- [C11] [前端 types](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src/types.ts)、[IPC API](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src/api.ts)、[文本设置 UI](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/src/TextExecutionSettings.tsx)
- [C12] [仓库既有 CLI 集成验证说明](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/docs/TEXT_CLI_INTEGRATION.md)：属于历史验证记录，不冒充本次执行结果
- [C13] [仓库交付约束 AGENTS.md](https://github.com/K1rsam/director-web/blob/5c10d315289862e8c452cb283e6df97616af57e5/director-desktop/AGENTS.md)
