# 旧 B/C 未知文本请求只读复核（2026-10-11）

本轮未发现能解除旧请求保护的新增可信终态。B 第八集原 CLI 与其一次历史重试、C 第三集原 SDK 继续保留未知；不能用客户端退出、当前账户连通或新任务成功代替旧请求完成证明。

## 方法与边界

从两个指定生产目录复制 `project.sqlite` 到私有 `.local/validation/old-receipt-audit-20261011`，复制前后与副本 SHA256 一致；当时不存在 WAL。只打开副本，`readOnly=true`，两个 `integrity_check` 均为 `ok`。外部 attempt、receipt、journal、runtime plan 和派发文件只读核对。审计末生产 DB 与全部指定外部文件哈希不变。

未启动 Node SDK worker、模型 CLI、任何 execute/resume，也未写生产库、状态或回执；模型与媒体提交均为零。只写私有审计摘要及本说明。未重跑工程测试，也没有实测旧任务恢复。

## 指定任务现状

| 项目 / 集数 | 原任务 | 数据库历史字段 | 当前证据 |
| --- | --- | --- | --- |
| B / 第八集原请求 | `b801f2d5-8663-4319-bd39-b1eaacd1621e` | `failed`；更新时间 2026-10-09 18:58:18 UTC | 已启动 thread/turn；8 个 CLI 事件、62 次心跳；没有 `turn.completed`、用量拒绝终态、result 或 receipt |
| B / 同节点历史重试 | `ee76d92e-6a9c-4763-a340-87fe1f8e46b1` | `failed`；更新时间 2026-10-09 19:02:28 UTC | 已启动 thread/turn；4 个 CLI 事件、11 次心跳；没有可信终态、result 或 receipt |
| C / 第三集原请求 | `fba0b199-3410-4229-b815-87436b97bb2f` | `submission_unknown`；更新时间 2026-10-09 04:59:56 UTC | 原 receipt 仍为 unknown，5 个连续原事件，错误代码 `SDK_TURN_FAILED`；无 rawResult、usage 或 turnId，domainAccepted=false |

B 两条请求的本机输出结果文件均不存在，完整 prompt/schema/wire-request 仍保留。派发记录的 jobId、prompt/schema SHA256 与原请求 hash 均匹配。原请求只关联到上表未完成重试，没有已完成且 accepted 的绑定重试。

C 的 receipt 封套、原冻结 attempt 封套、journal、原请求、编译 prompt/schema、runtime-plan 身份和语义请求绑定均核对通过；五个 journal rawEvent 与数据库原事件逐值相同。外部文件修改时间仍停在 10 月 9 日，未出现迟到成稿。Rust attempt 的 observation 序号为 11，worker 原事件序号为 5，属于不同序号域，不能据此误报回执不完整。

## 恢复安全结论

现有 `text_sdk_passive.rs::historical_receipt` 仅核验封存本地证据，明确零派发、`remoteObservationSupported=false`，即使迟到封套自称完成也不直接采用未经领域校验的原包。当前 bridge 的 observe 同样是本地回执观察，无法被动查询远端原执行。执行接口拒绝再次 execute 已存在的 attempt。

SDK supervisor 与 CLI 使用 `--ephemeral`。按原三个 thread ID 核对本机 sessions/archived_sessions：未找到对应会话文件；内容搜索命中的三个文件都是其他宿主聊天中引用这些 ID，session_meta 身份不同，不能当成原模型会话。SDK/CLI 的新 resume turn 不属于同一原请求恢复，不采用。

现有 CLI 未知保护根据“已经派发但没有可信终态”只读分类历史 failed 记录，不能因本地进程不在而推定远端确定失败；跨渠道、新工作流和新重试仍不能绕过。原 B/C 不具备安全续跑依据。网络中断的上游根因、本次未知请求是否曾消耗额度及远端最终状态仍无法由本地证据证明。

## 私有证据指纹

- B 稳定 DB：`d712f6d933c7c35857b091ee001658412c3ec2ed2706758d555d4acb6e10c189`。
- C 稳定 DB：`c3bbcb25953c10e3acfcc744b80997dce99a1be2e93360882d0fed9f7bafbc35`。
- C 原 receipt：`1834d1a0d30d60c84d5ee830a0bcf335270c6c0051dc29f7d6a3af5cf650610d`。
- C 原 journal：`f0695746b7c6ea3a49c96fa5cf749828f37ec814cf69670e86c151d009689deb`。
- C 原 attempt 文件：`5965094c7666f46c4127a4e87a4d3b3e9cfff8afa10ac63b6b17baf989d4d334`。

原文、账户、完整原回包和数据库仅保留本机私有目录，不纳入本报告或 Git 发布。
