# 独立 Edge 外部生命周期控制：隔离实测与最小接入设计

日期：2026-10-13。本文记录技术可行性验证，**没有切换生产浏览器、登录账户或提交生成任务**。

## 本机实际证据

| 项目 | 结果 |
| --- | --- |
| 浏览器 | Microsoft Edge `155.0.4283.45`，CDP protocol `1.3` |
| 隔离 | 新建无登录临时 `user-data-dir`，独立进程，`--headless=new`；未复制用户 profile、cookies 或登录数据 |
| 页面范围 | `about:blank` 和本程序扩展 worker；没有打开 ChatGPT 业务页面 |
| 扩展加载 | `--load-extension=<独立代码副本>` 实际加载 `ChatGPT Browser Bridge 2.4.5`；CDP 读取 runtime identity 与 manifest 核对一致 |
| CDP 监听 | 仅 `127.0.0.1`；原生监听记录的进程归属与本次启动 PID 一致 |
| 实例绑定 | `Browser.getBrowserCommandLine` 核对实际 headless 参数及本次独立 `user-data-dir`，不是默认浏览器实例 |
| 外部重载 | CDP attach 原扩展 worker 后调用 `chrome.runtime.reload()`；同扩展 URL 出现不同 worker target，`workerReloadVerified=true` |
| 外部关闭 | 直接调用浏览器级 `Browser.close`，专属 Edge 正常退出，不经桥接 API 或网页执行页 |
| 进程回收 | 原生 Windows Job Object 回执确认进程树清空；本轮约 6.49 秒，无超时 |
| 费用与生产状态 | 登录尝试、模型提交、视频提交均为 0；没有改正式连接配置或生产浏览器存储 |

上述日志、完整 target 对照、临时 profile 和原生回执保留在本机私有验证目录，不随提交上传。代码副本来自本机已安装组件，复制前核对其连接配置模板为空；没有复制扩展的用户存储。

## 这次验证的确切边界

已证明本机该 Edge 版本可以在**无登录 headless 隔离实例**加载现有扩展，且浏览器外部的 CDP 控制者可以核对实例、重载 worker 和结束专属进程。CDP 的浏览器级生命周期操作不依赖现有桥接客户端或 ChatGPT 页面。

没有验收：第一次有头登录、登录持久性、有头转后台是否抢焦点、真实后台图片生成、图片原图下载、休眠/浏览器重启后的原任务恢复、持续 alarm 唤醒及内存表现。headless 成功不能写成这些项目通过，也不能保证登录状态永不失效。

## 最小产品路径

1. 一次性建立应用专属、持久 Edge profile，初次通过可见官方页面完成登录。之后复用同一个目录；不复制 Chrome cookies、登录文件或 profile UUID，不操作用户正常浏览器。
2. 应用拥有独立进程的启动记录，核对可执行文件、profile 目录、PID/启动时间和 loopback CDP 目标。控制范围仅限该实例；不能按浏览器进程名批量结束进程。
3. 正常生成继续使用现有扩展、Protocol 5、冻结请求、父资产调度、原回包与采用合同。CDP 只作为外部安装检查、生命周期及恢复控制层，不直接提交提示词或绕过未知任务保护。
4. 在没有桥接执行页时，外部控制层仍能诊断、重载已证明归属的 worker，或在允许的安全状态恢复专属浏览器。正在生成或提交结果未知时，先保留并恢复原任务，不自动重启或重发。
5. 新 Edge 使用自己的 profile binding。现有 Chrome 任务保持原冻结身份；不把切换浏览器当作重置未知结果或再次生成的理由。

## 下一步最小修改文件

| 文件 | 限定职责 |
| --- | --- |
| 新 `apps/desktop/src-tauri/src/managed_browser.rs` | 独立 profile、启动身份与生命周期状态；调用受限控制层，不接管创作或任务采用 |
| 新 `apps/desktop/scripts/managed-browser-bridge.mjs` | 从本轮探针提取实例/loopback 校验及 CDP 生命周期操作；严格限定目标，返回非秘密结构化回执 |
| `apps/desktop/src-tauri/src/chrome_setup.rs` | 保留现有 Chrome 入口，增加专属 Edge 的发现、启动与连接状态；不默默改变历史绑定 |
| `apps/desktop/src-tauri/src/app_settings.rs` | 独立浏览器模式与非秘密配置的校验，保持既有设置迁移与 CAS 合同 |
| `apps/desktop/src-tauri/src/app_settings_engine.rs` | 全局连接/打开动作接入受管理实例；启动或升级不能自动提交生成任务 |
| `apps/desktop/src-tauri/src/lib.rs` | 注册新增窄模块，具体模块入口按当前工程核对 |
| `apps/desktop/src/Panels.tsx`、`apps/desktop/src/types.ts` | 展示专属浏览器、一次登录及连接/恢复状态，区分已安装、已连接和已验收 |
| 对应 Rust/Node/UI 专项 | 无扩展连接时的外部控制、首次登录后重开、未知任务保护、重复恢复不重复开页、后台焦点与下载验收 |

现有 `providers/chrome.rs`、队列、父资产与图片采用逻辑原则上复用，不因新增浏览器品牌而整体重写。真正需要新增适配的契约差异必须另有测试证据。

本轮只新增本文与本机私有探针，没有开始上述产品实现。
