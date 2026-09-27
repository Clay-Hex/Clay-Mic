//! Persistent text-list storage backed by SQLite.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::overlay::{ItemStatus, TextItem};

/// `<data_local>/clay-mic/clay-mic.db`.
pub fn path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clay-mic")
        .join("clay-mic.db")
}

/// Open (creating if needed) the database and ensure the schema exists.
pub fn open() -> Result<Connection, rusqlite::Error> {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    create_schema(&conn)?;
    Ok(conn)
}

fn create_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS items (
            id TEXT PRIMARY KEY,
            raw_text TEXT NOT NULL,
            formatted_text TEXT NOT NULL,
            timestamp_ms INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            stt_ms INTEGER NOT NULL DEFAULT 0,
            llm_ms INTEGER NOT NULL DEFAULT 0,
            llm_ttft_ms INTEGER NOT NULL DEFAULT 0,
            llm_gen_ms INTEGER NOT NULL DEFAULT 0,
            thinking_ms INTEGER NOT NULL DEFAULT 0,
            reasoning_text TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS stats (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            voice_sessions INTEGER NOT NULL DEFAULT 0,
            total_voice_seconds REAL NOT NULL DEFAULT 0,
            longest_session_seconds REAL NOT NULL DEFAULT 0,
            stt_chars INTEGER NOT NULL DEFAULT 0,
            llm_input_tokens INTEGER NOT NULL DEFAULT 0,
            llm_output_tokens INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS stats_daily (
            date TEXT PRIMARY KEY,
            sessions INTEGER NOT NULL DEFAULT 0,
            voice_seconds REAL NOT NULL DEFAULT 0,
            stt_chars INTEGER NOT NULL DEFAULT 0,
            llm_input_tokens INTEGER NOT NULL DEFAULT 0,
            llm_output_tokens INTEGER NOT NULL DEFAULT 0
        );",
    )
}

fn map_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<TextItem> {
    let status: String = row.get(4)?;
    Ok(TextItem {
        id: row.get(0)?,
        raw_text: row.get(1)?,
        formatted_text: row.get(2)?,
        timestamp: DateTime::<Utc>::from_timestamp_millis(row.get::<_, i64>(3)?)
            .unwrap_or_else(Utc::now),
        status: serde_json::from_str(&status).unwrap_or(ItemStatus::Ready),
        stt_ms: row.get::<_, i64>(5)?.max(0) as u64,
        llm_ms: row.get::<_, i64>(6)?.max(0) as u64,
        llm_ttft_ms: row.get::<_, i64>(7)?.max(0) as u64,
        llm_gen_ms: row.get::<_, i64>(8)?.max(0) as u64,
        thinking_ms: row.get::<_, i64>(9)?.max(0) as u64,
        reasoning_text: row.get(10)?,
    })
}

fn count_items(conn: &Connection) -> u64 {
    conn.query_row("SELECT COUNT(*) FROM items", [], |row| row.get::<_, i64>(0))
        .map(|count| count.max(0) as u64)
        .unwrap_or_else(|error| {
            log::warn!("db: count items failed: {error}");
            0
        })
}

/// One page of history, newest first. `page` is 1-based and is clamped to the
/// real last page, so a shrinking list can never return an empty page.
pub fn load_items_page(conn: &Connection, page: u32, size: u32) -> (Vec<TextItem>, u64, u32) {
    let total = count_items(conn);
    let size = size.max(1) as u64;
    let max_page = ((total + size - 1) / size).max(1) as u32;
    let page = page.clamp(1, max_page);
    let offset = ((page - 1) as u64 * size) as i64;

    let sql = "SELECT id, raw_text, formatted_text, timestamp_ms, status,
                      stt_ms, llm_ms, llm_ttft_ms, llm_gen_ms, thinking_ms, reasoning_text
               FROM items ORDER BY timestamp_ms DESC LIMIT ?1 OFFSET ?2";
    let mut items = Vec::new();
    let Ok(mut stmt) = conn.prepare(sql) else {
        log::warn!("db: failed to prepare page query");
        return (items, total, page);
    };
    match stmt.query_map(params![size as i64, offset], map_item) {
        Ok(rows) => items.extend(rows.flatten()),
        Err(error) => log::warn!("db: page query failed: {error}"),
    }
    (items, total, page)
}

/// One stored item by id.
pub fn get_item(conn: &Connection, id: &str) -> Option<TextItem> {
    let sql = "SELECT id, raw_text, formatted_text, timestamp_ms, status,
                      stt_ms, llm_ms, llm_ttft_ms, llm_gen_ms, thinking_ms, reasoning_text
               FROM items WHERE id = ?1";
    match conn.query_row(sql, params![id], map_item) {
        Ok(item) => Some(item),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(error) => {
            log::warn!("db: get item {id} failed: {error}");
            None
        }
    }
}

/// Insert or replace one item.
pub fn save_item(conn: &Connection, item: &TextItem) {
    let status = serde_json::to_string(&item.status).unwrap_or_else(|_| "\"Ready\"".to_string());
    let result = conn.execute(
        "INSERT INTO items (
            id, raw_text, formatted_text, timestamp_ms, status,
            stt_ms, llm_ms, llm_ttft_ms, llm_gen_ms, thinking_ms, reasoning_text
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(id) DO UPDATE SET
            raw_text = excluded.raw_text,
            formatted_text = excluded.formatted_text,
            timestamp_ms = excluded.timestamp_ms,
            status = excluded.status,
            stt_ms = excluded.stt_ms,
            llm_ms = excluded.llm_ms,
            llm_ttft_ms = excluded.llm_ttft_ms,
            llm_gen_ms = excluded.llm_gen_ms,
            thinking_ms = excluded.thinking_ms,
            reasoning_text = excluded.reasoning_text",
        params![
            item.id,
            item.raw_text,
            item.formatted_text,
            item.timestamp.timestamp_millis(),
            status,
            item.stt_ms as i64,
            item.llm_ms as i64,
            item.llm_ttft_ms as i64,
            item.llm_gen_ms as i64,
            item.thinking_ms as i64,
            item.reasoning_text,
        ],
    );
    if let Err(error) = result {
        log::warn!("db: save item {} failed: {error}", item.id);
    }
}

/// Remove one item.
pub fn delete_item(conn: &Connection, id: &str) {
    if let Err(error) = conn.execute("DELETE FROM items WHERE id = ?1", params![id]) {
        log::warn!("db: delete item {id} failed: {error}");
    }
}

/// Remove all items.
pub fn clear_items(conn: &Connection) {
    if let Err(error) = conn.execute("DELETE FROM items", []) {
        log::warn!("db: clear items failed: {error}");
    }
}
