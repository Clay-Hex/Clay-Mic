#!/usr/bin/env node
// Synchronize the version from package.json to all other version-bearing files.
//
//   node scripts/sync-version.mjs          # sync (write)
//   node scripts/sync-version.mjs --check  # dry-run: exit 1 if any drift
//
// Pure sync logic is exported for testability.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// ---------------------------------------------------------------------------
// Target descriptors
// ---------------------------------------------------------------------------

/** Paths relative to project root for each file that carries a version. */
const TARGETS = {
  tauriConf: "src-tauri/tauri.conf.json",
  cargoMain: "src-tauri/Cargo.toml",
  cargoKeyslots: "src-tauri/keyslots/Cargo.toml",
  cargoTapDll: "src-tauri/tap-dll/Cargo.toml",
  cargoLock: "src-tauri/Cargo.lock",
};

/** Workspace members whose `version` lives inside Cargo.lock. */
const LOCK_MEMBERS = ["clay-mic", "clay-mic-keyslots", "clay-tap"];

// ---------------------------------------------------------------------------
// Pure helpers (exported for testing)
// ---------------------------------------------------------------------------

/**
 * Read the canonical version from package.json.
 * @param {(path: string) => string} read  – file reader callback
 * @param {string} root                     – project root
 * @returns {string} semver string, e.g. "0.1.0"
 */
export function getVersion(read, root) {
  const pkg = JSON.parse(read(join(root, "package.json")));
  return pkg.version;
}

/**
 * Replace the version in a tauri.conf.json string.
 * Uses line-level regex to preserve all other formatting exactly.
 * @param {string} content  – raw file content
 * @param {string} version  – desired version
 * @returns {string} updated content (or identical string if already correct)
 */
export function patchTauriConf(content, version) {
  const re = /^(\s*"version"\s*:\s*)"[^"]*"/m;
  if (!re.test(content)) return content;
  const replaced = content.replace(re, `$1"${version}"`);
  if (replaced === content) return content; // idempotent: zero diff
  return replaced;
}

/**
 * Replace the *first* `version = "…"` line in a Cargo.toml string.
 * Only touches the [package] section's version; dependency versions are untouched.
 * @param {string} content  – raw file content
 * @param {string} version  – desired version
 * @returns {string} updated content (or identical string if already correct)
 */
export function patchCargoToml(content, version) {
  // Match the very first `version = "…"` in the file (always under [package]).
  const re = /^(version\s*=\s*)"[^"]*"/m;
  if (!re.test(content)) return content; // no version line found – leave untouched
  const replaced = content.replace(re, `$1"${version}"`);
  if (replaced === content) return content; // idempotent: zero diff
  return replaced;
}

/**
 * Replace the version of every workspace member inside Cargo.lock.
 * Each member's `name = "X"` line is immediately followed by its
 * `version = "…"` line; only those pairs are touched, dependency
 * versions stay untouched. CRLF line endings are preserved.
 * @param {string} content  – raw Cargo.lock content
 * @param {string} version  – desired version
 * @returns {string} updated content (or identical string if already correct)
 */
export function patchCargoLock(content, version) {
  let out = content;
  for (const member of LOCK_MEMBERS) {
    const re = new RegExp(
      `(name = "${member}"\\r?\\nversion = ")[^"]*(")`,
      "g",
    );
    out = out.replace(re, `$1${version}$2`);
  }
  return out;
}

/**
 * Given a version and file contents, compute the list of per-file results.
 * Each result: { key, path, current, desired, ok, patched }
 *
 * @param {string} version  – desired version
 * @param {Record<string, string>} files  – map of target key → raw file content
 * @returns {Array<{key:string, path:string, current:string, desired:string, ok:boolean, patched:string}>}
 */
export function computeEdits(version, files) {
  const results = [];

  // tauri.conf.json
  const tauriContent = files.tauriConf;
  if (tauriContent !== undefined) {
    const re = /^\s*("version"\s*:\s*)"([^"]*)"/m;
    const m = tauriContent.match(re);
    const current = m ? m[2] : "(missing)";
    const patched = patchTauriConf(tauriContent, version);
    results.push({
      key: "tauriConf",
      path: TARGETS.tauriConf,
      current,
      desired: version,
      ok: current === version,
      patched,
    });
  }

  // Cargo.toml files
  for (const key of ["cargoMain", "cargoKeyslots", "cargoTapDll"]) {
    const content = files[key];
    if (content === undefined) continue;
    const re = /^(version\s*=\s*)"([^"]*)"/m;
    const m = content.match(re);
    const current = m ? m[2] : "(missing)";
    const patched = patchCargoToml(content, version);
    results.push({
      key,
      path: TARGETS[key],
      current,
      desired: version,
      ok: current === version,
      patched,
    });
  }

  // Cargo.lock — one result row per workspace member, shared file path.
  const lockContent = files.cargoLock;
  if (lockContent !== undefined) {
    const patched = patchCargoLock(lockContent, version);
    for (const member of LOCK_MEMBERS) {
      const re = new RegExp(
        `name = "${member}"\\r?\\nversion = "([^"]*)"`,
      );
      const m = lockContent.match(re);
      const current = m ? m[1] : "(missing)";
      results.push({
        key: `cargoLock:${member}`,
        path: TARGETS.cargoLock,
        label: `${TARGETS.cargoLock} (${member})`,
        current,
        desired: version,
        ok: current === version,
        patched,
      });
    }
  }

  return results;
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

function main() {
  const checkMode = process.argv.includes("--check");

  const read = (p) => readFileSync(p, "utf8");
  const version = getVersion(read, root);

  // Read all target files
  const files = {};
  for (const [key, rel] of Object.entries(TARGETS)) {
    try {
      files[key] = read(join(root, rel));
    } catch {
      // File missing – computeEdits will handle it
    }
  }

  const results = computeEdits(version, files);
  const fieldCount = results.length + 1; // + package.json itself

  if (checkMode) {
    const drifted = results.filter((r) => !r.ok);
    if (drifted.length === 0) {
      console.log(`✓ All ${fieldCount} version fields are "${version}" — no drift.`);
      process.exit(0);
    }
    console.error(`✗ Version drift detected (package.json = "${version}"):\n`);
    console.error(
      "  File".padEnd(44) + "Current".padEnd(14) + "Expected",
    );
    console.error("  " + "─".repeat(60));
    for (const r of drifted) {
      console.error(
        `  ${(r.label ?? r.path).padEnd(42)} ${r.current.padEnd(13)} ${r.desired}`,
      );
    }
    console.error(
      `\nRun \`npm run version:sync\` to fix, or \`npm version patch\` to bump.`,
    );
    process.exit(1);
  }

  // Sync mode: write files that differ (Cargo.lock rows share one path, so
  // dedupe — every row's `patched` already contains all member updates).
  const write = (p, c) => writeFileSync(p, c, "utf8");
  const writtenPaths = new Set();
  for (const r of results) {
    if (r.ok || writtenPaths.has(r.path)) continue;
    write(join(root, r.path), r.patched);
    writtenPaths.add(r.path);
  }

  if (writtenPaths.size === 0) {
    console.log(
      `All ${fieldCount} version fields already at "${version}" — nothing to do.`,
    );
  } else {
    console.log(
      `Synced version "${version}" to ${[...writtenPaths].join(", ")}`,
    );
  }
}

main();
