# 桌面 ChatGPT 网页图片适配

本组件通过 OpenCLI 控制用户已登录的普通浏览器，在 ChatGPT 网页生成图片，并核对官方原图下载字节。它不调用图片 API、Codex 图片或即梦，不导出 cookies，不支持视频。

- 上游 runtime 固定在 `vendor-source.json` 的 commit；vendor 保留 Apache-2.0 LICENSE。
- `bridge.mjs` 提供 `status/connect/prepare/submit/query/cleanup`，Rust provider 负责冻结运输配置、进程与文件校验。浏览器扩展需要先安装并连接；已有电脑已完成接入。
- 一个资产一个会话，网页任务串行；桌面批量队列和直接父资产依赖继续由原 Rust 引擎管理。研究阶段的同请求三图不会按返回顺序自动绑定资产。
- 原请求、发送意图、账户摘要、稳定会话 URL 和下载证明持久化在应用 home 的 `opencli-requests/`。查询不发送提示词；未知提交不能用新请求替代。
- 只有原图核验入库后才清理本程序创建的任务页；清理前保留自有控制页。用户浏览器页面不受清理。
- 页面无结果超过 15 分钟会保留原任务并返回诊断，不无限等待或自动重发。

准备依赖与资源：

```powershell
npm ci --prefix web-image-bridge
npm test --prefix web-image-bridge
node web-image-bridge/package-runtime.mjs
```

`runtime.sha256` 由打包器生成并编入 Rust 客户端；发布资源按 EXE SHA 隔离。发布版不回退到源码目录，旧冻结任务保留对应版本组件。`node_modules`、生成资源、账户和真实产物不入 Git。

真实验收入口是 `apps/desktop/src-tauri/examples/opencli_desktop_acceptance.rs`，必须显式传入隔离 home 和 `--allow-free-web-images`；它会真实生成两张合成测试图片。验收报告见 `docs/handoff/OPENCLI_DESKTOP_INTEGRATION_20261017.md`。
