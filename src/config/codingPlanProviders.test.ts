import { describe, expect, it } from "vitest";
import {
  detectCodingPlanProvider,
  injectCodingPlanUsageScript,
} from "./codingPlanProviders";
import { TEMPLATE_TYPES } from "./constants";

describe("detectCodingPlanProvider", () => {
  describe("M4 预设 base_url 命中矩阵", () => {
    const hits: Array<[string, string]> = [
      ["https://api.kimi.com/coding/v1", "kimi"],
      ["https://open.bigmodel.cn/api/paas/v4", "zhipu"],
      ["https://api.z.ai/api/paas/v4", "zhipu"],
      ["https://api.minimaxi.com/v1", "minimax"],
      ["https://api.minimax.io/v1", "minimax"],
    ];

    it.each(hits)("detects %s as %s", (baseUrl, expected) => {
      expect(detectCodingPlanProvider(baseUrl)).toBe(expected);
    });
  });

  describe("不命中时返回 null（走 unsupported 降级）", () => {
    const misses: string[] = [
      "https://api.moonshot.ai/v1",
      "https://chatgpt.com/backend-api/codex",
      "https://api.deepseek.com/v1",
      // 特别注意：xiaomimimo 不得误命中 minimax
      "https://api.xiaomimimo.com/v1",
      "https://token-plan-cn.xiaomimimo.com/v1",
      "http://127.0.0.1:15721/openai",
    ];

    it.each(misses)("returns null for %s", (baseUrl) => {
      expect(detectCodingPlanProvider(baseUrl)).toBeNull();
    });

    it("returns null for undefined / null / empty input", () => {
      expect(detectCodingPlanProvider(undefined)).toBeNull();
      expect(detectCodingPlanProvider(null)).toBeNull();
      expect(detectCodingPlanProvider("")).toBeNull();
    });
  });

  it("bigmodel.cn 首匹配命中 zhipu 而非 zhipu_team（团队版仅靠显式参数路由）", () => {
    expect(detectCodingPlanProvider("https://open.bigmodel.cn/api/paas/v4")).toBe(
      "zhipu",
    );
    expect(detectCodingPlanProvider("https://open.bigmodel.cn/api/paas/v4")).not.toBe(
      "zhipu_team",
    );
  });
});

interface TestProvider {
  name?: string;
  settingsConfig?: Record<string, any>;
  meta?: Record<string, any>;
}

const kimiCodeProvider = (baseUrl: string): TestProvider => ({
  name: "test",
  settingsConfig: {
    config: `default_model = "test"

[providers.test]
name = "Test"
base_url = "${baseUrl}"
api_key = "sk-test"

[models.test]
provider = "test"
model = "test"
max_context_size = 262144
`,
  },
});

describe("injectCodingPlanUsageScript", () => {
  describe("kimi-code app", () => {
    it("kimi coding base_url 注入 token_plan/kimi", () => {
      const result = injectCodingPlanUsageScript(
        "kimi-code",
        kimiCodeProvider("https://api.kimi.com/coding/v1"),
      );
      expect(result.meta?.usage_script).toMatchObject({
        enabled: true,
        templateType: TEMPLATE_TYPES.TOKEN_PLAN,
        codingPlanProvider: "kimi",
      });
    });

    it("deepseek base_url 注入 balance", () => {
      const result = injectCodingPlanUsageScript(
        "kimi-code",
        kimiCodeProvider("https://api.deepseek.com/v1"),
      );
      expect(result.meta?.usage_script).toMatchObject({
        enabled: true,
        templateType: TEMPLATE_TYPES.BALANCE,
      });
      expect(result.meta?.usage_script).not.toHaveProperty(
        "codingPlanProvider",
      );
    });

    it.each([
      "https://api.xiaomimimo.com/v1",
      "https://api.moonshot.ai/v1",
      "https://chatgpt.com/backend-api/codex",
      "http://127.0.0.1:15721/openai",
    ])("%s 不注入", (baseUrl) => {
      const result = injectCodingPlanUsageScript(
        "kimi-code",
        kimiCodeProvider(baseUrl),
      );
      expect(result.meta?.usage_script).toBeUndefined();
    });

    it("TOML 解析失败/缺 base_url 不注入且不抛异常", () => {
      expect(
        injectCodingPlanUsageScript<TestProvider>("kimi-code", {
          settingsConfig: { config: "not = [valid toml" },
        }).meta?.usage_script,
      ).toBeUndefined();
      expect(
        injectCodingPlanUsageScript<TestProvider>("kimi-code", {
          settingsConfig: { config: '[providers.test]\nname = "T"\n' },
        }).meta?.usage_script,
      ).toBeUndefined();
      expect(
        injectCodingPlanUsageScript<TestProvider>("kimi-code", {
          settingsConfig: {},
        }).meta?.usage_script,
      ).toBeUndefined();
    });

    it("已有 usage_script 不覆盖", () => {
      const existing = { enabled: false, language: "javascript", code: "x" };
      const provider = {
        ...kimiCodeProvider("https://api.kimi.com/coding/v1"),
        meta: { usage_script: existing },
      };
      const result = injectCodingPlanUsageScript("kimi-code", provider);
      expect(result.meta?.usage_script).toBe(existing);
    });
  });

  describe("claude 既有行为不变", () => {
    it("kimi coding base_url 注入 token_plan/kimi", () => {
      const result = injectCodingPlanUsageScript<TestProvider>("claude", {
        settingsConfig: {
          env: { ANTHROPIC_BASE_URL: "https://api.kimi.com/coding/v1" },
        },
      });
      expect(result.meta?.usage_script).toMatchObject({
        enabled: true,
        templateType: TEMPLATE_TYPES.TOKEN_PLAN,
        codingPlanProvider: "kimi",
      });
    });

    it("不命中 base_url 不注入（含 deepseek，claude 不走 balance 分支）", () => {
      const result = injectCodingPlanUsageScript<TestProvider>("claude", {
        settingsConfig: {
          env: { ANTHROPIC_BASE_URL: "https://api.deepseek.com/v1" },
        },
      });
      expect(result.meta?.usage_script).toBeUndefined();
    });

    it("已有 usage_script 不覆盖", () => {
      const existing = { enabled: false, language: "javascript", code: "x" };
      const result = injectCodingPlanUsageScript<TestProvider>("claude", {
        settingsConfig: {
          env: { ANTHROPIC_BASE_URL: "https://api.kimi.com/coding/v1" },
        },
        meta: { usage_script: existing },
      });
      expect(result.meta?.usage_script).toBe(existing);
    });
  });

  it("其他 app 一律不注入", () => {
    const result = injectCodingPlanUsageScript(
      "codex",
      kimiCodeProvider("https://api.kimi.com/coding/v1"),
    );
    expect(result.meta?.usage_script).toBeUndefined();
  });
});
