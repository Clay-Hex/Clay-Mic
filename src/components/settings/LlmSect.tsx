import { useCallback, useEffect, useRef, useState } from "react";
import {
  tauriInvoke,
  type CapsProvider,
  type CapsStatus,
  type CustomProvider,
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

interface CustomProviderValues {
  name: string;
  base_url: string;
  model: string;
  api_key: string;
  thinking: string;
}

/// Thinking shapes a custom endpoint can declare. The capability table says
/// nothing about user-defined endpoints, so the level list is fixed here.
const THINKING_STYLES = [
  { id: "", label: "不注入（服务端默认）" },
  { id: "effort", label: "reasoning_effort（OpenAI 系）" },
  { id: "toggle", label: "thinking.type（DeepSeek / GLM / Kimi）" },
  { id: "enable", label: "enable_thinking（Qwen / DashScope）" },
];

/// Levels on offer once an entry declares a shape; "off" is the empty value
/// the select already carries.
const CUSTOM_THINKING_LEVELS: ThinkingOption[] = [
  { value: "low", label: "low" },
  { value: "medium", label: "medium" },
  { value: "high", label: "high" },
];

/// A fresh stable key for a new entry: its name, disambiguated when that name
/// is already taken. The key — not the name — is what api_keys and models are
/// stored under, so a later rename never orphans them.
///
/// `taken` must include the built-in ids too: an entry named `openai` would
/// otherwise satisfy `findProvider`, which answers with the preset's base url
/// and would silently point the entry at somebody else's endpoint.
function uniqueProviderId(name: string, taken: { id: string }[]): string {
  const base = name.trim() || "custom";
  let id = base;
  let suffix = 2;
  while (taken.some((entry) => entry.id === id)) {
    id = `${base}-${suffix++}`;
  }
  return id;
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
      // A custom entry binds exactly one model — asking the endpoint for a
      // list could only contradict the model the user typed when creating it.
      const custom = config.llm.custom_providers.find(
        (entry) => entry.id === provider,
      );
      if (custom) {
        const own = config.llm.models[provider]?.trim() ?? "";
        const list = own ? [own] : [];
        setModels(list);
        setModelsError(null);
        setModelsLoading(false);
        return list;
      }
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
    [config.llm.custom_providers, config.llm.models],
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
        // A custom entry declares its own shape, so the capability table has
        // nothing to say about it: the fixed level list applies instead and
        // the stored level stays — it is the user's own choice to make here.
        const custom = config.llm.custom_providers.find(
          (entry) => entry.id === config.llm.provider,
        );
        if (custom) {
          setThinkingOptions(custom.thinking ? CUSTOM_THINKING_LEVELS : null);
          return;
        }
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
  }, [
    config.llm.model,
    config.llm.provider,
    config.llm.custom_providers,
    capsVersion,
    setConfig,
  ]);

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
  // Ollama — stay offered alongside the catalog. User-defined connections come
  // last: they are the ones no catalog can ever know about, and `api` carries
  // their base url so selecting one fills the endpoint like a preset does.
  const providerOptions = [
    ...capsProviders,
    ...PROVIDERS.filter(
      (preset) => !capsProviders.some((option) => option.id === preset.id),
    ).map((preset) => ({
      id: preset.id,
      name: preset.name,
      api: preset.baseUrl,
    })),
    ...config.llm.custom_providers.map((entry) => ({
      id: entry.id,
      name: entry.name,
      api: entry.base_url,
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

  const [customDialog, setCustomDialog] = useState<{
    mode: "add" | "edit";
    entry?: CustomProvider;
  } | null>(null);

  // Both halves of a custom entry live together: the entry carries the name
  // and endpoint, the model and key stay in the maps every other provider
  // already uses — so selecting one goes through `handleProviderChange` with
  // nothing special-cased.
  const saveCustomProvider = (
    values: CustomProviderValues,
    entry?: CustomProvider,
  ) => {
    const name = values.name.trim();
    const base_url = values.base_url.trim();
    const model = values.model.trim();
    const api_key = values.api_key.trim();
    if (!name || !base_url) return;
    // Built-in ids count as taken too — see `uniqueProviderId`.
    const id =
      entry?.id ??
      uniqueProviderId(name, [
        ...capsProviders,
        ...PROVIDERS,
        ...config.llm.custom_providers,
      ]);
    setConfig((prev) => {
      const saved: CustomProvider = {
        id,
        name,
        base_url,
        thinking: values.thinking,
      };
      const custom_providers = entry
        ? prev.llm.custom_providers.map((item) =>
            item.id === id ? saved : item,
          )
        : [...prev.llm.custom_providers, saved];
      const llm = {
        ...prev.llm,
        custom_providers,
        models: { ...prev.llm.models, [id]: model },
        api_keys: { ...prev.llm.api_keys, [id]: api_key },
      };
      // Creating selects the entry at once; editing the entry currently in
      // use keeps the live fields in step with the dialog.
      if (!entry || prev.llm.provider === id) {
        llm.provider = id;
        llm.base_url = base_url;
        llm.model = model;
        llm.api_key = api_key;
      }
      return { ...prev, llm };
    });
    // The entry's model list is exactly this one value, so set it directly
    // instead of going through `loadModels`: `setConfig` only lands on the
    // next render and that helper would still miss the entry, falling through
    // to an endpoint fetch the entry exists to avoid.
    setModels(model ? [model] : []);
    setModelsError(null);
    setCustomDialog(null);
  };

  const removeCustomProvider = (entry: CustomProvider) => {
    setConfig((prev) => {
      const models = { ...prev.llm.models };
      const api_keys = { ...prev.llm.api_keys };
      delete models[entry.id];
      delete api_keys[entry.id];
      const llm = {
        ...prev.llm,
        custom_providers: prev.llm.custom_providers.filter(
          (item) => item.id !== entry.id,
        ),
        models,
        api_keys,
      };
      // Deleting the entry in use falls back to the built-in default rather
      // than leaving a dangling provider id behind, restoring whatever key
      // and model that default had remembered.
      if (prev.llm.provider === entry.id) {
        llm.provider = "openai";
        llm.base_url = findProvider("openai")?.baseUrl ?? prev.llm.base_url;
        llm.model = models["openai"] ?? "";
        llm.api_key = api_keys["openai"] ?? "";
      }
      return { ...prev, llm };
    });
    if (config.llm.provider === entry.id) {
      const fallback = findProvider("openai")?.baseUrl ?? config.llm.base_url;
      void loadModels(fallback, config.llm.api_keys["openai"] ?? "", "openai");
    }
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
        <Field
          label="自定义 Provider"
          help="一条 = 一个端点 + 一个模型，选定即用，不拉取模型列表；仅支持 OpenAI Chat 兼容端点"
        >
          <div className="flex flex-wrap items-center gap-2">
            {config.llm.custom_providers.length === 0 && (
              <span className="text-[12px] text-gray-400">还没有添加</span>
            )}
            {config.llm.custom_providers.map((entry) => (
              <span
                key={entry.id}
                className={`inline-flex items-center gap-0.5 text-[12px] border rounded-lg pl-2.5 pr-1 py-1 ${
                  config.llm.provider === entry.id
                    ? "border-blue-200 bg-blue-50 text-blue-700"
                    : "border-gray-200 bg-white text-gray-600"
                }`}
              >
                <button
                  type="button"
                  onClick={() => commitProvider(entry.id)}
                  title={entry.base_url}
                  className="hover:text-blue-600 transition-colors"
                >
                  {entry.name}
                </button>
                <button
                  type="button"
                  onClick={() => setCustomDialog({ mode: "edit", entry })}
                  title="编辑"
                  className="px-1 text-gray-400 hover:text-blue-600 transition-colors"
                >
                  ✎
                </button>
                <button
                  type="button"
                  onClick={() => removeCustomProvider(entry)}
                  title="删除"
                  className="px-1 text-gray-400 hover:text-red-600 transition-colors"
                >
                  ✕
                </button>
              </span>
            ))}
            <button
              type="button"
              onClick={() => setCustomDialog({ mode: "add" })}
              className="px-2.5 py-1 text-[12px] rounded-lg bg-gray-50 text-gray-600 hover:bg-gray-100 hover:text-gray-800 transition-colors"
            >
              + 添加
            </button>
          </div>
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
            {/* No known switch here: claiming "关闭思考" would promise an off
                that never reaches the wire. */}
            <option value="">
              {thinkingOptions ? "关闭思考" : "由服务端决定"}
            </option>
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
      {customDialog && (
        <CustomProviderDialog
          initial={
            customDialog.entry
              ? {
                  name: customDialog.entry.name,
                  base_url: customDialog.entry.base_url,
                  thinking: customDialog.entry.thinking,
                  model: config.llm.models[customDialog.entry.id] ?? "",
                  api_key: config.llm.api_keys[customDialog.entry.id] ?? "",
                }
              : {
                  name: "",
                  base_url: "",
                  thinking: "",
                  model: "",
                  api_key: "",
                }
          }
          editing={Boolean(customDialog.entry)}
          onSave={(values) => saveCustomProvider(values, customDialog.entry)}
          onClose={() => setCustomDialog(null)}
        />
      )}
    </SectionBox>
  );
}

function CustomProviderDialog({
  initial,
  editing,
  onSave,
  onClose,
}: {
  initial: CustomProviderValues;
  editing: boolean;
  onSave: (values: CustomProviderValues) => void;
  onClose: () => void;
}) {
  const [values, setValues] = useState<CustomProviderValues>(initial);
  const set = (patch: Partial<CustomProviderValues>) =>
    setValues((prev) => ({ ...prev, ...patch }));
  const ready = values.name.trim() !== "" && values.base_url.trim() !== "";

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/30">
      <div className="bg-white rounded-xl shadow-lg border border-gray-200 p-5 w-80">
        <h3 className="text-[13px] font-semibold text-gray-800 mb-4">
          {editing ? "编辑自定义 Provider" : "添加自定义 Provider"}
        </h3>
        <div className="space-y-3">
          <label className="block">
            <span className="text-[11px] text-gray-500">名称</span>
            <input
              value={values.name}
              onChange={(e) => set({ name: e.target.value })}
              placeholder="我的网关"
              spellCheck={false}
              className="input mt-1"
            />
          </label>
          <label className="block">
            <span className="text-[11px] text-gray-500">Base URL</span>
            <input
              value={values.base_url}
              onChange={(e) => set({ base_url: e.target.value })}
              placeholder="https://example.com/v1"
              spellCheck={false}
              className="input mt-1"
            />
            <span className="block mt-1 text-[11px] text-gray-400">
              仅支持 OpenAI Chat 兼容端点（POST /chat/completions），不支持
              Anthropic /v1/messages 与 OpenAI /v1/responses
            </span>
          </label>
          <label className="block">
            <span className="text-[11px] text-gray-500">模型</span>
            <input
              value={values.model}
              onChange={(e) => set({ model: e.target.value })}
              placeholder="只填这一个会用到的"
              spellCheck={false}
              className="input mt-1"
            />
          </label>
          <label className="block">
            <span className="text-[11px] text-gray-500">API Key</span>
            <div className="mt-1">
              <ApiKeyInput
                value={values.api_key}
                onChange={(value) => set({ api_key: value })}
              />
            </div>
          </label>
          <label className="block">
            <span className="text-[11px] text-gray-500">思考参数形态</span>
            <select
              value={values.thinking}
              onChange={(e) => set({ thinking: e.target.value })}
              className="input mt-1"
            >
              {THINKING_STYLES.map((style) => (
                <option key={style.id} value={style.id}>
                  {style.label}
                </option>
              ))}
            </select>
            <span className="block mt-1 text-[11px] text-gray-400">
              决定开/关思考时向 body 写哪个字段；选「不注入」则由服务端默认
            </span>
          </label>
        </div>
        <div className="flex justify-end gap-2 mt-5">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 text-[12px] text-gray-600 bg-gray-50 hover:bg-gray-100 rounded-lg transition-colors"
          >
            取消
          </button>
          <button
            type="button"
            disabled={!ready}
            onClick={() => onSave(values)}
            className="px-3 py-1.5 text-[12px] text-white bg-blue-600 hover:bg-blue-700 disabled:opacity-40 rounded-lg transition-colors"
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
