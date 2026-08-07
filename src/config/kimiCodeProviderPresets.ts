/**
 * Kimi Code 预设供应商配置模板
 *
 * 设计依据：custom/03-provider-presets.md §2（七家厂商十条预设矩阵）与 §1.3
 * （config.toml 的 provider/model 声明范式）。每条预设的 settingsConfig 是
 * `{"config": "<整份 config.toml 文本>"}` 单键 JSON——切换 = 整体重写
 * ~/.kimi-code/config.toml 的 default_model / [providers.*] / [models.*]。
 *
 * API Key 一律写 `[providers.<name>].api_key` 字段（不使用 env 子表，
 * Kimi Code 不读 shell 环境变量），模板中以 `${apiKey}` 占位，选中预设时经
 * applyTemplateValues（与 claude 预设同一套变量替换逻辑）替换后留空待填。
 */
import { ProviderCategory } from "../types";
import type { PresetTheme, TemplateValueConfig } from "./claudeProviderPresets";
import { KIMI_CODE_DEFAULT_CONFIG_TOML } from "./kimiCode";

export interface KimiCodeProviderPreset {
  name: string; // 显示名
  nameKey?: string; // i18n key for localized display name
  websiteUrl: string;
  // 第三方供应商可提供单独的获取 API Key 链接
  apiKeyUrl?: string;
  category?: ProviderCategory;
  isOfficial?: boolean; // 标识是否为官方预设
  isPartner?: boolean; // 标识是否为商业合作伙伴
  primePartner?: boolean; // 置顶合作伙伴（顶级）：徽章显示为心形
  partnerPromotionKey?: string; // 合作伙伴促销信息的 i18n key
  // 视觉主题配置（与 claude/codex 预设同构，ProviderPresetSelector 直接消费）
  theme?: PresetTheme;
  icon?: string; // 图标名称
  iconColor?: string; // 图标颜色
  // 托管 managed:kimi-code 条目 = true（oauth 模式，无 api_key，登录走 CLI /login）
  requiresOAuth?: boolean;
  // 整段 config.toml 模板，api_key 处为 ${apiKey} 占位符
  settingsConfig: { config: string };
  // 表单中 api_key 的模板变量，结构与 claude 预设的 templateValues 对齐，
  // 保持 ProviderForm 的 applyTemplateValues 替换逻辑可复用
  templateValues?: Record<string, TemplateValueConfig>;
}

/** API Key 占位符的模板变量定义（选中预设后留空，由用户在 TOML 中填写） */
function apiKeyTemplateValues(
  placeholder: string,
): Record<string, TemplateValueConfig> {
  return {
    apiKey: {
      label: "API Key",
      placeholder,
      defaultValue: "",
      editorValue: "",
    },
  };
}

/**
 * 生成 API Key 型供应商的 config.toml（§1.3 范式：
 * default_model + [providers.*] + [models.*]）。
 */
export function generateKimiCodeConfig(options: {
  /** [providers.<name>] 键名（小写短名，如 glm / minimax / deepseek / mimo） */
  providerName: string;
  /** 协议类型：官方客户端有专用 type 用专用，其余一律 openai */
  providerType: "kimi" | "openai" | "anthropic" | "openai_responses";
  /** 官方 SDK 有默认端点时（Claude 官方）可省略 */
  baseUrl?: string;
  /** 模型别名，统一 <provider>/<model> 格式 */
  alias: string;
  model: string;
  /** 必填，缺省 Kimi Code 启动报错 */
  maxContextSize: number;
  /** 中文友好名，用量页/切换器直接复用 */
  displayName: string;
  /** 仅当模型确实支持 thinking 时声明 */
  capabilities?: string[];
}): string {
  const {
    providerName,
    providerType,
    baseUrl,
    alias,
    model,
    maxContextSize,
    displayName,
    capabilities,
  } = options;
  const tomlString = (value: string) => JSON.stringify(value);

  const providerLines = [
    `type = ${tomlString(providerType)}`,
    `api_key = "\${apiKey}"`,
    ...(baseUrl ? [`base_url = ${tomlString(baseUrl)}`] : []),
  ].join("\n");

  const modelLines = [
    `provider = ${tomlString(providerName)}`,
    `model = ${tomlString(model)}`,
    `max_context_size = ${maxContextSize}`,
    `display_name = ${tomlString(displayName)}`,
    ...(capabilities
      ? [`capabilities = [${capabilities.map(tomlString).join(", ")}]`]
      : []),
  ].join("\n");

  return `default_model = ${tomlString(alias)}

[providers.${providerName}]
${providerLines}

[models.${tomlString(alias)}]
${modelLines}
`;
}

export const kimiCodeProviderPresets: KimiCodeProviderPreset[] = [
  // ===== Kimi 一家三形态（托管 OAuth / Coding 套餐 API Key / 开放平台） =====
  {
    name: "Kimi For Coding (Managed)",
    websiteUrl: "https://www.kimi.com/code/?aff=cc-switch",
    isOfficial: true,
    category: "official",
    // 托管 OAuth：无 api_key，oauth 凭据由 CLI /login 落
    // ~/.kimi-code/credentials/kimi-code.json；模板与 M1 默认配置同源
    requiresOAuth: true,
    settingsConfig: { config: KIMI_CODE_DEFAULT_CONFIG_TOML },
    icon: "kimi",
    iconColor: "#6366F1",
  },
  {
    name: "Kimi For Coding",
    primePartner: true,
    websiteUrl: "https://www.kimi.com/code/?aff=cc-switch",
    apiKeyUrl: "https://www.kimi.com/code/?aff=cc-switch",
    category: "cn_official",
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "kimi",
        providerType: "kimi",
        baseUrl: "https://api.kimi.com/coding/v1",
        alias: "kimi/kimi-for-coding",
        model: "kimi-for-coding",
        maxContextSize: 262144,
        displayName: "Kimi For Coding",
      }),
    },
    templateValues: apiKeyTemplateValues("请输入 Kimi For Coding API Key"),
    icon: "kimi",
    iconColor: "#6366F1",
  },
  {
    name: "Kimi",
    primePartner: true,
    websiteUrl: "https://platform.kimi.com?aff=cc-switch",
    apiKeyUrl: "https://platform.kimi.com/console/api-keys?aff=cc-switch",
    category: "cn_official",
    // 开放平台按 token 计费，无套餐额度，用量页走本地 wire 统计
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "kimi_open",
        providerType: "kimi",
        baseUrl: "https://api.moonshot.ai/v1",
        alias: "kimi-open/kimi-k2.7-code",
        model: "kimi-k2.7-code",
        maxContextSize: 262144,
        displayName: "Kimi K2.7 Code",
      }),
    },
    templateValues: apiKeyTemplateValues("请输入 Kimi 开放平台 API Key"),
    icon: "kimi",
    iconColor: "#6366F1",
  },
  // ===== 其余厂商：官方有专用 type 用专用，OpenAI 兼容一律 openai =====
  {
    name: "Zhipu GLM",
    websiteUrl: "https://open.bigmodel.cn",
    apiKeyUrl: "https://www.bigmodel.cn/claude-code?ic=RRVJPB5SII",
    category: "cn_official",
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "glm",
        providerType: "openai",
        baseUrl: "https://open.bigmodel.cn/api/paas/v4",
        alias: "glm/glm-5.2",
        model: "glm-5.2",
        maxContextSize: 204800,
        displayName: "GLM-5.2",
        capabilities: ["thinking"],
      }),
    },
    templateValues: apiKeyTemplateValues("请输入智谱 GLM API Key"),
    icon: "zhipu",
    iconColor: "#0F62FE",
  },
  {
    name: "MiniMax",
    websiteUrl: "https://platform.minimaxi.com",
    apiKeyUrl: "https://platform.minimaxi.com/subscribe/coding-plan",
    category: "cn_official",
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "minimax",
        providerType: "openai",
        baseUrl: "https://api.minimaxi.com/v1",
        alias: "minimax/MiniMax-M3",
        model: "MiniMax-M3",
        maxContextSize: 204800,
        displayName: "MiniMax M3",
        capabilities: ["thinking"],
      }),
    },
    templateValues: apiKeyTemplateValues("请输入 MiniMax API Key"),
    icon: "minimax",
    iconColor: "#FF6B6B",
  },
  {
    name: "Claude",
    websiteUrl: "https://console.anthropic.com",
    apiKeyUrl: "https://console.anthropic.com/settings/keys",
    category: "official",
    // base_url 留空走 SDK 默认官方端点；中转场景用户手改 TOML 补 base_url
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "claude",
        providerType: "anthropic",
        alias: "claude/claude-sonnet-4-6",
        model: "claude-sonnet-4-6",
        maxContextSize: 200000,
        displayName: "Claude Sonnet 4.6",
      }),
    },
    templateValues: apiKeyTemplateValues("请输入 Anthropic API Key"),
    icon: "anthropic",
    iconColor: "#D4915D",
  },
  {
    name: "Codex",
    websiteUrl: "https://chatgpt.com/codex",
    apiKeyUrl: "https://platform.openai.com/api-keys",
    category: "third_party",
    // ChatGPT OAuth 反代必须走本地代理；此模板为直连自填 key 场景
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "codex",
        providerType: "openai_responses",
        baseUrl: "https://chatgpt.com/backend-api/codex",
        alias: "codex/gpt-5.3-codex",
        model: "gpt-5.3-codex",
        maxContextSize: 400000,
        displayName: "GPT-5.3 Codex",
      }),
    },
    templateValues: apiKeyTemplateValues("请输入 OpenAI API Key"),
    icon: "openai",
    iconColor: "#000000",
  },
  {
    name: "DeepSeek",
    websiteUrl: "https://platform.deepseek.com",
    apiKeyUrl: "https://platform.deepseek.com/api_keys",
    category: "cn_official",
    // 无套餐额度 API，用量页降级为账户余额展示（设计 §4.3）
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "deepseek",
        providerType: "openai",
        baseUrl: "https://api.deepseek.com/v1",
        alias: "deepseek/deepseek-chat",
        model: "deepseek-chat",
        maxContextSize: 131072,
        displayName: "DeepSeek Chat",
      }),
    },
    templateValues: apiKeyTemplateValues("请输入 DeepSeek API Key"),
    icon: "deepseek",
    iconColor: "#1E88E5",
  },
  {
    name: "Xiaomi MiMo",
    websiteUrl: "https://platform.xiaomimimo.com",
    apiKeyUrl: "https://platform.xiaomimimo.com/#/console/api-keys",
    category: "cn_official",
    // 无额度 API，用量页仅展示本地 wire 统计（设计 §4.3）
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "mimo",
        providerType: "openai",
        baseUrl: "https://api.xiaomimimo.com/v1",
        alias: "mimo/mimo-v2.5-pro",
        model: "mimo-v2.5-pro",
        maxContextSize: 262144,
        displayName: "MiMo V2.5 Pro",
      }),
    },
    templateValues: apiKeyTemplateValues("请输入小米 MiMo API Key"),
    icon: "xiaomimimo",
    iconColor: "#000000",
  },
  {
    name: "Xiaomi MiMo Token Plan (China)",
    websiteUrl: "https://platform.xiaomimimo.com/#/token-plan",
    apiKeyUrl: "https://platform.xiaomimimo.com/#/console/plan-manage",
    category: "cn_official",
    settingsConfig: {
      config: generateKimiCodeConfig({
        providerName: "mimo_plan",
        providerType: "openai",
        baseUrl: "https://token-plan-cn.xiaomimimo.com/v1",
        alias: "mimo-plan/mimo-v2.5-pro",
        model: "mimo-v2.5-pro",
        maxContextSize: 262144,
        displayName: "MiMo V2.5 Pro (Token Plan)",
      }),
    },
    templateValues: apiKeyTemplateValues("请输入小米 MiMo Token Plan API Key"),
    icon: "xiaomimimo",
    iconColor: "#000000",
  },
];
