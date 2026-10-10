import { cloneElement, isValidElement } from "react";
import { useTranslation } from "react-i18next";
import { motion } from "framer-motion";
import { Card, CardContent } from "@/components/ui/card";
import {
  supportsKimiRealtimeMetrics,
  useKimiRealtimeMetrics,
  useUsageSummaryByApp,
} from "@/lib/query/usage";
import { cn } from "@/lib/utils";
import { APP_ICON_MAP } from "@/config/appConfig";
import type { AppId } from "@/lib/api/types";
import {
  Activity,
  ArrowDownToLine,
  ArrowUpFromLine,
  Gauge,
  Info,
  Loader2,
  Percent,
  Sparkles,
  Zap,
} from "lucide-react";
import {
  fmtUsd,
  formatCacheHitRate5m,
  formatRealtimeTps,
  formatTokensShort,
  getResolvedLang,
  parseFiniteNumber,
} from "./format";
import {
  type AppType,
  type UsageRangeSelection,
  type UsageSummary,
  type UsageSummaryByApp,
} from "@/types/usage";

interface UsageHeroProps {
  range: UsageRangeSelection;
  appType?: string;
  providerName?: string;
  model?: string;
  refreshIntervalMs: number;
}

interface TitleTheme {
  /** Foreground color for the icon glyph (text-* class). */
  accent: string;
  /** Background tint for the icon square (bg-* class). */
  iconBg: string;
}

const TITLE_THEMES: Record<AppType | "all", TitleTheme> = {
  all: { accent: "text-primary", iconBg: "bg-primary/10" },
  claude: {
    accent: "text-amber-600 dark:text-amber-400",
    iconBg: "bg-amber-500/10",
  },
  codex: {
    // OpenAI/Codex 走黑白单色调；中性灰在深浅模式都能透出方块底色，
    // 不像纯黑 bg-black/10 在深色背景下会糊掉。
    accent: "text-neutral-700 dark:text-neutral-300",
    iconBg: "bg-neutral-500/10",
  },
  "kimi-code": {
    accent: "text-teal-600 dark:text-teal-400",
    iconBg: "bg-teal-500/10",
  },
};

/**
 * Combine per-app summaries into a single rolled-up summary.
 *
 * The backend's per-app rows already use fresh-input semantics (cache-inclusive
 * providers have been normalized in SQL), so plain addition is correct here.
 * `cacheHitRate` and `successRate` must be re-derived from the summed counts
 * rather than averaged across rows.
 */
function aggregateSummaries(items: UsageSummary[]): UsageSummary {
  let totalRequests = 0;
  let successCount = 0;
  let totalCostNum = 0;
  let input = 0;
  let output = 0;
  let cacheCreation = 0;
  let cacheRead = 0;
  let speedOutput = 0;
  let speedDurationMs = 0;

  for (const s of items) {
    totalRequests += s.totalRequests;
    successCount += Math.round((s.totalRequests * s.successRate) / 100);
    totalCostNum += parseFiniteNumber(s.totalCost) ?? 0;
    input += s.totalInputTokens;
    output += s.totalOutputTokens;
    cacheCreation += s.totalCacheCreationTokens;
    cacheRead += s.totalCacheReadTokens;
    speedOutput += s.totalSpeedOutputTokens ?? 0;
    speedDurationMs += s.totalSpeedDurationMs ?? 0;
  }

  const cacheableInput = input + cacheCreation + cacheRead;
  return {
    totalRequests,
    totalCost: totalCostNum.toFixed(6),
    totalInputTokens: input,
    totalOutputTokens: output,
    totalCacheCreationTokens: cacheCreation,
    totalCacheReadTokens: cacheRead,
    successRate: totalRequests > 0 ? (successCount / totalRequests) * 100 : 0,
    realTotalTokens: input + output + cacheCreation + cacheRead,
    cacheHitRate: cacheableInput > 0 ? cacheRead / cacheableInput : 0,
    totalSpeedOutputTokens: speedOutput,
    totalSpeedDurationMs: speedDurationMs,
  };
}

function pickSummary(
  apps: UsageSummaryByApp[],
  appType: string | undefined,
): UsageSummary | undefined {
  if (apps.length === 0) return undefined;
  if (appType) {
    return apps.find((a) => a.appType === appType)?.summary;
  }
  return aggregateSummaries(apps.map((a) => a.summary));
}

/**
 * Hero 标题图标：选中具体应用时显示该应用的品牌图标，"全部"时回退到通用闪电。
 * 复用 APP_ICON_MAP（与侧边栏 / 应用切换器同一套图标），用 cloneElement 放大到
 * 与原闪电一致的 20px；品牌图标自带配色，外层方块仍按 titleTheme 主题色着色。
 */
function AppGlyph({
  appType,
  accentClass,
}: {
  appType?: string;
  accentClass: string;
}) {
  if (appType && appType in APP_ICON_MAP) {
    const base = APP_ICON_MAP[appType as AppId].icon;
    if (isValidElement<{ size?: number }>(base)) {
      return cloneElement(base, { size: 20 });
    }
  }
  return <Zap className={cn("h-5 w-5", accentClass)} />;
}

export function UsageHero({
  range,
  appType,
  providerName,
  model,
  refreshIntervalMs,
}: UsageHeroProps) {
  const { t, i18n } = useTranslation();
  const lang = getResolvedLang(i18n);

  const { data, isLoading } = useUsageSummaryByApp(
    range,
    { providerName, model },
    {
      refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
    },
  );

  // kimi-code 实时指标（tps / 5m 缓存命中率）：仅「全部 / kimi-code」视角显示
  const showRealtime = supportsKimiRealtimeMetrics(appType);
  const { data: realtime } = useKimiRealtimeMetrics(appType);

  // No client-side filtering: Hero's totals must match the Trend/Logs/Stats
  // below, which all go through the backend's full set of app_types. The
  // KNOWN_APP_TYPES list only governs which filter buttons appear, not which
  // rows participate in the "all" aggregate.
  const allApps = data ?? [];
  const summary = pickSummary(allApps, appType);

  const titleTheme =
    TITLE_THEMES[(appType ?? "all") as keyof typeof TITLE_THEMES] ??
    TITLE_THEMES.all;
  const appLabel =
    appType && appType in TITLE_THEMES ? t(`usage.appFilter.${appType}`) : null;

  const input = summary?.totalInputTokens ?? 0;
  const output = summary?.totalOutputTokens ?? 0;
  const cacheRead = summary?.totalCacheReadTokens ?? 0;
  const realTotal = summary?.realTotalTokens ?? 0;
  const hitRate = summary?.cacheHitRate ?? 0;
  const totalCost = parseFiniteNumber(summary?.totalCost);
  const requests = summary?.totalRequests ?? 0;
  const speedDurationMs = summary?.totalSpeedDurationMs ?? 0;
  const averageTps =
    speedDurationMs > 0
      ? ((summary?.totalSpeedOutputTokens ?? 0) * 1000) / speedDurationMs
      : null;

  if (isLoading) {
    return (
      <Card className="border border-border/50 bg-card/40 backdrop-blur-sm">
        <CardContent className="flex items-center justify-center min-h-[200px]">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground/50" />
        </CardContent>
      </Card>
    );
  }

  const hitPercent = Math.max(0, Math.min(100, hitRate * 100));
  const hitPercentLabel = hitPercent.toFixed(hitPercent >= 99.95 ? 0 : 1);

  return (
    <motion.div
      initial={{ opacity: 0, y: 5 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4 }}
    >
      <Card className="relative overflow-hidden border border-border/50 bg-card/60 backdrop-blur-xl shadow-sm">
        <CardContent className="p-4 md:p-5">
          <div className="flex flex-col gap-4">
            {/* Top row: Main Token Count, Requests, Cost */}
            <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
              <div className="flex items-center gap-3">
                <div
                  className={cn(
                    "p-2.5 rounded-xl bg-gradient-to-br shadow-sm",
                    titleTheme.iconBg,
                  )}
                >
                  <AppGlyph appType={appType} accentClass={titleTheme.accent} />
                </div>
                <div>
                  <div className="text-xs font-medium text-muted-foreground flex items-center gap-1.5 mb-0.5">
                    {appLabel && (
                      <>
                        <span
                          className={cn("font-semibold", titleTheme.accent)}
                        >
                          {appLabel}
                        </span>
                        <span className="text-muted-foreground/30">•</span>
                      </>
                    )}
                    {t("usage.realTotal", "真实消耗 Tokens")}
                  </div>
                  <div className="flex items-baseline gap-2">
                    <span
                      className="text-2xl md:text-3xl font-bold tabular-nums tracking-tight leading-none"
                      title={realTotal.toLocaleString()}
                    >
                      {realTotal.toLocaleString()}
                    </span>
                    <span className="text-xs text-muted-foreground font-medium bg-muted/40 px-1.5 py-0.5 rounded-md">
                      ≈ {formatTokensShort(realTotal, lang, 2)}
                    </span>
                  </div>
                </div>
              </div>

              <div className="flex items-center gap-5 bg-background/50 px-4 py-2.5 rounded-xl border border-border/40 shadow-sm">
                <div className="flex flex-col">
                  <span className="text-[10px] text-muted-foreground uppercase tracking-wider font-medium">
                    {t("usage.totalRequests")}
                  </span>
                  <span className="font-semibold flex items-center gap-1.5 text-sm tabular-nums">
                    <Activity className="h-3.5 w-3.5 text-blue-500" />
                    {requests.toLocaleString()}
                  </span>
                </div>
                <div className="w-px h-8 bg-border/60" />
                <div className="flex flex-col">
                  <span className="text-[10px] text-muted-foreground uppercase tracking-wider font-medium">
                    {t("usage.totalCost")}
                  </span>
                  <span className="font-semibold text-green-500 text-sm tabular-nums">
                    {totalCost == null ? "--" : fmtUsd(totalCost, 4)}
                  </span>
                </div>
              </div>
            </div>

            {/* Bottom row: Breakdown and Hit Rate */}
            <div
              className={cn(
                "grid grid-cols-2 gap-3",
                showRealtime
                  ? "md:grid-cols-4 xl:grid-cols-7"
                  : "md:grid-cols-3 xl:grid-cols-5",
              )}
            >
              <MiniStat
                icon={<ArrowDownToLine className="h-3.5 w-3.5" />}
                label={t("usage.freshInput", "新增输入")}
                value={formatTokensShort(input, lang)}
                accent="text-blue-500"
              />
              <MiniStat
                icon={<ArrowUpFromLine className="h-3.5 w-3.5" />}
                label={t("usage.output")}
                value={formatTokensShort(output, lang)}
                accent="text-purple-500"
              />
              <MiniStat
                icon={<Sparkles className="h-3.5 w-3.5" />}
                label={t("usage.cacheRead", "缓存命中")}
                value={formatTokensShort(cacheRead, lang)}
                accent="text-emerald-500"
              />
              <MiniStat
                icon={<Gauge className="h-3.5 w-3.5" />}
                label={t("usage.averageSpeed", "平均速度")}
                value={formatRealtimeTps(averageTps)}
                accent="text-teal-500"
                muted={averageTps == null}
                tooltip={t(
                  "usage.averageSpeedHint",
                  "所选范围内，有效请求的输出 Tokens ÷ 生成耗时；优先扣除首字等待，无计时的请求不参与，Codex 计时按会话事件估算",
                )}
              />
              <div className="flex flex-col justify-center rounded-xl border border-border/40 bg-background/40 p-3 shadow-sm">
                <div className="flex items-center justify-between text-[11px] mb-2 gap-2">
                  <span className="text-muted-foreground font-medium">
                    {t("usage.cacheHitRate", "缓存命中率")}
                  </span>
                  <span className="font-bold text-emerald-500 tabular-nums">
                    {hitPercentLabel}%
                  </span>
                </div>
                <div className="relative h-1.5 rounded-full bg-muted/60 overflow-hidden">
                  <motion.div
                    className="absolute inset-y-0 left-0 bg-emerald-500 rounded-full"
                    initial={{ width: 0 }}
                    animate={{ width: `${hitPercent}%` }}
                    transition={{ duration: 0.8, ease: "easeOut" }}
                  />
                </div>
              </div>
              {showRealtime && (
                <>
                  <MiniStat
                    icon={<Gauge className="h-3.5 w-3.5" />}
                    label={t("usage.realtime.tps", "实时速度")}
                    value={formatRealtimeTps(realtime?.tps ?? null)}
                    accent="text-teal-500"
                    muted={realtime?.tps == null}
                  />
                  <MiniStat
                    icon={<Percent className="h-3.5 w-3.5" />}
                    label={t("usage.realtime.cacheHit", "缓存命中（5分钟）")}
                    value={formatCacheHitRate5m(
                      realtime?.cacheHitRate5m ?? null,
                    )}
                    accent="text-emerald-500"
                    muted={realtime?.cacheHitRate5m == null}
                    tooltip={t(
                      "usage.realtime.cacheHitHint",
                      "按供应商上报口径统计，OpenAI 兼容端点可能偏低",
                    )}
                  />
                </>
              )}
            </div>
          </div>
        </CardContent>
      </Card>
    </motion.div>
  );
}

interface MiniStatProps {
  icon: React.ReactNode;
  label: string;
  value: string;
  accent: string;
  /** Optional hover tooltip — used to flag protocol-level caveats. */
  tooltip?: string;
  /** Visually de-emphasize the value (e.g. for "N/A" cases). */
  muted?: boolean;
}

function MiniStat({
  icon,
  label,
  value,
  accent,
  tooltip,
  muted,
}: MiniStatProps) {
  return (
    <div
      className="flex flex-col gap-1 rounded-xl border border-border/40 bg-background/40 p-3 shadow-sm"
      title={tooltip}
    >
      <div
        className={`flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground ${accent}`}
      >
        {icon}
        <span className="text-foreground/70 tracking-wide">{label}</span>
        {tooltip && (
          <Info className="h-3 w-3 text-muted-foreground/60 shrink-0 ml-auto" />
        )}
      </div>
      <div
        className={cn(
          "text-sm font-semibold tabular-nums",
          muted && "text-muted-foreground/70",
        )}
      >
        {value}
      </div>
    </div>
  );
}
