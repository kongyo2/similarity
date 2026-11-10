export { analyzeProject, resolveAnalyzeOptions } from "./analyze.js";
export type { ResolvedAnalyzeOptions } from "./analyze.js";
export {
  ANALYZER_MODES,
  DEFAULT_EXCLUDES,
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
export { formatJsonReport, formatPrettyReport } from "./format.js";
export type { PrettyReportOptions } from "./format.js";
export type {
  AnalyzeProjectOptions,
  AnalyzeReport,
  AnalyzerLocation,
  AnalyzerMode,
  AnalyzerWarning,
  SimilarityCluster,
  SimilarityPair,
  TypeFilter,
} from "./types.js";
