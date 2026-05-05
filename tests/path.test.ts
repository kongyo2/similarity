import path from "node:path";
import { describe, expect, it } from "vitest";
import { toPosixPath, toRelativePath } from "../src/utils/path.js";

describe("path utilities", () => {
  it("normalizes platform separators to POSIX separators", () => {
    expect(toPosixPath(["src", "nested", "file.ts"].join(path.sep))).toBe(
      "src/nested/file.ts",
    );
  });

  it("returns a POSIX relative path for files inside cwd", () => {
    expect(toRelativePath(path.join("/repo", "src", "file.ts"), "/repo")).toBe(
      "src/file.ts",
    );
  });

  it("keeps absolute POSIX paths for files outside cwd", () => {
    expect(toRelativePath(path.join("/external", "file.ts"), "/repo")).toBe(
      "/external/file.ts",
    );
  });
});
