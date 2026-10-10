import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageHero } from "@/components/usage/UsageHero";
import { ProviderStatsTable } from "@/components/usage/ProviderStatsTable";
import { ModelStatsTable } from "@/components/usage/ModelStatsTable";
import type {
  ModelStats,
  ProviderStats,
  UsageSummaryByApp,
} from "@/types/usage";

const mocks = vi.hoisted(() => ({
  summaries: vi.fn(),
  providers: vi.fn(),
  models: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
    i18n: { resolvedLanguage: "zh", language: "zh" },
  }),
}));

vi.mock("framer-motion", () => ({
  motion: {
    div: ({
      children,
      initial: _initial,
      animate: _animate,
      transition: _transition,
      ...props
    }: any) => <div {...props}>{children}</div>,
  },
}));

vi.mock("@/lib/query/usage", () => ({
  useUsageSummaryByApp: () => mocks.summaries(),
  supportsKimiRealtimeMetrics: (app?: string) => !app || app === "kimi-code",
  useKimiRealtimeMetrics: () => ({ data: { tps: 174, cacheHitRate5m: 95.2 } }),
  useProviderStats: () => mocks.providers(),
  useModelStats: () => mocks.models(),
}));

const props = { range: { preset: "today" as const }, refreshIntervalMs: 0 };
const summary = (
  appType: string,
  output: number,
  duration: number,
): UsageSummaryByApp => ({
  appType,
  summary: {
    totalRequests: 1,
    totalCost: "1",
    totalInputTokens: 1000,
    totalOutputTokens: output,
    totalCacheCreationTokens: 0,
    totalCacheReadTokens: 9000,
    successRate: 100,
    realTotalTokens: 10000 + output,
    cacheHitRate: 0.9,
    totalSpeedOutputTokens: output,
    totalSpeedDurationMs: duration,
  },
});

describe("usage metrics presentation", () => {
  beforeEach(() => {
    mocks.summaries.mockReturnValue({
      data: [summary("codex", 100, 4000), summary("kimi-code", 200, 2000)],
      isLoading: false,
    });
  });

  it("shows the weighted generation speed before cache hit rate and realtime metrics", () => {
    const { container } = render(<UsageHero {...props} />);
    const average = screen.getByText("平均速度").parentElement!.parentElement!;
    expect(within(average).getByText("50 t/s")).toBeInTheDocument();
    const content = container.textContent!;
    expect(content.indexOf("平均速度")).toBeLessThan(
      content.indexOf("缓存命中率"),
    );
    expect(content.indexOf("缓存命中率")).toBeLessThan(
      content.indexOf("实时速度"),
    );
    expect(content.indexOf("实时速度")).toBeLessThan(
      content.indexOf("缓存命中（5分钟）"),
    );
  });

  it("uses only the selected app's speed samples", () => {
    render(<UsageHero {...props} appType="codex" />);
    expect(screen.getByText("25 t/s")).toBeInTheDocument();
    expect(screen.queryByText("实时速度")).not.toBeInTheDocument();
  });

  it("shows no speed when the selected range has no valid timing samples", () => {
    mocks.summaries.mockReturnValue({
      data: [summary("codex", 100, 0)],
      isLoading: false,
    });
    render(<UsageHero {...props} appType="codex" />);
    const average = screen.getByText("平均速度").parentElement!.parentElement!;
    expect(within(average).getByText("—")).toBeInTheDocument();
  });

  it("compacts provider counts, tokens and costs while retaining exact hover values", () => {
    const provider: ProviderStats = {
      providerId: "p1",
      providerName: "Provider",
      appType: "codex",
      requestCount: 12345,
      totalTokens: 0,
      totalCost: "12345",
      successRate: 100,
      avgLatencyMs: 500,
      totalInputTokens: 123456789,
      totalOutputTokens: 12345,
      totalCacheCreationTokens: 0,
      totalCacheReadTokens: 1234567890123,
      cacheHitRate: 0.9,
    };
    mocks.providers.mockReturnValue({ data: [provider], isLoading: false });
    render(<ProviderStatsTable {...props} />);
    expect(screen.getByText("1.23 亿")).toHaveAttribute("title", "123,456,789");
    expect(screen.getByText("1.23 兆")).toHaveAttribute(
      "title",
      "1,234,567,890,123",
    );
    expect(screen.getByText("$1.23 万")).toHaveAttribute(
      "title",
      "$12345.000000",
    );
  });

  it("compacts model metrics without reducing small average-cost precision", () => {
    const model: ModelStats = {
      model: "model",
      appTypes: "codex",
      requestCount: 12345,
      totalTokens: 0,
      totalCost: "7.6838",
      avgCostPerRequest: "0.031491",
      totalInputTokens: 1431168,
      totalOutputTokens: 199537,
      totalCacheCreationTokens: 0,
      totalCacheReadTokens: 28260480,
      cacheHitRate: 0.952,
    };
    mocks.models.mockReturnValue({ data: [model], isLoading: false });
    render(<ModelStatsTable {...props} />);
    expect(screen.getByText("143.12 万")).toHaveAttribute("title", "1,431,168");
    expect(screen.getByText("2826.05 万")).toHaveAttribute(
      "title",
      "28,260,480",
    );
    expect(screen.getByText("$0.031491")).toBeInTheDocument();
  });
});
