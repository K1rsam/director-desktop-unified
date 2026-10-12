# 第二阶段同批人物设计对照

新工作流策略：`stage2ComparisonProfile=asset-accepted-comparisons/v1`。

此前启动整批时冻结 `comparisonDesignsByAssetId`，新项目所有提示词尚为空，后面完成的设计也不会进入后续请求。新策略将人物放入稳定的依赖顺序通道，首次调度时从已有有效设计和本批已接受设计中选择对照；非人物仍使用原并发槽位。不增加模型岗位或调用次数。

对照仍限于美术比较，不成为源事实、图片引用或设计有效性依赖。已删除、失效设计和同身份／来源链仍排除，数量与字符上限不变。人物基础或派生请求生成后将实际对照冻结入该次请求及设计上下文。

旧工作流没有新策略字段，保持原来的整批冻结和并发行为。新工作流恢复从 `workflow_steps` 的原 text job 读取请求，校验父流程、节点、源哈希和请求摘要；图片续跑只沿快照哈希绑定的显式原流程查找。已派发请求采用原对照，并比较完整创作输入、schema、模型、reasoning 和规则；漂移直接拒绝，不重发、不改 raw／回执／采用。运输重试证明仍由原 TextExecutor 在派发前验证，不能用对照恢复跳过它。

离线回归过滤：`incremental_comparison_`。涵盖新项目 A→B→C 对照回流、接受后才可引用、人物单通道与非人物并发四槽、父级拓扑、失败前件恢复后后件请求 hash 不变与已接受缓存复用、旧流程兼容，以及临时 SQLite 中绑定请求的只读恢复和摘要漂移拒绝。

2026-10-12 Windows 本机离线结果：新增过滤 5/5 通过；资产执行模块 32/32 通过；设计合同模块 7/7 通过（新增项包含在模块统计内，不重复计总数）。`git diff --check` 通过。测试使用合成原文、mock executor 与临时 SQLite，没有读取或写入生产数据库。未构建 release 或替换桌面程序。

本改动不修改本轮已冻结的真实图片设计，不调用模型、不生成图片或视频，不代表视觉质量已完成实测验收。

完整 native 核心回归随后发现两个旧图片续跑失败：历史记录尚无 `stage2Context`，上下文重建分支误把它当作全新制作，加入新人物对照策略，沿原父工作流恢复时被严格策略校验拒绝。已修正为只有不存在 `imageResumeOrigin` 的真正新请求才启用新策略。旧上下文重建继续采用旧合同，不修改原快照、参数、对照、回包或预算；原父工作流策略校验不放宽。

修复后离线回归：`incremental_comparison_` 7/7，`engine::repair_tests` 24/24（含原失败的 `native_resume_of_legacy_completed_but_blocked_image_production_preserves_record_and_never_submits` 和 `stage2_restart_resume_uses_original_settings_and_final_submission_snapshot`）。补充证明旧非空冻结对照在部分失败恢复后完整请求不变、接受缓存不重新生成，以及意外新策略跨旧祖先仍失败关闭。未删原断言、未改生产数据或启动真实执行；本次只证明这些定向与相关模块回归，不宣称完整 native 套件或真实端到端通过。
