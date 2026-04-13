import fs from "node:fs/promises";
import os from "node:os";
import pLimit from "p-limit";
import {
  DEFAULT_EXTENSIONS,
  DEFAULT_MIN_LINES,
  DEFAULT_MODES,
  DEFAULT_OVERLAP_MAX_WINDOW,
  DEFAULT_OVERLAP_MIN_WINDOW,
  DEFAULT_OVERLAP_SIZE_TOLERANCE,
  DEFAULT_THRESHOLD,
} from "./defaults.js";
import { analyzeWithWasm } from "./engine/wasm.js";
import type { AnalyzeProjectOptions, AnalyzeReport, AnalyzerMode, LoadedFile } from "./types.js";
import { collectTypeScriptFiles } from "./utils/files.js";

function uniqueModes(modes: AnalyzerMode[] | undefined): AnalyzerMode[] {
  const resolved = modes && modes.length > 0 ? modes : DEFAULT_MODES;
  return [...new Set(resolved)];
}

export interface ResolvedAnalyzeOptions {
  cwd: string;
  paths: string[];
  modes: AnalyzerMode[];
  threshold: number;
  minLines: number;
  sizePenalty: boolean;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
  extensions: string[];
  exclude: string[];
  typesOnly: "interface" | "type" | "all";
  allowCrossKind: boolean;
  includeTypeLiterals: boolean;
  overlapMinWindow: number;
  overlapMaxWindow: number;
  overlapSizeTolerance: number;
}

function requirePositiveInteger(
  value: number | undefined,
  fallback: number,
  field: string,
): number {
  const resolved = value ?? fallback;
  if (!Number.isFinite(resolved) || !Number.isInteger(resolved)) {
    throw new Error(`${field} must be a positive integer`);
  }
  if (resolved < 1) {
    throw new Error(`${field} must be greater than or equal to 1`);
  }
  return resolved;
}

export function resolveAnalyzeOptions(input: AnalyzeProjectOptions): ResolvedAnalyzeOptions {
  if (!input.paths || input.paths.length === 0) {
    throw new Error("At least one path is required");
  }
  if (input.sameFileOnly && input.crossFileOnly) {
    throw new Error("Cannot use both sameFileOnly and crossFileOnly");
  }
  const threshold = input.threshold ?? DEFAULT_THRESHOLD;
  if (!Number.isFinite(threshold) || threshold < 0 || threshold > 1) {
    throw new Error("threshold must be between 0 and 1");
  }
  const overlapSizeTolerance = input.overlapSizeTolerance ?? DEFAULT_OVERLAP_SIZE_TOLERANCE;
  if (
    !Number.isFinite(overlapSizeTolerance) ||
    overlapSizeTolerance < 0 ||
    overlapSizeTolerance > 1
  ) {
    throw new Error("overlapSizeTolerance must be between 0 and 1");
  }
  const minLines = requirePositiveInteger(input.minLines, DEFAULT_MIN_LINES, "minLines");
  const overlapMinWindow = requirePositiveInteger(
    input.overlapMinWindow,
    DEFAULT_OVERLAP_MIN_WINDOW,
    "overlapMinWindow",
  );
  const overlapMaxWindow = requirePositiveInteger(
    input.overlapMaxWindow,
    DEFAULT_OVERLAP_MAX_WINDOW,
    "overlapMaxWindow",
  );
  if (overlapMinWindow > overlapMaxWindow) {
    throw new Error("overlapMinWindow must be less than or equal to overlapMaxWindow");
  }

  return {
    cwd: input.cwd ?? process.cwd(),
    paths: input.paths,
    modes: uniqueModes(input.modes),
    threshold,
    minLines,
    sizePenalty: !input.noSizePenalty,
    sameFileOnly: Boolean(input.sameFileOnly),
    crossFileOnly: Boolean(input.crossFileOnly),
    extensions: (input.extensions ?? DEFAULT_EXTENSIONS).map((extension) => extension.replace(/^\./, "").toLowerCase()),
    exclude: input.exclude ?? [],
    typesOnly: input.typesOnly ?? "all",
    allowCrossKind: Boolean(input.allowCrossKind),
    includeTypeLiterals: Boolean(input.includeTypeLiterals),
    overlapMinWindow,
    overlapMaxWindow,
    overlapSizeTolerance,
  };
}

async function loadFiles(filePaths: string[]): Promise<{ files: LoadedFile[]; warnings: string[] }> {
  const warnings: string[] = [];
  const limiter = pLimit(Math.max(4, Math.min(32, os.cpus().length)));
  const loaded = await Promise.all(
    filePaths.map((filePath) =>
      limiter(async () => {
        try {
          const content = await fs.readFile(filePath, "utf8");
          return { filePath, content } satisfies LoadedFile;
        } catch (error) {
          warnings.push(`Failed to read ${filePath}: ${(error as Error).message}`);
          return null;
        }
      }),
    ),
  );
  return { files: loaded.filter((entry): entry is LoadedFile => entry !== null), warnings };
}

export async function analyzeProject(input: AnalyzeProjectOptions): Promise<AnalyzeReport> {
  const startTime = Date.now();
  const options = resolveAnalyzeOptions(input);
  const warnings: string[] = [];

  const collected = await collectTypeScriptFiles({
    cwd: options.cwd,
    paths: options.paths,
    extensions: options.extensions,
    exclude: options.exclude,
  });
  warnings.push(...collected.warnings);

  const { files, warnings: readWarnings } = await loadFiles(collected.files);
  warnings.push(...readWarnings);

  const wasmReport = (await analyzeWithWasm({
    files,
    modes: options.modes,
    threshold: options.threshold,
    minLines: options.minLines,
    sizePenalty: options.sizePenalty,
    sameFileOnly: options.sameFileOnly,
    crossFileOnly: options.crossFileOnly,
    typesOnly: options.typesOnly,
    allowCrossKind: options.allowCrossKind,
    includeTypeLiterals: options.includeTypeLiterals,
    overlapMinWindow: options.overlapMinWindow,
    overlapMaxWindow: options.overlapMaxWindow,
    overlapSizeTolerance: options.overlapSizeTolerance,
  })) as AnalyzeReport;

  return {
    ...wasmReport,
    skippedFiles: collected.skipped,
    warnings: [...(wasmReport.warnings ?? []), ...warnings.map((message) => ({ message }))],
    stats: {
      ...wasmReport.stats,
      elapsedMs: Date.now() - startTime,
    },
  };
}
