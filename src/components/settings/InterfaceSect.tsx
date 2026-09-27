import { Field, Group, SectionBox } from "./common";
import { HotkeyInput } from "../HotkeyInput";
import { INDICATOR_STYLES, OVERLAY_STYLES, StylePicker } from "../StylePicker";
import { useSettingsCore } from "./context";

export function InterfaceSect() {
  const { config, setConfig, update } = useSettingsCore();

  return (
    <SectionBox>
      <Group label="交互">
        <Field label="唤出快捷键">
          <HotkeyInput
            value={config.overlay.hotkey}
            onChange={(value) => update("overlay.hotkey", value)}
          />
        </Field>
        <Field label="注入方式">
          <select
            value={config.inject.method}
            onChange={(e) => update("inject.method", e.target.value)}
            className="input"
          >
            <option value="clipboard">剪贴板粘贴 (Ctrl+V)</option>
            <option value="keyboard">键盘模拟 (逐字输入)</option>
          </select>
        </Field>
        <Field label="关闭时最小化到托盘">
          <label className="flex items-center gap-2 cursor-pointer pt-2">
            <input
              type="checkbox"
              checked={config.window.close_to_tray}
              onChange={(e) =>
                setConfig((prev) => ({
                  ...prev,
                  window: { ...prev.window, close_to_tray: e.target.checked },
                }))
              }
              className="w-4 h-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
            />
            <span className="text-[12px] text-gray-400">
              关闭窗口后程序继续在托盘运行；取消则直接退出
            </span>
          </label>
        </Field>
      </Group>

      <Group label="样式">
        <Field label="浮窗样式">
          <StylePicker
            options={OVERLAY_STYLES}
            value={config.overlay.style}
            onChange={(value) => update("overlay.style", value)}
          />
        </Field>
        <Field label="语音指示器样式">
          <StylePicker
            options={INDICATOR_STYLES}
            value={config.indicator.style}
            onChange={(value) => update("indicator.style", value)}
          />
        </Field>
      </Group>
    </SectionBox>
  );
}
