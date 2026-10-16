# 网页生图候选源码核查与 Windows 原版检查

日期：2026-10-16（UTC+8）。分支 `integration/web-image-research-20261016`，桌面基线 `aceb8ec1ec5205e206e5fb6800cb5548fe7b8556`。

## 结果

找到具备网页生成与本地保存代码的候选，但**本机尚没有任何替代方案通过真实网页生成、原图下载、资产入库与连续批量验收**。当前只完成源码核查、原版构建、适用自动测试和原版连接检查，不能把它们合并表述为“已能稳定出图”。

优先验证 OpenCLI 的浏览器运输层，再结合本项目的冻结任务、父资产依赖、unknown 保护和原图身份校验。暂不整体替换已发布渠道，也不回滚用户项目。旧 Chrome 完成的 29 张记录保持原样；新候选不能接手重发旧 unknown 请求。

## 候选对照

| 上游 | 观察到的能力与范围 | 本轮判断 |
| --- | --- | --- |
| [jackwener/OpenCLI](https://github.com/jackwener/OpenCLI) | 1.8.8，Apache-2.0；扩展 1.0.24，使用 `chrome.debugger`、本机 daemon、会话/标签租约及后台容器；有 ChatGPT image 适配与参考图上传 | 优先运输层候选；整体 image 命令不满足本项目完整采用合同 |
| [mxzs123/chatgpt-image-cli](https://github.com/mxzs123/chatgpt-image-cli) | 0.1.0，MIT；通过官方 Playwright 扩展连接，持久 task_id、原会话恢复、参考图顺序和 Save 下载；全局 browser lock 串行 | 可借鉴任务恢复及下载流程；未证明当前新版网页兼容与本机真实生成 |
| [TianLin0509/chatgpt-web-images](https://github.com/TianLin0509/chatgpt-web-images) | 0.7.42，MIT；Windows 队列、车道、原图保存、恢复及官方 Playwright 扩展运输；文档有本机真实小批记录 | 有价值的原图/恢复参考；需要其配套运行环境与配对，不能直接当成独立已接入组件；其 Codex fallback 不纳入本项目测试 |
| [hrmasss/pixelq](https://github.com/hrmasss/pixelq) | MIT，Windows 桌面/扩展队列，0.1.1-alpha；最近源码推送 2026-03-25 | 未见足够证据证明支持本轮网页结构及严格原图、未知提交合同，不作首选 |
| [Tangdou0322/chatgpt-web-image-runner](https://github.com/Tangdou0322/chatgpt-web-image-runner) | MIT；配置、状态判断和 manifest 骨架 | README 将实际 CDP、上传、发送、下载运行器列为计划，不能当作可直接生成的成品 |
| [HelicopterHelicopter/gpt-imagegen-plugin](https://github.com/HelicopterHelicopter/gpt-imagegen-plugin) | MIT；有生成/编辑/保存代码，作者只确认 macOS 实际端到端测试；数量不足仍可能退出成功 | 不作为 Windows 无焦点批量的首选，不能照搬其成功判据 |

搜索同时发现多个仅下载图库的扩展。它们没有生成和任务恢复闭环，不能替代运行器。仓库热度只反映维护/使用信号，不等于图片稳定性证明。

## 为什么不直接整体复制 OpenCLI image

核对 `clis/chatgpt/image.js`、`utils.js` 及其测试：

- 它会包装生成/改图提示词；不能不加记录地替换本项目冻结提示词。
- 等待逻辑已经排除 data URL 的半成品 canvas 快照；上游 [#1898](https://github.com/jackwener/OpenCLI/issues/1898) 曾报告“命令成功但图片未完整生成”，不能仅看退出状态。
- `getChatGPTImageAssets` 优先取网页资源，但仍保留 canvas 重绘导出兜底。这样的字节不能自动标成网页原始文件。
- `image` 命令没有本项目的持久 unknown 提交与采用合同；超时后重新执行会开始新发送，不能让原队列按此建议盲重试。

因此可以验证其独立于 content 心跳的浏览器命令运输、后台容器、下载事件和租约；最终入库仍需核对原请求、会话/分支、原图字节与直接父图。测试应先跑原版，再按明确差异接适配，不将修补版说成上游原版通过。

## 固定来源

| 源码 | 提交 |
| --- | --- |
| OpenCLI | `b9213a15d63df578612ddaedc4e3b5c3428d5305` |
| chatgpt-image-cli | `a03d08f9a754b6b1cadbf2f5c5d168d47d8fd90e` |
| chatgpt-web-images | `44ba51a33b63a982e5a19778911eabe08969d28a` |

三个 checkout 位于忽略的 `.local/research/web-images-20261016/`。没有把第三方整仓或私有运行数据迁入正式源码。依赖安装使用各自 lock 与 `npm ci --ignore-scripts`，没有执行 npm 生命周期安装脚本；随后显式执行检查过的 build/test 命令。

## 本机实测

| 命令/检查 | 实际结果 |
| --- | --- |
| OpenCLI `npm run build` | 通过；manifest 由上游构建重新生成，源码未打补丁 |
| OpenCLI `npx --no-install vitest run --project adapter clis/chatgpt` | 5 文件、159 项通过 |
| OpenCLI `npx --no-install vitest run --project extension` | 3 文件、110 项通过；属于合成扩展测试 |
| OpenCLI `doctor` 及 `runBrowserDoctor` | daemon 正常；`extensionConnected=false`、`profiles=0`、`connectivity.ok=false`，实际连接不通过；CLI 退出 0 不能表示网页可用 |
| chatgpt-image-cli `npm run check` | build 通过，6 文件、29 项通过 |
| chatgpt-image-cli `doctor` | `status=ok`，但 token 未配置、sessions 为空；仅说明程序可运行，未连接浏览器 |
| chatgpt-web-images | 阅读实现和公开验证边界；未安装其配套环境、未执行真实账号测试 |
| 原项目/SDK unknown 保护检查 | 再次通过，没有重发原任务 |

曾在确认 Chrome 主进程未运行后，通过普通 Chrome 的无启动窗口参数尝试加载本地候选扩展，不复制 profile/cookies，不开启调试端口。没有取得扩展连接，后续未修改浏览器安全策略、未强制安装或反复重启。该尝试不能被记录为扩展安装成功，也不足以断言具体失败原因。

缺口是**新运输扩展尚未在已登录浏览器完成首次接入**。现有自建 Bridge 的连接不能冒充 OpenCLI/Playwright 扩展连接；没有通过已有断开接口安装新扩展或完成配对的受支持后台入口。没有为跳过这个缺口复制 cookie、绕过验证或改用 API/Codex 生图。

本轮实际新图片提交 0、视频提交 0；没有真实图片视觉验收。桌面当前 EXE、一个备用 EXE 和原浏览器绑定均未轮换。正式功能接入与真实验收仍未完成。

## 后续验收门槛

取得真实扩展连接后，先用原版候选在本机做独立小批基础图及一条直接父图衍生，记录原始会话、每次发送、完成数、原图格式/尺寸/哈希与耗时；随后测后台闲置、下载中断和保存重开，证明没有重新发送。任何一步只有预览/canvas、错误分支、未知提交、数量不足或需要反复人工恢复，都不计入稳定通过。未通过前不整体替换桌面运行器，不设置未经实测的并发上限，不将个人套餐内使用承诺为网站额度无限。
