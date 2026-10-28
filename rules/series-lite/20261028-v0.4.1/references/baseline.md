# 单集基线：仅主控建立时查

基线是单集 `series-storyboard-bundle-v1`：根含 `testFixture:false,title,unresolved:[],generationTextVersion:"voice-delivery-v1",generationBoundaryVersion:"video-prompt-v1",exportProfile:"lingmo-v7",sourceHash,sourceLockHash,assetRegistry:{id,revision},scope:[{number,sceneIds}],assets,episodes:[...]`。选季时保留同一 `productionScope` 和 `scopeHash`。每集含 `number,source,settings,scenes,plan,selection,draft`；创作前后三项可为空对象，源/设置/集场/资产必须已受核。

source 使用接入工具完整结果，不由作者重抄。实际对白ID、speaker、quote、channel、cue/performanceCues 从 `inspect-source` 取，不能按行号猜D编号。sourceRows含空行/元信息并保持顺序，contextRows只供前情。冻结源保护接入后文本，仍需接入时对照原件。

settings 含完整 `visualBaseline`、与其逐字相同的非空字符串 `visualStyle`、非空字符串 `prohibitions`（方向资料是数组时，主控保留全部项目并确定性拼成文字，不能直接传数组）、`maxUnitSeconds:15|30,fantasyMode:off|recommended|all|review`。scenes 含 `{id,dayInterior,sourceRefs,bindings}`；所有源行同序恰归一场，bindings为该场单元逻辑绑定的精确并集。源场号不因换SC或单元而重编号；同原场内真实换地/回忆可以另事件。

assets 项使用总库有效投影：`key,name,type:CH|SC|PR,status,assetId,assetRevision,importName,placeholderOrdinal`，`key=name+type`；上游类型、源名和设计来源保留。内部名与安全导入名分开，状态不同不能合并身份；作者不新造ID或修改库。需要修正基线归场/资产/设置，交主控核源并新建基线，不把修改塞进creative响应。


不另建分析报告；按已有源、设置与资产程序复制。岗位创作字段见[schema](schema.md)，一键准备见[tools](tools.md)。
