# 网页出图浏览器选择与网络诊断 · 2026-10-19

基线：`caf689d09e96b347bada9bf221e6025a433eae78`，分支 `fix/web-image-queue-scope-20261019`。

## 现场和实际边界

用户明确允许使用当前 Chrome 账户，为指定资产创建一张独立网页试稿。原 Edge 失败任务保留，不迁移冻结账户或重置预算；不提交视频，不切到 API/CLI 图片渠道。

只读核对发现原任务绑定 Edge，而截图对应 Chrome，两者账户摘要不同。此前“连接并检查”没有传递界面所选浏览器，仍按保存的 profile/account 连接，导致选择和实际执行目标不一致。

本轮新建的 Chrome 工作页随后实际显示 `ERR_CONNECTION_TIMED_OUT` / `chrome-error://chromewebdata/`。两个浏览器的 OpenCLI 扩展均在线、pending=0；Windows 系统代理关闭，本机代理端口在监听，终端代理通道能收到远端 HTTP 返回。这证明浏览器网络未就绪，不能将其归因为登录失效、扩展断线或单纯页面休眠。没有自动修改系统代理、复制 Cookie 或绕过网站检查。

## 修改

- 界面把 Chrome/Edge 选择传到正式全局连接服务；连接按实际网页 User-Agent 区分浏览器，不能在选择 Chrome 后保存 Edge。
- 连接状态返回并显示实际浏览器类型；读取失败时明确标明类型待核对，不把下拉框默认值当作已绑定浏览器。不向严格的共享设置字段表新增显示字段，保留旧版备用程序的读取兼容。
- 普通检查继续保留账户保护。登录账户不同必须显式勾选“使用所选浏览器的当前账户”后连接，该确认只用于本次全局连接；不放入生成配置，不重绑旧任务。字段格式或浏览器选择无效时不修改绑定。
- 账户读取增加六秒网页请求截止，Rust 外部状态检查仍为二十秒截止。浏览器错误页、请求失败、401、403、429 和加载中的页面分别处理；网络失败立即返回明确状态，不进入十五次登录等待。
- 状态与界面将网络故障显示为网络异常，提示检查网络/浏览器代理；403 要求在正常网页完成网站检查，不将其误报为重新登录或重载扩展。
- 更新网页运行资源摘要。已冻结任务继续保留各自原运行资源；不覆盖历史请求、原回包和媒体。

## 已执行验证

- `node --test bridge.test.mjs browser-connection.test.mjs`：20 项通过。包括选择 Chrome 跳过已保存 Edge、不同账户默认拦截与显式切换、同类型多配置不擅自选择、浏览器错误页/网络错误不触发登录重试、401/403/429 区分、只返回账户摘要。
- `node --experimental-strip-types --test src/SettingsChannels.test.ts`：32 项通过。包括实际 React 事件到 IPC 参数、账户切换勾选、显示真实浏览器和网络错误，及保留其他渠道草稿。
- `cargo test --locked --lib opencli --quiet`：14 项通过。
- `cargo test --locked --lib app_settings --quiet`：25 项通过。筛选集合可能重叠，不相加宣称唯一总数。
- `npm run build`：TypeScript 和生产前端构建通过。
- 新诊断代码针对真实 Chrome 故障页只读运行，约 0.4 秒返回 `BROWSER_NETWORK_UNAVAILABLE`，没有图片提交。
- 生产数据库只读复核：原失败任务 snapshot 与保存证据完全一致，prepared/receipt/taskId/canonical 请求均不存在；本轮预备独立试稿尚未分配图片 job。

## 尚未通过

真实试稿生成、官方原图下载、资产采用和视觉审核尚未通过。需要先恢复该浏览器访问 ChatGPT 的网络，再完成账户检查和唯一一张已获准的试稿；不能把离线回归、打包或启动通过写成真实出图通过。

全局 Chrome 绑定只有在实际账户检查通过后才保存；没有以历史摘要代替当前账户检查。

## Windows 发布结果

- 最终 `update-client.ps1 -BuildOnly` 完成，Windows release 构建通过。
- 最终 EXE SHA256：`af354c0cfec809debcabbe4725176673adad7ec00f5a7721fb87091410ad5784`。
- 运行资源 SHA256：`0b47eec7bf0851ab66bf84f89e345908240a3dd080d6695d98405a131ae0ea57`。
- 最终候选隐藏 WebView 启动及相同隔离目录保存重开，两次均 `ok=true`、`standalone=true`、`backgroundWindowVerified=true`、`generationRequestsSent=0`。
- 已通过正式发布脚本替换同名 `director-desktop-next.exe`。仅保留当前版及一轮备用版；备用 SHA256 为 `66a75d27c57e1f3a1de3abc14e8a889c7e530569bdda9fe6fff42c731b63052e`。历史冻结资源目录保留。
- 最终源码重新执行 OpenCLI 筛选 14 项、app_settings 筛选 25 项，均通过；前端渠道交互筛选 32 项再次通过。
- 最后通过 Rust 官方配置/调用入口核对已安装资源时，Chrome 工作页已不存在，返回 `OPENCLI_OWNED_PAGE_REQUIRED`；没有把缺少工作页认定成授权失败，也没有为了验证生成而新建图片任务。
- 生产全局设置仍为 revision 50、原 Edge profile。原任务快照再次核对未变。Chrome 新试稿仅在私有目录准备请求文件，没有分配 job、没有提交图片或视频。

需要恢复正常的 Chrome ChatGPT 网页，然后显式连接已获准的 Chrome 账户，再继续唯一一张试稿的生成、原图入库及视觉审核。用户本轮账户切换授权已取得，无需再次询问。
