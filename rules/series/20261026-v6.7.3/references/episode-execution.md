# 单集执行、恢复与采用

v6.7 新单集流程使用岗位包、正常交接和下面的本地执行入口。主控负责命令；作者/整编者只接本次岗位包，交完整候选与必要改动说明。工具不调用模型、不选择幻想，也不修改分镜正文。

## 一次准备，直接交接

先按[job-packet](job-packet.md)编译作者包，从已审核的资料选择完整源、方向、有效状态、完整采用设计和实际入口。新上下文只读包中当前有效规则及材料，不再从总入口重新启动制作。统筹和完整主创在同次创作中完成；收到完整候选后编译整编包，包含全部源、plan、selection及draft。

作者停止写入后，正常换整编者用：

```sh
python scripts/workspace.py handoff --workdir JOB --episode 1 --owner author-1 --to finisher-1 --candidate ep1/bundle_v1.json --evidence ep1/author-notes.md --role finisher
```

交接不转回pending，不再claim，不要求另写恢复报告。evidence复用已有作者说明，candidate冻结完整候选。`resume`留给实际中断、阻塞修复或重开已采用集。换owner不证明新上下文；宿主仍须实际新建执行者。

在有效recommended/all委托内发现额外幻想机会，可在同次创作交回更新的plan、selection与完整候选；主控核源锚和许可后编译新版供料包，不让作者为正常补漏停工编写恢复文件。off、未定review和用户取消仍不能越过。若整编改变计划/选择，也须核定后生成与最终候选一致的新包，再执行采用。

## 一条命令完成本地预览

保存一次执行清单，例如 `ep1/execution_v1.json`：

```json
{
  "version":"series-episode-execution-v1",
  "episode":1,
  "packet":"ep1/packet-finisher_v1.json",
  "bundle":"ep1/bundle_v2.json",
  "sourceLock":"shared/source-lock_v1.json",
  "registry":"shared/assets"
}
```

所有路径相对JOB；output使用独立空目录，不能指向源、候选或其它输入所在目录。选季项目另填 `seasonScope` 为本季locked文件路径。packet须含唯一已采用visual-baseline，与bundle的settings.visualBaseline及完整visualStyle一致，省略或改写时拒绝采用。packet必须是当前完整整编/审阅包，其中唯一source、plan、selection分别对应bundle内该集的完整值；draft是整编输入基稿，允许最终稿有实际改进。相关决定和设计由包输入指纹冻结，但方向语义是否落实仍需读稿。

```sh
python scripts/episode_execution.py run --workdir JOB --manifest ep1/execution_v1.json --output ep1/run_v1
python scripts/episode_execution.py verify --workdir JOB --receipt ep1/run_v1/adoption.json
```

`run`顺序执行sync/check/export/verify，输出完整日志、消费回执、预览、配套资产目录和`adoption.json`。同一命令再次运行时校验输入及已完成输出，复用成功步骤；输入改变须保留旧目录并使用新候选版本。导出只写出一半就中断时保留片段，在新attempt子目录恢复完整双文件；不覆盖旧预览。失败说明留在命令stderr与已有日志，不重买创作、不自动重写正文。

收到收据后实际读预览，核原声、可见行动、情绪、空间及正文—图片用途。完成证据加 `"adoptionReceipt":"ep1/run_v1/adoption.json"`，`adopted`引用收据绑定的完整bundle，再由当前owner调用workspace complete。使用handoff的新链路缺收据会拒绝；收据不是独审结果，reviewMode仍按实际填写。

## 校验与兼容边界

- 收据绑定岗位输入、有效规则、运行工具版本、bundle、源锁、相关资产、预览和目录；旧包配新plan/selection、输入或输出原地改变均拒绝。哈希用于版本一致性，不证明审阅者实际读过材料。
- 当前总库每次仍读其有效快照，但执行指纹仅取本集实际资产及母设计依赖、当前集补项和范围；无关新增不使所有集失效。引用整个可变总库作为岗位输入仍会整体失效，应供料采用版本化的相关设计文件。
- 程序只检查已声明的依赖，无法发现源/设计遗漏或判创意好坏。独审、实际跨集承接、全剧最终合并和白图交付继续沿原合同。
- 旧context_packet v1、无新交接记录的旧workspace和手动工具入口继续可用，不批量迁移在制工程。新运行使用新包和收据；历史采用验证核冻结岗位包、原始运行版本记录和真实项目依赖；单纯升级Skill不使旧采用稿失效。新派发及尚未采用的运行仍严格使用当前规则，不能把历史收据冒称新版本重跑结果。

该工具减少重复机械执行，不提供CLI模型沙箱或后台自动调度。实际模型、思考强度、工具权限和用量以宿主真实派发为准。
