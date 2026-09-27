import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";

import logoUrl from "../../src-tauri/icons/source.svg";

export function AboutPanel() {
  const [version, setVersion] = useState("");

  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch((error) => console.warn("[clay-mic] get version failed:", error));
  }, []);

  return (
    <div className="flex min-h-[60vh] flex-col items-center justify-center gap-2 text-center">
      <img
        src={logoUrl}
        alt=""
        className="h-16 w-16 rounded-2xl shadow-lg ring-1 ring-black/5"
      />
      <h2 className="mt-2 text-base font-semibold text-gray-800">Clay-Mic</h2>
      {version && <p className="text-sm text-gray-500">v{version}</p>}
      <p className="max-w-xs text-[13px] leading-relaxed text-gray-600">
        把蓝牙遥控器的语音接到你的 Windows PC
      </p>
      <p className="text-xs text-gray-400">本地离线语音输入 · MIT 许可</p>
    </div>
  );
}
