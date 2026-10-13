# Windows SDK 本地预备失败恢复（2026-10-14）

## 问题与边界

本次新案例的旧桥接回执为 `SDK_STREAM_ERROR / SDK_NATIVE_IDENTITY: attempt directory`。Windows 的 `C:/...` 输入与 `path.dirname()` 返回的反斜杠路径被直接比较，导致 `nativeRuntime.create()` 在构造 SDK 之前失败。旧桥提前持久化了 dispatching，因此 Rust 将它保守地保留为 submission_unknown。

这份补丁只识别已审查桥版本 `fc252738882b4fcdca3a52aeed2c50e1613b290ff3297e7f5cfd12f6b108b08a` 的这个精确错误。TLS 中断、任何 SDK event/thread/runtime/journal、未知旧版本、身份或原回执变化仍阻断，不构成重发许可。

## 实现

- `native-runtime.mjs` 规范化 Windows 绝对路径，拒绝父级跳转、嵌套/兄弟目录、symlink/junction，并以 realpath 和 attemptId 再次确认直接子目录身份。
- 原冻结请求、attempt/workflow ID、编译输入、设置与预算不变，`sdkDispatchStarted` 保持 true。只增加独立的恢复证明和执行身份交接。
- Rust 验证原回执 seal、冻结请求及编译字节、按序 ACK/observation/进程树停止记录、零 SDK 事件及空运行目录。SQLite IMMEDIATE 事务再次校验证据并 CAS 占用一次性意图。
- 桥接新增 `recover-local-preparation`。先以 exclusive create 和 fsync 逐字归档原回执、保存一次性意图，再使用原 record 执行。已有意图、第二次调用或重启只允许 observe，不能 execute/resume 原请求。
- 执行身份只允许桥、监督器及资源目录变化；SDK、Node、官方 runtime、账户、权限及 credential 位置保持原绑定。当前资源清单及可执行字节必须匹配。
- 成功采用前核对原归档、持久恢复意图、当前 sealed completed receipt 与原 attempt 的 rawResult。普通领域校验及原回包保护继续生效。
- 同一 workflow 后续缺失节点可沿已完成恢复锚点连续交接到当前运行身份；仍要求真实已采用前驱、原请求域/parent 绑定、无现有节点与无未知提交。不会改写历史节点或重新生成已完成内容。

## 恢复入口

先构建并校验当前 SDK 资源包和新的原生监督器；必须保留原监督器文件以便历史身份核验。使用原 workflow 的 `workflows.resume`，由 SavedTextExecutor 自动验证本地预备失败证据。不要重新调用 `foundation.run`，不要清除 unknown、删除历史、修改数据库或直接调用模型来绕过冻结请求。

占用意图后，即使中断发生在 RPC 发送前，也只允许读取原回执，不能再次发送；这项限制优先防止重复生成。一般网络失败没有自动放宽。

## 第二次真实预备失败与独立修复

主控首次使用上述恢复后，真实监督器在官方 runtime 启动之前返回 `SDK_WRAPPER_FROZEN_ATTEMPT_MISMATCH`。旧监督器直接比较原 frozen identity 和恢复后的 effective identity；先前 93 项 Node 与 79 项 Rust 回归没有覆盖完整 wrapper 校验，这是先前验证的明确缺口。

此次新增独立 `sdk-wrapper-preparation-repair/v1`，保留原 v1 恢复意图，不将其清除、改成未发送或重新执行。准入仅针对已审查监督器 SHA `48a5b6a608163207297f56240c1f871755d9dd51516ecfad7188543e4a37e146` 和桥 SHA `47de63a6a0cc0a7a1255d593fb428b2a6daac80ddd21dd5d953e16f92347fabb`：验证原可执行字节和原 plan 中必然触发该版本 pre-spawn 早退的身份不匹配，再核对 sealed receipt、编译输入、v1 意图、数据库事件、原 runtime 文件清单及停止的进程树。零事件仅作交叉证据；不依赖项目外 `exec-error.json` 授权。

原失败留下了 `worker.lock`。证据层只读绑定原锁字节和 owner，在 Windows 上明确证明其 PID 不存在或进程已退出；活 PID、权限拒绝、未知状态及已有 recovery guard 均阻断。CAS 前再次验证，实际锁恢复仍由桥已有的互斥 guard 与 PID 重查处理，本层不删除锁。

新恢复分别 exclusive-create 并 fsync `wrapper-preparation-original.json`（第二次失败回执的原字节）和 `wrapper-preparation-repair.json`（独立意图），使用 `native-runtime-wrapper-repair`；原 `native-runtime` 完整保留。原 workflow/job/attempt/request、模型、账户、权限、预算均不变。任何占用后中断仍只 observe，不再 dispatch。完成后经过原 `sdk_receive`、领域校验和 `sdk_commit`，后续节点依赖真实已采用前驱分配。

wrapper 本身现在核对正常、v1 恢复及第二次 repair 的完整谱系，再验证 stdin、argv、环境、schema、工作目录和 runtime 字节。测试只替换最后的 launch callback，所有生产 wrapper 校验均真实执行，没有生产测试开关。历史 identity 的读取兼容只登记已知版本，当前执行另有 `SdkIdentity.validate()` 和字节校验，不开放任意旧桥。

## 第一轮验证（已暴露覆盖缺口）

基于 `integration/windows-mac-20261008` / `c1fac32e8e8c5e0801f34942759cbbfb4cc4d169` 上的未提交工作区变更。

- `cd sdk-bridge; npm run verify`：93/93，通过。包含一次恢复、普通 unknown 阻断、恢复再次失败/重启零新增调用、路径规范化及链接/目录逃逸拒绝。
- `cargo test --lib --no-default-features --no-run --message-format=json`：最新源码编译通过。
- Windows native runner，过滤 `sdk_local_`：5/5，通过；包含已完成恢复任务到后续节点交接、缺失证明/未知前驱/原回执变更拒绝，确认测试进程树清空。
- Windows native runner，过滤 `sdk_ --test-threads=1 --nocapture`：79 passed / 0 failed / 2 ignored，260.76 秒，未超时；runner 已清理测试后代进程并确认进程树为空。忽略项分别依赖私有 SDK 成稿容量资料和明确指定的 Q1 原始回执，未作为通过项。
- `git diff --check` 通过；仓库原有 CRLF 提示不属于差异错误。

本专项没有真实模型调用，没有写生产数据库，没有恢复原生产 workflow，没有提交代码。离线专项通过不代表真实 SDK、客户端 GUI、整批第一至第三阶段或最终发布验收已完成；这些由主控在新包就绪后执行。两个 ignored 项也不能计为通过。

本机证据保留在 `.local/validation/stability-case-20261014/`：`sdk-node-tests.log`、`sdk-recovery-build.jsonl`、`sdk-local-final-run/receipt.json` 与 `sdk-final-regression/`。原回执审计记录生成于恢复协议实现前，其“未实现恢复协议”字段仅表示当时状态。私人请求、原文和回包不进入本文档或提交。

用于最后专项及全套回归的独立测试程序为 `sdk-local-final-tests/sdk-local-tests.exe`，SHA256 为 `126bdc6c00712e6739382feba241cc235e0e8d50de776e9bd5ca66c7875ffae8`。它是离线测试程序，不是交付客户端或生产监督器；原生产 `case-executor.exe` 没有被覆盖。

## 第二轮最终验证与交付边界

以下结果针对补齐 wrapper 谱系、独立 repair 与 stale-lock 证据后的源码；不将第一轮结果当作这次完整验证。

| 检查 | 实际结果 | 本机证据（不提交） |
| --- | --- | --- |
| `cd sdk-bridge; npm run verify` | 102/102；修复一次提交、失败重开零新增调用、原目录保持、污染拒绝 | `sdk-wrapper-repair-node-tests.log` |
| Rust `cargo test --lib --no-default-features --no-run --message-format=json` | 编译通过 | `sdk-repair-final-build.jsonl` / `.stderr.log` |
| 完整生产 wrapper 校验到 launch 边界 | 4/4；仅末尾 launch callback 使用离线替代，完整正向与污染检查均执行 | `sdk-repair-wrapper-tests/receipt.json`；也纳入后述全量 |
| unknown → 原 raw 接收/采用 → 下一节点 → Store 重开 | 1/1；保留原请求、v1 意图与第二次 repair 谱系，重复节点拒绝 | `sdk-repair-chain-tests/receipt.json`；也纳入后述全量 |
| `sdk_wrapper_preparation_captured --ignored --test-threads=1 --nocapture` | 1/1，4.90 秒；原项目 SQLite 只读、原文件双读不变；11 类污染拒绝 | `sdk-repair-captured-readonly-v2/receipt.json` |
| `sdk_ --test-threads=1 --nocapture`（600 秒界限） | 85 passed / 0 failed / 3 ignored，359.06 秒；包含恢复进度投影测试 | `sdk-repair-final-regression/receipt.json` / `stdout.log` |
| 新增常规 `sdk_wrapper_worker_lock` 回归 | 1/1，0.05 秒；无需私有项目，验证活/非法 PID 拒绝及真实已退出子进程只读证明 | `sdk-lock-default-regression/receipt.json` |
| `git diff --check` | 通过 | 只有已有 CRLF 提示，无差异错误 |

所有上述 Windows native runner 回执均确认 `processTreeEmptyVerified=true`，无超时。全量回归共回收 197 个测试进程。其 3 项 ignored 中，本次 captured wrapper 证据检查已单独显式运行通过；另两项仍为私有容量资料与指定 Q1 原回执测试，未作为通过项。常规锁回归在全量启动后补入，仅增加 `cfg(test)` 代码，单独编译与验证，没有改变生产实现。

全量独立测试程序 `sdk-repair-final-tests.exe` 的 SHA256：`c26b56c9abc757557cb89fcc0467a631636bb050429ad0c4f9aeafd20615841f`。原 `case-executor.exe` 与已失败 `case-resume-executor.exe` 都必须保留；新监督器另存，不覆盖历史可执行文件。

恢复入口仍是原 workflow 的 `workflows.resume`，由 SavedTextExecutor 自动识别固定失败并占用独立 repair。上述子任务未调用真实模型、未写生产数据库/回执、未删除原锁、未恢复生产 workflow、未提交。主控负责新监督器资源校验及唯一真实恢复，真实 SDK 成稿、客户端、阶段一至三整批与最终发布仍须另行验收。

## 主控真实续验

上述离线专项结束后，主控于本地 2026-10-14 02:43:54 执行唯一受保护恢复，02:51:32 同一第一阶段 workflow 完成，F2→Q1 自动衔接，发布 47 个资产和 10 集制作基础。未新建 workflow 或替换原请求；第二阶段新节点已通过正常 SDK 入口继续。真实调用与产品安装记录见 [新案例稳定性记录](STABILITY_CASE_AND_MANAGED_EDGE_20261014.md)，不将第二、三阶段或浏览器图片未验收范围计为通过。
