import { describe, expect, it } from "vitest";
import TOML from "smol-toml";
import { kimiCodeProviderPresets } from "./kimiCodeProviderPresets";
import { applyTemplateValues } from "../utils/providerConfigUtils";

/** 与 ProviderForm 选中预设时一致：先替换 ${apiKey} 占位符再取 TOML 文本 */
function resolveToml(index: number): string {
  const preset = kimiCodeProviderPresets[index];
  const resolved = applyTemplateValues(
    preset.settingsConfig,
    preset.templateValues,
  ) as { config: string };
  return resolved.config;
}

function presetByName(name: string) {
  const preset = kimiCodeProviderPresets.find((p) => p.name === name);
  expect(preset, name).toBeDefined();
  return preset!;
}

describe("kimiCodeProviderPresets", () => {
  it("has ten presets with unique names", () => {
    const names = kimiCodeProviderPresets.map((p) => p.name);
    expect(names.length).toBe(10);
    expect(new Set(names).size).toBe(names.length);
  });

  it("generates TOML with default_model / [providers.*] / [models.*] / max_context_size", () => {
    for (let i = 0; i < kimiCodeProviderPresets.length; i++) {
      const toml = resolveToml(i);
      const name = kimiCodeProviderPresets[i].name;
      expect(toml, name).toContain("default_model");
      expect(toml, name).toContain("[providers.");
      expect(toml, name).toContain("[models.");
      expect(toml, name).toContain("max_context_size");
    }
  });

  it("parses as valid TOML with a consistent default_model -> models alias", () => {
    for (let i = 0; i < kimiCodeProviderPresets.length; i++) {
      const name = kimiCodeProviderPresets[i].name;
      const parsed = TOML.parse(resolveToml(i)) as Record<string, any>;

      expect(typeof parsed.default_model, name).toBe("string");
      const providers = parsed.providers as Record<string, any>;
      const models = parsed.models as Record<string, any>;
      expect(Object.keys(providers).length, name).toBeGreaterThan(0);
      // default_model 必须指向某个 [models."<alias>"] 条目
      const modelEntry = models[parsed.default_model];
      expect(modelEntry, name).toBeDefined();
      expect(typeof modelEntry.max_context_size, name).toBe("number");
      // 模型条目反向引用的 provider 必须存在
      expect(providers[modelEntry.provider], name).toBeDefined();
    }
  });

  it("uses type kimi for the three Kimi entries", () => {
    for (const name of [
      "Kimi For Coding (Managed)",
      "Kimi For Coding",
      "Kimi",
    ]) {
      const preset = presetByName(name);
      const parsed = TOML.parse(
        (preset.settingsConfig as { config: string }).config,
      ) as Record<string, any>;
      const provider = Object.values(parsed.providers as object)[0] as any;
      expect(provider.type, name).toBe("kimi");
    }
  });

  it("uses type openai with the matrix base_url for glm/minimax/deepseek/mimo", () => {
    const cases: Array<[string, string, string]> = [
      ["Zhipu GLM", "glm", "https://open.bigmodel.cn/api/paas/v4"],
      ["MiniMax", "minimax", "https://api.minimaxi.com/v1"],
      ["DeepSeek", "deepseek", "https://api.deepseek.com/v1"],
      ["Xiaomi MiMo", "mimo", "https://api.xiaomimimo.com/v1"],
      [
        "Xiaomi MiMo Token Plan (China)",
        "mimo_plan",
        "https://token-plan-cn.xiaomimimo.com/v1",
      ],
    ];
    for (const [name, providerName, baseUrl] of cases) {
      const preset = presetByName(name);
      const parsed = TOML.parse(
        (preset.settingsConfig as { config: string }).config,
      ) as Record<string, any>;
      const provider = (parsed.providers as Record<string, any>)[providerName];
      expect(provider.type, name).toBe("openai");
      expect(provider.base_url, name).toBe(baseUrl);
    }
  });

  it("uses dedicated types for Claude (anthropic) and Codex (openai_responses)", () => {
    const claude = TOML.parse(
      (presetByName("Claude").settingsConfig as { config: string }).config,
    ) as Record<string, any>;
    expect((claude.providers as any).claude.type).toBe("anthropic");

    const codex = TOML.parse(
      (presetByName("Codex").settingsConfig as { config: string }).config,
    ) as Record<string, any>;
    const codexProvider = (codex.providers as any).codex;
    expect(codexProvider.type).toBe("openai_responses");
    expect(codexProvider.base_url).toBe(
      "https://chatgpt.com/backend-api/codex",
    );
  });

  it("marks the managed entry requiresOAuth with no api_key in its TOML", () => {
    const managed = presetByName("Kimi For Coding (Managed)");
    expect(managed.requiresOAuth).toBe(true);
    const toml = (managed.settingsConfig as { config: string }).config;
    expect(toml).not.toContain("api_key");
    expect(toml).toContain(
      'oauth = { storage = "file", key = "oauth/kimi-code" }',
    );
  });

  it("leaves an empty api_key after template substitution on key-based presets", () => {
    for (let i = 0; i < kimiCodeProviderPresets.length; i++) {
      const preset = kimiCodeProviderPresets[i];
      if (preset.requiresOAuth) continue;
      const parsed = TOML.parse(resolveToml(i)) as Record<string, any>;
      const provider = Object.values(parsed.providers as object)[0] as any;
      expect(provider.api_key, preset.name).toBe("");
    }
  });
});
