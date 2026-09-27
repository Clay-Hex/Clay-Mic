import { describe, it, expect } from "vitest";
import { itemStatus, type TextItem } from "../config";

function makeItem(
  overrides: Partial<Pick<TextItem, "status" | "raw_text">> = {},
): TextItem {
  return {
    id: "test-1",
    raw_text: "",
    formatted_text: "",
    timestamp: "2025-01-01T00:00:00Z",
    status: "Ready",
    stt_ms: 0,
    llm_ms: 0,
    llm_ttft_ms: 0,
    llm_gen_ms: 0,
    thinking_ms: 0,
    reasoning_text: "",
    ...overrides,
  };
}

describe("itemStatus", () => {
  it("maps Ready status to ok with label '已完成'", () => {
    const result = itemStatus(makeItem({ status: "Ready" }));
    expect(result.label).toBe("已完成");
    expect(result.tone).toBe("ok");
    expect(result.error).toBeUndefined();
  });

  it("maps Injected status to ok with label '已注入'", () => {
    const result = itemStatus(makeItem({ status: "Injected" }));
    expect(result.label).toBe("已注入");
    expect(result.tone).toBe("ok");
  });

  it("maps Skipped status to muted with label about LLM not configured", () => {
    const result = itemStatus(makeItem({ status: "Skipped" }));
    expect(result.label).toBe("未配置 LLM");
    expect(result.tone).toBe("muted");
  });

  it("maps Processing with empty raw_text to '转录中…'", () => {
    const result = itemStatus(makeItem({ status: "Processing", raw_text: "" }));
    expect(result.label).toBe("转录中…");
    expect(result.tone).toBe("busy");
  });

  it("maps Processing with non-empty raw_text to 'LLM 改写中…'", () => {
    const result = itemStatus(
      makeItem({ status: "Processing", raw_text: "hello world" }),
    );
    expect(result.label).toBe("LLM 改写中…");
    expect(result.tone).toBe("busy");
  });

  it("maps Failed object status to fail with error message", () => {
    const result = itemStatus(
      makeItem({ status: { Failed: "API timeout" } }),
    );
    expect(result.label).toBe("失败");
    expect(result.tone).toBe("fail");
    expect(result.error).toBe("API timeout");
  });

  it("handles Failed object with empty string as default error", () => {
    const result = itemStatus(makeItem({ status: { Failed: "" } }));
    expect(result.label).toBe("失败");
    expect(result.tone).toBe("fail");
    expect(result.error).toBe("处理失败");
  });

  it("defaults unknown string status to busy/转录中…", () => {
    // The switch has a default case; casting to exercise it
    const result = itemStatus(
      makeItem({ status: "UnknownStatus" as TextItem["status"] }),
    );
    expect(result.tone).toBe("busy");
  });
});
