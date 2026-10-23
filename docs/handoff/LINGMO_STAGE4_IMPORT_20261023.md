# 第四阶段灵墨 MD 分层导入

日期：2026-10-23。基线 `9570afb809c04b1434de05c538b0c8f6797cd91d`，功能分支 `feat/lingmo-stage4-import-20261023`。

## 使用方式

打开原桌面快捷方式，创建或打开一个项目，进入第四阶段「单元视频」。点击「选择分镜 MD」→「预览结构」→「确认导入」。不需要第一至三阶段结果。

与 MD 同目录、同名追加 `.assets.json` 的资产目录会自动读取，也可在可选附件中指定。图片 ZIP 或文件夹可选；无图时仍可完整浏览、编辑。图片必须按同版目录精确匹配，不能靠相似名字猜绑定。

选择分镜来源、剧集、场次后，上方按 CH/PR/SC/SE/OM 展示本场资产，下方展示本场片段及完整分镜。点击正文标签定位真实资产。编辑片段可输入 `@` 选择资产、粘贴完整标签、替换引用及调整配图顺序。声音和其他音视频素材显式选择媒体版本。

导入只进行本地解析、保存及已有媒体入库，不启动文字、图片或视频生成。后续素材准备和制作沿用原第四阶段能力及其渠道、费用、确认和未知提交保护。

## 实现与保留边界

- Rust 独立解析器扫描集、场、片段及镜头，原编号、顺序、UTF-8 字节范围、小数镜头时间线原样保存。场次按原 `N-M` 行建立，不按 SC 资产数拆分。
- 不可变 MD、SHA-256、目录及片段原件范围存入项目 metadata。编辑版本、内部标记编译稿及渠道 `@图片N`/音视频编号分开保存；不覆写原件，不伪造第一阶段来源或第三阶段回执。
- 原集场索引与完整正文分离，workspace 只返回单元摘要；按当前场读取正文。片段仍使用原工作台的 24 条分页。
- `@[名称CH/PR/SC/SE/OM]` 按完整括号和类型匹配；每次出现保存字节位置及 assetId/variantId。外部 ID 带文档命名空间；目录 SHA 不符时只诊断，不套用外部身份。
- 保留目录资产版本；同名不同类型不合并。新修订建立独立版本，预览差异，保留旧稿编辑和媒体。重复导入同 SHA 幂等。
- 场级清单与片段实际引用分别保存。缺图、无效标签和歧义不静默绑定，可先保存文本；不合格引用阻止素材准备。
- 白图只保存为占位候选，自动选图跳过白图，素材编译拒绝占位。补充真实图后保持同一资产身份。
- SE 进入音频引用；OM 保留独立类型并按实际媒体编译，音视频须有已核验元数据。
- 原普通单元的 Stage1 来源守卫保持；新增导入来源证明另行验证。冻结任务、原回包、已有媒体和 unknown 记录不重建。
- 新动作同时注册 Rust/前端执行策略：`lingmo.preview/import/document/scene/original`。
- 图片导入失败不撤销成功的文本导入，界面显示缺图与失败原因。未改变数据库 schema。

## 本机实际验收

所有私人输入、图片、数据库、原生日志均留在被 Git 忽略的 `.local/lingmo-import-20261023/`。仓库只含合成 fixture 和无正文报告。

| 验收 | 结果 |
| --- | --- |
| 完整样本 | 150 集、371 场、883 片段、5407 镜、4736 处 token、518 资产 |
| 原文 | 5,441,567 字节；逐片段正文与原 UTF-8 范围完全一致 |
| 时长 | 首片段容器/实际 24/23.8 秒，末片段 4/2.7 秒 |
| 场次 | 第 2 集切换多场通过；150-1 仍一场、两种 SC |
| 既有图片 | 518/518 入库，全部标记白图占位，缺图 0、失败 0 |
| 恢复 | 同 SHA 幂等、保存重开、项目导出再导入，正文、层级、资产及原件一致 |
| 合成合同 | SE/OM、同名异类、标签编辑、修订身份、非法 token、来源伪造、占位图、原 unknown 任务保护通过 |
| Rust 专项 | `cargo test --lib lingmo`：22 passed、2 ignored；imported prepared proof：2 passed |
| 私人全量测试 | 显式运行 `full_private_import_acceptance`：两轮通过，未批量启用 ignored tests |
| 前端 | `npm --prefix apps/desktop run test:frontend`：267 passed；TypeScript/Vite 构建通过 |
| Windows 打包界面 | 隐藏真实 WebView 完整导入、首场标签/资产、第 2/150 集切换、重开通过；普通启动的设置、资产编辑、项目归档恢复等回归通过 |
| 生成边界 | 导入与浏览的生成请求、生成任务均为 0 |

Rust 全套曾为 1496 passed、7 failed、60 ignored；7 项原生上下文失败分别在真实 native runner 复测通过（windows_runtime 3/3、task_record_maintenance 1/1、SDK wrapper 5/5，包含原失败项）。这不是在同一上下文重新跑全套全绿的声明。随后新增专项另行验证。

隐藏 GUI 首轮验收脚本在项目加载前点击了禁用导航，导致超时；改为等待项目加载、导航启用后，首次/重开/发布版均通过。一次并行重编译遇到运行中测试 EXE 文件锁，待全量测试退出后重试通过，无需修改产品代码。

### 实测耗时

- 发布 release 客户端完整 MD 预览加文本导入：996 ms；当前场首次显示 159 ms，切到 150-1 为 116 ms。
- 同 SHA 重开验收（同一 release 字节）：预览加幂等导入 660 ms；首次显示 225 ms、切场 131 ms。
- Debug 全量附件验收：解析 701 ms；解析及文本持久化合计 2649 ms；518 张 PNG 解码、验证、入库 128733 ms。包括后续项目压缩导出/恢复的全测试耗时 232 秒。
- 以上分别是 release 文本/界面和 debug 媒体流程，不将约 1 秒文本导入表述为包含全部图片的速度承诺。

证据：`full-acceptance-v2.json`、`full-v2.log`、`lingmo-final.log`、`imported-proof-final-retry.log`、`frontend-final.log`、`native-v2/first.json`、`native-v2/reopen.json`、`standard-native/first.json`、`standard-native/reopen.json`、`published-native/first.json`。原生专项 receipts 在原 `.local/full-acceptance-20261020/client-final-20261020/native-runs/lingmo-native-*`。

## 发布与未测范围

使用既有 `apps/desktop/scripts/update-client.ps1 -PublishOnly -CandidatePath ...` 原位发布。桌面 `director-desktop.lnk` 仍指向 `apps/desktop/.local/client/director-desktop-next.exe`；只保留当前版和上一版备用，没有改动真实项目或登录状态。

- 首轮发布 EXE SHA-256：`e0b7c34e87eca744d80283788283db67c64039065badcebe13ef800537bde210`。
- 首轮发布时上一版 SHA-256：`94ecf1fa3d9bf5f76c45cee98583c19f8e062bab9f5ce633c54158bfad704f0b`。
- SDK/OpenCLI 资源沿用 SHA 隔离发布；本轮不重试账户授权或真实生成。
- 未做 macOS 原生验收、外部灵墨平台导入或真实视频提交。518 张占位图不代表取得全部 539 项资产设计/声音，也不代表真实成图质量通过。
- 父 CL 的 AOCI 现有索引陈旧且未管理此子仓库源码；本次未修改父索引，也不宣称父目录治理已对齐。

## 同日修复：定时刷新导致场次闪动

用户导入另一份 MD 后报告页面每隔几秒闪动。根因是 `LingmoPanel` 的场次读取 effect 依赖 `workspace.media` 数组对象；每次后台读取返回新数组（空数组也相同）都会清空场次和片段，再读取一遍。空闲读取间隔约 8 秒，有活动任务时更短。

已移除媒体数组对象作为场次重载条件。媒体展示仍随 workspace 更新，场次正文仅在项目、文档、场次、当前片段版本或显式重试变化时重新读取。同一场次的版本更新保留已有内容，实际切场仍清空旧范围，保留异步结果取消保护。

验证：前端 267 项通过，新增连续 workspace 快照、无关媒体变化不重读，以及片段版本变化仍刷新且不清空内容的断言。真实隐藏 WebView 在完整 150 集样本上连续点击实际刷新按钮三轮（走与轮询相同的 workspace 更新路径），资产区与首片段 DOM 身份保持、展开状态保持、区域移除次数为 0；候选及发布位置各通过一次。该测试没有冒称隐藏空闲窗口执行了定时轮询。测试探针最初尝试拦截原生调用计数但未生效，已改为验证真实刷新完成提示和 MutationObserver，不将该次失败计为通过。

证据在 `.local/lingmo-import-20261023/flicker-frontend.log`、`flicker-native-v2/first.json` 和 `flicker-published/first.json`。无生成提交。用户关闭程序后已按原路径发布：当前 EXE SHA-256 为 `b0920affc0e24aea1d04e268efb40ff58be4a887311e96b1359a6f896ff89df2`，上一版为 `e0b7c34e87eca744d80283788283db67c64039065badcebe13ef800537bde210`，发布目录只保留这两份 EXE。
