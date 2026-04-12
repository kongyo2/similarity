import type { LoadedFile } from '../types.js';
import { access } from 'node:fs/promises';

interface WasmAnalyzeInput {
  files: LoadedFile[];
  modes: string[];
  threshold: number;
  minLines: number;
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
  const outputJson = wasm.analyze_project(JSON.stringify(input));
  return JSON.parse(outputJson) as WasmAnalyzeOutput;
}
