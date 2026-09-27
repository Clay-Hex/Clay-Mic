# 安全策略

发现安全问题时请**不要**开公开 issue，请使用仓库的 **Security → Report a vulnerability** 私密提交（维护者需先在仓库 Settings → Security 中开启 “Private vulnerability reporting”）。

## 需要了解的事实

- **内核驱动**：启用「按键屏蔽」会加载 **Interception 内核驱动**，可拦截键盘输入。安装需要**管理员权限**，通常需重启生效——请只在你信任的机器上使用。
- **文本注入**：识别结果会注入**当前前台窗口**；避免在密码框等敏感场景依赖自动注入。
- **隐私**：语音识别完全在本机（whisper.cpp），音频不外发。**若启用 LLM 格式化**，转写文本会发送到**你自己配置的**第三方 API。本项目**无任何遥测**，使用统计仅存本地。
- **API Key**：以明文保存在本机 `%LOCALAPPDATA%/clay-mic/config.json`，请自行保管。
