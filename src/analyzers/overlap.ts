import type { LoadedFile, SimilarityPair } from "../types.js";
import { normalizedLevenshtein } from "../utils/similarity.js";
import { tokenizeNormalizedWithLines, type TokenWithLine } from "../utils/typescript.js";

interface TokenizedFile {
  filePath: string;
  tokens: TokenWithLine[];
}

interface Occurrence {
  fileIndex: number;
  startIndex: number;
  windowSize: number;
}

export interface AnalyzeOverlapOptions {
  threshold: number;
  minWindow: number;
  maxWindow: number;
  sizeTolerance: number;
  sameFileOnly: boolean;
  crossFileOnly: boolean;
}

const MAX_OCCURRENCES_PER_KEY = 32;

function buildTokenIndex(files: LoadedFile[]): TokenizedFile[] {
  return files.map((file) => ({
    filePath: file.filePath,
    tokens: tokenizeNormalizedWithLines(file.content),
  }));
}

function extendExactOverlap(left: TokenWithLine[], right: TokenWithLine[], leftStart: number, rightStart: number): number {
  let overlap = 0;
  while (
    leftStart + overlap < left.length &&
    rightStart + overlap < right.length
  ) {
    const leftToken = left[leftStart + overlap];
    const rightToken = right[rightStart + overlap];
    if (!leftToken || !rightToken || leftToken.token !== rightToken.token) {
      break;
    }
    overlap += 1;
  }
  return overlap;
}

function tokenSliceString(tokens: TokenWithLine[], startIndex: number, length: number): string {
  return tokens
    .slice(startIndex, startIndex + length)
    .map((token) => token.token)
    .join(" ");
}

function createResult(
  leftFile: TokenizedFile,
  rightFile: TokenizedFile,
  leftStart: number,
  rightStart: number,
  overlapLength: number,
  similarity: number,
): SimilarityPair {
  const leftEndIndex = Math.min(leftFile.tokens.length - 1, leftStart + overlapLength - 1);
  const rightEndIndex = Math.min(rightFile.tokens.length - 1, rightStart + overlapLength - 1);
  const leftStartToken = leftFile.tokens[leftStart];
  const rightStartToken = rightFile.tokens[rightStart];
  const leftEndToken = leftFile.tokens[leftEndIndex];
  const rightEndToken = rightFile.tokens[rightEndIndex];

  return {
    mode: "overlap",
    similarity,
    left: {
      filePath: leftFile.filePath,
      startLine: leftStartToken?.line ?? 1,
      endLine: leftEndToken?.line ?? leftStartToken?.line ?? 1,
      symbolName: `<overlap@${leftStart + 1}>`,
      kind: "token-window",
    },
    right: {
      filePath: rightFile.filePath,
      startLine: rightStartToken?.line ?? 1,
      endLine: rightEndToken?.line ?? rightStartToken?.line ?? 1,
      symbolName: `<overlap@${rightStart + 1}>`,
      kind: "token-window",
    },
    details: {
      overlapTokens: overlapLength,
    },
  };
}

function occurrencePairs(occurrences: Occurrence[]): Array<[Occurrence, Occurrence]> {
  const pairs: Array<[Occurrence, Occurrence]> = [];
  for (let i = 0; i < occurrences.length; i += 1) {
    const left = occurrences[i];
    if (!left) {
      continue;
    }
    for (let j = i + 1; j < occurrences.length; j += 1) {
      const right = occurrences[j];
      if (!right) {
        continue;
      }
      pairs.push([left, right]);
    }
  }
  return pairs;
}

export function analyzeOverlap(files: LoadedFile[], options: AnalyzeOverlapOptions): SimilarityPair[] {
  const minWindow = Math.max(1, Math.min(options.minWindow, options.maxWindow));
  const maxWindow = Math.max(minWindow, options.maxWindow);
  const tokenized = buildTokenIndex(files);

  const candidates = new Map<string, Occurrence[]>();
  for (let fileIndex = 0; fileIndex < tokenized.length; fileIndex += 1) {
    const current = tokenized[fileIndex];
    if (!current) {
      continue;
    }
    for (let windowSize = minWindow; windowSize <= maxWindow; windowSize += 1) {
      if (current.tokens.length < windowSize) {
        continue;
      }
      for (let start = 0; start <= current.tokens.length - windowSize; start += 1) {
        const key = current.tokens
          .slice(start, start + windowSize)
          .map((token) => token.token)
          .join(" ");
        if (!key) {
          continue;
        }
        const list = candidates.get(key) ?? [];
        if (list.length < MAX_OCCURRENCES_PER_KEY) {
          list.push({ fileIndex, startIndex: start, windowSize });
        }
        candidates.set(key, list);
      }
    }
  }

  const deduped = new Map<string, SimilarityPair>();

  for (const occurrences of candidates.values()) {
    if (occurrences.length < 2) {
      continue;
    }
    for (const [leftOccur, rightOccur] of occurrencePairs(occurrences)) {
      const leftFile = tokenized[leftOccur.fileIndex];
      const rightFile = tokenized[rightOccur.fileIndex];
      if (!leftFile || !rightFile) {
        continue;
      }
      const sameFile = leftFile.filePath === rightFile.filePath;
      if (options.sameFileOnly && !sameFile) {
        continue;
      }
      if (options.crossFileOnly && sameFile) {
        continue;
      }
      const overlapLength = extendExactOverlap(
        leftFile.tokens,
        rightFile.tokens,
        leftOccur.startIndex,
        rightOccur.startIndex,
      );
      if (overlapLength < minWindow) {
        continue;
      }

      const leftSegmentLength = Math.min(maxWindow, leftFile.tokens.length - leftOccur.startIndex);
      const rightSegmentLength = Math.min(maxWindow, rightFile.tokens.length - rightOccur.startIndex);
      const segmentDiffRatio =
        Math.abs(leftSegmentLength - rightSegmentLength) / Math.max(leftSegmentLength, rightSegmentLength, 1);
      if (segmentDiffRatio > options.sizeTolerance) {
        continue;
      }

      const leftSegment = tokenSliceString(leftFile.tokens, leftOccur.startIndex, leftSegmentLength);
      const rightSegment = tokenSliceString(rightFile.tokens, rightOccur.startIndex, rightSegmentLength);
      const similarity = normalizedLevenshtein(leftSegment, rightSegment);
      if (similarity < options.threshold) {
        continue;
      }

      const result = createResult(
        leftFile,
        rightFile,
        leftOccur.startIndex,
        rightOccur.startIndex,
        overlapLength,
        similarity,
      );
      const key = [
        result.left.filePath,
        result.left.startLine,
        result.left.endLine,
        result.right.filePath,
        result.right.startLine,
        result.right.endLine,
      ].join(":");
      const previous = deduped.get(key);
      if (!previous || previous.similarity < result.similarity) {
        deduped.set(key, result);
      }
    }
  }

  return [...deduped.values()].sort((left, right) => right.similarity - left.similarity);
}
