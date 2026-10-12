# Chrome 原任务准备恢复（2026-10-12）

## 问题与本轮边界

本轮29项图片批次已有5张原图入库。后续7项在开工作页／握手准备阶段失败，未保存 prepared 或 remote task ID；其中存在开页已发生、握手未完成的情况。不能把这些任务重置成新任务，也不能以 connected clients 列表没有原页推断 Chrome 原页不存在。

用户要求继续使用电脑前台，因此恢复通过产品后台入口与扩展专属工作页执行。图片限定已确认无按张费用的个人套餐；视频从未授权，不提交。原批次、源文、资产身份、原错误和历史结果保留。

## 采用的恢复方式

- 第一次 prepare 在外部开页前保存原请求、配置、snapshot 摘要和实际 runtime 意图。后续恢复核对这些绑定，已有 intent 不再 open。
- 旧版本没有 intent 的失败任务，必须证明完整 queued → preparing → failed 历史，且没有 prepared、receipt、result、submit/query/download/reconcile 痕迹。不存在旧 nonce 时不伪造，首次接管冻结本次 runtime。
- 原 remote turn 精确404及 canonical state/history/deadline 均不存在只是未提交核对的一部分，不单独证明可以重发。Chrome 原页清单须通过实际 tabs/session 双采样及 runtime/hash 校验。
- 清单 matched 时仅接回原页。仅旧无 intent 且完整证明为 confirmed_absent 时允许第一次 open；该 absence 只描述当前清单，不证明历史 open 从未发生。有 intent 后 timeout/busy 不借此再次开页。
- 恢复预算最多3次、180秒。通道 busy 作为等待，扣回本次尝试但不延长期限；期限耗尽保留任务。
- 同批 blocked 子任务只有明确源于原父任务准备失败、身份和依赖不变时才能回到 waiting_dependency，仍等直接父图成功下载并采用。其它未知失败不放行。
- 服务器的安全 detail/code/phase 进入诊断，保留原错误事件；提交仍重新检查原 runtime 和未知任务门禁。
- 最后一张下载后有最长90秒页面清理收尾，遵守原15/60秒重试；报告分别区分完成、保留用户查看页和清理待确认。重开仅清同批原任务，不再生成。

## 当前验证，不等于真实恢复验收

定向合成测试：后台29项、Chrome provider29项、wire5项、共享任务14项，共77项通过。native inventory/schema专项14项与这些测试重叠，不再累加。

2026-10-12完整 unpackaged Windows 核心回归第一次结果为1160通过、2失败、44忽略，167.68秒。两项失败均来自新人物对照策略对旧工作流缺少 profile 字段的兼容要求，已修复（`caea747`）；保留失败证据，不将首轮写成通过。一次复跑误用了修复前 release 测试文件，另出现一项在并行编译期间的计时断言失败，这些日志同样保留，不作为新源码的回归结论。

主控重新执行 `cargo test --locked --release --lib --no-run`，核对当前1208项测试清单及 EXE SHA256 `ac8514f93f8592862ea362e6da0ec46e685cb6d526a3d65d157ac9bc8e10038c` 后，通过原生启动器运行 `--test-threads=4`：1164通过、0失败、44忽略，141.81秒，原生 receipt 确认退出码0及进程树已空。忽略项没有批量运行；全部为离线测试，不证明真实生成链路通过。

最终候选 EXE SHA256 `5b54c2734d0cbb9665edf7c261c6f7edbdefac30b76e6de416c84da665300d03`。实际运行该 EXE 的原批次 stop-before-queue 入口，29项原 job 的观察证据逐值不变，5图保留、原文精确一致、视频任务0；原请求与停止标记未改，运行报告的前一版留在私有证据目录。

已用现有 `update-client.ps1 -PublishOnly` 发布同名 `apps/desktop/.local/client/director-desktop-next.exe`，核对 SHA256 与上文候选相同。唯一备用 `director-desktop-next-previous.exe` SHA256 为 `83f4aff0c2109a40025bcbb0b4614962f075a0b8e2fbb5a5ea2ed928c631db9f`；候选文件已由发布脚本核对后清理。实现提交为 `69fe4ae`，同一集成分支已推送到 `K1rsam/director-desktop-unified`。未启动 GUI。

本轮准备恢复和清单源码尚未执行真实 first-open 验收，已入库5图不能代表全29项完成。现场原请求停止标记仍保留，浏览器扩展暂时0连接；须先完成原页连接恢复再启动原批次。当前阻塞和已部署的扩展修复见 `CHROME_RELOAD_SOURCE_READY_20261012.md`。私有请求、数据库、剧本、回包和媒体不提交仓库。

## 续接顺序

### 本轮准备预算隔离补丁

准备恢复达到原 3 次／180 秒预算时，原执行器直接传播错误，导致整个批次停止；另一个有完整未提交证明但预算已过期的任务仍被算作 runnable，会造成无效等待。现改为完整核对原零提交证据、冻结 snapshot 与预算后，原子封存这一项；原 ID、snapshot、错误及预算均保留，独立健康分支继续。封存事件明确 `generationCalled=false`、`budgetResetAllowed=false`；未知提交、身份漂移和证据矛盾仍保持原门禁，不能当成预算耗尽消化。

Windows debug 定向 32 项后台及 14 项共享任务回归通过。随后重新编译 release 核心并以 Cargo JSON 固定唯一测试产物、清单 1211 项及 SHA256 `fe63eea24d9e7871a9892304fec1fab0dd1af04b5634865d00b39a93f90f3b60`。原生 job runner 实际执行 `--test-threads=4`：1167 通过、0 失败、44 忽略，144.52 秒；receipt 确认 unpackaged、共享写探针和进程树已空。没有运行真实模型或 ignored 测试。

2026-10-12 05:06:48 UTC 对原项目稳定 SQLite/WAL 副本进行只读核验：仍为 5 downloaded、8 failed、2 blocked、13 queued、1 waiting_dependency；`chrome_prepare_recovery` 预算元数据为空。因此现有八项准备失败尚未消耗此恢复预算，不应仅根据旧 GUI 的 prepare_error 事件数量判断预算耗尽。原文精确一致、视频任务 0；没有重新创建批次。

1. 通过可用的后台浏览器控制连接唤醒原工作页／实际重载扩展，核对真实 profile、bundle、原 source launch 和 hello ACK。当前 CUA Chrome 只读清单也返回连接失败，不能用磁盘 overlay 当成已唤醒；用户前台页面及 Chrome 配置不改。
2. 验证新 worker 与 owned-launch inventory 实际运行。只有这些前置条件满足，才移除本次 controller 停止标记并以原请求文件启动同批后台入口；不创建新的29项批次。
3. 恢复7项未提交准备失败和2项原依赖 blocked，续跑其余 waiting/queued。已有5图不重发。完成全29原图入库后逐图审核，核对人物视图、身份和父图承接以及最后工作页清理。
4. 再接前十集 Stage3 与 Stage4 非视频验收，旧 B/C 未知文本任务继续保护。当前测试项目已完成 Stage1/Stage2文字设计，尚无采用单元；不能写成十集分镜或第四阶段已经通过。视频始终禁止。
