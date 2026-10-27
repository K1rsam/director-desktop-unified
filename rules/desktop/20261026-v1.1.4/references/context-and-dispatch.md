# 有界供料与新上下文执行

生产调度前由主控读本文件；执行者只读派单指定部分。文件是长期记忆，聊天是短期执行场所。可迁移的是3008的当次材料投影、独立执行和完整整编原则，不照搬其数据库、CLI禁用规则或进程参数。

## 子代理模型与强度

首次制作派发前落实主入口的模型选择；两个推荐为 `gpt-6.1-sol` / `high` 和 `gpt-6-astra` / `medium`，同时允许其它模型与强度的自由输入。提问工具已有原生自由输入入口时，只列两个推荐选项并保留该入口；纯文本提问则明确列“其它：自行输入模型和思考强度”。推荐、预选和等待时间均不代表用户已选定。

- 按当前宿主工具核对模型和强度是否受支持，明确显示实际模型ID；不把用户指定的 `gpt-6-sol` 等版本静默换成 `gpt-6.1-sol`。自定义信息确有歧义或缺项时只问缺项；不支持时说明限制并让用户改选，不自行替换或升降档。
- 在当前控制检查点或决定文件保存实际 `model`、`reasoning_effort`、用户选择依据及适用范围。工作区尚未建立时先保留会话决定，建立后落盘；换执行者与恢复时复用。用户改选按其明确生效范围应用于后续派发。
- 创建子代理时同时传入模型与强度。当前 `spawn_agent` 使用 `model`、`reasoning_effort` 和 `fork_turns:"none"`；全历史继承不用于模型/强度覆盖。其它宿主只使用其实际支持的参数；不能把配置文件字段 `model_reasoning_effort` 当成当前工具的参数名。
- 所选设置贯穿源接入、资产、作者、整编与独审，用户明确分角色配置时遵从该配置。只在真实派发支持所选参数时报告已设置；宿主无法覆盖时说明缺口，不默认为继承值已满足用户选择。此选择不修改总控模型或全局配置。

## 角色与生命周期

|角色|本次上下文范围|交回|
|---|---|---|
|主控|紧凑状态、共享裁决、当前问题；按需核关键源与成稿|队列推进、版本采用、审核与交付决定|
|源/资产执行者|明确划定的完整源区间或完整设计任务；共享导航与真实依赖|源索引/完整设计及精简供料；不能用短卡替完整设计|
|分集作者|一集完整源/原声、相关状态与卡、前集实际尾段；本集统筹→完整主创|全体工作包、入口/出口、选择依据与未决坐标|
|整编者|与本集主创分开的新上下文；完整源、全体工作包及相关计划/状态/设计|完整整编候选、实际改动与保全依据、检查证据；参与改稿后属于作者|
|独审者|一集完整源、完整采用稿、相关计划/幻想/状态与设计依据|坐标化问题及回查，不输出另一份逐栏复述|

作者默认最多领取2集（复杂集1集），配置范围1～3；审核默认每集新上下文。源接入/资产设计同样按完整小批任务交接，不能在同一代理内从全剧拆解一直累积到逐镜制作。量大或历史材料多时缩批，不以固定集数掩盖输入体积。模型沿已明确配置，不借优化静默降档。

使用宿主子代理工具派发，支持时明确 `fork_turns:"none"`；在消息中给当前请求、Skill与材料包绝对路径、可写范围和交回要求。不要fork全部父聊天，不附已完成几十集正文，也不要让作者按总入口再次启动全剧。结束旧执行者后创建新身份；复用的是宿主槽位，不是带历史的长期对话。interrupt/停止不等于清空上下文，换owner字符串也不等于新代理。

宿主已报告压缩，或最近一次实际输入达默认60,000 Token时，登记观察并停止追加新工作包和新集；当前工作先落盘，最迟在正在处理的完整工作包边界交接，不能因本集未完继续开下一包。已在边界则立即交接；半包中断时保存候选、已核范围与断点，由接任者核读后续完，不能为补齐边界无限延长旧上下文。不要截断源事件、丢弃已写稿或把半稿标完成。支持读取真实用量则记录 input/cache/output；不可得记null，不能用字节估计冒充Token。自动压缩可帮助恢复，却不替代有来源的交接包，也不保证清除过时决策。

整集主创齐稿后，无论是否触发阈值，都交与主创分开的新上下文整编；同一作者改角色名、续聊或自称独立不算。按[工作区交接步骤](workspace-contract.md#单集内交接)保存并停止旧任务，再让新整编者领取同集；整编者直接修完整稿，可在本次上下文完成必要精修。整编是创作工序，不能代替后续非作者独审。没有可用槽位先保留完整候选、顺序调度，不重复生产或假记已整编。

主控同样保持有界：状态命令默认紧凑输出；报告与全文落盘。按[可见进度规则](parallel-and-continuity.md#可见进度与估时)向用户报告阶段和真实完成数，进度消息不回贴整稿或内部推演。遇主控上下文恢复，只读当前控制检查点、变化范围与待办，核最近上报口径和当前成果后续报，再按需读证据；不重载全剧、完整旧聊天或全部历史审阅记录。

## 必须留下的材料

长期资产存JOB：源文件与索引、完整全剧导航、事实/知情账、完整资产设计、采用记录、实际尾态与问题。不得为了省上下文删这些成果。

派单包只选当次必要内容：

- `source`：本集完整正文、完整原声与必要提示；不能拿故事摘要代替。重复机械索引及编译回执留磁盘，按需查。
- `direction`：已确认风格、表演、画幅、容量、幻想模式及当前有效决定，指向其版本。源有系统时带本集相关系统的完整有效[呈现决定](system-presentation.md)，包含身份、源约束、所选表现、可见/可听/接触范围和版本，不能只传模式名；可用额外direction文件或精确JSON子树，不新增工具字段。
- `state`：当前有效伤势、形态、持物/数量/归属、空间及每人知情；保留SOURCE/CLAIM/DESIGN区分、发生/解除/适用范围与源坐标。只携相关当前事实，但不能因年代远就删仍有效旧伤、秘密或约定；未知不压成确定。把会约束本集行动、辨认或因果的事实显式放入此包，不只留在完整设计的回查链接中；同类实体按个体或有据群组区分，数量相同不证明位置、伤势与归属一致。
- `assets`：短卡用于定位本集依赖与版本，不作为作者已经读过完整设计的证据。主创、整集整编及非作者复核实际读取本集所用正式视觉资产的当前完整采用设计；可嵌入本次包或按精确路径/JSON Pointer读取，记录实际读取位置与版本，不要求每次工具交互重复携带。完整保留当前`design.full.body`及其有效源约束、补足、适用范围和真实依赖；变体所依赖的母设计仍可追溯。只取本集相关资产，不加载全剧全部资产、历次提案、无关集场索引或历史审核流水。必要行动限制另显式进入`state`，不能假定作者从外观设计中自行找齐；缺失完整设计时局部补料，不以短卡足够为由跳过。
- `entry`：明确选定前集采用稿的hash、相关实际尾段/末态和本集承接关系。实际稿未到写PRELIMINARY及来源，不能把预测改名actual。不能只凭一句摘要声称已核空间接触。
- 审核另带本集完整`draft`及相关`plan/selection/issues`。整集整编/独审必须看完整源和全体工作包，不能为了限额降成摘要审稿。

接任者实际读取包并核版本；矛盾或缺关键依据时回读相关原文/完整设计。有效状态由前期账、实际尾稿和新源共同核定，工具不能推断语义完整性。以既有文件投影供料，不为每角色重新撰写相同故事摘要；只更新变化事实和真实依赖。

## 文件包工具

新开工包必含唯一 `kind:"visual-baseline", purpose:"support"` 的已采用对象，精确文件/Pointer与hash写法沿现有inputs。按[基准合同](visual-baseline.md)供料，settings原样继承；无基准旧task仅历史回查，不视为新派发就绪。

齐稿后主控必须运行 `stage3-task.py verify-output --workdir JOB --task ep1/task.json --bundle ep1/bundle.json`，把完整成稿的settings与本次真实供料直接比对，再进入采用与导出。不能只分别验证两份合法但不相同的基准。

第三阶段新任务默认使用 `scripts/stage3-task.py`，在**派单前**从现有材料构包；岗位准确填写planner/author/finisher/reviewer/refiner。程序将第三阶段入口common与本岗段、相关完整创作方法正文及其hash带入任务包，派单正文给出真实执行指令和材料用途。执行者读一次包内rules与materials，不再重复读取同一规则原文件。只有build、verify均成功且task/dispatch真实存在，才派给执行者；build失败只修具体缺料/体积，不能继续verify不存在的包或让作者重做构包。相同有效包不重复构建。构包不证明模型实读或质量；已完成任务的事后冻结不能冒称开工供料。

```sh
python3 -X utf8 scripts/stage3-task.py build --workdir /absolute/JOB --manifest ep1/task-request.json --output ep1/task.json --dispatch ep1/dispatch.md
python3 -X utf8 scripts/stage3-task.py verify --workdir /absolute/JOB --task ep1/task.json
```

manifest使用 `version:"desktop-stage3-request-v1"`、episode、role、实际task、`fantasy:{mode,selected,sourceRequired}`及inputs。每个input为 `{kind,purpose,path,sha256,pointer?}`；kind沿下列材料类型，purpose为production/context/entry/nextBoundary/support。仅完整本集规范化source和完整draft用production；不可把同一投影换用途重复塞包。source必须含完整sourceRows和dialogue（原句字段quote，可空）；所有岗有source与direction，整编/精修/独审另有完整draft、plan、selection。state/assets/entry按真实相关资料提供，不制造空文件凑数。

新生产manifest另绑定 `sourceLock:{path:"shared/source-lock_v1.json",sha256:"锁文件实际SHA-256"}`，路径在JOB内。此sha256为文件字节hash，与bundle的规范化sourceLockHash区分；工具按本集production source对独立锁核源。已声明sourceLockHash的新任务及季模式任务必须提供该绑定，旧未声明任务兼容回查，不倒造旧稿的创作前冻结。source投影可去重复standardPackage/intakeEvidence，必须保留同版规范化sourceRows/dialogue/onscreenTexts、身份、standardScope及有效提示，不改权威原件或锁。任务只保存 `sourceFidelity:{mode:"locked-production-source-v1",path,sha256,sourceLockHash}` 验证凭据，独立锁不重复塞入作者materials；凭据只证明本次production投影与锁相符，不证明最初提取已忠实于原件。

已启用季模式时stage3-task与旧context_packet均复核workspace冻结范围，不将后续集塞进production。完整相关设计在主控供料时一次精确投影；重复机械接入结构、编译回执、历史审阅流水留原文件回查，不交作者再次整理。

普通author没有plan时，工具同包装入源准备与情绪统筹方法，让作者完成计划后直接写稿，不新增默认planner、二次构包或重复完整阅读。有plan时沿已有版本；selection缺失由作者按已明确权限补齐，不冒称已选择。finisher/refiner自动携带只读时间线信号，不另设AI分析岗。默认包与派单合计512KiB；超限拒写、不截断，先去无关历史/重复回执，必要完整材料仍超限可有据调整max-bytes。字节不是Token。verify可核源、规则、清单和派单漂移；材料完整性的语义和实际阅读仍由执行者负责。

### 旧包兼容

`scripts/context_packet.py`保留原调用，供已有任务冻结精确文件/JSON子树，无模型、摘要或截词。清单与输出均版本化；路径限定JOB内，原始文件hash必须匹配。旧示例形状：

```json
{"version":"series-context-request-v1","episode":1,"role":"author","inputs":[
  {"kind":"source","path":"ep1/source.json","sha256":"实际SHA256"},
  {"kind":"direction","path":"shared/direction_v1.json","sha256":"实际SHA256"},
  {"kind":"state","path":"ep1/effective-state_v1.json","sha256":"实际SHA256"},
  {"kind":"assets","path":"ep1/supply_v1.json","sha256":"实际SHA256","pointer":"/cards"},
  {"kind":"entry","path":"ep1/entry_v1.json","sha256":"实际SHA256"}
]}
```

这是旧接口示例，hash须从真实文件生成。旧role仅author/reviewer，reviewer另需draft，不能据其标签认定独审身份；新整编使用上方finisher真实岗位。kind还可为plan/selection/issues/rules。不允许history/chat/transcript材料种类；不能把旧聊天伪装source。JSON Pointer精确选取既有值，不从名称猜相关性；选择正确本集、状态与完整源仍须上游审核。

```sh
python3 -X utf8 scripts/context_packet.py build --workdir /absolute/JOB --manifest ep1/request-author_v1.json --output ep1/packet-author_v1.json
python3 -X utf8 scripts/context_packet.py verify --workdir /absolute/JOB --packet ep1/packet-author_v1.json
```

默认整包131072字节，超限列各类体积并拒写，绝不自动截断。先去无关集、重复回执和无关卡；必要完整源/稿本身仍超限时，可有据调大`--max-bytes`并记录，或按完整事件分工后仍保留整集整编。字节限制不是Token预算，不能据此承诺费用。规则可按当前角色在Skill原路径加载，避免既嵌包又重复读；包冻结不证明材料可信、已读或质量合格。

## 回执、核验与复用

交接复用当前检查点，固定只保留：当前角色与范围、源/方向/资产/基稿版本、结果路径与已核范围、实际入口/出口依据、未决坐标、下一动作；真实用量可得时附上，不可得为null。无需再生成第二份故事复述或全文回执。完整返修理由和逐镜内容留文件，主控负责协调与重点审核，具体创作返修交执行者。

确定性的sync/check/export/verify在稿稳定后顺序批量执行一次，输出只留摘要；失败返回坐标，修完仅重验相关范围。源、规则、采用稿与相关资产版本未变，不因换消息、换代理或换窗口重复整集检查。整编直接修本集内容后再导出，避免每个微小字段修改都跑全链。

质量整编遵守共同契约的[观看收益与声画取舍](creative/references/common-contract.md#观看收益与声画取舍)。重复字段与跨镜长句分析只定位待读范围，不能自动删镜、强压秒数或作为放行分数。每批解决内容问题，保真、连续性、整集整编及非作者复核仍是交付条件。

关键事实由整编者在既有review记录中对应到实际单元/镜及picture、camera或sound的成立依据；源依据与采用稿证据分别引用，设计不伪装源行。事实只需限制演法而无需特写时注明如何保持；不会影响本段的背景不强塞镜头。具体声音与构想取舍见[短例](execution-examples.md)，不新增逐项审批表。
