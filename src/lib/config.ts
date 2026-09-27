export interface Config {
  device: {
    name: string;
    protocol: string;
    sample_rate: number;
    header_size: number;
    device_id: string | null;
    vendor_id: number | null;
    product_id: number | null;
    model: string | null;
  };
  llm: {
    enabled: boolean;
    provider: string;
    api_key: string;
    base_url: string;
    model: string;
    format_prompt: string;
    reasoning: string;
    api_keys: Record<string, string>;
    models: Record<string, string>;
    timeout_secs: number;
    max_tokens: number;
  };
  stt: {
    model: string;
    language: string;
    binary_path?: string | null;
    model_path?: string | null;
    runtime: string;
    prompt: string;
    streaming: boolean;
    min_audio_ms: number;
  };
  overlay: {
    hotkey: string;
    position: string;
    max_items: number;
    style: string;
    width: number;
    height: number;
  };
  indicator: { style: string };
  window: { width: number; height: number; close_to_tray: boolean };
  inject: { method: string };
  language: string;
}

/**
 * Mirrors the Rust `AppConfig::default()` in
 * `src-tauri/src/config/mod.rs`. Keep the two in sync — `update_config`
 * sends this whole object and Rust deserializes all fields.
 */
export const defaultConfig: Config = {
  device: {
    name: "",
    protocol: "atvv-1.0",
    sample_rate: 16000,
    header_size: 2,
    device_id: null,
    vendor_id: null,
    product_id: null,
    model: null,
  },
  llm: {
    enabled: true,
    provider: "openai",
    api_key: "",
    base_url: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    format_prompt:
      "将以下语音转写文本格式化为清晰的 Markdown。保持原意，修正可能的听写错误，添加适当的结构（标题、列表、代码块等）。只输出格式化后的 Markdown，不要解释。",
    reasoning: "",
    api_keys: {},
    models: {},
    timeout_secs: 120,
    max_tokens: 8192,
  },
  stt: { model: "base", language: "auto", binary_path: null, model_path: null, runtime: "cpu", prompt: "", streaming: false, min_audio_ms: 200 },
  overlay: {
    hotkey: "Alt+,",
    position: "cursor",
    max_items: 20,
    style: "aurora",
    width: 420,
    height: 560,
  },
  indicator: { style: "remote-wave" },
  window: { width: 1040, height: 760, close_to_tray: true },
  inject: { method: "clipboard" },
  language: "zh-CN",
};

/** A processed transcription, mirroring Rust `overlay::TextItem`. */
export interface TextItem {
  id: string;
  raw_text: string;
  formatted_text: string;
  /** ISO-8601 timestamp. */
  timestamp: string;
  /**
   * Rust `ItemStatus` is externally tagged, so `Failed(String)` arrives as
   * `{ Failed: "..." }` while the others are plain strings.
   */
  status: "Processing" | "Ready" | "Injected" | "Skipped" | { Failed: string };
  stt_ms: number;
  llm_ms: number;
  llm_ttft_ms: number;
  llm_gen_ms: number;
  thinking_ms: number;
  reasoning_text: string;
}

/** Voice pipeline phase, mirroring the `voice://state` event payload. */
export type VoicePhase =
  | "idle"
  | "recording"
  | "transcribing"
  | "formatting"
  | "error";

export interface VoiceStatePayload {
  state: VoicePhase;
  message?: string;
}

/** Readiness of the local whisper.cpp backend (`stt_status`). */
export interface SttStatus {
  runtime: string;
  runtime_ready: boolean;
  installed_runtimes: string[];
  binary_ready: boolean;
  binary_path: string | null;
  model_ready: boolean;
  model_path: string;
  warm: boolean;
}

/** Interception driver state (`get_interception_status`). */
export interface DriverStatus {
  dll_found: boolean;
  dll_path: string | null;
  driver_ready: boolean;
  installer_found: boolean;
  install_dir: string;
}

export interface FilterChainStatus {
  keyboard: string[];
  mouse: string[];
  interception_installed: boolean;
}

/** Keyboard/mouse device-numbering service state (`get_keyslot_status`). */
export interface KeySlotStatus {
  service_installed: boolean;
  helper_found: boolean;
  helper_path: string | null;
  applied_unix: number | null;
  keyboard: number | null;
  pointer: number | null;
  ok: boolean | null;
  error: string | null;
}

/** Download progress event payload. */
export interface DownloadProgress {
  source: "binary" | "model" | "interception";
  phase: "downloading" | "extracting" | "done";
  percent: number;
}

/** HID Tap component + injection state (`get_tap_status`). */
export interface TapStatus {
  version: string;
  dll_ready: boolean;
  host_pid: number | null;
  host_alive: boolean;
  injected: boolean;
  listening: boolean;
  client_connected: boolean;
  last_error: string | null;
  device_identity: string | null;
  lookup: string | null;
  /** Version reported by the resident hook's heartbeat. */
  dll_version: string | null;
  /** Resident hook differs from this build (unversioned hook counts). */
  dll_needs_update: boolean;
}

/** Thinking-level option returned by `get_thinking_options`. */
export interface ThinkingOption {
  value: string;
  label: string;
}

/** Loaded model-capability table (`get_caps_status` / `refresh_model_caps`). */
export interface CapsStatus {
  source: string;
  generated: string;
  models: number;
  reasoning: number;
  from_disk: boolean;
}

/** One page of history (`get_text_list`); `page` is 1-based. */
export interface TextListPage {
  items: TextItem[];
  total: number;
  page: number;
  size: number;
}

/** Provider row from the capability table (`get_caps_providers`). */
export interface CapsProvider {
  id: string;
  name: string;
  api: string | null;
}

/** Usage/character statistics (`get_stats` and the `stats://updated` event). */
export interface UsageStats {
  voice_sessions: number;
  total_voice_seconds: number;
  longest_session_seconds: number;
  stt_chars: number;
  llm_input_tokens: number;
  llm_output_tokens: number;
}

interface TauriGlobal {
  core: {
    invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  };
  event?: {
    listen: (
      event: string,
      handler: (event: { payload: unknown }) => void,
    ) => Promise<() => void>;
  };
}

function tauriApi(): TauriGlobal | undefined {
  return (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
}

/**
 * Thin typed wrapper over the Tauri invoke bridge. Rejects with a helpful
 * message when the app is opened in a plain browser (no Tauri runtime).
 */
export function tauriInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const api = tauriApi();
  if (!api) {
    return Promise.reject(new Error("请使用 npm run tauri dev 启动"));
  }
  return api.core.invoke(cmd, args) as Promise<T>;
}

/**
 * Subscribe to a Tauri event. Returns an unsubscribe function. When there is
 * no Tauri runtime (plain browser) it resolves to a no-op unsubscribe.
 */
export async function tauriListen<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  const api = tauriApi();
  if (!api?.event) {
    return () => {};
  }
  return api.event.listen(event, (e) => handler(e.payload as T));
}

export type ItemTone = "busy" | "ok" | "muted" | "fail";

export interface ItemState {
  label: string;
  tone: ItemTone;
  error?: string;
}

/** Display state for a `TextItem`, including the current pipeline phase. */
export function itemStatus(item: TextItem): ItemState {
  const status = item.status;
  if (typeof status !== "string") {
    return { label: "失败", tone: "fail", error: status.Failed || "处理失败" };
  }
  switch (status) {
    case "Ready":
      return { label: "已完成", tone: "ok" };
    case "Injected":
      return { label: "已注入", tone: "ok" };
    case "Skipped":
      return { label: "未配置 LLM", tone: "muted" };
    case "Processing":
    default:
      return {
        label: item.raw_text.trim() ? "LLM 改写中…" : "转录中…",
        tone: "busy",
      };
  }
}
