#!/usr/bin/env node
import { realpathSync } from "node:fs";
import fs from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import chalk from "chalk";
import { Command, CommanderError, Option } from "commander";
import { analyzeProject, resolveModes } from "./analyze.js";
import {
  ANALYZER_MODES,
  DEFAULT_EXTENSIONS,
  DEFAULT_MODES,
  DEFAULT_MIN_LINES,
  DEFAULT_OVERLAP_MAX_WINDOW,
  DEFAULT_OVERLAP_MIN_WINDOW,
  DEFAULT_OVERLAP_SIZE_TOLERANCE,
  DEFAULT_THRESHOLD,
  MAX_OVERLAP_WINDOW,
  TYPE_FILTERS,
} from "./defaults.js";
import { formatJsonReport, formatPrettyReport, formatWarning } from "./format.js";
import type { TypeFilter } from "./types.js";
import { errorMessage } from "./utils/errors.js";

/** Where the CLI writes: the report and help to `log`, problems to `error`. */
export interface CliIO {
  log: (message: string) => void;
  error: (message: string) => void;
}

/**
 * The options as commander hands them over: every option that takes a
 * value arrives as the string typed (or the default, itself a string).
 */
interface ParsedCliOptions {
  modes: string;
  threshold: string;
  minLines: string;
  sizePenalty: boolean;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
  extensions: string;
  /** Absent when the option was not given. */
  exclude?: string[];
  typesOnly: TypeFilter;
  /**
   * Commander's `--no-allow-cross-kind` flag stores its value on this key.
   * It defaults to `true` and flips to `false` when the negative flag is
   * passed, so we keep the boolean shape but rely on the underlying
   * default rather than an explicit one.
   */
  allowCrossKind: boolean;
  typeLiterals: boolean;
  minTokens?: string;
  failOnDuplicates: boolean;
  overlapMinWindow: string;
  overlapMaxWindow: string;
  overlapSizeTolerance: string;
  format: "pretty" | "json";
  output?: string;
  failOnWarnings: boolean;
}

// Both `src/cli.ts` (dev/test) and the bundled `dist/cli.js` sit one level
// below the package root, so the same relative specifier resolves to the
// package manifest from either location.
const packageJson = createRequire(import.meta.url)("../package.json") as { version: string };

function parseCommaList(rawValue: string): string[] {
  return rawValue
    .split(",")
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

function buildProgram(io: CliIO): Command {
  const program = new Command();
  program
    .name("similarity-ts")
    .description("TypeScript code similarity analyzer")
    .version(packageJson.version)
    .argument("<paths...>", "Files and directories to analyze")
    .option(
      "--modes <list>",
      `Comma-separated modes: ${ANALYZER_MODES.join(",")}`,
      DEFAULT_MODES.join(","),
    )
    .option("-t, --threshold <number>", "Similarity threshold (0-1)", String(DEFAULT_THRESHOLD))
    .option("--min-lines <number>", "Minimum function line count", String(DEFAULT_MIN_LINES))
    .option(
      "--min-tokens <number>",
      "Minimum function size in AST nodes (replaces the line gate; ~50 recommended for noisy code)",
    )
    .option("--no-size-penalty", "Disable line-count size penalty for function mode")
    .option("--same-file-only", "Only compare symbols from the same file", false)
    .option("--cross-file-only", "Only compare symbols across different files", false)
    .option(
      "--extensions <list>",
      "Comma-separated extensions of the files to scan",
      DEFAULT_EXTENSIONS.join(","),
    )
    .option(
      "--exclude <pattern>",
      "Leave out what a gitignore-style pattern matches (repeatable)",
      (value: string, previous: string[] | undefined) => [...(previous ?? []), value],
    )
    .addOption(
      new Option("--types-only <kind>", "Type mode filter").choices([...TYPE_FILTERS]).default("all"),
    )
    // Cross-kind comparison is on by default — pass `--no-allow-cross-kind`
    // to restrict matches to interface↔interface and type↔type pairs only.
    // The positive `--allow-cross-kind` flag is kept as a deprecated no-op
    // alias so existing CI commands that still pass it don't fall over.
    .option(
      "--allow-cross-kind",
      "(deprecated; cross-kind matches are detected by default)",
    )
    .option(
      "--no-allow-cross-kind",
      "Disable cross-kind type comparisons (cross-kind matches are detected by default)",
    )
    .option("--type-literals", "Include anonymous type literals in type mode", false)
    .option(
      "--overlap-min-window <number>",
      "Smallest overlap fragment, in AST nodes",
      String(DEFAULT_OVERLAP_MIN_WINDOW),
    )
    .option(
      "--overlap-max-window <number>",
      `Largest overlap fragment, in AST nodes (at most ${MAX_OVERLAP_WINDOW})`,
      String(DEFAULT_OVERLAP_MAX_WINDOW),
    )
    .option(
      "--overlap-size-tolerance <number>",
      "Allowed segment-size ratio difference in overlap mode",
      String(DEFAULT_OVERLAP_SIZE_TOLERANCE),
    )
    .addOption(new Option("--format <format>", "Output format").choices(["pretty", "json"]).default("pretty"))
    .option("--output <path>", "Write report to file")
    .option(
      "--fail-on-warnings",
      "Exit with a non-zero code when the analysis emits any warning",
      false,
    )
    .option(
      "--fail-on-duplicates",
      "Exit with a non-zero code when any similar pair is reported (for CI gates)",
      false,
    )
    .showHelpAfterError(true);
  // Route commander's own output (`--help`, `--version`, parse errors)
  // through the injected io and surface terminations as return codes
  // instead of `process.exit`, keeping `runCli` side-effect free for
  // library and test callers.
  program.exitOverride();
  program.configureOutput({
    writeOut: (text) => io.log(text.replace(/\n$/, "")),
    writeErr: (text) => io.error(text.replace(/\n$/, "")),
  });
  return program;
}

// Only plain decimal notation is accepted: `Number("")` is 0 and
// `Number("0x10")`/`Number("1e9")` silently succeed, which turned typos
// into surprising thresholds and limits.
const DECIMAL_NUMBER = /^-?(?:\d+|\d*\.\d+)$/;
const DECIMAL_INTEGER = /^-?\d+$/;

function parseNumber(value: string, field: string): number {
  const raw = value.trim();
  if (!DECIMAL_NUMBER.test(raw)) {
    throw new Error(`${field} must be a valid number`);
  }
  return Number(raw);
}

function parseInteger(value: string, field: string, minimum = 1): number {
  const raw = value.trim();
  if (!DECIMAL_INTEGER.test(raw)) {
    throw new Error(`${field} must be an integer`);
  }
  const parsed = Number(raw);
  if (!Number.isSafeInteger(parsed)) {
    throw new Error(`${field} must be an integer`);
  }
  if (parsed < minimum) {
    throw new Error(`${field} must be greater than or equal to ${minimum}`);
  }
  return parsed;
}

function normalizeOptions(rawOptions: ParsedCliOptions) {
  const threshold = parseNumber(rawOptions.threshold, "threshold");
  const minLines = parseInteger(rawOptions.minLines, "min-lines");
  const minTokens =
    rawOptions.minTokens === undefined
      ? undefined
      : parseInteger(rawOptions.minTokens, "min-tokens");
  const overlapMinWindow = parseInteger(rawOptions.overlapMinWindow, "overlap-min-window");
  const overlapMaxWindow = parseInteger(rawOptions.overlapMaxWindow, "overlap-max-window");
  const overlapSizeTolerance = parseNumber(
    rawOptions.overlapSizeTolerance,
    "overlap-size-tolerance",
  );

  if (threshold < 0 || threshold > 1) {
    throw new Error("threshold must be between 0 and 1");
  }
  if (overlapSizeTolerance < 0 || overlapSizeTolerance > 1) {
    throw new Error("overlap-size-tolerance must be between 0 and 1");
  }
  if (overlapMinWindow > overlapMaxWindow) {
    throw new Error(
      "overlap-min-window must be less than or equal to overlap-max-window",
    );
  }
  if (overlapMaxWindow > MAX_OVERLAP_WINDOW) {
    throw new Error(`overlap-max-window must be less than or equal to ${MAX_OVERLAP_WINDOW}`);
  }
  if (rawOptions.sameFileOnly && rawOptions.crossFileOnly) {
    throw new Error("Cannot use both --same-file-only and --cross-file-only");
  }

  // Commander's `--no-<name>` convention stores the parsed value on the
  // positive key (`sizePenalty`) rather than `noSizePenalty`, and defaults
  // to `true` unless the flag is passed. Translate back to the library's
  // `noSizePenalty` flag.
  const sizePenalty = rawOptions.sizePenalty;
  return {
    // An unknown mode is rejected here, by name; repeats are dropped so a
    // mode listed twice is not rendered twice.
    modes: resolveModes(parseCommaList(rawOptions.modes)),
    threshold,
    minLines,
    minTokens,
    noSizePenalty: !sizePenalty,
    sameFileOnly: rawOptions.sameFileOnly,
    crossFileOnly: rawOptions.crossFileOnly,
    // Dots, case, repeats and an empty list are the library's to settle.
    extensions: parseCommaList(rawOptions.extensions),
    exclude: rawOptions.exclude ?? [],
    typesOnly: rawOptions.typesOnly,
    allowCrossKind: rawOptions.allowCrossKind,
    includeTypeLiterals: rawOptions.typeLiterals,
    overlapMinWindow,
    overlapMaxWindow,
    overlapSizeTolerance,
    format: rawOptions.format,
    output: rawOptions.output,
    failOnWarnings: Boolean(rawOptions.failOnWarnings),
    failOnDuplicates: Boolean(rawOptions.failOnDuplicates),
  };
}

export async function runCli(argv: string[], io: CliIO = console): Promise<number> {
  try {
    const program = buildProgram(io);
    program.parse(argv, { from: "user" });

    const paths = program.args.map(String);
    const rawOptions = program.opts<ParsedCliOptions>();
    const options = normalizeOptions(rawOptions);
    const report = await analyzeProject({
      paths,
      modes: options.modes,
      threshold: options.threshold,
      minLines: options.minLines,
      minTokens: options.minTokens,
      noSizePenalty: options.noSizePenalty,
      sameFileOnly: options.sameFileOnly,
      crossFileOnly: options.crossFileOnly,
      extensions: options.extensions,
      exclude: options.exclude,
      typesOnly: options.typesOnly,
      allowCrossKind: options.allowCrossKind,
      includeTypeLiterals: options.includeTypeLiterals,
      overlapMinWindow: options.overlapMinWindow,
      overlapMaxWindow: options.overlapMaxWindow,
      overlapSizeTolerance: options.overlapSizeTolerance,
    });

    const rendered = options.format === "json"
      ? formatJsonReport(report)
      : formatPrettyReport(report, process.cwd(), options.modes, {
          // Escape codes belong on a terminal that renders them: not in a
          // report written to a file, and not in piped output (`chalk`
          // looks at the standard output and at `NO_COLOR`/`FORCE_COLOR`).
          color: options.output === undefined && chalk.level > 0,
        });

    if (options.output) {
      // The analysis is already done — don't lose it to a missing parent
      // directory.
      await fs.mkdir(path.dirname(path.resolve(options.output)), { recursive: true });
      await fs.writeFile(options.output, `${rendered}\n`, "utf8");
      io.log(`Report written: ${options.output}`);
    } else {
      io.log(rendered);
    }

    const warnings = report.warnings;
    for (const warning of warnings) {
      io.error(formatWarning(warning, process.cwd()));
    }
    // Zero analyzed files is only an error when something went wrong
    // getting there (missing paths, unreadable files — i.e. a warning
    // fired). An empty or fully-ignored target with a clean run is a
    // legitimate no-op and exits 0.
    if (warnings.length > 0 && (report.stats.fileCount === 0 || options.failOnWarnings)) {
      return 1;
    }
    if (options.failOnDuplicates && report.stats.pairCount > 0) {
      return 1;
    }
    return 0;
  } catch (error) {
    if (error instanceof CommanderError) {
      // Commander already wrote the relevant text (help, version or the
      // parse error plus usage) through `configureOutput`; only the exit
      // code remains to be propagated.
      return error.exitCode;
    }
    io.error(errorMessage(error));
    return 1;
  }
}

export function isCliEntrypoint(argvPath: string | undefined, moduleUrl: string): boolean {
  if (!argvPath) {
    return false;
  }
  const currentFile = fileURLToPath(moduleUrl);
  // `process.argv[1]` may point to a symlink (e.g. `node_modules/.bin/similarity-ts`
  // when invoked through `npx`), while `import.meta.url` always resolves to the
  // real file path. Resolve the symlink so the comparison still succeeds.
  let resolvedArgv: string;
  try {
    resolvedArgv = realpathSync(argvPath);
  } catch {
    resolvedArgv = argvPath;
  }
  return path.resolve(currentFile) === path.resolve(resolvedArgv);
}

/* c8 ignore next 5 */
if (isCliEntrypoint(process.argv[1], import.meta.url)) {
  runCli(process.argv.slice(2)).then((exitCode) => {
    process.exitCode = exitCode;
  });
}
