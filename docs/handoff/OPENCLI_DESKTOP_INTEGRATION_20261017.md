# OpenCLI 网页生图接入桌面端 · 2026-10-17

## 范围与版本

- 分支：`integration/opencli-desktop-20261017`，研究基线 `a433e99`。
- 上游 OpenCLI 固定 commit：`b9213a15d63df578612ddaedc4e3b5c3428d5305`，浏览器 Bridge 1.0.24。
- 使用用户已登录的普通 Edge；用户已确认套餐内网页图片无按张额外扣费。本轮桌面接口新生成 5 张合成图片，视频提交 0；未切换到图片 API 或其他渠道。
- 本文只验收这次网页图片接口；不替代全剧十集、SDK、第三阶段创作和其他渠道的历史/后续验收。

## 实际接入

1. `ProviderConfig.browserTransport` 显式冻结 `opencli/v1`、profile、账户摘要、运行组件和状态目录。旧字段缺失的历史请求仍走旧 Bridge，不迁移或重发。
2. 全局设置的“连接 OpenCLI 网页出图”使用既有 `chrome.connect/check/status` 接口；连接无须选择项目。新图片任务保留原 Rust 队列、直接父图依赖、导入与自动采用。
3. Node 适配器持久化原请求哈希、发送意图、页面与稳定会话 URL。Rust OS lease 防止并发命令；持久 active owner 保护未知提交。查询只回收原任务。
4. 网页图库与官方文件下载结果必须字节哈希一致，Rust 再核验本地文件后导入；拒绝把预览、canvas 或无来源文件作为原图。
5. 每个资产独立会话。完成并入库后，先选自有控制页再关闭程序拥有的任务页，避免 OpenCLI 释放整个会话租约。
6. `web-image-bridge` 随客户端打包，固定 vendor 来源与文件哈希；组件摘要编入 EXE，运行时校验与冻结版本一致。发布版不回退开发源码；当前与备用程序的资源按各自 EXE SHA 隔离。

## 真实接口结果

通过 `Engine::new_standard` 与客户端相同的 dispatch 动作测试：全局连接、模型刷新、`assets.save`、`images.run`、自动队列推进、原图核验与导入、直接父图垫图、自动采用、Engine 保存重开。

| 轮次 | 实测结果 | 处理 |
|---|---|---|
| 第一轮 | 基础图与红围巾衍生最终 2/2 完成；包含故障恢复 | 修复 Windows `\\?\` Node 路径、下载错误路由、关闭任务页释放会话租约；已生成图片只恢复下载，没有重生 |
| 第二轮 | 基础图完成；衍生 prepare 尚未提交时误报 AUTH_REQUIRED | 定位为已登录页面 composer 尚在加载，修复为有界等待/BRIDGE_BUSY；保留原失败记录，不把本轮计为通过 |
| 最终干净轮 | 2/2 全程自动完成，约 161 秒；无 failed/auth_required/query_failed | 基础图先入库，衍生随后取父图；保存重开、两张原图核验、任务页清理通过 |

最终轮两张图片均人工看图：蓝色机器人的脸型、比例、琥珀色眼睛和银色关节保持一致；衍生在相同布局上添加红围巾。此为合成样本视觉验收，不等于已验证所有剧本资产。

正式应用 home 的全局设置已连接并返回 `ready`，模型 `chatgpt-image` 可用；连接验收没有生成任务，`projects.json` 前后字节不变。真实凭据、项目正文、浏览器身份及图片只留本机，未提交。

## 修复与回归

- 页面首次查询还未挂载 user turn 时最多等待 60 秒；15 分钟无结果转可诊断状态，保留原任务，禁止无限循环或重发。
- 已登录但 composer 加载中与真正退出/换账户分别处理。
- 旧回归暴露：没有原文的纯手写图片资产在查询“原文重解析迁移证明”时得到 `Query returned no rows`。现在无原文返回“没有迁移证明”；有记录但损坏/哈希变化仍失败关闭。原“设计需原文、手写图片无需文本 CLI”测试与迁移保护 8 项通过。
- 前端 252/252、Node 8/8、OpenCLI Rust 6/6、更新器合成测试 14/14 通过。
- 首次 Rust 全套 1398 项：1341 通过、6 失败、51 忽略。5 项是打包宿主的 Windows 上下文差异，转正常桌面父进程逐项复测通过；另 1 项为上述无原文缺陷，修复后对应测试通过。新增组件版本保护测试单独通过。不把首次全套记为全绿，也没有执行忽略的真实生成测试。

## 本机证据与复现命令

本机证据在忽略目录 `.local/opencli-native-acceptance-20261017*`，含每轮 result、原请求、原图证明与运行日志，不入仓库。最终轮位于 `-round3/home/result.json`；正式 home 的只读连接验收记录是 `opencli-channel-acceptance.json`。

```powershell
npm test --prefix web-image-bridge
npm run test:frontend --prefix apps/desktop
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib providers::opencli::tests
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib store_source_migration::tests
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib engine::repair_tests::missing_source_blocks_text_design_but_manual_image_prompt_can_queue_without_text_cli
powershell -NoProfile -ExecutionPolicy Bypass -File apps/desktop/scripts/test-update-client.ps1 -TestDirectory <new-absolute-isolated-directory>
```

需明确授权的真实图片命令（不是默认离线检查）：

```powershell
cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml --example opencli_desktop_acceptance -- <isolated-home-containing-opencli-native-acceptance> --allow-free-web-images
```

## 发布与未测范围

固定入口 `apps/desktop/.local/client/director-desktop-next.exe`，上一版为同目录 `director-desktop-next-previous.exe`。

最终构建 SHA-256：`d3deacf492bd51bcdc2b65b79837821b0de59fb853b26db52893ba075d192ee2`。组件摘要 `8b597220d4a4380790b34000d0b0f619274c997c2dd832568d9ac2c06dd6f404`，随包 1786 个文件逐项哈希验证通过，使用该资源目录的真实浏览器只读检查返回 ready（新生成请求 0）。

已执行 `update-client.ps1 -PublishOnly -CandidatePath <prepared-candidate>`，发布后当前程序哈希与上述最终构建一致；备用版 SHA-256 为 `3efaa8a691475379f8b4d2b3dbf560d452deffa2bc000927d2356b40dca4c766`（原当前版）。发布目录仅剩这两个正式 EXE，候选文件已由更新器核验后清理。桌面的 `director-desktop.lnk` 仍指向该固定当前版，无需另找新程序。

隐藏的 release WebView 验证 `--verify-startup <absolute-report-path>` 通过：embedded frontend、custom protocol、IPC、SQLite、资源协议、账户/设置面板、源文界面、资产删除恢复、项目归档/恢复/删除与媒体保留均通过；隔离验收发送生成请求 0，运行处于 unvirtualized desktop。该验收不会接管用户前台。结果在 `.local/opencli-packaged-acceptance-final-20261017/`；真实接口的保存重开另由上文 Engine 验收证明。

本轮没有前台点击 GUI 验收，也没有证明整日运行、浏览器彻底退出/重启或长时间最小化后的批量稳定性。正式任务目前一资产一会话、网页任务串行；此前单请求三图研究已回传三张，但观察到顺序 A/C/B，因此未把返回顺序作为生产资产绑定规则。长批量、同请求多资产自动分类另行验收，不能据此宣称无限稳定或任意并行数量。
