import { describe, expect, it } from "vitest";
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
