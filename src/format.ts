import chalk from "chalk";
import Table from "cli-table3";
import type { AnalyzeReport, AnalyzerMode, SimilarityPair } from "./types.js";
import { toRelativePath } from "./utils/path.js";

function formatLocation(pair: SimilarityPair["left"], cwd: string): string {
  const relativePath = toRelativePath(pair.filePath, cwd);
  return `${relativePath}:${pair.startLine}-${pair.endLine} ${pair.symbolName}`;
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

function renderMode(mode: AnalyzerMode, pairs: SimilarityPair[], cwd: string): string {
  const lines: string[] = [];
  lines.push(chalk.bold(`=== ${formatModeLabel(mode)} ===`));
  if (pairs.length === 0) {
    lines.push("No similar pairs found.");
    lines.push("");
    return lines.join("\n");
  }

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

  lines.push(table.toString());
  lines.push(`Total pairs: ${pairs.length}`);
  lines.push("");
  return lines.join("\n");
}

export function formatPrettyReport(report: AnalyzeReport, cwd: string, modes: AnalyzerMode[]): string {
  const lines: string[] = [];
  lines.push(chalk.bold("Analyzing code similarity..."));
  lines.push(`Files analyzed: ${report.stats.fileCount}`);
  lines.push(`Pairs detected: ${report.stats.pairCount}`);
  lines.push(`Elapsed: ${report.stats.elapsedMs} ms`);
  lines.push("");

  for (const mode of modes) {
    lines.push(renderMode(mode, report.byMode[mode], cwd));
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
