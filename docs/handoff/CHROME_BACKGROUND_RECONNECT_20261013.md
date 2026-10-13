# Chrome 后台重连修复与交付边界

日期：2026-10-13。集成分支：`integration/windows-mac-20261008`；修改前提交：`81ac0ea3e4f0fa233020ccd0a64f35a42bda0ee6`。本说明随修复一并提交。

## 用户问题与采用方案

用户不应为普通断线反复重载扩展。桌面端固定使用已安装组件的 `extension/`，由扩展的后台 worker 自主唤醒、恢复连接，持久图片队列在执行页真正就绪后继续。扩展包用于安装或更新，不能像独立 EXE 一样从外部执行，以唤醒已禁用或尚未加载的 Chrome 扩展。

本轮统一安装指引、setup 页面和 ZIP 导出的扩展来源；不新增第二套扩展身份、不移动 Chrome 登录配置、不通过前台点击完成测试。组件升级加载与平时断线重连分开处理。

## 已修复

1. 服务端 setup/ZIP 原先读取 `tools/chrome-bridge-extension/`，桌面安装与绑定使用 `extension/`。现在按服务端模块位置解析同一个目录，校验 manifest、必需入口，拒绝符号链接和不完整包。
2. worker 每次唤醒无条件创建同名 alarm 会重置下一次触发时间。现在先查询既有 alarm；冷启动先恢复维护记录，再立即运行一次 watchdog，后续约一分钟检查。Chrome 休眠可能推迟事件，不能把该间隔当成恢复时间保证。
3. 维护重载清空 session 后，旧控制页所有权仍在，但原 session nonce 无法继续使用。现在仅对有维护闲置证明、唯一归属、原生隐藏窗口和无未决工作的原控制页创建新 nonce，完整归档原 journal。维护导航先保留 120 秒完成机会，不立即重复刷新。
4. 维护前进一步检查 effects、downloads 和 outbox。未知创建、未知刷新、未决执行、用户页面和前台窗口继续阻止自动动作；缺失 runtime 不伪造成空闲 runtime。
5. 增加独立于页面端口的扩展后台状态回报。只包含版本、profile/bundle/worker 身份、连接数和短状态码；不含剧本、提示词、图片、凭据。回报只发 localhost、使用已有 Bridge token，禁止重定向。读取诊断需要 API token；不同 worker 分开保留，避免相互覆盖。
6. Windows 设置区只采信绑定 profile/bundle、90 秒内、唯一 worker 的诊断，区分“扩展后台在线、工作页恢复中”和“没有收到扩展后台连接”。诊断不能代替执行页握手，也不能让任务成为 ready。

上述恢复死锁和定时器问题已在固定旧版源码的合成环境复现；不能据此断言现场全部断线均由这几个原因触发。

## 实际验证

| 范围 | 结果 |
| --- | --- |
| `node --test`，PowerShell 展开 `apps/chrome-bridge/tests/*.test.cjs` | 161 passed，0 failed、skipped |
| `cargo test --lib chrome_setup -- --test-threads=1` | 15 passed |
| `cargo test --lib chrome -- --test-threads=1` | 152 passed，0 failed、ignored；包含上述设置专项 |
| 本地双 HTTP 服务重定向回归 | 原端点收到合成 token；重定向目标收到 0 请求 |
| `git diff --check` | 通过 |
| `update-client.ps1 -BuildOnly` | release 编译成功 |
| `update-client.ps1 -PublishOnly -CandidatePath <固定候选>` | 同名当前版更新成功，仅保留一轮备用 |
| `node apps/chrome-bridge/scripts/apply-overrides.cjs` | 20 文件全部校验，5 文件更新；状态与凭据未改 |

当前 EXE SHA-256：`d36d1aa8e2d0c7f55e0162dc2606ad089b49f18050625f426c65868ee1567ebb`。

上一轮备用 SHA-256：`6d3252a5e7b322646db74c441fbcc7f5e71d09f341ce96cacd5ad780fefba561`。候选已由发布脚本清理，客户端目录只有当前版和备用版两个 EXE。SDK 资源继续按 EXE SHA 隔离。

未做 GUI 启动/保存重开测试：当前测试要求不抢用户前台，现有启动探针会创建可见窗口。本轮 Rust 与 JS 回归不能替代 GUI 验收。

## 真实运行仍未通过的部分

现场桥接服务可响应，但最后只读核验仍为 0 个浏览器客户端、0 个 pending request/command。运行中的 Node 服务缓存旧模块；文件写入成功不表示进程已经加载新模块。此前针对该服务的重启被自动审批拒绝，返回仅为 `blocked by policy`，没有执行；本轮未用其他终止方式绕过。

本轮没有再次要求用户无补丁地反复重载，也没有操作前台。当前不能声称 Chrome 已加载新 worker、断连恢复已真实通过，或最后一张已完成。

原测试批次仍为 28 条 downloaded、2 条 failed 历史、1 条 queued；video jobs 为 0，源文校验通过。最后 queued 任务的 taskId/prepared/result 均为空，未绑定旧会话、未重新发送。之前后台运行已在 900 秒有界超时后退出，没有为了等待连接反复重开任务。

后续验收入口：先证明服务和扩展加载新代码、取得绑定 profile 的真实后台执行页；再续跑已存在的同一个 queued 请求，不创建第四次重试。完成新会话生成、原图下载、父图核对与采用后，才能把最终批次标为 29 downloaded、2 failed 历史、29 项资产采用。免费图片授权继续有效，任何视频与付费图片均不提交。

所有含真实项目正文、凭据或运行回包的材料只留在本机忽略目录，没有提交。
