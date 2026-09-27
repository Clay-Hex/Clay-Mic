//! Process-wide runtime state shared by commands and background workers.
//!
//! The Tauri app handle is stored once at startup so workers (voice capture,
//! STT, LLM) can emit events without threading the handle through every call.
//! `AppState` (config + text list) lives here too so both command handlers and
//! background workers mutate the same data.

use crate::config::AppConfig;
use crate::overlay::TextItem;
use rusqlite::Connection;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager};

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Store the app handle for the process lifetime. Safe to call once.
pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

/// The global app handle, if the app has started.
pub fn handle() -> Option<&'static AppHandle> {
    APP.get()
}

/// Emit a Tauri event to all windows. No-op before startup.
pub fn emit<S: serde::Serialize + Clone>(event: &str, payload: S) {
    if let Some(app) = APP.get() {
        let _ = app.emit(event, payload);
    }
}

/// Shared application state: settings plus the SQLite handle that holds the
/// text history and statistics. The history itself is read page by page from
/// the database instead of being kept whole in memory.
pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub stats: Mutex<crate::stats::UsageStats>,
    db: Mutex<Option<Connection>>,
}

impl AppState {
    pub fn new() -> Self {
        crate::stt::migrate_flat_install();
        let db = match crate::db::open() {
            Ok(conn) => Some(conn),
            Err(error) => {
                log::warn!("db: open failed ({error}); history will not persist");
                None
            }
        };
        let stats = db
            .as_ref()
            .map(crate::stats::load_stats)
            .unwrap_or_default();
        Self {
            config: Mutex::new(AppConfig::load()),
            stats: Mutex::new(stats),
            db: Mutex::new(db),
        }
    }

    fn persist(&self, item: &TextItem) {
        if let Ok(guard) = self.db.lock() {
            if let Some(conn) = guard.as_ref() {
                crate::db::save_item(conn, item);
            }
        }
    }

    /// Write the current stats to the database, using the same connection as
    /// the history so both stay in one file, then push the new values to the
    /// UI. Every mutation path ends here, so a `stats://updated` listener can
    /// never miss an update.
    pub fn persist_stats(&self, stats: &crate::stats::UsageStats) {
        if let Ok(guard) = self.db.lock() {
            if let Some(conn) = guard.as_ref() {
                crate::stats::save_stats(conn, stats);
            }
        }
        emit("stats://updated", stats.clone());
    }

    /// One page of history (newest first) plus the total and the clamped
    /// page actually served; an empty result when history is unavailable.
    pub fn items_page(&self, page: u32, size: u32) -> (Vec<TextItem>, u64, u32) {
        if let Ok(guard) = self.db.lock() {
            if let Some(conn) = guard.as_ref() {
                return crate::db::load_items_page(conn, page, size);
            }
        }
        (Vec::new(), 0, page)
    }

    /// One stored item by id.
    pub fn item(&self, id: &str) -> Option<TextItem> {
        let Ok(guard) = self.db.lock() else {
            return None;
        };
        guard.as_ref().and_then(|conn| crate::db::get_item(conn, id))
    }

    /// Persist a new text item.
    pub fn push_item(&self, item: TextItem) {
        self.persist(&item);
    }

    /// Persist an updated item.
    pub fn update_item(&self, item: TextItem) {
        self.persist(&item);
    }

    /// Remove one item from the database.
    pub fn remove_item(&self, id: &str) {
        if let Ok(guard) = self.db.lock() {
            if let Some(conn) = guard.as_ref() {
                crate::db::delete_item(conn, id);
            }
        }
    }

    /// Remove all items from the database.
    pub fn clear_items(&self) {
        if let Ok(guard) = self.db.lock() {
            if let Some(conn) = guard.as_ref() {
                crate::db::clear_items(conn);
            }
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience accessor for the managed `AppState` from anywhere with a handle.
pub fn state() -> Option<tauri::State<'static, AppState>> {
    APP.get().map(|app| app.state::<AppState>())
}
