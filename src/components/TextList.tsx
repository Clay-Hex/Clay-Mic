import { useCallback, useEffect, useRef, useState } from "react";
import {
  itemStatus,
  tauriInvoke,
  tauriListen,
  type ItemTone,
  type TextItem,
  type TextListPage,
  type VoicePhase,
  type VoiceStatePayload,
} from "../lib/config";
import { formatTimestamp } from "../lib/time";

const TONE_CLASS: Record<ItemTone, string> = {
  busy: "bg-amber-50 text-amber-600",
  ok: "bg-emerald-50 text-emerald-600",
  muted: "bg-gray-100 text-gray-500",
  fail: "bg-red-50 text-red-600",
};

function formatDuration(ms: number): string {
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
}

function CopyButton({ text, disabled }: { text: string; disabled?: boolean }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch (error) {
      console.error("[clay-mic] copy failed:", error);
    }
  }, [text]);

  return (
    <button
      onClick={handleCopy}
      disabled={disabled}
      className={`p-1 rounded transition-colors shrink-0 ${
        disabled
          ? "text-gray-200 cursor-not-allowed"
          : "text-gray-400 hover:text-gray-700 hover:bg-white"
      }`}
      title={copied ? "已复制" : "复制"}
    >
      {copied ? (
        <svg
          className="w-3.5 h-3.5"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
        </svg>
      ) : (
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
      )}
    </button>
  );
}

export function TextList() {
  const [items, setItems] = useState<TextItem[]>([]);
  const [phase, setPhase] = useState<VoicePhase>("idle");
  const [phaseMessage, setPhaseMessage] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; id: string } | null>(
    null,
  );
  const [confirmClear, setConfirmClear] = useState(false);
  const [page, setPage] = useState(1);
  const [total, setTotal] = useState(0);
  const [pageSize, setPageSize] = useState<number>(() => {
    const stored = Number(localStorage.getItem("textlist_page_size"));
    return stored === 20 || stored === 30 || stored === 50 ? stored : 30;
  });

  const pageRef = useRef(page);
  useEffect(() => {
    pageRef.current = page;
  }, [page]);
  const itemsRef = useRef(items);
  useEffect(() => {
    itemsRef.current = items;
  }, [items]);

  const refreshPage = useCallback(
    (targetPage: number) => {
      tauriInvoke<TextListPage>("get_text_list", {
        page: targetPage,
        size: pageSize,
      })
        .then((result) => {
          setItems(result.items);
          setTotal(result.total);
          setPage(result.page);
        })
        .catch((error) =>
          console.warn("[clay-mic] load text list failed:", error),
        );
    },
    [pageSize],
  );
  const refreshRef = useRef(refreshPage);
  useEffect(() => {
    refreshRef.current = refreshPage;
  });

  // Load page one now; re-run whenever the page size changes.
  useEffect(() => {
    refreshPage(1);
  }, [refreshPage]);

  // Live updates: emitted text items and pipeline phase changes.
  useEffect(() => {
    let disposed = false;
    let unlistenResult: (() => void) | undefined;
    let unlistenUpdate: (() => void) | undefined;
    let unlistenRemoved: (() => void) | undefined;
    let unlistenCleared: (() => void) | undefined;
    let unlistenState: (() => void) | undefined;

    void (async () => {
      try {
        const refreshCurrent = () => refreshRef.current(pageRef.current);
        const onResult = await tauriListen<TextItem>("voice://result", () =>
          refreshCurrent(),
        );
        const onUpdate = await tauriListen<TextItem>("voice://update", (item) => {
          const known = itemsRef.current.some((entry) => entry.id === item.id);
          if (known) {
            setItems((prev) => {
              const index = prev.findIndex((existing) => existing.id === item.id);
              if (index === -1) return prev;
              const next = prev.slice();
              next[index] = item;
              return next;
            });
          } else {
            refreshCurrent();
          }
        });
        const onRemoved = await tauriListen<string>("text://removed", () =>
          refreshCurrent(),
        );
        const onCleared = await tauriListen<string>("text://cleared", () =>
          refreshCurrent(),
        );
        const onState = await tauriListen<VoiceStatePayload>(
          "voice://state",
          (payload) => {
            setPhase(payload.state);
            setPhaseMessage(payload.message ?? null);
          },
        );
        if (disposed) {
          onResult();
          onUpdate();
          onRemoved();
          onCleared();
          onState();
          return;
        }
        unlistenResult = onResult;
        unlistenUpdate = onUpdate;
        unlistenRemoved = onRemoved;
        unlistenCleared = onCleared;
        unlistenState = onState;
      } catch (error) {
        console.warn("[clay-mic] subscribe voice events failed:", error);
      }
    })();

    return () => {
      disposed = true;
      unlistenResult?.();
      unlistenUpdate?.();
      unlistenRemoved?.();
      unlistenCleared?.();
      unlistenState?.();
    };
  }, []);

  const handleInject = useCallback(async (item: TextItem) => {
    try {
      await tauriInvoke("inject_text", {
        text: item.formatted_text || item.raw_text,
      });
    } catch (error) {
      console.error("[clay-mic] inject failed:", error);
    }
  }, []);

  const handleRetry = useCallback(async (item: TextItem) => {
    try {
      await tauriInvoke("retry_item", { id: item.id });
    } catch (error) {
      console.error("[clay-mic] retry failed:", error);
    }
  }, []);

  const handleDelete = useCallback(async (id: string) => {
    try {
      await tauriInvoke("delete_item", { id });
    } catch (error) {
      console.error("[clay-mic] delete failed:", error);
    }
  }, []);

  const handleClearAll = useCallback(async () => {
    try {
      await tauriInvoke("clear_text_list");
    } catch (error) {
      console.error("[clay-mic] clear all failed:", error);
    }
  }, []);

  // Close the context menu on any outside interaction.
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenu(null);
    };
    window.addEventListener("click", close);
    window.addEventListener("contextmenu", close);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [menu]);

  const busy =
    phase === "recording" || phase === "transcribing" || phase === "formatting";
  const maxPage = Math.max(1, Math.ceil(total / pageSize));
  const scrollListTop = () => {
    document.querySelector<HTMLElement>("[data-scroll]")?.scrollTo({ top: 0 });
  };
  const goToPage = (next: number) => {
    setPage(next);
    refreshPage(next);
    scrollListTop();
  };
  const changePageSize = (size: number) => {
    localStorage.setItem("textlist_page_size", String(size));
    setPageSize(size);
    setPage(1);
    scrollListTop();
  };

  return (
    <div className="space-y-2 max-w-4xl">
      {phase === "error" && (
        <div className="p-3 bg-red-50 rounded-lg text-xs text-red-600 break-all">
          {phaseMessage || "语音处理失败"}
        </div>
      )}

      {total > 0 && (
        <div className="flex items-center justify-end gap-3 text-[11px] text-gray-400">
          <span>共 {total} 条</span>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => goToPage(page - 1)}
              disabled={page <= 1}
              className="px-2 py-1 rounded border border-gray-200 bg-white hover:bg-gray-50 disabled:text-gray-300 disabled:bg-gray-50 transition-colors"
              aria-label="上一页"
            >
              ‹
            </button>
            <span className="tabular-nums">
              第 {page} / {maxPage} 页
            </span>
            <button
              type="button"
              onClick={() => goToPage(page + 1)}
              disabled={page >= maxPage}
              className="px-2 py-1 rounded border border-gray-200 bg-white hover:bg-gray-50 disabled:text-gray-300 disabled:bg-gray-50 transition-colors"
              aria-label="下一页"
            >
              ›
            </button>
            <select
              value={pageSize}
              onChange={(e) => changePageSize(Number(e.target.value))}
              className="text-[11px] bg-white border border-gray-200 rounded px-1 py-0.5"
              aria-label="每页条数"
            >
              <option value={20}>20 条/页</option>
              <option value={30}>30 条/页</option>
              <option value={50}>50 条/页</option>
            </select>
          </div>
        </div>
      )}

      {items.length === 0 && !busy && phase !== "error" ? (
        <div className="flex flex-col items-center justify-center h-64 text-gray-300">
          <svg
            className="w-16 h-16 mb-4 text-gray-200"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={1}
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z"
            />
          </svg>
          <p className="text-sm font-medium text-gray-400">暂无处理结果</p>
          <p className="text-xs text-gray-300 mt-1">
            按住遥控器语音键开始说话
          </p>
        </div>
      ) : (
        items.map((item) => {
          const state = itemStatus(item);
          const stamp = formatTimestamp(item.timestamp);
          const placeholder = !item.raw_text.trim()
            ? "等待转写…"
            : state.tone === "busy"
              ? "正在改写…"
              : state.tone === "muted"
                ? "未配置 LLM"
                : "无改写结果";
          return (
            <div
              key={item.id}
              onContextMenu={(event) => {
                event.preventDefault();
                event.stopPropagation();
                setMenu({
                  x: Math.min(event.clientX, window.innerWidth - 140),
                  y: Math.min(event.clientY, window.innerHeight - 56),
                  id: item.id,
                });
              }}
              className="bg-white rounded-xl border border-gray-200 p-4 hover:border-gray-300 transition-colors"
            >
              <div className="flex items-stretch gap-4">
                <div className="flex-1 min-w-0 rounded-lg bg-gray-50 border border-gray-100 p-3">
                  <div className="flex items-center justify-between gap-2 mb-1.5">
                    <span className="text-[10px] tracking-wide text-gray-400 font-semibold">
                      STT 原始结果
                      {item.stt_ms > 0 && (
                        <span className="text-gray-300 font-normal">
                          {" · "}
                          {formatDuration(item.stt_ms)}
                        </span>
                      )}
                    </span>
                    <CopyButton text={item.raw_text} disabled={!item.raw_text} />
                  </div>
                  <div className="text-[13px] text-gray-500 leading-relaxed whitespace-pre-wrap break-words">
                    {item.raw_text || "—"}
                  </div>
                </div>

                <div className="flex flex-col items-center justify-center shrink-0 gap-1.5 px-1">
                  <time
                    title={stamp.title}
                    className="text-[10px] text-gray-300 font-medium whitespace-nowrap"
                  >
                    {stamp.text}
                  </time>
                  <span
                    title={state.error ?? state.label}
                    className={`text-[10px] px-2 py-1 rounded-full font-medium text-center whitespace-nowrap ${TONE_CLASS[state.tone]}`}
                  >
                    {state.label}
                  </span>
                  {state.tone === "fail" && item.raw_text.trim() !== "" && (
                    <button
                      onClick={() => void handleRetry(item)}
                      title={state.error ?? "重试"}
                      className="text-[10px] px-2 py-1 rounded-full font-medium text-red-600 bg-red-50 hover:bg-red-100 transition-colors whitespace-nowrap"
                    >
                      重试
                    </button>
                  )}
                </div>

                <div className="flex-1 min-w-0 rounded-lg bg-blue-50 border border-blue-100 p-3">
                  <div className="flex items-center justify-between gap-2 mb-1.5">
                    <span className="text-[10px] tracking-wide text-blue-400 font-semibold">
                      LLM 改写结果
                      {item.llm_ms > 0 && (
                        <span className="text-blue-300 font-normal">
                          {" · "}
                          {item.thinking_ms > 0
                            ? `思考 ${formatDuration(item.thinking_ms)} · `
                            : ""}
                          {item.llm_ttft_ms > 0
                            ? `首字 ${formatDuration(item.llm_ttft_ms)} · 生成 ${formatDuration(item.llm_gen_ms)} · 共 ${formatDuration(item.llm_ms)}`
                            : `共 ${formatDuration(item.llm_ms)}`}
                        </span>
                      )}
                    </span>
                    <div className="flex items-center gap-0.5 shrink-0">
                      <CopyButton
                        text={item.formatted_text}
                        disabled={!item.formatted_text}
                      />
                      <button
                        onClick={() => handleInject(item)}
                        className="p-1 text-blue-400 hover:text-blue-700 hover:bg-white rounded transition-colors shrink-0"
                        title="注入到输入框"
                      >
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
                            d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8"
                          />
                        </svg>
                      </button>
                    </div>
                  </div>
                  {item.reasoning_text && (
                    <ReasoningBlock
                      text={item.reasoning_text}
                      thinking={state.tone === "busy" && !item.formatted_text}
                    />
                  )}
                  {item.formatted_text ? (
                    <div className="text-[13px] text-gray-700 leading-relaxed whitespace-pre-wrap break-words">
                      {item.formatted_text}
                    </div>
                  ) : (
                    <div className="text-[13px] text-gray-300 italic">
                      {placeholder}
                    </div>
                  )}
                </div>
              </div>
            </div>
          );
        })
      )}

      {menu && (
        <div
          className="fixed z-50 min-w-[120px] rounded-lg border border-gray-200 bg-white shadow-lg py-1"
          style={{ left: menu.x, top: menu.y }}
          onClick={(event) => event.stopPropagation()}
          onContextMenu={(event) => event.preventDefault()}
        >
          <button
            onClick={() => {
              void handleDelete(menu.id);
              setMenu(null);
            }}
            className="w-full text-left px-3 py-1.5 text-[12px] text-red-600 hover:bg-red-50 transition-colors"
          >
            删除
          </button>
          <button
            onClick={() => {
              setConfirmClear(true);
              setMenu(null);
            }}
            className="w-full text-left px-3 py-1.5 text-[12px] text-red-600 hover:bg-red-50 transition-colors"
          >
            清空
          </button>
        </div>
      )}

      {confirmClear && (
        <ConfirmDialog
          message="确定要清空所有文本吗？此操作不可撤销。"
          onConfirm={() => {
            void handleClearAll();
            setConfirmClear(false);
          }}
          onCancel={() => setConfirmClear(false)}
        />
      )}
    </div>
  );
}

function ConfirmDialog({
  message,
  onConfirm,
  onCancel,
}: {
  message: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/30"
      onClick={onCancel}
    >
      <div
        className="bg-white rounded-xl shadow-lg border border-gray-200 p-5 w-72"
        onClick={(e) => e.stopPropagation()}
      >
        <p className="text-sm text-gray-700 mb-4">{message}</p>
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-[12px] text-gray-500 hover:bg-gray-100 rounded-lg transition-colors"
          >
            取消
          </button>
          <button
            onClick={onConfirm}
            className="px-3 py-1.5 text-[12px] text-white bg-red-500 hover:bg-red-600 rounded-lg transition-colors"
          >
            确定
          </button>
        </div>
      </div>
    </div>
  );
}

function ReasoningBlock({ text, thinking }: { text: string; thinking: boolean }) {
  const [open, setOpen] = useState(thinking);
  useEffect(() => {
    setOpen(thinking);
  }, [thinking]);
  return (
    <div className="mb-2 rounded-lg border border-indigo-100 bg-white/70">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="w-full flex items-center gap-1.5 px-2 py-1.5 text-[11px] font-medium text-indigo-500 hover:bg-indigo-50/70 rounded-lg transition-colors"
      >
        <svg
          className={`w-3 h-3 shrink-0 transition-transform ${open ? "rotate-90" : ""}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
        </svg>
        {thinking ? "思考中…" : "思考过程"}
      </button>
      {open && (
        <div className="max-h-40 overflow-y-auto px-2.5 pb-2 text-[12px] leading-relaxed text-gray-500 whitespace-pre-wrap break-words">
          {text}
        </div>
      )}
    </div>
  );
}
