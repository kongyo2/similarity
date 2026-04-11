import ts from "typescript";
import type { LoadedFile, SimilarityPair } from "../types.js";
import { jaccardSimilarity, normalizedLevenshtein, withSizePenalty } from "../utils/similarity.js";
import { getLineRange, parseSourceFile, tokenizeNormalized } from "../utils/typescript.js";

interface FunctionCandidate {
  filePath: string;
  name: string;
  kind: string;
  startLine: number;
  endLine: number;
  lineCount: number;
  tokenCount: number;
  nodeStart: number;
  nodeEnd: number;
  tokens: string[];
  structureTokens: string[];
  tokenSet: Set<string>;
}

export interface AnalyzeFunctionsOptions {
  threshold: number;
  minLines: number;
  minTokens: number;
  sizePenalty: boolean;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
}

function getNodeBody(node: ts.Node): ts.Node | undefined {
  if (
    ts.isFunctionDeclaration(node) ||
    ts.isFunctionExpression(node) ||
    ts.isMethodDeclaration(node) ||
    ts.isGetAccessorDeclaration(node) ||
    ts.isSetAccessorDeclaration(node) ||
    ts.isArrowFunction(node)
  ) {
    return node.body;
  }
  return undefined;
}

function inferFunctionName(sourceFile: ts.SourceFile, node: ts.Node): string {
  if (ts.isFunctionDeclaration(node)) {
    return node.name?.text ?? "<anonymous-function>";
  }
  if (ts.isMethodDeclaration(node) || ts.isGetAccessorDeclaration(node) || ts.isSetAccessorDeclaration(node)) {
    const classNode = node.parent && ts.isClassLike(node.parent) ? node.parent : undefined;
    const className = classNode?.name?.text ?? "<anonymous-class>";
    const methodName = node.name.getText(sourceFile);
    return `${className}.${methodName}`;
  }
  if (ts.isFunctionExpression(node) || ts.isArrowFunction(node)) {
    if (node.parent && ts.isVariableDeclaration(node.parent) && ts.isIdentifier(node.parent.name)) {
      return node.parent.name.text;
    }
    if (node.parent && ts.isPropertyAssignment(node.parent)) {
      return node.parent.name.getText(sourceFile);
    }
    return "<anonymous-lambda>";
  }
  return "<unknown-function>";
}

function inferFunctionKind(node: ts.Node): string {
  if (ts.isFunctionDeclaration(node)) {
    return "function";
  }
  if (ts.isMethodDeclaration(node)) {
    return "method";
  }
  if (ts.isGetAccessorDeclaration(node)) {
    return "getter";
  }
  if (ts.isSetAccessorDeclaration(node)) {
    return "setter";
  }
  if (ts.isArrowFunction(node)) {
    return "arrow";
  }
  if (ts.isFunctionExpression(node)) {
    return "function-expression";
  }
  return "unknown";
}

function extractFunctionsFromFile(file: LoadedFile, warnings: string[]): FunctionCandidate[] {
  const { sourceFile, parseErrors } = parseSourceFile(file.filePath, file.content);
  if (parseErrors.length > 0) {
    warnings.push(`Parse diagnostics in ${file.filePath}: ${parseErrors.join("; ")}`);
  }
  const candidates: FunctionCandidate[] = [];

  const visit = (node: ts.Node): void => {
    const body = getNodeBody(node);
    if (body) {
      const { startLine, endLine } = getLineRange(sourceFile, body);
      const lineCount = Math.max(1, endLine - startLine + 1);
      const bodyText = body.getText(sourceFile);
      const tokens = tokenizeNormalized(bodyText);
      const structureTokens = extractStructureTokens(body);
      if (tokens.length > 0) {
        candidates.push({
          filePath: file.filePath,
          name: inferFunctionName(sourceFile, node),
          kind: inferFunctionKind(node),
          startLine,
          endLine,
          lineCount,
          tokenCount: tokens.length,
          nodeStart: body.getStart(sourceFile),
          nodeEnd: body.getEnd(),
          tokenSet: new Set(tokens),
          tokens,
          structureTokens,
        });
      }
    }

    ts.forEachChild(node, visit);
  };

  visit(sourceFile);
  return candidates;
}

function extractStructureTokens(node: ts.Node): string[] {
  const tokens: string[] = [];
  const visit = (current: ts.Node): void => {
    if (ts.isIdentifier(current) || ts.isPrivateIdentifier(current)) {
      tokens.push("Identifier");
      return;
    }
    if (
      ts.isStringLiteral(current) ||
      ts.isNoSubstitutionTemplateLiteral(current) ||
      ts.isTemplateExpression(current)
    ) {
      tokens.push("StringLiteral");
      return;
    }
    if (ts.isNumericLiteral(current) || current.kind === ts.SyntaxKind.BigIntLiteral) {
      tokens.push("NumericLiteral");
      return;
    }
    if (current.kind === ts.SyntaxKind.TrueKeyword || current.kind === ts.SyntaxKind.FalseKeyword) {
      tokens.push("BooleanLiteral");
      return;
    }

    const kindName = ts.SyntaxKind[current.kind] ?? String(current.kind);
    tokens.push(kindName);
    ts.forEachChild(current, visit);
  };
  ts.forEachChild(node, visit);
  return tokens;
}

function isParentChildRelationship(left: FunctionCandidate, right: FunctionCandidate): boolean {
  if (left.filePath !== right.filePath) {
    return false;
  }
  return (
    (left.nodeStart <= right.nodeStart && left.nodeEnd >= right.nodeEnd) ||
    (right.nodeStart <= left.nodeStart && right.nodeEnd >= left.nodeEnd)
  );
}

function compareFunctionCandidates(left: FunctionCandidate, right: FunctionCandidate, sizePenalty: boolean): number {
  const leftText = left.tokens.join(" ");
  const rightText = right.tokens.join(" ");
  const leftStructureText = left.structureTokens.join(" ");
  const rightStructureText = right.structureTokens.join(" ");
  const structureScore = normalizedLevenshtein(leftStructureText, rightStructureText);
  const sequenceScore = normalizedLevenshtein(leftText, rightText);
  const tokenScore = jaccardSimilarity(left.tokenSet, right.tokenSet);
  const combined = structureScore * 0.5 + sequenceScore * 0.35 + tokenScore * 0.15;
  return withSizePenalty(combined, left.lineCount, right.lineCount, sizePenalty);
}

export function analyzeFunctions(
  files: LoadedFile[],
  options: AnalyzeFunctionsOptions,
  warnings: string[],
): SimilarityPair[] {
  const candidates = files.flatMap((file) => extractFunctionsFromFile(file, warnings));
  const pairs: SimilarityPair[] = [];

  for (let i = 0; i < candidates.length; i += 1) {
    const left = candidates[i];
    if (!left) {
      continue;
    }
    if (left.lineCount < options.minLines) {
      continue;
    }
    if (left.tokenCount < options.minTokens) {
      continue;
    }
    for (let j = i + 1; j < candidates.length; j += 1) {
      const right = candidates[j];
      if (!right) {
        continue;
      }
      if (right.lineCount < options.minLines) {
        continue;
      }
      if (right.tokenCount < options.minTokens) {
        continue;
      }
      const sameFile = left.filePath === right.filePath;
      if (options.sameFileOnly && !sameFile) {
        continue;
      }
      if (options.crossFileOnly && sameFile) {
        continue;
      }
      if (isParentChildRelationship(left, right)) {
        continue;
      }

      const similarity = compareFunctionCandidates(left, right, options.sizePenalty);
      if (similarity < options.threshold) {
        continue;
      }

      pairs.push({
        mode: "functions",
        similarity,
        left: {
          filePath: left.filePath,
          startLine: left.startLine,
          endLine: left.endLine,
          symbolName: left.name,
          kind: left.kind,
        },
        right: {
          filePath: right.filePath,
          startLine: right.startLine,
          endLine: right.endLine,
          symbolName: right.name,
          kind: right.kind,
        },
        details: {
          leftLines: left.lineCount,
          rightLines: right.lineCount,
          leftTokens: left.tokenCount,
          rightTokens: right.tokenCount,
        },
      });
    }
  }

  return pairs.sort((a, b) => b.similarity - a.similarity);
}
