import fs from "node:fs/promises";
import os from "node:os";
import pLimit from "p-limit";
import {
  ANALYZER_MODES,
  DEFAULT_EXTENSIONS,
  DEFAULT_MIN_LINES,
  DEFAULT_MODES,
  DEFAULT_OVERLAP_MAX_WINDOW,
  DEFAULT_OVERLAP_MIN_WINDOW,
  DEFAULT_OVERLAP_SIZE_TOLERANCE,
  DEFAULT_THRESHOLD,
  MAX_OVERLAP_WINDOW,
  TYPE_FILTERS,
} from "./defaults.js";
import { buildClusters } from "./clusters.js";
import { analyzeWithWasm } from "./engine/wasm.js";
import type {
  AnalyzeProjectOptions,
  AnalyzeReport,
  AnalyzerMode,
  AnalyzerWarning,
  LoadedFile,
  TypeFilter,
} from "./types.js";
import { collectTypeScriptFiles } from "./utils/files.js";

/**
 * The modes to run: the requested ones without repeats — a mode listed
 * twice would be reported twice — or all of them when none is requested.
 */
export function resolveModes(modes: readonly string[] | undefined): AnalyzerMode[] {
  if (!modes || modes.length === 0) {
    return [...DEFAULT_MODES];
  }
  for (const mode of modes) {
    if (!(ANALYZER_MODES as readonly string[]).includes(mode)) {
      throw new Error(`Unknown mode "${mode}" (expected one of: ${ANALYZER_MODES.join(", ")})`);
    }
  }
  return [...new Set(modes as readonly AnalyzerMode[])];
}

function resolveTypeFilter(typesOnly: string | undefined): TypeFilter {
  if (typesOnly === undefined) {
    return "all";
  }
  if (!(TYPE_FILTERS as readonly string[]).includes(typesOnly)) {
    throw new Error(`typesOnly must be one of: ${TYPE_FILTERS.join(", ")}`);
  }
  return typesOnly as TypeFilter;
}

/** {@link AnalyzeProjectOptions} with every default filled in. */
export interface ResolvedAnalyzeOptions {
  cwd: string;
  paths: string[];
  modes: AnalyzerMode[];
  threshold: number;
  minLines: number;
  minTokens?: number;
  sizePenalty: boolean;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
  extensions: string[];
  exclude: string[];
  typesOnly: TypeFilter;
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
  const minTokens = input.minTokens === undefined
    ? undefined
    : requirePositiveInteger(input.minTokens, 1, "minTokens");
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
  if (overlapMaxWindow > MAX_OVERLAP_WINDOW) {
    throw new Error(`overlapMaxWindow must be less than or equal to ${MAX_OVERLAP_WINDOW}`);
  }

  return {
    cwd: input.cwd ?? process.cwd(),
    paths: input.paths,
    modes: resolveModes(input.modes),
    threshold,
    minLines,
    minTokens,
    sizePenalty: !input.noSizePenalty,
    sameFileOnly: Boolean(input.sameFileOnly),
    crossFileOnly: Boolean(input.crossFileOnly),
    // An explicitly-empty extension list behaves like "unspecified" (the
    // CLI normalizes `--extensions ""` the same way) instead of silently
    // matching zero files.
    extensions: (input.extensions?.length ? input.extensions : DEFAULT_EXTENSIONS).map(
      (extension) => extension.replace(/^\./, "").toLowerCase(),
    ),
    exclude: input.exclude ?? [],
    typesOnly: resolveTypeFilter(input.typesOnly),
    // Default to cross-kind comparison so an `interface User` and a
    // structurally identical `type User = {...}` show up as a refactoring
    // candidate. Pass `false` explicitly when the caller wants to restrict
    // results to same-kind pairs only.
    allowCrossKind: input.allowCrossKind ?? true,
    includeTypeLiterals: Boolean(input.includeTypeLiterals),
    overlapMinWindow,
    overlapMaxWindow,
    overlapSizeTolerance,
  };
}

async function loadFiles(
  filePaths: string[],
): Promise<{ files: LoadedFile[]; unreadable: AnalyzerWarning[] }> {
  const limiter = pLimit(Math.max(4, Math.min(32, os.availableParallelism())));
  const loaded = await Promise.all(
    filePaths.map((filePath) =>
      limiter(async (): Promise<LoadedFile | AnalyzerWarning> => {
        try {
          return { filePath, content: await fs.readFile(filePath, "utf8") };
        } catch (error) {
          return { filePath, message: `Failed to read ${filePath}: ${(error as Error).message}` };
        }
      }),
    ),
  );
  // In the order of `filePaths`, whatever order the reads finished in.
  return {
    files: loaded.filter((entry): entry is LoadedFile => "content" in entry),
    unreadable: loaded.filter((entry): entry is AnalyzerWarning => "message" in entry),
  };
}

export async function analyzeProject(input: AnalyzeProjectOptions): Promise<AnalyzeReport> {
  const startTime = Date.now();
  const options = resolveAnalyzeOptions(input);

  const collected = await collectTypeScriptFiles({
    cwd: options.cwd,
    paths: options.paths,
    extensions: options.extensions,
    exclude: options.exclude,
  });
  const { files, unreadable } = await loadFiles(collected.files);

  const engineReport = analyzeWithWasm({
    files,
    modes: options.modes,
    threshold: options.threshold,
    minLines: options.minLines,
    minTokens: options.minTokens,
    sizePenalty: options.sizePenalty,
    sameFileOnly: options.sameFileOnly,
    crossFileOnly: options.crossFileOnly,
    typesOnly: options.typesOnly,
    allowCrossKind: options.allowCrossKind,
    includeTypeLiterals: options.includeTypeLiterals,
    overlapMinWindow: options.overlapMinWindow,
    overlapMaxWindow: options.overlapMaxWindow,
    overlapSizeTolerance: options.overlapSizeTolerance,
  });

  const clusters = buildClusters(engineReport.byMode);
  return {
    ...engineReport,
    // Everything that was found or named and not analyzed: files the
    // engine could not parse, files that could not be read, and files an
    // ignore rule or the extension filter left out.
    skippedFiles: [
      ...engineReport.skippedFiles,
      ...unreadable.flatMap((warning) => warning.filePath ?? []),
      ...collected.skipped,
    ],
    warnings: [
      ...engineReport.warnings,
      ...collected.warnings.map((message) => ({ message })),
      ...unreadable,
    ],
    clusters,
    stats: {
      ...engineReport.stats,
      clusterCount: clusters.length,
      elapsedMs: Date.now() - startTime,
    },
  };
}
