// Assemble a clean distribution folder at `packages/<productName>-<version>/`.
// Green-only: no installers — the folder itself (plus a zip) is the release.
//
//   node scripts/package.mjs             build, then package
//   node scripts/package.mjs --no-build  package an existing build
import { spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const skipBuild = process.argv.includes("--no-build");

// --- Version drift gate ---------------------------------------------------
const check = spawnSync("node", ["scripts/sync-version.mjs", "--check"], {
  stdio: "inherit",
  cwd: root,
});
if (check.status !== 0) {
  console.error(
    "\n版本不一致，无法打包。请先运行：npm run version:sync",
  );
  process.exit(1);
}

// --- Build (optional) -----------------------------------------------------
if (!skipBuild) {
  console.log("> tauri build --no-bundle\n");
  const build = spawnSync("tauri build --no-bundle", {
    stdio: "inherit",
    shell: true,
    cwd: root,
  });
  if (build.status !== 0) {
    process.exit(build.status ?? 1);
  }
}

// --- Read config & version ------------------------------------------------
const config = JSON.parse(
  readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const productName = config.productName || "clay-mic";
const version = config.version || "0.0.0";
const releaseDir = join(root, "src-tauri", "target", "release");
const outDir = join(root, "packages", `${productName}-${version}`);

if (!existsSync(releaseDir)) {
  console.error(`找不到构建目录：${releaseDir}（请先运行 tauri build）`);
  process.exit(1);
}

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

const copied = [];
const take = (from) => {
  const destName = from.split(/[\\/]/).pop();
  cpSync(from, join(outDir, destName));
  copied.push(destName);
};

// --- Runtime files (stable names, no version suffix) ----------------------
// mainBinaryName makes `tauri build` rename cargo's output to Clay-Mic.exe;
// a package without the main executable is broken, so fail loudly instead of
// quietly shipping a folder of sidecars.
const exe = join(releaseDir, `${productName}.exe`);
if (existsSync(exe)) {
  take(exe);
} else {
  console.error(`未找到主程序 ${exe}（mainBinaryName 未生效？请重新构建）。`);
  process.exit(1);
}

const tapDll = join(releaseDir, "clay_tap.dll");
if (existsSync(tapDll)) {
  take(tapDll);
}

// Keyslots: prefer exact name, fall back to glob (tauri may strip triple suffix)
const keyslotsExact = join(releaseDir, "clay-mic-keyslots.exe");
if (existsSync(keyslotsExact)) {
  take(keyslotsExact);
} else {
  // Glob for clay-mic-keyslots*.exe, pick newest
  const candidates = readdirSync(releaseDir)
    .filter((n) => /^clay-mic-keyslots.*\.exe$/i.test(n))
    .map((n) => ({ name: n, mtime: statSync(join(releaseDir, n)).mtimeMs }))
    .sort((a, b) => b.mtime - a.mtime);
  if (candidates.length > 0) {
    take(join(releaseDir, candidates[0].name));
  }
}

if (copied.length === 0) {
  console.error("未找到可打包的产物，构建可能未成功。");
  process.exit(1);
}

// --- Portable zip (best-effort, Windows only) -----------------------------
const zipPath = join(root, "packages", `${productName}-${version}-portable.zip`);
const ps = spawnSync("powershell.exe", ["-NoProfile", "-Command", "Get-Command Compress-Archive | Out-Null"], {
  stdio: "ignore",
  cwd: root,
});
if (ps.status === 0) {
  rmSync(zipPath, { force: true });
  spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-Command",
      `Compress-Archive -Path '${outDir}\\*' -DestinationPath '${zipPath}' -Force`,
    ],
    { stdio: "inherit", cwd: root },
  );
  if (existsSync(zipPath)) {
    copied.push(`${productName}-${version}-portable.zip`);
  }
} else {
  console.log("⚠ powershell.exe 不可用，跳过 portable.zip 生成。");
}

// --- Summary ---------------------------------------------------------------
console.log(
  `\n已输出 ${copied.length} 个文件到 packages/${productName}-${version}/：`,
);
for (const name of copied) {
  console.log(`  ${name}  (${name.endsWith(".zip") ? "zip" : "v" + version})`);
}
