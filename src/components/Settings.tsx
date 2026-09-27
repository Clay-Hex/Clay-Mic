import { useCallback, useEffect, useState } from "react";
import {
  Config,
  defaultConfig,
  tauriInvoke,
  tauriListen,
  type DownloadProgress,
  type SttStatus,
} from "../lib/config";
import { SettingsCoreContext } from "./settings/context";
import { DriverSect } from "./settings/DriverSect";
import { InterfaceSect } from "./settings/InterfaceSect";
import { LlmSect } from "./settings/LlmSect";
import { STTSect } from "./settings/STTSect";

const SECTIONS = [
  { id: "stt", label: "语音识别", hint: "模型、语言与下载" },
  { id: "llm", label: "LLM 格式化", hint: "Provider、提示词与思考" },
  { id: "ui", label: "界面与行为", hint: "快捷键、样式与托盘" },
  { id: "driver", label: "按键拦截", hint: "屏蔽与修复服务" },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

const SECTION_STORAGE_KEY = "settings_section";

function readStoredSection(): SectionId {
  const stored = localStorage.getItem(SECTION_STORAGE_KEY);
  return SECTIONS.some((section) => section.id === stored)
    ? (stored as SectionId)
    : "stt";
}

export function Settings({
  onDirtyChange,
  registerSave,
}: {
  onDirtyChange?: (dirty: boolean) => void;
  registerSave?: (save: () => Promise<boolean>) => void;
} = {}) {
  const [config, setConfig] = useState<Config>(defaultConfig);
  const [loaded, setLoaded] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [baseline, setBaseline] = useState<string | null>(null);
  const [section, setSection] = useState<SectionId>(readStoredSection);
  const [sttStatus, setSttStatus] = useState<SttStatus | null>(null);
  const [sttBusy, setSttBusy] = useState(false);
  const [sttMessage, setSttMessage] = useState<string | null>(null);
  const [dlProgress, setDlProgress] = useState<DownloadProgress | null>(null);
  const clearProgress = useCallback(() => setDlProgress(null), []);

  useEffect(() => {
    localStorage.setItem(SECTION_STORAGE_KEY, section);
  }, [section]);

  useEffect(() => {
    let cancelled = false;
    tauriInvoke<Config>("get_config")
      .then((cfg) => {
        if (cancelled) return;
        setConfig(cfg);
        setBaseline(JSON.stringify(cfg));
        setLoaded(true);
      })
      .catch((e) => {
        console.warn("[clay-mic] load config failed:", e);
        // Fall back to defaults rather than leaving the page on the loader.
        setBaseline(JSON.stringify(defaultConfig));
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriListen<DownloadProgress>("download://progress", (p) => {
      if (p.phase === "done") {
        setDlProgress(null);
      } else {
        setDlProgress(p);
      }
    })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((e) => console.warn("[clay-mic] subscribe progress failed:", e));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      setSttStatus(await tauriInvoke<SttStatus>("stt_status"));
    } catch (e) {
      console.warn("[clay-mic] stt_status failed:", e);
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const update = (path: string, value: string | number) => {
    setConfig((prev) => {
      const next = JSON.parse(JSON.stringify(prev));
      const keys = path.split(".");
      let obj: Record<string, unknown> = next;
      for (let i = 0; i < keys.length - 1; i++)
        obj = obj[keys[i]] as Record<string, unknown>;
      obj[keys[keys.length - 1]] = value;
      return next;
    });
  };

  const save = useCallback(async (): Promise<boolean> => {
    setSaveError(null);
    try {
      await tauriInvoke("update_config", { config });
      // Clear the dirty marker only once the save actually succeeded.
      setBaseline(JSON.stringify(config));
      void refreshStatus();
      return true;
    } catch (e) {
      console.error("[clay-mic] save config failed:", e);
      setSaveError(String(e));
      return false;
    }
  }, [config, refreshStatus]);

  const handleSave = () => {
    void save();
  };

  const runDownload = useCallback(
    async (
      cmd: "download_stt_binary" | "download_stt_model",
      args?: Record<string, unknown>,
    ) => {
      const source = cmd === "download_stt_binary" ? "binary" : "model";
      setSttBusy(true);
      setSttMessage(null);
      // Optimistic: show the bar immediately, before the first backend event.
      setDlProgress({ source, phase: "downloading", percent: 0 });
      try {
        const path = await tauriInvoke<string>(cmd, args);
        setSttMessage(`已完成：${path}`);
        await refreshStatus();
      } catch (e) {
        setSttMessage(`失败：${String(e)}`);
      } finally {
        setSttBusy(false);
        setDlProgress(null);
      }
    },
    [refreshStatus],
  );

  const dirty = baseline !== null && JSON.stringify(config) !== baseline;

  useEffect(() => {
    onDirtyChange?.(dirty);
    // Leaving the page unmounts this component; report clean so a discarded
    // edit cannot leave App's flag stuck on.
    return () => onDirtyChange?.(false);
  }, [dirty, onDirtyChange]);

  useEffect(() => {
    registerSave?.(save);
  }, [registerSave, save]);

  // Must stay below every hook: returning earlier would change the hook count
  // between renders and crash React.
  if (!loaded) {
    return <div className="w-full max-w-4xl text-sm text-gray-400">加载中…</div>;
  }

  return (
    <SettingsCoreContext.Provider value={{ config, setConfig, update }}>
      <div className="w-full max-w-4xl flex items-start gap-5">
        <nav className="w-40 shrink-0 space-y-1 sticky top-0 self-start">
          {SECTIONS.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-current={section === item.id ? "true" : undefined}
              onClick={() => setSection(item.id)}
              className={`w-full text-left px-3 py-2 rounded-lg transition-colors ${
                section === item.id
                  ? "bg-blue-50 text-blue-700 font-medium"
                  : "text-gray-600 hover:bg-gray-100"
              }`}
            >
              <span className="block text-[13px]">{item.label}</span>
              <span
                className={`block text-[11px] mt-0.5 ${
                  section === item.id ? "text-blue-500" : "text-gray-400"
                }`}
              >
                {item.hint}
              </span>
            </button>
          ))}
        </nav>

        <div className="flex-1 min-w-0 space-y-4 pb-14">
          <div className={section === "stt" ? "" : "hidden"}>
            <STTSect
              sttStatus={sttStatus}
              sttBusy={sttBusy}
              sttMessage={sttMessage}
              refreshStatus={refreshStatus}
              runDownload={runDownload}
              dlProgress={dlProgress}
            />
          </div>
          <div className={section === "llm" ? "" : "hidden"}>
            <LlmSect />
          </div>
          <div className={section === "ui" ? "" : "hidden"}>
            <InterfaceSect />
          </div>
          <div className={section === "driver" ? "" : "hidden"}>
            <DriverSect progress={dlProgress} onProgressEnd={clearProgress} />
          </div>

          {dirty && (
            <div className="sticky bottom-0 z-10 pt-3 pb-1 bg-gray-50/95 backdrop-blur-sm">
              <button
                onClick={handleSave}
                className="w-full py-2.5 bg-blue-600 text-white text-sm font-medium rounded-lg hover:bg-blue-700 active:bg-blue-800 transition-colors shadow-sm"
              >
                保存设置
              </button>
            </div>
          )}
        </div>

        {saveError && (
          <ErrorDialog
            message={`保存失败：${saveError}`}
            onClose={() => setSaveError(null)}
          />
        )}
      </div>
    </SettingsCoreContext.Provider>
  );
}

function ErrorDialog({
  message,
  onClose,
}: {
  message: string;
  onClose: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/30"
      onClick={onClose}
    >
      <div
        className="bg-white rounded-xl shadow-lg border border-gray-200 p-5 w-72"
        onClick={(e) => e.stopPropagation()}
      >
        <p className="text-sm text-gray-700 mb-4 break-all">{message}</p>
        <div className="flex justify-end">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors"
          >
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
