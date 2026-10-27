# Qoder 双区域每日 Credits 协议与验收

日期：2026-10-27。范围为 Jolu 的 Qoder 国内、国际版每日领取适配。研究与验收均未发送真实领取 POST，未登录、刷新令牌或改写原工具配置。

## 结论与官方规则

两区都存在每日主动领取 100 通用 Credits 的官方活动，个人免费、试用和付费账号可以参与；企业订阅不适用。每日 UTC+8 10:00 开新一轮，持续至次日 10:00 前，每轮一次，不能补领。每笔奖励有效 30 天，领取入口在桌面端。

- 国内规则：<https://docs.qoder.cn/events/100credits>
- 国际规则：<https://docs.qoder.com/zh/events/100credits>

`period_key` 使用官方 HTTP `Date` 按 UTC+8 10:00 换轮，不按本机日期或午夜自然日；`reset_at` 为下一次 10:00 的 Unix 秒，`server_time` 保存响应 Date 的 Unix 秒。凌晨查询显示的“本轮已领取”不能阻止上午新一轮再次查询。成功活动响应的 Date 缺失、重复、无效或非 UTC 时返回未知，不猜轮次，也不使用本机时间覆盖服务端活动状态；HTTP 认证或权限错误仍保留其明确状态。

## 官方代码证据

只读读取本机官方发行包的 ASAR 目录及代码，未读取设计文件：

- `D:/Qoder/resources/app.asar`：`package.json` 标记 `qoder 0.3.4`，`out/main/index.js`。
- `D:/Qoder CN/resources/app.asar`：`package.json` 标记 `qoder-cn 0.3.4`，`out/main/index.js`。
- 国内卸载注册表显示 `0.4.8`，与 ASAR 内部版本有差异，不能把注册表版本视为内部 JS 版本。
- 两区主进程均包含 `campaignMainService`、`nativeCampaignRequestMainService` 和对应区域端点配置。
- 官方公开活动页面：`https://openapi.qoder.com.cn/growth-page/activity-iframe`、`https://openapi.qoder.sh/growth-page/activity-iframe`。
- 页面引用的官方公开脚本：<https://g.alicdn.com/qbase/qoder/0.0.739/growth-page/activity-iframe/activity-iframe.js>。
- 官网路由对应脚本：<https://g.alicdn.com/qbase/qoder/0.0.739/1869.js>。

公开脚本副本仅保存在 Git 忽略的 `output/research-qoder-checkin/`。当前官方代码未出现旧 `daily-check-in` 路径，未采用第三方项目中的旧接口。

Context7 已先执行 `library Qoder`，再执行 `docs /websites/qoder`，确认每日活动规则；认证和活动接口未在其文档中提供，以上官方发行代码作为直接证据。`reqwest` 同样先 `library reqwest` 再 `docs /websites/rs_reqwest`；并核对本机 `reqwest 0.12.28` 的 `retry` 方法。

## 请求与响应

| 区域 | 固定 API 主机 | 本机登录目录 |
|---|---|---|
| 国内 | `https://openapi.qoder.com.cn` | `%APPDATA%/com.qodercn.app.stable` |
| 国际 | `https://openapi.qoder.sh` | `%APPDATA%/com.qoder.app.stable` |

| 操作 | 方法与路径 | 请求体 |
|---|---|---|
| 活动与领取状态 | `GET /sash/api/v1/me/campaigns` | 无 |
| 领取指定活动 | `POST /sash/api/v1/me/campaigns/{campaignId}/claim` | 无 |
| 奖励结果 | `GET /sash/api/v1/me/campaigns/{campaignId}/reward` | 无；当前 Credits 适配不需要调用 |

状态响应包含 `campaigns`；官方页面还兼容顶层数组、`data.campaigns`、`data` 数组。活动字段包含 `campaignId`、`campaignKey`、`actionType`、`claimStatus`、`startAt`、`endAt`、`benefit`、`placements`。

`claimStatus=CLAIMABLE` 表示可领，`CLAIMED` 表示已领。领取响应须 HTTP 2xx 且顶层或 `data.status=CLAIMED` 才算成功，不能仅凭 HTTP 200。HTTP 409 表示资格不足或活动结束，不作为重复成功；401/403 分别表达认证失败/无权限，429/503 表达繁忙。请求发出后网络结果不明时返回未知，重新读取状态，不自动重试领取。

## 真实只读样本与每日活动选择器

显式运行忽略的 `qoder_desktop::tests::checkin_campaign_read_only_probe`，使用原有 `local_session` 在后端内存取得认证，GET 两区固定地址。只输出白名单活动字段，没有输出服务端 `uid`、本机用户标识、账号资料或任何令牌。

- 国内 HTTP 200，返回一个每日活动，`claimStatus=CLAIMED`。
- 国际本机未登录，没有向国际区发送真实认证请求；不得声称国际真实账号验收通过。

国内样本明确返回：

```json
{
  "actionType": "CLAIM_BENEFIT",
  "benefit": {
    "kind": "CREDITS",
    "amount": 100,
    "modelScope": {"modelSeries": {"key": "ALL_MODELS"}},
    "validity": {"mode": "RELATIVE_DAYS", "days": 30}
  },
  "claimStatus": "CLAIMED",
  "placements": [{
    "type": "POPUP",
    "content": {
      "zh": {
        "title": "每天领 100 Credits",
        "description": "每日 10:00（UTC+8）刷新，领取后 30 天有效",
        "detailUrl": "https://docs.qoder.cn/events/100credits"
      },
      "en": {
        "title": "Claim 100 Credits Daily",
        "detailUrl": "https://docs.qoder.cn/en/events/100credits"
      }
    }
  }]
}
```

适配器不硬编码样本中的活动 ID/key，也不使用活动标题模糊匹配。每日选择器同时要求：

1. `placements` 中的官方规则链接为本区域的 HTTPS `docs.qoder.cn` 或 `docs.qoder.com`，路径为 `/events/100credits`，或对应 `/en/`、`/zh/` 语言路径。拒绝伪造后缀域名、其他主机、HTTP 和其他活动路径。
2. `actionType=CLAIM_BENEFIT`，奖励为 `CREDITS`、100、`ALL_MODELS`、`RELATIVE_DAYS`、30 天。
3. 活动 ID 合法、起止时间有效，官方响应 Date 位于活动窗口内。
4. 只有一个明确候选，且状态为 `CLAIMABLE`。

未知活动、其他 Credits 活动不领取；每日活动关键字段缺失、规则变化、多个候选返回未知；`showCampaign=false` 尊重服务端停用信号；空活动列表返回当前不可领取，不冒充已签到。国际选择器依据对应国际官方规则页与相同官方活动脚本，但其真实响应形状仍缺账号验证。

## 认证与设备信息

复用 `qoder_desktop::Session`：本机读取现有安全存储，浏览器连接使用该连接自己的 Session。适配器要求有效用户标识及令牌，已提供的到期时间必须可识别且尚未到期。本机每次请求前重新核对用户标识和令牌，变化时终止。稳定身份取服务端用户 ID，公共层再按平台/区域哈希去重；不使用含令牌的 `source_identity`。

请求发送 `Authorization: Bearer …`、`Cosy-ClientType: 10`、`Accept: application/json`、`User-Agent: Qoder`。本次国内 GET 已证明这些字段能读取当前每日活动。

官方主进程还支持 `Cosy-Version`、`Cosy-MachineOS`、`Cosy-MachineHostname` 及机器风控字段。后者由 `resources/umid/runtime-info.exe` 获取：国内参数 `0 --account-stdin`，国际参数 `3 --account-stdin`，输入后端用户 ID，输出 `machineToken/machineType/machineCode`；另有原工具的 `auth.machine-id`。本次适配未伪造设备字段，也未执行该程序。缺少设备字段是否影响国际活动资格尚未真实验证；读取不到活动时如实显示不可领取。

## 改动总结

- 新增 `backend/src/checkin_qoder.rs`：国内/国际账号构造、稳定身份、真实状态查询、严格每日筛选、领取前再次查询、一次领取请求与分状态结果。
- 网络复用 Jolu 联网方式，20 秒超时、禁重定向、禁请求自动重试、1 MiB 响应上限；不回显原始服务端响应或网络错误。
- `backend/src/qoder_desktop.rs` 仅新增显式忽略的 GET-only 脱敏协议探测测试，不改变现有额度或登录逻辑。
- 后续独立审查修正了活动时间误用本机时钟的问题：GET/POST 都提取 HTTP Date，缺少可靠时间时保持未知。
- `Account::claim` 接收 `before_post` 回调；内部活动 GET 完成、领取 RequestBuilder 准备完毕之后，在实际 `send` 前执行最终连接快照及真实身份校验。失败时直接返回，发送次数为零。

## 验证

- 六项前置测试先在 stub 上全部失败，之后实现转绿：每日与区域筛选、非每日/未知链接拒领、字段缺失/多个候选、409 与成功区别、UTC+8 10:00 边界、登录有效性与稳定身份。随后补充服务端停用信号回归测试，先观察失败再修复。
- 初版 `cargo test -q --manifest-path backend/Cargo.toml checkin_qoder::tests -- --include-ignored --nocapture`：8 通过（7 个合成测试、1 个显式运行的真实只读适配器验收），0 失败。
- 原始只读协议探测：国内 HTTP 200、每日活动已领取；国际本机未登录。
- 真实适配器 `Account.status`：国内 `state=Claimed`、`reward=100`、`period_key=2026-10-26`、`reset_at=1793066400`。当时本机北京时间已到 27 日凌晨，仍归属 26 日 10:00 开始的一轮，验证了非午夜边界。国际返回 `Unauthorized`，没有跨区回退或发送领取请求。
- 领取前再次读取服务端状态，不再与本机当前时间比较。POST 成功响应的服务端 Date 若跨过预检 GET 所属轮次，返回未知并要求刷新确认，不把结果写入猜测的轮次。
- 时钟与最终发送门禁修复新增四项回归：可靠 Date 解析、服务端时间决定活动窗口/缺 Date 保持未知、校验失败发送次数为零、POST 时间及跨 10:00 结果。四项先观察失败，再修复转绿；最终同命令 12 通过（11 个合成测试、1 个真实只读验收），0 失败。真实状态仍为国内已领取、国际未授权。
- 未验证真实领取 POST、国际真实账号或企业样本。未修改 Git 状态。
