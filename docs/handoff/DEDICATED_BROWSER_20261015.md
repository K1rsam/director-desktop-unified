# 专用普通浏览器接入与原任务恢复（2026-10-15）

分支：`integration/dedicated-browser-20261015`。基线：`c0c7d6a4e5fc5e269b21a4efa68019e5ba1a64db`。

## 采用的行为

- 在原绑定的 profile/bundle 内，优先使用唯一的已登录 ChatGPT 首页作为连接页。必须是真实网页运行层就绪、隐藏、未聚焦、无活动请求，不能把 Native/后台握手当成网页就绪。
- 连接页保留用户所有权，不写入任务标签。工作页通过它在同一浏览器窗口后台创建；不同浏览器、隐身、身份变化和多个连接首页仍拒绝含糊派发。
- Native inventory 的 `sourceRole=connection` 进入 canonical JSON 与 SHA 校验。仍检查两次原生清单、当前只读命令、ACK、队列、代际和时效；不能仅凭连接快照跳过独立证明。
- 原图片工作页或旧只读 reader 丢失时，最多额外预约一次只读 reader，打开原冻结 locator URL。先写 replacement intent，再开页；原 reader 与导航记录进入 `retiredReadOnlyRecoveries`，不结算旧未知命令，不重新生成。
- 恢复页稳定性检查按真实连接页 client ID 核对没有 launch token 的连接页，不能把其他无 token 的普通用户对话计成重复控制页。程序拥有的控制页继续检查 ownership token 唯一性。
- 全局设置说明同步为“保留已登录首页，连接后最小化，工作在后台页执行”。

没有改写 BrowserBinding、既有 profile/bundle 身份、冻结请求、原回包、预算或原任务 ID。没有修改原 Web/Mac 工程、Stage 3 候选规则或私人原文。

## Windows 实测

证据在被忽略的 `.local/validation/native-messaging-20261015/`，不提交真实网页数据和凭据。

| 检查 | 结果与范围 |
| --- | --- |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib chrome_` | 163 通过，0 失败；是浏览器相关子集，不是完整 Rust 套件 |
| `node --test` 对 `apps/chrome-bridge/tests/*.test.cjs` 的实际文件列表 | 331 通过，0 失败；包括服务端→content handler→原生清单角色传递回归 |
| 前端测试与 build | 本轮前序记录 252 通过、build 通过；最终追加的修复仅涉及 Rust 和桥接测试 |
| Windows release build | 通过；存在已有编译 warning |
| 最终候选隐藏启动、保存重开 | 通过；独立 home、真实 WebView/IPC，无生成提交，详见最终启动回执 |
| 原七项目、生产表、十集源与 SDK unknown 证据保护 | 再次核对通过；没有用新任务覆盖 unknown |

多页面恢复回归先在修改前实际失败，修复按 client ID 选择后通过。

## 真实图片结果：尚未验收通过

用户正常重开专用普通 Edge 后，曾读到真实首页：page/composer/chatMain ready，hidden/unfocused，无活动请求。

随后只对既有基础图做一次显式原任务查询：原请求 ID、job、prepared 与 locator 保持不变。Native inventory 返回 `status=unknown / reason=source_owned_idle_root_unproved`，回收停在身份/空闲证明门禁；没有重新提交图片。该原因还不足以区分源页原生记录不符与已加载扩展代码不一致，不能把推测写成已定位根因。

一次后台页面刷新后，该页仅保持 `backgroundOnly=true` 的传输握手，完整网页 hello 未恢复；服务端 pending request/command 均为 0。后台健康另报告旧维护链 `maintenance_rebind_journal_hash_unresolved`，没有证据证明它是当前页面未就绪的直接原因。未清除旧维护日志、未知命令或重试预算。

额外后台维护页升级脚本在“没有真实就绪的专用页”的检查前退出，没有开维护页或发升级指令。不再对同一旧接口反复重载。

基础图尚未回收，衍生图仍等待直接父图成功入库。尚不能宣称普通 Edge 连续出图、原图下载、真实父图衍生、断线后零重发或整批稳定性通过。旧 Chrome 已完成图片不重复生成。

## 后续入口与限制

保留当前失败证据，优先查明完整网页 hello 缺失和 Native 源页记录门禁的具体字段差异，再沿原请求恢复。不得通过删除日志、重建项目/图片任务、改旧 locator 或伪造空闲/ACK 绕过。

只允许已授权个人套餐内无按张额外收费的网页图片。付费图片和所有视频均不提交。十集总验收中既有 SDK unknown 及后续集仍需继续，本文不宣称总任务完成。

## 18:01 再次恢复与方案结论（Windows，本机 UTC+8）

在用户明确要求再次尝试后，使用同一份 `resumeJobIds` 请求重新启动一次隐藏恢复进程；没有创建图片任务或更换原 conversation。恢复返回 `BACKGROUND_CHROME_HELD`，原基础图仍为 `recovery_paused`、无已入库媒体，衍生图仍等待直接父图。恢复进程已经退出，未留下一轮持续重试。

实际后台仅存在一个 reader 的 `backgroundOnly=true / pageReady=false` 握手。网页完整 hello 缺失，后台健康继续报告 `maintenance_rebind_journal_hash_unresolved`。普通 Edge 主进程没有自动化或远程调试参数。这里没有足够证据断言是 Edge 休眠、网页异常、升级日志还是特定单一原因导致全部问题。

此前追加的有限修复保留并记录：

- `tabController` 返回实际 `tabs.update` 的 `tabState`，避免原生后台页检查因字段遗漏而拒绝。
- 网页新展示结构把 gallery 放在最终答复外时，仅在唯一 user/assistant 和严格相邻范围内补充图片扫描；用户附件、多轮或多 gallery 歧义不扩大匹配。artifact 提取和原图证明共用范围。
- 服务端 WebSocket 控制 Ping 与 Native 宿主原样 Pong 独立于网页线程保活。三分钟实测 18 次 socket Pong、零 stale-close；不更新 DOM 就绪、任务状态或原图采用。这是传输验收，不是图片验收。

当前结论：**普通浏览器网页自动后台批量出图不通过本项目稳定性验收**。不继续依靠反复重载、刷新或追加 reader 证明成功，也不宣称网页版在所有环境中不可行。

转为已有的网页人工交接路径：资产页复制“最终请求” → 用户在 ChatGPT 网页生成并下载原图 → 对当前资产“导入图片” → 审核并采用。衍生图必须随请求提供直接父图；手动导入不代表旧未知云端任务已经完成或可以重发。这个路径不需要浏览器桥接，保留网页视觉效果，但没有后台自动批量承诺。本轮没有执行新的人工网页生成，因而也不把替代路径的真实生成标为通过。

界面已移除“连接后最小化即可制作”的承诺，并在设置和资产页说明实验状态与上述交接路径。保留原渠道、绑定、项目、历史和 unknown 证据，没有替用户切换 CLI/API、账号、模型或费用范围。全自动制作仍应通过受支持的生成渠道另行验收；网页与 CLI/API 的效果不能视为相同。

本轮追加验证：桥接 338/338、前端 252/252、Native 宿主独立控制帧测试 1/1；源文件与七个原项目、SDK unknown 原始证据保护再次核对通过。私有快照、URL、正文及凭据不提交。

`npm run desktop:update -- -BuildOnly` 完成 Windows release；独立隐藏启动与同一隔离目录保存重开均通过，真实 WebView/IPC/本地图片协议可用，生成请求和生成任务均为 0。该检查不涉及网页真实生成或图片视觉验收。`-PublishOnly` 已轮换同名当前版，发布目录只保留当前版与一个备用版：

- 当前 `director-desktop-next.exe`：`3efaa8a691475379f8b4d2b3dbf560d452deffa2bc000927d2356b40dca4c766`
- 备用 `director-desktop-next-previous.exe`：`7f476b9096dd02fefed206795d949156f436eee2514ef83fb45542158e7bd469`
