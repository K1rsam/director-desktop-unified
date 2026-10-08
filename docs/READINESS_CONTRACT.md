# M4 连接检查与生成权限分离

本合同保留 M4 的只读检查边界，按 2026-10-08 Windows SDK 原生实现更新。M4 对应版本的零生成证据仍是历史记录，不代表后续标准 Windows SDK live 一直关闭。

用户反馈 M1 设置页的 CLI/即梦检查按钮被离线策略整体禁用。M4 修复这一产品入口，同时保留真实生成、登录写入和凭据保存的独立门禁。程序安装、协议握手、账户、模型目录、网络和真实模型调用分别报告；不会把任何前一层直接称为后一层成功。

## 明确点击才检查

共享前端使用 `readiness.check`，provider 精确为 `codex_cli`、`dreamina`、`dreamina_canvas` 或 `codex_sdk`。当前未保存的对应可执行文件路径会用于这一次检查，不写回项目设置；SDK 仅接受 `textSdkNodePath` 与 `codexPath` 两个路径草稿，分别用于 Node 和既有官方 Codex runtime。模型/强度取已保存设置，IPC 不接受它们或测试 fixture、桥路径、凭据、登录及执行参数的覆盖。打开页面、切换项目或切换入口均不自动访问账户；同一检查的重复点击和旧路径/旧项目回包被隔离。

普通 Windows/macOS 桌面按已注册能力开放只读检查，检查许可与真实生成许可独立。只有标准 Windows GUI 注册原生 SDK 能力后，SDK 检查才在包/Node/健康协议通过后进一步读取官方 Codex 的既有账户和模型目录；离线/headless、受控 CLI 验收入口和未验收平台仍不能据此执行 SDK live。云端 Linux、注入离线测试通道和 `--verify-startup` 隔离探针不开放宿主账户检查。探针显式使用 `Engine::new_offline_verification`，不能借新按钮访问账户。SDK 资源目录只由 Tauri 提供，不接受 IPC 中的路径。

旧 `dreamina.status` 会轮询 OAuth 并恢复任务，所以新检查完全绕过 DreaminaSessions 与原任务恢复。没有将所有 provider/credentials 动作放行。生成、登录/退出、凭据写入、真实 SDK 调用、第三阶段 inactive 门禁均保持独立。

## 分层返回合同

每个结果是 `provider-readiness/v1`，包含 provider、检查时间、以下分层状态、消息及有限的安全详情：

- installation：程序/包是否找到与验证，版本和本机可执行文件位置
- protocol：实际 CLI RPC 或 SDK worker 协议是否完成
- authentication：authenticated / not_logged_in / unknown / not_checked，以及是否核对了账户身份；不导出账户邮箱、用户 ID 或 OAuth 材料
- catalog：本次读取的模型目录。目录不是账户资格或实际生成证明
- network：独立网络证据。本轮 CLI 返回和 SDK 本地握手不冒充真实模型请求；没有单独证明时为 not_tested
- generation：检查自身始终 `testPerformed=false`、`calls=0`，没有真实模型请求。`enabled` 是后续执行能力，不是本次生成结果：CLI、即梦及 Canvas 的只读报告保持 false；SDK 仅在标准 Windows 原生能力已注册、包/Node/协议通过、官方账户身份已核验且模型目录 `state=available` 时可为 true，其余为 false

Codex 检查使用既有的 `--version`、`exec --help` 和只读 app-server RPC：`account/read` 明确 `refreshToken=false`，随后 `model/list` 和额度查询。账户/模型 RPC 失败时保留独立取得的 CLI 版本，账户标为“未知”，不能把 SQLite 初始化失败解释为“未登录”或所选模型不支持。没有自动降级版本、换模型或登录。检查输出分别限制 stdout 256 KiB、stderr 64 KiB；RPC 在分配完整行前限制 1 MiB，并限制累计 16 MiB/2048 帧，超限后停止自有检查进程。

即梦检查使用已适配的 `version`、命令帮助和 `user_credit`，不发起图片/视频任务或 OAuth 流程，不修改队列、能力缓存表或任务状态。模型帮助和积分均不能证明当前账户的 5.0/5.0Pro 免费资格；资格始终单独标为未核验。未找到本机官方 CLI 时如实显示缺少依赖，不能用伪造的连接成功掩盖。若 version/help 虽退出0，但明确报告日志初始化因 operation not permitted/permission denied 受限，则返回 CLI_LOCAL_LOG_WRITE_RESTRICTED 并停止后续账户检查；不把它解释为登录失效，不重定向目录或修改权限。

SDK 基础健康 worker 实际导入固定官方 `@openai/codex-sdk@0.160.1`，只做 hello/inspect/shutdown 并退出，使用独立空 HOME/CODEX_HOME、临时目录及白名单环境，不构造推理 runtime、不读账户、不执行模型或 resumeThread。这一层的包/worker 就绪不等于账户、模型或真实生成通过。

标准 Windows GUI 的原生能力已注册时，基础健康层成功后调用 `sdk_native::add_readiness`，复用 `codexPath` 所指官方 runtime 的只读账户/模型 RPC；账户查询不刷新 token，不执行推理。结果保留 `provider-readiness/v1`，补充 authentication、catalog 以及 `details.officialRuntimeChecked`、`accountChecked`、`runtimeExecutable`、`runtimeVersion`，Node 路径/版本仍来自 installation 与 `details.nodeVersion`。UI 读取这次实际目录中的模型和支持强度，不能因选择了 SDK 或某个模型而声称 ready。账户或目录失败时分别报告，`generation.enabled=false`，不自动登录、换模型或后备到 CLI。

`generation.enabled=true` 只允许用户下一步明确选择 SDK 并启动文字工作流；执行前还会再次核验具体模型/强度并冻结 v2 身份。生产 Node worker 清空继承环境；受控 supervisor 对官方 runtime 单独传入既有 `credentialHome`/`codexHome` 的目录位置及 attempt 临时目录，使用 ignore-user-config、ephemeral 和 read-only sandbox。没有凭据复制到项目/worker/临时目录，官方 runtime 仍按正常账户机制使用既有凭据。不能把无推理健康 worker 的空 HOME 扩写成真实 SDK 永不读取账户。具体执行、恢复和当前真实验收边界见 [SDK_CORE_INTEGRATION.md](SDK_CORE_INTEGRATION.md)；资源哈希、许可证及健康层约束见 [SDK_READINESS.md](SDK_READINESS.md)。

后端 `textBackends` 的 SDK 状态为 `explicit_selection_when_ready` 时，UI 才显示原生检查/选择能力；`offline_contract_only` 仍明确关闭。静态 `textSdk.status` 不做账户/目录探测，原生能力注册也不能提前认定模型结果、领域采用或本次进程树停止。

## 原生验收顺序

用同一份评审后 M4 提交构建应用，并一起保留 M3 启动诊断；不重复对 M3/M4 分别跑全量构建。构建前分别在 apps/desktop 与 sdk-bridge 安装锁定依赖。Tauri 构建钩子将真实 SDK 只读健康所需资源打入应用。

1. 打开普通新应用设置页，确认三项检查可点击；进入设置或切换入口时不自动触发检查
2. Codex：检测实际安装版本，读取现有账户；若 app-server 失败，回传具体错误与本地 SQLite 初始化上下文后再诊断，不重置登录或推断模型不支持
3. 即梦：先确定官方 macOS arm64 CLI 的安装来源与文件；缺失时由已授权本地工程流程补齐官方依赖，再检查实际版本、帮助和账户。免费样本仍需另行核验，不能因积分存在就出图
4. SDK：验证资源、Node 与实际健康 worker 握手。标准 Windows GUI 再核实际账户/模型目录和模型/强度选项，检查本身仍为 `testPerformed=false`、`calls=0`；未注册原生能力的入口只报告基础健康，账户/模型未检查、SDK live 仍关闭。后续明确启动的真实工作流单独验收，不把 enabled=true 当作生成成功
5. 三次检查前后对照项目任务与设置，证明没有生成、恢复、登录改写或设置写回。结果分别记录，不能合并成“三条模型连接成功”

云端/离线证据只使用合成 CLI、合成账户数据和无推理 SDK 健康 worker。M4 对应版本的测试数量、源码摘要与当时原生未验证项以 `verification/M4.json` 为准；后续 Windows 只读检查、真实模型调用和领域采用分别记录。

2026-10-08 首个合成输入的 SDK 真实调用已留下 unknown：当时非 fatal 配置警告被误作致命错误，原记录保留且不重发。第二个独立新 case 尚未完成验收，不能写成 SDK 真实生成闭环通过，也不能继续把整个集成阶段记为零真实 dispatch。这个状态不改变即梦图片当前免费资格的独立门禁，也不开放真实视频。
