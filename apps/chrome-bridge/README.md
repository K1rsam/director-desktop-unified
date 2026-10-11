# Chrome 原图恢复补丁

适配已安装的 `chatgpt-browser-bridge-node` **6.4.0**。补丁用于恢复已提交的原图片任务，不重新发送提示词，不创建工作流，不改桥接持久回执、账户或数据库。

覆盖文件按 manifest 校验原始字节；`.gitattributes` 对这些文件关闭行尾转换，保留上游空白。不要格式化覆盖文件或仅复制 Git 自动转换后的文本。

部署：`node apps/chrome-bridge/scripts/apply-overrides.cjs [组件绝对目录]`。省略目录时使用当前 Windows 安装位置。脚本先验证全部目标的版本和字节哈希，拒绝未知修改，保存原文件到组件的 `.director-native-recovery-backup/`，重复执行为无操作。已经加载的 Chrome 扩展需重新加载内容运行时；正在执行的其他任务不得被打断。不要把包版本升级视为此补丁仍然适用。

原生恢复要求：原请求 UUID、原会话、完整提示词哈希、原标签页身份、原 assistant 分支、完成状态、原文件 ID、下载字节哈希及分支复读一致。扩展重载后只能恢复原 sourceClientId 已验证且没有其他任务占用的标签页；页面还必须包含原任务标记。

Rust provider 兼容 Protocol 5 独立恢复回包的空外层 requestId 与 artifact 的 `recovered` 标记，仅在完整 native proof 验证后建立本地关联。不同任务 ID 必须拒绝；原桥接回包完整保留。

验证：`node --test apps/chrome-bridge/tests/native-recovery.test.cjs`。这些合成测试不代表新任务真实生成通过。当前真实原任务的恢复、下载与资产采用证据见对应 handoff 文档。

桌面 Chrome 渠道使用全局 Chrome profile 绑定、后台专属工作页和跨项目持久单图队列。普通页面增减不改变执行浏览器；网页忙碌保持排队，未知提交仅恢复原任务。CLI/API 继续并行并等待直接父图入库。

完成图片会话在原图入库后安全回收。最后连接页需要先核实一个同 profile 后台首页控制页；控制页不提交创作请求，每个 profile 保留一页，未知创建不会重复打开。仅本程序工作页及控制页禁止普通 Memory Saver 自动丢弃，用户页面不改动。完整 Chrome 重启、电脑休眠及历史缺失归属页面的处理边界单独保留。

最初实现范围见 [后台队列记录](../../docs/handoff/CHROME_BACKGROUND_QUEUE_20261010.md)，本轮实际部署、四图生成/恢复、父链、页面回收及 Windows 发布证据见 [后台链路验收](../../docs/handoff/CHROME_BACKGROUND_ACCEPTANCE_20261011.md)。扩展维护前必须确认零活跃任务，只刷新本程序页面，并保全 launch 身份；不能用全浏览器刷新替代。
