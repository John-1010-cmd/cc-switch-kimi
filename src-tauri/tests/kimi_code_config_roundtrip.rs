//! Kimi Code 供应商切换 ↔ live config.toml 的集成测试（roadmap §2.3 `kimi_code_config_roundtrip`）。
//!
//! 覆盖场景：构造 kimi-code 供应商（settings_config = {"config": "<TOML 文本>"}）
//! → `ProviderService::switch` 全链路写 live → 读回 `$KIMI_CODE_HOME/config.toml`
//! 断言 [providers]/[models]/default_model 正确 → 再切换另一家供应商，断言内容被
//! 完整覆写、无残留键。
//!
//! 取舍说明：switch 全链路（DB + settings 单例 + live 写入）在集成测试中可以搭建
//! ——`support::create_test_state_with_config` 把 MultiAppConfig 迁移进测试 DB，
//! `support::ensure_test_home`/`reset_test_fs` 隔离 HOME 与 settings 单例，
//! `KIMI_CODE_HOME` 指到独立 tempdir 隔离 live 文件——因此本测试直接走
//! `ProviderService::switch`（与 UI 点击"切换"同一条链路），不退化到写读层。
//! 写读层自身的单元测试（校验失败不落盘、空文本、路径解析）已在
//! `src/kimi_code_config.rs` 的 `#[cfg(test)]` 模块覆盖，这里不重复。
//!
//! 并行安全：`KIMI_CODE_HOME` 是进程级环境变量。本文件所有用例先抢
//! `support::test_mutex()` 串行执行，并用 `KimiCodeHomeGuard` 在用例结束（含 panic）
//! 后恢复原值。
//! 不同测试二进制（lib 单测中 kimi_code_config 的 env_lock 用例）是独立进程，
//! 环境变量互不可见，无跨二进制污染。

mod support;

use std::path::PathBuf;
use std::sync::MutexGuard;

use cc_switch_lib::{AppType, MultiAppConfig, Provider, ProviderService};
use serde_json::json;
use support::{create_test_state_with_config, ensure_test_home, reset_test_fs, test_mutex};

/// 设计文档 §2.2 官方托管模板（与 providers_seed.rs 的 kimi-code-official 种子同形）。
const MANAGED_TOML: &str = r#"default_model = "managed:kimi-code/kimi-for-coding"

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
oauth = { storage = "file", key = "oauth/kimi-code" }

[models."managed:kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 262144
"#;

/// 设计文档 §2.2 第三方（OpenAI 兼容）模板。
const ZHIPU_TOML: &str = r#"default_model = "zhipu/glm-4.6"

[providers.zhipu]
type = "openai"
base_url = "https://open.bigmodel.cn/api/paas/v4"
api_key = "sk-zhipu-test-key"

[models."zhipu/glm-4.6"]
provider = "zhipu"
model = "glm-4.6"
max_context_size = 204800
"#;

/// RAII 守卫：设置 `KIMI_CODE_HOME` 指向测试目录，drop（含 panic 路径）时恢复原值。
struct KimiCodeHomeGuard {
    prev: Option<std::ffi::OsString>,
}

impl KimiCodeHomeGuard {
    fn set(dir: &std::path::Path) -> Self {
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

/// 测试环境：串行锁 + 隔离 HOME + KIMI_CODE_HOME 指向独立 tempdir。
/// struct 字段按声明顺序 drop：tempdir 清理 → 恢复 KIMI_CODE_HOME → 最后释放
/// 串行锁，即全部清理由本用例锁内完成，下一个抢到锁的用例看到的是干净环境。
struct KimiCodeTestEnv {
    kimi_home: PathBuf,
    _tempdir: tempfile::TempDir,
    _home_guard: KimiCodeHomeGuard,
    _lock: MutexGuard<'static, ()>,
}

fn setup() -> KimiCodeTestEnv {
    let lock = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let tempdir = tempfile::tempdir().expect("create kimi code home tempdir");
    let kimi_home = tempdir.path().to_path_buf();
    let home_guard = KimiCodeHomeGuard::set(&kimi_home);

    KimiCodeTestEnv {
        kimi_home,
        _tempdir: tempdir,
        _home_guard: home_guard,
        _lock: lock,
    }
}

/// 构造 kimi-code 供应商的 settings_config：`{"config": "<整份 config.toml 文本>"}`。
fn kimi_provider(id: &str, name: &str, toml_text: &str) -> (String, Provider) {
    (
        id.to_string(),
        Provider::with_id(
            id.to_string(),
            name.to_string(),
            json!({ "config": toml_text }),
            None,
        ),
    )
}

/// 构造含两个 kimi-code 供应商的 MultiAppConfig，current 指向 `current`。
fn build_kimi_config(current: &str) -> MultiAppConfig {
    let mut config = MultiAppConfig::default();
    let manager = config
        .get_manager_mut(&AppType::KimiCode)
        .expect("kimi-code manager must exist in default config");
    manager.current = current.to_string();
    for (id, provider) in [
        kimi_provider("kimi-managed", "Kimi Managed", MANAGED_TOML),
        kimi_provider("zhipu", "Zhipu GLM", ZHIPU_TOML),
    ] {
        manager.providers.insert(id, provider);
    }
    config
}

fn live_config_path(env: &KimiCodeTestEnv) -> PathBuf {
    env.kimi_home.join("config.toml")
}

fn read_live(env: &KimiCodeTestEnv) -> String {
    std::fs::read_to_string(live_config_path(env)).expect("read live config.toml")
}

#[test]
fn switch_writes_managed_template_and_preserves_credentials() {
    let env = setup();

    // 红线（设计 §2.2）：切换写 config.toml 不得触碰 credentials/ 目录。
    let credentials_dir = env.kimi_home.join("credentials");
    std::fs::create_dir_all(&credentials_dir).expect("create credentials dir");
    let sentinel = credentials_dir.join("kimi-code.json");
    std::fs::write(&sentinel, r#"{"refresh_token":"user-owned"}"#).expect("seed credential");

    let config = build_kimi_config("kimi-managed");
    let state = create_test_state_with_config(&config).expect("create test state");

    ProviderService::switch(&state, AppType::KimiCode, "kimi-managed")
        .expect("switch to managed provider");

    // 验收（设计 §6 #2）：live 字节内容与 provider 的 settings_config.config 一致。
    let live = read_live(&env);
    assert_eq!(live, MANAGED_TOML, "live config.toml 必须与写入文本逐字节一致");

    // 结构断言：[providers] / [models] / default_model。
    let doc: toml::Value = toml::from_str(&live).expect("live config is valid TOML");
    assert_eq!(
        doc.get("default_model").and_then(|v| v.as_str()),
        Some("managed:kimi-code/kimi-for-coding")
    );
    let managed = doc
        .get("providers")
        .and_then(|p| p.get("managed:kimi-code"))
        .expect("managed:kimi-code provider block");
    assert_eq!(
        managed.get("type").and_then(|v| v.as_str()),
        Some("kimi")
    );
    assert_eq!(
        managed.get("base_url").and_then(|v| v.as_str()),
        Some("https://api.kimi.com/coding/v1")
    );
    assert!(
        managed.get("api_key").is_none(),
        "托管 block 走 OAuth，不得出现 api_key"
    );
    let model = doc
        .get("models")
        .and_then(|m| m.get("managed:kimi-code/kimi-for-coding"))
        .expect("managed model block");
    assert_eq!(
        model.get("max_context_size").and_then(|v| v.as_integer()),
        Some(262144)
    );

    // credentials/ 原样未动（验收 §6 #5）。
    let sentinel_after = std::fs::read_to_string(&sentinel).expect("credential untouched");
    assert_eq!(sentinel_after, r#"{"refresh_token":"user-owned"}"#);
}

#[test]
fn switch_to_another_provider_fully_overwrites_live_without_residual_keys() {
    let env = setup();

    let config = build_kimi_config("kimi-managed");
    let state = create_test_state_with_config(&config).expect("create test state");

    // 先切到托管供应商，让 live 里存在第一份内容。
    ProviderService::switch(&state, AppType::KimiCode, "kimi-managed")
        .expect("switch to managed provider");
    assert_eq!(read_live(&env), MANAGED_TOML);

    // 再切到智谱：live 必须被完整覆写，无 managed 模板残留键。
    // 这条路径同时覆盖切走前的 backfill（读 live 回填旧供应商）。
    ProviderService::switch(&state, AppType::KimiCode, "zhipu")
        .expect("switch to zhipu provider");

    let live = read_live(&env);
    assert_eq!(live, ZHIPU_TOML, "切换后 live 必须与新供应商文本逐字节一致");
    assert!(
        !live.contains("managed:kimi-code"),
        "旧供应商 provider block 残留:\n{live}"
    );
    assert!(
        !live.contains("kimi-for-coding"),
        "旧供应商模型块残留:\n{live}"
    );

    let doc: toml::Value = toml::from_str(&live).expect("live config is valid TOML");
    assert_eq!(
        doc.get("default_model").and_then(|v| v.as_str()),
        Some("zhipu/glm-4.6")
    );
    let providers: Vec<&str> = doc
        .get("providers")
        .and_then(|p| p.as_table())
        .expect("providers table")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(providers, vec!["zhipu"], "[providers] 不得残留旧供应商键");
    let models: Vec<&str> = doc
        .get("models")
        .and_then(|m| m.as_table())
        .expect("models table")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(models, vec!["zhipu/glm-4.6"], "[models] 不得残留旧模型键");

    // DB 当前供应商已更新。
    let current_id = state
        .db
        .get_current_provider(AppType::KimiCode.as_str())
        .expect("read current provider");
    assert_eq!(current_id.as_deref(), Some("zhipu"));

    // backfill 往返：被切走的托管供应商，DB 中存储的 config 文本保持原样。
    let providers = state
        .db
        .get_all_providers(AppType::KimiCode.as_str())
        .expect("read providers");
    let stored = providers
        .get("kimi-managed")
        .and_then(|p| p.settings_config.get("config"))
        .and_then(|v| v.as_str())
        .expect("managed provider stored config");
    assert_eq!(
        stored, MANAGED_TOML,
        "backfill 后旧供应商的存储配置不得被改写"
    );
}

#[test]
fn switch_to_invalid_toml_provider_fails_and_keeps_live_untouched() {
    let env = setup();

    let mut config = build_kimi_config("kimi-managed");
    {
        let manager = config
            .get_manager_mut(&AppType::KimiCode)
            .expect("kimi-code manager");
        let (id, provider) = kimi_provider("broken", "Broken TOML", "default_model = [broken\n");
        manager.providers.insert(id, provider);
    }
    let state = create_test_state_with_config(&config).expect("create test state");

    // 先落一份合法 live。
    ProviderService::switch(&state, AppType::KimiCode, "kimi-managed")
        .expect("switch to managed provider");
    assert_eq!(read_live(&env), MANAGED_TOML);

    // 切到坏 TOML 供应商：必须报错，且 live 保持旧内容（校验失败不落盘，验收 §6 #3）。
    let err = ProviderService::switch(&state, AppType::KimiCode, "broken")
        .expect_err("invalid TOML must be rejected");
    let err_text = err.to_string();
    assert!(
        err_text.contains("TOML") || err_text.contains("toml"),
        "错误应指向 TOML 校验失败: {err_text}"
    );
    assert_eq!(
        read_live(&env),
        MANAGED_TOML,
        "切换失败时 live 文件必须保持旧内容"
    );
}

#[test]
fn official_seed_switch_writes_managed_template() {
    let env = setup();

    // 空 DB + 官方种子补播，覆盖验收 §6 #10：
    // 全新 DB 上 kimi-code-official 出现，切换后生成 §2.2 托管模板内容。
    let config = MultiAppConfig::default();
    let state = create_test_state_with_config(&config).expect("create test state");
    state
        .db
        .init_default_official_providers()
        .expect("seed official providers");

    let providers = state
        .db
        .get_all_providers(AppType::KimiCode.as_str())
        .expect("read kimi-code providers");
    assert!(
        providers.contains_key("kimi-code-official"),
        "kimi-code-official 官方种子必须存在"
    );

    ProviderService::switch(&state, AppType::KimiCode, "kimi-code-official")
        .expect("switch to official seed");

    let live = read_live(&env);
    let doc: toml::Value = toml::from_str(&live).expect("live config is valid TOML");
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
        Some("https://api.kimi.com/coding/v1")
    );
    assert!(managed.get("api_key").is_none(), "托管 block 不得内嵌 api_key");
}
