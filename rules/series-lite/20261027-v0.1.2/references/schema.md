# 岗位数据接口

只保留模型实际需要写/读的结构；原文、哈希、源锁、资产ID与视觉设置由接入和主控程序准备。新生产不得借 `testFixture:true` 或历史兼容结构绕过检查。所有数字用原精度，JSON不得重复键。

## 冻结基线与作者响应

基线是单集 `series-storyboard-bundle-v1`：根含 `testFixture:false,title,unresolved:[],generationTextVersion:"voice-delivery-v1",exportProfile:"lingmo-v7",sourceHash,sourceLockHash,assetRegistry:{id,revision},scope:[{number,sceneIds}],assets,episodes:[...]`。选季时保留同一 `productionScope` 和 `scopeHash`。每集含 `number,source,settings,scenes,plan,selection,draft`；创作前后三项可为空对象，源/设置/集场/资产必须已受核。

source 使用接入工具完整结果，不由作者重抄。实际对白ID、speaker、quote、channel、cue/performanceCues 从 `inspect-source` 取，不能按行号猜D编号。sourceRows含空行/元信息并保持顺序，contextRows只供前情。冻结源保护接入后文本，仍需接入时对照原件。

settings 含完整 `visualBaseline`、与其逐字相同的非空字符串 `visualStyle`、非空字符串 `prohibitions`（方向资料是数组时，主控保留全部项目并确定性拼成文字，不能直接传数组）、`maxUnitSeconds:15|30,fantasyMode:off|recommended|all|review`。scenes 含 `{id,dayInterior,sourceRefs,bindings}`；所有源行同序恰归一场，bindings为该场单元逻辑绑定的精确并集。源场号不因换SC或单元而重编号；同原场内真实换地/回忆可以另事件。

assets 项使用总库有效投影：`key,name,type:CH|SC|PR,status,assetId,assetRevision,importName,placeholderOrdinal`，`key=name+type`；上游类型、源名和设计来源保留。内部名与安全导入名分开，状态不同不能合并身份；作者不新造ID或修改库。需要修正基线归场/资产/设置，交主控核源并新建基线，不把修改塞进creative响应。

作者响应严格只有：

```json
{"plan":{},"selection":{"selectedIds":[]},"draft":{}}
```

由 `lite_edit.py compose` 保留其余字段并合入；它只做组装，之后仍须完整check/export/verify和内容审阅。

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
  "workPackages":[{
    "id":"WP1","eventId":"EV1","sourceRefs":["E01:R1","E01:R2"],
    "emotionContract":{"subject":"甲","contextTone":"警觉","audienceEffect":"读到危险逼近","change":"等待转为提醒","landing":"提醒后仍留意门口","retain":["必要原话与持物接续"]},
    "entryFactIds":[],"contextRefs":[],"dependsOn":[],"scenarioKeys":[]
  }],
  "provisionalUnits":[{"id":"PU1","packageId":"WP1","sourceRefs":["E01:R1","E01:R2"],"secondsRange":[4,8]}],
  "fantasyCandidates":[]
}
```

events覆盖所有剧情行、互不交错；完整原句不跨事件。`boundaryBefore.kind`为 `episode_start/continuous/new_event/time_space_jump`，首项才用episode_start，continuous不跨真实场景。workPackages留在所属事件，dependsOn只引用源序更早且实际需要出口的包；不是多派岗位的命令。provisionalUnits是自然声画估算，可重排，不锁视频数量。entryFacts每项 `{id,subject,statement,sourceRefs,evidenceMode:"source_rows"}`，只引用实际读取源/前情；计划推断不混入。emotionContract的retain非空，保护作用和必要先后，不锁画法。

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
- participants只列现实出场的正式状态名。`visualSubjects:[{name,domain:"subjective"|"prospective"}]`，另用 `visualSubjectBindings:[{name,domain,assetKey:null|string}]` 一一匹配；null可仅正文表达，不强建卡。`mediaSubjects:[{name,medium:"screen"|"photo"|"flashback",sourceRefs:[...]}]`用于源支持的媒体或历史，画面也要表达边界，不能当现实人物入场。
- voiceBindings每单元每原句一项，assetKey指本单元CH；系统音可空，不为配声音把人物加进participants/imageBindings。
- imageBindings必填且是bindings子集，最多30项；只选本段实际参考图。纯声/提及不自动加图，需要图的实际视觉主体不能漏。场三行是所属单元图片并集，单元@只自己的集合。长中文名称不默认改成裸字母代码。

dialogueParts按原句/发声顺序拼接逐字还原，不重播跨镜前文。voiceStart/End是本镜局部秒，不能越镜；跨切连续发声要与实际声窗一致。源现场声仅 `画内/画外/画外转画内/画内转画外`；VO仅VO，OS仅OS，系统音仅系统音或画外。导出保留源speaker/channel/kind（speech/roar/scream/vocalization）；创作演法不改变原身份和原话。

原始cue/performanceCues留在source供核查；新voice-delivery-v1由作者将真实发声提示写delivery、动作写picture/sound，句内quoteOffset定位应正确，跨镜提示只落实一次。指定后期叠字另交，不能改成旁白或删源来分离。

幻想兑现最低 `{candidateId,status:"realized",unitIds,shotIds,screenSeconds,entry,exit,domain,preservedFunction}`；单位/镜号实际存在，入出和回报在正文可见。已有 `fantasy-spans-v1` 的screenSpans/timingVersion保留。不能用not_realized、空入出或删ID过门。

已有segments保留 `{title,lighting,initialState,exitState,shotIds}` 并完整同序分区，局部演出不丢。每镜duration与局部声窗最多9位有效小数，真实总长与外层 `max(4,ceil(真实总长))` 均不超项目15/30秒；3.2→外层4、29.1→30，不填空白凑整。

导出器自动完整拼接项目视觉基准、禁止项、本段演出/初末态和镜条目；模型不重写最终MD。单区最终稿只有片段资产标签内嵌@，内部审核/ID/评分留工作区。机械覆盖不证明事实、表演、空间或图片用途正确，仍须完整审编与独审。
