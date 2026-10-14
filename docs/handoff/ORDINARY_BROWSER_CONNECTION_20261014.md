# 普通 Chrome／Edge 登录复用与连接入口

## 问题和采用方案

用户的普通 Edge 已有 ChatGPT 登录，但原界面把独立配置的专属 Edge 标为推荐。专属 Edge 使用应用自有 profile、自动化及 CDP 参数，不继承普通 Edge 的登录。现有 `chrome.open` 又只启动 Chrome，并强制新窗口，不能从桌面设置选择普通 Edge。这两种入口容易被误认为同一个已登录浏览器。

本轮保留原专属浏览器作为可选项，优先使用既有普通 Chrome／Edge 的扩展连接。普通浏览器启动没有自动化、CDP、headless、新 user-data-dir、扩展加载或绕过检测参数；不复制 cookies、不输入凭据、不替用户登录。浏览器在当前普通配置里正常打开固定 ChatGPT 首页；仅在原浏览器 Preferences 明确登记本机同一扩展目录时选择该 profile，否则沿用浏览器自身的默认选择。旧未指定 browser 的调用仍使用 Chrome，显式选择 Edge 时不自动转 Chrome。

这不是确认网站登录风控由某参数造成，也不承诺网站必然允许后台生成。专属 Edge 旧路径和冻结任务身份保留，不能靠普通 Edge 新入口恢复另一浏览器的未知任务。

## 产品变化

- 网页渠道增加“打开普通 Edge”和“打开普通 Chrome”，主按钮改为“连接已登录浏览器”；打开浏览器不绑定执行端，不发生成请求。
- 独立配置的专属 Edge 移到可选详情，并明确需要单独登录。
- 本机桥接安装、扩展连接、实际网站登录与生成就绪分别说明。连接消息不再把本机服务启动或扩展握手称为已验证登录。
- 未连接时展开一次性扩展接入说明，沿用已经安装的扩展目录与 Bridge `/setup` 配置方式。扩展安装授权、保存连接信息由用户在普通浏览器完成；程序没有自动修改普通浏览器的扩展权限或安全设置。
- 全局持久 profile 绑定、后台专属工作页、原任务/会话恢复、队列并发和直接父资产依赖仍用原实现。没有改图像派发、prompt、预算或视频提交路径。

## 已执行检查与边界

前端全组 **252/252**，渠道真实 handlers 回归 **28/28**，均使用合成 IPC；新增回归验证普通 Edge/Chrome 请求保留其他渠道草稿、不调用 managed login/bind/生成，并展示缺连接指引。

Windows 原生 `chrome_setup::` **20/20**，其中 4 项新增普通浏览器回归；runner 0.911 秒，无超时、进程树为空。涵盖显式浏览器选择、旧默认保持、固定启动参数、非法 profile 拒绝、准确扩展路径与只读 profile 文件保护，以及原持久浏览器绑定和冲突门禁。证据位于私有 `.local/validation/ordinary-edge-reuse-20261014/`，冻结 core 测试文件及 hash 见 `core-build-proof.json`。

现场只读检查：本机桥接 HTTP 服务响应正常，连接页面 **0**，pending requests/commands 均 **0**；独立 Edge 未启动。因此普通 Edge 的已登录页面尚未被工作台扩展连接。本轮请求用户完成一次性接入后，再被动核验同一普通 profile 的连接和登录，不能把代码/合成回归算作实际普通 Edge 登录或图片验收。

原稳定性案例全 SQL 表、SDK unknown 请求/事件/文件保护继续通过。第五集原提交仍 unknown，未重发、未关闭或替换；没有提交任何图片或视频。

Windows 原生 `app_settings_engine::tests` **11/11**，runner 2.673 秒，无超时、进程树为空；核验全局设置保存重开、旧数据库保护、原媒体凭据/回执恢复以及渠道检查不自动生成。证据 `global-settings-native/`。

## 最终 Windows 客户端安装

产品源码提交 `8998e3ae161040393dca56eebdd8d342cc1fdf64`，已上传 `integration/windows-mac-20261008`。`update-client.ps1 -BuildOnly` 通过 SDK 资源准备、TypeScript/Vite 与 Cargo release。候选经 Windows Job Object 原生 runner 隐藏启动 5.804 秒、保存重开 2.312 秒，均无超时且进程树为空；内嵌前端、IPC、素材协议、资产删除恢复、项目归档恢复与持久化通过，生成请求/任务为 0。没有操作用户的前台 Edge 或执行真实登录。

`-PublishOnly` 已更新同名 `apps/desktop/.local/client/director-desktop-next.exe`，SHA-256 `17c375316729b9d7636c8cb787eff7f02176f1e6cfa780e210e5d1039b9282da`。唯一备用仍为 `director-desktop-next-previous.exe`，保留上一轮产物 SHA-256 `e1e3803a84d84d0689e3b9a0a4ea4f14fab14c0ec3caebdd6b17a5bbda82ab8f`。两版各自 9 份 SDK 资源原字节核验通过，目录恰好两个客户端 EXE，候选已清理；私有冻结测试产物保留。

证据：`client-build.log`、`startup-native-first/`、`startup-native-reopen/`、`client-startup/`、`client-publish.log`、`installed-proof.json`，均在上述私有证据目录。发布后生产全表、原 unknown 任务文件、先前已完成节点与源/资产/单元/事件仍精确一致。

发布后桥接再次只读检查：连接页面仍 **0**，pending requests/commands 为 **0**。待用户完成普通 Edge 的一次性扩展安装和 Bridge 保存连接，再核验同一原浏览器的后台工作页与实际图片链路。网站风险提示的确切原因、普通 Edge 实际登录和后台生成尚未验收；本轮的工程回归与隐藏 GUI 验证不能代替这些项目。

## 扩展安装后的实测与 setup 状态误报

用户提供截图确认普通 Edge 的 Default 配置已安装并启用扩展 2.4.5；setup 页显示绿色 `Connected and ready`。只读源码核对发现，该旧组件页面仅凭服务级 `activeClient` 就显示此文字，不检查当前 setup 查看者的浏览器、网站登录、`pageReady` 或 composer。截图的 setup 绿灯不能证明普通 Edge 已连到服务。Edge 扩展“服务工作进程不活动”仅表示该时点待机，不证明安装失败。

本机服务实际收到 1 个后台控制页，仍使用此前的 profile/controller 身份。新原生 inventory 在两个样本中证明该程序专属页没有 lease、其他未决命令或待回包；布局捕获显示网站错误恢复界面、没有 composer。仅在空闲证明后后台刷新同一控制页一次，得到原回执；没有重载扩展、切换用户焦点或重新提交生成。刷新后错误恢复表面消失，但 composer 仍缺失。用户的普通 Edge 真实生成能力尚未验收。证据位于私有 `ordinary-edge-reuse-20261014/` 的连接、布局、原生 inventory 与一次刷新记录，包含用户运行信息，不进入 Git。

桌面端连接检查新增窄门禁：当同一正式 opener 明确报告 `pageReady`、`composerReady` 或 `chatMainReady=false` 时，返回 `waiting_browser` 并说明扩展握手与输入框就绪的区别。缺省历史字段继续保留原兼容行为，碰撞、忙态、原绑定和原请求门禁不变。这是渠道状态投影修复，没有改图片 submit、旧 prepared、原回包或后台队列。界面在等待浏览器时展开接入说明，并提示必须在实际 ChatGPT 页的 Bridge 面板保存连接；旧外部组件 `/setup` 页面本身本轮未修改。

前端全组 **252/252** 通过。Windows 原生 `chrome_setup::` **21/21** 通过；新增回归以合成 HTTP 响应复现“transport ready但无输入框”，验证只做 GET、正式 profile 保留且不显示 ready。证据 `browser-page-readiness-20261014/chrome-setup-native/`，无超时、进程树为空；本轮安装状态另行追加。原稳定性案例全表及第五集 unknown 原文件再次精确不变，图片与视频提交均为 0。
