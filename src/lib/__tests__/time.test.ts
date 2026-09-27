import { describe, it, expect, vi, afterEach } from "vitest";
import { formatTimestamp } from "../time";

describe("formatTimestamp", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns the input as-is for an invalid ISO string", () => {
    const result = formatTimestamp("not-a-date");
    expect(result.text).toBe("not-a-date");
    expect(result.title).toBe("not-a-date");
  });

  it("formats a date within today as just the time", () => {
    // Fix "now" to 2025-06-15T10:30:00
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2025-06-15T10:30:00"));

    const result = formatTimestamp("2025-06-15T08:15:42.123Z");
    // time-only text for same day
    expect(result.text).toMatch(/^\d{2}:\d{2}:\d{2}$/);
    // title must be full datetime
    expect(result.title).toMatch(/^2025-06-15 \d{2}:\d{2}:\d{2}\.\d{3}$/);
  });

  it("formats yesterday's date with '昨天' prefix", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2025-06-15T14:00:00"));

    const result = formatTimestamp("2025-06-14T09:30:00");
    expect(result.text).toMatch(/^昨天\d{2}:\d{2}:\d{2}$/);
  });

  it("formats two-days-ago date with '前天' prefix", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2025-06-15T14:00:00"));

    const result = formatTimestamp("2025-06-13T12:00:00");
    expect(result.text).toMatch(/^前天\d{2}:\d{2}:\d{2}$/);
  });

  it("formats older dates with full date in text", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2025-06-15T14:00:00"));

    const result = formatTimestamp("2025-06-10T08:00:00");
    // full date + time
    expect(result.text).toMatch(/^2025-06-10 \d{2}:\d{2}:\d{2}$/);
    expect(result.title).toMatch(/^2025-06-10 \d{2}:\d{2}:\d{2}\.\d{3}$/);
  });
});
