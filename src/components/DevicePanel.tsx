import { useState, useEffect, useRef, useCallback } from "react";
import { Config, defaultConfig, tauriInvoke } from "../lib/config";

interface DevicePanelProps {
  onStatusChange: (
    status: "disconnected" | "connecting" | "connected" | "streaming",
  ) => void;
}

interface PairedDevice {
  id: string;
  name: string;
}

interface ConnectedDevice {
  name: string;
  vendor_id: number | null;
  product_id: number | null;
  model: string | null;
}

type ConnectionState = "idle" | "connecting" | "connected" | "error";

export function DevicePanel({ onStatusChange }: DevicePanelProps) {
  const [state, setState] = useState<ConnectionState>("idle");
  const [devices, setDevices] = useState<PairedDevice[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanned, setScanned] = useState(false);
  const [deviceName, setDeviceName] = useState<string | null>(null);
  const [deviceModel, setDeviceModel] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [config, setConfig] = useState<Config>(defaultConfig);
  const [suppress, setSuppress] = useState(false);
  const [suppressBusy, setSuppressBusy] = useState(false);

  const persistDevice = useCallback(
    async (
      base: Config,
      device: {
        id: string;
        name: string;
        vendor_id: number | null;
        product_id: number | null;
        model: string | null;
      },
    ) => {
      const updated: Config = {
        ...base,
        device: {
          ...base.device,
          device_id: device.id,
          name: device.name,
          vendor_id: device.vendor_id,
          product_id: device.product_id,
          model: device.model,
        },
      };
      setConfig(updated);
      try {
        await tauriInvoke("update_config", { config: updated });
      } catch (e) {
        console.error("[clay-mic] persist config failed:", e);
      }
    },
    [],
  );

  useEffect(() => {
    tauriInvoke<{ suppress: boolean }>("get_keymap")
      .then((keymap) => setSuppress(keymap.suppress))
      .catch((e) => console.warn("[clay-mic] get_keymap failed:", e));
  }, []);

  const toggleSuppress = useCallback(async () => {
    const next = !suppress;
    setSuppress(next);
    setSuppressBusy(true);
    try {
      // Applies immediately: the backend starts/stops HID capture from this.
      await tauriInvoke("set_suppression_enabled", { enabled: next });
    } catch (e) {
      console.error("[clay-mic] set_suppression_enabled failed:", e);
      setSuppress(!next);
    } finally {
      setSuppressBusy(false);
    }
  }, [suppress]);

  const connectTo = useCallback(
    async (device: PairedDevice, base: Config, automatic = false) => {
      setState("connecting");
      setErrorMsg(null);
      onStatusChange("connecting");
      try {
        const connected = await tauriInvoke<ConnectedDevice>("connect_device", {
          address: device.id,
        });
        const label = connected.name || device.name;
        setDeviceName(label);
        setDeviceModel(connected.model);
        setSelectedId(device.id);
        setState("connected");
        onStatusChange("connected");
        await persistDevice(base, {
          id: device.id,
          name: label,
          vendor_id: connected.vendor_id,
          product_id: connected.product_id,
          model: connected.model,
        });
      } catch (e) {
        console.error("[clay-mic] connect error:", e);
        setState("error");
        onStatusChange("disconnected");
        setErrorMsg(
          automatic ? `自动连接已保存的设备失败：${String(e)}` : String(e),
        );
      }
    },
    [onStatusChange, persistDevice],
  );

  // Load saved config once and auto-reconnect to the remembered device.
  // Guarded by a ref so React StrictMode's double-invoke does not reconnect twice.
  const didInit = useRef(false);
  useEffect(() => {
    if (didInit.current) return;
    didInit.current = true;
    (async () => {
      try {
        const cfg = await tauriInvoke<Config>("get_config");
        setConfig(cfg);
        const savedId = cfg.device?.device_id;
        if (savedId) {
          setSelectedId(savedId);
          const savedName = cfg.device?.name || "已保存设备";
          await connectTo({ id: savedId, name: savedName }, cfg, true);
        }
      } catch (e) {
        console.error("[clay-mic] load config failed:", e);
      }
    })();
  }, [connectTo]);

  const refreshDevices = useCallback(async () => {
    setScanning(true);
    setErrorMsg(null);
    try {
      const list = await tauriInvoke<PairedDevice[]>("list_paired_devices");
      const sorted = [...list].sort((a, b) => a.name.localeCompare(b.name));
      setDevices(sorted);
      setScanned(true);
      setSelectedId((current) => {
        if (current && sorted.some((d) => d.id === current)) return current;
        return sorted[0]?.id ?? null;
      });
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setScanning(false);
    }
  }, []);

  const selectedDevice = devices.find((d) => d.id === selectedId) ?? null;
  const isConnected = state === "connected";

  const handleConnect = () => {
    if (!selectedDevice) {
      setErrorMsg("请先选择要连接的设备");
      return;
    }
    void connectTo(selectedDevice, config);
  };

  return (
    <div className="max-w-md space-y-5">
      {/* Connection Card */}
      <div className="bg-white rounded-xl border border-gray-200 overflow-hidden">
        <div className="px-5 py-4 border-b border-gray-100 flex items-center justify-between">
          <div>
            <h2 className="text-[15px] font-semibold text-gray-800">遥控器</h2>
            <p className="text-xs text-gray-400 mt-0.5">
              仅显示支持语音（ATVV）的已配对设备
            </p>
          </div>
          <button
            onClick={refreshDevices}
            disabled={scanning || state === "connecting"}
            className="flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] font-medium text-blue-600 bg-blue-50 rounded-lg hover:bg-blue-100 disabled:text-gray-300 disabled:bg-gray-50 transition-colors"
          >
            <svg
              className={`w-3.5 h-3.5 ${scanning ? "animate-spin" : ""}`}
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99"
              />
            </svg>
            {scanning ? "扫描中..." : "刷新设备"}
          </button>
        </div>

        <div className="p-5 space-y-4">
          {/* Connected status */}
          <div
            className={`flex items-center justify-between p-3 rounded-lg transition-colors ${
              isConnected ? "bg-emerald-50" : "bg-gray-50"
            }`}
          >
            <div className="flex items-center gap-3">
              <div
                className={`w-9 h-9 rounded-lg flex items-center justify-center ${
                  isConnected ? "bg-emerald-100" : "bg-gray-100"
                }`}
              >
                <svg
                  className={`w-5 h-5 ${
                    isConnected ? "text-emerald-600" : "text-gray-400"
                  }`}
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={1.8}
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M8.288 15.038a5.25 5.25 0 017.424 0M5.106 11.856c3.807-3.808 9.98-3.808 13.788 0M1.924 8.674c5.565-5.565 14.587-5.565 20.152 0M12.53 18.22l-.53.53-.53-.53a.75.75 0 011.06 0z"
                  />
                </svg>
              </div>
              <div>
                <div className="flex items-center gap-1.5">
                  <p className="text-sm font-medium text-gray-800">
                    {isConnected
                      ? deviceName || "已连接"
                      : state === "connecting"
                        ? "连接中..."
                        : "未连接设备"}
                  </p>
                  {isConnected && deviceModel && (
                    <span className="text-[10px] px-1.5 py-0.5 bg-gray-100 text-gray-500 rounded font-medium shrink-0">
                      {deviceModel}
                    </span>
                  )}
                </div>
                <p className="text-[11px] text-gray-400">
                  {isConnected ? "语音输入就绪" : "刷新设备列表后选择并连接"}
                </p>
              </div>
            </div>
            <div
              className={`w-2 h-2 rounded-full ${
                isConnected
                  ? "bg-emerald-500"
                  : state === "connecting"
                    ? "bg-amber-400 animate-pulse"
                    : "bg-gray-300"
              }`}
            />
          </div>

          {/* Error */}
          {errorMsg && (
            <div className="p-3 bg-red-50 rounded-lg text-xs text-red-600 break-all">
              {errorMsg}
            </div>
          )}

          {/* Device list */}
          {scanned && devices.length === 0 && (
            <div className="p-3 bg-amber-50 rounded-lg text-xs text-amber-700 leading-relaxed">
              没有找到支持语音的已配对蓝牙设备。请先在 Windows「设置 →
              蓝牙」中配对遥控器（同时长按遥控器「主页 +
              菜单」进入配对），然后点击「刷新设备」。
            </div>
          )}

          {devices.length > 0 && (
            <div className="space-y-1.5">
              <label className="block text-[12px] text-gray-500 font-medium">
                已配对设备
              </label>
              <div className="space-y-1.5 max-h-56 overflow-y-auto">
                {devices.map((device) => {
                  const selected = device.id === selectedId;
                  return (
                    <button
                      key={device.id}
                      onClick={() => setSelectedId(device.id)}
                      disabled={isConnected || state === "connecting"}
                      className={`w-full flex items-center gap-3 p-3 rounded-lg border text-left transition-colors disabled:cursor-not-allowed ${
                        selected
                          ? "border-blue-400 bg-blue-50"
                          : "border-gray-200 hover:border-gray-300 hover:bg-gray-50"
                      }`}
                    >
                      <span
                        className={`w-4 h-4 rounded-full border-2 flex items-center justify-center shrink-0 ${
                          selected
                            ? "border-blue-500 bg-blue-500"
                            : "border-gray-300"
                        }`}
                      >
                        {selected && (
                          <svg
                            className="w-2.5 h-2.5 text-white"
                            fill="none"
                            viewBox="0 0 24 24"
                            stroke="currentColor"
                            strokeWidth={4}
                          >
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              d="M4.5 12.75l6 6 9-13.5"
                            />
                          </svg>
                        )}
                      </span>
                      <span className="flex-1 min-w-0">
                        <span className="text-[13px] font-medium text-gray-800 truncate">
                          {device.name}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {/* Connect Button */}
          <button
            onClick={handleConnect}
            disabled={state === "connecting" || isConnected || !selectedDevice}
            className={`w-full py-2.5 px-4 text-sm font-medium rounded-lg transition-colors ${
              isConnected
                ? "bg-emerald-50 text-emerald-600 cursor-default"
                : state === "connecting"
                  ? "bg-gray-100 text-gray-400 cursor-wait"
                  : !selectedDevice
                    ? "bg-gray-100 text-gray-400 cursor-not-allowed"
                    : "bg-blue-600 text-white hover:bg-blue-700 active:bg-blue-800"
            }`}
          >
            {state === "connecting" ? (
              <span className="flex items-center justify-center gap-2">
                <svg
                  className="animate-spin w-4 h-4"
                  fill="none"
                  viewBox="0 0 24 24"
                >
                  <circle
                    className="opacity-25"
                    cx="12"
                    cy="12"
                    r="10"
                    stroke="currentColor"
                    strokeWidth="4"
                  />
                  <path
                    className="opacity-75"
                    fill="currentColor"
                    d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                  />
                </svg>
                连接中...
              </span>
            ) : isConnected ? (
              <span className="flex items-center justify-center gap-2">
                <svg
                  className="w-4 h-4"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2}
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M4.5 12.75l6 6 9-13.5"
                  />
                </svg>
                已连接
              </span>
            ) : (
              "连接遥控器"
            )}
          </button>
        </div>
      </div>

      {/* Suppress */}
      <div className="bg-white rounded-xl border border-gray-200 p-5 flex items-center justify-between">
        <div className="pr-4">
          <h3 className="text-[13px] font-semibold text-gray-800">
            屏蔽遥控器按键
          </h3>
          <p className="text-[11px] text-gray-400 mt-0.5 leading-relaxed">
            开启后，遥控器按键按「按键」页的配置执行，不再直接操作电脑。立即生效，
            返回/音量需要 Tap 支持时会自动按需启用（可能弹一次 UAC）。
          </p>
        </div>
        <button
          onClick={() => void toggleSuppress()}
          disabled={suppressBusy}
          className={`relative w-11 h-6 rounded-full transition-colors shrink-0 ${
            suppress ? "bg-blue-600" : "bg-gray-300"
          } disabled:opacity-60`}
          aria-label="切换屏蔽"
        >
          <span
            className={`absolute top-0.5 w-5 h-5 bg-white rounded-full shadow transition-all ${
              suppress ? "left-[22px]" : "left-0.5"
            }`}
          />
        </button>
      </div>

      {/* Help */}
      <div className="bg-white rounded-xl border border-gray-200 p-5">
        <h3 className="text-[13px] font-semibold text-gray-700 mb-3">
          使用方法
        </h3>
        <ol className="space-y-2.5">
          {[
            "同时长按遥控器「主页 + 菜单」进入配对模式",
            "在 Windows「设置 → 蓝牙」中配对遥控器",
            "点击「刷新设备」扫描已配对的蓝牙设备",
            "从列表中选择遥控器并点击「连接遥控器」",
            "按住遥控器语音键说话",
          ].map((step, i) => (
            <li key={i} className="flex items-start gap-2.5">
              <span className="w-5 h-5 rounded-full bg-blue-50 text-blue-600 text-[11px] font-semibold flex items-center justify-center shrink-0 mt-0.5">
                {i + 1}
              </span>
              <span className="text-xs text-gray-600 leading-relaxed">
                {step}
              </span>
            </li>
          ))}
        </ol>
      </div>
    </div>
  );
}
