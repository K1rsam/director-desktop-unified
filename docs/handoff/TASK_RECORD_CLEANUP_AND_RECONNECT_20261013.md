# 失败记录清理与旧控制页兼容修复

2026-10-13；分支 `integration/windows-mac-20261008`；基于 `c1fac32e8e8c5e0801f34942759cbbfb4cc4d169`。用户要求清理两条已失败记录并重新开始剩余图片制作。

## 实现与实际数据操作

- 新增 `jobs.archive` / `jobs.restore`，默认任务列表隐藏已删除记录，可在“已删除记录”中恢复。仅接受已证明终结的失败图片；Chrome 必须满足原终态证明。未知、进行中、成功或证据不足的任务不能隐藏。
- 原子写入独立 metadata 和审计 event，不删除或修改 jobs 原列、冻结请求、原回包、历史预算、图片与会话关联。`job_summaries`、调度输入和跨项目未知提交门禁保持完整；`taskRecordArchived` 仅投影到工作区与任务详情。
- 新增同名客户端的窄后台入口 `--maintain-task-records <绝对 JSON 路径>`。持有同一个 HomeExecutionLease；请求绑定项目、源版本/摘要及各任务冻结 snapshot 摘要；只允许归档/恢复操作，不加载账户、验收 grant，不恢复其他项目，不启动调度器。
- 已用候选客户端在真实测试项目执行两条 `jobs.archive`。正式回执为 completed，两条均 `taskRecordArchived=true`，生成调用为 0。
- 操作前后只读副本核验：源文相同；全部任务摘要及冻结/准备/结果摘要相同；媒体记录相同；会话关联相同；video jobs 仍为 0。物理历史保留 28 downloaded、2 failed、1 queued；默认列表不再显示那两条 failed。

## 重连实际进展

前一轮桥接服务仍加载旧代码。本次用户继续测试后，经本地闲置、端口进程身份和任务状态检查，已成功重启本项目桥接服务，Chrome 进程及用户页面未重启。新 `/browser/background/status` 接口返回 200，原 404 已解决。

用户加载新版扩展后，首次取得真实 worker 回报，卡点为 `maintenance_rebind_journal_unresolved`。由此暴露前版未覆盖的历史兼容缺口：旧 bootstrap 继承上一控制页的 `reloadInvalidated`；journal SHA 包含旧标记，下一次维护覆盖它后，直接删除当前标记无法重建原完整对象。

本轮修复：

1. 旧格式仅从保存的 `retiredControllers` / `incidentHistory` 提取历史 marker，重建完整 controller，再与 journal 完整 SHA 精确比较。缺证据、摘要不符或未知 journal 继续 hold，不硬编码现场 ID 或摘要。
2. 新维护冻结完整原 controller 与 SHA，在写入新 marker 前复核完整对象，防止仅身份字段一致但其他字节已漂移。
3. 新 bootstrap 与创建对账不再继承旧 marker，原对象仍留在退休历史。
4. 分开报告 journal 状态未决与摘要不匹配，避免将不同阻塞混成同一个原因。

用户再次加载后，旧摘要关卡已通过，但现场回报 `maintenance_rebind_owner`。手动重载清空 `storage.session`，而已过期的维护记录在恢复 launch receipt 前返回，导致原控制页归属永久缺失。本轮一并修复两条路径：

- 原页存在：保全过期维护锚点，从唯一原 launch receipt、闲置证明、完整 controller/journal SHA 和两次原生隐藏页检查恢复归属；分配新 nonce，不伪造 runtime，精确 hello ACK 后才确认。
- 原页消失：两次原生枚举证明原页缺失，绑定 controller、journal、原对象、维护记录和原生页清单摘要；既有创建器再读回核对后，仅预约一次后台控制窗口。不给不存在的旧页写归属；创建结果未知时对账原创建，不重复开窗。
- 过期维护不重播旧源页导航；未知 journal、冲突、用户页、漂移继续保留。新控制页不继承旧失效标记，原件保存在历史中。

最新本机旧记录（缺少新版 `contentEpoch` / `originalController` 字段）配合合成空 session，分别测试存在/缺失两条原生页分支，均经过精确合成 ACK；后续重复检查没有重复开窗。此项仍属于兼容复现，不能代替真实 Chrome。

2026-10-13 12:09 左右用户加载补丁后，实际 Chrome 自动走 `bootstrap_completed`，建立同一 profile/bundle 下隐藏、未聚焦的就绪控制页；后续分钟级回报为 `connected`。没有重启用户 Chrome 或操作用户页面。随后续跑原 retry03 请求，没有新建第四次尝试。

旧本机诊断快照加合成新维护 marker 的实算，证明可从保存的退休历史重建完整原对象并匹配其原 SHA。此证据不是 Chrome 已执行恢复的证明。

## 验证与发布

| 范围 | 结果 |
| --- | --- |
| Windows 原生 `task_record_` 专项 | 7/7；原生 runner exitCode=0、共享写探针通过、进程树退出验证通过 |
| 初次宿主直接运行专项 | 6/7；维护入口被 MSIX 门禁提前拒绝，随后在原生 runner 完整复测通过，不计为产品失败 |
| Chrome JS 最终全套 | 178/178，0 failed、skipped；含恢复专项 69 项 |
| 任务 UI 专项 | 9/9，零网络及真实 mutation |
| 执行策略与 UI 回归 | 21/21，包含上述 9 项 |
| TypeScript 与 release 构建 | 通过 |
| 实际维护入口 | 两条已归档，原任务与媒体等校验一致，0 次生成 |
| 覆盖文件部署 | 20 文件哈希校验通过；本轮更新 2 文件；重复执行 changed=0 |

发布仍为同名 `director-desktop-next.exe`，SHA-256：`2584d13be4a290bd1feec6dbf0d9db767da67b013f1505c45fe80852812e7a9c`。

唯一备用：`d36d1aa8e2d0c7f55e0162dc2606ad089b49f18050625f426c65868ee1567ebb`。候选已清理，SDK 资源继续按 EXE SHA 隔离。

## 当前未验收范围

扩展恢复已取得真实 Chrome 连接证明。最后一张沿原 retry03 执行，原生会话读取得到 `matched_complete`；在后台执行器保存会话定位并停止后，已关闭本程序原工作页，原生枚举确认其消失，正以同一冻结请求验证按链接重开和下载。最终下载、采用及全批审计需以本节后续记录为准，不将原生图片完成视为整个验收完成。没有视频提交；禁止视频与付费图片的范围继续有效。

本轮没有可见 GUI 启动/保存重开实测，避免抢用户前台；UI 回归及后台原生维护不能代替该项。私人正文、凭据、任务回包及运行报告仅留本机忽略目录。

## 同一恢复页导航修复

12:14 的按链接恢复曾停在 ChatGPT 首页：扩展保存的 requestedUrl 是原会话地址，但网页实际 URL 未到目标。旧开页完成条件只核 launchToken/ready，错误地把首页连接当作完成；不能放宽 Rust 对完整会话 URL 的校验。

- 开页回执同时绑定原生 tabId、launchToken、完整 requestedUrl；首页 hello 不再代表会话就绪，监听后续导航观察。
- 新窄接口 `recover-owned-conversation` 只接受已预约且隐藏空闲的原 alias，原生双读核对队列及原工作页消失后，只导航该页一次；导航中断回包仍读回原页，不开新页、不改全局选择。
- Rust 在导航前持久化独立 intent，不改原 locator、冻结请求、恢复预约或预算。未知导航保留 intent，不自动重放。
- `CHROME_LINK_RECOVERY_HELD` 直接暂停本轮恢复，后续队列循环不再次空查；显式续跑仍只核对原任务一次。

新增集成回归后：Chrome JS 全套 **194/194**；Windows 原生 Chrome 专项 **142/142**，含导航持久化和 HELD 预算保护；原生 runner 验证进程树清空。TypeScript、前端构建、release 构建通过。

12:35 实际同 alias 导航到保存的原会话，保持隐藏、未聚焦，并持久化 `recovery_opened`，没有新生成。随后原图对账报 `dom_user_boundary`，继续检查网页用户消息结构；仍未将最后图片下载/采用计为通过。

当前 EXE 更新为 `5ea56456cb691f0e1f9e5328a832634bbc2629e94c52906eef239ae505e753dd`；上一轮 `2584d13be4a290bd1feec6dbf0d9db767da67b013f1505c45fe80852812e7a9c` 为唯一备用。

独立 Edge 外部控制的隔离实测和未验收边界见 [Edge 接入记录](EDGE_MANAGED_BROWSER_PROBE_20261013.md)。该探针成功不代表已迁移生产账户。
