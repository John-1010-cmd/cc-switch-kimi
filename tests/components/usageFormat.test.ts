import { describe, expect, it } from "vitest";
import {
  formatCacheHitRate5m,
  formatRealtimeTps,
  formatTokensShort,
  formatTokensAxis,
  formatTimingMs,
  getRequestTiming,
  getLocaleFromLanguage,
} from "@/components/usage/format";

describe("usage format helpers", () => {
  it("formats Traditional Chinese token units with Traditional characters", () => {
    expect(formatTokensShort(12_345, "zh-TW")).toBe("1.2 萬");
    expect(formatTokensShort(123_456_789, "zh-Hant", 2)).toBe("1.23 億");
  });

  it("resolves Traditional Chinese locale aliases", () => {
    expect(getLocaleFromLanguage("zh_TW")).toBe("zh-TW");
    expect(getLocaleFromLanguage("zh-HK")).toBe("zh-TW");
  });

  it("uses 万、亿、兆 and promotes rounded values at unit boundaries", () => {
    expect(formatTokensShort(9999, "zh")).toBe("9,999");
    expect(formatTokensShort(28_260_480, "zh", 2)).toBe("2826.05 万");
    expect(formatTokensShort(999_999_999_999, "zh", 2)).toBe("1.00 兆");
    expect(formatTokensShort(1_234_567_890_123, "zh", 2)).toBe("1.23 兆");
    expect(formatTokensShort(1_234_567_890_123, "zh-TW", 2)).toBe("1.23 兆");
    expect(formatTokensShort(1_234_567_890_123, "en", 2)).toBe("1.23T");
  });

  it("keeps chart axis labels compact and unbroken", () => {
    expect(formatTokensAxis(24_000_000, "zh")).toBe("2400万");
    expect(formatTokensAxis(100_000_000, "zh")).toBe("1亿");
    expect(formatTokensAxis(1_000_000_000_000, "zh")).toBe("1兆");
    expect(formatTokensAxis(1200, "en")).toBe("1.2K");
  });
});

describe("kimi realtime metrics formatting", () => {
  it("shows an em dash when there are no tps samples", () => {
    expect(formatRealtimeTps(null)).toBe("—");
  });

  it("formats tps with the t/s unit", () => {
    expect(formatRealtimeTps(42)).toBe("42 t/s");
    expect(formatRealtimeTps(0)).toBe("0 t/s");
  });

  it("shows an em dash instead of 0% when the 5m window has no input", () => {
    expect(formatCacheHitRate5m(null)).toBe("—");
  });

  it("formats the 5m cache hit rate with one decimal", () => {
    expect(formatCacheHitRate5m(87.25)).toBe("87.3%");
    expect(formatCacheHitRate5m(0)).toBe("0.0%");
    expect(formatCacheHitRate5m(100)).toBe("100.0%");
  });
});

describe("request timing formatting", () => {
  it("reconstructs total time from a stream duration and first-token wait", () => {
    expect(
      getRequestTiming({ latencyMs: 0, durationMs: 4000, firstTokenMs: 1000 })
        .elapsedMs,
    ).toBe(5000);
  });
  it("keeps missing total time separate from a recorded first-token time", () => {
    const timing = getRequestTiming({ latencyMs: 0, firstTokenMs: 700 });
    expect(formatTimingMs(timing.elapsedMs)).toBe("—");
    expect(formatTimingMs(timing.firstTokenMs)).toBe("0.7s");
  });

  it("formats session estimates and transport measurements without a prefix", () => {
    const estimated = getRequestTiming({
      latencyMs: 12400,
      firstTokenMs: 700,
      dataSource: "codex_session",
    });
    expect(formatTimingMs(estimated.elapsedMs)).toBe("12.4s");
    expect(formatTimingMs(estimated.firstTokenMs)).toBe("0.7s");
    const measured = getRequestTiming({
      latencyMs: 12400,
      firstTokenMs: 700,
      dataSource: "proxy",
    });
    expect(formatTimingMs(measured.elapsedMs)).toBe("12.4s");
  });
});
