/**
 * 后端调用封装。
 *
 * 浏览器里（`pnpm dev` 预览）没有 Tauri，所有命令会被标记为不可用，
 * 调用方回落到 `mock/data.ts` 的设计示例——预览页因此仍然可用。
 */

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window

/**
 * 额度类查询的新鲜度：`true` 为用户显式刷新（跳过成功缓存）；`{ maxAgeMs }` 为定时刷新——
 * 后端缓存不超过这么久就复用，多个窗口同一周期只打一次上游，又不会把刷新间隔拖长。
 */
export type Freshness = boolean | { maxAgeMs: number }

export function freshnessArgs(value: Freshness): { force: boolean; maxAgeMs?: number } {
  return typeof value === "boolean" ? { force: value } : { force: false, maxAgeMs: Math.max(0, Math.round(value.maxAgeMs)) }
}

export const localCodexQuota = (force: Freshness = false) => invoke<QuotaStateDto>("local_codex_quota", freshnessArgs(force))

export const localGrokQuota = (force: Freshness = false) => invoke<QuotaStateDto>("local_grok_quota", freshnessArgs(force))
export const localZcodeQuota = (force: Freshness = false) => invoke<QuotaStateDto>("local_zcode_quota", freshnessArgs(force))

/** 各模块的 API 文件（如 lib/automationApi.ts）复用同一个桥接 */
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) throw new Error("not-tauri")
  const { invoke } = await import("@tauri-apps/api/core")
  return invoke<T>(cmd, args)
}

export interface SourceMetrics {
  credits: number | null
  credit_requests: number
  context_ratio: number | null
  context_at_ms: number | null
  /** 区间思考 / 推理 Token 合计；来源不提供时为 null */
  reasoning: number | null
  /** 区间工具调用提示 Token 合计（Gemini tokens.tool）；来源不提供时为 null */
  tool: number | null
}

export interface ActiveSession {
  session_id: string
  title: string
  delta_tokens: number
  last_seen_ms: number
  started_at_ms?: number | null
  running_tokens?: number | null
  state: "running" | "done" | "failed" | "recent"
}

export interface LiveUsageDto {
  platform: string
  cursor: number
  delta_tokens: number
  delta_sessions: ActiveSession[]
  active_sessions: ActiveSession[]
  running_tokens?: number | null
  active_window_seconds: number
  today_tokens: number | null
  realtime_delta: boolean
  initial: boolean
  /** 本平台统计自上次推送后有变化；仅续期运行中会话的心跳为 false，统计视图据此跳过重查 */
  data_changed?: boolean
}

/** 接入方式只有官方订阅（Auth）与 API Key 两种 */
export interface ConnectionInfo {
  kind: "auth" | "api" | null
  label: string | null
}

/** 连接（§6.3）。**后端刻意不返回凭证原值**，只有 masked */
export interface ConnectionDto {
  id: string
  platform: string
  kind: "auth" | "api"
  name: string
  label: string
  masked: string
  status: "connected" | "expired" | "invalid" | "offline" | "paused"
  last_sync_text: string | null
  /** sub2api 等自建网关地址；地址不是凭证，可回显 */
  base_url: string | null
  /** 手动添加时保存的默认参数；本机读取与历史连接为 null */
  model: string | null
  effort: string | null
  context_1m: boolean | null
}

export interface FetchResult {
  ok: boolean
  masked: string | null
  message: string
  /** 扫描过的路径与环境变量，界面需说明读取范围 */
  scanned: string[]
  /** 本次是否真的写入了变更；按钮据此区分「已更新」与「已是最新」 */
  changed: boolean
}

/** 本机自动发现的连接候选，用于「+ 添加连接」预填 */
/** 「检测本机登录」结果：账号与套餐未返回时为 null，界面显示「未获取」 */
export interface LoginDetectionDto {
  found: boolean
  masked: string | null
  account: string | null
  plan: string | null
  sources: string[]
  message: string | null
}

export interface Candidate {
  base_url?: string | null
  platform: string
  kind: "auth" | "api"
  suggested_name: string
  masked: string
}

/** 自定义时间范围（§2.3）。end 为 null 表示跟随当前时刻 */
export interface CustomRange {
  start: number
  end: number | null
}

export interface ScanResult {
  files_scanned: number
  records_inserted: number
  claude_found: boolean
  codex_found: boolean
  errors: string[]
}

export interface CollectionStatusDto {
  ok: boolean
  errors: string[]
  last_success_ms: number | null
  /** 出错文件所属的平台（claude / codex）：采集异常按平台定界，不跨平台污染会话状态 */
  failed_sources: string[]
  /** 连续失败轮数；单次瞬时错误（下一轮心跳自愈）不置异常 */
  consecutive_failures: number
}

export interface DataInfo {
  path: string
  size_bytes: number
}

export type MainIntentDto =
  | { kind: "overview"; settings: AppSettings }
  | { kind: "settings"; section: string }
  | { kind: "about"; version: string }

/** 在系统默认浏览器打开链接。桌面端走 opener 插件，浏览器预览用新标签 */
export async function openUrl(url: string): Promise<void> {
  if (!isTauri) {
    window.open(url, "_blank", "noopener,noreferrer")
    return
  }
  const { openUrl: open } = await import("@tauri-apps/plugin-opener")
  await open(url)
}

/** 一个额度窗口。usedPercent 为 null 时**不画进度条**（§7.4） */
export interface QuotaWindowDto {
  key: string
  window_name: string
  used_percent: number | null
  amount_text: string | null
  remaining_text?: string | null
  resets_at: string | null
}

/**
 * 额度查询状态（§2.5）。五种失败态各带原因，界面显示「—」+ 原因，
 * 绝不渲染为 0% 或满格。
 */
export type QuotaStateDto =
  | { state: "ok"; windows: QuotaWindowDto[]; plan: string | null }
  | { state: "unsupported"; reason: string }
  | { state: "forbidden"; reason: string }
  | { state: "unauthorized"; reason: string }
  | { state: "rate_limited"; reason: string }
  | { state: "failed"; reason: string }

/** 余额查询状态。真实余额为零显示 0.00，查不到才显示「—」 */
export type BalanceStateDto =
  | { state: "ok"; balance: number; currency: string; used: number | null }
  | { state: "unsupported"; reason: string }
  | { state: "forbidden"; reason: string }
  | { state: "unauthorized"; reason: string }
  | { state: "rate_limited"; reason: string }
  | { state: "failed"; reason: string }

/** 直连 API Key 的官方组织用量。普通 Key 无管理权限时返回 forbidden。 */
export type ApiUsageStateDto =
  | {
      state: "ok"
      input_tokens: number
      output_tokens: number
      cache_read_tokens: number
      cache_write_tokens: number
      total_tokens: number
      cost_usd: number | null
      cost_reason: string | null
      source: string
    }
  | { state: "unsupported"; reason: string }
  | { state: "forbidden"; reason: string }
  | { state: "unauthorized"; reason: string }
  | { state: "rate_limited"; reason: string }
  | { state: "failed"; reason: string }

/** 估算费用（§2.4）。estimated 恒为 true —— 本地记录不含账单 */
export interface CostEstimateDto {
  amount: number
  estimated: boolean
  /** 价目表未覆盖模型的 Token 数；不为零时界面需说明 */
  uncovered_tokens: number
  complete: boolean
}

/** 停靠边缘（§2.1.3） */
export type DockEdgeDto = "top" | "bottom" | "left" | "right"

/** 灵动岛分身（画布 22）：本体仍用 AppSettings 的 island_* 与 dock 字段 */
export interface IslandCloneDto {
  id: string
  platform: string
  kind: "auth" | "api"
  connection_id: string | null
  connection_name: string | null
  source_id: string | null
  dock: { edge: DockEdgeDto | null; offset: number; monitor: string | null }
  /** 自由态左上角物理像素；null 表示尚未记录 */
  position: [number, number] | null
}

/** 停靠事件带岛 id（本体为 null），各岛只处理自己的 */
export interface DockChangedEvent {
  island: string | null
  dock: { edge: DockEdgeDto | null; offset: number; monitor: string | null }
}

export interface DockHintEvent {
  island: string | null
  hint: { edge: DockEdgeDto; offset: number; centered: boolean } | null
}

/** 右键菜单的来源：托盘或某个灵动岛（clone 为分身 id，本体为 null） */
export interface MenuSourceEvent {
  island: boolean
  clone: string | null
}

/** 持久化设置（§2.6 / §7.3）。托盘与设置界面共享同一份值 */
export interface AppSettings {
  silent_startup: boolean
  island_platform: string
  island_kind: "auth" | "api"
  island_connection_id: string | null
  island_connection_name: string | null
  island_source_id: string | null
  always_on_top: boolean
  theme: "light" | "dark" | "system"
  island_opacity: number
  island_scale: number
  island_shrink_scale: number
  refresh_minutes: number
  balance_alert_threshold: number | null
  balance_alert_currency: string
  dock_enabled: boolean
  retention_days: number | null
  dock: { edge: DockEdgeDto | null; offset: number; monitor: string | null }
  /** 免打扰：只暂停提示与动效，不停止采集与统计 */
  dnd: boolean
  island_visible: boolean
  /** 分身列表；旧版设置缺少此字段时为空数组 */
  island_clones: IslandCloneDto[]
  /** 本地代理：默认关闭，不开启时应用保持纯只读采集行为 */
  proxy_enabled: boolean
  proxy_port: number
  proxy_fallback_direct: boolean
  /** 接管时保存的真实上游（网关地址，非凭证） */
  proxy_claude_upstream: string | null
  proxy_codex_upstream: string | null
}

export interface ProxyStatusDto {
  running: boolean
  port: number
  fallback_reason: string | null
  last_error: string | null
  claude_upstream: string | null
  codex_upstream: string | null
  claude_taken_over: boolean
  codex_taken_over: boolean
}

export interface CleanupPreviewDto {
  cutoff_ms: number
  requests: number
  usage_events: number
  sessions: number
}

export interface ImportSummaryDto {
  /** 新导入的请求条数；重复项不再合并更新 */
  requests_changed: number
  sessions_changed: number
  /** 跳过的条数：重复项 + 无效项 */
  skipped: number
}

/** 导入前预览（画板 100 · 05）；token 10 分钟内有效，确认导入时传回 */
export interface ImportPreviewDto {
  token: string
  file_name: string
  valid: number
  duplicate: number
  invalid: number
  /** 无效项原因，最多 20 条 */
  invalid_reasons: string[]
}

export interface UpdateCheckDto {
  available: boolean
  current_version: string
  available_version: string
  /** Release notes 摘要；发布未提供时为 null */
  notes: string | null
  pub_date: string | null
}

/** download_app_update 期间由后端 emit 的下载进度（事件名 update-download-progress） */
export interface UpdateProgressDto {
  downloaded: number
  total: number | null
}

/** 通用事件订阅。返回取消订阅函数 */
export async function listenEvent<T>(
  name: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (!isTauri) return () => {}
  const { listen } = await import("@tauri-apps/api/event")
  return listen<T>(name, (e) => handler(e.payload))
}

/**
 * 订阅后端推送的实时用量。Rust 侧监听 jsonl 写入，写入即推送，
 * 因此这里不做任何定时轮询。返回取消订阅函数。
 */
export async function listenLiveUsage(
  handler: (u: LiveUsageDto) => void,
): Promise<() => void> {
  return listenEvent<LiveUsageDto>("live-usage", handler)
}

export const api = {
  sourceMetrics: (platform: string, period: string, custom: CustomRange | null, model: string | null, queryEndMs: number) => invoke<SourceMetrics>("source_metrics", { platform, period, custom, model, queryEndMs }),
  mainPanelReady: () => invoke<void>("main_panel_ready"),
  takeMainIntent: () => invoke<MainIntentDto | null>("take_main_intent"),
  menuAction: (id: string) => invoke<void>("menu_action", { id }),
  menuFit: (rootHeight: number) => invoke<{ root_x: number; root_y: number; root_width: number; sub_width: number; side: "left" | "right" }>("menu_fit", { rootHeight }),
  menuShow: () => invoke<void>("menu_show"),
  menuClose: () => invoke<void>("menu_close"),
  menuRefreshing: () => invoke<boolean>("menu_refreshing"),
  islandMenu: (refreshing: boolean) => invoke<void>("island_menu", { refreshing }),
  islandDrag: () => invoke<void>("island_drag"),
  mainDrag: () => invoke<void>("main_drag"),
  islandTopmost: (on: boolean) => invoke<AppSettings>("island_topmost", { on }),
  setDockEnabled: (on: boolean) => invoke<AppSettings>("set_dock_enabled", { on }),
  /** 显示 / 隐藏本体与全部分身，与托盘同一个值 */
  setIslandVisibility: (visible: boolean) => invoke<AppSettings>("set_island_visibility", { visible }),
  /** 显示位置：edge 为 null 表示自由移动；island 为 null 表示本体 */
  islandPosition: (edge: DockEdgeDto | null, island: string | null) => invoke<void>("island_position", { edge, island }),
  /** 灵动岛页操作：开启分身时 island 为来源窗口，销毁时为目标窗口 */
  islandAction: (id: "refresh" | "reset_layout" | "clone_create" | "clone_destroy", island: string | null = null) =>
    invoke<void>("island_action", { id, island }),
  setTheme: (theme: "light" | "dark" | "system") => invoke<AppSettings>("set_theme", { theme }),
  setBalanceAlert: (threshold: number | null, currency: string) => invoke<AppSettings>("set_balance_alert", { threshold, currency }),
  setDisplayPreferences: (opacity: number, scale: number, shrinkScale: number, refreshMinutes: number) => invoke<AppSettings>("set_display_preferences", { opacity, scale, shrinkScale, refreshMinutes }),
  setRetentionDays: (days: number | null) => invoke<AppSettings>("set_retention_days", { days }),
  cleanupPreview: (days: number) => invoke<CleanupPreviewDto>("cleanup_preview", { days }),
  cleanupHistory: (days: number) => invoke<CleanupPreviewDto>("cleanup_history", { days }),
  readLocalConnections: (platform: string, kind: "auth" | "api", name?: string | null) => invoke<{ added: number; existing: number; warnings?: string[] }>("read_local_connections", { platform, kind, name: name ?? null }),
  /** 选择备份文件并预览；用户取消选择时为 null */
  previewImportFile: () => invoke<ImportPreviewDto | null>("preview_import_file"),
  confirmImport: (token: string) => invoke<ImportSummaryDto>("confirm_import", { token }),
  /** 取消 / 关闭预览：释放后端暂存的备份 */
  discardImport: (token: string) => invoke<void>("discard_import", { token }),
  /** 日期为 [startMs, endMs) 毫秒区间，都传 null 表示全部日期 */
  exportDataFile: (platform: string | null, startMs: number | null, endMs: number | null) =>
    invoke<string | null>("export_data_file", { platform, startMs, endMs }),
  scanLocalSessions: () => invoke<ScanResult>("scan_local_sessions"),
  collectionStatus: () => invoke<CollectionStatusDto>("collection_status"),
  dataInfo: () => invoke<DataInfo>("data_info"),
  openDataDirectory: () => invoke<void>("open_data_directory"),
  autostartStatus: () => invoke<boolean>("autostart_status"),
  setAutostart: (on: boolean) => invoke<boolean>("set_autostart", { on }),
  /** 只查询是否有新版本，不下载（§更新：启动 1 秒后与设置页手动检查共用） */
  checkAppUpdate: () => invoke<UpdateCheckDto>("check_app_update_available"),
  /** 下载并验证签名，返回本次下载的版本 */
  downloadAppUpdate: () => invoke<UpdateCheckDto>("download_app_update"),
  /** 安装已下载的安装包；Windows 上安装器接管并自动重启 */
  installDownloadedUpdate: () => invoke<void>("install_downloaded_update_and_restart"),
  setSilentStartup: (on: boolean) => invoke<AppSettings>("set_silent_startup", { on }),
  connectionQuota: (id: string, force: Freshness = false) => invoke<QuotaStateDto>("connection_quota", { id, ...freshnessArgs(force) }),
  connectionApiUsage: (id: string, force: Freshness = false) => invoke<ApiUsageStateDto>("connection_api_usage", { id, ...freshnessArgs(force) }),
  connectionBalance: (id: string, force: Freshness = false) => invoke<BalanceStateDto>("connection_balance", { id, ...freshnessArgs(force) }),
  costEstimate: (
    platform: string,
    period: string,
    custom: CustomRange | null,
    model: string | null,
    queryEndMs: number,
    connection: string | null = null,
  ) => invoke<CostEstimateDto>("cost_estimate", { platform, period, custom, model, queryEndMs, connection }),
  dockUndock: () => invoke<void>("dock_undock"),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setIslandPlatform: (platform: string) =>
    invoke<AppSettings>("set_island_platform", { platform }),
  /** island 为目标灵动岛的分身 id；null 为本体 */
  setIslandConnection: (id: string | null, island: string | null = null) =>
    invoke<AppSettings>("set_island_connection", { id, island }),
  setIslandSource: (id: string | null) =>
    invoke<AppSettings>("set_island_source", { id }),
  setDnd: (on: boolean) => invoke<AppSettings>("set_dnd", { on }),
  proxyStatus: () => invoke<ProxyStatusDto>("proxy_status"),
  /** 开关切换即执行 CLI 配置接管（或还原直连）并启动（或停止）监听；失败原因直接抛出 */
  setProxy: (port: number, enabled: boolean, fallbackDirect: boolean) =>
    invoke<AppSettings>("set_proxy", { port, enabled, fallbackDirect }),
  listConnections: () => invoke<ConnectionDto[]>("list_connections"),
  addConnection: (input: {
    platform: string
    kind: "auth" | "api"
    name: string
    secret?: string | null
    base_url?: string | null
    model?: string | null
    effort?: string | null
    context_1m?: boolean | null
  }) => invoke<string>("add_connection", { input }),
  /** 「获取模型」：用用户填写的地址与 Key 拉取网关模型列表（画布 18）。Tauri 参数走 camelCase */
  fetchRemoteModels: (platform: string, base_url: string, secret: string) =>
    invoke<string[]>("fetch_remote_models", { platform, baseUrl: base_url, secret }),
  setConnectionPaused: (id: string, paused: boolean) => invoke<void>("set_connection_paused", { id, paused }),
  /** 启用连接：把凭证与地址写入对应 CLI 的配置文件（cc-switch 式切换），返回说明文案 */
  enableConnection: (id: string) => invoke<string>("enable_connection", { id }),
  /** 统一编辑连接：完整快照式更新；secret 空串表示不换 Key，地址或 Key 变化时后端先验证 */
  updateConnection: (input: {
    id: string
    name: string
    base_url?: string | null
    secret?: string | null
    model?: string | null
    effort?: string | null
    context_1m?: boolean | null
  }) => invoke<void>("update_connection", {
    id: input.id,
    name: input.name,
    baseUrl: input.base_url ?? null,
    secret: input.secret ?? null,
    model: input.model ?? null,
    effort: input.effort ?? null,
    context1m: input.context_1m ?? null,
  }),
  removeConnection: (id: string) => invoke<void>("remove_connection", { id }),
  fetchCredentials: (id: string) => invoke<FetchResult>("fetch_credentials", { id }),
  replaceApiKey: (id: string, secret: string) => invoke<void>("replace_api_key", { id, secret }),
  renameConnection: (id: string, name: string) => invoke<void>("rename_connection", { id, name }),
  discoverConnections: () => invoke<Candidate[]>("discover_connections"),
  /** 只读检测本机官方客户端登录，不入库；保存走 readLocalConnections */
  detectLocalLogin: (platform: string) => invoke<LoginDetectionDto>("detect_local_login", { platform }),
  /** 本机 CLI 当前在用的连接 id（每个平台至多一条） */
  currentConnections: () => invoke<string[]>("current_connections"),
  connectionKind: (platform: string) =>
    invoke<ConnectionInfo>("connection_kind", { platform }),
  liveUsage: (platform: string, since: number | null) =>
    invoke<LiveUsageDto>("live_usage", { platform, since }),
  /** 用量统计（模块 C）：概览与用量分析 */
  statsOverview: (filter: StatsFilterDto) => invoke<StatsOverviewDto>("stats_overview", { filter }),
  /** 全年活跃日历：只按供应商与账号筛选 */
  statsActivity: (platform: string | null, connections: string[], nowMs: number) =>
    invoke<StatsActivityDto>("stats_activity", { platform, connections, nowMs }),
  statsCalls: (filter: StatsFilterDto, page: number) => invoke<StatsCallPageDto>("stats_calls", { filter, page }),
  /** 按当前筛选导出 CSV；取消保存返回 null */
  exportCallsCsv: (filter: StatsFilterDto) => invoke<{ path: string; count: number } | null>("export_calls_csv", { filter }),
  /** 会话线程（模块 D） */
  sessionsList: (filter: SessionFilterDto) => invoke<SessionListDto>("sessions_list", { filter }),
  sessionMessages: (platform: string, sessionId: string, before: number | null) =>
    invoke<MessagePageDto>("session_messages", { platform, sessionId, before }),
  sessionContinueCheck: (platform: string, sessionId: string) =>
    invoke<ContinueCheckDto>("session_continue_check", { platform, sessionId }),
  sessionContinue: (platform: string, sessionId: string) => invoke<void>("session_continue", { platform, sessionId }),
}

export interface SessionFilterDto {
  platform: string | null
  /** 连接 id 或 "__unattributed__" */
  connection: string | null
  /** 项目目录原值；"" 表示未识别项目 */
  project: string | null
  since: number | null
  search: string | null
}

export type SessionStatus = "running" | "done" | "failed" | "recent" | "usage_only"

export interface SessionItemDto {
  platform: string
  session_id: string
  title: string | null
  cwd: string | null
  first_ts: number
  last_ts: number
  requests: number
  fresh_input: number | null
  cache_read: number | null
  cache_write: number | null
  output: number | null
  connection_id: string | null
  status: SessionStatus
  has_source: boolean
}

export interface SessionListDto {
  sessions: SessionItemDto[]
  files: number
}

export interface ToolCallDto {
  name: string
  summary: string
  output: string | null
}

export interface MessageDto {
  role: "user" | "assistant"
  ts: number | null
  text: string
  tools: ToolCallDto[]
}

export interface MessagePageDto {
  available: boolean
  total: number
  offset: number
  messages: MessageDto[]
  models: string[]
  cost: number
  priced_requests: number
  requests: number
}

export interface ContinueCheckDto {
  supported: boolean
  tool_found: boolean
  cwd: string | null
  cwd_accessible: boolean
}

/** 用量统计筛选。区间为 [start, end) 的 UTC 毫秒；connections 为空表示全部账号 */
export interface StatsFilterDto {
  start: number
  end: number
  platform: string | null
  connections: string[]
  model?: string | null
  status?: "success" | "error" | null
  search?: string | null
}

/** Token 分项；null 表示区间内有请求缺该字段，界面显示「—」 */
export interface TokenSplitDto {
  fresh_input: number | null
  output: number | null
  cache_read: number | null
  cache_write: number | null
  total: number | null
}

export interface CostSummaryDto {
  amount: number
  priced_requests: number
  requests: number
}

export interface StatsTrendBucketDto extends TokenSplitDto {
  start: number
  end: number
  label: string
  requests: number
  cost: number
}

export interface StatsModelRowDto {
  platform: string
  model: string | null
  requests: number
  tokens: number | null
  cache_hit_rate: number | null
  avg_duration_ms: number | null
  /** 组内全部请求可计价时才有值 */
  cost: number | null
}

export interface StatsProviderRowDto {
  platform: string
  requests: number
  tokens: number | null
  cost: CostSummaryDto
}

export interface StatsSessionRowDto {
  platform: string
  session_id: string
  title: string | null
  tokens: number
  /** 项目目录；来源文件缺失或没写 cwd 时为 null，界面只显示供应商 */
  cwd: string | null
}

export interface StatsOverviewDto {
  tokens: TokenSplitDto
  requests: number
  failed: number
  cache_hit_rate: number | null
  cost: CostSummaryDto
  previous_total: number | null
  trend: { bucket: "2h" | "day" | "week"; points: StatsTrendBucketDto[] }
  models: StatsModelRowDto[]
  providers: StatsProviderRowDto[]
  attributed: number
  sessions: StatsSessionRowDto[]
}

export interface StatsActivityDto {
  first_date: string
  last_date: string
  days: { date: string; tokens: number; requests: number }[]
  today_active_minutes: number
  week_active_days: number
  month_active_days: number
}

export interface StatsCallRowDto {
  id: number
  call_id: string
  ts: number
  platform: string
  model: string | null
  session_id: string | null
  fresh_input: number | null
  cache_read: number | null
  cache_write: number | null
  output: number | null
  total: number | null
  cost: number | null
  effort: string | null
  duration_ms: number | null
  first_token_ms: number | null
  status_code: number | null
  failed: boolean
}

export interface StatsCallPageDto {
  rows: StatsCallRowDto[]
  total_count: number
  unpriced_count: number
  page: number
  page_size: number
  page_count: number
  models: string[]
}

/** 紧凑格式：K / M / B，保留 Token 单位或所属统计标题（§3） */
export function compactTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return String(n)
}

