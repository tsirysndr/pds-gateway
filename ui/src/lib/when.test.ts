import { describe, expect, it } from "vitest";
import { formatWhen } from "./when";

describe("showing a server timestamp", () => {
  it("reads ISO 8601 UTC", () => {
    expect(formatWhen("2026-10-01T18:23:20.014Z")).toMatch(/2026/);
  });

  it("reads java.sql.Timestamp's spelling as UTC instead of refusing it", () => {
    // "2026-10-01 18:23:20.014" rendered as Invalid Date on the passkey list.
    const legacy = formatWhen("2026-10-01 18:23:20.014");
    expect(legacy).toMatch(/2026/);
    expect(legacy).toBe(formatWhen("2026-10-01T18:23:20.014Z"));
  });

  it("treats a zone-less ISO stamp as UTC too", () => {
    expect(formatWhen("2026-10-01T18:23:20")).toBe(formatWhen("2026-10-01T18:23:20Z"));
  });

  it("renders nothing rather than Invalid Date", () => {
    expect(formatWhen("not a date")).toBeNull();
    expect(formatWhen("")).toBeNull();
    expect(formatWhen(null)).toBeNull();
    expect(formatWhen(undefined)).toBeNull();
  });
});
