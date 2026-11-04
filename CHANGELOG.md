# Changelog

## 0.8.0 — 2026-11-04

Accuracy work driven by three more real codebases and by the review
findings left open on the v0.7.0 pull request (kongyo2/similarity#30). The
engine was run over the fallow analyzer (its VS Code extension, tools, npm
packages and visualization frontend), tsrs (its npm package, tools and
playground, and the 116 standard-library declaration files it bundles),
and the current TypeScript repository (the API package, which now ships
sync and async siblings, the VS Code extension, the generator scripts, and
1,381 conformance tests), and every class of reported pair was audited. The
mislabel families were added to the labeled corpus as 25 new ground-truth
pairs (corpus now **325 pairs**); the v0.7.0 engine gets 22 of them wrong
(6.77% error rate on the extended corpus), v0.8.0 classifies all 325
correctly and keeps the original 300 at 100%.

### Detection accuracy

- **Functions — call contracts** (precision): an `async` function returns a
  promise and a generator an iterator, so a plain twin with the same body is
  a different contract. Such pairs cap at 0.72 — unless the sync side chains
  the promise the async side awaits (`return fetch(u).then((r) => …)` vs
  `const r = await fetch(u); …`, which the canonicalizer already lowers to
  the same `await`). The sync and async API packages' `getSymbol` twins
  scored 0.970, and static async methods and accessors, whose class wrapper
  never carried the `async` marker, scored 1.000 against their sync
  versions. (XF-N42, XF-N43, XF-N44)
- **Functions — named function expressions** (recall): `const factorial =
  function recur(n) {…}` may recurse through either name. The body is
  compared under the function's own binding, and the variable binding is
  now an alias of it, so `factorial(n - 1)` resolves to the function too
  instead of staying a free identifier that differed between copies (the
  pair scored 0.75). (XF-P49)
- **Functions — wrapped initializers** (recall): parentheses and
  TypeScript-only wrappers around a function initializer —
  `const parse = (function (line) {…});`, `(… => …) satisfies Splitter`,
  `as Handler`, `<Handler>(…)`, a trailing `!` — no longer hide the
  function from the scan; the same holds for arrow-function class fields
  and `export default`. (XF-P50)
- **Functions — object-literal members** (recall): the methods, getters
  and setters, and function-valued properties of an object literal bound to
  a name or default-exported are extracted, nested objects included, and
  reported under the binding (`handlers.onSave`, `pipeline.hooks.transform`,
  `table[SyntaxKind.Block]`). Dispatch tables, handler and plugin objects,
  and test doubles are written this way; none of their functions were
  visible before. An object passed as an argument or returned from a
  function stays part of the function that builds it. (XF-P51, XF-P52)
- **Types — single-atom aliases** (precision): an alias whose body is one
  primitive, literal or bare reference (`type GLsizei = number`,
  `type SchemaVersion = "1"`, `type FunctionBody = Block`) has no structure
  of its own to duplicate; it gives the atom a name. Two such aliases now
  pair only when they are also named alike (the same alias copied into two
  files, `UserId` vs `UserID`): the structural evidence is the name
  similarity cubed. Every two of the DOM library's WebGL/WebGPU `number`
  labels and every two of a generated contract's `"1"` schema versions used
  to be reported. (XT-N21, XT-N22, XT-P24)
- **Types — qualified generic references**: `React.ComponentType<Props>`
  and `Preact.ComponentType<Props>` are compared like bare generic
  references — nominal base, then arguments — instead of by edit distance
  of their spelling (0.93), and qualified generics normalize their
  arguments (`ns.Box<Array<T>>` ⇔ `ns.Box<T[]>`). (XT-N23)
- **Types — literal unions**: a union of string literals (`"a" | "b"`)
  starts and ends with a quote and was mistaken for a single literal, which
  made two different literal unions a "discriminant mismatch". Only a
  single literal is one now.
- **Classes — member annotations** (precision): class mode renders member,
  parameter and return types with type mode's renderer and normalization.
  Its own renderer collapsed every qualified reference (`SyntaxKind.Block`,
  `React.ComponentType<P>`) onto `unknown`, dropped the type arguments of
  nested references (`Promise<User>` rendered as `Promise`), and rendered
  literal, tuple, `keyof`, `readonly` and `typeof` types as `any`. Fields
  without an annotation are typed by a literal or `new` initializer
  (`count = 0` is a `number`, `cache = new Map<string, User>()` a
  `Map<string, User>`), signatures keep optional and rest parameters, and
  the rename tolerance for class-reference types no longer erases the
  segments of a qualified name. (XC-N12)
- **Classes — generic parameters** (recall): class type parameters are
  spelled positionally in member annotations and in `extends`/`implements`
  clauses, as type mode does for interfaces, so `ItemList<TItem> extends
  Collection<TItem>` and `EntryList<U> extends Collection<U>` are the same
  contract (the heritage mismatch used to cost 30%); a method's own type
  parameters are positional within the method. (XC-P15)
- **Classes — constructor wiring** (precision): where a constructor stores
  its parameters — fields assigned `this.x = p`, parameter properties,
  `super(…)` argument slots — is recorded and compared through the field
  pairing, so `this.x = y; this.y = x` is no longer the same constructor as
  `this.x = x; this.y = y` (the corpus pair scored 0.932), while parameter
  properties and explicit assignments still agree. A parameter stored into
  a different place on each side counts as two unmatched elements.
  (XC-N13, XC-P16)
- **Classes — static and optional members**: members were keyed by bare
  name, so a static member silently overwrote its instance namesake (a
  store with `static create()` beside `create(id)` had structural
  similarity 1.0 against one without it); both are kept now, and a
  same-named static/instance pair still matches at the existing surface
  discount. An optional member (`x?: T`, `f?(): T`) matched against a
  required one is discounted like an optionality difference in type mode
  (×0.5). (XC-N14, XC-N15)
- **Classes — ignore directives**: `// similarity-ignore` excludes a class
  from the report, as the README promised; the directive was recorded but
  never consulted in class mode.
- **Classes — auto-accessors**: `accessor count = 0` fields are members.

### Reports

- **Duplicate clusters**, ported from mizchi/similarity#27: three or more
  declarations connected by reported pairs of one mode are listed once, as a
  cluster with its members, pair count and similarity range, before the
  remaining standalone pairs. The JSON report keeps every pair in
  `results`/`byMode` and adds `clusters` and `stats.clusterCount`. On the
  TypeScript API package, 1,437 function pairs are 58 clusters and 168
  standalone pairs.

### Overlap mode

- **Deterministic reports**: the sliding windows were built from subtrees
  collected out of a hash map and sorted by start line only, so subtrees
  starting on the same line came in a different order on every run — the
  same input gave a different set of overlaps each time (1,416 vs 1,423 on
  one fallow directory with the v0.7.0 engine). The windows follow the
  fingerprints' pre-order now.
- **Speed**: the windows and size buckets of a function are prepared once
  per function instead of once per compared pair (output identical). On
  the 59-file TypeScript API package, overlap mode drops from 294 s to 26 s,
  and a default run (all four modes) from 314 s to 56 s even though more
  functions are extracted.

### Agent skill

- `.claude/skills/check-similarity-ts`, adapted from mizchi/similarity's
  per-language skills: runs the analyzer on TypeScript files and turns its
  clusters and pairs into a refactoring plan. Copy it into a project's
  `.claude/skills/` directory to use it there.

### Weights

- The APTED operation weights (rename 0.3, delete 1.0, insert 1.0) were
  re-validated against the extended corpus: every setting in rename
  0.2–0.5 × delete/insert 0.8–1.0 keeps 325/325. The closest call under any
  setting is XF-N36 at rename 0.2 (0.797 against the 0.8 threshold), the
  same as on v0.7.0, so the calibrated defaults stay.

### Real-world effect

Pairs reported at the default threshold on the audited snapshots:

| Target | Mode | v0.7.0 | v0.8.0 |
| --- | --- | ---: | ---: |
| TypeScript `packages/typescript/src` (59 files) | functions | 1,103 | 1,437 |
| TypeScript `packages/typescript/src` | types | 540 | 533 |
| tsrs npm package, tools, playground (217 files) | functions | 1,126 | 1,460 |
| tsrs npm package, tools, playground | types | 518 | 510 |
| tsrs standard-library declarations (116 files) | types | 4,585 | 2,383 |
| fallow VS Code extension (122 files) | types | 855 | 757 |
| fallow tools, npm packages, frontend (77 files) | types | 938 | 840 |
| TypeScript conformance tests (1,381 files) | classes | 12,371 | 12,058 |
| TypeScript conformance tests | types | 8,473 | 8,382 |

The added function pairs are the identical-body entries of the generated
`yieldEachChildTable` dispatch table (339 on each package, newly visible as
object-literal members); the removed ones are the sync/async API siblings.
The removed type pairs are single-atom aliases (`GLsizei`/`GPUSize32`,
`CodeActionTag = 1`/`SymbolTag = 1`, `DocumentUri = string`/`URI = string`).
In the conformance classes, 644 pairs are gone (static members, optional
members, member annotations and generic signatures now tell them apart)
and 331 single-member pairs of same-named classes appear, because an
initialized field (`x = 1`) now has the type its annotated twin declares.

### Accuracy

| Engine | Corpus | Wrong labels | Error rate | Accuracy |
| --- | --- | ---: | ---: | ---: |
| v0.7.0 | 325 pairs | 22 / 325 | 6.77% | 93.23% |
| v0.8.0 | 325 pairs | 0 / 325 | 0.00% | 100.00% |

## 0.7.0 — 2026-09-01

Accuracy work driven by a real codebase. The engine was run over a snapshot
of the TypeScript compiler repository — the `packages/typescript/src` API
package, the VS Code extension, the AST/protocol generator scripts, and
1,482 conformance test files — and every class of reported pair was
audited. The mislabel families found there were added to the labeled corpus
as 39 new ground-truth pairs (corpus now **300 pairs**); the v0.6.0 engine
gets 29 of them wrong (9.67% error rate on the extended corpus), v0.7.0
classifies all 300 correctly and keeps the original 261 at 100% with
unchanged margins.

### Detection accuracy

- **Functions — bodiless declarations** (precision): overload signatures,
  `declare function …;` and `abstract` methods have no body and are no
  longer extracted. Their type-less parameter lists used to compare as
  1.000 duplicates of each other (the `visitNode` overloads in the
  compiler's `visitor.generated.ts`). (XF-N41)
- **Functions — declaration scopes** (recall): function expressions bound
  to a variable (`const f = function () {…}`), functions declared inside
  `namespace`/`module` blocks, arrow-function class fields
  (`handle = (event) => {…}`), and `export default` classes, functions and
  arrows are now extracted, so their twins can be found at all.
  (XF-P43, XF-P44, XF-P45, XF-P48)
- **Functions — named function expressions** (recall): `const factorial =
  function recur(n) {…}` is compared under its own binding, so
  `recur(n - 1)` keeps resolving to the function itself instead of becoming
  a free identifier that differed between otherwise identical copies.
  (XF-P47)
- **Functions — constructor parameter properties** (recall): the canonical
  tree desugars `constructor(private readonly repo: Repo) {}` into the
  `this.repo = repo;` assignments TypeScript emits — right after the
  top-level `super(…)` call when there is one, even when statements precede
  it — so the shorthand and the explicit spelling compare as equal.
  (XF-P46, XC-P14)
- **Functions — member names**: string-literal, numeric and computed method
  keys are reported by their source (`alpha`, `0`, `[Symbol.iterator]`)
  instead of `anonymous`.
- **Types — nominal atoms**: qualified references (`SyntaxKind.Block`) and
  literal types (`"click"`, `1`, `-1`, `1n`, `true`) compare nominally
  instead of by edit distance of their spelling
  (`SyntaxKind.NonNullExpression` vs `SyntaxKind.VoidExpression` scored
  0.75 as *types*, `-1` vs `-2` 0.5), and a same-named member typed with
  differing literals or members of the same enum is a discriminant
  mismatch that halves the structural evidence: the variants of a
  discriminated union (`ForInStatement` vs `ForOfStatement`,
  `MouseEnterEvent` vs `MouseLeaveEvent`) are distinct types however many
  members they share. (XT-N13, XT-N19, XT-N20)
- **Types — heritage**: `extends` clauses are part of the contract,
  rendered like property annotations — qualified bases and type arguments
  included, generic parameters positional — so `Box<string>` and
  `Box<number>` are different bases while `A<T> extends Box<T>` and
  `B<U> extends Box<U>` are the same one. A differing or one-sided clause
  discounts structural similarity (×0.7); the same clauses in any order
  are unaffected. (XT-N14, XT-N18, XT-P21)
- **Types — rename evidence**: the rename-tolerant property phase needs a
  skeleton to stand on. One-member types never pair by rename, two-member
  shapes earn half credit, three or more full credit (the existing
  XT-P04/T-N01 positives are unchanged), and members typed
  `any`/`unknown`/`never` — the `_fooBrand: any` marker idiom — never pair
  by rename. (XT-N15, XT-N16)
- **Types — exotic type nodes**: `typeof x` queries, type predicates,
  conditional, mapped and template-literal types and every other node
  without a structural renderer keep their whitespace-normalized source
  spelling instead of collapsing onto one `unknown` token that made them
  all compare equal; `typeof`/predicate members compare by what they name.
  (XT-N17, XT-P22)
- **Types and classes — namespaces**: interfaces, aliases and classes
  declared inside `namespace`/`module` blocks (including
  `declare namespace` in declaration files) are extracted and reported
  under their qualified name (`Legacy.Options`); the comparators score on
  the bare name, so a shared namespace prefix is not lexical agreement and
  a qualifier on one side does not hide a match. `export default interface`
  is extracted too. (XT-P23, XC-P12)
- **Classes — constructors**: a constructor that does real work is a
  member. Its signature and canonical body (parameter properties
  desugared) take part in the comparison, so same-fields-same-methods
  classes whose constructors differ (one schedules a refill timer, the
  other stores a limit) no longer compare as identical. Wiring-only
  constructors (`super(…)` plus `this.x = x` storing the constructor's own
  parameters, or the empty body of a parameter-property constructor) add
  nothing the field list does not already say and stay out, so a class
  that happens to have one is not "missing" it on the other side; storing
  anything else (`this.registry = globalRegistry`) is work.
  (XC-N07, XC-P11, XC-P13)
- **Classes — heritage and tiny shapes**: the base class is recorded as
  written (qualified bases such as `React.Component`, mixin calls, type
  arguments) and compared by value, so a different, one-sided, or
  qualified-vs-absent `extends` discounts structural similarity (×0.7),
  and an `implements` clause one side lacks is one edit's worth of
  difference (the same clauses in any order are neutral: sharing an
  interface is not evidence of duplication). A pair with a single member
  each is capped like the member-less case (0.85 structural), so
  `class Marker { id = 0 }` vs `class Slot { id = 0 }` needs naming
  agreement to pass. (XC-N08, XC-N09, XC-N10, XC-N11)
- **Classes — member keys**: `#private` members and computed keys
  (`[Symbol.iterator]`) are kept as members instead of being dropped,
  which left such classes looking member-less.

### Determinism

- Type mode compared declarations in `HashMap` iteration order, so which
  side of a pair drove the greedy rename-tolerant matching changed from
  run to run (one corpus pair scored 0.830 on one run and 0.823 on the
  next). Declarations are now compared in file/line order; the same input
  always yields the same report.

### Weights

- The APTED operation weights (rename 0.3, delete 1.0, insert 1.0) were
  re-validated against the extended corpus following the TSED paper's RQ3
  finding that they are influential: every setting in rename 0.2–0.5 ×
  delete/insert 0.8–1.0 keeps 300/300 and moves the tightest margins by
  less than 0.03, so the calibrated defaults stay.

### Real-world effect

Pairs reported at the default threshold on the TypeScript compiler
repository snapshot the release was audited against:

| Target | Mode | v0.6.0 | v0.7.0 |
| --- | --- | ---: | ---: |
| `packages/typescript/src` (108 files) | types | 685 | 419 |
| `packages/typescript/src` | functions | 926 | 925 |
| VS Code extension + generator scripts (27 files) | types | 16 | 5 |
| conformance tests (1,482 files) | classes | 15,062 | 5,793 |
| conformance tests | types | 823 | 945 |

The removed type pairs are the AST-node and protocol-parameter interfaces
that differ only in their `kind` discriminant, their heritage, or a single
renamed member; the removed conformance class pairs are single-field
lookalikes and same-member classes under different (or differently
instantiated) bases; the added conformance pairs are namespace-scoped
declarations that were previously invisible. Scan time on the 108-file
package moved from 10.5 s to 11.8 s.

### Accuracy

| Engine | Corpus | Wrong labels | Error rate | Accuracy |
| --- | --- | ---: | ---: | ---: |
| v0.6.0 | 300 pairs | 29 / 300 | 9.67% | 90.33% |
| v0.7.0 | 300 pairs | 0 / 300 | 0.00% | 100.00% |

## 0.6.0 — 2026-07-09

Closes out the accuracy program started in 0.5.0: the seven remaining
mislabeled pairs (six false positives, one false negative) are fixed and
the 261-pair labeled corpus now scores **100.00%** at the default
threshold, with the original 71-pair core corpus still at 100%. The CI
gate pins zero mislabels going forward. A README References section now
documents the TSED papers (Song et al., ACL 2024) the engine's metric is
built on.

### Detection accuracy

- **Boundary-index atoms** (functions): numeric index literals and call
  arity on the positional/slicing builtins (`at`, `slice`, `splice`,
  `charAt`, `charCodeAt`, `codePointAt`, `substring`, `substr`) are now
  behavior-carrying atoms. `.at(-1)` vs `.at(0)` reads a different
  element, and `.slice(0, n)` keeps exactly the head that `.slice(n)`
  drops — such twins cap below the reporting threshold instead of scoring
  ~0.89–0.95. Data literals elsewhere (a table name, a status code) stay
  parameterizable duplicates. (fixes XF-N15, XF-N17)
- **Fold-direction atoms** (functions): an assignment that rebuilds its
  own target from a `+` chain now marks the accumulator's position in the
  chain (head/mid/tail). String `+` is not commutative, so
  `trail = trail + seg + "/"` (append) vs `trail = seg + "/" + trail`
  (prepend) stay distinct, while same-direction folds — including the
  `+=` spelling the canonicalizer already contracts onto — keep comparing
  as duplicates. (fixes XF-N40)
- **Types**: array types compare element-wise, so `ShopUser[]` vs
  `ShopOrder[]` is the nominal payload contrast rather than an
  edit-distance near-match of the bracketed spellings, and array vs
  non-array is a shape mismatch outright; a permutation of the same
  generic arguments (`Map<string, number>` vs `Map<number, string>`)
  scores as the key/value swap it is; index signatures never pair with
  concrete properties in the rename-tolerant phase (`[flag: string]:
  boolean` admits every string key, `enabled: boolean` exactly one).
  (fixes XT-N07, XT-N08, XT-N12)
- **Classes**: the fuzzy method matcher now consults the canonical body
  fingerprints introduced in 0.5.0 — a fully-renamed twin (class name,
  fields, AND method names all changed) whose method bodies parse to
  identical canonical trees matches through the rename instead of being
  dropped by the name-similarity term. Naming weight rebalanced
  0.3 → 0.15 (structural 0.7 → 0.85), mirroring the type comparator's
  0.5.0 rebalance and for the same reason: the agreement factors inside
  the structural score already discount same-name/different-body
  lookalikes. (fixes XC-P04)

### Scope guards on the new detectors (review hardening)

- The fold-direction atoms only fire when a chain operand is a string
  LITERAL — the one case where the concatenation (and hence its
  direction) is provable from syntax. Commutative numeric folds
  (`sum += n` vs `sum = n + sum`) are never direction-capped.
- Fold-direction targets may be member slots (`this.trail`,
  `state.trail`), compared structurally, not just local variables.
- `splice` boundary atoms cover only `start`/`deleteCount` (positions 0
  and 1); arguments from position 2 onward are inserted values, so twins
  differing there stay parameterizable data-literal duplicates.
- Boundary and fold atoms carry occurrence COUNTS: with two `.at()`
  reads (or two accumulators), changing just one of them is a
  substitution even though the unchanged occurrence keeps the shared
  atom alive on both sides — set semantics would have misread that as a
  one-sided extension.
- The zero-argument form of the boundary members (except `splice`)
  normalizes to an explicit leading `0`: `xs.slice()` ⇔ `xs.slice(0)`
  and `s.charAt()` ⇔ `s.charAt(0)` are the same operation, so spelling
  the default must not trip the cap. `splice()` (removes nothing) vs
  `splice(0)` (removes everything) stays literal.
- `substring` records its index literals position-insensitively because
  JavaScript reorders the arguments after clamping: `s.substring(0, n)`
  ⇔ `s.substring(n, 0)`; negatives clamp to 0 first, so
  `s.substring(-1, n)` ⇔ `s.substring(0, n)` too.
- Explicit trailing `undefined` arguments trim before arity is recorded
  (`xs.slice(0, undefined)` ⇔ `xs.slice(0)`), except on `splice`, where
  `undefined` coerces deleteCount to 0 and means something different.
- Bracket-notation property calls (`xs["slice"](0)`) collect the same
  boundary atoms as their dot-notation spelling.
- A local declared with a string-literal initializer (`let trail = ""`)
  proves its folds are string concatenation even when the `+` chain
  itself carries no literal, so `trail = trail + seg` vs
  `trail = seg + trail` stays distinct under that declaration.
- Arguments beyond a boundary builtin's meaningful parameter count are
  ignored, matching JavaScript (`xs.at(0, traceId)` ⇔ `xs.at(0)`).
- Class property matching discounts `static` vs instance mismatches
  (×0.4, the same treatment the method pass already applied): a static
  field and an instance field of the same name and type live on
  different runtime surfaces.
- The class fingerprint boost requires signature agreement (exact or
  after fuzzy name-stripping): the fingerprint tree carries no
  TypeScript type annotations, so same-body methods over different
  parameter types must not match through it.
- Index signatures never enter the rename-tolerant property phase at
  all: `[index: string]` and `[index: number]` are different key-domain
  contracts even when their value types agree.

### Accuracy

| Engine | Corpus | Wrong labels | Error rate | Accuracy |
| --- | --- | ---: | ---: | ---: |
| v0.5.0 | 261 pairs | 7 / 261 | 2.68% | 97.32% |
| v0.6.0 | 261 pairs | 0 / 261 | 0.00% | 100.00% |

### CI

- `tests/accuracy-benchmark.test.ts` now pins 100% corpus accuracy: any
  mislabeled pair fails the suite (previously the budget was one tenth of
  the v0.4.1 error-rate baseline, ~8 pairs).

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
