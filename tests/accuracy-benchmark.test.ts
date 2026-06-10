import { describe, expect, it } from "vitest";
import { runBenchmark } from "../scripts/accuracy-benchmark.js";

// Error rate measured for this corpus on v0.3.0 (before the semantic
// canonicalization layer): 15 wrong labels out of 71 (21.13%). The suite
// pins the "at least 10x more accurate" guarantee: the labeled-corpus
// error rate must stay at or below one tenth of that baseline.
const BASELINE_ERROR_RATE = 15 / 71;

describe("refactoring accuracy benchmark", () => {
  it("keeps the corpus error rate at least 10x below the 0.3.0 baseline", async () => {
    const { summary } = await runBenchmark();

    // Guard against the corpus accidentally shrinking — the budget is
    // only meaningful while the corpus stays representative.
    expect(summary.totalPairs).toBeGreaterThanOrEqual(70);
    expect(summary.errorRate).toBeLessThanOrEqual(BASELINE_ERROR_RATE / 10);
  }, 120_000);
});
