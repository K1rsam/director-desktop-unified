# Director Desktop Unified

Windows 与 Mac 后续开发合并后的四阶段导演工作台。共享 Rust 核心、React 界面和版本化规则；后续在本仓库开发，原 Windows 工程与 Mac 交接版本继续保留。

## 本次整合

- 第一阶段保留完整原文、全剧本资产整理、来源覆盖和已有资产身份。阅读某一集不会自动缩小整理范围。F2/Q1 2.0.1 明确同批基础形象与换装的直接来源；新解析器保留跨行界面文字的原字节，不将界面字段当成对白。
- 第二阶段采用 Mac 资产美术提示词 **2.0.0**：读取剧本视觉事实，人物采用肖像与正、侧、背三视图设计；保留设计收据、最终提示词编译、冻结和恢复。合入 Windows 多层直接父资产、根资产并行、父图返回即唤醒后代、提交/查询/下载独立调度，以及当前图片和历史切换。
- 第三阶段加载 Mac 最新 3008 的 **22 份原字节规则**，以兼容 Rust 适配器执行情绪统筹、完整主创、统一整编；另有整集取料的导演精修、候选预览、显式采用和保存重开。旧请求保留原规则与执行身份。此适配器是重建实现，不能等同于移入未交付正文的 42 个 Mac 运行文件；真实新旧成稿质量仍待同源比较。
- 保留 Windows 项目归档/恢复、资产编辑/软删除/恢复、第四阶段手动配图、视频提交确认、未知提交保护及原回包。仅准备素材不会提交生成视频。
- 即梦生成 CLI 与 Canvas OAuth 分开显示。现有生成链继续使用即梦生成 CLI；Canvas 图片生成桥尚未接入。标准 Windows 客户端可显式选择官方 Codex SDK，先检查本机 Node、运行器、账户及模型，再冻结执行身份。离线验证模式保持关闭，未知提交不会自动重发。

来源、19 个 Windows 后续提交、冲突处理及本轮实际验收见 [Windows/Mac 集成记录](docs/handoff/WINDOWS_MAC_INTEGRATION_20261008.md)。第三阶段合同见 [兼容适配说明](docs/handoff/STAGE3_COMPATIBLE_RUST_ADAPTER_20261008.md)。历史 Mac 测试记录保留原日期，不冒充本轮 Windows 结果。

## Windows 构建与启动

需要 Rust MSVC 工具链、Node.js、npm 和 WebView2。首次在 `apps/desktop` 与 `sdk-bridge` 分别执行 `npm ci`。

```powershell
cd apps/desktop
npm run desktop:update -- -BuildOnly
```

构建脚本输出候选 EXE 路径。验证后以同一脚本发布：

```powershell
npm run desktop:update -- -PublishOnly -CandidatePath <已验证候选EXE绝对路径>
npm run desktop:launch
```

固定入口为 `apps/desktop/.local/client/director-desktop-next.exe`，只保留上一轮 `director-desktop-next-previous.exe`；SDK 资源按 EXE 哈希配对。发布目录与 Cargo 构建目录分开，BuildOnly 不触动当前版；程序在使用中会拒绝替换。正式构建内嵌界面，不依赖 localhost 开发服务。

新工程使用独立 `app.director.next` 数据目录，不自动改动旧工作台项目、账号文件或数据库。已有项目可通过项目包显式导入；导入后的项目是独立副本。旧项目目录保持原样。

## 验证入口

```powershell
# 仓库根目录
node scripts/verify-integration-lineage.mjs
cargo test --release --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --tests -- --test-threads=1
# apps/desktop
npm run test:frontend
npm run build
# sdk-bridge
npm run verify
```

Windows 文件/进程行为测试应从普通未打包终端执行；从 MSIX 宿主启动时使用 `scripts/windows_native_test_runner.rs` 对应的隔离运行器，检查其原生 receipt，不把宿主退出码当成测试结果。

候选 EXE 的 `--verify-startup <绝对报告路径>` 使用独立离线项目，验证真实 WebView、IPC、原文、资产和项目生命周期。第二次在同一报告父目录运行会核对保存后重开。诊断模式不调用账户、模型或生成视频。

## 保留资料与验收边界

- [完整 Windows 合并指南](docs/handoff/WINDOWS_MERGE_GUIDE_20261008.md)、[独立历史交接](docs/handoff/WINDOWS_INTEGRATION_HISTORY.md)
- [Mac 本机改动](docs/handoff/LOCAL_STANDARD_EDITION.md)、[Mac 原始验收回执](docs/handoff/LOCAL_NATIVE_RECEIPT.json)
- [规则来源](reference-skills/README.md)：72 份参考快照；独立历史分支 65 份文件是另一口径。运行器只加载显式注册项。
- [SDK 合同](docs/SDK_CORE_INTEGRATION.md)、[只读检查](docs/READINESS_CONTRACT.md)、[Canvas 诊断](docs/CANVAS_FAILURE_DIAGNOSTICS.md)
- [Mac 签名及扩展属性验证](docs/MACOS_DEVELOPMENT_BUNDLE.md)

本轮授权范围为现有 SDK 模型额度、已核实免费的图片；禁止真实视频，私有剧本、请求回包和凭据不上传仓库。Windows 合成与原生启动验收不替代真实文本质量、免费图片资格或 Mac 原生验收；具体通过项与缺口以本轮集成记录为准。
