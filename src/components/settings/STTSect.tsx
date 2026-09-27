import type { DownloadProgress, SttStatus } from "../../lib/config";
import { DownloadBar, Field, Group, Message, MetaRow, SectionBox } from "./common";
import { useSettingsCore } from "./context";

const RUNTIME_LABELS: Record<string, string> = {
  cpu: "CPU",
  cuda12: "GPU (CUDA 12.4)",
  cuda11: "GPU (CUDA 11.8)",
};

export interface STTSectProps {
  sttStatus: SttStatus | null;
  sttBusy: boolean;
  sttMessage: string | null;
  refreshStatus: () => Promise<void>;
  runDownload: (
    cmd: "download_stt_binary" | "download_stt_model",
    args?: Record<string, unknown>,
  ) => Promise<void>;
  dlProgress: DownloadProgress | null;
}

export function STTSect({
  sttStatus,
  sttBusy,
  sttMessage,
  refreshStatus,
  runDownload,
  dlProgress,
}: STTSectProps) {
  const { config, setConfig, update } = useSettingsCore();
  const runtimeInstalled = Boolean(
    sttStatus?.installed_runtimes.includes(config.stt.runtime),
  );
  const smallButton =
    "shrink-0 px-2.5 py-1.5 text-[12px] font-medium text-blue-600 bg-blue-50 rounded-lg hover:bg-blue-100 disabled:text-gray-300 disabled:bg-gray-50 transition-colors";

  return (
    <SectionBox>
      <Group label="录音行为">
        <Field label="边说边转">
          <label className="flex items-center gap-2 cursor-pointer pt-2">
            <input
              type="checkbox"
              checked={config.stt.streaming}
              onChange={(e) =>
                setConfig((prev) => ({
                  ...prev,
                  stt: { ...prev.stt, streaming: e.target.checked },
                }))
              }
              className="w-4 h-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
            />
            <span className="text-[12px] text-gray-400">
              录音时实时显示部分转写结果，最终以上屏文本为准
            </span>
          </label>
        </Field>
        <Field label="过滤短音频">
          <label className="flex items-center gap-2 cursor-pointer pt-2">
            <input
              type="checkbox"
              checked={config.stt.min_audio_ms > 0}
              onChange={(e) =>
                setConfig((prev) => ({
                  ...prev,
                  stt: {
                    ...prev.stt,
                    min_audio_ms: e.target.checked
                      ? prev.stt.min_audio_ms > 0
                        ? prev.stt.min_audio_ms
                        : 200
                      : 0,
                  },
                }))
              }
              className="w-4 h-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
            />
            <span className="text-[12px] text-gray-400">
              低于阈值的录音会被丢弃，避免误触产生空结果
            </span>
          </label>
          {config.stt.min_audio_ms > 0 && (
            <div className="flex items-center gap-2 mt-2">
              <input
                type="number"
                min={10}
                max={2000}
                value={config.stt.min_audio_ms}
                onChange={(e) =>
                  update(
                    "stt.min_audio_ms",
                    Math.min(2000, Math.max(10, Number(e.target.value) || 10)),
                  )
                }
                className="input w-24"
              />
              <span className="text-[12px] text-gray-400">毫秒</span>
            </div>
          )}
        </Field>
      </Group>

      <Group label="引擎与模型">
        <Field
          label="模型大小"
          action={
            <button
              type="button"
              onClick={() =>
                void runDownload("download_stt_model", { model: config.stt.model })
              }
              disabled={sttBusy}
              className={smallButton}
            >
              下载模型
            </button>
          }
        >
          <select value={config.stt.model} onChange={(e) => update("stt.model", e.target.value)} className="input">
            <optgroup label="多语言（推荐）">
              <option value="tiny">tiny (~75MB)</option>
              <option value="base">base (~142MB)</option>
              <option value="small">small (~466MB)</option>
              <option value="medium">medium (~1.5GB)</option>
              <option value="large-v1">large-v1 (~2.9GB)</option>
              <option value="large-v2">large-v2 (~2.9GB)</option>
              <option value="large-v3">large-v3 (~2.9GB)</option>
              <option value="large-v3-turbo">large-v3-turbo (~1.6GB，推荐)</option>
            </optgroup>
            <optgroup label="英文专用（不识别中文）">
              <option value="tiny.en">tiny.en (~75MB)</option>
              <option value="base.en">base.en (~142MB)</option>
              <option value="small.en">small.en (~466MB)</option>
              <option value="medium.en">medium.en (~1.5GB)</option>
            </optgroup>
            <optgroup label="量化 · 多语言（省内存/显存）">
              <option value="tiny-q5_1">tiny-q5_1</option>
              <option value="tiny-q8_0">tiny-q8_0</option>
              <option value="base-q5_1">base-q5_1</option>
              <option value="base-q8_0">base-q8_0</option>
              <option value="small-q5_1">small-q5_1</option>
              <option value="small-q8_0">small-q8_0</option>
              <option value="medium-q5_0">medium-q5_0</option>
              <option value="medium-q8_0">medium-q8_0</option>
              <option value="large-v2-q5_0">large-v2-q5_0</option>
              <option value="large-v2-q8_0">large-v2-q8_0</option>
              <option value="large-v3-q5_0">large-v3-q5_0</option>
              <option value="large-v3-turbo-q5_0">large-v3-turbo-q5_0</option>
              <option value="large-v3-turbo-q8_0">large-v3-turbo-q8_0</option>
            </optgroup>
            <optgroup label="量化 · 英文专用">
              <option value="tiny.en-q5_1">tiny.en-q5_1</option>
              <option value="tiny.en-q8_0">tiny.en-q8_0</option>
              <option value="base.en-q5_1">base.en-q5_1</option>
              <option value="base.en-q8_0">base.en-q8_0</option>
              <option value="small.en-q5_1">small.en-q5_1</option>
              <option value="small.en-q8_0">small.en-q8_0</option>
              <option value="medium.en-q5_0">medium.en-q5_0</option>
              <option value="medium.en-q8_0">medium.en-q8_0</option>
            </optgroup>
          </select>
        </Field>
        <Field label="语言">
          <select value={config.stt.language} onChange={(e) => update("stt.language", e.target.value)} className="input">
            <option value="auto">自动检测</option>
            <option value="zh">中文</option>
            <option value="en">英文</option>
          </select>
        </Field>
        <Field label="初始 Prompt" help="提升专业词与术语的识别率">
          <input
            value={config.stt.prompt}
            onChange={(e) => update("stt.prompt", e.target.value)}
            placeholder="张三 李四 产品需求 排期…"
            className="input"
          />
        </Field>
        <Field
          label="运行时 · 后端"
          help="本地 whisper.cpp 运行时"
          action={
            <button
              type="button"
              onClick={() =>
                void runDownload("download_stt_binary", { runtime: config.stt.runtime })
              }
              disabled={sttBusy}
              className={smallButton}
            >
              下载运行时
            </button>
          }
        >
          <select value={config.stt.runtime} onChange={(e) => update("stt.runtime", e.target.value)} className="input">
            <option value="cpu">CPU</option>
            <option value="cuda12">GPU · CUDA 12.4（NVIDIA，需较新驱动）</option>
            <option value="cuda11">GPU · CUDA 11.8（NVIDIA，兼容旧驱动）</option>
          </select>
        </Field>

        {sttStatus && (
          <div className="space-y-1.5">
            <MetaRow
              label="运行时"
              tone={runtimeInstalled ? "ok" : "warn"}
              value={`${RUNTIME_LABELS[config.stt.runtime] ?? config.stt.runtime}${runtimeInstalled ? " · 已下载" : " · 未下载"}`}
            />
            <MetaRow
              label="模型"
              tone={sttStatus.model_ready ? "ok" : "warn"}
              value={sttStatus.model_ready ? "已就绪" : "缺失"}
            />
          </div>
        )}
        <div className="flex items-center justify-between gap-3">
          <button
            type="button"
            onClick={() => void refreshStatus()}
            disabled={sttBusy}
            className="shrink-0 px-2.5 py-1.5 text-[12px] font-medium text-gray-600 bg-gray-50 rounded-lg hover:bg-gray-100 disabled:text-gray-300 transition-colors"
          >
            重新检测
          </button>
          {dlProgress && dlProgress.source !== "interception" && (
            <div className="flex-1 min-w-0">
              <DownloadBar progress={dlProgress} />
            </div>
          )}
        </div>
        {sttMessage && <Message>{sttMessage}</Message>}
      </Group>

      <Group label="高级">
        <details>
          <summary className="text-[12px] text-gray-400 cursor-pointer select-none hover:text-gray-600">
            覆盖默认路径
          </summary>
          <div className="space-y-3 mt-2">
            <Field label="可执行文件路径" help="留空时自动查找">
              <input
                value={config.stt.binary_path ?? ""}
                onChange={(e) => update("stt.binary_path", e.target.value)}
                placeholder="…\\clay-mic\\whisper\\whisper-cli.exe"
                className="input"
              />
            </Field>
            <Field label="模型路径" help="留空时使用默认目录">
              <input
                value={config.stt.model_path ?? ""}
                onChange={(e) => update("stt.model_path", e.target.value)}
                placeholder="…\\clay-mic\\whisper\\models\\ggml-base.bin"
                className="input"
              />
            </Field>
          </div>
        </details>
      </Group>
    </SectionBox>
  );
}
