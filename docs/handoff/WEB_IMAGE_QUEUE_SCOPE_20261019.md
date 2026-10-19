# 网页图片排队阻塞修复 · 2026-10-19

## 现场与原因

基线 `9606d85`。用户点击第二阶段人物形象“生成图片”后，原任务持续 queued；本地记录没有 prepared、providerTaskId、发送事件或网页请求日志。它没有进入图片生成。

只读核对全部九个项目的数据库副本，旧跨项目 Chrome 门禁命中了另一个项目在 10 月 14 日留下的旧 Bridge recovery_paused 任务。新请求冻结为 opencli/v1，旧请求具有完整旧 Bridge 绑定，两者使用独立执行接口，却共用了原队列门禁。

## 修改范围

- 新 OpenCLI 队列只排除能证明属于旧 Bridge 的记录：原 requestId、taskId、prepared.request.requestId、wire.id 一致，运输字段未声明新接口，sourceClientId 非空，原消息与规范化 promptSha256 匹配。
- 无法证明来源的旧记录、损坏准备包、未知运输及所有未决 OpenCLI 请求继续阻塞。旧 Bridge 的门禁行为保持原样。
- 不修改、终结、删除、迁移或重发旧请求；同资产的未决请求保护保持原样。
- 队列等待原因单独保存到 metadata，通过任务详情与列表投影显示具体阻塞项目和原任务。等待不写成 error，不改变原任务快照、回执、FIFO 时间，也不逐轮追加历史事件；队列可执行时清除旧原因。

## Windows 实测

- `cargo test --locked --lib chrome_queue_scheduler_tests --quiet`：7 通过，0 失败。
- `cargo test --locked --lib store::tests --quiet`：25 通过，1 忽略，0 失败；未运行该 ignored 测试。
- `npm run test:frontend`：255 通过，0 失败。
- 使用本轮编译的 Rust Store 对九个真实只读副本分别调用旧门禁和 scoped OpenCLI 门禁：唯一旧阻塞仍由旧门禁返回，OpenCLI 门禁不再被它拦住；其余八个项目均无门禁阻塞。没有修改生产数据库。
- `update-client.ps1 -BuildOnly`：Windows release 候选构建通过，包含配套前端、SDK 资源及 OpenCLI 资源。
- 候选 SHA256：`0bf697bc575a811a1af7569b7757b441ec86dc02cf9b4414045fb602739458fb`。
- 候选真实隐藏 WebView 启动与同目录保存重开均 ok=true；前端、IPC、SQLite、素材访问通过；generationRequestsSent=0。

## 尚未验收

原绑定浏览器可列出 ChatGPT 首页，简单 DOM 与立即完成的 Promise 可读；账户检查反复出现 Browser exec timeout。网页资源中也存在耗时较长的 session 请求，因此不能把扩展在线等同于页面和账户已就绪，亦不能把它直接归因为单纯网络故障。

本轮没有提交任何新的图片或视频请求。真实原任务的准备、发送、原图下载入库仍待更新客户端后继续跟踪。不能用以上离线与启动证据声称图片生成成功或长期稳定性验收通过。

候选准备完成后，当前客户端仍占用正式 EXE；已请求用户正常关闭。发布时使用原同名入口并只轮换一轮备用，不改变原请求冻结的运输资源。
