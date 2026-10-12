# Chrome 扩展重载：原页完整文档恢复与真实 hello 验收

## 已确认的故障

2026-10-12 对 Chrome 的 extension local LevelDB 日志作只读、稳定字节、CRC32C 与 WriteBatch 校验，确认原维护启动恢复已经运行。原 operation 在约 401ms 内接受导航，随后删除 `bridgePendingExtensionReload` 并标为 succeeded；当时 bridge 仍无 clients。

此前 `navigateTab` 只执行 `chrome.tabs.update({url})`。恢复 URL 主要增加 hash；同地址或同文档 hash 导航不会可靠地重建被 extension reload 失效的 content context。导航接受也不证明源页已握手，因此原完成条件错误。

## 本次实现

- 恢复前持久化 `awaiting_source_ready`；仅原 planned tab 先恢复目标 URL，再显式 `chrome.tabs.reload`。不激活页，不操作普通用户页，不新增页面，不调用 OS browser fallback。
- 新 worker 重复唤醒只检查既有 pending，不重复导航；导航不确定时进入 held。两分钟仍没有源页受理证明时保留 held anchor，停止自动恢复循环。
- `outboxV5` 保留精确 materialized hello envelope。background 在实际 server envelope handler 成功后，只接受同一 messageId、sequence、client/content/background epoch 的 `accepted=true` ACK。
- coordinator 核对原 source tab/launch record、profile、固定 bundle、指定 extension version、nonincognito，以及实际 hello content runtime。socket.open、本地 protocolReady、导航返回不再算成功。
- 正常 reload 在调度时冻结完整原 runtime；真实 extension restart 必须观察新的 background nonce。maintenance 成功持久化之后才移除 recovery anchor；成功持久化失败仍保留 pending。
- 若 worker 在成功落盘后、删除 anchor 前崩溃，下一次恢复仅在原 operation 的已持久 connectionVerified proof 与 source/launch/profile/bundle/版本/server 完全匹配时清理 anchor，不再刷新页，不重写历史。
- 服务器可在同 authenticated loopback URL 合法重启。最终 server nonce 必须非空，等于实际 server ACK source 和 handler 后持久化的 `runtime.transport.serverEpoch`。旧 runtime 保留为 `previousRuntime`，不伪造复用旧 nonce。

## 旧错误 success 的窄恢复

旧代码已删除 pending 的现场使用窄 legacy adoption：只检查最新 active maintenance，为 `extension.reload`、succeeded、recovered、缺少 connectionVerified、24 小时内且 source 与原 preconditions tabIds/tabCount 一致。

只读取完整 session owned launch records，要求原 source 是后台、非隐身、ChatGPT 根页，launch token 合法且 server URL/profile/bundle/runtime 可验证；原 source 不能有 lease 或 unresolved command。其他页即使有 quarantine 也保留，绝不刷新。维护调度原本已禁止 active lease，legacy 只恢复原 idle root。

写入原 operation 的新 pending anchor，然后只完整刷新原 source 一次。原 succeeded/history 保留，真实握手完成后另外记录 `chatgptBridge:reloadConnectionProof:<原operationId>`。超时保留 held，不能再次 legacy adoption。

extension reload 或 idle runtime compaction 可能使历史 server nonce 为空。该字段保持 unknown，不作为旧身份证明、不制造旧 nonce；最终仍强制实际新 server ACK nonce。用户页、前台页、非根页、损坏 profile、lease、unknown command、错误 operation 及已有 connection proof 都不触发恢复。

## 验证与边界

首次 source-ready 候选执行 `node --test apps/chrome-bridge/tests/*.test.cjs` 为 77 passed、0 failed、0 skipped。其中新增 11 项 source-ready 回归覆盖同文档完整 reload、错误 identity/nonce/version/bundle、late hold、有界一次恢复、持久化失败、合法 server restart、crash 后精确 proof 清理、legacy 历史保留及 scope 拒绝。后续 controller/inventory/self-command 合并后的最终全 bridge 回归为 89 passed、0 failed、0 skipped。

日志：`.local/validation/chrome-batch-20261012/reload-source-ready-all-tests.log`。这是离线行为回归；Chrome 自然 worker 唤醒、实际原页 hello、焦点和内存观察由主控记录，不能仅据这些测试标为真实链路已验收。

扩展无 externally_connectable/onMessageExternal，当前无既有 Chrome debugging 通道。现有 `bridge.connection.watchdog` alarm 每分钟唤醒 worker；overlay 能否在真实 worker 停休后加载新模块须现场证明。无需也不得重启 Chrome、清 storage 或新开 maintenance 页。

## 当前 source 的维护自锁修复

实际 canonical controller 作为 extension.reload source 时，snapshotIdleControllers 把当前 reload 自身已经 dispatched 的 command 也计为 unresolved，从而预 dispatch 拒绝。不是模型任务或服务 pending 问题。

只排除当前精确 sourceTabId/map key/commandId、`commandType=extension.reload`、`scope=standalone`、`mode=result`、`operation=maintenance`、空 requestId、`status=dispatched` 的自身维护命令。其它 source/controller、其它 commandId、registered/uncertain、请求级 command、错误类型/mode/operation 和任何 lease 仍阻断。不会删除或修改 runtime command。

两组新增回归验证自身实际 normalized command 合同能通过，并逐一验证其它命令、unknown 和 lease 不能绕过；该 source 成功路径保留原 command/nonce。实际最后 loader 安装可用已有临时 owned root 做一次后台 reload，避免用旧代码的 controller source 再触发自锁，无须操作用户页。

最终稳定 manifest SHA256：`af915ed8dfa9650dff02e4d5f66a982d1ec2854a4bbc18244e8bbc5a397407d7`。

- `extension/background.js`：`9e87eadd0e369ab4547a9dca69defcdb48ca3c9605fe91b71742318d5918e463`
- `extension/background/extensionReloadCoordinator.js`：`28df2ceb4eebc30c954ff98d87a710f76e2858fe1974b4024c2b9fd4f4cc7a88`
- `extension/background/outboxV5.js`：`f702f0788fedbaaeb0ef59ac9e6a136afb20d87645d35b25ec2ecc211c83c840`

manifest 保存前候选 after hash，支持精确识别第一次 overlay 字节再升级；未知第三方变更仍拒绝整批。

## 本机部署观察

主控通过 unpackaged 原生启动器完成固定清单 overlay：首次16项核验、8项变化；后两轮各仅升级1项 coordinator，原文件备份及运行状态保留。最终部署清单为上文固定版本。

空闲桥接服务核对原 PID、Node entry、端口、0 pending/0 clients 且无 workflow restore 配置后，精确更换服务进程（65292 → 101292），保持原 DATA_DIR、URL 和凭据来源，使用 `--server --no-auto-open-tab --no-reload-extension`。没有重启 Chrome、打开窗口或提交生成任务。第一次维护脚本在停止前因原生 PowerShell 缺少 Get-FileHash 失败；使用 .NET SHA256 后第二次成功，失败证据保留。

部署后的现场仍为0 clients。Chrome Service Worker 缓存只读观察仍能命中旧 coordinator/background 文本，尚未观察到新 source-ready 逻辑或新 pending/proof；不能以磁盘 overlay 已成功声称运行中扩展已更新或自动重连通过。现有批次停止标记继续保留，不盲重载、不重发29项任务。
