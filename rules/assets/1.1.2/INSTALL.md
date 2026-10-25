# 安装建议

将整个 `gpt-image25-asset-workflow` 文件夹交给 Codex，保留配套 modules、templates 与 schemas。当前版本见 `SKILL.md` 标题。
主入口为 `SKILL.md`。

建议 Codex 首次加载顺序：
1. `SKILL.md`
2. `modules/01_project_intake.md`
3. 按任务路由至对应模块
4. 仅在需要时读取对应 templates / schemas

已有范围、风格和连续生成授权的项目，不重走首次接入；读取 `modules/21_batch_production.md` 后恢复既有队列。技能更新不触发重新生图或自动续跑。

不要一次把所有模板完整塞进单条提示词；应由编译器按当前资产动态抽取。
