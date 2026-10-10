# Chrome 图片按钮账户门禁修复

基线 `3599ed7375dcbcf3d3f71f2a543b8499481fef57`。用户在 B 第二阶段已选择 Chrome 网页、chatgpt-image 和网页原生尺寸，按钮仍禁用，并要求填写设置界面根本没有的“稳定账户标签”。

根因是前后端账户合同不同：标准桌面后端以软件级保存凭据绑定 Chrome/BH 身份，允许没有手写 AccountId；AssetPanel 仍只判断旧项目账户标签。没有修改 Chrome 生成执行器、原任务绑定或未知提交恢复策略。

修复让 workspace 返回只读 `credentialStatus.bh/chrome.configured`，使用与实际执行相同的凭据门禁。仅返回布尔值，不暴露密钥、引用或哈希，不写项目数据库、不检查远程账户、不启动生成。第二阶段的单图、批量图及一键制作共用该状态；明确 false 优先于任何过期标签。没有此字段的旧预览继续沿用旧合同。即梦仍要求经过核实的官方登录。

连接提示也按渠道区分：Chrome 提示连接本机组件并检查专用浏览器；BH 提示保存服务地址和凭据。无需用户为了点亮按钮填写虚构账户名称。凭据已保存只证明本地身份可用，实际浏览器就绪、模型参数、原任务和结果校验仍由原执行器检查。

回归覆盖实际 AssetPanel 无标签启用单图/一键制作、保存参数后只调用图片入口、凭据失效时过期标签不能绕过门禁；调用均为合成 IPC，不生成真实媒体。Rust 核对软件级无标签凭据、空配置拒绝、workspace 不泄露凭据，以及读取前后项目数据库字节不变。

## Windows 验证与同名客户端发布

代码提交：`cb6b265`。Windows 本轮实际执行了前端全部 236 项测试（236 通过）、`app_settings_engine` 11 项测试和 `workspace_job_summary_tests` 3 项测试（全部通过），以及 TypeScript/Vite 构建、Rust release/custom-protocol 构建和 9 份 SDK 资源校验。Rust 其他测试未在本轮重跑。原生测试进程收据：exitCode=0、success=true、timedOut=false、elapsedMilliseconds=444092，进程树清空已验证。

验证脚本与日志位于私有 `.local/validation/run-image-credentials-20261010.ps1`、`.local/validation/image-credentials-20261010/` 和 `.local/validation/image-credentials-native-20261010/receipt.json`；不上传私有配置、运行数据库、账户数据或生成原包。

先正常关闭客户端，再执行 `update-client.ps1 -PublishOnly -CandidatePath <私有候选绝对路径>`，成功轮换同名客户端并重新打开。当前交付目录仅有两个 EXE：

- `apps/desktop/.local/client/director-desktop-next.exe`，SHA256 `1a5bd4d4c1b9a0d297361479ab2a5e01ceb2461c20f0384793457b4670fcb07a`。
- `apps/desktop/.local/client/director-desktop-next-previous.exe`，SHA256 `4da07eff1ed75ba0333404e4d407ce50ed5f8e3832e40313b2dbff2b555d14a1`。

Computer Use 在正式客户端重新打开 B 项目第二阶段，实际确认渠道仍为 Chrome 网页、模型 chatgpt-image、网页原生尺寸和 16:9；“生成图片”与“一键制作当前资产”按钮均启用，页面没有稳定账户标签警告。没有点击这两个提交按钮。首页四项目保留，资产/单元数量分别为：整合版本机验收 0/0、A 59/0、B 65/38、C 58/14。当前留在 B 的图片资产页。

## 验收边界

本轮通过的是本地凭据门禁、前端按钮和客户端重开验证。没有真实模型调用，没有生成图片或视频，不代表 Chrome 真实出图和图片质量已验收。既有 B 第 8 集、C 第 3 集未知提交仍按原保护策略处理，本轮没有继续、重提或新建流程绕过；十集端到端验收仍有缺口，详见 `FULLFLOW_VALIDATION_20261009.md`。
