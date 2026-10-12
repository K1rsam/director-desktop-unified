# Chrome 控制页跨扩展重载的窄恢复

## 现场问题

用户手动 reload 正确扩展并刷新 ChatGPT 后，两个普通后台空白根页已 ready，profile/bundle/background nonce 正确。调用既有 controller bootstrap 接口被 `Controller belongs to a previous browser session; no replacement was opened` 拒绝。没有创建页、没有模型提交。

原因：手动 extension reload 清除 session storage 中的 controllerBrowserSession/launch records；local storage 保留旧 `opened` controller。旧实现只允许同 session 或带明确 reloadInvalidated 的原 controller。手动 reload 没有原 maintenance marker，导致已关闭的旧 controller 永久阻断。

## 恢复规则

只改既有 `tabController`，复用现 `/browser/tabs/open` / `bridge.tab.open`，没有新增通用协议。

1. prior 必须是明确 `opened`、有效旧 browserSession、有效原 tabId、同 launch token、固定 profile、authenticated loopback server 和根页 requestedUrl。`reserved`/未知 create 不可恢复。
2. 读取实际 source runtime；source 必须 ready、socket open、后台非隐身根页，profile/bundle/BG/server/client/tab/content 身份完整。普通 ready 空白根页可以作为本次 controller bootstrap source；不能充当业务任务的 owned inventory source。
3. 原生枚举两次完整 Chrome tab 清单、全部 session launch records 和 local prior，确保前后完全稳定、profile/原 local 状态没有漂移、ownership 没有重复或矛盾。clients 列表不证明关闭。
4. 旧 tab 原生双清单均不存在：先封存完整旧 opened、旧 nonce 和只含身份/哈希的恢复证明，再持久化当前 session 的 reserved。之后仅创建一次新的后台 controller。
5. 旧 tab 仍存在：缺失 session ownership 时不能根据 tabId/root 猜测身份，不导航、不替换。只有唯一原 session launch record、目标实际 ready connection 与 source 的 profile/bundle/BG/server 一致、根页后台非隐身，才复用原页角色；不修改 URL/active，不创建页。
6. 新 create、存储或导航结果未知：保持 reserved 和 retired history，后续调用拒绝重新 create。controller 从不执行模型生成，本修复不恢复、重发或解锁未知业务任务。

历史 browserSession 不复制回当前 session。封存证明包含双枚举计数、实际 runtime、原 tab/nonce、inventory SHA256；不保存普通用户页 URL 正文。非匹配的 profile、server、client 和 runtime 均拒绝。旧状态没有不可复制的机器密钥，完整字节克隆同 UUID/配置不能靠现字段作密码学区分；不得把 profile 对齐检查宣称为密码学防克隆。

## 部署和真实验收边界

扩展注册的 worker ScriptCache 不因磁盘 overlay 自动更新，需实际 runtime reload。当前 ready 用户页不应为维护被导航或抢焦点。

若 canonical controller 尚未恢复，可以用既有 API 持久冻结一个唯一临时维护 UUID，创建一次后台 owned 空白 bootstrap 页。核对原回执/native identity/真实 hello 后，以其作为 owned-only reload source；不发 prompt/model、不替代 canonical controller。create/reload 回执未知即保原 intent/page，不重建。正式 controller ready 后以精确 expectedLaunchToken 关闭临时页。用户的普通根页、未知业务页和旧 controller hold 都保留。

本子任务执行离线回归，没有 GUI、live POST、模型调用、SQLite 修改或视频提交。主控已记录实际 overlay、精确 idle server restart、临时 owned 根页后台 reload 并重连；canonical controller 随后实际创建成功（tabId 1846646767），原生 tabState 为 active=false/autoDiscardable=false/discarded=false，ready 且 hidden/focusedfalse，模型提交 0。这证明旧 closed controller 的双 inventory 恢复已在现场运行，不需要放宽 live 丢 ownership 守卫。私有证据为 `.local/validation/chrome-batch-20261012/controller-bootstrap-after-session-repair.json`。

主控部署返回字段修复后，owned-launch inventory 以临时页为 source 已真实 200 matched、完整原字节 hash 验证成功。后来通过 ownedTabsOnly+reloadTabs=true 恢复两原 owned 页，tab ID 没变，均 ready/hidden/focusedfalse。canonical controller 当前原生 active=true，旧 inventory source 守卫仍拒绝；后述窗口证明修复尚需主控现场验收。

## Windows 验证

- `node --test apps/chrome-bridge/tests/*.test.cjs`：93 passed、0 failed、0 skipped（包含精确自身维护 command 自锁和原生背景窗口证明）。
- 新 `controller-session-recovery.test.cjs` 六组测试覆盖手动 reload 已关闭恢复、仍 live 丢 ownership 拒绝、精确 live owned 角色复用、用户/前台/unknown/duplicate hold、profile/server/source/runtime 漂移和 unknown create 防重复。
- 既有 controller/owned launch/reload/native artifact 回归同时通过；ESM syntax 与 `git diff --check` 通过。
- 日志：`.local/validation/chrome-batch-20261012/controller-session-recovery-all-tests.log`。

## owned-launch inventory 精准诊断

临时维护页已创建且后台 ready，但实际 inventory 接口返回 `unknown/inventory_contract_unavailable`，仍未通过本地判定。本轮只把 fail-closed 前置条件拆成精确 reason，没有放宽 identity/能力守卫或增加 fallback。

分别报告 lookup UUID 不合法、launch token 不匹配、runtime reader 缺失、WebCrypto digest 缺失、session.get 缺失、tabs.query 缺失，以及 UUID generator/TextEncoder 缺失。专项验证所有缺失条件不枚举、不创建、不导航、不写 storage。

源码和完整运输回归确认了 identity 丢失路径：既有 `serverEnvelopeRouter.registerAndDispatchCommand` 把 standalone content payload 的 `requestId` 覆盖为空，而旧 `sessionCommands` 使用 `payload.requestId` 作为业务 lookup UUID。不能凭原笼统 reason 认定 WebCrypto 或 constructor 缺失。

最终修复：GET 对应 domain UUID 由 `browserBridge.ownedLaunchInventory` 写入独立 `lookupRequestId`，原样穿过 standalone router，再由 `sessionCommands` 写入 extension RPC 的同名业务字段。background inventory 结果也独立携带 lookupRequestId；实际 `portRouter.commandResultBody` 仍按原协议把返回 requestId 覆盖为 standalone command 的空 requestId。BrowserBridge 验证独立 lookupRequestId、完整 runtime/token/hash/时间等原守卫后，才恢复最终 HTTP domain requestId。原 standalone `requestId=''` 及 extension RPC 自己的 requestId 关联行为均保留；缺字段直接 held，不从 token 猜 ID。

完整运输回归使用测试目录里未经修改的实际生产 `serverEnvelopeRouter` 和 `portRouter` 2.4.5 快照，固定 LF 规范 SHA256 分别为 `3938d4d9f4e289c9661c1eefcfb26e5f5ca45bf26a7dd300027f2c98f4c98ad2`、`d56e3c14659f5801afef0995e106bacd03a8231f79a22c6c112a5b8e644cbf73`。测试运行实际 standalone 注册/dispatch、content sessionCommands、RPC correlation overwrite、readonly tabController，再运行实际 portRouter 的 command.result 结算与返回覆盖，最后由实际 server inventory receipt validator 核验。快照只用于测试，不属于 overlay，不修改 release/lease 协议。

BrowserBridge receipt 校验失败现在逐项报告：版本、status、complete、lookup identity、launch token、每个 runtime 字段、generation UUID、observedAt、过期、inventory 缺失、canonical JSON/value 和 SHA256。只输出失败字段名，不输出用户正文、凭据或完整 inventory。所有守卫保持 fail-closed，真实 Chrome 运输及原生 inventory 成功仍需主控验收。

## 背景窗口中选中控制 tab 的只读 source 证明

原生 tab 的 active 表示它在所属窗口中被选中，不等于窗口当前获得焦点。canonical 的 actual active=true 必须如实保留；不能把它改成 false，也不能为修复切换用户 tab、焦点或新增页。

对 active=false 的原路径保留现有守卫。只有 active=true 时增加两份独立证据：两次完整 tabs 清单分别结合 `chrome.windows.get(actual windowId)`，要求同 id、focused=false、state 合法且前后稳定；content 在本次只读命令 RPC 时实际读取当前 `document.visibilityState/hasFocus()/Date.now()`，要求 hidden/falsefocused、时间 0..30 秒。已有 server before/after source ready/hidden/falsefocused/无activeRequest/非quarantine、profile/BG/server/content runtime 一致守卫仍适用。

使用现有 `chrome.windows.get`，无需新增扩展权限。任何窗口 API/字段/当前 document/稳定性证明不足继续 held，不清 state、不 activate/deactivate、不导航、不输入、不关闭或创建页。transportHealth.tabQueue 是 ws open 内部采集的自观察快照，不是当前队列运行证明，不能凭它清理未知 command 或 lease。

证明进入 `inventoryCanonicalJson` 及原字节 SHA256，固定 schema 只在 source native active=true 时出现：

```json
{
  "sourceBackgroundProof": {
    "version": "director-source-background-proof/v1",
    "sourceTabId": 1846646767,
    "sourceWindowId": 5,
    "nativeSamples": 2,
    "native": { "tabActive": true, "windowFocused": false, "windowState": "normal" },
    "content": { "visibilityState": "hidden", "focused": false, "contentEpoch": "actual-current-content-epoch", "sampledAt": 1791779000000 }
  }
}
```

示例数值只说明类型，不是现场证明。state 白名单为 normal/minimized/maximized/fullscreen。inventory.tabs 中唯一 source 行如实保留 active=true，并带实际 windowId。sourceBackgroundProof 的 sourceTabId/windowId 必须与其对应，contentEpoch 必须等于实际 source/runtime；server 验证同 hash 内的证明、时间及字段，不接受无校验散字段。Rust 首次 open 守卫由主控/对应代理同步采用同 schema，本子任务不改 Rust。

新增合成回归涵盖后台窗口 active 接受、前台/visible/current focused 拒绝、窗口身份/焦点变化、缺失 window API/document、unknown ownership、时间异常以及重新计算 canonical hash 后错误 proof 仍拒绝。实际 standalone outbound、当前 document RPC 采样、portRouter return 的完整运输回归也覆盖 active-window canonical proof。

最终稳定 manifest SHA256：`8e2f6fabac3128caa54ab8174ad43d78e5cebd8a2365248ecc405186e04c8b85`。

`extension/background/tabController.js` after SHA256：`a89220f69bb3adca7a28ce007e9602ecc81e0ef0d2a0895e546377c280b736f6`。manifest 保留已部署 previous hash，未知第三方字节仍拒绝整批。

`extension/content/sessionCommands.js`：`f0bdb1611dacbf5a6ead1dfaa655cdb91b269c2f4408dcf6c5faf242d4c69e0f`。

`src/browserBridge.js`：`629196e7d03b3ccb0e316cf20dab1eec40487997394ba83704413fddac7bfa2d`。

## 本轮 Windows 真实部署结果

最终 overlay 原生核验 16 份文件、更新 3 份，`stateUntouched=true`；随后只重启已确证 idle 的 Node 桥服务，Chrome 进程、配置、数据目录及用户页面保持原状。通过已有临时 owned source 对原两页完成一次 owned-only 后台重载，实际 hello 证明新版 worker 已载入，不以磁盘 overlay 代替运行证据。

正式 controller 原生 `active=true` 的现场原因已确认是其窗口 `state=minimized`、`focused=false`。最新版从当前 document 采样 hidden/未聚焦，并用两次原生窗口枚举证明背景状态。由正式 controller 查询临时 owned 页，真实 HTTP 200／matched、完整 canonical JSON SHA256 校验通过；关闭临时页后再次查询，真实 HTTP 200／confirmed_absent、完整哈希通过。临时页的原 create/close intent 与回执均保留，没有创建替代批次、发送模型或视频。

现场最后仅一个正式 controller ready，`/health` 为 `ok=true`、`needsSelection=false`，pendingRequests/pendingCommands 均 0；普通用户页不作为执行目标。先前的 `transportHealth.tabQueue.running` 是 WebSocket open 内采集的历史快照，不单独作为当前队列卡死证据，未清 session、quarantine 或未知记录来消除它。

私有证据位于 `.local/validation/chrome-batch-20261012/`：`native-background-window-proof-overlay/receipt.json`、`native-bridge-idle-restart-05/result.json`、`owned-two-rootpages-background-proof-reload.json`、`temporary-maintenance-final-close.json`。本节验收的是后台连接与只读清单，不代表 29 张图片或十集分镜已经完成。
