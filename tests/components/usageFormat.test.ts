import { describe, expect, it } from "vitest";
import {
  formatCacheHitRate5m,
  formatRealtimeTps,
  formatTokensShort,
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
