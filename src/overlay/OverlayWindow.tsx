import { useCallback, useEffect, useRef, useState } from "react";
import {
  itemStatus,
  tauriInvoke,
  tauriListen,
  type Config,
  type ItemTone,
  type TextItem,
  type VoicePhase,
  type VoiceStatePayload,
} from "../lib/config";
import "./overlay.css";

const TONE_CLASS: Record<ItemTone, string> = {
  busy: "tone-busy",
  ok: "tone-ok",
  muted: "tone-muted",
  fail: "tone-fail",
};

const PHASE_BANNER: Partial<Record<VoicePhase, string>> = {
  recording: "语音录入中…",
  transcribing: "转录中…",
  formatting: "LLM 改写中…",
};

function byTimestamp(a: TextItem, b: TextItem): number {
  return new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime();
}

interface OverlayPayload {
  style: string;
  items: TextItem[];
}

export function OverlayWindow() {
  const [items, setItems] = useState<TextItem[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [theme, setTheme] = useState("aurora");
  const [phase, setPhase] = useState<VoicePhase>("idle");
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const newestId = useRef<string | null>(null);

  // Load the configured theme up front so the window never flashes the default
  // theme before the first refresh event arrives.
  useEffect(() => {
    tauriInvoke<Config>("get_config")
      .then((config) => setTheme(config.overlay.style))
      .catch((error) =>
        console.warn("[clay-mic] overlay load config failed:", error),
      );
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    const upsert = (item: TextItem) => {
      setItems((prev) => {
        const index = prev.findIndex((existing) => existing.id === item.id);
        if (index >= 0) {
          const next = prev.slice();
          next[index] = item;
          return next;
        }
        return [...prev, item].slice(-10).sort(byTimestamp);
      });
    };

    void (async () => {
      try {
        const offs = await Promise.all([
          tauriListen<OverlayPayload>("overlay://refresh", (payload) => {
            if (payload.style) setTheme(payload.style);
            setItems([...payload.items].sort(byTimestamp));
            setSelectedIndex(Math.max(0, payload.items.length - 1));
          }),
          tauriListen<TextItem>("voice://result", upsert),
          tauriListen<TextItem>("voice://update", upsert),
          tauriListen<string>("text://removed", (id) =>
            setItems((prev) => prev.filter((item) => item.id !== id)),
          ),
          tauriListen<string>("text://cleared", () =>
            setItems([]),
          ),
          tauriListen<VoiceStatePayload>("voice://state", (payload) => {
            setPhase(payload.state);
          }),
        ]);
        if (disposed) offs.forEach((off) => off());
        else unlisteners.push(...offs);
      } catch (error) {
        console.warn("[clay-mic] subscribe overlay events failed:", error);
      }
    })();

    return () => {
      disposed = true;
      unlisteners.forEach((off) => off());
    };
  }, []);

  // Keep the keyboard-selected item inside the scroll viewport.
  useEffect(() => {
    const node = listRef.current?.children[selectedIndex] as
      | HTMLElement
      | undefined;
    node?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex, items]);

  // After a removal, keep the selection in range (land on the new last item).
  useEffect(() => {
    setSelectedIndex((index) =>
      items.length === 0 ? 0 : Math.min(index, items.length - 1),
    );
  }, [items]);

  // Jump to the newest item whenever one is inserted.
  useEffect(() => {
    const newest = items[items.length - 1];
    if (newest && newest.id !== newestId.current) {
      setSelectedIndex(items.length - 1);
      newestId.current = newest.id;
    }
  }, [items]);

  const hide = useCallback(() => {
    void tauriInvoke("hide_overlay");
  }, []);

  const startResize = useCallback(() => {
    const current = window.__TAURI__?.window.getCurrentWindow();
    void current?.startResizeDragging("SouthEast");
  }, []);

  const injectItem = useCallback(async (item: TextItem) => {
    const text = item.formatted_text || item.raw_text;
    if (!text) return;
    try {
      // Hide first so focus returns to the window the user was typing in,
      // then inject into it.
      await tauriInvoke("hide_overlay");
      await new Promise((resolve) => setTimeout(resolve, 80));
      await tauriInvoke("inject_text", { text });
    } catch (error) {
      console.error("[clay-mic] overlay inject failed:", error);
    }
  }, []);

  const copyItem = useCallback(async (item: TextItem) => {
    const text = item.formatted_text || item.raw_text;
    try {
      await navigator.clipboard.writeText(text);
      setCopiedId(item.id);
      window.setTimeout(
        () => setCopiedId((id) => (id === item.id ? null : id)),
        1200,
      );
    } catch (error) {
      console.error("[clay-mic] overlay copy failed:", error);
    }
  }, []);

  const retryItem = useCallback(async (item: TextItem) => {
    try {
      await tauriInvoke("retry_item", { id: item.id });
    } catch (error) {
      console.error("[clay-mic] overlay retry failed:", error);
    }
  }, []);

  const deleteSelected = useCallback(async () => {
    const item = items[selectedIndex];
    if (!item) return;
    try {
      await tauriInvoke("delete_item", { id: item.id });
    } catch (error) {
      console.error("[clay-mic] overlay delete failed:", error);
    }
  }, [items, selectedIndex]);

  // A pending Ctrl+D confirm expires on its own, or when the selection moves.
  useEffect(() => {
    if (!confirmDelete) return;
    const timer = window.setTimeout(() => setConfirmDelete(false), 3000);
    return () => window.clearTimeout(timer);
  }, [confirmDelete]);

  useEffect(() => {
    setConfirmDelete(false);
  }, [selectedIndex]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      switch (e.key) {
        case "ArrowUp":
          e.preventDefault();
          setSelectedIndex((i) => Math.max(0, i - 1));
          break;
        case "ArrowDown":
          e.preventDefault();
          setSelectedIndex((i) => Math.min(items.length - 1, i + 1));
          break;
        case "Enter":
          e.preventDefault();
          if (items[selectedIndex]) void injectItem(items[selectedIndex]);
          break;
        case "d":
        case "D":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            if (!items[selectedIndex]) break;
            if (confirmDelete) {
              setConfirmDelete(false);
              void deleteSelected();
            } else {
              setConfirmDelete(true);
            }
          }
          break;
        case "Escape":
          e.preventDefault();
          hide();
          break;
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [items, selectedIndex, injectItem, hide, confirmDelete, deleteSelected]);

  const banner = PHASE_BANNER[phase];

  return (
    <div className={`ov-root ov-theme-${theme}`}>
      <div className="ov-card">
        <div className="ov-title" data-tauri-drag-region>
          <span data-tauri-drag-region className="ov-title-left">
            选择文本注入
            {banner && (
              <span className="ov-banner-inline">
                <span className="dot" />
                {banner}
              </span>
            )}
          </span>
          <button
            type="button"
            tabIndex={-1}
            className="ov-close"
            title="关闭"
            onMouseDown={(event) => event.preventDefault()}
            onClick={hide}
          >
            <CloseIcon />
          </button>
        </div>
        {items.length === 0 ? (
          <div className="ov-empty">暂无文本</div>
        ) : (
          <div className="ov-list" ref={listRef}>
            {items.map((item, i) => {
              const state = itemStatus(item);
              const text = item.formatted_text || item.raw_text;
              return (
                <div
                  key={item.id}
                  role="button"
                  tabIndex={-1}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => void injectItem(item)}
                  className={`ov-item${i === selectedIndex ? " is-active" : ""}`}
                >
                  <div className="ov-text">{text || state.label}</div>
                  <div className="ov-meta">
                    <span className="ov-time">
                      {new Date(item.timestamp).toLocaleTimeString()}
                    </span>
                    <span
                      className={`ov-status ${TONE_CLASS[state.tone]}`}
                      title={state.error ?? state.label}
                    >
                      {state.label}
                    </span>
                    <span className="ov-actions">
                      {state.tone === "fail" && item.raw_text.trim() !== "" && (
                        <button
                          type="button"
                          tabIndex={-1}
                          className="ov-retry"
                          title={state.error ?? "重试"}
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={(event) => {
                            event.stopPropagation();
                            void retryItem(item);
                          }}
                        >
                          重试
                        </button>
                      )}
                      {i === selectedIndex && confirmDelete && (
                        <span className="ov-confirm-inline">再按一次 Ctrl+D 删除</span>
                      )}
                      <button
                        type="button"
                        tabIndex={-1}
                        className="ov-copy"
                        title={copiedId === item.id ? "已复制" : "复制"}
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={(event) => {
                          event.stopPropagation();
                          void copyItem(item);
                        }}
                      >
                        {copiedId === item.id ? <CheckIcon /> : <CopyIcon />}
                      </button>
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        )}
        <div className="ov-foot">↑↓ 选择 · Enter 注入 · Ctrl+D 删除 · Esc 关闭</div>
        <div
          className="ov-resize"
          onMouseDown={(event) => {
            event.preventDefault();
            startResize();
          }}
        />
      </div>
    </div>
  );
}

function CloseIcon() {
  return (
    <svg
      className="w-3 h-3"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2.5}
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M6 6l12 12M18 6L6 18" />
    </svg>
  );
}

function CopyIcon() {
  return (
    <svg
      className="w-3.5 h-3.5"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
      />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg
      className="w-3.5 h-3.5"
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
    </svg>
  );
}
