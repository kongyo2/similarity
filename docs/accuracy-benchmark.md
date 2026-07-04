
> @kongyo2/similarity-ts@0.5.0 bench:accuracy
> tsx scripts/accuracy-benchmark.ts

# Refactoring Accuracy Benchmark

Generated at: 2026-07-04T09:38:59.115Z

Settings: threshold 0.8 (CLI default), minLines 3 (CLI default)

## Summary

| Metric | Value |
| --- | ---: |
| Labeled pairs | 261 |
| True positives | 171 |
| False negatives | 1 |
| True negatives | 83 |
| False positives | 6 |
| Accuracy | 97.32% |
| Error rate | 2.68% |

## By mode

| Mode | Pairs | Errors | Accuracy |
| --- | ---: | ---: | ---: |
| functions | 186 | 3 | 98.39% |
| types | 52 | 3 | 94.23% |
| classes | 23 | 1 | 95.65% |

## Failures

| Case | Mode | Pair | Expected | Observed sim |
| --- | --- | --- | --- | ---: |
| XF-N15 | functions | newestSnapshot ↔ oldestSnapshot | distinct | 0.887 |
| XF-N17 | functions | takeTopJobs ↔ dropTopJobs | distinct | 0.948 |
| XF-N40 | functions | buildTrailForward ↔ buildTrailReversed | distinct | 0.884 |
| XT-N07 | types | UserGateway ↔ OrderGateway | distinct | 0.821 |
| XT-N08 | types | PriceLookup ↔ SkuLookup | distinct | 0.825 |
| XT-N12 | types | FeatureFlags ↔ FeatureToggle | distinct | 0.834 |
| XC-P04 | classes | SampleWindow ↔ ReadingWindow | duplicate | 0.537 |
