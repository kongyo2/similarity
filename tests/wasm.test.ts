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
    vi.doUnmock("node:module");
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

describe("analyzeWithWasm around a failing engine", () => {
  afterEach(() => {
    vi.doUnmock("node:fs");
    vi.doUnmock("node:module");
    vi.resetModules();
  });

  interface FakeEngine {
    analyze_project?: (inputJson: string) => string;
    check_source?: (filePath: string, content: string) => string | undefined;
  }

  /** The wrapper, loading `engine` in place of the WebAssembly module. */
  async function wrapperAround(engine: FakeEngine | (() => FakeEngine)) {
    let instances = 0;
    const load = () => {
      instances += 1;
      return typeof engine === "function" ? engine() : engine;
    };
    const cache: Record<string, unknown> = {};
    vi.doMock("node:fs", () => ({ existsSync: () => true }));
    vi.doMock("node:module", () => ({
      createRequire: () =>
        Object.assign(
          (modulePath: string) => (cache[modulePath] ??= load()),
          { cache, resolve: (modulePath: string) => modulePath },
        ),
    }));
    const { analyzeWithWasm } = await freshEngine();
    return { analyzeWithWasm, instances: () => instances };
  }

  const emptyReport = {
    analyzedFiles: [],
    skippedFiles: [],
    warnings: [],
    results: [],
    byMode: { functions: [], types: [], classes: [], overlap: [] },
    stats: { fileCount: 0, pairCount: 0 },
  };

  const files = ["a.ts", "broken.ts", "deep.ts", "b.ts"].map((name) => ({
    filePath: `/project/${name}`,
    content: `export const name = "${name}";\n`,
  }));

  /** An engine that goes down on any input holding `deep.ts`. */
  function fragileEngine(crash: () => Error): FakeEngine {
    return {
      analyze_project: (inputJson) => {
        const input = JSON.parse(inputJson) as EngineInput;
        const paths = input.files.map((file) => file.filePath);
        if (paths.includes("/project/deep.ts")) {
          throw crash();
        }
        const analyzed = paths.filter((path) => path !== "/project/broken.ts");
        return JSON.stringify({
          ...emptyReport,
          analyzedFiles: analyzed,
          skippedFiles: ["/project/broken.ts"],
          warnings: [
            { filePath: "/project/broken.ts", message: "skipped: Parse errors: Unexpected token" },
          ],
          stats: { fileCount: analyzed.length, pairCount: 0 },
        });
      },
      check_source: (filePath) => {
        if (filePath === "/project/deep.ts") {
          throw crash();
        }
        return filePath === "/project/broken.ts" ? "Parse errors: Unexpected token" : undefined;
      },
    };
  }

  it("refuses a module without the engine's entry points", async () => {
    const { analyzeWithWasm } = await wrapperAround({ analyze_project: () => "{}" });
    expect(() => analyzeWithWasm(baseInput)).toThrow(
      "WASM module does not export the engine entry points — rebuild with `npm run build:wasm`.",
    );
  });

  it("refuses a report of another shape than the wrapper was built for", async () => {
    const shapes: [unknown, string][] = [
      [null, "the report is not an object"],
      [{ ...emptyReport, results: undefined }, "`results` is not an array"],
      [{ ...emptyReport, warnings: {} }, "`warnings` is not an array"],
      [
        { ...emptyReport, byMode: { functions: [], types: [], classes: [] } },
        "`byMode` does not list every mode",
      ],
      [{ ...emptyReport, byMode: [] }, "`byMode` does not list every mode"],
      [{ ...emptyReport, stats: { fileCount: 0 } }, "`stats` has no file and pair counts"],
      [{ ...emptyReport, stats: undefined }, "`stats` has no file and pair counts"],
    ];
    for (const [report, problem] of shapes) {
      const { analyzeWithWasm } = await wrapperAround({
        analyze_project: () => JSON.stringify(report),
        check_source: () => undefined,
      });
      expect(() => analyzeWithWasm(baseInput), problem).toThrow(
        `WASM engine returned an unexpected report shape: ${problem}. ` +
          "The engine and JS wrapper are likely out of sync — rebuild with `npm run build:wasm`.",
      );
      vi.resetModules();
    }
  });

  it("leaves out the file that takes the engine down, for either kind of crash", async () => {
    const crashes = [
      () => new RangeError("Maximum call stack size exceeded"),
      () => new WebAssembly.RuntimeError("memory access out of bounds"),
    ];
    for (const crash of crashes) {
      const { analyzeWithWasm, instances } = await wrapperAround(() => fragileEngine(crash));

      const report = analyzeWithWasm({ ...baseInput, files });

      expect(report.analyzedFiles).toEqual(["/project/a.ts", "/project/b.ts"]);
      expect(report.skippedFiles).toEqual(["/project/broken.ts", "/project/deep.ts"]);
      expect(report.warnings).toEqual([
        { filePath: "/project/broken.ts", message: "skipped: Parse errors: Unexpected token" },
        { filePath: "/project/deep.ts", message: "skipped: code is nested too deeply to parse" },
      ]);
      expect(report.stats).toEqual({ fileCount: 2, pairCount: 0 });
      // One instance went down with the analysis, one with the check of
      // `deep.ts`; the third finished the job.
      expect(instances()).toBe(3);
      vi.resetModules();
    }
  });

  it("reports a crash it cannot pin on a file as a bug", async () => {
    const { analyzeWithWasm } = await wrapperAround({
      analyze_project: () => {
        throw new WebAssembly.RuntimeError("unreachable");
      },
      check_source: () => undefined,
    });

    let thrown: unknown;
    try {
      analyzeWithWasm({ ...baseInput, files });
    } catch (error) {
      thrown = error;
    }
    expect(thrown).toBeInstanceOf(Error);
    const error = thrown as Error;
    expect(error.message).toContain("The analysis engine crashed (unreachable).");
    expect(error.message).toContain("https://github.com/kongyo2/similarity/issues");
    expect(error.cause).toBeInstanceOf(WebAssembly.RuntimeError);
  });

  it("reports a crash that outlives the file it was pinned on as a bug", async () => {
    const { analyzeWithWasm } = await wrapperAround(() => ({
      ...fragileEngine(() => new RangeError("Maximum call stack size exceeded")),
      analyze_project: () => {
        throw new RangeError("Maximum call stack size exceeded");
      },
    }));

    expect(() => analyzeWithWasm({ ...baseInput, files })).toThrow(
      "The analysis engine crashed (Maximum call stack size exceeded).",
    );
  });

  it("passes on what is no crash", async () => {
    const refusal = await wrapperAround({
      analyze_project: () => {
        throw new Error("threshold must be between 0 and 1");
      },
      check_source: () => undefined,
    });
    expect(() => refusal.analyzeWithWasm({ ...baseInput, files })).toThrow(
      "threshold must be between 0 and 1",
    );
    expect(refusal.instances()).toBe(1);
    vi.resetModules();

    // An error while looking for the file is not swallowed either.
    const { analyzeWithWasm } = await wrapperAround(() => ({
      analyze_project: () => {
        throw new RangeError("Maximum call stack size exceeded");
      },
      check_source: () => {
        throw new TypeError("content must be a string");
      },
    }));
    expect(() => analyzeWithWasm({ ...baseInput, files })).toThrow("content must be a string");
    vi.resetModules();

    // And neither is a refusal of the analysis without the file.
    let calls = 0;
    const retried = await wrapperAround(() => ({
      ...fragileEngine(() => new RangeError("Maximum call stack size exceeded")),
      analyze_project: () => {
        calls += 1;
        throw calls === 1
          ? new RangeError("Maximum call stack size exceeded")
          : new Error("invalid input: missing field `threshold`");
      },
    }));
    expect(() => retried.analyzeWithWasm({ ...baseInput, files })).toThrow(
      "invalid input: missing field `threshold`",
    );
  });
});

