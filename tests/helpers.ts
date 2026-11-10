import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

/**
 * Run `run` in a temporary directory of its own, removed afterwards. The
 * path handed over is resolved: on macOS the temporary directory is itself
 * behind a symbolic link.
 */
export async function withTempProject(run: (projectDir: string) => Promise<void>): Promise<void> {
  const projectDir = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), "similarity-ts-")));
  try {
    await run(projectDir);
  } finally {
    await fs.rm(projectDir, { recursive: true, force: true });
  }
}

/** Create one file under `root`, with its directories, and return its path. */
export async function writeSource(
  root: string,
  relativePath: string,
  content: string,
): Promise<string> {
  const filePath = path.join(root, relativePath);
  await fs.mkdir(path.dirname(filePath), { recursive: true });
  await fs.writeFile(filePath, content, "utf8");
  return filePath;
}

/** Create `files` (path → content) under `root`, with their directories. */
export async function writeTree(root: string, files: Record<string, string>): Promise<void> {
  for (const [relativePath, content] of Object.entries(files)) {
    await writeSource(root, relativePath, content);
  }
}
