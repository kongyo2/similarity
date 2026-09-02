import { describe, expect, it } from "vitest";
import { runBenchmark } from "../scripts/accuracy-benchmark.js";

// The corpus (300 labeled pairs: the original 71-pair core, the extended
// hard categories — whole-function renames, guard/negation/ternary
// spellings, loop-form rewrites, destructuring, nullish sugar,
// literal-vs-behavior twins, boundary-index and fold-direction twins,
// realistic cross-file copy-paste — and the real-world lookalikes and
// declaration scopes lifted from the TypeScript compiler sources in 0.7.0)
// scored 89 wrong labels (34.10%) on the v0.4.1 engine, 7 (2.68%) on
// v0.5.0, and 29 of the 39 pairs added in 0.7.0 on v0.6.0. v0.7.0
// classifies every labeled pair correctly, and this suite pins that: any
// mislabeled pair is a regression.
describe("refactoring accuracy benchmark", () => {
  it("classifies every labeled corpus pair correctly", async () => {
    const { summary } = await runBenchmark();

    // Guard against the corpus accidentally shrinking below its 0.7.0 size
    // — the gate is only meaningful while the corpus stays representative.
    // Keep this in step with the count in README.md and CHANGELOG.md.
    expect(summary.totalPairs).toBeGreaterThanOrEqual(300);

    // 100% accuracy: failures carry the offending pairs, so an empty-array
    // assertion names the regressed cases directly in the diff.
    expect(summary.failures).toEqual([]);
  }, 240_000);
});
