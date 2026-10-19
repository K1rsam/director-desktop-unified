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

## 上次提交时尚未通过

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

## 网络恢复后的一张真实验收

用户恢复系统代理后，首次检查发现 Chrome 已关闭、只有 Edge 在线。后台自动启动 Chrome 被环境审批拦截；用户正常打开 Chrome 后，实际账户和输入框检查通过，没有重新登录或重载扩展。

通过公共 `app_settings::load/save`、CAS revision 和 `HomeExecutionLease` 保存已批准的 Chrome 账户绑定：revision 50 → 51。凭据引用完整保留，没有修改原 Edge 任务快照、预算或回执。

使用当时正式 EXE `af354c0c…` 的 `--background-chrome-images` 图片专用入口执行恰好一张目标资产试稿。它使用浏览器网页图片渠道，不调用图片 API/CLI，也不启动文本或视频工作流。

实际结果：

- 新任务一次发送已确认，持久保存原会话链接；中途查询网络超时后恢复查询同一个请求，没有另建生成请求。
- 从网页发送确认到官方原图保存为 93.989 秒。任务最终 `downloaded`，后台批次 `completed`，`imagesComplete=true`。
- PNG 为 1448 × 1086、2,456,017 字节，SHA256 `15bba0a6d26e507b74c90e361a1e536b78d560170195074b4e949139c320d7d1`；官方原图身份、MIME、字节数、PNG 头和项目媒体副本 SHA 均核验一致。
- 当前形象已关联这一张入库图片。两次独立只读重新打开生产数据库，任务和采用关系均一致；这项证据是持久化重读，不冒充生产项目 GUI 重开视觉验收。
- 程序创建的工作页自动关闭，cleanup 一次完成。用户浏览器保持运行。
- 原 Edge 失败记录的冻结快照再次严格比对未变，原来为空的 prepared/receipt/taskId/canonical 请求仍为空。新批次恰好一个图片 job；本轮没有第二次图片生成，也没有视频提交。

本地目视复核：成年男性、象牙白浴袍和白色拖鞋符合输入，左肖像与右侧正/侧/背全身图一致，主体清晰，无附加标签；肖像头顶留白偏紧。实际图片为 4:3，而请求为 16:9，不能把本图判成比例完全符合要求。

## 真实验收发现的比例漏传

`GenerationRequest.ratio` 虽然保存为 16:9，旧网页消息只拼入正文，从未传给 ChatGPT。现将允许的画幅明确编译进新请求的实际网页消息，同时原文正文保持不变；非法比例不能作为消息指令注入。

只改变未来新任务的消息构造。历史 canonical 请求、saved.prompt、requestHash、原图和冻结运行资源均保留；当前已完成图片不重做、不裁切。

`npm test`（web-image-bridge）：24 项通过，覆盖全部声明画幅的实际消息、非法参数、原正文不变及历史消息不被重编译。真实新图片是否严格遵守画幅尚未额外生成验收；本轮一张授权已使用，不为测试比例追加生成。

比例修复 Windows release 已发布到原同名客户端，EXE SHA256 `03f4a66bbab05b804ae3c01be9c02f71c79d4a5735ec853c7a7d5efdf4796f14`，网页运行资源 SHA256 `054b657069bc97f82f0cbe7587d59af32adc8f3b2e328c77fb7a22e423fac5d2`。当前/备用两份 EXE 的备用为前述 `af354c0c…`，旧冻结资源仍保留。

最终版本两次隐藏启动/保存重开均通过，均为生产嵌入前端、未抢占焦点、隔离探针 generationRequestsSent=0。发布后直接加载已安装网页资源做只读检查：当前 Chrome ready，已完成原回执可读，旧冻结运行资源摘要未变；没有新提交。原图 SHA、项目媒体副本、采用关系和原失败快照再次核对一致。
