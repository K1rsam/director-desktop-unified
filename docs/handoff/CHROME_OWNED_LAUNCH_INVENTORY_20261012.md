# Chrome 专属任务页只读清单合同

## 当前实现与测试

本轮保留已提交的原图最终身份修复。暂停的 release 协议草稿已保存于私有 `.local/validation/chrome-batch-20261012/parked-release-protocol/`，其 server 改动及两份未部署 upstream 草稿已精确撤回，没有修改旧任务、原回包或历史失败。

新增清单通过现有 `browser.tab.identify` 的 `owned_launch_inventory_v1` 专用模式，在 content 中调用已有 `bridge.tab.open` 的 `lookupOnly=true` 分支。该分支在任何 create/navigate/close 之前返回，只读取 Chrome tabs 和 session launched records。查询会产生正常协议命令回执，业务状态、媒体、认证与页面不修改；不是严格零元数据写入合同。

当前 component 为 6.4.0；extension 实际包版本由 `chrome.runtime.getManifest().version` 读取，本机基准为 2.4.5。实际 bundle、background nonce 和当前 server nonce 从原 runtime 获取，profile ID 只读取已有 local storage，不创建或轮换。

`node --test apps/chrome-bridge/tests/*.test.cjs`：62 通过，0 失败/取消/忽略。新增7项覆盖不依赖 connected clients 的完整 absence、存在但未连接的原页、重复 token、孤立 record、枚举错误、跨 profile/bundle/background/server/source runtime、读取期间清单变化、RPC correlation ID 隔离、服务器 SHA/身份复核与用户页拒绝。尚未部署或执行真实清单查询，不代表旧7项已恢复。

## HTTP 入口

`GET /browser/owned-launch/<requestId>`，仅 localhost，受现有 API token 鉴权保护。

查询参数全部必需：`sourceClientId`、`profileId`、`bundleId`、`backgroundEpoch`、`serverInstanceId`。目标 launchToken 只能从 UUIDv4 requestId 推导为 `bridge-director-desktop-<requestId>`，不能自选任意用户页。

source 必须是本程序专属、hidden、未聚焦、idle、非隐身、已连接兼容 root 页面，URL 精确为 `https://chatgpt.com/`。其 launchToken 为合法 UUIDv4 desktop token，或 `bridge-director-control-<当前 profileId>`。服务器 before/after 核对绑定；后台另用实际 Chrome tab 状态与持久 launch record 核对 source 身份。

完整成功响应：

```json
{
  "ok": true,
  "version": "director-owned-launch-inventory/v1",
  "status": "matched | confirmed_absent",
  "reason": "unique_record_and_live_tab | complete_inventory_no_matching_launch",
  "requestId": "UUIDv4",
  "launchToken": "bridge-director-desktop-UUIDv4",
  "complete": true,
  "componentVersion": "6.4.0",
  "runtime": {
    "profileId": "UUIDv4",
    "bundleId": "32 lowercase hex",
    "backgroundEpoch": "actual current nonce",
    "serverInstanceId": "actual current nonce",
    "sourceClientId": "exact client id",
    "sourceTabId": 1,
    "extensionVersion": "actual installed manifest version",
    "contentEpoch": "actual source content nonce"
  },
  "inventoryGeneration": "new UUID",
  "inventorySha256": "64 lowercase hex",
  "inventoryCanonicalJson": "exact UTF8 JSON string of inventory",
  "inventory": {
    "tabs": [{"id": 1, "urlSha256": "64 hex", "incognito": false, "active": false, "discarded": false}],
    "launchRecords": [{"key": "chatgptBridgeLaunchedTab:1", "recordSha256": "64 hex"}],
    "sourceTabId": 1
  },
  "observedAt": 0,
  "absenceScope": "current_browser_inventory_only",
  "sourceClientId": "exact client id",
  "match": {"tabId": 2, "url": "original owned URL", "launchToken": "exact token", "createdAt": 0, "serverUrl": "original loopback URL", "connected": false}
}
```

`match` 仅 matched 时出现，`connected=false` 仍表示 Chrome 原页存在。runtime.sourceTabId 对应 clients.browserTabId，runtime.sourceClientId 对应 clients.id。launchRecords 是全 session launched records 的摘要，包含 controller；confirmed_absent 不能要求该数组为空。

hash 算法是 SHA256(`inventoryCanonicalJson` 的 UTF8 字节)。Rust 应解析该字符串与 inventory Value 作等值校验，再对原字符串 hash；不能重新序列化 Value 后猜测 JS key 顺序。

unknown 响应至少包含 `ok=true`、version、status=`unknown`、reason、requestId、complete=false；不保证完整 runtime/hash。unknown 不作为 first-open、重发或清除历史记录的证据。

## 证明边界

后台使用两次完整 `chrome.tabs.query({})` 与全 session launched records；两份清单及 actual runtime nonce 必须相等。唯一 record 与 live tab 一致时 matched；完全没有该 launch record 时 confirmed_absent。重复 token、record 指向不存在的 tab、server 不符、damaged metadata、跨 runtime、枚举或哈希阶段失效均 unknown。无关页面仅返回 URL 摘要，不返回 URL 或标题。

confirmed_absent 只证明本次实际浏览器清单没有可绑定的目标 launch；它不是旧 open command 的 `proved_not_started`、历史 nonce 或未知提交清除证明。已有首次 open intent 超时后，不因 absence 再次 open。旧 legacy 无 intent 首次接管须另行核对原本地未提交证据、原 turn/canonical absence、渠道 idle，并持久冻结新的执行身份与一次 open 意图。

指定 requestId 的既有 `/diagnostics/request-state` 返回 `requests={state:null|canonical,history:[],deadlines:[]}` 对象；只有不指定 requestId 时才为数组。不得拿离线 fixture 中的 `requests=[]` 作为 live 单任务 absence 契约。
