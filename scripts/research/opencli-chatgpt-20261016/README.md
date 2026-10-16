# OpenCLI ChatGPT 网页适配试验

这是隔离候选补丁与回归，不是已接入桌面端的生产渠道。上游为 [jackwener/OpenCLI](https://github.com/jackwener/OpenCLI)，Apache-2.0，固定提交 `b9213a15d63df578612ddaedc4e3b5c3428d5305`，CLI 1.8.8 / 扩展 1.0.24。补丁改动及原字节摘要见 `adaptation.json`。使用上游代码仍须遵守其许可证；这里没有打包第三方完整源码、账户、私有日志或图片。

## 适配范围

- 等待期间保留原文档，避免把 `local-chatgpt:` 临时会话地址当成可导航的持久 URL；结束后读取真正当前链接。
- 识别新版 `contenteditable=true, role=textbox` 输入框中的附件。
- 严格模式只收取 `generated-image-gallery` 中的结果，防止参考图换成 blob URL 后被误认为新输出。
- 禁止 canvas 重绘兜底。资源 fetch 成功仍不等于原图验收，必须另行核对生成图区、会话及官方下载文件的实际字节哈希。

补丁仅在以下显式开关启用时改变导航/导出/结果范围；附件选择器兼容补充始终生效：

```powershell
$env:OPENCLI_CHATGPT_PRESERVE_PENDING_DOCUMENT = '1'
$env:OPENCLI_CHATGPT_REQUIRE_ORIGINAL_FETCH = '1'
$env:OPENCLI_CHATGPT_REQUIRE_GENERATED_GALLERY = '1'
```

## 离线复现

在独立目录检出上述固定提交，按上游 lock 安装依赖并构建，然后在该 checkout 应用补丁：

```powershell
git apply --check <本目录绝对路径>/chatgpt-compat.patch
git apply <本目录绝对路径>/chatgpt-compat.patch
$env:OPENCLI_CHECKOUT = '<已打补丁的 checkout 绝对路径>'
node --test <本目录绝对路径>/no-renavigate.test.mjs <本目录绝对路径>/upload-role-textbox.test.mjs <本目录绝对路径>/generated-gallery.test.mjs
```

这五项测试只验证临时地址不重开、当前附件识别、历史附件拒绝以及参考图/生成图分离，不发送任何网页请求。另需在上游目录运行其 ChatGPT adapter 测试。

`recover-saved-image.mjs` 是独立的只读恢复候选：仅接受已完成且原图字节核对过的 receipt，绑定自有页后按保存链接读取；30 秒未显示时最多再重开一次。活动生成、临时地址、未知任务、身份不匹配均停止。它没有发送、上传或生成入口。对应五项测试：

```powershell
node --test <本目录绝对路径>/recover-saved-image.test.mjs
```

真实恢复须提供本机私有 JSON receipt（`conversationUrl`、`sha256`、`bytes`、`originalBytesVerified: true`），与已核验的 profile、自有 page 身份：

```powershell
node <本目录绝对路径>/recover-saved-image.mjs <checkout> <receipt.json> <profile> <owned-page>
```

该命令不关闭或重启用户浏览器，不允许拿未知任务构造已完成 receipt。测试代码与合成 fixture 不证明任何真实生成或上游可用性。

## 运行约束

实际试验使用用户授权的已登录普通 Edge、后台自有工作页和串行派发，没有切换到 API、Codex 生图或视频。原始失败/未知提交记录保留，未重新发送。单个请求保存成功不能直接标记 `APPROVED`，参考图字节也不能充当生成结果。

本补丁没有实现本项目的资产采用、冻结任务、原请求恢复或桌面打包。具体真实结果、失败过程与未测边界以 `docs/handoff/WEB_IMAGE_GITHUB_RESEARCH_20261016.md` 为准。网页结构再次变化时应失败关闭并恢复原任务，不盲重试。
