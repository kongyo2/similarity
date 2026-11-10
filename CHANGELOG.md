# Changelog

## 0.9.0 — 2026-11-10

A release about what the engine did not look at, and about how long it
took. Syntax that was dropped or flattened on the way to the comparison —
JSX, call signatures, overloads, parameter defaults, destructuring
assignments, class expressions — is compared now, overlap mode sees every
function and reports where a fragment really is, the scans are indexed
instead of quadratic, and code that used to bring the engine down is
skipped with a warning. 29 new ground-truth pairs pin the fixes (corpus now
**366 pairs**); the v0.8.1 engine gets 18 of them wrong, v0.9.0 classifies
all 366 correctly.

Reports change shape in three places, all of them to say more: functions
are named by the path that leads to them (`Repo.save`, not `save`), a
fragment repeated in three or more functions is one cluster instead of a
row per pair, and a location too long for the table wraps instead of being
cut off.

### Detection accuracy

- **Functions — JSX** (precision): JSX was converted to a single leaf, so
  two components were the same tree whenever the statements around their
  markup matched. Components of one outline rendering unrelated markup
  scored 0.92–1.00. Elements, attributes, children and expression
  containers are structure now, and those pairs score 0.06–0.15, while
  renamed copies and the same markup laid out differently still pair at
  1.000. (XF-N47 – XF-N50, XF-P55 – XF-P57)
- **Functions — parameters**: a default value and being the rest parameter
  are part of a signature. `f(limit = 10)`, `f(limit = compute())` and
  `f(limit)` — or `f(...items)` and `f()` — used to be one signature, and a
  rest parameter was a free identifier in the body. A default costs what
  its value holds. (XF-P59)
- **Functions — destructuring assignments** (precision): the pattern on the
  left of `({ clientX: x, clientY: y } = event)`, `[a, b] = [b, a]` or
  `for ([key, value] of entries)` was one leaf, whatever it held, so two
  functions that assigned different properties or targets scored 1.000.
  Patterns are converted like the patterns of a declaration; the variables
  assigned rename with the function, so a renamed twin is still a twin.
  (XF-N51, XF-P61)
- **Functions — the rest of the syntax**: each link of an optional chain has
  its own optionality (`a?.b.c` is not `a.b?.c`), a computed key is not the
  static key of the same spelling, directives (`"use server"`) and
  `#field in object` are represented, and a `var` declared twice is one
  variable. (XF-P58)
- **Functions and classes — class expressions** (recall): a class written as
  an expression bound to a name, `export const Store = class { … }`, was
  invisible to every mode. Its methods take part in function and overlap
  mode and the class in class mode, under the name it is bound to.
  (XF-P60, XC-P20, XC-N17)
- **Types — call and construct signatures** (precision and recall):
  signatures were not members, so an interface that is all signature was an
  empty shape. An empty marker interface scored 1.000 against the callable
  interface of the same name, a getter contract against the setter
  contract 0.96, a callable shape against a constructible one 0.91 — and
  the same signature under other parameter names 0.58, because only the
  names were left to compare. Signatures are members, compared by
  parameter and return types. (XT-N26 – XT-N29, XT-P26)
- **Types — numeric and computed keys** (precision): members keyed by a
  number (`200: {…}` in generated response maps) or by a computed key
  (`[Symbol.iterator]`) were dropped. They are members under their own
  key. (XT-N30, XT-N31, XT-P27)
- **Types — overloads and accessors**: an overloaded member was its last
  signature, so two factories sharing the last of three overloads scored
  0.99; each overload is a member now. `get size(): number` in an
  interface is the readonly property it declares (0.45 before, 0.99 now).
  (XT-N32, XT-P28, XT-P29)
- **Types — parameter lists**: rest and optional parameters are part of a
  function type (`(...parts: string[]) => void` is not `() => void`, 0.98
  before), and the parameter names of a callback that takes a callback no
  longer count (0.76 before, 0.98 now). (XT-N33, XT-P30)
- **Classes — `.tsx`**: method bodies of classes in `.tsx` files were never
  fingerprinted, because the fragment was parsed as plain TypeScript, so a
  different `render()` body did not count against two class components
  with the same members (0.797 before, 0.670 now for the corpus pair).
  (XC-N16, XC-P19)
- **`// similarity-ignore`** also applies above the statement that declares
  a function (`export const handler: Handler = async (event) => {…}`),
  above a class's decorators, and to everything declared inside an ignored
  class, object literal, function or namespace.
- **Order independence**: function and class reports depended on the order
  in which the files were listed. `similarity-ts a.ts b.ts` and
  `similarity-ts b.ts a.ts` now give the same report.

### Overlap mode

- Class methods, object-literal members and functions that share a line
  with other code take part. They were cut out of the file by whole lines
  and failed to parse on their own.
- An overlap is reported at the lines of the fragment itself, not at those
  of the statement or function around it.
- When a fragment and something inside it both repeat, the enclosing one is
  kept; a fragment inside a nested function is reported once, for the
  innermost function; fragments that repeat on both sides pair off in order
  instead of each with each; and a window no longer "matches" a single leaf
  at 0.5.
- `details.nodeType` is the syntactic kind of the fragment's root
  (`IfStatement`, `VariableDeclaration`, `LogicalExpression`, …) instead
  of the engine's internal label for it (`ConstDeclaration`, `Or`, `.`, or
  the name of a function).
- The scan answers from two indexes over all functions instead of
  comparing every pair of functions, with the same result as the pairwise
  comparison (a test holds the two against each other for eleven settings
  and three scopes).

### Reports

- **Names**: a function is reported under the path that leads to it — the
  namespaces it is declared in, then the class or object that holds it:
  `JwtTokenExpired.constructor`, `ZodOptional.unwrap`,
  `LegacyIds.normalizeId`, `handlers[200]`. Function mode used to print the
  bare member name, so a report could list four functions called
  `constructor` in one file. Types and classes were qualified already.
  **`symbolName` changes for every class member and every function in a
  namespace**, and for object members under a numeric key (`handlers.0` is
  `handlers[0]`).
- **Long locations** wrap after a path separator instead of being cut off.
  The pair table truncated every location wider than its column
  (`packages/server/src/unstable-core-do-not-import/strea…`): on a project
  with deep directories, most rows had no file name and no line numbers.
  Widths are measured in terminal columns, so paths in full-width
  characters fit too.
- **Overlap clusters**: three or more copies of one fragment are a cluster,
  as copies of a declaration already were, headed by the fragment's kind
  and size (`Cluster 1: 5 fragments (IfStatement, 26 nodes), …`). In the
  JSON report such a cluster has `mode: "overlap"` and a `fragment` field
  (`nodeCount`, `nodeType`), and `stats.clusterCount` counts it. On zod's
  v3 sources the overlap section goes from 481 rows to 23 clusters and 38
  rows.
- A cluster whose pairs all score the same prints the score once.
- A report written with `--output`, or piped, carries no ANSI escape codes.
  `formatPrettyReport` takes `{ color }` and is plain by default.
- Warnings name the file they are about, relative to the working
  directory, in the report and on stderr alike. Files that cannot be read
  are listed in `skippedFiles`.
- Files, pairs, cluster members and clusters are ordered by code unit, not
  by the host's locale.

### Files

- Directories are walked top-down and an ignored directory is never
  entered. The scan used to list every file under the target,
  `node_modules` included, and filter afterwards.
- `node_modules`, `dist` and `coverage` are excluded at any depth below a
  scanned directory, not only at its top (workspace packages have their
  own), and a directory named explicitly as a target is scanned whatever it
  is called.
- `.gitignore` files apply with git's precedence, from the repository root
  down — also when the root is above the working directory: a deeper file
  overrides a shallower one and `!pattern` re-includes. `--exclude` always
  wins.
- `skippedFiles` lists what was named or found and left out: targets that
  are no source file, files an ignore rule names, files that cannot be
  parsed or read. The contents of ignored directories are not listed.
- Entries whose name starts with a dot are skipped and symbolic links are
  not followed, as before; a directory that cannot be read is a warning
  instead of silently empty.
- `--extensions` with nothing usable in it (`""`, `"."`) means the
  defaults; a lone `.` used to match no file.

### Robustness

- **Deeply nested code** no longer overflows the stack. A file whose
  brackets nest more than 256 levels, or whose syntax tree is deeper than
  512, is skipped with a warning. Nesting the parser itself cannot survive
  (thousands of chained conditionals, arrows or generic arguments) takes
  the WebAssembly instance down; the wrapper then finds the file, skips it
  with a warning, and analyzes the rest on a fresh instance. v0.8.1 crashed
  on 32 of 33 nesting shapes somewhere between 1,000 and 20,000 levels.
- **Very long lists** — a table of thousands of entries, a generated
  function of thousands of statements — are aligned along the entries the
  two sides share instead of through a matrix of every entry against every
  other. Two functions holding 20,000 elements compare in 0.07–0.3 s
  within 130 MB; v0.8.1 took 2.4 s and 790 MB for 3,000 object properties
  and crashed on 3,000 statements.
- A crash that cannot be pinned on a file is reported as a bug in
  similarity-ts, with the reason, instead of as a raw `RuntimeError`.
- `--overlap-max-window` is limited to 1000 nodes; a window of thousands of
  nodes is a whole function, which function mode compares.

### Performance

Reports are the same as without the optimizations; the corpus below is
compared pair for pair after every change.

- Function mode numbers labels and kinds once per scan, flattens each
  function once instead of once per pair, and rules pairs out with a lower
  bound on the edit distance that prices unmatched nodes by kind; 1 pair of
  similar size in 60 reaches the alignment, where 3 in 4 did.
- Type mode rules out pairs that share too few member names and
  annotations to reach the threshold before normalizing anything, and
  computes the similarity of two annotations once per scan.
- Overlap mode is indexed (see above).
- A file is parsed once for all modes.

### CLI and library

- An unknown `--modes` value is named
  (`Unknown mode "x" (expected one of: functions, types, classes, overlap)`)
  instead of dumped as a schema error; `modes` and `typesOnly` are
  validated for library callers too.
- The library exports the formatters (`formatPrettyReport`,
  `formatJsonReport`), `resolveAnalyzeOptions`, the defaults
  (`DEFAULT_THRESHOLD`, `DEFAULT_EXCLUDES`, `MAX_OVERLAP_WINDOW`, …) and
  the types `ResolvedAnalyzeOptions`, `PrettyReportOptions`,
  `SimilarityFragment` and `TypeFilter`.
- `--help` states each default once.

### Dependencies and tooling

- `zod` and `fast-glob` are gone: the option parser and the file walker no
  longer need them, and installing the package brings 12 packages instead
  of 33. `string-width` is a direct dependency at the release `cli-table3`
  already installs.
- The package runs on Node 20 and later, as before. Building and testing it
  needs Node 22.18: TypeScript 6.0, ESLint 10, Vitest 5.1, tsup 9.
- CI tests on Node 22, 24 and 26 and installs the packed tarball on Node
  20, 22, 24 and 26 (`npm run test:package -- <tarball>`).
- `npm run bench:accuracy` checks that every label resolves — a forbidden
  pair under a name the report no longer uses would pass forever — and
  exits 1 on a mislabel.
- `npm run build` no longer deletes the built engine, and
  `npm run build:wasm` fails when the built module does not load.

### Internals

- The Rust core is restructured: the 4,800-line parser is a module
  directory whose conversion state travels as an explicit context instead
  of three thread-locals, the function extractor is split into extraction,
  fragment building, atoms and comparison, tree nodes carry the span of
  their source and a static kind, and the JSON boundary is typed and
  validates its options.

### Weights

- The APTED operation weights (rename 0.3, delete 1.0, insert 1.0) keep
  366/366 under every setting in rename 0.2–0.5 × delete/insert 0.8–1.0;
  the closest call is still XF-N36 at rename 0.2 (0.797 against the 0.8
  threshold).

### Real-world effect

Default settings, pairs reported by v0.8.1 → v0.9.0 and the engine's time,
on snapshots of 2026-11-06 to 2026-11-09 (the upstream fixtures:
2026-10-03):

| Target | functions | types | classes | overlap | time |
| --- | ---: | ---: | ---: | ---: | ---: |
| hono `src` (314 files) | 34 → 34 | 53 → 23 | 0 → 0 | 248 → 261 | 23.5 s → 1.0 s |
| zustand (35 files) | 0 → 0 | 35 → 35 | 0 → 0 | 1 → 1 | 0.3 s → 0.05 s |
| zod v4 core and classic, v3 (236 files) | 856 → 856 | 148 → 144 | 1 → 1 | 1,493 → 1,936 | 29.2 s → 1.2 s |
| tRPC server, client, react-query, next (202 files) | 25 → 25 | 48 → 47 | 4 → 4 | 178 → 235 | 15.0 s → 0.7 s |
| tRPC `examples` and `www` (490 files) | 1,699 → 71 | 602 → 600 | 1 → 1 | 330 → 390 | 15.6 s → 0.5 s |
| vitest node, runner, expect, utils (156 files) | 37 → 37 | 10 → 10 | 4 → 4 | 127 → 436 | 36.7 s → 1.4 s |
| mizchi/similarity `examples` and `test` (84 files) | 84,771 → 84,770 | 315 → 315 | 195 → 195 | 81 → 84,683 | 11.3 s → 5.2 s |

- **tRPC's examples**: 1,631 function pairs are gone, all between React
  components whose markup differs (`SupportedBy` and `HomePage`,
  `RootLayout` and `DefaultLayout`), and three are new: the icon components
  of one file, whose SVG differs in its path data.
- **hono's types**: the 30 pairs gone are signature-only interfaces —
  `CssType` returning `Promise<string>` in the server helper and `string`
  in the DOM one, the empty `ContextRenderer` against its augmentation in
  the tests — and interfaces of dozens of overloads compared by their
  last.
- **zod's types**: three brand types of one member each, keyed by
  `[BRAND]`, `[core.$brand]` and `[$brand]`, which used to be three empty
  shapes, and a callable `$ZodErrorMap` against a constructible
  `$ZodErrorClass`.
- **Overlap** grows where methods are: vitest's node package is classes.
  The 84,683 pairs of the upstream fixtures are one generated file of 400
  identical functions, reported as one cluster of 400 fragments; function
  mode has always reported them, as a cluster of 400 functions.
- Every other function and class pair is reported as before, and four
  scores move: two vitest helpers that differ in a rest parameter score
  0.89 instead of 0.96, two components whose markup differs slightly 0.97
  instead of 1.00, one pair in zod moves in the fourth decimal, and two
  four-line constructors of the upstream fixtures that differ in a default
  value score 0.72 instead of 0.89 and leave the report.

### Accuracy

| Engine | Corpus | Wrong labels | Error rate | Accuracy |
| --- | --- | ---: | ---: | ---: |
| v0.8.1 | 366 pairs | 18 / 366 | 4.92% | 95.08% |
| v0.9.0 | 366 pairs | 0 / 366 | 0.00% | 100.00% |

## 0.8.1 — 2026-11-05

Fixes for gaps left in the 0.8.0 extraction and comparison rules, and a
machine-independent cluster order. Twelve new ground-truth pairs pin the
fixes (corpus now **337 pairs**); the v0.8.0 engine gets all twelve wrong,
v0.8.1 classifies all 337 correctly, and the 325 pairs of 0.8.0 keep their
scores.

### Detection accuracy

- **Functions — methods calling a helper named like themselves**
  (precision): a member key binds nothing inside the member's body, so
  `debug(…)` inside a `debug()` method calls the outer `debug`. Plain
  methods and arrow-valued class fields resolved such a call to the method
  itself, as if it were recursion, so methods delegating to different
  helpers (`debug()` calling `debug(…)`, `info()` calling `info(…)`)
  scored 1.000 against each other, and a method delegating to a helper
  scored 1.000 against a recursive function with its body. The call is a
  free call now, as it already was in static methods, accessors,
  arrow-valued object members and class mode, and a different callee caps
  the pair like any other. (XF-N45, XF-N46)
- **Functions — object members that recurse** (recall): in
  `visit: function walk(node) {… walk(node.left) …}` the function
  expression's own name is the function, and it now alpha-renames with it.
  Two copies whose own names differ scored 0.75 and pair at 1.000 now,
  under identifier and computed keys alike (`[OpKind.Sum]: function
  evaluateSum(…)`), while a call spelled like the key (`visit(…)`) stays
  a call to the outer `visit`. (XF-P53, XF-P54)
- **Types — literals containing quotes** (precision): a string-literal type
  is spelled with its inner quotes escaped (`'say "yes"'` is
  `"say \"yes\""`), so it reads as one literal. Discriminated variants
  told apart by such literals (`quote: '"'` vs `quote: "'"`,
  `kind: 'say "yes"'` vs `kind: 'say "no"'`) scored about 0.92 and were
  reported; they score about 0.45 now, like any other discriminant
  mismatch. (XT-N24)
- **Types — quote style**: string literals in type-alias bodies and in
  source-rendered types are spelled in double quotes whatever the source
  used. `type SchemaVersion = '1'` is now a literal atom like
  `type SchemaVersion = "1"` and takes the atomic-alias name check:
  single-quoted version singletons scored about 0.99 against each other
  and score about 0.3 now, while the same alias copied into a single-quoted
  and a double-quoted file pairs at 1.000 (0.43 before). (XT-N25, XT-P25)
- **Classes — type parameter order** (recall): member annotations are
  normalized again after class and method type parameters are spelled
  positionally, so unions and intersections of the parameters compare
  equal when the parameters are declared in another order
  (`Settled<TValue, TError>` vs `Outcome<E, T>` with `TValue | TError`
  members: 0.62 before, 0.87 now). Heritage type arguments are normalized
  like annotations in class and type mode (`extends Base<A | B>` ⇔
  `extends Base<B | A>`). (XC-P17)
- **Classes — constructor wiring** (recall): a constructor that stores a
  parameter through parentheses or a type assertion
  (`this.canvas = canvas as HTMLCanvasElement`, `this.x = (x)`, `x!`,
  `x satisfies T`) only wires its parameters, like `this.x = x`, instead of
  counting as a member the other class lacks (0.77 before, 0.89 now for
  the corpus pair). (XC-P18)

### Reports

- Cluster members, and clusters that tie, are ordered by code unit instead
  of the host locale, so the JSON `clusters` field is the same on every
  machine.

### Weights

- The APTED operation weights (rename 0.3, delete 1.0, insert 1.0) keep
  337/337 under every setting in rename 0.2–0.5 × delete/insert 0.8–1.0;
  the closest call is still XF-N36 at rename 0.2 (0.797 against the 0.8
  threshold).

### Real-world effect

On current snapshots of the repositories audited for 0.8.0 — the
TypeScript API package (59 files), its VS Code extension and scripts (58),
the tsrs npm package, tools and playground (246) and standard-library
declarations (116), and fallow's VS Code extension (123) and tools, npm
packages and frontend (76) — v0.8.1 reports the same type, class and
overlap pairs as v0.8.0. Function pairs change where the TypeScript API
code is, in the TypeScript repository and in the copy the tsrs npm package
bundles:

| Target | Mode | v0.8.0 | v0.8.1 |
| --- | --- | ---: | ---: |
| TypeScript `packages/typescript/src` (59 files) | functions | 1,485 | 861 |
| tsrs npm package, tools, playground (246 files) | functions | 1,460 | 836 |

The 624 pairs gone from each are methods that call the module function of
their own name, which scored 1.000 against each other: the type guards of
the async and sync `Type` classes (`isUnionType()` returns
`isUnionType(this)`, `isObjectType()` returns `isObjectType(this)`) and
the `getChildren`/`getFirstToken`/`getLastToken` wrappers of the generated
node classes. Each method still pairs with its copy in the other file
(`isUnionType` in the async and the sync class). One score changes:
fallow declares `type SimilarCodeProvider = 'official-local-companion'` in
its N-API typings and the double-quoted spelling in its npm output
contract, and the pair scores 1.000 instead of 0.935.

### Accuracy

| Engine | Corpus | Wrong labels | Error rate | Accuracy |
| --- | --- | ---: | ---: | ---: |
| v0.8.0 | 337 pairs | 12 / 337 | 3.56% | 96.44% |
| v0.8.1 | 337 pairs | 0 / 337 | 0.00% | 100.00% |

## 0.8.0 — 2026-11-04

Accuracy work driven by three more real codebases. The engine was run over
the fallow analyzer (its VS Code extension, tools, npm packages and
visualization frontend), tsrs (its npm package, tools and playground, and
the 116 standard-library declaration files it bundles), and the current
TypeScript repository (the API package, which now ships sync and async
siblings, the VS Code extension, the generator scripts, and 1,381
conformance tests), and every class of reported pair was audited. The
mislabel families were added to the labeled corpus as 25 new ground-truth
pairs (corpus now **325 pairs**); the v0.7.0 engine gets 22 of them wrong
(6.77% error rate on the extended corpus), v0.8.0 classifies all 325
correctly and keeps the original 300 at 100%.

### Detection accuracy

- **Functions — call contracts** (precision): an `async` function returns a
  promise and a generator an iterator, so a plain twin with the same body is
  a different contract. Such pairs cap at 0.72 unless the sync side visibly
  returns a promise: it declares `Promise<…>`/`PromiseLike<…>`, returns
  `new Promise(…)` or a `Promise.*(…)` call, or chains the promise the async
  side awaits (`return fetch(u).then((r) => …)` vs `const r = await
  fetch(u); …`, which the canonicalizer already lowers to the same `await`).
  On v0.7.0 the sync and async API packages' `getSymbol` twins scored
  0.970, and a static async method scored 1.000 against its sync version
  (the label of a class-wrapped method never carried the `async` marker);
  v0.8.0 keeps these pairs below the 0.8 threshold. (XF-N42, XF-N43,
  XF-N44)
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
  (`count = 0` and `offset = -1` are `number`s, `cache = new Map<string,
  User>()` a `Map<string, User>`), signatures keep optional and rest
  parameters, and the rename tolerance for class-reference types no longer
  erases the segments of a qualified name. (XC-N12)
- **Classes — generic parameters** (recall): class type parameters are
  spelled positionally in member annotations and in `extends`/`implements`
  clauses, as type mode does for interfaces, so `ItemList<TItem> extends
  Collection<TItem>` and `EntryList<U> extends Collection<U>` are the same
  contract (the heritage mismatch used to cost 30%); a method's own type
  parameters are positional within the method. In both modes only type
  references are substituted: string-literal text (`"Node"`) and qualified
  name segments (`ts.Node`) that spell a parameter name keep their
  spelling. (XC-P15)
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
