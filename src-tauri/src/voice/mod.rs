//! ATVV voice session listener.
//!
//! The voice key is driven by the remote, not by us. The control sequence is:
//!
//! ```text
//! remote control 0x08 (mic request) → host writes MIC_OPEN
//! remote control 0x04 (stream start) → audio notifications on the audio char
//! remote control 0x00 (stream stop)  → transcribe + format + store
//! ```
//!
//! Audio notifications may be partial frames, so they are accumulated to the
//! capabilities frame size before ADPCM decoding.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::audio::pipeline::AudioPipeline;
use crate::config::AppConfig;
use crate::llm::provider::LlmDelta;
use crate::overlay::{ItemStatus, TextItem};
use crate::protocol::atvv::{
    self, AtvvCapabilities, OP_AUDIO_SYNC, OP_REMOTE_MIC_REQUEST, OP_STREAM_START, OP_STREAM_STOP,
};

/// Hard cap so a wedged stream can never record forever.
const MAX_SESSION: Duration = Duration::from_secs(120);

static RECORDING: AtomicBool = AtomicBool::new(false);
static FINALIZING: AtomicUsize = AtomicUsize::new(0);
/// Serializes partial (streaming-preview) transcriptions.
static PARTIAL_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
/// Bumped per session so results from an ended session are dropped.
static PARTIAL_EPOCH: AtomicU64 = AtomicU64::new(0);
/// How often the accumulated audio is re-transcribed for the live preview.
const PARTIAL_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Serialize)]
struct PartialPayload {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct VoiceState {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn emit_state(state: &'static str, message: Option<String>) {
    // A finishing utterance must not override the state of one that is still
    // recording — the indicator should keep showing "recording".
    if state != "recording" && RECORDING.load(Ordering::SeqCst) {
        return;
    }
    crate::runtime::emit("voice://state", VoiceState { state, message });
    if let Some(app) = crate::runtime::handle() {
        crate::overlay::window::set_indicator_visible(app, !matches!(state, "idle" | "error"));
    }
}

/// Spawn the long-lived listener that owns the ATVV audio/control channels.
pub fn start_session(
    audio: Receiver<Vec<u8>>,
    control: Receiver<Vec<u8>>,
    caps: AtvvCapabilities,
) {
    let spawned = std::thread::Builder::new()
        .name("clay-mic voice session".into())
        .spawn(move || run_listener(audio, control, caps));
    if let Err(error) = spawned {
        log::error!("failed to start voice listener: {error}");
    }
}

struct ActiveSession {
    pipeline: AudioPipeline,
    stream_id: u8,
    sample_rate: u32,
    streaming: bool,
    started_at: Instant,
}

fn run_listener(audio: Receiver<Vec<u8>>, control: Receiver<Vec<u8>>, caps: AtvvCapabilities) {
    let mut session: Option<ActiveSession> = None;
    let mut last_stop: Option<Instant> = None;
    let mut next_partial = Instant::now();

    loop {
        // --- control channel: drives the whole session lifecycle ---
        while let Ok(message) = control.try_recv() {
            let Some(opcode) = message.first().copied() else {
                continue;
            };
            // With suppression off the remote's keys act natively, so the voice
            // key must not record either.
            if !suppression_active() {
                continue;
            }
            match opcode {
                OP_REMOTE_MIC_REQUEST => {
                    log::info!("atvv: remote mic request -> MIC_OPEN");
                    // A new mic session may again race audio ahead of STREAM_START.
                    last_stop = None;
                    if let Err(error) = crate::ble::scanner::send_mic_open() {
                        log::warn!("atvv: MIC_OPEN failed: {error}");
                    }
                }
                OP_STREAM_START => {
                    let codec = message.get(2).copied().unwrap_or(atvv::CODEC_ADPCM_16K);
                    let stream_id = message.get(3).copied().unwrap_or(0);
                    let rate = match codec {
                        atvv::CODEC_ADPCM_16K => 16_000,
                        atvv::CODEC_ADPCM_8K => 8_000,
                        _ => caps.sample_rate,
                    };
                    log::info!(
                        "atvv: stream start codec={codec:#04x} rate={rate} stream={stream_id}"
                    );
                    session = Some(ActiveSession {
                        pipeline: AudioPipeline::new(rate, caps.frame_size),
                        stream_id,
                        sample_rate: rate,
                        streaming: streaming_enabled(),
                        started_at: Instant::now(),
                    });
                    RECORDING.store(true, Ordering::SeqCst);
                    PARTIAL_IN_FLIGHT.store(false, Ordering::SeqCst);
                    PARTIAL_EPOCH.fetch_add(1, Ordering::SeqCst);
                    emit_partial(String::new());
                    emit_state("recording", None);
                }
                OP_STREAM_STOP => {
                    log::info!("atvv: stream stop");
                    if let Some(active) = session.take() {
                        last_stop = Some(Instant::now());
                        RECORDING.store(false, Ordering::SeqCst);
                        PARTIAL_EPOCH.fetch_add(1, Ordering::SeqCst);
                        emit_partial(String::new());
                        // Transcribe off-thread so the listener keeps draining
                        // the channels for the next utterance.
                        std::thread::spawn(move || finalize(active));
                    }
                }
                OP_AUDIO_SYNC => {
                    if let Some(active) = session.as_mut() {
                        if message.len() >= 7 {
                            let predictor = i16::from_be_bytes([message[4], message[5]]);
                            let step = message[6];
                            active.pipeline.set_sync(predictor, step);
                            log::info!("atvv: audio sync predictor={predictor} step={step}");
                        }
                    }
                }
                other => log::info!("atvv: control opcode {other:#04x} len={}", message.len()),
            }
        }

        // --- audio channel ---
        match audio.recv_timeout(Duration::from_millis(50)) {
            Ok(frame) => {
                if session.is_none() {
                    // Audio can race ahead of the STREAM_START control message,
                    // but only at the start of a mic session. After a stream
                    // stop the mic is still open and the remote may keep
                    // notifying (ambient audio or a late flush); that audio
                    // belongs to no utterance — only an explicit STREAM_START
                    // may begin the next one.
                    if last_stop.is_none() && suppression_active() {
                        log::info!("atvv: implicit stream start from audio");
                        session = Some(ActiveSession {
                            pipeline: AudioPipeline::new(caps.sample_rate, caps.frame_size),
                            stream_id: 0,
                            sample_rate: caps.sample_rate,
                            streaming: streaming_enabled(),
                            started_at: Instant::now(),
                        });
                        RECORDING.store(true, Ordering::SeqCst);
                        PARTIAL_IN_FLIGHT.store(false, Ordering::SeqCst);
                        PARTIAL_EPOCH.fetch_add(1, Ordering::SeqCst);
                        emit_partial(String::new());
                        emit_state("recording", None);
                    } else {
                        log::debug!("atvv: dropping audio outside an active stream");
                    }
                }
                if let Some(active) = session.as_mut() {
                    active.pipeline.feed(&frame);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                log::warn!("atvv: audio channel closed; voice listener exiting");
                return;
            }
        }

        // --- streaming preview ---
        if let Some(active) = session.as_ref() {
            if active.streaming && Instant::now() >= next_partial {
                next_partial = Instant::now() + PARTIAL_INTERVAL;
                spawn_partial(active);
            }
        }

        // --- safety cap ---
        if session
            .as_ref()
            .map(|active| active.started_at.elapsed() >= MAX_SESSION)
            .unwrap_or(false)
        {
            log::warn!("atvv: session exceeded {MAX_SESSION:?}; finalizing");
            if let Some(active) = session.take() {
                RECORDING.store(false, Ordering::SeqCst);
                // Transcribe off-thread so the listener keeps draining the
                // channels while the capped session is processed.
                std::thread::spawn(move || finalize(active));
            }
        }
    }
}

fn streaming_enabled() -> bool {
    crate::runtime::state()
        .map(|state| state.config.lock().unwrap().stt.streaming)
        .unwrap_or(false)
}

/// Whether the remote's keys are currently being suppressed by the keymap.
fn suppression_active() -> bool {
    crate::keymap::snapshot().suppress
}

/// Whether `session` is shorter than the configured minimum clip length.
///
/// A threshold of `0` disables the filter.
fn below_min_audio_ms(session: &ActiveSession) -> bool {
    let threshold = crate::runtime::state()
        .map(|state| state.config.lock().unwrap().stt.min_audio_ms)
        .unwrap_or(0);
    if threshold == 0 {
        return false;
    }
    let elapsed_ms = session.pipeline.sample_count() as u64 * 1000 / session.sample_rate as u64;
    elapsed_ms < threshold as u64
}

fn emit_partial(text: String) {
    crate::runtime::emit("voice://partial", PartialPayload { text });
}

/// Re-transcribe the audio accumulated so far for the live preview. Skipped
/// while a previous partial is still running or the clip is too short.
fn spawn_partial(session: &ActiveSession) {
    if session.pipeline.sample_count() < session.sample_rate as usize / 2 {
        return;
    }
    if PARTIAL_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }

    let wav = session.pipeline.to_wav();
    let epoch = PARTIAL_EPOCH.load(Ordering::SeqCst);
    std::thread::spawn(move || {
        let config = current_config();
        let text = (|| -> Result<String, String> {
            let path = write_temp_wav(&wav)?;
            let result = crate::stt::transcribe_wav(
                &path,
                &config.stt.model,
                &config.stt.language,
                config.stt.binary_path.as_deref(),
                config.stt.model_path.as_deref(),
                &config.stt.runtime,
                &config.stt.prompt,
            );
            let _ = std::fs::remove_file(&path);
            result
        })();
        if epoch == PARTIAL_EPOCH.load(Ordering::SeqCst) {
            match text {
                Ok(text) if !text.trim().is_empty() => emit_partial(text),
                Ok(_) => {}
                Err(error) => log::debug!("partial transcription failed: {error}"),
            }
        }
        PARTIAL_IN_FLIGHT.store(false, Ordering::SeqCst);
    });
}

fn finalize(session: ActiveSession) {
    FINALIZING.fetch_add(1, Ordering::SeqCst);
    let result = if session.pipeline.sample_count() == 0 {
        log::warn!("atvv: session produced no samples");
        Err("未捕获到音频".to_string())
    } else if below_min_audio_ms(&session) {
        log::debug!(
            "atvv: dropping short clip stream={} samples={}",
            session.stream_id,
            session.pipeline.sample_count()
        );
        Ok(())
    } else {
        log::info!(
            "atvv: finalizing stream={} samples={}",
            session.stream_id,
            session.pipeline.sample_count()
        );
        transcribe_and_store(&session)
    };
    FINALIZING.fetch_sub(1, Ordering::SeqCst);

    if session.pipeline.sample_count() > 0 && !below_min_audio_ms(&session) {
        let duration_secs = session.pipeline.sample_count() as f64 / session.sample_rate as f64;
        if let Some(state) = crate::runtime::state() {
            let mut stats = state.stats.lock().unwrap();
            stats.record_session(duration_secs);
            state.persist_stats(&stats);
        }
    }

    let idle = !RECORDING.load(Ordering::SeqCst) && FINALIZING.load(Ordering::SeqCst) == 0;
    match result {
        Ok(()) => {
            if idle {
                emit_state("idle", None);
            }
        }
        Err(error) => {
            log::warn!("voice session failed: {error}");
            if !RECORDING.load(Ordering::SeqCst) {
                emit_state("error", Some(error));
            }
        }
    }
}

#[cfg(feature = "inject")]
fn auto_inject(item: &mut TextItem) {
    let Some(text) = item.injectable_text() else {
        return;
    };

    if crate::inject::foreground_is_self() {
        log::info!("auto-inject skipped: own window is focused");
        return;
    }

    let method = current_config().inject.method;
    match crate::inject::inject(text, &method) {
        Ok(()) => {
            log::info!("auto-inject ok");
            if matches!(item.status, ItemStatus::Ready | ItemStatus::Skipped) {
                item.status = ItemStatus::Injected;
            }
        }
        Err(error) => log::warn!("auto-inject failed: {error}"),
    }
}

#[cfg(not(feature = "inject"))]
fn auto_inject(_item: &mut TextItem) {}

fn transcribe_and_store(session: &ActiveSession) -> Result<(), String> {
    let config = current_config();

    emit_state("transcribing", None);

    // Publish a placeholder item before STT so the UI can show "转录中".
    let mut item = TextItem::new(String::new());
    if let Some(state) = crate::runtime::state() {
        state.push_item(item.clone());
    }
    crate::runtime::emit("voice://result", item.clone());

    let stt_started = Instant::now();
    let transcript = (|| -> Result<String, String> {
        let wav = session.pipeline.to_wav();
        let wav_path = write_temp_wav(&wav)?;
        let result = crate::stt::transcribe_wav(
            &wav_path,
            &config.stt.model,
            &config.stt.language,
            config.stt.binary_path.as_deref(),
            config.stt.model_path.as_deref(),
            &config.stt.runtime,
            &config.stt.prompt,
        );
        let _ = std::fs::remove_file(&wav_path);
        result
    })();
    item.stt_ms = stt_started.elapsed().as_millis() as u64;

    let raw = match transcript {
        Ok(text) => text.trim().to_string(),
        Err(error) => {
            item.status = ItemStatus::Failed(error.clone());
            publish_item(&item);
            return Err(error);
        }
    };
    if raw.is_empty() {
        item.status = ItemStatus::Failed("没有识别到文字".into());
        publish_item(&item);
        return Err("没有识别到文字".into());
    }

    item.raw_text = raw.clone();
    publish_item(&item);

    if let Some(state) = crate::runtime::state() {
        let mut stats = state.stats.lock().unwrap();
        stats.record_stt(raw.len() as u64);
        state.persist_stats(&stats);
    }

    if config.llm.api_key.trim().is_empty() || !config.llm.enabled {
        item.status = ItemStatus::Skipped;
    } else {
        emit_state("formatting", None);
        let llm_started = Instant::now();
        let first_delta_ms = Arc::new(AtomicU64::new(0));
        let first_delta_cb = first_delta_ms.clone();
        let thinking_ms = Arc::new(AtomicU64::new(0));
        let thinking_cb = thinking_ms.clone();
        let reasoning = Arc::new(Mutex::new(String::new()));
        let reasoning_cb = reasoning.clone();
        let started_cb = llm_started;
        let mut reasoning_started: Option<Instant> = None;
        let mut stream_item = item.clone();
        let result = format_with_llm(
            &config,
            &raw,
            Box::new(move |event: LlmDelta| {
                match event {
                    LlmDelta::Reasoning(text) => {
                        if reasoning_started.is_none() {
                            reasoning_started = Some(Instant::now());
                        }
                        reasoning_cb.lock().unwrap().push_str(&text);
                        stream_item.reasoning_text.push_str(&text);
                    }
                    LlmDelta::Content(text) => {
                        if first_delta_cb.load(Ordering::SeqCst) == 0 {
                            let ms = started_cb.elapsed().as_millis() as u64;
                            let _ = first_delta_cb.compare_exchange(
                                0,
                                ms.max(1),
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            );
                            if let Some(started) = reasoning_started.take() {
                                thinking_cb.store(
                                    started.elapsed().as_millis() as u64,
                                    Ordering::SeqCst,
                                );
                            }
                        }
                        stream_item.formatted_text.push_str(&text);
                    }
                }
                crate::runtime::emit("voice://update", stream_item.clone());
            }),
        );
        item.llm_ms = llm_started.elapsed().as_millis() as u64;
        item.llm_ttft_ms = first_delta_ms.load(Ordering::SeqCst);
        item.llm_gen_ms = if item.llm_ttft_ms > 0 {
            item.llm_ms.saturating_sub(item.llm_ttft_ms)
        } else {
            0
        };
        item.thinking_ms = thinking_ms.load(Ordering::SeqCst);
        item.reasoning_text = reasoning.lock().unwrap().clone();
        match result {
            Ok((text, llm_usage)) if !text.trim().is_empty() => {
                item.formatted_text = text;
                item.status = ItemStatus::Ready;
                if let Some(state) = crate::runtime::state() {
                    let mut stats = state.stats.lock().unwrap();
                    stats.record_llm(llm_usage.prompt_tokens, llm_usage.completion_tokens);
                    state.persist_stats(&stats);
                }
            }
            Ok(_) => {
                item.status = ItemStatus::Failed("LLM 未返回内容".into());
            }
            Err(error) => {
                log::warn!("LLM formatting failed: {error}");
                item.status = ItemStatus::Failed(error);
            }
        }
    }

    auto_inject(&mut item);
    publish_item(&item);
    Ok(())
}

/// Re-run LLM formatting for an existing item (used by the retry button).
pub fn retry_format(config: AppConfig, id: String) {
    let Some(mut item) = find_item(&id) else {
        return;
    };
    if item.raw_text.trim().is_empty() {
        return;
    }

    item.status = ItemStatus::Processing;
    item.formatted_text.clear();
    item.reasoning_text.clear();
    publish_item(&item);

    let llm_started = Instant::now();
    let first_delta_ms = Arc::new(AtomicU64::new(0));
    let first_delta_cb = first_delta_ms.clone();
    let thinking_ms = Arc::new(AtomicU64::new(0));
    let thinking_cb = thinking_ms.clone();
    let reasoning = Arc::new(Mutex::new(String::new()));
    let reasoning_cb = reasoning.clone();
    let started_cb = llm_started;
    let mut reasoning_started: Option<Instant> = None;
    let mut stream_item = item.clone();
    let result = format_with_llm(
        &config,
        &item.raw_text,
        Box::new(move |event: LlmDelta| {
            match event {
                LlmDelta::Reasoning(text) => {
                    if reasoning_started.is_none() {
                        reasoning_started = Some(Instant::now());
                    }
                    reasoning_cb.lock().unwrap().push_str(&text);
                    stream_item.reasoning_text.push_str(&text);
                }
                LlmDelta::Content(text) => {
                    if first_delta_cb.load(Ordering::SeqCst) == 0 {
                        let ms = started_cb.elapsed().as_millis() as u64;
                        let _ = first_delta_cb.compare_exchange(
                            0,
                            ms.max(1),
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        );
                        if let Some(started) = reasoning_started.take() {
                            thinking_cb
                                .store(started.elapsed().as_millis() as u64, Ordering::SeqCst);
                        }
                    }
                    stream_item.formatted_text.push_str(&text);
                }
            }
            crate::runtime::emit("voice://update", stream_item.clone());
        }),
    );
    item.llm_ms = llm_started.elapsed().as_millis() as u64;
    item.llm_ttft_ms = first_delta_ms.load(Ordering::SeqCst);
    item.llm_gen_ms = if item.llm_ttft_ms > 0 {
        item.llm_ms.saturating_sub(item.llm_ttft_ms)
    } else {
        0
    };
    item.thinking_ms = thinking_ms.load(Ordering::SeqCst);
    item.reasoning_text = reasoning.lock().unwrap().clone();
    match result {
        Ok((text, _llm_usage)) if !text.trim().is_empty() => {
            item.formatted_text = text;
            item.status = ItemStatus::Ready;
        }
        Ok(_) => {
            item.status = ItemStatus::Failed("LLM 未返回内容".into());
        }
        Err(error) => {
            log::warn!("retry formatting failed: {error}");
            item.status = ItemStatus::Failed(error);
        }
    }
    publish_item(&item);
}

fn find_item(id: &str) -> Option<TextItem> {
    crate::runtime::state().and_then(|state| state.item(id))
}

fn publish_item(item: &TextItem) {
    if let Some(state) = crate::runtime::state() {
        state.update_item(item.clone());
    }
    crate::runtime::emit("voice://update", item.clone());
}

fn current_config() -> AppConfig {
    crate::runtime::state()
        .map(|state| state.config.lock().unwrap().clone())
        .unwrap_or_default()
}

fn format_with_llm(
    config: &AppConfig,
    text: &str,
    on_event: Box<dyn FnMut(LlmDelta) + Send>,
) -> Result<(String, crate::llm::provider::LlmUsage), String> {
    let provider = crate::llm::provider::create_provider(&config.llm);
    tauri::async_runtime::block_on(provider.format(text, &config.llm.format_prompt, on_event))
}

fn write_temp_wav(wav: &[u8]) -> Result<std::path::PathBuf, String> {
    let path = std::env::temp_dir().join(format!("clay-mic-{}.wav", uuid::Uuid::new_v4()));
    std::fs::write(&path, wav).map_err(|e| format!("写入临时音频失败：{e}"))?;
    Ok(path)
}
