# 本地工具：只查当前一步

命令在本包根执行；`JOB`替换为绝对工作目录，Python可执行名按本机。工具不调用模型或生图。所有生产文件另存新版本；日志留文件，主控只读摘要/具体失败坐标。

## 接入和状态

先按[intake](intake.md)确认范围/方向并完成源接入和独立源锁。已有标准源复用，不每岗位重新接入。正式索引最低为 `{"version":"series-source-v1","sourceHash":"原件SHA256","episodes":[{"number":1,"scenes":[{"id":"1-1"}]}]}`，数组保持源序。

```sh
python scripts/workspace.py extract --source SCRIPT.docx --output-dir JOB
python scripts/workspace.py init --workdir JOB --index JOB/source-index.json --active-slots 4 --batch-size 2
python scripts/workspace.py status --workdir JOB --compact
python scripts/workspace.py claim --workdir JOB --group 1 --owner author-1
```

范围未锁定只做extract/轻量全读。选季init另传 `--season-scope JOB/season-scope.json`；明确局部可传 `--episodes 3,5`，不越locked季范围。已init不重新初始化；源版本改变另建JOB、按真实差异复用有效旧稿。status/claim已有紧凑输出，必要明细才加`--full`。

主控从受核source、已采用设置、集场分配、总库投影一次建立单集基线（[schema](schema.md)），程序复制受核source并带源锁，不让作者生成/重抄source。源归场与场绑定必须准确；如果成稿真实使用范围改变，由主控修正新基线并保留旧版，不能在creative响应绕改。

派作者前先运行 `python scripts/lite_edit.py preflight --workdir JOB --base ep1/base.json`，只读核源锁声明、风格/禁止项类型、采用视觉基准和容量/幻想配置。允许创作字段尚空；它不核源锁文件内容、资产充分性或剧情质量，后续正式检查照常。配置错误在此修正，别等创作完成再重跑。

## 岗位包与可读视图

示例finisher manifest，路径相对JOB；作者省略plan/selection/draft可在同次生成。

```json
{
  "version":"series-job-request-v1","episode":1,"role":"finisher",
  "review":{"path":"ep1/dependency-review.md"},
  "inputs":[
    {"kind":"source","path":"ep1/base.json","pointer":"/episodes/0/source"},
    {"kind":"direction","path":"shared/direction_v1.json"},
    {"kind":"visual-baseline","path":"shared/visual-baseline_v1.json"},
    {"kind":"state","path":"ep1/facts.json"},
    {"kind":"assets","path":"ep1/complete-designs_v1.json"},
    {"kind":"entry","path":"ep1/entry_v1.json"},
    {"kind":"plan","path":"ep1/bundle_v1.json","pointer":"/episodes/0/plan"},
    {"kind":"selection","path":"ep1/bundle_v1.json","pointer":"/episodes/0/selection"},
    {"kind":"draft","path":"ep1/bundle_v1.json","pointer":"/episodes/0/draft"}
  ],
  "methods":[],"contracts":[]
}
```

共同规则已含表演/摄影/声音/光影核心。源有或已选幻想追加methods的`fantasy-expression-methods`；复杂系统可追加contracts的`system-presentation`。review引用主控实际依赖核查，不新增审批。资料须完整且路径不越JOB；同内容去重，JSON Pointer保留值精度但仍绑定整个不可变源文件hash。禁止用可变总库当每集输入。

```sh
python scripts/job_packet.py build --workdir JOB --manifest ep1/request.json --output ep1/packet_v1.json
python scripts/packet_view.py --workdir JOB --packet ep1/packet_v1.json --output ep1/read_v1.md
```

view已调用live verify，无需紧接再verify一次。派执行者读read_v1.md到EOF；它完整保留规则和资料及各自归属，包hash留头部，元数据细账留packet。默认包512 KiB只是字节护栏；确需更多完整资料可提高build的`--max-bytes`，不能靠截断/删必要设计过门。

## 只写创作变化

作者creative.json严格只含plan/selection/draft；主控从文件字节计算base SHA-256（如PowerShell `Get-FileHash -Algorithm SHA256`，传小写），执行：

```sh
python scripts/lite_edit.py compose --workdir JOB --base ep1/base.json --response ep1/creative_v1.json --expected-base-sha256 HASH --output ep1/bundle_v1.json
python scripts/lite_edit.py patch --workdir JOB --base ep1/bundle_v1.json --patch ep1/patch_v2.json --output ep1/bundle_v2.json
```

patch形状：

```json
{"baseSha256":"本次原始bundle字节SHA256","operations":[{"op":"set","path":"/episodes/0/draft/units/0/shots/0/picture","expected":"基稿原值","value":"真实局修后的值"}]}
```

只替换已有plan/selection/draft路径；不支持append/remove或重叠路径，删合镜用完整相关单元/units数组替换并保留未改内容。完整合成稿才交采用，patch本身不算交付。原值不符或基线过期先核当前版本；工具不自动合并冲突、不改源/设置/场表/资产。changed=false表示没有变化，继续沿用原稿，不能冒称改善。输出`validation:not-run`，必须后续检查。

## 正常交接及一次本地采用

作者停止写入后，实际新建整编上下文再handoff；不用resume或重新claim：

```sh
python scripts/workspace.py handoff --workdir JOB --episode 1 --owner author-1 --to finisher-1 --candidate ep1/bundle_v1.json --evidence ep1/author-notes.md --role finisher
```

整编候选的source/plan/selection及完整视觉基准须与当前岗位包一致；改了plan/selection就重建包，不是改hash。将以下保存为ep1/execution_v1.json：

```json
{"version":"series-episode-execution-v1","episode":1,"packet":"ep1/packet-finisher_v1.json","bundle":"ep1/bundle_v2.json","sourceLock":"shared/source-lock_v1.json","registry":"shared/assets"}
```

选季另加seasonScope相对路径。run输出目录独立，不是输入所在目录：

```sh
python scripts/episode_execution.py run --workdir JOB --manifest ep1/execution_v1.json --output ep1/run_v1
```

run已顺序sync/check/export/verify，保存预览、目录、消费回执与adoption.json；相同命令可恢复/复用，输入改变用新版本目录，不能覆盖旧版。收到结果实际读预览核字段、图片用途与表演，不能再让模型完整重写MD。历史收据核验才用`episode_execution.py verify --workdir JOB --receipt ep1/run_v1/adoption.json`。

complete证据最低如下，填真实当前文件，不复制多份阶段报告：

```json
{"version":"series-episode-evidence-v1","episode":1,"sourceHash":"源hash","sourceIndexHash":"索引hash","scenes":["1-1"],"artifacts":{"intake":["ep1/dependency-review.md"],"assets":["ep1/complete-designs_v1.json","ep1/run_v1/adoption.json"],"direction":["ep1/bundle_v2.json"],"delivery":["ep1/review_v1.json","ep1/run_v1/adoption.json"]},"adopted":"ep1/bundle_v2.json","adoptionReceipt":"ep1/run_v1/adoption.json","reviewMode":"self"}
```

run文件名以实际结果为准；独审已真实完成才independent。组内续作另含 `entryReview:{upstreamHash,notes}`，hash来自claim绑定的实际前集采用稿。四阶段文件可复用已有真实记录，不要求各写一篇。

```sh
python scripts/workspace.py complete --workdir JOB --episode 1 --owner finisher-1 --evidence ep1/evidence_v1.json
```

独审无改动另存绑定当前稿hash记录即可，不改旧self证据。真返修/中断使用 `workspace.py resume --workdir JOB --episode 1 --evidence ep1/recovery.md`，先停该集写入；同组其他in_flight先安全落盘/回收。跨组或变更后实际边界用 `reconcile`，证据 `{version:"series-boundary-v1",upstream,current,upstreamHash,currentHash,outcome:"pass",notes}`；只可写真实核对结果。无关库新增不使旧稿失效，已用项变化核实际消费者。

## 最终导出与白图

按冻结scope合并全部采用bundle，保持源序和资产ID/revision，不能只合已有的那部分反推范围。独审和实际边界闭环后，对全范围执行：

```sh
node scripts/assets.mjs sync --registry JOB/shared/assets --input JOB/final-bundle_v1.json --source-lock JOB/shared/source-lock_v1.json --output JOB/final-consumption_v1.json
node scripts/storyboard.mjs check --input JOB/final-bundle_v1.json --source-lock JOB/shared/source-lock_v1.json --registry JOB/shared/assets
node scripts/storyboard.mjs export --input JOB/final-bundle_v1.json --source-lock JOB/shared/source-lock_v1.json --registry JOB/shared/assets --output JOB/final_v1.md
node scripts/storyboard.mjs verify --input JOB/final-bundle_v1.json --source-lock JOB/shared/source-lock_v1.json --registry JOB/shared/assets --output JOB/final_v1.md
python scripts/make-white-placeholders.py --index JOB/final_v1.md.assets.json --output-dir JOB/white_v1
python scripts/make-white-placeholders.py --index JOB/final_v1.md.assets.json --output-dir JOB/white_v1 --verify
```

选季四步均加 `--season-scope JOB/season-scope.json`（assets sync亦支持），最终bundle明确 `seasonDelivery:"final"`，范围/实际时长及续季资料见[season](season.md)，不能拿preview覆盖当final。export排他创建，verify独立回读MD及配套目录。white目录只放PNG，压缩同版ZIP并核成员；资产接续、后期备注和日志放外面。无本次平台导入证据仅标本地包就绪。

## 旧库迁移（只有需要时）

仅未声明importProfile的旧库另存请求 `{"version":"series-asset-import-v1","baseRevision":当前版本,"importProfile":"lingmo-v7","names":[]}`，用 `node scripts/assets.mjs prepare-import --registry DIR --input REQUEST.json` 新建映射快照；同类安全名碰撞时以 `names:[{id:"实际资产ID",importName:"明确安全名"}]` 处理，不合并身份、不改旧快照。已有v7库不重复迁移。

## 维护自检

运行本包 `python -m unittest discover -s tests -p 'test_*.py'` 与 `node --test tests/*.test.mjs`，按实际改动选必要项。测试样例是合成数据，仅验合同/代码，不证明源审阅充分、节省费用或成片质量。

提前交接复用 `assets.mjs image-queue`，无最终MD亦可生成；示例与材料要求见[资产交接](asset-handoff.md)。不另建生图运行器。

## 已有设计追加用途

`assets.mjs record-uses --registry DIR --input JSON` 只追加同版设计的集场用途，不改资产或替代消费回执。输入、适用边界与旧工程旁路接入见[用途复用](asset-usage-reuse.md)。
