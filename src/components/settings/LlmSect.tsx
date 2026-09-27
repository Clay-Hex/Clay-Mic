import { useCallback, useEffect, useRef, useState } from "react";
import {
  tauriInvoke,
  type CapsProvider,
  type CapsStatus,
  type ThinkingOption,
} from "../../lib/config";
import { PROVIDERS, findProvider } from "../../lib/providers";
import { Field, Group, Message, SectionBox } from "./common";
import { useSettingsCore } from "./context";

function maskKey(key: string): string {
  const trimmed = key.trim();
  if (trimmed.length === 0) return "";
  if (trimmed.length <= 8) return "•".repeat(trimmed.length);
  return `${trimmed.slice(0, 6)}••••••${trimmed.slice(-4)}`;
}

function ApiKeyInput({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  return (
    <input
      value={editing ? value : maskKey(value)}
      onChange={(e) => onChange(e.target.value)}
      onFocus={() => setEditing(true)}
      onBlur={() => setEditing(false)}
      placeholder="sk-..."
      spellCheck={false}
      autoComplete="off"
      className="input"
    />
  );
}

export function LlmSect() {
  const { config, setConfig, update } = useSettingsCore();
  const [models, setModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [thinkingOptions, setThinkingOptions] = useState<
    ThinkingOption[] | null
  >(null);
  const [capsStatus, setCapsStatus] = useState<CapsStatus | null>(null);
  const [capsBusy, setCapsBusy] = useState(false);
  const [capsMessage, setCapsMessage] = useState<string | null>(null);
  const [capsVersion, setCapsVersion] = useState(0);
  const [capsProviders, setCapsProviders] = useState<CapsProvider[]>([]);
  const committedProvider = useRef<string | null>(config.llm.provider);

  const loadModels = useCallback(
    async (
      baseUrl: string,
      apiKey: string,
      provider: string,
    ): Promise<string[]> => {
      if (!baseUrl.trim()) return [];
      setModelsLoading(true);
      setModelsError(null);
      try {
        const list = await tauriInvoke<string[]>("list_models", {
          baseUrl,
          apiKey,
          provider,
        });
        setModels(list);
        return list;
      } catch (error) {
        setModels([]);
        setModelsError(String(error));
        return [];
      } finally {
        setModelsLoading(false);
      }
    },
    [],
  );

  // The shell only renders sections after the config finished loading, so this
  // mount call already sees the final base url / key / provider.
  useEffect(() => {
    void loadModels(config.llm.base_url, config.llm.api_key, config.llm.provider);
  }, []);

  // Thinking support is answered by the backend so a capability-cache refresh
  // takes effect without reloading the page; an unsupported model clears the
  // stored level the moment it becomes the selection.
  useEffect(() => {
    let cancelled = false;
    tauriInvoke<ThinkingOption[] | null>("get_thinking_options", {
      model: config.llm.model,
      provider: config.llm.provider,
    })
      .then((options) => {
        if (cancelled) return;
        const valid = options && options.length > 0 ? options : null;
        setThinkingOptions(valid);
        if (!valid) {
          setConfig((prev) =>
            prev.llm.reasoning
              ? { ...prev, llm: { ...prev.llm, reasoning: "" } }
              : prev,
          );
        }
      })
      .catch((error) =>
        console.warn("[clay-mic] thinking options failed:", error),
      );
    return () => {
      cancelled = true;
    };
  }, [config.llm.model, config.llm.provider, capsVersion, setConfig]);

  useEffect(() => {
    tauriInvoke<CapsStatus>("get_caps_status")
      .then(setCapsStatus)
      .catch((error) =>
        console.warn("[clay-mic] caps status failed:", error),
      );
  }, []);

  useEffect(() => {
    tauriInvoke<CapsProvider[]>("get_caps_providers")
      .then(setCapsProviders)
      .catch((error) =>
        console.warn("[clay-mic] caps providers failed:", error),
      );
  }, [capsVersion]);

  const refreshCaps = useCallback(async () => {
    setCapsBusy(true);
    setCapsMessage(null);
    try {
      const status = await tauriInvoke<CapsStatus>("refresh_model_caps");
      setCapsStatus(status);
      setCapsMessage(
        `已刷新：${status.models} 个模型（${status.reasoning} 个会思考）`,
      );
      setCapsVersion((version) => version + 1);
    } catch (error) {
      setCapsMessage(`刷新失败：${String(error)}`);
    } finally {
      setCapsBusy(false);
    }
  }, []);

  // Table first, then clay-mic's presets so ids models.dev lacks — notably
  // Ollama — stay offered alongside the catalog.
  const providerOptions = [
    ...capsProviders,
    ...PROVIDERS.filter(
      (preset) => !capsProviders.some((option) => option.id === preset.id),
    ).map((preset) => ({
      id: preset.id,
      name: preset.name,
      api: preset.baseUrl,
    })),
  ];
  const knownProviderIds = new Set(providerOptions.map((option) => option.id));

  const handleProviderChange = (id: string) => {
    const preset = findProvider(id);
    const hint = providerOptions.find((option) => option.id === id);
    // Preset first: clay-mic's curated base urls stay authoritative; the
    // table's `api` field only fills providers it does not preset (often null).
    const baseUrl = preset?.baseUrl ?? hint?.api ?? config.llm.base_url;
    const apiKey = config.llm.api_keys[id] ?? "";
    const model = config.llm.models[id] ?? "";
    setConfig((prev) => ({
      ...prev,
      llm: {
        ...prev.llm,
        provider: id,
        base_url: baseUrl,
        model,
        api_key: apiKey,
      },
    }));
    void loadModels(baseUrl, apiKey, id).then((list) => {
      if (list.length === 0) return;
      setConfig((prev) =>
        prev.llm.model
          ? prev
          : { ...prev, llm: { ...prev.llm, model: list[0] } },
      );
    });
  };

  const commitProvider = (id: string) => {
    if (!id || id === committedProvider.current) return;
    committedProvider.current = id;
    handleProviderChange(id);
  };

  const handleProviderInput = (value: string) => {
    setConfig((prev) => ({ ...prev, llm: { ...prev.llm, provider: value } }));
    // Full ids arrive from datalist picks; partial typing only edits the field
    // until blur/Enter commits, so keystrokes never swap base urls or refetch.
    if (knownProviderIds.has(value)) commitProvider(value);
  };

  const handleApiKeyChange = (value: string) => {
    setConfig((prev) => ({
      ...prev,
      llm: {
        ...prev.llm,
        api_key: value,
        api_keys: { ...prev.llm.api_keys, [prev.llm.provider]: value },
      },
    }));
  };

  const handleModelInput = (value: string) => {
    setConfig((prev) => ({
      ...prev,
      llm: {
        ...prev.llm,
        model: value,
        models: { ...prev.llm.models, [prev.llm.provider]: value },
      },
    }));
  };

  const capsStatusText = capsStatus
    ? `${capsStatus.from_disk ? "磁盘缓存" : "内嵌"} · ${capsStatus.generated} · ${capsStatus.models} 个模型（${capsStatus.reasoning} 个会思考）`
    : "—";

  return (
    <SectionBox>
      <Field label="启用 LLM 格式化">
        <label className="flex items-center gap-2 cursor-pointer pt-2">
          <input
            type="checkbox"
            checked={config.llm.enabled}
            onChange={(e) => setConfig((prev) => ({ ...prev, llm: { ...prev.llm, enabled: e.target.checked } }))}
            className="w-4 h-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
          />
          <span className="text-[12px] text-gray-400">启用后转写结果会经过 LLM 格式化</span>
        </label>
      </Field>
      {config.llm.enabled && (<>
      <Group label="模型选择">
        <Field label="Provider">
          <input
            value={config.llm.provider}
            list="clay-mic-providers"
            onChange={(e) => handleProviderInput(e.target.value)}
            onBlur={() => commitProvider(config.llm.provider)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitProvider(config.llm.provider);
            }}
            placeholder="搜索或输入 provider"
            spellCheck={false}
            autoComplete="off"
            className="input"
          />
          <datalist id="clay-mic-providers">
            {providerOptions.map((option) => (
              <option key={option.id} value={option.id} label={option.name} />
            ))}
          </datalist>
        </Field>
        <Field label="Model">
          <div className="flex gap-2">
            <input
              value={config.llm.model}
              list="clay-mic-models"
              onChange={(e) => handleModelInput(e.target.value)}
              placeholder={modelsLoading ? "获取中…" : "搜索或输入模型"}
              spellCheck={false}
              autoComplete="off"
              className="input"
            />
            <datalist id="clay-mic-models">
              {models.map((model) => (
                <option key={model} value={model} />
              ))}
            </datalist>
          </div>
          {modelsError && <Message tone="warn">{modelsError}</Message>}
        </Field>
        <Field label="启用思考">
          <select
            value={thinkingOptions ? config.llm.reasoning : ""}
            onChange={(e) => update("llm.reasoning", e.target.value)}
            disabled={!thinkingOptions}
            className="input disabled:bg-gray-50 disabled:text-gray-300"
          >
            <option value="">关闭思考</option>
            {thinkingOptions?.map((option) => (
              <option key={option.value} value={option.value}>思考：{option.label}</option>
            ))}
          </select>
        </Field>
        <Field
          label="模型能力表"
          help={capsStatusText}
          action={
            <button
              type="button"
              onClick={() => void refreshCaps()}
              disabled={capsBusy}
              className="shrink-0 px-2.5 py-1.5 text-[12px] font-medium text-blue-600 bg-blue-50 rounded-lg hover:bg-blue-100 disabled:text-gray-300 disabled:bg-gray-50 transition-colors"
            >
              {capsBusy ? "拉取中…" : "刷新能力缓存"}
            </button>
          }
        >
          <div className="text-[12px] text-gray-400">
            能力数据来自 models.dev，供思考开关与档位判定使用
          </div>
        </Field>
        {capsMessage && (
          <Message tone={capsMessage.startsWith("刷新失败") ? "warn" : "muted"}>
            {capsMessage}
          </Message>
        )}
      </Group>

      <Group label="认证与端点">
        <Field label="API Key">
          <ApiKeyInput
            value={config.llm.api_key}
            onChange={handleApiKeyChange}
          />
        </Field>
        <Field label="Base URL">
          <input value={config.llm.base_url} onChange={(e) => update("llm.base_url", e.target.value)} className="input" />
        </Field>
      </Group>

      <Group label="输出">
        <Field label="请求超时（秒）">
          <input
            type="number"
            min={10}
            value={config.llm.timeout_secs}
            onChange={(e) => update("llm.timeout_secs", Number(e.target.value) || 0)}
            className="input"
          />
        </Field>
        <Field label="最大输出 Tokens">
          <input
            type="number"
            min={256}
            step={256}
            value={config.llm.max_tokens}
            onChange={(e) => update("llm.max_tokens", Number(e.target.value) || 0)}
            className="input"
          />
        </Field>
        <Field label="格式化 Prompt">
          <textarea value={config.llm.format_prompt} onChange={(e) => update("llm.format_prompt", e.target.value)} rows={3} className="input resize-none" />
        </Field>
      </Group>
      </>)}
    </SectionBox>
  );
}
