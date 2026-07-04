# @kongyo2/similarity-ts

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
  parameter properties ⇔ explicit field + assignment

Rewrites that change behavior keep their distinct shapes on purpose:
swapped builtins (`.map` vs `.filter`, `Math.max` vs `Math.min`), a
different free callee (`sendEmail` vs `sendSms`), flipped operators
(`*` vs `/`, `&&` vs `||`, `<` vs `<=`), `??` vs `||`, `x == null` vs
`x === null`, optional chaining vs plain access, `for-of` vs `for-in`, an
added `break`, reordered statements that share data, `async` vs sync
contracts, `then(onFulfilled, onRejected)`, fall-through `switch` cases,
`Object.assign` with a mutated target, string prepend vs append folds, and
templates that stringify adjacent values (`` `${a}${b}` `` vs numeric
`a + b`). Twins that differ **only in data literals** (a table name, a
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
| **v0.5.0** | **261 pairs** | **7 / 261** | **2.68%** | **97.32%** |

v0.5.0 keeps the original 71-pair corpus at 100% while cutting the
extended-corpus error rate **12.7x** below the previous engine. It is also
~1.6x faster end-to-end (11.0s vs 17.2s for a 311-file project across all
four modes; 557ms vs 879ms for functions+types+classes), despite doing far
more canonicalization work per function.

Run it yourself with `npm run bench:accuracy`; the suite in
`tests/accuracy-benchmark.test.ts` fails CI if the error rate ever rises
above one tenth of the v0.4.1 baseline, or if any original 71-pair label
regresses.

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
