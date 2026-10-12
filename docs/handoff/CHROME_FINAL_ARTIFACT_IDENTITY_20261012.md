# Chrome 原生图片最终身份一致性修复

## 问题与结果

原生图片回收先验证网页快照，再将匿名 DOM 消息锚点绑定到真实 user/assistant message ID。图片 artifact ID 包含 assistant turn key；绑定后再次读取网页会产生新的 artifact ID，却仍携带绑定前的 proof，导致服务端严格完成检查拒绝。图片已可靠入库，网页任务仍可能等到超时才释放。

现在在 nativeKeys 绑定完成后，以同份 native record 重新验证最终网页快照；要求原分支、user/assistant、最终节点、delivery 节点和原图 file ID 完全一致。proof binding 使用最终快照的 artifact ID，原观察 ID 保留为 `originalObservedArtifactId` 审计字段。回收与 request.snapshot 直接发布同一份已验证的 detached snapshot，不再自行读另一个 DOM 快照。既有服务端一对一 artifact ID、文件、字节、分支、来源和 lease 检查保持。

## 改动文件

- `apps/chrome-bridge/overrides/extension/content/conversationReconciliation.js`
- `apps/chrome-bridge/overrides/extension/content/responseRecovery.js`
- `apps/chrome-bridge/overrides/manifest.json`：只更新上述两份源码的目标 hash，并将已部署旧 hash 加入允许升级历史。
- `apps/chrome-bridge/tests/native-recovery.test.cjs`

目标 SHA256：

| 文件 | SHA256 |
| --- | --- |
| conversationReconciliation.js | 627c9333fb97c90ffb633ebc6e264fcf627b0de1aafbeb2d1a933cc6ea92149b |
| responseRecovery.js | 38f6623f4b84c666a6b0ab8bde009aeeb5413d78b533e766c90f1321b90692ff |

## 验证与边界

`node --test apps/chrome-bridge/tests/native-recovery.test.cjs apps/chrome-bridge/tests/native-completion.test.cjs`：17 项通过。

`node --test apps/chrome-bridge/tests/*.test.cjs`：55 项通过，0 失败、取消、忽略。包括可逆 pinned overlay 部署测试；它仅操作临时 fixture，没有部署真实 bridge。

新增回归覆盖 blob→native turn key 变化后的 artifact ID、原观察 ID 保留、最终 snapshot 交付、错文件/重复图片/原文变化拒绝、原严格 native completion guard 的完整接受与旧 ID 拒绝，以及 request.snapshot 一致性。测试没有访问真实浏览器、账户或生成服务。

这份补丁未真实部署，不代表图片整批或 GUI 已验收。历史 raw、已入库媒体、failed terminal 不改写。已经超时且释放 quarantine 的旧任务仍需以原 request/lease/server/epoch 和准确 content cleanup 证据完成独立恢复；本补丁不清除 quarantine、不重发、不恢复其他项目。

旧准备失败中，普通 owned-tab 创建记录只保留 launchToken、requestedUrl、createdAt、serverUrl；它不保存完整原 command/opener/runtime nonce。仅依据当前同 PID、同浏览器和当前空白页不能伪造历史 prepare 冻结证明。新 prepare 恢复应冻结原执行意图；旧任务缺证据时保留记录并明确标记缺口。
