# 四阶段安装版验收（第 62 轮，2026-10-26）

## 当前状态

已按最新授权完成本轮功能验收并替换原桌面快捷方式对应的同名安装版；安装后的启动、保存重开、完整 MD 导入/重开共四轮隐藏 Windows 检查通过。本记录覆盖此前报告的发布条件；历史测试和失败证据仍保留。网页渠道本轮未取得明确连接结果，内容质量待办未冒称通过。

用户明确允许切换本人备用 SDK 账户，并同意各功能再测试一轮后更新安装版。第三集四项内容问题、场景候选 REPAIR、十集未全部采用分别留单，不作为此次发布前置。SDK 使用现有额度；图片仅限已经确认的免费或套餐内渠道；不提交任何视频。

## 本轮账户处理

后续尚未分配的新节点读取当前账户和模型，独立记录原工作流账户、实际前序账户、当前账户及对应证据。旧冻结请求、原始回包、预算和已采用成果不改。旧 queued 节点仍使用其原冻结身份，UNKNOWN 不因换号而重发。

账户交接属于桌面应用的明确授权合同，不宣称官方 SDK 提供了跨账户请求幂等。官方 TypeScript SDK 的启动、继续和按 thread ID 恢复说明已通过 [OpenAI Docs](https://learn.chatgpt.com/docs/codex-sdk) 核对；本次不升级 SDK 或更换模型。

## 已完成的一轮前端验收

265 项通过：四阶段工作台 41、视频界面模拟回调 34、资产生命周期 10、任务准备 9、第一阶段范围 7、MD/路由/提示词等定向检查 66、全局设置与渠道 98。只修正一项过期测试文案，产品界面生产代码未因此改变。模拟视频回调不产生真实视频请求。

Rust 账户/原流程/分镜恢复组 35 通过、0 失败、1 忽略；最终 SDK 相关组 187 通过、0 失败、8 忽略，用时 365.12 秒，包含全部 7 项新增账户回归。两组有重叠，不相加冒称独立测试总数。最终运行器 exitCode=0、未超时、受控进程树已清空；测试 EXE SHA256 为 `862f410e53f486469f4079030460877c6ca3368a118847cc5d948f6a27077e0f`。

## 本轮实际后端、连接和原生结果

| 项目 | 本轮结果 | 验收边界 |
| --- | --- | --- |
| SDK 当前账户/模型 | `gpt-6.1-sol` / `medium` 只读检查成功 | 与原工作流账户不同，身份不公开上传 |
| SDK 原流程真实续跑 | 原第一阶段工作流完成，2 个文字节点 completed；重开后任务不变，原 workflow snapshot SHA 不变 | 测试第一阶段，不冒称新增分镜已创作/采用；未新建 workflow |
| 第四阶段部分配图 | 8 资产、2 片段、12 镜；3 张有效图导出，待复核场景继续排除；重开通过，642ms | 不补生、不擅自采用；历史 jobs/raw/adoptions/原媒体保留 |
| 外部完整接续包 | 新项目 1 集、4 单元、29 镜、11 项图片范围，26 份包文件验证，重复导入和重开通过，2715ms | 导入 jobs/media/adoptions 为 0 |
| 完整灵墨 MD | 150 集、371 场、883 片段、5407 镜、518 资产、4736 标签逐项通过，2099ms | 原编号、原文、顺序、版本保留；150-1 仍一场两种场景资产；零生成 |
| 输入保护 | 三项后端验收前后 44 份文件 hash 一致 | 私有输入及受保护的任务、采用、媒体不变 |
| Windows 客户端 | 候选及安装后的同名 EXE 各完成启动/重开、完整 MD 导入/重开四轮隐藏 WebView 检查 | 最终安装结果与同一 EXE SHA 绑定；所有探针生成请求为 0 |
| 即梦生成 CLI | 安装、授权、账户读取和模型目录通过 | 免费资格未核实，不提交生成；目录里有模型不等于免费可用 |
| 网页图片渠道 | 正确部署组件后，status/check 均返回 `command_result_unknown`，显示 waiting_browser | 本轮无明确网页结果，不提交图片、不重发旧任务；历史成功不能覆盖本轮未就绪 |

网页检查最初的独立测试程序缺少其旁置 OpenCLI 运行组件，返回组件缺失；已把候选配套组件置于测试程序旁，用新证据目录复测，随后才得到上述浏览器状态。保留初次记录，不将测试程序打包遗漏误记为安装版缺组件。

本轮不再新增整集分镜或资产图片生产；第三阶段写词、依赖、采用、导出按本轮定向回归与后端真实素材核对，历史真实生产质量证据单列。测试和安装回执以本轮独立目录为准。

本轮执行入口：`cargo test --locked --release --lib --no-run --message-format=json`（实际带桌面 Cargo.toml 路径），以 Windows 原生受控运行器分别运行 `sdk_supervisor_handoff sdk_first_node_handoff series_frame_render --test-threads=1` 和 `sdk_ --test-threads=2`。忽略测试没有批量启用。发布构建使用 `apps/desktop/scripts/update-client.ps1 -BuildOnly`；编译警告保留，未为清除既有未使用代码警告扩大修改。

核心绑定：`libdirector_core-f9bf68dcb9e0e816.rlib`，SHA256 `81a7ed768e1c597ef6a99db295e3f44edb61de2878463238c545b85fb1d5eb11`。候选 SHA256 `b44d2bfe2475e3ec8d359aea61b13f77a6efa68381e25f43c03dc692170ddd1a`。对应前端、SDK、OpenCLI 资源已随候选校验。

安装后第一次额外传入 3 张正式图片的 partial 目录，命中探针固定的 `full folder exact matching` 断言。源码核对表明该可选参数专用于完整 518 张白占位图库，并非部分真实图库入口，因此本次参数不适用。没有修改产品或放宽断言；改用正常无图库参数，在全新 `native-ui-installed-r62b` 目录完整四轮通过。保留 `native-ui-installed-r62` 的首次失败。3 张正式图片的部分导入/导出/重开仅按独立后端验收证明，不扩大为该 GUI 探针已经覆盖。

## 安装、提交与回退

- 分支：`feat/series672-assets112-workflow-20261025`；仓库 `K1rsam/director-desktop-unified`。
- 账户切换实现提交：`a01d741c3d7192a9ac3f5eddca04b5f76d8896e7`；最新授权及前端断言提交：`f37f259`。
- 当前入口：`apps/desktop/.local/client/director-desktop-next.exe`，SHA256 `b44d2bfe2475e3ec8d359aea61b13f77a6efa68381e25f43c03dc692170ddd1a`。
- 上一版：同目录 `director-desktop-next-previous.exe`，SHA256 `5f31dff7a16c23cc4b4a1ba8bc3b3ef3b4cb0d5700c65dc93305d690c3ab4522`，精确等于发布前当前版。
- 桌面 `D:/EDY/Desktop/director-desktop.lnk` 仍指向原同名入口，无新增参数。客户端目录中只有上述两个 EXE，候选已由更新脚本清理。
- `PublishOnly` 执行前确认当前安装 EXE 无占用；未关闭用户应用或操作浏览器前台。SDK/OpenCLI 资源按程序 SHA 分开保存。

关键本机回执：`functional-pass-20261026-r62/installed-release.json`、`real-sdk-resume-and-reopen.json`、`sdk-readiness.json`（后两者亦位于 functional-pass 目录）、`native-sdk-final-r62/receipt.json`、`native-account-existing-r62/receipt.json`、`non-generating-acceptance-r62/summary.json`、`native-ui-final-r62/acceptance.json`、`native-ui-installed-r62b/acceptance.json`、`image-channel-readiness-r62b/reports.private.json`。私有日志/素材不提交，仅此脱敏结论进入 Git。

## 保留的质量待办与未测范围

- 第三集候选 24 镜、76.6 秒；22 镜为 2–5 秒，另两镜为 1.5/1.8 秒。四项内容问题为原声与动作先后、下一集入口状态、绕行空间和受力动作；候选未采用。
- 历史场景第三候选仍待修，未将下载成功或审图分数当作合格采用。
- 原十集前两集已采用，后续未全采用；本轮不冒称十集质量全部通过。
- 已采用分镜的主动精修入口、Mac 原生、NSIS/签名、外部灵墨网站上传及全部付费渠道真实生成未验收。
- 不生成付费图片，不提交视频。已有真实素材、私有剧本、请求/回包和数据库不上传 Git。

证据保存在本机 `.local/series672-assets112-20261025/` 的本轮独立目录。
