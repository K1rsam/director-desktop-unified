# Chrome 后台工作页与持久队列

基线：`e6b1996`，分支 `integration/windows-mac-20261008`。本轮按用户认可的后台开页、网页排队、成功入库后回收方案选择性修改，未改原版创作规则、CLI/API 并行、父资产依赖或浏览器扩展覆盖文件。

## 根因与采用的实现

- 原实现每张图创建前台工作页，完成后留下标签页。多个未选择的页面使桌面检查报 `needs_selection`。上游 `/health.ok` 也随泛用选择失败变为 false，因此仅修改 opener 仍不能闭环。
- 多页面只有同一 extension `backgroundEpoch`、bundle 和 server 身份一致时才自动选择 opener；明确选择优先，跨 runtime 或缺少身份仍拒绝猜测。该判定只绑定执行浏览器，不宣称核实了 ChatGPT 账户身份。
- 健康检查仅在 `needsSelection=true` 且已得到唯一可绑定 opener 时容许总体 `ok=false`；其他非就绪状态不绕过。请求/命令忙碌仍等待。
- 保留独立空白工作页及原生 proof，使用 `active=false`、`select=false`、`allowSystemFallback=false`，不复用用户正在交互的会话。
- Chrome 新图片在 channel 锁前和锁内核对全部已登记项目的持久任务，包括归档项目、未知提交、下载待恢复和含原回执的历史失败。原任务未完成时新图保持 queued/ready；查询下载不受此门禁阻断。
- 每轮只对等待的新图队列扫描一次，不按每张排队图片反复扫描。普通等待只 CAS 更新延迟时间，不改 FIFO 时间、不写重复状态事件或 dirty 标记。提交前 `BRIDGE_BUSY` 返回 queued 并延后，保留恢复预算。
- 原图导入、关联、采用事务完成后才能请求关闭工作页。核对原 request、prompt hash、native proof、conversation、sourceClientId、launch token，发送 expectedUrl 和 expectedLaunchToken。用户正在查看、会话变化、其他任务占用、归属不明、最后可用连接等情况保留页面；清理失败不撤销下载、不重试生成。
- 前端明确区分网页排队/准备与原任务查询，不再把尚未提交的图显示成正在核对远端任务。

参考上游：[DrA1ex/chatgpt-bridge](https://github.com/DrA1ex/chatgpt-bridge)，本机固定 `671815e1939db9effb556ea7ef67673fc21e4121`、6.4.0。采用其后台开页和归属校验关闭合同，本轮没有部署网页并行池。

## 本轮 Windows 验证

- `cargo test --locked --lib chrome`：30 通过，含新增 4 项本地 HTTP 后台/回收、3 项事务/忙等待、6 项跨项目队列回归。所有 HTTP fixture 仅回环，合成图片仅由本地测试程序写入。
- `cargo test --locked --lib image_scheduler_tests`：16 通过，包含直接父资产、多层及十一层依赖；最后 Engine 优化后重跑通过。
- `cargo test --locked --lib repair_tests`：24 通过。
- `npm run test:frontend`：243 通过；`node --test apps/chrome-bridge/tests/native-recovery.test.cjs`：6 通过。
- `update-client.ps1 -BuildOnly`：前端及 Windows release 构建成功。存在既有 unused/dead-code 编译 warning，未作为本轮范围额外处理。
- 最终候选 SHA-256：`027320fe31991a9f5c22d2d7d9427f2cfeb44976712147fe75e370097efd9fca`。
- 候选 `--verify-startup` 首次及相同隔离 home 保存重开均 `ok=true`；重开 `saveReopenVerified=true`，两次 `generationRequestsSent=0`。不读取生产项目，不提交文字/图片/视频。
- 真实连接只读核对发现两个同一 runtime 的 ready/idle 页面和泛用选择引起的 `health.ok=false`；对应新错误任务没有 task ID、prepared 或 receipt。本轮不自动重试这条失败任务，不创建真实生成任务。

## 发布与验收边界

新版已构建和完成离线/原生启动验证，不把上述结果写成真实网页生成或像素质量通过。当前用户客户端仍运行，更新暂存在私有验证目录；一次性隐藏更新助手最多等待一小时正常关闭后调用既有同名发布程序，保留原当前版为唯一 previous。助手不关闭或重启应用，不中断原任务；超时保留候选并记录 pending。实际 installed/pending 以本机私有 publication receipt 和当前 EXE 哈希为准，不以本文件提前宣称已安装。

本轮未测：真实后台页面生成、后台节流/内存节省丢弃、实际页面回收、账户额度和图片质量、macOS 原生；没有提交任何图片或视频。单客户端进程内跨项目队列已验证，跨多个 EXE 同时运行不在此门禁保证内。浏览器页面仍耗内存，未给出实测节省比例。失败但含原 task/receipt 的历史任务保守持有队列，须核对原任务才能释放，不能将失败标签当成未受理证据。网页自动化的工程限流不代表官方账号风控许可。
