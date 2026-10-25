# 四阶段迁移实施证据

工作分支：`feat/series672-assets112-workflow-20261025`。原基线 `f5a347f3994d1ea3148f514a260c8019e0ab2318`，任务书保护提交 `67ae3ac`。保留原工程、项目、媒体和冻结请求；未将本轮未验收客户端覆盖安装版。

## 第一步：规则来源与兼容边界

完整冻结 series `20261025-v6.7.2-readable-image-bindings` 128 文件、资产 `V1.1.2` 41 文件。series 是已发布 v6.7.0 加本机增补的本次完整快照，不声称与旧发布提交逐字一致。运行注册和完整归档分别核对，归档存在不等于全部正文都会发送给模型。

2026-10-25 执行 `python scripts/freeze-workflow-rules.py --verify`，两包全量通过：

| 包 | 完整 manifest SHA-256 |
|---|---|
| series | `0ff5dcdd7e963497e06c39e806650ebb0dc1face2e661af9f6d29db6ea9f2653` |
| assets | `02dd6f668eae9b374ce69c7b251e7524f955515c1cb2bcfcceb98e3d75bdfa4d` |

新工作流标识 `desktop-series672-assets112/v1`。稳定页面 ID 仍为 `foundation/storyboard/images/videos`，历史任务号、预算、原回包、规则快照和 UNKNOWN 不批量改写。新任务与原请求恢复使用分开的构造入口。

## 验收状态

工程实现与新增回归正在进行。真实 SDK/CLI、一集到十集读稿、免费图片的实际审图、最终 Windows 安装版和完整外部接续包尚未完成本轮验收。不得将历史测试数或单个解析 PASS 当作新生产完成。视频提交始终禁止。

私有测试日志、真实输入输出及账户材料保存在 `.local/series672-assets112-20261025/`，不随源码提交；最终报告仅记录无正文的命令、哈希、计数、结果和未测范围。
