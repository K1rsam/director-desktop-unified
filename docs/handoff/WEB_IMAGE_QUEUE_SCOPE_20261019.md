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

## 原任务续跑发现的恢复路由错误

用户确认关闭后，核对到仍存在旧窗口，使用 computer-use 正常关闭该窗口，再成功发布上述同名版本；备用为原 `8d481ede…` 版本。

图片专用后台执行器复用了用户原 jobId、requestId、资产版本及冻结快照，没有创建第二条图片任务。它解除队列阻塞后，第一次 OpenCLI prepare 读取达到 90 秒截止，随后恢复错误地进入了旧 Bridge `recover_prepare`，因为旧授权参数不适用而报出通用 AUTH_REQUIRED。事件证明两次均 generationCalled=false，没有 prepared、云端任务 ID 或生成回执。这不是原账户失效的证明。

修复：

- 恢复路由按冻结 transport 选择 OpenCLI；恢复只允许原请求、原组件、已完整证明未提交的准备链，存在 canonical 请求日志或本请求 active 标记时拒绝重新准备。
- OpenCLI 与旧 Bridge 的准备恢复预算分别保存；旧的误路由预算与原错误事件完整保留，不覆盖、不删除。OpenCLI 明确返回非忙碌的读取结果后记录自己的尝试，仍限制最多三次、180 秒；纯 BRIDGE_BUSY 等待不计执行次数，也不退款或重置已发生的 OpenCLI 尝试。
- 仅当完整原请求历史证明“OpenCLI 读取超时→误用旧恢复入口→旧入口通用授权错误”、且没有成功准备或任何提交记录时，允许从该误报恢复；真实 OpenCLI 登录失败、账户变化及未知提交保持阻塞。
- OpenCLI prepare 只做本地校验和页面/账户读取，将 Rust 外部截止从 90 秒收紧为 20 秒；submit 仍单独保留 90 秒及结果未知保护。

回归：`opencli` 11 项、`chrome_prepare_recovery` 10 项、`background_chrome_images` 45 项全部通过；这些筛选集合有重叠，不相加宣称总数。包含原误报复现、旧预算保留、原快照不变、真实授权错误不重试、canonical 原请求禁止重新准备和缺少旧 Bridge token 时仍进入正确 OpenCLI 校验路径。

专用 Chrome 经只读窗口核对处于最小化状态；简单同步 DOM 可读，但账户的异步读取仍超时。没有通过激活窗口抢占用户前台，没有改绑另一个在线账户。真实图片成功与长期稳定性尚未验收。

恢复路由修复的 Windows release 候选构建通过，SHA256 为 `3c3553cbad8d3e8ab0ad9b8a8619b44c24dd92654104c8c05c108e3e3ce137bf`；真实隐藏 WebView 启动通过，generationRequestsSent=0。

该版实际续跑原任务时，已从旧 AUTH_REQUIRED 误报进入正确 OpenCLI 读取路径。第一次读取仍超时，后两次桥接器报告 BRIDGE_BUSY，仍无 prepared、回执、taskId 或 canonical 请求日志，原冻结快照完全一致。该次有界运行结束后没有继续启动新任务。原误路由的旧预算也完整保留。

这次现场又暴露了 OpenCLI 在调用前就计次数、把纯忙碌等待也消耗掉的问题。随后将计数移动到非忙碌结果返回处，新增连续四次 BRIDGE_BUSY 仍保持 attempts=0、executionStartedAt=null、同一任务和快照不变的回归；最终 `chrome_prepare_recovery` 11 项通过。既有现场记录和已用预算不通过清理数据库来重置。

最终版 Windows release SHA256：`c21aa18dc70bf6d2279a690b1c7317e70954015fdae412b4392cdcfa20d4a68f`。真实隐藏 WebView 启动与保存重开均通过，generationRequestsSent=0；本轮测试二进制重新运行 background_chrome_images 的 45 项也全部通过。已发布到原同名程序，目录仅保留 current 与 previous 两个 EXE；上一轮备用 SHA256 为 `3c3553cb…`。

最终真实生成状态仍未通过：原任务已在有界恢复终点保留，冻结快照未改、无云端 ID、无 canonical 请求记录、没有新建替代图片任务、没有生成图片或视频。下一步先确认原绑定浏览器可完成只读账户核对；不能把启动或单元测试通过当作真实出图通过，也不能清理原恢复预算来继续尝试。
