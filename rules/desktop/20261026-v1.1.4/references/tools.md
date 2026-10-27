# 本地工具与使用顺序

依赖：Python 3.9+和Node.js 24+，仅标准库。命令不调用模型、读取密钥或访问网络。Codex负责实际阅读和创作，工具负责提取、文件证据、调度记录及机械校验。

本桌面流程版以[第三阶段真实岗位](../stages/episode-production/SKILL.md)执行，接口见[本地适配](desktop-adapter.md)。开工前stage3-task装入规则正文与材料；新主创用materialize-dialogue取回逐字原声并按需处理源绑定非人声；整编默认apply-delta字段修订或完整关联组，由程序保全未改内容。工具均不代表采用。Node所需Python通过 `STORYBOARD_PYTHON` 或实际可用命令发现，详见[使用说明](../使用说明.md)。

新制作先按[前期质量合同](preproduction-contract.md)完成全剧导航、当前生产依赖的完整设计与复核供料；命令成功不替代这些内容工作。仅用户要求分季/总时长且实际启用季模式时按[季范围及命令](season-scope-contract.md)锁定范围：workspace init传--season-scope，分镜与资产使用同一productionScope；普通全剧和旧稿不自动截季。资产工具保留design.full与真实design.dependencies，最终按[资产交接](asset-handoff.md)保留完整资料，季模式另按需[续季对接](season-continuation.md)。生图清单仅在另有真实图片请求时生成，不是分镜交付必做步骤。

下面路径相对本Skill目录；从其它工作目录运行时用Skill的实际绝对路径替换 `scripts/...`，不要求把Skill复制进JOB。JOB应是一个新的工作目录。

## 视觉基准

新制作先按[基准工具与示例](visual-baseline.md)执行 `visual-baseline.mjs propose → adopt → validate`；只有实际已有采用依据才adopt。岗位包必带该对象，必要时 `apply` 投影到新候选settings；不覆盖旧稿。预设仅含视觉，不改声音/字幕政策。

## 接入与恢复

```sh
python3 -X utf8 scripts/workspace.py extract --source /absolute/剧本.docx --output-dir /absolute/JOB
```

完整读取 `JOB/source-body.txt`，检查extraction.json未提取范围；必要时另读原件补充证据，建立source-index.json。数据结构和后续命令见[工作区合同](workspace-contract.md)。按源索引取完整单集原文，优先使用[标准接入](intake-contract.md)：

```sh
node scripts/prepare-source.mjs --text /absolute/JOB/ep1/source-text_v1.txt --key E01 --known-speakers /absolute/JOB/shared/speaker-names_v1.json --output /absolute/JOB/ep1/source_v1.json
node scripts/storyboard.mjs inspect-source --input /absolute/JOB/ep1/source_v1.json
```

`speaker-names_v1.json`是从全文确认的原说者名称数组，不是把所有冒号前文字列成说者。标准编译保留原文、来源坐标、对白/动作/辅助资料分型及表演提示；有诊断时非零返回，不生成假就绪source。机械保真凭据不是内容审核。已有受核source可直接inspect；读取实际对白ID后再构plan/draft，避免猜编号。

```sh
python3 -X utf8 scripts/workspace.py init --workdir /absolute/JOB --index /absolute/source-index.json --active-slots 4 --batch-size 2
python3 -X utf8 scripts/workspace.py status --workdir /absolute/JOB
python3 -X utf8 scripts/workspace.py claim --workdir /absolute/JOB --group 1 --owner author-g1-b01
```

主控以实际子作者owner领取并派发，自己不领取创作。按[有界供料与轮换](context-and-dispatch.md)创建无父聊天继承的新执行者，默认最多2集后更换；子代理只写获派目录，主控核证据后以同一owner complete。active-slots填真实宿主容量，队列数不等于并发槽；观察到输入阈值或压缩时用workspace observe登记。每次恢复先status，默认读紧凑输出，需要逐项证据才加--full；in_flight不是自动重派许可，先核执行者与文件，再保存恢复证据并显式resume。

作者包至少包含完整本集source、已定方向、有效状态、相关资产和实际入口；审阅包另含完整draft及必要计划/选择。按轮换合同保存带源hash的manifest，再构建与核验：

```sh
python3 -X utf8 scripts/stage3-task.py build --workdir /absolute/JOB --manifest ep1/task-request.json --output ep1/task.json --dispatch ep1/dispatch.md
python3 -X utf8 scripts/stage3-task.py verify --workdir /absolute/JOB --task ep1/task.json
```

build与verify均成功且task/dispatch存在后再派单；失败修具体问题，不派作者重构材料或verify不存在的包。工具精确投影并装入当前岗位规则正文，不摘要、不截断；默认包与派单合计512KiB，超限先去无关历史与重复资料，必要完整材料仍超限可有据调整。author无plan同包完成统筹再写稿；finisher/refiner自动带时间线信号，不另调AI。旧context_packet的128KiB接口保留兼容，不能事后补它冒称开工供料。字节数不等于Token。原文、完整设计和长期状态仍存工作区，完整输出优先落盘按本集读取，不重复回贴主控。

共享源、决定、资产和已采用稿必须版本化。完成证据引用的文件原地变化会失效，新增设计应写新文件，再定位实际消费范围，不能为了继续claim直接改状态。

阶段一用[统一资产合同](asset-registry-contract.md)的 baseline 初始化 `JOB/shared/assets`，新生产写 `importProfile:"lingmo-v7"`。总库只由主代理通过 assets.mjs 的 init/apply/prepare-import/amend 更新，子代理交本集补项文件。已采用短卡、稳定ID、别名、版本、安全导入名、稳定白图编号和使用回执共用此库。生产稿必须绑定它，工具不会从第三阶段分镜倒造第一阶段源目录。

```sh
node scripts/assets.mjs init --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/assets-baseline.json
node scripts/assets.mjs prepare-import --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/asset-import_v1.json
```

prepare-import 仅用于尚无 importProfile 的旧库一次迁移，输入 `version:"series-asset-import-v1",baseRevision:N,importProfile:"lingmo-v7"`，可附 `names:[{id,importName}]` 解决冲突。省略项按内部 name 用 Python isalnum 清洗；空名、同类重名和文件名冲突拒绝。新快照保留单项 revision、来源与设计，并分配稳定编号；不手改 current.json，不改变内部 name/key。新 init 已建立映射时不再调用；后续改名走 apply/revise，递增单项 revision 并保留编号。

## 创作与交付

新岗位产物在采用、check/export/verify之前，先核本集成稿与真实岗位包的视觉基准一致：

```sh
python3 -X utf8 scripts/stage3-task.py verify-output --workdir /absolute/JOB --task ep1/task.json --bundle ep1/bundle_v2.json
```

通过只证明视觉基准确从该岗位包进入同集settings；后续原文、结构、独立回读与内容复核仍须完成。旧v1任务只能历史回查，不能用于新输出放行。

构造本集bundle前，先用 `node scripts/storyboard.mjs inspect-source --input /absolute/JOB/SOURCE.json` 只读取得规范化source与dialogue的真实ID、说者、原句、声道、提示和声类；SOURCE.json是单集source对象，D序号按本集对白全局顺序递增，不是每行都D1。

主创派单前按[冻结源合同](data-contract.md#创作前冻结源)，从独立受核接入source生成源锁（已有同版锁复用，不由成稿重建）：

```sh
node scripts/storyboard.mjs freeze-source --input /absolute/JOB/shared/intake-source.json --output /absolute/JOB/shared/source-lock_v1.json
```

返回sourceLockHash绑定新bundle，并声明 `generationTextVersion:"voice-delivery-v1"`；check/export/verify均传同一锁。stage3-task manifest另以sourceLock.path与锁文件实际sha256绑定供料，字段见[供料合同](context-and-dispatch.md)。后期人名条等只交独立postproduction-notes.md；原始cue完整留源，真正演法由作者落实六栏。

按四阶段子Skill在JOB内保存完整过程。本集统筹、主创、整编/精修后，把实际采用的源、计划、选择、设置、结构化稿、场绑定装入[成果bundle](data-contract.md)，新生产根字段为 `exportProfile:"lingmo-v7"`。每单元明确 imageBindings，保留逻辑 bindings/voiceBindings；纯声源不自动加图。不是让工具从原剧本自动猜分镜，也不需要人工逐栏敲JSON。

整集整编及独审读取当前稿实际切点；stage3-task已附同版信号时直接复用，否则运行：

```sh
python3 -X utf8 scripts/inspect_editing.py --input /absolute/JOB/ep1/bundle_v1.json --output /absolute/JOB/ep1/editing-signals_v1.json
```

报告的cut_pairs保留相邻两镜实际camera/picture/transition/sound、声窗与源引用，跨单元附原入口/出口及边界，segment_headers按原shotIds保留分段状态；不跨集或空单元猜配。按input_sha256绑定版本，依[切点合同](shot-handoffs.md)读实际两端后再对状态，修改后核新切点及最终导出；不新增自动评分或全剧重检。

报告保留字段重复与共同时间线，并增加源锚、同一句跨镜连接/间隙、分片时速、长镜和尾段定位；声音事件单列窗口与源锚，不计人声语速。声窗缺失时完整统计为null，只列已知窗口；负间隔表示重叠。声窗外可能是动作、读屏、环境音或有效情绪，均不自动判错或删镜。stage3-task在整编/精修开工时已带同样信号，无需再跑独立评分。整编者连读完整源与稿，实际变化写既有revisionNotes；工具没有语义判断能力，无信号也须内容复核。

每集稳定采用前检查当前v7预览，不等全剧结束；下例顺序批量执行，完整输出留磁盘、回主控只给摘要。相同源/规则/稿/相关资产未变不重复跑，修后验证真实影响范围：

```sh
node scripts/assets.mjs sync --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/bundle_v1.json --output /absolute/JOB/ep1/asset-consumption_v1.json
node scripts/storyboard.mjs check --input /absolute/JOB/ep1/bundle_v1.json --source-lock /absolute/JOB/shared/source-lock_v1.json --registry /absolute/JOB/shared/assets
node scripts/storyboard.mjs export --input /absolute/JOB/ep1/bundle_v1.json --source-lock /absolute/JOB/shared/source-lock_v1.json --registry /absolute/JOB/shared/assets --output /absolute/JOB/ep1/preview_v1.md
node scripts/storyboard.mjs verify --input /absolute/JOB/ep1/bundle_v1.json --source-lock /absolute/JOB/shared/source-lock_v1.json --registry /absolute/JOB/shared/assets --output /absolute/JOB/ep1/preview_v1.md
```

默认同时生成 `preview_v1.md.assets.json`，保存该版采用正文、预览当前字节和目录；实际读稿核字段保留及正文—图片用途，纳入采用证据。全剧合并前按内容独审和实际跨集记录关闭问题，再对当前总库执行：

```sh
node scripts/assets.mjs sync --registry /absolute/JOB/shared/assets --input /absolute/JOB/final-bundle_v1.json --output /absolute/JOB/asset-consumption_v1.json
node scripts/storyboard.mjs check --input /absolute/JOB/final-bundle_v1.json --source-lock /absolute/JOB/shared/source-lock_v1.json --registry /absolute/JOB/shared/assets
node scripts/storyboard.mjs export --input /absolute/JOB/final-bundle_v1.json --source-lock /absolute/JOB/shared/source-lock_v1.json --registry /absolute/JOB/shared/assets --output /absolute/JOB/全剧_灵墨v7_v1.md --assets-index /absolute/JOB/asset-index_v1.json
node scripts/storyboard.mjs verify --input /absolute/JOB/final-bundle_v1.json --source-lock /absolute/JOB/shared/source-lock_v1.json --registry /absolute/JOB/shared/assets --output /absolute/JOB/全剧_灵墨v7_v1.md --assets-index /absolute/JOB/asset-index_v1.json
python3 -X utf8 scripts/make-white-placeholders.py --index /absolute/JOB/asset-index_v1.json --output-dir /absolute/JOB/灵墨素材_白图_v1
python3 -X utf8 scripts/make-white-placeholders.py --index /absolute/JOB/asset-index_v1.json --output-dir /absolute/JOB/灵墨素材_白图_v1 --verify
```

export 对 MD/目录排他创建，已有文件只能 verify 或另选新版本。v7 默认资产目录为 `${output}.assets.json`，`--assets-index FILE` 可指定；verify 同时核两者，不能只搬 MD 后继续用另一版目录。镜/声窗最多9位小数保持；v7 外层只取 `max(4,ceil(实际总长))`，实际与外层都不超项目15/30秒上限。旧协议仅回查其原整数总长。verify 独立解析MD并比对可见字段，另核目录配套，不把重新渲染相同或绑定hash当内容审。

白图工具读取同一 `lingmo-v7-assets-v1` 目录，按稳定编号生成尺寸和MD5各异的白PNG；新目录排他创建，`--verify` 只读核目录中名称、数量、可解码性、尺寸和唯一MD5。零图范围允许空目录。验证后将纯PNG文件夹打成同版本ZIP，核压缩包仅包含这些图片；目录JSON、日志与导入说明保留在文件夹外。补项后交新版配套包，旧文件名/编号不因排序改变。

所有范围内内容复核和必要实际跨集核对完成后，交一份完整MD、同版本白图文件夹及ZIP。`status.readyForDelivery`只是工作区证据就绪；`check`/`verify`和白图检查只是机械边界，均不能代替读稿或用户质量验收。没有本次灵墨导入证据只能标“本地上传包准备好”。

## 本包自检样例

`tests/fixtures/export-bundle.json`是用于旧协议回查的合成testFixture，不是用户剧本或 v7 生产交付。以下只做本地测试；输出目录请另选新路径：

```sh
node scripts/storyboard.mjs check --input tests/fixtures/export-bundle.json
node scripts/storyboard.mjs export --input tests/fixtures/export-bundle.json --output /absolute/新测试目录/样例.md
node scripts/storyboard.mjs verify --input tests/fixtures/export-bundle.json --output /absolute/新测试目录/样例.md
node --test tests/storyboard.test.mjs
node --test tests/intake.test.mjs
node --test tests/asset-registry.test.mjs
node --test tests/lingmo-v7.test.mjs
python3 -X utf8 -m unittest discover -s tests -p 'test_workspace.py'
python3 -X utf8 -m unittest discover -s tests -p 'test_white_placeholders.py'
python3 -X utf8 -m unittest discover -s tests -p 'test_dispatch.py'
python3 -X utf8 -m unittest discover -s tests -p 'test_context_packet.py'
python3 -X utf8 -m unittest discover -s tests -p 'test_inspect_editing.py'
```

复制整个技能目录到另一目录后，从无关cwd执行同样命令，检查相对import和包内文档链接。技能结构验证可另用安装环境实际提供的skill-creator校验器；它不是生产运行依赖。Windows的锁和实际终端兼容性只有实测才声称通过。
