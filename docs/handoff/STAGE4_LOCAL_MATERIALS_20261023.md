# 第四阶段：全剧素材与本地文件夹回填

2026-10-23，本轮基于 `7e421f1`，分支 `feat/lingmo-stage4-import-20261023`。

## 使用方式

第四阶段打开已导入的分镜文档，点击场次标题右侧的 **本剧素材 → 导入文件夹 → 确认导入**。文件夹包含子目录也可读取。也可以多选图片或音频文件。

全剧素材按人物、场景、道具、声音和其他群演筛选，支持搜索名称、场次与仅待补图。素材清单和文件预览每页最多 80 项；只有主动修改绑定的行才加载完整资产选择器，避免 80 行各自重复挂载 518 个选项。

这是本机校验、复制、缩略图与入库，不上传云端，不调用生成渠道，不重新执行前三阶段。不存在云端加白排队。场次与场景资产保持独立，一个场次可绑定多个场景图片。

## 匹配与保护

- 优先精确匹配导入资产目录中的相对路径/文件名，再匹配完整文件名或类型加完整名称（包括合法 `@[名称类型]`）。不做包含匹配或模糊猜测。
- 匹配仅限当前文档命名空间和形象版本；同名歧义、多文件争用同一版本需手动区分，不自动覆盖。
- 默认保留已有正式配图。缺图时可采用导入真图；替换已有图需要显式勾选。采用比较预览时的当前图片；预览后有新选择，则保留新选择，文件仅进入候选。
- 白图和全透明图保持占位候选，不自动成为正式配图。历史占位标记按实际像素纠偏。共享媒体通过资产关联表读取，不能只依赖媒体第一属主字段。
- 文件哈希、文档、资产版本和路径在提交时重新验证；不跟随符号链接、Windows junction 或越界路径。单批上限 20,000 文件，单文件上限 2 GiB，图片另受已有像素上限保护。
- 每项独立记录成功或失败；同项目重复并发提交串行核验完成回执，避免重复媒体或覆盖采用。不同项目独立。已入库候选不会在之后的 workspace 刷新中被隐式采用。
- 保留原文、单元 ID/版本、资产版本、冻结请求、原回包和未知生成任务；入库不创建生成任务。

图片支持 PNG/JPEG/WebP。音频支持 WAV/MP3/M4A/AAC/FLAC/OGG/OPUS/AIF/AIFF，必须通过本机 ffprobe 容器检查；仅可绑定声音或 OM 资产，入候选后在片段中选择。音频容器通过不代表主观音质验收。旧缺组件失败媒体可在组件就绪后重新探测恢复。

## Windows 实测

候选程序 SHA-256：`d09d2f0597188ead1a98a149f6a2ea0ec4699075280d8f6b6988907c10b7d0e2`。

| 验证 | 结果 |
| --- | --- |
| `npm --prefix apps/desktop run test:frontend` | 271 通过，0 失败 |
| `npm --prefix apps/desktop run build` | TypeScript 与生产前端构建通过 |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib store::tests -- --nocapture` | 28 通过，1 显式忽略 |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib lingmo_ -- --nocapture` | 43 通过，3 显式忽略 |
| `lingmo_media::tests::installed_ffprobe_audio_folder_backfill_preserves_audio_type_and_source -- --ignored --exact --nocapture` | 单独明确执行，1 通过：真实 WAV、坏容器拒绝、旧失败元数据恢复 |
| `cargo rustc --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --release --features desktop,custom-protocol --bin director-desktop-next -- -C extra-filename=-candidate` | Windows 原生生产构建通过 |
| 隐藏原生 WebView 完整样本验收 | 150 集、371 场、883 片段、5407 镜、4736 标签、518 资产；计数、原文、集场切换通过 |
| 本地素材文件夹 | 518/518 精确匹配：扫描 4350.44 ms，入库 23141.73 ms；518 占位候选，采用 0，失败 0 |
| 同一隔离项目保存重开 | 原生重开通过；518 媒体、518 关联、518 资产、883 单元、0 采用、0 任务，SQLite integrity_check=ok |
| 刷新与布局 | 原生每次启动连续三轮刷新，当前场次/卡片/正文节点保留；隐藏浏览器另测素材列表、筛选、批量预览、手动绑定和反馈布局 |

完整样本原 MD SHA-256：`c3796cedc5ac9d1a2db033deec7eb48eda9deff886273d00fa4fe5620b996636`。原文、图片、SQLite、私人报告和截图均仅存本机 `.local`，未加入 Git。

本机安装了官方 FFmpeg 网站推荐的 Gyan Windows essentials 发行版 ffprobe，用于容器检查；来源和校验记录保留在本机，不把二进制混入本次源码提交。

## 验收边界

- 518 份完整素材样本为小体积白图，占位入库通过不等于真实美术成图质量通过。大尺寸成图需要额外解码和缩略图处理，不能承诺沿用该批速度。
- 视觉浏览器验收使用本地只读样本投影及模拟回填反馈；实际文件读写、匹配与持久化另由原生 IPC 和 SQLite 验收，不混称证据。
- 本轮没有调用 SDK/CLI/API 生成，没有图片或视频提交，没有渠道扣费；未重复上轮的生成验收，也未进行 macOS 原生验证。
- 已有编译未使用符号和前端体积警告保留，未扩展到无关清理。
