# @kongyo2/similarity-ts

[![npm version](https://img.shields.io/npm/v/@kongyo2/similarity-ts.svg)](https://www.npmjs.com/package/@kongyo2/similarity-ts)
[![npm downloads](https://img.shields.io/npm/dm/@kongyo2/similarity-ts.svg)](https://www.npmjs.com/package/@kongyo2/similarity-ts)
[![CI](https://github.com/kongyo2/similarity/actions/workflows/ci.yml/badge.svg)](https://github.com/kongyo2/similarity/actions/workflows/ci.yml)
[![node](https://img.shields.io/node/v/@kongyo2/similarity-ts.svg)](https://www.npmjs.com/package/@kongyo2/similarity-ts)
[![license](https://img.shields.io/npm/l/@kongyo2/similarity-ts.svg)](https://github.com/kongyo2/similarity/blob/main/LICENSE)
[![DeepWiki](https://img.shields.io/badge/DeepWiki-kongyo2%2Fsimilarity-blue.svg?logo=data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACwAAAAyCAYAAAAnWDnqAAAAAXNSR0IArs4c6QAAA05JREFUaEPtmUtyEzEQhtWTQyQLHNak2AB7ZnyXZMEjXMGeK/AIi+QuHrMnbChYY7MIh8g01fJoopFb0uhhEqqcbWTp06/uv1saEDv4O3n3dV60RfP947Mm9/SQc0ICFQgzfc4CYZoTPAswgSJCCUJUnAAoRHOAUOcATwbmVLWdGoH//PB8mnKqScAhsD0kYP3j/Yt5LPQe2KvcXmGvRHcDnpxfL2zOYJ1mFwrryWTz0advv1Ut4CJgf5uhDuDj5eUcAUoahrdY/56ebRWeraTjMt/00Sh3UDtjgHtQNHwcRGOC98BJEAEymycmYcWwOprTgcB6VZ5JK5TAJ+fXGLBm3FDAmn6oPPjR4rKCAoJCal2eAiQp2x0vxTPB3ALO2CRkwmDy5WohzBDwSEFKRwPbknEggCPB/imwrycgxX2NzoMCHhPkDwqYMr9tRcP5qNrMZHkVnOjRMWwLCcr8ohBVb1OMjxLwGCvjTikrsBOiA6fNyCrm8V1rP93iVPpwaE+gO0SsWmPiXB+jikdf6SizrT5qKasx5j8ABbHpFTx+vFXp9EnYQmLx02h1QTTrl6eDqxLnGjporxl3NL3agEvXdT0WmEost648sQOYAeJS9Q7bfUVoMGnjo4AZdUMQku50McDcMWcBPvr0SzbTAFDfvJqwLzgxwATnCgnp4wDl6Aa+Ax283gghmj+vj7feE2KBBRMW3FzOpLOADl0Isb5587h/U4gGvkt5v60Z1VLG8BhYjbzRwyQZemwAd6cCR5/XFWLYZRIMpX39AR0tjaGGiGzLVyhse5C9RKC6ai42ppWPKiBagOvaYk8lO7DajerabOZP46Lby5wKjw1HCRx7p9sVMOWGzb/vA1hwiWc6jm3MvQDTogQkiqIhJV0nBQBTU+3okKCFDy9WwferkHjtxib7t3xIUQtHxnIwtx4mpg26/HfwVNVDb4oI9RHmx5WGelRVlrtiw43zboCLaxv46AZeB3IlTkwouebTr1y2NjSpHz68WNFjHvupy3q8TFn3Hos2IAk4Ju5dCo8B3wP7VPr/FGaKiG+T+v+TQqIrOqMTL1VdWV1DdmcbO8KXBz6esmYWYKPwDL5b5FA1a0hwapHiom0r/cKaoqr+27/XcrS5UwSMbQAAAABJRU5ErkJggg==)](https://deepwiki.com/kongyo2/similarity)
![CodeRabbit Pull Request Reviews](https://img.shields.io/coderabbit/prs/github/kongyo2/similarity?utm_source=oss&utm_medium=github&utm_campaign=kongyo2%2Fsimilarity&labelColor=171717&color=FF570A&link=https%3A%2F%2Fcoderabbit.ai&label=CodeRabbit+Reviews)

TypeScript-only similarity analyzer

Private edition of [mizchi/similarity](https://github.com/mizchi/similarity).

## Quick Start

```bash
npx @kongyo2/similarity-ts .
```

## Refactor with AI

Use the following prompt with your AI coding assistant:

```text
Run `npx @kongyo2/similarity-ts .` to detect semantic code similarities.
Execute this command, analyze the duplicate code patterns, and create a refactoring plan.
```

For Claude Code, copy
[`.claude/skills/check-similarity-ts`](.claude/skills/check-similarity-ts/SKILL.md)
into your project's `.claude/skills/` directory. The skill runs the analyzer
on TypeScript files and turns its clusters and pairs into a refactoring plan.

Three or more copies of one declaration are reported once, as a **cluster**,
instead of as every pair among them: five identical `dispose()` methods are
one block listing five locations, not ten table rows. The JSON report keeps
every pair in `results`/`byMode` and adds the groups as `clusters`.

## What it detects

The analyzer compares functions, types, and classes structurally. Every
function is alpha-renamed before comparison — parameters, locals, inner
functions, and the declaration name all get positional canonical names — so
**consistent renames never hide a duplicate**, no matter how many
identifiers changed. Free identifiers (imported helpers, globals, property
names) keep their names, because *which* function you call is behavior.

On top of that, the comparison canonicalizes common style alternatives —
two snippets a reviewer would call "the same code written differently"
score as equal:

- arrow functions ⇔ function declarations ⇔ class methods ⇔ object-literal
  methods ⇔ arrow-valued class fields, expression bodies ⇔ block bodies
- `promise.then((v) => …)` ⇔ `const v = await promise; …`
- `items.forEach((item) => …)` ⇔ `for (const item of items) …` ⇔
  `for (let i = 0; i < items.length; i++) { const item = items[i]; … }`
- push-accumulator loops ⇔ `items.map(...)` / `items.filter(...)`
- `` `Hello ${name}` `` ⇔ `"Hello " + name`
- `total += n` ⇔ `total = total + n` ⇔ `i++` (in statement position)
- `return c ? a : b` ⇔ `if (c) { return a } else { return b }` ⇔
  `if (c) { return a } return b` (guard style)
- `if (!c) { A } else { B }` ⇔ `if (c) { B } else { A }`, `!==` guards ⇔
  flipped `===` guards, De Morgan (`!(a && b)` ⇔ `!a || !b`)
- `x ? x : y` ⇔ `x || y` · `x == null ? d : x` ⇔ `x ?? d` ·
  `x === null || x === undefined` ⇔ `x == null` · `void 0` ⇔ `undefined` ·
  `Boolean(x)` ⇔ `!!x`
- `const { a, b } = obj` ⇔ `const a = obj.a; const b = obj.b`, including
  single-use temporaries inlined either way
- `const x = c ? a : b` ⇔ `let x; if (c) { x = a } else { x = b }`
- jump-terminated `switch` ⇔ `if`/`else if` chains ⇔ guard ladders
- `Object.assign({}, a, b)` ⇔ `{ ...a, ...b }`
- `constructor(private readonly repo: Repo) {}` ⇔
  `constructor(repo: Repo) { this.repo = repo; }` (parameter properties are
  desugared the way the compiler emits them, after any `super()` call)
- `for (; cond; )` ⇔ `while (cond)`, braced ⇔ brace-less bodies,
  Yoda comparisons (`0 === n`) ⇔ natural order, `arr[arr.length - 1]` ⇔
  `arr.at(-1)`, reordered independent `const` declarations
- interfaces ⇔ structurally identical type aliases, reordered members,
  renamed-but-identically-typed property sets, `Array<T>` ⇔ `T[]`,
  `x?: T` ⇔ `x: T | undefined`, reordered unions, renamed generic params,
  `'a'` ⇔ `"a"` literal types
- classes: reordered members, renamed private fields, renamed class type
  parameters (`ItemList<T> extends Collection<T>` ⇔ `EntryList<U> extends
  Collection<U>`, also when they are declared in another order and used
  together, as in `T | U`), constructor parameter properties ⇔ explicit
  field + assignment (a parameter stored as `this.x = x as T` is still
  stored), and fully renamed method names when the canonical method bodies
  match

Every declaration form takes part: function declarations, arrow and
function expressions bound to a variable (also when parenthesized or
wrapped in `as`/`satisfies`; a named function expression keeps recursing
through either of its names), the methods and function-valued properties of
an object literal bound to a name or default-exported (`handlers.onSave`,
nested objects included; a named function expression there recurses
through its own name), `export default` classes and functions, class
methods and arrow-function class fields, constructors, and declarations
inside `namespace`/`module` blocks, which are reported under their
qualified name (`Legacy.Options`). Bodiless declarations — overload
signatures, `declare function`, `abstract` methods — are never reported:
there is no code behind them.

Rewrites that change behavior keep their distinct shapes on purpose:
swapped builtins (`.map` vs `.filter`, `Math.max` vs `Math.min`), a
different free callee (`sendEmail` vs `sendSms`; a method's key binds
nothing in its body, so `debug(…)` inside a `debug()` method calls the
outer `debug`), flipped operators (`*` vs `/`, `&&` vs `||`, `<` vs `<=`),
`??` vs `||`, `x == null` vs `x === null`,
optional chaining vs plain access, `for-of` vs `for-in`, an added `break`,
reordered statements that share data, `async` vs sync and generator vs
plain contracts (a sync twin is the same code only when it visibly returns
a promise: a `Promise<…>` return type, a returned `Promise.resolve(…)`, or
a chained `.then`), `then(onFulfilled, onRejected)`, fall-through `switch`
cases, `Object.assign` with a mutated target, string prepend vs append
folds (`trail = trail + seg` vs `trail = seg + trail`), boundary indexes
(`.at(-1)` vs `.at(0)`, `.slice(0, n)` vs `.slice(n)`), templates that
stringify adjacent values (`` `${a}${b}` `` vs numeric `a + b`), and on the
type side `Promise<ShopUser>` vs `Promise<ShopOrder>` payloads, `Map<K, V>`
vs `Map<V, K>` swaps, index signatures vs concrete members,
discriminated-union variants that differ in their `kind` literal or enum
member (signed numbers, bigints and string literals containing quotes
included), differing `extends` heritage (type arguments included:
`Box<string>` is not `Box<number>`), brand markers (`_fooBrand: any`),
`typeof`/type-predicate members naming different things, qualified generic
bases (`React.ComponentType<P>` vs `Preact.ComponentType<P>`), one- or
two-member shapes whose only shared trait is a primitive, and aliases of a
single atom (`type GLsizei = number`, `type SchemaVersion = "1"` in either
quote style), which pair only when they are also named alike — the same
alias copied into two files. On the class side member annotations compare
the way type mode compares them (literal unions, enum members, qualified
and generic types keep their identity; renamed class type parameters do
not count), a constructor that does real work counts as a member,
constructors storing the same parameters into different fields
(`this.x = y; this.y = x`) or `super(…)` slots differ, a static member is
not its instance namesake, an optional member is not a required one, a
different, qualified-vs-absent, or one-sided base class is a contract
difference, an `implements` clause the other side lacks is one more
difference, and a single shared field is not a duplicate class. Twins that
differ **only in data literals** (a table name, a status code, a locale
string) are reported — parameterizing them is the refactor.

## Accuracy

Accuracy is tracked by a labeled benchmark (`bench/cases.ts` plus the
extended corpora in `bench/cases/`) that mirrors the refactoring flow
above: **337 ground-truth pairs** across functions, types, and classes —
semantic duplicates a refactoring plan must see, and similarly-shaped
lookalikes it must not flag — evaluated at the default threshold. The
corpus covers whole-function renames, guard/negation/ternary spellings,
loop-form rewrites, destructuring, nullish sugar, literal-vs-behavior
twins, realistic cross-file copy-paste, and — since 0.7.0 — the lookalike
families found by auditing the engine's reports on a snapshot of the
TypeScript compiler repository: bodiless overload signatures,
discriminated-union variants (enum members, string, signed-number and
bigint literals), heritage (one-sided, differing, differently instantiated
or qualified bases), brand markers, tiny renamed shapes, `typeof`/predicate
members, constructor work, and declaration scopes — namespaces, default
exports, named function expressions — and, since 0.8.0, the families
found on the fallow, tsrs and TypeScript repositories: sync/async API
siblings, single-atom type aliases, qualified generic bases, class members
whose annotations differ, renamed class type parameters, constructor
wiring, static/instance and optional members, object-literal members and
wrapped initializers — and, since 0.8.1, string literals that contain
quotes or use single quotes, object members that recurse through their own
name, methods that delegate to a helper named like themselves, class type
parameters declared in another order, and constructors that narrow a
parameter before storing it.

| Engine | Corpus | Wrong labels | Error rate | Accuracy |
| --- | --- | ---: | ---: | ---: |
| v0.3.0 | 71 pairs | 15 / 71 | 21.13% | 78.87% |
| v0.4.1 | 71 pairs | 0 / 71 | 0.00% | 100.00% |
| v0.4.1 | 261 pairs | 89 / 261 | 34.10% | 65.90% |
| v0.5.0 | 261 pairs | 7 / 261 | 2.68% | 97.32% |
| v0.6.0 | 261 pairs | 0 / 261 | 0.00% | 100.00% |
| v0.6.0 | 300 pairs | 29 / 300 | 9.67% | 90.33% |
| v0.7.0 | 300 pairs | 0 / 300 | 0.00% | 100.00% |
| v0.7.0 | 325 pairs | 22 / 325 | 6.77% | 93.23% |
| v0.8.0 | 325 pairs | 0 / 325 | 0.00% | 100.00% |
| v0.8.0 | 337 pairs | 12 / 337 | 3.56% | 96.44% |
| **v0.8.1** | **337 pairs** | **0 / 337** | **0.00%** | **100.00%** |

v0.7.0 was audited against real code: run over the TypeScript compiler's
API package, its VS Code extension, and 1,482 conformance test files, the
v0.6.0 engine reported every AST-node interface pair that differs only in
its `kind` discriminant, every overload signature as a 1.000 duplicate of
the next, and thousands of single-field conformance classes as twins,
while never extracting function expressions, arrow-function class fields,
or anything declared inside a `namespace`. v0.7.0 fixes each family (type
pairs on the 108-file API package drop from 685 to 419, conformance class
pairs from 15,062 to 5,793, and the newly extracted declaration forms
surface their twins), adds the families to the corpus, and keeps the
original 261 pairs at 100% with unchanged margins. Reports are also
deterministic now — type mode used to compare declarations in hash-map
order, which moved borderline scores between runs.

v0.8.0 was audited against three more codebases: the fallow analyzer (its
VS Code extension, tools, npm packages and visualization frontend), tsrs
(its npm package, tools and playground, and the 116 standard-library
declaration files it bundles), and the current TypeScript repository,
whose API package now ships sync and async siblings. The v0.7.0 engine
paired the async API's methods with their sync namesakes, reported every
two aliases of a single atom with equal bodies (the WebGL/WebGPU `number`
labels of the DOM library, the `"1"` schema versions of a generated output
contract), rendered class member annotations onto `any`/`unknown`, and
never saw the members of an object literal — dispatch tables, handler
objects, test doubles. v0.8.0 fixes each family (type pairs on the tsrs
standard library drop from 4,585 to 2,383, on fallow's VS Code extension
from 855 to 757), adds the families to the corpus, and keeps the original
300 pairs at 100%. Overlap mode, part of every default run, used to give a
different report on each run of the same input and took over five minutes
on the 59-file TypeScript API package; it is deterministic now, and a
default run there takes 56 s instead of 314 s.

Run it yourself with `npm run bench:accuracy`; the suite in
`tests/accuracy-benchmark.test.ts` fails CI if **any** labeled pair is
mislabeled.

## CLI options

| Option | Default | Description |
| --- | --- | --- |
| `--modes <list>` | `functions,types,classes,overlap` | Comma-separated analysis modes |
| `-t, --threshold <number>` | `0.8` | Similarity threshold (0–1) |
| `--min-lines <number>` | `3` | Minimum function line count |
| `--min-tokens <number>` | — | Minimum function size in AST nodes (replaces the line gate; ~50 recommended for noisy code) |
| `--no-size-penalty` | off | Disable the short-function score penalty |
| `--same-file-only` / `--cross-file-only` | off | Restrict pair scope |
| `--extensions <list>` | `ts,tsx,mts,cts` | File extensions to scan |
| `--exclude <pattern>` | — | Exclude glob (repeatable) |
| `--types-only <kind>` | `all` | Restrict type mode to `interface` or `type` |
| `--no-allow-cross-kind` | off | Disable interface ⇔ type alias matching |
| `--type-literals` | off | Include anonymous type literals in type mode |
| `--format <pretty\|json>` | `pretty` | Output format |
| `--output <path>` | — | Write the report to a file (parent directories are created) |
| `--fail-on-warnings` | off | Non-zero exit on analyzer warnings |
| `--fail-on-duplicates` | off | Non-zero exit when any pair is reported (CI gate) |

Annotate a declaration with a `// similarity-ignore` comment on the
preceding line to exclude it from the report.

## References

The function comparison implements **TSED** (Tree Similarity of Edit
Distance): both fragments are parsed to ASTs, an APTED-style tree edit
distance δ with configurable per-operation weights (rename 0.3, delete
1.0, insert 1.0 — tuned against the labeled corpus) is computed, and the
score is normalized as `TSED = max(1 − δ / MaxNodes(G1, G2), 0)`. The
alpha-renaming, refactor canonicalization, behavioral-atom guard, and
size-penalty layers documented above are this project's additions on top
of that metric; the operation-weight sensitivity they exploit is the
paper's RQ3 finding that TSED's penalty weights are influential and
language-dependent. Following that finding, the weights are re-validated
against the labeled corpus on every accuracy release: for 0.8.1 every
setting in rename 0.2–0.5 × delete/insert 0.8–1.0 classifies all 337 pairs
correctly (the closest call is XF-N36 at rename 0.2, 0.797 against the 0.8
threshold, the same as on v0.7.0 and v0.8.0), so the calibrated defaults
stand.

- Yewei Song, Cedric Lothritz, Daniel Tang, Tegawendé F. Bissyandé, and
  Jacques Klein. 2024. *Revisiting Code Similarity Evaluation with
  Abstract Syntax Tree Edit Distance.* In Proceedings of the 62nd Annual
  Meeting of the Association for Computational Linguistics (Volume 2:
  Short Papers). [ACL Anthology 2024.acl-short.3](https://aclanthology.org/2024.acl-short.3/)
  · [arXiv:2404.08817](https://arxiv.org/abs/2404.08817)
- Yewei Song, Saad Ezzini, Xunzhu Tang, Cedric Lothritz, Jacques Klein,
  Tegawendé Bissyandé, Andrey Boytsov, Ulrick Ble, and Anne Goujon. 2023.
  *Enhancing Text-to-SQL Translation for Financial System Design.* The
  paper that introduced the original TSED metric.
  [arXiv:2312.14725](https://arxiv.org/abs/2312.14725)
- Mateusz Pawlik and Nikolaus Augsten. 2015. *Efficient Computation of
  the Tree Edit Distance.* ACM Transactions on Database Systems 40(1) —
  the APTED algorithm family used for δ. See also Pawlik and Augsten
  2016, *Tree edit distance: Robust and memory-efficient,* Information
  Systems 56.
