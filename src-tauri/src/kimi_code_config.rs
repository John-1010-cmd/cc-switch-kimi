//! Kimi Code 配置文件管理模块
//!
//! Kimi Code（CLI 与 VSCode 扩展共享同一个 `$KIMI_CODE_HOME`）的配置布局：
//! - `config.toml`：供应商路由（`default_model` + `[providers.*]` + `[models.*]`），
//!   cc-switch 切换供应商 = 整体原子覆写该文件（switch 模式，与 Codex 一致）。
//! - `mcp.json`：MCP 服务器（`{"mcpServers": {...}}`），见 `mcp/kimi.rs`。
//! - `credentials/`：OAuth 凭据（0600），由 CLI 的 `/login` 设备码流程产生与刷新；
//!   cc-switch 只读不写，切换供应商时不得触碰。
//! - `sessions/`：官方明确禁止修改，全链路只读。

use std::path::{Path, PathBuf};

use crate::config::{atomic_write, get_home_dir};
use crate::error::AppError;

/// 托管供应商（Kimi For Coding 订阅额度路径）的默认 base_url。
/// M3 额度查询（`GET {base_url}/usages`，`services/subscription_kimi.rs`）使用。
pub const KIMI_CODE_DEFAULT_BASE_URL: &str = "https://api.kimi.com/coding/v1";
/// 托管供应商的 OAuth 认证主机。M3 凭据槽位推导（oauth key sha256 输入）使用。
pub const KIMI_CODE_OAUTH_HOST: &str = "https://auth.kimi.com";
/// 官方托管模板中的 provider block 名（`[providers."managed:kimi-code"]`）。
/// 该 block 是登录态的路由声明，任何第三方配置中被用户保留时原样保留。
pub const KIMI_CODE_MANAGED_PROVIDER: &str = "managed:kimi-code";

/// 获取 Kimi Code 配置目录：`KIMI_CODE_HOME` 环境变量优先，缺省 `~/.kimi-code`。
///
/// 不做 settings 级 override（与 Codex 的 `get_codex_override_dir()` 模式不同，
/// 设计决策见 custom/01 §1.1 #21）。
pub fn get_kimi_code_config_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("KIMI_CODE_HOME") {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    get_home_dir().join(".kimi-code")
}

/// 获取 Kimi Code `config.toml` 路径
pub fn get_kimi_code_config_path() -> PathBuf {
    get_kimi_code_config_dir().join("config.toml")
}

/// 获取 Kimi Code `mcp.json` 路径
pub fn get_kimi_code_mcp_path() -> PathBuf {
    get_kimi_code_config_dir().join("mcp.json")
}

/// 获取 Kimi Code `credentials/` 目录（OAuth 凭据，cc-switch 只读）
// M3 订阅额度查询（`services/subscription_kimi.rs`）读取。
pub fn get_kimi_code_credentials_dir() -> PathBuf {
    get_kimi_code_config_dir().join("credentials")
}

/// 对非空的 TOML 文本进行语法校验（空文本视为合法，等价于"清空配置"）。
///
/// 先用 `toml_edit` 解析（保留注释/格式的 crate，live 写回链路的实际格式载体）；
/// 失败时用 `toml` crate 再解析一次拿到 `toml::de::Error`，以复用既有
/// `AppError::toml` 变体与前端文案映射。
pub fn validate_kimi_code_config_toml(text: &str) -> Result<(), AppError> {
    if text.trim().is_empty() {
        return Ok(());
    }

    match text.parse::<toml_edit::DocumentMut>() {
        Ok(_) => Ok(()),
        Err(edit_err) => match toml::from_str::<toml::Table>(text) {
            Err(de_err) => Err(AppError::toml(Path::new("config.toml"), de_err)),
            Ok(_) => Err(AppError::Message(format!(
                "TOML 解析错误: config.toml: {edit_err}"
            ))),
        },
    }
}

/// 原子写 Kimi Code 的 `config.toml`。
///
/// 非空文本先做 TOML 语法校验，**校验失败不落盘**，live 文件保持旧内容。
/// 只写 `config.toml` 本身，绝不触碰 `credentials/` 与 `sessions/`。
pub fn write_kimi_code_live_config_atomic(text: &str) -> Result<(), AppError> {
    let config_path = get_kimi_code_config_path();

    validate_kimi_code_config_toml(text)?;

    atomic_write(&config_path, text.as_bytes())
}

/// 读取 `config.toml`，若不存在返回空字符串
pub fn read_kimi_code_config_text() -> Result<String, AppError> {
    let path = get_kimi_code_config_path();
    if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))
    } else {
        Ok(String::new())
    }
}

/// 读取并校验 `config.toml`，返回文本（可能为空）。
///
/// 存量配置语法损坏时报错并阻止切换覆写（用户先在 CLI 侧修复），
/// 避免 cc-switch 把坏文件"洗白"成自己的模板后丢失用户现场。
pub fn read_and_validate_kimi_code_config_text() -> Result<String, AppError> {
    let s = read_kimi_code_config_text()?;
    validate_kimi_code_config_toml(&s)?;
    Ok(s)
}

/// 判断 live 的 config.toml 是否为"官方登录态"：`default_model` 路由到
/// `managed:kimi-code` provider block（OAuth 订阅额度路径，无自定义第三方供应商）。
///
/// 与 `grok_config::is_official_live_config` 同语义：用于手动导入时识别
/// "官方条目应成为当前供应商"的场景，避免重复产出 default 供应商。
pub fn is_official_live_config(config_text: &str) -> bool {
    let trimmed = config_text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Ok(doc) = trimmed.parse::<toml::Value>() else {
        return false;
    };
    let Some(default_model) = doc.get("default_model").and_then(|v| v.as_str()) else {
        return false;
    };
    default_model.split('/').next() == Some(KIMI_CODE_MANAGED_PROVIDER)
}

/// 从 config.toml 文本中提取上游 base_url：`default_model`（`<provider>/<model>`）
/// 指向的 provider block 优先，否则取 `[providers.*]` 中首个带非空 base_url 的 block。
pub fn extract_kimi_code_base_url(config_text: &str) -> Option<String> {
    let doc = config_text.parse::<toml::Value>().ok()?;
    let providers = doc.get("providers")?.as_table()?;

    let non_empty_base_url = |block: &toml::Value| {
        block
            .get("base_url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|url| url.trim_end_matches('/').to_string())
    };

    if let Some(default_model) = doc.get("default_model").and_then(|v| v.as_str()) {
        let provider_id = default_model.split('/').next().unwrap_or("");
        if let Some(url) = providers.get(provider_id).and_then(non_empty_base_url) {
            return Some(url);
        }
    }

    providers.values().find_map(non_empty_base_url)
}

/// 从 config.toml 文本中提取首个非空 `api_key`（`managed:kimi-code` block 走 OAuth、
/// 通常没有 `api_key`，自然被跳过，不视为错误——托管凭据由订阅额度路径从
/// `credentials/*.json` 读取）。
pub fn extract_kimi_code_api_key(config_text: &str) -> Option<String> {
    let doc = config_text.parse::<toml::Value>().ok()?;
    let providers = doc.get("providers")?.as_table()?;

    let non_empty_api_key = |block: &toml::Value| {
        block
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string)
    };

    if let Some(default_model) = doc.get("default_model").and_then(|v| v.as_str()) {
        let provider_id = default_model.split('/').next().unwrap_or("");
        if let Some(key) = providers.get(provider_id).and_then(non_empty_api_key) {
            return Some(key);
        }
    }

    providers.values().find_map(non_empty_api_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    // KIMI_CODE_HOME 是进程级环境变量，所有读写它的用例必须串行，
    // 避免与并行测试线程互相污染。
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct KimiCodeHomeGuard {
        prev: Option<std::ffi::OsString>,
    }

    impl KimiCodeHomeGuard {
        fn set(dir: &Path) -> Self {
            let prev = std::env::var_os("KIMI_CODE_HOME");
            std::env::set_var("KIMI_CODE_HOME", dir);
            Self { prev }
        }
    }

    impl Drop for KimiCodeHomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(value) => std::env::set_var("KIMI_CODE_HOME", value),
                None => std::env::remove_var("KIMI_CODE_HOME"),
            }
        }
    }

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cc-switch-kimi-code-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn kimi_code_home_env_overrides_all_paths() {
        let _guard_lock = env_lock().lock().unwrap();
        let dir = unique_temp_dir("env");
        let _guard = KimiCodeHomeGuard::set(&dir);

        assert_eq!(get_kimi_code_config_dir(), dir);
        assert_eq!(get_kimi_code_config_path(), dir.join("config.toml"));
        assert_eq!(get_kimi_code_mcp_path(), dir.join("mcp.json"));
        assert_eq!(get_kimi_code_credentials_dir(), dir.join("credentials"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_config_dir_falls_back_to_home_dot_kimi_code() {
        let _guard_lock = env_lock().lock().unwrap();
        let prev = std::env::var_os("KIMI_CODE_HOME");
        std::env::remove_var("KIMI_CODE_HOME");
        let _restore = KimiCodeHomeGuard { prev };

        assert_eq!(get_kimi_code_config_dir(), get_home_dir().join(".kimi-code"));
    }

    #[test]
    fn atomic_write_roundtrip_preserves_text_exactly() {
        let _guard_lock = env_lock().lock().unwrap();
        let dir = unique_temp_dir("roundtrip");
        let _guard = KimiCodeHomeGuard::set(&dir);

        let text = r#"default_model = "managed:kimi-code/kimi-for-coding"

# 用户注释必须原样保留
[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
oauth = { storage = "file", key = "oauth/kimi-code" }

[models."managed:kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 262144
"#;

        write_kimi_code_live_config_atomic(text).expect("write valid toml");
        let read_back = read_and_validate_kimi_code_config_text().expect("read back");
        assert_eq!(read_back, text, "live 文件字节内容必须与写入文本一致");

        // 再切换另一家：原内容被完整覆写
        let other = "default_model = \"zhipu/glm-4.6\"\n[providers.zhipu]\ntype = \"openai\"\n";
        write_kimi_code_live_config_atomic(other).expect("overwrite with another provider");
        let read_back = read_and_validate_kimi_code_config_text().expect("read back");
        assert_eq!(read_back, other);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_toml_is_rejected_without_touching_disk() {
        let _guard_lock = env_lock().lock().unwrap();
        let dir = unique_temp_dir("invalid");
        let _guard = KimiCodeHomeGuard::set(&dir);

        let good = "default_model = \"zhipu/glm-4.6\"\n";
        write_kimi_code_live_config_atomic(good).expect("seed valid config");

        let err = write_kimi_code_live_config_atomic("default_model = [broken\n")
            .expect_err("invalid toml must be rejected");
        assert!(
            matches!(err, AppError::Toml { .. } | AppError::Message(_)),
            "unexpected error variant: {err:?}"
        );

        let on_disk = std::fs::read_to_string(get_kimi_code_config_path()).expect("read live");
        assert_eq!(on_disk, good, "校验失败不得落盘，live 保持旧内容");

        // 存量坏文件同样被 read_and_validate 拦截（阻止切换覆写把坏文件洗白）
        std::fs::write(get_kimi_code_config_path(), "default_model = [broken\n")
            .expect("corrupt live");
        assert!(read_and_validate_kimi_code_config_text().is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_text_is_allowed_and_means_empty_config() {
        let _guard_lock = env_lock().lock().unwrap();
        let dir = unique_temp_dir("empty");
        let _guard = KimiCodeHomeGuard::set(&dir);

        write_kimi_code_live_config_atomic("").expect("empty text is valid");
        assert_eq!(read_kimi_code_config_text().expect("read"), "");

        std::fs::remove_dir_all(&dir).ok();
    }
}
