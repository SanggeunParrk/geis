import { describe, expect, it } from "vitest";
import { count, formatBytes, formatDuration, formatWhen, webLink } from "../dom";

describe("links from files the app did not write", () => {
  it("keeps http and https addresses", () => {
    expect(webLink("https://github.com/CSSB-SNU/BioMol")).toBe("https://github.com/CSSB-SNU/BioMol");
    expect(webLink("  http://example.org/model ")).toBe("http://example.org/model");
  });

  it("refuses anything that would run or open inside the app", () => {
    for (const bad of [
      "javascript:alert(1)",
      "JavaScript:alert(1)",
      " javascript:alert(1)",
      "java\tscript:alert(1)",
      "data:text/html,<script>alert(1)</script>",
      "file:///etc/passwd",
      "vbscript:msgbox",
      "//evil.example",
      "not a url",
      "",
      null,
      undefined,
    ]) {
      expect(webLink(bad), String(bad)).toBeNull();
    }
  });
});

describe("formatting for the jobs list", () => {
  it("never shows 1024 of a unit", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1024 * 1024 - 1)).toBe("1.0 MB");
    expect(formatBytes(1024 ** 3 - 1)).toBe("1.0 GB");
    expect(formatBytes(1536 * 1024)).toBe("1.5 MB");
    expect(formatBytes(5 * 1024 ** 5)).toBe("5120 TB");
  });

  it("shows a dash for a size or a time it cannot read", () => {
    expect(formatBytes(Number.NaN)).toBe("-");
    expect(formatWhen("not a date")).toBe("-");
    expect(formatWhen(null)).toBe("-");
    expect(formatDuration("not a date", null)).toBe("-");
    expect(formatDuration("2026-01-01T00:00:00Z", "garbage")).toBe("-");
  });

  it("measures a duration, and a finish before the start as none", () => {
    expect(formatDuration("2026-01-01T00:00:00Z", "2026-01-01T00:00:42Z")).toBe("42s");
    expect(formatDuration("2026-01-01T00:00:00Z", "2026-01-01T01:02:03Z")).toBe("1h 2m");
    expect(formatDuration("2026-01-01T00:01:00Z", "2026-01-01T00:00:00Z")).toBe("0s");
  });

  it("counts with the right noun", () => {
    expect(count(1, "chain")).toBe("1 chain");
    expect(count(0, "chain")).toBe("0 chains");
    expect(count(2, "copy", "copies")).toBe("2 copies");
  });
});
