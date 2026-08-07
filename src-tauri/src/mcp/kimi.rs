//! Kimi Code MCP 同步和导入模块
//!
//! 管理 `~/.kimi-code/mcp.json`（CLI 与 VSCode 扩展共享）：
//! - 文件格式：`{ "mcpServers": { "name": { ... } } }`，读-改-写保留未知顶层键
//! - 三种传输形态（cc-switch 内部 spec → mcp.json）：
//!   - `command` 存在 → stdio：`{command, args?, env?}`
//!   - `url` 存在且非 SSE → HTTP：`{url, headers?}`
//!   - `url` 存在且 `type == "sse"` → 旧式 SSE：`{url, transport: "sse"}`
//! - 只按单 server 增量写（与 claude/codex 模块同策略），不做全量重建

use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::app_config::{McpApps, McpServer, MultiAppConfig};
use crate::error::AppError;

use super::validation::validate_server_spec;

fn should_sync_kimi_code_mcp() -> bool {
    // Kimi Code 未安装/未初始化时：配置目录不存在。
    // 按用户偏好：目录缺失时跳过写入/删除，不创建任何文件或目录。
    crate::kimi_code_config::get_kimi_code_config_dir().exists()
}

fn get_kimi_code_mcp_path() -> std::path::PathBuf {
    crate::kimi_code_config::get_kimi_code_mcp_path()
}

/// 读取整个 mcp.json（保留未知顶层键）；文件不存在时返回空骨架
fn read_mcp_json_doc() -> Result<Value, AppError> {
    let path = get_kimi_code_mcp_path();
    if !path.exists() {
        return Ok(json!({ "mcpServers": {} }));
    }

    let text = std::fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
    let mut doc: Value = serde_json::from_str(&text).map_err(|e| AppError::json(&path, e))?;

    // 顶层必须是对象；mcpServers 缺失时补空表（已有但非对象时视为损坏，报错不覆盖）
    if !doc.is_object() {
        return Err(AppError::McpValidation(
            "Kimi Code mcp.json 顶层必须是 JSON 对象".into(),
        ));
    }
    match doc.get("mcpServers") {
        None => {
            doc.as_object_mut()
                .expect("checked object")
                .insert("mcpServers".into(), json!({}));
        }
        Some(servers) if servers.is_object() => {}
        Some(_) => {
            return Err(AppError::McpValidation(
                "Kimi Code mcp.json 的 mcpServers 必须是 JSON 对象".into(),
            ));
        }
    }

    Ok(doc)
}

fn write_mcp_json_doc(doc: &Value) -> Result<(), AppError> {
    crate::config::write_json_file(&get_kimi_code_mcp_path(), doc)
}

/// 内部 MCP spec → mcp.json 条目（三种传输形态映射）
fn spec_to_kimi_mcp_entry(spec: &Value) -> Result<Value, AppError> {
    let obj = spec
        .as_object()
        .ok_or_else(|| AppError::McpValidation("MCP 服务器连接定义必须为 JSON 对象".into()))?;

    let command = obj
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let url = obj
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(command) = command {
        // stdio：{command, args?, env?}
        let mut entry = Map::new();
        entry.insert("command".into(), json!(command));
        if let Some(args) = obj.get("args").and_then(|v| v.as_array()) {
            let args: Vec<Value> = args
                .iter()
                .filter_map(|a| a.as_str().map(|s| json!(s)))
                .collect();
            if !args.is_empty() {
                entry.insert("args".into(), Value::Array(args));
            }
        }
        if let Some(env) = obj.get("env").and_then(|v| v.as_object()) {
            let env: Map<String, Value> = env
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), json!(s))))
                .collect();
            if !env.is_empty() {
                entry.insert("env".into(), Value::Object(env));
            }
        }
        return Ok(Value::Object(entry));
    }

    if let Some(url) = url {
        let is_sse = obj.get("type").and_then(|v| v.as_str()) == Some("sse")
            || obj.get("transport").and_then(|v| v.as_str()) == Some("sse");
        let mut entry = Map::new();
        entry.insert("url".into(), json!(url));
        if is_sse {
            // 旧式 SSE：{url, transport: "sse"}
            entry.insert("transport".into(), json!("sse"));
        } else if let Some(headers) = obj.get("headers").and_then(|v| v.as_object()) {
            // HTTP：{url, headers?}
            let headers: Map<String, Value> = headers
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), json!(s))))
                .collect();
            if !headers.is_empty() {
                entry.insert("headers".into(), Value::Object(headers));
            }
        }
        return Ok(Value::Object(entry));
    }

    Err(AppError::McpValidation(
        "MCP 服务器缺少 command 或 url 字段，无法写入 Kimi Code mcp.json".into(),
    ))
}

/// mcp.json 条目 → 内部 MCP spec（三种形态逆映射）
fn kimi_mcp_entry_to_spec(entry: &Value) -> Option<Value> {
    let obj = entry.as_object()?;

    if let Some(command) = obj.get("command").and_then(|v| v.as_str()) {
        let mut spec = Map::new();
        spec.insert("type".into(), json!("stdio"));
        spec.insert("command".into(), json!(command));
        if let Some(args) = obj.get("args").and_then(|v| v.as_array()) {
            spec.insert("args".into(), Value::Array(args.clone()));
        }
        if let Some(env) = obj.get("env").and_then(|v| v.as_object()) {
            spec.insert("env".into(), Value::Object(env.clone()));
        }
        return Some(Value::Object(spec));
    }

    if let Some(url) = obj.get("url").and_then(|v| v.as_str()) {
        let is_sse = obj.get("transport").and_then(|v| v.as_str()) == Some("sse");
        let mut spec = Map::new();
        spec.insert("type".into(), json!(if is_sse { "sse" } else { "http" }));
        spec.insert("url".into(), json!(url));
        if !is_sse {
            if let Some(headers) = obj.get("headers").and_then(|v| v.as_object()) {
                spec.insert("headers".into(), Value::Object(headers.clone()));
            }
        }
        return Some(Value::Object(spec));
    }

    None
}

/// 将单个 MCP 服务器同步到 Kimi Code 的 mcp.json（单 server 增量写，保留其他条目与未知顶层键）
pub fn sync_single_server_to_kimi_code(
    _config: &MultiAppConfig,
    id: &str,
    server_spec: &Value,
) -> Result<(), AppError> {
    if !should_sync_kimi_code_mcp() {
        return Ok(());
    }

    let entry = spec_to_kimi_mcp_entry(server_spec)?;
    let mut doc = read_mcp_json_doc()?;
    doc.get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .expect("read_mcp_json_doc guarantees mcpServers object")
        .insert(id.to_string(), entry);

    write_mcp_json_doc(&doc)
}

/// 从 Kimi Code 的 mcp.json 中移除单个 MCP 服务器
pub fn remove_server_from_kimi_code(id: &str) -> Result<(), AppError> {
    if !should_sync_kimi_code_mcp() {
        return Ok(());
    }

    let path = get_kimi_code_mcp_path();
    if !path.exists() {
        return Ok(()); // 文件不存在，无需删除
    }

    let mut doc = read_mcp_json_doc()?;
    doc.get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .expect("read_mcp_json_doc guarantees mcpServers object")
        .remove(id);

    write_mcp_json_doc(&doc)
}

/// 从 mcp.json 导入 MCP 到统一结构（用户在 CLI 侧手写/安装的 server）。
///
/// 已存在的服务器仅启用 Kimi Code 应用，不覆盖其他字段和应用状态；
/// `mcpServers` 外的顶层键忽略。
pub fn import_from_kimi_code(config: &mut MultiAppConfig) -> Result<usize, AppError> {
    let path = get_kimi_code_mcp_path();
    if !path.exists() {
        return Ok(0);
    }

    let text = std::fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
    let doc: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::McpValidation(format!("解析 Kimi Code mcp.json 失败: {e}")))?;
    let Some(map) = doc.get("mcpServers").and_then(|v| v.as_object()) else {
        return Ok(0);
    };

    // 确保新结构存在
    let servers = config.mcp.servers.get_or_insert_with(HashMap::new);

    let mut changed = 0;
    let mut errors = Vec::new();

    for (id, entry) in map.iter() {
        let Some(spec) = kimi_mcp_entry_to_spec(entry) else {
            log::warn!("跳过无法识别的 Kimi Code MCP 条目 '{id}'");
            errors.push(format!("{id}: 无法识别的条目形态"));
            continue;
        };

        // 校验：单项失败不中止，收集错误继续处理
        if let Err(e) = validate_server_spec(&spec) {
            log::warn!("跳过无效 MCP 服务器 '{id}': {e}");
            errors.push(format!("{id}: {e}"));
            continue;
        }

        if let Some(existing) = servers.get_mut(id) {
            // 已存在：仅启用 Kimi Code 应用
            if !existing.apps.kimi_code {
                existing.apps.kimi_code = true;
                changed += 1;
                log::info!("MCP 服务器 '{id}' 已启用 Kimi Code 应用");
            }
        } else {
            // 新建服务器：默认仅启用 Kimi Code
            servers.insert(
                id.clone(),
                McpServer {
                    id: id.clone(),
                    name: id.clone(),
                    server: spec,
                    apps: McpApps {
                        claude: false,
                        codex: false,
                        gemini: false,
                        grokbuild: false,
                        opencode: false,
                        hermes: false,
                        kimi_code: true,
                    },
                    description: None,
                    homepage: None,
                    docs: None,
                    tags: Vec::new(),
                },
            );
            changed += 1;
            log::info!("导入新 MCP 服务器 '{id}'");
        }
    }

    if !errors.is_empty() {
        log::warn!("导入完成，但有 {} 项失败: {:?}", errors.len(), errors);
    }

    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_spec_maps_to_command_args_env() {
        let entry = spec_to_kimi_mcp_entry(&json!({
            "type": "stdio",
            "command": "npx",
            "args": ["-y", "some-mcp"],
            "env": { "TOKEN": "x" }
        }))
        .expect("stdio entry");

        assert_eq!(
            entry,
            json!({
                "command": "npx",
                "args": ["-y", "some-mcp"],
                "env": { "TOKEN": "x" }
            })
        );
    }

    #[test]
    fn http_spec_maps_to_url_headers() {
        let entry = spec_to_kimi_mcp_entry(&json!({
            "type": "http",
            "url": "https://mcp.example.com",
            "headers": { "Authorization": "Bearer t" }
        }))
        .expect("http entry");

        assert_eq!(
            entry,
            json!({
                "url": "https://mcp.example.com",
                "headers": { "Authorization": "Bearer t" }
            })
        );
        assert!(entry.get("transport").is_none());
    }

    #[test]
    fn sse_spec_maps_to_url_with_transport_sse() {
        let entry = spec_to_kimi_mcp_entry(&json!({
            "type": "sse",
            "url": "https://mcp.example.com/sse"
        }))
        .expect("sse entry");

        assert_eq!(
            entry,
            json!({ "url": "https://mcp.example.com/sse", "transport": "sse" })
        );
    }

    #[test]
    fn entries_roundtrip_back_to_internal_spec() {
        for (entry, expected_type) in [
            (json!({"command": "npx", "args": ["a"]}), "stdio"),
            (json!({"url": "https://x", "headers": {"k": "v"}}), "http"),
            (json!({"url": "https://x", "transport": "sse"}), "sse"),
        ] {
            let spec = kimi_mcp_entry_to_spec(&entry).expect("spec");
            assert_eq!(
                spec.get("type").and_then(|v| v.as_str()),
                Some(expected_type)
            );
            validate_server_spec(&spec).expect("imported spec must validate");
        }
    }
}
