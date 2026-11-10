import type {
  AnalyzerLocation,
  AnalyzerMode,
  SimilarityCluster,
  SimilarityPair,
} from "./types.js";
import { compareStrings } from "./utils/path.js";

/**
 * Modes whose pairs relate whole declarations. Overlap pairs relate token
 * windows inside functions, which do not form declaration groups.
 */
export const CLUSTERED_MODES: readonly AnalyzerMode[] = ["functions", "types", "classes"];

/** The smallest group reported as a cluster; two members stay a pair. */
const MIN_CLUSTER_SIZE = 3;

function locationKey(location: AnalyzerLocation): string {
  return [
    location.filePath,
    location.startLine,
    location.endLine,
    location.symbolName,
    location.kind,
  ].join("\u0000");
}

function compareLocations(a: AnalyzerLocation, b: AnalyzerLocation): number {
  return (
    compareStrings(a.filePath, b.filePath) ||
    a.startLine - b.startLine ||
    compareStrings(a.symbolName, b.symbolName)
  );
}

export interface ClusteredPairs {
  clusters: SimilarityCluster[];
  /** Pairs whose declarations belong to no cluster, in input order. */
  standalone: SimilarityPair[];
}

/**
 * Group one mode's pairs into duplicate clusters: the connected components
 * of the "reported as similar" relation with at least three declarations.
 * Five copies of one helper are five declarations to consolidate, not the
 * ten pairs the pairwise report lists. Members are connected through
 * reported pairs, so two members of a cluster need not be a reported pair
 * themselves; `pairCount` against `members.length` shows how dense the
 * group is.
 */
export function clusterPairs(mode: AnalyzerMode, pairs: SimilarityPair[]): ClusteredPairs {
  const parent = new Map<string, string>();
  const locations = new Map<string, AnalyzerLocation>();
  const find = (key: string): string => {
    let root = key;
    while (parent.get(root) !== root) {
      root = parent.get(root) as string;
    }
    let node = key;
    while (node !== root) {
      const next = parent.get(node) as string;
      parent.set(node, root);
      node = next;
    }
    return root;
  };
  const add = (location: AnalyzerLocation): string => {
    const key = locationKey(location);
    if (!parent.has(key)) {
      parent.set(key, key);
      locations.set(key, location);
    }
    return key;
  };

  for (const pair of pairs) {
    const left = find(add(pair.left));
    const right = find(add(pair.right));
    if (left !== right) {
      parent.set(left, right);
    }
  }

  const components = new Map<string, string[]>();
  for (const key of parent.keys()) {
    const root = find(key);
    const members = components.get(root);
    if (members) {
      members.push(key);
    } else {
      components.set(root, [key]);
    }
  }

  const clusterOf = new Map<string, SimilarityCluster>();
  const clusters: SimilarityCluster[] = [];
  for (const keys of components.values()) {
    if (keys.length < MIN_CLUSTER_SIZE) {
      continue;
    }
    const cluster: SimilarityCluster = {
      mode,
      members: keys.map((key) => locations.get(key) as AnalyzerLocation).sort(compareLocations),
      pairCount: 0,
      maxSimilarity: 0,
      minSimilarity: 1,
      averageSimilarity: 0,
    };
    clusters.push(cluster);
    for (const key of keys) {
      clusterOf.set(key, cluster);
    }
  }

  const standalone: SimilarityPair[] = [];
  const totals = new Map<SimilarityCluster, number>();
  for (const pair of pairs) {
    const cluster = clusterOf.get(locationKey(pair.left));
    if (!cluster) {
      standalone.push(pair);
      continue;
    }
    cluster.pairCount += 1;
    cluster.maxSimilarity = Math.max(cluster.maxSimilarity, pair.similarity);
    cluster.minSimilarity = Math.min(cluster.minSimilarity, pair.similarity);
    totals.set(cluster, (totals.get(cluster) ?? 0) + pair.similarity);
  }
  for (const cluster of clusters) {
    cluster.averageSimilarity = (totals.get(cluster) ?? 0) / cluster.pairCount;
  }

  // Every cluster has at least MIN_CLUSTER_SIZE members.
  clusters.sort(
    (a, b) =>
      b.members.length - a.members.length ||
      b.maxSimilarity - a.maxSimilarity ||
      compareLocations(a.members[0] as AnalyzerLocation, b.members[0] as AnalyzerLocation),
  );
  return { clusters, standalone };
}

/** Clusters for every declaration mode of a report, in mode order. */
export function buildClusters(
  byMode: Partial<Record<AnalyzerMode, SimilarityPair[]>>,
): SimilarityCluster[] {
  return CLUSTERED_MODES.flatMap((mode) => clusterPairs(mode, byMode[mode] ?? []).clusters);
}

/**
 * The pairs of one mode that given clusters do not cover — those whose two
 * declarations are not members of one cluster of that mode — in input
 * order. For clusters built from the same pairs this is
 * {@link clusterPairs}' `standalone` list.
 */
export function standalonePairs(
  mode: AnalyzerMode,
  pairs: SimilarityPair[],
  clusters: SimilarityCluster[],
): SimilarityPair[] {
  const clusterOf = new Map<string, number>();
  clusters.forEach((cluster, index) => {
    if (cluster.mode !== mode) {
      return;
    }
    for (const member of cluster.members) {
      clusterOf.set(locationKey(member), index);
    }
  });
  return pairs.filter((pair) => {
    const left = clusterOf.get(locationKey(pair.left));
    return left === undefined || left !== clusterOf.get(locationKey(pair.right));
  });
}
