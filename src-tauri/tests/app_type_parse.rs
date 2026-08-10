use std::str::FromStr;

use cc_switch_lib::AppType;

#[test]
fn parse_known_apps_case_insensitive_and_trim() {
    assert!(matches!(AppType::from_str("claude"), Ok(AppType::Claude)));
    assert!(matches!(AppType::from_str("codex"), Ok(AppType::Codex)));
    assert!(matches!(
        AppType::from_str(" ClAuDe \n"),
        Ok(AppType::Claude)
    ));
    assert!(matches!(AppType::from_str("\tcoDeX\t"), Ok(AppType::Codex)));
    assert!(matches!(
        AppType::from_str("kimi-code"),
        Ok(AppType::KimiCode)
    ));
    assert!(matches!(
        AppType::from_str("kimi_code"),
        Ok(AppType::KimiCode)
    ));
    assert!(matches!(
        AppType::from_str("kimicode"),
        Ok(AppType::KimiCode)
    ));
}

#[test]
fn parse_unknown_app_returns_localized_error_message() {
    let err = AppType::from_str("unknown").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("可选值") || msg.contains("Allowed"));
    assert!(msg.contains("unknown"));
}

#[test]
fn parse_removed_claude_desktop_aliases_rejected() {
    // claudeDesktop app 已物理裁剪：所有历史别名一律按不支持处理
    for id in ["claude-desktop", "claude_desktop", "claudeDesktop", "claudedesktop"] {
        assert!(
            AppType::from_str(id).is_err(),
            "{id} should no longer parse after claudeDesktop removal"
        );
    }
}
