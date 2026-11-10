import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  compareStrings,
  isInsideDirectory,
  toPosixPath,
  toRelativePath,
} from "../src/utils/path.js";

describe("path utilities", () => {
  it("normalizes platform separators to POSIX separators", () => {
    expect(toPosixPath(["src", "nested", "file.ts"].join(path.sep))).toBe("src/nested/file.ts");
  });

  it("returns a POSIX relative path for files inside cwd", () => {
    expect(toRelativePath(path.join("/repo", "src", "file.ts"), "/repo")).toBe("src/file.ts");
    // A name that merely starts with two dots is inside.
    expect(toRelativePath(path.join("/repo", "..cache", "file.ts"), "/repo")).toBe(
      "..cache/file.ts",
    );
  });

  it("keeps absolute POSIX paths for files outside cwd", () => {
    expect(toRelativePath(path.join("/external", "file.ts"), "/repo")).toBe("/external/file.ts");
    expect(toRelativePath(path.join("/repository", "file.ts"), "/repo")).toBe(
      "/repository/file.ts",
    );
    // The directory itself has no relative name.
    expect(toRelativePath("/repo", "/repo")).toBe("/repo");
  });

  it("tells what lies inside a directory", () => {
    expect(isInsideDirectory("/repo", "/repo")).toBe(true);
    expect(isInsideDirectory("/repo", path.join("/repo", "src", "a.ts"))).toBe(true);
    expect(isInsideDirectory("/repo", path.join("/repo", "..hidden"))).toBe(true);
    expect(isInsideDirectory("/repo", "/")).toBe(false);
    expect(isInsideDirectory("/repo", "/repository")).toBe(false);
    expect(isInsideDirectory(path.join("/repo", "src"), path.join("/repo", "lib"))).toBe(false);
  });

  it("orders strings by code unit", () => {
    const names = ["b", "ä", "B", "a", "Z", "a1", "_"];
    expect([...names].sort(compareStrings)).toEqual(["B", "Z", "_", "a", "a1", "b", "ä"]);
    expect(compareStrings("same", "same")).toBe(0);
  });
});
