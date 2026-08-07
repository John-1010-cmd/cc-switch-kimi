//! 官方供应商种子数据
//!
//! 启动时调用 `Database::init_default_official_providers` 把这些条目
//! 写入 `providers` 表，让所有用户都能看到一个"一键切回官方"的入口。
//!
//! 字段与前端预设保持一致，参见：
//! - `src/config/claudeProviderPresets.ts`（"Claude Official"）
//! - `src/config/codexProviderPresets.ts`（"OpenAI Official"）
//! - `src/config/geminiProviderPresets.ts`（"Google Official"）
//! - `src/components/providers/forms/GrokBuildProviderForm.tsx`（"Grok Official"）

use crate::app_config::AppType;

pub(crate) const CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID: &str = "claude-desktop-official";
pub(crate) const CODEX_OFFICIAL_PROVIDER_ID: &str = "codex-official";
pub(crate) const GROKBUILD_OFFICIAL_PROVIDER_ID: &str = "grokbuild-official";
pub(crate) const KIMI_CODE_OFFICIAL_PROVIDER_ID: &str = "kimi-code-official";

/// 单条官方供应商种子定义。
pub(crate) struct OfficialProviderSeed {
    pub id: &'static str,
    pub app_type: AppType,
    pub name: &'static str,
    pub website_url: &'static str,
    pub icon: &'static str,
    pub icon_color: &'static str,
    /// settings_config 的 JSON 字符串，每个 app 结构不同。
    pub settings_config_json: &'static str,
}

/// Claude / Claude Desktop / Codex / Gemini 的官方预设。
///
/// id 固定，便于幂等检查；name 直接用英文原名（与前端预设一致），不做 i18n。
pub(crate) const OFFICIAL_SEEDS: &[OfficialProviderSeed] = &[
    OfficialProviderSeed {
        id: "claude-official",
        app_type: AppType::Claude,
        name: "Claude Official",
        website_url: "https://www.anthropic.com/claude-code",
        icon: "anthropic",
        icon_color: "#D4915D",
        // 空 env 让用户走 Claude CLI 默认认证流程
        settings_config_json: r#"{"env":{}}"#,
    },
    OfficialProviderSeed {
        id: CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID,
        app_type: AppType::ClaudeDesktop,
        name: "Claude Desktop Official",
        website_url: "https://claude.ai/download",
        icon: "anthropic",
        icon_color: "#D4915D",
        // 空 env 只是占位；切换该 provider 时会恢复 Claude Desktop 1P 模式
        settings_config_json: r#"{"env":{}}"#,
    },
    OfficialProviderSeed {
        id: CODEX_OFFICIAL_PROVIDER_ID,
        app_type: AppType::Codex,
        name: "OpenAI Official",
        website_url: "https://chatgpt.com/codex",
        icon: "openai",
        icon_color: "#00A67E",
        // 空 auth + 空 config 让用户走 ChatGPT Plus/Pro OAuth
        settings_config_json: r#"{"auth":{},"config":""}"#,
    },
    OfficialProviderSeed {
        id: "gemini-official",
        app_type: AppType::Gemini,
        name: "Google Official",
        website_url: "https://ai.google.dev/",
        icon: "gemini",
        icon_color: "#4285F4",
        // 空 env + 空 config 让用户走 Google OAuth
        settings_config_json: r#"{"env":{},"config":{}}"#,
    },
    OfficialProviderSeed {
        id: GROKBUILD_OFFICIAL_PROVIDER_ID,
        app_type: AppType::GrokBuild,
        name: "Grok Official",
        website_url: "https://x.ai/grok",
        icon: "grok",
        icon_color: "currentColor",
        // 空 config = 不写自定义模型表，Grok CLI 回落到自带的 xAI OAuth 登录
        settings_config_json: r#"{"config":""}"#,
    },
    OfficialProviderSeed {
        id: KIMI_CODE_OFFICIAL_PROVIDER_ID,
        app_type: AppType::KimiCode,
        name: "Kimi For Coding Official",
        website_url: "https://www.kimi.com/code",
        icon: "kimi",
        icon_color: "currentColor",
        // §2.2 托管模板：managed:kimi-code block 是登录态路由声明（OAuth 凭据在
        // credentials/，由 CLI /login 管理，cc-switch 不接管）；形态对齐 GrokBuild
        // 种子的 {"config": <纯 TOML 文本>} 包装。
        settings_config_json: r#"{"config":"default_model = \"managed:kimi-code/kimi-for-coding\"\n\n[providers.\"managed:kimi-code\"]\ntype = \"kimi\"\nbase_url = \"https://api.kimi.com/coding/v1\"\noauth = { storage = \"file\", key = \"oauth/kimi-code\" }\n\n[models.\"managed:kimi-code/kimi-for-coding\"]\nprovider = \"managed:kimi-code\"\nmodel = \"kimi-for-coding\"\nmax_context_size = 262144\n"}"#,
    },
];

/// 判断给定的 provider id 是否属于内置官方种子。
///
/// 单一事实源：直接扫描 `OFFICIAL_SEEDS`，避免在多处重复维护 id 列表。
pub(crate) fn is_official_seed_id(id: &str) -> bool {
    OFFICIAL_SEEDS.iter().any(|seed| seed.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_seeds_include_claude_desktop() {
        let seed = OFFICIAL_SEEDS
            .iter()
            .find(|seed| seed.id == CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID)
            .expect("claude desktop official seed");

        assert_eq!(seed.app_type, AppType::ClaudeDesktop);
        assert!(is_official_seed_id(CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID));
    }

    #[test]
    fn official_seeds_include_grokbuild() {
        let seed = OFFICIAL_SEEDS
            .iter()
            .find(|seed| seed.id == GROKBUILD_OFFICIAL_PROVIDER_ID)
            .expect("grok build official seed");

        assert_eq!(seed.app_type, AppType::GrokBuild);
        assert!(is_official_seed_id(GROKBUILD_OFFICIAL_PROVIDER_ID));
        // 空 config = 官方登录态：切换时不注入自定义模型表
        assert_eq!(seed.settings_config_json, r#"{"config":""}"#);
    }

    #[test]
    fn official_seeds_include_kimi_code_managed_template() {
        let seed = OFFICIAL_SEEDS
            .iter()
            .find(|seed| seed.id == KIMI_CODE_OFFICIAL_PROVIDER_ID)
            .expect("kimi code official seed");

        assert_eq!(seed.app_type, AppType::KimiCode);
        assert!(is_official_seed_id(KIMI_CODE_OFFICIAL_PROVIDER_ID));

        // settings_config 必须是 {"config": "<合法 TOML 文本>"} 单键包装
        let settings: serde_json::Value =
            serde_json::from_str(seed.settings_config_json).expect("seed settings is valid JSON");
        let config_text = settings
            .get("config")
            .and_then(|v| v.as_str())
            .expect("seed settings has config string");

        let doc: toml::Value = toml::from_str(config_text).expect("managed template is valid TOML");
        assert_eq!(
            doc.get("default_model").and_then(|v| v.as_str()),
            Some("managed:kimi-code/kimi-for-coding")
        );
        let managed = doc
            .get("providers")
            .and_then(|p| p.get("managed:kimi-code"))
            .expect("managed:kimi-code provider block");
        assert_eq!(
            managed.get("base_url").and_then(|v| v.as_str()),
            Some(crate::kimi_code_config::KIMI_CODE_DEFAULT_BASE_URL)
        );
        // 托管 block 走 OAuth，不得内嵌 api_key
        assert!(managed.get("api_key").is_none());
        let model = doc
            .get("models")
            .and_then(|m| m.get("managed:kimi-code/kimi-for-coding"))
            .expect("managed model block");
        assert_eq!(
            model.get("max_context_size").and_then(|v| v.as_integer()),
            Some(262144)
        );
    }
}
