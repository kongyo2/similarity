import { afterEach, describe, expect, it, vi } from "vitest";
import type { EngineInput } from "../src/engine/wasm.js";

const baseInput: EngineInput = {
  files: [],
  modes: ["functions", "types"],
  threshold: 0.8,
  minLines: 3,
  sizePenalty: true,
  sameFileOnly: false,
  crossFileOnly: false,
  typesOnly: "all",
  allowCrossKind: true,
  includeTypeLiterals: false,
  overlapMinWindow: 8,
  overlapMaxWindow: 30,
  overlapSizeTolerance: 0.2,
};

const DUPLICATED = `
export function collectActive(rows: Row[]): string[] {
  const names = [];
  for (const row of rows) {
    if (row.id > 0 && row.name.length > 0) {
      names.push(row.name.trim().toLowerCase());
    }
  }
  return names;
}
`;

/**
 * A conditional chain `arms` long. It has no bracket for the engine's
 * nesting check to count, and the parser recurses once per arm: a few
 * thousand arms exhaust the engine's stack before the file is parsed.
 */
function conditionalChain(arms: number): string {
  const chain = Array.from({ length: arms }, (_, index) => `a === ${index} ? ${index} : `).join("");
  return `export function pick(a: number) {\n  return ${chain}0;\n}\n`;
}

async function freshEngine() {
  vi.resetModules();
  return import("../src/engine/wasm.js");
}

describe("analyzeWithWasm", () => {
  afterEach(() => {
    vi.doUnmock("node:fs");
    vi.resetModules();
  });

  it("reports what the engine finds", async () => {
    const { analyzeWithWasm } = await freshEngine();
    const report = analyzeWithWasm({
      ...baseInput,
      files: [
        { filePath: "/project/b.ts", content: DUPLICATED },
        { filePath: "/project/a.ts", content: DUPLICATED },
      ],
    });

    expect(report.analyzedFiles).toEqual(["/project/b.ts", "/project/a.ts"]);
    expect(report.skippedFiles).toEqual([]);
    expect(report.stats).toEqual({ fileCount: 2, pairCount: 1 });
    expect(report.byMode.functions).toHaveLength(1);
    // Files are compared in path order, whatever order they are listed in.
    expect(report.byMode.functions[0]?.left.filePath).toBe("/project/a.ts");
    expect(report.byMode.functions[0]?.details).toEqual({ impact: 9 });
  });

  it("passes on what the engine rejects", async () => {
    const { analyzeWithWasm } = await freshEngine();
    expect(() => analyzeWithWasm({ ...baseInput, threshold: 2 })).toThrow(
      "threshold must be between 0 and 1",
    );
    expect(() =>
      analyzeWithWasm({ ...baseInput, modes: ["everything" as "functions"] }),
    ).toThrow("invalid input");
  });

  it("fails clearly when the WASM module is missing, and looks again on the next call", async () => {
    const existsSync = vi.fn().mockReturnValue(false);
    vi.doMock("node:fs", () => ({ existsSync }));
    const { analyzeWithWasm } = await freshEngine();

    expect(() => analyzeWithWasm(baseInput)).toThrow("WASM module was not found");
    expect(() => analyzeWithWasm(baseInput)).toThrow("WASM module was not found");
    // Both candidate locations, both times: a failed lookup is not cached.
    expect(existsSync).toHaveBeenCalledTimes(4);

    existsSync.mockImplementation(() => true);
  });

  it("skips a file that exhausts the engine's stack and analyzes the rest", async () => {
    const { analyzeWithWasm } = await freshEngine();
    const files = [
      { filePath: "/project/a.ts", content: DUPLICATED },
      { filePath: "/project/broken.ts", content: "export class {{{ nope" },
      { filePath: "/project/deep.ts", content: conditionalChain(20_000) },
      { filePath: "/project/b.ts", content: DUPLICATED },
      { filePath: "/project/deeper.ts", content: conditionalChain(30_000) },
    ];

    const report = analyzeWithWasm({ ...baseInput, files });

    expect(report.analyzedFiles).toEqual(["/project/a.ts", "/project/b.ts"]);
    // In the order of the input, with the file the engine skipped itself.
    expect(report.skippedFiles).toEqual([
      "/project/broken.ts",
      "/project/deep.ts",
      "/project/deeper.ts",
    ]);
    expect(report.warnings.map((warning) => warning.filePath)).toEqual(report.skippedFiles);
    expect(report.warnings[0]?.message).toMatch(/^skipped: Parse errors: /);
    expect(report.warnings[1]?.message).toBe("skipped: code is nested too deeply to parse");
    expect(report.warnings[2]?.message).toBe("skipped: code is nested too deeply to parse");
    expect(report.stats).toEqual({ fileCount: 2, pairCount: 1 });
    expect(report.byMode.functions).toHaveLength(1);

    // The engine that went down was replaced: the next run is unaffected.
    const again = analyzeWithWasm({ ...baseInput, files: [files[0]!, files[3]!] });
    expect(again.skippedFiles).toEqual([]);
    expect(again.byMode.functions).toHaveLength(1);
  });

  it("skips what it can tell is nested too deeply without going down", async () => {
    const { analyzeWithWasm } = await freshEngine();
    const brackets = `export const value = ${"[".repeat(5_000)}1${"]".repeat(5_000)};\n`;
    const chain = `export const total = ${Array.from({ length: 5_000 }, () => "a").join(" + ")};\n`;
    const templates = `export const text = ${"`x${".repeat(2_000)}a${"}`".repeat(2_000)};\n`;

    const report = analyzeWithWasm({
      ...baseInput,
      modes: ["functions", "types", "classes", "overlap"],
      files: [
        { filePath: "/project/brackets.ts", content: brackets },
        { filePath: "/project/chain.ts", content: chain },
        { filePath: "/project/templates.ts", content: templates },
        { filePath: "/project/a.ts", content: DUPLICATED },
      ],
    });

    expect(report.analyzedFiles).toEqual(["/project/a.ts"]);
    expect(report.warnings.map((warning) => warning.message)).toEqual([
      "skipped: brackets are nested more than 256 levels deep",
      "skipped: code is nested more than 512 levels deep",
      "skipped: brackets are nested more than 256 levels deep",
    ]);
  });

  it("compares functions that hold very long lists", async () => {
    // Two functions returning a table of 20,000 entries used to take the
    // engine down: aligning the two lists entry by entry needs a matrix of
    // 400 million cells.
    const table = (changed: number) =>
      `export function table${changed}() {\n  return [${Array.from(
        { length: 20_000 },
        (_, index) => (index === changed ? -1 : index % 97),
      ).join(", ")}];\n}\n`;
    const { analyzeWithWasm } = await freshEngine();

    const report = analyzeWithWasm({
      ...baseInput,
      modes: ["functions", "overlap"],
      files: [
        { filePath: "/project/a.ts", content: table(10) },
        { filePath: "/project/b.ts", content: table(15_000) },
      ],
    });

    expect(report.skippedFiles).toEqual([]);
    expect(report.byMode.functions).toHaveLength(1);
    expect(report.byMode.functions[0]?.similarity).toBeGreaterThan(0.99);
  });
});
