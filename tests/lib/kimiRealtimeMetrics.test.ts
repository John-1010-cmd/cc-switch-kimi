import { describe, expect, it } from "vitest";
import {
  supportsKimiRealtimeMetrics,
  usageKeys,
} from "@/lib/query/usage";

// kimi-code 实时指标只在「全部 / kimi-code」视角下有语义：其他 app 过滤视角下
// 数据源（kimi-code wire.jsonl）恒为空样本，轮询没有意义。
describe("supportsKimiRealtimeMetrics", () => {
  it("无过滤（undefined）与全部（all）视角 → enabled", () => {
    expect(supportsKimiRealtimeMetrics(undefined)).toBe(true);
    expect(supportsKimiRealtimeMetrics("all")).toBe(true);
  });

  it("kimi-code 视角 → enabled", () => {
    expect(supportsKimiRealtimeMetrics("kimi-code")).toBe(true);
  });

  it("其他 app 过滤视角 → disabled", () => {
    expect(supportsKimiRealtimeMetrics("claude")).toBe(false);
    expect(supportsKimiRealtimeMetrics("codex")).toBe(false);
  });
});

describe("usageKeys.kimiRealtimeMetrics", () => {
  it("挂在 usageKeys.all 命名空间下（usage-log-recorded 的 invalidate 可覆盖）", () => {
    expect(usageKeys.kimiRealtimeMetrics()).toEqual([
      "usage",
      "kimi-realtime-metrics",
    ]);
    expect(usageKeys.kimiRealtimeMetrics()[0]).toBe(usageKeys.all[0]);
  });
});
