import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { ANALYZER_MODES } from "../defaults.js";
import type {
  AnalyzeReport,
  AnalyzerMode,
  AnalyzerWarning,
  LoadedFile,
  TypeFilter,
} from "../types.js";
import { errorMessage } from "../utils/errors.js";

/**
 * The report the engine returns: everything in {@link AnalyzeReport}
 * except what the JavaScript layer derives (duplicate clusters, timing).
 */
export type EngineReport = Omit<AnalyzeReport, "clusters" | "stats"> & {
  stats: Omit<AnalyzeReport["stats"], "clusterCount" | "elapsedMs">;
};

/** What the engine analyzes and how. */
export interface EngineInput {
  files: LoadedFile[];
  modes: AnalyzerMode[];
  threshold: number;
  minLines: number;
  minTokens?: number;
  sizePenalty: boolean;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
  typesOnly: TypeFilter;
  allowCrossKind: boolean;
  includeTypeLiterals: boolean;
  overlapMinWindow: number;
  overlapMaxWindow: number;
  overlapSizeTolerance: number;
}

/** What the generated WebAssembly glue module exports. */
interface EngineModule {
  analyze_project(inputJson: string): string;
  check_source(filePath: string, content: string): string | undefined;
}

const REBUILD_HINT = "rebuild with `npm run build:wasm`";

// The glue module is CommonJS and instantiates the WebAssembly module when
// it is evaluated, so one evaluation is one engine instance.
const require = createRequire(import.meta.url);

/**
 * Where the glue module is: next to the bundle when packaged
 * (`dist/index.js` ↔ `dist/wasm/`), in `dist/` when running from source
 * (`src/engine/wasm.ts` ↔ `dist/wasm/`, built by `npm run build:wasm`).
 */
function engineModulePath(): string {
  const candidates = ["./wasm/similarity_wasm.js", "../../dist/wasm/similarity_wasm.js"].map(
    (relative) => fileURLToPath(new URL(relative, import.meta.url)),
  );
  const found = candidates.find((candidate) => existsSync(candidate));
  if (!found) {
    throw new Error(
      "WASM module was not found. Run `npm run build:wasm` to generate distribution assets before publishing.",
    );
  }
  return found;
}

let engine: EngineModule | undefined;

function loadEngine(): EngineModule {
  if (!engine) {
    const loaded = require(engineModulePath()) as Partial<EngineModule>;
    if (typeof loaded.analyze_project !== "function" || typeof loaded.check_source !== "function") {
      throw new Error(`WASM module does not export the engine entry points — ${REBUILD_HINT}.`);
    }
    engine = loaded as EngineModule;
  }
  return engine;
}

/**
 * Drop the engine instance, so the next call starts a fresh one. After a
 * trap the instance is in no known state: its stack pointer and allocator
 * are wherever the trap left them.
 */
function discardEngine(): void {
  delete require.cache[require.resolve(engineModulePath())];
  engine = undefined;
}

/**
 * Whether `error` is the engine failing rather than the engine refusing its
 * input. Running out of stack inside the module surfaces as a
 * `RangeError`, every other trap (out-of-bounds access, `unreachable`) as
 * a `WebAssembly.RuntimeError`; what the engine itself throws is a plain
 * `Error`.
 */
function isEngineCrash(error: unknown): boolean {
  return error instanceof RangeError || error instanceof WebAssembly.RuntimeError;
}

/**
 * After a call that threw: pass on what is no crash, and drop the instance
 * a crash leaves behind.
 */
function recoverFromCrash(error: unknown): void {
  if (!isEngineCrash(error)) {
    throw error;
  }
  discardEngine();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * Structural check on the WebAssembly boundary. The engine and this
 * wrapper ship together, but in a working tree `dist/wasm` can be older
 * than the sources; without the check that surfaces as `undefined`
 * accesses far from the cause.
 */
function assertEngineReport(report: unknown): asserts report is EngineReport {
  const problem = ((): string | undefined => {
    if (!isRecord(report)) {
      return "the report is not an object";
    }
    for (const key of ["analyzedFiles", "skippedFiles", "warnings", "results"]) {
      if (!Array.isArray(report[key])) {
        return `\`${key}\` is not an array`;
      }
    }
    const { byMode, stats } = report;
    if (!isRecord(byMode) || ANALYZER_MODES.some((mode) => !Array.isArray(byMode[mode]))) {
      return "`byMode` does not list every mode";
    }
    if (
      !isRecord(stats) ||
      typeof stats.fileCount !== "number" ||
      typeof stats.pairCount !== "number"
    ) {
      return "`stats` has no file and pair counts";
    }
    return undefined;
  })();
  if (problem) {
    throw new Error(
      `WASM engine returned an unexpected report shape: ${problem}. ` +
        `The engine and JS wrapper are likely out of sync — ${REBUILD_HINT}.`,
    );
  }
}

function runEngine(input: EngineInput): EngineReport {
  const report: unknown = JSON.parse(loadEngine().analyze_project(JSON.stringify(input)));
  assertEngineReport(report);
  return report;
}

/**
 * The files whose parse alone brings the engine down, with the warning for
 * each. The engine skips what it can tell is nested too deeply before it
 * parses it, but it parses with a recursive-descent parser on a fixed
 * stack, and some nesting has no bracket to count: a conditional or
 * `else if` chain thousands of arms long, say. Such a file cannot report
 * itself; it is found by trying the files one at a time.
 */
function findUnparsableFiles(files: LoadedFile[]): Map<string, AnalyzerWarning> {
  const unparsable = new Map<string, AnalyzerWarning>();
  for (const file of files) {
    try {
      // What the check returns — a syntax error, say — the analysis
      // reports itself; only whether it survives matters here.
      loadEngine().check_source(file.filePath, file.content);
    } catch (error) {
      recoverFromCrash(error);
      unparsable.set(file.filePath, {
        filePath: file.filePath,
        message: "skipped: code is nested too deeply to parse",
      });
    }
  }
  return unparsable;
}

/**
 * Run the engine on `input`.
 *
 * A file the engine cannot survive parsing is left out and reported as
 * skipped, like the files the engine skips itself.
 */
export function analyzeWithWasm(input: EngineInput): EngineReport {
  try {
    return runEngine(input);
  } catch (error) {
    recoverFromCrash(error);

    const unparsable = findUnparsableFiles(input.files);
    const bug = (reason: unknown) =>
      new Error(
        `The analysis engine crashed (${errorMessage(reason)}). ` +
          "This is a bug in similarity-ts; please report it at " +
          "https://github.com/kongyo2/similarity/issues with the input that triggers it.",
        { cause: reason },
      );
    if (unparsable.size === 0) {
      throw bug(error);
    }

    let report: EngineReport;
    try {
      report = runEngine({
        ...input,
        files: input.files.filter((file) => !unparsable.has(file.filePath)),
      });
    } catch (retryError) {
      recoverFromCrash(retryError);
      throw bug(retryError);
    }

    // Skipped files and their warnings keep the order of the input.
    const skipped = new Set(report.skippedFiles);
    const warningOf = new Map(report.warnings.map((warning) => [warning.filePath, warning]));
    const notAnalyzed = input.files.filter(
      (file) => skipped.has(file.filePath) || unparsable.has(file.filePath),
    );
    return {
      ...report,
      skippedFiles: notAnalyzed.map((file) => file.filePath),
      warnings: [
        ...notAnalyzed.flatMap(
          (file) => unparsable.get(file.filePath) ?? warningOf.get(file.filePath) ?? [],
        ),
        ...report.warnings.filter(
          (warning) => warning.filePath === undefined || !skipped.has(warning.filePath),
        ),
      ],
    };
  }
}
