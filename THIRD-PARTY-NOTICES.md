# 第三方组件与许可

Clay-Mic 以 **MIT** 发布（见 [`LICENSE`](LICENSE)）。

## 一、随二进制分发的依赖

`Clay-Mic.exe` 把下列依赖编译 / 打包进二进制，分发时须保留其版权与许可声明：

### Rust 后端

| 组件 | 许可证 |
|------|--------|
| Tauri 2 及其插件（global-shortcut、shell、single-instance、tray-icon） | MIT / Apache-2.0 |
| `serde`、`serde_json` | MIT / Apache-2.0 |
| `tokio` | MIT |
| `reqwest` | MIT / Apache-2.0 |
| `rusqlite`（含内嵌 SQLite） | MIT |
| `windows` crate（Win32/WinRT API 绑定） | MIT / Apache-2.0 |
| `uuid` | MIT / Apache-2.0 |
| `chrono` | MIT / Apache-2.0 |
| `libloading` | ISC |
| `min_hook_rs` | MIT |
| `enigo` | MIT |
| `dirs` | MIT / Apache-2.0 |
| `async-trait` | MIT / Apache-2.0 |
| `thiserror` | MIT / Apache-2.0 |
| `log`、`env_logger` | MIT / Apache-2.0 |
| `futures` | MIT / Apache-2.0 |

### 前端

| 组件 | 许可证 |
|------|--------|
| `react`、`react-dom` | MIT |
| `@tauri-apps/api`、`@tauri-apps/plugin-*` | MIT / Apache-2.0 |
| `vite` | MIT |
| `typescript` | Apache-2.0 |
| `tailwindcss`、`postcss`、`autoprefixer` | MIT |
| `vitest`、`eslint`、`typescript-eslint` | MIT |

完整清单见 `src-tauri/Cargo.lock` 与 `package.json`。需要含完整许可文本的 NOTICE 时，在 `src-tauri/` 运行 `cargo about generate about.hbs > NOTICE.md`，或在根目录运行 `npx license-checker`。

### 随包数据

| 数据 | 来源 | 许可证 |
|------|------|--------|
| `assets/model-caps.json`（模型能力表，裁剪自 [models.dev](https://models.dev) 的 `api.json`，由 `npm run caps` 生成；运行时可经设置页「刷新模型能力缓存」更新，缓存于 `%LOCALAPPDATA%\clay-mic\model-caps.json`） | models.dev | MIT |

## 二、运行时下载（不随包分发）

以下由用户运行时获取，本项目不重新分发：

| 组件 | 许可证 | 说明 |
|------|--------|------|
| whisper.cpp 运行时 | MIT | 从 GitHub Releases 下载 |
| Whisper 模型（`ggml-*.bin`） | MIT | 从 HuggingFace 下载 |
| Interception 驱动 | LGPL-3.0（非商业）/ 商业授权 | 本项目通过 `libloading` 在运行时动态加载 `interception.dll`，只调用其公开 API，不链接、不重新分发 |

> **关于 Interception**：商业用途且启用「按键屏蔽」时，需另行向作者取得商业授权（联系方式见其[仓库](https://github.com/oblitum/Interception)）。

## 三、借鉴的开源实现

| 项目 | 作者 | 许可证 |
|------|------|--------|
| [interception-driver-fix](https://github.com/hygorostrowskij/interception-driver-fix) | Hygor Ostrowskij de Morais | BSD-3-Clause |

「驱动补丁」功能（`src-tauri/keyslots/`）的核心算法**借鉴自 interception-driver-fix**：通过 `NtCreateSymbolicLinkObject` 在 `\Device\KeyboardClass10..999` 区间批量创建 `OBJ_PERMANENT` 符号链接并折返回 `KeyboardClass0..9`，配合 `SeCreateSymbolicLinkPrivilege` / `SeCreatePermanentPrivilege` 特权与一次性 Windows 服务，规避 Interception 驱动只识别个位数设备编号的上游缺陷。

在该实现之上自研的部分：Rust 手写全部 NT API FFI（不依赖 phnt）、`Applied`/`Report` 统计与 `link_exists()` 持久化验证、与主程序的安装流程整合。

```text
Copyright (c) 2025 Hygor Ostrowskij de Morais <hygor.o.morais@gmail.com>

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```
