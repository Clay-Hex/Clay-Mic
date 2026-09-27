import { useCallback, useEffect, useState } from "react";
import {
  tauriInvoke,
  tauriListen,
  type Config,
  type VoicePhase,
  type VoiceStatePayload,
} from "../lib/config";
import "./indicator.css";

const PHASE_LABEL: Partial<Record<VoicePhase, string>> = {
  recording: "正在聆听…",
  transcribing: "正在转写…",
  formatting: "正在整理…",
};

function formatClock(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return `${minutes}:${rest.toString().padStart(2, "0")}`;
}

export function VoiceIndicator() {
  const [phase, setPhase] = useState<VoicePhase>("recording");
  const [style, setStyle] = useState("pulse");
  const [elapsed, setElapsed] = useState(0);
  const [recordingStart, setRecordingStart] = useState<number | null>(null);
  const [partial, setPartial] = useState("");

  const refreshStyle = useCallback(() => {
    tauriInvoke<Config>("get_config")
      .then((config) => setStyle(config.indicator.style))
      .catch((error) =>
        console.warn("[clay-mic] indicator load config failed:", error),
      );
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriListen<VoiceStatePayload>("voice://state", (payload) => {
      setPhase(payload.state);
      // A fresh timestamp per recording event restarts the timer even when the
      // phase value is unchanged (e.g. the very first recording).
      setRecordingStart(payload.state === "recording" ? Date.now() : null);
      if (payload.state !== "recording") setPartial("");
      refreshStyle();
    })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((error) =>
        console.warn("[clay-mic] indicator subscribe failed:", error),
      );
    refreshStyle();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshStyle]);

  // Live partial transcription while recording (streaming preview).
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriListen<{ text: string }>("voice://partial", (payload) =>
      setPartial(payload.text),
    )
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((error) =>
        console.warn("[clay-mic] indicator partial subscribe failed:", error),
      );
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // Elapsed recording time, measured from the session start.
  useEffect(() => {
    if (recordingStart === null) {
      setElapsed(0);
      return;
    }
    const tick = () =>
      setElapsed(Math.floor((Date.now() - recordingStart) / 1000));
    tick();
    const timer = window.setInterval(tick, 250);
    return () => window.clearInterval(timer);
  }, [recordingStart]);

  return (
    <div className={`ind-root ind-theme-${style} ind-phase-${phase}`}>
      <div className="ind-panel">
        <IndicatorVisual style={style} />
        <div className="ind-label">
          {PHASE_LABEL[phase] ?? "处理中…"}
          {recordingStart !== null && (
            <span className="ind-time">{formatClock(elapsed)}</span>
          )}
        </div>
        {partial && (
          <div className="ind-partial">
            <span>{partial.slice(-200)}</span>
          </div>
        )}
      </div>
    </div>
  );
}

export function IndicatorVisual({ style }: { style: string }) {
  switch (style) {
    case "spectrum":
      return (
        <div className="ind-spectrum">
          <div className="bars">
            {Array.from({ length: 11 }).map((_, i) => (
              <i key={i} />
            ))}
          </div>
        </div>
      );
    case "orbit":
      return (
        <div className="ind-orbit">
          <div className="orbit">
            <span className="track" />
            <span className="arc" />
            <span className="arc slow" />
            <span className="core">
              <MicIcon />
            </span>
          </div>
        </div>
      );
    case "particles":
      return (
        <div className="ind-particles">
          <div className="wrap">
            <span className="core" />
            <span className="spin">
              <i className="dot" />
            </span>
            <span className="spin b">
              <i className="dot" />
            </span>
            <span className="spin c">
              <i className="dot" />
            </span>
          </div>
        </div>
      );
    case "glow":
      return (
        <div className="ind-glow">
          <div className="wrap">
            <span className="halo" />
            <span className="halo b" />
            <span className="core" />
          </div>
        </div>
      );
    case "remote-wave":
      return (
        <div className="ind-remote-wave">
          <span className="body" />
          <span className="hl" />
          <span className="key" />
          <span className="w w1" />
          <span className="w w2" />
          <span className="w w3" />
        </div>
      );
    default:
      return (
        <div className="ind-pulse">
          <div className="orb">
            <span className="ring" />
            <span className="ring" />
            <span className="core">
              <MicIcon />
            </span>
          </div>
          <div className="bars">
            {Array.from({ length: 5 }).map((_, i) => (
              <i key={i} />
            ))}
          </div>
        </div>
      );
  }
}

function MicIcon() {
  return (
    <svg
      className="ind-mic"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z"
      />
    </svg>
  );
}
