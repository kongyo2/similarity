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
  // Normalize cwd so cache keys and equality checks against directories produced
  // by `path.resolve(cwd, ...)` are robust against trailing slashes or relative
  // paths passed in by library callers.
  const cwd = path.resolve(options.cwd);
  const { paths, extensions, exclude } = options;
  const globPatterns = buildDirectoryGlobs(extensions);
  const rootGitIgnoreContent = await loadGitIgnore(cwd);
  const isIgnoredByRoot = createRootIgnoreMatcher(cwd, rootGitIgnoreContent, exclude);

  // Per-directory matchers are cached so we only load each `.gitignore` once
  // even when many files share ancestors.
  const directoryMatcherCache = new Map<string, IgnoreFn>();
  async function getDirectoryMatcher(dir: string): Promise<IgnoreFn> {
    const cached = directoryMatcherCache.get(dir);
    if (cached) {
      return cached;
    }
    // cwd's `.gitignore` is already covered by the root matcher (which also
    // handles DEFAULT_EXCLUDES and --exclude). Skip it here to avoid double
    // evaluation.
    const content = dir === cwd ? "" : await loadGitIgnore(dir);
    const matcher = createTargetIgnoreMatcher(dir, content);
    directoryMatcherCache.set(dir, matcher);
    return matcher;
  }

  function isInsideCwd(absolute: string): boolean {
    const relative = path.relative(cwd, absolute);
    return !relative.startsWith("..") && !path.isAbsolute(relative);
  }

  // Walk every `.gitignore` from `base` down to `leafDir` (inclusive) and
  // return the corresponding matchers. This mirrors git's behavior of
  // applying every ancestor `.gitignore` between the walk root and the file
  // being considered — and, by extension, any `.gitignore` nested inside a
  // scan target is honored as long as a discovered file lives under it.
  //
  // `base` is either the project `cwd` (when files live under it) or a scan
  // target that lives outside `cwd`. When `base === cwd`, the first matcher
  // is omitted because `isIgnoredByRoot` already covers `cwd/.gitignore`
  // together with `DEFAULT_EXCLUDES` and `--exclude` patterns.
  async function getAncestorMatchers(base: string, leafDir: string): Promise<IgnoreFn[]> {
    const relative = path.relative(base, leafDir);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      return [];
    }
    const matchers: IgnoreFn[] = [];
    if (base !== cwd) {
      matchers.push(await getDirectoryMatcher(base));
    }
    const parts = relative.length === 0 ? [] : relative.split(path.sep).filter((p) => p.length > 0);
    let current = base;
    for (const part of parts) {
      current = path.join(current, part);
      matchers.push(await getDirectoryMatcher(current));
    }
    return matchers;
  }

  // Walk upward from `startDir` looking for an enclosing git worktree root.
  // We stop at either the first directory containing a `.git` entry (directory
  // or file, since git worktrees use files) or the filesystem root. A small
  // depth cap prevents pathological walks on deeply nested paths.
  async function findEnclosingGitRoot(startDir: string): Promise<string | null> {
    let current = path.resolve(startDir);
    for (let depth = 0; depth < 128; depth += 1) {
      try {
        await fs.stat(path.join(current, ".git"));
        return current;
      } catch {
        // not a git root — keep walking
      }
      const parent = path.dirname(current);
      if (parent === current) {
        return null;
      }
      current = parent;
    }
    return null;
  }

  async function resolveIgnoreBase(
    absoluteTarget: string,
    isFileTarget: boolean,
  ): Promise<string> {
    if (isInsideCwd(absoluteTarget)) {
      return cwd;
    }
    // Scan target lives outside `cwd` (e.g. an absolute path pointing into
    // another tree). Anchor the ancestor walk at the enclosing git root so
    // ancestor `.gitignore` files between the target and the repo root are
    // honoured, matching git's own discovery semantics. When no git root is
    // present we fall back to the target's own directory.
    const startDir = isFileTarget ? path.dirname(absoluteTarget) : absoluteTarget;
    const repoRoot = await findEnclosingGitRoot(startDir);
    return repoRoot ?? startDir;
  }

  function isIgnoredByAnyAncestor(ancestors: IgnoreFn[], absolutePath: string): boolean {
    for (const matcher of ancestors) {
      if (matcher(absolutePath)) {
        return true;
      }
    }
    return false;
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
      // Walk every `.gitignore` from the effective base down to the file's
      // parent directory so explicit file targets honour the same rules as
      // directory scans.
      const ignoreBase = await resolveIgnoreBase(absoluteTarget, true);
      const ancestors = await getAncestorMatchers(ignoreBase, path.dirname(absoluteTarget));
      if (
        isIgnoredByRoot(absoluteTarget) ||
        isIgnoredByAnyAncestor(ancestors, absoluteTarget)
      ) {
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

    const ignoreBase = await resolveIgnoreBase(absoluteTarget, false);
    for (const file of matches) {
      const resolved = path.resolve(file);
      const ancestors = await getAncestorMatchers(ignoreBase, path.dirname(resolved));
      if (
        isIgnoredByRoot(resolved) ||
        isIgnoredByAnyAncestor(ancestors, resolved)
      ) {
        skipped.push(resolved);
        continue;
      }
      discovered.add(resolved);
    }
  }

  const files = [...discovered].sort((left, right) => left.localeCompare(right));
  return { files, skipped, warnings };
}
