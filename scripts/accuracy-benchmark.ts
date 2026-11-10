/**
 * Accuracy benchmark for the README refactoring flow.
 *
 * Runs every labeled case from `bench/cases.ts` through `analyzeProject`
 * with the CLI defaults (`threshold 0.8`, `minLines 3`) and scores the
 * report against the case's ground truth:
 *
 *   - expected pair reported      → true positive
 *   - expected pair missing       → false negative
 *   - forbidden pair reported     → false positive
 *   - forbidden pair not reported → true negative
 *
 * Usage:
 *   tsx scripts/accuracy-benchmark.ts            # markdown summary
 *   tsx scripts/accuracy-benchmark.ts --json     # machine-readable output
 *   tsx scripts/accuracy-benchmark.ts --verbose  # per-pair detail rows
 */

import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { benchCases, type BenchCase } from "../bench/cases.js";
import { DEFAULT_MIN_LINES, DEFAULT_THRESHOLD } from "../src/defaults.js";
import { analyzeProject } from "../src/index.js";
import type { SimilarityPair } from "../src/types.js";

interface PairOutcome {
  caseId: string;
  category: string;
  mode: string;
  left: string;
  right: string;
  expected: "duplicate" | "distinct";
  reported: boolean;
  correct: boolean;
  /** Best observed similarity for the pair regardless of threshold. */
  observed: number | null;
}

function pairKey(a: string, b: string): string {
  return [a, b].sort().join("\u0000");
}

function findPair(pairs: SimilarityPair[], left: string, right: string): SimilarityPair | undefined {
  const wanted = pairKey(left, right);
  return pairs.find(
    (pair) => pairKey(pair.left.symbolName, pair.right.symbolName) === wanted,
  );
}

async function runCase(benchCase: BenchCase): Promise<PairOutcome[]> {
  const projectDir = await fs.mkdtemp(path.join(os.tmpdir(), "similarity-bench-"));
  try {
    for (const [relative, content] of Object.entries(benchCase.files)) {
      const filePath = path.join(projectDir, relative);
      await fs.mkdir(path.dirname(filePath), { recursive: true });
      await fs.writeFile(filePath, content, "utf8");
    }

    const analyze = (threshold: number) =>
      analyzeProject({
        cwd: projectDir,
        paths: ["."],
        modes: [benchCase.mode],
        threshold,
        minLines: DEFAULT_MIN_LINES,
      });

    const report = await analyze(DEFAULT_THRESHOLD);
    const reportedPairs = report.byMode[benchCase.mode];

    const outcomes: PairOutcome[] = [];
    const labeled: { left: string; right: string; expected: "duplicate" | "distinct" }[] = [
      ...(benchCase.expectPairs ?? []).map(([left, right]) => ({
        left,
        right,
        expected: "duplicate" as const,
      })),
      ...(benchCase.forbidPairs ?? []).map(([left, right]) => ({
        left,
        right,
        expected: "distinct" as const,
      })),
    ];

    // A second pass at threshold 0 recovers the raw similarity for
    // diagnostics; it never affects scoring. Only run it when at least
    // one labeled pair needs the number (missing duplicate or any
    // distinct pair, whose observed score is informative either way).
    let unfiltered: SimilarityPair[] | null = null;
    const needUnfiltered = labeled.some(({ left, right, expected }) => {
      const reported = findPair(reportedPairs, left, right) !== undefined;
      return expected === "distinct" || !reported;
    });
    if (needUnfiltered) {
      const fullReport = await analyze(0);
      unfiltered = fullReport.byMode[benchCase.mode];
    }

    for (const { left, right, expected } of labeled) {
      const match = findPair(reportedPairs, left, right);
      const reported = match !== undefined;
      const observed = match?.similarity
        ?? (unfiltered ? findPair(unfiltered, left, right)?.similarity ?? null : null);
      outcomes.push({
        caseId: benchCase.id,
        category: benchCase.category,
        mode: benchCase.mode,
        left,
        right,
        expected,
        reported,
        correct: expected === "duplicate" ? reported : !reported,
        observed,
      });
    }

    return outcomes;
  } finally {
    await fs.rm(projectDir, { recursive: true, force: true });
  }
}

interface Summary {
  generatedAt: string;
  threshold: number;
  minLines: number;
  totalPairs: number;
  truePositives: number;
  falseNegatives: number;
  trueNegatives: number;
  falsePositives: number;
  accuracy: number;
  errorRate: number;
  byMode: Record<string, { total: number; errors: number }>;
  failures: PairOutcome[];
}

function summarize(outcomes: PairOutcome[]): Summary {
  const truePositives = outcomes.filter((o) => o.expected === "duplicate" && o.reported).length;
  const falseNegatives = outcomes.filter((o) => o.expected === "duplicate" && !o.reported).length;
  const trueNegatives = outcomes.filter((o) => o.expected === "distinct" && !o.reported).length;
  const falsePositives = outcomes.filter((o) => o.expected === "distinct" && o.reported).length;
  const totalPairs = outcomes.length;
  const correct = truePositives + trueNegatives;

  const byMode: Record<string, { total: number; errors: number }> = {};
  for (const outcome of outcomes) {
    const mode = (byMode[outcome.mode] ??= { total: 0, errors: 0 });
    mode.total += 1;
    if (!outcome.correct) {
      mode.errors += 1;
    }
  }

  return {
    generatedAt: new Date().toISOString(),
    threshold: DEFAULT_THRESHOLD,
    minLines: DEFAULT_MIN_LINES,
    totalPairs,
    truePositives,
    falseNegatives,
    trueNegatives,
    falsePositives,
    accuracy: totalPairs === 0 ? 1 : correct / totalPairs,
    errorRate: totalPairs === 0 ? 0 : (totalPairs - correct) / totalPairs,
    byMode,
    failures: outcomes.filter((o) => !o.correct),
  };
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(2)}%`;
}

function formatObserved(observed: number | null): string {
  return observed === null ? "n/a" : observed.toFixed(3);
}

function toMarkdown(summary: Summary, outcomes: PairOutcome[], verbose: boolean): string {
  const lines: string[] = [
    "# Refactoring Accuracy Benchmark",
    "",
    `Generated at: ${summary.generatedAt}`,
    "",
    `Settings: threshold ${summary.threshold} (CLI default), minLines ${summary.minLines} (CLI default)`,
    "",
    "## Summary",
    "",
    "| Metric | Value |",
    "| --- | ---: |",
    `| Labeled pairs | ${summary.totalPairs} |`,
    `| True positives | ${summary.truePositives} |`,
    `| False negatives | ${summary.falseNegatives} |`,
    `| True negatives | ${summary.trueNegatives} |`,
    `| False positives | ${summary.falsePositives} |`,
    `| Accuracy | ${formatPercent(summary.accuracy)} |`,
    `| Error rate | ${formatPercent(summary.errorRate)} |`,
    "",
    "## By mode",
    "",
    "| Mode | Pairs | Errors | Accuracy |",
    "| --- | ---: | ---: | ---: |",
  ];

  for (const [mode, { total, errors }] of Object.entries(summary.byMode)) {
    lines.push(`| ${mode} | ${total} | ${errors} | ${formatPercent((total - errors) / total)} |`);
  }

  if (summary.failures.length > 0) {
    lines.push("", "## Failures", "", "| Case | Mode | Pair | Expected | Observed sim |", "| --- | --- | --- | --- | ---: |");
    for (const failure of summary.failures) {
      lines.push(
        `| ${failure.caseId} | ${failure.mode} | ${failure.left} ↔ ${failure.right} | ${failure.expected} | ${formatObserved(failure.observed)} |`,
      );
    }
  } else {
    lines.push("", "## Failures", "", "None — every labeled pair was classified correctly.");
  }

  if (verbose) {
    lines.push("", "## All pairs", "", "| Case | Mode | Pair | Expected | Reported | Observed sim | OK |", "| --- | --- | --- | --- | --- | ---: | --- |");
    for (const outcome of outcomes) {
      lines.push(
        `| ${outcome.caseId} | ${outcome.mode} | ${outcome.left} ↔ ${outcome.right} | ${outcome.expected} | ${outcome.reported ? "yes" : "no"} | ${formatObserved(outcome.observed)} | ${outcome.correct ? "✓" : "✗"} |`,
      );
    }
  }

  return `${lines.join("\n")}\n`;
}

export async function runBenchmark(): Promise<{ summary: Summary; outcomes: PairOutcome[] }> {
  const outcomes: PairOutcome[] = [];
  for (const benchCase of benchCases) {
    outcomes.push(...(await runCase(benchCase)));
  }
  return { summary: summarize(outcomes), outcomes };
}

const isDirectRun = process.argv[1]
  ? path.resolve(process.argv[1]).includes("accuracy-benchmark")
  : false;

if (isDirectRun) {
  const json = process.argv.includes("--json");
  const verbose = process.argv.includes("--verbose");
  const { summary, outcomes } = await runBenchmark();
  if (json) {
    process.stdout.write(`${JSON.stringify({ summary, outcomes }, null, 2)}\n`);
  } else {
    process.stdout.write(toMarkdown(summary, outcomes, verbose));
  }
}
