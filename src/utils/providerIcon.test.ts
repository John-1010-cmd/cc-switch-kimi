import { describe, expect, it } from "vitest";
import { resolveProviderIcon } from "./providerIcon";

describe("resolveProviderIcon", () => {
  it("preserves a Grok icon explicitly selected by the user", () => {
    expect(resolveProviderIcon("claude", "grok", "currentColor")).toBe("grok");
  });

  it("does not reinterpret another app's provider icon", () => {
    expect(resolveProviderIcon("codex", "grok", "")).toBe("grok");
  });

  it("normalizes an empty icon to the initials fallback", () => {
    expect(resolveProviderIcon("claude", "  ", "")).toBeUndefined();
  });
});
