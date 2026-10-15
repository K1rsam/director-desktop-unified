# 专用普通浏览器接入与原任务恢复（2026-10-15）

分支：`integration/dedicated-browser-20261015`。基线：`c0c7d6a4e5fc5e269b21a4efa68019e5ba1a64db`。

## 采用的行为

- 在原绑定的 profile/bundle 内，优先使用唯一的已登录 ChatGPT 首页作为连接页。必须是真实网页运行层就绪、隐藏、未聚焦、无活动请求，不能把 Native/后台握手当成网页就绪。
- 连接页保留用户所有权，不写入任务标签。工作页通过它在同一浏览器窗口后台创建；不同浏览器、隐身、身份变化和多个连接首页仍拒绝含糊派发。
- Native inventory 的 `sourceRole=connection` 进入 canonical JSON 与 SHA 校验。仍检查两次原生清单、当前只读命令、ACK、队列、代际和时效；不能仅凭连接快照跳过独立证明。
- 原图片工作页或旧只读 reader 丢失时，最多额外预约一次只读 reader，打开原冻结 locator URL。先写 replacement intent，再开页；原 reader 与导航记录进入 `retiredReadOnlyRecoveries`，不结算旧未知命令，不重新生成。
- 恢复页稳定性检查按真实连接页 client ID 核对没有 launch token 的连接页，不能把其他无 token 的普通用户对话计成重复控制页。程序拥有的控制页继续检查 ownership token 唯一性。
- 全局设置说明同步为“保留已登录首页，连接后最小化，工作在后台页执行”。

没有改写 BrowserBinding、既有 profile/bundle 身份、冻结请求、原回包、预算或原任务 ID。没有修改原 Web/Mac 工程、Stage 3 候选规则或私人原文。

## Windows 实测

证据在被忽略的 `.local/validation/native-messaging-20261015/`，不提交真实网页数据和凭据。

| 检查 | 结果与范围 |
| --- | --- |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib chrome_` | 163 通过，0 失败；是浏览器相关子集，不是完整 Rust 套件 |
| `node --test` 对 `apps/chrome-bridge/tests/*.test.cjs` 的实际文件列表 | 331 通过，0 失败；包括服务端→content handler→原生清单角色传递回归 |
| 前端测试与 build | 本轮前序记录 252 通过、build 通过；最终追加的修复仅涉及 Rust 和桥接测试 |
| Windows release build | 通过；存在已有编译 warning |
| 最终候选隐藏启动、保存重开 | 通过；独立 home、真实 WebView/IPC，无生成提交，详见最终启动回执 |
| 原七项目、生产表、十集源与 SDK unknown 证据保护 | 再次核对通过；没有用新任务覆盖 unknown |

多页面恢复回归先在修改前实际失败，修复按 client ID 选择后通过。

## 真实图片结果：尚未验收通过

用户正常重开专用普通 Edge 后，曾读到真实首页：page/composer/chatMain ready，hidden/unfocused，无活动请求。

随后只对既有基础图做一次显式原任务查询：原请求 ID、job、prepared 与 locator 保持不变。Native inventory 返回 `status=unknown / reason=source_owned_idle_root_unproved`，回收停在身份/空闲证明门禁；没有重新提交图片。该原因还不足以区分源页原生记录不符与已加载扩展代码不一致，不能把推测写成已定位根因。

一次后台页面刷新后，该页仅保持 `backgroundOnly=true` 的传输握手，完整网页 hello 未恢复；服务端 pending request/command 均为 0。后台健康另报告旧维护链 `maintenance_rebind_journal_hash_unresolved`，没有证据证明它是当前页面未就绪的直接原因。未清除旧维护日志、未知命令或重试预算。

额外后台维护页升级脚本在“没有真实就绪的专用页”的检查前退出，没有开维护页或发升级指令。不再对同一旧接口反复重载。

基础图尚未回收，衍生图仍等待直接父图成功入库。尚不能宣称普通 Edge 连续出图、原图下载、真实父图衍生、断线后零重发或整批稳定性通过。旧 Chrome 已完成图片不重复生成。

## 后续入口与限制

保留当前失败证据，优先查明完整网页 hello 缺失和 Native 源页记录门禁的具体字段差异，再沿原请求恢复。不得通过删除日志、重建项目/图片任务、改旧 locator 或伪造空闲/ACK 绕过。

只允许已授权个人套餐内无按张额外收费的网页图片。付费图片和所有视频均不提交。十集总验收中既有 SDK unknown 及后续集仍需继续，本文不宣称总任务完成。
