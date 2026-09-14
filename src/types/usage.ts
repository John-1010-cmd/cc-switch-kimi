// 使用统计相关类型定义

export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
}

export interface RequestLog {
  requestId: string;
  providerId: string;
  providerName?: string;
  appType: string;
  model: string;
  requestModel?: string;
  /** 写入时实际用于计价的模型名；路由接管 + request 计价模式下可能与 model 不同 */
  pricingModel?: string;
  costMultiplier: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  /**
   * input_tokens 的存储语义：0=legacy（缓存包含型 app 只含 cache_read）、
   * 1=TOTAL（含 cache_read + cache_creation）、2=FRESH（已扣减缓存）。
   * 与 Rust `INPUT_TOKEN_SEMANTICS_*` 对应，见 [getFreshInputTokens]。
   */
  inputTokenSemantics?: number;
  inputCostUsd: string;
  outputCostUsd: string;
  cacheReadCostUsd: string;
  cacheCreationCostUsd: string;
  totalCostUsd: string;
  isStreaming: boolean;
  latencyMs: number;
  firstTokenMs?: number;
  durationMs?: number;
  statusCode: number;
  errorMessage?: string;
  createdAt: number;
  dataSource?: string;
}

export interface SessionSyncResult {
  imported: number;
  skipped: number;
  filesScanned: number;
  suspectedDuplicates: number;
  deferredFiles: number;
  errors: string[];
}

export interface DataSourceSummary {
  dataSource: string;
  requestCount: number;
  totalCostUsd: string;
}

export interface PaginatedLogs {
  data: RequestLog[];
  total: number;
  page: number;
  pageSize: number;
}

export interface ModelPricing {
  modelId: string;
  displayName: string;
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheCreationCostPerMillion: string;
}

export interface ModelsDevSyncConfig {
  autoSyncEnabled: boolean;
  includeCommonModels: boolean;
  selectedModelKeys: string[];
  excludedCommonModelKeys: string[];
  lastSyncAt: number | null;
  lastSyncError: string | null;
}

export interface ModelsDevSyncState {
  config: ModelsDevSyncConfig;
  configPath: string;
}

export interface UsageSummary {
  totalRequests: number;
  totalCost: string;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
  successRate: number;
  /** input + output + cache_creation + cache_read, all cache-normalized */
  realTotalTokens: number;
  /** cache_read / (input + cache_creation + cache_read), range 0–1 */
  cacheHitRate: number;
}

export interface UsageSummaryByApp {
  appType: string;
  summary: UsageSummary;
}

export interface DailyStats {
  date: string;
  requestCount: number;
  totalCost: string;
  totalTokens: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
}

export interface ProviderStats {
  providerId: string;
  providerName: string;
  /** 折叠后的应用类型（claude-desktop → claude） */
  appType: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  successRate: number;
  avgLatencyMs: number;
  /** fresh-input 口径（缓存包含型 app 已扣减缓存部分） */
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
  /** cache_read / (input + cache_creation + cache_read)，0–1 分数 */
  cacheHitRate: number;
}

export interface ModelStats {
  model: string;
  /** 该模型行贡献者的去重 folded app_type 列表（逗号分隔） */
  appTypes: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  avgCostPerRequest: string;
  /** fresh-input 口径（缓存包含型 app 已扣减缓存部分） */
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
  /** cache_read / (input + cache_creation + cache_read)，0–1 分数 */
  cacheHitRate: number;
}

export interface LogFilters {
  appType?: string;
  providerName?: string;
  model?: string;
  statusCode?: number;
  startDate?: number;
  endDate?: number;
}

/**
 * Dashboard 顶栏的全局筛选维度，作用于 Hero / 趋势图 / 三个统计 Tab。
 *
 * - `providerName` 按展示名精确匹配（与 Provider 统计列表同口径，含
 *   "Claude (Session)" 等会话占位名）；
 * - `model` 按「有效计价模型」匹配（pricing_model 优先、回落 model，
 *   与模型统计的分组口径一致）。
 */
export interface UsageScopeFilters {
  appType?: string;
  providerName?: string;
  model?: string;
}

export interface ProviderLimitStatus {
  providerId: string;
  dailyUsage: string;
  dailyLimit?: string;
  dailyExceeded: boolean;
  monthlyUsage: string;
  monthlyLimit?: string;
  monthlyExceeded: boolean;
}

/**
 * kimi-code 实时指标（get_kimi_realtime_metrics，纯内存读取，可秒级轮询）。
 *
 * `tps` 为最近 5 个有效样本的中位生成速度（tokens/s，已取整），无样本为 null
 * → UI 显示 "—"；`cacheHitRate5m` 为最近 5 分钟缓存命中率（0–100 百分比），
 * 窗口总输入为 0 时 null → UI 显示 "—"（绝不显示 0%）。
 */
export interface KimiRealtimeMetrics {
  tps: number | null;
  sampleCount: number;
  activeSessions: number;
  cacheHitRate5m: number | null;
  lastEventAt: number | null;
}

export type UsageRangePreset = "today" | "1d" | "7d" | "14d" | "30d" | "custom";

export interface UsageRangeSelection {
  preset: UsageRangePreset;
  customStartDate?: number;
  customEndDate?: number;
  /** When true (custom mode only), endDate resolves to "now" instead of the
   *  fixed customEndDate snapshot, and the end-time field becomes read-only. */
  liveEndTime?: boolean;
}

/**
 * App types surfaced as dashboard filter buttons.
 *
 * `claude-desktop` is intentionally NOT listed: the Desktop gateway's proxy
 * traffic is still recorded under its own `app_type` (preserving route-takeover
 * billing audit — the request detail panel shows the real value), but the
 * dashboard folds it into `claude` for display. It is the embedded Claude Code
 * runtime running inside the Desktop shell, and Desktop *chat* usage never
 * passes through this app at all, so a separate "Claude Desktop" bucket would
 * only ever show a partial number and mislead users into reading it as the
 * Desktop's full usage. The backend collapses `claude-desktop → claude` in
 * every dashboard query (see `folded_app_type_sql`).
 */
export type AppType = "claude" | "codex" | "kimi-code";

export type AppTypeFilter = "all" | AppType;

export const KNOWN_APP_TYPES: ReadonlyArray<AppType> = [
  "kimi-code",
  "codex",
  "claude",
];

/**
 * App types whose `inputTokens` already includes the cached portion (must
 * subtract `cacheReadTokens` to get fresh-input semantics — see
 * [getFreshInputTokens]).
 *
 * For the OpenAI-style proxy apps (codex) there is a second
 * consequence: the protocol does not report cache _creation_ separately, only
 * cache _reads_, so `cacheCreationTokens` is always 0 for them; `kimi-code`
 * speaks the Anthropic protocol and reports real cache-creation tokens.
 *
 * Mirror of the Rust `CACHE_INCLUSIVE_APP_TYPES` whitelist.
 */
export const CACHE_INCLUSIVE_APP_TYPES: ReadonlySet<string> = new Set([
  "codex",
  "kimi-code",
]);

/** Subset of request-log fields needed to derive cache-normalized input. */
export interface CacheNormalizableLog {
  appType: string;
  inputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens?: number;
  inputTokenSemantics?: number;
}

/** 与 Rust `INPUT_TOKEN_SEMANTICS_*`（sql_helpers.rs）对应，改动须同步。 */
export const INPUT_TOKEN_SEMANTICS = {
  LEGACY: 0,
  TOTAL: 1,
  FRESH: 2,
} as const;

/**
 * For a single request log, return the input token count with the cached
 * portion removed. Mirrors the Rust `fresh_input_sql` semantics:
 * FRESH rows are already normalized; TOTAL rows subtract cache reads *and*
 * cache writes; legacy rows of cache-inclusive apps subtract cache reads
 * only. Anthropic-style providers pass through unchanged.
 */
export function getFreshInputTokens(log: CacheNormalizableLog): number {
  if (log.inputTokenSemantics === INPUT_TOKEN_SEMANTICS.FRESH) {
    return log.inputTokens;
  }
  if (!CACHE_INCLUSIVE_APP_TYPES.has(log.appType)) {
    return log.inputTokens;
  }
  if (log.inputTokenSemantics === INPUT_TOKEN_SEMANTICS.TOTAL) {
    const cached = log.cacheReadTokens + (log.cacheCreationTokens ?? 0);
    return log.inputTokens >= cached
      ? log.inputTokens - cached
      : log.inputTokens;
  }
  if (log.inputTokens >= log.cacheReadTokens) {
    return log.inputTokens - log.cacheReadTokens;
  }
  return log.inputTokens;
}

/**
 * 单条请求日志的缓存命中率：cache_read / (fresh_input + cache_creation +
 * cache_read)，与后端聚合口径一致。可缓存输入为 0 时返回 null（UI 显示
 * "—"，不显示误导性的 0%）。
 */
export function getLogCacheHitRate(log: CacheNormalizableLog): number | null {
  const cacheRead = log.cacheReadTokens;
  const cacheCreation = log.cacheCreationTokens ?? 0;
  const cacheable = getFreshInputTokens(log) + cacheCreation + cacheRead;
  if (cacheable <= 0) return null;
  return cacheRead / cacheable;
}

export const NON_NEGATIVE_DECIMAL_REGEX = /^\d+(?:\.\d+)?$/;

export function isNonNegativeDecimalString(value: string): boolean {
  const trimmed = value.trim();
  if (!NON_NEGATIVE_DECIMAL_REGEX.test(trimmed)) return false;
  return Number.isFinite(Number(trimmed));
}

type UsageCostLog = Pick<
  RequestLog,
  | "inputTokens"
  | "outputTokens"
  | "cacheReadTokens"
  | "cacheCreationTokens"
  | "totalCostUsd"
  | "statusCode"
> &
  Partial<Pick<RequestLog, "costMultiplier">>;

export function hasUsageTokens(log: UsageCostLog): boolean {
  return (
    log.inputTokens > 0 ||
    log.outputTokens > 0 ||
    log.cacheReadTokens > 0 ||
    log.cacheCreationTokens > 0
  );
}

export function isUnpricedUsage(log: UsageCostLog): boolean {
  const totalCost = Number.parseFloat(log.totalCostUsd);
  const multiplier =
    log.costMultiplier == null
      ? undefined
      : Number.parseFloat(log.costMultiplier);
  return (
    log.statusCode >= 200 &&
    log.statusCode < 300 &&
    hasUsageTokens(log) &&
    Number.isFinite(totalCost) &&
    (!Number.isFinite(multiplier) || multiplier !== 0) &&
    totalCost === 0
  );
}

export interface StatsFilters {
  timeRange: UsageRangePreset;
  providerId?: string;
  appType?: string;
}
