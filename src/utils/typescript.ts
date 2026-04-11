import ts from "typescript";

export interface ParseSourceResult {
  sourceFile: ts.SourceFile;
  parseErrors: string[];
}

export interface TokenWithLine {
  token: string;
  line: number;
}

function scriptKindFromPath(filePath: string): ts.ScriptKind {
  const normalized = filePath.toLowerCase();
  if (normalized.endsWith(".tsx")) {
    return ts.ScriptKind.TSX;
  }
  return ts.ScriptKind.TS;
}

export function parseSourceFile(filePath: string, content: string): ParseSourceResult {
  const sourceFile = ts.createSourceFile(
    filePath,
    content,
    ts.ScriptTarget.Latest,
    true,
    scriptKindFromPath(filePath),
  );
  return { sourceFile, parseErrors: [] };
}

export function getLineRange(sourceFile: ts.SourceFile, node: ts.Node): { startLine: number; endLine: number } {
  const start = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
  const end = sourceFile.getLineAndCharacterOfPosition(node.getEnd());
  return { startLine: start.line + 1, endLine: end.line + 1 };
}

function normalizeToken(kind: ts.SyntaxKind, tokenText: string): string | null {
  if (kind === ts.SyntaxKind.WhitespaceTrivia || kind === ts.SyntaxKind.NewLineTrivia) {
    return null;
  }
  if (kind === ts.SyntaxKind.Identifier || kind === ts.SyntaxKind.PrivateIdentifier) {
    return "id";
  }
  if (
    kind === ts.SyntaxKind.StringLiteral ||
    kind === ts.SyntaxKind.NoSubstitutionTemplateLiteral ||
    kind === ts.SyntaxKind.TemplateHead ||
    kind === ts.SyntaxKind.TemplateMiddle ||
    kind === ts.SyntaxKind.TemplateTail
  ) {
    return "str";
  }
  if (
    kind === ts.SyntaxKind.NumericLiteral ||
    kind === ts.SyntaxKind.BigIntLiteral
  ) {
    return "num";
  }
  if (kind === ts.SyntaxKind.TrueKeyword || kind === ts.SyntaxKind.FalseKeyword) {
    return "bool";
  }
  if (kind === ts.SyntaxKind.RegularExpressionLiteral) {
    return "regex";
  }
  const symbol = ts.tokenToString(kind);
  if (symbol) {
    return symbol;
  }
  if (kind >= ts.SyntaxKind.FirstKeyword && kind <= ts.SyntaxKind.LastKeyword) {
    return tokenText;
  }
  return tokenText;
}

export function tokenizeNormalized(content: string): string[] {
  const scanner = ts.createScanner(ts.ScriptTarget.Latest, false, ts.LanguageVariant.Standard, content);
  const tokens: string[] = [];
  for (;;) {
    const kind = scanner.scan();
    if (kind === ts.SyntaxKind.EndOfFileToken) {
      break;
    }
    const normalized = normalizeToken(kind, scanner.getTokenText());
    if (normalized) {
      tokens.push(normalized);
    }
  }
  return tokens;
}

export function tokenizeNormalizedWithLines(content: string): TokenWithLine[] {
  const lineStarts: number[] = [0];
  for (let index = 0; index < content.length; index += 1) {
    if (content[index] === "\n") {
      lineStarts.push(index + 1);
    }
  }

  const lineAt = (position: number): number => {
    let low = 0;
    let high = lineStarts.length - 1;
    while (low <= high) {
      const mid = Math.floor((low + high) / 2);
      const current = lineStarts[mid] ?? 0;
      const next = lineStarts[mid + 1] ?? Number.POSITIVE_INFINITY;
      if (position >= current && position < next) {
        return mid + 1;
      }
      if (position < current) {
        high = mid - 1;
      } else {
        low = mid + 1;
      }
    }
    return 1;
  };

  const scanner = ts.createScanner(ts.ScriptTarget.Latest, false, ts.LanguageVariant.Standard, content);
  const tokens: TokenWithLine[] = [];

  for (;;) {
    const kind = scanner.scan();
    if (kind === ts.SyntaxKind.EndOfFileToken) {
      break;
    }
    const normalized = normalizeToken(kind, scanner.getTokenText());
    if (!normalized) {
      continue;
    }
    const tokenStart = scanner.getTokenPos();
    const line = lineAt(tokenStart);
    tokens.push({ token: normalized, line });
  }

  return tokens;
}

export function normalizeTypeNode(sourceFile: ts.SourceFile, node: ts.TypeNode | undefined): string {
  if (!node) {
    return "any";
  }
  const text = node.getText(sourceFile);
  return tokenizeNormalized(text).join(" ");
}

export function getNodeText(sourceFile: ts.SourceFile, node: ts.Node | undefined): string {
  if (!node) {
    return "";
  }
  return node.getText(sourceFile);
}
