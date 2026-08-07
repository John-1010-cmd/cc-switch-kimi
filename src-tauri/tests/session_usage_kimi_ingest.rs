//! Kimi Code wire.jsonl → proxy_request_logs 入库链路的集成测试
//! （roadmap §2.3 `session_usage_kimi_ingest`）。
//!
//! 覆盖场景：在 tempdir 构造 `sessions/<workDirKey>/<sessionId>/agents/<agentId>/wire.jsonl`
//! fixture（主代理 step.end 四分量 + 计时、`__secondary__` 子代理经 `llm.request`
//! 回填真实模型名、与 step.end 同 usage 的配对 usage.record（必须被抑制防双算）、
//! 独立 legacy 会话文件的 fallback `usage.record(turn)`、撕裂末行、中间损坏行）
//! → 走公共入口 `services::session_usage_kimi::sync_kimi_usage`
//! （经 `KIMI_CODE_HOME` 定位 sessions 根目录，与生产同一条链路）
//! → 对账 `proxy_request_logs`：行数、四分量（总输入口径）、
//! `input_token_semantics`、`latency_ms/first_token_ms/duration_ms`、
//! `app_type/provider_id/data_source`、`request_id` 前缀、`created_at`（epoch 秒）、
//! 成本（> 0，不硬编码具体数值）→ 跑两遍断言幂等 → 追加新行断言增量游标
//! 只处理新增行。
//!
//! 取舍说明：解析层细节（多别名优先级链、时间戳秒/毫秒/ISO 混杂、全零跳过、
//! protocol_version 漂移、文件截断重扫）已由 `src/services/session_usage_kimi.rs`
//! 的 `#[cfg(test)]` 单测覆盖，这里不重复，只验证"文件系统 → 公共入口 → DB"
//! 的端到端契约。
//!
//! DB 断言经第二条 rusqlite 连接读同一个库文件（`Database.conn` 是 crate 私有，
//! 集成测试不可达）；定价用 `INSERT OR REPLACE` 种子 `kimi-for-coding`，成本断言
//! 与计价口径解耦（CACHE_INCLUSIVE_APP_TYPES 白名单并行演进中，只断言 > 0）。
//!
//! 并行安全：`KIMI_CODE_HOME` 是进程级环境变量，用例先抢 `support::test_mutex()`
//! 串行执行，`KimiCodeHomeGuard` 在用例结束（含 panic）后恢复原值——与
//! `kimi_code_config_roundtrip.rs` 的模式一致。

mod support;

use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use rusqlite::Connection;
use support::{ensure_test_home, reset_test_fs, test_mutex};

/// RAII 守卫：设置 `KIMI_CODE_HOME` 指向测试目录，drop（含 panic 路径）时恢复原值。
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

/// 测试环境：串行锁 + 隔离 HOME + KIMI_CODE_HOME 指向独立 tempdir。
/// struct 字段按声明顺序 drop：tempdir 清理 → 恢复 KIMI_CODE_HOME → 最后释放
/// 串行锁（同 `kimi_code_config_roundtrip.rs` 的环境模式）。
struct KimiIngestTestEnv {
    kimi_home: PathBuf,
    _tempdir: tempfile::TempDir,
    _home_guard: KimiCodeHomeGuard,
    _lock: MutexGuard<'static, ()>,
}

fn setup() -> KimiIngestTestEnv {
    let lock = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let tempdir = tempfile::tempdir().expect("create kimi code home tempdir");
    let kimi_home = tempdir.path().to_path_buf();
    let home_guard = KimiCodeHomeGuard::set(&kimi_home);

    KimiIngestTestEnv {
        kimi_home,
        _tempdir: tempdir,
        _home_guard: home_guard,
        _lock: lock,
    }
}

/// `Database::init()` 的库文件路径（`get_app_config_dir()/cc-switch.db`，
/// 测试环境下即 `<test-home>/.cc-switch/cc-switch.db`）。
fn test_db_path() -> PathBuf {
    ensure_test_home().join(".cc-switch").join("cc-switch.db")
}

/// 打开指向同一库文件的第二条连接，供断言/种子用（`Database.conn` 为 crate 私有）。
fn open_assert_conn() -> Connection {
    Connection::open(test_db_path()).expect("open assert connection to test db")
}

/// 种子 `kimi-for-coding` 定价（与核心单测同一份数字），使成本列非零。
/// 具体金额不参与断言，成本只断言 > 0，与计价口径解耦。
fn seed_kimi_pricing(conn: &Connection) {
    conn.execute(
        "INSERT OR REPLACE INTO model_pricing (
            model_id, display_name, input_cost_per_million, output_cost_per_million,
            cache_read_cost_per_million, cache_creation_cost_per_million
        ) VALUES ('kimi-for-coding', 'Kimi For Coding', '3', '15', '0.3', '3.75')",
        [],
    )
    .expect("seed kimi-for-coding pricing");
}

fn write_wire_raw(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create wire.jsonl parent dirs");
    }
    std::fs::write(path, content).expect("write wire.jsonl fixture");
}

/// proxy_request_logs 的单行对账投影。
struct LogRow {
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    input_token_semantics: i64,
    first_token_ms: Option<i64>,
    duration_ms: Option<i64>,
    latency_ms: i64,
    app_type: String,
    provider_id: String,
    data_source: String,
    session_id: String,
    model: String,
    is_streaming: i64,
    status_code: i64,
    created_at: i64,
    total_cost_usd: String,
}

fn query_log_row(conn: &Connection, request_id: &str) -> LogRow {
    conn.query_row(
        "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_token_semantics, first_token_ms, duration_ms, latency_ms,
                app_type, provider_id, data_source, session_id, model,
                is_streaming, status_code, created_at, total_cost_usd
         FROM proxy_request_logs WHERE request_id = ?1",
        [request_id],
        |row| {
            Ok(LogRow {
                input_tokens: row.get(0)?,
                output_tokens: row.get(1)?,
                cache_read_tokens: row.get(2)?,
                cache_creation_tokens: row.get(3)?,
                input_token_semantics: row.get(4)?,
                first_token_ms: row.get(5)?,
                duration_ms: row.get(6)?,
                latency_ms: row.get(7)?,
                app_type: row.get(8)?,
                provider_id: row.get(9)?,
                data_source: row.get(10)?,
                session_id: row.get(11)?,
                model: row.get(12)?,
                is_streaming: row.get(13)?,
                status_code: row.get(14)?,
                created_at: row.get(15)?,
                total_cost_usd: row.get(16)?,
            })
        },
    )
    .unwrap_or_else(|e| panic!("query log row {request_id}: {e}"))
}

fn kimi_session_row_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM proxy_request_logs WHERE data_source = 'kimi_session'",
        [],
        |row| row.get(0),
    )
    .expect("count kimi_session rows")
}

fn row_exists(conn: &Connection, request_id: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM proxy_request_logs WHERE request_id = ?1",
        [request_id],
        |row| row.get::<_, i64>(0),
    )
    .expect("check row existence")
        > 0
}

/// session_log_sync 游标（last_line_offset）。
fn sync_cursor(conn: &Connection, file_path: &Path) -> i64 {
    conn.query_row(
        "SELECT last_line_offset FROM session_log_sync WHERE file_path = ?1",
        [file_path.to_string_lossy().as_ref()],
        |row| row.get(0),
    )
    .expect("read sync cursor")
}

/// 成本列断言：只要求可解析为正数（与 CACHE_INCLUSIVE_APP_TYPES 计价口径解耦）。
fn assert_cost_positive(row: &LogRow, request_id: &str) {
    let cost: f64 = row
        .total_cost_usd
        .parse()
        .unwrap_or_else(|_| panic!("{request_id} total_cost_usd 不是数字: {}", row.total_cost_usd));
    assert!(
        cost > 0.0,
        "{request_id} 已种子 kimi-for-coding 定价，成本必须 > 0，实际 {}",
        row.total_cost_usd
    );
}

/// 端到端：主/子代理 + 配对 usage.record 抑制 + legacy fallback + 中间损坏行
/// + 撕裂末行 → 入库对账 → 幂等重跑 → 增量追加。
#[test]
fn wire_ingest_is_idempotent_and_incremental() {
    let env = setup();

    let sessions = env.kimi_home.join("sessions");

    // 主代理 wire.jsonl：metadata → llm.request → step.end → 损坏行 →
    // 与第 3 行 step.end 同 usage 的配对 usage.record（真实当前代格式，
    // 必须被抑制防双算）→ step.end → 撕裂末行（无结尾 \n，不得消费）。
    let main_wire = sessions.join("wd_proj/sess-1/agents/main/wire.jsonl");
    let main_content = concat!(
        "{\"type\":\"metadata\",\"protocol_version\":\"1.5\",\"created_at\":1754000000000}\n",
        "{\"type\":\"llm.request\",\"time\":1753999999000,\"model\":\"kimi-for-coding\",\"modelAlias\":\"main\"}\n",
        "{\"type\":\"context.append_loop_event\",\"time\":1754000000000,\"event\":{\"type\":\"step.end\",\"usage\":{\"inputOther\":100,\"output\":20,\"inputCacheRead\":400,\"inputCacheCreation\":50},\"llmFirstTokenLatencyMs\":320,\"llmStreamDurationMs\":1500,\"llmRequestBuildMs\":45}}\n",
        "{broken json\n",
        "{\"type\":\"usage.record\",\"time\":1754000001000,\"usageScope\":\"turn\",\"model\":\"kimi-for-coding\",\"usage\":{\"inputOther\":100,\"output\":20,\"inputCacheRead\":400,\"inputCacheCreation\":50}}\n",
        "{\"type\":\"context.append_loop_event\",\"time\":1754000002000,\"event\":{\"type\":\"step.end\",\"model\":\"kimi-for-coding\",\"usage\":{\"inputOther\":7,\"output\":3,\"inputCacheRead\":11,\"inputCacheCreation\":2},\"llmFirstTokenLatencyMs\":100,\"llmStreamDurationMs\":900,\"llmRequestBuildMs\":30}}\n",
        // 撕裂末行：合法 JSON 但无结尾 \n
        "{\"type\":\"context.append_loop_event\",\"time\":1754000003000,\"event\":{\"type\":\"step.end\",\"usage\":{\"inputOther\":1,\"output\":1,\"inputCacheRead\":0,\"inputCacheCreation\":0},\"llmFirstTokenLatencyMs\":10,\"llmStreamDurationMs\":100,\"llmRequestBuildMs\":5}}"
    );
    write_wire_raw(&main_wire, main_content);

    // 子代理 wire.jsonl：usage.record.model 为 __secondary__ 占位符，
    // 由前面的 llm.request{model, modelAlias} 前向回填真实模型名。
    // 文件内无 step.end（legacy 形态），usage.record 作 fallback 入库。
    let sub_wire = sessions.join("wd_proj/sess-1/agents/agent-2/wire.jsonl");
    let sub_content = concat!(
        "{\"type\":\"metadata\",\"protocol_version\":\"1.5\",\"created_at\":1754000000000}\n",
        "{\"type\":\"llm.request\",\"time\":1754000001200,\"model\":\"kimi-for-coding\",\"modelAlias\":\"agent-2\"}\n",
        "{\"type\":\"usage.record\",\"time\":1754000001500,\"usageScope\":\"turn\",\"model\":\"__secondary__\",\"usage\":{\"inputOther\":50,\"output\":10,\"inputCacheRead\":100,\"inputCacheCreation\":25}}\n"
    );
    write_wire_raw(&sub_wire, sub_content);

    // 独立 legacy 会话文件（全文件无 step.end）：usage.record(turn) 作
    // fallback 用量行入库，覆盖旧版本格式通路。
    let legacy_wire = sessions.join("wd_proj/sess-legacy/agents/main/wire.jsonl");
    let legacy_content = concat!(
        "{\"type\":\"metadata\",\"protocol_version\":\"1.4\",\"created_at\":1754000000000}\n",
        "{\"type\":\"usage.record\",\"time\":1754000001000,\"usageScope\":\"turn\",\"model\":\"kimi-for-coding\",\"usage\":{\"inputOther\":10,\"output\":5,\"inputCacheRead\":0,\"inputCacheCreation\":0}}\n"
    );
    write_wire_raw(&legacy_wire, legacy_content);

    // 文件 DB（init 完成 schema 迁移）+ 定价种子。
    let db = cc_switch_lib::Database::init().expect("init test database");
    seed_kimi_pricing(&open_assert_conn());

    // ===== 第一遍：全量入库 =====
    let first = cc_switch_lib::services::session_usage_kimi::sync_kimi_usage(&db)
        .expect("first sync_kimi_usage");
    assert_eq!(first.files_scanned, 3);
    assert_eq!(
        first.imported, 4,
        "主代理 2 条 step.end（配对 usage.record 被抑制）+ 子代理 1 条 + legacy 会话 1 条"
    );
    assert_eq!(
        first.errors.len(),
        1,
        "中间损坏行计入 errors 但不中断: {:?}",
        first.errors
    );
    assert!(
        first.errors[0].contains("第 4 行"),
        "错误应定位到损坏行: {}",
        first.errors[0]
    );

    let conn = open_assert_conn();
    assert_eq!(kimi_session_row_count(&conn), 4);

    // 主代理 step.end（第 3 行）：四分量 + 总输入口径 + 计时映射 + 固定列。
    let main_step = query_log_row(&conn, "kimi_code_session:sess-1:main:3");
    assert_eq!(main_step.input_tokens, 550, "总输入 = 100 + 400 + 50");
    assert_eq!(main_step.output_tokens, 20);
    assert_eq!(main_step.cache_read_tokens, 400);
    assert_eq!(main_step.cache_creation_tokens, 50);
    // INPUT_TOKEN_SEMANTICS_TOTAL（sql_helpers，pub(crate) 不可直接引用）= 总输入口径
    assert_eq!(main_step.input_token_semantics, 1);
    assert_eq!(main_step.first_token_ms, Some(320));
    assert_eq!(main_step.duration_ms, Some(1500));
    assert_eq!(main_step.latency_ms, 1545, "latency = 45 + 1500");
    assert_eq!(main_step.app_type, "kimi-code");
    assert_eq!(main_step.provider_id, "_kimi_code_session");
    assert_eq!(main_step.data_source, "kimi_session");
    assert_eq!(main_step.session_id, "sess-1");
    assert_eq!(main_step.model, "kimi-for-coding");
    assert_eq!(main_step.is_streaming, 1);
    assert_eq!(main_step.status_code, 200);
    assert_eq!(main_step.created_at, 1754000000, "time(ms) 归一为 epoch 秒");
    assert_cost_positive(&main_step, "kimi_code_session:sess-1:main:3");

    // 与第 3 行 step.end 同 usage 的配对 usage.record（第 5 行）必须被抑制：
    // 不产生额外行，行数对账不变（防 2 倍双算）。
    assert!(
        !row_exists(&conn, "kimi_code_session:sess-1:main:5"),
        "配对的 usage.record 必须被抑制，不得入库"
    );

    // legacy 会话文件的 fallback usage.record(turn)（第 2 行）：无计时字段。
    let legacy = query_log_row(&conn, "kimi_code_session:sess-legacy:main:2");
    assert_eq!(legacy.input_tokens, 10);
    assert_eq!(legacy.output_tokens, 5);
    assert_eq!(legacy.first_token_ms, None);
    assert_eq!(legacy.duration_ms, None);
    assert_eq!(legacy.latency_ms, 0);
    assert_eq!(legacy.created_at, 1754000001);

    // 主代理第二条 step.end（第 6 行，event 自带 model）。
    let main_step2 = query_log_row(&conn, "kimi_code_session:sess-1:main:6");
    assert_eq!(main_step2.input_tokens, 20, "总输入 = 7 + 11 + 2");
    assert_eq!(main_step2.latency_ms, 930, "latency = 30 + 900");
    assert_eq!(main_step2.created_at, 1754000002);

    // 子代理 __secondary__（agent-2 第 3 行）：回填真实模型名、成本不为 0。
    let sub = query_log_row(&conn, "kimi_code_session:sess-1:agent-2:3");
    assert_eq!(sub.model, "kimi-for-coding", "__secondary__ 必须回填");
    assert_eq!(sub.input_tokens, 175, "总输入 = 50 + 100 + 25");
    assert_cost_positive(&sub, "kimi_code_session:sess-1:agent-2:3");

    // 撕裂末行（第 7 行）不入库；游标停在最后完整行（6）。
    assert!(
        !row_exists(&conn, "kimi_code_session:sess-1:main:7"),
        "撕裂末行不得入库"
    );
    assert_eq!(sync_cursor(&conn, &main_wire), 6);
    drop(conn);

    // ===== 第二遍：mtime 未变，幂等跳过 =====
    let second = cc_switch_lib::services::session_usage_kimi::sync_kimi_usage(&db)
        .expect("second sync_kimi_usage");
    assert_eq!(second.imported, 0, "第二遍不得重复入库");
    let conn = open_assert_conn();
    assert_eq!(kimi_session_row_count(&conn), 4, "行数不变");
    drop(conn);

    // ===== 追加：撕裂行补全 + 一条新完整行 + 一条新撕裂行 =====
    std::thread::sleep(std::time::Duration::from_millis(10));
    let mut appended = std::fs::read_to_string(&main_wire).expect("read main wire");
    appended.push('\n'); // 补全原撕裂行（第 7 行）
    appended.push_str("{\"type\":\"context.append_loop_event\",\"time\":1754000004000,\"event\":{\"type\":\"step.end\",\"usage\":{\"inputOther\":3,\"output\":2,\"inputCacheRead\":1,\"inputCacheCreation\":1},\"llmFirstTokenLatencyMs\":20,\"llmStreamDurationMs\":200,\"llmRequestBuildMs\":10}}\n");
    // 新撕裂末行（第 9 行）：本轮仍不得消费
    appended.push_str("{\"type\":\"context.append_loop_event\",\"time\":1754000005000,\"event\":{\"type\":\"step.end\",\"usage\":{\"inputOther\":9,\"output\":9,\"inputCacheRead\":9,\"inputCacheCreation\":9}}}");
    std::fs::write(&main_wire, appended).expect("append to main wire");

    let third = cc_switch_lib::services::session_usage_kimi::sync_kimi_usage(&db)
        .expect("third sync_kimi_usage");
    assert_eq!(
        third.imported, 2,
        "增量游标只处理新增完整行（原撕裂行 + 新行），新撕裂行不消费"
    );
    let conn = open_assert_conn();
    assert_eq!(kimi_session_row_count(&conn), 6, "行数只增新增条数");
    assert_eq!(sync_cursor(&conn, &main_wire), 8);
    let appended_torn = query_log_row(&conn, "kimi_code_session:sess-1:main:7");
    assert_eq!(appended_torn.input_tokens, 1, "总输入 = 1 + 0 + 0");
    assert_eq!(appended_torn.created_at, 1754000003);
    let new_line = query_log_row(&conn, "kimi_code_session:sess-1:main:8");
    assert_eq!(new_line.input_tokens, 5, "总输入 = 3 + 1 + 1");
    assert_eq!(new_line.latency_ms, 210, "latency = 10 + 200");
    assert!(
        !row_exists(&conn, "kimi_code_session:sess-1:main:9"),
        "新撕裂末行不得入库"
    );
    drop(conn);

    // ===== 第四遍：游标已落，仍幂等 =====
    let fourth = cc_switch_lib::services::session_usage_kimi::sync_kimi_usage(&db)
        .expect("fourth sync_kimi_usage");
    assert_eq!(fourth.imported, 0);
    let conn = open_assert_conn();
    assert_eq!(kimi_session_row_count(&conn), 6);
    drop(conn);
}
