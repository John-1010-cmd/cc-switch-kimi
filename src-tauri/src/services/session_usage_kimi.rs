//! Kimi Code 会话日志使用追踪
//!
//! 从 `$KIMI_CODE_HOME/sessions/` 下的 wire.jsonl 会话文件中提取精确 token
//! 使用数据与流式计时字段，复用"会话日志 → proxy_request_logs"的既有管线
//! （与 claude/codex session 同步同构）。Kimi Code 不走本地代理，这是它唯一
//! 的本地用量数据源。
//!
//! ## 数据流
//! ```text
//! sessions/<workDirKey>/<sessionId>/agents/<agentId>/wire.jsonl
//!   → 增量解析（session_log_sync 游标）
//!   → INSERT OR IGNORE proxy_request_logs (app_type="kimi-code", data_source="kimi_session")
//! ```
//!
//! ## 解析的记录类型
//! - `context.append_loop_event` 且 `event.type=="step.end"` → 主用量行：
//!   usage 四分量 + 计时字段（TTFT / 流式时长 / 请求构建时长）
//! - `usage.record` 且 `usageScope=="turn"` → 权威模型名来源（无论是否入库
//!   都先同步 lastModel）。其 usage 行**仅在 legacy 文件入库**：当前代文件
//!   （实测 protocol_version 已见 "1.5"）每个 step.end 都配对一条 usage
//!   完全相同的 usage.record——文件任一位置存在 step.end 时，本文件所有
//!   usage.record 的 usage 行一律抑制，否则每步入两行精确 2 倍双算；
//!   只有全文件无 step.end 的旧版本文件（step.end 缺 usage），usage.record
//!   才作 fallback 用量行。
//! - `llm.request` → 更新 per-file `{lastModel, lastModelAlias}`，供
//!   `__secondary__`（或空 model）回填真实模型名
//! - 其余 type 一律跳过计数；首行 `metadata.protocol_version` 与
//!   [`EXPECTED_PROTOCOL_VERSION`] 不一致仅 log warn 继续解析
//!
//! sessions/ 目录是官方红线：全程只读打开，不写、不删、不建锁文件。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::kimi_code_config::get_kimi_code_config_dir;
use crate::proxy::usage::calculator::{CostCalculator, ModelPricing};
use crate::proxy::usage::parser::TokenUsage;
use crate::services::session_usage::{
    get_sync_state, metadata_modified_nanos, update_sync_state, SessionSyncResult,
};
use crate::services::sql_helpers::INPUT_TOKEN_SEMANTICS_TOTAL;
use crate::services::usage_stats::{find_model_pricing, should_skip_session_insert, DedupKey};
use chrono::DateTime;
use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::SystemTime;

const KIMI_SESSION_REQUEST_ID_PREFIX: &str = "kimi_code_session";
const KIMI_APP_TYPE: &str = "kimi-code";
const KIMI_PROVIDER_ID: &str = "_kimi_code_session";
const KIMI_DATA_SOURCE: &str = "kimi_session";
/// 子代理 patched secondary model 时 usage.record.model 的占位符
const SECONDARY_MODEL_PLACEHOLDER: &str = "__secondary__";
/// 已知的 wire.jsonl 协议版本："1.4" 为 npm 0.31.1 源码佐证，真机实测当前代
/// 已是 "1.5"。不一致仅 log warn 继续尽力解析（warn-only 策略不变），
/// 版本升级时的格式漂移由解析层的多别名/全 Option 容错吸收。
const EXPECTED_PROTOCOL_VERSION: &str = "1.5";
/// 根级 wire.jsonl 兜底布局（无 agents/ 目录的旧版本）视为 main agent
const ROOT_WIRE_FALLBACK_AGENT: &str = "main";

// ── 实时监控内存态（设计文档 02 §1.2/§2.3 实时层）──────────────────
//
// tail 层与 60s 全扫在事件**实际入库**时喂入本状态；托盘标题与前端实时卡片
// 经 `get_kimi_realtime_metrics` 直接读取，不查 DB。状态是纯派生态，
// 进程重启即清零，由下一轮同步自然重建，无需持久化。

/// TPS 中位数窗口大小（官方 kimi-code-monitor 的 SPEED_SAMPLE_WINDOW）
const TPS_SAMPLE_WINDOW: usize = 5;
/// 有效 TPS 样本的最小流式时长：低于该值的样本受代理/本地缓冲影响虚高，丢弃
/// （不清空历史窗口，设计文档 02 §1.2）
const MIN_SPEED_DURATION_MS: i64 = 100;
/// 窗口指标（缓存命中率等）的统计窗口：最近 5 分钟
const RECENT_WINDOW_MS: i64 = 5 * 60 * 1000;
/// wire 文件 mtime 距今小于该值视为"活跃"，纳入 tail 层 1s 轮询
const ACTIVE_FILE_WINDOW: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Debug, Default)]
struct KimiRealtimeState {
    /// 最近 N 个有效 TPS 样本（无效样本只丢弃、不重置窗口）
    tps_samples: VecDeque<f64>,
    /// 最近 5 分钟的入库事件（event_ts_ms, usage），供窗口指标聚合
    recent_events: VecDeque<(i64, KimiTokenUsage)>,
    /// 活跃 wire 文件集：全扫重建、tail 按 mtime 新鲜度就地维护
    active_files: HashSet<PathBuf>,
    /// 最后一次入库事件的记录时间（epoch ms）
    last_event_at: Option<i64>,
}

impl KimiRealtimeState {
    /// 事件入库成功时喂入：TPS 样本 + 5min 窗口累计 + 最后事件时间。
    /// `now_ms` 显式传入以便单测控制窗口逐出行为。
    fn record_event(&mut self, event: &ParsedKimiUsage, now_ms: i64) {
        let event_ts = event.timestamp_ms.unwrap_or(now_ms);
        self.recent_events.push_back((event_ts, event.usage));
        while let Some((ts, _)) = self.recent_events.front() {
            if now_ms - ts > RECENT_WINDOW_MS {
                self.recent_events.pop_front();
            } else {
                break;
            }
        }

        // 单步 TPS = output / (llmStreamDurationMs / 1000)；无效样本丢弃
        if let Some(duration_ms) = event.duration_ms {
            if duration_ms >= MIN_SPEED_DURATION_MS && event.usage.output > 0 {
                let tps = event.usage.output as f64 / (duration_ms as f64 / 1000.0);
                if tps.is_finite() && tps > 0.0 {
                    self.tps_samples.push_back(tps);
                    while self.tps_samples.len() > TPS_SAMPLE_WINDOW {
                        self.tps_samples.pop_front();
                    }
                }
            }
        }

        self.last_event_at = Some(self.last_event_at.map_or(event_ts, |ts| ts.max(event_ts)));
    }

    /// 指标快照：TPS 中位数 + 5min 缓存命中率（分母 0 → null）
    fn metrics(&self, now_ms: i64) -> KimiRealtimeMetrics {
        // 缓存命中率 = inputCacheRead / (inputOther+inputCacheRead+inputCacheCreation)，
        // 窗口内总输入 0 → null（UI 显示 "—"，不显示 0%）
        let (mut cache_read, mut total_input) = (0u64, 0u64);
        for (ts, usage) in &self.recent_events {
            if now_ms - ts <= RECENT_WINDOW_MS {
                cache_read = cache_read.saturating_add(usage.cache_read);
                total_input = total_input.saturating_add(usage.total_input());
            }
        }
        let cache_hit_rate_5m = if total_input > 0 {
            Some(cache_read as f64 / total_input as f64 * 100.0)
        } else {
            None
        };

        KimiRealtimeMetrics {
            tps: median_tps(&self.tps_samples),
            sample_count: self.tps_samples.len().min(u8::MAX as usize) as u8,
            active_sessions: self.active_files.len().min(u32::MAX as usize) as u32,
            cache_hit_rate_5m,
            last_event_at: self.last_event_at,
        }
    }
}

static REALTIME: OnceLock<RwLock<KimiRealtimeState>> = OnceLock::new();

fn realtime() -> &'static RwLock<KimiRealtimeState> {
    REALTIME.get_or_init(|| RwLock::new(KimiRealtimeState::default()))
}

fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// 事件入库成功时喂入实时态（全局单例委托，生产路径）
fn realtime_record_event(event: &ParsedKimiUsage) {
    let now_ms = now_epoch_ms();
    realtime()
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .record_event(event, now_ms);
}

/// 全扫/tail 重建活跃文件集（mtime 距今 < ACTIVE_FILE_WINDOW 的 wire 文件）
fn realtime_set_active_files(files: HashSet<PathBuf>) {
    let mut state = realtime().write().unwrap_or_else(|p| p.into_inner());
    state.active_files = files;
}

/// 最近一分钟内有事件入库视为活跃（决定托盘标题的刷新档位：5s/60s）
pub fn kimi_realtime_is_active() -> bool {
    let state = realtime().read().unwrap_or_else(|p| p.into_inner());
    state
        .last_event_at
        .map(|ts| now_epoch_ms() - ts < 60_000)
        .unwrap_or(false)
}

/// 实时指标快照（前端实时卡片与托盘标题共用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiRealtimeMetrics {
    /// 最近 5 个有效样本的中位 TPS（取整）；无有效样本为 null
    pub tps: Option<f64>,
    /// 当前窗口内的有效样本数
    pub sample_count: u8,
    /// 活跃 wire 文件数（≈ 活跃会话×代理数）
    pub active_sessions: u32,
    /// 最近 5 分钟缓存命中率（百分比 0–100）；窗口总输入为 0 时 null
    pub cache_hit_rate_5m: Option<f64>,
    /// 最后一次入库事件时间（epoch ms）
    pub last_event_at: Option<i64>,
}

/// 样本中位数：偶数个取中间两值平均后 round（设计文档 02 §1.2）
fn median_tps(samples: &VecDeque<f64>) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = samples.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let mid = n / 2;
    let value = if n % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    };
    Some(value.round())
}

pub fn get_kimi_realtime_metrics() -> KimiRealtimeMetrics {
    let state = realtime().read().unwrap_or_else(|p| p.into_inner());
    state.metrics(now_epoch_ms())
}

// usage 四分量字段的多别名优先级链（应对未文档化格式漂移，参考官方
// metrics.js normalizeUsage 的候选键表）
const INPUT_OTHER_KEYS: &[&str] = &[
    "inputOther",
    "input_other",
    "inputTokens",
    "input_tokens",
    "prompt_tokens",
];
const OUTPUT_KEYS: &[&str] = &[
    "output",
    "outputTokens",
    "output_tokens",
    "completion_tokens",
];
const CACHE_READ_KEYS: &[&str] = &[
    "inputCacheRead",
    "input_cache_read",
    "cacheReadTokens",
    "cache_read_input_tokens",
    "cached_tokens",
];
const CACHE_CREATION_KEYS: &[&str] = &[
    "inputCacheCreation",
    "input_cache_creation",
    "cacheCreationTokens",
    "cache_creation_input_tokens",
];

// step.end 计时字段（主名 + snake_case 防御性别名）
const FIRST_TOKEN_KEYS: &[&str] = &["llmFirstTokenLatencyMs", "llm_first_token_latency_ms"];
const STREAM_DURATION_KEYS: &[&str] = &["llmStreamDurationMs", "llm_stream_duration_ms"];
const REQUEST_BUILD_KEYS: &[&str] = &["llmRequestBuildMs", "llm_request_build_ms"];

/// wire.jsonl 的 usage 四分量（`inputOther/output/inputCacheRead/inputCacheCreation`）
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct KimiTokenUsage {
    input_other: u64,
    output: u64,
    cache_read: u64,
    cache_creation: u64,
}

impl KimiTokenUsage {
    fn is_zero(&self) -> bool {
        self.input_other == 0
            && self.output == 0
            && self.cache_read == 0
            && self.cache_creation == 0
    }

    /// 总输入口径（设计文档 02 §1.1）：三种输入分量之和
    fn total_input(&self) -> u64 {
        self.input_other
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_creation)
    }
}

/// 单个 wire.jsonl 解析出的用量事件（step.end 或 usage.record(turn)）
#[derive(Debug)]
struct ParsedKimiUsage {
    line_offset: i64,
    usage: KimiTokenUsage,
    /// 回填 `__secondary__` 后的真实模型名
    model: String,
    first_token_ms: Option<i64>,
    duration_ms: Option<i64>,
    latency_ms: Option<i64>,
    /// 记录 `time` 字段归一化为 epoch 毫秒
    timestamp_ms: Option<i64>,
}

/// 解析过程中的 per-file 模型状态（供 __secondary__ 前向回填）
#[derive(Debug, Default)]
struct WireFileState {
    last_model: Option<String>,
    last_model_alias: Option<String>,
}

/// 单个 wire.jsonl 的解析结果
#[derive(Debug, Default)]
struct ParsedWireFile {
    events: Vec<ParsedKimiUsage>,
    /// 以 `\n` 结尾的完整行数（撕裂末行不计入，游标也不越过它）
    complete_lines: i64,
    /// 未知 type / 全零 usage / 结构缺字段的跳过计数（仅日志用）
    skipped_records: u32,
    /// 因同文件存在 step.end 而被抑制的 usage.record(turn) 用量行计数
    /// （防双算，模型名仍已同步 lastModel）
    suppressed_usage_records: u32,
    /// 首行 metadata 的 protocol_version 与期望值不一致时记录实际值
    protocol_mismatch: Option<String>,
    errors: Vec<String>,
}

/// 一个待同步的 wire.jsonl 及其路径推导出的会话身份
#[derive(Debug)]
struct KimiWireFile {
    path: PathBuf,
    session_id: String,
    agent_id: String,
}

/// 按别名优先级链取字段
fn get_alias<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|key| value.get(*key))
}

/// 非负整数归一：接受数字 / 数字字符串 / 小数（截断），负数归零
fn coerce_non_negative_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    match value? {
        serde_json::Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_i64()
                .map(|v| v.max(0) as u64)
                .or_else(|| number.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
        }),
        serde_json::Value::String(text) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| *f >= 0.0)
            .map(|f| f as u64),
        _ => None,
    }
}

/// 计时字段（毫秒）归一
fn coerce_ms(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    coerce_non_negative_u64(get_alias(value, keys)).map(|v| v.min(i64::MAX as u64) as i64)
}

/// 记录时间戳归一为 epoch 毫秒：秒 / 毫秒 / ISO8601 字符串混杂兼容。
///
/// 阈值 1e11：小于它视作秒（year 5138 才到 1e11 秒），大于等于视作毫秒。
fn parse_record_time_ms(value: Option<&serde_json::Value>) -> Option<i64> {
    let normalize = |n: u64| {
        let ms = if n < 100_000_000_000 {
            n.saturating_mul(1000)
        } else {
            n
        };
        ms.min(i64::MAX as u64) as i64
    };
    match value? {
        serde_json::Value::Number(_) => coerce_non_negative_u64(value).map(normalize),
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
                return Some(dt.timestamp_millis());
            }
            trimmed
                .parse::<f64>()
                .ok()
                .filter(|f| *f >= 0.0)
                .map(|f| normalize(f as u64))
        }
        _ => None,
    }
}

/// 解析 usage 对象四分量（多别名优先级链 + 非负整数归一）
fn parse_usage_object(usage: Option<&serde_json::Value>) -> Option<KimiTokenUsage> {
    let usage = usage.filter(|v| v.is_object())?;
    Some(KimiTokenUsage {
        input_other: coerce_non_negative_u64(get_alias(usage, INPUT_OTHER_KEYS)).unwrap_or(0),
        output: coerce_non_negative_u64(get_alias(usage, OUTPUT_KEYS)).unwrap_or(0),
        cache_read: coerce_non_negative_u64(get_alias(usage, CACHE_READ_KEYS)).unwrap_or(0),
        cache_creation: coerce_non_negative_u64(get_alias(usage, CACHE_CREATION_KEYS))
            .unwrap_or(0),
    })
}

fn str_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    get_alias(value, keys)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// 模型名解析：真实名直接采用并同步 lastModel；`__secondary__`（或空 model）
/// 以最近一次 `llm.request` / `usage.record` 的 lastModel 前向回填。
///
/// 历史存量行的批量回填一期不做（设计文档 02 §2.1 明确决策），只做新行的
/// 前向别名替换。
fn resolve_model(state: &mut WireFileState, raw: Option<&str>) -> String {
    let raw = raw.map(str::trim).filter(|m| !m.is_empty());
    match raw {
        Some(model) if model == SECONDARY_MODEL_PLACEHOLDER => {
            let resolved = state
                .last_model
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            log::debug!(
                "[KIMI-SYNC] {SECONDARY_MODEL_PLACEHOLDER} 回填为 {resolved}（lastModelAlias={:?}）",
                state.last_model_alias
            );
            resolved
        }
        Some(model) => {
            state.last_model = Some(model.to_string());
            model.to_string()
        }
        None => state
            .last_model
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

/// 解析单个 wire.jsonl，只消费以 `\n` 结尾的完整行（撕裂末行不解析）。
fn parse_wire_file(file_path: &Path) -> Result<ParsedWireFile, AppError> {
    let bytes = fs::read(file_path)
        .map_err(|e| AppError::Config(format!("无法打开文件: {e}")))?;
    let mut parsed = ParsedWireFile::default();
    let mut state = WireFileState::default();

    // 撕裂末行：只消费到最后一个 \n 为止；末尾残段下轮自然重读
    let complete_len = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |pos| pos + 1);
    let content = &bytes[..complete_len];
    parsed.complete_lines = content.iter().filter(|b| **b == b'\n').count() as i64;

    // 世代探测（文件级，对全部完整行生效，跨增量批次稳定）：当前代文件
    // 每个 step.end 都配对一条 usage 完全相同的 usage.record(turn)，两者
    // 并存时 usage.record 的 usage 行必须一律抑制，否则精确 2 倍双算；
    // 只有全文件无 step.end 的 legacy 文件才让 usage.record 作 fallback。
    // 子串探测的取舍：官方 writer 产出紧凑 JSON（"type":"step.end"），扫
    // 带引号的 "step.end" 字节串即可，"type": "step.end" 带空格变体同样
    // 命中；消息文本里出现该词时是 JSON 转义形态（\"step.end\"），不会误中。
    //
    // 已知边界（一期接受，不修）：活跃文件的 60s 同步若恰好落在某步
    // usage.record 已 flush 而 step.end 未写的毫秒级窗口，该批会把这条
    // usage.record 当 legacy 行入库、下批再入 step.end，产生一次单行重复。
    const STEP_END_PROBE: &[u8] = b"\"step.end\"";
    let has_step_end = content
        .windows(STEP_END_PROBE.len())
        .any(|window| window == STEP_END_PROBE);

    for (index, raw_line) in content.split(|b| *b == b'\n').enumerate() {
        if index as i64 >= parsed.complete_lines {
            break;
        }
        let line_offset = index as i64 + 1;
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_slice(line) {
            Ok(value) => value,
            Err(e) => {
                // 单行损坏计入 errors 跳过，不中断文件
                parsed.errors.push(format!(
                    "Kimi wire 第 {line_offset} 行 JSON 解析失败 {}: {e}",
                    file_path.display()
                ));
                continue;
            }
        };
        let record_type = value.get("type").and_then(serde_json::Value::as_str).unwrap_or("");
        let record_time_ms =
            parse_record_time_ms(value.get("time").or_else(|| value.get("timestamp")));

        match record_type {
            "metadata" => {
                let version = value
                    .get("protocol_version")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                if version != EXPECTED_PROTOCOL_VERSION {
                    log::warn!(
                        "[KIMI-SYNC] {} protocol_version={version}（期望 {EXPECTED_PROTOCOL_VERSION}），继续尽力解析",
                        file_path.display()
                    );
                    parsed.protocol_mismatch = Some(version.to_string());
                }
            }
            "context.append_loop_event" => {
                let Some(event) = value.get("event") else {
                    parsed.skipped_records = parsed.skipped_records.saturating_add(1);
                    continue;
                };
                if event.get("type").and_then(serde_json::Value::as_str) != Some("step.end") {
                    parsed.skipped_records = parsed.skipped_records.saturating_add(1);
                    continue;
                }
                let Some(usage) = parse_usage_object(event.get("usage")) else {
                    parsed.skipped_records = parsed.skipped_records.saturating_add(1);
                    continue;
                };
                if usage.is_zero() {
                    parsed.skipped_records = parsed.skipped_records.saturating_add(1);
                    continue;
                }
                let model = resolve_model(&mut state, str_field(event, &["model"]).as_deref());
                let first_token_ms = coerce_ms(event, FIRST_TOKEN_KEYS);
                let duration_ms = coerce_ms(event, STREAM_DURATION_KEYS);
                let build_ms = coerce_ms(event, REQUEST_BUILD_KEYS);
                // latency = 请求构建 + 流式解码（设计文档 02 §2.2 字段映射表）
                let latency_ms = match (build_ms, duration_ms) {
                    (Some(build), Some(stream)) => Some(build.saturating_add(stream)),
                    (Some(build), None) => Some(build),
                    (None, stream) => stream,
                };
                parsed.events.push(ParsedKimiUsage {
                    line_offset,
                    usage,
                    model,
                    first_token_ms,
                    duration_ms,
                    latency_ms,
                    timestamp_ms: record_time_ms,
                });
            }
            "usage.record" => {
                if str_field(&value, &["usageScope", "usage_scope"]).as_deref() != Some("turn") {
                    parsed.skipped_records = parsed.skipped_records.saturating_add(1);
                    continue;
                }
                // 权威模型名来源：即使 usage 全零跳过入库，也要先同步 lastModel
                let model = resolve_model(&mut state, str_field(&value, &["model"]).as_deref());
                let Some(usage) = parse_usage_object(value.get("usage")) else {
                    parsed.skipped_records = parsed.skipped_records.saturating_add(1);
                    continue;
                };
                if usage.is_zero() {
                    parsed.skipped_records = parsed.skipped_records.saturating_add(1);
                    continue;
                }
                if has_step_end {
                    // 防双算抑制：当前代文件 usage.record 只作模型名来源
                    // （上面的 resolve_model 已同步 lastModel），usage 行由
                    // 配对的 step.end 入账。
                    parsed.suppressed_usage_records =
                        parsed.suppressed_usage_records.saturating_add(1);
                    continue;
                }
                parsed.events.push(ParsedKimiUsage {
                    line_offset,
                    usage,
                    model,
                    first_token_ms: None,
                    duration_ms: None,
                    latency_ms: None,
                    timestamp_ms: record_time_ms,
                });
            }
            "llm.request" => {
                if let Some(model) = str_field(&value, &["model"]) {
                    state.last_model = Some(model);
                }
                if let Some(alias) = str_field(&value, &["modelAlias", "model_alias"]) {
                    state.last_model_alias = Some(alias);
                }
            }
            // context.append_message / apply_compaction / undo / clear 等一律跳过
            _ => {
                parsed.skipped_records = parsed.skipped_records.saturating_add(1);
            }
        }
    }

    Ok(parsed)
}

/// 枚举 sessions/<workDirKey>/<sessionId>/(agents/<agentId>/)?wire.jsonl，只读
fn collect_kimi_wire_files(sessions_dir: &Path) -> Vec<KimiWireFile> {
    let mut files = Vec::new();
    let Ok(work_dirs) = fs::read_dir(sessions_dir) else {
        return files;
    };

    for work_entry in work_dirs.flatten() {
        let work_path = work_entry.path();
        if !work_path.is_dir() {
            continue;
        }
        let Ok(session_dirs) = fs::read_dir(&work_path) else {
            continue;
        };
        for session_entry in session_dirs.flatten() {
            let session_path = session_entry.path();
            if !session_path.is_dir() {
                continue;
            }
            let Some(session_id) = session_entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };

            // 根级兜底布局（旧版本无 agents/ 目录）
            let root_wire = session_path.join("wire.jsonl");
            if root_wire.is_file() {
                files.push(KimiWireFile {
                    path: root_wire,
                    session_id: session_id.clone(),
                    agent_id: ROOT_WIRE_FALLBACK_AGENT.to_string(),
                });
            }

            let Ok(agent_dirs) = fs::read_dir(session_path.join("agents")) else {
                continue;
            };
            for agent_entry in agent_dirs.flatten() {
                let agent_path = agent_entry.path();
                if !agent_path.is_dir() {
                    continue;
                }
                let Some(agent_id) = agent_entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let wire = agent_path.join("wire.jsonl");
                if wire.is_file() {
                    files.push(KimiWireFile {
                        path: wire,
                        session_id: session_id.clone(),
                        agent_id,
                    });
                }
            }
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

#[derive(Debug, Default)]
struct KimiFileSyncResult {
    imported: u32,
    skipped: u32,
    errors: Vec<String>,
}

/// 同步 Kimi Code 使用数据（从 wire.jsonl 会话日志）
pub fn sync_kimi_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    let sessions_dir = get_kimi_code_config_dir().join("sessions");
    sync_kimi_usage_from(db, &sessions_dir)
}

/// 同步内核（与目录来源解耦，便于测试注入临时目录）
fn sync_kimi_usage_from(db: &Database, sessions_dir: &Path) -> Result<SessionSyncResult, AppError> {
    let files = collect_kimi_wire_files(sessions_dir);
    // 全扫负责重建实时 tail 的活跃文件集（设计文档 02 §2.3：60s 兜底层
    // 发现新文件，tail 层只处理活跃集）
    realtime_set_active_files(collect_active_files(&files));

    let mut result = SessionSyncResult {
        files_scanned: files.len() as u32,
        ..SessionSyncResult::default()
    };

    for wire in &files {
        match sync_single_kimi_file(db, wire) {
            Ok(file_result) => {
                result.imported = result.imported.saturating_add(file_result.imported);
                result.skipped = result.skipped.saturating_add(file_result.skipped);
                result.errors.extend(file_result.errors);
            }
            Err(e) => {
                let msg = format!("Kimi 会话文件解析失败 {}: {e}", wire.path.display());
                log::warn!("[KIMI-SYNC] {msg}");
                result.errors.push(msg);
            }
        }
    }

    if result.imported > 0 {
        log::info!(
            "[KIMI-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }

    Ok(result)
}

/// 从活跃文件路径反推会话身份（与 `collect_kimi_wire_files` 的推导互逆）：
/// `<workDirKey>/<sessionId>/wire.jsonl` → main agent；
/// `<workDirKey>/<sessionId>/agents/<agentId>/wire.jsonl` → 该 agent。
fn wire_file_from_path(sessions_dir: &Path, path: PathBuf) -> Option<KimiWireFile> {
    let (session_id, agent_id) = {
        let rel = path.strip_prefix(sessions_dir).ok()?;
        let comps: Vec<&str> = rel
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => s.to_str(),
                _ => None,
            })
            .collect();
        match comps.as_slice() {
            [_, session_id, file] if *file == "wire.jsonl" => {
                ((*session_id).to_string(), ROOT_WIRE_FALLBACK_AGENT.to_string())
            }
            [_, session_id, "agents", agent_id, file] if *file == "wire.jsonl" => {
                ((*session_id).to_string(), (*agent_id).to_string())
            }
            _ => return None,
        }
    };
    Some(KimiWireFile {
        path,
        session_id,
        agent_id,
    })
}

/// 实时 tail（设计文档 02 §2.3 实时层）：只处理活跃集内的 wire 文件。
///
/// 活跃集由 60s 全扫重建；tail 过程中按 mtime 新鲜度就地剔除静默文件。
/// `sync_single_kimi_file` 自带 `(mtime, offset)` 游标检查，未变化的文件
/// 每轮只做一次 stat，开销极小。调用方需持有 `session_sync_mutex`（与
/// 全扫互斥），行号 request_id + INSERT OR IGNORE 幂等兜底。
pub fn tail_active_kimi_files(db: &Database) -> Result<SessionSyncResult, AppError> {
    let sessions_dir = get_kimi_code_config_dir().join("sessions");
    let candidates: Vec<PathBuf> = {
        let state = realtime().read().unwrap_or_else(|p| p.into_inner());
        state.active_files.iter().cloned().collect()
    };
    tail_kimi_files(db, &sessions_dir, candidates)
}

/// tail 内核（与活跃集来源解耦，便于测试注入候选文件）
fn tail_kimi_files(
    db: &Database,
    sessions_dir: &Path,
    candidates: Vec<PathBuf>,
) -> Result<SessionSyncResult, AppError> {
    if candidates.is_empty() {
        return Ok(SessionSyncResult::default());
    }

    let mut result = SessionSyncResult::default();
    let mut still_active = HashSet::new();
    for path in candidates {
        // 新鲜度统计：无论同步结果如何都先更新活跃集，剔除静默文件
        if let Ok(metadata) = fs::metadata(&path) {
            let age = SystemTime::now()
                .duration_since(metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or_default();
            if age < ACTIVE_FILE_WINDOW {
                still_active.insert(path.clone());
            }
        } else {
            // 文件已删除：从活跃集剔除
            continue;
        }

        let Some(wire) = wire_file_from_path(&sessions_dir, path.clone()) else {
            log::debug!("[KIMI-TAIL] 无法从路径推导会话身份: {}", path.display());
            continue;
        };
        result.files_scanned = result.files_scanned.saturating_add(1);
        match sync_single_kimi_file(db, &wire) {
            Ok(file_result) => {
                result.imported = result.imported.saturating_add(file_result.imported);
                result.skipped = result.skipped.saturating_add(file_result.skipped);
                result.errors.extend(file_result.errors);
            }
            Err(e) => {
                let msg = format!("Kimi tail 解析失败 {}: {e}", wire.path.display());
                log::warn!("[KIMI-TAIL] {msg}");
                result.errors.push(msg);
            }
        }
    }
    realtime_set_active_files(still_active);

    if result.imported > 0 {
        crate::usage_events::notify_log_recorded();
    }
    Ok(result)
}

/// 统计文件集中 mtime 距今 < ACTIVE_FILE_WINDOW 的活跃 wire 文件
fn collect_active_files(files: &[KimiWireFile]) -> HashSet<PathBuf> {
    let now = SystemTime::now();
    files
        .iter()
        .filter(|f| {
            fs::metadata(&f.path)
                .and_then(|m| m.modified())
                .map(|modified| {
                    now.duration_since(modified).unwrap_or_default() < ACTIVE_FILE_WINDOW
                })
                .unwrap_or(false)
        })
        .map(|f| f.path.clone())
        .collect()
}

/// 同步单个 wire.jsonl（增量游标 `(file_mtime, line_offset)`）
fn sync_single_kimi_file(db: &Database, wire: &KimiWireFile) -> Result<KimiFileSyncResult, AppError> {
    let file_path_str = wire.path.to_string_lossy().to_string();

    let metadata = fs::metadata(&wire.path)
        .map_err(|e| AppError::Config(format!("无法读取文件元数据: {e}")))?;
    let file_modified = metadata_modified_nanos(&metadata);

    let (last_modified, last_offset) = get_sync_state(db, &file_path_str)?;

    // mtime 未变直接跳过
    if file_modified <= last_modified {
        return Ok(KimiFileSyncResult::default());
    }

    let parsed = parse_wire_file(&wire.path)?;

    // 文件变小（截断/重建）：offset 归零全量重扫，request_id 幂等兜底
    let effective_offset = if parsed.complete_lines < last_offset {
        0
    } else {
        last_offset
    };

    let mut result = KimiFileSyncResult {
        errors: parsed.errors,
        ..KimiFileSyncResult::default()
    };
    for event in &parsed.events {
        if event.line_offset <= effective_offset {
            continue;
        }
        let request_id = format!(
            "{KIMI_SESSION_REQUEST_ID_PREFIX}:{}:{}:{}",
            wire.session_id, wire.agent_id, event.line_offset
        );
        match insert_kimi_session_entry(db, &request_id, event, &wire.session_id) {
            Ok(true) => {
                result.imported = result.imported.saturating_add(1);
                // 喂入实时监控内存态（TPS 样本 + 5min 窗口，设计文档 02 §2.3）
                realtime_record_event(event);
            }
            Ok(false) => result.skipped = result.skipped.saturating_add(1),
            Err(e) => {
                log::warn!("[KIMI-SYNC] 插入失败 ({request_id}): {e}");
                result.skipped = result.skipped.saturating_add(1);
            }
        }
    }

    update_sync_state(db, &file_path_str, file_modified, parsed.complete_lines)?;
    if parsed.skipped_records > 0 || parsed.suppressed_usage_records > 0 {
        log::debug!(
            "[KIMI-SYNC] {} 跳过 {} 条非用量记录, 抑制 {} 条 usage.record（防双算）",
            wire.path.display(),
            parsed.skipped_records,
            parsed.suppressed_usage_records
        );
    }
    Ok(result)
}

/// 插入单条 Kimi 会话记录到 proxy_request_logs，返回是否成功插入 (true=新插入, false=已存在)
fn insert_kimi_session_entry(
    db: &Database,
    request_id: &str,
    event: &ParsedKimiUsage,
    session_id: &str,
) -> Result<bool, AppError> {
    let conn = lock_conn!(db.conn);

    // created_at 列语义为 epoch 秒（与 claude/codex session 行一致）
    let created_at = event.timestamp_ms.map(|ms| ms / 1000).unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });

    let clamp = |v: u64| v.min(u32::MAX as u64) as u32;
    let usage = TokenUsage {
        // 总输入口径（含 cache read/creation），与 input_token_semantics=TOTAL 配套
        input_tokens: clamp(event.usage.total_input()),
        output_tokens: clamp(event.usage.output),
        cache_read_tokens: clamp(event.usage.cache_read),
        cache_creation_tokens: clamp(event.usage.cache_creation),
        model: Some(event.model.clone()),
        message_id: None,
    };

    let dedup_key = DedupKey {
        app_type: KIMI_APP_TYPE,
        model: &event.model,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        created_at,
    };
    if should_skip_session_insert(&conn, request_id, &dedup_key)? {
        return Ok(false);
    }

    // 计算费用；缺定价沿用"缺价不计费"行为，成本记 0
    let pricing: Option<ModelPricing> = find_model_pricing(&conn, &event.model);
    let multiplier = Decimal::from(1);
    let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) = match pricing
    {
        Some(p) => {
            let cost = CostCalculator::calculate_for_app(KIMI_APP_TYPE, &usage, &p, multiplier);
            (
                cost.input_cost.to_string(),
                cost.output_cost.to_string(),
                cost.cache_read_cost.to_string(),
                cost.cache_creation_cost.to_string(),
                cost.total_cost.to_string(),
            )
        }
        None => (
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
        ),
    };

    let inserted_rows = conn
        .execute(
            "INSERT OR IGNORE INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
            latency_ms, first_token_ms, duration_ms, status_code, error_message, session_id,
            provider_type, is_streaming, cost_multiplier, created_at, data_source,
            input_token_semantics
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
            rusqlite::params![
                request_id,
                KIMI_PROVIDER_ID,       // provider_id
                KIMI_APP_TYPE,          // app_type
                event.model,
                event.model,            // request_model = model（回填后的真实名）
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens,
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                total_cost,
                event.latency_ms.unwrap_or(0), // latency_ms = build + stream
                event.first_token_ms,   // first_token_ms = llmFirstTokenLatencyMs
                event.duration_ms,      // duration_ms = llmStreamDurationMs
                200i64,                 // status_code
                Option::<String>::None, // error_message
                session_id,
                Some(KIMI_DATA_SOURCE), // provider_type
                1i64,                   // is_streaming
                "1.0",                  // cost_multiplier
                created_at,
                KIMI_DATA_SOURCE,       // data_source
                INPUT_TOKEN_SEMANTICS_TOTAL,
            ],
        )
        .map_err(|e| AppError::Database(format!("插入 Kimi 会话日志失败: {e}")))?;

    Ok(inserted_rows > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    // KIMI_CODE_HOME 是进程级环境变量，读写它的用例必须串行，
    // 避免与并行测试线程互相污染（同 kimi_code_config.rs 测试的模式）。
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

    fn metadata_line() -> serde_json::Value {
        json!({"type": "metadata", "protocol_version": "1.4", "created_at": 1754000000000i64})
    }

    fn usage_four() -> serde_json::Value {
        json!({"inputOther": 100, "output": 20, "inputCacheRead": 400, "inputCacheCreation": 50})
    }

    fn step_end(time: serde_json::Value, usage: serde_json::Value) -> serde_json::Value {
        json!({
            "type": "context.append_loop_event",
            "time": time,
            "event": {
                "type": "step.end",
                "usage": usage,
                "llmFirstTokenLatencyMs": 320,
                "llmStreamDurationMs": 1500,
                "llmRequestBuildMs": 45
            }
        })
    }

    fn usage_record_turn(model: &str, time: serde_json::Value, usage: serde_json::Value) -> serde_json::Value {
        json!({
            "type": "usage.record",
            "time": time,
            "usageScope": "turn",
            "model": model,
            "usage": usage
        })
    }

    fn llm_request(model: &str, alias: &str) -> serde_json::Value {
        json!({"type": "llm.request", "time": 1754000000100i64, "model": model, "modelAlias": alias})
    }

    fn write_wire_raw(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn write_wire(path: &Path, lines: &[serde_json::Value]) {
        let content = lines
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        write_wire_raw(path, &content);
    }

    #[test]
    fn torn_last_line_is_not_consumed() {
        let temp = tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        let mut content = [metadata_line(), step_end(json!(1754000000000i64), usage_four())]
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        // 撕裂末行：合法 JSON 但没有结尾 \n，不得消费
        content.push_str(&step_end(json!(1754000001000i64), usage_four()).to_string());
        write_wire_raw(&wire, &content);

        let parsed = parse_wire_file(&wire).unwrap();
        assert_eq!(parsed.complete_lines, 2);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].line_offset, 2);
    }

    #[test]
    fn corrupt_middle_line_does_not_panic() {
        let temp = tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        let lines = vec![
            metadata_line().to_string(),
            step_end(json!(1754000000000i64), usage_four()).to_string(),
            "{broken json".to_string(),
            step_end(json!(1754000001000i64), usage_four()).to_string(),
        ];
        write_wire_raw(&wire, &(lines.join("\n") + "\n"));

        let parsed = parse_wire_file(&wire).unwrap();
        assert_eq!(parsed.events.len(), 2);
        assert_eq!(parsed.errors.len(), 1);
        assert!(parsed.errors[0].contains("第 3 行"));
    }

    #[test]
    fn protocol_version_mismatch_warns_but_parses() {
        let temp = tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        write_wire(
            &wire,
            &[
                json!({"type": "metadata", "protocol_version": "1.6"}),
                step_end(json!(1754000000000i64), usage_four()),
            ],
        );

        let parsed = parse_wire_file(&wire).unwrap();
        assert_eq!(parsed.protocol_mismatch.as_deref(), Some("1.6"));
        assert_eq!(parsed.events.len(), 1);
    }

    #[test]
    fn mixed_timestamp_formats_normalize_to_ms() {
        let temp = tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        let iso = "2025-08-01T00:13:20Z";
        write_wire(
            &wire,
            &[
                step_end(json!(1754000000i64), usage_four()),      // 秒
                step_end(json!(1754000000500i64), usage_four()),   // 毫秒
                step_end(json!(iso), usage_four()),                // ISO8601
            ],
        );

        let parsed = parse_wire_file(&wire).unwrap();
        assert_eq!(parsed.events.len(), 3);
        assert_eq!(parsed.events[0].timestamp_ms, Some(1754000000000));
        assert_eq!(parsed.events[1].timestamp_ms, Some(1754000000500));
        let expected_iso = DateTime::parse_from_rfc3339(iso)
            .unwrap()
            .timestamp_millis();
        assert_eq!(parsed.events[2].timestamp_ms, Some(expected_iso));
    }

    #[test]
    fn zero_usage_records_are_skipped() {
        let temp = tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        let zero = json!({"inputOther": 0, "output": 0, "inputCacheRead": 0, "inputCacheCreation": 0});
        write_wire(
            &wire,
            &[
                step_end(json!(1754000000000i64), zero.clone()),
                usage_record_turn("kimi-for-coding", json!(1754000001000i64), zero),
            ],
        );

        let parsed = parse_wire_file(&wire).unwrap();
        assert!(parsed.events.is_empty());
        assert_eq!(parsed.skipped_records, 2);
    }

    #[test]
    fn usage_field_aliases_follow_priority_chain() {
        let temp = tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        let aliased = json!({
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "cached_tokens": 400,
            "cache_creation_input_tokens": 50
        });
        write_wire(&wire, &[step_end(json!(1754000000000i64), aliased)]);

        let parsed = parse_wire_file(&wire).unwrap();
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(
            parsed.events[0].usage,
            KimiTokenUsage {
                input_other: 100,
                output: 20,
                cache_read: 400,
                cache_creation: 50,
            }
        );
    }

    #[test]
    fn secondary_model_is_replaced_by_last_llm_request_model() {
        let temp = tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        write_wire(
            &wire,
            &[
                llm_request("kimi-for-coding", "agent-2"),
                usage_record_turn("__secondary__", json!(1754000000000i64), usage_four()),
                // 空 model 同样以 lastModel 回填
                json!({
                    "type": "usage.record",
                    "time": 1754000001000i64,
                    "usageScope": "turn",
                    "usage": usage_four()
                }),
            ],
        );

        let parsed = parse_wire_file(&wire).unwrap();
        assert_eq!(parsed.events.len(), 2);
        assert_eq!(parsed.events[0].model, "kimi-for-coding");
        assert_eq!(parsed.events[1].model, "kimi-for-coding");
    }

    #[test]
    fn both_record_generations_produce_events() {
        let temp = tempdir().unwrap();

        // legacy 文件（全文件无 step.end）：usage.record 作 fallback 用量行
        let legacy_wire = temp.path().join("legacy.jsonl");
        write_wire(
            &legacy_wire,
            &[
                metadata_line(),
                usage_record_turn("kimi-for-coding", json!(1754000000000i64), usage_four()),
            ],
        );
        let legacy = parse_wire_file(&legacy_wire).unwrap();
        assert_eq!(legacy.events.len(), 1);
        // usage.record 行无计时字段
        assert_eq!(legacy.events[0].first_token_ms, None);
        assert_eq!(legacy.events[0].duration_ms, None);
        assert_eq!(legacy.events[0].model, "kimi-for-coding");
        assert_eq!(legacy.suppressed_usage_records, 0);

        // 当前代文件：step.end 自带四分量与计时
        let current_wire = temp.path().join("current.jsonl");
        write_wire(
            &current_wire,
            &[
                metadata_line(),
                llm_request("kimi-for-coding", "main"),
                step_end(json!(1754000001000i64), usage_four()),
            ],
        );
        let current = parse_wire_file(&current_wire).unwrap();
        assert_eq!(current.events.len(), 1);
        assert_eq!(current.events[0].first_token_ms, Some(320));
        assert_eq!(current.events[0].duration_ms, Some(1500));
        assert_eq!(current.events[0].latency_ms, Some(45 + 1500));
        assert_eq!(current.events[0].model, "kimi-for-coding");
    }

    /// 当前代防双算：同文件交错的 step.end + usage.record（同 usage）只入
    /// step.end 行；usage.record 的模型名仍同步 lastModel（权威来源）
    #[test]
    fn interleaved_step_end_suppresses_paired_usage_record() {
        let temp = tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        write_wire(
            &wire,
            &[
                metadata_line(),
                llm_request("k3-256k", "main"),
                // 真实当前代行序：usage.record 比其 step.end 早几行，usage 完全相同
                usage_record_turn("kimi-code/k3-256k", json!(1754000000000i64), usage_four()),
                step_end(json!(1754000000007i64), usage_four()),
            ],
        );

        let parsed = parse_wire_file(&wire).unwrap();
        // events 只含 step.end 行，无重复
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].line_offset, 4);
        assert_eq!(parsed.suppressed_usage_records, 1);
        // lastModel 已被 usage.record 同步（step.end 自身不带 model 字段）
        assert_eq!(parsed.events[0].model, "kimi-code/k3-256k");
    }

    fn seed_kimi_pricing(db: &Database) -> Result<(), AppError> {
        let conn = lock_conn!(db.conn);
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (
                model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
            ) VALUES ('kimi-for-coding', 'Kimi For Coding', '3', '15', '0.3', '3.75')",
            [],
        )
        .unwrap();
        Ok(())
    }

    /// 端到端：主/子代理 + 根级兜底 + 撕裂末行 + 两代格式 → 入库对账、
    /// 幂等重跑、增量追加（对应 05-roadmap §3 session_usage_kimi_ingest）
    #[test]
    #[serial_test::serial]
    fn end_to_end_sync_is_idempotent_and_incremental() -> Result<(), AppError> {
        let db = Database::memory()?;
        seed_kimi_pricing(&db)?;
        let temp = tempdir().unwrap();
        let sessions = temp.path().join("sessions");

        let main_wire = sessions.join("wd_proj/sess-1/agents/main/wire.jsonl");
        write_wire(
            &main_wire,
            &[
                metadata_line(),
                llm_request("kimi-for-coding", "main"),
                step_end(json!(1754000000000i64), usage_four()),
            ],
        );

        let sub_wire = sessions.join("wd_proj/sess-1/agents/agent-2/wire.jsonl");
        write_wire(
            &sub_wire,
            &[
                metadata_line(),
                llm_request("kimi-for-coding", "agent-2"),
                usage_record_turn("__secondary__", json!(1754000001000i64), usage_four()),
            ],
        );

        // 根级兜底布局（无 agents/ 目录）
        let root_wire = sessions.join("wd_proj/sess-2/wire.jsonl");
        write_wire(
            &root_wire,
            &[
                metadata_line(),
                usage_record_turn("kimi-for-coding", json!(1754000002000i64), usage_four()),
            ],
        );

        let result = sync_kimi_usage_from(&db, &sessions)?;
        assert_eq!(result.files_scanned, 3);
        assert_eq!(result.imported, 3);
        assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);

        let conn = lock_conn!(db.conn);

        // 主代理 step.end 行：四分量 + 总输入口径 + 计时映射 + 固定列
        //（拆成多个小元组断言——Rust 内建 trait 对元组的实现最多 12 元）
        let tokens: (i64, i64, i64, i64, i64) = conn.query_row(
            "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_token_semantics
             FROM proxy_request_logs WHERE request_id = 'kimi_code_session:sess-1:main:3'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
        assert_eq!(
            tokens,
            (550, 20, 400, 50, INPUT_TOKEN_SEMANTICS_TOTAL),
            "input_tokens 应为总输入口径 100 + 400 + 50"
        );
        let timing: (i64, i64, i64, i64) = conn.query_row(
            "SELECT first_token_ms, duration_ms, latency_ms, created_at
             FROM proxy_request_logs WHERE request_id = 'kimi_code_session:sess-1:main:3'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(timing, (320, 1500, 1545, 1754000000), "latency = 45 + 1500；created_at 归一为秒");
        let fixed: (String, String, String, String, i64, i64) = conn.query_row(
            "SELECT app_type, provider_id, data_source, session_id, is_streaming, status_code
             FROM proxy_request_logs WHERE request_id = 'kimi_code_session:sess-1:main:3'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )?;
        assert_eq!(
            fixed,
            (
                "kimi-code".to_string(),
                "_kimi_code_session".to_string(),
                "kimi_session".to_string(),
                "sess-1".to_string(),
                1,
                200,
            )
        );

        // 成本：与 CostCalculator 同口径复算对账
        let pricing = ModelPricing::from_strings("3", "15", "0.3", "3.75").unwrap();
        let expected_usage = TokenUsage {
            input_tokens: 550,
            output_tokens: 20,
            cache_read_tokens: 400,
            cache_creation_tokens: 50,
            model: Some("kimi-for-coding".to_string()),
            message_id: None,
        };
        let expected_cost =
            CostCalculator::calculate_for_app("kimi-code", &expected_usage, &pricing, Decimal::from(1));
        let stored_cost: String = conn.query_row(
            "SELECT total_cost_usd FROM proxy_request_logs WHERE request_id = 'kimi_code_session:sess-1:main:3'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(stored_cost, expected_cost.total_cost.to_string());

        // 子代理 __secondary__ 已回填真实模型名
        let sub_model: String = conn.query_row(
            "SELECT model FROM proxy_request_logs WHERE request_id = 'kimi_code_session:sess-1:agent-2:3'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(sub_model, "kimi-for-coding");

        // 根级兜底文件入库，agent 段为 main
        let root_session: String = conn.query_row(
            "SELECT session_id FROM proxy_request_logs WHERE request_id = 'kimi_code_session:sess-2:main:2'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(root_session, "sess-2");
        drop(conn);

        // 幂等：mtime 未变直接跳过
        let second = sync_kimi_usage_from(&db, &sessions)?;
        assert_eq!(second.imported, 0);

        // 增量：追加一行（含撕裂末行，撕裂行不消费）
        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut content = fs::read_to_string(&main_wire).unwrap();
        content.push_str(&step_end(json!(1754000003000i64), usage_four()).to_string());
        content.push('\n');
        content.push_str(&step_end(json!(1754000004000i64), usage_four()).to_string()); // 撕裂
        fs::write(&main_wire, content).unwrap();

        let third = sync_kimi_usage_from(&db, &sessions)?;
        assert_eq!(third.imported, 1, "撕裂末行不应入库");
        let conn = lock_conn!(db.conn);
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM proxy_request_logs WHERE data_source = 'kimi_session'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 4);
        // get_sync_state 内部会重新获取 db.conn（std Mutex 不可重入），
        // 必须先释放 conn，否则本线程自锁死锁
        drop(conn);
        // 游标停在新完整行（4 行：metadata + llm.request + 2 条 step.end）
        assert_eq!(
            get_sync_state(&db, &main_wire.to_string_lossy())?.1,
            4
        );

        // 再跑一遍仍幂等（新行也已落游标）
        let fourth = sync_kimi_usage_from(&db, &sessions)?;
        assert_eq!(fourth.imported, 0);
        Ok(())
    }

    /// 文件变小（截断/重建）：offset 归零全量重扫，request_id 幂等兜底
    #[test]
    #[serial_test::serial]
    fn shrunk_file_rescans_from_zero() -> Result<(), AppError> {
        let db = Database::memory()?;
        let temp = tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let wire = sessions.join("wd_proj/sess-1/agents/main/wire.jsonl");
        write_wire(
            &wire,
            &[
                metadata_line(),
                step_end(json!(1754000000000i64), usage_four()),
                step_end(json!(1754000001000i64), usage_four()),
            ],
        );
        assert_eq!(sync_kimi_usage_from(&db, &sessions)?.imported, 2);
        assert_eq!(get_sync_state(&db, &wire.to_string_lossy())?.1, 3);

        std::thread::sleep(std::time::Duration::from_millis(5));
        // 重建为更短的文件（只保留一条用量记录）
        write_wire(
            &wire,
            &[metadata_line(), step_end(json!(1754000002000i64), usage_four())],
        );
        let result = sync_kimi_usage_from(&db, &sessions)?;
        // request_id 撞行号（:2 已存在）→ INSERT OR IGNORE 幂等跳过
        assert_eq!(result.imported, 0);
        assert_eq!(get_sync_state(&db, &wire.to_string_lossy())?.1, 2);
        Ok(())
    }

    /// 代际升级追加场景：legacy 文件后续追加了 step.end，新批次起
    /// usage.record 被抑制（世代探测是文件级、对全量内容生效）
    #[test]
    #[serial_test::serial]
    fn legacy_file_upgraded_with_step_end_suppresses_new_usage_records() -> Result<(), AppError> {
        let db = Database::memory()?;
        let temp = tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let wire = sessions.join("wd_proj/sess-1/agents/main/wire.jsonl");

        // legacy 阶段：无 step.end，usage.record 作 fallback 正常入库
        write_wire(
            &wire,
            &[
                metadata_line(),
                usage_record_turn("kimi-for-coding", json!(1754000000000i64), usage_four()),
            ],
        );
        assert_eq!(sync_kimi_usage_from(&db, &sessions)?.imported, 1);

        // 升级代际：追加 llm.request + step.end + 与之配对的 usage.record
        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut content = fs::read_to_string(&wire).unwrap();
        for line in [
            llm_request("kimi-for-coding", "main"),
            step_end(json!(1754000001000i64), usage_four()),
            usage_record_turn("kimi-for-coding", json!(1754000001007i64), usage_four()),
        ] {
            content.push_str(&line.to_string());
            content.push('\n');
        }
        fs::write(&wire, content).unwrap();

        // 新批次：step.end 入库，其后配对的 usage.record 被抑制
        let result = sync_kimi_usage_from(&db, &sessions)?;
        assert_eq!(result.imported, 1, "新批次只有 step.end 入库，usage.record 应被抑制");
        let conn = lock_conn!(db.conn);
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM proxy_request_logs WHERE data_source = 'kimi_session'",
            [],
            |row| row.get(0),
        )?;
        // legacy 阶段 1 行 + 升级后 step.end 1 行
        assert_eq!(count, 2);
        Ok(())
    }

    /// 公共入口经 KIMI_CODE_HOME 环境变量定位 sessions 根目录
    #[test]
    fn sync_kimi_usage_resolves_sessions_via_env_home() -> Result<(), AppError> {
        let _lock = env_lock().lock().unwrap();
        let temp = tempdir().unwrap();
        let _guard = KimiCodeHomeGuard::set(temp.path());

        let wire = temp
            .path()
            .join("sessions/wd_proj/sess-1/agents/main/wire.jsonl");
        write_wire(
            &wire,
            &[
                metadata_line(),
                step_end(json!(1754000000000i64), usage_four()),
            ],
        );

        let db = Database::memory()?;
        let result = sync_kimi_usage(&db)?;
        assert_eq!(result.files_scanned, 1);
        assert_eq!(result.imported, 1);
        Ok(())
    }

    // ── 实时监控内存态（设计文档 02 §1.2/§2.3）──────────────────
    // 全部用例操作本地 KimiRealtimeState 实例，不碰全局单例，避免与并行
    // 测试线程的全局状态互相污染。

    fn make_event(output: u64, cache_read: u64, duration_ms: Option<i64>, ts: i64) -> ParsedKimiUsage {
        ParsedKimiUsage {
            line_offset: 1,
            usage: KimiTokenUsage {
                input_other: 100,
                output,
                cache_read,
                cache_creation: 0,
            },
            model: "kimi-for-coding".to_string(),
            first_token_ms: None,
            duration_ms,
            latency_ms: None,
            timestamp_ms: Some(ts),
        }
    }

    #[test]
    fn realtime_tps_median_and_sample_filtering() {
        let mut state = KimiRealtimeState::default();
        let now = now_epoch_ms();

        // 无效样本：duration < 100ms、output == 0、时长缺失，只丢弃不清空窗口
        state.record_event(&make_event(50, 0, Some(80), now), now);
        state.record_event(&make_event(0, 0, Some(2000), now), now);
        state.record_event(&make_event(50, 0, None, now), now);
        assert_eq!(state.metrics(now).sample_count, 0);
        assert_eq!(state.metrics(now).tps, None);

        // 有效样本：100 output / 2000ms = 50 t/s；300 output / 1500ms = 200 t/s
        state.record_event(&make_event(100, 0, Some(2000), now), now);
        state.record_event(&make_event(300, 0, Some(1500), now), now);
        let metrics = state.metrics(now);
        assert_eq!(metrics.sample_count, 2);
        // 偶数个取中间两值平均后 round：(50+200)/2 = 125
        assert_eq!(metrics.tps, Some(125.0));

        // 奇数个取中位
        state.record_event(&make_event(90, 0, Some(3000), now), now); // 30 t/s
        let metrics = state.metrics(now);
        assert_eq!(metrics.tps, Some(50.0), "中位数应为 50, 实际 {:?}", metrics.tps);

        // 窗口只保留最近 5 个有效样本，无效样本不重置历史
        for _ in 0..4 {
            state.record_event(&make_event(1000, 0, Some(1000), now), now); // 1000 t/s
        }
        state.record_event(&make_event(0, 0, None, now), now);
        let metrics = state.metrics(now);
        assert_eq!(metrics.sample_count, 5);
        assert_eq!(metrics.tps, Some(1000.0));
    }

    #[test]
    fn realtime_cache_hit_rate_window() {
        let mut state = KimiRealtimeState::default();
        let now = now_epoch_ms();

        // 窗口为空 → null（UI 显示 "—"，不显示 0%）
        assert_eq!(state.metrics(now).cache_hit_rate_5m, None);

        // inputOther=100, cacheRead=300, cacheCreation=0 → 命中率 300/400 = 75%
        state.record_event(&make_event(10, 300, Some(1000), now), now);
        let metrics = state.metrics(now);
        let rate = metrics.cache_hit_rate_5m.expect("应有命中率");
        assert!((rate - 75.0).abs() < 1e-9, "命中率 {rate} 应为 75%");
        assert_eq!(metrics.last_event_at, Some(now));

        // 6 分钟前的事件在下一轮喂入时被逐出窗口
        let mut state = KimiRealtimeState::default();
        state.record_event(&make_event(10, 300, Some(1000), now - 6 * 60 * 1000), now - 6 * 60 * 1000);
        state.record_event(&make_event(10, 100, Some(1000), now), now);
        let metrics = state.metrics(now);
        let rate = metrics.cache_hit_rate_5m.expect("应有命中率");
        assert!((rate - 50.0).abs() < 1e-9, "旧事件逐出后命中率 {rate} 应为 50%");
    }

    #[test]
    fn realtime_active_files_count() {
        let mut state = KimiRealtimeState::default();
        assert_eq!(state.metrics(now_epoch_ms()).active_sessions, 0);

        state.active_files.insert(PathBuf::from("/tmp/a/wire.jsonl"));
        state.active_files.insert(PathBuf::from("/tmp/b/wire.jsonl"));
        assert_eq!(state.metrics(now_epoch_ms()).active_sessions, 2);
    }

    #[test]
    fn wire_file_from_path_recovers_session_identity() {
        let sessions = Path::new("/home/u/.kimi-code/sessions");

        let root = wire_file_from_path(
            sessions,
            sessions.join("wd_proj/sess-1/wire.jsonl"),
        )
        .expect("根级布局");
        assert_eq!(root.session_id, "sess-1");
        assert_eq!(root.agent_id, ROOT_WIRE_FALLBACK_AGENT);

        let agent = wire_file_from_path(
            sessions,
            sessions.join("wd_proj/sess-2/agents/agent-9/wire.jsonl"),
        )
        .expect("agents 布局");
        assert_eq!(agent.session_id, "sess-2");
        assert_eq!(agent.agent_id, "agent-9");

        // 目录外的路径与不符合布局的路径一律拒绝
        assert!(wire_file_from_path(sessions, PathBuf::from("/elsewhere/wire.jsonl")).is_none());
        assert!(wire_file_from_path(sessions, sessions.join("wd_proj/sess-1/other.jsonl")).is_none());
    }

    #[test]
    fn tail_picks_up_appended_lines_incrementally() -> Result<(), AppError> {
        let temp = tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let wire = sessions.join("wd_proj/sess-tail/agents/main/wire.jsonl");
        write_wire(
            &wire,
            &[metadata_line(), step_end(json!(1754000000000i64), usage_four())],
        );

        let db = Database::memory()?;
        // 首批由全扫入库
        assert_eq!(sync_kimi_usage_from(&db, &sessions)?.imported, 1);

        // 模拟活跃会话追加两行
        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut content = fs::read_to_string(&wire).unwrap();
        for line in [
            step_end(json!(1754000001000i64), usage_four()),
            step_end(json!(1754000002000i64), usage_four()),
        ] {
            content.push_str(&line.to_string());
            content.push('\n');
        }
        fs::write(&wire, content).unwrap();

        // tail 只增量处理新增行；再跑一遍幂等（游标未变直接跳过）
        let result = tail_kimi_files(&db, &sessions, vec![wire.clone()])?;
        assert_eq!(result.imported, 2);
        let result = tail_kimi_files(&db, &sessions, vec![wire.clone()])?;
        assert_eq!(result.imported, 0);

        // 撕裂末行：未完结残段不消费、游标不前进、不产生错误
        let mut content = fs::read_to_string(&wire).unwrap();
        content.push_str(&step_end(json!(1754000003000i64), usage_four()).to_string());
        fs::write(&wire, content).unwrap();
        let result = tail_kimi_files(&db, &sessions, vec![wire.clone()])?;
        assert_eq!(result.imported, 0, "撕裂末行不得入库");
        assert!(result.errors.is_empty());

        // 残段补全（加上 \n）后下一轮 tail 正常消费
        let mut content = fs::read_to_string(&wire).unwrap();
        content.push('\n');
        fs::write(&wire, content).unwrap();
        let result = tail_kimi_files(&db, &sessions, vec![wire.clone()])?;
        assert_eq!(result.imported, 1);
        Ok(())
    }
}
