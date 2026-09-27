# 更新日志

本项目的重要变更记录于此。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [1.0.0] - 2026-09-27

首个公开版本。

- 蓝牙遥控器（BLE ATVV）语音键 → 本地 whisper.cpp 转写 → 注入当前前台窗口
- 支持 CPU / CUDA 后端与模型切换，可离线运行
- 可选 LLM 格式化为 Markdown
- 按键映射、原生按键屏蔽、键鼠设备槽位驱动补丁
- 悬浮窗、托盘常驻、历史与统计

<!-- v1.0.0 为手工冻结的基线；此后由 git-cliff 以 --prepend 方式追加新版本。 -->
