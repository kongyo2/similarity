import { describe, expect, it } from "vitest";
import { runBenchmark } from "../scripts/accuracy-benchmark.js";

// The corpus (337 labeled pairs: the original 71-pair core, the extended
// hard categories — whole-function renames, guard/negation/ternary
// spellings, loop-form rewrites, destructuring, nullish sugar,
// literal-vs-behavior twins, boundary-index and fold-direction twins,
// realistic cross-file copy-paste — the real-world lookalikes and
// declaration scopes lifted from the TypeScript compiler sources in 0.7.0,
// the families found on the fallow, tsrs and TypeScript repositories in
// 0.8.0, and the quoted-literal, own-name recursion, same-named delegation,
// type-parameter order and constructor-narrowing cases of 0.8.1) scored 89
// wrong labels (34.10%) on the v0.4.1 engine, 7 (2.68%) on v0.5.0, 29 of
// the 39 pairs added in 0.7.0 on v0.6.0, 22 of the 25 pairs added in 0.8.0
// on v0.7.0, and all 12 pairs added in 0.8.1 on v0.8.0. v0.8.1 classifies
// every labeled pair correctly, and this suite pins that: any mislabeled
// pair is a regression.
describe("refactoring accuracy benchmark", () => {
  it("classifies every labeled corpus pair correctly", async () => {
    const { summary } = await runBenchmark();

    // Guard against the corpus accidentally shrinking below its 0.8.1 size
    // — the gate is only meaningful while the corpus stays representative.
    // Keep this in step with the count in README.md and CHANGELOG.md.
    expect(summary.totalPairs).toBeGreaterThanOrEqual(337);

    // 100% accuracy: failures carry the offending pairs, so an empty-array
    // assertion names the regressed cases directly in the diff.
    expect(summary.failures).toEqual([]);

    // And every label measures something: a pair labeled under a name the
    // report does not use would count as "not reported" whatever the
    // engine did.
    expect(summary.unresolved).toEqual([]);
  }, 240_000);
});
