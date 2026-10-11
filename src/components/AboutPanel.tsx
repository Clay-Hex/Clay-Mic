import { useEffect, useRef, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";

import logoUrl from "../../src-tauri/icons/source.svg";
import { tauriInvoke } from "../lib/config";
import {
  verdictFor,
  type UpdateInfo,
  type UpdatePhase,
  type UpdateState,
} from "../lib/update";
import { useDownloadProgress } from "../lib/useDownloadProgress";
import { DownloadBar } from "./settings/common";

/** Fallback link when the check failed before any release URL was known. */
const RELEASES_URL = "https://github.com/Clay-Hex/Clay-Mic/releases";

const PRIMARY_BTN =
  "rounded-lg bg-blue-600 px-3 py-1.5 text-xs font-medium text-white shadow-sm transition-colors hover:bg-blue-700 disabled:bg-gray-200 disabled:text-gray-400";
const SECONDARY_BTN =
  "rounded-lg border border-gray-200 bg-white px-3 py-1.5 text-xs font-medium text-gray-600 transition-colors hover:bg-gray-50 disabled:text-gray-300";

export function AboutPanel() {
  const [version, setVersion] = useState("");
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [info, setInfo] = useState<UpdateInfo | undefined>(undefined);
  const [error, setError] = useState<string | undefined>(undefined);
  const [progress, setProgress] = useDownloadProgress(["update"]);
  // Which action the error state's「重试」runs again.
  const lastAction = useRef<"check" | "download" | "apply">("check");

  // Single render-facing view over the four pieces of state.
  const view: UpdateState = { phase, info, progress, error };
  const releaseUrl = info?.html_url ?? RELEASES_URL;
  const meta = [
    view.info?.asset_size
      ? `${(view.info.asset_size / 1048576).toFixed(1)} MB`
      : "",
    view.info?.published_at ? view.info.published_at.slice(0, 10) : "",
  ]
    .filter(Boolean)
    .join(" · ");

  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch((e) => console.warn("[clay-mic] get version failed:", e));
  }, []);

  const check = async (force: boolean) => {
    lastAction.current = "check";
    setPhase("checking");
    setInfo(undefined);
    setError(undefined);
    try {
      const result = await tauriInvoke<UpdateInfo | null>("check_update", {
        force,
      });
      if (!result) {
        setPhase("no-release");
        return;
      }
      setInfo(result);
      setPhase(verdictFor(result));
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  };

  const download = async () => {
    lastAction.current = "download";
    setPhase("downloading");
    setError(undefined);
    // Optimistic: show the bar before the backend's first event.
    setProgress({ source: "update", phase: "downloading", percent: 0 });
    try {
      await tauriInvoke<string>("download_update");
      setProgress(null);
      setPhase("ready");
    } catch (e) {
      setProgress(null);
      const message = String(e);
      if (message.includes("已取消")) {
        setPhase("available");
        return;
      }
      setError(message);
      setPhase("error");
    }
  };

  const cancelDownload = () => {
    void tauriInvoke("cancel_download").catch((e) =>
      console.warn("[clay-mic] cancel download failed:", e),
    );
  };

  const openUpdatesDir = () => {
    void tauriInvoke("open_updates_dir").catch((e) =>
      console.warn("[clay-mic] open updates dir failed:", e),
    );
  };

  const openUrl = (url: string) => {
    void tauriInvoke("open_url", { url }).catch((e) =>
      console.warn("[clay-mic] open url failed:", e),
    );
  };

  const retry = () => {
    void (lastAction.current === "download" ? download() : check(true));
  };

  // On success the app exits; a pre-flight failure stays on ready.
  const applyNow = async () => {
    lastAction.current = "apply";
    setError(undefined);
    setPhase("applying");
    try {
      await tauriInvoke("apply_update");
    } catch (e) {
      setError(String(e));
      setPhase("ready");
    }
  };

  const checkButton = (
    <button
      type="button"
      disabled={view.phase === "checking"}
      onClick={() => void check(true)}
      className={PRIMARY_BTN}
    >
      {view.phase === "checking" ? "检查中…" : "检查更新"}
    </button>
  );

  let status: ReactNode;
  let actions: ReactNode;
  switch (view.phase) {
    case "checking":
      status = <p className="text-xs text-gray-400">正在检查…</p>;
      actions = checkButton;
      break;
    case "uptodate":
      status = <p className="text-xs text-emerald-600">✓ 已是最新版本</p>;
      actions = checkButton;
      break;
    case "no-release":
      status = <p className="text-xs text-gray-400">仓库暂无发布版本</p>;
      actions = checkButton;
      break;
    case "dev":
      status = <p className="text-xs text-amber-600">ℹ 你正在运行开发版</p>;
      actions = checkButton;
      break;
    case "available":
      status = (
        <div className="space-y-1.5">
          <p className="text-[13px] font-medium text-gray-900">
            发现新版本 v{view.info?.latest}
          </p>
          {view.info?.notes && (
            <p className="max-h-24 overflow-y-auto whitespace-pre-line text-xs leading-relaxed text-gray-500">
              {view.info.notes}
            </p>
          )}
          {meta && <p className="text-[11px] text-gray-400">{meta}</p>}
          <button
            type="button"
            onClick={() => openUrl(releaseUrl)}
            className="text-[11px] text-blue-500 hover:underline"
          >
            查看完整更新日志 →
          </button>
        </div>
      );
      actions = (
        <button
          type="button"
          onClick={() => void download()}
          className={PRIMARY_BTN}
        >
          下载更新
        </button>
      );
      break;
    case "downloading":
      status = view.progress ? (
        <DownloadBar progress={view.progress} />
      ) : (
        <p className="text-xs text-gray-400">下载中…</p>
      );
      actions = (
        <button
          type="button"
          onClick={cancelDownload}
          className={SECONDARY_BTN}
        >
          取消
        </button>
      );
      break;
    case "ready":
      status = (
        <div className="space-y-1">
          <p className="text-xs text-emerald-600">
            ✓ 下载完成{view.info?.sha256 ? "，已校验" : "（该版本未提供校验和）"}
          </p>
          {view.error && (
            <p className="break-all text-xs text-red-500">⚠ {view.error}</p>
          )}
        </div>
      );
      actions = (
        <>
          <button
            type="button"
            onClick={openUpdatesDir}
            className={SECONDARY_BTN}
          >
            打开所在文件夹
          </button>
          <button
            type="button"
            onClick={() => void applyNow()}
            className={PRIMARY_BTN}
          >
            立即更新
          </button>
        </>
      );
      break;
    case "applying":
      status = view.progress ? (
        <DownloadBar progress={view.progress} />
      ) : (
        <p className="text-xs text-gray-400">正在更新，请勿关闭应用…</p>
      );
      actions = null;
      break;
    case "error":
      status = <p className="break-all text-xs text-red-500">⚠ {view.error}</p>;
      actions = (
        <button type="button" onClick={retry} className={PRIMARY_BTN}>
          重试
        </button>
      );
      break;
    default:
      status = <p className="text-xs text-gray-400">检查是否有新版本可用</p>;
      actions = checkButton;
  }

  return (
    <div className="mx-auto flex w-full max-w-md flex-col py-2">
      {/* Hero */}
      <div className="flex flex-col items-center text-center">
        <img
          src={logoUrl}
          alt=""
          className="h-20 w-20 rounded-[22px] shadow-lg ring-1 ring-black/5"
        />
        <h2 className="mt-4 text-lg font-semibold tracking-tight text-gray-900">
          Clay-Mic
        </h2>
        <div className="mt-1.5 flex items-center gap-3">
          {version && (
            <span className="rounded-full bg-gray-100 px-2.5 py-0.5 text-[11px] font-medium text-gray-500">
              v{version}
            </span>
          )}
          <button
            type="button"
            onClick={() => openUrl(releaseUrl)}
            className="text-[11px] font-medium text-blue-500 transition-colors hover:text-blue-600"
          >
            打开发布页
          </button>
        </div>
        <p className="mt-3 text-[13px] leading-relaxed text-gray-500">
          把蓝牙遥控器的语音接到你的 Windows PC
        </p>
        <p className="mt-1 text-xs text-gray-400">本地离线语音输入 · MIT 许可</p>
      </div>

      {/* 版本更新：卡片式，与设置 / 设备页风格一致 */}
      <div className="mt-7 bg-white rounded-xl border border-gray-200 p-5">
        <div className="flex items-center justify-between gap-4">
          <p className="text-[13px] font-semibold text-gray-800">版本更新</p>
          <div className="flex shrink-0 items-center gap-2">{actions}</div>
        </div>
        <div className="mt-2.5">{status}</div>
      </div>
    </div>
  );
}
