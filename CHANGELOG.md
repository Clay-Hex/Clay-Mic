# 更新日志

本项目的重要变更记录于此。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [1.2.0] - 2026-10-11

### 修复

- 监听线程退出泄漏 RECORDING 导致指示器不消失
- **tap**: 健康判定改用当前宿主的新鲜心跳
- **tap**: 状态不再信任 marker，只认当前宿主的新鲜心跳
- **ble**: 重连用 generation 判定被取代，disconnect 一并失效
- **ble**: 用事件 sender 读链路状态，消除漏判窗口
- **tap**: 无人值守自愈统一加屏蔽/移除守卫
- **ble**: 在事件线程捕获重连 epoch，消除启动竞态
- **tap**: 启动按 marker 起 UDP 服务器，修复心跳循环依赖
- **ble**: 连接期间忽略链路事件，避免误触重连
- **ble**: 重连循环限定尝试次数，避免设备移除后永远重试
- **hid**: 稳态不再额外调用 Interception 驱动，修复退出卡死

### 文档

- **tap**: 修正 TapStatus.injected 注释为心跳语义

### 新增

- 设置页增加结束 WUDFHost 进程按钮
- **tap**: Hook 心跳携带宿主 PID
- **hid**: 设备到达/移除事件驱动重绑定，去掉热路径轮询
- **ble**: 语音会话掉线自动重连
- **tap**: 注入结果类型化；无人值守自愈不再弹 UAC
- **ble**: 重连成功后静默补一次 tap 注入
- **hid**: 恢复 filter 读回自检，避免每次无谓重装
- **update**: 关于页应用内检查更新，支持下载与一键自动更新

### 杂项

- 补充语音指示器可见性诊断日志
- **docs**: 修正 capture 循环注释与文件末尾换行
- **interception**: 统一抑制 Driver 的 raw-pointer lint

### 重构

- **interception**: Filter 操作串行化，消除设备选择竞态
- **tap**: 自愈守卫拆出 HOOK_WANTED，与 ACCEPTING 解耦
## [1.1.1] - 2026-10-05

### 修复

- LLM 日志补充 prompt 长度
- 兼容思考内容的 reasoning 字段名
- 忽略流式响应中的空 finish_reason
- Prompt 与转写合并为单条消息并加定界符

### 新增

- 格式化 Prompt 增加恢复默认按钮
## [1.1.0] - 2026-09-28

### 修复

- 模型能力列表刷新失败与下载过慢
- 发版生成的 CHANGELOG 标题应为版本号而非 Unreleased

### 新增

- 按键映射新增「注入最新文本」自定义动作
- 按键配置支持长按行为
- LLM 支持自定义 Provider 与思考参数形态
## [1.0.0] - 2026-09-27

首个公开版本。

- 蓝牙遥控器（BLE ATVV）语音键 → 本地 whisper.cpp 转写 → 注入当前前台窗口
- 支持 CPU / CUDA 后端与模型切换，可离线运行
- 可选 LLM 格式化为 Markdown
- 按键映射、原生按键屏蔽、键鼠设备槽位驱动补丁
- 悬浮窗、托盘常驻、历史与统计

<!-- v1.0.0 为手工冻结的基线；此后由 git-cliff 以 --prepend 方式追加新版本。 -->
