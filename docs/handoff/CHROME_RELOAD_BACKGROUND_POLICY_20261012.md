# Chrome 扩展后台重载的副作用与重连边界

## 已确认的问题

此前 `BrowserTabCoordinator.reloadExtension` 在原重载命令已经受理、等待原页重连失败之后，只检查原页面是否为 owned tab，就调用操作系统浏览器并创建替代页。`ownedTabsOnly=true` 与 `allowMaintenancePageBootstrap=false` 没有限制该最终分支，导致后台操作意外触发系统浏览器。

本轮把 owned-only 模式的 maintenance bootstrap 全部禁用；已经受理但未重连时，owned-only 或没有显式允许 bootstrap 的请求返回 `EXTENSION_RELOAD_RECONNECT_PENDING`，保留 `reloadAccepted=true`、原 accepted 回执和未使用替代恢复的事实。不创建替代身份，不打开系统浏览器，不关闭旧页。HTTP 错误响应保留 `reloadAccepted=true` 与 `originalOutcomePreserved=true`，防止把等待重连误解为原操作没有执行。

仅一般兼容模式显式 `allowMaintenancePageBootstrap=true` 时，才保留既有 bootstrap 行为；本桌面后台流程不使用它。

## 现场只读证据的含义

原现场 HTTP500 来自重载受理之后的系统替代页握手超时；服务端所有 client 已断开，不能据此认定重载没有执行。

只读读取扩展 local-storage LevelDB 日志时，没有打开 LevelDB 连接或修改原文件；验证稳定文件 size/mtime、全部物理记录 CRC32C、片段顺序与 WriteBatch 结构后，仅提取目标 pending-reload 和 maintenance 状态的脱敏字段。本次 pending 的两个原工作页确实已记录，启动恢复在约401ms内执行导航、标记 maintenance succeeded，随后删除 pending。

源码中的这个 succeeded 仅证明 tabs 更新操作已返回：`extensionReloadCoordinator` 在导航后立即删除 pending 并标记维护成功，没有等待原 source client、版本、bundle、runtime 的 `client.ready` 证据。现场仍为0 clients，因此“恢复导航已执行”和“桥接已经重连”必须分开报告。没有再次发起真实重载、导航或生成。

未完成的恢复改进是：保留原 pending intent，直到原来源及运行身份的 hello 确认；仅在原 owned 页范围内继续恢复，不建立替代页或把未知副作用当成可重发。该项不能用本次源码回归标成真实已通过。

## 离线验证与部署范围

新增4项回归覆盖 owned-only 的两个 bootstrap 参数分支、明确关闭 bootstrap 的 accepted 重载、原命令拒绝和原页成功重连；测试的系统打开、前台 bootstrap、替代身份分配均设为直接失败，以证明这些动作未执行。

`node --test apps/chrome-bridge/tests/*.test.cjs`：66 项通过，0 失败、取消、忽略。

本轮没有部署真实 component 或操作浏览器。目标 SHA256：

- `src/bridge/coordinator/browserTabCoordinator.js`：`67c52316753686dc0daad0914ec6b44ddf0d7b389b4e5b10a0fa00781096716d`
- `src/routes.js`：`aeeb0d11c00ddab8f37537faeb904dce01e8507398043db9054852cbc1b97d94`

routes 还包括本轮已验过的只读 owned-launch inventory 入口，部署时须与其 manifest 和配套来源一起核对。历史原回包、failed 任务和已入库图片保持。
