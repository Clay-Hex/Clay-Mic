import { describe, it, expect } from "vitest";
import { findProvider, PROVIDERS } from "../providers";

describe("findProvider", () => {
  it("returns the correct provider for known ids", () => {
    expect(findProvider("openai")).toEqual({
      id: "openai",
      name: "OpenAI",
      baseUrl: "https://api.openai.com/v1",
    });
    expect(findProvider("deepseek")).toEqual({
      id: "deepseek",
      name: "DeepSeek",
      baseUrl: "https://api.deepseek.com/v1",
    });
    expect(findProvider("ollama")).toEqual({
      id: "ollama",
      name: "Ollama（本地）",
      baseUrl: "http://localhost:11434/v1",
    });
  });

  it("maps 'anthropic' to the 'claude' preset", () => {
    const result = findProvider("anthropic");
    expect(result).toBeDefined();
    expect(result!.id).toBe("claude");
    expect(result!.name).toBe("Anthropic Claude");
  });

  it("returns undefined for unknown ids", () => {
    expect(findProvider("nonexistent")).toBeUndefined();
    expect(findProvider("")).toBeUndefined();
    expect(findProvider("openai-chatgpt")).toBeUndefined();
  });

  it("all PROVIDERS entries are findable by their own id", () => {
    for (const provider of PROVIDERS) {
      expect(findProvider(provider.id)).toBe(provider);
    }
  });
});
