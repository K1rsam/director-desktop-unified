# 完整资产设计、精准供料与接续

范围/方向明确后使用，见[intake](intake.md)。只有一个当前总库，`bundle.assets`等均为投影。字段见[schema](schema.md)，采用见[workflow](workflow.md)。

## 1. 按实际需要完整设计

读源导航、状态、视觉基准与范围，按场景/角色家族批量设计和复核，不逐项另调模型。核心资产完整采用后开主创，其余正式视觉资产首次使用前达到同样标准。首季库绑定同一locked `productionScope`/`productionScopeHash`，资产含范围内 `scopeUses:[{episode,sceneId}]`；未来变体不借父依赖提前制作。

提源实体，再查拿取/交接/操作、拍摄/通信/乘坐、特写/持续物件、屏幕/历史主体及声源依赖。只列对身份、动作、空间或连续性有用的项；普通表情/姿势/短特效不机械生卡，需要独立稳定外观、复用状态或图片时才登记。源只给功能，所选设备仍属设计。

每项分源事实、采用设计、未知。无源外观仍补足完整设计，明确为制作选择，不增身份/能力/关系/剧情结果。人物/兽类落实识别、体态/物种、脸与轮廓、服装、材质配色和必要状态，保持差异；场景落实拓扑、出入口、地标、尺度、表演区和可操作细节；道具落实结构、握持操作、功能边界与状态。系统载体依已定呈现设计布局、出入和感知边界，纯声不强配图，有化身不改嘴部对白。

完整 `design.full`保留 `version/body/sourceRefs/supplements/applicability/review`，可回查源约束、补足、时点与实际复核；既有sourceFacts等不删。采用后提炼 `design.text`，不能复制短卡假装full。大附件另存版本文件与相对路径/hash，交接带走。

`design.dependencies:[{assetId,assetRevision,purpose}]`记真实继承设计版本；`parentId/familyId`只表身份家族。独立设计可无依赖，禁止自指/重复/未知/环。使用时递归核版本与采用状态；母设计变化只复核真实派生卡及消费者，不全族重做。

`design.state`用UNADOPTED/TEXT_ADOPTED/REFERENCE_ADOPTED/STALE。未读像素记NO_PIXELS_READ；文字采用、白图、像素复核、正式图与用户认可分开。普通设计依委托自主采用，仅关键源冲突/超界才询问；缺图不阻塞文字制作。

## 2. 唯一总库与低重复登记

主控只用 `scripts/assets.mjs`写 `JOB/shared/assets`，不手改current/旧快照。首次基线统一收齐已核必要设计。最小形状如下；正式采用须补真实full及短卡，不能以UNADOPTED例子冒称就绪：

```json
{"sourceHash":"原件实际SHA-256","importProfile":"lingmo-v7","assets":[{"key":"门厅SC","name":"门厅","type":"SC","status":"常态","origin":{"kind":"source","sourceRefs":["E01:R1"],"basis":"源明示地点"},"design":{"state":"UNADOPTED","text":""}}]}
```

```sh
node scripts/assets.mjs init --registry /absolute/JOB/shared/assets --input /absolute/JOB/shared/assets-baseline.json
node scripts/assets.mjs status --registry /absolute/JOB/shared/assets
node scripts/assets.mjs apply --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/asset-request.json
```

`status`只读，stdout留本地。`apply` **仅单项** `series-asset-change-v1`，每次新快照，没有批量接口。批量设计/核查后首次统一init，后续复用或仅改真实变化；不删历史、不重新init采用库、不按岗位克隆库。

总库分配稳定ID。`name/key`保留正式状态名/标点，`key=name+type`；源名留originalName，旧键留upstreamKey。语义CH/GROUP/BEAST/LOC/PROP与导出SC/CH/PR分开。必要状态不同ID，同物异名用aliases，originalName可跨状态共享。

v7的 `importName`按Python isalnum核安全字符与同类重名/文件名冲突；MD/@/PNG共用映射。`placeholderOrdinal`新增追加、修订保留，不按排序重编。仅尚无importProfile的旧库使用prepare-import（见[tools](tools.md)）；新库不重复迁移，改名走revise。

## 3. 只供相关的完整材料

主创、整编、非作者复核实际读所用正式资产的完整采用设计、源限定、时点与父依赖，短卡只定位。按已核必要清单选择版本化完整设计文件，同内容只载一次；工具不会自动判断需求是否挑齐，须读源/实际稿核闭包。全库留本地，不灌无关设计、历史快照或重复证明。

分清现实、纯提及、画外声、屏幕/回忆/主观主体。`unit.bindings`及场bindings表逻辑使用，`voiceBindings`绑定原声，`imageBindings`仅本段所需图片，不能互抄。瞬时Q版/图解可用 `visualSubjectBindings` 的 `assetKey:null`，正式可复用状态须回补；单元不照搬全场图片并集。

## 4. 补项、消费与局部修订

1. 作者报集/场/包/单元、用途、源锚、候选名/同物线索，不自分ID。已知映射直接复用；未知或冲突由主控专项核源核重：source表漏拆，design表制作补足。
2. 请求用 `version:"series-asset-change-v1"`、当前baseRevision、唯一requestId、reason、实际duplicateReview及 `action:add|revise|reuse`。add给完整asset；revise给assetId/完整新asset并保留原字段/编号；reuse只给assetId，不改该项版本。
3. 无单元的阶段二用 `stage:"asset-design",uses:[]`，选季另给scopeUses；阶段三用 `stage:"production",uses:[{episode,sceneId,unitId}]`真实位置。到料仅awaiting_consumption，作者落实画面/人物/道具/绑定与当前预览。
4. 同版消费回执核过才关闭，收到卡或加@不够。单元移位用amend/retarget，有据撤回可选设计用withdraw，保留资产，不撤销源必需物。

```sh
node scripts/assets.mjs sync --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/bundle_v2.json --source-lock /absolute/JOB/shared/source-lock_v1.json --output /absolute/JOB/ep1/asset-consumption_v2.json
node scripts/assets.mjs amend --registry /absolute/JOB/shared/assets --input /absolute/JOB/ep1/asset-amend.json
```

amend用 `version:"series-asset-amend-v1",baseRevision,requestId,action,uses,reason,sourceReview`；retarget给最终位置，withdraw的uses为空。选季sync加同一 `--season-scope`；正常四步执行见[workflow](workflow.md)，作者不重复编排。

总库与单项版本不同：无关新增不使旧稿失效，只核变化项的真实消费者。兼容则保留镜头、更新bundle/绑定/回执，否则局修。证据绑定不可变快照与当前bundle，不用current.json冒充历史。持物、接触/去向和图片用途仍须读稿双向核查。

## 5. 提前资产接续与最终白图

首批完整设计采用后即按[提前交接](asset-handoff.md)交可读资产MD/接续目录，随后继续分镜；不等全集MD或图片完成。阶段1目录只是预览，已有采用设计直接复用，不重复创作或重审全库。

白图读取同版 `lingmo-v7-assets-v1`目录，范围为imageBindings并集，纯声不虚配图：

```sh
python scripts/make-white-placeholders.py --index /absolute/JOB/asset-index_v1.json --output-dir /absolute/JOB/白图_v1
python scripts/make-white-placeholders.py --index /absolute/JOB/asset-index_v1.json --output-dir /absolute/JOB/白图_v1 --verify
```

新目录排他创建；按稳定编号生成可解码、尺寸/MD5各异的纯白PNG，核名称/数量/目录后打同版纯PNG ZIP，JSON/说明在目录外。白图只作占位，不证明真实图片或平台导入；真实图像/视频另需明确请求。

最终再补齐独立可读接续目录：初始基线、最终current/不可变快照、被引用历史版本、完整设计/附件、源/索引、方向/系统/视觉决定、最终bundle/MD/图片目录/回执、全库只读表及未完成范围。区分已用图、纯声、提及、未用/待设计；MD三行不代全库。旧绝对路径原件不改字节，清单映射包内相对路径；抽查初始/补项/变体真实链可追到源、设计、ID、用途和文件名。

生图准备按明确用途给 `series-image-request-v1`、`baseRevision`、`uses:[{assetId,assetRevision,episode,sceneId,purpose}]`，用 `assets.mjs image-queue --registry DIR --input FILE --output FILE`仅生成清单。总库定设计；early依据核过的源集场/制作决定确定用途，队列给精确文件名，无单元不造unitId；局部采用稿补镜头用途，final再用实际MD/目录核定导入范围；增量按ID/版本交接，无关旧图保留，仅复核真实衍生项，不删历史或全库重生。

只补同版设计集场用途用[record-uses](asset-usage-reuse.md)，不改assetRevision/full/dependencies，不另造逐集核重报告。原apply/reuse负责已有生产补项消费，不能以用途登记替代。
