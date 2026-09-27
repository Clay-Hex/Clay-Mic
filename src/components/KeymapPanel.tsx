import { useCallback, useEffect, useState } from "react";
import { tauriInvoke } from "../lib/config";

interface Binding {
  action: string;
  key?: string | null;
  command?: string | null;
}

interface KeymapConfig {
  suppress: boolean;
  terminal_exit: string;
  bindings: Record<string, Binding>;
}

const TERMINAL_EXIT_OPTIONS = [
  { id: "keep", label: "执行后保留终端" },
  { id: "close", label: "执行后关闭终端" },
  { id: "auto", label: "自动（return 0 时退出，否则保留）" },
];

const BUTTONS: { id: string; label: string }[] = [
  { id: "mic", label: "麦克风" },
  { id: "back", label: "返回" },
  { id: "ok", label: "确定" },
  { id: "tv", label: "TV" },
  { id: "home", label: "主页" },
  { id: "menu", label: "菜单" },
  { id: "power", label: "电源" },
  { id: "up", label: "上" },
  { id: "down", label: "下" },
  { id: "left", label: "左" },
  { id: "right", label: "右" },
  { id: "volume_up", label: "音量 +" },
  { id: "volume_down", label: "音量 −" },
  { id: "volume_mute", label: "静音" },
];

const ACTIONS = [
  { id: "ignore", label: "忽略（屏蔽）" },
  { id: "pass", label: "放行（系统原生）" },
  { id: "send", label: "发送按键" },
  { id: "exec", label: "执行命令" },
  { id: "custom", label: "自定义" },
];

// App-defined actions behind the「程序自定义」entry. Storage keeps the concrete
// id (e.g. "clear"), never "custom", so keymap.json stays flat and old configs
// keep working; growing this list is all the UI side of a new action needs.
const CUSTOM_ACTIONS = [
  { id: "clear", label: "清空输入框" },
  { id: "backspace", label: "退格" },
];
const CUSTOM_ACTION_IDS = new Set(CUSTOM_ACTIONS.map((action) => action.id));

const TAP_ONLY = new Set(["back", "volume_up", "volume_down"]);

function comboFromEvent(event: KeyboardEvent): string | null {
  const key = event.key;
  if (key === "Control" || key === "Shift" || key === "Alt" || key === "Meta") return null;
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.shiftKey) parts.push("Shift");
  if (event.altKey) parts.push("Alt");
  if (event.metaKey) parts.push("Win");
  let main = key;
  if (key === " ") main = "Space";
  else if (key === "Enter") main = "Enter";
  else if (key === "Escape") main = "Esc";
  else if (key === "Backspace") main = "Backspace";
  else if (key === "Delete") main = "Delete";
  else if (key === "Tab") main = "Tab";
  else if (key.startsWith("Arrow")) main = key.slice(5);
  else if (key.length === 1) main = key.toUpperCase();
  parts.push(main);
  return parts.join("+");
}

export function KeymapPanel({
  onDirtyChange,
  registerSave,
}: {
  onDirtyChange?: (dirty: boolean) => void;
  registerSave?: (save: () => Promise<boolean>) => void;
} = {}) {
  const [config, setConfig] = useState<KeymapConfig | null>(null);
  const [recording, setRecording] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [baseline, setBaseline] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    tauriInvoke<KeymapConfig>("get_keymap")
      .then((keymap) => {
        setConfig(keymap);
        setBaseline(JSON.stringify(keymap));
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (!recording) return;
    const handler = (event: KeyboardEvent) => {
      event.preventDefault();
      const combo = comboFromEvent(event);
      if (!combo) return;
      setConfig((prev) =>
        prev
          ? {
              ...prev,
              bindings: {
                ...prev.bindings,
                [recording]: {
                  action: "send",
                  key: combo,
                },
              },
            }
          : prev,
      );
      setRecording(null);
    };
    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [recording]);

  const setAction = useCallback((button: string, action: string) => {
    setConfig((prev) => {
      if (!prev) return prev;
      const previous = prev.bindings[button];
      const binding: Binding = {
        ...previous,
        action,
        key: action === "send" ? previous?.key : null,
        command: action === "exec" ? previous?.command : null,
      };
      return {
        ...prev,
        bindings: { ...prev.bindings, [button]: binding },
      };
    });
  }, []);

  const setBindingCommand = useCallback((button: string, command: string) => {
    setConfig((prev) => {
      if (!prev) return prev;
      const binding: Binding = {
        ...(prev.bindings[button] ?? { action: "exec" }),
        command,
      };
      return {
        ...prev,
        bindings: { ...prev.bindings, [button]: binding },
      };
    });
  }, []);

  const setTerminalExit = useCallback((value: string) => {
    setConfig((prev) => (prev ? { ...prev, terminal_exit: value } : prev));
  }, []);

  const browseScript = useCallback(async (button: string) => {
    setError(null);
    try {
      const path = await tauriInvoke<string | null>("pick_script");
      if (path) setBindingCommand(button, path);
    } catch (e) {
      setError(String(e));
    }
  }, [setBindingCommand]);

  const saveConfig = useCallback(async (): Promise<boolean> => {
    if (!config) return false;
    setSaveError(null);
    try {
      await tauriInvoke("save_keymap", { config });
      // Clear the dirty marker only once the save actually succeeded.
      setBaseline(JSON.stringify(config));
      return true;
    } catch (e) {
      setSaveError(String(e));
      return false;
    }
  }, [config]);

  const handleSave = () => {
    void saveConfig();
  };

  const dirty = baseline !== null && config !== null && JSON.stringify(config) !== baseline;

  useEffect(() => {
    onDirtyChange?.(dirty);
    // Leaving the page unmounts this component; report clean so a discarded
    // edit cannot leave App's flag stuck on.
    return () => onDirtyChange?.(false);
  }, [dirty, onDirtyChange]);

  useEffect(() => {
    registerSave?.(saveConfig);
  }, [registerSave, saveConfig]);

  if (!config) {
    return (
      <div className="max-w-2xl text-sm text-gray-400">
        {error ? `加载失败：${error}` : "加载中…"}
      </div>
    );
  }

  return (
    <div className="max-w-2xl space-y-5">
      {/* Exec terminal */}
      <div className="bg-white rounded-xl border border-gray-200 p-5 flex items-center justify-between">
        <div className="pr-4">
          <h3 className="text-[13px] font-semibold text-gray-800">命令终端</h3>
          <p className="text-[11px] text-gray-400 mt-0.5">
            「执行命令」打开的终端窗口如何收尾
          </p>
        </div>
        <select
          value={config.terminal_exit}
          onChange={(e) => setTerminalExit(e.target.value)}
          className="px-2 py-1.5 text-[12px] border border-gray-200 rounded-lg bg-white text-gray-700 focus:outline-none focus:border-blue-400 shrink-0"
        >
          {TERMINAL_EXIT_OPTIONS.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label}
            </option>
          ))}
        </select>
      </div>

      {/* Bindings */}
      <div className="bg-white rounded-xl border border-gray-200 overflow-hidden">
        <div className="px-5 py-3.5 border-b border-gray-100">
          <h3 className="text-[13px] font-semibold text-gray-800">按键配置</h3>
          <p className="text-[11px] text-gray-400 mt-0.5">
            选择每个遥控器按键触发的功能；「发送按键」点右侧按钮后按你的键盘录制。
          </p>
        </div>
        <div className="divide-y divide-gray-50">
          {BUTTONS.map((button) => {
            const binding = config.bindings[button.id] ?? { action: "ignore" };
            const actionValue = ACTIONS.some(
              (option) => option.id === binding.action,
            )
              ? binding.action
              : CUSTOM_ACTION_IDS.has(binding.action)
                ? "custom"
                : "ignore";
            const isSend = binding.action === "send";
            const isExec = binding.action === "exec";
            const isRecording = recording === button.id;
            return (
              <div
                key={button.id}
                className="px-5 py-2.5 flex items-center gap-3"
              >
                <span className="w-16 text-[13px] text-gray-700 shrink-0">
                  {button.label}
                </span>
                {button.id === "mic" ? (
                  <span className="px-2 py-1.5 text-[12px] text-gray-500">
                    语音（固定）
                  </span>
                ) : (
                  <>
                    <select
                      value={actionValue}
                      onChange={(e) => {
                        const value = e.target.value;
                        if (value === "custom") {
                          // Reveal the sub-list; bind the first custom action
                          // right away so the row shows a concrete choice the
                          // user can change (or cancel by not saving) — a
                          // pending-only state would need extra per-row UI.
                          if (!CUSTOM_ACTION_IDS.has(binding.action)) {
                            setAction(button.id, CUSTOM_ACTIONS[0].id);
                          }
                          return;
                        }
                        setAction(button.id, value);
                      }}
                      className="px-2 py-1.5 text-[12px] border border-gray-200 rounded-lg bg-white text-gray-700 focus:outline-none focus:border-blue-400"
                    >
                      {ACTIONS.map((action) => (
                        <option key={action.id} value={action.id}>
                          {action.label}
                        </option>
                      ))}
                    </select>
                    {actionValue === "custom" && (
                      <select
                        value={
                          CUSTOM_ACTION_IDS.has(binding.action)
                            ? binding.action
                            : CUSTOM_ACTIONS[0].id
                        }
                        onChange={(e) => setAction(button.id, e.target.value)}
                        className="px-2 py-1.5 text-[12px] border border-gray-200 rounded-lg bg-white text-gray-700 focus:outline-none focus:border-blue-400"
                      >
                        {CUSTOM_ACTIONS.map((action) => (
                          <option key={action.id} value={action.id}>
                            {action.label}
                          </option>
                        ))}
                      </select>
                    )}
                  </>
                )}
                {isSend &&
                  (isRecording ? (
                    <>
                      <span className="px-2.5 py-1.5 text-[12px] rounded-lg font-mono bg-blue-600 text-white">
                        按下按键…
                      </span>
                      <button
                        onClick={() => setRecording(null)}
                        className="px-2.5 py-1.5 text-[12px] rounded-lg bg-gray-50 text-gray-600 hover:bg-gray-100 transition-colors"
                      >
                        取消
                      </button>
                    </>
                  ) : (
                    <button
                      onClick={() => setRecording(button.id)}
                      className="px-2.5 py-1.5 text-[12px] rounded-lg font-mono bg-gray-50 text-gray-600 hover:bg-gray-100 transition-colors"
                    >
                      {binding.key || "未录制"}
                    </button>
                  ))}
                {isExec && (
                  <>
                    <input
                      value={binding.command ?? ""}
                      onChange={(e) =>
                        setBindingCommand(button.id, e.target.value)
                      }
                      placeholder="命令，或点「浏览」选脚本"
                      spellCheck={false}
                      className="flex-1 min-w-0 px-2 py-1.5 text-[12px] border border-gray-200 rounded-lg bg-white text-gray-700 focus:outline-none focus:border-blue-400"
                    />
                    <button
                      onClick={() => void browseScript(button.id)}
                      className="shrink-0 px-2.5 py-1.5 text-[12px] rounded-lg bg-gray-50 text-gray-600 hover:bg-gray-100 transition-colors"
                    >
                      浏览
                    </button>
                  </>
                )}
                {TAP_ONLY.has(button.id) && binding.action !== "pass" && (
                  <span className="text-[10px] px-1.5 py-0.5 bg-amber-50 text-amber-600 rounded">
                    需 Tap
                  </span>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {error && (
        <div className="p-3 bg-red-50 rounded-lg text-xs text-red-600 break-all">
          {error}
        </div>
      )}

      <div className="sticky bottom-0 bg-gray-50 pt-3">
        <button
          onClick={handleSave}
          className="w-full py-2.5 bg-blue-600 text-white text-sm font-medium rounded-lg hover:bg-blue-700 active:bg-blue-800 transition-colors"
        >
          {dirty ? "* 保存配置" : "保存配置"}
        </button>
      </div>

      {saveError && (
        <ErrorDialog
          message={`保存失败：${saveError}`}
          onClose={() => setSaveError(null)}
        />
      )}
    </div>
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
