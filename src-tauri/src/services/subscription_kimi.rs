//! Kimi Code 托管订阅额度查询（设计文档 02 §1.5/§3，通路 B）
//!
//! 只读 `$KIMI_CODE_HOME/credentials/<key>.json` 的 OAuth access_token，
//! 调 `GET {base_url}/usages` 拿 5h 滚动窗 / 周额度 / 加油包三件套。
//!
//! 合规红线（风险登记册 R2/R5）：
//! - **只读凭据、不写回**：refresh_token 由官方 CLI 自刷新轮换，第三方写回
//!   会与 CLI 互踢丢 token。401 时重读磁盘一次（防 CLI 刚刷新过），仍失败
//!   则 `credentialStatus=expired`，引导用户在 CLI 执行 `/login`。
//! - 不实现 device_authorization 登录流、不伪造 `X-Msh-*` 设备头、不改 UA。

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Duration;

use super::subscription::{CredentialStatus, ExtraUsage, QuotaTier, SubscriptionQuota};
use crate::kimi_code_config::{
    extract_kimi_code_api_key, get_kimi_code_credentials_dir, read_kimi_code_config_text,
    KIMI_CODE_DEFAULT_BASE_URL, KIMI_CODE_MANAGED_PROVIDER, KIMI_CODE_OAUTH_HOST,
};

const TOOL_NAME: &str = "kimi-code";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

// detail/usage 容器字段的多别名优先级链（proto3 网关漂移防御，02 §1.5）
const LIMIT_KEYS: &[&str] = &["limit", "max", "total"];
const USED_KEYS: &[&str] = &["used", "consumed"];
const REMAINING_KEYS: &[&str] = &["remaining", "left"];
const RESET_KEYS: &[&str] = &["resetAt", "reset_at", "resetTime", "reset_time"];

// ── 端点与凭据槽位推导 ─────────────────────────────────────

/// 有效端点：`KIMI_CODE_BASE_URL`/`KIMI_CODE_OAUTH_HOST` 环境变量
/// > config.toml `[providers."managed:kimi-code"]` block > 内置默认。
#[derive(Debug, Clone, PartialEq, Eq)]
struct KimiEndpoint {
    base_url: String,
    oauth_host: String,
}

fn trim_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn read_env_url(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| trim_url(&v))
        .filter(|v| !v.is_empty())
}

/// 从 config.toml 文本提取 `[providers."managed:kimi-code"]` block 的 base_url
fn managed_block_base_url(config_text: &str) -> Option<String> {
    let doc = config_text.parse::<toml::Value>().ok()?;
    doc.get("providers")?
        .get(KIMI_CODE_MANAGED_PROVIDER)?
        .get("base_url")?
        .as_str()
        .map(trim_url)
        .filter(|url| !url.is_empty())
}

fn resolve_kimi_endpoint(config_text: &str) -> KimiEndpoint {
    let base_url = read_env_url("KIMI_CODE_BASE_URL")
        .or_else(|| managed_block_base_url(config_text))
        .unwrap_or_else(|| KIMI_CODE_DEFAULT_BASE_URL.to_string());
    let oauth_host = read_env_url("KIMI_CODE_OAUTH_HOST")
        .unwrap_or_else(|| KIMI_CODE_OAUTH_HOST.to_string());
    KimiEndpoint {
        base_url,
        oauth_host,
    }
}

/// 凭据槽位 key（字节级对齐官方 `resolveKimiCodeOAuthKey()`）：
/// 默认端点 → `kimi-code`；端点被覆盖 → `oauth/kimi-code-env-<sha256[:16]>`，
/// sha256 输入为 `{"oauthHost":...,"baseUrl":...}` 的 JSON 序列化，
/// 属性顺序固定（oauthHost 在前），不能变。
fn credential_key(endpoint: &KimiEndpoint) -> String {
    let is_default = endpoint.base_url == KIMI_CODE_DEFAULT_BASE_URL
        && endpoint.oauth_host == KIMI_CODE_OAUTH_HOST;
    if is_default {
        return "kimi-code".to_string();
    }

    // 手动拼接保证属性顺序与无空白序列化；字符串经 serde_json 转义
    let oauth = serde_json::to_string(&endpoint.oauth_host).unwrap_or_default();
    let base = serde_json::to_string(&endpoint.base_url).unwrap_or_default();
    let input = format!("{{\"oauthHost\":{oauth},\"baseUrl\":{base}}}");
    let digest = Sha256::digest(input.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("oauth/kimi-code-env-{}", &hex[..16])
}

fn credential_path(endpoint: &KimiEndpoint) -> PathBuf {
    get_kimi_code_credentials_dir().join(format!("{}.json", credential_key(endpoint)))
}

// ── 凭据读取（只读，0600 私密文件，内容不入日志）─────────────────

/// expires_at 兼容：秒 / 毫秒 / ISO8601 / 小数 / 0（=未知，不判过期）
fn kimi_token_expired(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let expires_ms: Option<i64> = match value {
        Value::Number(n) => n.as_f64().filter(|f| *f > 0.0).map(|f| {
            let v = f as i64;
            if v < 100_000_000_000 {
                v.saturating_mul(1000)
            } else {
                v
            }
        }),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                Some(dt.timestamp_millis())
            } else {
                s.parse::<f64>().ok().filter(|f| *f > 0.0).map(|f| {
                    let v = f as i64;
                    if v < 100_000_000_000 {
                        v.saturating_mul(1000)
                    } else {
                        v
                    }
                })
            }
        }
        _ => None,
    };
    let Some(expires_ms) = expires_ms else {
        return false;
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    expires_ms <= now_ms
}

/// 读取 OAuth 凭据文件。返回 `(access_token, status, message)`。
fn read_kimi_credentials(
    endpoint: &KimiEndpoint,
) -> (Option<String>, CredentialStatus, Option<String>) {
    let path = credential_path(endpoint);
    if !path.is_file() {
        return (None, CredentialStatus::NotFound, None);
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read credentials: {e}")),
            )
        }
    };
    let json: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse credentials: {e}")),
            )
        }
    };

    let token = json
        .get("access_token")
        .or_else(|| json.get("accessToken"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    let Some(token) = token else {
        return (
            None,
            CredentialStatus::ParseError,
            Some("Credentials file has no access_token".to_string()),
        );
    };

    let expires_at = json.get("expires_at").or_else(|| json.get("expiresAt"));
    if kimi_token_expired(expires_at) {
        // 本地判定过期也先返回 token：调用方仍会尝试一次 API（token 可能
        // 实际仍有效，或 CLI 已刷新但本地文件时间戳字段漂移）
        (
            Some(token),
            CredentialStatus::Expired,
            Some("OAuth token may have expired".to_string()),
        )
    } else {
        (Some(token), CredentialStatus::Valid, None)
    }
}

// ── /usages 查询与错误分类 ──────────────────────────────────

enum KimiQueryError {
    /// 瞬时失败（网络/超时/读体中断/5xx/429）→ command reject，前端
    /// react-query retry + keep-last-good
    Transient(String),
    /// 401/403 鉴权失败 → 重读磁盘重试一次后转 expired
    Unauthorized,
    /// 404：非托管端点（代理/自托管网关）无此 API → 确定性失败
    UnsupportedEndpoint,
    /// 响应解析失败 → 确定性失败
    ParseFailure(String),
}

async fn query_kimi_usages_once(base_url: &str, token: &str) -> Result<Value, KimiQueryError> {
    let client = crate::proxy::http_client::get();
    let resp = client
        .get(format!("{base_url}/usages"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| KimiQueryError::Transient(format!("Network error: {e}")))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(KimiQueryError::Unauthorized);
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(KimiQueryError::UnsupportedEndpoint);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Err(KimiQueryError::Transient(format!(
            "API temporarily unavailable (HTTP {status})"
        )));
    }
    if !status.is_success() {
        return Err(KimiQueryError::Transient(format!("API error (HTTP {status})")));
    }

    // 先 bytes() 再解析：读体失败是瞬时，拿到完整响应体后解析失败才是确定性
    let raw = resp
        .bytes()
        .await
        .map_err(|e| KimiQueryError::Transient(format!("Failed to read response: {e}")))?;
    serde_json::from_slice(&raw)
        .map_err(|e| KimiQueryError::ParseFailure(format!("Failed to parse response: {e}")))
}

// ── 响应解析（全字段 Option + 多别名 + proto3 字符串数字宽松解析）──────────

fn alias<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| value.get(*k))
}

/// proto3 风格宽松数值解析：数字 / 数字字符串 / 小数均接受
fn parse_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// reset 时间归一为 ISO 8601：ISO 字符串原样透传，epoch 秒/毫秒转 UTC ISO
fn parse_reset_time(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                Some(dt.to_rfc3339())
            } else if let Ok(n) = s.parse::<f64>() {
                epoch_to_iso(n as i64)
            } else {
                Some(s.to_string())
            }
        }
        Value::Number(n) => n.as_f64().map(|f| epoch_to_iso(f as i64))?,
        _ => None,
    }
}

fn epoch_to_iso(v: i64) -> Option<String> {
    if v <= 0 {
        return None;
    }
    let ms = if v < 100_000_000_000 {
        v.saturating_mul(1000)
    } else {
        v
    };
    chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339())
}

/// 5h 窗口判定：把时间窗归一成分钟后比较 == 300，兼容
/// `300 MINUTE` / `5 HOUR` / `18000 SECOND` / `0.208 DAY` 等一切等价形态；
/// 非 5h 的已知单位（如 `7 DAY`）返回 Some(false)，window 缺失或单位
/// 不认识返回 None（调用方走"无名首项兜底"链）。
fn is_five_hour_window(limit_item: &Value) -> Option<bool> {
    let window = limit_item.get("window")?;
    let duration = parse_f64(window.get("duration"))?;
    let unit = window
        .get("timeUnit")
        .or_else(|| window.get("time_unit"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_uppercase();
    let minutes = if unit.contains("MINUTE") {
        duration
    } else if unit.contains("HOUR") {
        duration * 60.0
    } else if unit.contains("SECOND") {
        duration / 60.0
    } else if unit.contains("DAY") {
        duration * 1440.0
    } else {
        return None;
    };
    Some((minutes - 300.0).abs() < f64::EPSILON)
}

/// 从 detail/usage 容器构建一个 tier；字段全缺时返回 None
fn tier_from_container(name: &str, container: &Value) -> Option<QuotaTier> {
    let limit = parse_f64(alias(container, LIMIT_KEYS));
    let used_direct = parse_f64(alias(container, USED_KEYS));
    let remaining = parse_f64(alias(container, REMAINING_KEYS));
    let resets_at = parse_reset_time(alias(container, RESET_KEYS));

    // used 缺失时 limit - remaining 兜底
    let used = used_direct.or_else(|| match (limit, remaining) {
        (Some(l), Some(r)) => Some((l - r).max(0.0)),
        _ => None,
    });
    let utilization = match (used, limit) {
        (Some(u), Some(l)) if l > 0.0 => (u / l) * 100.0,
        (Some(_), Some(_)) => 0.0,
        _ => return None,
    };

    Some(QuotaTier {
        name: name.to_string(),
        utilization,
        resets_at,
        used_value_usd: None,
        max_value_usd: None,
    })
}

/// 加油包（boosterWallet）→ ExtraUsage：
/// - `status ∈ {STATUS_ACTIVE, STATUS_ENABLED}` 才生效，否则按 0 显示；
/// - `monthlyUsed.priceInCents ÷100`（元）、`monthlyChargeLimit.priceInCents ÷100`；
/// - `monthlyChargeLimitEnabled` 省略即无上限（monthly_limit = None）。
fn extra_usage_from_booster_wallet(body: &Value) -> Option<ExtraUsage> {
    let wallet = body.get("boosterWallet")?;
    let status = wallet.get("status").and_then(Value::as_str).unwrap_or("");
    let active = matches!(
        status,
        "STATUS_ACTIVE" | "STATUS_ENABLED" | "ACTIVE" | "ENABLED"
    );

    let cents_to_yuan = |v: Option<&Value>| parse_f64(v).map(|c| c / 100.0);
    let used_credits = if active {
        cents_to_yuan(wallet.get("monthlyUsed").and_then(|m| m.get("priceInCents")))
    } else {
        Some(0.0)
    };

    let limit_enabled = wallet
        .get("monthlyChargeLimitEnabled")
        .map(|v| match v {
            Value::Bool(b) => *b,
            Value::String(s) => s.trim().eq_ignore_ascii_case("true"),
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            _ => false,
        })
        .unwrap_or(false);
    let monthly_limit = if active && limit_enabled {
        cents_to_yuan(
            wallet
                .get("monthlyChargeLimit")
                .and_then(|m| m.get("priceInCents")),
        )
    } else {
        None
    };

    let utilization = match (used_credits, monthly_limit) {
        (Some(u), Some(l)) if l > 0.0 => Some((u / l) * 100.0),
        _ => None,
    };

    Some(ExtraUsage {
        is_enabled: active,
        monthly_limit,
        used_credits,
        utilization,
        currency: Some("CNY".to_string()),
    })
}

/// `/usages` 响应体 → 统一 SubscriptionQuota（5h / 周 / 加油包三件套）
fn kimi_code_quota_from_body(body: &Value) -> SubscriptionQuota {
    let mut tiers = Vec::new();
    let limits = body.get("limits").and_then(Value::as_array);

    // 5h 滚动窗：window 判定优先，无 5h 项时取 limits[0] 兜底
    let mut five_hour_idx: Option<usize> = None;
    if let Some(limits) = limits {
        for (idx, item) in limits.iter().enumerate() {
            if is_five_hour_window(item) == Some(true) {
                five_hour_idx = Some(idx);
                break;
            }
        }
        if five_hour_idx.is_none() {
            // 无名兜底：取首个无法从 window 判定的条目（已知非 5h 的条目
            // 如 7 DAY 不占位，留给 weekly 兜底链）
            five_hour_idx = limits
                .iter()
                .position(|item| is_five_hour_window(item).is_none());
        }
        if let Some(idx) = five_hour_idx {
            if let Some(detail) = limits[idx].get("detail") {
                if let Some(tier) =
                    tier_from_container(super::subscription::TIER_FIVE_HOUR, detail)
                {
                    tiers.push(tier);
                }
            }
        }
    }

    // 周额度：顶层 usage{...} 优先；缺失时从 limits[] 找非 5h 项兜底
    let weekly = body
        .get("usage")
        .and_then(|u| tier_from_container(super::subscription::TIER_WEEKLY_LIMIT, u))
        .or_else(|| {
            limits.and_then(|limits| {
                limits.iter().enumerate().find_map(|(idx, item)| {
                    if Some(idx) == five_hour_idx || is_five_hour_window(item) == Some(true) {
                        return None;
                    }
                    tier_from_container(
                        super::subscription::TIER_WEEKLY_LIMIT,
                        item.get("detail")?,
                    )
                })
            })
        });
    if let Some(tier) = weekly {
        tiers.push(tier);
    }

    SubscriptionQuota {
        tool: TOOL_NAME.to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage: extra_usage_from_booster_wallet(body),
        error: None,
        queried_at: Some(now_millis()),
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── 入口 ──────────────────────────────────────────────────

const LOGIN_HINT: &str =
    "Kimi Code OAuth token expired. Run /login in the Kimi Code CLI to re-authenticate.";

fn expired_quota() -> SubscriptionQuota {
    SubscriptionQuota::error(TOOL_NAME, CredentialStatus::Expired, LOGIN_HINT.to_string())
}

fn unsupported_endpoint_quota() -> SubscriptionQuota {
    SubscriptionQuota {
        tool: TOOL_NAME.to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: false,
        tiers: vec![],
        extra_usage: None,
        error: Some(
            "Current endpoint does not support quota query (/usages not found)".to_string(),
        ),
        queried_at: Some(now_millis()),
    }
}

/// 用指定 token 查询一次并把结果映射进 SubscriptionQuota 三态约定
async fn query_with_token(base_url: &str, token: &str) -> Result<SubscriptionQuota, String> {
    match query_kimi_usages_once(base_url, token).await {
        Ok(body) => Ok(kimi_code_quota_from_body(&body)),
        Err(KimiQueryError::Transient(msg)) => Err(msg),
        Err(KimiQueryError::Unauthorized) => Ok(expired_quota()),
        Err(KimiQueryError::UnsupportedEndpoint) => Ok(unsupported_endpoint_quota()),
        Err(KimiQueryError::ParseFailure(msg)) => Ok(SubscriptionQuota::error(
            TOOL_NAME,
            CredentialStatus::ParseError,
            msg,
        )),
    }
}

/// Kimi Code 订阅额度查询入口（`get_subscription_quota("kimi-code")` 分支）
pub(crate) async fn get_kimi_code_subscription_quota() -> Result<SubscriptionQuota, String> {
    let config_text = read_kimi_code_config_text().unwrap_or_default();
    let endpoint = resolve_kimi_endpoint(&config_text);

    let (token, status, message) = read_kimi_credentials(&endpoint);
    match status {
        CredentialStatus::NotFound => {
            // 隐藏兼容（R6，未实测验证）：凭据文件不存在且当前供应商在
            // api.kimi.com 上配了静态 API Key（sk-kimi-）时，尝试作 Bearer
            // 直接调 /usages；失败一律按 not_found 处理，不文档化。
            if endpoint.base_url.contains("api.kimi.com") {
                if let Some(api_key) = extract_kimi_code_api_key(&config_text) {
                    if let Ok(quota) = query_with_token(&endpoint.base_url, &api_key).await {
                        if quota.success {
                            return Ok(quota);
                        }
                    }
                }
            }
            Ok(SubscriptionQuota::not_found(TOOL_NAME))
        }
        CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
            TOOL_NAME,
            CredentialStatus::ParseError,
            message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
        )),
        CredentialStatus::Valid | CredentialStatus::Expired => {
            let Some(token) = token else {
                return Ok(SubscriptionQuota::not_found(TOOL_NAME));
            };
            match query_kimi_usages_once(&endpoint.base_url, &token).await {
                Ok(body) => Ok(kimi_code_quota_from_body(&body)),
                Err(KimiQueryError::Transient(msg)) => Err(msg),
                Err(KimiQueryError::UnsupportedEndpoint) => Ok(unsupported_endpoint_quota()),
                Err(KimiQueryError::ParseFailure(msg)) => Ok(SubscriptionQuota::error(
                    TOOL_NAME,
                    CredentialStatus::ParseError,
                    msg,
                )),
                Err(KimiQueryError::Unauthorized) => {
                    // 401/403：重读磁盘一次（官方 CLI 可能刚刷新过 token），
                    // token 变化则重试一次；仍失败 → expired，引导 /login。
                    // 绝不写回凭据文件（R2：refresh_token 轮换并发写竞争，
                    // CLI 是凭据的唯一 owner）。
                    let (retry_token, _, _) = read_kimi_credentials(&endpoint);
                    if let Some(retry_token) = retry_token {
                        if retry_token != token {
                            return query_with_token(&endpoint.base_url, &retry_token).await;
                        }
                    }
                    Ok(expired_quota())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── 凭据槽位推导（字节级对齐官方 resolveKimiCodeOAuthKey）──────────

    #[test]
    fn credential_key_default_endpoint_is_kimi_code() {
        let endpoint = KimiEndpoint {
            base_url: KIMI_CODE_DEFAULT_BASE_URL.to_string(),
            oauth_host: KIMI_CODE_OAUTH_HOST.to_string(),
        };
        assert_eq!(credential_key(&endpoint), "kimi-code");
    }

    #[test]
    fn credential_key_custom_endpoint_uses_ordered_sha256() {
        let endpoint = KimiEndpoint {
            base_url: "https://gateway.example.com/coding/v1".to_string(),
            oauth_host: KIMI_CODE_OAUTH_HOST.to_string(),
        };
        let key = credential_key(&endpoint);
        assert!(key.starts_with("oauth/kimi-code-env-"), "unexpected key {key}");
        assert_eq!(key.len(), "oauth/kimi-code-env-".len() + 16);

        // sha256 输入字节级对齐：{"oauthHost":...,"baseUrl":...} 属性顺序固定
        let input = format!(
            "{{\"oauthHost\":\"{}\",\"baseUrl\":\"{}\"}}",
            KIMI_CODE_OAUTH_HOST, endpoint.base_url
        );
        let digest = Sha256::digest(input.as_bytes());
        let expected: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        assert!(key.ends_with(&expected), "key {key} != sha256[:16] {expected}");
    }

    #[test]
    fn endpoint_resolution_priority() {
        // config.toml managed block
        let config = r#"
default_model = "managed:kimi-code/kimi-for-coding"
[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1/"
"#;
        let endpoint = resolve_kimi_endpoint(config);
        assert_eq!(endpoint.base_url, KIMI_CODE_DEFAULT_BASE_URL); // 尾斜杠归一
        assert_eq!(endpoint.oauth_host, KIMI_CODE_OAUTH_HOST);

        // 空配置 → 内置默认
        let endpoint = resolve_kimi_endpoint("");
        assert_eq!(endpoint.base_url, KIMI_CODE_DEFAULT_BASE_URL);
    }

    // ── 额度响应解析（录制响应体表驱动）─────────────────────────

    fn quota_tier<'a>(quota: &'a SubscriptionQuota, name: &str) -> &'a QuotaTier {
        quota
            .tiers
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("tier {name} missing in {:?}", quota.tiers))
    }

    #[test]
    fn parses_full_response_with_string_numbers() {
        // proto3 风格：数值字符串化、false 字段省略
        let body = json!({
            "limits": [{
                "window": {"duration": "300", "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {
                    "limit": "1000000",
                    "used": "420000",
                    "remaining": "580000",
                    "resetTime": "2026-08-05T10:00:00Z"
                }
            }],
            "usage": {
                "limit": "5000000",
                "remaining": "2500000",
                "resetAt": "2026-08-12T00:00:00Z"
            },
            "boosterWallet": {
                "status": "STATUS_ACTIVE",
                "balance": {"amountLeft": "12345678"},
                "monthlyUsed": {"priceInCents": "1599"},
                "monthlyChargeLimit": {"priceInCents": "10000"},
                "monthlyChargeLimitEnabled": true
            }
        });
        let quota = kimi_code_quota_from_body(&body);
        assert!(quota.success);
        assert_eq!(quota.tiers.len(), 2);

        let five = quota_tier(&quota, "five_hour");
        assert!((five.utilization - 42.0).abs() < 1e-9);
        assert_eq!(five.resets_at.as_deref(), Some("2026-08-05T10:00:00+00:00"));

        // 周额度：used 缺失 → limit - remaining 兜底 → 50%
        let weekly = quota_tier(&quota, "weekly_limit");
        assert!((weekly.utilization - 50.0).abs() < 1e-9);

        let extra = quota.extra_usage.expect("boosterWallet 应映射 extraUsage");
        assert!(extra.is_enabled);
        assert_eq!(extra.used_credits, Some(15.99)); // priceInCents ÷100
        assert_eq!(extra.monthly_limit, Some(100.0));
        assert!((extra.utilization.unwrap() - 15.99).abs() < 1e-6);
        assert_eq!(extra.currency.as_deref(), Some("CNY"));
    }

    #[test]
    fn five_hour_window_variants() {
        // 5 + TIME_UNIT_HOUR
        let body = json!({
            "limits": [{
                "window": {"duration": 5, "timeUnit": "TIME_UNIT_HOUR"},
                "detail": {"limit": 100, "used": 87}
            }],
            "usage": {"limit": 1000, "used": 420}
        });
        let quota = kimi_code_quota_from_body(&body);
        assert!((quota_tier(&quota, "five_hour").utilization - 87.0).abs() < 1e-9);
        assert!((quota_tier(&quota, "weekly_limit").utilization - 42.0).abs() < 1e-9);

        // 18000 秒
        let body = json!({
            "limits": [{
                "window": {"duration": 18000, "timeUnit": "TIME_UNIT_SECOND"},
                "detail": {"max": 100, "consumed": 30}
            }]
        });
        let quota = kimi_code_quota_from_body(&body);
        assert!((quota_tier(&quota, "five_hour").utilization - 30.0).abs() < 1e-9);

        // 非 5h 窗口（7 天）应归 weekly 兜底而非 five_hour
        let body = json!({
            "limits": [{
                "window": {"duration": 7, "timeUnit": "TIME_UNIT_DAY"},
                "detail": {"limit": 200, "left": 100}
            }]
        });
        let quota = kimi_code_quota_from_body(&body);
        assert!(quota.tiers.iter().all(|t| t.name != "five_hour"));
        assert!((quota_tier(&quota, "weekly_limit").utilization - 50.0).abs() < 1e-9);
    }

    #[test]
    fn unnamed_first_limit_falls_back_to_five_hour() {
        let body = json!({
            "limits": [
                {"detail": {"total": 100, "remaining": 25}},
                {"window": {"duration": 7, "timeUnit": "TIME_UNIT_DAY"},
                 "detail": {"limit": 1000, "used": 100}}
            ]
        });
        let quota = kimi_code_quota_from_body(&body);
        assert!((quota_tier(&quota, "five_hour").utilization - 75.0).abs() < 1e-9);
        // 第二条非 5h 项兜底为 weekly
        assert!((quota_tier(&quota, "weekly_limit").utilization - 10.0).abs() < 1e-9);
    }

    #[test]
    fn multi_alias_detail_fields() {
        // limit|max|total, used|consumed, remaining|left, resetAt|reset_at|resetTime|reset_time
        let body = json!({
            "limits": [{
                "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {"max": 1000, "consumed": 500, "reset_at": 1786000000}
            }],
            "usage": {"total": 8000, "left": 4000, "reset_time": "1786000000000"}
        });
        let quota = kimi_code_quota_from_body(&body);
        assert!((quota_tier(&quota, "five_hour").utilization - 50.0).abs() < 1e-9);
        assert!((quota_tier(&quota, "weekly_limit").utilization - 50.0).abs() < 1e-9);
        // epoch 秒/毫秒都归一为 ISO
        assert!(quota_tier(&quota, "five_hour").resets_at.is_some());
        assert!(quota_tier(&quota, "weekly_limit").resets_at.is_some());
    }

    #[test]
    fn booster_wallet_inactive_or_limit_omitted() {
        // status 非 ACTIVE/ENABLED → 按 0 显示
        let body = json!({
            "boosterWallet": {
                "status": "STATUS_DISABLED",
                "monthlyUsed": {"priceInCents": 5000}
            }
        });
        let quota = kimi_code_quota_from_body(&body);
        let extra = quota.extra_usage.expect("inactive 钱包仍出 ExtraUsage");
        assert!(!extra.is_enabled);
        assert_eq!(extra.used_credits, Some(0.0));

        // monthlyChargeLimitEnabled 省略即无上限
        let body = json!({
            "boosterWallet": {
                "status": "STATUS_ENABLED",
                "monthlyUsed": {"priceInCents": 2599}
            }
        });
        let quota = kimi_code_quota_from_body(&body);
        let extra = quota.extra_usage.unwrap();
        assert!(extra.is_enabled);
        assert_eq!(extra.used_credits, Some(25.99));
        assert_eq!(extra.monthly_limit, None);
        assert_eq!(extra.utilization, None);
    }

    #[test]
    fn missing_fields_degrade_instead_of_error() {
        // 完全空的响应：200 应答但无可识别字段 → success 但无 tier（漂移容错）
        let quota = kimi_code_quota_from_body(&json!({}));
        assert!(quota.success);
        assert!(quota.tiers.is_empty());
        assert!(quota.extra_usage.is_none());

        // detail 全缺 → 该 tier 被跳过而非 panic
        let quota = kimi_code_quota_from_body(&json!({"limits": [{"detail": {}}]}));
        assert!(quota.tiers.is_empty());
    }

    // ── 凭据过期判定 ─────────────────────────────────────────

    #[test]
    fn expires_at_formats() {
        // 0 / 缺失 / 负数 → 未知，不判过期
        assert!(!kimi_token_expired(None));
        assert!(!kimi_token_expired(Some(&json!(0))));
        assert!(!kimi_token_expired(Some(&json!(-5))));
        // 过去的秒/毫秒/小数/ISO → 过期
        assert!(kimi_token_expired(Some(&json!(1600000000))));
        assert!(kimi_token_expired(Some(&json!(1600000000000i64))));
        assert!(kimi_token_expired(Some(&json!(1600000000.5))));
        assert!(kimi_token_expired(Some(&json!("2020-09-13T12:26:40Z"))));
        // 未来 → 未过期（秒与毫秒两种量纲）
        assert!(!kimi_token_expired(Some(&json!(4000000000i64))));
        assert!(!kimi_token_expired(Some(&json!(4000000000000i64))));
    }
}
