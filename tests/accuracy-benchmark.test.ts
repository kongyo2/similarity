import { describe, expect, it } from "vitest";
import { runBenchmark } from "../scripts/accuracy-benchmark.js";

// Error rate measured for the v0.5.0 corpus (261 labeled pairs across the
// original 71-pair core plus the extended hard categories: whole-function
// renames, guard/negation/ternary spellings, loop-form rewrites,
// destructuring, nullish sugar, literal-vs-behavior twins, realistic
// cross-file copy-paste) when analyzed with the v0.4.1 engine:
// 89 wrong labels out of 261 (34.10%). The suite pins the "at least 10x
// more accurate" guarantee: the labeled-corpus error rate must stay at or
// below one tenth of that baseline.
const BASELINE_ERROR_RATE = 89 / 261;

describe("refactoring accuracy benchmark", () => {
  it("keeps the corpus error rate at least 10x below the 0.4.1 baseline", async () => {
    const { summary, outcomes } = await runBenchmark();

    // Guard against the corpus accidentally shrinking — the budget is
    // only meaningful while the corpus stays representative.
    expect(summary.totalPairs).toBeGreaterThanOrEqual(260);
    expect(summary.errorRate).toBeLessThanOrEqual(BASELINE_ERROR_RATE / 10);

    // The original 71-pair corpus (F-/T-/C- ids) gated v0.4.0 at 100%
    // accuracy; that must not regress while the extended corpus evolves.
    const coreErrors = outcomes.filter(
      (outcome) => !outcome.caseId.startsWith("X") && !outcome.correct,
    );
    expect(coreErrors).toEqual([]);
  }, 240_000);
});
