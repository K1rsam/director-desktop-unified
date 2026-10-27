# 分集创作与本地采用合同

这是新剧本从统筹到采用稿的文件接口，与[成果数据合同](data-contract.md)合用。Codex读源后写这些文件；本包没有自动调用统筹、自动选择幻想或自动应用精修的执行器。下列形状取自随包 `output-schema.mjs`、`event-unit-packing.mjs` 与实际导出接口，示例文字和秒数只解释字段，不作为创作模板。

新生产 bundle 明确 `exportProfile:"lingmo-v7"`、`generationTextVersion:"voice-delivery-v1"`，并绑定[接入方创作前冻结的sourceLockHash](data-contract.md#创作前冻结源)；新主创/整编/精修不改source或锁，后期文字独立交接。已有无锁稿保留旧协议和证据边界，不因升级全部重做。图片引用及输出按[灵墨 v7 合同](lingmo-v7-contract.md)。该 profile 只替换导入投影和内部整数总长格式门，不改变下列原场/事件范围、完整主创、幻想选择、整编精修或资产回补。包内创作来源的整数终点说明在 v7 下以精确内部时间线和外层容器取整规则为准；15/30秒项目上限仍严格执行。

岗位引用的capacityBrief、rhythm.audit、平读音频或既有图像是可选辅助。实际提供且已读取时使用；未提供时直接按源文、完整原句和实际镜/声窗核容量、连读间隔、动作先后与观看收益，不等待外部程序、不编造审计数值或实配结果。后续真实配音/视频才用于实测复核。

分集开工先核[前期质量合同](preproduction-contract.md)及本集依赖；按[供料合同](context-and-dispatch.md)实际读取本集所用正式视觉资产的完整采用设计及有效源约束，短卡仅作定位与投影。每集实际消费必要全剧导航、知情/揭示时点、采用设计及其依赖版本，保留来源记录。该条件由总导演读内容执行，不把workspace领取或格式检查成功当质量放行。

表现与文字决定按[对应合同](expression-and-screen-text.md)随本集供料，保存在项目决定与处理索引，不向原有严格创作响应中随意塞服务器字段。幻想模式仍写下述settings/selection；其它倾向由实际正文兑现，源文字目录不直接当成上屏清单。

## 新任务的完整计划

新制作使用 `schemaVersion:"performance-core-v4-plan"` 和 `unitPackingPolicy:"event-first-v1"`。`sourceRefs`取本次 `inspect-source` 的真实ID；不能从样例抄ID。最小计划如下，假设本次源确有同一场的两行 `E01:R1/E01:R2`：

```json
{
  "schemaVersion":"performance-core-v4-plan",
  "unitPackingPolicy":"event-first-v1",
  "emotionArc":"等待中的警觉转为明确提醒，仍保持克制。",
  "events":[{
    "id":"EV1", "sourceRefs":["E01:R1","E01:R2"], "sceneKey":"1-1|门厅|现实",
    "boundaryBefore":{"kind":"episode_start","reason":"本次完整单集源范围开始。"},
    "emotionContract":{"subject":"值守者","contextTone":"警觉","audienceEffect":"读到提醒的必要性","change":"等待转为提醒","landing":"提醒后继续留意门口","retain":["提醒的原句与当下持物状态"]}
  }],
  "entryFacts":[],
  "workPackages":[{
    "id":"WP1", "eventId":"EV1", "sourceRefs":["E01:R1","E01:R2"],
    "emotionContract":{"subject":"值守者","contextTone":"警觉","audienceEffect":"读到提醒的必要性","change":"等待转为提醒","landing":"提醒后继续留意门口","retain":["提醒的原句与当下持物状态"]},
    "entryFactIds":[], "contextRefs":[], "dependsOn":[], "scenarioKeys":[]
  }],
  "provisionalUnits":[{"id":"PU1","packageId":"WP1","sourceRefs":["E01:R1","E01:R2"],"secondsRange":[4,8]}],
  "fantasyCandidates":[]
}
```

- `events`按源顺序覆盖所有实际剧情行，非空、互不交错；同一原句涉及多源行时完整归入同一事件。标题/空白等元信息保留在源与归场表，不必独占镜头。`sceneKey`描述事件的实际时空域，与必须保留的原场号 `sceneId`分开；同一原场内的现实、回忆或换地可以分事件。
- `emotionContract`的六个字段为 `subject/contextTone/audienceEffect/change/landing/retain`；前五项非空，`retain`为非空字符串数组。它保护源支持的作用与必要先后，不锁机位、动作方案、幻想形式、镜数或秒数。
- 事件 `boundaryBefore.kind`只取 `episode_start/continuous/new_event/time_space_jump`，`reason`给真实依据。仅首事件为 `episode_start`；`continuous`不能跨实际场景。同场可顺接的关联小事件由同一工作包负责，内部拍点可留在 `beatContracts:[{packageId,sourceRefs,emotionContract}]`，不能借换说者把完整事件拆给不同作者。
- `entryFacts`每项为 `{id,subject,statement,sourceRefs,evidenceMode:"source_rows"}`，引用本集或已读前情的真实源行。无额外入口事实可空；计划设计及前稿实际拍法另记，不能伪装成源事实。工作包 `entryFactIds`指这些事实，`contextRefs`只用于必要前情。
- `workPackages`给每个完整事件明确作者范围；`eventId`引用现有事件，`sourceRefs`限于该事件。`dependsOn`只引用源顺序中确需实际出口的前包，不能前包依赖后包；`scenarioKeys`只列本次实际使用的方法卡，无需凑满。
- `provisionalUnits`实际分配源范围并给估算区间，`packageId`引用工作包；区间不是最终镜长或单元数量命令。完整主创和整编可按真实声画重新联排，保持源覆盖及合法边界。

每个实际 `draft.units[]`除成果合同字段外，填 `eventId`及 `boundaryBefore:{kind,reason}`。首次进入事件沿其计划边界；事件内续接用 `continuous`，真实容量冲突才用 `capacity`并说明切点依据。单元边界可取 `episode_start/continuous/capacity/new_event/time_space_jump`。新稿不能为绕过事件归属检查删掉 `unitPackingPolicy`；已有受核旧稿保留其来源结构，按本次实际需改范围适配，不无故全剧重写。

## 幻想候选、模式与选择

额外候选最低形状：

```json
{"id":"F1","anchorRefs":["E01:R2"],"subject":"本拍情绪主体","emotionPurpose":"有源锚的独立情绪收益","recommended":true,"proposal":"可被主创超越的具体表达起点","screenSecondsRange":[1,3],"extraSecondsRange":[0,1]}
```

`anchorRefs`须属本集真实源行；区间为估算，前者含进入、发展、退出，后者只计不能与原声/动作共时的净增量。不能把字段示例变成幻想时长上限。没有贴切增益时 `fantasyCandidates:[]`；源明确的Q版、梦境、法术等进入事件的 `retain`并在正文兑现，无须伪造额外候选。

集中确认后的模式写入每集 `settings`，采用结果只写真实候选ID：

```json
{"settings":{"fantasyMode":"recommended","maxUnitSeconds":30,"visualStyle":"已采用的项目画风与画幅","prohibitions":"已采用的项目禁止项"},"selection":{"selectedIds":[]}}
```

| 模式 | 候选和 `selection.selectedIds` 的形成 |
| --- | --- |
| `off` | 不推荐或选择额外幻想，两个数组均为空；源明确表现照常保留。 |
| `recommended` | 统筹比较有力常规演出与候选的净增益，自主形成或调整本版推荐判断，保存实际采用ID；普通推荐不逐集再问。候选的 `recommended`是AI建议，不冒充用户选择或锁定。 |
| `all` | 采用本次实际提出的全部候选，已选ID与候选ID集合一致；不强造每镜候选。 |
| `review` | 用户明确选逐项确认，或本集已有其实际逐项覆盖决定时使用；保存真实确认/取消结果。已有答案直接采用，不能重复询问；需要用户逐项决定而尚未回复的候选不能擅当已选，实际无依赖内容仍可推进。 |

`fantasyEnabled`可省略，由消费端从模式派生；若保存，必须与模式一致：`off`为false，其余为true，且不证明某一候选已选。已有明确模式直接继承，未决时放入首次核心方向集中确认，不自行增加第二轮审批。用户在all或recommended下已明确逐项取消/锁定时，将其依据与本次选择覆盖留在版本化决定文件，把**该集**有效模式记为review并直接使用已有答案；不自动改变后续全剧默认模式，也不再发一轮确认。实际创作只按 `selectedIds`消费额外机会；候选建议与实际采用分别保存，不能让下游因个人喜好重筛已选作用。候选、已选ID、兑现记录闭合，兑现结构见成果合同。

## 额外幻想补漏的本地执行

本地使用版本化文件完成补漏，不依赖3008服务接口。按以下步骤串起已有工具与内容核对，不另建岗位或重复用户审批：

1. 主控读取本次有效模式及决定版本，确认存在自主推荐/采用委托。off、待决定review、明确只执行已选项和用户取消不能越过；已取消的作用不能仅换ID再加入。候选的`anchorRefs`逐项对照本集`inspect-source`，不能只因镜头引用合法就推定候选源锚也合法。
2. 保留旧版，新建如`plan_v2.json`、`selection_v2.json`及完整`bundle_v2.json`的候选版本，使用唯一新ID，保留原事件、有效既有作用和未改正文。新机会同时写实际画面、源引用、兑现记录、入出与尾态；未改旧镜不能仅挂新ID冒领补漏。候选准备不等于已经采用，失败时原采用计划、选择和正文不变。
3. 已有执行者先保存并停止旧写入；受影响的已领取/已采用集按既有`resume → claim`交接，pending集正常领取，不改`state.json`或重开无关集。新供料manifest显式列出新版plan、selection及作为direction输入的有效决定文件与其hash，连同完整必要输入重新`build/verify`；不沿用旧包投影。
4. 用当前总库对完整新bundle执行`sync → check → export → verify`，再实际读源、决定及正文。工具尚未自动核全候选源锚与逐项授权，也不会因某文件verify成功就证明它是本次最新版，不能省略上述回读。
5. `complete`前，由主控核本次plan、selection、决定、供料包、bundle及预览确为同一有效版本；完成证据的direction明确纳入这条输入/输出链。旧包仍能verify、当前owner能complete，均不证明旧证据可替代本次新版。全部核实后才一并采用；只有部分新文件或不匹配证据时继续保留候选，不覆盖原采用。
6. 旧采用历史保留。若下游实际绑定因此变为STALE，读取新旧出口与实际入口：兼容则更新`reconcile`证据，真实有影响才局部返修，不凭计划标签直接续作。

同集分多个工作包或因上下文换作者时，仍可按上述步骤补漏；主控串行采用共享计划/选择，候选标清所据版本，后一候选基线过期须先合并此前已采用变化再核完整稿，不能最后写入者覆盖前一包成果。保留未改单元与既有选中作用，新机会不得跨包冒领；采用稿换版后回读工具标出的下游旧绑定；出口兼容则据实reconcile，真实有影响才局部改稿。系统呈现改选同时按[系统呈现合同](system-presentation.md)更新真实决定/设计消费者。该本地流程不依赖服务端manifest，也不把3008多批限制照搬成禁止本地补漏。

## 整编、精修与采用

本桌面流程版默认使用[本地差量合并](desktop-adapter.md#差量合并)：少量既有演出/声窗字段用 `desktop-field-patch-v1`，结构拆合用 `desktop-draft-delta-v1`完整关联组，由工具按基稿hash生成**完整候选**并保全未改声画。模型只提交实际变化，不每集再写一份合并脚本。它不依赖原Web/Rust执行器，也不改变下述完整阅读、内容采用及导出要求。新增原声区间先按[原声物化](desktop-adapter.md#原声区间物化)得到本地draft；真正非人声事件按同页核源合同处理，不改source换取通过。

整集整编由与本集主创分开的新上下文读取本集完整source、计划、选择、当前关键事实及全部工作包，按[单集内交接](workspace-contract.md#单集内交接)接管后写出完整新 `draft`；同一作者续聊不算换新。整编者参与改稿，后续非作者审阅不能由其冒充。按[观看收益规则](creative/references/common-contract.md#观看收益与声画取舍)核模板复述、讲解画面和无效切镜；可用inspect_editing定位完全相同镜级字段与跨镜长句，在既有review/revisionNotes记录具体返修或有据保留，不增设评分审批。本地精修也保留基稿，输出完整新采用候选；只替换有实际问题的范围和必要依赖，未受影响单元原样沿用。零改动保留原采用版本并记“未产生改善”。

实际变化写入 `revisionNotes:[{coordinate,sourceRefs,change,preservedFunction}]`；未能安全解决的内容写入 `unresolved:[{coordinate,problem}]`。问题关闭另记录问题坐标、修后采用版本及实际回查依据，不以“已修”标签自证。必须将修好的正文、原句分片、声窗、幻想引用、入出状态及场/资产绑定一同合入新版本，v7 另同步 imageBindings 和 visualSubjectBindings，再核完整稿及预览；只有差量、问题卡或整编待办不算采用稿。

局部返修输出完整有效尾态与当前素材依赖，逐项说明保留、替换或退役；未改部分沿用，已失效项不得靠旧新列表取并集复活。比较修前修后的原声、状态和绑定，不能用只含一条反馈的局部列表覆盖整集有效出口。资料缺键与画面真实错误分开处理：对白正文已有且仅引用缺失时，核原字、说者、声道、声窗和事件唯一匹配后只补引用；多解不猜。实际伤势、左右手、接触和动作相位缺陷必须改正文并回查，不能以nonblocking、模糊遮挡或删unresolved放行。

`director-refinement`沿同一局部工具路径，仅在整编后仍有具体可改善弱点或用户明确要求时运行，不自动为每集增加一轮。工具生成的完整候选才可进入后续检查，delta不能直接交complete或bundle。既有能真实核验的交换格式继续兼容，基稿、候选、实际变化和采用依据留本地；普通局部改进不回到用户审批。

## 逐集续作与独审记录

一集经完整主创、整集整编、必要精修、自查及机械检查后，可形成**文字采用稿**。v7 采用前须走下述逐集预览门。在非作者复核尚未完成时，完成证据如实写 `reviewMode:"self"`，其 `delivery`成果记录“自查已做、独审待做”及实际范围，然后 `complete`让组内继续；这不等于独审放行。

非作者复核按[调度合同](parallel-and-continuity.md)在空闲槽新建独立上下文，记录被审采用稿版本/哈希、实际阅读范围、问题及关闭依据。无内容修改时新增独审记录即可，不重开已完成集、也不篡改旧完成证据；最终总控同时读取它与工作区状态。有内容修改则先保留旧稿，按恢复合同重开受影响集，采用新稿并重新检查真实下游依赖。

仅元数据重绑/导出外壳重编时，比较完整源、方向/选择、实际声画、已用资产设计与适用范围、图片映射、实际入口/出口；这些均未变且旧独审已闭环，可引用旧独审原件并补新旧hash与逐类无内容变化证据，无需再次整集创作审阅。原独审仍绑定旧版本，不把改hash当新独审；实际内容、身份/时点或图片用途变了便按真实关联范围复核。前尾和后集入口未变的兼容边界用既有workspace reconcile更新证据，不自动链式返修；脚本不替主控判断语义兼容。

正式全范围交付必须具备全量文字采用稿、实际跨集边界核对及非作者内容复核闭环；无子代理时主控可做两向核查并明确独审缺口，不代写创作或返修。`readyForDelivery`仅为文件证据门，不能覆盖仍未完成的独审或返修。

## v7 逐集预览与正文冻结

1. 逐单元读实际 picture、演出、分段入口/出口、持物、媒体主体与声音，核逻辑 bindings/voiceBindings；需要参考图的现实/媒体/明确视觉主体另填 imageBindings。纯声源不自动配图，瞬时 Q 版或幻想可在 visualSubjectBindings 中记 assetKey:null，不强造卡。正式可复用形态和有据设备缺项按[统一资产合同](asset-registry-contract.md)回补，收到卡后必须落实正文和实际消费。
2. 回补处理完成，用当前总库 `sync → check → export → verify` 生成本集 v7 预览及配套 `lingmo-v7-assets-v1` 目录。场三行取本场图片并集，单元只用自己的集合；30项仅计图片。命名碰撞、旧状态、跨段误带引用或时间超限在本集局部处理。
3. 从结构化采用稿到正文逐字段核：项目风格/禁止项、整段与分段演出、入口/出口、结束状态、全部镜头六项、逐字原声、kind/声道/演法/声窗不得漏。新voice-delivery-v1不自动注入原始cue/performanceCues；对照冻结源逐项核真正表演在正确说者、句内位置落实delivery/picture/sound，后期说明只入独立文件，不能把删原始提示目录当表演已保全。历史稿按原投影回查。v7 删除旧格式标签不能删除幻想或演出。内部总长可小数，外层 `max(4,ceil(实际总长))`，不通过拖演和凑整改内容。
4. 保存该版采用正文、预览当前字节、配套目录、总库不可变快照、消费回执和正文—图片核对记录到[工作区证据](workspace-contract.md)。后续仅加外层和@，冻结正文去掉新增令牌后逐字节保持；这不能代替前一步字段级检查。只保存生成计划或旧预览不算当前采用证据。
5. 已用资产、映射、实际内容发生变化时只更新真实消费者及必要下游，保留旧版，重新产生预览/目录和采用证据；无关总库新增不重写全剧。最终阶段仍须以当前总库核全剧并检查实际跨集承接。

预览通过表示本集本地格式和配套关系可核；没有本次灵墨实际上传结果，不标实导成功。白图生成不成为每集文字制作的等待条件，试导需要时或最终打包再执行。
