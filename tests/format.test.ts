import path from "node:path";
import { describe, expect, it } from "vitest";
import { buildClusters, clusterPairs, standalonePairs } from "../src/clusters.js";
import { formatJsonReport, formatPrettyReport } from "../src/format.js";
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
});

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

    // Overlap windows are not declarations and never cluster.
    const all = buildClusters({ functions: pairs, overlap: pairs });
    expect(all).toHaveLength(1);
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
