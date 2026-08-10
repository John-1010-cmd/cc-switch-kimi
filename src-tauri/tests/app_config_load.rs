use std::fs;
use std::path::PathBuf;

use cc_switch_lib::{AppError, MultiAppConfig};

mod support;
use support::{ensure_test_home, reset_test_fs, test_mutex};

fn cfg_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME should be set by ensure_test_home");
    PathBuf::from(home).join(".cc-switch").join("config.json")
}

#[test]
fn load_v1_config_returns_error_and_does_not_write() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let path = cfg_path();
    fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    // 最小 v1 形状：providers + current，且不含 version/apps/mcp
    let v1_json = r#"{"providers":{},"current":""}"#;
    fs::write(&path, v1_json).expect("seed v1 json");
    let before = fs::read_to_string(&path).expect("read before");

    let err = MultiAppConfig::load().expect_err("v1 should not be auto-migrated");
    match err {
        AppError::Localized { key, .. } => assert_eq!(key, "config.unsupported_v1"),
        other => panic!("expected Localized v1 error, got {other:?}"),
    }

    // 文件不应有任何变化，且不应生成 .bak
    let after = fs::read_to_string(&path).expect("read after");
    assert_eq!(before, after, "config.json should not be modified");
    let bak = home.join(".cc-switch").join("config.json.bak");
    assert!(!bak.exists(), ".bak should not be created on load error");
}

#[test]
fn load_v1_with_extra_version_still_treated_as_v1() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let path = cfg_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    // 畸形：包含 providers + current + version，但没有 apps，应按 v1 处理
    let v1_like = r#"{"providers":{},"current":"","version":2}"#;
    std::fs::write(&path, v1_like).expect("seed v1-like json");
    let before = std::fs::read_to_string(&path).expect("read before");

    let err = MultiAppConfig::load().expect_err("v1-like should not be parsed as v2");
    match err {
        AppError::Localized { key, .. } => assert_eq!(key, "config.unsupported_v1"),
        other => panic!("expected Localized v1 error, got {other:?}"),
    }

    let after = std::fs::read_to_string(&path).expect("read after");
    assert_eq!(before, after, "config.json should not be modified");
    let bak = home.join(".cc-switch").join("config.json.bak");
    assert!(!bak.exists(), ".bak should not be created on v1-like error");
}

#[test]
fn load_invalid_json_returns_parse_error_and_does_not_write() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let path = cfg_path();
    fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    fs::write(&path, "{not json").expect("seed invalid json");
    let before = fs::read_to_string(&path).expect("read before");

    let err = MultiAppConfig::load().expect_err("invalid json should error");
    match err {
        AppError::Json { .. } => {}
        other => panic!("expected Json error, got {other:?}"),
    }

    let after = fs::read_to_string(&path).expect("read after");
    assert_eq!(before, after, "config.json should remain unchanged");
    let bak = home.join(".cc-switch").join("config.json.bak");
    assert!(!bak.exists(), ".bak should not be created on parse error");
}

#[test]
fn load_valid_v2_config_succeeds() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();
    let path = cfg_path();
    fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    // 使用默认结构序列化为 v2
    let default_cfg = MultiAppConfig::default();
    let json = serde_json::to_string_pretty(&default_cfg).expect("serialize default cfg");
    fs::write(&path, json).expect("write v2 json");

    let loaded = MultiAppConfig::load().expect("v2 should load successfully");
    assert_eq!(loaded.version, 2);
    assert!(loaded
        .get_manager(&cc_switch_lib::AppType::Claude)
        .is_some());
    assert!(loaded.get_manager(&cc_switch_lib::AppType::Codex).is_some());
}

/// 裁剪后兼容性：含六个已删除 app（gemini/grokbuild/opencode/openclaw/hermes/
/// claudeDesktop）与 skills/prompts 残留字段的老 v2 配置必须能无损加载，
/// 残留数据只读忽略、不回写删除。
#[test]
fn load_legacy_v2_config_with_removed_apps_and_modules_succeeds() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();
    let path = cfg_path();
    fs::create_dir_all(path.parent().unwrap()).expect("create cfg dir");

    // 以当前默认 v2 配置为基底（保证各 section 形状正确），再注入六个已删除
    // app 与 skills/prompts 等残留字段，模拟真实的老用户 config.json。
    let mut legacy = serde_json::to_value(MultiAppConfig::default()).expect("serialize default");
    let root = legacy.as_object_mut().expect("root object");

    // 注意：MultiAppConfig.apps 是 #[serde(flatten)]，app 条目位于顶层键。
    for app in [
        "gemini",
        "grokbuild",
        "opencode",
        "openclaw",
        "hermes",
        "claude-desktop",
    ] {
        root.insert(
            app.to_string(),
            serde_json::json!({
                "providers": { "old-1": { "id": "old-1", "name": "Old", "settingsConfig": {}, "createdAt": 1 } },
                "current": "old-1"
            }),
        );
    }

    if let Some(mcp) = root.get_mut("mcp").and_then(|v| v.as_object_mut()) {
        for app in ["gemini", "grokbuild", "opencode", "hermes", "claudeDesktop"] {
            mcp.insert(app.to_string(), serde_json::json!({}));
        }
    }

    root.insert(
        "skills".to_string(),
        serde_json::json!({
            "skills": { "pdf": { "installed": true, "installedAt": "2023-11-14T22:13:20Z" } },
            "repos": [ { "owner": "anthropics", "name": "skills", "branch": "main", "enabled": true } ]
        }),
    );

    if let Some(prompts) = root.get_mut("prompts").and_then(|v| v.as_object_mut()) {
        prompts.insert("gemini".to_string(), serde_json::json!({ "prompts": {} }));
        prompts.insert(
            "claude".to_string(),
            serde_json::json!({ "prompts": { "p1": { "id": "p1", "name": "legacy", "content": "hi", "enabled": true } } }),
        );
    }

    if let Some(snippets) = root
        .get_mut("commonConfigSnippets")
        .and_then(|v| v.as_object_mut())
    {
        snippets.insert("gemini".to_string(), serde_json::json!({ "common": "{}" }));
    }

    let legacy_json = serde_json::to_string_pretty(&legacy).expect("serialize legacy");
    fs::write(&path, legacy_json).expect("write legacy v2 json");

    let loaded = MultiAppConfig::load().expect("legacy v2 config should load");
    assert_eq!(loaded.version, 2);
    // 三个保留 app 的管理器可用
    for app in [
        cc_switch_lib::AppType::Claude,
        cc_switch_lib::AppType::Codex,
        cc_switch_lib::AppType::KimiCode,
    ] {
        assert!(loaded.get_manager(&app).is_some(), "manager for {app:?}");
    }
    // 已删除 app 的枚举解析被拒绝
    for removed in ["gemini", "grokbuild", "opencode", "openclaw", "hermes", "claude-desktop"] {
        assert!(
            removed.parse::<cc_switch_lib::AppType>().is_err(),
            "{removed} should no longer parse"
        );
    }
}
