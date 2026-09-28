//! Model capability table, pruned from https://models.dev/api.json.
//!
//! Load order: the disk cache written by the 「刷新模型能力缓存」 button first,
//! the table embedded at build time otherwise. There is no TTL and no
//! background fetch — refreshing is manual only.
//!
//! The prune here is the single implementation: it feeds the embedded asset
//! (`npm run caps` → `--generate-caps`), the runtime refresh, and every
//! capability lookup below.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// One model's record in the pruned table.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CapsEntry {
    pub reasoning: bool,
    #[serde(default)]
    pub effort: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct CapsFile {
    #[serde(default)]
    source: String,
    #[serde(default)]
    generated: String,
    #[serde(default)]
    models: BTreeMap<String, CapsEntry>,
    #[serde(default)]
    providers: BTreeMap<String, CapsProvider>,
    /// Which source supplied the currently loaded table; not part of the file.
    #[serde(skip)]
    from_disk: bool,
}

/// Display name and API base hint for one provider.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CapsProvider {
    pub name: String,
    #[serde(default)]
    pub api: Option<String>,
}

/// One row of the provider picker.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub api: Option<String>,
}

/// Thinking-level option handed to the settings UI.
#[derive(Debug, Clone, Serialize)]
pub struct ThinkingOption {
    pub value: String,
    pub label: String,
}

/// Snapshot of the loaded table, for the settings UI.
#[derive(Debug, Clone, Serialize)]
pub struct CapsStatus {
    pub source: String,
    pub generated: String,
    pub models: usize,
    pub reasoning: usize,
    pub from_disk: bool,
}

const REFRESH_URL: &str = "https://models.dev/api.json";
const EMBEDDED: &str = include_str!("../../assets/model-caps.json");

const LEVELS: [(&str, &str); 3] = [("low", "低"), ("medium", "中"), ("high", "高")];

static TABLE: Mutex<Option<CapsFile>> = Mutex::new(None);

fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clay-mic")
}

fn disk_path() -> PathBuf {
    data_dir().join("model-caps.json")
}

fn embedded() -> CapsFile {
    serde_json::from_str(EMBEDDED).unwrap_or_else(|error| {
        log::warn!("embedded model caps failed to parse: {error}");
        CapsFile::default()
    })
}

fn load() -> CapsFile {
    match std::fs::read_to_string(disk_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<CapsFile>(&raw).ok())
    {
        Some(mut file) => {
            file.from_disk = true;
            if file.providers.is_empty() {
                // A cache written before the providers block existed would
                // otherwise shadow the embedded table and empty the picker.
                file.providers = embedded().providers;
            }
            file
        }
        None => embedded(),
    }
}

fn with_table<T>(read: impl FnOnce(&CapsFile) -> T) -> T {
    let mut guard = TABLE.lock().unwrap_or_else(|error| error.into_inner());
    read(guard.get_or_insert_with(load))
}

/// Current table: disk cache if present, embedded snapshot otherwise.
fn store(file: CapsFile) {
    let mut guard = TABLE.lock().unwrap_or_else(|error| error.into_inner());
    *guard = Some(file);
}

/// Capability lookup for one provider + model: the provider-qualified record
/// wins (effort values are served per endpoint), the bare id is a fallback.
/// Model ids in models.dev are not uniformly lowercase, so both casings are
/// tried before giving up.
pub fn lookup(provider: &str, model: &str) -> Option<CapsEntry> {
    let alias = provider_alias(provider);
    with_table(|table| {
        let find = |key: &str| table.models.get(key).cloned();
        find(&format!("{alias}/{model}"))
            .or_else(|| find(&format!("{}/{}", alias.to_lowercase(), model.to_lowercase())))
            .or_else(|| find(model))
            .or_else(|| find(&model.to_lowercase()))
    })
}

/// Preset ids in clay-mic that name a different key in models.dev.
fn provider_alias(provider: &str) -> &str {
    if provider.eq_ignore_ascii_case("claude") {
        "anthropic"
    } else {
        provider
    }
}

/// Model ids this provider offers in the catalog, or `None` when the table
/// knows nothing about it — the caller then falls back to the live
/// `/models` endpoint, which stays the source of truth for local and
/// custom endpoints (models.dev has no `ollama` entry at all, for one).
pub fn list_ids(provider: &str) -> Option<Vec<String>> {
    let prefix = provider_alias(provider);
    with_table(|table| {
        let ids: Vec<String> = table
            .models
            .keys()
            .filter(|key| key.len() > prefix.len() && key[..prefix.len()].eq_ignore_ascii_case(prefix))
            .filter(|key| key.as_bytes().get(prefix.len()) == Some(&b'/'))
            .map(|key| key[prefix.len() + 1..].to_string())
            .collect();
        if ids.is_empty() {
            None
        } else {
            Some(ids)
        }
    })
}

/// Every provider in the table, for the settings provider picker.
pub fn provider_list() -> Vec<ProviderInfo> {
    with_table(|table| {
        table
            .providers
            .iter()
            .map(|(id, meta)| ProviderInfo {
                id: id.clone(),
                name: meta.name.clone(),
                api: meta.api.clone(),
            })
            .collect()
    })
}

/// Thinking-level options for a model: `None` keeps the control disabled.
pub fn thinking_options(provider: &str, model: &str) -> Option<Vec<ThinkingOption>> {
    let known = lookup(provider, model);
    let reasoning = match known {
        Some(entry) => entry.reasoning,
        None => name_says_reasoning(model),
    };
    if !reasoning {
        return None;
    }
    Some(
        LEVELS
            .iter()
            .map(|(value, label)| ThinkingOption {
                value: (*value).to_string(),
                label: (*label).to_string(),
            })
            .collect(),
    )
}

/// Fallback for ids the table never heard of, ported from the original
/// TypeScript: `/o[1-9]/` (not preceded by a letter) plus keyword matches.
fn name_says_reasoning(model: &str) -> bool {
    let id = model.to_lowercase();
    const KEYWORDS: [&str; 9] = [
        "qwen",
        "deepseek",
        "glm",
        "kimi",
        "moonshot",
        "gpt-5",
        "gpt-oss",
        "reasoner",
        "thinking",
    ];
    if KEYWORDS.iter().any(|keyword| id.contains(keyword)) {
        return true;
    }
    let bytes = id.as_bytes();
    bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'o'
            && matches!(bytes.get(index + 1), Some(digit) if digit.is_ascii_digit() && *digit != b'0')
            && (index == 0 || !bytes[index - 1].is_ascii_alphabetic())
    })
}

fn status_of(file: &CapsFile) -> CapsStatus {
    CapsStatus {
        source: file.source.clone(),
        generated: file.generated.clone(),
        models: file.models.len(),
        reasoning: file.models.values().filter(|entry| entry.reasoning).count(),
        from_disk: file.from_disk,
    }
}

/// Status of the currently loaded table.
pub fn status() -> CapsStatus {
    with_table(status_of)
}

/// Raw records as they arrive from models.dev; every other field is dropped.
#[derive(Deserialize)]
struct RawModel {
    id: Option<String>,
    reasoning: Option<bool>,
    #[serde(default)]
    reasoning_options: Option<Vec<RawReasoningOption>>,
}

#[derive(Deserialize)]
struct RawReasoningOption {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    values: Option<Vec<Option<String>>>,
}

#[derive(Deserialize)]
struct RawProvider {
    id: Option<String>,
    name: Option<String>,
    api: Option<String>,
    #[serde(default)]
    models: Option<HashMap<String, RawModel>>,
}

fn prune(raw: &str) -> Result<CapsFile, String> {
    let catalog: HashMap<String, RawProvider> =
        serde_json::from_str(raw).map_err(|error| format!("解析 models.dev 数据失败：{error}"))?;

    let mut models: BTreeMap<String, CapsEntry> = BTreeMap::new();
    let mut providers: BTreeMap<String, CapsProvider> = BTreeMap::new();
    for (provider_key, provider) in &catalog {
        let provider_id = provider
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(provider_key);
        providers.insert(
            provider_id.to_string(),
            CapsProvider {
                name: provider
                    .name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(provider_id)
                    .to_string(),
                api: provider.api.clone(),
            },
        );
        for (model_key, raw_model) in provider.models.iter().flatten() {
            let id = raw_model
                .id
                .as_deref()
                .filter(|id| !id.is_empty())
                .unwrap_or(model_key);
            let effort = raw_model
                .reasoning_options
                .as_ref()
                .and_then(|options| options.iter().find(|option| option.kind.as_deref() == Some("effort")))
                .and_then(|option| option.values.as_ref())
                .map(|values| values.iter().flatten().cloned().collect::<Vec<String>>())
                .filter(|values| !values.is_empty());
            let entry = CapsEntry {
                reasoning: raw_model.reasoning.unwrap_or(false),
                effort,
            };

            models.insert(format!("{}/{}", provider_id, id), entry.clone());

            // The bare id is only a fallback for providers models.dev does not
            // know; keep the richest record across providers.
            match models.get(id) {
                None => {
                    models.insert(id.to_string(), entry);
                }
                Some(existing) => {
                    let richer = (!existing.reasoning && entry.reasoning)
                        || (existing.reasoning == entry.reasoning
                            && existing.effort.is_none()
                            && entry.effort.is_some());
                    if richer {
                        models.insert(id.to_string(), entry);
                    }
                }
            }
        }
    }

    Ok(CapsFile {
        source: REFRESH_URL.to_string(),
        generated: chrono::Utc::now().format("%Y-%m-%d").to_string(),
        models,
        providers,
        from_disk: false,
    })
}

/// Write through a temp file and rename, so a concurrent reader (or a failed
/// write) never observes a half-written table.
fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
    }
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&temp, contents).map_err(|error| format!("写入临时文件失败：{error}"))?;
    std::fs::rename(&temp, path).map_err(|error| format!("替换文件失败：{error}"))
}

/// Shared pool for the models.dev fetch: a fresh client per refresh would
/// pay the TLS handshake every time.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

async fn fetch_raw() -> Result<String, String> {
    let response = client()
        .get(REFRESH_URL)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|error| format!("拉取 {REFRESH_URL} 失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("拉取 {REFRESH_URL} 失败：{error}"))?;

    // A decode failure only says "unreadable"; these headers say whether the
    // body arrived compressed or as something other than JSON.
    let head = response
        .headers()
        .iter()
        .filter(|(name, _)| matches!(name.as_str(), "content-type" | "content-encoding"))
        .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap_or("?")))
        .collect::<Vec<_>>()
        .join(", ");

    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取响应失败：{error}（{head}）"))?;
    String::from_utf8(bytes.to_vec()).map_err(|error| {
        let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(80)]).replace('\n', " ");
        format!(
            "读取响应失败：{error}（{head}，{} 字节，开头：{}）",
            bytes.len(),
            prefix
        )
    })
}

/// Download models.dev, prune it, and write the disk cache the app loads from
/// on startup. Manual only — nothing calls this automatically.
pub async fn refresh() -> Result<CapsStatus, String> {
    let file = prune(&fetch_raw().await?)?;
    let serialized = format!("{}\n", serde_json::to_string(&file).map_err(|error| error.to_string())?);
    write_atomic(&disk_path(), &serialized)?;
    let mut file = file;
    file.from_disk = true;
    let status = status_of(&file);
    store(file);
    Ok(status)
}

/// Regenerate `assets/model-caps.json` for the build to embed
/// (`npm run caps`). Shares the fetch and prune with [`refresh`].
pub fn generate_assets() -> Result<CapsStatus, String> {
    let runtime =
        tokio::runtime::Runtime::new().map_err(|error| format!("创建运行时失败：{error}"))?;
    runtime.block_on(async {
        let file = prune(&fetch_raw().await?)?;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/model-caps.json");
        let serialized =
            format!("{}\n", serde_json::to_string(&file).map_err(|error| error.to_string())?);
        write_atomic(&path, &serialized)?;
        Ok(status_of(&file))
    })
}
