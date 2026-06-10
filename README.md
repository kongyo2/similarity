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

The analyzer compares functions, types, and classes structurally, so renames
never hide a duplicate. On top of that, the comparison canonicalizes common
style alternatives — two snippets a reviewer would call "the same code
written differently" score as equal:

- arrow functions ⇔ function declarations ⇔ class methods
- `promise.then((v) => …)` ⇔ `const v = await promise; …`
- `items.forEach((item) => …)` ⇔ `for (const item of items) …`
- `` `Hello ${name}` `` ⇔ `"Hello " + name`
- `total += n` ⇔ `total = total + n` ⇔ `i++` (in statement position)
- `return c ? a : b` ⇔ `if (c) { return a } else { return b }`
- `const x = c ? a : b` ⇔ `let x; if (c) { x = a } else { x = b }`
- jump-terminated `switch` ⇔ `if`/`else if` chains
- `Object.assign({}, a, b)` ⇔ `{ ...a, ...b }`
- `for (; cond; )` ⇔ `while (cond)`, braced ⇔ brace-less bodies
- interfaces ⇔ structurally identical type aliases, reordered members

Rewrites that change behavior (logical assignments, `then(onFulfilled,
onRejected)`, fall-through `switch` cases, `Object.assign` with a mutated
target, string prepend vs append folds, `async` vs sync contracts) keep
their distinct shapes on purpose.

## Accuracy

Accuracy is tracked by a labeled benchmark (`bench/cases.ts`) that mirrors
the refactoring flow above: 71 ground-truth pairs across functions, types,
and classes — semantic duplicates a refactoring plan must see, and
similarly-shaped lookalikes it must not flag — evaluated at the default
threshold.

| Engine | Wrong labels | Error rate | Accuracy |
| --- | ---: | ---: | ---: |
| v0.3.0 | 15 / 71 | 21.13% | 78.87% |
| v0.4.0 | 0 / 71 | 0.00% | 100.00% |

Run it yourself with `npm run bench:accuracy`; the suite in
`tests/accuracy-benchmark.test.ts` fails CI if the error rate ever rises
above one tenth of the v0.3.0 baseline.

## CLI options

| Option | Default | Description |
| --- | --- | --- |
| `--modes <list>` | `functions,types,classes,overlap` | Comma-separated analysis modes |
| `-t, --threshold <number>` | `0.8` | Similarity threshold (0–1) |
| `--min-lines <number>` | `3` | Minimum function line count |
| `--no-size-penalty` | off | Disable the short-function score penalty |
| `--same-file-only` / `--cross-file-only` | off | Restrict pair scope |
| `--extensions <list>` | `ts,tsx,mts,cts` | File extensions to scan |
| `--exclude <pattern>` | — | Exclude glob (repeatable) |
| `--types-only <kind>` | `all` | Restrict type mode to `interface` or `type` |
| `--no-allow-cross-kind` | off | Disable interface ⇔ type alias matching |
| `--type-literals` | off | Include anonymous type literals in type mode |
| `--format <pretty\|json>` | `pretty` | Output format |
| `--output <path>` | — | Write the report to a file |
| `--fail-on-warnings` | off | Non-zero exit on analyzer warnings |

Annotate a declaration with a `// similarity-ignore` comment on the
preceding line to exclude it from the report.
