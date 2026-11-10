import path from 'node:path';
import process from 'node:process';
import { performance } from 'node:perf_hooks';
import { analyzeProject } from '../src/index.js';
import type { AnalyzerMode } from '../src/index.js';

const fixtureRoot = path.resolve(process.cwd(), 'tests/fixtures');

async function runCase(name: string, target: string, modes: AnalyzerMode[]) {
  const abs = path.join(fixtureRoot, target);
  const start = performance.now();
  const report = await analyzeProject({
    cwd: process.cwd(),
    paths: [abs],
    modes,
    threshold: 0.7,
    minLines: 3,
  });
  const elapsedMs = performance.now() - start;
  return {
    name,
    target,
    modes,
    elapsedMs,
    pairCount: report.results.length,
    byMode: Object.fromEntries(Object.entries(report.byMode).map(([k, v]) => [k, v.length])),
  };
}

function toMarkdown(rows: Awaited<ReturnType<typeof runCase>>[]) {
  const lines = [
    '# Baseline Metrics',
    '',
    `Generated at: ${new Date().toISOString()}`,
    '',
    '| Dataset | Modes | Pairs | functions | types | classes | overlap | elapsedMs |',
    '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |',
  ];

  for (const row of rows) {
    lines.push(
      `| ${row.target} | ${row.modes.join(',')} | ${row.pairCount} | ${row.byMode.functions ?? 0} | ${row.byMode.types ?? 0} | ${row.byMode.classes ?? 0} | ${row.byMode.overlap ?? 0} | ${row.elapsedMs.toFixed(2)} |`,
    );
  }

  lines.push('', '## Notes', '', '- Pair counts and timings of the bundled fixtures at threshold 0.7.', '- `similar/refactoring` is used for recall-oriented spot checks.', '- `dissimilar` is used for false-positive spot checks.');
  return `${lines.join('\n')}\n`;
}

const rows = await Promise.all([
  runCase('similar-refactoring', 'refactoring', ['functions', 'types', 'classes']),
  runCase('dissimilar', 'dissimilar', ['functions', 'types', 'classes']),
  runCase('similar-overlap', 'similar', ['overlap']),
]);

process.stdout.write(toMarkdown(rows));
