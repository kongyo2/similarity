---
name: check-similarity-ts
description: Detect duplicate TypeScript code with @kongyo2/similarity-ts (AST-based, alpha-renamed and refactor-canonicalized) and turn the report into a refactoring plan. Use when working with .ts/.tsx/.mts/.cts files and looking for duplicated functions, types, or classes.
argument-hint: "[path] [--threshold 0.8] [--modes functions,types,classes]"
allowed-tools: Bash(npx @kongyo2/similarity-ts *) Bash(npx similarity-ts *) Read Grep Glob
paths: "**/*.ts,**/*.tsx,**/*.mts,**/*.cts"
---

# TypeScript Code Similarity Detection

## What to do

Run `@kongyo2/similarity-ts` on the target, read the duplicate clusters and
pairs it reports, and propose a refactoring for each.

## Step 1: Run the analysis

```bash
npx @kongyo2/similarity-ts $ARGUMENTS
```

If no arguments were given, scan the project and keep the report machine
readable:

```bash
npx @kongyo2/similarity-ts . --format json --output similarity-report.json
```

Generated code (`*.generated.ts`, protocol and contract `.d.ts` files) is
duplicated by construction; exclude it with `--exclude "**/*.generated.ts"`
unless the generator itself is the subject.

## Step 2: Read the report

- **Clusters** (`clusters` in JSON, `Cluster N:` blocks in the pretty
  output) are three or more declarations connected by reported pairs. Treat
  each cluster as one refactoring: one shared implementation, every member
  rewritten to use it.
- **Overlap clusters** (`mode: "overlap"`, "N fragments") are three or more
  copies of one fragment inside functions that differ as a whole — a guard,
  a lookup, a block. `fragment` gives its size in syntax nodes and the kind
  of its root (`IfStatement`, `VariableDeclaration`, …); the members are
  the line ranges to replace with a call to one extracted helper. Overlap
  fragments are compared as written, so these are pasted copies, not
  rewrites.
- `symbolName` is the path to a declaration: `Repo.save` is the method
  `save` of the class `Repo`, `handlers.onSave` a member of the object
  `handlers`, `Legacy.parse` a function in the namespace `Legacy`.
- **Standalone pairs** are two declarations, or two fragments, with no
  third copy.
- `similarity` 1.000 means the canonical trees are identical: the code is
  the same up to renames and accepted style spellings (arrow vs function,
  `.then` vs `await`, `forEach` vs `for-of`, guard style, …).
- Pairs that differ only in data literals (a table name, a status code, an
  RPC method name) are reported on purpose: parameterize the literal.

## Step 3: Propose the refactoring

| Report | Refactoring |
|--------|-------------|
| Function cluster or pair at 0.95–1.000 | Extract one function; pass the differing literals or callees as parameters |
| Function pair at 0.80–0.95 | Extract the shared skeleton; keep the differing step as a callback or strategy |
| Interfaces / type aliases | Keep one definition and re-export it, or extract a base interface and `extends` it |
| Classes | Extract a base class or a shared helper the classes delegate to; generic parameters for differing payload types |
| Overlap cluster or pair | Extract the fragment into a helper when it is a step of its own (a guard, a conversion); leave incidental one-line repeats alone |

Show before/after code for each high-similarity cluster. Check behavior
before merging: a cluster is connected through pairs, so two members can be
less similar than either is to a third — compare the members' bodies, not
just the scores.

## Key options

| Option | Description |
|--------|-------------|
| `--modes <list>` | `functions,types,classes,overlap` (default: all four) |
| `-t, --threshold <0-1>` | Similarity threshold (default: 0.8) |
| `--min-tokens <n>` | Skip functions smaller than `n` AST nodes (~50 for noisy code) |
| `--cross-file-only` / `--same-file-only` | Restrict pair scope |
| `--exclude <glob>` | Exclude files (repeatable) |
| `--format json` | Machine-readable report with `results`, `byMode`, `clusters` |
| `--fail-on-duplicates` | Exit 1 when any pair is reported (CI gate) |

Mark a declaration that must stay duplicated with a `// similarity-ignore`
comment on the line before it.
