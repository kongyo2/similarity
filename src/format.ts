import { Chalk, type ChalkInstance } from "chalk";
import Table from "cli-table3";
// The release the table library itself measures cells with: a line this
// module makes to fit, the table sees as fitting.
import stringWidth from "string-width";
import { standalonePairs } from "./clusters.js";
import type {
  AnalyzeReport,
  AnalyzerLocation,
  AnalyzerMode,
  AnalyzerWarning,
  SimilarityCluster,
  SimilarityPair,
} from "./types.js";
import { toRelativePath } from "./utils/path.js";

export interface PrettyReportOptions {
  /**
   * Style the report with ANSI escape codes. Off by default: the text is
   * as fit for a file or a pipe as for a terminal.
   */
  color?: boolean;
}

/** How the report is styled: with escape codes, or not at all. */
interface Styles {
  bold: (text: string) => string;
  warning: (text: string) => string;
  /** cli-table3 colors for the header row and the borders. */
  table: { head: string[]; border: string[] };
}

function stylesFor(color: boolean): Styles {
  // An instance of its own: the level of the shared `chalk` follows the
  // process's standard output, which is not necessarily where the report
  // goes.
  const paint: ChalkInstance = new Chalk({ level: color ? 1 : 0 });
  return {
    bold: (text) => paint.bold(text),
    warning: (text) => paint.yellow(text),
    table: color ? { head: ["cyan"], border: ["grey"] } : { head: [], border: [] },
  };
}

/** Width of the pair table's columns, borders excluded, padding included. */
const COLUMN_WIDTHS = { similarity: 12, location: 56 };
/** Columns of text a location cell holds: its width less the padding. */
const LOCATION_TEXT_WIDTH = COLUMN_WIDTHS.location - 2;

function formatLocation(location: AnalyzerLocation, cwd: string): string {
  const relativePath = toRelativePath(location.filePath, cwd);
  return `${relativePath}:${location.startLine}-${location.endLine} ${location.symbolName}`;
}

/**
 * `word` in pieces of at most `width` columns, each ending after a path
 * separator where one fits.
 */
function breakWord(word: string, width: number): string[] {
  const pieces: string[] = [];
  let piece = "";
  let pieceWidth = 0;
  // Length of `piece` up to and including its last path separator.
  let separatorEnd = 0;
  for (const character of word) {
    const characterWidth = stringWidth(character);
    // Twice at most: what follows the separator can still be too wide for
    // a character of two columns, and then goes out whole.
    while (piece.length > 0 && pieceWidth + characterWidth > width) {
      const cut = separatorEnd > 0 ? separatorEnd : piece.length;
      pieces.push(piece.slice(0, cut));
      piece = piece.slice(cut);
      pieceWidth = stringWidth(piece);
      separatorEnd = 0;
    }
    piece += character;
    pieceWidth += characterWidth;
    if (character === "/" || character === "\\") {
      separatorEnd = piece.length;
    }
  }
  pieces.push(piece);
  return pieces;
}

/**
 * `text` with a line break inside every word wider than `width` columns.
 * The table wraps between words and cuts off a word it cannot fit, which
 * for a location is the end of the path: the file name and the lines.
 */
function wrapLongWords(text: string, width: number): string {
  return text
    .split(" ")
    .map((word) => (stringWidth(word) > width ? breakWord(word, width).join("\n") : word))
    .join(" ");
}

/** How a mode is called in the report: its section, and what it pairs. */
const MODE_WORDING: Record<AnalyzerMode, { heading: string; members: string }> = {
  functions: { heading: "Function Similarity", members: "functions" },
  types: { heading: "Type Similarity", members: "types" },
  classes: { heading: "Class Similarity", members: "classes" },
  overlap: { heading: "Overlap Detection", members: "fragments" },
};

/**
 * The wording for `mode`. A report is data, and may name a mode this
 * version does not know: it gets a section under its own name.
 */
function wordingOf(mode: AnalyzerMode): { heading: string; members: string } {
  const known: Partial<typeof MODE_WORDING> = MODE_WORDING;
  return known[mode] ?? { heading: mode, members: "declarations" };
}

/**
 * A warning as one line. A warning about a file names the file, relative
 * to `cwd` — "skipped: Parse errors: …" on its own says nothing about
 * which — unless the message names it already.
 */
export function formatWarning(warning: AnalyzerWarning, cwd: string): string {
  if (warning.filePath === undefined || warning.message.includes(warning.filePath)) {
    return warning.message;
  }
  return `${toRelativePath(warning.filePath, cwd)}: ${warning.message}`;
}

function plural(count: number, singular: string, pluralForm = `${singular}s`): string {
  return `${count} ${count === 1 ? singular : pluralForm}`;
}

function renderCluster(
  cluster: SimilarityCluster,
  index: number,
  mode: AnalyzerMode,
  cwd: string,
  styles: Styles,
): string[] {
  // One number says it all when every pair scores the same.
  const similarity = cluster.minSimilarity === cluster.maxSimilarity
    ? cluster.maxSimilarity.toFixed(3)
    : `${cluster.minSimilarity.toFixed(3)}–${cluster.maxSimilarity.toFixed(3)} ` +
      `(avg ${cluster.averageSimilarity.toFixed(3)})`;
  const shape = cluster.fragment
    ? ` (${cluster.fragment.nodeType}, ${plural(cluster.fragment.nodeCount, "node")})`
    : "";
  const lines = [
    styles.bold(
      `Cluster ${index + 1}: ${cluster.members.length} ${wordingOf(mode).members}${shape}, ` +
        `${plural(cluster.pairCount, "pairwise match", "pairwise matches")}, ` +
        `similarity ${similarity}`,
    ),
  ];
  for (const member of cluster.members) {
    lines.push(`  ${formatLocation(member, cwd)}`);
  }
  return lines;
}

function renderPairTable(pairs: SimilarityPair[], cwd: string, styles: Styles): string {
  const table = new Table({
    head: ["Similarity", "Left", "Right"],
    style: styles.table,
    wordWrap: true,
    colWidths: [COLUMN_WIDTHS.similarity, COLUMN_WIDTHS.location, COLUMN_WIDTHS.location],
  });
  const cell = (location: AnalyzerLocation) =>
    wrapLongWords(formatLocation(location, cwd), LOCATION_TEXT_WIDTH);
  for (const pair of pairs) {
    table.push([pair.similarity.toFixed(3), cell(pair.left), cell(pair.right)]);
  }
  return table.toString();
}

function renderMode(
  mode: AnalyzerMode,
  pairs: SimilarityPair[],
  reportClusters: SimilarityCluster[],
  cwd: string,
  styles: Styles,
): string {
  const lines: string[] = [];
  lines.push(styles.bold(`=== ${wordingOf(mode).heading} ===`));
  if (pairs.length === 0) {
    lines.push("No similar pairs found.");
    lines.push("");
    return lines.join("\n");
  }

  // Three or more copies of one declaration — or, in overlap mode, of one
  // fragment — are listed once, as a cluster, instead of as every pair
  // among them. The clusters are the report's own, so the blocks agree
  // with the summary's cluster count and with the JSON report.
  const clusters = reportClusters.filter((cluster) => cluster.mode === mode);
  const standalone = standalonePairs(mode, pairs, clusters);
  clusters.forEach((cluster, index) => {
    lines.push(...renderCluster(cluster, index, mode, cwd, styles));
    lines.push("");
  });
  if (standalone.length > 0) {
    lines.push(renderPairTable(standalone, cwd, styles));
  }

  if (clusters.length === 0) {
    lines.push(`Total pairs: ${pairs.length}`);
  } else {
    const clustered = pairs.length - standalone.length;
    lines.push(
      `Total pairs: ${pairs.length} (${clustered} in ${plural(clusters.length, "cluster")}, ` +
        `${standalone.length} standalone)`,
    );
  }
  lines.push("");
  return lines.join("\n");
}

/**
 * The report as text for people: a summary, then one section per mode in
 * `modes`, then the warnings. Paths are shown relative to `cwd`.
 */
export function formatPrettyReport(
  report: AnalyzeReport,
  cwd: string,
  modes: readonly AnalyzerMode[],
  options: PrettyReportOptions = {},
): string {
  const styles = stylesFor(options.color ?? false);
  const lines: string[] = [];
  lines.push(styles.bold("Analyzing code similarity..."));
  lines.push(`Files analyzed: ${report.stats.fileCount}`);
  lines.push(`Pairs detected: ${report.stats.pairCount}`);
  if (report.clusters.length > 0) {
    lines.push(`Duplicate clusters: ${report.clusters.length}`);
  }
  lines.push(`Elapsed: ${report.stats.elapsedMs} ms`);
  lines.push("");

  for (const mode of modes) {
    // Guard against engine/report drift: a requested mode missing from
    // `byMode` renders as an empty section instead of crashing.
    lines.push(renderMode(mode, report.byMode[mode] ?? [], report.clusters, cwd, styles));
  }

  if (report.warnings.length > 0) {
    lines.push(styles.warning("Warnings:"));
    for (const warning of report.warnings) {
      lines.push(`- ${formatWarning(warning, cwd)}`);
    }
  }

  return lines.join("\n").trimEnd();
}

export function formatJsonReport(report: AnalyzeReport): string {
  return JSON.stringify(report, null, 2);
}
