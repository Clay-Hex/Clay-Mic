import { useCallback, useEffect, useRef, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { DevicePanel } from "./components/DevicePanel";
import { TextList } from "./components/TextList";
import { Settings } from "./components/Settings";
import { Statistics } from "./components/Statistics";
import { AboutPanel } from "./components/AboutPanel";
import { KeymapPanel } from "./components/KeymapPanel";
import {
  tauriInvoke,
  tauriListen,
  type SttStatus,
  type TapStatus,
  type VoicePhase,
  type VoiceStatePayload,
} from "./lib/config";

type Page = "device" | "keys" | "list" | "stats" | "settings" | "about";

const PAGE_TITLES: Record<Page, string> = {
  device: "设备连接",
  keys: "按键配置",
  list: "文本列表",
  stats: "使用统计",
  settings: "设置",
  about: "关于",
};

export default function App() {
  const [activePage, setActivePage] = useState<Page>("device");
  const [connectionStatus, setConnectionStatus] = useState<
    "disconnected" | "connecting" | "connected" | "streaming"
  >("disconnected");
  const [voicePhase, setVoicePhase] = useState<VoicePhase>("idle");
  // Both the settings and keys pages are save-on-demand, so the same
  // dirty/navigation guard covers whichever one is open.
  const [dirtyPage, setDirtyPage] = useState<Page | null>(null);
  const [pendingPage, setPendingPage] = useState<Page | null>(null);
  const [saving, setSaving] = useState(false);
  const pageSave = useRef<(() => Promise<boolean>) | null>(null);
  const [tapNudge, setTapNudge] = useState(false);
  const tapNudgeHandled = useRef(false);

  const navigate = useCallback(
    (page: Page) => {
      if (dirtyPage === activePage && page !== activePage) {
        setPendingPage(page);
        return;
      }
      setActivePage(page);
    },
    [activePage, dirtyPage],
  );

  const leaveWithoutSaving = useCallback(() => {
    // The edits are being thrown away, so the page is no longer dirty.
    setDirtyPage(null);
    if (pendingPage) setActivePage(pendingPage);
    setPendingPage(null);
  }, [pendingPage]);

  const saveAndLeave = useCallback(async () => {
    if (saving) return;
    setSaving(true);
    try {
      const ok = (await pageSave.current?.()) ?? true;
      if (ok) {
        if (pendingPage) setActivePage(pendingPage);
      }
      // On failure the dialog closes and the user stays here, where the save
      // error is already shown.
      setPendingPage(null);
    } finally {
      setSaving(false);
    }
  }, [pendingPage, saving]);

  // Stable per page: a fresh closure each render would re-fire the child's
  // report effect (and its cleanup) on every App render.
  const keysDirtyChange = useCallback(
    (dirty: boolean) => setDirtyPage(dirty ? "keys" : null),
    [],
  );
  const settingsDirtyChange = useCallback(
    (dirty: boolean) => setDirtyPage(dirty ? "settings" : null),
    [],
  );
  const registerPageSave = useCallback((save: () => Promise<boolean>) => {
    pageSave.current = save;
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriListen<VoiceStatePayload>("voice://state", (payload) => {
      setVoicePhase(payload.state);
    })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((error) =>
        console.warn("[clay-mic] subscribe voice state failed:", error),
      );
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // A resident hook can predate this build after an app update; remind once
  // per session. The invoke catches an already-established heartbeat, the
  // listener catches the 0→1 transition when the first one lands later.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const check = (status: TapStatus) => {
      if (disposed || tapNudgeHandled.current) return;
      if (status.injected && status.dll_needs_update) {
        tapNudgeHandled.current = true;
        setTapNudge(true);
      }
    };
    void tauriInvoke<TapStatus>("get_tap_status")
      .then(check)
      .catch((error) =>
        console.warn("[clay-mic] tap update nudge check failed:", error),
      );
    void tauriListen<TapStatus>("tap://status", check)
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((error) =>
        console.warn("[clay-mic] subscribe tap status for nudge failed:", error),
      );
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return (
    <div className="flex h-screen bg-gray-50 overflow-hidden">
      <Sidebar
        activePage={activePage}
        onNavigate={navigate}
      />

      <main className="flex-1 flex flex-col min-w-0">
        <header className="h-12 flex items-center px-5 border-b border-gray-200 bg-white shrink-0">
          <h1 className="text-sm font-semibold text-gray-800">
            {PAGE_TITLES[activePage]}
          </h1>
          <div className="ml-auto flex items-center gap-3">
            <SttBadge />
            <VoiceBadge phase={voicePhase} />
            <StatusBadge status={connectionStatus} />
          </div>
        </header>

        <div className="flex-1 overflow-y-auto p-5" data-scroll>
          {/* Kept mounted (hidden when inactive) so switching pages does not
              reset connection state or re-trigger auto-reconnect. */}
          <div className={activePage === "device" ? "" : "hidden"}>
            <DevicePanel onStatusChange={setConnectionStatus} />
          </div>
          {activePage === "keys" && (
            <KeymapPanel
              onDirtyChange={keysDirtyChange}
              registerSave={registerPageSave}
            />
          )}
          {activePage === "list" && <TextList />}
          {activePage === "stats" && <Statistics />}
          {activePage === "about" && <AboutPanel />}
          {activePage === "settings" && (
            <Settings
              onDirtyChange={settingsDirtyChange}
              registerSave={registerPageSave}
            />
          )}
        </div>
      </main>

      {tapNudge && <TapUpdateNudgeDialog onClose={() => setTapNudge(false)} />}
      {pendingPage && (
        <UnsavedDialog
          onSave={() => void saveAndLeave()}
          onDiscard={leaveWithoutSaving}
          onCancel={() => setPendingPage(null)}
        />
      )}
    </div>
  );
}

function TapUpdateNudgeDialog({ onClose }: { onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/30">
      <div className="bg-white rounded-xl shadow-lg border border-gray-200 p-5 w-80">
        <p className="text-sm font-semibold text-gray-800 mb-2">
          HID Tap 组件版本不匹配
        </p>
        <p className="text-[12px] text-gray-500 leading-relaxed mb-4">
          按键拦截可能异常，请到「设置 → 按键拦截」重新注入。
        </p>
        <div className="flex justify-end">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-[12px] text-gray-600 bg-gray-50 hover:bg-gray-100 rounded-lg transition-colors"
          >
            知道了
          </button>
        </div>
      </div>
    </div>
  );
}

function UnsavedDialog({
  onSave,
  onDiscard,
  onCancel,
}: {
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/30"
      onClick={onCancel}
    >
      <div
        className="bg-white rounded-xl shadow-lg border border-gray-200 p-5 w-80"
        onClick={(e) => e.stopPropagation()}
      >
        <p className="text-sm text-gray-700 mb-4">
          设置尚未保存，离开将丢失这些修改。
        </p>
        <div className="flex justify-end gap-2">
          <button
            onClick={onDiscard}
            className="px-3 py-1.5 text-[12px] text-gray-600 bg-gray-100 hover:bg-gray-200 rounded-lg transition-colors"
          >
            不保存
          </button>
          <button
            onClick={onSave}
            className="px-3 py-1.5 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors"
          >
            保存并离开
          </button>
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-[12px] text-gray-500 hover:bg-gray-100 rounded-lg transition-colors"
          >
            取消
          </button>
        </div>
      </div>
    </div>
  );
}

function SttBadge() {
  const [status, setStatus] = useState<SttStatus | null>(null);
  const [warming, setWarming] = useState(false);

  const refresh = useCallback(() => {
    tauriInvoke<SttStatus>("stt_status")
      .then(setStatus)
      .catch((error) => console.warn("[clay-mic] stt status failed:", error));
  }, []);

  useEffect(() => {
    refresh();
    const timer = window.setInterval(refresh, 4000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const ready = Boolean(status?.runtime_ready && status?.model_ready);
  const warm = Boolean(status?.warm);

  const handleWarm = useCallback(async () => {
    if (warming || warm || !ready) return;
    setWarming(true);
    try {
      await tauriInvoke("warmup_stt");
    } catch (error) {
      console.warn("[clay-mic] stt warmup failed:", error);
    } finally {
      setWarming(false);
      refresh();
    }
  }, [warming, warm, ready, refresh]);

  const state = warming ? "warming" : !ready ? "missing" : warm ? "warm" : "cold";
  const RUNTIME_NAMES: Record<string, string> = {
    cpu: "CPU",
    cuda12: "GPU·CUDA 12.4",
    cuda11: "GPU·CUDA 11.8",
  };
  const runtimeName = RUNTIME_NAMES[status?.runtime ?? "cpu"] ?? "CPU";
  const view = {
    warming: {
      dot: "bg-amber-400 animate-pulse",
      text: "text-amber-600",
      label: `STT 引擎预热中…`,
    },
    missing: {
      dot: "bg-gray-300",
      text: "text-gray-400",
      label: "STT 引擎未就绪",
    },
    warm: {
      dot: "bg-emerald-500",
      text: "text-emerald-600",
      label: `STT 引擎就绪 · ${runtimeName}`,
    },
    cold: {
      dot: "bg-gray-400",
      text: "text-gray-500",
      label: `STT 引擎待机 · ${runtimeName}`,
    },
  }[state];
  const title =
    state === "missing"
      ? "缺少 whisper 运行时或模型，请到设置中下载"
      : warm
        ? "STT 引擎已在内存中就绪"
        : "点击预热 STT 引擎（首次识别需冷启动，预热后更快）";

  return (
    <button
      type="button"
      onClick={() => void handleWarm()}
      disabled={warming || warm || !ready}
      title={title}
      className="flex items-center gap-1.5 disabled:cursor-default"
    >
      <span className={`w-1.5 h-1.5 rounded-full ${view.dot}`} />
      <span className={`text-xs font-medium ${view.text}`}>{view.label}</span>
    </button>
  );
}

function VoiceBadge({ phase }: { phase: VoicePhase }) {
  const map: Partial<
    Record<VoicePhase, { dot: string; text: string; label: string }>
  > = {
    recording: {
      dot: "bg-blue-500 animate-pulse",
      text: "text-blue-600",
      label: "录音中",
    },
    transcribing: {
      dot: "bg-indigo-500 animate-pulse",
      text: "text-indigo-600",
      label: "转写中",
    },
    formatting: {
      dot: "bg-purple-500 animate-pulse",
      text: "text-purple-600",
      label: "格式化中",
    },
    error: { dot: "bg-red-500", text: "text-red-600", label: "语音出错" },
  };
  const s = map[phase];
  if (!s) return null;
  return (
    <div className="flex items-center gap-1.5">
      <span className={`w-1.5 h-1.5 rounded-full ${s.dot}`} />
      <span className={`text-xs font-medium ${s.text}`}>{s.label}</span>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const map: Record<string, { dot: string; text: string; label: string }> = {
    disconnected: {
      dot: "bg-gray-400",
      text: "text-gray-500",
      label: "未连接",
    },
    connecting: {
      dot: "bg-amber-400",
      text: "text-amber-600",
      label: "连接中",
    },
    connected: {
      dot: "bg-emerald-500",
      text: "text-emerald-600",
      label: "已连接",
    },
    streaming: {
      dot: "bg-blue-500 animate-pulse",
      text: "text-blue-600",
      label: "录音中",
    },
  };
  const s = map[status] || map.disconnected;
  return (
    <div className="flex items-center gap-1.5">
      <span className={`w-1.5 h-1.5 rounded-full ${s.dot}`} />
      <span className={`text-xs font-medium ${s.text}`}>{s.label}</span>
    </div>
  );
}
