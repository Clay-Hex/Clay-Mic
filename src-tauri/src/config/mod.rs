use crate::ble::device::DeviceProfile;
use crate::llm::provider::LlmConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub device: DeviceProfile,
    pub llm: LlmConfig,
    pub stt: SttConfig,
    pub overlay: OverlayConfig,
    #[serde(default)]
    pub indicator: IndicatorConfig,
    #[serde(default)]
    pub window: MainWindowConfig,
    pub inject: InjectConfig,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainWindowConfig {
    #[serde(default = "default_main_width")]
    pub width: u32,
    #[serde(default = "default_main_height")]
    pub height: u32,
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
}

impl Default for MainWindowConfig {
    fn default() -> Self {
        Self {
            width: default_main_width(),
            height: default_main_height(),
            close_to_tray: default_close_to_tray(),
        }
    }
}

fn default_main_width() -> u32 {
    1040
}

fn default_main_height() -> u32 {
    760
}

fn default_close_to_tray() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    pub model: String,
    pub language: String,
    /// Explicit path to `whisper-cli.exe`. `None` = auto-detect.
    #[serde(default)]
    pub binary_path: Option<String>,
    /// Explicit path to the `.bin` model. `None` = `<data>/clay-mic/whisper/models/ggml-<model>.bin`.
    #[serde(default)]
    pub model_path: Option<String>,
    /// Installed whisper.cpp runtime: `cpu`, `cuda12` or `cuda11`.
    #[serde(default = "default_stt_runtime")]
    pub runtime: String,
    #[serde(default)]
    pub prompt: String,
    /// Show partial transcription while recording (sliding-window preview).
    #[serde(default)]
    pub streaming: bool,
    /// Drop recordings shorter than this before transcribing. `0` disables the
    /// filter; the default mirrors `default_min_audio_ms`.
    #[serde(default = "default_min_audio_ms")]
    pub min_audio_ms: u32,
}

fn default_min_audio_ms() -> u32 {
    200
}

fn default_stt_runtime() -> String {
    "cpu".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayConfig {
    pub hotkey: String,
    pub position: String,
    pub max_items: usize,
    #[serde(default = "default_overlay_style")]
    pub style: String,
    #[serde(default = "default_overlay_width")]
    pub width: u32,
    #[serde(default = "default_overlay_height")]
    pub height: u32,
}

fn default_overlay_width() -> u32 {
    420
}

fn default_overlay_height() -> u32 {
    560
}

fn default_overlay_style() -> String {
    "aurora".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorConfig {
    pub style: String,
}

impl Default for IndicatorConfig {
    fn default() -> Self {
        Self {
            style: "remote-wave".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectConfig {
    pub method: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            device: DeviceProfile::default(),
            llm: LlmConfig::default(),
            stt: SttConfig {
                model: "base".to_string(),
                language: "auto".to_string(),
                binary_path: None,
                model_path: None,
                runtime: default_stt_runtime(),
                prompt: String::new(),
                streaming: false,
                min_audio_ms: default_min_audio_ms(),
            },
            overlay: OverlayConfig {
                hotkey: "Alt+,".to_string(),
                position: "cursor".to_string(),
                max_items: 20,
                style: "aurora".to_string(),
                width: default_overlay_width(),
                height: default_overlay_height(),
            },
            indicator: IndicatorConfig::default(),
            window: MainWindowConfig::default(),
            inject: InjectConfig {
                method: "clipboard".to_string(),
            },
            language: "zh-CN".to_string(),
        }
    }
}

impl AppConfig {
    /// Location of the persisted config file:
    /// `%LOCALAPPDATA%/clay-mic/config.json` on Windows.
    pub fn config_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clay-mic")
            .join("config.json")
    }

    /// Load config from disk. Falls back to defaults when the file is
    /// missing or cannot be parsed (never fails).
    pub fn load() -> Self {
        let path = Self::config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<AppConfig>(&text) {
                Ok(config) => {
                    log::info!("Loaded config from {:?}", path);
                    config
                }
                Err(e) => {
                    log::warn!("Failed to parse config {:?}: {}; using defaults", path, e);
                    Self::default()
                }
            },
            Err(_) => {
                log::info!("No config file at {:?}; using defaults", path);
                Self::default()
            }
        }
    }

    /// Persist config to disk as pretty JSON. Writing the same content again
    /// is skipped, so callers may save freely.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize config failed: {}", e))?;
        if std::fs::read_to_string(&path).is_ok_and(|current| current == text) {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Create config dir failed: {}", e))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("Write config failed: {}", e))?;
        log::info!("Saved config to {:?}", path);
        Ok(())
    }
}
