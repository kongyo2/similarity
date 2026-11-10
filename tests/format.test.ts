import path from "node:path";
import { describe, expect, it } from "vitest";
import { buildClusters, clusterPairs, standalonePairs } from "../src/clusters.js";
import { formatJsonReport, formatPrettyReport, formatWarning } from "../src/format.js";
import type { AnalyzeReport, SimilarityPair } from "../src/types.js";

function createPair(overrides: Partial<SimilarityPair> = {}): SimilarityPair {
  return {
    mode: "functions",
    similarity: 0.91234,
    left: {
      filePath: path.join("/repo", "src", "left.ts"),
      startLine: 2,
      endLine: 8,
      symbolName: "computeTotal",
      kind: "function",
    },
    right: {
      filePath: path.join("/repo", "src", "right.ts"),
      startLine: 3,
      endLine: 9,
      symbolName: "calculateSum",
      kind: "function",
    },
    ...overrides,
  };
}

function createReport(overrides: Partial<AnalyzeReport> = {}): AnalyzeReport {
  const pair = createPair();
  return {
    analyzedFiles: [pair.left.filePath, pair.right.filePath],
    skippedFiles: [],
    warnings: [],
    results: [pair],
    byMode: {
      functions: [pair],
      types: [],
      classes: [],
      overlap: [],
    },
    clusters: [],
    stats: {
      fileCount: 2,
      pairCount: 1,
      clusterCount: 0,
      elapsedMs: 17,
    },
    ...overrides,
  };
}

describe("formatJsonReport", () => {
  it("serializes the complete report as indented JSON", () => {
    const report = createReport({
      warnings: [{ filePath: "/repo/src/broken.ts", message: "Parse errors: 1" }],
    });

    const rendered = formatJsonReport(report);
    const parsed = JSON.parse(rendered) as AnalyzeReport;

    expect(rendered).toContain('\n  "analyzedFiles": [');
    expect(parsed).toEqual(report);
  });
});

describe("formatPrettyReport", () => {
  it("renders summary, mode sections, pair rows, empty states, and warnings", () => {
    const report = createReport({
      warnings: [{ message: "Path not found or not accessible: missing.ts" }],
    });

    const rendered = formatPrettyReport(report, "/repo", [
      "functions",
      "types",
      "classes",
      "overlap",
    ]);

    expect(rendered).toContain("Analyzing code similarity...");
    expect(rendered).toContain("Files analyzed: 2");
    expect(rendered).toContain("Pairs detected: 1");
    expect(rendered).toContain("Elapsed: 17 ms");
    expect(rendered).toContain("=== Function Similarity ===");
    expect(rendered).toContain("=== Type Similarity ===");
    expect(rendered).toContain("=== Class Similarity ===");
    expect(rendered).toContain("=== Overlap Detection ===");
    expect(rendered).toContain("0.912");
    expect(rendered).toContain("src/left.ts:2-8 computeTotal");
    expect(rendered).toContain("src/right.ts:3-9 calculateSum");
    expect(rendered).toContain("Total pairs: 1");
    expect(rendered).toContain("No similar pairs found.");
    expect(rendered).toContain("Warnings:");
    expect(rendered).toContain("- Path not found or not accessible: missing.ts");
  });

  it("styles the report only when asked to", () => {
    const report = createReport({ warnings: [{ message: "Path not found: missing.ts" }] });
    const escape = "\u001b";

    // Plain by default: the text is as fit for a file or a pipe as for a
    // terminal, table borders and header included.
    const plain = formatPrettyReport(report, "/repo", ["functions"]);
    expect(plain).not.toContain(escape);
    expect(plain).toContain("│ Similarity │");
    expect(formatPrettyReport(report, "/repo", ["functions"], { color: false })).toBe(plain);

    const styled = formatPrettyReport(report, "/repo", ["functions"], { color: true });
    expect(styled).toContain(`${escape}[1mAnalyzing code similarity...${escape}[22m`);
    expect(styled).toContain(`${escape}[1m=== Function Similarity ===${escape}[22m`);
    expect(styled).toContain(`${escape}[33mWarnings:${escape}[39m`);
    // The same text once the styling is taken away.
    // eslint-disable-next-line no-control-regex
    expect(styled.replace(/\u001b\[\d+m/g, "")).toBe(plain);
  });

  it("names the file a warning is about, relative to the working directory", () => {
    const report = createReport({
      warnings: [
        { filePath: "/repo/src/broken.ts", message: "skipped: Parse errors: Unexpected token" },
        { filePath: "/external/deep.ts", message: "skipped: code is nested too deeply to parse" },
        // A message that names its file already is not prefixed again.
        {
          filePath: "/repo/src/locked.ts",
          message: "Failed to read /repo/src/locked.ts: EACCES: permission denied",
        },
        { message: "Path not found or not accessible: missing.ts" },
      ],
    });

    const rendered = formatPrettyReport(report, "/repo", ["functions"]);
    const warnings = rendered.slice(rendered.indexOf("Warnings:")).split("\n");

    expect(warnings).toEqual([
      "Warnings:",
      "- src/broken.ts: skipped: Parse errors: Unexpected token",
      "- /external/deep.ts: skipped: code is nested too deeply to parse",
      "- Failed to read /repo/src/locked.ts: EACCES: permission denied",
      "- Path not found or not accessible: missing.ts",
    ]);
  });

  it("words a warning the same wherever it is printed", () => {
    // The report lists warnings and the command repeats them on stderr.
    expect(
      formatWarning({ filePath: "/repo/src/broken.ts", message: "skipped: Parse errors: 1" }, "/repo"),
    ).toBe("src/broken.ts: skipped: Parse errors: 1");
    expect(
      formatWarning({ filePath: "/external/deep.ts", message: "skipped: nested" }, "/repo"),
    ).toBe("/external/deep.ts: skipped: nested");
    expect(
      formatWarning(
        { filePath: "/repo/locked.ts", message: "Failed to read /repo/locked.ts: EACCES" },
        "/repo",
      ),
    ).toBe("Failed to read /repo/locked.ts: EACCES");
    expect(formatWarning({ message: "Path not found: missing.ts" }, "/repo")).toBe(
      "Path not found: missing.ts",
    );
  });

  it("uses absolute paths for pair locations outside the current working directory", () => {
    const outsidePair = createPair({
      right: {
        filePath: path.join("/external", "right.ts"),
        startLine: 10,
        endLine: 12,
        symbolName: "outside",
        kind: "function",
      },
    });
    const report = createReport({
      results: [outsidePair],
      byMode: {
        functions: [outsidePair],
        types: [],
        classes: [],
        overlap: [],
      },
    });

    const rendered = formatPrettyReport(report, "/repo", ["functions"]);

    expect(rendered).toContain("/external/right.ts:10-12 outside");
  });

  /** The text of a table column, its cells joined the way they were wrapped. */
  function columnText(rendered: string, column: number): string {
    return rendered
      .split("\n")
      .filter((line) => line.startsWith("│"))
      .map((line) => (line.split("│")[column] ?? "").trim())
      .join("");
  }

  it("wraps a location the table has no room for instead of cutting it off", () => {
    const deep = "packages/server/src/unstable-core-do-not-import/stream/utils";
    const pair = createPair({
      left: {
        filePath: path.join("/repo", deep, "withPing.test.ts"),
        startLine: 108,
        endLine: 115,
        symbolName: "IterableEventEmitter",
        kind: "class",
      },
      right: {
        // No separator to break at: one name wider than the column.
        filePath: path.join("/repo", `${"generated".repeat(8)}.ts`),
        startLine: 7,
        endLine: 9,
        symbolName: "IterableEventEmitter",
        kind: "class",
      },
    });
    const report = createReport({
      results: [pair],
      byMode: { functions: [pair], types: [], classes: [], overlap: [] },
    });

    const rendered = formatPrettyReport(report, "/repo", ["functions"]);

    // The file name and the lines are what the row is for.
    expect(rendered).not.toContain("…");
    expect(columnText(rendered, 2)).toContain(`${deep}/withPing.test.ts:108-115`);
    expect(columnText(rendered, 3)).toContain(`${"generated".repeat(8)}.ts:7-9`);
    // Path segments stay whole where a line can end between them.
    expect(rendered).toContain("│ packages/server/src/unstable-core-do-not-import/       │");
    expect(rendered).toContain("│ stream/utils/withPing.test.ts:108-115                  │");
    // Every row keeps the table's width.
    const rows = rendered.split("\n").filter((line) => /^[│├┌└]/.test(line));
    expect(new Set(rows.map((row) => row.length)).size).toBe(1);
  });

  it("measures a location in columns, not in characters", () => {
    // 20 characters of two columns each, then a file: 44 characters that
    // take 64 columns, in a cell of 54.
    const wide = "機能".repeat(10);
    const pair = createPair({
      left: {
        filePath: path.join("/repo", "src", wide, "集計.ts"),
        startLine: 3,
        endLine: 9,
        symbolName: "合計",
        kind: "function",
      },
    });
    const report = createReport({
      results: [pair],
      byMode: { functions: [pair], types: [], classes: [], overlap: [] },
    });

    const rendered = formatPrettyReport(report, "/repo", ["functions"]);

    expect(rendered).not.toContain("…");
    expect(columnText(rendered, 2)).toContain(`src/${wide}/集計.ts:3-9`);
  });

  it("falls back to the raw mode name for unknown runtime mode values", () => {
    const report = {
      ...createReport({ results: [], stats: { fileCount: 0, pairCount: 0, clusterCount: 0, elapsedMs: 0 } }),
      byMode: {
        functions: [],
        types: [],
        classes: [],
        overlap: [],
        custom: [],
      },
    };

    const rendered = formatPrettyReport(report as AnalyzeReport, "/repo", [
      "custom" as never,
    ]);

    expect(rendered).toContain("=== custom ===");
    expect(rendered).toContain("No similar pairs found.");
  });

  it("lists three or more connected declarations once, as a cluster", () => {
    const at = (name: string, line: number) => ({
      filePath: path.join("/repo", "src", `${name}.ts`),
      startLine: line,
      endLine: line + 4,
      symbolName: name,
      kind: "function",
    });
    const [a, b, c, d, e] = [at("alpha", 1), at("beta", 2), at("gamma", 3), at("delta", 4), at("epsilon", 5)];
    const pairs: SimilarityPair[] = [
      createPair({ similarity: 1, left: a, right: b }),
      createPair({ similarity: 0.95, left: b, right: c }),
      createPair({ similarity: 0.9, left: a, right: c }),
      createPair({ similarity: 0.85, left: d, right: e }),
    ];
    const byMode = { functions: pairs, types: [], classes: [], overlap: [] };
    const report = createReport({
      results: pairs,
      byMode,
      clusters: buildClusters(byMode),
      stats: { fileCount: 5, pairCount: 4, clusterCount: 1, elapsedMs: 3 },
    });

    const rendered = formatPrettyReport(report, "/repo", ["functions"]);

    expect(rendered).toContain("Duplicate clusters: 1");
    expect(rendered).toContain(
      "Cluster 1: 3 functions, 3 pairwise matches, similarity 0.900–1.000 (avg 0.950)",
    );
    expect(rendered).toContain("  src/alpha.ts:1-5 alpha");
    expect(rendered).toContain("  src/gamma.ts:3-7 gamma");
    // The two-member component stays a plain pair row.
    expect(rendered).toContain("src/delta.ts:4-8 delta");
    expect(rendered).toContain("Total pairs: 4 (3 in 1 cluster, 1 standalone)");
    expect(rendered).not.toContain("Cluster 2");

    // The blocks are the report's clusters, the same list the summary line
    // counts: a report without clusters lists every pair as a row.
    const unclustered = formatPrettyReport({ ...report, clusters: [] }, "/repo", ["functions"]);
    expect(unclustered).not.toContain("Duplicate clusters");
    expect(unclustered).not.toContain("Cluster 1");
    expect(unclustered).toContain("Total pairs: 4");
    expect(unclustered).toContain("src/beta.ts:2-6 beta");
  });

  it("lists a fragment repeated in three or more functions once, with its shape", () => {
    const guard = { nodeCount: 26, nodeType: "IfStatement" };
    const pairs: SimilarityPair[] = [
      fragmentPair(["loadUser", 3, 6], ["loadOrder", 13, 16], guard),
      fragmentPair(["loadUser", 3, 6], ["loadInvoice", 27, 30], guard),
      fragmentPair(["loadOrder", 13, 16], ["loadInvoice", 27, 30], guard),
      fragmentPair(["loadUser", 2, 2], ["loadOrder", 12, 12], {
        nodeCount: 8,
        nodeType: "VariableDeclaration",
      }),
    ];
    const byMode = { functions: [], types: [], classes: [], overlap: pairs };
    const report = createReport({
      results: pairs,
      byMode,
      clusters: buildClusters(byMode),
      stats: { fileCount: 1, pairCount: 4, clusterCount: 1, elapsedMs: 3 },
    });

    const rendered = formatPrettyReport(report, "/repo", ["overlap"]);

    expect(rendered).toContain("Duplicate clusters: 1");
    // Every pair of the cluster scores the same: one number, no average.
    expect(rendered).toContain(
      "Cluster 1: 3 fragments (IfStatement, 26 nodes), 3 pairwise matches, similarity 1.000\n",
    );
    expect(rendered).toContain("  src/load.ts:3-6 loadUser\n");
    expect(rendered).toContain("  src/load.ts:13-16 loadOrder\n");
    expect(rendered).toContain("  src/load.ts:27-30 loadInvoice\n");
    expect(rendered).toContain("src/load.ts:2-2 loadUser");
    expect(rendered).toContain("Total pairs: 4 (3 in 1 cluster, 1 standalone)");
  });
});

/** An overlap pair between two fragments of functions in one file. */
function fragmentPair(
  left: [name: string, startLine: number, endLine: number],
  right: [name: string, startLine: number, endLine: number],
  details: { nodeCount: number; nodeType: string },
): SimilarityPair {
  const at = ([symbolName, startLine, endLine]: [string, number, number]) => ({
    filePath: path.join("/repo", "src", "load.ts"),
    startLine,
    endLine,
    symbolName,
    kind: "token-window",
  });
  return { mode: "overlap", similarity: 1, left: at(left), right: at(right), details };
}

describe("clusterPairs", () => {
  it("groups connected pairs and leaves two-member components standalone", () => {
    const at = (name: string) => ({
      filePath: `/repo/${name}.ts`,
      startLine: 1,
      endLine: 5,
      symbolName: name,
      kind: "function",
    });
    const pairs: SimilarityPair[] = [
      createPair({ similarity: 0.9, left: at("c"), right: at("d") }),
      createPair({ similarity: 1, left: at("a"), right: at("b") }),
      createPair({ similarity: 0.8, left: at("b"), right: at("c") }),
      createPair({ similarity: 0.99, left: at("x"), right: at("y") }),
    ];

    const { clusters, standalone } = clusterPairs("functions", pairs);

    expect(clusters).toHaveLength(1);
    expect(clusters[0]?.members.map((member) => member.symbolName)).toEqual(["a", "b", "c", "d"]);
    expect(clusters[0]).toMatchObject({
      mode: "functions",
      pairCount: 3,
      maxSimilarity: 1,
      minSimilarity: 0.8,
    });
    expect(clusters[0]?.averageSimilarity).toBeCloseTo(0.9, 10);
    expect(standalone.map((pair) => pair.left.symbolName)).toEqual(["x"]);
    expect(standalonePairs("functions", pairs, clusters)).toEqual(standalone);
    // A declaration is no fragment, and the cluster says nothing of one.
    expect(clusters[0]).not.toHaveProperty("fragment");

    // Every mode clusters, in the order reports list the modes.
    const all = buildClusters({ classes: pairs, functions: pairs });
    expect(all.map((cluster) => cluster.mode)).toEqual(["functions", "classes"]);
  });

  it("groups the copies of one fragment and tells fragments on the same lines apart", () => {
    const loop = { nodeCount: 40, nodeType: "ForOfStatement" };
    const body = { nodeCount: 37, nodeType: "BlockStatement" };
    // `sum`, `total` and `count` hold the same loop; the loop of `sum` and
    // its body span the same lines, and the body alone recurs in `drain`
    // and `flush`.
    const pairs: SimilarityPair[] = [
      fragmentPair(["sum", 4, 9], ["total", 14, 19], loop),
      fragmentPair(["total", 14, 19], ["count", 24, 29], loop),
      fragmentPair(["sum", 4, 9], ["drain", 33, 38], body),
      fragmentPair(["drain", 33, 38], ["flush", 43, 48], body),
      fragmentPair(["sum", 2, 2], ["total", 12, 12], { nodeCount: 9, nodeType: "IfStatement" }),
    ];

    const { clusters, standalone } = clusterPairs("overlap", pairs);

    expect(clusters).toHaveLength(2);
    expect(clusters.map((cluster) => cluster.fragment)).toEqual([loop, body]);
    expect(clusters.map((cluster) => cluster.members.map((member) => member.symbolName))).toEqual([
      ["sum", "total", "count"],
      ["sum", "drain", "flush"],
    ]);
    expect(clusters[0]).toMatchObject({ mode: "overlap", pairCount: 2, minSimilarity: 1 });
    expect(standalone).toEqual([pairs[4]]);
    expect(standalonePairs("overlap", pairs, clusters)).toEqual(standalone);
    // The clusters of another mode cover none of these pairs.
    expect(standalonePairs("functions", pairs, clusters)).toEqual(pairs);
  });

  it("orders members by code unit, independent of the host locale", () => {
    const at = (name: string) => ({
      filePath: `/repo/${name}.ts`,
      startLine: 1,
      endLine: 5,
      symbolName: name,
      kind: "function",
    });
    const pairs: SimilarityPair[] = [
      createPair({ left: at("a"), right: at("B") }),
      createPair({ left: at("B"), right: at("c") }),
    ];

    const { clusters } = clusterPairs("functions", pairs);

    // A locale-aware comparison sorts "a" before "B"; code-unit order puts
    // the uppercase letter first on every machine.
    expect(clusters[0]?.members.map((member) => member.symbolName)).toEqual(["B", "a", "c"]);
  });
});
