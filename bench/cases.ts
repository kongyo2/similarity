/**
 * Labeled accuracy corpus for the README refactoring flow.
 *
 * Every case represents a realistic situation an AI coding assistant hits
 * when it runs `npx @kongyo2/similarity-ts .` over a project and builds a
 * refactoring plan from the report:
 *
 * - `expectPairs` are semantic duplicates that a refactoring plan must see.
 *   Missing one is a false negative.
 * - `forbidPairs` are similarly-shaped but semantically different symbols.
 *   Reporting one is a false positive that pollutes the plan.
 *
 * All cases are evaluated with the CLI defaults from the README invocation
 * (threshold 0.8, minLines 3) in the mode the case targets.
 */

export type BenchMode = "functions" | "types" | "classes";

export interface BenchCase {
  id: string;
  mode: BenchMode;
  category: string;
  description: string;
  files: Record<string, string>;
  expectPairs?: [string, string][];
  forbidPairs?: [string, string][];
}

export const benchCases: BenchCase[] = [
  // -------------------------------------------------------------------
  // functions — positives: semantic duplicates a refactor plan must see
  // -------------------------------------------------------------------
  {
    id: "F-P01",
    mode: "functions",
    category: "rename",
    description: "Fully renamed accumulation helper",
    files: {
      "a.ts": `
export function calculateCartTotal(prices: number[]): number {
  if (prices.length === 0) return 0;
  let total = 0;
  for (const price of prices) {
    total += price;
  }
  return total;
}

export function sumInvoiceAmounts(amounts: number[]): number {
  if (amounts.length === 0) return 0;
  let sum = 0;
  for (const amount of amounts) {
    sum += amount;
  }
  return sum;
}
`,
    },
    expectPairs: [["calculateCartTotal", "sumInvoiceAmounts"]],
  },
  {
    id: "F-P02",
    mode: "functions",
    category: "rename",
    description: "Renamed promise-chain fetch helpers across files",
    files: {
      "a.ts": `
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
`,
      "b.ts": `
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
    },
    expectPairs: [["fetchUserData", "loadCustomerProfile"]],
  },
  {
    id: "F-P03",
    mode: "functions",
    category: "declaration-form",
    description: "Arrow function vs function declaration, identical body",
    files: {
      "a.ts": `
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
    },
    expectPairs: [["sumArray", "totalArray"]],
  },
  {
    id: "F-P04",
    mode: "functions",
    category: "declaration-form",
    description: "Class method vs standalone function with the same body",
    files: {
      "a.ts": `
export class PriceFormatter {
  formatPrice(amount: number, currency: string): string {
    const rounded = Math.round(amount * 100) / 100;
    const fixed = rounded.toFixed(2);
    return currency + " " + fixed;
  }
}
`,
      "b.ts": `
export function formatCost(amount: number, currency: string): string {
  const rounded = Math.round(amount * 100) / 100;
  const fixed = rounded.toFixed(2);
  return currency + " " + fixed;
}
`,
    },
    expectPairs: [["formatPrice", "formatCost"]],
  },
  {
    id: "F-P05",
    mode: "functions",
    category: "async-style",
    description: "Promise .then chain vs async/await, same logic",
    files: {
      "a.ts": `
export function loadOrderJson(orderId: string): Promise<unknown> {
  return fetch("/api/orders/" + orderId).then((response) => {
    if (!response.ok) {
      throw new Error("HTTP " + response.status);
    }
    return response.json();
  });
}
`,
      "b.ts": `
export async function readOrderJson(orderId: string): Promise<unknown> {
  const response = await fetch("/api/orders/" + orderId);
  if (!response.ok) {
    throw new Error("HTTP " + response.status);
  }
  return response.json();
}
`,
    },
    expectPairs: [["loadOrderJson", "readOrderJson"]],
  },
  {
    id: "F-P06",
    mode: "functions",
    category: "async-style",
    description: "Fire-and-forget .then(() => ...) vs await sequence",
    files: {
      "a.ts": `
export function persistSettings(settings: { save(): Promise<void> }, log: (m: string) => void) {
  return settings.save().then(() => {
    log("settings saved");
    log("notifying listeners");
  });
}
`,
      "b.ts": `
export async function storeSettings(settings: { save(): Promise<void> }, log: (m: string) => void) {
  await settings.save();
  log("settings saved");
  log("notifying listeners");
}
`,
    },
    expectPairs: [["persistSettings", "storeSettings"]],
  },
  {
    id: "F-P07",
    mode: "functions",
    category: "loop-style",
    description: "Array.forEach callback vs for-of loop, same body",
    files: {
      "a.ts": `
export function indexUsersByEmail(users: { email: string; id: string }[]): Map<string, string> {
  const index = new Map<string, string>();
  users.forEach((user) => {
    const key = user.email.toLowerCase();
    index.set(key, user.id);
  });
  return index;
}
`,
      "b.ts": `
export function buildEmailIndex(users: { email: string; id: string }[]): Map<string, string> {
  const index = new Map<string, string>();
  for (const user of users) {
    const key = user.email.toLowerCase();
    index.set(key, user.id);
  }
  return index;
}
`,
    },
    expectPairs: [["indexUsersByEmail", "buildEmailIndex"]],
  },
  {
    id: "F-P08",
    mode: "functions",
    category: "string-style",
    description: "Template literal vs string concatenation",
    files: {
      "a.ts": `
export function describeOrder(id: string, count: number, total: number): string {
  const head = \`Order \${id} contains \${count} items\`;
  const tail = \`for a total of \${total} USD\`;
  return head + " " + tail;
}
`,
      "b.ts": `
export function orderSummary(id: string, count: number, total: number): string {
  const head = "Order " + id + " contains " + count + " items";
  const tail = "for a total of " + total + " USD";
  return head + " " + tail;
}
`,
    },
    expectPairs: [["describeOrder", "orderSummary"]],
  },
  {
    id: "F-P09",
    mode: "functions",
    category: "operator-style",
    description: "Compound assignment vs expanded assignment",
    files: {
      "a.ts": `
export function applyDiscounts(subtotal: number, discounts: number[]): number {
  let price = subtotal;
  for (const discount of discounts) {
    price -= discount;
    price = Math.max(price, 0);
  }
  return price;
}
`,
      "b.ts": `
export function deductDiscounts(subtotal: number, discounts: number[]): number {
  let price = subtotal;
  for (const discount of discounts) {
    price = price - discount;
    price = Math.max(price, 0);
  }
  return price;
}
`,
    },
    expectPairs: [["applyDiscounts", "deductDiscounts"]],
  },
  {
    id: "F-P10",
    mode: "functions",
    category: "branch-style",
    description: "Conditional return vs if/else return",
    files: {
      "a.ts": `
export function pickShippingLabel(weight: number, express: boolean): string {
  const base = weight > 20 ? "freight" : "parcel";
  if (express) {
    return base + "-express";
  }
  return base + "-standard";
}
`,
      "b.ts": `
export function chooseShippingLabel(weight: number, express: boolean): string {
  let base;
  if (weight > 20) {
    base = "freight";
  } else {
    base = "parcel";
  }
  if (express) {
    return base + "-express";
  }
  return base + "-standard";
}
`,
    },
    expectPairs: [["pickShippingLabel", "chooseShippingLabel"]],
  },
  {
    id: "F-P11",
    mode: "functions",
    category: "branch-style",
    description: "Ternary-return vs explicit if/else return",
    files: {
      "a.ts": `
export function gradeScore(score: number): string {
  if (score < 0 || score > 100) {
    throw new Error("score out of range");
  }
  return score >= 60 ? "pass" : "fail";
}
`,
      "b.ts": `
export function rateScore(score: number): string {
  if (score < 0 || score > 100) {
    throw new Error("score out of range");
  }
  if (score >= 60) {
    return "pass";
  } else {
    return "fail";
  }
}
`,
    },
    expectPairs: [["gradeScore", "rateScore"]],
  },
  {
    id: "F-P12",
    mode: "functions",
    category: "rename",
    description: "Duplicated validation block with renamed bindings",
    files: {
      "a.ts": `
export function validateUserPayload(user: { email: string; name: string }) {
  if (!user.email) {
    throw new Error("Email is required");
  }
  if (!user.email.includes("@")) {
    throw new Error("Invalid email format");
  }
  if (!user.name) {
    throw new Error("Name is required");
  }
  return user;
}

export function validateAdminPayload(admin: { email: string; name: string }) {
  if (!admin.email) {
    throw new Error("Email is required");
  }
  if (!admin.email.includes("@")) {
    throw new Error("Invalid email format");
  }
  if (!admin.name) {
    throw new Error("Name is required");
  }
  return admin;
}
`,
    },
    expectPairs: [["validateUserPayload", "validateAdminPayload"]],
  },
  {
    id: "F-P13",
    mode: "functions",
    category: "copy-paste",
    description: "Copy-pasted function with one extra logging line",
    files: {
      "a.ts": `
export function importLegacyRecords(rows: string[][]): { id: string; label: string }[] {
  const records: { id: string; label: string }[] = [];
  for (const row of rows) {
    if (row.length < 2) {
      continue;
    }
    const id = row[0].trim();
    const label = row[1].trim();
    records.push({ id, label });
  }
  return records;
}

export function importCurrentRecords(rows: string[][]): { id: string; label: string }[] {
  const records: { id: string; label: string }[] = [];
  for (const row of rows) {
    if (row.length < 2) {
      continue;
    }
    const id = row[0].trim();
    const label = row[1].trim();
    records.push({ id, label });
    console.log("imported", id);
  }
  return records;
}
`,
    },
    expectPairs: [["importLegacyRecords", "importCurrentRecords"]],
  },
  {
    id: "F-P14",
    mode: "functions",
    category: "object-style",
    description: "Object.assign with empty target vs object spread",
    files: {
      "a.ts": `
export function mergeConfig(defaults: object, overrides: object) {
  const merged = Object.assign({}, defaults, overrides, { migrated: true });
  if (!merged) {
    throw new Error("merge failed");
  }
  return merged;
}
`,
      "b.ts": `
export function combineConfig(defaults: object, overrides: object) {
  const merged = { ...defaults, ...overrides, migrated: true };
  if (!merged) {
    throw new Error("merge failed");
  }
  return merged;
}
`,
    },
    expectPairs: [["mergeConfig", "combineConfig"]],
  },
  {
    id: "F-P15",
    mode: "functions",
    category: "loop-style",
    description: "while loop vs for loop without init/update",
    files: {
      "a.ts": `
export function drainQueue(queue: { pop(): string | undefined }, sink: (v: string) => void) {
  let item = queue.pop();
  while (item !== undefined) {
    sink(item);
    item = queue.pop();
  }
  return true;
}
`,
      "b.ts": `
export function flushQueue(queue: { pop(): string | undefined }, sink: (v: string) => void) {
  let entry = queue.pop();
  for (; entry !== undefined; ) {
    sink(entry);
    entry = queue.pop();
  }
  return true;
}
`,
    },
    expectPairs: [["drainQueue", "flushQueue"]],
  },
  {
    id: "F-P16",
    mode: "functions",
    category: "branch-style",
    description: "switch over literals vs if/else-if chain",
    files: {
      "a.ts": `
export function httpStatusText(code: number): string {
  switch (code) {
    case 200:
      return "ok";
    case 404:
      return "not found";
    case 500:
      return "server error";
    default:
      return "unknown";
  }
}
`,
      "b.ts": `
export function statusCodeText(code: number): string {
  if (code === 200) {
    return "ok";
  } else if (code === 404) {
    return "not found";
  } else if (code === 500) {
    return "server error";
  } else {
    return "unknown";
  }
}
`,
    },
    expectPairs: [["httpStatusText", "statusCodeText"]],
  },
  {
    id: "F-P17",
    mode: "functions",
    category: "declaration-form",
    description: "Async arrow vs async function declaration",
    files: {
      "a.ts": `
export async function refreshSession(token: string): Promise<string> {
  const response = await fetch("/api/session", { headers: { token } });
  const payload = await response.json();
  return payload.sessionId;
}

export const renewSession = async (token: string): Promise<string> => {
  const response = await fetch("/api/session", { headers: { token } });
  const payload = await response.json();
  return payload.sessionId;
};
`,
    },
    expectPairs: [["refreshSession", "renewSession"]],
  },
  {
    id: "F-P18",
    mode: "functions",
    category: "rename",
    description: "Same-name repository methods across classes",
    files: {
      "a.ts": `
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
    },
    expectPairs: [["findById", "findById"]],
  },
  {
    id: "F-P19",
    mode: "functions",
    category: "rename",
    description: "Destructured parameter names renamed",
    files: {
      "a.ts": `
export function formatPoint({ x, y }: { x: number; y: number }): string {
  const horizontal = x.toFixed(2);
  const vertical = y.toFixed(2);
  return "(" + horizontal + ", " + vertical + ")";
}

export function renderCoordinate({ left, top }: { left: number; top: number }): string {
  const horizontal = left.toFixed(2);
  const vertical = top.toFixed(2);
  return "(" + horizontal + ", " + vertical + ")";
}
`,
    },
    expectPairs: [["formatPoint", "renderCoordinate"]],
  },
  {
    id: "F-P20",
    mode: "functions",
    category: "copy-paste",
    description: "Reordered independent declarations before identical logic",
    files: {
      "a.ts": `
export function shippingQuote(weight: number, distance: number): number {
  const ratePerKg = 1.25;
  const ratePerKm = 0.1;
  const base = weight * ratePerKg + distance * ratePerKm;
  if (base < 5) {
    return 5;
  }
  return Math.round(base * 100) / 100;
}

export function deliveryQuote(weight: number, distance: number): number {
  const ratePerKm = 0.1;
  const ratePerKg = 1.25;
  const base = weight * ratePerKg + distance * ratePerKm;
  if (base < 5) {
    return 5;
  }
  return Math.round(base * 100) / 100;
}
`,
    },
    expectPairs: [["shippingQuote", "deliveryQuote"]],
  },
  {
    id: "F-P21",
    mode: "functions",
    category: "rename",
    description: "Renamed private methods with identical bodies",
    files: {
      "a.ts": `
export class ProfileStore {
  #readEntry(key: string): string {
    const namespaced = "profile:" + key;
    const value = localStorage.getItem(namespaced);
    return value ?? "";
  }
}

export class SessionStore {
  #loadEntry(key: string): string {
    const namespaced = "profile:" + key;
    const value = localStorage.getItem(namespaced);
    return value ?? "";
  }
}
`,
    },
    expectPairs: [["#readEntry", "#loadEntry"]],
  },
  {
    id: "F-P22",
    mode: "functions",
    category: "copy-paste",
    description: "Identical try/catch wrapper bodies with renames",
    files: {
      "a.ts": `
export async function loadInventory(warehouse: string): Promise<unknown[]> {
  try {
    const response = await fetch("/api/inventory/" + warehouse);
    const items = await response.json();
    return items.filter((item: { active: boolean }) => item.active);
  } catch (error) {
    console.error("inventory load failed", error);
    return [];
  }
}

export async function loadCatalog(region: string): Promise<unknown[]> {
  try {
    const result = await fetch("/api/inventory/" + region);
    const entries = await result.json();
    return entries.filter((entry: { active: boolean }) => entry.active);
  } catch (problem) {
    console.error("inventory load failed", problem);
    return [];
  }
}
`,
    },
    expectPairs: [["loadInventory", "loadCatalog"]],
  },
  {
    id: "F-P23",
    mode: "functions",
    category: "branch-style",
    description: "Braced vs brace-less single-statement guards",
    files: {
      "a.ts": `
export function normalizeTag(tag: string): string {
  if (!tag) return "";
  const trimmed = tag.trim();
  if (trimmed.length > 32) {
    return trimmed.slice(0, 32);
  }
  return trimmed.toLowerCase();
}
`,
      "b.ts": `
export function canonicalTag(tag: string): string {
  if (!tag) {
    return "";
  }
  const trimmed = tag.trim();
  if (trimmed.length > 32) return trimmed.slice(0, 32);
  return trimmed.toLowerCase();
}
`,
    },
    expectPairs: [["normalizeTag", "canonicalTag"]],
  },
  {
    id: "F-P24",
    mode: "functions",
    category: "string-style",
    description: "Quote style and formatting differences only",
    files: {
      "a.ts": `
export function buildGreeting(name: string): string {
  const trimmed = name.trim();
  if (trimmed === '') {
    return 'Hello, guest!';
  }
  return 'Hello, ' + trimmed + '!';
}
`,
      "b.ts": `
export function makeGreeting(name: string): string {
  const trimmed = name.trim();

  if (trimmed === "") {
    return "Hello, guest!";
  }

  return "Hello, " + trimmed + "!";
}
`,
    },
    expectPairs: [["buildGreeting", "makeGreeting"]],
  },
  {
    id: "F-P25",
    mode: "functions",
    category: "async-style",
    description: "Promise chain inside method vs await in standalone function",
    files: {
      "a.ts": `
export class ReportClient {
  downloadReport(reportId: string): Promise<string> {
    return fetch("/api/reports/" + reportId).then((response) => {
      if (!response.ok) {
        throw new Error("report fetch failed");
      }
      return response.text();
    });
  }
}
`,
      "b.ts": `
export async function fetchReportBody(reportId: string): Promise<string> {
  const response = await fetch("/api/reports/" + reportId);
  if (!response.ok) {
    throw new Error("report fetch failed");
  }
  return response.text();
}
`,
    },
    expectPairs: [["downloadReport", "fetchReportBody"]],
  },
  {
    id: "F-P26",
    mode: "functions",
    category: "loop-style",
    description: "forEach with index-free callback vs for-of with renames",
    files: {
      "a.ts": `
export function collectActiveIds(records: { id: string; active: boolean }[]): string[] {
  const ids: string[] = [];
  records.forEach((record) => {
    if (record.active) {
      ids.push(record.id);
    }
  });
  return ids;
}
`,
      "b.ts": `
export function gatherEnabledIds(rows: { id: string; active: boolean }[]): string[] {
  const ids: string[] = [];
  for (const row of rows) {
    if (row.active) {
      ids.push(row.id);
    }
  }
  return ids;
}
`,
    },
    expectPairs: [["collectActiveIds", "gatherEnabledIds"]],
  },
  {
    id: "F-P27",
    mode: "functions",
    category: "string-style",
    description: "Template-only strings vs quoted strings",
    files: {
      "a.ts": `
export function auditMessage(actor: string, action: string): string {
  const stamp = new Date().toISOString();
  const summary = \`actor=\${actor} action=\${action}\`;
  return \`[audit] \${stamp} \${summary}\`;
}
`,
      "b.ts": `
export function auditLine(actor: string, action: string): string {
  const stamp = new Date().toISOString();
  const summary = "actor=" + actor + " action=" + action;
  return "[audit] " + stamp + " " + summary;
}
`,
    },
    expectPairs: [["auditMessage", "auditLine"]],
  },
  {
    id: "F-P28",
    mode: "functions",
    category: "operator-style",
    description: "String append via += vs expanded concatenation",
    files: {
      "a.ts": `
export function renderCsvRow(cells: string[]): string {
  let row = "";
  for (const cell of cells) {
    if (row.length > 0) {
      row += ",";
    }
    row += cell.replace(",", " ");
  }
  return row;
}
`,
      "b.ts": `
export function formatCsvLine(cells: string[]): string {
  let line = "";
  for (const cell of cells) {
    if (line.length > 0) {
      line = line + ",";
    }
    line = line + cell.replace(",", " ");
  }
  return line;
}
`,
    },
    expectPairs: [["renderCsvRow", "formatCsvLine"]],
  },

  // -------------------------------------------------------------------
  // functions — negatives: lookalikes a refactor plan must not contain
  // -------------------------------------------------------------------
  {
    id: "F-N01",
    mode: "functions",
    category: "trivial",
    description: "One-line arithmetic helpers with different operators",
    files: {
      "a.ts": `
export const add = (a: number, b: number) => a + b;
export const sub = (a: number, b: number) => a - b;
export const mul = (a: number, b: number) => a * b;
export const div = (a: number, b: number) => a / b;
`,
    },
    forbidPairs: [
      ["add", "sub"],
      ["mul", "div"],
      ["add", "div"],
    ],
  },
  {
    id: "F-N02",
    mode: "functions",
    category: "intent",
    description: "Loop with same shape but different intent (max vs count)",
    files: {
      "a.ts": `
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
    },
    forbidPairs: [["findMax", "countOccurrences"]],
  },
  {
    id: "F-N03",
    mode: "functions",
    category: "intent",
    description: "String fold loops whose body operation diverges",
    files: {
      "a.ts": `
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
    },
    forbidPairs: [["transformUppercase", "transformReverse"]],
  },
  {
    id: "F-N04",
    mode: "functions",
    category: "intent",
    description: "Reduce callbacks computing different aggregates",
    files: {
      "a.ts": `
export function averageLatency(samples: { ms: number }[]): number {
  if (samples.length === 0) {
    return 0;
  }
  const total = samples.reduce((sum, sample) => sum + sample.ms, 0);
  return total / samples.length;
}

export function slowestLatency(samples: { ms: number }[]): number {
  if (samples.length === 0) {
    return 0;
  }
  const worst = samples.reduce((max, sample) => Math.max(max, sample.ms), 0);
  return worst;
}
`,
    },
    forbidPairs: [["averageLatency", "slowestLatency"]],
  },
  {
    id: "F-N05",
    mode: "functions",
    category: "trivial",
    description: "Tiny accessors returning different fields",
    files: {
      "a.ts": `
export class Person {
  constructor(private name: string, private age: number) {}

  getName(): string {
    return this.name;
  }

  getAge(): number {
    return this.age;
  }
}
`,
    },
    forbidPairs: [["getName", "getAge"]],
  },
  {
    id: "F-N06",
    mode: "functions",
    category: "inverse",
    description: "Serializer vs deserializer of the same record",
    files: {
      "a.ts": `
export function serializeSession(session: { id: string; expires: number }): string {
  const payload = { id: session.id, expires: session.expires };
  const json = JSON.stringify(payload);
  return Buffer.from(json).toString("base64");
}

export function deserializeSession(raw: string): { id: string; expires: number } {
  const json = Buffer.from(raw, "base64").toString("utf8");
  const payload = JSON.parse(json);
  return { id: payload.id, expires: payload.expires };
}
`,
    },
    forbidPairs: [["serializeSession", "deserializeSession"]],
  },
  {
    id: "F-N07",
    mode: "functions",
    category: "skeleton",
    description: "Same async skeleton around genuinely different work",
    files: {
      "a.ts": `
export async function syncUserDirectory(client: { listUsers(): Promise<{ id: string; deleted: boolean }[]> }) {
  const users = await client.listUsers();
  const active = users.filter((user) => !user.deleted);
  const index = new Map(active.map((user) => [user.id, user]));
  return { count: active.length, index };
}

export async function purgeExpiredTokens(store: { scan(): Promise<{ token: string; expires: number }[]> }) {
  const tokens = await store.scan();
  const now = Date.now();
  let removed = 0;
  for (const entry of tokens) {
    if (entry.expires < now) {
      removed += 1;
    }
  }
  return removed;
}
`,
    },
    forbidPairs: [["syncUserDirectory", "purgeExpiredTokens"]],
  },
  {
    id: "F-N08",
    mode: "functions",
    category: "skeleton",
    description: "Render helpers building different elements",
    files: {
      "a.ts": `
declare function h(tag: string, props: object, ...children: unknown[]): unknown;

export function renderAlert(message: string, level: string) {
  const className = "alert alert-" + level;
  return h("div", { className, role: "alert" }, h("strong", {}, level), message);
}

export function renderBadgeList(labels: string[]) {
  const items = labels.map((label) => h("li", { className: "badge" }, label));
  return h("ul", { className: "badge-list" }, ...items);
}
`,
    },
    forbidPairs: [["renderAlert", "renderBadgeList"]],
  },
  {
    id: "F-N09",
    mode: "functions",
    category: "contract",
    description: "Sync vs async functions with the same body",
    files: {
      "a.ts": `
export async function loadValueAsync(): Promise<number> {
  const next = 42;
  return next;
}

export function loadValueSync(): number {
  const next = 42;
  return next;
}
`,
    },
    forbidPairs: [["loadValueAsync", "loadValueSync"]],
  },
  {
    id: "F-N10",
    mode: "functions",
    category: "inverse",
    description: "URL builder vs URL parser",
    files: {
      "a.ts": `
export function buildSearchUrl(base: string, terms: string[]): string {
  const encoded = terms.map((term) => encodeURIComponent(term));
  const query = encoded.join("+");
  return base + "/search?q=" + query;
}

export function parseSearchTerms(url: string): string[] {
  const query = url.split("?q=")[1] ?? "";
  const parts = query.split("+");
  return parts.map((part) => decodeURIComponent(part));
}
`,
    },
    forbidPairs: [["buildSearchUrl", "parseSearchTerms"]],
  },
  {
    id: "F-N11",
    mode: "functions",
    category: "intent",
    description: "Validators for unrelated domains (email vs port)",
    files: {
      "a.ts": `
export function validateEmail(input: string): string {
  const trimmed = input.trim();
  if (!trimmed.includes("@") || trimmed.startsWith("@")) {
    throw new Error("invalid email");
  }
  return trimmed.toLowerCase();
}

export function validatePort(input: string): number {
  const port = Number(input);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error("invalid port");
  }
  return port;
}
`,
    },
    forbidPairs: [["validateEmail", "validatePort"]],
  },
  {
    id: "F-N12",
    mode: "functions",
    category: "skeleton",
    description: "Event handlers with same wrapper, different work",
    files: {
      "a.ts": `
export async function handleUserCreated(event: { payload: { id: string; email: string } }, db: { insert(table: string, row: object): Promise<void> }, mailer: { send(to: string, template: string): Promise<void> }) {
  const { id, email } = event.payload;
  await db.insert("users", { id, email });
  await mailer.send(email, "welcome");
  return { handled: true };
}

export async function handleOrderShipped(event: { payload: { orderId: string; tracking: string } }, db: { update(table: string, key: string, row: object): Promise<void> }, queue: { push(topic: string, body: object): Promise<void> }) {
  const { orderId, tracking } = event.payload;
  await db.update("orders", orderId, { status: "shipped", tracking });
  await queue.push("notifications", { orderId, tracking });
  return { handled: true };
}
`,
    },
    forbidPairs: [["handleUserCreated", "handleOrderShipped"]],
  },
  {
    id: "F-N13",
    mode: "functions",
    category: "intent",
    description: "Math helpers with different formulas",
    files: {
      "a.ts": `
export function clampValue(value: number, min: number, max: number): number {
  const low = Math.max(value, min);
  const result = Math.min(low, max);
  return result;
}

export function lerpValue(start: number, end: number, ratio: number): number {
  const span = end - start;
  const result = start + span * ratio;
  return result;
}
`,
    },
    forbidPairs: [["clampValue", "lerpValue"]],
  },
  {
    id: "F-N14",
    mode: "functions",
    category: "inverse",
    description: "Encoder vs decoder loops",
    files: {
      "a.ts": `
export function encodeRunLength(text: string): string {
  let encoded = "";
  let index = 0;
  while (index < text.length) {
    let run = 1;
    while (index + run < text.length && text[index + run] === text[index]) {
      run += 1;
    }
    encoded += String(run) + text[index];
    index += run;
  }
  return encoded;
}

export function decodeRunLength(encoded: string): string {
  let decoded = "";
  let index = 0;
  while (index < encoded.length) {
    const run = Number(encoded[index]);
    const ch = encoded[index + 1];
    decoded += ch.repeat(run);
    index += 2;
  }
  return decoded;
}
`,
    },
    forbidPairs: [["encodeRunLength", "decodeRunLength"]],
  },

  // -------------------------------------------------------------------
  // types — positives
  // -------------------------------------------------------------------
  {
    id: "T-P01",
    mode: "types",
    category: "rename",
    description: "Renamed interface with identical members",
    files: {
      "a.ts": `
export interface UserAccount {
  id: string;
  email: string;
  displayName: string;
  createdAt: Date;
  roles: string[];
}
`,
      "b.ts": `
export interface MemberAccount {
  id: string;
  email: string;
  displayName: string;
  createdAt: Date;
  roles: string[];
}
`,
    },
    expectPairs: [["UserAccount", "MemberAccount"]],
  },
  {
    id: "T-P02",
    mode: "types",
    category: "cross-kind",
    description: "Interface vs structurally identical type alias",
    files: {
      "a.ts": `
export interface IInvoice {
  id: string;
  total: number;
  currency: string;
  lines: { sku: string; quantity: number }[];
}

export type TInvoice = {
  id: string;
  total: number;
  currency: string;
  lines: { sku: string; quantity: number }[];
};
`,
    },
    expectPairs: [["IInvoice", "TInvoice"]],
  },
  {
    id: "T-P03",
    mode: "types",
    category: "ordering",
    description: "Same properties declared in a different order",
    files: {
      "a.ts": `
export interface ShipmentRecord {
  trackingId: string;
  carrier: string;
  weightKg: number;
  destination: string;
  insured: boolean;
}
`,
      "b.ts": `
export interface ParcelRecord {
  destination: string;
  insured: boolean;
  trackingId: string;
  carrier: string;
  weightKg: number;
}
`,
    },
    expectPairs: [["ShipmentRecord", "ParcelRecord"]],
  },
  {
    id: "T-P04",
    mode: "types",
    category: "ordering",
    description: "String-literal unions with members reordered",
    files: {
      "a.ts": `
export type OrderState = "pending" | "paid" | "shipped" | "delivered";
export type FulfilmentState = "paid" | "delivered" | "pending" | "shipped";
`,
    },
    expectPairs: [["OrderState", "FulfilmentState"]],
  },
  {
    id: "T-P05",
    mode: "types",
    category: "rename",
    description: "Identical union aliases with different names",
    files: {
      "a.ts": `
export type LogLevel = "debug" | "info" | "warn" | "error";
export type Severity = "debug" | "info" | "warn" | "error";
`,
    },
    expectPairs: [["LogLevel", "Severity"]],
  },
  {
    id: "T-P06",
    mode: "types",
    category: "function-type",
    description: "Function-type aliases differing only in parameter names",
    files: {
      "a.ts": `
export type FetchHandler = (request: string, retries: number) => Promise<string>;
export type LoadHandler = (url: string, attempts: number) => Promise<string>;
`,
    },
    expectPairs: [["FetchHandler", "LoadHandler"]],
  },
  {
    id: "T-P07",
    mode: "types",
    category: "rename",
    description: "Nested object interfaces renamed",
    files: {
      "a.ts": `
export interface PaymentEvent {
  id: string;
  amount: { value: number; currency: string };
  customer: { id: string; email: string };
  capturedAt: Date;
}
`,
      "b.ts": `
export interface ChargeEvent {
  id: string;
  amount: { value: number; currency: string };
  customer: { id: string; email: string };
  capturedAt: Date;
}
`,
    },
    expectPairs: [["PaymentEvent", "ChargeEvent"]],
  },
  {
    id: "T-P08",
    mode: "types",
    category: "modifier",
    description: "Readonly modifiers should not hide duplication",
    files: {
      "a.ts": `
export interface SnapshotMeta {
  readonly id: string;
  readonly takenAt: Date;
  sizeBytes: number;
  label: string;
}

export interface BackupMeta {
  id: string;
  takenAt: Date;
  sizeBytes: number;
  label: string;
}
`,
    },
    expectPairs: [["SnapshotMeta", "BackupMeta"]],
  },
  {
    id: "T-P09",
    mode: "types",
    category: "rename",
    description: "Identical Record aliases",
    files: {
      "a.ts": `
export type CountsByDay = Record<string, number>;
export type DailyTotals = Record<string, number>;
`,
    },
    expectPairs: [["CountsByDay", "DailyTotals"]],
  },
  {
    id: "T-P10",
    mode: "types",
    category: "modifier",
    description: "One optional marker difference out of five properties",
    files: {
      "a.ts": `
export interface WebhookConfig {
  url: string;
  secret: string;
  retries: number;
  timeoutMs: number;
  description?: string;
}

export interface CallbackConfig {
  url: string;
  secret: string;
  retries: number;
  timeoutMs: number;
  description: string;
}
`,
    },
    expectPairs: [["WebhookConfig", "CallbackConfig"]],
  },

  // -------------------------------------------------------------------
  // types — negatives
  // -------------------------------------------------------------------
  {
    id: "T-N01",
    mode: "types",
    category: "generic-shape",
    description: "Generic API wrappers with different field names",
    files: {
      "a.ts": `
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
    },
    forbidPairs: [
      ["Response", "ApiResult"],
      ["ApiResult", "ServerResponse"],
      ["Response", "ServerResponse"],
    ],
  },
  {
    id: "T-N02",
    mode: "types",
    category: "small-shape",
    description: "Unrelated small object types",
    files: {
      "a.ts": `
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
    },
    forbidPairs: [
      ["Point", "User"],
      ["User", "Config"],
      ["Point", "Config"],
    ],
  },
  {
    id: "T-N03",
    mode: "types",
    category: "union",
    description: "Unions over different literal sets",
    files: {
      "a.ts": `
export type Alignment = "left" | "center" | "right";
export type Theme = "light" | "dark" | "system" | "high-contrast";
`,
    },
    forbidPairs: [["Alignment", "Theme"]],
  },
  {
    id: "T-N04",
    mode: "types",
    category: "partial-overlap",
    description: "Entities sharing id/name but otherwise different",
    files: {
      "a.ts": `
export interface Product {
  id: string;
  name: string;
  priceCents: number;
  stock: number;
  tags: string[];
}

export interface Employee {
  id: string;
  name: string;
  department: string;
  hiredAt: Date;
  manager?: string;
}
`,
    },
    forbidPairs: [["Product", "Employee"]],
  },
  {
    id: "T-N05",
    mode: "types",
    category: "alias-body",
    description: "Record aliases with different value types",
    files: {
      "a.ts": `
export type LabelsById = Record<string, string>;
export type FlagsById = Record<string, boolean>;
`,
    },
    forbidPairs: [["LabelsById", "FlagsById"]],
  },
  {
    id: "T-N06",
    mode: "types",
    category: "function-type",
    description: "Function-type aliases with different signatures",
    files: {
      "a.ts": `
export type RetryPolicy = (attempt: number, error: Error) => boolean;
export type BackoffSchedule = (attempt: number) => Promise<number>;
`,
    },
    forbidPairs: [["RetryPolicy", "BackoffSchedule"]],
  },

  // -------------------------------------------------------------------
  // classes — positives
  // -------------------------------------------------------------------
  {
    id: "C-P01",
    mode: "classes",
    category: "rename",
    description: "Renamed cache classes with renamed private storage",
    files: {
      "a.ts": `
export class UserCache {
  private store = new Map<string, unknown>();
  get(key: string): unknown { return this.store.get(key); }
  set(key: string, value: unknown): void { this.store.set(key, value); }
  delete(key: string): boolean { return this.store.delete(key); }
  clear(): void { this.store.clear(); }
}
`,
      "b.ts": `
export class SessionCache {
  private items = new Map<string, unknown>();
  get(key: string): unknown { return this.items.get(key); }
  set(key: string, value: unknown): void { this.items.set(key, value); }
  delete(key: string): boolean { return this.items.delete(key); }
  clear(): void { this.items.clear(); }
}
`,
    },
    expectPairs: [["UserCache", "SessionCache"]],
  },
  {
    id: "C-P02",
    mode: "classes",
    category: "ordering",
    description: "Same members declared in a different order",
    files: {
      "a.ts": `
export class MetricsBuffer {
  private samples: number[] = [];
  add(sample: number): void { this.samples.push(sample); }
  count(): number { return this.samples.length; }
  reset(): void { this.samples = []; }
}
`,
      "b.ts": `
export class TelemetryBuffer {
  private samples: number[] = [];
  reset(): void { this.samples = []; }
  count(): number { return this.samples.length; }
  add(sample: number): void { this.samples.push(sample); }
}
`,
    },
    expectPairs: [["MetricsBuffer", "TelemetryBuffer"]],
  },
  {
    id: "C-P03",
    mode: "classes",
    category: "rename",
    description: "Repository clones for different entities",
    files: {
      "a.ts": `
export class OrderRepository {
  constructor(private baseUrl: string) {}
  async findById(id: string): Promise<unknown> {
    const response = await fetch(this.baseUrl + "/orders/" + id);
    return response.json();
  }
  async remove(id: string): Promise<void> {
    await fetch(this.baseUrl + "/orders/" + id, { method: "DELETE" });
  }
}
`,
      "b.ts": `
export class InvoiceRepository {
  constructor(private rootUrl: string) {}
  async findById(key: string): Promise<unknown> {
    const reply = await fetch(this.rootUrl + "/orders/" + key);
    return reply.json();
  }
  async remove(key: string): Promise<void> {
    await fetch(this.rootUrl + "/orders/" + key, { method: "DELETE" });
  }
}
`,
    },
    expectPairs: [["OrderRepository", "InvoiceRepository"]],
  },
  {
    id: "C-P04",
    mode: "classes",
    category: "rename",
    description: "Value-object classes with renamed getters",
    files: {
      "a.ts": `
export class TemperatureReading {
  constructor(private celsius: number, private at: Date) {}
  get value(): number { return this.celsius; }
  get recordedAt(): Date { return this.at; }
  isFreezing(): boolean { return this.celsius <= 0; }
}
`,
      "b.ts": `
export class PressureReading {
  constructor(private pascals: number, private at: Date) {}
  get value(): number { return this.pascals; }
  get recordedAt(): Date { return this.at; }
  isFreezing(): boolean { return this.pascals <= 0; }
}
`,
    },
    expectPairs: [["TemperatureReading", "PressureReading"]],
  },

  // -------------------------------------------------------------------
  // classes — negatives
  // -------------------------------------------------------------------
  {
    id: "C-N01",
    mode: "classes",
    category: "skeleton",
    description: "Same member count, different responsibilities",
    files: {
      "a.ts": `
export class EmailService {
  constructor(private transport: { deliver(to: string, body: string): Promise<void> }) {}
  async sendWelcome(to: string): Promise<void> {
    await this.transport.deliver(to, "welcome aboard");
  }
  async sendReceipt(to: string, total: number): Promise<void> {
    await this.transport.deliver(to, "you paid " + total);
  }
}
`,
      "b.ts": `
export class RateLimiter {
  constructor(private maxPerMinute: number) {}
  private hits: number[] = [];
  allow(now: number): boolean {
    this.hits = this.hits.filter((t) => now - t < 60000);
    if (this.hits.length >= this.maxPerMinute) return false;
    this.hits.push(now);
    return true;
  }
}
`,
    },
    forbidPairs: [["EmailService", "RateLimiter"]],
  },
  {
    id: "C-N02",
    mode: "classes",
    category: "skeleton",
    description: "Stack vs key-value bag",
    files: {
      "a.ts": `
export class TaskStack {
  private tasks: string[] = [];
  push(task: string): void { this.tasks.push(task); }
  pop(): string | undefined { return this.tasks.pop(); }
  size(): number { return this.tasks.length; }
}
`,
      "b.ts": `
export class HeaderBag {
  private headers = new Map<string, string>();
  set(name: string, value: string): void { this.headers.set(name.toLowerCase(), value); }
  get(name: string): string | undefined { return this.headers.get(name.toLowerCase()); }
  has(name: string): boolean { return this.headers.has(name.toLowerCase()); }
}
`,
    },
    forbidPairs: [["TaskStack", "HeaderBag"]],
  },
  {
    id: "C-N03",
    mode: "classes",
    category: "intent",
    description: "Validator vs transformer with similar arity",
    files: {
      "a.ts": `
export class InputValidator {
  private errors: string[] = [];
  check(condition: boolean, message: string): void {
    if (!condition) this.errors.push(message);
  }
  isValid(): boolean { return this.errors.length === 0; }
  report(): string[] { return [...this.errors]; }
}
`,
      "b.ts": `
export class TextPipeline {
  private steps: ((text: string) => string)[] = [];
  use(step: (text: string) => string): void {
    this.steps.push(step);
  }
  run(input: string): string { return this.steps.reduce((acc, step) => step(acc), input); }
  length(): number { return this.steps.length; }
}
`,
    },
    forbidPairs: [["InputValidator", "TextPipeline"]],
  },
];
