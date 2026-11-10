import fs from "node:fs/promises";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { collectTypeScriptFiles } from "../src/utils/files.js";
import { withTempProject, writeTree } from "./helpers.js";

const SOURCE = "export const value = true;\n";

function collect(cwd: string, paths: string[], exclude: string[] = [], extensions = ["ts"]) {
  return collectTypeScriptFiles({ cwd, paths, extensions, exclude });
}

/** Paths relative to `root`, with forward slashes. */
function relativeTo(root: string, files: string[]): string[] {
  return files.map((file) => path.relative(root, file).split(path.sep).join("/"));
}

describe("collectTypeScriptFiles", () => {
  it("includes explicitly passed supported files that are not ignored", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "src/direct.ts": SOURCE });

      const result = await collect(projectDir, ["src/direct.ts"]);

      expect(result.files).toEqual([path.join(projectDir, "src", "direct.ts")]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("collects several explicit files and lists each file once", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "first.ts": SOURCE, "second.ts": SOURCE, "src/third.ts": SOURCE });

      // `second.ts` is named twice and found again under `.`.
      const result = await collect(projectDir, ["second.ts", "first.ts", ".", "second.ts"]);

      expect(relativeTo(projectDir, result.files)).toEqual(["first.ts", "second.ts", "src/third.ts"]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("reports missing paths and unsupported path types as warnings", async () => {
    await withTempProject(async (projectDir) => {
      await fs.symlink("/dev/null", path.join(projectDir, "device.ts"));

      const result = await collect(projectDir, ["device.ts", "missing"]);

      expect(result.files).toEqual([]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([
        "Unsupported path type: device.ts",
        "Path not found or not accessible: missing",
      ]);
    });
  });

  it("skips explicitly passed files with disabled extensions", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "plain.js": SOURCE, "view.TSX": SOURCE });

      const result = await collect(projectDir, ["plain.js", "view.TSX"], [], [".tsx"]);

      // Extensions compare without their dot and without regard to case.
      expect(relativeTo(projectDir, result.files)).toEqual(["view.TSX"]);
      expect(relativeTo(projectDir, result.skipped)).toEqual(["plain.js"]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("falls back to the target directory when no enclosing git root is found", async () => {
    await withTempProject(async (projectDir) => {
      const cwd = path.join(projectDir, "cwd");
      await fs.mkdir(cwd);

      let externalTarget = path.join(projectDir, "external");
      for (let index = 0; index < 130; index += 1) {
        externalTarget = path.join(externalTarget, "d");
      }
      await fs.mkdir(externalTarget, { recursive: true });
      const filePath = path.join(externalTarget, "deep.ts");
      await fs.writeFile(filePath, SOURCE, "utf8");

      const result = await collect(cwd, [externalTarget]);

      expect(result.files).toEqual([filePath]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("returns no directory matches when no extensions are enabled", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "src/ignored.ts": SOURCE });

      const result = await collect(projectDir, ["src"], [], []);

      expect(result.files).toEqual([]);
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("applies explicit exclude patterns in addition to default ignores", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, { "src/kept.ts": SOURCE, "src/skip.generated.ts": SOURCE });

      const result = await collect(projectDir, ["src"], ["**/*.generated.ts"]);

      expect(relativeTo(projectDir, result.files)).toEqual(["src/kept.ts"]);
      expect(relativeTo(projectDir, result.skipped)).toEqual(["src/skip.generated.ts"]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("does not enter node_modules, dist or coverage at any depth", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "src/app.ts": SOURCE,
        "node_modules/pkg/index.ts": SOURCE,
        "dist/app.ts": SOURCE,
        "coverage/report.ts": SOURCE,
        // Only the top-level directories used to be excluded: the
        // dependencies and build output of every workspace package were
        // analyzed.
        "packages/a/src/a.ts": SOURCE,
        "packages/a/node_modules/dep/index.ts": SOURCE,
        "packages/a/dist/a.ts": SOURCE,
        "packages/b/coverage/lcov-report/b.ts": SOURCE,
        // Files of those names are not directories.
        "src/dist.ts": SOURCE,
      });

      const result = await collect(projectDir, ["."]);

      expect(relativeTo(projectDir, result.files)).toEqual([
        "packages/a/src/a.ts",
        "src/app.ts",
        "src/dist.ts",
      ]);
      // What an ignored directory holds is never read, let alone listed.
      expect(result.skipped).toEqual([]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("scans a default-excluded directory that is named explicitly", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "dist/app.ts": SOURCE,
        "dist/nested/dist/deep.ts": SOURCE,
        "dist/node_modules/dep/index.ts": SOURCE,
        "node_modules/pkg/index.ts": SOURCE,
      });

      const directory = await collect(projectDir, ["dist"]);
      // The target itself is exempt; what lies below it is not.
      expect(relativeTo(projectDir, directory.files)).toEqual(["dist/app.ts"]);

      const file = await collect(projectDir, ["node_modules/pkg/index.ts"]);
      expect(relativeTo(projectDir, file.files)).toEqual(["node_modules/pkg/index.ts"]);
      expect(file.skipped).toEqual([]);
    });
  });

  it("gives a deeper .gitignore precedence over a shallower one", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        ".gitignore": "*.gen.ts\nbuild/\n",
        "src/.gitignore": "!keep.gen.ts\nlocal.ts\n",
        "src/a.gen.ts": SOURCE,
        "src/keep.gen.ts": SOURCE,
        "src/local.ts": SOURCE,
        "src/plain.ts": SOURCE,
        "other/keep.gen.ts": SOURCE,
        // A file cannot be re-included when its directory is ignored.
        "build/.gitignore": "!out.ts\n",
        "build/out.ts": SOURCE,
      });

      const result = await collect(projectDir, ["."]);

      expect(relativeTo(projectDir, result.files)).toEqual(["src/keep.gen.ts", "src/plain.ts"]);
      expect(relativeTo(projectDir, result.skipped)).toEqual([
        "other/keep.gen.ts",
        "src/a.gen.ts",
        "src/local.ts",
      ]);
    });
  });

  it("lets --exclude win over a .gitignore that re-includes a file", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        ".gitignore": "*.gen.ts\n!keep.gen.ts\n",
        "src/keep.gen.ts": SOURCE,
        "src/plain.ts": SOURCE,
      });

      const kept = await collect(projectDir, ["."]);
      expect(relativeTo(projectDir, kept.files)).toEqual(["src/keep.gen.ts", "src/plain.ts"]);

      const excluded = await collect(projectDir, ["."], ["src/keep.gen.ts"]);
      expect(relativeTo(projectDir, excluded.files)).toEqual(["src/plain.ts"]);
      expect(relativeTo(projectDir, excluded.skipped)).toEqual(["src/keep.gen.ts"]);
    });
  });

  it("applies the .gitignore files of the repository above the working directory", async () => {
    await withTempProject(async (projectDir) => {
      // The scan runs in a package of a repository whose root ignores
      // generated sources.
      await writeTree(projectDir, {
        ".git/HEAD": "ref: refs/heads/main\n",
        ".gitignore": "generated/\n*.snap.ts\n",
        "packages/app/src/app.ts": SOURCE,
        "packages/app/src/view.snap.ts": SOURCE,
        "packages/app/generated/client.ts": SOURCE,
      });
      const cwd = path.join(projectDir, "packages", "app");

      const result = await collect(cwd, ["."]);

      expect(relativeTo(cwd, result.files)).toEqual(["src/app.ts"]);
      expect(relativeTo(cwd, result.skipped)).toEqual(["src/view.snap.ts"]);

      // `--exclude` patterns stay relative to the working directory.
      const excluded = await collect(cwd, ["."], ["/src/app.ts"]);
      expect(excluded.files).toEqual([]);
    });
  });

  it("skips an explicitly named target that is ignored", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        ".gitignore": "generated/\nsecret.ts\n",
        "generated/client.ts": SOURCE,
        "generated/deep/more.ts": SOURCE,
        "secret.ts": SOURCE,
        "src/app.ts": SOURCE,
      });

      const result = await collect(projectDir, ["generated", "secret.ts", "generated/client.ts"]);

      expect(result.files).toEqual([]);
      // The directory is listed, not everything in it.
      expect(relativeTo(projectDir, result.skipped)).toEqual([
        "generated",
        "generated/client.ts",
        "secret.ts",
      ]);
      expect(result.warnings).toEqual([]);
    });
  });

  it("skips dot entries and does not follow symbolic links", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "src/app.ts": SOURCE,
        "src/.hidden.ts": SOURCE,
        ".cache/generated.ts": SOURCE,
        "elsewhere/linked.ts": SOURCE,
      });
      await fs.symlink(path.join(projectDir, "elsewhere"), path.join(projectDir, "src", "link"));
      await fs.symlink(
        path.join(projectDir, "elsewhere", "linked.ts"),
        path.join(projectDir, "src", "alias.ts"),
      );

      const result = await collect(projectDir, ["src"]);
      expect(relativeTo(projectDir, result.files)).toEqual(["src/app.ts"]);

      // A dot directory named as the target is scanned.
      const hidden = await collect(projectDir, [".cache"]);
      expect(relativeTo(projectDir, hidden.files)).toEqual([".cache/generated.ts"]);
    });
  });

  it("orders files by code unit, whatever the locale", async () => {
    await withTempProject(async (projectDir) => {
      await writeTree(projectDir, {
        "b.ts": SOURCE,
        "B.ts": SOURCE,
        "a.ts": SOURCE,
        "Z/z.ts": SOURCE,
        "a/z.ts": SOURCE,
        "ä.ts": SOURCE,
      });

      const result = await collect(projectDir, ["."]);

      // Locale order would interleave the cases and put `ä` next to `a`.
      expect(relativeTo(projectDir, result.files)).toEqual([
        "B.ts",
        "Z/z.ts",
        "a.ts",
        "a/z.ts",
        "b.ts",
        "ä.ts",
      ]);
    });
  });

  it.skipIf(typeof process.getuid === "function" && process.getuid() === 0)(
    "warns about a directory that cannot be read",
    async () => {
      await withTempProject(async (projectDir) => {
        await writeTree(projectDir, { "src/app.ts": SOURCE, "src/locked/inner.ts": SOURCE });
        const locked = path.join(projectDir, "src", "locked");
        await fs.chmod(locked, 0o000);
        try {
          const result = await collect(projectDir, ["src"]);
          expect(relativeTo(projectDir, result.files)).toEqual(["src/app.ts"]);
          expect(result.warnings).toHaveLength(1);
          expect(result.warnings[0]).toContain(`Failed to read directory ${locked}`);
        } finally {
          await fs.chmod(locked, 0o755);
        }
      });
    },
  );
});
