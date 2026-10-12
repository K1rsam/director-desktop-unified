# Chrome 控制页持续恢复与 Windows 窗口回执

基线：`add25734b1cee2cc373680813d41618f1f7986ea`，Windows 原批次 requestId `95ee9d55-22b0-4ed6-9e6f-08b7de617f78`。私人请求、源文、数据库、账户和运行回包均留在本地，不提交。

## 根因与修改

1. Windows Chrome 的 `windows.create({state:'minimized',focused:false})` 请求不等于最小化已完成。本轮现场控制页曾报告 visible，原 bootstrap journal 处于 outcome_unknown；窗口检查也未发现最小化窗口。新创建在必要时显式 `windows.update`，随后回读同一窗口状态。原未知创建仅在预约、唯一新 launch 记录、时间、会话、连接、全量空闲状态、独占原窗口及两次稳定原生读一致时对账，不另建窗口；前台窗口、混有用户页面、身份漂移或未决工作均拒绝。
2. 旧恢复预算是累计两次，连接成功不会结算；成功 bootstrap 的旧 journal 还会挡住后续 session 丢失。现按独立断线事件保留预算，只有 exact hello 的服务端 ACK、原 controller 哈希、profile/bundle/server/content 身份和原生后台归属一致时结算。保留每次原 journal 与 proof。未知预约或未知动作不能用普通在线状态清掉。
3. 独立 alarm 即使已有连接仍核验待对账的创建。Socket 在线只证明运输连接，不代表窗口已在后台。会话标记被维护清除后采用新随机 nonce，不从旧 journal 复活 nonce。
4. 原工作页 `tabs.create` 没指定 windowId，Chrome 可选择用户最后聚焦的窗口。新工作页锁定已经验证的专属 source window，保持 active=false；源归属不明或 Chrome 返回不同窗口时拒绝导航。普通桥接页面的既有行为保留。

## 验证与边界

- Node Chrome 回归：`node --test apps/chrome-bridge/tests/*.test.cjs`，119 passed，0 failed。包含四轮独立断线、错误 ACK/epoch、第二次 session 丢失、创建回执 normal、原未知创建精确对账、用户窗口/未决执行拒绝与工作窗口绑定。
- 原生维护接口已后台加载恢复补丁，无前台操作、无模型/视频调用；重新读取扩展持久记录证明 controller 已转 `connection_confirmed`，attempts=0，旧预约保留在 history。
- 原生 owned-launch 查验取得双采样、完整哈希校验、零其他执行/待回包和后台文档 proof；原批次继续使用同一请求与29个任务，五张原图未重发。
- 续跑已从5张增长至8张；新增瑶湖与白青禾家场景原图已进行视觉核对，符合其冻结场景提示词，无人物、明显水印或损坏。逐图审核与最终批次验收仍进行中，不能把119项合成回归当成29张生成全部通过。
- 工作窗口绑定补丁在首次记录时已通过工程回归，等待活跃生成结束后的安全加载；不重载正在生成的页面。
- 心跳已有默认10秒 ping、30秒 stale；现场原因不能归结为“没有心跳”。auth preflight 缺少超时、旧 socket 回调重入为审计发现的后续风险，尚未证明本次现场触发。
- 本轮仅扩展补丁，客户端 EXE 字节未改，维持当前版及唯一备用。禁止视频、付费图片和替换渠道。

本地证据位于 `.local/validation/chrome-batch-20261012/`：`controller-window-incident-final-tests.log`、`native-original-window-repair-reload/`、`native-reconciled-window-execution-proof/`、`native-extension-owner-inspection-05/`、原批次观察和原任务回执。最终数量与未验收范围以新增最终记录为准。
