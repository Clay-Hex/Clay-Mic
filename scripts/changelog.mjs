#!/usr/bin/env node
// Generate the CHANGELOG section with git-cliff.
//
//   node scripts/changelog.mjs              # release: un-released commits,
//                                           #   titled with package.json's version
//   node scripts/changelog.mjs -- -o -      # preview: same range, stdout only
//
// `--unreleased` only picks the commit range; the section title comes from
// `--tag`, so the release flow passes `v<version>` to stamp it with the real
// number instead of leaving a bare "Unreleased" heading behind. The version is
// read from package.json rather than via `${npm_package_version}`, which the
// Windows shell this project releases from does not expand.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * Build the git-cliff arguments for one run.
 *
 * Extra arguments (npm's `-- …`) mean a preview or an explicit override: the
 * range is kept so the caller still sees unreleased commits, but the output
 * side is theirs to choose — CONTRIBUTING documents `-- -o -` that way.
 *
 * @param {string} version  – package.json version, e.g. "1.1.0"
 * @param {string[]} extra  – arguments passed through on the command line
 * @returns {string[]} argv for git-cliff
 */
export function buildArgs(version, extra) {
  if (extra.length > 0) {
    return ["--unreleased", ...extra];
  }
  return [
    "--unreleased",
    "--tag",
    `v${version}`,
    "--prepend",
    "CHANGELOG.md",
  ];
}

/**
 * Read the canonical version from package.json.
 * @param {string} dir  – project root
 * @returns {string} semver string
 */
export function readVersion(dir) {
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  return pkg.version;
}

function main() {
  const args = buildArgs(readVersion(root), process.argv.slice(2));
  execFileSync("git-cliff", args, { cwd: root, stdio: "inherit" });
}

// Run only when executed directly: importing the helpers above (for their
// tests) must not rewrite the changelog as a side effect.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
