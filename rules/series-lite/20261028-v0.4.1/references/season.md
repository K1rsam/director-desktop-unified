# 季范围与续季技术接口

仅分季时读取，开工见[intake](intake.md)。**示例不构成授权，均须换实际记录。** 从Skill目录运行，路径换为本机绝对路径。

## 1. locked范围文件

`season-scope.json`形状：

```json
{
  "version":"series-season-scope-v1","id":"S01","revision":1,"seasonNumber":1,"status":"locked",
  "sourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "readingScope":[{"number":1,"sceneIds":["1-1"]},{"number":2,"sceneIds":["2-1"]},{"number":3,"sceneIds":["3-1"]}],
  "episodes":[{"number":1,"sceneIds":["1-1"]},{"number":2,"sceneIds":["2-1"]}],
  "target":{"minSeconds":7200,"maxSeconds":9000,"planningSeconds":8100},
  "ending":{"episode":2,"sceneId":"2-1","sourceRef":"source-body.txt:L40-L47","closure":"源支持的本季回报","hook":"源已有的未解期待","estimate":{"minSeconds":7500,"maxSeconds":8500,"basis":"实际逐段估时依据"}},
  "confirmation":{"kind":"user","basis":"本工程真实范围选择记录"}
}
```

- id非空单行；revision/seasonNumber为正整数，status为proposed/locked；confirmation.kind仅user/delegated，basis须真实。
- sourceHash为原件文件SHA-256。readingScope对应全剧原始索引；episodes源序连续整集、sceneIds全等；首季从第一集到选中末集末场。可附非空、不重复ending.sourceRefs。
- target固定7200/9000/8100。估时min>0、max≥min；locked估时须与目标区间相交，否则proposed或附真实例外：

```json
{"durationException":{"basis":"实际偏离原因及批准范围","confirmation":{"kind":"user","basis":"针对本次偏离的真实批准记录"}}}
```

另定时长仍保留target，以例外表达。范围改变另建revision；scopeHash为规范化JSON哈希，不是文件字节hash，取工具结果。

```sh
node scripts/season-scope.mjs check --input /JOB/season-scope.json --index /JOB/source-index.json
```

check可读proposed并返回scopeHash。init带同一 `--season-scope`（见[intake](intake.md)），自动投影；投影含readingScope/productionScope/scopeHash，不能冒充全剧原始索引。

## 2. 生产消费者与最终交付

bundle根内嵌productionScope，可附相符scopeHash；总库绑定同scope/productionScopeHash，派发/资产/图像/导出共用，不能删内嵌scope绕门。

局部bundle用 `seasonDelivery:"preview"`（缺省同），最终合并用 `seasonDelivery:"final"`，须全范围每场有正时长镜头。实际秒数仅求和shot.duration，支持9位小数，不加容器/重叠声窗；final在7200–9000秒外拒绝，有真实durationException才为exception-authorized。

```sh
node scripts/storyboard.mjs check --input /JOB/final-bundle.json --source-lock /JOB/shared/source-lock.json --registry /JOB/shared/assets --season-scope /JOB/season-scope.json
```

export/verify同参并加 `--output /JOB/delivery/season.md`；sync也传同源锁/范围，正常走统一执行链。

## 3. planned与closed交接请求

复制引用文件到独立包根，保留原件/旧包。引用仅 `{path,sha256?}`；path为包内相对路径，用 `/`，禁止绝对路径、反斜杠、冒号、`.`/`..`及越界链接。create计算hash、排他创建；verify核冻结hash。

计划请求 `planned-request.json`：

```json
{
  "version":"series-season-handoff-v1","kind":"planned",
  "source":{"original":{"path":"source.docx"},"body":{"path":"body.txt"},"index":{"path":"index.json"}},
  "scope":{"path":"season-scope.json"}
}
```

source.index必须为全剧原始索引，不含投影readingScope/scopeHash，可内嵌合法本季productionScope。sourceHash绑定original，body另有hash。planned不含closed/actualTail/reusableAssets，不能闭季或续季。

闭季请求 `closed-request.json`完整形状：

```json
{
  "version":"series-season-handoff-v1","kind":"closed",
  "source":{"original":{"path":"source.docx"},"body":{"path":"body.txt"},"index":{"path":"index.json"}},
  "scope":{"path":"season-scope.json"},
  "closed":{
    "bundle":{"path":"bundle.json"},"sourceLock":{"path":"source-lock.json"},
    "registry":{"path":"assets-v000012.json"},"adoptedStoryboard":{"path":"采用分镜.md"},
    "continuity":{"path":"actual-continuity.json"},
    "adoption":{"kind":"delegated","basis":"真实委托及审稿依据"},"images":[]
  }
}
```

registry为不可变快照，bundle须 `testFixture:false`；preview也强制全覆盖/时长/源锁/库匹配与MD回读。adoption.kind为user/delegated；未生图可 `images:[]`。

images每项为 `{"assetId":"A0001","assetRevision":3,"state":"candidate_pending_user","file":{"path":"images/角色.png"}}`。采用图用state:adopted并加 `adoption:{"kind":"user","basis":"真实采用依据"}`（亦可有效delegated）；候选不得带adoption。每ID一项，限本季实用版本，不评像素。

```sh
node scripts/season-handoff.mjs create --input /HANDOFF/request.json --root /HANDOFF --output /HANDOFF/handoff.json
node scripts/season-handoff.mjs verify --input /HANDOFF/handoff.json --root /HANDOFF
```

两种请求分别create并另存planned-handoff/closed-handoff，不互覆盖。

## 4. 实际尾态文件

`actual-continuity.json`：

```json
{
  "version":"series-season-continuity-v1","scopeHash":"本季check返回的64位scopeHash",
  "tail":{"episode":2,"sceneId":"2-1","unitId":"U17","shotId":"S52"},
  "states":[{"assetId":"A0001","assetRevision":3,"position":"实际位置/朝向/动作停点","emotion":"实际情绪强度","knowledge":["当时已知事实"],"heldProps":["A0012"],"injuries":[],"sourceRefs":["E02:R18"]}],
  "openHooks":[{"text":"源中未解悬念及接点","sourceRefs":["E02:R18"]}],
  "deferred":["延后资产或事项"],"unresolved":[],
  "review":{"kind":"delegated","basis":"真实核源/尾稿审阅依据"}
}
```

tail四坐标等于采用末镜。states为当前CH ID/版本且不重复，position/emotion非空；knowledge/heldProps/injuries为字符串数组，heldProps为已知PR ID。sourceRefs非空不重复且存在于本季sourceRows。空states须加stateBasis解释末场/前场状态；openHooks须真实非空，unresolved非空不能closed，不编钩子过门。

工具生成文件hash、scopeHash、`resume:{previousSeasonId,previousScopeHash,nextEpisode}`；closed另有 `actualTail:{episode,sceneId,unitId,shotId,exitState,lastPicture,lastSound}`（后三项取采用末稿）、`reusableAssets:[{assetId,assetRevision,key,designState}]`，不凭计划填。源/稿/尾态/库变化须另版收口。

## 5. 下一季与同源限制

续季先verify上季closed并读实际尾稿/尾态。nextEpisode按源序取下一整集，不是数值加1；null表示源结束。新scope换id、seasonNumber加1、同sourceHash，从nextEpisode开始，readingScope完整。上季resume原样放入新scope.continuation：

```json
{"continuation":{"previousSeasonId":"S01","previousScopeHash":"上季生成的真实scopeHash","nextEpisode":3}}
```

```sh
node scripts/season-handoff.mjs verify-next --input /HANDOFF/closed-handoff.json --root /HANDOFF --next-scope /S02/season-scope.json
python scripts/workspace.py init --workdir /S02/JOB --index /S02/source-index.json --season-scope /S02/season-scope.json --previous-handoff /S02/JOB/previous-season/closed-handoff.json
```

新JOB先同源提取。季号≥2须将上季closed及全部引用文件复制到新JOB内，JSON父目录为包根，再传previous-handoff；verify-next/hash/孤立JSON不能替代。init及后续状态读取继续核包与游标；首季禁传previous-handoff，各季独立JOB。

工具不自动转绑旧季库，拒绝不同sourceHash的verify-next。复用按本季用途登记，留旧库ID＋资产ID＋版本映射，不手改JSON解锁。追加稿另存原件/hash/索引/锁；只追加核旧正文完整字节前缀及旧集场/原声/事实未改。独立新集标前驱scopeHash/末镜；跨源或旧文冲突须独立接入、裁决及继承入口，不能改旧hash自动接续。
