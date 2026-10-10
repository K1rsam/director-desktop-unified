# Chrome 原图恢复补丁

适配已安装的 `chatgpt-browser-bridge-node` **6.4.0**。补丁用于恢复已提交的原图片任务，不重新发送提示词，不创建工作流，不改桥接持久回执、账户或数据库。

覆盖文件按 manifest 校验原始字节；`.gitattributes` 对这些文件关闭行尾转换，保留上游空白。不要格式化覆盖文件或仅复制 Git 自动转换后的文本。

部署：`node apps/chrome-bridge/scripts/apply-overrides.cjs [组件绝对目录]`。省略目录时使用当前 Windows 安装位置。脚本先验证全部目标的版本和字节哈希，拒绝未知修改，保存原文件到组件的 `.director-native-recovery-backup/`，重复执行为无操作。已经加载的 Chrome 扩展需重新加载内容运行时；正在执行的其他任务不得被打断。不要把包版本升级视为此补丁仍然适用。

原生恢复要求：原请求 UUID、原会话、完整提示词哈希、原标签页身份、原 assistant 分支、完成状态、原文件 ID、下载字节哈希及分支复读一致。扩展重载后只能恢复原 sourceClientId 已验证且没有其他任务占用的标签页；页面还必须包含原任务标记。

Rust provider 兼容 Protocol 5 独立恢复回包的空外层 requestId 与 artifact 的 `recovered` 标记，仅在完整 native proof 验证后建立本地关联。不同任务 ID 必须拒绝；原桥接回包完整保留。

验证：`node --test apps/chrome-bridge/tests/native-recovery.test.cjs`。这些合成测试不代表新任务真实生成通过。当前真实原任务的恢复、下载与资产采用证据见对应 handoff 文档。
