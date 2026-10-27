# 本地适配与工具边界

本包保留共享标准接入、资产总库、bundle、灵墨导出、独立回读、白图和工作区工具；桌面层提供开工岗位包、原声物化与声音事件、时间线定位、局部差量合并。v1.1.1另吸收独立源锁、实际切点及按需季范围工具，使用见[工具入口](tools.md)。全部离线且不调用模型。岗位执行规则在[第三阶段入口](../stages/episode-production/SKILL.md)，工具不从剧本自动猜分镜。

## 协议分层

| 桌面实现 | 本 Skill 的落地 |
| --- | --- |
| Rust锁定源、绝对UTF16区间、DTO | `prepare-source.mjs`保存标准源；本地物化按规范化quote的相对UTF16区间 |
| owner范围、资产用途、依赖出口 | `stage3-task.py`开工前投影材料和真实岗位规则正文；底层context_packet核文件/hash/子树，语义适用性仍读源核 |
| 差量整编及候选采用 | `apply-delta.mjs`支持既有字段小修与完整单元分区；程序保全未改正文，完整新稿再核验 |
| 桌面整秒视频单元、seedance导出 | 本包`lingmo-v7`，内部小数原样、外层向上取整；不重抄Rust运输协议 |
| 数据库事务、缓存、运行预算、媒体恢复 | 本包工作区/hash/单写者工作法；不宣称实现同等级事务或零重发 |

已有桌面 JSON、SQLite 或自由格式旧分镜**不能直接喂本包 exporter**。先保留原件，核原集场/源/采用状态，再逐字段映射为[本地bundle](data-contract.md)。用户只要求转换时不重创作；缺失的真实来源或资产映射不能伪造。当前 Skill 不提供通用桌面数据库导入器。

`bindings`为逻辑范围，`voiceBindings`为逐原句声源，`imageBindings`仅本段真正图像参考。共同说者原句只发声一次；原始联合标签与各成员身份分别保留，不按人数复制台词。当前便携voiceBindings为每句单一键，需要联合声源时可用有明确成员的GROUP声源记录并保留成员身份，纯声源不配图；不能把不同人物合并成同一身份。

v1.1支持源绑定的 `presentation:"声音事件"`，见下方合同。源保留原text、speaker、channel与区间；不是改源分型、删除台词或仅换枚举。旁白、OS、系统语音及其它实际发声不能借事件吞掉原话。

## 开工岗位包

按[供料与派发](context-and-dispatch.md#文件包工具)构建并核验stage3-task。包内有执行规则正文、实际材料用途与真实文件hash；派单必须在创作前到达执行者。author无plan时在同一包内完成统筹和主创；finisher/refiner附带inspect_editing时间线信号。构包成功不等于已阅读或语义合格，不能事后补包冒领前置供料。

## 原声区间物化

新制作先在创作前冻结接入源锁（全剧/本季锁可供同源单集复用），bundle绑定 `sourceLockHash`、`generationTextVersion:"voice-delivery-v1"`，check/export/verify提供同一锁。物化收据只证明对输入source取回原句，不能替代独立源锁。

先用标准接入生成完整 `source.json`，从实际目录取 ID，不能猜。

```sh
node scripts/materialize-dialogue.mjs inspect --source /absolute/JOB/ep1/source.json
node scripts/materialize-dialogue.mjs build --source /absolute/JOB/ep1/source.json --input /absolute/JOB/ep1/author-selection_v1.json --output /absolute/JOB/ep1/materialized_v1.json
node scripts/materialize-dialogue.mjs verify --source /absolute/JOB/ep1/source.json --input /absolute/JOB/ep1/materialized_v1.json
```

`inspect`给真实源文件SHA256、`lineId/quote/utf16Length/speaker/channel/kind/performanceCues`。作者输入：

```json
{
  "version":"desktop-dialogue-selection-v1",
  "coordinateProfile":"normalized-quote-utf16-v1",
  "sourceSha256":"inspect返回的当前source文件SHA256",
  "scopeLineIds":["实际原句ID"],
  "draft":{
    "schemaVersion":"performance-core-v4",
    "units":[{
      "id":"实际单元ID",
      "shots":[{
        "id":"实际镜头ID",
        "dialogueParts":[{
          "lineId":"实际原句ID",
          "quoteStart":0,"quoteEnd":3,
          "presentation":"画内","delivery":"当前实际演法",
          "voiceStart":0,"voiceEnd":1.1
        }]
      }]
    }]
  }
}
```

上例只说明工具接口，不是完整可导出稿。实际`draft`含数据合同规定的完整六栏/单元字段。`quoteStart/quoteEnd`以该`quote`字符串为起点，左闭右开、UTF16代码单元；必须在原句范围内，不能切开emoji代理对。不能混入Rust全文绝对坐标或Python字符索引。括号演法已由标准源分出`performanceCues`，不得重新计入朗读。

`scopeLineIds`须按当前源目录顺序，表示这个完整责任包的全部原句；省略时表示整集全部原句。局部声明不证明责任范围已经完整，主控仍对照plan/sourceRefs；整集采用前还须以全句范围核验和运行bundle检查。

作者只选择区间，不回传`text/speaker/channel`。每句从0连续覆盖到末尾且只一次，可跨镜/单元，不能跳字、重复或乱序。工具按当前source逐字生成本地`dialogueParts.text`；说者、声道和声型由原source供exporter读取；源演法按说者和句内位置由作者落实到 `delivery/picture/sound`，新生成模式不自动注入原始cue或后期说明，不能在作者片段里覆盖。原声窗和演法原样保留，是否自然/越界由下游完整检查及读稿判断。

输出为`desktop-materialized-draft-v1`，含`draft`和`receipt`。将其中`draft`放入本集bundle；receipt与原选择留工作区，不进媒体正文。receipt绑定源hash、责任原句、区间和生成稿hash。`verify`重新核源与区间、原话覆盖及稿hash；不判断动作语义或语速。

修改原声切点/镜头后重新物化，不能只手改receipt。旧稿已逐字受核可保持，不要求倒造选择输入或重生成。

## 源绑定声音事件

仅当源 `channel:VO`、`kind:speech` 的原句实际描述敲击/环境等非人声时，作者核源后才作此解释。VO标签本身不能判音效；仍需朗读的旁白继续VO。现场对白、OS、系统音、兽吼、尖叫、非词人声不能转事件。工具只核源绑定和覆盖，不能替作者判断语义。

inspect返回逐句sourceRefs、quoteSha256与sourceBindingSha256。将实际依据存入draft，不改source：

```json
{"soundEventDecisions":{"version":"source-bound-sound-events-v1","items":[{
  "lineId":"实际原句ID","sourceRefs":["实际源行ID"],
  "quoteSha256":"inspect实际返回值","sourceBindingSha256":"inspect实际返回值",
  "interpretation":"non_speech_sound","reason":"本句描述何种非人声及核源依据"
}]}}
```

同一句所有片段均用presentation:"声音事件"，选择输入仍只写quoteStart/quoteEnd、实际delivery和voiceStart/voiceEnd，不手填text/sourceQuoteRange。物化后保留原quote片段为text，并派生sourceQuoteRange；完整连续覆盖一次且源序不变。语义理由和声窗不是删改原句的许可。

事件无需CH/voiceBindings，不占人声朗读时速，可与人声同镜同期；仍须有本镜内有效发生窗。灵墨声音栏追加“0.2–2.2秒｜非人声：实际演绎”；事件独镜的台词为“无”，因为原描述不应被朗读。追溯text、reason和hash留结构化稿，不进媒体正文。build/verify、bundle check、export与独立verify共用此合同，事件要求显式lingmo-v7。原有无事件bundle继续兼容。

## 差量合并

小范围既有字段修改为默认路径，base可为完整draft或完整bundle：

```json
{"version":"desktop-field-patch-v1","baseHash":"原base文件实际字节SHA256","episode":59,
 "patches":[{"path":"/units/0/shots/0/picture","before":"完整旧值","after":"完整新值"}],
 "revisionNotes":"实际问题、修订与保全功能","unresolved":[]}
```

bundle必须显式指定唯一episode；单draft不得填episode。path相对所选draft，before必须与基稿完全一致且不可重复。支持既有unit的title/unitDirection/scene/lighting/initialState/exitState，shot的camera/picture/lighting/sound/transition/duration，dialogue part的delivery/voiceStart/voiceEnd；不允许改源文text、ID、源锚、presentation、绑定或数组结构。关联的原句/幻想邻单元由程序复制保全，模型不重抄；真要拆合镜头用下方完整单元差量。零实改返回原值，不造新创作版本。数值和语义仍由完整后续核验判断，不能因合并成功认为自然可拍。

```sh
node scripts/apply-delta.mjs --base /absolute/JOB/ep1/draft_v1.json --delta /absolute/JOB/ep1/finish-delta_v2.json --output /absolute/JOB/ep1/draft-candidate_v2.json
```

同一CLI接受字段patch或下列完整单元差量；完整单元差量的base仍为draft：

```json
{
  "version":"desktop-draft-delta-v1",
  "baseDraftHash":"原base文件实际字节SHA256",
  "retainedUnitIds":["U1"],
  "replacements":[{"unitIds":["U2","U3"],"units":[{"id":"U2R","shots":[]}]}],
  "revisionNotes":"实际问题、修改及保留依据",
  "unresolved":[]
}
```

这里只演示合并接口，实际新单元必须完整；空镜数组不能作为可导出稿。每个旧单元只能保留或替换一次，替换组按旧稿连续源序；未改单元由程序逐值复制，新单元/镜头ID唯一。跨单元同一句原话及幻想关联组完整闭合；触及幻想时提供完整`fantasyRealization`，保持候选ID集合并保留未影响记录，不能借删账逃过关联组。

本工具不改变plan/selection，也不自动添加新幻想。获准补漏用[完整版本候选流程](production-contract.md#额外幻想补漏的本地执行)，之后再核全部源/选择/正文。差量工具的成功只表示局部合并约束成立，未检查所有sourceRefs/台词声窗/资产/演出，输出仍是候选。核完整稿后才接资产sync及bundle check/export/verify、内容复核和真实采用。

构包、物化与合并均排他创建新输出、拒绝覆盖。恢复先verify已有结果或按原基稿/输入核它，不因为文件存在就另起全稿。
