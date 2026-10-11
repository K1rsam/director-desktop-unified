# Chrome 后台图片链路 Windows 验收（2026-10-11）

本轮在 `integration/windows-mac-20261008` 上延续 `afffbc0`，保留 Windows/Mac 集成、CLI/SDK、API 和直接父资产调度。仅测试图片；没有提交视频，没有用付费渠道替代。

## 修复范围

- 浏览器身份由扩展 background 生成并保存在当前 Chrome profile 的 local storage，网页不能覆盖。全局绑定 endpoint、profileId、bundleId，普通页面数量变化不改变执行浏览器；重复 profile/runtime 身份及隐身配置禁止新生成。
- 网页渠道使用一个持久生成队列，busy 表示等待。每个任务后台独立会话，不激活页面，不依赖用户正在查看的 ChatGPT 会话。CLI/API 并行与直接父图依赖保留。
- 原生附件校验兼容实际网页两种附件结构，核对文件名、已加载图片及上传状态；不得在父图未上传时只发送文字。
- 已关联的原会话查询使用原关联码与提示词哈希，首页上传及尚无 assistant 的合法阶段保持等待，不反复消耗恢复预算。未知提交、原请求、回包和媒体继续保留。
- 原图完整入库及采用后，独立持久清理队列只回收身份、会话、闲置和隐藏状态均已核实的本程序页面。首次未看到原页不等于证明已经关闭；历史缺失归属页保留。
- 回收最后一个连接页时，先在同 profile 后台建立最多一个 ChatGPT 首页控制连接页；它不上传、不发送提示词、不生成。确认真实 Chrome tab 的 `active=false`、`autoDiscardable=false`、`discarded=false` 后才回收完成页。创建未知状态持久保留，不盲目重复开页。
- 扩展维护只刷新本程序页面。重载前保全所有已登记的本程序 launch 身份，session 读取失败在正式维护前停止。
- 没有 task/prepared/receipt 或任何历史提交证据的旧 Chrome 队列，可在渠道身份严格一致时固定一次全局绑定；独立 metadata 不改原 snapshot/config，后续全局切换不会迁移它。父图补齐 references 不损坏该固定身份。

## 已完成真实图片验证

合成测试项目包含机器人基础、独立灯笼、蓝雨衣、湿雨衣四资产，父链为基础 → 蓝雨衣 → 湿雨衣。

| 样本 | 实际结果 |
|---|---|
| 机器人基础 | 历史原任务原图保留，当前仍采用 |
| 蓝雨衣 | 查询原任务恢复并入库，没有重新提交；引用基础图 |
| 独立灯笼 | 新后台任务约 63 秒完成原图入库，无参考图 |
| 湿雨衣 | 同一批持久排队，约 139 秒含等待完成；仅引用蓝雨衣直接父图 |

四张均为 1672×941 原图。逐图人工视觉检查、文件 SHA256、native proof、原任务 ID、采用与父图引用交叉核对通过。独立灯笼、蓝雨衣及湿雨衣页面已确认自动回收，清理记录为 `done/closeRequested=true`。基础原页在较早维护中失去归属，记录为 `retained/page_absence_unproven`，不能伪补身份或冒称回收。

本次新两项从客户端“生成所选图片”推进，原无远端提交的失败记录保留。已经成功的基础、蓝雨衣没有重复生成。监测到实际工作页始终 `hidden/focused=false`，客户端仍可操作设置；灯笼生成时湿雨衣无 task/prepared，前项完成后才提交湿雨衣。

桥接历史蓝雨衣 timeout 发生在恢复前。原始桥回包保持失败记录，本地采用的是随后查询取得并通过完整原生证明的原图；没有把桥的旧状态改写为成功。

## 工程与发布验收

最终源码来自本轮集成分支工作区；构建前后未更改产品源码。私有日志、数据库、原图、凭据和网页记录均位于忽略的 `.local/validation/chrome-background`，不上传仓库。

| 验收 | 命令或实际操作 | 结果 |
|---|---|---|
| Windows 原生核心全库 | 未打包 native runner 执行 `cargo test --locked --release --lib -- --test-threads=1` | 1087 通过、42 忽略、0 失败；独立 receipt `success=true`、`timedOut=false`、子树空验证通过 |
| 扩展合成回归 | `node --test apps/chrome-bridge/tests/*.test.cjs` | 38/38 通过 |
| 前端合同回归 | 桌面目录 `npm run test:frontend` | 247/247 通过 |
| 前端构建 | `npm run build`，含 `tsc -b` | 通过；保留体积 warning |
| Windows 最终候选 | `update-client.ps1 -BuildOnly` | release 构建通过；保留已有 unused warning |
| 隔离启动探针 | 最终候选 `--verify-startup <report>` | 独立报告 `ok=true`；embedded frontend、IPC、SQLite、媒体、归档恢复通过；真实生成请求为 0 |
| 末页自动回收 | 候选客户端打开真实测试项目 | 后台创建一个控制首页，湿雨衣完成会话消失，持久清理状态 `done` |
| 全局绑定及普通页增减 | 同 profile 新增、移除一个本轮专用普通首页，GUI 分别检查连接与模型 | 2 页及 1 页均显示已连接，profile/bundle 不变；未选择或操作用户页 |
| 控制页复用与已知关闭恢复 | 同固定 token 重复 open；有本轮普通页作连接源时关闭已知控制页，证实消失后重建，再复用 | 复用不增加页；已知关闭后的新控制页隐藏、无焦点、禁自动 discard，身份一致 |
| 保存重开 | 关闭旧客户端，候选打开项目和资产；随后关闭候选、同名发布，再重开当前版 | 5 个项目和四张已采用图保留；不重发已有图片任务 |
| CLI/API 调度回归 | 全库中的 `image_scheduler_tests` 与 Chrome queue tests | 并行槽、直接父图、11 层链、无效分支隔离、跨项目网页单生成及未知任务保护通过；本轮未真实调用付费 API |

测试脚本第一次在双页状态误把上游 generic `health.ok=false/needsSelection=true` 当作连接失败；客户端按全局 profile 检查实际显示 ready。修正只读验收脚本后继续。关闭页接口返回的是 `closing`，不是已消失；本轮依据后续 clients 清单证明消失后才恢复控制页，没有因脚本即时检查失败而重复创建。

保留审计：原四个项目的任务及媒体记录完全不变。验收项目原 taskId、prepared、receipt 和已有媒体 SHA 全部保留；唯一旧 snapshot 变化是尚无 task/prepared/receipt 的湿雨衣请求补入已采用直接父图 references。将该 references 恢复为空后，完整 snapshot SHA 与测试前记录一致。项目总数保持 5，任务从 223 到 225（新增两项图片），媒体从 2 到 5（恢复一张、新增两张），视频无新增。

当前版与唯一备用版位于 `apps/desktop/.local/client`，使用同名发布脚本轮换，候选临时 EXE 已回收：

- 当前 `director-desktop-next.exe` SHA256：`88e18097ada7346e087da81f2bcc45718ab78656fbbbcf46be10cb36ba7332af`。
- 备用 `director-desktop-next-previous.exe` SHA256：`bdcf4b6f7a7250608e9fcd543408e783f2e200365cd2106135f7af6b160f4234`。
- 已安装的 14 份扩展覆盖件均匹配 manifest 指纹，重复部署 `changed=0/stateUntouched=true`。原扩展文件保留版本校验备份，未修改桥接凭据和原任务状态。

首轮全库直接从打包的 Codex 进程执行，1075 通过、42 忽略，2 项原生路径测试要求未打包进程而失败。使用已有 Windows native runner 复核这 3 项后全通过；这是环境边界，未为消除失败修改产品路径代码。最终适用的全库验收使用同一 native runner。

后台生成成功不等于电脑休眠、退出 Chrome、网络中断或网页额度限制全部实测通过。工作页及控制页的真实 tabState 已确认禁自动 discard，隐藏页图片生产成功；未强制休眠电脑或重启用户 Chrome。扩展维护后已有控制页可按真实身份复用；跨 browser-session 变化且缺乏原控制页归属证据时保持 hold，不能宣称任意冷启动均可恢复。

内存只采样整族 Chrome 进程：两次均 27 个进程，working set 约 4.27 GB 与 4.07 GB，包含用户页面及 Chrome 自身活动，不能据此声称每页内存或节省比例。确定的改进是正常情况最多一个控制页和一个生成工作页，已完成会话回收；未知和待下载任务例外保留。

本轮仅验收 Chrome 图片链路和相关工程回归，不把它写成十集 SDK 制作、全产品或双平台最终验收。网页完成时间仍取决于上游；不承诺固定秒数。
