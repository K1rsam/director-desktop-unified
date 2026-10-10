# Chrome 图片回传修复 · 2026-10-10

基线：`6b5c5500af4aaad91e0ae9e6a9e94308be8c4ac7`；集成分支：`integration/windows-mac-20261008`。

## 问题与修复

网页已完成原图，但桥接器依赖的旧 DOM 回复锚点没有完成关联，原任务在 15 分钟后记为 `CANONICAL_DEADLINE_EXCEEDED`。桌面原先不能从 running/failed 回执恢复 native 原图。扩展重载又会丢失旧 launch token。即使原生证明完整，Protocol 5 standalone 回包的外层 requestId 为空，coordinator 将 artifact.requestId 写成 `recovered`，桌面严格来源校验仍会拒绝图片。

1. 原图查询支持 running、failed 与 completed 状态；保留原桥接回执，从同一原会话读取 native proof。
2. 新 launch token 采用桥接器合法前缀。旧任务兼容保留；扩展重载仅在冻结原 sourceClientId、原会话和无竞争归属均证明后恢复。
3. 校验完整提示词哈希、原任务、原 assistant、完成状态、文件身份与图片字节，再建立本地恢复 envelope。只对 standalone 空/缺失 requestId 兼容 artifact 空/缺失/`recovered` 标记；任意其他任务 ID、分支、图片或字节差异均拒绝。原回包保存为 recoveredResponse，不改桥接持久 turn。
4. failed 且有原 task ID 的 Chrome 图片继续保留任务归属。前后端批量与依赖调度均阻止切换渠道或再次生图绕过恢复。
5. 资产卡片和任务历史显示原图恢复进度、等待时长及无新信号提示。使用同一个前端 poller：活跃 Chrome 图片每 2 秒读取本地 workspace，空闲每 8 秒；这是本地读取频率。后台 provider processing 仍约 10 秒安排一次查询，engine tick 上限 5 秒；不宣称渠道查询每 2 秒。

## 实际验证

- 在原项目用正式 `tasks::resume_job(download=true)` 恢复用户已生成的图片：约 3 秒完成，1 次查询、1 次下载、0 次生成提交，状态 downloaded。
- 原图 PNG 2,230,277 bytes、1672×941。下载 SHA-256 与网页 native 字节证明一致。已核对媒体、job_media、media_assets、adoptions 的真实关联，图片加入原资产历史并自动采用。
- 原 snapshot、prepared、receipt、accountId、providerTaskId 保持不变。原项目任务数保持 125；没有新建生成任务。恢复前已制作一致 SQLite 快照；真实回包和含用户内容的诊断仅留在忽略的 `.local/validation`，不发布。
- 前端 242/242，前端 build 通过；桥接 Node 合成回归 6/6；图片调度 16/16；engine repair 24/24。
- Rust Chrome 7/7：包含 standalone sentinel 的接受/拒绝与原回包保留回归。
- Windows 发布 candidate 的 `--verify-startup` 首次运行通过，第二次同一隔离 home 的 `saveReopenVerified=true`；嵌入前端、IPC、SQLite、图片协议和软件设置持久化通过，generationRequestsSent=0。
- 已安装同名 `director-desktop-next.exe`，SHA-256 `83b398a9819a3f677029bc9d82f7b2f9ec99b1eda1813dd83e9925a0a0e61431`；previous 为更新前的 `1a5bd4d4c1b9a0d297361479ab2a5e01ceb2461c20f0384793457b4670fcb07a`。发布目录仅保留当前版和上一版 EXE。
- 通过 computer-use 实际启动已安装当前版、打开原项目第二阶段；确认原资产卡片、右侧当前图片均正常显示恢复原图，任务显示已下载。原项目完整任务/资产记录保留。

## 验收边界

本轮不提交任何图片或视频生成，只回收用户已有原图。原桥接 task 的历史 failed 回执保持原状，桌面下载成功由完整 native proof 支撑。不能把离线回归或这个原任务恢复当成全量新任务端到端生成验收。

观察到扩展内容运行时曾重载并导致部分恢复请求 40 秒超时；原标签页重新就绪后读取成功。本轮没有证实其具体 transport 根因，没有改 heartbeat/sequence 安全边界，也不保证所有网络等待都在 3 秒内结束。

此次只修改独立集成工程，CL 根索引受管业务文件没有变更，不对 CL 索引执行维护。
