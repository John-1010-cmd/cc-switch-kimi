/**
 * Kimi Code 默认托管配置模板（managed:kimi-code）。
 *
 * 每个 kimi-code 供应商的 settingsConfig 固定为 `{"config": "<整份 config.toml 文本>"}`
 * 单键 JSON（switch 模式，与 Codex 的 TOML 暂存形态一致，但没有 auth 键——
 * 登录态由 CLI 的 /login 设备码流程管理，落在 ~/.kimi-code/credentials/）。
 *
 * 供应商预设（7 家）归 M4（kimiCodeProviderPresets），本文件只放默认托管模板。
 */
export const KIMI_CODE_DEFAULT_CONFIG_TOML = `default_model = "managed:kimi-code/kimi-for-coding"

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
oauth = { storage = "file", key = "oauth/kimi-code" }

[models."managed:kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 262144
`;

/** ProviderForm 的 settingsConfig 字段初值（zod schema 要求是合法 JSON 文本） */
export const KIMI_CODE_DEFAULT_SETTINGS_CONFIG = JSON.stringify(
  { config: KIMI_CODE_DEFAULT_CONFIG_TOML },
  null,
  2,
);
