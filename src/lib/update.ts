import type { DownloadProgress } from "./config";

/**
 * Release metadata behind the about page's update check. Mirrors
 * `UpdateInfo` in `src-tauri/src/update/mod.rs`; `check_update` resolves to
 * `null` when the repository has no releases yet.
 */
export interface UpdateInfo {
  /** Version of the running build, re-filled by the backend on every read. */
  current: string;
  /** Newest released tag without the `v` prefix. */
  latest: string;
  /** Release notes reduced to plain lines (a few lines, no Markdown). */
  notes: string;
  /** ISO timestamp from the release API; empty when the notes fetch failed. */
  published_at: string;
  asset_url: string;
  asset_size: number;
  /** Hex digest of the release checksum; null = this release ships none. */
  sha256: string | null;
  html_url: string;
}

export type UpdatePhase =
  | "idle" // not checked yet
  | "checking"
  | "uptodate"
  | "no-release" // repository has no releases
  | "dev"
  | "available"
  | "downloading"
  | "extracting" // P2
  | "ready" // downloaded and verified
  | "applying" // P2: handed off to the updater
  | "error";

export interface UpdateState {
  phase: UpdatePhase;
  info?: UpdateInfo;
  progress?: DownloadProgress | null;
  error?: string;
}

/** `1.2.3` / `v1.2.3-beta.1` → comparable triple; unusable input → null. */
function versionTuple(raw: string): [number, number, number] | null {
  const core = raw.trim().replace(/^v/, "").split("-")[0];
  const parts = core.split(".");
  if (parts.length < 3) return null;
  const numbers = parts.slice(0, 3).map((part) => Number.parseInt(part, 10));
  if (numbers.some((value) => Number.isNaN(value))) return null;
  return [numbers[0], numbers[1], numbers[2]];
}

/**
 * Compare two versions numerically. Unparseable input compares as equal,
 * which the backend already rejects before the UI ever sees it.
 */
function compareVersions(left: string, right: string): number {
  const a = versionTuple(left);
  const b = versionTuple(right);
  if (!a || !b) return 0;
  for (let index = 0; index < 3; index++) {
    if (a[index] !== b[index]) return a[index] < b[index] ? -1 : 1;
  }
  return 0;
}

/** The phase a fetched release maps to, given the running version. */
export function verdictFor(info: UpdateInfo): UpdatePhase {
  const diff = compareVersions(info.latest, info.current);
  return diff > 0 ? "available" : diff < 0 ? "dev" : "uptodate";
}
