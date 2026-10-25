# 首季范围：先定季终，再投入制作

本合同适用于用户提出总时长、分季或先做一季的项目。本轮目标为 **120–150分钟，135分钟作为规划中心**。全文阅读范围与生产范围必须分开；已明确要求全稿到底、旧无分季模式的项目按原范围执行，不自动截季、迁移或重跑。

## 前置位置与选点

阶段1先轻量通读完整源稿，建立原集场索引、主线和事实/揭示导航；**在全量详细资产拆解、完整设计、分镜派发和生图准备前**选季终。后半部只保存阅读导航及必要的延后事项，不逐项展开完整资产设计、逐句生产source或分镜。原件仍保留到EOF，不删后半部冒充全文。

主控提出2–3个有出处的完整原集尾候选（没有足够合适点时如实说明），每个包含：原集/场/源行、本季主冲突兑现、原生悬念、累计时长区间及依据、相比前一候选新增的场景/人物/服装/形态/道具负担。不能只按“有反转”等关键词判断钩子，不能虚构后续剧情。优先在区间内完成主要阶段回报并留下明确后续期待。

估时拆自然语速下的原声、必要表演/动作、观看与转场；同期动作与原声按共同时间线估算，不能重复相加。不得用“每集平均几分钟”或总字数直接确定集数，不为时长改台词、加水或催读。估时写区间和不确定性，**不是成片承诺**。首批真实分镜完成后以及接近季终前，使用实际镜头秒数替换已完成部分，重新估剩余；偏离时停在未派发边界，提出邻近原生终点或明确偏离，不静默扩季。

候选是proposed。用户选定，或已明确委托主控按该目标决策且保存真实授权依据后，才写locked。估时区间与120–150分钟完全不相交时，保持proposed并说明源不足或调整终点；只有针对该偏离的明确durationException才可锁定，不能先设计/生图再发现必停。只有时长目标、AI自填confirmation或格式PASS不等于具体范围获得批准。源hash、集场、季号或结尾改变，创建新revision/hash，并复核实际消费者；不能直接覆盖旧JOB、旧采用稿或已冻结请求。

## 一个范围，四个消费者

`productionScope.version = "series-season-scope-v1"`，保存完整`readingScope`和连续整集`episodes`。首季从源序第一集开始；数组顺序是源序，不能按数字大小重排。**首版不支持半集/场内截断**：候选须以所选最后原集的最后原场结束，不自动向外取整。

最小例子（源hash替换为原件实际SHA-256）：

```json
{
  "version":"series-season-scope-v1","id":"S01","revision":1,"seasonNumber":1,"status":"locked",
  "sourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "readingScope":[{"number":1,"sceneIds":["1-1"]},{"number":2,"sceneIds":["2-1"]},{"number":3,"sceneIds":["3-1"]}],
  "episodes":[{"number":1,"sceneIds":["1-1"]},{"number":2,"sceneIds":["2-1"]}],
  "target":{"minSeconds":7200,"maxSeconds":9000,"planningSeconds":8100},
  "ending":{"episode":2,"sceneId":"2-1","sourceRef":"source-body.txt:L40-L47","closure":"填写源支持的阶段回报","hook":"填写源支持的未解期待","estimate":{"minSeconds":7500,"maxSeconds":8500,"basis":"填写逐段原声与动作估时依据"}},
  "confirmation":{"kind":"user","basis":"填写真实选择记录，示例不构成授权"}
}
```

| 消费者 | 必须执行的限制 |
| --- | --- |
| 分镜派发与供料 | workspace先投影索引，只创建选季任务；claim和context_packet复核冻结范围。不把后续集放进author/reviewer包。 |
| 资产设计 | 总库绑定相同scope；每项独立资产含明确`scopeUses:[{episode,sceneId}]`。未来换装/形态/道具即使同一人物也不能借身份依赖混入。补项和修订同样核用途。 |
| 生图清单 | 从选季内明确实际用途生成`series-image-queue-v1`，绑定资产ID/设计版本/filename/范围hash；生图Skill派批前验证任务清单。不可扫描全剧或整个总库补齐图片。 |
| 导出 | bundle内嵌同scope；外部scope/季库不允许删掉内嵌字段降级。check/export/verify和v7图片目录只容纳选季内集场。 |

已有设计、父设计依赖或历史图片的保留/只读复用不等于本次新设计或生图授权。当前季所需资产的完整设计与实际供料标准不缩水。未来资产仅列延后导航；不要提前建立其正式设计和生图任务。

## 工具入口

路径相对Skill；在其它目录运行时改成Skill实际绝对路径。只校验/投影文件，不调用生产模型。

```sh
node scripts/season-scope.mjs check --input /absolute/season-scope.json --index /absolute/source-index.json
python3 scripts/workspace.py init --workdir /absolute/JOB --index /absolute/source-index.json --season-scope /absolute/season-scope.json --active-slots 4
node scripts/storyboard.mjs check --input /absolute/bundle.json --source-lock /absolute/source-lock.json --registry /absolute/registry --season-scope /absolute/season-scope.json
node scripts/assets.mjs image-queue --registry /absolute/registry --input /absolute/image-uses.json --output /absolute/image-queue.json
```

`check`可读取proposed；init、资产、分镜和图像消费只接受locked。Node.js24+与Python3.9+仅用标准库；Python入口调用同一Node规则，特殊运行环境可设`SERIES_NODE`指向已有Node，不另写一套边界算法。原始完整索引可内嵌productionScope；投影索引另保留完整readingScope、productionScope和scopeHash。后续派发复核冻结索引与状态，不因工具已运行就声称策划/设计已采用。

`image-uses.json`的形状为`{"version":"series-image-request-v1","baseRevision":当前总库版本,"uses":[{"assetId":"A0001","assetRevision":1,"episode":1,"sceneId":"1-1","purpose":"本季此场实际使用的已采用设计图"}]}`；ID/版本/集场填写实际值，不能用“全部”代替用途。纯声源不自动要求图片；需要独立实体图时仍须核源设计。具体图像交接见[资产交接](asset-handoff.md)及生图Skill模块22。

## 实测时长、交付与续季

逐集/局部导出标`seasonDelivery:"preview"`（缺省亦为preview）；只证明当前部分合法，不冒充全季交付。最终bundle声明`seasonDelivery:"final"`，必须完整覆盖选定全部集场且每场确有正时长镜头。总时长只求和镜头`duration`，不把15/30秒容器、重叠声窗或预告再加一次。最终7200–9000秒以外拒绝，只有真实明确批准的`durationException:{basis,confirmation:{kind,basis}}`才可偏离，并显示`exception-authorized`，不能标成时长达标。

季终正文必须真实完成已批准的回报与钩子，不因机械覆盖/秒数PASS就认定故事收口成功。保留计划季终与实际采用尾态的差异；只有实际采用稿和核源审阅才能建立实际接续。按[续季对接文档](season-continuation.md)保存planned/closed交接包：下一季另建scope/JOB并从下一未制作原集开始，init必带复制在新JOB内的`--previous-handoff`实际closed包，后续claim/供料继续核同一包；自填hash或planned不能回头开工。复用有效资产，**等待用户另行启动，不自动接着生成后半部**。
