import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { describe, expect, it } from "vitest";
import { analyzeProject } from "../src/index.js";
import type { AnalyzerMode, SimilarityPair } from "../src/types.js";

async function withTempProject(run: (projectDir: string) => Promise<void>) {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "similarity-ts-accuracy-"));
  try {
    await run(tempRoot);
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

async function writeSource(projectDir: string, relativePath: string, content: string) {
  const filePath = path.join(projectDir, relativePath);
  await fs.mkdir(path.dirname(filePath), { recursive: true });
  await fs.writeFile(filePath, content, "utf8");
  return filePath;
}

function hasPair(pairs: SimilarityPair[], leftName: string, rightName: string): boolean {
  return pairs.some((pair) => {
    const names = [pair.left.symbolName, pair.right.symbolName].sort();
    return names[0] === [leftName, rightName].sort()[0] &&
      names[1] === [leftName, rightName].sort()[1];
  });
}

async function analyzeTempProject(
  projectDir: string,
  modes: AnalyzerMode[],
  extra: Partial<Parameters<typeof analyzeProject>[0]> = {},
) {
  return analyzeProject({
    cwd: projectDir,
    paths: ["src"],
    modes,
    threshold: 0.7,
    ...extra,
  });
}

describe("accuracy regressions adapted from mizchi/similarity", () => {
  it("detects renamed but structurally equivalent functions within one file", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/sums.ts",
        `
export function calculateSum(numbers: number[]): number {
  if (numbers.length === 0) return 0;
  let total = 0;
  for (const number of numbers) {
    total += number;
  }
  return total;
}

export function computeTotal(values: number[]): number {
  if (values.length === 0) return 0;
  let sum = 0;
  for (const value of values) {
    sum += value;
  }
  return sum;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.7,
        minLines: 3,
        noSizePenalty: true,
        sameFileOnly: true,
      });

      expect(hasPair(report.byMode.functions, "calculateSum", "computeTotal")).toBe(true);
      expect(report.byMode.functions[0]?.similarity).toBeGreaterThan(0.8);
    });
  });

  it("filters moderate short-function matches at a high threshold", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/math.ts",
        `
export function add(a: number, b: number): number {
  return a + b;
}

export function sum(x: number, y: number): number {
  return x + y;
}
`,
      );

      const low = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.5,
        minLines: 1,
        noSizePenalty: true,
      });
      const high = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.95,
        minLines: 1,
        noSizePenalty: true,
      });

      expect(hasPair(low.byMode.functions, "add", "sum")).toBe(true);
      expect(hasPair(high.byMode.functions, "add", "sum")).toBe(false);
    });
  });

  it("keeps all members visible when three functions share the same structure", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/cluster.ts",
        `
export function calculateSum(numbers: number[]): number {
  let total = 0;
  for (const number of numbers) {
    total += number;
  }
  return total;
}

export function computeTotal(values: number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}

export function aggregateAmount(items: number[]): number {
  let total = 0;
  for (const item of items) {
    total += item;
  }
  return total;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.8,
        minLines: 1,
        noSizePenalty: true,
      });
      const detectedNames = new Set(
        report.byMode.functions.flatMap((pair) => [
          pair.left.symbolName,
          pair.right.symbolName,
        ]),
      );

      expect(detectedNames).toEqual(
        new Set(["calculateSum", "computeTotal", "aggregateAmount"]),
      );
    });
  });

  it("ignores node_modules through the default exclude set", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "node_modules/package/ignored.ts",
        "export const ignored = true;\n",
      );
      await writeSource(projectDir, "src/app.ts", "export const found = true;\n");

      const report = await analyzeProject({
        cwd: projectDir,
        paths: ["."],
      });

      expect(report.stats.fileCount).toBe(1);
      expect(report.analyzedFiles[0]?.endsWith("src/app.ts")).toBe(true);
      expect(report.skippedFiles.some((file) =>
        file.endsWith("node_modules/package/ignored.ts"),
      )).toBe(true);
    });
  });

  it("avoids high-confidence false positives for similar loops with different intent", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/loop_intent.ts",
        `
export function findMax(numbers: number[]): number {
  let max = numbers[0];
  for (let index = 1; index < numbers.length; index++) {
    if (numbers[index] > max) {
      max = numbers[index];
    }
  }
  return max;
}

export function countOccurrences(text: string, char: string): number {
  let count = 0;
  for (let index = 0; index < text.length; index++) {
    if (text[index] === char) {
      count += 1;
    }
  }
  return count;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.6,
        minLines: 3,
      });

      expect(hasPair(report.byMode.functions, "findMax", "countOccurrences")).toBe(false);
    });
  });

  it("detects similar interfaces across files", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/user.ts",
        `
export interface User {
  id: number;
  name: string;
  email: string;
  createdAt: Date;
}
`,
      );
      await writeSource(
        projectDir,
        "src/person.ts",
        `
export interface Person {
  id: number;
  name: string;
  email: string;
  birthDate: Date;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.7,
      });

      expect(hasPair(report.byMode.types, "User", "Person")).toBe(true);
    });
  });

  it("matches an interface to an equivalent type alias by default and can be opted out", async () => {
    // Cross-kind comparison is on by default so a refactoring run flags
    // an `interface User { ... }` and an equivalent `type User = { ... }`
    // automatically; pass `allowCrossKind: false` to restrict the report
    // to same-kind pairs only.
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/mixed.ts",
        `
export interface IUser {
  id: number;
  name: string;
  email: string;
  permissions: string[];
}

export type TUser = {
  id: number;
  name: string;
  email: string;
  permissions: string[];
};
`,
      );

      const defaultReport = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.9,
      });
      const sameKindOnlyReport = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.9,
        allowCrossKind: false,
      });

      expect(hasPair(defaultReport.byMode.types, "IUser", "TUser")).toBe(true);
      expect(hasPair(sameKindOnlyReport.byMode.types, "IUser", "TUser")).toBe(false);
    });
  });

  it("does not report broad generic interfaces as similar by default", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/generics.ts",
        `
export interface Response<T> {
  data: T;
  status: number;
  message: string;
}

export interface ApiResult<T> {
  result: T;
  code: number;
  description: string;
}

export interface ServerResponse<T> {
  payload: T;
  statusCode: number;
  error?: string;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.7,
      });

      expect(report.byMode.types).toHaveLength(0);
    });
  });

  it("does not report unrelated small object types at a high threshold", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/different.ts",
        `
export interface Point {
  x: number;
  y: number;
}

export interface User {
  id: string;
  name: string;
  email: string;
}

export interface Config {
  debug: boolean;
  timeout: number;
  retryCount: number;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.9,
      });

      expect(report.byMode.types).toHaveLength(0);
    });
  });

  it("honors similarity-ignore comments for functions and types", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/ignored.ts",
        `
export function calculateTotal(items: number[]): number {
  let total = 0;
  for (const item of items) {
    total += item;
  }
  return total;
}

// similarity-ignore
export function computeTotal(values: number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}

export interface User {
  id: string;
  email: string;
}

// similarity-ignore
export interface IgnoredUser {
  id: string;
  email: string;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions", "types"], {
        threshold: 0.6,
        noSizePenalty: true,
      });

      expect(hasPair(report.byMode.functions, "calculateTotal", "computeTotal")).toBe(false);
      expect(hasPair(report.byMode.types, "User", "IgnoredUser")).toBe(false);
    });
  });

  it("includes TSX files in the default extension set", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/component.tsx",
        `
interface ButtonProps {
  label: string;
  onClick: () => void;
}

export function Button({ label, onClick }: ButtonProps) {
  return React.createElement("button", { onClick }, label);
}

export function PrimaryButton({ label, onClick }: ButtonProps) {
  return React.createElement("button", { onClick, className: "primary" }, label);
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.5,
        minLines: 1,
        noSizePenalty: true,
      });

      expect(report.stats.fileCount).toBe(1);
      expect(hasPair(report.byMode.functions, "Button", "PrimaryButton")).toBe(true);
    });
  });

  it("detects exact duplicated validation blocks in overlap mode", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/validation.ts",
        `
export function validateUser(user: { email: string }) {
  if (!user.email) {
    throw new Error("Email is required");
  }
  if (!user.email.includes("@")) {
    throw new Error("Invalid email format");
  }
  if (user.email.length > 100) {
    throw new Error("Email too long");
  }
  return user.email;
}

export function validateAdmin(admin: { email: string }) {
  if (!admin.email) {
    throw new Error("Email is required");
  }
  if (!admin.email.includes("@")) {
    throw new Error("Invalid email format");
  }
  if (admin.email.length > 100) {
    throw new Error("Email too long");
  }
  return admin.email;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["overlap"], {
        threshold: 0.5,
        overlapMinWindow: 3,
        overlapMaxWindow: 25,
        overlapSizeTolerance: 0.5,
      });

      expect(report.byMode.overlap.length).toBeGreaterThan(0);
      expect(report.byMode.overlap.some((pair) => pair.similarity > 0.9)).toBe(true);
    });
  });

  it("keeps type fingerprint analysis practical on many related declarations", async () => {
    await withTempProject(async (projectDir) => {
      const declarations: string[] = [];
      for (let index = 0; index < 50; index += 1) {
        declarations.push(`
export interface Type${index} {
  field1: string;
  field2: number;
  field3: boolean;
  nested: { value: number };
  field${index}: unknown;
}
`);
      }
      await writeSource(projectDir, "src/many_types.ts", declarations.join("\n"));

      const start = performance.now();
      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.95,
      });
      const elapsedMs = performance.now() - start;

      expect(report.stats.fileCount).toBe(1);
      expect(elapsedMs).toBeLessThan(5_000);
    });
  }, 10_000);
});

describe("accuracy improvements over upstream for refactoring use", () => {
  it("matches medium-sized rename refactors that upstream over-penalises", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/fetch.ts",
        `
export function fetchUserData(userId: string): Promise<{ id: string; name: string }> {
  return fetch(\`/api/users/\${userId}\`)
    .then((response) => {
      if (!response.ok) {
        throw new Error(\`Failed to fetch user \${userId}\`);
      }
      return response.json();
    })
    .then((payload) => ({ id: payload.id, name: payload.name }));
}

export function loadCustomerProfile(customerId: string): Promise<{ id: string; name: string }> {
  return fetch(\`/api/customers/\${customerId}\`)
    .then((res) => {
      if (!res.ok) {
        throw new Error(\`Failed to fetch customer \${customerId}\`);
      }
      return res.json();
    })
    .then((data) => ({ id: data.id, name: data.name }));
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.85,
        minLines: 5,
      });

      expect(hasPair(report.byMode.functions, "fetchUserData", "loadCustomerProfile")).toBe(true);
    });
  });

  it("detects renamed class methods even though upstream cannot reparse them in isolation", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/repos.ts",
        `
export class UserRepository {
  async findById(id: string): Promise<unknown> {
    const url = \`/api/users/\${id}\`;
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(\`HTTP \${response.status}\`);
    }
    return response.json();
  }
}

export class CustomerRepository {
  async findById(id: string): Promise<unknown> {
    const path = \`/api/customers/\${id}\`;
    const res = await fetch(path);
    if (!res.ok) {
      throw new Error(\`HTTP \${res.status}\`);
    }
    return res.json();
  }
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.8,
        minLines: 3,
      });

      expect(hasPair(report.byMode.functions, "findById", "findById")).toBe(true);
    });
  });

  it("distinguishes for-loops whose body operations diverge (assignment vs aggregation)", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/loops.ts",
        `
export function transformUppercase(text: string): string {
  if (!text) return "";
  let result = "";
  for (const ch of text) {
    result += ch.toUpperCase();
  }
  return result;
}

export function transformReverse(text: string): string {
  if (!text) return "";
  let result = "";
  for (const ch of text) {
    result = ch + result;
  }
  return result;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.85,
        minLines: 3,
      });

      expect(hasPair(report.byMode.functions, "transformUppercase", "transformReverse")).toBe(
        false,
      );
    });
  });

  it("rejects trivial 1-line operator-differs pairs as duplicates", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/math.ts",
        `
export const add = (a: number, b: number) => a + b;
export const sub = (a: number, b: number) => a - b;
export const mul = (a: number, b: number) => a * b;
export const div = (a: number, b: number) => a / b;
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.85,
        minLines: 1,
      });

      expect(report.byMode.functions).toHaveLength(0);
    });
  });

  it("matches duplicated cache classes despite renamed private storage", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/caches.ts",
        `
export class UserCache {
  private store = new Map<string, unknown>();
  get(key: string): unknown { return this.store.get(key); }
  set(key: string, value: unknown): void { this.store.set(key, value); }
  delete(key: string): boolean { return this.store.delete(key); }
  clear(): void { this.store.clear(); }
}

export class SessionCache {
  private items = new Map<string, unknown>();
  get(key: string): unknown { return this.items.get(key); }
  set(key: string, value: unknown): void { this.items.set(key, value); }
  delete(key: string): boolean { return this.items.delete(key); }
  clear(): void { this.items.clear(); }
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["classes"], {
        threshold: 0.8,
        minLines: 3,
      });

      expect(hasPair(report.byMode.classes, "UserCache", "SessionCache")).toBe(true);
    });
  });

  it("flags identical validation blocks across two functions in overlap mode", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/validation.ts",
        `
export function createUser(payload: { email: string }) {
  if (!payload.email) {
    throw new Error("Email is required");
  }
  if (!payload.email.includes("@")) {
    throw new Error("Invalid email format");
  }
  if (payload.email.length > 100) {
    throw new Error("Email too long");
  }
  return payload;
}

export function createAdmin(payload: { email: string }) {
  if (!payload.email) {
    throw new Error("Email is required");
  }
  if (!payload.email.includes("@")) {
    throw new Error("Invalid email format");
  }
  if (payload.email.length > 100) {
    throw new Error("Email too long");
  }
  return payload;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["overlap"], {
        threshold: 0.7,
        overlapMinWindow: 4,
        overlapMaxWindow: 25,
        overlapSizeTolerance: 0.5,
      });

      expect(report.byMode.overlap.some((pair) => pair.similarity >= 0.99)).toBe(true);
    });
  });

  it("matches duplicated class methods that upstream's parser drops as unparseable", async () => {
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/repos.ts",
        `
export class UserRepository {
  async findById(id: string): Promise<unknown> {
    const url = \`/api/users/\${id}\`;
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(\`HTTP \${response.status}\`);
    }
    return response.json();
  }
}

export class CustomerRepository {
  async findById(id: string): Promise<unknown> {
    const path = \`/api/customers/\${id}\`;
    const res = await fetch(path);
    if (!res.ok) {
      throw new Error(\`HTTP \${res.status}\`);
    }
    return res.json();
  }
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.85,
        minLines: 3,
      });

      expect(hasPair(report.byMode.functions, "findById", "findById")).toBe(true);
    });
  });

  it("scores property-less type aliases with identical bodies as duplicates", async () => {
    // Regression: two type aliases like `type X = "a" | "b" | "c"` used to
    // collapse onto a flat 0.6 similarity score regardless of how close their
    // bodies were, because the type-alias extractor produced an empty
    // property list for non-object bodies. The synthetic `<type-body>`
    // property added during extraction now lets the comparator distinguish
    // identical-body aliases from merely-named-similarly ones.
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/aliases.ts",
        `
export type StatusA = "pending" | "active" | "archived";
export type StatusB = "pending" | "active" | "archived";

export type NumberMapA = Record<string, number>;
export type NumberMapB = Record<string, number>;

export type Unrelated = "left" | "right";
`,
      );

      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.85,
      });

      expect(hasPair(report.byMode.types, "StatusA", "StatusB")).toBe(true);
      expect(hasPair(report.byMode.types, "NumberMapA", "NumberMapB")).toBe(true);
      expect(hasPair(report.byMode.types, "StatusA", "Unrelated")).toBe(false);
    });
  });

  it("flags an arrow function and a regular function with identical bodies as duplicates", async () => {
    // Regression: the structural distance used to be dominated by the
    // FunctionDeclaration vs ArrowFunctionExpression wrapping, so e.g.
    // `function sum(a, b) { return a + b; }` and
    // `const sum = (a, b) => { return a + b; }` reported around 0.6 even
    // though the bodies were byte-identical. The normalization wrapper
    // applied during comparison erases that wrapping difference.
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/arrow_vs_fn.ts",
        `
export function sumArray(values: number[]): number {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
}

export const totalArray = (values: number[]): number => {
  let total = 0;
  for (const value of values) {
    total += value;
  }
  return total;
};
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.85,
        minLines: 3,
      });

      expect(hasPair(report.byMode.functions, "sumArray", "totalArray")).toBe(true);
    });
  });

  it("keeps identical short helpers visible instead of crushing them by size penalty", async () => {
    // Regression: two byte-identical 3-line helpers used to land at ~0.13
    // similarity because the compounded short-function and node-count
    // penalties were applied even when the structural edit distance was
    // exactly zero. The penalty layer now releases the discount when the
    // trees match exactly.
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/short.ts",
        `
export function smallA(value: number): number {
  const next = value + 1;
  return next;
}

export function smallB(value: number): number {
  const next = value + 1;
  return next;
}
`,
      );

      const report = await analyzeTempProject(projectDir, ["functions"], {
        threshold: 0.8,
        minLines: 3,
      });

      expect(hasPair(report.byMode.functions, "smallA", "smallB")).toBe(true);
    });
  });

  it("extracts anonymous type literals from exports and arrow function parameters", async () => {
    // Regression: the `--type-literals` extractor only walked top-level
    // FunctionDeclaration / VariableDeclaration nodes, so anonymous type
    // literals attached to exported or arrow-bound functions never made
    // it into the type comparison pool and `--type-literals` reported zero
    // additional pairs.
    await withTempProject(async (projectDir) => {
      await writeSource(
        projectDir,
        "src/literals.ts",
        `
export function createUser(payload: { id: string; name: string; email: string }) {
  return payload;
}

export const createAdmin = (payload: { id: string; name: string; email: string }) => {
  return payload;
};
`,
      );

      const report = await analyzeTempProject(projectDir, ["types"], {
        threshold: 0.85,
        includeTypeLiterals: true,
      });

      expect(report.byMode.types.length).toBeGreaterThan(0);
    });
  });
});
