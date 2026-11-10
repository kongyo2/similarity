import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { analyzeProject, resolveAnalyzeOptions } from "../src/index.js";
import type { AnalyzeProjectOptions, AnalyzeReport } from "../src/index.js";

async function withTempProject(run: (projectDir: string) => Promise<void>) {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "similarity-ts-"));
  try {
    await run(tempRoot);
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

async function createFixtureProject(projectDir: string) {
  await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
  await fs.writeFile(
    path.join(projectDir, "src", "functions_a.ts"),
    `
export function calculateSum(values: number[]): number {
  let total = 0;
  let count = 0;
  for (const value of values) {
    if (value > 0) {
      total += value;
      count += 1;
    }
  }
  if (count === 0) {
    return 0;
  }
  return total;
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "functions_b.ts"),
    `
export function computeTotal(numbers: number[]): number {
  let total = 0;
  let count = 0;
  for (const item of numbers) {
    if (item > 0) {
      total += item;
      count += 1;
    }
  }
  if (count === 0) {
    return 0;
  }
  return total;
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "types.ts"),
    `
export interface User {
  id: string;
  name: string;
  active: boolean;
}

export interface Person {
  id: string;
  name: string;
  active: boolean;
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "classes.ts"),
    `
export class UserStore {
  constructor(private readonly items: string[]) {}
  add(name: string): void {
    this.items.push(name);
  }
}

export class PersonStore {
  constructor(private readonly items: string[]) {}
  add(name: string): void {
    this.items.push(name);
  }
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "overlap.ts"),
    `
export function overlapA(items: string[]): string[] {
  const output: string[] = [];
  for (const item of items) {
    output.push(item.trim().toLowerCase());
  }
  return output;
}

export function overlapB(items: string[]): string[] {
  const output: string[] = [];
  for (const item of items) {
    output.push(item.trim().toLowerCase());
  }
  return output;
}
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "ignore.js"),
    `
export const x = 1;
`,
    "utf8",
  );
}

describe("analyzeProject", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("detects similarities across all modes", async () => {
    await withTempProject(async (projectDir) => {
      await createFixtureProject(projectDir);
      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["src"],
        threshold: 0.6,
        modes: ["functions", "types", "classes", "overlap"],
        includeTypeLiterals: true,
      });

      expect(report.stats.fileCount).toBe(5);
      expect(report.byMode.functions.length).toBeGreaterThan(0);
      expect(report.byMode.types.length).toBeGreaterThan(0);
      expect(report.byMode.classes.length).toBeGreaterThan(0);
      expect(report.byMode.overlap.length).toBeGreaterThan(0);
    });
  });

  it("groups a fragment pasted into three functions into one cluster", async () => {
    await withTempProject(async (projectDir) => {
      const cacheGuard = [
        "  if (cached !== undefined && cached.expires > Date.now()) {",
        '    metrics.increment("cache.hit", { kind: "lookup" });',
        "    return cached;",
        "  }",
      ];
      await fs.writeFile(
        path.join(projectDir, "load.ts"),
        [
          "export function loadUser(id: string, cache: Map<string, User>) {",
          "  const cached = cache.get(id);",
          ...cacheGuard,
          "  return fetchUser(id);",
          "}",
          "",
          "export function loadOrder(id: string, cache: Map<string, Order>, audit: Audit) {",
          '  audit.record("order.load", id);',
          "  const cached = cache.get(id);",
          ...cacheGuard,
          "  const order = fetchOrder(id);",
          '  audit.record("order.loaded", order.total);',
          "  return order;",
          "}",
          "",
          "export function loadInvoice(key: string, cache: Map<string, Invoice>) {",
          "  if (key.length === 0) {",
          '    throw new Error("empty key");',
          "  }",
          "  const cached = cache.get(key);",
          ...cacheGuard,
          "  return fetchInvoice(key, { retries: 3, timeout: 1000 });",
          "}",
          "",
        ].join("\n"),
        "utf8",
      );

      const report = await analyzeProject({ cwd: projectDir, paths: ["."], modes: ["overlap"] });

      // The guard in each pair of the three functions, and the lookup
      // before it where it reads the same parameter.
      expect(report.byMode.overlap).toHaveLength(4);
      expect(report.stats.clusterCount).toBe(1);
      expect(report.clusters).toEqual([
        {
          mode: "overlap",
          members: [
            expect.objectContaining({ symbolName: "loadUser", startLine: 3, endLine: 6 }),
            expect.objectContaining({ symbolName: "loadOrder", startLine: 13, endLine: 16 }),
            expect.objectContaining({ symbolName: "loadInvoice", startLine: 27, endLine: 30 }),
          ],
          fragment: { nodeCount: 26, nodeType: "IfStatement" },
          pairCount: 3,
          maxSimilarity: 1,
          minSimilarity: 1,
          averageSimilarity: 1,
        },
      ]);
    });
  });

  it("supports cross-file-only constraints", async () => {
    await withTempProject(async (projectDir) => {
      await createFixtureProject(projectDir);
      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["src"],
        modes: ["functions"],
        threshold: 0.6,
        crossFileOnly: true,
      });

      expect(report.byMode.functions.length).toBeGreaterThan(0);
      expect(
        report.byMode.functions.every((pair) => pair.left.filePath !== pair.right.filePath),
      ).toBe(true);
    });
  });

  it("rejects non-integer numeric options via resolveAnalyzeOptions", () => {
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], minLines: Number.NaN }),
    ).toThrow("minLines must be a positive integer");
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], minLines: 0 }),
    ).toThrow("minLines must be greater than or equal to 1");
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], overlapMinWindow: 1.5 }),
    ).toThrow("overlapMinWindow must be a positive integer");
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], overlapMaxWindow: 0 }),
    ).toThrow("overlapMaxWindow must be greater than or equal to 1");
  });

  it("rejects invalid path and comparison constraint options", () => {
    expect(() => resolveAnalyzeOptions({ paths: [] })).toThrow(
      "At least one path is required",
    );
    expect(() =>
      resolveAnalyzeOptions({
        paths: ["."],
        sameFileOnly: true,
        crossFileOnly: true,
      }),
    ).toThrow("Cannot use both sameFileOnly and crossFileOnly");
  });

  it("rejects out-of-range ratio options via resolveAnalyzeOptions", () => {
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], threshold: -0.01 }),
    ).toThrow("threshold must be between 0 and 1");
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], threshold: Number.NaN }),
    ).toThrow("threshold must be between 0 and 1");
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], overlapSizeTolerance: 1.01 }),
    ).toThrow("overlapSizeTolerance must be between 0 and 1");
    expect(() =>
      resolveAnalyzeOptions({ paths: ["."], overlapSizeTolerance: Number.NaN }),
    ).toThrow("overlapSizeTolerance must be between 0 and 1");
  });

  it("rejects overlapMinWindow greater than overlapMaxWindow", () => {
    expect(() =>
      resolveAnalyzeOptions({
        paths: ["."],
        overlapMinWindow: 30,
        overlapMaxWindow: 5,
      }),
    ).toThrow("overlapMinWindow must be less than or equal to overlapMaxWindow");
  });

  it("rejects an overlap window larger than the engine takes", () => {
    expect(() => resolveAnalyzeOptions({ paths: ["."], overlapMaxWindow: 1001 })).toThrow(
      "overlapMaxWindow must be less than or equal to 1000",
    );
    expect(resolveAnalyzeOptions({ paths: ["."], overlapMaxWindow: 1000 }).overlapMaxWindow).toBe(
      1000,
    );
  });

  it("rejects modes and type filters it does not know", () => {
    // Values that reach the library untyped — from a configuration file,
    // say — used to travel to the engine and come back as a parse error.
    const untyped = (options: Record<string, unknown>) =>
      resolveAnalyzeOptions({ paths: ["."], ...options } as AnalyzeProjectOptions);
    expect(() => untyped({ modes: ["functions", "everything"] })).toThrow(
      'Unknown mode "everything" (expected one of: functions, types, classes, overlap)',
    );
    expect(() => untyped({ typesOnly: "enum" })).toThrow(
      "typesOnly must be one of: all, interface, type",
    );
  });

  it("fills in every default", () => {
    expect(resolveAnalyzeOptions({ paths: ["src"], cwd: "/project" })).toEqual({
      cwd: "/project",
      paths: ["src"],
      modes: ["functions", "types", "classes", "overlap"],
      threshold: 0.8,
      minLines: 3,
      minTokens: undefined,
      sizePenalty: true,
      sameFileOnly: false,
      crossFileOnly: false,
      extensions: ["ts", "tsx", "mts", "cts"],
      exclude: [],
      typesOnly: "all",
      allowCrossKind: true,
      includeTypeLiterals: false,
      overlapMinWindow: 8,
      overlapMaxWindow: 30,
      overlapSizeTolerance: 0.2,
    });
    // Repeats are dropped, an empty list means every mode, and extensions
    // lose their dot and their case.
    const resolved = resolveAnalyzeOptions({
      paths: ["."],
      modes: ["types", "functions", "types"],
      extensions: [".TS", "tsx"],
    });
    expect(resolved.modes).toEqual(["types", "functions"]);
    expect(resolved.extensions).toEqual(["ts", "tsx"]);
    expect(resolveAnalyzeOptions({ paths: ["."], modes: [], extensions: [] })).toMatchObject({
      modes: ["functions", "types", "classes", "overlap"],
      extensions: ["ts", "tsx", "mts", "cts"],
    });
    // A list that names no extension is no list, and a repeat is one entry.
    expect(resolveAnalyzeOptions({ paths: ["."], extensions: ["", "."] }).extensions).toEqual([
      "ts",
      "tsx",
      "mts",
      "cts",
    ]);
    expect(resolveAnalyzeOptions({ paths: ["."], extensions: ["ts", ".TS"] }).extensions).toEqual([
      "ts",
    ]);
    expect(resolveAnalyzeOptions({ paths: ["."] }).cwd).toBe(process.cwd());
  });

  it("reports the same pairs however the targets are listed", async () => {
    await withTempProject(async (projectDir) => {
      await createFixtureProject(projectDir);
      const files = (await fs.readdir(path.join(projectDir, "src")))
        .filter((file) => file.endsWith(".ts"))
        .map((file) => path.join("src", file));
      const comparable = (report: AnalyzeReport) => ({
        analyzedFiles: report.analyzedFiles,
        results: report.results,
        byMode: report.byMode,
        clusters: report.clusters,
      });

      const options = { cwd: projectDir, threshold: 0.6 };
      const reference = comparable(await analyzeProject({ ...options, paths: ["src"] }));
      expect(reference.results.length).toBeGreaterThan(3);
      for (const paths of [files, [...files].reverse(), ["src", ...files]]) {
        expect(comparable(await analyzeProject({ ...options, paths }))).toEqual(reference);
      }
    });
  });

  it("discovers files with a single extension filter", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "only.ts"),
        "export const only = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "src", "other.tsx"),
        "export const other = 2;\n",
        "utf8",
      );
      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["src"],
        extensions: ["ts"],
      });
      expect(report.stats.fileCount).toBe(1);
      expect(report.analyzedFiles[0]?.endsWith("only.ts")).toBe(true);
    });
  });

  it("honors a .gitignore located in the target directory", async () => {
    await withTempProject(async (projectDir) => {
      const targetDir = path.join(projectDir, "app");
      await fs.mkdir(targetDir, { recursive: true });
      await fs.writeFile(path.join(targetDir, ".gitignore"), "ignored.ts\n", "utf8");
      await fs.writeFile(
        path.join(targetDir, "kept.ts"),
        "export const a = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(targetDir, "ignored.ts"),
        "export const b = 2;\n",
        "utf8",
      );

      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["app"],
      });
      expect(report.stats.fileCount).toBe(1);
      expect(report.analyzedFiles[0]?.endsWith("kept.ts")).toBe(true);
      expect(report.skippedFiles.some((file) => file.endsWith("ignored.ts"))).toBe(true);
    });
  });

  it("honors an ancestor .gitignore above the scan target", async () => {
    await withTempProject(async (projectDir) => {
      // Simulates pointing at `app/src` while the rule lives in
      // `app/.gitignore`. Git would apply the ancestor rule — so should we.
      await fs.mkdir(path.join(projectDir, "app", "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "app", ".gitignore"),
        "src/ignored.ts\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "app", "src", "kept.ts"),
        "export const a = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "app", "src", "ignored.ts"),
        "export const b = 2;\n",
        "utf8",
      );

      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["app/src"],
      });
      expect(report.stats.fileCount).toBe(1);
      expect(report.analyzedFiles[0]?.endsWith("kept.ts")).toBe(true);
      expect(report.skippedFiles.some((file) => file.endsWith("ignored.ts"))).toBe(true);
    });
  });

  it("honors a nested .gitignore discovered under the scan target", async () => {
    await withTempProject(async (projectDir) => {
      // `.gitignore` lives inside a subdirectory of the scan target and must
      // still be applied to files that live beneath it.
      await fs.mkdir(path.join(projectDir, "app", "sub"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "app", "sub", ".gitignore"),
        "nested_ignored.ts\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "app", "top.ts"),
        "export const a = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "app", "sub", "nested_kept.ts"),
        "export const b = 2;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "app", "sub", "nested_ignored.ts"),
        "export const c = 3;\n",
        "utf8",
      );

      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["app"],
      });
      const analyzed = report.analyzedFiles.map((file) => path.basename(file)).sort();
      expect(analyzed).toEqual(["nested_kept.ts", "top.ts"]);
      expect(
        report.skippedFiles.some((file) => file.endsWith("nested_ignored.ts")),
      ).toBe(true);
    });
  });

  it("reports a file that cannot be read as skipped, with a warning", async () => {
    await withTempProject(async (projectDir) => {
      await createFixtureProject(projectDir);
      const unreadablePath = path.join(projectDir, "src", "types.ts");
      // Fails for every user, the superuser included, unlike a file mode.
      const readFile = fs.readFile.bind(fs);
      vi.spyOn(fs, "readFile").mockImplementation(((file: unknown, ...rest: unknown[]) =>
        file === unreadablePath
          ? Promise.reject(new Error("EACCES: permission denied"))
          : (readFile as (...args: unknown[]) => Promise<unknown>)(file, ...rest)) as typeof fs.readFile);

      const report = await analyzeProject({ cwd: projectDir, paths: ["src"], modes: ["types"] });

      expect(report.stats.fileCount).toBe(4);
      expect(report.analyzedFiles).not.toContain(unreadablePath);
      expect(report.skippedFiles).toEqual([unreadablePath]);
      expect(report.warnings).toEqual([
        {
          filePath: unreadablePath,
          message: `Failed to read ${unreadablePath}: EACCES: permission denied`,
        },
      ]);
    });
  });

  it("translates noSizePenalty into sizePenalty=false", () => {
    const withPenalty = resolveAnalyzeOptions({ paths: ["."] });
    expect(withPenalty.sizePenalty).toBe(true);
    const noPenalty = resolveAnalyzeOptions({ paths: ["."], noSizePenalty: true });
    expect(noPenalty.sizePenalty).toBe(false);
  });

  it("supports same-file-only constraints", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "duplicates.ts"),
        `
export function alpha(values: number[]): number {
  let total = 0;
  let count = 0;
  for (const value of values) {
    if (value > 0) {
      total += value;
      count += 1;
    }
  }
  if (count === 0) {
    return 0;
  }
  return total;
}

export function beta(numbers: number[]): number {
  let total = 0;
  let count = 0;
  for (const item of numbers) {
    if (item > 0) {
      total += item;
      count += 1;
    }
  }
  if (count === 0) {
    return 0;
  }
  return total;
}
`,
        "utf8",
      );

      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["src"],
        modes: ["functions"],
        threshold: 0.6,
        sameFileOnly: true,
      });

      expect(report.byMode.functions.length).toBeGreaterThan(0);
      expect(
        report.byMode.functions.every((pair) => pair.left.filePath === pair.right.filePath),
      ).toBe(true);
    });
  });
});
