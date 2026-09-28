# 贡献指南

感谢你对 Clay-Mic 的关注。本文档说明开发环境搭建、开发流程、提交规范和 PR 流程。

---

## 开发环境要求

| 依赖 | 版本 | 说明 |
|---|---|---|
| **Windows** | 10/11 | 项目独占 Windows，核心功能依赖 WinRT BLE 与 Win32 输入 API |
| **Node.js** | ≥ 18 | 前端构建 |
| **Rust** | stable | Tauri 后端与 Rust 工具链 |
| **Tauri 2 系统依赖** | — | 参考 [Tauri 前置条件](https://tauri.app/start/prerequisites/) |

### 可选依赖

- **NVIDIA CUDA**：GPU 加速转写需要。CPU 后端无需。
- **Interception 驱动**：仅「按键屏蔽」功能需要，不安装时其余功能不受影响。

---

## 本地开发

```bash
# 安装前端依赖
npm install

# 开发模式（前端热更新 + Rust 后端，保留控制台日志）
npm run tauri:dev
```

开发模式下，前端修改会热更新，Rust 修改需手动触发重新编译。发布产物的打包方式见下方「版本发布」。

---

## 提交信息规范

本仓库使用 [Conventional Commits](https://www.conventionalcommits.org/) 风格，提交信息用**简体中文**。

格式：`<type>: <描述>`

常用 type：

| type | 用途 |
|---|---|
| `feat` | 新功能 |
| `fix` | 修复 bug |
| `refactor` | 重构（不改功能、不修 bug） |
| `perf` | 性能优化 |
| `docs` | 文档 |
| `style` | 样式/布局调整 |
| `test` | 测试 |
| `build` / `ci` | 构建、CI 配置 |
| `chore` | 工具链、一次性迁移等杂项 |

### 示例

```
feat: 添加退格功能到按键映射，更新自定义操作列表
fix: 启动时反复保存配置刷屏；窗口尺寸改为关闭时保存
refactor: ShellExecuteExW 替代 PowerShell 提权，统一补丁目录
docs: 返回/音量键拦截调研文档与 WUDFHost 验证脚本
chore: 移除已完成的一次性迁移代码
```

---

## Pull Request 规范

### 基本要求

- **小步提交**：一个 PR 解决一个问题或实现一个功能，避免大杂烩。
- **说明测试方式**：PR 描述中写清楚你如何验证了改动（手动操作步骤、涉及的设备型号等）。
- **不要提交构建产物**：`packages/` 目录已在 `.gitignore` 中，请勿手动添加。

### PR 检查清单

提交 PR 前请确认：

- [ ] 本地 `npm run lint` 与 `npm test` 通过
- [ ] 本地 `npm run build` 通过（前端 TypeScript 编译无报错）
- [ ] 涉及 Rust 改动时 `npm run check:rust` 通过
- [ ] 未引入新的依赖警告
- [ ] 文档已同步更新（如涉及 UI/配置变化）

> PR 标题须符合 Conventional Commits（CI 会校验），squash 合并后即为提交信息。

---

## Issue 指南

### 提交 Bug 报告

请使用 Issue 模板，并尽量包含以下信息：

- **操作系统版本**（如 Windows 11 23H2）
- **Clay-Mic 版本**（「关于」页或 `package.json` 中查看）
- **STT 后端与模型**（CPU / CUDA，使用了哪个模型）
- **遥控器型号与 VID:PID**（如有，设备管理器中可查）
- **复现步骤**
- **日志文件**：`%LOCALAPPDATA%/clay-mic/clay-mic.log`（日志记录了设备连接、转写、上屏的完整流程，排查问题非常有用）

### 提交功能建议

使用 Feature Request 模板，描述你的使用场景和期望行为即可。

---

## 项目结构概览

```
clay-mic/
├── src/                  # 前端 React 代码（主窗口 / 悬浮窗 / 指示器）
├── src-tauri/            # Rust 后端（Tauri）
│   ├── src/              # 后端主 crate
│   ├── keyslots/         # 键鼠设备槽位修补服务（独立 SYSTEM 服务）
│   └── tap-dll/          # 注入 WUDFHost 的原生 hook DLL
├── docs/                 # 文档与参考图片
├── scripts/              # 构建脚本（package.mjs、sync-version.mjs）
├── .github/workflows/    # CI 与 Release 工作流
├── cliff.toml            # CHANGELOG 生成配置（git-cliff）
├── package.json          # 前端依赖与脚本（版本唯一源）
└── src-tauri/Cargo.toml  # Rust 依赖
```

---

## 版本发布

版本的**唯一来源**是 `package.json` 的 `version` 字段。其余 5 个文件（`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/keyslots/Cargo.toml`、`src-tauri/tap-dll/Cargo.toml`、`src-tauri/Cargo.lock` 的三个工作区成员行）由脚本自动同步（`package-lock.json` 由 npm 自行维护），**禁止手动修改**。

### 发布流程

```bash
# 一条命令：递版 + 同步 5 处 + 更新 CHANGELOG + 版本提交 + 打 tag
npm run release:patch   # 修 bug：0.1.0 → 0.1.1
npm run release:minor   # 新功能：0.1.0 → 0.2.0
npm run release:major   # 破坏性变更：0.1.0 → 1.0.0

# 推送提交与 tag，触发 GitHub Actions 自动打包并发布 Release
git push --follow-tags
```

`release:*` 只做一件事——`npm version <level>`：递增版本号、经 `version` 生命周期钩子把版本同步到其余 5 个文件、自动生成 `CHANGELOG.md`、创建版本提交并打 `vX.Y.Z` tag。**本地不再出包**：打包与发布由 `release.yml` 在 GitHub 的 Windows runner 上完成。

> **前提**：工作区必须干净（`npm version` 会拒绝脏树）。
> **tag 只在 `main` 上打**：`release.yml` 会校验 tag 指向的提交位于 `main`，否则拒绝发布。

日常分支模型：`feature/* → dev → main`。功能开发在特性分支上，PR 到 `dev`；`dev` 稳定后 PR 到 `main`；发布只在 `main` 上打 tag。

### 本地出包（可选，仅自测）

需要在推 tag 之前先在本机验证绿色版时：

```bash
npm run package   # tauri build + 组装 packages/Clay-Mic-<版本>/ 与 portable.zip
```

`npm run package` 只出产物、不碰 git，与发布流程相互独立。

### CHANGELOG

`CHANGELOG.md` 由 [git-cliff](https://git-cliff.org/) 依据 Conventional Commits 自动生成。发版时由 `version` 钩子把新版本一节**追加**到文件顶部（`--prepend`），**不会重写已有内容**；`v1.0.0` 一节为手工冻结的首版简介，其后各版本由工具维护。

```bash
npm run changelog            # 追加新版本一节到 CHANGELOG.md 顶部（标题取 package.json 版本号）
npm run changelog -- -o -    # 仅输出到标准输出，不落盘
```

生成依赖本机已安装 `git-cliff`（`cargo install git-cliff` 或从其 [Releases](https://github.com/orhun/git-cliff/releases) 下载二进制）。

### 手动同步（开发期间）

如果其他 5 处版本与 `package.json` 不一致，执行：

```bash
npm run version:sync     # 同步
npm run version:sync -- --check  # 仅检查，不一致则 exit 1
```

打包脚本（`npm run package`）启动时会自动执行 `--check`，版本不一致将拒绝打包。
