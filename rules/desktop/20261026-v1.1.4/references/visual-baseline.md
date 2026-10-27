# 项目视觉基准

## 前期采用

在已有的一次方向确认中确定完整视觉基准：画风/造型、渲染/着色、材质响应、色彩/光影、画幅和局部风格例外。实际读文判断这些维度是否具体且相容；没有新增逐集审批。详细自定义说明优先保留，不能为凑术语、字数或采用预设而改写。只有“3D国漫”等标签时先提出明确的自然3D、三渲二等适配候选，不自动替用户决定。C4D/Octane不是必填词或实际渲染器证明。

预设只提供视觉候选；项目BGM、对白/字幕、叠字与其他输出决定继续沿用既有设置。三渲二候选让赛璐璐明暗主导画风，PBR区分材质；不要同时追求写实毛孔、塑料皮肤与平涂。资产设计采用同一基准，不因镜头光线变化另造项目风格。

## 岗位与输出

岗位输入 `visual-baseline` 是一份已采用的 `series-visual-baseline-v1` 完整对象。作者、整编者、审阅者实际读取其 `visualStyle`、采用依据与适用边界；原样保存到 `settings.visualBaseline`，并令 `settings.visualStyle` 与其中的 `visualStyle` 逐字相等。各集不自行概括或缩短。共同资料在工作包中可去重，每个最终独立生成片段必须完整携带，不能写“同上”或“沿用采用设计”。

通用提示词由完整视觉基准、已采用共同输出要求及本段 `unitDirection` 合成。本段演出仍保留情绪、表演、动作接触、观看重点与声光节拍；不拿共同风格头替代具体导演设计，不重复整张镜表。源保真口号、修改原因、内部评分和工作流说明留工作区；生成端确需执行的口型、私有声音、字幕/水印条件照实际决定保留。不得自动删除含“不”字的有效演出指令。

局部二维平涂、图解、Q版和主观幻想按已采用镜头明确主体、进入、退出及回接；未切换的现实部分保持项目主风格。三维Q版不因“Q版”字样变二维；基准不授予额外幻想许可，也不以统一风格删除已有有效幻想。原话、说者、因果与揭示继续受源锁和内容核查保护。

机械验证核基准已采用、内容标注可回查、各集设置一致及每段完整传递。内容复核另核视觉基准是否充分、与资产/镜表/局部幻想是否冲突。hash、维度标注与脚本PASS不证明美术质量或成片风格统一。

## 数据与本地工具

基准格式：`version:"series-visual-baseline-v1"`、`id`、正整数 `revision`、`status`、完整 `visualStyle`、`coverage`，采用后另有 `adoptionBasis`。coverage 的 `form/rendering/materials/colorLighting/aspectRatio/localExceptions` 分别引用 visualStyle 中对应的原文片段；不是另写六段互相冲突的摘要。片段存在只证可定位，不能自动评判其充分性。采用依据引用当前真实用户决定或有效委托，不以命令运行代替确认。

候选定义两种方式：

- 明确预设：`{id, preset, aspectRatio, localExceptions?}`，可附 revision；preset可用 `3D三渲二国漫`、`自然3D国漫`、`2D动漫`、`真人电影写实`，不使用含糊标签或不存在的名称。
- 自定义：`{id, visualStyle, coverage}`，可附 revision；完整保留自定义文字和其实际维度依据，不混用preset覆盖。

```sh
node scripts/visual-baseline.mjs propose --input direction/visual-request.json --output direction/visual-candidate_v1.json
node scripts/visual-baseline.mjs adopt --input direction/visual-candidate_v1.json --basis "本工程已有方向确认的实际记录/范围" --output direction/visual-baseline_v1.json
node scripts/visual-baseline.mjs validate --input direction/visual-baseline_v1.json
```

仅在已有采用依据后运行adopt。新文件拒绝覆盖旧版本。把基准以唯一 `visual-baseline` 输入供岗位包；JSON文件可整读，或用pointer精确选取对象。主控可为新候选一次投影设置：

```sh
node scripts/visual-baseline.mjs apply --baseline direction/visual-baseline_v1.json --bundle ep1/bundle_v1.json --output ep1/bundle_v2.json
```

apply仅改变各集settings的visualBaseline/visualStyle，不改声音/文字政策、unitDirection、镜表、原声、时长、编号或资产。它不自动采用新稿、更新旧收据或评美术质量；候选继续走实际供料及采用检查。同一bundle使用同一采用基准，局部例外留在该基准和具体演出范围内。

桌面新任务采用前另用 `stage3-task.py verify-output --workdir JOB --task ep1/task.json --bundle ep1/bundle_v2.json`，直接核本集成稿settings与开工包所据基准相同。基准自身合法不代表它就是该任务采用的版本；纯draft未带settings时尤其不能省略。本命令只证明该项绑定，不代替后续check/export/verify和内容复核。

## 恢复与已有稿

新岗位包必需视觉基准；旧已采用包/收据按冻结依据核验，不因Skill升级失效。旧无基准bundle保真检查时报告 `legacy-unverified`，表示未验证本项，不等于视觉质量通过；不能据此绕过新岗位开工要求。维护Skill不会批量迁移在制工程。

修已有稿需用户明确范围：先盘点采用来源与共同基准，只改授权的通用提示词字段，保留本段有效演出，字段外逐字/字节比较。原生工程同步settings及新版本派生交付，保留旧稿/回执；只有MD时标为MD专项修订，不冒称已更新原生bundle。遇设计真实冲突仅列受影响项，不顺带重写资产或生图。真实视频验证另按用户请求执行。
