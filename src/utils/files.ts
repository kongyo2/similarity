import type { Dirent } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import ignore, { type Ignore } from "ignore";
import { DEFAULT_EXCLUDES } from "../defaults.js";
import { errorMessage } from "./errors.js";
import { compareStrings, isInsideDirectory, normalizeExtensions, toPosixPath } from "./path.js";

export interface CollectFilesOptions {
  cwd: string;
  paths: string[];
  extensions: string[];
  exclude: string[];
}

export interface CollectFilesResult {
  /** Files to analyze: absolute paths, sorted. */
  files: string[];
  /**
   * What was asked for or seen but left out: a target that is not a
   * source file or is ignored, and source files an ignore rule names. The
   * contents of ignored directories are not listed — nothing below them is
   * ever read.
   */
  skipped: string[];
  warnings: string[];
}

/** Ignore patterns and the directory they are relative to. */
interface IgnoreLayer {
  base: string;
  matcher: Ignore;
}

/** The ignore rules in force for one scan target. */
interface IgnoreRules {
  /** `--exclude` patterns: they always win. */
  exclude: IgnoreLayer | undefined;
  /**
   * `.gitignore` files, the repository root's first. A deeper file
   * overrides a shallower one, as in git.
   */
  gitignores: IgnoreLayer[];
  /** {@link DEFAULT_EXCLUDES}: what applies when nothing else decided. */
  defaults: IgnoreLayer | undefined;
}

/** How far up `findRepositoryRoot` looks for a `.git` entry. */
const MAX_REPOSITORY_DEPTH = 128;

function ruledOut(layer: IgnoreLayer, absolutePath: string, isDirectory: boolean) {
  const relative = toPosixPath(path.relative(layer.base, absolutePath));
  // Patterns say nothing about their own directory or anything outside it.
  if (relative === "" || relative === ".." || relative.startsWith("../")) {
    return { ignored: false, unignored: false };
  }
  // A trailing slash is how the matcher is told the path is a directory;
  // `build/` only matches one.
  return layer.matcher.test(isDirectory ? `${relative}/` : relative);
}

function isIgnored(rules: IgnoreRules, absolutePath: string, isDirectory: boolean): boolean {
  if (rules.exclude && ruledOut(rules.exclude, absolutePath, isDirectory).ignored) {
    return true;
  }
  for (let index = rules.gitignores.length - 1; index >= 0; index -= 1) {
    const verdict = ruledOut(rules.gitignores[index] as IgnoreLayer, absolutePath, isDirectory);
    if (verdict.ignored) {
      return true;
    }
    if (verdict.unignored) {
      // `!keep.ts` in a deeper `.gitignore` re-includes what a shallower
      // one ignores.
      return false;
    }
  }
  return rules.defaults !== undefined && ruledOut(rules.defaults, absolutePath, isDirectory).ignored;
}

function extensionOf(fileName: string): string {
  return path.extname(fileName).slice(1).toLowerCase();
}

/**
 * Find the source files under `paths`.
 *
 * Directories are walked top-down and an ignored directory is never
 * entered, so the cost of a scan is that of the files it keeps —
 * `node_modules` and build output are not read just to be thrown away.
 * What is ignored: `--exclude` patterns, every `.gitignore` from the
 * repository root down to the file (with git's precedence), the default
 * excludes at any depth below a scanned directory, and entries whose name
 * starts with a dot. Symbolic links are not followed.
 *
 * A target named explicitly is exempt from the default excludes —
 * `similarity-ts dist` scans `dist` — but not from `.gitignore` or
 * `--exclude`.
 */
export async function collectTypeScriptFiles(
  options: CollectFilesOptions,
): Promise<CollectFilesResult> {
  // Resolved so comparisons with paths produced by `path.resolve(cwd, …)`
  // hold whatever form the caller passed.
  const cwd = path.resolve(options.cwd);
  const extensions = new Set(normalizeExtensions(options.extensions));

  const discovered = new Set<string>();
  const skipped = new Set<string>();
  const warnings: string[] = [];

  const gitignoreCache = new Map<string, Promise<IgnoreLayer | undefined>>();
  function gitignoreOf(directory: string): Promise<IgnoreLayer | undefined> {
    let cached = gitignoreCache.get(directory);
    if (!cached) {
      cached = fs.readFile(path.join(directory, ".gitignore"), "utf8").then(
        (content) =>
          content.trim().length > 0 ? { base: directory, matcher: ignore().add(content) } : undefined,
        () => undefined,
      );
      gitignoreCache.set(directory, cached);
    }
    return cached;
  }

  const repositoryRootCache = new Map<string, Promise<string | undefined>>();
  /** The closest directory at or above `directory` that holds a `.git` entry. */
  function findRepositoryRoot(directory: string): Promise<string | undefined> {
    let cached = repositoryRootCache.get(directory);
    if (!cached) {
      cached = (async () => {
        let current = directory;
        for (let depth = 0; depth < MAX_REPOSITORY_DEPTH; depth += 1) {
          try {
            // A directory in a clone, a file in a worktree or submodule.
            await fs.stat(path.join(current, ".git"));
            return current;
          } catch {
            // Not a repository root; keep walking up.
          }
          const parent = path.dirname(current);
          if (parent === current) {
            return undefined;
          }
          current = parent;
        }
        return undefined;
      })();
      repositoryRootCache.set(directory, cached);
    }
    return cached;
  }

  function excludeLayer(base: string): IgnoreLayer | undefined {
    return options.exclude.length > 0
      ? { base, matcher: ignore().add(options.exclude) }
      : undefined;
  }

  /**
   * The rules for a target in `directory`: `--exclude` patterns relative
   * to `cwd` (to the repository or the directory itself for a target
   * outside `cwd`), and the `.gitignore` files from the repository root
   * down to `directory`.
   */
  async function rulesFor(directory: string): Promise<IgnoreRules> {
    const insideCwd = isInsideDirectory(cwd, directory);
    const repositoryRoot = await findRepositoryRoot(directory);
    const top = repositoryRoot ?? (insideCwd ? cwd : directory);

    const gitignores: IgnoreLayer[] = [];
    const below = path.relative(top, directory);
    let current = top;
    for (const segment of ["", ...below.split(path.sep).filter((part) => part.length > 0)]) {
      current = path.join(current, segment);
      const layer = await gitignoreOf(current);
      if (layer) {
        gitignores.push(layer);
      }
    }
    return { exclude: excludeLayer(insideCwd ? cwd : top), gitignores, defaults: undefined };
  }

  async function walk(directory: string, rules: IgnoreRules): Promise<void> {
    let entries: Dirent[];
    try {
      entries = await fs.readdir(directory, { withFileTypes: true });
    } catch (error) {
      warnings.push(`Failed to read directory ${directory}: ${errorMessage(error)}`);
      return;
    }
    entries.sort((left, right) => compareStrings(left.name, right.name));

    for (const entry of entries) {
      if (entry.name.startsWith(".")) {
        continue;
      }
      const entryPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        if (isIgnored(rules, entryPath, true)) {
          continue;
        }
        const own = await gitignoreOf(entryPath);
        await walk(entryPath, own ? { ...rules, gitignores: [...rules.gitignores, own] } : rules);
      } else if (entry.isFile() && extensions.has(extensionOf(entry.name))) {
        if (isIgnored(rules, entryPath, false)) {
          skipped.add(entryPath);
        } else {
          discovered.add(entryPath);
        }
      }
    }
  }

  for (const targetPath of options.paths) {
    const target = path.resolve(cwd, targetPath);
    let stat: Awaited<ReturnType<typeof fs.stat>>;
    try {
      stat = await fs.stat(target);
    } catch {
      warnings.push(`Path not found or not accessible: ${targetPath}`);
      continue;
    }

    if (stat.isFile()) {
      const rules = await rulesFor(path.dirname(target));
      if (!extensions.has(extensionOf(target)) || isIgnored(rules, target, false)) {
        skipped.add(target);
      } else {
        discovered.add(target);
      }
    } else if (stat.isDirectory()) {
      const rules = await rulesFor(target);
      if (isIgnored(rules, target, true)) {
        skipped.add(target);
        continue;
      }
      await walk(target, {
        ...rules,
        defaults: { base: target, matcher: ignore().add(DEFAULT_EXCLUDES) },
      });
    } else {
      warnings.push(`Unsupported path type: ${targetPath}`);
    }
  }

  return {
    files: [...discovered].sort(compareStrings),
    // A file named twice — as a target and under one — is skipped once.
    skipped: [...skipped].filter((file) => !discovered.has(file)).sort(compareStrings),
    warnings,
  };
}
