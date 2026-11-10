import path from "node:path";

export function toPosixPath(inputPath: string): string {
  return inputPath.split(path.sep).join("/");
}

/** Whether `candidate` is `directory` itself or lies below it. */
export function isInsideDirectory(directory: string, candidate: string): boolean {
  const relative = path.relative(directory, candidate);
  return relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

/**
 * `filePath` relative to `cwd`, with forward slashes; a path outside `cwd`
 * stays as it is.
 */
export function toRelativePath(filePath: string, cwd: string): string {
  const relative = path.relative(cwd, filePath);
  if (!relative || !isInsideDirectory(cwd, filePath)) {
    return toPosixPath(filePath);
  }
  return toPosixPath(relative);
}

/**
 * File extensions the way the scan compares them: without the leading dot,
 * in lower case, each once, in the order given. An entry with nothing
 * behind the dot is dropped.
 */
export function normalizeExtensions(extensions: readonly string[]): string[] {
  const normalized = extensions
    .map((extension) => extension.trim().replace(/^\./, "").toLowerCase())
    .filter((extension) => extension.length > 0);
  return [...new Set(normalized)];
}

/**
 * Code-unit order, the same on every machine. `localeCompare` depends on
 * the host's default locale and ICU data, and the order of files, cluster
 * members and clusters is part of the report.
 */
export function compareStrings(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}
