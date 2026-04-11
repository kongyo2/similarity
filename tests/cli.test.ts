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

  it("accepts --min-tokens option", async () => {
    await withTempProject(async (projectDir) => {
      await createCliFixture(projectDir);
      const logs: string[] = [];
      const errors: string[] = [];
      const exitCode = await runCli(
        [path.join(projectDir, "src"), "--modes", "functions", "--min-lines", "1", "--min-tokens", "1"],
        {
          log: (message) => logs.push(message),
          error: (message) => errors.push(message),
        },
      );

      expect(exitCode).toBe(0);
      expect(errors).toHaveLength(0);
      expect(logs).toHaveLength(1);
    });
  });
});
