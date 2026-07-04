import { access } from "node:fs/promises";
import { z } from "zod";
import type { LoadedFile } from "../types.js";

const locationSchema = z.object({
  filePath: z.string(),
  startLine: z.number(),
  endLine: z.number(),
  symbolName: z.string(),
  kind: z.string(),
});

const pairSchema = z.object({
  mode: z.string(),
  similarity: z.number(),
  left: locationSchema,
  right: locationSchema,
  details: z.record(z.string(), z.unknown()).optional(),
});

// Structural check on the WASM boundary: version skew between the JS
// wrapper and the engine otherwise surfaces as `undefined` accesses far
// from the cause.
const reportSchema = z.object({
  analyzedFiles: z.array(z.string()),
  skippedFiles: z.array(z.string()),
  warnings: z.array(z.object({ filePath: z.string().optional(), message: z.string() })),
  results: z.array(pairSchema),
  byMode: z.object({
    functions: z.array(pairSchema),
    types: z.array(pairSchema),
    classes: z.array(pairSchema),
    overlap: z.array(pairSchema),
  }),
  stats: z.object({
    fileCount: z.number(),
    pairCount: z.number(),
    elapsedMs: z.number(),
  }),
});

interface WasmAnalyzeInput {
  files: LoadedFile[];
  modes: string[];
  threshold: number;
  minLines: number;
  minTokens?: number;
  sizePenalty: boolean;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
  typesOnly: 'interface' | 'type' | 'all';
  allowCrossKind: boolean;
  includeTypeLiterals: boolean;
  overlapMinWindow: number;
  overlapMaxWindow: number;
  overlapSizeTolerance: number;
}

type WasmAnalyzeOutput = unknown;

type WasmModule = {
  analyze_project: (inputJson: string) => string;
};

let wasmModulePromise: Promise<WasmModule> | null = null;

async function loadWasmModule(): Promise<WasmModule> {
  if (!wasmModulePromise) {
    // Resolve candidates in priority order:
    // 1. Production: dist/cli.js or dist/index.js ↔ dist/wasm/similarity_wasm.js
    //    (packaged together; both sibling to `wasm/`).
    // 2. Dev/test: src/engine/wasm.ts ↔ dist/wasm/similarity_wasm.js (built via
    //    `npm run build:wasm`).
    const candidatePaths = [
      new URL('./wasm/similarity_wasm.js', import.meta.url),
      new URL('../../dist/wasm/similarity_wasm.js', import.meta.url),
    ];

    wasmModulePromise = (async () => {
      for (const wasmModulePath of candidatePaths) {
        try {
          await access(wasmModulePath);
        } catch {
          continue;
        }
        return (await import(wasmModulePath.href)) as WasmModule;
      }

      throw new Error(
        'WASM module was not found. Run `npm run build:wasm` to generate distribution assets before publishing.',
      );
    })().catch((error: unknown) => {
      wasmModulePromise = null;
      throw error;
    });
  }

  return wasmModulePromise;
}

export async function analyzeWithWasm(input: WasmAnalyzeInput): Promise<WasmAnalyzeOutput> {
  const wasm = await loadWasmModule();
  if (typeof wasm.analyze_project !== "function") {
    throw new Error("WASM module does not export analyze_project — rebuild with `npm run build:wasm`.");
  }
  const outputJson = wasm.analyze_project(JSON.stringify(input));
  const parsed: unknown = JSON.parse(outputJson);
  const validated = reportSchema.safeParse(parsed);
  if (!validated.success) {
    throw new Error(
      `WASM engine returned an unexpected report shape: ${validated.error.issues[0]?.message ?? "unknown"}. ` +
        "The engine and JS wrapper are likely out of sync — rebuild with `npm run build:wasm`.",
    );
  }
  return validated.data;
}
