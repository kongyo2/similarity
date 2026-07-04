# Changelog

## 0.5.0 — 2026-07-04

The detection engine was rebuilt around scope-aware alpha-renaming and a
much deeper refactor-equivalence canonicalizer, validated against a
ground-truth corpus grown from 71 to 261 labeled pairs. On that corpus the
v0.4.1 engine mislabels 89 pairs (34.10% error rate); v0.5.0 mislabels 7
(2.68%) — a **12.7x error-rate reduction** — while keeping the original
71-pair corpus at 100%. End-to-end analysis is also ~1.6x faster.

### Detection accuracy

- **Alpha-renaming**: every comparison fragment now runs through
  `oxc_semantic`; parameters, locals, inner functions/classes, and the
  declaration name get positional canonical names (`§0`, `§1`, …), so
  consistently-renamed duplicates compare as *exactly equal* trees. Free
  identifiers (imports, globals, property names) keep their names.
  Ordinals are assigned by first occurrence in the final canonical tree,
  so structural rewrites cannot skew them.
- **New canonicalizations** (behavior-preserving spellings converge):
  guard style (`if/else return` ⇔ early return ⇔ ternary return, flat
  guard ladders ⇔ jump-terminated switches), negation swaps (`if (!c)`
  branch flip, `!==`⇔`===` guard flips, De Morgan, `!(a === b)` ⇔
  `a !== b`), logic sugar (`x ? x : y` ⇔ `x || y`, nullish ternaries ⇔
  `??`, strict null pairs ⇔ `== null`, `void 0` ⇔ `undefined`,
  `Boolean(x)` ⇔ `!!x`, test-position `!!` stripping), object
  destructuring ⇔ member declarations, single-use pure-const inlining,
  index loops ⇔ `for-of`, push-accumulator loops ⇔ `.map`/`.filter`,
  arrow expression bodies ⇔ block bodies, `arr[arr.length - 1]` ⇔
  `arr.at(-1)`, Yoda comparisons, `else { if … }` ⇔ `else if`,
  order-canonicalized independent `const` runs, temp-return elimination.
- **Behavioral-atom guard**: near-identical twins that differ in
  behavior-carrying atoms — a swapped builtin (`.map` vs `.filter`), a
  different free callee, a flipped operator, a changed loop kind or an
  added `break` — are capped below the reporting threshold. One-sided
  additions (copy-paste-plus-extra-logging) stay reportable, and pairs
  differing only in data literals are reported as parameterizable
  duplicates. Statement reorderings that share data also score down.
- **Types**: property matching is now rename-tolerant (two-phase: exact
  names, then identical-type pairing), optionality participates in match
  quality, `x?: T` unifies with `x: T | undefined`, generic type
  arguments are extracted (previously `Array<string>` and `Array<number>`
  both extracted as `Array`), index/tuple/keyof/nested-object types render
  structurally instead of collapsing to `unknown`/`object`, generic type
  parameters substitute positionally, unions sort at any nesting depth,
  and function types compare parameter-name-insensitively with
  return-type-weighted scoring. Naming weight rebalanced 0.4 → 0.15.
- **Classes**: methods carry a canonical body fingerprint (locals
  alpha-renamed, `this.<field>` names position-normalized), so renamed
  fields/locals don't hide equal bodies and equal signatures don't hide
  different bodies. Static/instance and accessor/method mismatches score
  down; arrow-valued class fields extract as methods; constructor
  parameter properties extract as fields; member-less classes compare by
  heritage instead of a flat 1.0.

### Bug fixes

- The tree-edit-distance "replace subtree" branch charged
  `min(delete, insert)` instead of `delete + insert`, letting a large
  subtree "become" a completely different one at the cost of only the
  smaller side. This both inflated similarity of size-mismatched pairs
  and made the size-ratio prefilter and threshold-cutoff bounds unsound
  (the threshold path could silently drop or mis-score true duplicates).
- Cross-file scans applied the nested-function containment check across
  files, spuriously dropping pairs whose line/byte ranges happened to
  nest between unrelated coordinate spaces.
- Overlap mode reported every overlap with the enclosing functions' whole
  line ranges, which made the dedup pass collapse all overlaps for a pair
  into one; overlaps now carry real source ranges (statement spans are
  recorded during conversion) and multiple distinct shared regions
  survive. Overlap results and pair ordering are now deterministic
  (HashMap iteration order no longer leaks into reports).
- Class-name similarity sized its Levenshtein matrix in bytes while
  filling it in chars, scoring any two multibyte names as identical
  (distance 0 → similarity 1.0).
- Property names are no longer lowercased into a map (case-differing
  properties silently merged); boxed-primitive normalization no longer
  corrupts identifiers by substring replacement (`PhoneNumber` →
  `Phonenumber`); union sorting no longer depends on exact `" | "`
  spacing; a byte/char mix in function-type normalization that could
  panic on multibyte parameter types was removed.
- `analyzedFiles` no longer lists files that failed to parse;
  `skippedFiles` reports them (with per-file warnings) instead of always
  being empty.
- The pretty formatter no longer crashes when a requested mode is missing
  from the report; CLI numeric options reject `""`, hex, and exponent
  forms instead of coercing them; repeated `--modes` values render once;
  `--output` creates missing parent directories; non-`Error` throws print
  their value instead of `undefined`; the WASM report shape is validated
  at the boundary with a clear out-of-sync message.

### Performance (~1.6x end-to-end)

- Functions mode runs one unified scan: every function is extracted and
  parsed exactly once (previously the cross-file pass and the per-file
  passes each re-extracted and re-parsed everything, and a further
  per-function parse computed a node count that the WASM path never
  used).
- The edit-distance memo is a dense flat-array over pre-flattened trees
  (previously `HashMap` probing per node pair plus a fresh child-cost
  `HashMap` and discarded backtracking work per internal pair).
- Line-number lookups use a per-file offset table (previously an O(file)
  rescan per lookup); the cross-file scan no longer clones the entire
  file text once per function.
- Overlap mode indexes each file's functions exactly once (previously
  every file *pair* re-extracted, re-parsed, and re-fingerprinted both
  files, and each target function was re-indexed per source function).

### CLI

- New: `--min-tokens <n>` (AST-node size gate measured on the same
  normalized tree the comparison scores) and `--fail-on-duplicates`
  (non-zero exit for CI gates).
- `minTokens` is available on the library API as well.

### Removed

- ~7,000 lines of modules unreachable from the analyzer: the tree-sitter
  multi-language scaffolding, the experimental structure-comparison
  framework, the unused fingerprint/fast-similarity paths, an
  `enhanced_similarity` module whose label matching never fired for
  TypeScript trees, and the CLI/config helpers of the original upstream
  binary. `fastest-levenshtein` was dropped from dependencies and
  `typescript` moved to devDependencies.

### Benchmark corpus

- `bench/cases/` adds 190 labeled pairs (functions positives/negatives,
  types, classes, realistic cross-file scenarios) on top of the original
  71; the regression gate now pins both the 10x error-rate budget against
  the v0.4.1 baseline and 100% accuracy on the original core corpus.
- Labeling follows two clarified principles: twins differing only in data
  literals are duplicates (parameterizing them is the refactor), and
  skeleton-only matches with entirely different callees are not.

## 0.4.1

- Fixed two canonicalization false equivalences from 0.4.0.

## 0.4.0

- 15x accuracy gain for refactor-mode TS similarity via the semantic
  canonicalization layer (template literals, compound assignments,
  ternary lowering, forEach⇔for-of, then⇔await, switch⇔if chains,
  Object.assign⇔spread).

## 0.3.0 and earlier

- TypeScript port of the mizchi/similarity analyzer with WASM engine.
