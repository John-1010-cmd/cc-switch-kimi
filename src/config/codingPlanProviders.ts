/**
 * Coding Plan 供应商的 base_url 路由表。
 *
 * 与后端 `src-tauri/src/services/coding_plan.rs::detect_provider` 保持一致：
 * 后端靠 `url.contains(...)` 做子串判断，前端这里用 RegExp 做同效匹配。
 * 新增供应商时改这一处即可（UsageScriptModal 下拉 + useProviderActions
 * 新建自动注入 + 托盘识别全部复用）。
 */
import { createUsageScript } from "@/types";
import { TEMPLATE_TYPES } from "@/config/constants";
import { extractKimiCodeBaseUrl } from "@/utils/providerConfigUtils";

/** DeepSeek 官方余额端点（与 UsageScriptModal 的 detectBalanceProvider 正则一致） */
const DEEPSEEK_BALANCE_PATTERN = /api\.deepseek\.com/i;

export interface CodingPlanProviderEntry {
  /** 与后端 QuotaTier 的 `codingPlanProvider` 取值对齐 */
  id: "kimi" | "zhipu" | "zhipu_team" | "minimax" | "zenmux" | "volcengine";
  /** UsageScriptModal 下拉显示用 */
  label: string;
  /** base_url 匹配规则 */
  pattern: RegExp;
}

export const CODING_PLAN_PROVIDERS: readonly CodingPlanProviderEntry[] = [
  { id: "kimi", label: "Kimi For Coding", pattern: /api\.kimi\.com\/coding/i },
  {
    id: "zhipu",
    label: "Zhipu GLM (智谱)",
    pattern: /bigmodel\.cn|api\.z\.ai/i,
  },
  {
    // 智谱团队套餐（Team Plan）。base_url 与个人版智谱（open.bigmodel.cn）相同，
    // 无法靠 base_url 自动区分——靠显式 codingPlanProvider === "zhipu_team" 路由。
    // 个人版 zhipu 排在前面，detectCodingPlanProvider 首匹配仍命中个人版，
    // 故团队版永不被 injectCodingPlanUsageScript 自动注入（必须用户手动选）。
    // pattern 仅占位（下拉展示用），实际不参与自动检测。
    id: "zhipu_team",
    label: "Zhipu GLM Team (智谱团队)",
    pattern: /bigmodel\.cn/i,
  },
  {
    id: "minimax",
    label: "MiniMax",
    pattern: /api\.minimaxi?\.com|api\.minimax\.io/i,
  },
  {
    id: "zenmux",
    label: "ZenMux",
    pattern: /zenmux\./i,
  },
  {
    // 火山方舟 Agent Plan / Coding Plan。base_url 形如
    // ark.cn-beijing.volces.com/api/coding[/v3]；与后端 detect_provider 的
    // `volces.com/api/coding` 子串判断同效。
    id: "volcengine",
    label: "火山方舟 (Volcengine)",
    pattern: /volces\.com\/api\/coding/i,
  },
] as const;

/** 根据 Base URL 自动检测 Coding Plan 供应商；未命中返回 null */
export function detectCodingPlanProvider(
  baseUrl: string | undefined | null,
): CodingPlanProviderEntry["id"] | null {
  if (!baseUrl) return null;
  for (const cp of CODING_PLAN_PROVIDERS) {
    if (cp.pattern.test(baseUrl)) return cp.id;
  }
  return null;
}

/**
 * 新建供应商时，按 app 自动注入用量脚本配置：
 *
 * - Claude：`settingsConfig.env.ANTHROPIC_BASE_URL` 命中 Coding Plan 路由表时，
 *   注入 token_plan（带 codingPlanProvider）
 * - kimi-code：`settingsConfig.config`（整份 config.toml）的 `[providers.*]`
 *   base_url 命中 Coding Plan 路由表时注入 token_plan；命中 DeepSeek 官方余额
 *   端点时注入 balance；其余（mimo/moonshot/本地代理等）不注入
 * - 仅在 `meta.usage_script` 完全缺失时注入，不覆盖用户/UsageScriptModal 已有配置
 * - code 置空：Rust 端走专用查询路径（coding_plan / balance），不执行 JS 脚本
 */
export function injectCodingPlanUsageScript<
  T extends {
    settingsConfig?: Record<string, any>;
    meta?: Record<string, any>;
  },
>(appId: string, provider: T): T {
  if (appId !== "claude" && appId !== "kimi-code") return provider;
  if (provider.meta?.usage_script) return provider;

  const baseUrl =
    appId === "claude"
      ? provider.settingsConfig?.env?.ANTHROPIC_BASE_URL
      : extractKimiCodeBaseUrl(provider.settingsConfig?.config);
  if (typeof baseUrl !== "string" || !baseUrl) return provider;

  const codingPlanProvider = detectCodingPlanProvider(baseUrl);
  if (codingPlanProvider) {
    return {
      ...provider,
      meta: {
        ...(provider.meta ?? {}),
        usage_script: createUsageScript({
          enabled: true,
          templateType: TEMPLATE_TYPES.TOKEN_PLAN,
          codingPlanProvider,
        }),
      },
    };
  }

  // DeepSeek 官方余额端点（仅 kimi-code 分支；保持 claude 既有行为不变）：
  // 注入 balance 模板，与 UsageScriptModal 的自动选择逻辑对齐，balance 模板无额外字段
  if (appId === "kimi-code" && DEEPSEEK_BALANCE_PATTERN.test(baseUrl)) {
    return {
      ...provider,
      meta: {
        ...(provider.meta ?? {}),
        usage_script: createUsageScript({
          enabled: true,
          templateType: TEMPLATE_TYPES.BALANCE,
        }),
      },
    };
  }

  return provider;
}
