# 本地工作区与子代理续作合同

全部路径相对当前工作区 JOB；工作区工具使用 Python 3.9+ 标准库，显式季模式另调用Node.js 24+同源规则。它记录文件与阶段证据，不能判断创作质量、用户认可、费用或平台效果。无真实图片不阻塞文字制作。新生产采用[灵墨 v7 合同](lingmo-v7-contract.md)，逐集保存正文/预览及配套目录；正式交付仍须内容复核、实际跨集检查、格式回读和最终白图包核验。

## 前期与完整资产资料

首次派发分集前，按[前期质量合同](preproduction-contract.md)读取全剧导航、源/状态账、资产基线与当前集实际设计依赖及前期复核依据；远期独立设计按首次使用继续推进。记录复用现有文件，不为阶段名重复建表；workspace的claim/complete不会自动判断设计是否足够完整，领取成功不等于开工条件已满足。

完整设计和短卡随统一总库版本保存，设计来源变化沿真实design.dependencies检查派生卡及其消费者。实际读取的必要导航、设计和决定版本纳入本集既有intake/assets证据，不让只有文件路径而未读内容的供料伪装完成。

最终在导入包旁按[资产交接合同](asset-handoff.md)另存可独立读取的接续资料，包含初始/最终完整资产表、设计、源/方向及最终使用映射。不要把分集bundle.assets或白图目录当全库；工作区保留旧版，交接包中的引用使用可解析的相对路径。

## CLI 与源接入

```sh
python3 -X utf8 scripts/workspace.py extract --source /absolute/script.docx --output-dir /absolute/JOB
python3 -X utf8 scripts/workspace.py init --workdir /absolute/JOB --index /absolute/source-index.json --active-slots 4 --batch-size 2
python3 -X utf8 scripts/workspace.py status --workdir /absolute/JOB
python3 -X utf8 scripts/workspace.py claim --workdir /absolute/JOB --group 1 --owner author-g1-b01
python3 -X utf8 scripts/workspace.py complete --workdir /absolute/JOB --episode 1 --owner author-g1-b01 --evidence ep1/evidence.json
python3 -X utf8 scripts/workspace.py block --workdir /absolute/JOB --episode 1 --owner author-g1-b01 --evidence ep1/block.md
python3 -X utf8 scripts/workspace.py resume --workdir /absolute/JOB --episode 1 --evidence ep1/recovery.md
python3 -X utf8 scripts/workspace.py reconcile --workdir /absolute/JOB --episode 3 --evidence ep3/boundary.json
```

`extract` 只接受 txt/md/docx；保留源文件 byte 副本、SHA-256、正文与提取范围记录。TXT/MD 必须可按 UTF-8 解码，不能猜编码丢字；DOCX 按 document.xml 段落顺序提取，包含表格中的正文段落；提取采用修订后主正文视图：包含插入和移入，排除删除与移出子树；保留制表符、显式换行和不间断连字符。特殊字体符号、外接正文等无法安全还原的内容，以及页眉、页脚、批注、脚注、尾注、文本框、绘图等额外范围，必须检查提取报告并另读原件，不能将提取成功当成全稿已读。提取不自动识别对白，不按冒号切角色。已有输出目录拒绝覆盖。

完整读取正文后，由 Codex 建立 `source-index.json`：

```json
{"version":"series-source-v1","sourceHash":"源文件SHA-256","episodes":[{"number":1,"scenes":[{"id":"1-1","sourceRefs":["source-body.txt:L1-L9"]}]}]}
```

集号是唯一整数，场 ID 是本集内唯一非空字符串；数组顺序即源顺序，不按数字重排。`sourceRefs` 可附带原文坐标；不得将只有目录的索引当正文已读。无选季模式时`init --episodes 3,5`只选择这些集，仍保留原稿顺序，默认全部。选季模式使用`init --season-scope /absolute/season-scope.json`（或索引内嵌相同productionScope），先验证locked与完整源目录，再仅创建选季任务；`--episodes`只能缩小到季内批次，不能扩回后续。冻结投影保存完整readingScope、选季productionScope及scopeHash，state与后续claim/context_packet复核同一范围。详见[季范围合同](season-scope-contract.md)。init固化索引及其hash，源文件副本/索引变化将拒绝沿用旧状态；重复init拒绝，不重置成果。

接入原文与原件核对后、首个创作派单前，按[冻结源合同](data-contract.md#创作前冻结源)保存shared/source-lock_v1.json及返回hash；它覆盖本次制作集的完整原文，逐集bundle绑定同一锁。已有全剧锁可只读复用，不为冻结源提前制作后续集的逐句生产输入。锁和完整原件留独立接入区，作者只读，不能由完成稿重建。 stage3-task新任务manifest另按[供料合同](context-and-dispatch.md)绑定源锁文件；不从完成稿补造创作前核源证据。

## 分配与证据

`workers` 取1～4，表示连续集段队列数；未指定时为 `min(4, active_slots-1)`。`--active-slots` 填宿主真实活动槽数，默认4，即主控加最多3名子执行者；真实五槽可填5启用四名子执行者。也可保留四个逻辑队列、最多三名同时执行，不能把队列数当槽位数。组ID从1开始，主控不领取创作。工具只分配状态，不启动模型、清空上下文或核查代理身份；主控按[轮换合同](context-and-dispatch.md)创建真实新代理并使用其owner。每组必须完成当前集才能claim下集；已完成稿直接复用。并发claim经操作系统短锁，只能成功一次；in_flight不会被自动重派。

新JOB记录dispatch策略：默认 `--coordinator /root`、`--batch-size 2`（允许1～3）、`--input-limit 60000`。claim拒绝主控及main/root别名、同一作者跨组同时领取、超过活动作者上限、达到批次上限或已触发轮换的owner。复杂集用1集批次；审核也占宿主槽，主控须为它安排空档，工具只计算领取中的作者，不能代替宿主槽管理。

有实际上下文观察时保存JSON并调用 `python3 -X utf8 scripts/workspace.py observe --workdir /absolute/JOB --owner author-g1-b01 --evidence observations/author-g1-b01.json`。示例字段：`{"version":"series-context-observation-v1","owner":"author-g1-b01","inputTokens":null,"compactions":1,"basis":"宿主报告本执行者发生一次压缩；输入用量不可得"}`。inputTokens是本次实际输入正整数或null，compactions为非负整数，basis写依据。达到输入阈值或发生压缩便禁止该owner领取新集；本集内停止开新包由主控执行，工具不检查工作包。当前内容安全落盘后按下述单集内交接步骤处理；后来的较小观察不能清除轮换要求。不能伪造用量或改owner冒充新上下文。

新JOB的status/init/complete等默认只返回紧凑控制视图，明细留文件；需要逐项证据时加 `--full`，旧JOB也可加 `--compact`。旧的无dispatch状态保持读取兼容，不自动改写或强行套新门；续作仍由主控执行新分工和轮换规则，不因此重建或重跑已完成集。

主控将状态整理成[用户可见进度](parallel-and-continuity.md#可见进度与估时)，不能只留下工具JSON。紧凑counts按状态名计数，须结合evidenceValid/文件证据确认当前文字采用；独审闭环另读绑定当前采用稿的审阅记录。scopeCount是本次范围分母，workerCount是登记执行者数量，不自动等于当前活跃并发；实际在制与并发结合active及宿主状态核对。上述既有字段不提供自动ETA或内容质量放行。

`complete` 的证据 JSON 如下；所有成果路径必须在 JOB 内，允许四阶段共用一个成果文件，不强制拆成八份文件：

```json
{
  "version":"series-episode-evidence-v1",
  "episode":1,
  "sourceHash":"源文件SHA-256",
  "sourceIndexHash":"status返回的源索引SHA-256",
  "scenes":["1-1"],
  "artifacts":{
    "intake":["shared/source-ledger.json"],
    "assets":["shared/assets/v0001.json","ep1/asset-consumption_v1.json","ep1/preview_v1.md.assets.json"],
    "direction":["ep1/director.json","ep1/adopted-body_v1.md"],
    "delivery":["ep1/review.json","ep1/preview_v1.md"]
  },
  "adopted":"ep1/adopted.json",
  "reviewMode":"independent"
}
```

四个键分别对应接入策划、文字资产、分集制作（内含情绪/幻想/主创/整编/精修依据）、复核交付。值允许单一路径或非空路径列表，文件可复用；不能使用空文件占位。`scenes` 必须与索引本集场顺序完全一致。`reviewMode` 为 independent 或 self，如实记录，并不意味着机器认可审核结论。组内第二集起，完成证据还必须含 `entryReview: {"upstreamHash":"claim 时绑定的实际前集采用稿 hash","notes":"已读取的前集出口与本集入口核对说明"}`，并验证前集实际文件未变；空说明、错误 hash 或缺项均拒绝 complete。此说明可复用现有整集审核记录，不增设审批。工具保存证据文件和所有成果 hash，不只改完成计数；之后文件变更会使状态显示证据失效，拒绝继续复用该受影响组。采用后引用文件必须保留版本，不要原地改共享设计账；新增决定写新版本并显式返修受影响集。采用稿可以是二代导出前的结构化本集稿；最终 bundle 由导出工具处理。

以上路径为布局示例，`v0001.json` 必须换成 assets status 返回的真实不可变快照。v7 的采用正文应是结构化字段已核完整后的实际文本，预览和配套目录必须对应同一当前 bundle。逐集先完成 `sync/check/export/verify`，再把实际文件纳入既有 artifacts 列表；工作区工具不会自动运行这些命令或判读报告。采用前保存当前字节，不以未来计划生成的路径占位，也不把可重写的预览当冻结证据。仅合并结构时核冻结正文去掉新增@后的字节保持，早期字段保留另有内容核对。

独审未完成时，可将已完成自查和机械检查的单集按`reviewMode:self`登记文字采用，delivery记录真实范围及独审待办，允许组内续作。非作者复核另存绑定采用稿哈希的审阅记录；没有改正文不必重开该集，也不能改旧self证据冒称当时已独审。最终总控须同时读取独审闭环与工作区状态，详见[分集创作合同](production-contract.md)。

资产按[统一总库合同](asset-registry-contract.md)登记：新制作将实际使用的不可变 `shared/assets/v*.json`、本集bundle及 `assets.mjs sync` 消费回执纳入assets阶段证据，另保留正文—清单/图片引用内容复核及 v7 目录。不可把可变current.json放进完成证据。workspace只验证文件未变，不自动比较后来总库的新版本；主代理每次采用及最终合并须对当前总库执行sync。无关新增不重开旧集；已用项或其导入映射变化时读真实依赖后局部更新bundle、预览、目录、回执和采用证据，不能只改hash。即使无需重写镜头，新采用文件也要走本工作区既有resume/complete流程。

`block` 仅阻塞本组，证据须说明关键缺口；其他组继续。`resume` 由总控明确核查恢复报告后调用，恢复报告绑定当前源/集与已有文件；用于回收中断、解除阻塞或重新打开受损完成稿。工具保留旧记录，当前集重新 pending，相关跨集边界失效。不要在仍有代理运行时抢回任务。

同组已有其它in_flight时不得重开旧集或领取第二项；先让当前作者停在已保存的边界，并明确回收其任务后再处理。曾绑定实际前稿的边界失效后，先检查新旧出口及受影响入口：内容仍兼容则以新hash重新reconcile，需要修改则显式resume局部返修。未核的旧实际依赖不能作为继续领取后集的依据。首次跨组PRELIMINARY仍按下述规则允许继续。

## 单集内交接

用于阈值轮换或主创齐稿后换新上下文整编，不改CLI或清空旧成果：

1. 当前执行者保存完整工作包或中断候选、实际已核范围、源/设计/稿版本、入口/出口和未决坐标，随后停止本集写入。主控确认任务已结束或已停止；不能与旧作者同时写同集。
2. 主控在JOB内保存恢复报告，绑定当前源/集、旧owner、已保存文件及下一责任范围。对当前集调用现有 `resume --episode N --evidence 路径`；保留旧记录并使本集回到pending，不用半稿调用complete来腾位。
3. 创建真实新上下文，供给当前包与完整必要材料，再以新owner在原组 `claim` 同集，先核已有成果后续作或整编。只有全部工作包齐全才开始整集整编；交接次数不替代完整源和全稿阅读。
4. 最后领取者完成整集整编、必要精修、自查及既有逐集 `sync → check → export → verify` 后，按真实贡献及审核方式整理证据，以自己的owner调用complete；交接本身不构成采用条件。整编者是改稿作者，单凭更换上下文仍记self；非作者复核另按既有规则记录。若resume涉及已有采用稿，仍需核工具已失效的实际下游边界。

## 跨组边界与就绪含义

组首前集已在本次范围内但实际采用稿尚未完成时标 PRELIMINARY，可先完成本集文字并继续组内后集；前集成稿到达后变为 ACTUAL_PENDING_REVIEW。前集不在范围标 NOT_IN_SCOPE，不能无期限等待范围外成稿。组内逐集实际续作绑定前集采用稿 hash，只有 complete 的 entryReview 证据齐全才记录 ACTUAL_RECONCILED；工具不因串行调度自动声称内容已经核对。变更任何已绑定采用稿时需复核其下游依赖。

`reconcile` 只处理跨组/重新失效的边界。报告须在 JOB 内，完整记录以下匹配当前实际采用稿的 hash，并有非空核对说明：

```json
{"version":"series-boundary-v1","upstream":2,"current":3,"upstreamHash":"前集采用稿SHA-256","currentHash":"本集采用稿SHA-256","outcome":"pass","notes":"入口/出口、持物、伤势、知情、空间及幻想退出已据实际稿核对；必要局部返修另附"}
```

`status` 返回 `textComplete`（所选集均完成且文件证据未变）和 `readyForDelivery`（再加所有范围内实际边界已记录核对）；后者仅代表工作区证据就绪，不自动证明 v7 预览有效、全剧内容审已完成或白图包已核。最终还须读取当前全剧 MD/目录、白图/ZIP 配套结果及内容闭环，才能标“本地上传包准备好”。本次平台实际导入、真实图片/视频效果和用户认可分别记录。校验失败、缺项或仍有 PRELIMINARY 时不得宣称正式交付。所有失败采用非零退出码，标准错误给出原因；标准输出 JSON。

## 原稿修订后的复用

旧JOB绑定原件、提取正文和索引版本；这些文件变化后，连resume也会拒绝，不能直接改hash绕过。需要采用新版原稿时建立新JOB：extract新版、完整核差异与新集场索引，再init。旧JOB保留供回查。

按新范围逐组claim。经源差异、当前设计/决定和真实依赖复核仍有效的旧采用稿及阶段证据，可复制到新JOB的版本目录，无需重新创作；为其重建本JOB的完成证据，使用新的sourceHash/sourceIndexHash及正确场目录。逐集entryReview绑定当前实际前稿，跨组边界重新登记。不能原样复用旧evidence或只改hash就声称内容已核。受影响部分局部返修；改变前集出口后，重核实际消费它的下游。
