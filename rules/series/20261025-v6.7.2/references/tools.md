# 本地工具与使用顺序

依赖：Python 3.9+和Node.js 24+，仅标准库。命令不调用模型、读取密钥或访问网络。Codex负责实际阅读和创作，工具负责提取、文件证据、调度记录及机械校验。

新制作先按[前期质量合同](preproduction-contract.md)完成全剧导航、当前生产范围完整设计与复核供料；命令成功不替代这些内容工作。有总时长/分季目标时先读[首季范围及实际命令](season-scope-contract.md)，在详细资产拆解前冻结范围；workspace init带--season-scope，storyboard check/export/verify和assets sync传同一范围，assets image-queue只从实际用途生成生图清单。最终另按[资产交接合同](asset-handoff.md)保留全库资料及[续季对接](season-continuation.md)，不能把v7导出图片目录当完整资产表。

下面路径相对本Skill目录；从其它工作目录运行时用Skill的实际绝对路径替换 `scripts/...`，不要求把Skill复制进JOB。JOB应是一个新的工作目录。

## 接入与恢复

```sh
python3 scripts/workspace.py extract --source /absolute/剧本.docx --output-dir /absolute/JOB
```

完整读取 `JOB/source-body.txt`，检查extraction.json未提取范围；必要时另读原件补充证据，建立source-index.json。数据结构和后续命令见[工作区合同](workspace-contract.md)。按源索引取完整单集原文，优先使用[标准接入](intake-contract.md)：

```sh
node scripts/prepare-source.mjs --text /absolute/JOB/ep1/source-text_v1.txt --key E01 --known-speakers /absolute/JOB/shared/speaker-names_v1.json --output /absolute/JOB/ep1/source_v1.json
node scripts/storyboard.mjs inspect-source --input /absolute/JOB/ep1/source_v1.json
```

`speaker-names_v1.json`是从全文确认的原说者名称数组，不是把所有冒号前文字列成说者。标准编译保留原文、来源坐标、对白/动作/辅助资料分型及表演提示；有诊断时非零返回，不生成假就绪source。机械保真凭据不是内容审核。已有受核source可直接inspect；读取实际对白ID后再构plan/draft，避免猜编号。

```sh
python3 scripts/workspace.py init --workdir /absolute/JOB --index /absolute/source-index.json --active-slots 4 --batch-size 2
python3 scripts/workspace.py status --workdir /absolute/JOB
python3 scripts/workspace.py claim --workdir /absolute/JOB --group 1 --owner author-g1-b01
```

主控以实际子作者owner领取并派发，自己不领取创作。按[有界供料与轮换](context-and-dispatch.md)创建无父聊天继承的新执行者，默认最多2集后更换；子代理只写获派目录，主控核证据后以同一owner complete。active-slots填真实宿主容量，队列数不等于并发槽；观察到输入阈值或压缩时用workspace observe登记。每次恢复先status，默认读紧凑输出，需要逐项证据才加--full；in_flight不是自动重派许可，先核执行者与文件，再保存恢复证据并显式resume。

v6.7新单集默认用[job_packet](job-packet.md)和[episode_execution](episode-execution.md)编译有效岗位包、正常handoff并产生同版采用收据。作者只写完整候选，主控负责这些命令。下面context_packet v1是旧工程兼容用法，不需同时制作两套包。

作者包至少包含完整本集source、已定方向、有效状态、相关资产和实际入口；审阅包另含完整draft及必要计划/选择。按轮换合同保存带源hash的manifest，再构建与核验：

```sh
python3 scripts/context_packet.py build --workdir /absolute/JOB --manifest ep1/request-author_v1.json --output ep1/packet-author_v1.json
python3 scripts/context_packet.py verify --workdir /absolute/JOB --packet ep1/packet-author_v1.json
```

工具只精确投影，不摘要、不截断；默认整包128KiB，超限先去无关与重复资料，必要完整材料仍超限可有据调整。字节数不等于Token。原文、完整设计和长期状态仍存工作区，不为省上下文删除。inspect-source等完整输出优先落盘按本集读取，不重复回贴主控。

共享源、决定、资产和已采用稿必须版本化。完成证据引用的文件原地变化会失效，新增设计应写新文件，再定位实际消费范围，不能为了继续claim直接改状态。

阶段一用[统一资产合同](asset-registry-contract.md)的 baseline 初始化 `JOB/shared/assets`，新生产写 `importProfile:"lingmo-v7"`。总库只由主代理通过 assets.mjs 的 init/apply/prepare-import/amend 更新，子代理交本集补项文件。已采用短卡、稳定ID、别名、版本、安全导入名、稳定白图编号和使用回执共用此库。生产稿必须绑定它，工具不会从第三阶段分镜倒造第一阶段源目录。

```sh
node scripts/assets.mjs init --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/assets-baseline.json
node scripts/assets.mjs prepare-import --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/asset-import_v1.json
```

prepare-import 仅用于尚无 importProfile 的旧库一次迁移，输入 `version:"series-asset-import-v1",baseRevision:N,importProfile:"lingmo-v7"`，可附 `names:[{id,importName}]` 解决冲突。省略项按内部 name 用 Python isalnum 清洗；空名、同类重名和文件名冲突拒绝。新快照保留单项 revision、来源与设计，并分配稳定编号；不手改 current.json，不改变内部 name/key。新 init 已建立映射时不再调用；后续改名走 apply/revise，递增单项 revision 并保留编号。

## 创作与交付

构造本集bundle前，先用 `node scripts/storyboard.mjs inspect-source --input /absolute/JOB/SOURCE.json` 只读取得规范化source与dialogue的真实ID、说者、原句、声道、提示和声类；SOURCE.json是单集source对象，D序号按本集对白全局顺序递增，不是每行都D1。

按[创作前冻结源](data-contract.md#创作前冻结源)先生成全剧source-lock，返回sourceLockHash绑定本集及最终bundle，并声明generationTextVersion:"voice-delivery-v1"；check/export/verify均传入该锁。锁由接入方在创作前保存，下游只读；不能从成稿反向建锁或改source配改词。后期人名条单独写postproduction-notes.md，不放进生成正文。

按四阶段子Skill在JOB内保存完整过程。本集统筹、主创、整编/精修后，把实际采用的源、计划、选择、设置、结构化稿、场绑定装入[成果bundle](data-contract.md)，新生产根字段为 `exportProfile:"lingmo-v7"`。每单元明确 imageBindings，保留逻辑 bindings/voiceBindings；纯声源不自动加图。不是让工具从原剧本自动猜分镜，也不需要人工逐栏敲JSON。

整集整编及内容复核对当前保存稿取实际切点，并定位模板与长句跨镜信号；同一input_sha256的报告可复用：

```sh
python3 scripts/inspect_editing.py --input /absolute/JOB/ep1/bundle_v1.json --output /absolute/JOB/ep1/editing-signals_v1.json
```

也可直接传入含units的draft。v3报告的cut_pairs按源稿顺序保留每对相邻镜的实际画面、机位、转场、声音、声窗及源引用，跨单元附原始入口/出口和边界资料；segment_headers按原shotIds保留各镜所属分段的原始状态，合并单元内部切点也不丢分段上下文。不跨集或空单元猜配。报告不补写动作出点、入点或去向，也不判语义PASS。整编/复核按[实际操作](shot-handoffs.md)读两端、联修当前镜文与必要声窗，再核新稿和最终导出；不能只阅读计数或初末态。

报告保留逐镜lighting/transition完全相同至少3次及同句原声跨至少3镜的坐标，并附共同时间线：原声窗并集、声窗外总量、逐句声窗/历时、跨单元相邻原句间隔与最长5处声窗外区间。声窗缺失时完整统计为null，只列已知窗口；负间隔表示声窗重叠。声窗外可能是动作、读屏、环境音和有效情绪，真实换场也可能落在其中，均不自动判错或删镜。整编者连读完整源与稿，按实际作用处理重复串行，在既有review/revisionNotes记组织变化与保全依据。工具没有语义判断能力，无信号也须内容复核。

每集稳定采用前检查当前v7预览，不等全剧结束；新单集使用episode_execution.py run自动执行并记录下列四步，失败从已有候选恢复。下例保留给旧工程及全剧最终导出，完整输出留磁盘、回主控只给摘要。相同源/规则/稿/相关资产未变不重复跑，修后验证真实影响范围：

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
python3 scripts/make-white-placeholders.py --index /absolute/JOB/asset-index_v1.json --output-dir /absolute/JOB/灵墨素材_白图_v1
python3 scripts/make-white-placeholders.py --index /absolute/JOB/asset-index_v1.json --output-dir /absolute/JOB/灵墨素材_白图_v1 --verify
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
python3 -m unittest discover -s tests -p 'test_workspace.py'
python3 -m unittest discover -s tests -p 'test_white_placeholders.py'
python3 -m unittest discover -s tests -p 'test_dispatch.py'
python3 -m unittest discover -s tests -p 'test_context_packet.py'
python3 -m unittest discover -s tests -p 'test_inspect_editing.py'
```

复制整个技能目录到另一目录后，从无关cwd执行同样命令，检查相对import和包内文档链接。技能结构验证可另用安装环境实际提供的skill-creator校验器；它不是生产运行依赖。Windows的锁和实际终端兼容性只有实测才声称通过。
