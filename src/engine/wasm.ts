import path from 'node:path';
import type { LoadedFile } from '../types.js';

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
    const wasmModulePath = path.resolve(process.cwd(), 'native/similarity-wasm/pkg/similarity_wasm.js');
    wasmModulePromise = import(wasmModulePath) as Promise<WasmModule>;
  }
  return wasmModulePromise;
}

export async function analyzeWithWasm(input: WasmAnalyzeInput): Promise<WasmAnalyzeOutput> {
  const wasm = await loadWasmModule();
  const outputJson = wasm.analyze_project(JSON.stringify(input));
  return JSON.parse(outputJson) as WasmAnalyzeOutput;
}
