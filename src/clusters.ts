import { ANALYZER_MODES } from "./defaults.js";
import type {
  AnalyzerLocation,
  AnalyzerMode,
  SimilarityCluster,
  SimilarityFragment,
  SimilarityPair,
} from "./types.js";
import { compareStrings } from "./utils/path.js";

/** The smallest group reported as a cluster; two members stay a pair. */
const MIN_CLUSTER_SIZE = 3;

/**
 * The fragment an overlap pair relates — both of its sides are that many
 * nodes under a node of that kind. Pairs of the declaration modes say
 * nothing of the sort.
 */
function fragmentOf(pair: SimilarityPair): SimilarityFragment | undefined {
  const { nodeCount, nodeType } = pair.details ?? {};
  return typeof nodeCount === "number" && typeof nodeType === "string"
    ? { nodeCount, nodeType }
    : undefined;
}

/**
 * What a member of a cluster is: a declaration, or one fragment of a
 * function. A location alone does not name a fragment — a loop and its
 * body, or two statements on one line, span the same lines — so a
 * fragment is a location and its shape.
 */
function memberKey(location: AnalyzerLocation, fragment: SimilarityFragment | undefined): string {
  const key: (string | number)[] = [
    location.filePath,
    location.startLine,
    location.endLine,
    location.symbolName,
    location.kind,
  ];
  if (fragment) {
    key.push(fragment.nodeType, fragment.nodeCount);
  }
  return key.join("\u0000");
}

function compareLocations(a: AnalyzerLocation, b: AnalyzerLocation): number {
  return (
    compareStrings(a.filePath, b.filePath) ||
    a.startLine - b.startLine ||
    compareStrings(a.symbolName, b.symbolName) ||
    a.endLine - b.endLine
  );
}

export interface ClusteredPairs {
  clusters: SimilarityCluster[];
  /** Pairs whose two sides belong to no cluster, in input order. */
  standalone: SimilarityPair[];
}

/**
 * Group one mode's pairs into duplicate clusters: the connected components
 * of the "reported as similar" relation with at least three members. Five
 * copies of one helper are five declarations to consolidate, not the ten
 * pairs the pairwise report lists; a guard clause pasted into five
 * functions is five fragments to extract. Members are connected through
 * reported pairs, so two members of a cluster need not be a reported pair
 * themselves; `pairCount` against `members.length` shows how dense the
 * group is.
 */
export function clusterPairs(mode: AnalyzerMode, pairs: SimilarityPair[]): ClusteredPairs {
  interface Member {
    location: AnalyzerLocation;
    fragment: SimilarityFragment | undefined;
  }
  const parent = new Map<string, string>();
  const members = new Map<string, Member>();
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
  const add = (location: AnalyzerLocation, fragment: SimilarityFragment | undefined): string => {
    const key = memberKey(location, fragment);
    if (!parent.has(key)) {
      parent.set(key, key);
      members.set(key, { location, fragment });
    }
    return key;
  };

  for (const pair of pairs) {
    const fragment = fragmentOf(pair);
    const left = find(add(pair.left, fragment));
    const right = find(add(pair.right, fragment));
    if (left !== right) {
      parent.set(left, right);
    }
  }

  const components = new Map<string, string[]>();
  for (const key of parent.keys()) {
    const root = find(key);
    const component = components.get(root);
    if (component) {
      component.push(key);
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
    // A pair joins two members of one shape, so a component has one shape.
    const fragment = (members.get(keys[0] as string) as Member).fragment;
    const cluster: SimilarityCluster = {
      mode,
      members: keys.map((key) => (members.get(key) as Member).location).sort(compareLocations),
      ...(fragment ? { fragment } : {}),
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
    const cluster = clusterOf.get(memberKey(pair.left, fragmentOf(pair)));
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

/** The clusters of every mode of a report, in mode order. */
export function buildClusters(
  byMode: Partial<Record<AnalyzerMode, SimilarityPair[]>>,
): SimilarityCluster[] {
  return ANALYZER_MODES.flatMap((mode) => clusterPairs(mode, byMode[mode] ?? []).clusters);
}

/**
 * The pairs of one mode that given clusters do not cover — those whose two
 * sides are not members of one cluster of that mode — in input order. For
 * clusters built from the same pairs this is {@link clusterPairs}'
 * `standalone` list.
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
      clusterOf.set(memberKey(member, cluster.fragment), index);
    }
  });
  return pairs.filter((pair) => {
    const fragment = fragmentOf(pair);
    const left = clusterOf.get(memberKey(pair.left, fragment));
    return left === undefined || left !== clusterOf.get(memberKey(pair.right, fragment));
  });
}
