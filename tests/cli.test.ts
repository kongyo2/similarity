import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { isCliEntrypoint, runCli } from "../src/cli.js";

async function withTempProject(run: (projectDir: string) => Promise<void>) {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "similarity-ts-cli-"));
  try {
    await run(tempRoot);
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

async function createCliFixture(projectDir: string) {
  await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
  await fs.writeFile(
    path.join(projectDir, "src", "a.ts"),
    `
export type User = {
  id: string;
  name: string;
};
`,
    "utf8",
  );
  await fs.writeFile(
    path.join(projectDir, "src", "b.ts"),
    `
export type Person = {
  id: string;
  name: string;
};
`,
    "utf8",
  );
}

describe("runCli", () => {
  it("renders json output", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "src"), "--modes", "types", "--format", "json", "--threshold", "0.5"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      expect(logs).toHaveLength(1);
      const parsed = JSON.parse(logs[0]) as { stats: { fileCount: number }; byMode: { types: unknown[] } };
      expect(parsed.stats.fileCount).toBe(2);
      expect(parsed.byMode.types.length).toBeGreaterThan(0);
    });
  });

  it("accepts multiple target paths", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src_a"), { recursive: true });
      await fs.mkdir(path.join(projectDir, "src_b"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src_a", "a.ts"),
        "export const a = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "src_b", "b.ts"),
        "export const b = 2;\n",
        "utf8",
      );
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [
          path.join(projectDir, "src_a"),
          path.join(projectDir, "src_b"),
          "--format",
          "json",
        ],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      const parsed = JSON.parse(logs[0]) as { stats: { fileCount: number } };
      expect(parsed.stats.fileCount).toBe(2);
    });
  });

  it("returns non-zero on invalid flag combinations", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli(
      [".", "--same-file-only", "--cross-file-only"],
      {
        log: (message) => logs.push(message),
        error: (message) => errors.push(message),
      },
    );
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain("Cannot use both --same-file-only and --cross-file-only");
  });

  it("discovers files when a single --extensions value is provided", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "a.ts"),
        "export const a = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "src", "b.tsx"),
        "export const b = 2;\n",
        "utf8",
      );

      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "src"), "--extensions", "ts", "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      const parsed = JSON.parse(logs[0]) as {
        analyzedFiles: string[];
        stats: { fileCount: number };
      };
      expect(parsed.stats.fileCount).toBe(1);
      expect(parsed.analyzedFiles).toHaveLength(1);
      expect(parsed.analyzedFiles[0].endsWith("a.ts")).toBe(true);
    });
  });

  it("falls back to default modes when --modes is empty", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "src"), "--modes", "", "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      const parsed = JSON.parse(logs[0]) as {
        byMode: Record<string, unknown[]>;
      };
      expect(Object.keys(parsed.byMode).sort()).toEqual([
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
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "src"), "--extensions", "", "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      const parsed = JSON.parse(logs[0]) as { stats: { fileCount: number } };
      expect(parsed.stats.fileCount).toBe(2);
    });
  });

  it("applies repeated --exclude patterns", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "kept.ts"),
        "export const kept = true;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "src", "first.generated.ts"),
        "export const first = true;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "src", "second.fixture.ts"),
        "export const second = true;\n",
        "utf8",
      );
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [
          path.join(projectDir, "src"),
          "--exclude",
          "**/*.generated.ts",
          "--exclude",
          "**/*.fixture.ts",
          "--format",
          "json",
        ],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      const parsed = JSON.parse(logs[0]) as {
        analyzedFiles: string[];
        skippedFiles: string[];
      };
      expect(parsed.analyzedFiles).toHaveLength(1);
      expect(parsed.analyzedFiles[0].endsWith("kept.ts")).toBe(true);
      expect(parsed.skippedFiles).toHaveLength(2);
    });
  });

  it("rejects non-integer values for --min-lines", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli([".", "--min-lines", "abc"], {
      log: (message) => logs.push(message),
      error: (message) => errors.push(message),
    });
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain("min-lines must be an integer");
  });

  it("rejects values below the minimum for --min-lines", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli([".", "--min-lines", "0"], {
      log: (message) => logs.push(message),
      error: (message) => errors.push(message),
    });
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain("min-lines must be greater than or equal to 1");
  });

  it("rejects empty values for --min-lines", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli([".", "--min-lines", ""], {
      log: (message) => logs.push(message),
      error: (message) => errors.push(message),
    });
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain("min-lines must be an integer");
  });

  it("rejects non-numeric threshold values", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli([".", "--threshold", "abc"], {
      log: (message) => logs.push(message),
      error: (message) => errors.push(message),
    });
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain("threshold must be a valid number");
  });

  it("rejects threshold values outside the accepted ratio range", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli([".", "--threshold", "2"], {
      log: (message) => logs.push(message),
      error: (message) => errors.push(message),
    });
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain("threshold must be between 0 and 1");
  });

  it("rejects non-integer values for --overlap-min-window", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli(
      [".", "--modes", "overlap", "--overlap-min-window", "foo"],
      {
        log: (message) => logs.push(message),
        error: (message) => errors.push(message),
      },
    );
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain("overlap-min-window must be an integer");
  });

  it("rejects overlap-min-window greater than overlap-max-window", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli(
      [
        ".",
        "--modes",
        "overlap",
        "--overlap-min-window",
        "30",
        "--overlap-max-window",
        "5",
      ],
      {
        log: (message) => logs.push(message),
        error: (message) => errors.push(message),
      },
    );
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain(
      "overlap-min-window must be less than or equal to overlap-max-window",
    );
  });

  it("rejects overlap-size-tolerance outside the accepted ratio range", async () => {
    const logs: string[] = [];
    const errors: string[] = [];
    const exitCode = await runCli(
      [".", "--modes", "overlap", "--overlap-size-tolerance", "1.5"],
      {
        log: (message) => logs.push(message),
        error: (message) => errors.push(message),
      },
    );
    expect(exitCode).toBe(1);
    expect(logs).toHaveLength(0);
    expect(errors[0]).toContain("overlap-size-tolerance must be between 0 and 1");
  });

  it("renders pretty output by default", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "src"), "--modes", "types", "--threshold", "0.5"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      expect(logs).toHaveLength(1);
      expect(logs[0]).toContain("Analyzing code similarity...");
      expect(logs[0]).toContain("=== Type Similarity ===");
      expect(logs[0]).toContain("Total pairs:");
    });
  });

  it("writes the rendered report to an output file", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const outputPath = path.join(projectDir, "report.json");
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [
          path.join(projectDir, "src"),
          "--modes",
          "types",
          "--format",
          "json",
          "--threshold",
          "0.5",
          "--output",
          outputPath,
        ],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      expect(logs).toEqual([`Report written: ${outputPath}`]);
      const parsed = JSON.parse(await fs.readFile(outputPath, "utf8")) as {
        stats: { fileCount: number };
        byMode: { types: unknown[] };
      };
      expect(parsed.stats.fileCount).toBe(2);
      expect(parsed.byMode.types.length).toBeGreaterThan(0);
    });
  });

  it("exits with a non-zero code when every target path is missing", async () => {
    await withTempProject(async (projectDir) => {
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "does_not_exist"), "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );
      expect(exitCode).toBe(1);
      expect(logs).toHaveLength(1);
      const parsed = JSON.parse(logs[0]) as {
        stats: { fileCount: number };
        warnings: { message: string }[];
      };
      expect(parsed.stats.fileCount).toBe(0);
      expect(parsed.warnings[0]?.message).toContain("Path not found");
      expect(errors.some((message) => message.includes("Path not found"))).toBe(true);
    });
  });

  it("fails with --fail-on-warnings when warnings are emitted", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "broken.ts"),
        "this is not valid typescript ###\n",
        "utf8",
      );
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "src"), "--fail-on-warnings", "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );
      expect(exitCode).toBe(1);
      expect(logs).toHaveLength(1);
      expect(errors.some((message) => message.includes("Parse errors"))).toBe(true);
    });
  });

  it("applies .gitignore located in the target directory", async () => {
    await withTempProject(async (projectDir) => {
      const projectSubdir = path.join(projectDir, "gitignore_proj");
      await fs.mkdir(projectSubdir, { recursive: true });
      await fs.writeFile(
        path.join(projectSubdir, ".gitignore"),
        "ignored.ts\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectSubdir, "a.ts"),
        "export const a = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectSubdir, "ignored.ts"),
        "export const i = 1;\n",
        "utf8",
      );

      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [projectSubdir, "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );
      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      const parsed = JSON.parse(logs[0]) as {
        analyzedFiles: string[];
        skippedFiles: string[];
      };
      expect(parsed.analyzedFiles).toHaveLength(1);
      expect(parsed.analyzedFiles[0].endsWith("a.ts")).toBe(true);
      expect(parsed.skippedFiles.some((file) => file.endsWith("ignored.ts"))).toBe(true);
    });
  });

  it("applies target .gitignore to explicitly passed file paths", async () => {
    await withTempProject(async (projectDir) => {
      const projectSubdir = path.join(projectDir, "app");
      await fs.mkdir(projectSubdir, { recursive: true });
      await fs.writeFile(
        path.join(projectSubdir, ".gitignore"),
        "ignored.ts\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectSubdir, "ignored.ts"),
        "export const i = 1;\n",
        "utf8",
      );

      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectSubdir, "ignored.ts"), "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );
      // Explicitly invoked file matches the local .gitignore and must be
      // skipped rather than analyzed; the run is otherwise warning-free so
      // the exit code stays 0 (mirroring "pointed at an empty directory").
      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      expect(logs).toHaveLength(1);
      const parsed = JSON.parse(logs[0]) as {
        analyzedFiles: string[];
        skippedFiles: string[];
        stats: { fileCount: number };
      };
      expect(parsed.stats.fileCount).toBe(0);
      expect(parsed.analyzedFiles).toHaveLength(0);
      expect(parsed.skippedFiles.some((file) => file.endsWith("ignored.ts"))).toBe(true);
    });
  });

  it("accepts --no-size-penalty without affecting other flag parsing", async () => {
    await withTempProject(async (projectDir) => {
      // Regression: previously the `--no-size-penalty` option had a
      // `false` default registered on Commander, which suppressed the
      // negation behavior and silently ignored the flag.
      await createCliFixture(projectDir);
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [
          path.join(projectDir, "src"),
          "--no-size-penalty",
          "--modes",
          "functions",
          "--format",
          "json",
        ],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );
      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      expect(logs).toHaveLength(1);
      const parsed = JSON.parse(logs[0]) as {
        stats: { fileCount: number };
      };
      expect(parsed.stats.fileCount).toBeGreaterThan(0);
    });
  });

  it("walks ancestor .gitignore files above the scan target", async () => {
    await withTempProject(async (projectDir) => {
      // Simulate running `similarity-ts /abs/path/to/repo/app/src` from
      // outside the project. The `.git` marker in `app/` lets our walker
      // identify the enclosing repo root so that `app/.gitignore` applies to
      // files discovered under `app/src`.
      await fs.mkdir(path.join(projectDir, "app", "src"), { recursive: true });
      await fs.mkdir(path.join(projectDir, "app", ".git"), { recursive: true });
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

      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "app", "src"), "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );
      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      const parsed = JSON.parse(logs[0]) as {
        analyzedFiles: string[];
        skippedFiles: string[];
      };
      expect(parsed.analyzedFiles).toHaveLength(1);
      expect(parsed.analyzedFiles[0].endsWith("kept.ts")).toBe(true);
      expect(parsed.skippedFiles.some((file) => file.endsWith("ignored.ts"))).toBe(true);
    });
  });

  it("walks nested .gitignore files discovered under the scan target", async () => {
    await withTempProject(async (projectDir) => {
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

      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "app"), "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );
      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      const parsed = JSON.parse(logs[0]) as {
        analyzedFiles: string[];
        skippedFiles: string[];
      };
      const analyzedNames = parsed.analyzedFiles
        .map((file) => path.basename(file))
        .sort();
      expect(analyzedNames).toEqual(["nested_kept.ts", "top.ts"]);
      expect(
        parsed.skippedFiles.some((file) => file.endsWith("nested_ignored.ts")),
      ).toBe(true);
    });
  });

  it("reports warnings to stderr even when the exit code is zero", async () => {
    await withTempProject(async (projectDir) => {
      await fs.mkdir(path.join(projectDir, "src"), { recursive: true });
      await fs.writeFile(
        path.join(projectDir, "src", "ok.ts"),
        "export const ok = 1;\n",
        "utf8",
      );
      await fs.writeFile(
        path.join(projectDir, "src", "broken.ts"),
        "this is not valid typescript ###\n",
        "utf8",
      );
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "src"), "--format", "json"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );
      // Without --fail-on-warnings the run is still successful because at
      // least one file was analyzed, but parse warnings must still surface
      // on stderr for visibility.
      expect(exitCode).toBe(0);
      expect(logs).toHaveLength(1);
      expect(errors.some((message) => message.includes("Parse errors"))).toBe(true);
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

      expect(
        isCliEntrypoint(linkedScript, pathToFileURL(realScript).href),
      ).toBe(true);
    });
  });

  it("falls back to the raw argv path when realpath resolution fails", async () => {
    await withTempProject(async (projectDir) => {
      const missingScript = path.join(projectDir, "missing.js");

      expect(
        isCliEntrypoint(missingScript, pathToFileURL(missingScript).href),
      ).toBe(true);
    });
  });
});
