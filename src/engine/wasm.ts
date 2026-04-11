import type { LoadedFile } from '../types.js';
import { access } from 'node:fs/promises';

interface WasmAnalyzeInput {
  files: LoadedFile[];
  modes: string[];
  threshold: number;
}

interface WasmAnalyzeOutput {
  warnings?: string[];
}

type WasmModule = {
  analyze_project: (inputJson: string) => string;
};

let wasmModulePromise: Promise<WasmModule> | null = null;

async function loadWasmModule(): Promise<WasmModule> {
  if (!wasmModulePromise) {
    const candidatePaths = [
      new URL('../wasm/similarity_wasm.js', import.meta.url),
      new URL('../../native/similarity-wasm/pkg/similarity_wasm.js', import.meta.url),
      new URL('../native/similarity-wasm/pkg/similarity_wasm.js', import.meta.url),
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
