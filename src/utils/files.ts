import fs from "node:fs/promises";
import path from "node:path";
import fg from "fast-glob";
import ignore from "ignore";
import { DEFAULT_EXCLUDES } from "../defaults.js";
import { toPosixPath } from "./path.js";

export interface CollectFilesOptions {
  cwd: string;
  paths: string[];
  extensions: string[];
  exclude: string[];
}

export interface CollectFilesResult {
  files: string[];
  skipped: string[];
  warnings: string[];
}

async function loadRootGitIgnore(cwd: string): Promise<string> {
  const gitIgnorePath = path.join(cwd, ".gitignore");
  try {
    return await fs.readFile(gitIgnorePath, "utf8");
  } catch {
    return "";
  }
}

function createIgnoreMatcher(cwd: string, gitIgnoreContent: string, extraExclude: string[]) {
  const matcher = ignore();
  if (gitIgnoreContent.trim().length > 0) {
    matcher.add(gitIgnoreContent);
  }
  matcher.add(DEFAULT_EXCLUDES);
  if (extraExclude.length > 0) {
    matcher.add(extraExclude);
  }

  return (absolutePath: string): boolean => {
    const relative = toPosixPath(path.relative(cwd, absolutePath));
    if (!relative || relative.startsWith("..")) {
      return false;
    }
    return matcher.ignores(relative);
  };
}

function toExtensionPattern(extensions: string[]): string {
  return extensions.map((ext) => ext.replace(/^\./, "").toLowerCase()).join(",");
}

export async function collectTypeScriptFiles(options: CollectFilesOptions): Promise<CollectFilesResult> {
  const { cwd, paths, extensions, exclude } = options;
  const extPattern = toExtensionPattern(extensions);
  const gitIgnoreContent = await loadRootGitIgnore(cwd);
  const isIgnored = createIgnoreMatcher(cwd, gitIgnoreContent, exclude);

  const discovered = new Set<string>();
  const skipped: string[] = [];
  const warnings: string[] = [];

  for (const targetPath of paths) {
    const absoluteTarget = path.resolve(cwd, targetPath);
    let stat: Awaited<ReturnType<typeof fs.stat>>;
    try {
      stat = await fs.stat(absoluteTarget);
    } catch {
      warnings.push(`Path not found or not accessible: ${targetPath}`);
      continue;
    }

    if (stat.isFile()) {
      const ext = path.extname(absoluteTarget).toLowerCase().replace(/^\./, "");
      if (!extensions.includes(ext)) {
        skipped.push(absoluteTarget);
        continue;
      }
      if (isIgnored(absoluteTarget)) {
        skipped.push(absoluteTarget);
        continue;
      }
      discovered.add(path.resolve(absoluteTarget));
      continue;
    }

    if (!stat.isDirectory()) {
      warnings.push(`Unsupported path type: ${targetPath}`);
      continue;
    }

    const matches = await fg(`**/*.{${extPattern}}`, {
      cwd: absoluteTarget,
      absolute: true,
      onlyFiles: true,
      followSymbolicLinks: false,
      suppressErrors: true,
      dot: false,
    });

    for (const file of matches) {
      const resolved = path.resolve(file);
      if (isIgnored(resolved)) {
        skipped.push(resolved);
        continue;
      }
      discovered.add(resolved);
    }
  }

  const files = [...discovered].sort((left, right) => left.localeCompare(right));
  return { files, skipped, warnings };
}
