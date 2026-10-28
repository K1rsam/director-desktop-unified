# 固定分镜设定，分离制图资料

仅主控与资产设计岗位在登记、交接时读取。分集岗位只拿本集完整源、必要前情/状态、相关分镜设计及真实父依赖；不读本页、制图历史和全库图片词。

## 开稿与资产交付

一次读完整个选定范围，保存必要身份/别名、时点变化、主要造型、源明示Q形态、关键空间和道具操作导航。已有资料复用，不额外抄全剧摘要、逐项审核证明或每集资产核重报告。准备够用就开始完整主创；核心人物/已定形态的可生产设计优先交用户，其他图片资料与分镜并行。

`asset.design` 固定分镜需要的完整设定：身份和稳定识别、源定外观/状态、必要制作选择、动作/操作条件、空间结构和真实父依赖。采用后直接引用；同一造型的新表情、机位或临时持物写入镜头和连续状态，不另造资产。新衣装/Q形态/必要新场景才补项。不得用一句名字或模糊短卡冒充完整必要设计，也不为补前期表格提前创作镜头。

| 变化 | 处理与影响 |
| --- | --- |
| 相同ID/状态在新集使用 | 直接引用；需要扩展源用途时批量 `record-uses`，不重审设计 |
| 制图版式、展示灯光、生图词润色等 | `record-image-design` 批量记录，不改分镜设计/assetRevision/采用指纹 |
| 正常图→同造型正常＋3D Q同图、换装等独立状态 | 新资产ID，继承直接父设计；只补受影响片段 |
| 改变身份、源定状态、动作条件或空间结构 | `apply/revise`，只检查实际消费者和派生设计 |

开稿不等图片，也不等整库制图资料。首包只交真正可生产的项，余项标待补；中间只处理必要变化，最终按采用稿收口新增/修改/复用/未用项。源文、完整主创→整集整编→非作者独审，以及强表演、有效快速摄影、许可内幻想和适合段落的覆盖保持。

## 最小运行合同

- 新分镜必要设计标 `design.detailLevel:"storyboard"`；采用状态下须有 `design.text` 和 `design.full.body`。`text`定位、`body`承载完整必要条件，不要求逐集复制审核史。这类设计可用于分镜，不能直接生成图片队列。
- 一开始已完成可生产设计可标 `detailLevel:"image"`；未标的历史完整设计按旧合同兼容，不自动迁移、不删字段或改旧采用证据。历史短卡可被工具读取不代表语义上足够生图。
- 完整制图设计存入同库 `registry.imageDesigns`，由命令分配 `imageDesignRevision`，绑定当前 `assetRevision` 与直接父制图版本。无分离记录的旧设计以制图版本0引用。新制图设计保留所有稳定分镜约束；`compatibilityBasis`用一句具体依据说明，不另开逐项兼容证明岗位。
- 机器只验证结构、版本与依赖，不能证明新外观没有改变动作或身份。真实变化必须走 `revise`，不得利用这个命令绕过源和分镜复核。

一个家族/批次一次登记，不每项另调模型；父子顺序由工具处理。例子中的正文、ID与依据须换为当前真实内容：

```json
{
  "version":"series-image-design-v1",
  "baseRevision":12,
  "requestId":"core-art-batch-1",
  "reason":"补足本批制图资料，分镜设定保持",
  "entries":[{
    "assetId":"A0002","assetRevision":1,
    "design":{
      "state":"TEXT_ADOPTED",
      "text":"本项完整制图设计的定位短卡",
      "full":{"body":"实际完整制图文字，含所需外观、构成和展示要求"}
    },
    "compatibilityBasis":"身份、衣装状态、可抓握袖口与动作条件保持；只补展示布局"
  }]
}
```

```sh
node scripts/assets.mjs record-image-design --registry /absolute/JOB/shared/assets --input /absolute/JOB/image-design-batch.json
```

命令保留全部资产行、资产版本、分镜设计依赖、请求及历史快照，仅追加制图批次和新总库快照。拒绝旧baseRevision、非法混入字段及不就绪父设计；相同requestId和相同内容可幂等重试，已被后续版本取代不能伪装当前采用。制图补充不要求重编分镜岗位包或重做采用。父制图更新后只复核真实图像派生项，不回退分镜；实际图片采用仍由资产项目管理。

## 图片队列 v2 与接收侧交接

`image-queue`继续只接显式用途；祖先设计不自动入生成任务，纯声源不配图。全部使用旧完整设计时仍输出 `series-image-queue-v1`，原任务ID算法不变。消费分离制图记录时显式输出 `series-image-queue-v2`，不能改标签冒充v1。

v2保留v1的范围、源/快照、文件名、用途与queueHash。每个采用分离记录的item另带：

- `imageDesignRevision`：当前该资产制图版本；没有该字段的混合旧项仍按v1规则解析。
- `dependencyImages`：直接父项的assetId/assetRevision/imageDesignRevision/designHash。`dependencies`仍是展开祖先链，不能把所有祖先当直接父图。
- `design`与`designHash`：当前有效制图设计及其稳定JSON SHA-256，不能再一律用`hash(asset.design)`验证。
- `taskId`：`IMG-`加稳定JSON SHA-256前24位；新项输入为`[registry.id, scopeHash或null, itemId, filename, designHash, imageDesignRevision, dependencyImages]`。旧项仍用前5项。`itemId`与稳定文件名不因版式修改而改名；新任务ID避免误复用旧制图结果。

接收实现可复用同包 `resolveImageDesign(registry, assetId)` 解析当前有效设计，或按同合同核快照、版本、hash及直接父依赖；再比较队列中完整设计和祖先链。图片实际文件hash/采用状态继续由接收侧核，制图hash不是成图凭证。生成任务只取显式items，已有相容正式图不全库重画。

**当前本机资产项目只识别v1，本轮未修改该Skill。** 新v2接入需在资产项目更新队列/批次校验和当前设计解析后使用；在此之前可以交可读设计MD，不能声称旧校验器直接支持v2，也不能回写asset.design来绕过。新分镜制作不受此接入进度阻塞。

道具建图范围按[独立资产界限](prop-asset-policy.md)：必要动作条件仍完整保留，普通物件可直接在镜头/状态中呈现；不因功能查漏、持有人或集号不同增加图片卡。关键系统面板等需固定视觉设计的项目优先精细制作。
