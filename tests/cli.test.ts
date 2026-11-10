import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import type * as ChalkModule from "chalk";
import { afterEach, describe, expect, it, vi } from "vitest";
import { isCliEntrypoint, runCli } from "../src/cli.js";
import type { AnalyzeReport } from "../src/index.js";

async function withTempProject(run: (projectDir: string) => Promise<void>) {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "similarity-ts-cli-"));
  try {
    await run(tempRoot);
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

/** Create `files` (path → content) under `root`, with their directories. */
async function writeTree(root: string, files: Record<string, string>): Promise<void> {
  for (const [relative, content] of Object.entries(files)) {
    const target = path.join(root, relative);
    await fs.mkdir(path.dirname(target), { recursive: true });
    await fs.writeFile(target, content, "utf8");
  }
}

/** Two files, each with one object type of the same shape. */
async function createCliFixture(projectDir: string) {
  await writeTree(projectDir, {
    "src/a.ts": "\nexport type User = {\n  id: string;\n  name: string;\n};\n",
    "src/b.ts": "\nexport type Person = {\n  id: string;\n  name: string;\n};\n",
  });
}

const SUM_TWINS = `
export function alphaSum(values: number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}

export function betaSum(items: number[]): number {
  let sum = 0;
  for (const item of items) {
    sum += item;
  }
  return sum;
}
`;

const ESCAPE = "\u001b";

interface CliRun {
  exitCode: number;
  /** What went to standard output, one entry per write. */
  logs: string[];
  /** What went to standard error, one entry per write. */
  errors: string[];
}

async function run(args: string[], cli: typeof runCli = runCli): Promise<CliRun> {
  const logs: string[] = [];
  const errors: string[] = [];
  const exitCode = await cli(args, {
    log: (message) => logs.push(message),
    error: (message) => errors.push(message),
  });
  return { exitCode, logs, errors };
}

/** The JSON report a run printed as its only output. */
function jsonReport(result: CliRun): AnalyzeReport {
  expect(result.logs).toHaveLength(1);
  return JSON.parse(result.logs[0] as string) as AnalyzeReport;
}

/** A run that must fail before it analyzes anything, with its message. */
async function rejected(args: string[]): Promise<string> {
  const result = await run(args);
  expect(result.exitCode, args.join(" ")).toBe(1);
  expect(result.logs, args.join(" ")).toHaveLength(0);
  expect(result.errors, args.join(" ")).toHaveLength(1);
  return result.errors[0] as string;
}

function baseNames(files: string[]): string[] {
  return files.map((file) => path.basename(file)).sort();
}

describe("runCli", () => {
  it("renders json output", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const result = await run([
        path.join(projectDir, "src"),
        "--modes",
        "types",
        "--format",
        "json",
        "--threshold",
        "0.5",
      ]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      const report = jsonReport(result);
      expect(report.stats.fileCount).toBe(2);
      expect(report.byMode.types.length).toBeGreaterThan(0);
    });
  });

  it("accepts multiple target paths", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "src_a/a.ts": "export const a = 1;\n",
        "src_b/b.ts": "export const b = 2;\n",
      });
      const result = await run([
        path.join(projectDir, "src_a"),
        path.join(projectDir, "src_b"),
        "--format",
        "json",
      ]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      expect(jsonReport(result).stats.fileCount).toBe(2);
    });
  });

  it("prints the package version for --version without exiting the process", async () => {
    const packageJson = JSON.parse(
      await fs.readFile(new URL("../package.json", import.meta.url), "utf8"),
    ) as { version: string };
    const result = await run(["--version"]);
    expect(result).toEqual({ exitCode: 0, logs: [packageJson.version], errors: [] });
  });

  it("prints usage for --help and reports success", async () => {
    const result = await run(["--help"]);
    expect(result.exitCode).toBe(0);
    expect(result.errors).toHaveLength(0);
    const usage = result.logs.join("\n");
    expect(usage).toContain("Usage: similarity-ts");
    // Every option the README documents is offered.
    for (const flag of [
      "--modes",
      "--threshold",
      "--min-lines",
      "--min-tokens",
      "--no-size-penalty",
      "--same-file-only",
      "--cross-file-only",
      "--extensions",
      "--exclude",
      "--types-only",
      "--no-allow-cross-kind",
      "--type-literals",
      "--overlap-min-window",
      "--overlap-max-window",
      "--overlap-size-tolerance",
      "--format",
      "--output",
      "--fail-on-warnings",
      "--fail-on-duplicates",
    ]) {
      expect(usage, flag).toContain(flag);
    }
  });

  it("reports unknown options on stderr with a non-zero exit code", async () => {
    const result = await run([".", "--no-such-flag"]);
    expect(result.exitCode).toBe(1);
    expect(result.logs).toHaveLength(0);
    expect(result.errors.join("\n")).toContain("unknown option '--no-such-flag'");
  });

  it("returns non-zero on invalid flag combinations", async () => {
    expect(await rejected([".", "--same-file-only", "--cross-file-only"])).toBe(
      "Cannot use both --same-file-only and --cross-file-only",
    );
  });

  it("names an unknown mode instead of dumping a validation error", async () => {
    expect(await rejected([".", "--modes", "functions,bogus"])).toBe(
      'Unknown mode "bogus" (expected one of: functions, types, classes, overlap)',
    );
  });

  it("discovers files when a single --extensions value is provided", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "src/a.ts": "export const a = 1;\n",
        "src/b.tsx": "export const b = 2;\n",
      });
      const result = await run([
        path.join(projectDir, "src"),
        "--extensions",
        "ts",
        "--format",
        "json",
      ]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      const report = jsonReport(result);
      expect(report.stats.fileCount).toBe(1);
      expect(baseNames(report.analyzedFiles)).toEqual(["a.ts"]);
    });
  });

  it("falls back to default modes when --modes is empty", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const result = await run([path.join(projectDir, "src"), "--modes", "", "--format", "json"]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      expect(Object.keys(jsonReport(result).byMode).sort()).toEqual([
        "classes",
        "functions",
        "overlap",
        "types",
      ]);
    });
  });

  it("falls back to default extensions when --extensions is empty", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const result = await run([
        path.join(projectDir, "src"),
        "--extensions",
        "",
        "--format",
        "json",
      ]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      expect(jsonReport(result).stats.fileCount).toBe(2);
    });
  });

  it("applies repeated --exclude patterns", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "src/kept.ts": "export const kept = true;\n",
        "src/first.generated.ts": "export const first = true;\n",
        "src/second.fixture.ts": "export const second = true;\n",
      });
      const result = await run([
        path.join(projectDir, "src"),
        "--exclude",
        "**/*.generated.ts",
        "--exclude",
        "**/*.fixture.ts",
        "--format",
        "json",
      ]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      const report = jsonReport(result);
      expect(baseNames(report.analyzedFiles)).toEqual(["kept.ts"]);
      expect(baseNames(report.skippedFiles)).toEqual(["first.generated.ts", "second.fixture.ts"]);
    });
  });

  it("rejects malformed and out-of-range numeric options", async () => {
    const cases: [string[], string][] = [
      [["--min-lines", "abc"], "min-lines must be an integer"],
      [["--min-lines", ""], "min-lines must be an integer"],
      [["--min-lines", "1e2"], "min-lines must be an integer"],
      [["--min-lines", "0x5"], "min-lines must be an integer"],
      [["--min-lines", "0"], "min-lines must be greater than or equal to 1"],
      [["--min-tokens", "0"], "min-tokens must be greater than or equal to 1"],
      [["--threshold", "abc"], "threshold must be a valid number"],
      [["--threshold", ""], "threshold must be a valid number"],
      [["--threshold", "0x1"], "threshold must be a valid number"],
      [["--threshold", "2"], "threshold must be between 0 and 1"],
      [["--threshold", "-0.5"], "threshold must be between 0 and 1"],
      [["--overlap-min-window", "foo"], "overlap-min-window must be an integer"],
      [
        ["--overlap-min-window", "30", "--overlap-max-window", "5"],
        "overlap-min-window must be less than or equal to overlap-max-window",
      ],
      // A window of a million nodes is a size for every one of which
      // every function is walked.
      [
        ["--overlap-max-window", "1000000"],
        "overlap-max-window must be less than or equal to 1000",
      ],
      [["--overlap-size-tolerance", "1.5"], "overlap-size-tolerance must be between 0 and 1"],
      [["--overlap-size-tolerance", "x"], "overlap-size-tolerance must be a valid number"],
    ];
    for (const [args, message] of cases) {
      expect(await rejected([".", ...args]), args.join(" ")).toBe(message);
    }
  });

  it("renders pretty output by default", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const result = await run([path.join(projectDir, "src"), "--modes", "types", "--threshold", "0.5"]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      expect(result.logs).toHaveLength(1);
      expect(result.logs[0]).toContain("Analyzing code similarity...");
      expect(result.logs[0]).toContain("=== Type Similarity ===");
      expect(result.logs[0]).toContain("Total pairs:");
    });
  });

  it("writes the rendered report to an output file", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      // The directory does not exist yet.
      const outputPath = path.join(projectDir, "reports", "nested", "report.json");
      const result = await run([
        path.join(projectDir, "src"),
        "--modes",
        "types",
        "--format",
        "json",
        "--threshold",
        "0.5",
        "--output",
        outputPath,
      ]);

      expect(result).toEqual({
        exitCode: 0,
        logs: [`Report written: ${outputPath}`],
        errors: [],
      });
      const written = await fs.readFile(outputPath, "utf8");
      expect(written.endsWith("}\n")).toBe(true);
      const report = JSON.parse(written) as AnalyzeReport;
      expect(report.stats.fileCount).toBe(2);
      expect(report.byMode.types.length).toBeGreaterThan(0);
    });
  });

  it("exits with a non-zero code when every target path is missing", async () => {
    await withTempProject(async (projectDir) => {
      const result = await run([path.join(projectDir, "does_not_exist"), "--format", "json"]);

      expect(result.exitCode).toBe(1);
      const report = jsonReport(result);
      expect(report.stats.fileCount).toBe(0);
      expect(report.warnings[0]?.message).toContain("Path not found");
      expect(result.errors.some((message) => message.includes("Path not found"))).toBe(true);
    });
  });

  it("exits with zero when a readable target simply holds nothing to analyze", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "src/readme.md": "# nothing here\n" });
      const result = await run([path.join(projectDir, "src"), "--format", "json"]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      expect(jsonReport(result).stats.fileCount).toBe(0);
    });
  });

  it("fails with --fail-on-warnings when warnings are emitted", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "src/broken.ts": "this is not valid typescript ###\n" });
      const result = await run([
        path.join(projectDir, "src"),
        "--fail-on-warnings",
        "--format",
        "json",
      ]);

      expect(result.exitCode).toBe(1);
      expect(result.logs).toHaveLength(1);
      expect(result.errors.some((message) => message.includes("Parse errors"))).toBe(true);
    });
  });

  it("reports warnings to stderr even when the exit code is zero", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "src/ok.ts": "export const ok = 1;\n",
        "src/broken.ts": "this is not valid typescript ###\n",
      });
      const broken = path.join(projectDir, "src", "broken.ts");
      const result = await run([path.join(projectDir, "src"), "--format", "json"]);

      // Without --fail-on-warnings the run is still successful because at
      // least one file was analyzed, but parse warnings must still surface
      // on stderr for visibility — and say which file they are about.
      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(1);
      expect(result.errors[0]).toMatch(/broken\.ts: skipped: Parse errors: /);
      expect(result.errors[0]?.startsWith(`${broken}: `)).toBe(true);
      const report = jsonReport(result);
      expect(report.skippedFiles).toEqual([broken]);
      expect(baseNames(report.analyzedFiles)).toEqual(["ok.ts"]);
    });
  });

  it("names the file a warning is about in the pretty report", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "src/ok.ts": "export const ok = 1;\n",
        "src/broken.ts": "this is not valid typescript ###\n",
      });
      const result = await run([path.join(projectDir, "src"), "--modes", "types"]);

      expect(result.exitCode).toBe(0);
      const report = result.logs[0] as string;
      expect(report).toContain("Warnings:");
      expect(report).toMatch(/- .*broken\.ts: skipped: Parse errors: /);
    });
  });

  it("applies .gitignore located in the target directory", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "gitignore_proj/.gitignore": "ignored.ts\n",
        "gitignore_proj/a.ts": "export const a = 1;\n",
        "gitignore_proj/ignored.ts": "export const i = 1;\n",
      });
      const result = await run([path.join(projectDir, "gitignore_proj"), "--format", "json"]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      const report = jsonReport(result);
      expect(baseNames(report.analyzedFiles)).toEqual(["a.ts"]);
      expect(baseNames(report.skippedFiles)).toEqual(["ignored.ts"]);
    });
  });

  it("applies target .gitignore to explicitly passed file paths", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "app/.gitignore": "ignored.ts\n",
        "app/ignored.ts": "export const i = 1;\n",
      });
      const result = await run([path.join(projectDir, "app", "ignored.ts"), "--format", "json"]);

      // Explicitly invoked file matches the local .gitignore and must be
      // skipped rather than analyzed; the run is otherwise warning-free so
      // the exit code stays 0 (mirroring "pointed at an empty directory").
      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      const report = jsonReport(result);
      expect(report.stats.fileCount).toBe(0);
      expect(report.analyzedFiles).toHaveLength(0);
      expect(baseNames(report.skippedFiles)).toEqual(["ignored.ts"]);
    });
  });

  it("accepts --no-size-penalty without affecting other flag parsing", async () => {
    await withTempProject(async (projectDir) => {
      // Regression: previously the `--no-size-penalty` option had a
      // `false` default registered on Commander, which suppressed the
      // negation behavior and silently ignored the flag.
      await createCliFixture(projectDir);
      const result = await run([
        path.join(projectDir, "src"),
        "--no-size-penalty",
        "--modes",
        "functions",
        "--format",
        "json",
      ]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      expect(jsonReport(result).stats.fileCount).toBeGreaterThan(0);
    });
  });

  it("walks ancestor .gitignore files above the scan target", async () => {
    await withTempProject(async (projectDir) => {
      // Simulate running `similarity-ts /abs/path/to/repo/app/src` from
      // outside the project. The `.git` marker in `app/` lets the walker
      // identify the enclosing repository root so that `app/.gitignore`
      // applies to files discovered under `app/src`.
      await writeTree(projectDir, {
        "app/.git/HEAD": "ref: refs/heads/main\n",
        "app/.gitignore": "src/ignored.ts\n",
        "app/src/kept.ts": "export const a = 1;\n",
        "app/src/ignored.ts": "export const b = 2;\n",
      });
      const result = await run([path.join(projectDir, "app", "src"), "--format", "json"]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      const report = jsonReport(result);
      expect(baseNames(report.analyzedFiles)).toEqual(["kept.ts"]);
      expect(baseNames(report.skippedFiles)).toEqual(["ignored.ts"]);
    });
  });

  it("walks nested .gitignore files discovered under the scan target", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "app/sub/.gitignore": "nested_ignored.ts\n",
        "app/top.ts": "export const a = 1;\n",
        "app/sub/nested_kept.ts": "export const b = 2;\n",
        "app/sub/nested_ignored.ts": "export const c = 3;\n",
      });
      const result = await run([path.join(projectDir, "app"), "--format", "json"]);

      expect(result.exitCode).toBe(0);
      expect(result.errors).toHaveLength(0);
      const report = jsonReport(result);
      expect(baseNames(report.analyzedFiles)).toEqual(["nested_kept.ts", "top.ts"]);
      expect(baseNames(report.skippedFiles)).toEqual(["nested_ignored.ts"]);
    });
  });

  it("de-duplicates repeated --modes so sections render once", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "a.ts": "export const x = 1;\n" });
      const result = await run([projectDir, "--modes", "functions,functions", "--format", "pretty"]);

      expect(result.exitCode).toBe(0);
      expect(result.logs.join("\n").match(/=== Function Similarity ===/g)).toHaveLength(1);
    });
  });

  it("filters small functions with --min-tokens", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "a.ts": `
export function tinyA(): number {
  const next = 1 + 1;
  return next;
}

export function tinyB(): number {
  const next = 1 + 1;
  return next;
}
`,
      });
      const pairCount = async (extra: string[]) => {
        const result = await run([projectDir, "--modes", "functions", "--format", "json", ...extra]);
        expect(result.exitCode).toBe(0);
        return jsonReport(result).stats.pairCount;
      };

      expect(await pairCount([])).toBeGreaterThan(0);
      expect(await pairCount(["--min-tokens", "500"])).toBe(0);
    });
  });

  it("exits non-zero with --fail-on-duplicates when pairs are reported", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "a.ts": SUM_TWINS });
      const base = [projectDir, "--modes", "functions", "--format", "json"];

      expect((await run([...base, "--fail-on-duplicates"])).exitCode).toBe(1);
      expect((await run(base)).exitCode).toBe(0);
      // Nothing reported: nothing to fail on.
      expect((await run([...base, "--fail-on-duplicates", "--min-tokens", "500"])).exitCode).toBe(0);
    });
  });

  it("restricts the comparison with --types-only and --no-allow-cross-kind", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "shapes.ts": `
export interface Account { id: string; owner: string; balance: number }
export type Wallet = { id: string; owner: string; balance: number };
export type Purse = { id: string; owner: string; balance: number };
`,
      });
      const typePairs = async (extra: string[]) => {
        const result = await run([projectDir, "--modes", "types", "--format", "json", ...extra]);
        expect(result.exitCode).toBe(0);
        return jsonReport(result)
          .byMode.types.map((pair) => [pair.left.symbolName, pair.right.symbolName].sort().join("+"))
          .sort();
      };

      expect(await typePairs([])).toEqual(["Account+Purse", "Account+Wallet", "Purse+Wallet"]);
      expect(await typePairs(["--no-allow-cross-kind"])).toEqual(["Purse+Wallet"]);
      expect(await typePairs(["--types-only", "type"])).toEqual(["Purse+Wallet"]);
      expect(await typePairs(["--types-only", "interface"])).toEqual([]);
      // The deprecated positive flag is still accepted.
      expect(await typePairs(["--allow-cross-kind"])).toHaveLength(3);
    });
  });
});

describe("runCli colors", () => {
  afterEach(() => {
    vi.doUnmock("chalk");
    vi.resetModules();
  });

  /** `runCli` as it behaves when the standard output is a color terminal. */
  async function runCliOnColorTerminal(): Promise<typeof runCli> {
    vi.resetModules();
    vi.doMock("chalk", async () => {
      const actual = await vi.importActual<typeof ChalkModule>("chalk");
      return { ...actual, default: new actual.Chalk({ level: 1 }) };
    });
    return (await import("../src/cli.js")).runCli;
  }

  it("styles the report on a terminal and nowhere else", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "a.ts": SUM_TWINS });
      const cli = await runCliOnColorTerminal();
      const args = [projectDir, "--modes", "functions"];

      const onTerminal = await run(args, cli);
      expect(onTerminal.exitCode).toBe(0);
      expect(onTerminal.logs[0]).toContain(`${ESCAPE}[1mAnalyzing code similarity...`);

      // A report written to a file used to carry the escape codes of the
      // terminal the command was typed in.
      const outputPath = path.join(projectDir, "report.txt");
      const toFile = await run([...args, "--output", outputPath], cli);
      expect(toFile.exitCode).toBe(0);
      const written = await fs.readFile(outputPath, "utf8");
      expect(written).toContain("=== Function Similarity ===");
      expect(written).toContain("alphaSum");
      expect(written).not.toContain(ESCAPE);

      // JSON is data whatever it is printed on.
      const json = await run([...args, "--format", "json"], cli);
      expect(json.logs[0]).not.toContain(ESCAPE);
    });
  });

  it("prints plain text when the output is not a terminal", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "a.ts": SUM_TWINS });
      // Under the test runner the standard output is a pipe.
      const result = await run([projectDir, "--modes", "functions"]);
      expect(result.exitCode).toBe(0);
      expect(result.logs[0]).toContain("alphaSum");
      expect(result.logs[0]).not.toContain(ESCAPE);
    });
  });
});

describe("isCliEntrypoint", () => {
  it("returns false when Node did not provide an argv script path", () => {
    expect(isCliEntrypoint(undefined, import.meta.url)).toBe(false);
  });

  it("resolves symlinked argv paths before comparing the current module", async () => {
    await withTempProject(async (projectDir) => {
      const realScript = path.join(projectDir, "dist", "cli.js");
      const linkedScript = path.join(projectDir, "node_modules", ".bin", "similarity-ts");
      await fs.mkdir(path.dirname(realScript), { recursive: true });
      await fs.mkdir(path.dirname(linkedScript), { recursive: true });
      await fs.writeFile(realScript, "#!/usr/bin/env node\n", "utf8");
      await fs.symlink(realScript, linkedScript);

      expect(isCliEntrypoint(linkedScript, pathToFileURL(realScript).href)).toBe(true);
    });
  });

  it("falls back to the raw argv path when realpath resolution fails", async () => {
    await withTempProject(async (projectDir) => {
      const missingScript = path.join(projectDir, "missing.js");

      expect(isCliEntrypoint(missingScript, pathToFileURL(missingScript).href)).toBe(true);
    });
  });
});
