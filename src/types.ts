export type AnalyzerMode = "functions" | "types" | "classes" | "overlap";

export interface AnalyzerLocation {
  filePath: string;
  startLine: number;
  endLine: number;
  symbolName: string;
  kind: string;
}

export interface SimilarityPair {
  mode: AnalyzerMode;
  similarity: number;
  left: AnalyzerLocation;
  right: AnalyzerLocation;
  details?: Record<string, unknown>;
}

export interface AnalyzerWarning {
  filePath?: string;
  message: string;
}

export interface AnalyzeProjectOptions {
  paths: string[];
  cwd?: string;
  modes?: AnalyzerMode[];
  threshold?: number;
  minLines?: number;
  noSizePenalty?: boolean;
  sameFileOnly?: boolean;
  crossFileOnly?: boolean;
  extensions?: string[];
  exclude?: string[];
  typesOnly?: "interface" | "type" | "all";
  /**
   * When true (the default), an `interface` and a structurally identical
   * `type` alias are eligible to match. Set to `false` to restrict the
   * comparison to same-kind pairs only.
   */
  allowCrossKind?: boolean;
  includeTypeLiterals?: boolean;
  overlapMinWindow?: number;
  overlapMaxWindow?: number;
  overlapSizeTolerance?: number;
}

export interface LoadedFile {
  filePath: string;
  content: string;
}

export interface AnalyzeReport {
  analyzedFiles: string[];
  skippedFiles: string[];
  warnings: AnalyzerWarning[];
  results: SimilarityPair[];
  byMode: Record<AnalyzerMode, SimilarityPair[]>;
  stats: {
    fileCount: number;
    pairCount: number;
    elapsedMs: number;
  };
}
