# 便携输入与版本化单区数据合同

选季项目另遵守[季范围数据合同](season-scope-contract.md)：bundle内嵌productionScope，可附同一scopeHash；preview与final分开，最终校验选季完整覆盖和真实镜头秒数。外部--season-scope或季总库绑定存在时，不能删内嵌字段降级；原文source、原声、动作接点与完整设计约束保持。新季另建范围，不改旧文件hash来复用旧完成证据。

`node scripts/storyboard.mjs` 使用 Node.js 24+、Python 3.9+ 标准库和包内纯逻辑模块。读取 bundle 及本地统一资产总库，不连接网页、数据库、模型或生产队列。新生产根字段明确 `exportProfile:"lingmo-v7"`，完整输出规则见[灵墨 v7 合同](lingmo-v7-contract.md)；省略或 `series-single-zone-v1` 仅保留旧格式核验。所有相对命令均在本技能包根目录执行。合成格式样例可单独检查；真实生产稿必须提供当前 `--registry DIR`。

```bash
node scripts/storyboard.mjs check --input tests/fixtures/export-bundle.json
node scripts/storyboard.mjs export --input tests/fixtures/export-bundle.json --output /tmp/series-test-fixture-v1.md
node scripts/storyboard.mjs verify --input tests/fixtures/export-bundle.json --output /tmp/series-test-fixture-v1.md
node --test tests/storyboard.test.mjs
```

该样例保留旧格式回查：完全合成的闭门短篇，`testFixture:true`，共一集两场、三个镜头，包含面罩遮挡、正式状态名与原说者不同、原句跨镜拆分、小数镜窗与系统音。不是用户剧本或真实生产。重复演示需另选递增输出名；`export` 使用独占创建，拒绝覆盖任何已有文件。v7 同时生成资产目录，`verify` 只读核既有 MD 和目录；旧格式核验仅读旧 MD。CLI 成功返回0，失败返回非0并输出错误码与坐标；没有模型调用或未知费用填0的问题。

## Bundle 根与原文

新岗位产物的每集 `settings.visualBaseline` 保存已采用的完整 `series-visual-baseline-v1` 对象，`settings.visualStyle` 与其中正文逐字相同，字段见[视觉基准](visual-baseline.md)。同一bundle不得混入缺基准或不同基准的集；局部幻想按镜头限定，不改共同头。旧无基准稿只保真兼容，明确报告本项未验证。

根字段：

- `version:"series-storyboard-bundle-v1"`、`testFixture:boolean`、`title:string`、`unresolved:[]`。
- `generationTextVersion:"voice-delivery-v1"`：新生产必填，原始cue目录只供核源；演员必要提示由作者写入已有delivery/picture/sound，后期说明独立交付。
- `sourceLockHash`：创作前freeze-source返回的hash；有锁的新稿必须同时声明上述生成文本策略。
- `exportProfile:"lingmo-v7"`：新生产必填；省略或 `series-single-zone-v1` 使用旧投影，未知值拒绝。旧稿迁移另存 bundle、MD 和配套目录，不覆盖旧交付。
- `scope:[{number:1,sceneIds:["1-1","1-2"]}]`：本次用户授权的完整集场目录，由源接入阶段建立，不能从已有成稿反推。集号为唯一整数，允许0及特集；scope数组的源顺序权威，不强制数字递增，原场号在每集唯一。
- `assets:[{key:"值守者·面罩CH",name:"值守者·面罩",type:"CH",status:"面罩遮住口鼻",upstreamKind:"CHARACTER",importName:"值守者面罩",placeholderOrdinal:0}]`：v7 的 `name/key` 是内部正式状态名/逻辑键，保持单行且 `key=name+type`；原有顿号、括号等标点不因导入被改写。类型仅 CH/SC/PR。另存安全 `importName` 和持久的零基 `placeholderOrdinal`，场级三行、@ 和 PNG 只使用安全名，原声与导演正文保留原称呼。`originalName` 保存源称呼，不能合并身份或改源说者来过格式。旧协议的 name 仍受旧引用字符限制（换行、`、`、`@`、方括号及占位词“无”）；它不能代替 v7 命名映射。`status` 记录文字状态，不表示有图片。`upstreamKind`及其他上游家族、设计来源字段原样留在账本；GROUP、BEAST等若投影为CH，必须记录原类型和投影依据，不能据CH改变其语义身份。
- `episodes:[...]`：必须与scope逐集同序且一一对应。不得交付缺集、缺场、重复或违反scope源顺序的文件。

真实生产另必填 `sourceHash`（原始剧本文件hash）、`assetRegistry:{id,revision}` 及所有已用assets项的 `assetId/assetRevision`，按[统一资产合同](asset-registry-contract.md)从总库采用。key/name/type/status必须与当前同ID资产一致，文字卡须已采用；v7 还须匹配总库 `importProfile:"lingmo-v7"`、importName/placeholderOrdinal。新增无关项不使旧版本稿失效，已用项或其导入映射变化必须局部复核并生成新证据。总库可多于本集使用范围；本集assets不反向覆盖源资产总库。仅合成测试可testFixture:true，不得用它绕过真实稿资产门。

每个 episode 字段：

```json
{
  "number": 1,
  "source": {
    "key": "E01",
    "title": "合成样例",
    "sourceRows": [
      {"id":"E01:R1","text":"△门厅内，值守者戴面罩，右手握着铜钥匙。"},
      {"id":"E01:R2","text":"值守者：门还没开，先别摘下面罩。"}
    ],
    "contextRows": []
  },
  "plan": {"fantasyCandidates":[]},
  "selection": {"selectedIds":[]},
  "settings": {
    "visualStyle":"项目已采用的画风、画质、材质及画幅说明。",
    "prohibitions":"项目已采用的禁止项。",
    "maxUnitSeconds":30,
    "fantasyMode":"recommended"
  },
  "scenes": [{
    "id":"1-1", "dayInterior":"夜 内",
    "sourceRefs":["E01:R1","E01:R2"],
    "bindings":["门厅SC","值守者·面罩CH","铜钥匙PR"]
  }],
  "draft": {"schemaVersion":"performance-core-v4", "unresolved":[], "fantasyRealization":[], "units":[]}
}
```

以上仅展示导出字段形态，不是完整新剧本计划或可通过校验的成稿。新制作的events、workPackages、完整plan及选择规则见[分集创作合同](production-contract.md)；`tests/fixtures/export-bundle.json`是兼容旧结构的可运行格式样例，不能拿它的简化plan代替新制作统筹。

`source`直接交给本包`normalizeSource`。原文行须稳定唯一坐标与逐字text；辅助前情放contextRows，不能借其替代本集覆盖。可携带标准接入的`sourceProtocol/standardPackage`、`dialogue/entryFacts`等原生字段，依赖一并保留。非标准原文无法安全识别对白、孤立标点或多行归属时，必须在接入阶段补证据，不能绕过diagnostics。显式dialogue目录还会与原文重新提取结果比较，拒绝借目录删除原声。原文冲突、对白更正与用户决定先在上游保留before/after与依据；本导出器不替用户作实质改写。

从原剧本开始时，优先使用随包[现行标准接入编译器](intake-contract.md)，其输出可直接作为source；它保留备注/名单/对白/行内表演等类型及真实来源证明，避免旧冒号推断。已有受核结构source不必重做，`inspect-source`读取真实结果即可。完整intakeEvidence/receipt只作本地回查，不默认重复送给所有岗位。

构稿前先把单集source对象另存为SOURCE.json，执行 `node scripts/storyboard.mjs inspect-source --input /absolute/SOURCE.json`；stdout直接返回原版normalizeSource的完整结果，含sourceRows、dialogue、diagnostics及每句id、speaker、quote、channel、cue、kind。此命令无需Bundle或draft，不创作、不写文件，诊断项原样返回供接入补证据。自动D编号是本集整份对白目录的顺序号，不是每原文行都从D1开始：例如第一句E01:R4:D1、第二句E01:R6:D2。实际id必须从inspect结果复制，不能按行号猜；标准协议或已有显式目录以返回结果为准。对白始终输出source里的原说者、原声道、逐字文本。资产正式状态名不同于原说者时使用下面的显式voiceBindings。

## 创作前冻结源

新生产在接入核源后、任何主创派单前保存 `intake-source.json`，格式为 `{sourceHash:"原件SHA-256",episodes:[{number,source}]}`；source是受核的完整单集原文对象，不含draft。由接入方执行：

```sh
node scripts/storyboard.mjs freeze-source --input /absolute/JOB/shared/intake-source.json --output /absolute/JOB/shared/source-lock_v1.json
```

只生成独立锁文件，拒绝覆盖；返回的 `sourceLockHash` 与 `generationTextVersion:"voice-delivery-v1"` 写入所有相应新生产bundle根。锁可覆盖全剧，逐集/局部bundle只使用其中保持源序的本次集，最终合并仍使用同一锁。主创、整编、精修不修改source和锁，出现疑句回查原件；确有用户更正时保存before/after及裁决，再由接入方新建受影响版本，不能从改过的成稿反向冻结。

check/export/verify必须携带同一 `--source-lock /absolute/JOB/shared/source-lock_v1.json`。工具先比锁hash、原件hash、集范围与完整规范化source，再拼接原声逐字比对quote；改source并同步改台词仍会失败。缩写、同义改写、称呼/数字/重复词/语气词、标点、说者或声道改变不属于表演增强；声窗不足改排镜头/时长，不改原话。

未声明锁的历史bundle保留只读回查兼容，结果 `sourceFidelity.mode:"bundle-only"` 明确只核内部一致，不能报告独立核源通过；新稿不得省锁借历史兼容放行。已声明锁却缺文件/错hash时拒绝降级。锁保证与创作前接入文本一致，不证明接入文本最初就与DOCX等原件完全一致，该步仍须核源。

## 后期文字交接

人名条及指定后期制作的编辑叠字写独立 `postproduction-notes.md`，记录源坐标、集/场/单元/镜号、准确字样、样式/位置及入驻退；不并入生成MD、素材PNG目录或通用提示词/头部/初末态/六栏。源行和onscreenTexts仍留内部source；已有内部 `postproductionNotes` 可作索引，但不代替独立后期交接文件，投影不输出它。不能通过删源行、删原声或把后期文字改为旁白实现隔离。最终导出须实际读回，工具不按关键词猜语义或静默过滤正文。

## 场、单元、镜头与人声

每场`sourceRefs`按原文同序，所有sourceRows恰归一场，包括空行与元信息；内部场级`bindings`精确等于其所有单元实际逻辑 bindings 的并集。v7 输出场三行另取该场所有 imageBindings 的并集，不能把它复制给每个片段。同一原场内的时空切换可有多个SC，单元各自只绑定实际场景；该单元的`dayInterior`可显式覆盖场默认值，未给时继承场值，不能把同场内日外历史段标成当前夜内。日夜内外逐维据源，未知可为`待确认`，如`夜 待确认`，v7 保留在场景描述内。场号是非空单行原标签，不套资产名的引用字符限制；场号和场景SC绑定都显式提供，不按名字或画面文字猜测。

每个native `draft.units[]` 最低结构如下；`sceneId/bindings/voiceBindings`、v7 的 `imageBindings/visualSubjectBindings` 以及可选 `dayInterior/mediaSubjects` 为本适配器字段，其余使用本包创作核心结构。

```json
{
  "id":"U1", "title":"闭门前的等待", "sceneId":"1-1", "scene":"门厅",
  "bindings":["门厅SC","值守者·面罩CH","铜钥匙PR"],
  "imageBindings":["门厅SC","值守者·面罩CH","铜钥匙PR"],
  "voiceBindings":[{"lineId":"E01:R2:D1","assetKey":"值守者·面罩CH"}],
  "participants":["值守者·面罩"], "props":["铜钥匙"], "visualSubjects":[],
  "unitDirection":"面罩始终遮挡口鼻，以眼神和手部变化承载表演。",
  "intent":{"subject":"值守者","change":"等待转为提醒","landing":"仍克制等待","retain":["提醒功能"]},
  "lighting":"冷白顶灯。", "initialState":"门左侧站立，右手握钥匙，面罩覆盖口鼻。",
  "exitState":"钥匙与面罩状态保持。",
  "shots":[{
    "id":"S1", "duration":6,
    "camera":"固定中近景。", "picture":"眼神移向关闭的门，右手钥匙微动即停。",
    "lighting":"顶灯维持均匀受光。", "sound":"通风机低鸣。", "transition":"保持视线方向。",
    "sourceRefs":["E01:R1","E01:R2"], "contextRefs":[],
    "dialogueParts":[{
      "lineId":"E01:R2:D1", "text":"门还没开，先别摘下面罩。",
      "presentation":"画内", "delivery":"压低音量，字音清楚", "voiceStart":0.4, "voiceEnd":5.7
    }]
  }]
}
```

- `participants`仅当前现实出场的采用状态名；`visualSubjects:[{name,domain:"subjective"|"prospective"}]`保留主观/未来图示域。媒体或历史插入用`mediaSubjects:[{name,medium:"screen"|"photo"|"flashback",sourceRefs:[...]}]`：screen为屏幕/监控，photo为照片/画像，flashback为源明确历史插入。引用真实本场源锚和本单元CH，不能把媒体里的人并入现实出场或误标主观想象。同一身份可同时在现实与媒体出现，无不同造型依据不强建新身份；确有另一个可区分化身时采用明确状态名。完整历史场景也可另列单元，以源依据、scene和初末态说明时空。媒体边界须实际写进picture/初末态；字段存在不证明画面已表达。
- `bindings` 是逻辑绑定，每键一份。一个单元恰有对应`scene`的SC；PR精确对应props；CH精确对应现实人物、正式视觉实体、媒体主体、实际发声者的正式绑定并集，不使用名字推断绑定。旧协议保留最多30项的原门；v7 的30项上限只核下面的图片数组。
- v7 每单元必填 `imageBindings:[]`，最多30项、无重复、为 bindings 子集。仅本段现实/媒体/明确视觉主体的实际参考图片可选入；纯声音、纯提及不自动加图。需要图却尚未入库者先回补，无图片的正文表现不被机械判为漏项。
- v7 有 `visualSubjects` 时，另以 `visualSubjectBindings:[{name,domain,assetKey:null|string}]` 逐项对应，domain 与 native 主体一致。null 表示只在正文表现，不强建独立卡；非 null 显式指向当前采用 CH 的内部 key、纳入 bindings，需要图时才选入 imageBindings。`visualSubjects` 本身仍仅 name/domain，保留 native 校验；无主体可省略 visualSubjectBindings。
- `voiceBindings`在每个单元必填，普通人声每原句一项，跨镜同句不重复映射；assetKey须为本单元已逻辑绑定CH。系统音不要求人物图，其voiceBindings为空；不得为了声音绑定将说者加入participants或imageBindings。VO/OS同样通过明确原句绑定，不意味着说者入画；确实同时入画时按画面另选图片。
- 每镜`dialogueParts`按原句与发声顺序拆段。所有片段拼接必须逐字还原完整原句，不漏字、重复或倒序。同一句跨镜只续接新文字，不重播前文。`voiceStart/voiceEnd`为本镜局部秒数，必要小数保持；导出时仅加本镜起点换成“本条”连续原声窗。当前native合同不接受越出本镜的局部声窗，跨镜说话须分配实际逐镜文字与发声窗，不能由导出器猜切词或改原声。
- native `presentation`按源声道严格配对：现场对白仅画内、画外、画外转画内、画内转画外；VO仅VO；OS仅OS；系统音仅系统音或画外。未知源声道直接拒绝，不默认回退；现场对白不能冒充系统音。check、export与verify均执行同一入口校验，原声道不改。原生kind仅speech/roar/scream/vocalization，媒体正文分别写中文“口语/兽吼/尖叫/非词人声”，不投放内部英文分类码；新生产的原始cue逐字保留在source及源锁，生成端只取作者明确写出的delivery及六栏，不自动复制原始cue。作者、整编与非作者逐项对照提示的说者、句内位置和作用：真正发声提示落delivery，动作提示落picture/sound，后期说明落独立文件；含混项回查原件。不可只凭delivery非空声称表演保全；不推断新声型、不套自然版节奏。旧无generationTextVersion稿仍保留原声提示的原协议回查。
- 标准源的 `performanceCues` 是句中表演括注，保留原提示文字及规范化 `quoteOffset` 位置。新生产不自动投影其原文目录，由作者把真实表演按该位置写入delivery或画面/声音，后期项目独立交接；内容复核逐项回读实际落实，不能只在源JSON留一份便称表演已保全。跨镜原句只落实一次，切点处归后段、句尾提示归最后分片；原quote与声窗照常独立核。无新策略的历史v7仍按原投影回查，不静默改变冻结稿。
- v7 镜长和实际总长可为小数，外层容器为 `max(4,ceil(精确实际总长))`；镜窗、声窗最多9位小数原样保留，更多有意义精度返回局部处理。实际总长与容器都不得超 settings 的15/30秒上限；29.1→外层30、3.2→外层4、30.000000001→拒绝，不补空白或改词凑整。旧协议仅回查原4至30秒整数总长要求。所有实际剧情行必须被本场镜头sourceRefs覆盖，原声实际播放场也须与其源行归场一致；引用覆盖不证明动作语义正确。
- 新制作按[分集创作合同](production-contract.md)使用event-first-v1的`plan.events`及unit.eventId、boundaryBefore；已有受核稿保留其兼容结构，声明的policy不得任意写未知值跳过检查。若声明`unitDirectionVersion:"unit-direction-v1"`，单位演出总指令及原分段指令必须存在。
- native装箱结果的`segments`可保留；每个分段至少含title、lighting、initialState、exitState、shotIds，shotIds完整同序分区。分段演出、入口、出口与光影在对应首镜前明确输出，不删除分段承接。内部其他来源及修订记录继续保存于bundle，不塞进交付正文。

幻想候选与选择按[分集创作合同](production-contract.md)形成；settings.fantasyMode仅off/recommended/all/review，省略时按recommended兼容旧稿。off关闭额外选择与兑现，源明确幻想继续保留；all采用实际候选全体；review要求明确selectedIds数组，其存在不证明真实用户授权。普通推荐由上游按已确认规则采用。选中项必须在`draft.fantasyRealization`兑现；最低兑现记录含candidateId、status:"realized"、unitIds、shotIds、screenSeconds、entry、exit、domain、preservedFunction。已有`fantasy-spans-v1`的screenSpans/timingVersion等原样保留并由native校验。不能靠把realized改为not_realized、删selection或虚构入出来通过。兑现入出必须落实在实际picture与初末态中，候选ID、domain、preservedFunction和screenSeconds等审核记录仅留本地账，不附加为媒体正文的元指令。

系统的呈现选择与源身份按[专用合同](system-presentation.md)保存在方向工作资料，使用现有direction供料；`systemPresentation`不是本地bundle的新增原生校验字段。所选面板、化身或脑洞载体必须落实到已有画面/声音/状态字段，不能靠附加元数据宣称已经输出；原系统音及绑定规则保持。

## 导出与证据边界

v7 输出只有一个 `**【标准化剧本】**` 区：原集场→场级三行→片段编号→外层整数时长→通用提示词→三个内嵌@的资产标签→单元光影/站位→逐镜六项。每镜按机位/运镜、画面、光影、声音、台词、转场排列；日夜内外并入场景描述。无顶部引用、独占日夜行、片段正文标签或主观视觉主体标签。完整演出总指令、分段演出/入口/出口、结束状态、禁止项、原声类别/提示均有输出去向，详见[v7字段映射](lingmo-v7-contract.md)。旧协议保留旧投影，仅供回查。

intent、visualSubjects与fantasyRealization等内部语义核验账不拼成媒体审核指令；对应创作必须落实在picture、unitDirection与初末态。多行文本用两空格续行序列化，反解恢复原换行。禁止项和画风由settings明确传入，不注入自然版画风或默认禁令；原文明示文字/声音与项目禁令冲突须在上游明确解决。先核结构化字段到采用正文，再冻结正文和逐集预览当前字节，后续仅加外壳与@；不能用后一次字节相等掩盖更早的字段遗漏。

`verify`使用独立文本语法解析成集、场、片段、各镜窗与每个可见字段，逐字段比较采用稿的投影；不以重新渲染后字符串相等、嵌入JSON或hash代替回读。能发现镜头正文改字、错绑引用、时间变更、顺序或片段缺漏。v7 export 同产 `${output}.assets.json`（可用 `--assets-index FILE` 指定），verify 同时核 MD 和 `lingmo-v7-assets-v1` 目录的映射与绑定hash；该hash只用于配套核验。另对当前总库核已声明的资产ID/版本/采用及补项消费。自由文本漏写进清单、任意同物异名、来源/设计分类仍需实际读稿；结构和回执不证明这些语义。内容独审、连续性审、真实素材、平台导入与视频效果分别记录，不能把CLI成功当用户认可。全剧分集生产、实际跨集承接和最终内容放行由工作区阶段门管理，export不会读取progress后宣称内容审通过。

vendor文件的源相对路径与SHA-256见 `scripts/core/vendor-manifest.json`。13个文件是原版quality.mjs的最小静态import闭包；随包保留runtime相对布局，源文件没有改写。source-resolution依赖的文件型store仅为满足原import关系随包保留，适配器不调用其读写接口。
