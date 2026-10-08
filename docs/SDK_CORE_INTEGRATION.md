# 官方 SDK 接入共享 Rust 核心

M2 历史检查点日期：2026-10-06；本次 Windows 集成状态更新：2026-10-08。M1/M2 的对应版本证据、既有验收清单及冻结创作原件保留，历史离线结果不能代替本次真实调用验收。

## 状态与边界

SDK 不再是 `SavedTextExecutor` 的入口占位拒绝。两条路径共享 Rust 的任务、编译、原回包、领域采用/拒绝和恢复合同：

- 离线合成路径：`SavedTextExecutor → OfficialSdkTransport → Node worker → @openai/codex-sdk@0.160.1 → 合成 Codex subprocess`，仅测试编译中的固定 fixture 可执行，不访问真实账户或模型。
- 标准 Windows GUI 路径：同一个官方 SDK 通过受控 Rust supervisor 调用已核验的官方 Codex runtime，使用既有账户；用户必须明确选择 SDK 并启动文字工作流，预检核对资源、Node、账户、模型和强度后冻结身份。

原生能力只由标准 Windows GUI 入口注册，IPC 设置不能开启它。离线、headless、受控 CLI 验收入口、未验收的平台及打包上下文仍拒绝 SDK live，不会自动后备到 CLI/API。选择 SDK、只读检查成功或能力已注册，均不等于真实模型结果通过；当前真实验收状态见文末。

## 实际代码与控制入口

- `apps/desktop/src-tauri/src/text_sdk.rs`：官方包身份 DTO、只读状态、预检、受控 Node worker、NDJSON 监督、observe/cancel
- `apps/desktop/src-tauri/src/sdk_native.rs`：标准 Windows 原生能力、账户/目录预检、v2 冻结身份、受控 runtime wrapper 和 Windows Job Object
- `apps/desktop/src-tauri/src/text_executor.rs`：SDK 分支仍属于 `SavedTextExecutor`；使用既有 Store 的 jobs/events/workflow_steps、全局文本信号量和领域回调
- `apps/desktop/src-tauri/src/text_transport.rs`：保留公共 FrozenAttempt/Observation/TextTransport；移除 SDK 未实现占位；封套读取有 4 MiB 上限
- `sdk-bridge/src/bridge.mjs`：仍实际导入并调用固定官方 SDK；公开回执增加原请求/执行/编译字节摘要供 Rust 对账
- `sdk-bridge/src/native-runtime.mjs`：核对冻结原生身份，建立该 attempt 的工作/临时目录和不可变运行计划；不复制凭据
- `TextExecutionSettings.tsx`：CLI/SDK 显式选择；只有后端报告 `explicit_selection_when_ready` 才展示原生 SDK 检查与选择能力，账户、目录与生成状态来自实际检查，不从下拉值推断 ready

引擎调用：

- `textSdk.status` → `text_sdk::status(&settings).await`，返回静态能力及已有冻结身份；不调用账号、Codex runtime 或模型。`liveCallsAllowed` 表示原生能力已注册，不表示账户、模型或具体任务已通过
- 明确点击 `readiness.check` 的 SDK 检查先运行无推理健康 worker；标准 Windows 原生能力已注册时再用官方 Codex 的只读账户/模型 RPC 核验，检查自身模型调用数仍为 0
- 新 SDK 工作流预检 → `text_sdk::resolve_for_workflow(&settings).await` → `sdk_native::resolve`；再次验证明确 SDK 选择、模型/强度、包/Node/runtime 和既有账户，冻结 `official-sdk-identity/v2`。没有原生能力或预检不通过时拒绝，不借旧检查结果直接生成
- 设置变更不改既有 workflow 的 settings 快照；执行与回调验证原 workflow/settings 绑定
- SDK unknown 不得经 `jobs.close_unknown` 清除；没有停止证明时只能保留

## 编译与不可变身份

SDK 和 CLI 调用同一个 Rust `compile_codex_input`。它继续调用 foundation/storyboard 已有无损运输编译、原 `cli_prompt` 和 strict optional schema 逻辑；不重新概括规则或修写创作正文。SDK 输出也经同一个 foundation alias/optional 恢复函数，再进入领域校验。

SDK 身份拒绝未知字段，独立于旧 CLI 的三字段对象。旧 `official-sdk-identity/v1` 仍用于 synthetic/unverified 离线合同，不能夹带真实运行器；原生 `official-sdk-identity/v2` 增加官方 runtime、supervisor、资源目录及既有 `credentialHome`/`codexHome` 的路径绑定。两者都冻结官方包/实现 SHA、桥版本/源码 SHA、协议、Node 路径/文件 SHA/版本和权限 profile；v2 另冻结实际 runtime 版本/文件 SHA、supervisor 文件 SHA 与非秘密账户状态指纹。启动前再次校验，不能把 v1 历史改写成 v2 或将 SDK 身份塞入旧 CLI 对象。

环境清理按进程层区分：Node worker 使用 `env_clear`，不继承 `NODE_OPTIONS`、宿主 HOME、凭据、代理、PATH 或配置变量，Windows 仅显式加入系统目录变量。原生 supervisor 再单独构造官方 Codex runtime 环境，将 HOME/USERPROFILE 指向冻结的既有 `credentialHome`、CODEX_HOME 指向既有 `codexHome`，并提供该 attempt 的临时目录。runtime 使用 `--ignore-user-config`、`--ephemeral`、read-only sandbox 与固定执行参数。程序传递的是账户目录位置，没有读取并复制凭据到项目、worker 或临时目录；官方 runtime 按正常账户机制使用原位置的凭据，不能把这一层描述成“不读取账户”。

离线 synthetic 子进程另行明确设置 `NODE_OPTIONS`，只加载固定 fixture 生成的 bootstrap，把原生 Node 的入口重定向到自带假 Codex 程序；不改官方 SDK，不用真实账户。无推理健康 worker 使用独立空 HOME/CODEX_HOME，只做 hello/inspect/shutdown，不能与原生推理环境混为一谈。

每个 attempt 冻结原请求、backend、model/reasoning、semanticRequestHash、executionFingerprint、attemptEnvelopeHash；Rust 另存 exact prompt/schema 与字节摘要。桥持久回执保存同一原包与编译字节。回放重新编译并核验这些绑定，不能只凭 task ID 或 completed 字样借用另一个回包。

正常非 managed CLI 新请求若已预检取得 textCliVersion，则通过实际 request_snapshot 构造完整三字段身份。没有已知版本的历史两字段快照保持旧形状，不填造假版本。旧非 managed workflow 的两字段缓存仅在父流程设置、完整领域请求、原快照哈希、原 raw/接收事件、已采用回执与 checkpoint 全部精确相符时只读回放；未知或不一致的旧节点失败关闭，不因三字段升级漏缓存而再次生成。新 SDK 身份绝不塞入旧 CLI 对象。

## 回执、序号与 raw

`lastSequence` 在桥中始终是最后一条 SDK journal 事件的序号。Rust 投影使用 SDK 事件序号乘 2；终态 observation 使用最后 SDK 序号乘 2 加 1，避免终态与最后事件同序号被丢弃。每个 SDK 事件先进入原 job 的 events，再更新冻结 attempt；重启本地 observe 时按原 journal 序号核验并补齐尚未入库的事件，重复原事件不重复写入。

文本位于原 job 的 `text-runs/<job>/result.json`，保留 SDK agent message 的 exact UTF-8 字节；SDK 返回的 parsedJson 不直接代表采用。SQLite jobs.result 和 text_received_raw 保存经过既有运输还原、尚未领域修写的值。`Store::job` 的 UI mediaIds 投影不参与原始数据比较。

领域 accept/reject 用 SQLite 原字段、最新原始接收事件比较和事务提交；采用后的领域物化结果置于 prepared 和 workflow checkpoint，SDK 原 jobs.result 不被 normalized 值覆盖。重复采用必须等于原已验证物化值；被拒绝 raw 不能经第二次 accept 晋升。原文/编译字节/快照/哈希/回执不一致则保留原证据并失败关闭。

## 恢复与持久预算

1. 先查该冻结请求的既有 job、回包和领域状态；完整回放不启动 Node 或再次 execute
2. 原节点/backend/请求变化不创建新任务
3. 原 dispatch intent 有记录但缺少可信终态时，只向原 worker 目录发送 observe；从不调用 SDK resumeThread/run 作为恢复
4. SDK unknown 保留原 job 与最初错误，不通过新 workflow、切 CLI/API、重置修订 attempt 或关闭 unknown 绕过
5. 域级 visual fact/carrier/author repair hook 仍由 SavedTextExecutor 转发。A15 原包/修订/已采用物化上下文与 accepted Finisher→后继 Author 连续性分支已分别经实际官方 SDK 合成执行证明；CLI 专用的旧快照迁移仍只用于其原 CLI 历史。新增 SDK 修订占位核验以原源/原请求/原 raw 身份和 attempt 为锚，排除 workflow/backend，保留两次上限
6. 全局四文本槽是两入口共用；SDK worker 另有一个 active attempt 的保守限制。unknown 不释放为可再次提交的容量

固定 TypeScript SDK 不提供远端被动观察，也没有 turn ID。即使标准 Windows 入口已经开放显式 SDK 执行，没有本地完整完成证据时仍不能推断云端是否接收/停止、结果已采用或费用为零；本地进程树清空也不能把 unknown 自动变成可重发。

## 监督与原生验收边界

Rust worker 限制 1 MiB 单帧、2048 条 SDK 事件、16 MiB journal、1 MiB sidecar stderr（最多保留 64 KiB 原字节前缀，以 hex 写入原 job 事件）、有界 stdin 写入、控制 RPC 超时和总执行 deadline；支持分片 UTF-8，非法 UTF-8/NDJSON、半行 EOF、超大帧和序号/身份变更失败关闭。inspect 到 execute 持有同一个 worker 锁。取消先向活动执行的等待循环发送独立信号，不等待其管道互斥锁，再通过原 worker 请求 AbortSignal。原生 Node/runtime 在恢复线程前纳入自有 Windows Job Object，设置 KILL_ON_CLOSE、1 GiB 聚合内存和最多 8 个进程；绑定失败不走无约束后备。只有实际停止并查得该 Job 的 activeProcesses=0，才在该次 `process-receipt.json` 或 `sdk_process_tree` 事件中记 `processTreeStopProven=true`；静态 status 和未决 observation 不提前宣称证明。

固定官方 SDK 内部仍先行累积 stderr 或 readline 单行，不能把 Rust/桥的帧限制描述成其内部实现有界。Windows 原生 runtime relay 与 Job Object 提供外层输出/聚合资源和进程树约束，但不等于官方包内部已改写。固定参数、既有凭据使用、真实结果和恢复须分项验收；macOS 真实 SDK、四平台 Node/SDK/runtime/helper 打包及正式签名仍需各自证据，Windows 实现或离线测试不能代替。

## 离线复现

```sh
npm ci --offline --ignore-scripts --omit=optional --no-audit --no-fund \
  --cache /workspace/shared/director-next-toolchain/npm-cache --prefix sdk-bridge
npm run verify --prefix sdk-bridge
cargo test --release --lib sdk_core_tests --offline
cargo test --release --lib text_sdk::tests --offline
cargo test --release --test next_isolation --offline
```

Rust 命令在 `apps/desktop/src-tauri` 执行，或传该 Cargo.toml 的 manifest-path。需要已安装 Node 与固定 SDK npm 依赖；测试不得因缺少依赖悄悄跳过。`DIRECTOR_TEST_NODE` 可给合成测试明确 Node 路径；未设置时仅测试代码从 PATH 定位 Node，并冻结其实际文件 SHA/版本。产品代码没有 PATH fallback 或合成 fixture 的用户设置入口。

M2 分项检查点日志见 `sdk-bridge/verification/m2-*`；该历史版本的聚合、源码摘要和精确总数以 `docs/verification/M2.json` 为准。其中 synthetic SDK subprocess 与真实模型调用分别计数；M2 当时真实 SDK 模型、账号和官方推理 runtime 调用为 0。这个历史结论不适用于 2026-10-08 后续原生检查与真实调用，不能继续写成整个集成阶段零真实 dispatch。

## M2 历史专门回归覆盖

- 14 项 Rust SavedTextExecutor 跨语言/身份用例：真实官方包调用；exact prompt/schema；领域采用、拒绝与不可变 raw；原流程重启；新 workflow 精确缓存的绑定 checkpoint；桥已完成而 Rust 未落库的恢复；断连未知占位；新流程/跨 backend 拒绝绕过；两次修订预算；原 raw/编译/身份/领域哈希篡改；非法文本 JSON；A15 与 Author 连续性 hook；非 managed CLI 实际 request_snapshot 三字段构造；Stage2 同父双资产 checkpoint 与重启零额外执行
- 4 项 Rust supervisor/身份用例：UTF-8 分片与超长/半帧；stderr flood 的有界逐字前缀；SDK 独立身份类型拒绝漂移；真实官方 SDK hang subprocess 的并发 cancel、错误 attempt 拒绝以及未知状态保留
- M2 对应版本的官方 SDK 桥 41 项 JS 离线回归通过；当时 Windows 实跑尚待 CI，不表示当前 Windows 状态仍未验证
- M2 对应版本的主库、入口隔离、前端结果以随附 m2 日志为准；当时没有执行原生和 ignored live tests。后续 Windows 检查、离线回归与真实模型调用分别记证据，不重写这些历史记录

## 2026-10-08 真实调用记录边界

已用合成、非私有输入发起首次 SDK 真实调用。该原 attempt 因当时将非 fatal 配置警告事件当作致命错误而记录 unknown；保留原请求、raw、事件和身份，不重发、不用新流程或另一 backend 覆盖它。后续代码仅对已观察到的配置警告格式作窄处理，真实 turn.failed/error 仍失败关闭，不能据修复追认旧 unknown 成功或零费用。

第二个独立新 case 尚未完成完整终态、结果与领域采用验收，不能预写通过。标准 Windows SDK 真实闭环和成稿质量仍待对应结果证明；只读就绪、离线合成通过和进程树停止分别报告。本次文档更新不发起调用或测试，不包含账户身份、用户本机路径或私有请求/响应。
