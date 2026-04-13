import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

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
