# Windows 与 Mac 集成记录（2026-10-08）

本记录固定源码来源、选择性采用理由、平台边界和本机验证口径。主体业务实现提交为 `dd2f5af20e34e5fa3620c40c0bfe674c1fb653c7`；`4be4016` 补齐 Windows 命令行入口拒绝前零写入与测试子进程回收；`0175c0d` 修复实际界面复验发现的 Windows 即梦版本识别与渠道显示问题。来源中的建议、已实现代码、合成测试和真实生产验收分别记载。

## 源码起点与工作区保护

| 对象 | 版本或位置 | 本次处理 |
|---|---|---|
| Windows 既有工程 | `director-desktop`；已发布 HEAD `1e2bf93553c1235ba56fddc2141058e719393d1c` | 原目录只读保留，按函数和行为迁入后续有效实现 |
| Mac 本次工程 | `handoff/mac-native-20261008`；代码 `05b64c920384200ce45be51ac9962c39714178cd`，指南 `8b07ca276a82fe6d91cc1c3b3c0db58ed76e24af` | 以独立 Next 工程为集成基底，保留已实现的第二阶段 2.0.0 全链 |
| Mac 对照父提交 | `2c952f67b96878a90338ab5770c7d864bcf1b78f` | M6.3 基线，区分后续 104 份变更 |
| 独立历史交接 | `handoff/windows-history-20261007`；`cb4f728e1f22dbf20b88e0eae0cf0a11c6c85f13` | 与本次 Mac 代码为兄弟分支，另行读取并合入归档 |
| 集成工作区与分支 | `director-desktop-integration-20261008`；`integration/windows-mac-20261008` | 独立目录；历史合入节点 `7c96c08bfb6a3e5738ecad721d2ef0c10f2e0ade`；业务实现 `dd2f5af` |
| 后续仓库 | [K1rsam/director-desktop-unified](https://github.com/K1rsam/director-desktop-unified) | 新建私有仓库；交付 `main` 与集成分支，不推送旧 Web 整个祖先图或其他工作分支 |

操作前保全了 Windows Git bundle、仅源码归档、源码清单和与已发布版本的差异报告，均留在本机 `_task-backups/integration-preserve-20261008`。最早尝试的大型 `windows-working-source.zip` 被中断，不算有效备份；有效源码包为 `windows-source-only.zip`。Git 参考工作树 patch/staged patch 均为零字节；原物理工作区仍包含 233 个本地新增条目，不能据此称整个工作区干净。差异报告中的已跟踪差异是 `.gitignore`；已有源码与已发布 Windows 版本的核对另有清单。依赖、构建产物、私人日志、运行数据库和项目素材继续原地保留，不进入发布源码。

保护证据摘要：`windows-history.bundle` 为 6,425,490 字节，SHA256 `5ee07ab0bcab76df5be3911c5fa565f4524bc1348e63769518e544c98eeab4a8`；`windows-source-only-manifest.json` 的 427 个文件条目，SHA256 `853f25cb09ac7343806cb4032bb9e88412b363df42523b964e2210f03001653a`；`working-vs-published.json` SHA256 `02b93799f549e93d7586d963c8e3d8db8df991abd2efb0375bddfc3f73634eb9`。这些保护资料没有替代本次代码测试，也不在公开交付中附私人日志正文。

## 为什么不是从最早审查稿直接覆盖

实际顺序是 `5c10d315289862e8c452cb283e6df97616af57e5` 首次审查，随后 `970c36f3ee22ed30df9981bb86f1cd703d709bd9` 根据上游新增恢复、视觉事实和主创链重新审查，最后从 `2ac7c216183f936aa08f16a93b2a24130cf5ebcd` 实际选择性迁入 Next。`docs/MIGRATED_SOURCE.json` 记录 144 份迁入对象。前两个版本的方案稿不构成本轮运行代码。

Mac 随后在独立 Next 中逐步实现 M1 受控运行与持久化、M2 第二阶段美术 2.0.0、M3 启动诊断、M4 手动 readiness、M5/M5.1 Mac 签名及属性证据、M6 独立 Canvas readiness、M6.1 属性策略收紧、M6.2 scope 分类与 M6.3 有界进程观测。`05b64c9` 再加入标准入口、Canvas OAuth 和账户卡、Codex 冷启动检查预算及规则来源保全；`8b07ca2` 加入完整合并指南。未完成的 M6.4/M7 草稿没有作为已实现能力迁入。

Mac 对应版本的 10 月 7 日 837 core 通过、24 忽略，109 前端和 121 签名属性策略，是 Mac 证据；10 月 8 日只复核 141 源码指纹和 8 日志哈希，没有全量重跑。本机 Windows 结果在下文另列。

## Windows 后续 19 个提交的采用矩阵

下列顺序来自 `2ac7c21..1e2bf93` 的实际 Git 提交链；完整 SHA 顺序由 `scripts/verify-integration-lineage.mjs` 固定。矩阵记录集成采用的功能和冲突处理取向，最终全库验收另列，不将提交标题本身当成测试证明。

| Windows 提交 | 后续有效实现及本次采用内容 | 保留与冲突处理 |
|---|---|---|
| `8548e9f5917b8960f693a052e395f3b6e4048b1e` | 面板说明静默、选取未制作集、对白展示解析 | 保留 Next source parser v6；不以旧 parser 覆盖新版来源证明 |
| `aecbd7902ac548c4f922cbd7d1aae611012a3690` | 已用额度 100% 提前阻断；绑定原 turn.failed 的恢复 | CLI 与 SDK 身份和未知状态保护各自保留；额度未知不折算成零 |
| `dc1daef54e063933cc970dc01e72301de8b2b5b9` | 有界 planner 修订、原文边界修复、恢复后的主创连续性 | 只在原冻结请求与已保存回包绑定下修订，保留修订预算 |
| `e4db55a2739fbf32088b14667aa1024a08a865d3` | 每次显式恢复额度拒绝任务；生图按集和缺失结果范围处理 | 显式恢复不自动扩成新批次，未知提交继续占用容量 |
| `d53099729c68b75b1792ae1269356e0be8b3863d` | 视觉事实缓存绑定来源与资产身份、复用已采用主创稿 | 新工作流不能仅按标题命中旧缓存 |
| `57176ee6d5a315c962df6ded514b2f4980de20c2` | 缓存比较前恢复紧凑视觉身份 | 校验原包哈希，身份恢复不修改原事实语义 |
| `bee5f85862159bb2bf402aadfd5a3e718450d507` | 零时长原稿经原 editor 有界修订；parser 迁移证明与 HTTP 授权分类 | 保留 parser 来源证明和原回包，不把空稿修订视为重新生成许可 |
| `0b4dfb6270188f36ae4c9cb435374d212ca1eb15` | 原 editor 前置恢复后继续复用已采用 author | 保留前后节点关联，不覆盖已采用历史稿 |
| `147b517a50c37d764d797173eb31818293478ca1` | 视频可用徽标与冻结素材包校验一致 | UI 徽标依据同一冻结包证明，不替代真实渠道验收 |
| `c2faa5386a78fc94278135d4e14c6d7c492c4091` | 区分生产记录与真正阻断问题 | 历史警告可展示；不因此阻断已有效完成的节点 |
| `87cfdaff0a272dfa47c482292a4055b62f330f62` | 旧 Dreamina 匿名账户识别、官方重新登录、前台项目列表刷新 | 已工作的 legacy 生成渠道继续保留，Canvas OAuth 为独立入口 |
| `11c70a292d9fed96775826a073f25e206241fd3f` | 本地准备与云任务分开、视频批量提交有费用确认 | `autoSubmit=false` 准备不计为已生成；本轮不实际提交 |
| `97b0ca9dfca67afded70b4733e606e788c625e18` | Windows 项目库锁、恢复消费记录、跨启动上下文路径保护 | 使用 Next `app.director.next` 独立命名空间，不自动接管旧项目库 |
| `390088c33cb5dafd776513378e2c73de3cd6c7f4` | Windows 真实文件路径验证、非虚拟化重启及启动门禁 | 在任何数据读写前判定 MSIX；增加未来写入的物理句柄核验 |
| `6bc49ff9776bd930a8b5e1dde8f5d22a9dfbd7ef` | 显式 OAuth 重新授权直接执行、默认浏览器打开 | 重新登录不依赖坏掉的旧账户探测；不复用或复制凭据 |
| `a4e203d049e8d1e01431fd11b4f9a05f859847fa` | 非人类主角、保存目录结果的精确恢复 | 保留源文资产语义与目录身份；不强行转成人类角色 |
| `3fda9a4df091ff2155f3cecaa539a80253cbf051` | 全剧本制作与阅读范围分离，显式单集及来源覆盖校验 | 阅读选择不缩小全局资产依据；已有 catalog key 受控更新 |
| `621e49e564f1251a71da9f5643dec04debdb8b79` | 批量提示词原任务回调证明、视觉事实修订引用剧本上下文 | 旧提示词记录保留原版本和原回包，不伪装为新的 Mac 2.0.0 设计结果 |
| `1e2bf93553c1235ba56fddc2141058e719393d1c` | 图片提交/查询/下载独立容量 8/8/4，多层直接父资产链、无父根资产并行、当前采用图哈希冻结 | Mac 提示词负责画面设计；Windows 1.2.0 生图策略负责身份、前置顺序和恢复，不丢任一边 |

## 第二、三阶段与实际规则注册

第二阶段采用 Mac `asset-art-director` 2.0.0，保持原字节：SHA256 `9193cd59b44bdc77fb413fb8c40bb6531c2cfb9377ca17fafc0749808ba6cd82`。规则依据剧本和来源视觉事实做美术设计，人物默认包含肖像及正、侧、背三视图，非人物不强行套用人物排版。集成对象包括输入、有效性检查、最终提示词编译、UI 展示、冻结队列和恢复，而不是单独复制 `SKILL.md`。直接父资产关系、素材哈希和生成顺序由 Windows 实现继续约束。

生图生产规则采用 Windows `asset-image-production` 1.2.0，SHA256 `7d4d32682a6261bbcff3a7f5cbffc3a34a6c313068fa0f39a3f34b454935194d`，来源提交 `1e2bf93`；它与 Mac 原 1.1.0 的差异是实际采用的策略变化。随后真实第一阶段测试发现模型将同批新建基础形象误当成不存在的来源，故 F2/Q1 更新为 2.0.1：同一人的连续换装、伤势及同一非人主体的连续进化须引用实际上一形态；独立个体及原文明确独立的形象不强连。新增对象可用本批 localKey/fromKey 绑定；明确独立的空来源不被程序猜改。其余 26 份注册规则保持 Mac 原字节，包含上述美术 2.0.0。72 份历史参考快照不变。

F2 2.0.1：11,227 bytes，SHA256 `4f7730877b5ffa6a664b655a81a7a5b40b0f13c396eebfa1d37b245064a9497c`；Q1 2.0.1：11,777 bytes，SHA256 `ab9b92a69698d9a0e4df5804c88f51e819b3e2d29bcc0b9884c8eea7b4fb85b8`。

第三阶段来源仍是 Mac 最新 3008 生产规则。交接提交的清单是 `active=false`、`production_rules_verified_runtime_adapter_pending`；本轮完成兼容 Rust 适配器和离线回归后将其启用，profile 为 `mac-3008-compatible-rust/v1`。执行顺序是情绪统筹、完整主创、统一整编；另接整集上下文的导演精修、候选预览、显式原子采用和恢复。22 份原规则正文和哈希保持不变。精修取料与采用均核验源文、资产、当前单元、执行设置和原回包；任何漂移会阻止覆盖，旧候选和原稿继续保留。兼容适配器是重建实现，不等于迁入未交付正文的 42 个 Mac 原运行文件，也不自动证明生产稿质量提升。详见 [第三阶段适配记录](STAGE3_COMPATIBLE_RUST_ADAPTER_20261008.md)。

| 统计对象 | 数量与来源 | 实际含义 |
|---|---|---|
| `reference-skills` | 72：桌面初版 29、旧 Git 历史 21、Mac 实际 3008 生产 22 | 只用于来源追溯，不按目录扫描加载 |
| `rules/stage3/candidates/mac-3008-20261006` | 22 | 同一组 Mac 生产规则的受哈希锁定运行候选，不另加成第 94 份参考快照 |
| 兄弟历史提交 | 65 份文件：60 份原始规则归档、5 份说明及索引 | 单独归档口径，不能与 72 相减或混算 |
| 桌面原有注册表 | `pipeline::rule_snapshots()` 显式 29 项 | 按当前源码实际路径、版本、哈希核对；历史 index 的注册列表不自动成为当前运行注册 |
| 本轮第三阶段适配器 | `pipeline::stage3_pack` 显式 22 个冻结来源条目 | 规则加载、调用顺序、采用和恢复已通过离线回归；真实文本成稿质量尚未验收 |

`node scripts/verify-integration-lineage.mjs` 无生成、账户或项目操作，核对上述原字节、Git 父提交与两个归档口径，并输出紧凑 JSON。当前 desktop registry digest 为 `ea4ff62fba77510e9619018a38c541cbfe94d0d2d7f0eaf957a72a4f49659a0b`，29 项中 26 项与 Mac 原字节一致，3 项变化为 Windows 生图生产规则及 F2/Q1 2.0.1。manifest 为 `production_rules_active_on_reconstructed_compatible_rust_adapter`。来源锁还核对 22 个原件、144 个迁入对象和 46 个重叠历史哈希；清洁源码副本的最新验证单独留回执。

新仓库无需携带整个旧 Web 仓库祖先图。脚本始终核对 Windows 1.2.0 固定原 blob SHA256/版本并记录来源提交；本地有 Windows Git 对象时再核对 19 提交和原 blob，并显示 `sourceGitVerified=true`。普通新仓库 clone 缺少这些 Windows Git 对象时不阻止同一来源锁校验，回执显示 `sourceGitVerified=false`。

## 平台与渠道冲突处理

| 对象 | 集成结果 | 验收边界 |
|---|---|---|
| 桌面入口 | 保留 Next 标准入口和执行策略，Windows 非虚拟化门禁在数据初始化前运行 | MSIX 身份和物理写入路径必须真实核验，合成检查不能替代成品启动 |
| 项目库 | Next 命名空间独立、库锁与已消费恢复记录可用 | 本轮仅允许显式项目包导入或隔离副本测试，未读取/自动迁移旧生产库 |
| 旧 Dreamina CLI | 保留有效生成链、匿名账户识别、官方重新授权、会话修订和排队保护 | 不与 Canvas 账户、可用性或生成能力串用；本轮未真实重新登录或付费生成 |
| Canvas OAuth | 保留 Mac OAuth 与账户卡、独立 readiness | Mac 最近实际检查仍 `oauth_scope_insufficient`；本轮不宣称 scope 或生图桥已解决 |
| Codex CLI | 保留 Mac 10 秒版本/帮助冷启动预算及 Windows 额度、原 turn 恢复 | 冷检查预算不改变生成 deadline；未知结果与原回包不能自动重试覆盖 |
| 官方 SDK | 固定 `@openai/codex-sdk` 0.160.1；标准 Windows GUI 可显式选择并核验运行器、Node、模型、推理强度及账户身份 | 离线/健康检查保持关闭；真实调用需要冻结执行身份，未知提交不自动重发。首个真实合成请求状态未知，不能称已验收 |
| Windows SDK 路径 | 校验仍使用规范路径，Node argv 将 verbatim DOS/UNC 转为等价普通路径；`--import` 使用 file URL | 设备命名空间、相对路径和 `..` 拒绝；含中文、空格、`#`、`%` 路径有独立原生 Node 回归 |
| Windows SDK 环境 | Node worker 清空继承环境；经审计的 native wrapper 单独启动官方 Codex runtime，使用已核验的既有 credentialHome | 不复制登录凭据，不混用 worker 环境与官方 runtime 登录路径；运行器、桥接、资源、模型、账户指纹绑定在冻结请求中。健康检查保持零生成 |
| 新程序更新 | 独立同名 EXE 的 current/previous、按 EXE SHA256 配对资源、持锁发布、在用拒绝、原子替换 | BuildOnly 不发布；资源不与另一个版本错配；旧应用原 EXE 路径不替换 |

SDK 首轮 Windows 全库测试暴露三个真实差异：Node 主入口无法消费 `\\?\` 脚本参数；Windows `--import D:\...` 不是可用模块 URL；Rust `env_clear` 的 Node worker 缺少 Windows 系统目录，触发 `ncrypto::CSPRNG` 断言。修复限定在路径和系统环境适配，没有跳过测试或放宽来源哈希、输出上限、生产执行开关。

最终集成测试又发现 headless 入口的物理路径探针早于异步动作拒绝，导致被拒绝后仍留下空目录。已将包身份只读检查留在前面，物理写探针移到请求、模式与授权文件门禁后，并增加 `storyboard.refine` 的 headless 阻断覆盖。测试断言没有放宽。

开发用 Windows native runner 原来在超时后只结束 cargo，不能保证其后代退出；现在在进程创建时绑定专属 Job Object，超时及父进程自然退出都回收该子树，确认零活动进程才写 receipt。两个原生合成测试覆盖正常退出和 2 秒超时，不按进程名结束其他应用。

实际界面复验又发现 Windows 已安装的旧即梦生成 CLI 返回 `version/commit/build_time` 裸 JSON（Git build），Mac 的旧版检查只接受版本文本，导致账户查询之前误报协议不兼容。修复后核对三个字段的一致性与时间格式，再核对根命令和四个生成命令的帮助旗标；只读检查仍不会调用生成命令。Canvas 的 `schemaVersion/ok/data` 协议继续排除。高级“更新模型目录”共用未保存路径与检查锁；实际生成仍使用保存后的冻结设置。Canvas 界面区分“本机有登录记录”与“身份已核验”，避免 scope 不足时误显示已连接。

## 本轮实际验证与未测范围

以下数值按执行时的源码和回执分别记载，不把重复定向组加进全库数量。自动化测试只使用临时项目与合成输入；另有成品 GUI 的真实只读渠道检查。完整日志和运行 receipt 留在本机 `.local/validation` 或 `_task-backups`；发布文档仅记录命令、摘要与限制。

| 检查 | 命令或入口 | 结果 |
|---|---|---|
| 规则与历史来源 | `node scripts/verify-integration-lineage.mjs` | 阶段激活后的干净 clone 核对 72 快照、22 candidate 与 60 history 原字节、65 history 变更口径、29 注册路径版本通过 |
| 平台 Rust 定向组 | native runner 执行 `dreamina_session::tests`、`providers::dreamina::tests`、`text_cli::tests`、`project_library::tests`、`windows_runtime::tests`、资源选择测试，`--test-threads=1` | 115 通过、0 失败、3 忽略；历史工作树回执，不冒充之后的全库结果 |
| 最终前端回归 | `npm run test:frontend`、`npm run build` | 127/127 通过，TypeScript 与 Vite 构建通过；包含 legacy/Canvas 未保存路径检查不串用、重新授权执行动作、全剧本覆盖、重复 UI 入口去重和未核验身份文案 |
| 五组 UI 合成交互 | `verify-foundation-scope-ui`、`verify-task-preparation-ui`、`verify-video-production-ui`、`verify-stage-workbenches`、`verify-asset-lifecycle-ui` | 7 + 6 + 34 + 38 + 10 = 95 场景通过；原 workflow 与哈希采用、跨集标签、离线门禁及 22 规则哈希展示通过；视频脚本不调用真实生成 |
| 来源锁篡改回归 | `node --test scripts/test_source_lock.mjs` | 8/8 通过；干净 clone 再核对来源锁及 lineage 通过 |
| SDK bridge | `cd sdk-bridge; npm run verify` | 44/44 通过，0 忽略；包含真实固定版本 SDK 对合成 runtime 的调用及禁止生产 execute 的健康检查 |
| Windows SDK Rust 定向 | native runner，`readiness_sdk::tests`、`text_sdk::tests`、`text_executor::sdk_core_tests`，`sdk-platform-*-04` | 路径及 Windows 环境修复后 6 + 6 + 12 = 24/24 通过，0 忽略；保留最初失败回执用于追溯；该轮测试 EXE SHA256 `598afc440d1c06648190280d3aa29737d2cec004ac6271afbc4e467cccea8d49` |
| 更新脚本 | PS5.1 `test-update-client.ps1` 的隔离合成验证 | 11/11 通过；fake EXE 实际启动 0，未改旧应用/生产库，无生成；测试 current/previous、占用拒绝、资源配对与候选窄清理 |
| Mac 来源属性策略 | Windows 本机执行适用策略检查 | 主控报告 121 项通过；Mac bundle 的 POSIX mode fixture 为 Windows 不适用，不计为 Mac 原生验收 |
| 首轮 Windows 全 lib | native runner `cargo test --lib`，`full-rust-01` | 历史失败：722 通过、15 失败、33 忽略；规则旧版本断言及 SDK 平台差异已修复，不混入最终通过数量 |
| 修复后的完整 Rust lib | `cargo test --release --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --tests -- --test-threads=1`，`full-rust-release-final` | lib 756 通过、0 失败、33 忽略；随后 integration 在 headless 零写入断言处失败，因此该条整次命令 exit 101，不能说整次通过 |
| headless 修复后的全部 integration | `cargo test --release --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --test '*' -- --test-threads=1`，`rust-integration-final` | 23 组，176 通过、0 失败、4 忽略，exit 0；结合未变化的 lib 为 932 通过、37 忽略 |
| 最后一轮渠道修复回归 | `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib readiness -- --test-threads=1`，同命令 filter 换 `providers::dreamina::tests` | readiness 49 通过；provider 76 通过、2 原有忽略，均 exit 0。二组有 1 个重叠项，未简单相加，也未重跑全库冒充新全量结果 |
| Windows 测试进程回收 | `node scripts/test-windows-native-runner.mjs`，`native-runner-job-regression-02` | 2/2 原生子树测试通过，正常退出及 2 秒超时后 Job 活动进程均为 0 |

native runner 由 MSIX 外的真实 Windows 进程执行，并记录 `unpackagedRunner=true`、`sharedWriteProbeVerified=true`；不能把初始宿主退出码当成测试结果，须读取独立 `receipt.json`。上表为此前检查点，新增源码的最终回归及生产验收另记，不以旧数值代替。

## 真实运行发现与修复中的验收

用户已授权 SDK 使用现有额度、仅免费图片，禁止视频。所有私人剧本、数据库、请求和原回包只留本机；没有退出账户、重新授权或打开旧生产项目库。

第一轮新建项目的十集原文一键整理完成，用时 442.8 秒；F2 369.5 秒、Q1 67.5 秒。得到 50 个基础身份、59 个形象，源文 SHA256 保持一致。只读审稿发现 3 个连续换装被错误声明为独立来源，已修订 F2/Q1；不手工补这三个结果冒充工作流通过，将用另一新建项目重新验收。

同时发现跨行 `【情报…价值:一星】` 界面板的字段被错误标成现场对白。source parser v7 只识别完整界面区间，保留原文、UTF-16 坐标和精确覆盖；畸形或未闭合区间不能吞对白。既有 v1–v6 来源身份不静默重写，只有投影实际变化的新解析采用 v7。

SDK 第一次真实合成请求产生 1 次 dispatch，确认模型调用次数未知。官方运行器的非致命配置警告出现在 turn.started 前，旧桥接误判 OUT_OF_ORDER。原事件和 `submission_unknown` 收据已保全，未重试。修复只接受已观察到的配置警告语法，不将其当成生成结果；桥接仍拒绝过早的模型内容、工具调用、畸形事件和没有完成事件的输出。第二次使用独立新 case 验证，结果另录。

第三阶段新增有界来源坐标提示，给情绪、主创、整编和精修共同定位镜头时长问题，去掉要求单元凑满 13–15 秒的适配偏好；22 份 3008 原规则不变。程序不将真实长镜头数字强行截成 5 秒，实际十集成稿仍须审查。

当前新增回归：前端 136/136 通过，SDK JS 54/54 通过，来源锁篡改 11/11 通过，本机可执行路径默认值 8/8 通过；SDK 资源增至 9 个后的更新脚本 11/11 通过。最新隔离源码副本的来源锁和 lineage 均通过，72/22/60 三组历史原字节保持。

最终完整 Rust 命令为 `cargo test --release --locked --lib --tests -- --test-threads=1`（工作目录 `apps/desktop/src-tauri`），native receipt `sdk-native-rust-final-02`：library 785 通过、0 失败、33 忽略；24 组 integration 184 通过、0 失败、4 忽略；合计 **969 通过、0 失败、37 忽略**。整次 exit 0，用时 521.013 秒，未超时，Job 进程树已确认清空。包含 SDK 31、source display 8、父链 13 及版本锁测试，不重复相加。成品构建和后续生产验收继续补录。

历史即梦图片任务的 `credit_count=0` 只能证明历史免费，账户总积分不证明下一次请求免费。当前 CLI 未提供可核验的本次免费资格或费用上限，因此尚未提交真实图片；Canvas scope 不足和 Canvas 生图桥仍是缺口，不能用付费图片或视频绕过。第一、二集同源新旧成稿质量比较也尚未完成。发布源码不等于完成这些验收。

## 最终收口项

主控补录：最终集成提交 SHA 与新仓库远端 SHA、最终 manifest/profile、完整 Rust/前端/SDK/适用平台策略结果、成品 EXE 与资源 SHA256、独立原生启动及隔离保存重开结果、实际渠道检查与未测范围。上述字段补齐后再移除本文草稿标记。
