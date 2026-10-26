# 原分镜流程的主控补充审稿与定向返修

日期：2026-10-26。适用 `desktop-series672-assets112/v1`。本文件只记录实现及合成回归，不含私人剧本、真实请求正文或账户材料。

## 解决的问题

已收到的独审可能漏掉主控发现的问题。此前恢复只能沿原独审 issues 继续，无法在保留原工作流与回包的同时加入完整反馈。本次增加显式、版本化的主控补充记录；它不冒充独立模型审稿，不改旧 snapshot、raw、原预算或 UNKNOWN。

## 接口与调用顺序

首次调用仍用 `workflows.resume`，保留原 `projectId`、`workflowId`，增加 `controllerReviewSupplement`。以下是字段示意；所有 ID/SHA 必须取自原任务，不能照抄占位符：

```json
{
  "action": "workflows.resume",
  "projectId": "original-project-id",
  "workflowId": "original-workflow-id",
  "controllerReviewSupplement": {
    "version": "series-controller-review-supplement/v1",
    "episodeId": "target-episode-id",
    "sourceSha256": "original-source-sha256",
    "candidateSha256": "reviewed-candidate-sha256",
    "reviewJobId": "accepted-original-review-job-id",
    "stopAfterEpisodeId": "target-episode-id",
    "nextEpisodeSourceRefs": ["next-entry-row-id"],
    "issues": [{
      "id": "C1",
      "severity": "must_fix",
      "unitIds": ["target-unit-id"],
      "shotIds": ["affected-shot-id"],
      "allowInsertAfterShotIds": ["affected-shot-id"],
      "sourceRefs": ["current-episode-row-id"],
      "problem": "带定位的具体问题",
      "requestedChange": "不改原文及无关内容的定向要求"
    }]
  }
}
```

本轮四项反馈分别列为 C1–C4。`nextEpisodeSourceRefs` 可为空；非空时仅允许同源的直接下一集首场入口，最多 12 行、合计 2048 UTF16 单位，原文由程序读取，调用者不能提供替代文本。它只是只读接续证据，不扩展当前候选的写作范围。

1. 验证原独审的任务归属、冻结请求、接收事件、raw、checkpoint 与原始 SDK 文件证据。目标须为未采用候选，且下一返修槽位尚未创建；已有节点、孤立 attempt 或 UNKNOWN 不会获得重发许可。
2. 以 CAS 原子写入 `prepared.seriesControllerReviewSupplement` 和一条冻结事件。重复相同授权幂等，不接受替换已冻结问题。原 workflow snapshot 不变。
3. 复用原 reviewer/repair 循环。原独审问题保留，补充问题以单独来源合并进入下一次返修，仍使用原 `MAX_REPAIRS=2` 上限及原校验修订限制，不建立新 workflow 或重置预算。
4. 仅修改指定镜头及必要单元说明。全部旧镜头 ID、相对顺序和未涉及镜头字段保持；只可在 `allowInsertAfterShotIds` 明确锚点后插入有本集源依据的新镜头。原 plan、selection、原话、说者、声道与设计身份继续受原校验保护。
5. 新独审绑定新候选 SHA，并必须逐条返回 `controllerIssueAssessments`。漏项、未解决却判通过、越界或不完整证据均阻止采用。交付与重开验证同时检查补充证明、角色回执及局部修订范围。
6. 首次执行在目标集正式采用后停止，后续集数记为 `waiting_controller`，不自动调用。
7. 目标已按该补充证明正式采用后，下一次普通 `workflows.resume`（不再携带补充参数）追加独立 continuation 回执，绑定原 workflow、context SHA 和目标集采用稿 SHA，然后允许继续原后续集数。未采用、证据变化或 UNKNOWN 不解锁。冻结补充记录本身不变。

现有单元边界不自动扩大；若返修必须改变未授权镜头或单元结构，保留当前结果并返回受限状态，不靠压缩原话或忽略错误通过。

## 实现文件

- `pipeline/series_controller_review.rs`：补充合同、只读邻集入口、逐项独审、镜头范围和采用证明。
- `text_executor_series_controller_review.rs`：原任务证据验证、CAS 冻结、幂等重开、已采用停点的显式继续。
- `pipeline/series.rs`、`pipeline/series_frame_render.rs`、`pipeline/series_repair.rs`：接入原有返修、渲染证明、最终采用与运行停点。
- `pipeline/mod.rs`、`text_executor.rs`、`engine.rs`：窄接口接线；没有前端修改或数据库 schema 迁移。

## 本轮工程验证

Windows 原生 Rust 工具链；默认 features 为 `[]`。

| 验证 | 结果 |
|---|---|
| `cargo check --no-default-features --tests` | PASS，exit 0 |
| `cargo test --lib series_controller_review -- --test-threads=1` | 8/8 PASS，覆盖冻结、幂等、旧证据保护、预算、UNKNOWN、只读邻集、插镜锚点、首次停点及后续继续 |
| 最终测试二进制：`two_episode_resume`、`durable_publication`、`scope_and_legacy_profiles`、`unknown_does_not_trigger_domain_repair` 四项筛选 | 4/4 PASS |
| 首版改动的 `series` 广泛回归 | 145 PASS、0 failed、0 ignored，exit 0，1635.45s；该二进制不含最后新增的 continuation 两项，其覆盖由最终 8 项提供 |
| `git diff --check` | PASS |

广泛回归命令为 `target/debug/deps/director_core-41b6b5895c40327f.exe series --test-threads=1`；最终 8 项与四项既有行为回归使用 `director_core-5c9f46e7844df9e3.exe`。首版新增 6 项已通过，随后补充显式继续机制并以最终 8 项替代该计数，不重复相加；各组存在覆盖重叠，不汇总为独立测试总数。

上述均为合成/离线工程证据，没有调用真实模型、修改真实项目库、生成图片或提交视频。真实 SDK 回包、逐项读稿及实际采用由主控另行记录，工程通过不等于内容已经修好。
