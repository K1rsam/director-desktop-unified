# Chrome 原会话链接恢复与无图终态

代码基线：`e75b6a6`，分支 `integration/windows-mac-20261008`。

新请求不再添加可见任务关联码。运行器仍保留内部 requestId，独立持久保存精确会话 URL、原生 userMessageId、完整提示词哈希、prepared 哈希和浏览器绑定。旧 prepared、原回包、预算和已入库媒体保持原样；旧关联码请求允许原生消息身份从空值单向升级。

原工作页丢失或用户正在查看旧失败页时，先保存一次只读恢复预约，再由后台控制页打开独立 owned 工作页读取原链接。开页结果未知时不重复开页；原任务身份不改，不提交提示词。读取校验包括原生分支、完整提示词、原始 user message、图片身份和原图字节。下载入库后的清理只关闭经过核验的恢复页，保留用户原页。

网页最终仅返回文字且没有图片时，必须通过当前分支双读、最新 DOM 回读、零图片证明，以及同 ID 的 bridge 终态结算，才记录图片生成失败并释放该任务的渠道占用。未知提交、旧失败但无完整证明的任务继续保护，不可用新任务绕过。记录失败不等于生成成功，后续显式重试必须保留失败历史。

## 工程验证

- Windows `cargo test --lib chrome -- --test-threads=1`：144 passed，0 failed，0 ignored。
- `node --test apps/chrome-bridge/tests/*.test.cjs`（展开实际文件列表）：131 passed，0 failed。
- 覆盖后台只读 alias、一次持久开页预约、未知开页保护、locator 身份固定和重开、legacy 单向升级、原无图终态、用户原页保留、原图回收与旧队列回归。
- 扩展 overlay 在空闲状态已部署；后台 reload `ownedTabsOnly=true` 回执成功，零模型提交。

## 真实验收边界

开始本轮时原批次为27张入库、1项已发送后无图、1项确定未发送；截图和原生只读回执证明无图失败。上述工程测试不能代替最后两项真实生成、恢复、原图入库和采用验证。真实进度及客户端发布证据后续单独补充；禁止视频提交，图片限定用户已确认的个人套餐内生成。

### 原批次现场实测与追加修复

- 第28项原请求双读证明最终无图，保存正式失败及同 ID 回执；原查询预算保留，没有再次发送原请求。
- 第29项继续原 prepared，保存原生 userMessageId 后暂停本地队列。网页生成完成并得到精确原图证明后，关闭仅本程序的隐藏原工作页，再恢复原批次：按保存链接创建后台只读 alias，下载并采用原图成功。原请求 ID 不变，恢复没有生成调用。当前28张已入库，原批次保持失败历史。
- 显式重试第28项时发现 UI 任务摘要缺少 prepared 与 nativeProof，队列因此误把已结算失败当成未决。修复为仅在 Chrome 队列中按需载入已知失败的完整原任务；未知或无证明的失败仍复用保护。新重试 snapshot 记录 retryOfJobId 和 retryOfRequestId，原失败不改写。
- Windows 独立 native runner 对前一个发布代码实际执行144项 Chrome 回归，exitCode=0、无超时、进程树退出已核验。
- 客户端仍使用固定同名当前版与唯一上一版备用。图片真实重试和最终资产审计在完成后另行记录。
- 追加修复后 `cargo test --lib chrome -- --test-threads=1`：145 passed，0 failed；新增回归同时验证完整证明与未决失败的不同队列行为、跨项目占用判断、失败历史不变和重试来源关联。

### 参考图重复文件名实测缺陷

无关联码重试实际遇到 ChatGPT 将重复参考图文件名改为 `(1).png`，原精确文件名等待因此停在上传环节。后台 layout capture 和原请求执行状态证实：图片预览已存在，attachments.upload 已停住，sentAt=0、没有原生用户轮次或生成；不能把这一情况猜成模型生成慢。

新上传文件名按 requestId 和参考图序号唯一命名；图片原 SHA、mediaId、顺序和上传回执独立保留，精确图片校验不放宽。旧冻结 prepared 不修改。新增停住上传的结算只接受原 request、原 source、原附件列表、已停止的 attachments.upload effect 和原生空页；在原任务取消前后各读取一次原生空页。任何已发送、未知作用或出现用户轮次的情况仍保护，不允许重复提交。

扩展恢复核验同时修正：附件期待列表可读取原 effect 的 preconditions；缺少期待列表或与原列表冲突时不再误报“没有附件需要上传”。最终该补丁 Windows Rust Chrome专项148项通过，扩展132项通过；真实上传恢复仍按后续回执分别验收。

原上传等待随后被 bridge deadline 终止；canonical lifecycle=failed、submission=accepted，原 attachments.upload 不确定记录和 sentAt=0 的 scoped snapshot 保留。结算兼容这个已经失败的状态，但强制 submission=accepted，并继续双读原生空页。该窄补充的 Chrome terminal 5项、上传结算HTTP专项1项通过；不将已提交、缺证据或其他失败视为未发送。

### 已失败准备页的隔离恢复

实测发现旧上传等待的 release cleanup 将准备页隔离。恢复仅对同 ID 已失败的原页开放只读未发送核验；核验完整后，先持久保存冻结 prepared 绑定的完整证明，再由空闲后台控制页处理这个本程序废弃页。保存证明后发生崩溃或页面丢失，可以恢复原失败，不能重新提交。没有完整未发送证明的隔离仍禁止绕过。owned close 增加当前 URL/非隐身核验，并从 HTTP 到原生后台完整传递。

本补丁 `cargo test --lib chrome -- --test-threads=1`：151 passed，0 failed；扩展完整测试134 passed，0 failed。包含关闭前持久化顺序、保存证明重开、错误 prepared 拒绝、原任务/原请求保留及原 URL 改变拒绝。
