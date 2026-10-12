# 原 Chrome 批次的后台续跑

## 修改

- 扩展自带一分钟 alarm 守护，不依赖网页 transport。原专属控制页的 profile、local opened 回执、session launch、原生窗口与执行状态一致时，只重载该后台控制页。
- 扩展重载可能清除 session 所有权。此时不重新标记或操作旧页面；只有原 controller 创建已经明确 opened、完整原生库存稳定、无已登记 controller owner、无未决执行与控制创建预约，才预约一次新的空白控制窗口。窗口 create 使用 focused=false、state=minimized、incognito=false；旧 opened 回执与页面保留。
- 控制创建预约先持久化，丢失创建回执或状态漂移保持 held，不重复开窗口。图片 generation、用户页面输入、会话与视频均不属于该守护权限。
- 后台图片队列按原任务是否已有远端 ID、nextAttemptAt 排序，避免多次到期的前几项挤占未触及任务；已知原任务和未知提交保护仍优先。

## 实际验证

- JS 全部 106 passed、0 failed；含 session 丢失新建一次控制窗、保留旧页与回执、拒绝未知操作、创建回执不明确不重试、后台 tick 串行、原 native inventory/恢复/部署回归。
- Rust background_chrome_ 专项 33 passed、0 failed；含等待任务不能挤占未触及任务，zero generation、父资产链、未知提交与原请求保护。
- pinned overlay 共 18 文件；原生部署收据为 success=true，2 文件更新，stateUntouched=true。manifest SHA256 `cf8eaaaea091f9f1598f8979a0e94200b5f06d424e3649173ff6d46124f8ec58`。
- release BuildOnly 与 PublishOnly 通过；当前客户端 SHA256 `b7428a0c33e6a95bdcd04fad48bc77c4fdccb4c4c6f72a72666965d1be44dc63`，唯一备用 `bf0183be4845dc2e525fba83aeb89f3abb4fff83f24ca8e35e71fa18bfa3aff4`。

上述结果是代码、组件部署与原生构建验证，不等于 Chrome 已加载最新补丁或真实图片批次通过。现场连接仍为零；原批次 29 项中五张已下载采用，余二十四项未完成。真实生成只用已确认套餐内无逐张附加费用的 Chrome；所有视频提交禁止。保持原请求、任务、回包与私人证据，不把运行正文提交仓库。

## 原生存储回读修正

后台只读 Chrome 本机扩展配置与白名单 controller 元数据，确认加载目录与绑定 profile 正确，恢复预约已到 reserved；未读取 cookies 或输出凭据。实际 Chrome dictionary 回读调整字段顺序，v1 直接比较 JSON.stringify 导致第一条创建前守卫误拒绝。

持久预约回读与双采样改为递归 canonical 字段比较和哈希，保留内容漂移拒绝。只对 pinned v1、完整同值预约、原 opened 回执未变、观察到与 v1 固定输入不同的回读字段顺序、无注册 controller owner/未决执行的“创建前回读守卫拒绝”保留原预约证明并修正；v2、未知创建、内容变化或原状态未明确仍 held。历史 controller 列表采用平面追加，不重复嵌套整个旧历史。

新增真实 Chrome 字段排序语义的合成回归，JS 全套 109 passed、0 failed。原生 overlay 更新收据 success=true，manifest SHA256 `4365f741c287541dc310da1c6d2fd9cb60e962e5f926e5fad7bf1168bb805d1f`。这不是完成二十四张真实图片的证据，继续核验最新扩展加载与原批次。
