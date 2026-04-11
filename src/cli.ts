#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Command, Option } from "commander";
import { z } from "zod";
import { analyzeProject } from "./analyze.js";
import {
  DEFAULT_EXTENSIONS,
  DEFAULT_MODES,
  DEFAULT_MIN_LINES,
  DEFAULT_OVERLAP_MAX_WINDOW,
  DEFAULT_OVERLAP_MIN_WINDOW,
  DEFAULT_OVERLAP_SIZE_TOLERANCE,
  DEFAULT_THRESHOLD,
} from "./defaults.js";
import { formatJsonReport, formatPrettyReport } from "./format.js";
import type { AnalyzerMode } from "./types.js";

interface CliIO {
  log: (message: string) => void;
  error: (message: string) => void;
}

interface ParsedCliOptions {
  modes: string;
  threshold: number;
  minLines: number;
  minTokens: number;
  noSizePenalty: boolean;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
  extensions: string;
  exclude: string[];
  typesOnly: "interface" | "type" | "all";
  allowCrossKind: boolean;
  typeLiterals: boolean;
  overlapMinWindow: number;
  overlapMaxWindow: number;
  overlapSizeTolerance: number;
  format: "pretty" | "json";
  output?: string;
}

const modeSchema = z.enum(["functions", "types", "classes", "overlap"]);

function parseCommaList(rawValue: string): string[] {
  return rawValue
    .split(",")
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

function parseModes(rawModes: string): AnalyzerMode[] {
  const parsed = parseCommaList(rawModes);
  if (parsed.length === 0) {
    return DEFAULT_MODES;
  }
  return parsed.map((mode) => modeSchema.parse(mode));
}

function parseExtensions(rawExtensions: string): string[] {
  const parsed = parseCommaList(rawExtensions).map((extension) =>
    extension.replace(/^\./, "").toLowerCase(),
  );
  if (parsed.length === 0) {
    return DEFAULT_EXTENSIONS;
  }
  return [...new Set(parsed)];
}

function buildProgram(): Command {
  const program = new Command();
  program
    .name("similarity-ts")
    .description("TypeScript code similarity analyzer")
    .argument("<paths...>", "Files and directories to analyze")
    .option(
      "--modes <list>",
      "Comma-separated modes: functions,types,classes,overlap",
      DEFAULT_MODES.join(","),
    )
    .option("-t, --threshold <number>", "Similarity threshold (0-1)", String(DEFAULT_THRESHOLD))
    .option("--min-lines <number>", "Minimum function line count", String(DEFAULT_MIN_LINES))
    .option("--min-tokens <number>", "Minimum function token count", "0")
    .option("--no-size-penalty", "Disable line-count size penalty for function mode", false)
    .option("--same-file-only", "Only compare symbols from the same file", false)
    .option("--cross-file-only", "Only compare symbols across different files", false)
    .option(
      "--extensions <list>",
      "Comma-separated extensions (default: ts,tsx,mts,cts)",
      DEFAULT_EXTENSIONS.join(","),
    )
    .option("--exclude <pattern>", "Exclude glob pattern (repeatable)", (value, previous: string[]) => {
      previous.push(value);
      return previous;
    }, [])
    .addOption(new Option("--types-only <kind>", "Type mode filter").choices(["all", "interface", "type"]).default("all"))
    .option("--allow-cross-kind", "Allow type comparisons across kind boundaries", false)
    .option("--type-literals", "Include anonymous type literals in type mode", false)
    .option("--overlap-min-window <number>", "Overlap mode minimum token window", String(DEFAULT_OVERLAP_MIN_WINDOW))
    .option("--overlap-max-window <number>", "Overlap mode maximum token window", String(DEFAULT_OVERLAP_MAX_WINDOW))
    .option(
      "--overlap-size-tolerance <number>",
      "Allowed segment-size ratio difference in overlap mode",
      String(DEFAULT_OVERLAP_SIZE_TOLERANCE),
    )
    .addOption(new Option("--format <format>", "Output format").choices(["pretty", "json"]).default("pretty"))
    .option("--output <path>", "Write report to file")
    .showHelpAfterError(true);
  return program;
}

function parseNumber(value: string, field: string): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    throw new Error(`${field} must be a valid number`);
  }
  return parsed;
}

function normalizeOptions(rawOptions: ParsedCliOptions) {
  const threshold = parseNumber(String(rawOptions.threshold), "threshold");
  const minLines = Math.max(1, parseInt(String(rawOptions.minLines), 10));
  const minTokens = Math.max(0, parseInt(String(rawOptions.minTokens), 10));
  const overlapMinWindow = Math.max(1, parseInt(String(rawOptions.overlapMinWindow), 10));
  const overlapMaxWindow = Math.max(1, parseInt(String(rawOptions.overlapMaxWindow), 10));
  const overlapSizeTolerance = parseNumber(String(rawOptions.overlapSizeTolerance), "overlap-size-tolerance");

  if (threshold < 0 || threshold > 1) {
    throw new Error("threshold must be between 0 and 1");
  }
  if (overlapSizeTolerance < 0 || overlapSizeTolerance > 1) {
    throw new Error("overlap-size-tolerance must be between 0 and 1");
  }
  if (rawOptions.sameFileOnly && rawOptions.crossFileOnly) {
    throw new Error("Cannot use both --same-file-only and --cross-file-only");
  }

  return {
    modes: parseModes(rawOptions.modes),
    threshold,
    minLines,
    minTokens,
    noSizePenalty: rawOptions.noSizePenalty,
    sameFileOnly: rawOptions.sameFileOnly,
    crossFileOnly: rawOptions.crossFileOnly,
    extensions: parseExtensions(rawOptions.extensions),
    exclude: rawOptions.exclude,
    typesOnly: rawOptions.typesOnly,
    allowCrossKind: rawOptions.allowCrossKind,
    includeTypeLiterals: rawOptions.typeLiterals,
    overlapMinWindow,
    overlapMaxWindow,
    overlapSizeTolerance,
    format: rawOptions.format,
    output: rawOptions.output,
  };
}

export async function runCli(argv: string[], io: CliIO = console): Promise<number> {
  try {
    const program = buildProgram();
    program.parse(argv, { from: "user" });

    const paths = program.processedArgs.flatMap((value) => {
      if (Array.isArray(value)) {
        return value.map(String);
      }
      return [String(value)];
    });
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
      : formatPrettyReport(report, process.cwd(), options.modes);

    if (options.output) {
      await fs.writeFile(options.output, `${rendered}\n`, "utf8");
      io.log(`Report written: ${options.output}`);
    } else {
      io.log(rendered);
    }
    return 0;
  } catch (error) {
    io.error((error as Error).message);
    return 1;
  }
}

const isMainModule = (() => {
  if (!process.argv[1]) {
    return false;
  }
  const currentFile = fileURLToPath(import.meta.url);
  return path.resolve(currentFile) === path.resolve(process.argv[1]);
})();

if (isMainModule) {
  runCli(process.argv.slice(2)).then((exitCode) => {
    process.exitCode = exitCode;
  });
}
