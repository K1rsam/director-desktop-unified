# Chrome 生成中重开与原任务恢复续测（2026-10-11）

从 `integration/windows-mac-20261008` 的 `936b766f0de1ace026c3fa06f89552042594ce7d` 继续。范围为 Chrome 后台图片队列，不提交视频，不用付费图片替代，不把本次结果写成十集 SDK 或全部产品验收。

## 实际测试与发现

在新的合成项目一次批量排入基础绿背包、独立红伞、湿背包三资产。第一图生成中从 GUI 正常关闭工作台，随后启动同名当前版。原 taskId、已准备请求、回执及 snapshot 哈希未改变；未选项目时后台调度已恢复。第二图排队、衍生图等待直接父图正确。专用后台页的刷新重连已另行验证，没有刷新用户控制页。

续测发现并修复三个根因：

1. 原生图片已可下载，但通用 DOM 观察器未定位最终身份，桥接仍占用队列，最初样本额外等到 `CANONICAL_DEADLINE_EXCEEDED` 才释放。加入严格的 native completion 入口：原 pending request、提示词、source、launch、lease/owner/epoch、会话与消息、最终输出节点、文件 ID、字节和记录复读证明全部吻合，才走已有 canonical terminal/release。不直接清 lease，不覆盖历史终态。
2. 红伞当前分支有两张历史渲染指针，旧校验把它们当成最终两张交付。现仅取最新已完成的图片输出消息；DOM 唯一文件、原始字节及认证分支复读仍须一致。最新输出多图、不完整或错分支拒绝，不选择较早的匹配图。
3. 只读诊断可缓存 blob 原图字节，但诊断不授予后台采用资格；旧路径看到缓存就跳过真实验证，导致后台 snapshot 无图片。真实恢复现在对这种缓存重新认证和复读，不由诊断升级采用。

## 三图结果

| 样本 | 实际结果 |
|---|---|
| 基础绿背包 | 原图入库及采用；正面/侧面/背面并列，颜色、米白拉链和肩带符合测试输入 |
| 独立红伞 | 在原会话恢复原任务，零重发；正面、侧面与收拢形象齐全，已入库及采用 |
| 湿背包 | 原持久队列自动推进，只引用基础背包这张直接父图；保留结构和三视图，新增湿润布面和水珠，已入库及采用 |

三张均为 1672×941；逐张视觉审核、原文件 SHA256、native proof、任务 ID 和三项 adoption 对照通过。合成提示词为手工输入，不能据此证明剧本资产 Skill 的质量。

红伞网页完成已经由认证 native record 的 final assistant 状态证明。为维护旧桥接，在工作台正常关闭后，只对这条原 turn 中断卡住的桥等待，再于零 pending/command/lease 时部署补丁及重启原服务。原网页、冻结请求及本地任务保留；其桥历史 `interrupted` 不改为成功。随后受限原生验证器调用产品 `tasks::resume_job` 的查询/下载路径，禁止创建任务，成功入库。没有将 diagnostic 当作成功采用。

湿背包由持续运行的当前客户端调度，桥任务从 `2026-10-11T03:02:12.700Z` 到 `03:03:36.624Z`，约 83.924 秒，正常 `completed/error=null`；实测 finishReason 为 `stable_normalized_observation`。原生证明绑定后正常观察路径完成，没有再等 watchdog 失败。新增 server native fallback 的严格边界经合成回归验证，本样本不声称强制走过 fallback 分支。

基础与湿背包工作页确认已回收。红伞页当前作为最后可用连接保留，清理为 `pending/retained/last_connected_page`：扩展运行时维护未刷新用户正在查看的原控制页，其旧连接失效，不能冒称该页已回收。新图片仍可通过同 profile 的保留连接后台开独立工作页；保留页不发送新提示词。任意 Chrome 冷启动及控制页断连后的最终回收不列为全部通过。

## 验证与部署

- `node --test apps/chrome-bridge/tests/*.test.cjs`：44 通过，0 失败。覆盖最新图与中间图、用户参考图、最新多图、错分支、诊断缓存、原任务身份、跨 lease/owner/epoch、错误字节证明和畸形回包。
- 原生恢复验证器使用工程现有 `Cargo.lock`、当前 Rust 源码和产品查询/下载/采用逻辑；只绑定新测试项目的一条暂停 Chrome 图片任务，不启动全局执行器、不提交新图或视频。私有 receipt 显示 `downloaded/newSubmissions=0/videoSubmissions=0`。
- 原生回收前后证明：原 taskId、prepared、receipt 和已提交 snapshot 保持；未提交衍生请求在父图入库后补齐 reference 是预期步骤，不将该 snapshot 哈希变化报为不变。
- 原有五项目的 225 条任务及五份媒体保留记录投影完全一致；只新增本轮一项目三项 image job，没有新增 video job。
- 15 份覆盖件的版本及原始/历史/当前指纹校验通过；最终重复部署 `changed=0/stateUntouched=true`。服务只在无 pending、commands 及 active lease 时重启；扩展维护仅刷新原隐藏红伞页。
- `git diff --check` 通过。未修改 Rust/前端产品源码，未重复全库/前端构建，前轮 1087/247 项结果仍仅作为对应版本历史证据。
- 当前产品 EXE 未替换，保持一个当前版与一个备用版；本轮仅更新已安装桥组件。

本轮已通过的 GUI 部分为批量入队、第一图生成中正常退出与重开。最终回查时 computer-use 连续报 `foreground window did not report a process id`，重定位后仍失败，已停止 UI 输入；浏览器工具也报 `nodeRepl.fetch request failed`。最终三图状态及后续调度由真实 native 运行、只读 SQLite/桥回执和本地原图交叉验证，最终 GUI 回查未完成。

私有证据位于忽略的 `.local/validation/chrome-background`：`restart-watch.jsonl`、`restart-native-recovery.json`、`restart-final-proof.json`、`restart-refresh-*.json`、原 turn 与保留记录。数据库、凭据、签名 URL、原生网页回包和图片不上传 GitHub。

当前版 SHA256：`88e18097ada7346e087da81f2bcc45718ab78656fbbbcf46be10cb36ba7332af`；唯一备用版：`bdcf4b6f7a7250608e9fcd543408e783f2e200365cd2106135f7af6b160f4234`。
