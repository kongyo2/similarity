import fs from "node:fs/promises";
import path from "node:path";
import fg from "fast-glob";
import ignore, { type Ignore } from "ignore";
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

type IgnoreFn = (absolutePath: string) => boolean;

async function loadGitIgnore(dir: string): Promise<string> {
  const gitIgnorePath = path.join(dir, ".gitignore");
  try {
    return await fs.readFile(gitIgnorePath, "utf8");
  } catch {
    return "";
  }
}

function createRootIgnoreMatcher(
  cwd: string,
  gitIgnoreContent: string,
  extraExclude: string[],
): IgnoreFn {
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

function createTargetIgnoreMatcher(baseDir: string, gitIgnoreContent: string): IgnoreFn {
  if (gitIgnoreContent.trim().length === 0) {
    return () => false;
  }
  const matcher: Ignore = ignore().add(gitIgnoreContent);
  return (absolutePath: string): boolean => {
    const relative = toPosixPath(path.relative(baseDir, absolutePath));
    if (!relative || relative.startsWith("..")) {
      return false;
    }
    return matcher.ignores(relative);
  };
}

function buildDirectoryGlobs(extensions: string[]): string[] {
  const unique = [
    ...new Set(
      extensions
        .map((ext) => ext.replace(/^\./, "").toLowerCase())
        .filter((ext) => ext.length > 0),
    ),
  ];
  // Emit one pattern per extension instead of a brace expression, because
  // `**/*.{ts}` (single-item brace) is treated literally by fast-glob/picomatch
  // and silently matches nothing.
  return unique.map((ext) => `**/*.${ext}`);
}

export async function collectTypeScriptFiles(options: CollectFilesOptions): Promise<CollectFilesResult> {
  const { cwd, paths, extensions, exclude } = options;
  const globPatterns = buildDirectoryGlobs(extensions);
  const rootGitIgnoreContent = await loadGitIgnore(cwd);
  const isIgnoredByRoot = createRootIgnoreMatcher(cwd, rootGitIgnoreContent, exclude);

  const targetMatcherCache = new Map<string, IgnoreFn>();
  async function getTargetMatcher(targetDir: string): Promise<IgnoreFn> {
    const cached = targetMatcherCache.get(targetDir);
    if (cached) {
      return cached;
    }
    const content = targetDir === cwd ? "" : await loadGitIgnore(targetDir);
    const matcher = createTargetIgnoreMatcher(targetDir, content);
    targetMatcherCache.set(targetDir, matcher);
    return matcher;
  }

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
      // Apply the parent directory's .gitignore so explicit file targets are
      // filtered the same way as files discovered by directory scans.
      const isIgnoredByTarget = await getTargetMatcher(path.dirname(absoluteTarget));
      if (isIgnoredByRoot(absoluteTarget) || isIgnoredByTarget(absoluteTarget)) {
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

    const matches = globPatterns.length === 0
      ? []
      : await fg(globPatterns, {
          cwd: absoluteTarget,
          absolute: true,
          onlyFiles: true,
          followSymbolicLinks: false,
          suppressErrors: true,
          dot: false,
        });

    const isIgnoredByTarget = await getTargetMatcher(absoluteTarget);

    for (const file of matches) {
      const resolved = path.resolve(file);
      if (isIgnoredByRoot(resolved) || isIgnoredByTarget(resolved)) {
        skipped.push(resolved);
        continue;
      }
      discovered.add(resolved);
    }
  }

  const files = [...discovered].sort((left, right) => left.localeCompare(right));
  return { files, skipped, warnings };
}
