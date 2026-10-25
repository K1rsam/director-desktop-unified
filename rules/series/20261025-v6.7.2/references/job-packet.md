# 岗位供料编译

新制作优先用 `scripts/job_packet.py`，把主控已审定的本集依赖清单一次编译成 author / finisher / reviewer 岗位包。作者读包内实际材料和有效规则，直接完成获派内容；主控执行交接、工具检查与采用，不让各岗位重新走主入口。已有 `context_packet.py` v1 包保持原接口，不要求迁移历史采用稿。

## 编译与派发

主控先实际核对本集完整源、方向、时点事实、所有必要正式资产的完整采用设计及继承设计、实际入口。把已有的审核记录作为 `review` 引用，不新增一轮评分或用户审批。已审定的单集依赖索引可直接生成下列清单；工具算 hash、装岗位规则并去重，不替主控猜必要资料。

```json
{
  "version": "series-job-request-v1",
  "episode": 1,
  "role": "finisher",
  "review": {"path": "episodes/001/dependency-review.md"},
  "inputs": [
    {"kind": "source", "path": "episodes/001/source.json"},
    {"kind": "direction", "path": "direction/confirmed.json"},
    {"kind": "state", "path": "episodes/001/facts.json"},
    {"kind": "assets", "path": "episodes/001/full-designs.json"},
    {"kind": "entry", "path": "episodes/001/entry.json"},
    {"kind": "plan", "path": "episodes/001/plan-v1.json"},
    {"kind": "selection", "path": "episodes/001/selection-v1.json"},
    {"kind": "draft", "path": "episodes/001/bundle-v1.json", "pointer": "/episodes/0/draft"}
  ],
  "methods": ["fantasy-expression-methods"],
  "contracts": ["system-presentation"]
}
```

示例路径及 JSON Pointer 只说明形状，按实际工程调整。所有资料路径必须在 JOB 内；不能用绝对路径、`..` 或越界符号链接。无 `pointer` 时保留完整 UTF-8 文本，包括换行和尾空格；有 `pointer` 时严格解析 JSON，保留所选完整值及精确小数，不保留 JSON 排版。工具不摘要、不补写、不截断内容。

```text
python scripts/job_packet.py build --workdir JOB --manifest episodes/001/finisher-request.json --output episodes/001/finisher-packet-v1.json
python scripts/job_packet.py verify --workdir JOB --packet episodes/001/finisher-packet-v1.json
```

`build` 拒绝覆盖旧文件。默认整个包最多 524288 字节，超限只报错；先去掉重复或无关依赖，完整必要资料确实更大时可显式提高 `--max-bytes`。字节上限不是模型 token 上限，也不是每集应尽量填满的预算。

岗位读取顶层 `task`；沿 `bindings`、`review` 和 `rules` 的 `blob` 键读取 `blobs[key].value`。同一正文只保存一次，不同种类、原文件路径和 Pointer 全部保留；不能把多条来源因内容相同而删掉。主控派发时指定本次输出路径及完整范围，不另附长聊天或重复整套 Skill。

## 角色与规则

- 所有岗位都必需 `source/direction/state/assets/entry`；每类可多项，空资产范围也以实际空库或说明提供，不用缺字段暗示。
- author 自动带源上下文准备和情绪统筹岗位能力，可随包给 `plan/selection`；未提供时在该次完整主创内完成统筹和选择，遵守既有模式与授权。不重写已经核准的完整源和设计。
- finisher / reviewer 必需 `draft/plan/selection`。整编读本集完整源与全部工作包后直接交完整候选；审阅读完整源与稿并给具体问题，不能由参与过改稿者冒充非作者独审。
- `issues` 是可选实际问题资料；不接受 `history/chat/transcript` 输入类别。
- 常驻表演、摄影、声音、光影四张方法卡；`methods` 可按实际需要增加 `fantasy-expression-methods`、`director-refinement`，也可显式列常驻方法而不会重复装载。幻想方法被装入不构成使用授权。
- `contracts` 可追加 `system-presentation`、`asset-registry-contract`、`shot-handoffs`；整编及审阅已自动带切点合同。发现实际还缺的资料由主控补依赖，不把默认卡表当作永久禁止其它相关方法。

规则从当前安装的 Skill 读取，不由主控复制旧提示词。共同创作契约、岗位和方法卡保留完整创作规则；production 合同只投影完整计划、幻想许可与完整改稿三节，data 合同投影源/成果结构和后期文字，v7 合同投影创作范围、绑定、正文、精确时间。reviewer 从实际独审 Skill 投影内容双向复核节。主控机械执行、白图打包及全季交付步骤不整篇送给作者。

共同契约、整编及按需精修卡中已知的内部整数终点要求，由确定性适配器准确替换成 v7 精确小数时间线和外层取整规则；源准备卡旧“上游可选、缺则自行设计”的入口条件替换为本包已审定必要输入，缺真实依赖交主控补齐。其它创作文字保留。每条规则记录原路径、原 SHA-256、实际调整和有效正文。原有条款发生无法匹配的变化时停止编译，维护适配器后再用，避免静默漏改或误删艺术要求。

## 版本、采用与恢复

新派发及新采用用 `verify` / `load_verified`：重读清单、原资料、审核记录、当前有效规则与 Skill `VERSION`，重新编译并比较整个冻结包。必要资料、计划、选择、规则或版本变动均拒绝旧包用于新任务；不读取的文件变化不影响它。已有工作区另核冻结源/索引及实际授权范围，范围外集不能派发；其它集的正常领取/完成不单独使本包失效。

已经采用的历史稿及其收据使用 `load_frozen`。它核包的严格结构、自校验、输入指纹、嵌入正文的内容 hash、原清单及实际 JOB 输入/审核记录和当前范围，不再依赖今天安装的 Skill 文件或 `VERSION`。升级 Skill 不会单独废掉历史采用稿；改源、计划、选择、采用依赖或包字节仍须重核。历史有效不表示可以拿旧包直接开启新派发。

JSON Pointer 同时冻结原文件 hash：同一文件的其它键变化也会使包失效。共享总库不断新增时，应引用已审核的本集必要完整设计及版本化依赖快照，不能把整个可变总库当每集必读输入。新包可指向新的快照；不能改旧文件来绕过旧版证据。

采用工具可直接调用：

```python
from job_packet import load_verified
packet = load_verified(job_root, "episodes/001/finisher-packet-v1.json")
fingerprint = packet["inputFingerprint"]

# 复核已经采用的历史收据：先核收据钉住的整个 packet 文件 SHA-256。
from job_packet import load_frozen
historical_packet = load_frozen(job_root, "episodes/001/finisher-packet-v1.json")
```

返回 `series-job-packet-v1` 字典。`bindings` 每项有 `kind/path/sha256/blob`，选择 JSON 时另有 `pointer`；`rules` 同样有 `path/sha256/blob` 与 `adjustments`。`blobs` 以内容 hash 为键保存 `{format,value}`。`inputFingerprint` 冻结输入来源、规则、审核记录、范围、Skill 版本及编译器；`packetSha256` 校核包结构。采用收据绑定本次 verified 指纹，不能用旧包加新计划或新选择冒充同一版。

包内保留有效规则正文及适配前规则文件 hash，避免再次装入整篇原始规则。`load_frozen` 能独立重算嵌入有效正文的 blob hash，原文件 hash 作为冻结来源记录保留。自校验不构成签名；收据必须同时固定整个包文件的 SHA-256，不能在复核时接受重写正文后重新计算的一套自校验值。

这些检查证明资料身份及本地一致性，不证明审核记录真实充分、资料选择完整、源动作落实、模型已经阅读或作品质量达标。完整创作、整编、非作者读稿和实际平台接收仍按各自证据确认。
