# 提前资产交接

只由主控在交包时读；不进作者常驻包。复用[资产规则](assets.md)与现有工具，不增加岗位或重复审核。

- 阶段1：给必要资产目录预览，标待设计；不等于最终库，不标可生图。
- 阶段2 early：一批完整设计采用即交用户可读资产MD与接续目录，不等全库/全集分镜，交后继续制作。可先做已定身份、服装、场景母版和关键道具。
- 阶段3 incremental：只对真实新增/修订批次补交，不每镜全量重包。
- 阶段4 final：补最终采用bundle/MD、图片目录与消费回执，核实际用途和差异，不全库重画。

early仅携接收所需完整design.full/附件、当前不可变快照及被引用版本/真实父依赖、源原件与必要可读索引、范围、采用方向/系统决定和显式队列；过程报告、初始基线及未引用历史留工程，final再交齐完整归档。用户可读MD按角色/场景/道具排列，列完整设计、ID/版本/文件名及可先做、等父图、等局部稿、待补项，不重新创作。首包独立可读；增量可引用前包，路径须明确、跨电脑携齐。旧绝对路径保留原字节另给映射。可选series-asset-release-v1清单的phase、queue和材料相对路径只定位，不增加包装门。

开场首帧、站位、镜头专用机位、未采用新状态只等对应局部稿/设计；其他就绪项继续。完整设计/源依据/附件缺项只阻塞真实依赖，已有有效复核不再重审。第一阶段与最终库会有补项与状态变化，故不能承诺初始目录完全一致。

图片请求示例（值换成当前库与已核用途）：

```json
{"version":"series-image-request-v1","baseRevision":1,"uses":[{"assetId":"A0001","assetRevision":1,"episode":1,"sceneId":"1-1","purpose":"源内出场，制作已采用身份母版"}]}
```

```sh
node scripts/assets.mjs image-queue --registry /absolute/JOB/shared/assets --input /absolute/JOB/asset-request_v1.json --output /absolute/JOB/asset-release_v1/image-queue.json
```

early不需要最终MD/bundle/unitId；不要伪造单元消费。queue校验不保证full语义完整，只有短卡不够。选季绑定同一locked范围/hash；明确全剧/局部保留范围决定。父设计引用不自动生图，需生成的母版也显式入uses；已有正式父图可核版本复用，衍生图仍等APPROVED母版。纯声/提及/未用不自动入图。

增量列旧新版本、稳定ID/单项版本、设计/依赖变化、文件名、源用途或真实单元、增/改/复用/明确移出及原因。仅库revision变或新增用途不废旧图；按[用途复用](asset-usage-reuse.md)登记同版用途，无真实设计变化不另出增量包，接收方确需新用途时只传必要映射/队列；改设计只复核本项与真实派生项，兼容旧图记依据后保留。增量未列旧项不等于撤销；明确不再用只移出本次待生成范围，保留历史。最终总表覆盖全库，再与实际MD/目录对账。

资产项目用gpt-image25-asset-workflow模块22接early/incremental/final，继承方向、ID、文件名、已有授权；批次仍跑其validate-season-image-queue.mjs，--source是原件。已有完整材料无需重包。准备交接不调用图片服务；候选、APPROVED和用户认可分开。给用户可点击资产MD/目录与待补说明后继续分镜，不等图片完成。
