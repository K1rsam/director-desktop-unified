# Chrome 最后连接页恢复补丁（2026-10-11）

基于 `3e79d90a3791b961ca9d7cd2dfea42a81e2d0370`，范围为桥组件的控制页恢复。没有提交任何图片或视频，没有修改既有项目、请求、回包或采用结果。主控部署后，已经实测从旧隐藏工作页恢复唯一后台控制连接并回收最后工作页；不将其扩大为任意 Chrome 冷启动或全产品 GUI 通过。

## 根因与采用方案

受控 `runtime.reload` 会清空 session storage 并使旧 content script 失效。原维护代码只恢复各页 launch identity，没有恢复控制页使用的 browser-session 身份；原控制页复用代码又只检查 Chrome tab 是否存在。因此原用户控制页即使仍打开，也无法提供有效连接，而最后一个已完成图片工作页必须保留，反复得到 `last_connected_page`。

不把旧 nonce 写回当前 browser session。维护前仅为已确证创建完成（`opened`）、身份与真实根页面一致、无 lease 和未确认命令的控制页冻结证据。恢复时将其标为本次受控 reload 后失效；普通断连、无证据的冷启动、未知创建和损坏身份没有替换资格。

下一次控制页 open 若发现旧页已经重新建立完整连接，直接复用，绑定当前真实 session；否则在 durable state 归档其原身份与旧 nonce，验证归档后只退休其控制角色和 session launch record，保留原页、URL、焦点。随后沿用原创建预约和未知结果保护，新建一个隐藏、不可自动休眠的专用根页。不刷新、导航或关闭用户正在看的旧页，不增加扩展权限，不改变生成任务 lease。

原创建 `reserved` 状态一律保持，不因已有 tab 而擅自升级。退休写入或 ownership 移除未确认时停止；之后不会重复创建。新控制页创建或导航未确认也保留预约，跨 worker 重启不重发。

## 验证

- `node --test apps/chrome-bridge/tests/*.test.cjs`：51 项通过，0 失败，包括新增 7 项控制页/reload 回归。
- 新增回归覆盖旧用户页保持、仅一个后台替代页、当前 nonce 不被旧 nonce 覆盖、旧创建归档、已恢复连接的复用、创建/导航/退休未知、错配失效证明、旧页被用户改为其他会话、活跃 lease/未确认命令阻断，以及 source-only maintenance 不刷新控制页。
- manifest 固定最新覆盖件 SHA256，同时保留已安装的前一指纹，部署不触及 bridge state 或凭据；部署器的幂等与未知代码拒绝回归通过。
- `2026-10-11T03:35:08.097Z` 只读现场快照：pendingRequests=0、pendingCommands=0；唯一已就绪连接是隐藏红伞工作页，activeRequest=null。三图仍 downloaded、媒体哈希不变；红伞 cleanup 仍为 pending/last_connected_page，累计 32 次。这是补丁前现场证据，不是补丁成功证明。
- 私有快照放在忽略的 `.local/validation/chrome-background/restart-controller-predeploy-readonly.json`，不上传原回包、媒体、私有会话内容或凭据。

## 部署后实际恢复

主控部署覆盖件后，首次 source-only runtime maintenance 在 `03:37:20Z` 完成；这次冻结仍由内存中的旧 coordinator 执行，pending 只有旧格式 launch records，没有控制页失效证明。虽然重启后加载了新源码，后续控制页创建仍报 `Controller belongs to a previous browser session`。这是旧内存冻结包与新恢复器的交接问题，不是新隐藏页创建失败。

在再次检查 pendingRequests=0、pendingCommands=0、原隐藏工作页 activeRequest=null 后，第二次 source-only runtime maintenance 使用已加载的新 coordinator 于 `03:46:03Z` 完成。两次原维护回执分别保存在私有证据中，未覆盖前一次证据；未刷新、导航或关闭原用户控制页。

`03:46:45.682Z` 的真实 `browser.tab.opened` 回执给出新控制页 `tabState={active:false,autoDiscardable:false,discarded:false}`；随后新控制页以同 profile、同 epoch 的 `https://chatgpt.com/` 隐藏连接上线。`03:46:46.224Z` 收到原红伞工作页的精确 `browser.tab.closing` 回执，服务日志确认其断连。最终 `03:47:41.746Z` 只读快照：唯一控制连接 ready、hidden、focused=false、activeRequest=null，pendingRequests/pendingCommands 均为零，三项 cleanup 全部 `done/already_closed`。红伞原任务未重发，原 bridge 历史仍为 `interrupted`。

`controller-acceptance-proof.json` 对比本轮修改前后，三项 jobs 的完整记录投影和三项 media 记录完全一致；包含原 taskId、snapshot/prepared/receipt/result 哈希、引用、历史终态和媒体哈希。角色归档与新 nonce 约束由实际通过的新控制页创建分支及合成回归证明；本轮没有独立读取 Chrome extension storage，也没有最终 GUI 重新读图，避免将这些范围写成单独实测。

私有续测证据：`restart-refresh-runtime-first-deployed-coordinator.json`、`restart-refresh-runtime-second-deployed-coordinator.json`、`restart-controller-final.json`、`controller-acceptance-proof.json` 和过滤后的 command diagnostics。没有新增生成请求。

## 后续维护注意

先重新确认全桥 pending、commands 与 active leases 为零，审查覆盖件并部署。只通过原隐藏工作页执行 source-only runtime maintenance；原用户控制页不得刷新、导航或关闭。首次从旧实现升级时，首次维护的冻结包仍由旧内存源码生成；不能用它假定新证明已经冻结。应先核对真实 command.error 与回收状态，确证尚未获得新证明时再进行一次受门禁保护的窄维护。任意真实 Chrome 冷启动及整个产品 GUI 验收仍需独立证明。
