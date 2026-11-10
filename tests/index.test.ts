import { describe, expect, it } from "vitest";
import * as library from "../src/index.js";

describe("public index exports", () => {
  it("exposes the analysis, the formatters and the defaults", () => {
    expect(Object.keys(library).sort()).toEqual([
      "ANALYZER_MODES",
      "DEFAULT_EXCLUDES",
      "DEFAULT_EXTENSIONS",
      "DEFAULT_MIN_LINES",
      "DEFAULT_MODES",
      "DEFAULT_OVERLAP_MAX_WINDOW",
      "DEFAULT_OVERLAP_MIN_WINDOW",
      "DEFAULT_OVERLAP_SIZE_TOLERANCE",
      "DEFAULT_THRESHOLD",
      "MAX_OVERLAP_WINDOW",
      "TYPE_FILTERS",
      "analyzeProject",
      "formatJsonReport",
      "formatPrettyReport",
      "resolveAnalyzeOptions",
    ]);
    expect(library.analyzeProject).toBeTypeOf("function");
    expect(library.resolveAnalyzeOptions).toBeTypeOf("function");
    expect(library.formatJsonReport).toBeTypeOf("function");
    expect(library.formatPrettyReport).toBeTypeOf("function");
  });

  it("resolves options to the exported defaults", () => {
    const resolved: library.ResolvedAnalyzeOptions = library.resolveAnalyzeOptions({
      paths: ["."],
    });
    expect(resolved.modes).toEqual([...library.DEFAULT_MODES]);
    expect(resolved.threshold).toBe(library.DEFAULT_THRESHOLD);
    expect(resolved.minLines).toBe(library.DEFAULT_MIN_LINES);
    expect(resolved.extensions).toEqual(library.DEFAULT_EXTENSIONS);
  });
});
