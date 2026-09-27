# AGENTS.md

本文件供 AI 编码代理阅读，说明项目的关键约束。改动代码前请先读这一节。

## 项目定位

Clay-Mic 是**仅支持 Windows 10/11** 的桌面应用：蓝牙遥控器（BLE + ATVV）语音键 → 本地 whisper.cpp 转写 →（可选）LLM 格式化 → 注入当前前台窗口。技术栈 Tauri 2 · React 19 · Rust。

核心功能依赖 WinRT BLE、Win32 输入注入与 Interception 内核驱动，**macOS / Linux 无法构建或运行**——不要尝试跨平台适配。

## 发布

- 版本唯一源是 `package.json`；发版只允许 `npm run release:patch|minor|major`。
- 禁止手改版本文件、禁止手动 `git tag`。详见 CONTRIBUTING.md「版本发布」。
- `CHANGELOG.md` 由 git-cliff 从 Conventional Commits 生成；发版时 `--prepend` 追加，不要整份重写。
- tag 只在 `main` 上打；CI/Release 流程见 `.github/workflows/`。
- **不要**在 GitHub 网页上手动创建 Release 或 tag：会绕过「tag 必须在 main」与「版本一致」两道校验，且不产出 `portable.zip`。发布失败时在 Actions 里 re-run，无需重新打 tag。

## 目录速览

- `src/` — 前端 React（主窗口 / 悬浮窗 / 指示器三个窗口）
- `src-tauri/src/` — Rust 后端：`protocol/atvv.rs`（ATVV 解析）、`audio/adpcm.rs`（IMA ADPCM 解码）、`ble/`、`stt/`、`llm/`、`hid/`、`keymap/`、`inject/`、`tap/`
- `src-tauri/keyslots/` — 独立 SYSTEM 服务，NT 符号链接修补键鼠设备槽位（零依赖，纯 FFI）
- `src-tauri/tap-dll/` — 注入 WUDFHost 的原生 hook DLL（基于 `min_hook_rs`）
- `scripts/` — 构建与版本脚本（`package.mjs`、`sync-version.mjs`）
- `docs/` — 技术文档与图片

## 提交规范

Conventional Commits，提交信息用简体中文。`feat` / `fix` / `refactor` / `docs` / `style` / `chore` 等，详见 CONTRIBUTING.md。
