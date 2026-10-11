import { useEffect, useState } from "react";

import { tauriListen, type DownloadProgress } from "./config";

/**
 * Subscribe to `download://progress` for the given sources only, so one
 * download (e.g. the updater) never drives another page's progress bar.
 *
 * Returns the latest event plus a setter: callers seed it optimistically
 * before the backend's first emit, and a "done" event clears it.
 */
export function useDownloadProgress(
  sources: readonly DownloadProgress["source"][],
): [DownloadProgress | null, (progress: DownloadProgress | null) => void] {
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  // Joined into a primitive: the array literal changes identity every
  // render, which must not re-run the subscription effect.
  const key = sources.join(",");

  useEffect(() => {
    const wanted = key.split(",");
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriListen<DownloadProgress>("download://progress", (payload) => {
      if (!wanted.includes(payload.source)) return;
      setProgress(payload.phase === "done" ? null : payload);
    })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((error) =>
        console.warn("[clay-mic] subscribe progress failed:", error),
      );
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [key]);

  return [progress, setProgress];
}
