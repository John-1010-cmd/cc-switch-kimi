import { describe, expect, it } from "vitest";
import { deriveCacheWriteState } from "@/components/usage/UsageHero";
import {
  CACHE_INCLUSIVE_APP_TYPES,
  getFreshInputTokens,
  KNOWN_APP_TYPES,
} from "@/types/usage";

describe("kimi-code usage visibility", () => {
  it("includes kimi-code in KNOWN_APP_TYPES (dashboard filter button)", () => {
    expect(KNOWN_APP_TYPES).toContain("kimi-code");
  });

  it("includes kimi-code in CACHE_INCLUSIVE_APP_TYPES", () => {
    expect(CACHE_INCLUSIVE_APP_TYPES.has("kimi-code")).toBe(true);
  });

  it("subtracts cache reads from kimi-code input tokens", () => {
    // kimi-code 入库 input_tokens 为总输入口径，fresh input 需减去 cacheRead
    expect(
      getFreshInputTokens({
        appType: "kimi-code",
        inputTokens: 60000,
        cacheReadTokens: 48000,
      }),
    ).toBe(12000);
  });
});

describe("deriveCacheWriteState", () => {
  it("keeps static N/A for OpenAI-protocol proxy apps", () => {
    expect(
      deriveCacheWriteState([{ appType: "codex", cacheCreationTokens: 0 }]),
    ).toBe("na");
  });

  it("is data-driven for kimi-code: real cacheCreation data clears N/A", () => {
    expect(
      deriveCacheWriteState([
        { appType: "kimi-code", cacheCreationTokens: 6000 },
      ]),
    ).toBe("ok");
  });

  it("shows N/A for kimi-code while no cache-creation data exists", () => {
    expect(
      deriveCacheWriteState([{ appType: "kimi-code", cacheCreationTokens: 0 }]),
    ).toBe("na");
  });

  it("keeps Anthropic-style apps as ok", () => {
    expect(
      deriveCacheWriteState([{ appType: "claude", cacheCreationTokens: 0 }]),
    ).toBe("ok");
  });

  it("reports partial when mixing N/A and reporting apps", () => {
    expect(
      deriveCacheWriteState([
        { appType: "codex", cacheCreationTokens: 0 },
        { appType: "kimi-code", cacheCreationTokens: 100 },
      ]),
    ).toBe("partial");
  });
});
