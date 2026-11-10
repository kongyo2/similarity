/** Every analysis mode, in the order reports list them. */
export const ANALYZER_MODES = ["functions", "types", "classes", "overlap"] as const;
/** What `typesOnly` accepts. */
export const TYPE_FILTERS = ["all", "interface", "type"] as const;

export const DEFAULT_MODES: readonly (typeof ANALYZER_MODES)[number][] = ANALYZER_MODES;
export const DEFAULT_EXTENSIONS = ["ts", "tsx", "mts", "cts"];
/**
 * Directories a scan does not enter, at any depth below the scanned
 * directory (gitignore syntax). A directory named explicitly as a target is
 * scanned whatever its name.
 */
export const DEFAULT_EXCLUDES = ["node_modules/", "dist/", "coverage/"];
export const DEFAULT_THRESHOLD = 0.8;
export const DEFAULT_MIN_LINES = 3;
export const DEFAULT_OVERLAP_MIN_WINDOW = 8;
export const DEFAULT_OVERLAP_MAX_WINDOW = 30;
/**
 * Largest overlap window, in AST nodes. Overlap mode looks for fragments
 * inside functions; a window of thousands of nodes is a whole function,
 * which function mode compares.
 */
export const MAX_OVERLAP_WINDOW = 1000;
export const DEFAULT_OVERLAP_SIZE_TOLERANCE = 0.2;
