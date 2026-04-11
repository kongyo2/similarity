import path from "node:path";

export function toPosixPath(inputPath: string): string {
  return inputPath.split(path.sep).join("/");
}

export function toRelativePath(filePath: string, cwd: string): string {
  const relative = path.relative(cwd, filePath);
  if (!relative || relative.startsWith("..")) {
    return toPosixPath(filePath);
  }
  return toPosixPath(relative);
}
