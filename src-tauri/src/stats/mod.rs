use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub voice_sessions: u64,
    pub total_voice_seconds: f64,
    pub longest_session_seconds: f64,
    pub stt_chars: u64,
    pub llm_input_tokens: u64,
    pub llm_output_tokens: u64,
    pub daily_stats: Vec<DailyStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyStats {
    pub date: String,
    pub sessions: u64,
    pub voice_seconds: f64,
    pub stt_chars: u64,
    pub llm_input_tokens: u64,
    pub llm_output_tokens: u64,
}

impl Default for UsageStats {
    fn default() -> Self {
        Self {
            voice_sessions: 0,
            total_voice_seconds: 0.0,
            longest_session_seconds: 0.0,
            stt_chars: 0,
            llm_input_tokens: 0,
            llm_output_tokens: 0,
            daily_stats: Vec::new(),
        }
    }
}

impl UsageStats {
    pub fn record_session(&mut self, duration_secs: f64) {
        self.voice_sessions += 1;
        self.total_voice_seconds += duration_secs;
        if duration_secs > self.longest_session_seconds {
            self.longest_session_seconds = duration_secs;
        }
        self.ensure_today();
        if let Some(daily) = self.daily_stats.last_mut() {
            daily.sessions += 1;
            daily.voice_seconds += duration_secs;
        }
    }

    pub fn record_stt(&mut self, chars: u64) {
        self.stt_chars += chars;
        self.ensure_today();
        if let Some(daily) = self.daily_stats.last_mut() {
            daily.stt_chars += chars;
        }
    }

    pub fn record_llm(&mut self, input_tokens: u64, output_tokens: u64) {
        self.llm_input_tokens += input_tokens;
        self.llm_output_tokens += output_tokens;
        self.ensure_today();
        if let Some(daily) = self.daily_stats.last_mut() {
            daily.llm_input_tokens += input_tokens;
            daily.llm_output_tokens += output_tokens;
        }
    }

    fn ensure_today(&mut self) {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        if self.daily_stats.last().map(|d| d.date.as_str()) != Some(&today) {
            self.daily_stats.push(DailyStats {
                date: today,
                sessions: 0,
                voice_seconds: 0.0,
                stt_chars: 0,
                llm_input_tokens: 0,
                llm_output_tokens: 0,
            });
        }
    }
}

fn write_totals(conn: &Connection, stats: &UsageStats) -> rusqlite::Result<()> {
    conn.execute(
        "REPLACE INTO stats (
            id, voice_sessions, total_voice_seconds, longest_session_seconds,
            stt_chars, llm_input_tokens, llm_output_tokens
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            stats.voice_sessions as i64,
            stats.total_voice_seconds,
            stats.longest_session_seconds,
            stats.stt_chars as i64,
            stats.llm_input_tokens as i64,
            stats.llm_output_tokens as i64,
        ],
    )?;
    Ok(())
}

fn write_daily_row(conn: &Connection, daily: &DailyStats) -> rusqlite::Result<()> {
    conn.execute(
        "REPLACE INTO stats_daily (
            date, sessions, voice_seconds, stt_chars, llm_input_tokens, llm_output_tokens
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            daily.date,
            daily.sessions as i64,
            daily.voice_seconds,
            daily.stt_chars as i64,
            daily.llm_input_tokens as i64,
            daily.llm_output_tokens as i64,
        ],
    )?;
    Ok(())
}

/// Persist the in-memory stats: the totals row plus today's bucket (only that
/// bucket ever changes while it is today; past buckets are already written).
/// Failures are logged — unlike the old file write, a partial state can no
/// longer reset everything to zero.
pub fn save_stats(conn: &Connection, stats: &UsageStats) {
    let result = (|| -> rusqlite::Result<()> {
        let tx = conn.unchecked_transaction()?;
        write_totals(&tx, stats)?;
        if let Some(today) = stats.daily_stats.last() {
            write_daily_row(&tx, today)?;
        }
        tx.commit()
    })();
    if let Err(error) = result {
        log::warn!("stats: persist failed: {error}");
    }
}

pub fn load_stats(conn: &Connection) -> UsageStats {
    let mut stats = UsageStats::default();
    match conn.query_row(
        "SELECT voice_sessions, total_voice_seconds, longest_session_seconds,
                stt_chars, llm_input_tokens, llm_output_tokens
         FROM stats WHERE id = 1",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    ) {
        Ok((sessions, total, longest, chars, input, output)) => {
            stats.voice_sessions = sessions.max(0) as u64;
            stats.total_voice_seconds = total.max(0.0);
            stats.longest_session_seconds = longest.max(0.0);
            stats.stt_chars = chars.max(0) as u64;
            stats.llm_input_tokens = input.max(0) as u64;
            stats.llm_output_tokens = output.max(0) as u64;
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {}
        Err(error) => log::warn!("stats: load failed: {error}"),
    }

    match conn.prepare(
        "SELECT date, sessions, voice_seconds, stt_chars,
                llm_input_tokens, llm_output_tokens
         FROM stats_daily ORDER BY date ASC",
    ) {
        Ok(mut stmt) => match stmt.query_map([], |row| {
            Ok(DailyStats {
                date: row.get(0)?,
                sessions: row.get::<_, i64>(1)?.max(0) as u64,
                voice_seconds: row.get::<_, f64>(2)?.max(0.0),
                stt_chars: row.get::<_, i64>(3)?.max(0) as u64,
                llm_input_tokens: row.get::<_, i64>(4)?.max(0) as u64,
                llm_output_tokens: row.get::<_, i64>(5)?.max(0) as u64,
            })
        }) {
            Ok(rows) => stats.daily_stats = rows.flatten().collect(),
            Err(error) => log::warn!("stats: daily load failed: {error}"),
        },
        Err(error) => log::warn!("stats: daily query failed: {error}"),
    }
    stats
}
