import path from "node:path";
import { describe, expect, it } from "vitest";
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
    stats: {
      fileCount: 2,
      pairCount: 1,
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
      ...createReport({ results: [], stats: { fileCount: 0, pairCount: 0, elapsedMs: 0 } }),
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
});
