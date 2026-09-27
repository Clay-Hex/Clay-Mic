# Windows BLE 遥控器返回/音量键拦截：WUDFHost 层逆向与实现

## 摘要

clay-mic 通过 Interception 驱动拦截蓝牙遥控器按键，但返回、音量+、音量- 三个键始终无法被捕获。逆向分析发现，这三个键走的是键盘页（0x07）的非标准扩展 usage，kbdhid 在翻译阶段将其丢弃，导致 Interception（键盘类过滤驱动）在丢弃之后无从拦截。

最初探索了 Frida Gadget 注入方案：在 WUDFHost.exe 中 hook `NtDeviceIoControlFile`，拦截 IOCTL `0x80018483` 的 9 字节 HID-over-GATT 报告，成功验证了可行性。但考虑到 Frida 的运行时依赖与许可证复杂度，最终将方案收敛为原生 hook DLL（`src-tauri/tap-dll/`，基于 min_hook_rs），以静态编译的 DLL 注入替代 Frida Gadget。本文记录完整的探索过程、根因分析与最终实现。

---

## 1. 背景

clay-mic 通过 Interception 驱动拦截遥控器按键（`src-tauri/src/hid/win.rs`），但**返回、音量+、音量- 三个键无法处理**：

- 按其他键有 Interception 日志输出
- 按这三个键完全没有日志 → 按键事件到不了 Interception 的 `read_events`

影响：这三个键无法配置为忽略 / 直通 / 发送组合键 / 程序自定义，按键屏蔽对它们无效。

---

## 2. 根因分析

### 2.1 机制

```
遥控器固件
  ├─ 方向/OK/Home 等标准键 → 键盘页(0x07)标准 usage
  │     → kbdhid 翻译为 scancode → KbdClass → Interception ✅ 拦得到
  │
  └─ 返回/音量± → 键盘页(0x07)非标准扩展 usage
        back=0xF1, vol+=0x80, vol-=0x81
        → kbdhid 丢弃/不翻译（0xF1 属非标准扩展键）
        → 永远到不了键盘类驱动 → Interception ❌ 完全看不到
        → 但 Windows 在 WUDFHost 的 HID-over-GATT 层已经读到了原始报告
```

**说明**：这些键走的是键盘页（0x07），不是 Consumer Page（0x0C）。`keymap/buttons.rs` 文件头也标明是 keyboard-page usages。问题是 **kbdhid 对非标准键盘页 usage 的丢弃**，导致 Interception（键盘类过滤驱动）在丢弃之后无从拦截；而原始字节在更上游的 WUDFHost HID-over-GATT 读取中完整存在。

### 2.2 代码证据

| 位置 | 证据 |
|---|---|
| `src-tauri/src/keymap/buttons.rs` | `back=0x00F1, volume_up=0x0080, volume_down=0x0081`（keyboard-page）；`BUTTON_SCAN_CODES` 有观测值但 Interception 实测收不到 |
| `src-tauri/src/protocol/atvv.rs` | ATVV 协议无按键 opcode，按键不走 BLE 语音通道，只能走 HID 路径 |
| `src-tauri/src/hid/win.rs` | 捕获循环依赖 Interception `read_events`，收不到则无日志、无映射 |

### 2.3 与 AGENTS.md 已知 TODO 的关系

AGENTS.md「设备槽位可视化与预警」（`probe_devices` 只扫 `1..=KEYBOARD_SLOT_COUNT`=10）是另一个独立问题：影响的是设备漂出槽位后所有键静默失效。本文三个键的丢失是 kbdhid 层丢弃，即使槽位探测正确也收不到。两者可并行修复，互不替代。

---

## 3. 方案探索

### 3.1 Frida 注入方案（已验证，后移除）

#### 原理

Interception 在 kbdhid 丢弃之后，永远见不到这三个键。需要在丢弃之前取回原始字节：

```
遥控器 → BLE HID-over-GATT
  → WUDFHost.exe 通过 IOCTL 0x80018483 读取特征值（9 字节报告）
       ↑ 在此 hook NtDeviceIoControlFile，取出含 F1/80/81 的原始数据
  → kbdhid 丢弃非标准 usage（现状丢失点）
  → KbdClass → Interception（仅标准键）
```

用 Frida Gadget（运行时下载的共享库）注入目标 WUDFHost.exe，hook `ntdll!NtDeviceIoControlFile`，在 IOCTL `0x80018483` 返回时读输出缓冲，解析 9 字节报告中的 usage，走现有 keymap 的 suppress / 动作执行。

目标进程 PID 通过注册表 `WUDFDiagnosticInfo\HostPid` 定位：

```
HKLM\SYSTEM\CurrentControlSet\Enum\BTHLEDevice
  \{00001812-...}_DEV_VID&012717_PID&32B8_...
    \<实例ID>\Device Parameters\WUDFDiagnosticInfo
      → HostPid (REG_DWORD)
```

#### 为何不用其它方案

| 方案 | 能否拿到 F1/80/81 | 结论 |
|---|---|---|
| Interception（现状） | ❌ | kbdhid 已丢弃，架构上晚了一步 |
| WH_KEYBOARD_LL | ❌ | 同样在丢弃之后 |
| Raw Input | ⚠️ | 实测这些键仍缺失 |
| HID filter 驱动 | ⚠️ | 需驱动签名；蓝牙路径不在 kbdclass 栈 |
| Frida 注入 WUDFHost | ✅ | 在丢弃前取原始报告，已验证可行 |

#### Frida 许可

Frida 使用 wxWindows Library Licence v3.1（等同于 LGPL-2.0-or-later + WxWindows exception，允许以自有条款发布二进制）。运行时下载、不随包分发等于不重新分发，义务最轻；在 `THIRD-PARTY-NOTICES.md`「二、运行时下载」章节登记即可。

### 3.2 最终方案：原生 hook DLL

考虑到 Frida 的运行时下载依赖与许可证复杂度，最终将方案收敛为原生 hook DLL（`src-tauri/tap-dll/`），基于 min_hook_rs 静态编译。核心逻辑不变：hook `NtDeviceIoControlFile`，拦截 IOCTL `0x80018483`，解析 9 字节报告，通过本机回环 UDP 上报按键事件。

与 Frida 方案相比的优势：

- 无运行时下载依赖，DLL 随应用分发
- 静态编译，无 Frida 版本兼容问题
- 许可证更简单（min_hook_rs 为 MIT）

---

## 4. 设计决策

### 4.1 约束

1. hook DLL 随应用分发，不需要运行时下载
2. 无独立顶层开关；状态与维护操作进设置 → 按键拦截（DriverSect）；不与「下载驱动/安装驱动和补丁」按钮绑定
3. 屏蔽开关不执行下载：只根据组件是否就绪决定是否注入；未就绪则跳过注入，屏蔽本身照常工作

### 4.2 交互模型

注入的三个前置条件（缺一不注，缺了就静默跳过，屏蔽照常）：

1. 组件已就绪（hook DLL 存在）
2. 能解析 HostPid（设备已配对 + 配置里有 VID/PID + 遥控器在线）
3. 管理员权限（首次注入弹一次 UAC）

**触发时机（任一发生时检查上述条件，齐了就注）**：

| 触发点 | 行为 |
|---|---|
| 打开「屏蔽遥控器按键」 | 后台检查并注入（主路径） |
| 启动时屏蔽本来就是开的 | 同上（恢复上次状态） |
| 点「连接遥控器」成功后 | 若屏蔽已开 → 后台注入 |
| DriverSect 点「重新注入」 | 强制注入（排障，不看屏蔽状态） |

**主路径（DevicePanel「屏蔽遥控器按键」）**：

```
apply_suppression(true):
  1. 启动 Interception 捕获（现有逻辑不变）
  2. 检查 hook DLL 就绪
     ├─ 就绪 → 尝试注入（可能一次 UAC）
     │         ├─ 成功 → 返回/音量 可映射
     │         └─ 失败/拒绝 → 降级，屏蔽照常，状态可见于 DriverSect
     └─ 未就绪 → 跳过，屏蔽照常（三键行为同今日）
```

**维护路径（DriverSect，独立卡片，不绑驱动安装按钮）**：

```
卡片 1：Interception 状态 / 操作 / 诊断与维护
卡片 2：【HID Tap（返回/音量）】
  状态: 未就绪 / 已就绪 / 注入中 / 运行中 / 不可用(原因)
  [重新注入] [移除]     ← 仅用户显式操作
  诊断: 版本 · WUDFHost PID · 错误信息
```

**与驱动安装分离的理由**：Interception 装完要重启、hook DLL 不用；前者屏蔽必需、后者可选增强；权限同意应分开；失败互不阻塞。

### 4.3 与现有文案的一致性

`DevicePanel` 帮助文案已写：「返回/音量需要 Tap 支持时会自动按需启用（可能弹一次 UAC）」。按需注入（非下载）与该承诺一致。

---

## 5. 最终实现

### 5.1 Hook DLL（`src-tauri/tap-dll/src/lib.rs`）

原生 DLL，基于 min_hook_rs，注入 WUDFHost.exe 后：

1. **Hook 目标**：`ntdll!NtDeviceIoControlFile`
2. **拦截 IOCTL**：`0x80018483`（HID-over-GATT 特征值读取）
3. **报告格式**：9 字节，`01 00 00 <usage16le> 00 00 00 00 00`
4. **按键识别**：`back=0x00F1, volume_up=0x0080, volume_down=0x0081`
5. **事件上报**：本机回环 UDP（端口 49733），JSON 格式 `{"button":"back","down":true}`，附 1s 心跳
6. **边沿触发**：按下带 usage，松开全零，防双触发

导出函数：

- `clay_tap_init`：由注入器在独立线程调用（避开 loader lock），初始化 socket + hook
- `clay_tap_cleanup`：完整卸载，unhook + WSACleanup
- `DllMain`：仅 `DisableThreadLibraryCalls`，不执行任何业务逻辑

### 5.2 注入模块（`src-tauri/src/tap/`）

核心模块结构：

| 文件 | 职责 |
|---|---|
| `src-tauri/src/tap/mod.rs` | 编排：注入/移除/状态管理/UDP 服务器生命周期 |
| `src-tauri/src/tap/process.rs` | Win32 注入/卸载：`CreateRemoteThread(LoadLibraryW)` + 提权辅助 |
| `src-tauri/src/tap/status.rs` | 状态快照与 UI 推送 |
| `src-tauri/src/tap/host.rs` | HostPid 查找（注册表 BTHLEDevice → VID/PID 匹配 → WUDFDiagnosticInfo） |
| `src-tauri/src/tap/udp.rs` | UDP 服务器：接收 hook DLL 的按键事件与心跳 |

注入流程：

1. 查找 hook DLL 源（exe 旁或 `target/` 目录）
2. 部署到 `%LOCALAPPDATA%\clay-mic\tap\clay_tap.dll`
3. 通过注册表定位 WUDFHost 的 HostPid
4. `OpenProcess` → `VirtualAllocEx` 写入 DLL 路径 → `CreateRemoteThread(LoadLibraryW)`
5. `EnumProcessModules` 确认 DLL 已加载
6. `CreateRemoteThread(clay_tap_init)` 在独立线程初始化 hook
7. 等待 UDP 心跳确认初始化完成

卸载流程：

1. `CreateRemoteThread(clay_tap_cleanup)` 清理 hook + socket
2. 多轮 `FreeLibrary` 直到模块列表不再包含 hook DLL
3. 停止 UDP 服务器，清除标记文件

### 5.3 安全与声明

- 注入前校验 hook DLL 完整性；目标进程必须匹配配置设备的 HostPid
- UAC 拒绝 → 降级，屏蔽照常，错误信息可见
- 提权操作通过 `ShellExecuteExW` + `runas` 动词，不保存管理员凭据
- 注明仅作硬件输入兼容

---

## 6. 验证脚本

`scripts/verify-arn9/` 下的脚本用于验证注册表结构和 GATT 报告：

| 脚本 | 用途 |
|---|---|
| `scripts/verify-arn9/device-probe.ps1` | 注册表探针：BTHLEDevice → WUDFDiagnosticInfo → HostPid；递归查找 + fallback |
| `scripts/verify-arn9/gatt-tap-probe.js` | Frida 只读探针：hook `NtDeviceIoControlFile` 抓 IOCTL `0x80018483` 报告 |

**运行方法**：

```powershell
# 第 1 步（普通 PowerShell，无需管理员）
powershell -ExecutionPolicy Bypass -File scripts\verify-arn9\device-probe.ps1

# 第 2 步（管理员 PowerShell；pip install frida-tools 一次即可）
frida -p <HostPid> -l scripts\verify-arn9\gatt-tap-probe.js
```

注意：`.ps1` 为 UTF-8 with BOM（中文 Windows PowerShell 5.1 兼容），勿转存为无 BOM。

### 6.1 实测环境

- 设备：`MI RC`，`VID&012717_PID&32B8_REV&00A4`
- 服务：HID-over-GATT `{00001812-...}` OK；ATVV `{AB5E0001-...}` 存在
- HostPid：`3404` → 进程 `WUDFHost`

### 6.2 探针结果

```
对照组（方向）:
  01 00 00 52 00 00 00 00 00   UP    + 松开(全零)
  01 00 00 51 00 ...           DOWN
  01 00 00 50 00 ...           LEFT
  01 00 00 4F 00 ...           RIGHT

目标键:
  01 00 00 F1 00 ...           BACK  ✓ + 松开
  01 00 00 80 00 ...           VOL+  ✓ + 松开
  01 00 00 81 00 ...           VOL-  ✓ + 松开
```

**判读**：

- IOCTL `0x80018483`，9 字节，格式 `01 00 00 <usage16le> 00 00 00 00 00`
- 按下/松开边沿清晰（按下带 usage，松开全零）—— 防双触发 arm/consume 有干净信号源
- 对照组（方向键）有输出，目标键有输出 → hook 有效，且三个键确实存在于 GATT 缓冲

**结论：方案成立，原生 hook DLL 已实现并通过端到端验证。**

### 6.3 端到端验收标准

1. 开启按键屏蔽（hook DLL 已就绪）→ 按返回/音量± → 走 keymap 绑定动作，不触发系统原生行为
2. 关闭屏蔽或 hook DLL 未就绪 → 屏蔽开关仍可正常开关，Interception 路径不受影响
3. 重复跑 `scripts/verify-arn9/` 两脚本 → HostPid 可定位、三键报告可抓到
