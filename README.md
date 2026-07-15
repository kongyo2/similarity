# @kongyo2/similarity-ts

[![npm version](https://img.shields.io/npm/v/@kongyo2/similarity-ts.svg)](https://www.npmjs.com/package/@kongyo2/similarity-ts)
[![npm downloads](https://img.shields.io/npm/dm/@kongyo2/similarity-ts.svg)](https://www.npmjs.com/package/@kongyo2/similarity-ts)
[![CI](https://github.com/kongyo2/similarity/actions/workflows/ci.yml/badge.svg)](https://github.com/kongyo2/similarity/actions/workflows/ci.yml)
[![node](https://img.shields.io/node/v/@kongyo2/similarity-ts.svg)](https://www.npmjs.com/package/@kongyo2/similarity-ts)
[![license](https://img.shields.io/npm/l/@kongyo2/similarity-ts.svg)](https://github.com/kongyo2/similarity/blob/main/LICENSE)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/kongyo2/similarity)
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

- arrow functions ⇔ function declarations ⇔ class methods ⇔ arrow-valued
  class fields, expression bodies ⇔ block bodies
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
- `for (; cond; )` ⇔ `while (cond)`, braced ⇔ brace-less bodies,
  Yoda comparisons (`0 === n`) ⇔ natural order, `arr[arr.length - 1]` ⇔
  `arr.at(-1)`, reordered independent `const` declarations
- interfaces ⇔ structurally identical type aliases, reordered members,
  renamed-but-identically-typed property sets, `Array<T>` ⇔ `T[]`,
  `x?: T` ⇔ `x: T | undefined`, reordered unions, renamed generic params
- classes: reordered members, renamed private fields, constructor
  parameter properties ⇔ explicit field + assignment, and fully renamed
  method names when the canonical method bodies match

Rewrites that change behavior keep their distinct shapes on purpose:
swapped builtins (`.map` vs `.filter`, `Math.max` vs `Math.min`), a
different free callee (`sendEmail` vs `sendSms`), flipped operators
(`*` vs `/`, `&&` vs `||`, `<` vs `<=`), `??` vs `||`, `x == null` vs
`x === null`, optional chaining vs plain access, `for-of` vs `for-in`, an
added `break`, reordered statements that share data, `async` vs sync
contracts, `then(onFulfilled, onRejected)`, fall-through `switch` cases,
`Object.assign` with a mutated target, string prepend vs append folds
(`trail = trail + seg` vs `trail = seg + trail`), boundary indexes
(`.at(-1)` vs `.at(0)`, `.slice(0, n)` vs `.slice(n)`), templates that
stringify adjacent values (`` `${a}${b}` `` vs numeric `a + b`), and on
the type side `Promise<ShopUser>` vs `Promise<ShopOrder>` payloads,
`Map<K, V>` vs `Map<V, K>` swaps, and index signatures vs concrete
members. Twins that differ **only in data literals** (a table name, a
status code, a locale string) are reported — parameterizing them is the
refactor.

## Accuracy

Accuracy is tracked by a labeled benchmark (`bench/cases.ts` plus the
extended corpora in `bench/cases/`) that mirrors the refactoring flow
above: **261 ground-truth pairs** across functions, types, and classes —
semantic duplicates a refactoring plan must see, and similarly-shaped
lookalikes it must not flag — evaluated at the default threshold. The
corpus covers whole-function renames, guard/negation/ternary spellings,
loop-form rewrites, destructuring, nullish sugar, literal-vs-behavior
twins, and realistic cross-file copy-paste.

| Engine | Corpus | Wrong labels | Error rate | Accuracy |
| --- | --- | ---: | ---: | ---: |
| v0.3.0 | 71 pairs | 15 / 71 | 21.13% | 78.87% |
| v0.4.1 | 71 pairs | 0 / 71 | 0.00% | 100.00% |
| v0.4.1 | 261 pairs | 89 / 261 | 34.10% | 65.90% |
| v0.5.0 | 261 pairs | 7 / 261 | 2.68% | 97.32% |
| **v0.6.0** | **261 pairs** | **0 / 261** | **0.00%** | **100.00%** |

v0.6.0 closes out the seven pairs v0.5.0 still mislabeled — boundary-index
twins (`.at(-1)` vs `.at(0)`, `.slice(0, n)` vs `.slice(n)`), string
append vs prepend folds, generic-payload and key/value-swap type twins,
index-signature lookalikes, and fully-renamed class twins whose method
bodies match — bringing the full corpus to 100% at the default threshold.
The engine remains ~1.6x faster end-to-end than v0.4.1 (11.0s vs 17.2s for
a 311-file project across all four modes).

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
language-dependent.

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
