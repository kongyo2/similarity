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
import { analyzeClasses } from "./analyzers/classes.js";
import { analyzeFunctions } from "./analyzers/functions.js";
import { analyzeOverlap } from "./analyzers/overlap.js";
import { analyzeTypes } from "./analyzers/types.js";
import type { AnalyzeProjectOptions, AnalyzeReport, AnalyzerMode, LoadedFile, SimilarityPair } from "./types.js";
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
  minTokens: number;
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

export function resolveAnalyzeOptions(input: AnalyzeProjectOptions): ResolvedAnalyzeOptions {
  if (!input.paths || input.paths.length === 0) {
    throw new Error("At least one path is required");
  }
  if (input.sameFileOnly && input.crossFileOnly) {
    throw new Error("Cannot use both sameFileOnly and crossFileOnly");
  }
  const threshold = input.threshold ?? DEFAULT_THRESHOLD;
  if (threshold < 0 || threshold > 1) {
    throw new Error("threshold must be between 0 and 1");
  }

  return {
    cwd: input.cwd ?? process.cwd(),
    paths: input.paths,
    modes: uniqueModes(input.modes),
    threshold,
    minLines: Math.max(1, input.minLines ?? DEFAULT_MIN_LINES),
    minTokens: Math.max(0, input.minTokens ?? 0),
    sizePenalty: !input.noSizePenalty,
    sameFileOnly: Boolean(input.sameFileOnly),
    crossFileOnly: Boolean(input.crossFileOnly),
    extensions: (input.extensions ?? DEFAULT_EXTENSIONS).map((extension) => extension.replace(/^\./, "").toLowerCase()),
    exclude: input.exclude ?? [],
    typesOnly: input.typesOnly ?? "all",
    allowCrossKind: Boolean(input.allowCrossKind),
    includeTypeLiterals: Boolean(input.includeTypeLiterals),
    overlapMinWindow: Math.max(1, input.overlapMinWindow ?? DEFAULT_OVERLAP_MIN_WINDOW),
    overlapMaxWindow: Math.max(1, input.overlapMaxWindow ?? DEFAULT_OVERLAP_MAX_WINDOW),
    overlapSizeTolerance: input.overlapSizeTolerance ?? DEFAULT_OVERLAP_SIZE_TOLERANCE,
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

function initializeByMode(): Record<AnalyzerMode, SimilarityPair[]> {
  return {
    functions: [],
    types: [],
    classes: [],
    overlap: [],
  };
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

  const byMode = initializeByMode();

  if (options.modes.includes("functions")) {
    byMode.functions = analyzeFunctions(
      files,
      {
        threshold: options.threshold,
        minLines: options.minLines,
        minTokens: options.minTokens,
        sizePenalty: options.sizePenalty,
        sameFileOnly: options.sameFileOnly,
        crossFileOnly: options.crossFileOnly,
      },
      warnings,
    );
  }

  if (options.modes.includes("types")) {
    byMode.types = analyzeTypes(
      files,
      {
        threshold: options.threshold,
        sameFileOnly: options.sameFileOnly,
        crossFileOnly: options.crossFileOnly,
        typesOnly: options.typesOnly,
        allowCrossKind: options.allowCrossKind,
        includeTypeLiterals: options.includeTypeLiterals,
      },
      warnings,
    );
  }

  if (options.modes.includes("classes")) {
    byMode.classes = analyzeClasses(
      files,
      {
        threshold: options.threshold,
        sameFileOnly: options.sameFileOnly,
        crossFileOnly: options.crossFileOnly,
      },
      warnings,
    );
  }

  if (options.modes.includes("overlap")) {
    byMode.overlap = analyzeOverlap(files, {
      threshold: options.threshold,
      minWindow: options.overlapMinWindow,
      maxWindow: options.overlapMaxWindow,
      sizeTolerance: options.overlapSizeTolerance,
      sameFileOnly: options.sameFileOnly,
      crossFileOnly: options.crossFileOnly,
    });
  }

  const results = [...byMode.functions, ...byMode.types, ...byMode.classes, ...byMode.overlap].sort(
    (left, right) => right.similarity - left.similarity,
  );

  return {
    analyzedFiles: files.map((file) => file.filePath),
    skippedFiles: collected.skipped,
    warnings: warnings.map((message) => ({ message })),
    results,
    byMode,
    stats: {
      fileCount: files.length,
      pairCount: results.length,
      elapsedMs: Date.now() - startTime,
    },
  };
}
