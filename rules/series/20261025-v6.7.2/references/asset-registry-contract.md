# 统一资产总库与补项闭环

选季项目先遵守[季范围合同](season-scope-contract.md)：初始总库绑定productionScope及productionScopeHash，所有独立资产包含本季明确scopeUses；asset-design申请同样提供scopeUses，production沿实际uses核集场，移位申请也不能越界。全库可大于单元子集，但不能借此提前设计后续季资产。`image-queue`只从显式实际用途、当前采用设计生成可核验清单；总库保留不等于全库出图授权。

全流程只有一份当前资产总库，通常位于 `JOB/shared/assets/`。阶段一建立基线，阶段二完成完整设计、复核采用并提炼短卡，阶段三提出缺项和实际使用，主代理核重、设计复核并采用，阶段四对当前总库核验及交接。分集 `bundle.assets` 只是实际使用投影，不是第二份总库。初始快照及旧版本保留；总库可大于任意单元的使用清单。

新生产采用 `registry.importProfile:"lingmo-v7"`，总库保存 `importName` 与 `placeholderOrdinal`；使用范围仍由实际分集逻辑绑定及图片绑定分别表达。具体输出见[灵墨 v7 合同](lingmo-v7-contract.md)。命名和编号只由主代理写同一总库，各路提交缺项和使用坐标。

## 第一阶段：提高基线完整度

按原集、原场、事件做两遍核对：先提明确实体及状态，再按动作链反查依赖——谁拿、给、操作、拍摄、通信、打开或乘坐什么；同时检查屏幕、照片、历史插入与纯声源。只纳入对动作、身份、空间或连续性有用的资产，不把每个背景杂物、姿势、表情都变成资产。

原文“拿手机直播”可记手机为源事实；只有“直播”时，仅能确定拍摄/传输需求，手机或固定相机仍是设计选择。记录未定的功能需求和源锚，阶段二/三按已确认方向选择后登记 `origin.kind:design`；不得把推定设备写成原文明示。阶段一按集场/事件登记适用范围，此时不虚构分镜单元使用位置。

稳定ID对应可复用的具体资产/状态；身份家族、父子变体、原名另存。相同身份的不同必要造型不能误合并，同一设备的不同称呼也不能重复建项。主代理核身份、功能、外观、持有关系、时点及已有别名，工具只会发现已登记的同名/别名碰撞，不能自动理解任意近义词。

originalName是源文称呼，可由同一家族多个必要状态共享；不能当全局唯一别名。name与aliases须区分具体采用状态，父子/家族关系另存。比如“直播手机”和“直播手机·碎屏”可以共用originalName“直播手机”，使用时仍绑定各自稳定ID及状态。

## 总库文件与文字卡

仅用包内 `scripts/assets.mjs` 写总库，不手改 `current.json` 或 `v*.json`。初始输入：

```json
{
  "sourceHash":"原始剧本文件的64位SHA256",
  "importProfile":"lingmo-v7",
  "assets":[{
    "key":"门厅SC", "name":"门厅", "type":"SC", "status":"常态",
    "importName":"门厅",
    "originalName":"门厅", "upstreamKind":"LOC", "aliases":[],
    "origin":{"kind":"source","sourceRefs":["E01:R1"],"basis":"原文明示地点"},
    "design":{"state":"UNADOPTED","text":""}
  }]
}
```

`type` 仅为导出分类SC/CH/PR，保留原语义分类，不由后缀反推身份。v7 的 `name/key` 是内部正式状态名和逻辑键，单行且 `key=name+type`，保留原标点；导入安全名另存 `importName`。F2/Q1已有的稳定key另存upstreamKey并保留原目录，不能用“名称+后缀”覆盖它。原目录成为来源快照，后续当前资产以总库为准，不能同时维护另一张“当前表”。总库id是本地跨阶段引用，工具分配后固定；原家族及必要变体映射到相应记录并保持父子关系。`origin.kind` 为source或design；sourceRefs是本工作区可回查的源坐标，basis说明源事实或设计依据。必要时另外保存sourceFacts、openItems、familyId、parentId、适用集场及用户决定，不能删掉上游信息来适配最小例子。

`design.state` 为UNADOPTED、TEXT_ADOPTED、REFERENCE_ADOPTED或STALE；已采用者必须有实际短生产卡 `design.text`。新制作的正式视觉资产还须按[前期质量合同](preproduction-contract.md)保存完整设计 `design.full`，短卡由该采用设计提炼。源事实、采用设计、未知项须可区分。REFERENCE_ADOPTED仍不由工具证明图片已读取或可用。

`design.full` 是总库内的结构化设计记录，包含 `version`、适用类型所需的完整 `body`、源约束引用 `sourceRefs`、补足及开放项 `supplements`、实际适用范围 `applicability` 和 `review`（源/方向依据、审核方式、实际问题与采用结论）。既有sourceFacts等字段继续保留，不为套此结构删掉来源。review记录总导演依授权采用，不伪称用户逐项看过。没有源明示外观时，body中的补足逐项与源事实分开。必要的大附件可另存版本文件并在full中附相对路径与hash；打包时随资料带走。

`design.dependencies:[{assetId,assetRevision,purpose}]` 记录当前设计实际继承的其它资产设计及其版本；独立设计可省略或为空。人物身份母版、派生换装/伤势/兽形的外观继承、场景状态的母空间继承应据实际采用关系记录。`parentId/familyId` 只表示身份/家族，不能自动代替设计依赖；同一家族也可能各有独立源约束，不能一律连成依赖链。依赖不得指向自己、重复、未知项或形成环。

总库允许保存依赖旧版本的待复核设计，但消费时递归核真实依赖：每项所据版本须是当前版本、所据卡须已采用。来源卡修订或STALE时，受影响派生卡即使仍标TEXT_ADOPTED也不能直接消费；先回读完整设计与实际差异，核兼容性后以revise保存当前依据、必要改稿及短卡。不自动升级版本号或改写下游设计来掩盖过期。没有声明依赖的旧库保持工具兼容，迁移时按实际设计补齐依赖，不冒称旧库已做此核验。

工具检查身份、版本和声明依赖，不评判 `design.full` 是否足够完整、补足是否忠实或短卡是否真的保住空间功能；这些仍由阶段2及内容复核实际阅读。历史短卡不因工具能读就自动满足新流程，已有完整材料可保真接入，缺少者局部补齐。

`init` 分配A0001等稳定ID、每项revision:1及唯一总库id。v7 同时按初始数组顺序分配零基 `placeholderOrdinal`；后续 add 只用当前最大号加1，revise 保留旧编号。编号0..5700对应300..6000像素，超范围报错，不复用已分配号。总库每次更新写新快照，原子更新当前指针；锁防并发写入，baseRevision防旧提案覆盖新修改。锁遗留须先核真实执行者，不能自动抢锁。当前指针含快照hash，可发现原地篡改；hash不是内容审核。

## v7 安全映射与旧库迁移

`importName` 按 Python `str.isalnum()` 字符规则，非空且不等于“无”。工具从内部 name 确定性去掉非该类字符；同类安全名重名、NFKC+casefold 后文件名冲突、清洗成空名或“无”均拒绝。冲突需显式安全名裁决，不改内部 name/key、不猜同物异名、不合并状态。三行、@ 和图片文件名只从这个映射投影，导演正文和原声称呼保持。

尚无 importProfile 的旧库首次准备 v7 映射时，由主代理先读当前版本，保存输入：

```json
{"version":"series-asset-import-v1","baseRevision":3,"importProfile":"lingmo-v7","names":[{"id":"A0001","importName":"门厅夜间"}]}
```

```sh
node scripts/assets.mjs prepare-import --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/asset-import_v1.json
```

`names` 可省略；未显式指定项按内部 name 的确定性字符清洗处理。显式项用于解决冲突，不能填写另一个实体的名称来偷换身份。成功写新总库快照（总库 revision +1），保留稳定 ID、单项 revision、原名/家族/源事实及设计采用字段，分配稳定占位编号。失败不改旧快照。此命令只做旧库一次迁移，已有 v7 总库再调用会拒绝；后续安全名修订用 apply/revise，递增单项 revision 并保持旧编号。把新映射同步到实际消费者的新版 bundle、目录、MD 和采用证据；首次迁移虽然单项 revision 未变，也不能沿用错映射的旧交付。

默认白图范围取本次全部 `imageBindings` 的并集；纯声音身份不虚造图。若明确要求整库图片包，先确认正式图片项范围，再从同一总库投影。白图只作占位，不改变 `design.state`，也不证明参考图已采用。

## 第二、三阶段：采用与回补

普通有据补料由总导演按已确认方向自主处理，不重复向用户审批。真正改变源事实或越过已确认方向才提出最小裁决。

1. 作者提出缺项，写在本集目录：申请ID、集/场/工作包、最终单元位置、用途、源锚、候选名及同物异名线索。未有稳定ID时不自行分配全局键；受影响单元保留待补，其他工作继续。
2. 主代理读取当前总库，核源与核重。漏拆用source；有据导演选择用design；源冲突先修冲突。已有项用reuse，需改已有卡用revise，真正新项用add。
3. 正式视觉资产先完成完整设计、逐项核源及必要跨项复核，保留full及真实设计依赖，再用 `apply` 登记新快照和短卡，返回采用ID、资产版本、内部正式键及 v7 导入名/稳定编号。到料状态为awaiting_consumption，此时尚未关闭缺口。既有有效设计直接复用；纯声源或临时表演不硬造图片设计。
4. 作者按采用结果更新实际画面、props/participants等、unit.bindings、voiceBindings及场级逻辑绑定并集；v7 另核 imageBindings 和需要时的 visualSubjectBindings，不把全部声源自动变图片。不能只填清单或加@而不读画面。整编重新核实际使用，`sync` 检查并产生与当前bundle字节绑定的使用回执。
5. 主代理读取回执，只有本次范围内请求确实消费才关闭。跨集请求仅部分到位时保持范围外待办，全范围合并再核。仅送到文字卡、作者说已用、存在文件或旧PASS都不足以关闭。

第三阶段补项接口例子（仅展示最小机械结构，真实正式视觉资产须同时补入上述full及复核依据，不能照抄短卡就称设计已完成；先据真实源和当前总库改值）：

```json
{
  "version":"series-asset-change-v1", "baseRevision":1,
  "requestId":"EP01-PHONE-01", "stage":"production", "action":"add",
  "reason":"第一单元直播拍摄设备漏列",
  "duplicateReview":"已读当前总库，无同一设备；手持拍摄手机为本项别名",
  "uses":[{"episode":1,"sceneId":"1-1","unitId":"U1"}],
  "asset":{
    "key":"直播手机PR", "name":"直播手机", "type":"PR", "status":"常态",
    "importName":"直播手机",
    "aliases":["手持拍摄手机"],
    "origin":{"kind":"design","sourceRefs":["E01:R3"],"basis":"原文仅写直播，按已确认方向选择手机承载拍摄功能"},
    "design":{"state":"TEXT_ADOPTED","text":"黑色直板手机，背部双摄；同一物件持续使用。"}
  }
}
```

`revise`另传assetId及完整新asset，保留原有源事实/家族/状态字段；工具保留省略的旧字段，清空旧内容须显式赋值并核其影响，嵌套对象须给完整采用内容。v7 修订保留旧 placeholderOrdinal，不因改名或重排换号。类型或身份改变不能套旧ID。`reuse`只传assetId，不传asset，也不递增资产版本。阶段二尚无单元时，用 `stage:asset-design, uses:[]` 采用或补卡；只登记变更，不虚构单元消费。第三阶段不得借asset-design跳过已发现缺项的使用闭环。

```sh
node scripts/assets.mjs init --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/assets-baseline.json
node scripts/assets.mjs apply --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/asset-request.json
node scripts/assets.mjs sync --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/bundle_v2.json --output /absolute/JOB/ep1/asset-consumption_v2.json
```

`status --registry DIR`只读返回当前快照路径、hash与目录，可将stdout保存本地后按需读短卡，不把全剧长库反复塞给各岗位。`apply`不改写分镜：正式名、状态和镜头语义由作者落实，`sync`负责验证已声明的消费。

## 修订、局部影响与退回

总库快照版本与单项资产版本不同。新增无关资产不会使旧采用稿失效；某项的名称、状态或设计改动后，核直接消费者，以及通过design.dependencies继承该设计的派生卡与其消费者。沿真实依赖逐层复核，不扩大到仅同族但独立设计的资产。跨集出口真的改变才继续检查相关下游。旧版本文件和完成证据保留，不原地改它们。

同一资产旧版本不会自动当成兼容：先读受影响实际画面；仍兼容时生成新bundle版本、更新资产版本引用及消费回执，可不重写镜头。需改画面时局部修稿并复核。当前工具要求同一最终bundle对同一ID采用当前版本；确实需要不同持续状态应建立有依据的状态变体ID，不能拿旧设计版本代替剧情状态。

精修重分单元，用 `amend` 更新原请求坐标；有据撤回可选设计则撤回其使用请求，保留总库记录。修订输入为 `version:series-asset-amend-v1, baseRevision, requestId, action:retarget|withdraw, uses, reason, sourceReview`；retarget的uses须有最终位置，withdraw的uses为空，sourceReview说明如何保留源要求。命令：

```sh
node scripts/assets.mjs amend --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/asset-request-amend.json
```

不能借withdraw删除源必需设备；改用其它设备时另登记其真实消费。旧快照保留此前位置和决定。反向使用差异只更新使用关系，不删除源资产。

## 采用、导出与证据门

生产bundle必须带原件sourceHash、`assetRegistry:{id,revision}`（制作所据总库版本）及每项投影的 `assetId/assetRevision`。全部已用项的键、名、类型、状态、版本及已采用文字卡必须匹配当前总库，其声明的设计依赖递归核有效版本与采用状态；v7 另核总库 importProfile 及 importName/placeholderOrdinal 映射。未用项不要求与当前单元数量相同。bundle的总库版本可落后于当前新增项，已消费的单项版本、设计依赖与映射不可落后。`check/export/verify --registry DIR`都执行此门；不能把真实稿改成testFixture来绕过。仍带任何生产绑定字段时即使改测试标记也必须核总库。纯合成格式样例可独立运行。

`sync`先执行完整check，再写排他创建的回执，包含bundle hash、总库快照hash、逐集/场/单元消费、已关闭请求及范围外待办。将当前bundle、实际使用的不可变总库快照、回执、正文—清单/图片复核记录、v7预览及配套目录纳入本集采用证据；不要把可变current.json列为旧完成证据。新总库到达时重核真实依赖，最终合并后再sync；旧回执不证明当前总库仍一致。

**正文—清单及图片复核必须读实际稿。** 作者及最终复核者逐单元核picture、开场/结束状态、演出与声音中的实际主体、持物和媒体主体，再对props、participants、声源映射、bindings及imageBindings。只提及、画外声、屏幕里的人、普通背景细节不机械扩成现实入画资产；视觉表现的 assetKey:null 不强立卡，正式可复用状态则照常回补。输出场三行是图片并集，单元只引用自身图片，不能照搬场目录。程序不做自由文本语义识别；即使正文漏写进清单仍可能机械通过，不能把check当这一步的替代。

实际持握/交接、特写辨认或跨段持续依赖的物件，不得因三行、props或旧图片目录未列出就跳过核查。先判定：已有正式资产复用、需要补正式道具、已由指定场景设计覆盖的固定细节，或仅文字/特效表现；保留所据实际镜位与设计依据。需要稳定外观和独立参考图的，完成上述总库回补并更新实际imageBindings；确无需独立图的写明覆盖方式，不能把每个背景名词都变成新图片。若已有生图队列，同时按[接续合同](asset-handoff.md)交付本次范围增量；只加道具名称而旧图片目录和队列仍停在旧版，不算已接续。

旧生产稿迁移时保留原稿：从受核上游资产账建立基线并与实际稿双向核对，prepare-import 建立安全映射，形成新bundle、逐段图片引用、预览/目录和回执；不能只从第三阶段清单反推“第一阶段已经齐全”。原始剧本未变不需要重做源接入，实际有效镜头无需重创作。旧转换的冻结范围只约束当时项目，新项目直播手机等有据漏项仍按此闭环处理。

最终按[资产交接合同](asset-handoff.md)保留并另交完整初始/当前表、完整设计、来源/方向和实际使用映射。MD三行、bundle.assets和v7图片目录都是不同用途的投影，不能覆盖总库或冒称它们就是完整资产表。
