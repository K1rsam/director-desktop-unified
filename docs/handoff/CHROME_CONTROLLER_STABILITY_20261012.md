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
- 工作窗口绑定、授权检查10秒 AbortController 超时、同 port 单一连接尝试及旧 socket 回调隔离已补齐。新增超时后可重试、alarm/timer 共用一次认证、旧 close/message/open/error 和延迟 timer 不能破坏新连接的回归，全套122 passed、0 failed。
- 上述补丁在原批次到12张、桥接零 pending request/command、零活跃 lease 后通过本地维护接口后台加载；读取新实时队列 proof 后继续原任务，已经推进到13张。维护未提交生成或视频，未知结果保护不变。
- 心跳已有默认10秒 ping、30秒 stale；现场原因不能归结为“没有心跳”。授权检查缺少超时、旧 socket 回调重入属于源码审计发现并防御性修复，未声称它们就是本次现场断线根因。
- 本轮仅扩展补丁，客户端 EXE 字节未改，维持当前版及唯一备用。禁止视频、付费图片和替换渠道。

本地证据位于 `.local/validation/chrome-batch-20261012/`：`controller-window-incident-final-tests.log`、`native-original-window-repair-reload/`、`native-reconciled-window-execution-proof/`、`native-extension-owner-inspection-05/`、原批次观察和原任务回执。最终数量与未验收范围以新增最终记录为准。

## 续跑发现的提示词正文提取缺陷

原批次推进到13张后，一项真实图片已经完成，但桥接仍显示 `waiting_for_user_turn`，不是正在生图。后台只读 layout 证明新版 presentation user bubble 含一个完整465字符正文节点及“显示更多”控件；原 native recovery 回包为 `dom_user_boundary`，userCount=1、assistantCount=1、promptMatches=false。

修复只读取 `.bg-user-message` 内唯一的 `div.whitespace-pre-wrap` 正文，排除控件标签；多正文、不明结构、正文改变仍不能通过。原完整提示词 SHA、原任务关联码、native branch、原图字节和二次原生记录回读全部保留，未放松来源校验。

补丁在零活跃请求后后台加载；同一原任务真实恢复返回 `matched_complete / exact_single_user_blob_bytes`、1个 artifact，modelSubmissions=0。无需再生成。新增对应合成回归，全套124 passed、0 failed，另补充旧 socket 已排队回调在新连接建立后不得污染当前状态的回归。原图片批次继续，未宣称全部完成。

恢复原图后还发现，客户端已将该任务的5次查询预算耗尽并记成 `recovery_paused`；旧 batch runner 只查询 processing/query_failed，跳过具有正式 providerTaskId 的暂停项，因全局未决保护再次 held。修复后每次显式继续原批次，可对每个正式已知暂停任务进行一次仅查询恢复。继续验证原账户、冻结准备快照、源文、资产和父图；不清查询预算、不 prepare/submit。失败后同次调用不会再尝试，也不会让后续新生成绕过未决保护。

Windows 原生 Rust `background_chrome_` 35 passed、0 failed，覆盖恢复成功后原请求/任务ID/快照不变、查询失败只尝试一次、原预算保留和后续新提交继续被保护。完整核心没有在本补丁重跑。客户端发布及现场续跑以随后原生记录为准。

## 重载后未提交准备快照的恢复

原批次达到20张后，7项只有 prepared，没有远端任务ID或回包；明确错误为“浏览器运行身份在准备后变化，保留原请求，未发送”。这是扩展维护改变运行 epoch 后，旧运输身份失效，不能把它当作允许重新生成的时间过期。

恢复只针对完整本地历史证明所有提交尝试都止于上述确定拒绝的原任务。核验原请求404与权威 canonical absence、原唯一后台空白工作页、固定 profile/bundle/launch、完整原生双采样和实时空闲证明，前后回读身份；不新开页面、不 prepare/upload/submit。只更新运输 epoch，原请求、完整提示词、附件、上传回执、账户和重试预算不变。CAS将原任务恢复 ready，并完整归档旧 prepared。首次提交仍通过既有跨项目队列及未知提交保护。

专项回归还发现该路径误调用只接受 queued/ready 的生成队列检查，导致 failed 原任务始终被跳过；现在只在真正生成时执行该检查。新增另一未知提交存在时恢复不会绕过提交保护、重复重载保留历史、忙时持久延后及载荷漂移拒绝等回归。当前已运行 Chrome Rust专项117项和最终后台专项40项，全部通过；新增末项之后的最终原生组合结果另记。尚未把代码回归计为原批次全部图片通过。

最终 Windows 原生组合：`director_core-f95efbd7b799e033.exe chrome_ --test-threads=1`，118 passed / 0 failed；原生 Job Object receipt success=true、exitCode=0、timedOut=false、进程树已空。Chrome JS最终125 passed / 0 failed。证据为 `native-prepared-refresh-tests-03/` 与 `final-js-stability-tests-03.log`；完整核心及前端未在本次仅后端恢复补丁重跑。

`13dd41e` 已发布，当前EXE SHA `fcef5b7592bf70611994ead59b974e32b921f4e7ad0d8b05f5cae84e1519686f`，唯一备用为原 `085da0bd…`。原请求续跑由20张推进到21张；第一项完整旧prepared归档，零重新prepare/upload，预算不重置，所有冻结snapshot哈希不变、videoJobs=0。

真实续跑暴露另一调度缺口：已知原任务处于processing但下一查询尚未到时，其他failed准备任务过早尝试证明，此时其他页面有活跃请求，被记为本轮held；当前原任务完成后这些一次性尝试已消耗，批次结束。修复抽取不依赖候选状态的只读跨项目owner检查，在刷新前先等待既有任务，不耗刷新标记；实际生成仍走原队列门禁。增加两份失败prepared同批次逐项恢复、当前processing有延迟时不消耗另一项机会的回归，同时未知提交会阻止刷新及提交。

队列修复的最终Windows原生Chrome组合119 passed / 0 failed，receipt success=true、exitCode=0、timedOut=false、进程树已空，证据 `native-prepared-refresh-owner-tests-04/`。此前125项JS未受此纯Rust调度修改影响。

## 会话链接恢复的设计判断

用户建议持久保存 `https://chatgpt.com/c/<conversationId>`，原工作页丢失时后台重开原会话进行只读恢复。采用这一方向，保留本地任务ID作为发送前与队列身份；会话URL是定位信息，完整提示词哈希、消息分支与原图文件身份是采用依据。现有实现已校验会话ID，但仍依赖原工作页且部分恢复要求正文任务标记，本补丁没有宣称链接重开或去标记已经完成。

后续应单独实现冻结的会话定位回执和后台只读恢复页，限制同一任务唯一恢复预约，继续核验原账户、原会话、原提示词与图片归属；不因404、断线或浏览器重载重新提交。只有新增请求的无标记恢复合同通过后才移除提示词关联码，历史冻结消息保持不变。
