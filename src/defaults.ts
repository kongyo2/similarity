import type { AnalyzerMode } from "./types.js";

export const DEFAULT_MODES: AnalyzerMode[] = ["functions", "types", "classes", "overlap"];
export const DEFAULT_EXTENSIONS = ["ts", "tsx", "mts", "cts"];
export const DEFAULT_EXCLUDES = ["node_modules/**", "dist/**", "coverage/**"];
export const DEFAULT_THRESHOLD = 0.8;
export const DEFAULT_MIN_LINES = 3;
export const DEFAULT_OVERLAP_MIN_WINDOW = 5;
export const DEFAULT_OVERLAP_MAX_WINDOW = 30;
export const DEFAULT_OVERLAP_SIZE_TOLERANCE = 0.2;
