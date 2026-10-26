# 同冻结流程资产补料实现与验收记录

本记录对应 `feat/series672-assets112-workflow-20261025` 工作区，尚未独立提交。主控统一编译和真实 SDK 验收；本文不把合成回归计为真实模型、图片或视频验收。

## 当前 v1 的问题与处理

第一集原作者在完整候选中指出缺少道具设计；原有设计入口只允许已选中的资产 ID，后续作者修订也保留原 ID 枚举，因此无法补齐缺项。原作者还采用了句内对白坐标，原文绝对坐标诊断已由主控提供。

本轮保留原作者原始回包和拒绝状态，不另买一份作者、不删除 `unresolved`、不在运行中提前写资产。原作者作为未采用输入证据，经过以下同流程附加链再进入原整编与独审：

1. 精确复用原 author 请求、原 raw 及其拒绝原因。
2. 在首次派补料前，冻结完整资产目录摘要、历史身份映射和序号高水位；保存在原 workflow 的 prepared 子项及独立事件中，不修改原 input，也不预留序号。
3. 补料设计逐项决策：必要的新图资产、已有本集图资产、场景细节、无需图的验收边界。新增设计需完整正文、源证据、制作选择及依赖。
4. 独立设计复核。一次补料最多 3 次设计/复核（初次 + 2 次修订），相同问题停止；所有节点有稳定执行 ID、请求/原回包摘要和前驱绑定，恢复复用原请求。
5. 原整编收到完整源、原未采用候选、绝对坐标诊断、原有完整设计和复核通过的新增完整设计。整编后继续原严格校验和非作者独审。
6. 同版交付保存 append-only 证明；原有供卡逐字段保留。最后发布事务先过原 source/foundation/asset/unit guard，再校验唯一原 rejected author 和所有 accepted 补料/整编/独审节点，才插入新卡、采用设计和分镜。序号/身份的历史或并发冲突会整笔回滚。

已有图片、手写图片提示词、旧资产版本、原 raw、预算和 UNKNOWN 状态不被清空或重置。重复逐集/总流程发布依据同 workflow、同交付摘要和原已保存交付幂等处理。

## 代码归属

- `pipeline/series_supplement.rs`：v1 补料、源证据、决策与独立复核、冻结上下文校验、append-only 证明。
- `pipeline/series.rs`：原作者未采用证据分支，补料后继续原整编与独审；原绝对坐标诊断和新任务 marker 保留。
- `pipeline/series_design.rs`：仅将已有完整设计/供卡投影 helper 开放给同模块补料使用。
- `series_store.rs`：最终事务新增身份、历史高水位/名称冲突保护、重复发布幂等；旧资产只合并原允许的设计字段。保存重开后的既有供卡按原发布/源文/单元/历史版本证据验证。
- `binding.rs`：第四阶段仅在上述证据通过后使用内存集场投影；不会把补供用途写回或泛化为资产的全局适用范围。
- `workflow.rs`：额外精确验证补料原作者仍是唯一同 workflow 的 rejected 节点；未放宽一般 accepted 岗位回执校验。
- `pipeline/series_supplement_tests.rs`：纯合成 executor、真实本地存储和发布事务回归；没有实际 provider 请求。
- `text_executor_series_supplement_context.rs` 与 SDK schema rejection 接线由 SDK 子任务负责，主控统一集成。

## 已执行的 Windows 证据

命令范围：冻结测试 EXE 的 `pipeline::series::tests::supplement_tests::`，由 Windows Job Object 原生 runner 限时执行。

`native-supplement-22/stdout.log`：13 项通过、0 失败，55.61 秒。

`native-supplement-22/receipt.json`：exitCode 0、success true、processTreeEmptyVerified true；这是本地合成和存储验收，不是模型内容质量验收。

证据根：`.local/series672-assets112-20261025/`。

覆盖：不重买/不采用原作者、原 raw 和原事件精确保留、绝对坐标由整编修正、原设计不变、场景细节零新增、UNKNOWN 停止、伪造源拒绝、独审有界修订及恢复、相同问题停止、历史名称/序号冻结、最终事务冲突回滚、保存重开、重复发布不重复插卡。

早期第 19 轮失败来自合成输入更换源文后未同步内容寻址的 episode/scene ID，已修 fixture，未放宽产品 scope。第 20/21 轮最后一项是测试误读 `Store::job` 的 UI 投影：真实状态字段为 status，UI result 还会附加 mediaIds。现改为直接核验数据库 `jobs.result` 和原 `text_received_raw` 事件，两者都必须与原 raw 全等。

## 真实首轮新发现与修复

主控真实补料回包出现程序误拒，原 raw 和请求均保留：

- 精确源事实可引用本集另一行，不必在 proposal.evidence 重复列一次。现逐条独立核本集 body row 和精确 quote，合法事实引用并入新卡 sourceRefs；不接受跨集/未知行或伪造文本。
- 仅说明尚无真实配音/动作演练验收的边界，可以 `no_visual_asset`、空 proposals/已有 ID 和空 evidence 表达，并保留原问题索引与理由，仍需独审。不得因文字交付先生成媒体，也不得宣称成片已验收。
- 新补料 review 明确提供候选摘要，并把 schema 字段限定为同一实际摘要。`reviewDigestBinding/v1` 区分新运输约定；已冻结的旧 review 保留原完整请求，不能因升级重派。
- SDK 的精确旧误拒重验证采用由 SDK 子任务实现，需保留原拒绝事件并记录迁移证据；不能把通用 rejected 都允许重新采用。

第 23 轮新增/更新后的 15 项回归已实际通过：`native-supplement-23/stdout.log` 显示 15 passed、0 failed，118.60 秒；对应 receipt 为 exitCode 0、success true、processTreeEmptyVerified true、timedOut false。该轮覆盖上面两处源/边界校验修复及新旧 review 摘要运输合同。真实 SDK 原回包继续、内容质量和整集最终交付仍由主控单独核验，不能由这 15 项合成结果代替。

## 既有资产供卡增量 v2（源码已接线，等待本轮编译验收）

v1 只支持全库缺失的新身份和原本集供卡中的已有身份。全库存在但漏了本集 applicability 的资产，不能偷偷复制同名新卡，也不能只放宽 ID 枚举。

主控已要求继续实现有据的既有资产供卡：有效完整设计复用全文；缺完整设计的同 ID 补设计并独审，保留旧版本并执行 assetVersion/designRevision CAS；本集用途记录与原 applicability 分开。前集新增资产在后集供料也属于此范围。

`pipeline/series_supply.rs` 已接入设计、独审、整编和交付证明。v2 新 origin 首次冻结选择新合同；已有 prepared v1 origin 原样返回原合同，不重买已完成设计或复核。SDK 上下文保存全库轻目录摘要，以及各资产的数据库版本、设计版本、完整设计摘要与完整供卡摘要。

- `existingSupply` 只选择已有而未供给本集的身份，并提供本集精确源证据。设计有效时必须 `reuse`，模型不能重写完整设计；本机读取被选中资产的原全文供独审。设计不可用时才 `complete`，沿原身份形成下一设计版本，仍必须通过独审。
- 完整源在 v2 设计/设计独审包只存一次，`source.sourceRowsBinding` 指向 `designSourceRows`。全库仅传轻目录；已供本集资产和选中补供卡保留完整设计。整编包不再重复携带 `existingSupplySources`，它已包含在全量 `assets` 中，其验证来源保存在原独审请求。
- 交付集合为原本集卡精确保留、既有补供卡、新卡。`episodeSupply` 单独保存本集用途增量，不改旧 `applicability`。设计事实来源与分镜单元用途来源明确分开：`unit.assetRefs[*].sourceRefs` 必须来自该单元 `sourceRefs`，不能使用其他单元/其他集的设计来源冒充用途。
- 最终事务按冻结 `assetVersion`、`designRevision`、原完整设计及供卡摘要检查；补全只合并允许的设计字段，保留旧 `asset_versions`、手写提示词、图片、身份与原适用范围。纯复用不重新采用设计。相同 workflow / 相同已发布交付直接幂等返回，避免重复升版。
- 保存重开时从已验证 `suppliedExistingAssets` 补回本集供卡后比较完整设计，避免仅因原适用范围未改而重买作者。前集新增身份即使不在初始 frozen input 中，也须由同流程冻结的历史身份和实际资产状态绑定，不能仅凭候选 ID 放行。

已新增合成回归：reuse 全文及额外旧字段不变、complete 独审和下一 revision、源文只传一次/轻目录无设计全文、前集已发布身份补供、缺冻结状态/换掉原设计拒绝、发布 CAS 冲突、手写提示词和原图引用不变、旧版本留存、同稿重复发布、保存重开复用。另补 v1 新增资产导出、initial/current 总库区分、独立灵墨导入零生成回归。

第 24/25 轮暴露了同一阻断：既有供卡及原设计已正确进入整编，但旧分镜校验仍按原 `applicability` 拒绝本集。现不放宽旧校验，也不改原卡；仅在补供链和独审证明通过后，由 `episodeSupply` 的精确 body-row/quote 推导有据场次，向校验器传入临时范围投影。其它集/场仍不放行。最终交付必须先验证完整补供证明，再进行候选校验，不能信任回包自称的范围。

第四阶段一次核验本单元的已发布回执、原岗位/原 raw、本集源文、单元集场和原文引用，以及对应已发布资产历史版本。当前资产必须仍有相同身份、原适用声明和完整采用设计；图片提示词或图片准备造成行版本前进可以保留，但完整设计变化不得借旧范围继续。该投影不改变数据库资产。媒体未就绪仍报告缺图，不因供卡成功宣称可生成视频。

第 27 轮实际结果：`native-supplement-27/stdout.log` 为 **19 passed / 2 failed**，92.39 秒；receipt 为 exitCode 101、timedOut false、processTreeEmptyVerified true。已有复用、同 ID 补全、CAS、前集身份补供已通过；两处失败分别是 `asset_versions` 正确列名为 `asset_id`（测试及新增第四阶段查询误用 `id`），以及合成测试定位源行未考虑原换行。已修正精确查询和测试查找；源正文及 evidence 仍逐字保留。修后完整第四阶段闭环和 SDK v2 尚待新一轮回执，不能将 19 项通过写成全部验收。真实首集仍使用先前冻结的 v1 请求。

## 新任务的返修规则正文去重

只对新冻结 input 增加 `seriesRuleMaterialization=series-rule-content-once/v1`。含原岗位与精修两套规则的返修包，以完整正文 SHA 为 key，在 `bodies` 中各存一次；`roleSets` 保留各岗位 primary、methods 原顺序、完整描述符、源哈希、适配信息和 creativePack。显式读包合同要求按角色顺序读取全部正文，不能用摘要或哈希代替规则。

还原必须与原规则对象逐字段全等。缺正文、错正文或引用、额外未引用正文、未知角色/版本均拒绝；岗位内容与顺序再同冻结规范比较。普通独审后精修原本只有一套规则，保持原 shape；没有新 marker 的历史任务也保持原请求结构与字节序列化，不为去重升级已准备/已提交请求。

仅规则正文去重，没有删减原文、采用设计、作者计划/选择/候选、被拒整编候选、源坐标诊断或预算。新增四项规则与领域返修合成回归，另有 SDK 兼容回归；其实际结果须单独记录，不能据字节减少声称真实 token 或耗时固定下降。
