# 季终收口与续季对接

本文件适用于 `series-season-scope-v1` 季模式。完整阅读全剧用于理解因果；分镜派发、资产设计、生图清单与导出只生产已锁定本季。首版按原剧本的**完整原集尾**收季，不支持在半集或镜中截断。120–150分钟是实际镜长目标，135分钟是规划中心；计划估时不能当作实际成片时长。

## 两次留档

选定本季范围后，先生成 `planned` 计划包，保存源文件、提取正文、全剧集场索引、锁定季范围及下一未制作原集。它只供规划，不含实际尾态，不能用来宣布闭季或直接启动续季。

本季分镜全部完成并核源采用后，再生成独立的 `closed` 实际收口包，保留计划包。收口包必须包含：

- 本季完整采用 bundle、同一冻结原文 `source-lock.json`、采用总库的不可变快照、从该 bundle 导出的完整采用分镜 MD。
- 尾态 JSON：实际最后原集/场/单元/镜坐标；人物位置与情绪、各人已知信息、持物、伤势、原生未兑现钩子、延后事项及未决冲突。事实写源锚，实现场面以采用稿为准。没有人物或某状态为空时写明依据，不凭下一季需要倒写上季。
- 明确的文字采用依据；图像另列 `candidate_pending_user` 或 `adopted` 及采用依据。候选图完整、自动检查通过或哈希一致，都不能自动成为用户采用。

实际闭季要求所选原集场全覆盖且均有实际镜头，累计逐镜秒数。时长越界会拒绝；仅在季范围内已有明确 `durationException` 授权时允许按真实秒数收口，并报告例外。不得补空镜、改词、缩节奏或虚填总时长凑范围。文件与结构校验不替代钩子、表演、空间或实际衔接的内容审阅。

## 对接目录与命令

将需交接的文件**复制**到一个独立目录，引用统一使用目录内相对路径。保留源稿、旧目录和不可变快照。工具不迁移项目、覆盖旧包、生产分镜或生图。

计划请求 `planned-request.json`：

```json
{
  "version": "series-season-handoff-v1",
  "kind": "planned",
  "source": {
    "original": {"path": "source-original.docx"},
    "body": {"path": "source-body.txt"},
    "index": {"path": "source-index.json"}
  },
  "scope": {"path": "season-scope.json"}
}
```

`source.index` 是全剧原始索引，`episodes` 必须完整对应季范围的全部 `readingScope`，不能用只剩本季的投影索引替代，也不能带 `readingScope`/`scopeHash` 投影凭据。完整原始索引可仅内嵌合法的本季 `productionScope`；续季校验会从已核验的索引构造去除旧季绑定的只读全剧视图，不修改索引原文件。季范围的 `sourceHash` 对应源文件原件哈希；正文另存自己的哈希。工具检查绑定关系，原件提取是否忠实仍由接入核源负责。

```sh
node scripts/season-handoff.mjs create --input /项目/季1对接/planned-request.json --root /项目/季1对接 --output /项目/季1对接/planned-handoff.json
node scripts/season-handoff.mjs verify --input /项目/季1对接/planned-handoff.json --root /项目/季1对接
```

闭季请求沿用相同 `source` 和 `scope`，将 `kind` 改为 `closed`，增加：

```json
{
  "closed": {
    "bundle": {"path": "adopted-bundle.json"},
    "sourceLock": {"path": "source-lock.json"},
    "registry": {"path": "assets-v000012.json"},
    "adoptedStoryboard": {"path": "采用分镜.md"},
    "continuity": {"path": "actual-continuity.json"},
    "adoption": {"kind": "user", "basis": "填写本会话中实际采用的裁决；不可预填或自认"},
    "images": []
  }
}
```

已有明确委托时可用 `kind: "delegated"`，依据指向该委托及实际审阅；该字段不创造授权。尚未生图时 `images: []` 合法。已有图像按实际状态列出：

```json
{
  "assetId": "A0001",
  "assetRevision": 3,
  "state": "candidate_pending_user",
  "file": {"path": "images/CH-角色.png"}
}
```

采用图使用 `state: "adopted"` 并增加 `adoption: {"kind":"user","basis":"实际采用依据"}`；候选图不能附采用依据。图像必须对应本季实际资产 ID/版本；工具仅核引用和文件哈希，不检查像素或替用户挑图。多个独立服装/形态按各自稳定资产 ID 列出。

尾态文件的最小结构：

```json
{
  "version": "series-season-continuity-v1",
  "scopeHash": "填写当前季规范化哈希",
  "tail": {"episode": 40, "sceneId": "40-3", "unitId": "U17", "shotId": "S52"},
  "states": [{
    "assetId": "A0001", "assetRevision": 3,
    "position": "填写实际尾稿中的位置、朝向、动作停点",
    "emotion": "填写实际带往下一段的情绪及强度",
    "knowledge": ["该角色此时已知的事实；不能提前知道后续揭示"],
    "heldProps": ["A0012"],
    "injuries": [],
    "sourceRefs": ["E40:R18"]
  }],
  "openHooks": [{"text": "源文已有、尚未兑现的悬念及下一季承接位置", "sourceRefs": ["E40:R18"]}],
  "deferred": ["后续才使用的场景、服装变体与道具不进入本季生产清单"],
  "unresolved": [],
  "review": {"kind": "delegated", "basis": "填写实际核源和采用尾稿复核依据"}
}
```

尾态 `sourceRefs` 使用采用 bundle 中已有的原文行 ID。`heldProps` 使用已知 PR 资产稳定 ID；空数组表示经核对无此项。若 `states` 为空，必须另写 `stateBasis`，说明无人末场及此前人物状态所在位置。尚未消除的来源、承接或采用冲突写入 `unresolved`，先修复，不能签 `closed`。

```sh
node scripts/season-handoff.mjs create --input /项目/季1对接/closed-request.json --root /项目/季1对接 --output /项目/季1对接/closed-handoff.json
node scripts/season-handoff.mjs verify --input /项目/季1对接/closed-handoff.json --root /项目/季1对接
```

生成包记录各文件 SHA-256、规范化 `scopeHash`、实际末镜与末单元 `exitState`、稳定资产 ID/版本、下一未制作原集。修改源稿、采用稿、尾态或设计快照后，旧包校验会失败，应保留旧包并重新核源收口。`closed` 只表示这些声明和引用通过检查，不证明用户真实接受或视频制作已完成。

## 同一完整源稿制作下一季

1. 等待用户另行发起续季。先回读并校验上季 `closed-handoff.json`、采用尾稿和实际尾态；不能以计划季终代替实际收口。
2. 从包中 `resume.nextEpisode` 的第一原场接起，不重做上季。数字集号不必连续，按原始索引顺序找下一集。若为 `null`，旧源已结束，不能猜出下一集。
3. 轻量阅读余下剧情，提出新季候选与时长区间，锁定新的连续完整原集范围。使用新季 ID、递增 `seasonNumber`，把上季 `resume` 原样作为新 `productionScope.continuation`；新季继续执行四个生产范围门。
4. 核对实际接点：同场同时间延续写清动作出点/入点、站位、持物、情绪和声画接续；换场/时间跳跃按真实新入口设置。新季中的角色只能继承此刻已知信息。
5. 稳定身份、设计卡和用户已采用图先核版本再复用。候选图仍为候选。上季伤势、服装和持物有改变时登记新版本/状态，不覆盖旧卡，不因母资产复用就自动生出未来变体。

可单独只读回查连接：

```sh
node scripts/season-handoff.mjs verify-next --input /项目/季1对接/closed-handoff.json --root /项目/季1对接 --next-scope /项目/季2/season-scope.json
```

该命令只检查同源、上季闭合文件、季号和精确游标；不启动续季。生产入口还会强制校验：季号2及以后的新工作区，必须先将上季闭季包**连同全部引用文件**复制到新JOB目录中，保留相对路径，以 `--previous-handoff` 传入闭季JSON。JSON的父目录就是对接包根，不能只复制孤立JSON或引用外部旧目录。

```sh
python3 scripts/workspace.py init --workdir /项目/季2/JOB --index /项目/全剧原始索引.json --season-scope /项目/季2/season-scope.json --previous-handoff /项目/季2/JOB/previous-season/closed-handoff.json
```

新JOB按标准流程先完成同源提取接入。初始化及此后每次读取生产状态，都核验上季包、引用文件、闭季与续季游标；冻结包内路径及哈希，变更后拒绝继续。单独跑过 `verify-next` 不能替代必需的 `--previous-handoff`。各季使用独立工作区与新范围，不修改已冻结的上季状态。

当前工具未提供旧资产总库自动转绑或跨源迁移入口：复用旧卡/图须作为只读来源，按下一季实际用途核源登记并保留 `旧总库ID+稳定资产ID+版本` 的继承映射；若现有总库范围绑定不支持直接消费，先维护明确的继承入口，不能手改 JSON 强行解锁。分镜出口仍须校验新季当前资产绑定。

## 后续追加了新剧本

保留原源文件、提取正文、索引、源锁和上季包。追加稿是新输入版本，产生新原件/正文哈希、新接入索引和源锁；不能改旧 `sourceHash`，不能覆盖原件或重编号旧资产。

进入新季规划前做一次有记录的来源过渡核对：

- 新旧提取正文使用同一提取方法与编码。若声明“只追加”，旧正文必须是新正文的完整字节前缀；新集场索引的旧部分也须保持集号、场号、源映射顺序一致，逐条核对旧原声和事实没有改变。
- 若无法保持前缀（例如交付只有新集文件），保留独立的新集原件，明确标注其前驱为哪一个上季 `scopeHash` 和实际末镜；核对第一新集与上季实际状态。不得假装它与旧源拥有同一哈希。
- 旧文任意改词、删场、回溯改设定或编号冲突，都属于来源冲突；先列差异并作最小裁决，不能按“续季”静默覆盖。
- 资产按旧稳定身份和版本建立继承映射，区分可复用设计、已采用图、候选图和本季新增状态。旧来源绑定不等于已获新源生产授权。

当前 `verify-next` **明确拒绝不同 `sourceHash` 的自动接续**。上述跨源过渡需要先完成独立接入/继承核对，再采用可验证的新季入口；本补丁不自动迁移旧库，也不自动拼接、重写或重新生产上季。
