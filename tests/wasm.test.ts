import { afterEach, describe, expect, it, vi } from "vitest";

const analyzeInput = {
  files: [],
  modes: ["functions"],
  threshold: 0.8,
  minLines: 5,
  sizePenalty: true,
  sameFileOnly: false,
  crossFileOnly: false,
  typesOnly: "all" as const,
  allowCrossKind: false,
  includeTypeLiterals: false,
  overlapMinWindow: 20,
  overlapMaxWindow: 120,
  overlapSizeTolerance: 0.25,
};

describe("analyzeWithWasm", () => {
  afterEach(() => {
    vi.doUnmock("node:fs/promises");
    vi.resetModules();
  });

  it("resets the cached loader promise after a missing WASM module failure", async () => {
    const access = vi.fn().mockRejectedValue(new Error("missing"));
    vi.doMock("node:fs/promises", () => ({ access }));
    vi.resetModules();

    const { analyzeWithWasm } = await import("../src/engine/wasm.js");

    await expect(analyzeWithWasm(analyzeInput)).rejects.toThrow(
      "WASM module was not found",
    );
    await expect(analyzeWithWasm(analyzeInput)).rejects.toThrow(
      "WASM module was not found",
    );

    expect(access).toHaveBeenCalledTimes(4);
  });
});
