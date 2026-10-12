# Chrome 当前执行队列证明

## 根因与边界

`transportHealth.tabQueue` 来自 WebSocket `open` 的 `transport.open` 队列操作内部。其 `running=true/pending=1` 是连接时自观察快照，并不证明现在仍有生成在执行。新版 hello 明示 `health.kind=connect_snapshot` 与 `observedAt`；不能用该历史数据判定当前可提交，也不能直接删除 busy 守卫。

复用既有只读 `GET /browser/owned-launch/:requestId`、`browser.tab.identify` standalone 命令和 `bridge.tab.open` 的 lookupOnly RPC，不增加通用执行协议，不扩大普通用户页 source 范围。当前源页仍须属于本程序，root URL、profile/bundle/BG/server/content 身份一致、hidden/未聚焦；原生 active=true 时须具备已验收的背景窗口双采样证明。

## 实时证明

background 从实际 `backgroundState.read(sourceTabId)` 读取 lease、commands、outbox，并读取真实 `TabOperationQueue.snapshot(sourceTabId)`。snapshot 只导出调度元数据和白名单关联字段，不导出 callback、promise、提示词或其他正文；不改变调度、取消、队列容量、release 或状态恢复逻辑。

完整 tabs/session/window/execution 事实双采样稳定后，只排除本次只读查询自身：

- `inventoryCommandId` 精确匹配当前 dispatched 的 `browser.tab.identify`，scope=standalone、mode=result、operation=control、requestId 为空。
- 当前 running item 必须为 `content:bridge.tab.open`，meta.type 相同且 meta.requestId 等于本次 extension RPC `rpcRequestId`。
- queued item 仅允许 `server:transport.ack:`，其 ackMessageId 必须来自当前 inventory 命令持久化 `command.accepted` outbox 的精确 messageId。

其他未决 command、任何 lease、外来或当前命令的非 accepted outbox、其他 queued work 都继续 held。读取期间任何完整事实变化仍返回 unknown，不清 queue/session/lease/outbox，不重复提交。

`inventory.sourceExecutionProof` 纳入 `inventoryCanonicalJson` 与其原字节 SHA256：

```json
{
  "version": "director-source-execution-idle/v1",
  "sourceTabId": 1,
  "inventoryCommandId": "actual-standalone-command-id",
  "rpcRequestId": "ext-actual-rpc-id",
  "nativeSamples": 2,
  "observedAt": 0,
  "lease": null,
  "ownCommand": {
    "commandId": "actual-standalone-command-id",
    "commandType": "browser.tab.identify",
    "scope": "standalone",
    "mode": "result",
    "operation": "control",
    "status": "dispatched",
    "requestId": ""
  },
  "otherUnresolvedCommands": [],
  "otherPendingOutbox": [],
  "ownAcceptedMessageIds": [],
  "queue": {
    "version": "tab-operation-snapshot/v1",
    "tabId": 1,
    "running": {
      "sequence": 1,
      "label": "content:bridge.tab.open",
      "meta": { "type": "bridge.tab.open", "requestId": "ext-actual-rpc-id" }
    },
    "queued": []
  }
}
```

示例仅说明类型，不是现场证明。外层必须独立返回 `inventoryCommandId/rpcRequestId`；server 和 Rust 验证 `inventoryCommandId===result.commandId`，内外 command/RPC 及 queue 关联精确一致。proof 的最大年龄为 5000ms，未来时间拒绝；原窗口/document 的 30 秒时间合同保持不变。旧无 execution proof 的回执不能自动升级为可提交。

Rust prepare/recover 对新 connect_snapshot 使用本次 fresh inventory GET，再读 clients 复核 runtime。等待 busy 不消耗生成执行预算，保留原 startedAt 和未知提交，不靠重置时间或新 job 绕过恢复。

## 本轮离线结果与部署边界

- `node --test apps/chrome-bridge/tests/*.test.cjs`：98 passed、0 failed、0 skipped。
- 5 份改动 JS 的 `node --input-type=module --check` 通过；`git diff --check` 通过。
- 新增回归覆盖真实 queue snapshot 的只读副本/白名单和原调度、实际 persisted command/lease/outbox 取料、精确本查询 RPC/ACK 排除、外来未决拒绝，以及重算 hash 后仍拒绝错关联/过期/自称 proof。
- 既有 actual standalone serverEnvelopeRouter/content/extension RPC/portRouter 返回运输回归仍通过。
- 日志仅在私有 `.local/validation/chrome-batch-20261012/live-execution-proof-all-tests.log`。

该证据证明代码回归，不代表真实图片已恢复或十集分镜完成。真实 overlay、worker 重载、fresh GET、原批次恢复由主控继续执行并单独记录；本子任务没有 live POST、模型调用、页面创建、生产数据库修改或视频提交。

稳定 overlay manifest SHA256：`d1da02d8628183e060ae5d7656e761ccfcca7047ccb5e0a1bc5f39b1fef5fb40`。共 17 份 pinned overlay 文件，保留已部署 previous hashes；未知第三方字节仍整批拒绝。

| 文件 | after SHA256 |
|---|---|
| extension/background.js | aa81eb9a777a71dae8d84e2d5dae730743bfbaf84af3b75cf4998f2c27a3d095 |
| extension/background/tabController.js | eaadda5a36035e58b6d4166c8cd893a522728eba1425aaa1cdf0bb35f7ce1ccf |
| extension/background/tabOperationQueue.js | f4fe508a12877ab22e131cb610a7c77cb7d45b63e3a571521b58696c1e865659 |
| extension/content/sessionCommands.js | 0214bb27f6ce6fb3717ec33677eff90c8f3f70521012ba4bdaf87e0e10b8de44 |
| src/browserBridge.js | c8b22067044d3f3b2b13bb00f93973d2f047d9ae9adc1b2c3f1e782fa6edcf4c |

新增 queue override before SHA256 为实际 installed 2.4.5 原件 `b4671c946509ddde596c7aa2c0973fd95380dfdac94393842875a4def1d84fb0`。

## Windows 主控恢复验证

- 实际后台 controller 已连接；只读 fresh inventory 返回 HTTP 200、confirmed_absent，canonical SHA256 一致，execution proof 双采样通过，lease、其他未决 command/outbox 与队列均为空。没有要求用户再次刷新，没有前台操作。
- Rust Chrome 专项：101 passed、0 failed。首次准备 fixture 多提供了一条仅恢复路径才会读取的 clients 回包，已修正为实际七次请求；共享 HTTP fixture 的 accept 加十秒边界，Windows accepted stream 明确恢复阻塞模式，避免测试无限 join。
- Windows 完整 debug 核心：1179 passed、1 failed、44 ignored，537.29 秒。失败为试用父资产链 fixture 的八秒推进断言；不记成完整通过。停止构建竞争后，该项独立原生复测通过（2.32 秒），整个 image_scheduler_tests 单线程原生复测 16 passed、0 failed（29.71 秒），原断言未放宽。
- 同名候选 release 构建通过，客户端 SHA256 `8fff57e4c532ea004664b4a7b70d72aa1ee65411d59a028fd9a8b7e5dbb9c0c1`，SDK 资源固定官方 0.160.1。按既有 PublishOnly 轮换当前/唯一备用。
- 上述只证明恢复补丁和工程验证；29 项真实图片批次、逐图审核、十集 SDK 分镜与第四阶段非视频验收继续进行，尚未据此宣布通过。禁止任何视频提交。
