import type { RequestLog } from "@/types/usage";

export function parseFiniteNumber(value: unknown): number | null {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : null;
  }

  if (typeof value === "string") {
    const parsed = Number.parseFloat(value);
    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
}

export function fmtInt(
  value: unknown,
  locale?: string,
  fallback: string = "--",
): string {
  const num = parseFiniteNumber(value);
  if (num == null) return fallback;
  return new Intl.NumberFormat(locale).format(Math.trunc(num));
}

export function fmtUsd(
  value: unknown,
  digits: number,
  fallback: string = "--",
): string {
  const num = parseFiniteNumber(value);
  if (num == null) return fallback;
  return `$${num.toFixed(digits)}`;
}

function normalizeLanguageTag(language: string): string {
  return language.toLowerCase().replace(/_/g, "-");
}

function isTraditionalChineseLanguage(normalizedLanguage: string): boolean {
  return (
    normalizedLanguage === "zh-tw" ||
    normalizedLanguage.startsWith("zh-hant") ||
    normalizedLanguage.startsWith("zh-hk") ||
    normalizedLanguage.startsWith("zh-mo")
  );
}

export function getLocaleFromLanguage(language: string): string {
  if (!language) return "en-US";
  const normalized = normalizeLanguageTag(language);
  if (normalized === "zh") return "zh-CN";
  if (isTraditionalChineseLanguage(normalized)) {
    return "zh-TW";
  }
  if (normalized.startsWith("zh")) return "zh-CN";
  return "en-US";
}

interface I18nLike {
  resolvedLanguage?: string;
  language?: string;
}

/**
 * 实时生成速度显示：null（无样本）→ "—"，否则 "<n> t/s"。
 * 后端已取整，这里只做空值兜底。
 */
export function formatRealtimeTps(tps: number | null): string {
  if (tps == null || !Number.isFinite(tps)) return "—";
  return `${Math.round(tps)} t/s`;
}

/**
 * 5 分钟缓存命中率显示：null（窗口总输入为 0）→ "—"，否则保留 1 位小数的
 * 百分比。绝不显示 "0.0%" —— null 与真实 0% 必须可区分。
 */
export function formatCacheHitRate5m(rate: number | null): string {
  if (rate == null || !Number.isFinite(rate)) return "—";
  return `${rate.toFixed(1)}%`;
}

export function getResolvedLang(i18n: I18nLike): string {
  return i18n.resolvedLanguage || i18n.language || "en";
}

/**
 * Token 数量的紧凑显示。
 *
 * Why: 中文用户期待 "亿/万" 量纲；英文用户期待 K/M/B。共用同一份格式化
 * 逻辑避免 Hero 卡和分应用卡显示不一致。`compactDecimals=2` 用于 Hero
 * 大数副标（更精确），默认 1 位用于卡片副字段。
 */
export function formatTokensShort(
  value: number,
  lang: string,
  compactDecimals: 1 | 2 = 1,
): string {
  if (!Number.isFinite(value) || value <= 0) return "0";
  const normalizedLang = normalizeLanguageTag(lang);
  const traditional = isTraditionalChineseLanguage(normalizedLang);
  const units = normalizedLang.startsWith("zh")
    ? [
        { scale: 1e12, suffix: " 兆", digits: 2 },
        { scale: 1e8, suffix: traditional ? " 億" : " 亿", digits: 2 },
        {
          scale: 1e4,
          suffix: traditional ? " 萬" : " 万",
          digits: compactDecimals,
        },
      ]
    : [
        { scale: 1e12, suffix: "T", digits: 2 },
        { scale: 1e9, suffix: "B", digits: 2 },
        { scale: 1e6, suffix: "M", digits: 2 },
        { scale: 1e3, suffix: "K", digits: compactDecimals },
      ];
  let index = units.findIndex((unit) => value >= unit.scale);
  if (index < 0) {
    return value.toLocaleString(getLocaleFromLanguage(lang));
  }
  const unit = units[index];
  if (
    index > 0 &&
    Number((value / unit.scale).toFixed(unit.digits)) >=
      units[index - 1].scale / unit.scale
  ) {
    index -= 1;
  }
  const selected = units[index];
  return `${(value / selected.scale).toFixed(selected.digits)}${selected.suffix}`;
}

export function fmtUsdCompact(
  value: unknown,
  lang: string,
  digits: number,
): string {
  const number = parseFiniteNumber(value);
  if (number == null) return "--";
  const threshold = normalizeLanguageTag(lang).startsWith("zh") ? 1e4 : 1e3;
  return Math.abs(number) >= threshold
    ? `${number < 0 ? "-" : ""}$${formatTokensShort(Math.abs(number), lang, 2)}`
    : fmtUsd(number, digits);
}

/** Keep chart ticks on one line while the tooltip retains more precision. */
export function formatTokensAxis(value: number, lang: string): string {
  return formatTokensShort(value, lang, 1)
    .replace(/\.0+(?=\s|[KMBT]|$)/, "")
    .replace(" ", "");
}

export function getRequestTiming(
  log: Pick<
    RequestLog,
    "latencyMs" | "durationMs" | "firstTokenMs" | "dataSource"
  >,
) {
  const firstTokenMs =
    log.firstTokenMs != null &&
    Number.isFinite(log.firstTokenMs) &&
    log.firstTokenMs >= 0
      ? log.firstTokenMs
      : null;
  const elapsedMs =
    Number.isFinite(log.latencyMs) && log.latencyMs > 0
      ? log.latencyMs
      : log.durationMs != null &&
          Number.isFinite(log.durationMs) &&
          log.durationMs > 0
        ? log.durationMs + (firstTokenMs ?? 0)
        : null;
  return {
    elapsedMs,
    firstTokenMs,
    estimated: log.dataSource === "codex_session",
  };
}

export function formatTimingMs(value: number | null): string {
  if (value == null || !Number.isFinite(value) || value < 0) return "—";
  return `${(value / 1000).toFixed(1)}s`;
}
