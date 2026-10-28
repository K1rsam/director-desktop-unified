# 岗位数据接口

只保留模型实际需要写/读的结构；原文、哈希、源锁、资产ID与视觉设置由接入和主控程序准备。新生产不得借 `testFixture:true` 或历史兼容结构绕过检查。所有数字用原精度，JSON不得重复键。

## 冻结基线与作者响应

源、设置、集场和资产已冻结，作者不重抄或改造；发现具体错误只报坐标。实际对白ID/speaker/quote/channel/cue从本集source取，不猜ID。原文事实与前情直接阅读，不另写事实表。

作者响应严格只有：

```json
{"plan":{},"selection":{"selectedIds":[]},"draft":{}}
```

由 `lite_edit.py compose --derive-plan` 保留其余字段、补齐机械索引并合入；它只做组装，之后仍须完整check/export/verify和内容审阅。

## 计划

新制作的完整形状（示例ID必须换成实际源ID）：

```json
{
  "schemaVersion":"performance-core-v4-plan",
  "unitPackingPolicy":"event-first-v1",
  "emotionArc":"本集实际变化与落点",
  "events":[{
    "id":"EV1","sourceRefs":["E01:R1","E01:R2"],"sceneKey":"1-1|门厅|现实",
    "boundaryBefore":{"kind":"episode_start","reason":"本集源范围开始"},
    "emotionContract":{"subject":"甲","contextTone":"警觉","audienceEffect":"读到危险逼近","change":"等待转为提醒","landing":"提醒后仍留意门口","retain":["必要原话与持物接续"]}
  }],
  "entryFacts":[],
  "fantasyCandidates":[]
}
```

events覆盖所有剧情行、互不交错；完整原句不跨事件。`boundaryBefore.kind`为 `episode_start/continuous/new_event/time_space_jump`，首项才用episode_start，continuous不跨真实场景。作者不写workPackages/provisionalUnits；主控compose --derive-plan按已写事件和真实镜头反算兼容索引，不能当作额外统筹或提前规划证明。复杂任务若已有真实分包计划可保留完整显式结构。entryFacts每项 `{id,subject,statement,sourceRefs,evidenceMode:"source_rows"}`，只引用实际读取源/前情；计划推断不混入。emotionContract的retain非空，保护作用和必要先后，不锁画法。

候选 `{id,anchorRefs,subject,emotionPurpose,recommended,proposal,screenSecondsRange:[min,max],extraSecondsRange:[min,max]}`：源锚属本集，前区间含进出和发展，后区间只计不能共时的净增。off两数组为空；recommended按授权采用；all选全部实际候选；review仅真实已确认结果。源要求的幻想进retain和镜头，不强造候选。选中项须兑现，不能删selection迁就遗漏。

## 成稿、单元、镜头

draft 使用 `schemaVersion:"performance-core-v4",unresolved:[],fantasyRealization:[],units:[...]`；实际修订可用 `revisionNotes:[{coordinate,sourceRefs,change,preservedFunction}]`。下例只解释形状，不能照抄成真实剧本：

```json
{
  "id":"U1","title":"门前提醒","eventId":"EV1",
  "boundaryBefore":{"kind":"episode_start","reason":"本集起点"},
  "sceneId":"1-1","scene":"门厅","dayInterior":"夜 内",
  "bindings":["门厅SC","甲CH","钥匙PR"],
  "imageBindings":["门厅SC","甲CH","钥匙PR"],
  "voiceBindings":[{"lineId":"E01:R2:D1","assetKey":"甲CH"}],
  "participants":["甲"],"props":["钥匙"],"visualSubjects":[],
  "unitDirection":"本段实际演出与必要局部表现边界",
  "intent":{"subject":"甲","change":"等待转为警觉提醒","landing":"提醒后继续盯住门","retain":["持物与原话作用"]},
  "lighting":"冷白顶灯照清面部与手背",
  "initialState":"门左侧站立，右手握钥匙",
  "exitState":"钥匙仍握在右手，目光停在门缝",
  "shots":[{
    "id":"S1","duration":6.2,
    "camera":"侧前中近景随视线短移至门缝，手仍在画幅下缘",
    "picture":"目光被门缝的动静截住，握钥匙的手绷紧后停住",
    "lighting":"顶光维持手背与面部可读",
    "sound":"室内底噪持续","transition":"沿同一门缝方向接下一观看点",
    "sourceRefs":["E01:R1","E01:R2"],"contextRefs":[],
    "dialogueParts":[{"lineId":"E01:R2:D1","text":"此处必须是原句的实际逐字分片","presentation":"画内","delivery":"依真实句意和源提示写演法","voiceStart":0.4,"voiceEnd":5.8}]
  }]
}
```

单元边界额外可用 `capacity`，需真实容量/切点依据；事件内延续用continuous。initialState/exitState是实际时点，独立可用，不能写同上。日夜内外据源逐维判定，有依据的普通留白主动补清，实质未知才待核，不默认夜内。

绑定各司其职：

- bindings无重复，恰一个当前scene的SC；PR对应props；CH是现实人物、视觉/媒体主体、实际发声者的显式并集。不是整场资产全部塞给每单元。
- participants只列现实出场的正式状态名。`visualSubjects:[{name,domain:"subjective"|"prospective"}]`，另用 `visualSubjectBindings:[{name,domain,assetKey:null|string}]` 一一匹配；null只用于确无需独立人物图片的表现。仅环境或物件主观变形，不把仍正常的现实人物重复登记成同名视觉专用主体；有独立主观主体就使用明确区分的名字。新稿实际人物3D Q形态须绑定对应“带Q版”资产并进入imageBindings，不因短暂置null。历史null稿仍按原协议回查。`mediaSubjects:[{name,medium:"screen"|"photo"|"flashback",sourceRefs:[...]}]`用于源支持的媒体或历史，画面也要表达边界，不能当现实人物入场。
- voiceBindings每单元每原句一项，assetKey指本单元CH；系统音可空，不为配声音把人物加进participants/imageBindings。
- imageBindings必填且是bindings子集，最多30项；只选本段实际参考图。纯声/提及不自动加图，需要图的实际视觉主体不能漏。场三行是所属单元图片并集，单元@只自己的集合。长中文名称不默认改成裸字母代码。

人物3D Q的图片选择按独立视频片段：无Q用正常图，有Q用对应衣装的“正常＋Q同图”对照图，通常不再重复引用正常父图。主观Q的visualSubjectBindings指对照图；正常现实人物和原声身份仍按实际保留在逻辑绑定，父子资产不是两个人。源定真实变身用实际现实状态，不伪造主观域。正文明确形态使用镜头、进入/退出和同框范围；图内两形态不默认同时入画。对照资产逐级继承实际造型：默认→婚礼→婚礼带Q，父图须为当前采用的婚礼图。视频字段不重复人物衣装、鞋帽和穿戴配饰的款式/颜色/材质/饰纹；这些留资产设计，外观由实际引用图承担。原台词、正式名、抓袖口/脱衣/破甲等源定动作与衣装线索的叙事作用保留；换图若不满足源要求，只处理具体冲突。未读图不宣称已核实。

dialogueParts按原句/发声顺序拼接逐字还原，不重播跨镜前文。voiceStart/End是本镜局部秒，不能越镜；跨切连续发声要与实际声窗一致。源现场声仅 `画内/画外/画外转画内/画内转画外`；VO仅VO，OS仅OS，系统音仅系统音或画外。导出保留源speaker/channel/kind（speech/roar/scream/vocalization）；创作演法不改变原身份和原话。

原始cue/performanceCues留在source供核查；新voice-delivery-v1由作者将真实发声提示写delivery、动作写picture/sound，句内quoteOffset定位应正确，跨镜提示只落实一次。指定后期叠字另交，不能改成旁白或删源来分离。

幻想兑现最低 `{candidateId,status:"realized",unitIds,shotIds,screenSeconds,entry,exit,domain,preservedFunction}`；单位/镜号实际存在，入出和回报在正文可见。已有 `fantasy-spans-v1` 的screenSpans/timingVersion保留。不能用not_realized、空入出或删ID过门。

已有segments保留 `{title,lighting,initialState,exitState,shotIds}` 并完整同序分区，局部演出不丢。每镜duration与局部声窗最多9位有效小数，真实总长与外层 `max(4,ceil(真实总长))` 均不超项目15/30秒；3.2→外层4、29.1→30，不填空白凑整。

导出器自动完整拼接项目视觉基准、禁止项、本段演出/初末态和镜条目；模型不重写最终MD。单区最终稿只有片段资产标签内嵌@，内部审核/ID/评分留工作区。机械覆盖不证明事实、表演、空间或图片用途正确，仍须完整审编与独审。

新生产bundle根字段必填 `generationBoundaryVersion:"video-prompt-v1"`；新岗位采用检查拒绝省略。所有编辑性字卡留独立后期资料，生成正文不写后期占位或排版指令；真实场内文字、系统及原话保留。旧冻结采用不自动迁移。

`props`只列本单元正式PR资产，不是所有可见物件的名册。普通纸杯/烟头/扫帚等可仅写picture和必要初末态，不增加总库、bindings、imageBindings或白图。需要稳定逻辑ID但无需独立图者可标imageRequired:false，保留props/逻辑绑定而省略图片绑定；独立图片目录只来自imageBindings。关键识别/特殊结构/系统面板不能借此省掉应有设计，见[道具界限](prop-asset-policy.md)。
