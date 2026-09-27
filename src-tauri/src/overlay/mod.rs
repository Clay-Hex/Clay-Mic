pub mod window;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextItem {
    pub id: String,
    pub raw_text: String,
    pub formatted_text: String,
    pub timestamp: DateTime<Utc>,
    pub status: ItemStatus,
    pub stt_ms: u64,
    pub llm_ms: u64,
    pub llm_ttft_ms: u64,
    pub llm_gen_ms: u64,
    pub thinking_ms: u64,
    pub reasoning_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ItemStatus {
    Processing,
    Ready,
    Injected,
    Skipped,
    Failed(String),
}

impl TextItem {
    pub fn new(raw_text: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            raw_text,
            formatted_text: String::new(),
            timestamp: Utc::now(),
            status: ItemStatus::Processing,
            stt_ms: 0,
            llm_ms: 0,
            llm_ttft_ms: 0,
            llm_gen_ms: 0,
            thinking_ms: 0,
            reasoning_text: String::new(),
        }
    }

    pub fn with_formatted(mut self, formatted: String) -> Self {
        self.formatted_text = formatted;
        self.status = ItemStatus::Ready;
        self
    }

    /// The text that belongs on screen: the LLM's formatting when there is
    /// any, otherwise the raw transcript; `None` when neither carries text.
    pub fn injectable_text(&self) -> Option<&str> {
        if !self.formatted_text.trim().is_empty() {
            Some(self.formatted_text.as_str())
        } else if !self.raw_text.trim().is_empty() {
            Some(self.raw_text.as_str())
        } else {
            None
        }
    }
}
