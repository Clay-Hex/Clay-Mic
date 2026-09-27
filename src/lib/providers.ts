export interface ProviderPreset {
  id: string;
  name: string;
  baseUrl: string;
}

export const PROVIDERS: ProviderPreset[] = [
  { id: "openai", name: "OpenAI", baseUrl: "https://api.openai.com/v1" },
  { id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com/v1" },
  {
    id: "claude",
    name: "Anthropic Claude",
    baseUrl: "https://api.anthropic.com/v1",
  },
  { id: "groq", name: "Groq", baseUrl: "https://api.groq.com/openai/v1" },
  { id: "ollama", name: "Ollama（本地）", baseUrl: "http://localhost:11434/v1" },
  {
    id: "opencode-go",
    name: "OpenCode Go",
    baseUrl: "https://opencode.ai/zen/go/v1",
  },
  { id: "aihubmix", name: "AiHubMix", baseUrl: "https://aihubmix.com/v1" },
];

export function findProvider(id: string): ProviderPreset | undefined {
  const found = PROVIDERS.find((provider) => provider.id === id);
  if (found) return found;
  // models.dev keys Anthropic as `anthropic`; clay-mic's legacy preset is `claude`.
  if (id === "anthropic") {
    return PROVIDERS.find((provider) => provider.id === "claude");
  }
  return undefined;
}
