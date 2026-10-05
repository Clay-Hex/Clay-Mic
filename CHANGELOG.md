# 更新日志

本项目的重要变更记录于此。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

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
