import ts from "typescript";
import type { LoadedFile, SimilarityPair } from "../types.js";
import { normalizedLevenshtein } from "../utils/similarity.js";
import { getLineRange, normalizeTypeNode, parseSourceFile, tokenizeNormalized } from "../utils/typescript.js";

type TypeKind = "interface" | "type" | "type-literal";

interface TypeCandidate {
  filePath: string;
  name: string;
  kind: TypeKind;
  startLine: number;
  endLine: number;
  structuralSignature: string;
}

export interface AnalyzeTypesOptions {
  threshold: number;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
  typesOnly: "interface" | "type" | "all";
  allowCrossKind: boolean;
  includeTypeLiterals: boolean;
}

function identifierName(sourceFile: ts.SourceFile, node: ts.PropertyName): string {
  if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) {
    return node.text;
  }
  return node.getText(sourceFile);
}

function propertySignatureToToken(sourceFile: ts.SourceFile, member: ts.TypeElement): string {
  if (ts.isPropertySignature(member)) {
    const name = identifierName(sourceFile, member.name);
    const optional = member.questionToken ? "?" : "";
    const readonly = member.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.ReadonlyKeyword)
      ? "readonly"
      : "";
    const typeText = normalizeTypeNode(sourceFile, member.type);
    return `prop:${readonly}:${name}${optional}:${typeText}`;
  }
  if (ts.isMethodSignature(member)) {
    const name = identifierName(sourceFile, member.name);
    const params = member.parameters.map((parameter) => normalizeTypeNode(sourceFile, parameter.type)).join("|");
    const returnType = normalizeTypeNode(sourceFile, member.type);
    return `method:${name}(${params}):${returnType}`;
  }
  if (ts.isIndexSignatureDeclaration(member)) {
    const params = member.parameters.map((parameter) => normalizeTypeNode(sourceFile, parameter.type)).join("|");
    const returnType = normalizeTypeNode(sourceFile, member.type);
    return `index:${params}:${returnType}`;
  }
  if (ts.isCallSignatureDeclaration(member)) {
    const params = member.parameters.map((parameter) => normalizeTypeNode(sourceFile, parameter.type)).join("|");
    const returnType = normalizeTypeNode(sourceFile, member.type);
    return `call:${params}:${returnType}`;
  }
  return tokenizeNormalized(member.getText(sourceFile)).join(" ");
}

function typeLiteralSignature(sourceFile: ts.SourceFile, typeLiteral: ts.TypeLiteralNode): string {
  const tokens = typeLiteral.members.map((member) => propertySignatureToToken(sourceFile, member)).sort();
  return tokens.join(";");
}

function typeAliasSignature(sourceFile: ts.SourceFile, declaration: ts.TypeAliasDeclaration): string {
  if (ts.isTypeLiteralNode(declaration.type)) {
    return typeLiteralSignature(sourceFile, declaration.type);
  }
  return normalizeTypeNode(sourceFile, declaration.type);
}

function extractTypeCandidates(file: LoadedFile, options: AnalyzeTypesOptions, warnings: string[]): TypeCandidate[] {
  const { sourceFile, parseErrors } = parseSourceFile(file.filePath, file.content);
  if (parseErrors.length > 0) {
    warnings.push(`Parse diagnostics in ${file.filePath}: ${parseErrors.join("; ")}`);
  }

  const candidates: TypeCandidate[] = [];
  const pushCandidate = (
    node: ts.Node,
    name: string,
    kind: TypeKind,
    structuralSignature: string,
  ) => {
    if (!structuralSignature.trim()) {
      return;
    }
    const { startLine, endLine } = getLineRange(sourceFile, node);
    candidates.push({
      filePath: file.filePath,
      name,
      kind,
      startLine,
      endLine,
      structuralSignature,
    });
  };

  const visit = (node: ts.Node): void => {
    if (ts.isInterfaceDeclaration(node)) {
      const signature = node.members.map((member) => propertySignatureToToken(sourceFile, member)).sort().join(";");
      pushCandidate(node, node.name.text, "interface", signature);
    } else if (ts.isTypeAliasDeclaration(node)) {
      const signature = typeAliasSignature(sourceFile, node);
      pushCandidate(node, node.name.text, "type", signature);
    } else if (options.includeTypeLiterals && ts.isTypeLiteralNode(node)) {
      if (!ts.isTypeAliasDeclaration(node.parent)) {
        const signature = typeLiteralSignature(sourceFile, node);
        const { startLine } = getLineRange(sourceFile, node);
        pushCandidate(node, `<type-literal@${startLine}>`, "type-literal", signature);
      }
    }
    ts.forEachChild(node, visit);
  };

  visit(sourceFile);

  return candidates.filter((candidate) => {
    if (options.typesOnly === "all") {
      return true;
    }
    return candidate.kind === options.typesOnly;
  });
}

function computeTypeSimilarity(left: TypeCandidate, right: TypeCandidate): number {
  const structureScore = normalizedLevenshtein(left.structuralSignature, right.structuralSignature);
  const useName = left.kind !== "type-literal" && right.kind !== "type-literal";
  if (!useName) {
    return structureScore;
  }
  const namingScore = normalizedLevenshtein(left.name.toLowerCase(), right.name.toLowerCase());
  return structureScore * 0.85 + namingScore * 0.15;
}

function isComparable(left: TypeCandidate, right: TypeCandidate, options: AnalyzeTypesOptions): boolean {
  if (options.allowCrossKind) {
    return true;
  }
  return left.kind === right.kind;
}

export function analyzeTypes(
  files: LoadedFile[],
  options: AnalyzeTypesOptions,
  warnings: string[],
): SimilarityPair[] {
  const candidates = files.flatMap((file) => extractTypeCandidates(file, options, warnings));
  const pairs: SimilarityPair[] = [];

  for (let i = 0; i < candidates.length; i += 1) {
    const left = candidates[i];
    if (!left) {
      continue;
    }
    for (let j = i + 1; j < candidates.length; j += 1) {
      const right = candidates[j];
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
      if (!isComparable(left, right, options)) {
        continue;
      }
      const similarity = computeTypeSimilarity(left, right);
      if (similarity < options.threshold) {
        continue;
      }
      pairs.push({
        mode: "types",
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
      });
    }
  }

  return pairs.sort((a, b) => b.similarity - a.similarity);
}
