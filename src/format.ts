import { Chalk, type ChalkInstance } from "chalk";
import Table from "cli-table3";
import { standalonePairs } from "./clusters.js";
import type {
  AnalyzeReport,
  AnalyzerLocation,
  AnalyzerMode,
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

function formatLocation(location: AnalyzerLocation, cwd: string): string {
  const relativePath = toRelativePath(location.filePath, cwd);
  return `${relativePath}:${location.startLine}-${location.endLine} ${location.symbolName}`;
}

function formatModeLabel(mode: AnalyzerMode): string {
  switch (mode) {
    case "functions":
      return "Function Similarity";
    case "types":
      return "Type Similarity";
    case "classes":
      return "Class Similarity";
    case "overlap":
      return "Overlap Detection";
    default:
      return mode;
  }
}

function memberNoun(mode: AnalyzerMode): string {
  switch (mode) {
    case "functions":
      return "functions";
    case "types":
      return "types";
    case "classes":
      return "classes";
    default:
      return "declarations";
  }
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
  const range = cluster.minSimilarity === cluster.maxSimilarity
    ? cluster.maxSimilarity.toFixed(3)
    : `${cluster.minSimilarity.toFixed(3)}–${cluster.maxSimilarity.toFixed(3)}`;
  const lines = [
    styles.bold(
      `Cluster ${index + 1}: ${cluster.members.length} ${memberNoun(mode)}, ` +
        `${plural(cluster.pairCount, "pairwise match", "pairwise matches")}, ` +
        `similarity ${range} (avg ${cluster.averageSimilarity.toFixed(3)})`,
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
    colWidths: [12, 56, 56],
  });
  for (const pair of pairs) {
    table.push([
      pair.similarity.toFixed(3),
      formatLocation(pair.left, cwd),
      formatLocation(pair.right, cwd),
    ]);
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
  lines.push(styles.bold(`=== ${formatModeLabel(mode)} ===`));
  if (pairs.length === 0) {
    lines.push("No similar pairs found.");
    lines.push("");
    return lines.join("\n");
  }

  // Three or more copies of one declaration are listed once, as a
  // cluster, instead of as every pair among them. The clusters are the
  // report's own, so the blocks agree with the summary's cluster count
  // and with the JSON report.
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
      // A warning about a file says which file: "skipped: Parse errors: …"
      // on its own names none.
      const subject = warning.filePath === undefined ? "" : toRelativePath(warning.filePath, cwd);
      lines.push(
        subject === "" || warning.message.includes(warning.filePath as string)
          ? `- ${warning.message}`
          : `- ${subject}: ${warning.message}`,
      );
    }
  }

  return lines.join("\n").trimEnd();
}

export function formatJsonReport(report: AnalyzeReport): string {
  return JSON.stringify(report, null, 2);
}
