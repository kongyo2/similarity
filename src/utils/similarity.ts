import { distance } from "fastest-levenshtein";

export function clamp01(value: number): number {
  if (Number.isNaN(value)) {
    return 0;
  }
  if (value < 0) {
    return 0;
  }
  if (value > 1) {
    return 1;
  }
  return value;
}

export function normalizedLevenshtein(left: string, right: string): number {
  if (left === right) {
    return 1;
  }
  if (!left.length || !right.length) {
    return 0;
  }
  const maxLength = Math.max(left.length, right.length);
  const editDistance = distance(left, right);
  return clamp01(1 - editDistance / maxLength);
}

export function jaccardSimilarity(left: Iterable<string>, right: Iterable<string>): number {
  const leftSet = new Set(left);
  const rightSet = new Set(right);
  if (leftSet.size === 0 && rightSet.size === 0) {
    return 1;
  }
  let intersection = 0;
  for (const item of leftSet) {
    if (rightSet.has(item)) {
      intersection += 1;
    }
  }
  const union = leftSet.size + rightSet.size - intersection;
  if (union === 0) {
    return 0;
  }
  return clamp01(intersection / union);
}

export function withSizePenalty(
  similarity: number,
  leftSize: number,
  rightSize: number,
  enabled: boolean,
): number {
  if (!enabled) {
    return clamp01(similarity);
  }
  if (leftSize <= 0 || rightSize <= 0) {
    return 0;
  }
  const ratio = Math.min(leftSize, rightSize) / Math.max(leftSize, rightSize);
  return clamp01(similarity * Math.sqrt(ratio));
}

export function average(values: number[]): number {
  if (values.length === 0) {
    return 0;
  }
  const total = values.reduce((sum, value) => sum + value, 0);
  return total / values.length;
}
