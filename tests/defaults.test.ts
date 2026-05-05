import { describe, expect, it } from "vitest";
import {
  DEFAULT_EXCLUDES,
  DEFAULT_EXTENSIONS,
  DEFAULT_MIN_LINES,
  DEFAULT_MODES,
  DEFAULT_OVERLAP_MAX_WINDOW,
  DEFAULT_OVERLAP_MIN_WINDOW,
  DEFAULT_OVERLAP_SIZE_TOLERANCE,
  DEFAULT_THRESHOLD,
} from "../src/defaults.js";

describe("defaults", () => {
  it("keeps the README-oriented zero-config TypeScript scan defaults stable", () => {
    expect(DEFAULT_MODES).toEqual(["functions", "types", "classes", "overlap"]);
    expect(DEFAULT_EXTENSIONS).toEqual(["ts", "tsx", "mts", "cts"]);
    expect(DEFAULT_EXCLUDES).toEqual(["node_modules/**", "dist/**", "coverage/**"]);
    expect(DEFAULT_THRESHOLD).toBe(0.8);
    expect(DEFAULT_MIN_LINES).toBe(3);
    expect(DEFAULT_OVERLAP_MIN_WINDOW).toBe(8);
    expect(DEFAULT_OVERLAP_MAX_WINDOW).toBe(30);
    expect(DEFAULT_OVERLAP_SIZE_TOLERANCE).toBe(0.2);
  });
});
