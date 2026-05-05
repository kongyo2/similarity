import { describe, expect, it } from "vitest";
import { analyzeProject, resolveAnalyzeOptions } from "../src/index.js";

describe("public index exports", () => {
  it("exposes the library analysis entrypoints", () => {
    expect(analyzeProject).toBeTypeOf("function");
    expect(resolveAnalyzeOptions).toBeTypeOf("function");
  });
});
