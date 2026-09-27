import { useCallback, useEffect, useState } from "react";
import {
  tauriInvoke,
  tauriListen,
  type DownloadProgress,
  type DriverStatus,
  type FilterChainStatus,
  type TapStatus,
  type KeySlotStatus,
} from "../../lib/config";
import { DownloadBar, Group, Message, MetaRow, Note, SectionBox } from "./common";

let cachedTap: TapStatus | null = null;

export function DriverSect({
  progress,
  onProgressEnd,
}: {
  progress: DownloadProgress | null;
  onProgressEnd: () => void;
}) {
  const [status, setStatus] = useState<DriverStatus | null>(null);
  const [filterStatus, setFilterStatus] = useState<FilterChainStatus | null>(null);
  const [keySlot, setKeySlot] = useState<KeySlotStatus | null>(null);
  const [tap, setTapState] = useState<TapStatus | null>(cachedTap);
  const setTap = useCallback((value: TapStatus | null) => {
    cachedTap = value;
    setTapState(value);
  }, []);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [tapMessage, setTapMessage] = useState<string | null>(null);
  const [tapBusy, setTapBusy] = useState(false);
  const [confirmUninstall, setConfirmUninstall] = useState(false);

  const refresh = useCallback(() => {
    tauriInvoke<DriverStatus>("get_interception_status")
      .then(setStatus)
      .catch((error) =>
        console.warn("[clay-mic] interception status failed:", error),
      );
    tauriInvoke<FilterChainStatus>("get_filter_chain_status")
      .then(setFilterStatus)
      .catch((error) =>
        console.warn("[clay-mic] filter chain status failed:", error),
      );
    tauriInvoke<KeySlotStatus>("get_keyslot_status")
      .then(setKeySlot)
      .catch((error) => console.warn("[clay-mic] keyslot status failed:", error));
    tauriInvoke<TapStatus>("get_tap_status")
      .then(setTap)
      .catch((error) => console.warn("[clay-mic] tap status failed:", error));
  }, [setTap]);

  useEffect(() => {
    refresh();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriListen<TapStatus>("tap://status", setTap).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refresh, setTap]);

  const runTap = useCallback(
    async (label: string, action: () => Promise<unknown>) => {
      setTapBusy(true);
      setTapMessage(null);
      try {
        await action();
        setTapMessage(`${label}完成`);
        refresh();
      } catch (error) {
        setTapMessage(`${label}失败：${String(error)}`);
        refresh();
      } finally {
        setTapBusy(false);
        onProgressEnd();
      }
    },
    [refresh, onProgressEnd],
  );

  const runAction = useCallback(
    async (label: string, action: () => Promise<unknown>) => {
      setBusy(true);
      setMessage(null);
      try {
        await action();
        setMessage(`${label}完成`);
        refresh();
      } catch (error) {
        setMessage(`${label}失败：${String(error)}`);
      } finally {
        setBusy(false);
        onProgressEnd();
      }
    },
    [refresh, onProgressEnd],
  );

  const runRepair = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const result = await tauriInvoke<string>("repair_driver_pair");
      setMessage(result);
      refresh();
    } catch (error) {
      setMessage(`检查并修复失败：${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const runRepairPatch = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const result = await tauriInvoke<string>("repair_patch");
      setMessage(result);
      refresh();
    } catch (error) {
      setMessage(`修复补丁失败：${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const dllFound = status?.dll_found ?? false;
  const driverReady = status?.driver_ready ?? false;
  const patchOk = Boolean(
    keySlot?.service_installed &&
      keySlot?.applied_unix != null &&
      keySlot?.ok === true,
  );
  const blueButton =
    "px-2.5 py-1.5 text-[12px] font-medium text-blue-600 bg-blue-50 rounded-lg hover:bg-blue-100 disabled:text-gray-300 disabled:bg-gray-50 transition-colors";
  const redButton =
    "px-2.5 py-1.5 text-[12px] font-medium text-red-600 bg-red-50 rounded-lg hover:bg-red-100 disabled:text-gray-300 disabled:bg-gray-50 transition-colors";
  const grayButton =
    "px-2.5 py-1.5 text-[12px] font-medium text-gray-600 bg-gray-50 rounded-lg hover:bg-gray-100 disabled:text-gray-300 transition-colors";

  const lastRun = keySlot?.applied_unix
    ? `${new Date(keySlot.applied_unix * 1000).toLocaleString()} · ${
        keySlot.ok ? "成功" : "失败"
      } · 键盘 ${keySlot.keyboard ?? 0} / 鼠标 ${keySlot.pointer ?? 0}`
    : "尚未执行";

  const tapState = tap
    ? tap.injected
      ? tap.client_connected
        ? `运行中 · WUDFHost ${tap.host_pid ?? "?"}`
        : tap.listening
          ? "已注入，等待 UDP 心跳"
          : "已注入 · 监听未启动"
      : !tap.dll_ready
        ? "未找到 clay_tap.dll"
        : tap.host_pid
          ? "未注入"
          : tap.lookup?.includes("进程已退出")
            ? "HostPid 已失效"
            : "等待连接遥控器"
    : "等待连接遥控器";
  const tapStateVersion = tap?.injected
    ? tap.client_connected
      ? tap.dll_version
        ? ` · v${tap.dll_version}`
        : " · 版本未知"
      : ` · v${tap.version}`
    : "";

  return (
    <div className="space-y-4">
      <SectionBox>
        <Group label="状态">
          <MetaRow
            label="驱动"
            tone={driverReady ? "ok" : "warn"}
            value={driverReady ? "运行中" : "未生效"}
          />
          <MetaRow
            label="驱动补丁"
            tone={patchOk ? "ok" : "warn"}
            value={patchOk ? "已生效" : "未生效"}
          />
        </Group>

        <Group label="操作">
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() =>
                void runAction("下载", () => tauriInvoke("download_interception"))
              }
              disabled={busy}
              className={blueButton}
            >
              下载驱动
            </button>
            <button
              onClick={() =>
                void runAction("安装", () =>
                  tauriInvoke("install_driver_and_service"),
                )
              }
              disabled={busy || !status?.installer_found || !keySlot?.helper_found}
              className={blueButton}
            >
              安装驱动和补丁
            </button>
            <button onClick={refresh} disabled={busy} className={grayButton}>
              重新检测
            </button>
          </div>
          {progress?.source === "interception" && (
            <DownloadBar progress={progress} />
          )}
          {message && <Message>{message}</Message>}
        </Group>

        <details className="pt-1">
          <summary className="text-[12px] text-gray-400 cursor-pointer select-none hover:text-gray-600">
            诊断与维护
          </summary>
          <div className="mt-2 space-y-3">
            <div className="space-y-1.5">
              <MetaRow
                label="DLL"
                tone={dllFound ? "ok" : "warn"}
                value={
                  <>
                    {dllFound ? "已找到" : "未找到"}
                    {status?.dll_path && (
                      <span className="font-normal text-gray-400 break-all">
                        {" "}
                        {status.dll_path}
                      </span>
                    )}
                  </>
                }
              />
              <MetaRow
                label="键盘过滤链"
                value={
                  filterStatus?.keyboard.length
                    ? filterStatus.keyboard.join(" ")
                    : "—"
                }
              />
              <MetaRow
                label="鼠标过滤链"
                value={
                  filterStatus?.mouse.length ? filterStatus.mouse.join(" ") : "—"
                }
              />
              <MetaRow
                label="补丁上次执行"
                tone={
                  keySlot && keySlot.applied_unix
                    ? keySlot.ok
                      ? "ok"
                      : "warn"
                    : undefined
                }
                value={lastRun}
              />
              {keySlot?.error && <Message tone="warn">{keySlot.error}</Message>}
            </div>

            <div className="flex flex-wrap gap-2">
              <button
                onClick={() => void runRepair()}
                disabled={busy}
                className={blueButton}
              >
                检查并修复
              </button>
              <button
                onClick={() => void runRepairPatch()}
                disabled={busy}
                className={blueButton}
              >
                修复补丁
              </button>
              <button
                onClick={() => setConfirmUninstall(true)}
                disabled={busy || !status?.installer_found || !filterStatus?.interception_installed}
                className={redButton}
              >
                卸载驱动和补丁
              </button>
            </div>

            <Note>
              该驱动有编号用尽的缺陷，会导致键鼠失灵，失灵时重启即可恢复；驱动补丁可修复该缺陷。
            </Note>
          </div>
        </details>
      </SectionBox>

      <SectionBox>
        <Group label="HID Tap（返回 / 音量）">
          <MetaRow
            label="状态"
            tone={
              tap?.injected
                ? tap.client_connected
                  ? "ok"
                  : "warn"
                : tap && !tap.dll_ready
                  ? "warn"
                  : undefined
            }
            value={`${tapState}${tapStateVersion}`}
          />
          <MetaRow
            label="事件通道"
            tone={tap?.client_connected ? "ok" : tap?.listening ? undefined : "warn"}
            value={
              tap?.client_connected
                ? "已连接 · UDP"
                : tap?.listening
                  ? "等待心跳"
                  : "未启动"
            }
          />
          {tap?.device_identity && (
            <MetaRow label="设备" value={tap.device_identity} />
          )}
          {tap?.lookup && !tap?.host_alive && (
            <MetaRow label="查找" tone="warn" value={tap.lookup} />
          )}
          {tap?.dll_needs_update && (
            <Message tone="warn">
              已注入的组件
              {tap.dll_version ? ` v${tap.dll_version}` : "版本未知"}
              ，当前 v{tap.version}；请先「移除」再「注入」完成更新（可能弹一次
              UAC）。
            </Message>
          )}
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() =>
                void runTap("注入", () => tauriInvoke("inject_tap"))
              }
              disabled={tapBusy || !tap?.dll_ready || !!tap?.injected}
              className={blueButton}
            >
              注入
            </button>
            <button
              onClick={() =>
                void runTap("移除", () => tauriInvoke("remove_tap"))
              }
              disabled={tapBusy || !tap?.injected}
              className={redButton}
            >
              移除
            </button>
          </div>
          {tapMessage && <Message>{tapMessage}</Message>}
          {tap?.last_error && <Message tone="warn">{tap.last_error}</Message>}
          <Note>用于返回 / 音量± 三个特殊按键的拦截</Note>
        </Group>
      </SectionBox>

      {confirmUninstall && (
        <UninstallConfirmDialog
          onCancel={() => setConfirmUninstall(false)}
          onConfirm={() => {
            setConfirmUninstall(false);
            void runAction("卸载", () =>
              tauriInvoke("uninstall_driver_and_service"),
            );
          }}
        />
      )}
    </div>
  );
}

function UninstallConfirmDialog({
  onCancel,
  onConfirm,
}: {
  onCancel: () => void;
  onConfirm: () => void;
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
        <p className="text-sm font-semibold text-gray-800 mb-2">
          卸载驱动和补丁？
        </p>
        <p className="text-[12px] text-gray-500 leading-relaxed mb-4">
          将卸载 Interception 驱动及其补丁（需一次管理员授权，重启后生效）。
          卸载后「按键屏蔽」不可用，直到重新安装。
        </p>
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-[12px] text-gray-600 bg-gray-50 hover:bg-gray-100 rounded-lg transition-colors"
          >
            取消
          </button>
          <button
            onClick={onConfirm}
            className="px-3 py-1.5 text-[12px] text-white bg-red-600 hover:bg-red-700 rounded-lg transition-colors"
          >
            卸载
          </button>
        </div>
      </div>
    </div>
  );
}
