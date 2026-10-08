# 创作规则版本与来源

这里保存用户授权上传的规则原件。目录只用于来源比对，应用不会从这里加载规则。逐文件大小、版本、SHA256、来源和运行角色见 [index.json](index.json)。

| 目录 | 文件数 | 来源与状态 |
| --- | ---: | --- |
| `desktop-original-2ac7c216` | 29 | 桌面初版 `2ac7c216183f936aa08f16a93b2a24130cf5ebcd`，按迁移来源索引逐字节复核 |
| `mac-3008-git-2511c56` | 21 | 旧 Mac Git 基线 `2511c56a52794cf5c5ee88434afafb96c787ad4c` 中存在的对应文件；此提交尚无 `director-refinement/SKILL.md` |
| `mac-3008-production-20261005` | 22 | 本机实际运行目录中的原件；与旧两集生产任务冻结的22份规则、现有候选导出逐字节一致，包括未提交的导演精修规则 |

72份快照共552341字节。它们分别保留，没有覆盖同名的初版、当前版或候选版。

桌面通用注册表是 `apps/desktop/src-tauri/src/pipeline/mod.rs::rule_snapshots`，编译 [apps/desktop/rules](../apps/desktop/rules) 中29份规则。`asset-art-director` 当前为2.0.0，初版快照为1.3.0；生图生产采用 Windows 1.2.0。第三阶段新任务由 `pipeline::stage3_pack` 显式加载下述22份原件，旧冻结请求保留原 profile 和规则。注册、发送请求与真实成稿质量是分别核验的状态。

[rules/stage3/manifest.json](../rules/stage3/manifest.json) 的22份原件现注册为 `mac-3008-compatible-rust/v1`：兼容 Rust 适配已接入取料、三岗、独立导演精修、采用和恢复，并有 Windows 离线回归。它不是未交付正文的42个 Mac 运行文件；真实第1、2集新旧质量比较尚未完成。没有启用导出的整合 SKILL，也没有把后续短段实验替换为生产规则。具体证据见 [适配记录](../docs/handoff/STAGE3_COMPATIBLE_RUST_ADAPTER_20261008.md)。

来源交接保留于 [Mac本机修改回执](../docs/handoff/LOCAL_STANDARD_EDITION.md)，当前结果见 [Windows/Mac集成记录](../docs/handoff/WINDOWS_MAC_INTEGRATION_20261008.md)。运行 `node scripts/verify-integration-lineage.mjs` 校验当前注册、规则原件与历史口径；此命令不读取用户登录目录或私人剧本。
