# 单集原文保真分型入口

使用本包携带的 `prepareStandardEpisode` 与编译器，将**单集完整原文**分成结构、正文、辅助说明和空白来源。它不是分镜创作器，也不授予语义通过或用户认可；不能把冒号前的任意标头当角色。

```sh
node scripts/prepare-source.mjs --text /absolute/JOB/ep1/source.txt --key E01 --known-speakers /absolute/JOB/known-speakers.json --output /absolute/JOB/ep1/source_v1.json
```

路径相对当前工作目录；从其它目录调用时使用 Skill 内脚本的实际绝对路径。运行仅需 Node.js 24+、标准库与随包 `.mjs`，不需要 TypeScript、npm或外部服务；命令入口按真实路径识别，从 `/tmp` 或符号链接调用不会静默跳过。

## 输入、输出及阻塞

- `--text`：UTF-8 的本集全文，包含集场标题、人物名单、说明、空白行、表格接入后正文及作者材料；不按已写分镜倒推输入。保留 CRLF/LF、原标点和原始字词；非法 UTF-8 拒绝，不替换为乱码。
- `--key`：本集稳定非空标识，例如 `E01`；不是对白编号规则。
- `--known-speakers`：JSON字符串数组，例如 `["守门人","阿青","阿白","众人"]`，可为 `[]`。只填据源确认的真实说者，不为压掉诊断虚构人物；文本中的明确人物名单也按原生规则作为来源证据。
- `--output`：新文件，独占创建；拒绝覆盖已有采用文件。输出对象本身可直接放入 bundle 的 `episode.source`，不是另一种待人工转录的模板。

实际调用参数固定为 `syntax:'current', postColonCue:true, inlinePerformance:true, inlinePerformanceVersion:'inline-performance-v2'`。明确的Markdown集场标题、全角空格/NBSP缩进标题由本地包装层标为STRUCTURE，使用原生plan/compile接口重新编译，保留原文与UTF16位置；这些映射另记structureMappings，不把标题编成动作。一个key只接收单集，出现多个集标题时要求按源索引拆回完整单集，不能将多集合为一集；对白引号内的集名仍是原话。

先 prepare/compile，再调用包内 `standardPackageSource`，最后 `normalizeSource`；任一步有 diagnostics、没有完整 compiled.package 或没有 BODY，均非零退出、不写 source。stdout 始终给JSON状态；失败列出诊断 code、原物理行 line 和原因，stderr附短错误。原始输入文件保留，不能删未知行、擅补说者或把未知标头改成对白来过关。

成功保留：`sourceProtocol:'standard-episode-v1'`、完整 `standardPackage`、typed `sourceRows`、规范化 `dialogue`/`onscreenTexts` 与相关 `contextRows`。连续性/情绪备注等进入有来源的辅助说明，人物名单保留结构属性，不当作需要朗读的对白；明确字幕不混入人声。OS/VO/系统音、同一物理行的有据多说者、群体声和受支持的多行对白按现行规则保留。行内演出提示从 spoken quote 分离，保存 `performanceCues` 的文字/标记坐标及插入位置，不能删提示或朗读提示代替处理。

源码根据实际特性生成V1～V5合同；含新版行内表演的样例是V5，普通单句无需伪装V5。真实 source ID、对白 ID、每节点 `:D` 编号及字幕 ID 都由结果/`storyboard.mjs inspect-source`取得；新标准与旧来源协议的编号不同，不按行数或全剧对白累计数猜。

## 来源证据与下游供给

输出的 `intakeEvidence` 保留完整 `sourceText`、该单集原始UTF-8文本的 `sourceSha256`、原生 `sourceVersion`、实际选项、plan、逐物理行分型 lines、结构标题适配structureMappings、原编译 receipt，以及 `semantic:'NOT_RUN'`。`standardPackage.evidence` 按UTF16坐标完整覆盖原文，compile回执绑定原文版本/计划摘要/包摘要；原文证据、正文节点或来源版本被篡改时包内core拒绝使用。

这里的单集文本 SHA-256 与 `workspace.py` 的全剧原始输入文件 SHA-256 是不同层级：完成证据的 `sourceHash` 仍用工作区固化值，不能拿单集文本hash替换。`intakeEvidence.sourceSha256` 是额外来源记录，消费正文的规范验证由 `standardPackage` 及包内core执行；不要把可编辑的单独hash字段当验收。

机械回执只证明原文文本、UTF16范围、无遗漏归属和字段映射，不证明分型语义、画面动作、创作质量、真实视听或用户验收。必须阅读受支持的分型是否符合本次原文；含糊的续行、未闭引号、未知资料标头、混合字幕等原生不能安全处理的输入，保留具体diagnostics并集中核缺项，不猜修。

完整证据只保存在本地源文件供回查。下游岗位常规只读 `sourceRows`、`dialogue`、本次相关 annotations/contextRows/entryFacts 和所需设计，不把全文 plan、receipt、全部坐标与重复原文默认灌给每个岗位；原始证明不删除。

## 转换来源和验证

生成文件、原TS SHA-256、生成物 SHA-256、Node24版本及两项import适配见 [intake/provenance.json](../scripts/intake/provenance.json)。只迁移两份现行纯逻辑，既有core vendor文件未改写。开发时使用 Node24 的 `node:module.stripTypeScriptTypes(source,{mode:'transform'})`，转换构造函数参数属性；仅将 `../runtime/inline-performance.mjs` 改到包内 `../core/runtime/inline-performance.mjs`，将 `./standard-script` 改为本地 `./standard-script.mjs`。生成物运行不调用转换器。

如需从获授权的新源码重生成，开发命令为 `node24 scripts/intake/generate.mjs --source-dir /absolute/original-project`。这里的 `node24` 指实际安装的 Node24 可执行文件；不是要求用户安装同名命令。已实测Node24.19.0；本机Node26移除了transform模式，因此开发生成器明确要求Node24，已生成的CLI也实测可在Node26运行。不要误把开发生成步骤当作生产运行依赖。

```sh
node --test tests/intake.test.mjs
```

测试包含备注/名单、OS/VO/系统音、行内表演、多行/群体声、字幕、全物理行回执、来源版本/原文篡改、诊断不写文件、独占输出、CRLF/emoji的UTF16坐标、异地与符号链接入口，以及原文prepare→两场bundle check/export/verify贯通。样例均是合成fixture，不启动用户全剧生产。
