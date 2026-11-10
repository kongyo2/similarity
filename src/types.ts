import type { ANALYZER_MODES, TYPE_FILTERS } from "./defaults.js";

export type AnalyzerMode = (typeof ANALYZER_MODES)[number];
/** Which declarations type mode compares. */
export type TypeFilter = (typeof TYPE_FILTERS)[number];

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

/**
 * What the members of an overlap cluster are copies of: a fragment of
 * `nodeCount` syntax nodes under a `nodeType` node (`IfStatement`,
 * `VariableDeclaration`, …).
 */
export interface SimilarityFragment {
  nodeCount: number;
  nodeType: string;
}

/**
 * A group of three or more declarations — in overlap mode, fragments of
 * functions — connected by reported pairs of one mode: the copies a
 * refactor would consolidate into one.
 */
export interface SimilarityCluster {
  mode: AnalyzerMode;
  /** Every member of the group, in path/line order. */
  members: AnalyzerLocation[];
  /** In overlap mode, the fragment the members are copies of. */
  fragment?: SimilarityFragment;
  /** Reported pairs between members. */
  pairCount: number;
  maxSimilarity: number;
  minSimilarity: number;
  averageSimilarity: number;
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
  /**
   * Minimum function size in AST nodes (measured on the same normalized
   * tree the comparison scores). When set it replaces the line-count
   * gate; ~50 is a good starting point for noisy codebases.
   */
  minTokens?: number;
  noSizePenalty?: boolean;
  sameFileOnly?: boolean;
  crossFileOnly?: boolean;
  extensions?: string[];
  exclude?: string[];
  typesOnly?: TypeFilter;
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
  /**
   * Duplicate clusters of every mode (see {@link SimilarityCluster}), in
   * mode order; their pairs stay listed in `byMode`.
   */
  clusters: SimilarityCluster[];
  stats: {
    fileCount: number;
    pairCount: number;
    clusterCount: number;
    elapsedMs: number;
  };
}
