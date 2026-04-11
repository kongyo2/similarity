import ts from "typescript";
import type { LoadedFile, SimilarityPair } from "../types.js";
import { normalizedLevenshtein } from "../utils/similarity.js";
import { getLineRange, normalizeTypeNode, parseSourceFile } from "../utils/typescript.js";

interface ClassCandidate {
  filePath: string;
  name: string;
  startLine: number;
  endLine: number;
  signature: string;
}

export interface AnalyzeClassesOptions {
  threshold: number;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
}

function memberSignature(sourceFile: ts.SourceFile, member: ts.ClassElement): string | null {
  if (ts.isPropertyDeclaration(member)) {
    const name = member.name.getText(sourceFile);
    const typeName = normalizeTypeNode(sourceFile, member.type);
    const optional = member.questionToken ? "?" : "";
    const staticMark = member.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.StaticKeyword)
      ? "static"
      : "";
    return `prop:${staticMark}:${name}${optional}:${typeName}`;
  }
  if (ts.isMethodDeclaration(member)) {
    const name = member.name.getText(sourceFile);
    const params = member.parameters.map((parameter) => normalizeTypeNode(sourceFile, parameter.type)).join("|");
    const returnType = normalizeTypeNode(sourceFile, member.type);
    return `method:${name}(${params}):${returnType}`;
  }
  if (ts.isConstructorDeclaration(member)) {
    const params = member.parameters.map((parameter) => normalizeTypeNode(sourceFile, parameter.type)).join("|");
    return `ctor(${params})`;
  }
  if (ts.isGetAccessorDeclaration(member) || ts.isSetAccessorDeclaration(member)) {
    return `accessor:${member.name.getText(sourceFile)}`;
  }
  return null;
}

function extractClassCandidates(file: LoadedFile, warnings: string[]): ClassCandidate[] {
  const { sourceFile, parseErrors } = parseSourceFile(file.filePath, file.content);
  if (parseErrors.length > 0) {
    warnings.push(`Parse diagnostics in ${file.filePath}: ${parseErrors.join("; ")}`);
  }
  const classes: ClassCandidate[] = [];

  const visit = (node: ts.Node): void => {
    if (ts.isClassDeclaration(node) || ts.isClassExpression(node)) {
      const name = node.name?.text ?? "<anonymous-class>";
      const signatures = node.members
        .map((member) => memberSignature(sourceFile, member))
        .filter((value): value is string => Boolean(value))
        .sort()
        .join(";");
      if (signatures.length > 0) {
        const { startLine, endLine } = getLineRange(sourceFile, node);
        classes.push({
          filePath: file.filePath,
          name,
          startLine,
          endLine,
          signature: signatures,
        });
      }
    }
    ts.forEachChild(node, visit);
  };

  visit(sourceFile);
  return classes;
}

function classSimilarity(left: ClassCandidate, right: ClassCandidate): number {
  const structure = normalizedLevenshtein(left.signature, right.signature);
  const naming = normalizedLevenshtein(left.name.toLowerCase(), right.name.toLowerCase());
  return structure * 0.8 + naming * 0.2;
}

export function analyzeClasses(
  files: LoadedFile[],
  options: AnalyzeClassesOptions,
  warnings: string[],
): SimilarityPair[] {
  const classes = files.flatMap((file) => extractClassCandidates(file, warnings));
  const pairs: SimilarityPair[] = [];

  for (let i = 0; i < classes.length; i += 1) {
    const left = classes[i];
    if (!left) {
      continue;
    }
    for (let j = i + 1; j < classes.length; j += 1) {
      const right = classes[j];
      if (!right) {
        continue;
      }
      const sameFile = left.filePath === right.filePath;
      if (options.sameFileOnly && !sameFile) {
        continue;
      }
      if (options.crossFileOnly && sameFile) {
        continue;
      }
      const similarity = classSimilarity(left, right);
      if (similarity < options.threshold) {
        continue;
      }
      pairs.push({
        mode: "classes",
        similarity,
        left: {
          filePath: left.filePath,
          startLine: left.startLine,
          endLine: left.endLine,
          symbolName: left.name,
          kind: "class",
        },
        right: {
          filePath: right.filePath,
          startLine: right.startLine,
          endLine: right.endLine,
          symbolName: right.name,
          kind: "class",
        },
      });
    }
  }

  return pairs.sort((a, b) => b.similarity - a.similarity);
}
