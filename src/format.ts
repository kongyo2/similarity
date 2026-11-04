import chalk from "chalk";
import Table from "cli-table3";
import { CLUSTERED_MODES, clusterPairs } from "./clusters.js";
import type { AnalyzeReport, AnalyzerLocation, AnalyzerMode, SimilarityCluster, SimilarityPair } from "./types.js";
import { toRelativePath } from "./utils/path.js";

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
): string[] {
  const range = cluster.minSimilarity === cluster.maxSimilarity
    ? cluster.maxSimilarity.toFixed(3)
    : `${cluster.minSimilarity.toFixed(3)}–${cluster.maxSimilarity.toFixed(3)}`;
  const lines = [
    chalk.bold(
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

function renderPairTable(pairs: SimilarityPair[], cwd: string): string {
  const table = new Table({
    head: ["Similarity", "Left", "Right"],
    style: { head: ["cyan"] },
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

function renderMode(mode: AnalyzerMode, pairs: SimilarityPair[], cwd: string): string {
  const lines: string[] = [];
  lines.push(chalk.bold(`=== ${formatModeLabel(mode)} ===`));
  if (pairs.length === 0) {
    lines.push("No similar pairs found.");
    lines.push("");
    return lines.join("\n");
  }

  // Three or more copies of one declaration are listed once, as a
  // cluster, instead of as every pair among them.
  const { clusters, standalone } = CLUSTERED_MODES.includes(mode)
    ? clusterPairs(mode, pairs)
    : { clusters: [], standalone: pairs };
  clusters.forEach((cluster, index) => {
    lines.push(...renderCluster(cluster, index, mode, cwd));
    lines.push("");
  });
  if (standalone.length > 0) {
    lines.push(renderPairTable(standalone, cwd));
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

export function formatPrettyReport(report: AnalyzeReport, cwd: string, modes: AnalyzerMode[]): string {
  const lines: string[] = [];
  lines.push(chalk.bold("Analyzing code similarity..."));
  lines.push(`Files analyzed: ${report.stats.fileCount}`);
  lines.push(`Pairs detected: ${report.stats.pairCount}`);
  if (report.stats.clusterCount > 0) {
    lines.push(`Duplicate clusters: ${report.stats.clusterCount}`);
  }
  lines.push(`Elapsed: ${report.stats.elapsedMs} ms`);
  lines.push("");

  for (const mode of modes) {
    // Guard against engine/report drift: a requested mode missing from
    // `byMode` renders as an empty section instead of crashing.
    lines.push(renderMode(mode, report.byMode[mode] ?? [], cwd));
  }

  if (report.warnings.length > 0) {
    lines.push(chalk.yellow("Warnings:"));
    for (const warning of report.warnings) {
      lines.push(`- ${warning.message}`);
    }
  }

  return lines.join("\n").trimEnd();
}

export function formatJsonReport(report: AnalyzeReport): string {
  return JSON.stringify(report, null, 2);
}
