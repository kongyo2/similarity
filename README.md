# @kongyo2/similarity-ts

TypeScript-only similarity analyzer for duplicate detection across:

- functions
- types
- classes
- token overlaps

## Quick Start

```bash
npx @kongyo2/similarity-ts src --threshold 0.8
```

## CLI

```bash
similarity-ts <paths...> [options]
```

Key options:

- `--modes functions,types,classes,overlap`
- `-t, --threshold <0-1>`
- `--min-lines <n>`
- `--no-size-penalty`
- `--same-file-only`
- `--cross-file-only`
- `--types-only all|interface|type`
- `--allow-cross-kind`
- `--type-literals`
- `--overlap-min-window <n>`
- `--overlap-max-window <n>`
- `--overlap-size-tolerance <0-1>`
- `--extensions ts,tsx,mts,cts`
- `--exclude <pattern>` (repeatable)
- `--format pretty|json`
- `--output <path>`

## Library

```ts
import { analyzeProject } from "@kongyo2/similarity-ts";

const report = await analyzeProject({
  paths: ["src"],
  modes: ["functions", "types"],
  threshold: 0.75,
});

console.log(report.stats.pairCount);
```

## Notes

- Supported source files: `.ts`, `.tsx`, `.mts`, `.cts`
- JavaScript files are intentionally excluded

## Development

```bash
npm run lint
npm run lint:fix
```
