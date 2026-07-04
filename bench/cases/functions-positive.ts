/**
 * Positive-only accuracy corpus for `functions` mode.
 *
 * Every case bundles two or three spellings of THE SAME routine — code a
 * senior reviewer would call "the same function written differently".
 * Behavior is identical including edge cases and side-effect order, modulo
 * the accepted caveats (template literal vs string concat, coercion-hint
 * nuances, array iteration-protocol equivalence).
 *
 * All `expectPairs` are duplicates the analyzer must report at the README
 * defaults (threshold 0.8, minLines 3); missing any pair is a false
 * negative. This file measures recall only — no `forbidPairs`.
 *
 * IDs XF-P01..XF-P56 across categories: rename-alpha, guard-style,
 * negation-swap, logic-sugar, destructure, loop-form, micro-idiom, combo.
 */

import type { BenchCase } from "../cases.js";

export const functionPositiveCases: BenchCase[] = [
  // -------------------------------------------------------------------
  // rename-alpha — consistent renaming of function, params, and locals
  // -------------------------------------------------------------------
  {
    id: "XF-P01",
    mode: "functions",
    category: "rename-alpha",
    description: "Tiny clamp helper under three consistent renamings",
    files: {
      "a.ts": `
export function clampRatio(value: number): number {
  const bounded = Math.min(1, Math.max(0, value));
  return bounded;
}

export function clampFraction(amount: number): number {
  const limited = Math.min(1, Math.max(0, amount));
  return limited;
}

export function clampShare(portion: number): number {
  const pinned = Math.min(1, Math.max(0, portion));
  return pinned;
}
`,
    },
    expectPairs: [
      ["clampRatio", "clampFraction"],
      ["clampRatio", "clampShare"],
      ["clampFraction", "clampShare"],
    ],
  },
  {
    id: "XF-P02",
    mode: "functions",
    category: "rename-alpha",
    description: "Slug helper with all locals renamed, body otherwise identical",
    files: {
      "a.ts": `
export function slugifyTitle(title: string): string {
  const lowered = title.toLowerCase().trim();
  const joined = lowered.replace(/ +/g, "-");
  return joined;
}

export function slugifyHeading(heading: string): string {
  const reduced = heading.toLowerCase().trim();
  const dashed = reduced.replace(/ +/g, "-");
  return dashed;
}

export function slugifyCaption(caption: string): string {
  const folded = caption.toLowerCase().trim();
  const hyphenated = folded.replace(/ +/g, "-");
  return hyphenated;
}
`,
    },
    expectPairs: [
      ["slugifyTitle", "slugifyHeading"],
      ["slugifyTitle", "slugifyCaption"],
      ["slugifyHeading", "slugifyCaption"],
    ],
  },
  {
    id: "XF-P03",
    mode: "functions",
    category: "rename-alpha",
    description: "Guarded accumulator-loop average, fully renamed across files",
    files: {
      "a.ts": `
export function averageLatency(samples: number[]): number {
  if (samples.length === 0) {
    return 0;
  }
  let total = 0;
  for (const sample of samples) {
    total += sample;
  }
  return total / samples.length;
}
`,
      "b.ts": `
export function meanReadingValue(readings: number[]): number {
  if (readings.length === 0) {
    return 0;
  }
  let sum = 0;
  for (const reading of readings) {
    sum += reading;
  }
  return sum / readings.length;
}

export function meanTravelTime(durations: number[]): number {
  if (durations.length === 0) {
    return 0;
  }
  let acc = 0;
  for (const duration of durations) {
    acc += duration;
  }
  return acc / durations.length;
}
`,
    },
    expectPairs: [
      ["averageLatency", "meanReadingValue"],
      ["averageLatency", "meanTravelTime"],
      ["meanReadingValue", "meanTravelTime"],
    ],
  },
  {
    id: "XF-P04",
    mode: "functions",
    category: "rename-alpha",
    description: "Early-return string validator under full renaming",
    files: {
      "a.ts": `
export function validateUsername(candidate: string): string | null {
  const trimmed = candidate.trim();
  if (trimmed.length < 3) {
    return "too short";
  }
  if (trimmed.length > 20) {
    return "too long";
  }
  if (trimmed.includes(" ")) {
    return "contains spaces";
  }
  return null;
}

export function checkHandle(input: string): string | null {
  const cleaned = input.trim();
  if (cleaned.length < 3) {
    return "too short";
  }
  if (cleaned.length > 20) {
    return "too long";
  }
  if (cleaned.includes(" ")) {
    return "contains spaces";
  }
  return null;
}

export function auditNickname(raw: string): string | null {
  const normalized = raw.trim();
  if (normalized.length < 3) {
    return "too short";
  }
  if (normalized.length > 20) {
    return "too long";
  }
  if (normalized.includes(" ")) {
    return "contains spaces";
  }
  return null;
}
`,
    },
    expectPairs: [
      ["validateUsername", "checkHandle"],
      ["validateUsername", "auditNickname"],
      ["checkHandle", "auditNickname"],
    ],
  },
  {
    id: "XF-P05",
    mode: "functions",
    category: "rename-alpha",
    description: "Small async/await fetch-text helper, fully renamed",
    files: {
      "a.ts": `
export async function loadConfigText(path: string): Promise<string> {
  const response = await fetch(path);
  const body = await response.text();
  return body;
}

export async function readRemoteText(url: string): Promise<string> {
  const reply = await fetch(url);
  const payload = await reply.text();
  return payload;
}

export async function pullDocumentText(target: string): Promise<string> {
  const result = await fetch(target);
  const content = await result.text();
  return content;
}
`,
    },
    expectPairs: [
      ["loadConfigText", "readRemoteText"],
      ["loadConfigText", "pullDocumentText"],
      ["readRemoteText", "pullDocumentText"],
    ],
  },
  {
    id: "XF-P06",
    mode: "functions",
    category: "rename-alpha",
    description: "Async retry loop with try/catch, consistently renamed",
    files: {
      "a.ts": `
export async function fetchJsonWithRetry(url: string, attempts: number): Promise<unknown> {
  let lastError: unknown = null;
  for (let attempt = 0; attempt < attempts; attempt++) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return await response.json();
      }
      lastError = new Error("HTTP " + String(response.status));
    } catch (error) {
      lastError = error;
    }
  }
  throw lastError;
}
`,
      "b.ts": `
export async function loadJsonWithRetries(endpoint: string, rounds: number): Promise<unknown> {
  let finalError: unknown = null;
  for (let round = 0; round < rounds; round++) {
    try {
      const reply = await fetch(endpoint);
      if (reply.ok) {
        return await reply.json();
      }
      finalError = new Error("HTTP " + String(reply.status));
    } catch (failure) {
      finalError = failure;
    }
  }
  throw finalError;
}

export async function requestJsonRepeatedly(target: string, tries: number): Promise<unknown> {
  let storedError: unknown = null;
  for (let pass = 0; pass < tries; pass++) {
    try {
      const outcome = await fetch(target);
      if (outcome.ok) {
        return await outcome.json();
      }
      storedError = new Error("HTTP " + String(outcome.status));
    } catch (problem) {
      storedError = problem;
    }
  }
  throw storedError;
}
`,
    },
    expectPairs: [
      ["fetchJsonWithRetry", "loadJsonWithRetries"],
      ["fetchJsonWithRetry", "requestJsonRepeatedly"],
      ["loadJsonWithRetries", "requestJsonRepeatedly"],
    ],
  },
  {
    id: "XF-P07",
    mode: "functions",
    category: "rename-alpha",
    description: "Class method vs renamed standalone functions, same tax math",
    files: {
      "a.ts": `
export class OrderPricer {
  totalWithTax(subtotal: number, taxRate: number): number {
    const tax = subtotal * taxRate;
    const combined = subtotal + tax;
    return Math.round(combined * 100) / 100;
  }
}
`,
      "b.ts": `
export function priceWithVat(net: number, vatRate: number): number {
  const vat = net * vatRate;
  const gross = net + vat;
  return Math.round(gross * 100) / 100;
}

export function amountWithLevy(base: number, levyRate: number): number {
  const levy = base * levyRate;
  const charged = base + levy;
  return Math.round(charged * 100) / 100;
}
`,
    },
    expectPairs: [
      ["totalWithTax", "priceWithVat"],
      ["totalWithTax", "amountWithLevy"],
      ["priceWithVat", "amountWithLevy"],
    ],
  },
  {
    id: "XF-P08",
    mode: "functions",
    category: "rename-alpha",
    description: "Order-summary fold with three counters, fully renamed",
    files: {
      "a.ts": `
export function summarizeOrders(orders: { total: number; status: string }[]): { count: number; revenue: number; open: number } {
  let count = 0;
  let revenue = 0;
  let open = 0;
  for (const order of orders) {
    count += 1;
    revenue += order.total;
    if (order.status === "open") {
      open += 1;
    }
  }
  return { count: count, revenue: revenue, open: open };
}
`,
      "b.ts": `
export function summarizeRecords(records: { total: number; status: string }[]): { count: number; revenue: number; open: number } {
  let tally = 0;
  let income = 0;
  let pending = 0;
  for (const record of records) {
    tally += 1;
    income += record.total;
    if (record.status === "open") {
      pending += 1;
    }
  }
  return { count: tally, revenue: income, open: pending };
}

export function summarizeEntries(entries: { total: number; status: string }[]): { count: number; revenue: number; open: number } {
  let num = 0;
  let earned = 0;
  let active = 0;
  for (const entry of entries) {
    num += 1;
    earned += entry.total;
    if (entry.status === "open") {
      active += 1;
    }
  }
  return { count: num, revenue: earned, open: active };
}
`,
    },
    expectPairs: [
      ["summarizeOrders", "summarizeRecords"],
      ["summarizeOrders", "summarizeEntries"],
      ["summarizeRecords", "summarizeEntries"],
    ],
  },
  {
    id: "XF-P09",
    mode: "functions",
    category: "rename-alpha",
    description: "Four-line modulo predicate under full renaming",
    files: {
      "a.ts": `
export function isWeekendDay(day: number): boolean {
  const wrapped = day % 7;
  return wrapped === 0 || wrapped === 6;
}

export function isRestDay(weekday: number): boolean {
  const folded = weekday % 7;
  return folded === 0 || folded === 6;
}

export function fallsOnWeekend(index: number): boolean {
  const reduced = index % 7;
  return reduced === 0 || reduced === 6;
}
`,
    },
    expectPairs: [
      ["isWeekendDay", "isRestDay"],
      ["isWeekendDay", "fallsOnWeekend"],
      ["isRestDay", "fallsOnWeekend"],
    ],
  },
  {
    id: "XF-P10",
    mode: "functions",
    category: "rename-alpha",
    description: "19-line quoted-CSV splitter, all state variables renamed",
    files: {
      "a.ts": `
export function parseCsvLine(line: string): string[] {
  const cells: string[] = [];
  let current = "";
  let inQuotes = false;
  for (const ch of line) {
    if (ch === '"') {
      inQuotes = !inQuotes;
      continue;
    }
    if (ch === "," && !inQuotes) {
      cells.push(current);
      current = "";
      continue;
    }
    current += ch;
  }
  cells.push(current);
  return cells;
}
`,
      "b.ts": `
export function splitCsvRow(row: string): string[] {
  const fields: string[] = [];
  let buffer = "";
  let quoted = false;
  for (const char of row) {
    if (char === '"') {
      quoted = !quoted;
      continue;
    }
    if (char === "," && !quoted) {
      fields.push(buffer);
      buffer = "";
      continue;
    }
    buffer += char;
  }
  fields.push(buffer);
  return fields;
}

export function tokenizeCsvRecord(record: string): string[] {
  const parts: string[] = [];
  let pending = "";
  let insideQuotes = false;
  for (const glyph of record) {
    if (glyph === '"') {
      insideQuotes = !insideQuotes;
      continue;
    }
    if (glyph === "," && !insideQuotes) {
      parts.push(pending);
      pending = "";
      continue;
    }
    pending += glyph;
  }
  parts.push(pending);
  return parts;
}
`,
    },
    expectPairs: [
      ["parseCsvLine", "splitCsvRow"],
      ["parseCsvLine", "tokenizeCsvRecord"],
      ["splitCsvRow", "tokenizeCsvRecord"],
    ],
  },
  {
    id: "XF-P11",
    mode: "functions",
    category: "rename-alpha",
    description: "Count-by-key accumulator over records, fully renamed",
    files: {
      "a.ts": `
export function countByStatus(items: { status: string }[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const item of items) {
    const key = item.status;
    const previous = counts[key] ?? 0;
    counts[key] = previous + 1;
  }
  return counts;
}

export function tallyStatuses(rows: { status: string }[]): Record<string, number> {
  const tally: Record<string, number> = {};
  for (const row of rows) {
    const bucket = row.status;
    const prior = tally[bucket] ?? 0;
    tally[bucket] = prior + 1;
  }
  return tally;
}

export function histogramOfStatuses(tickets: { status: string }[]): Record<string, number> {
  const histogram: Record<string, number> = {};
  for (const ticket of tickets) {
    const label = ticket.status;
    const existing = histogram[label] ?? 0;
    histogram[label] = existing + 1;
  }
  return histogram;
}
`,
    },
    expectPairs: [
      ["countByStatus", "tallyStatuses"],
      ["countByStatus", "histogramOfStatuses"],
      ["tallyStatuses", "histogramOfStatuses"],
    ],
  },
  {
    id: "XF-P12",
    mode: "functions",
    category: "rename-alpha",
    description: "12-line early-return port validator, parameter renamed",
    files: {
      "a.ts": `
export function validatePortNumber(value: number): boolean {
  if (!Number.isInteger(value)) {
    return false;
  }
  if (value <= 0) {
    return false;
  }
  if (value > 65535) {
    return false;
  }
  return true;
}

export function isUsablePort(port: number): boolean {
  if (!Number.isInteger(port)) {
    return false;
  }
  if (port <= 0) {
    return false;
  }
  if (port > 65535) {
    return false;
  }
  return true;
}

export function checkTcpPort(candidate: number): boolean {
  if (!Number.isInteger(candidate)) {
    return false;
  }
  if (candidate <= 0) {
    return false;
  }
  if (candidate > 65535) {
    return false;
  }
  return true;
}
`,
    },
    expectPairs: [
      ["validatePortNumber", "isUsablePort"],
      ["validatePortNumber", "checkTcpPort"],
      ["isUsablePort", "checkTcpPort"],
    ],
  },

  // -------------------------------------------------------------------
  // guard-style — else-return vs early-return vs ternary-return
  // -------------------------------------------------------------------
  {
    id: "XF-P13",
    mode: "functions",
    category: "guard-style",
    description: "if/else return vs guard return vs ternary return",
    files: {
      "a.ts": `
export function gradeWithElse(score: number): string {
  if (score >= 60) {
    return "pass";
  } else {
    return "fail";
  }
}

export function gradeWithGuard(score: number): string {
  if (score >= 60) {
    return "pass";
  }
  return "fail";
}

export function gradeWithTernary(score: number): string {
  return score >= 60 ? "pass" : "fail";
}
`,
    },
    expectPairs: [
      ["gradeWithElse", "gradeWithGuard"],
      ["gradeWithElse", "gradeWithTernary"],
      ["gradeWithGuard", "gradeWithTernary"],
    ],
  },
  {
    id: "XF-P14",
    mode: "functions",
    category: "guard-style",
    description: "throw in else vs flat throw after returning guard",
    files: {
      "a.ts": `
export function requirePositiveDelta(delta: number): number {
  if (delta > 0) {
    return delta;
  } else {
    throw new Error("delta must be positive");
  }
}

export function assertPositiveDelta(delta: number): number {
  if (delta > 0) {
    return delta;
  }
  throw new Error("delta must be positive");
}
`,
    },
    expectPairs: [["requirePositiveDelta", "assertPositiveDelta"]],
  },
  {
    id: "XF-P15",
    mode: "functions",
    category: "guard-style",
    description: "multi-statement else tail hoisted out of the branch",
    files: {
      "a.ts": `
export function renderBadgeBranchy(count: number): string {
  if (count === 0) {
    return "";
  } else {
    const capped = Math.min(count, 99);
    const suffix = capped === 99 ? "+" : "";
    return String(capped) + suffix;
  }
}

export function renderBadgeFlat(count: number): string {
  if (count === 0) {
    return "";
  }
  const capped = Math.min(count, 99);
  const suffix = capped === 99 ? "+" : "";
  return String(capped) + suffix;
}
`,
    },
    expectPairs: [["renderBadgeBranchy", "renderBadgeFlat"]],
  },
  {
    id: "XF-P16",
    mode: "functions",
    category: "guard-style",
    description: "nested ternary chain vs else-if return ladder",
    files: {
      "a.ts": `
export function sizeLabelChain(bytes: number): string {
  return bytes >= 1048576 ? "large" : bytes >= 1024 ? "medium" : "small";
}

export function sizeLabelLadder(bytes: number): string {
  if (bytes >= 1048576) {
    return "large";
  } else if (bytes >= 1024) {
    return "medium";
  } else {
    return "small";
  }
}
`,
    },
    expectPairs: [["sizeLabelChain", "sizeLabelLadder"]],
  },
  {
    id: "XF-P17",
    mode: "functions",
    category: "guard-style",
    description: "jump-terminated switch vs guard ladder",
    files: {
      "a.ts": `
export function httpStatusTextSwitch(code: number): string {
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

export function httpStatusTextGuards(code: number): string {
  if (code === 200) {
    return "ok";
  }
  if (code === 404) {
    return "not found";
  }
  if (code === 500) {
    return "server error";
  }
  return "unknown";
}
`,
    },
    expectPairs: [["httpStatusTextSwitch", "httpStatusTextGuards"]],
  },

  // -------------------------------------------------------------------
  // negation-swap — inverted tests with swapped branches, De Morgan
  // -------------------------------------------------------------------
  {
    id: "XF-P18",
    mode: "functions",
    category: "negation-swap",
    description: "negated test with swapped branches",
    files: {
      "a.ts": `
declare function startCollector(): void;
declare function stopCollector(): void;

export function toggleTrackingNegated(enabled: boolean): string {
  if (!enabled) {
    startCollector();
    return "started";
  } else {
    stopCollector();
    return "stopped";
  }
}

export function toggleTrackingPlain(enabled: boolean): string {
  if (enabled) {
    stopCollector();
    return "stopped";
  } else {
    startCollector();
    return "started";
  }
}
`,
    },
    expectPairs: [["toggleTrackingNegated", "toggleTrackingPlain"]],
  },
  {
    id: "XF-P19",
    mode: "functions",
    category: "negation-swap",
    description: "strict inequality guard vs flipped equality guard",
    files: {
      "a.ts": `
export function describeEnvA(env: string): string {
  if (env !== "production") {
    return "sandbox traffic";
  }
  return "live traffic";
}

export function describeEnvB(env: string): string {
  if (env === "production") {
    return "live traffic";
  }
  return "sandbox traffic";
}
`,
    },
    expectPairs: [["describeEnvA", "describeEnvB"]],
  },
  {
    id: "XF-P20",
    mode: "functions",
    category: "negation-swap",
    description: "De Morgan over && inside a guard",
    files: {
      "a.ts": `
export function canShipWrapped(paid: boolean, packed: boolean): string {
  if (!(paid && packed)) {
    return "blocked";
  }
  return "ready";
}

export function canShipDistributed(paid: boolean, packed: boolean): string {
  if (!paid || !packed) {
    return "blocked";
  }
  return "ready";
}
`,
    },
    expectPairs: [["canShipWrapped", "canShipDistributed"]],
  },
  {
    id: "XF-P21",
    mode: "functions",
    category: "negation-swap",
    description: "De Morgan over || plus negated-equality flip",
    files: {
      "a.ts": `
export function isQuietHourWrapped(hour: number, muted: boolean): boolean {
  if (!(muted || hour === 22)) {
    return false;
  }
  return true;
}

export function isQuietHourDistributed(hour: number, muted: boolean): boolean {
  if (!muted && hour !== 22) {
    return false;
  }
  return true;
}
`,
    },
    expectPairs: [["isQuietHourWrapped", "isQuietHourDistributed"]],
  },

  // -------------------------------------------------------------------
  // logic-sugar — ||/&&/?? vs ternary, null-check spellings
  // -------------------------------------------------------------------
  {
    id: "XF-P22",
    mode: "functions",
    category: "logic-sugar",
    description: "self-selecting ternary vs logical-or fallback",
    files: {
      "a.ts": `
export function displayNameTernary(nickname: string, fullName: string): string {
  const chosen = nickname ? nickname : fullName;
  const trimmed = chosen.trim();
  return trimmed;
}

export function displayNameOr(nickname: string, fullName: string): string {
  const chosen = nickname || fullName;
  const trimmed = chosen.trim();
  return trimmed;
}
`,
    },
    expectPairs: [["displayNameTernary", "displayNameOr"]],
  },
  {
    id: "XF-P23",
    mode: "functions",
    category: "logic-sugar",
    description: "nullish ternary spellings vs ?? operator",
    files: {
      "a.ts": `
export function pageSizeVerbose(requested: number | null, fallback: number): number {
  const size = requested == null ? fallback : requested;
  const bounded = Math.min(size, 200);
  return bounded;
}

export function pageSizeInverse(requested: number | null, fallback: number): number {
  const size = requested != null ? requested : fallback;
  const bounded = Math.min(size, 200);
  return bounded;
}

export function pageSizeCoalesce(requested: number | null, fallback: number): number {
  const size = requested ?? fallback;
  const bounded = Math.min(size, 200);
  return bounded;
}
`,
    },
    expectPairs: [
      ["pageSizeVerbose", "pageSizeInverse"],
      ["pageSizeVerbose", "pageSizeCoalesce"],
      ["pageSizeInverse", "pageSizeCoalesce"],
    ],
  },
  {
    id: "XF-P24",
    mode: "functions",
    category: "logic-sugar",
    description: "strict null pair vs loose == null guard",
    files: {
      "a.ts": `
export function labelValueStrict(value: string | null | undefined): string {
  if (value === null || value === undefined) {
    return "(empty)";
  }
  return value.toUpperCase();
}

export function labelValueLoose(value: string | null | undefined): string {
  if (value == null) {
    return "(empty)";
  }
  return value.toUpperCase();
}
`,
    },
    expectPairs: [["labelValueStrict", "labelValueLoose"]],
  },
  {
    id: "XF-P25",
    mode: "functions",
    category: "logic-sugar",
    description: "void 0 vs undefined comparison",
    files: {
      "a.ts": `
export function hasSlotVoid(slot: string | undefined): boolean {
  if (slot === void 0) {
    return false;
  }
  return slot.length > 0;
}

export function hasSlotUndefined(slot: string | undefined): boolean {
  if (slot === undefined) {
    return false;
  }
  return slot.length > 0;
}
`,
    },
    expectPairs: [["hasSlotVoid", "hasSlotUndefined"]],
  },
  {
    id: "XF-P26",
    mode: "functions",
    category: "logic-sugar",
    description: "Boolean() vs !! coercion before a guard",
    files: {
      "a.ts": `
export function summarizeFlagsBooleanCall(flags: string[]): string {
  const active = Boolean(flags.length);
  if (active) {
    return flags.join(",");
  }
  return "none";
}

export function summarizeFlagsDoubleNot(flags: string[]): string {
  const active = !!flags.length;
  if (active) {
    return flags.join(",");
  }
  return "none";
}
`,
    },
    expectPairs: [["summarizeFlagsBooleanCall", "summarizeFlagsDoubleNot"]],
  },

  // -------------------------------------------------------------------
  // destructure — object destructuring vs member-access declarations
  // -------------------------------------------------------------------
  {
    id: "XF-P27",
    mode: "functions",
    category: "destructure",
    description: "plain destructuring vs member declarations",
    files: {
      "a.ts": `
interface Endpoint {
  host: string;
  port: number;
}

export function formatEndpointDestructured(endpoint: Endpoint): string {
  const { host, port } = endpoint;
  const rendered = host + ":" + port;
  return rendered;
}

export function formatEndpointMembers(endpoint: Endpoint): string {
  const host = endpoint.host;
  const port = endpoint.port;
  const rendered = host + ":" + port;
  return rendered;
}
`,
    },
    expectPairs: [["formatEndpointDestructured", "formatEndpointMembers"]],
  },
  {
    id: "XF-P28",
    mode: "functions",
    category: "destructure",
    description: "renaming destructuring vs member declaration",
    files: {
      "a.ts": `
interface Envelope {
  payload: string;
  ts: number;
}

export function unpackRenamed(envelope: Envelope): string {
  const { payload: body } = envelope;
  const stamped = body + "@" + envelope.ts;
  return stamped;
}

export function unpackMember(envelope: Envelope): string {
  const body = envelope.payload;
  const stamped = body + "@" + envelope.ts;
  return stamped;
}
`,
    },
    expectPairs: [["unpackRenamed", "unpackMember"]],
  },
  {
    id: "XF-P29",
    mode: "functions",
    category: "destructure",
    description: "two-property destructuring inside a mapper",
    files: {
      "a.ts": `
interface RawUser {
  id: string;
  email: string;
  active: boolean;
}

export function toContactDestructured(raw: RawUser): { id: string; email: string } {
  const { id, email } = raw;
  return { id, email };
}

export function toContactMembers(raw: RawUser): { id: string; email: string } {
  const id = raw.id;
  const email = raw.email;
  return { id, email };
}
`,
    },
    expectPairs: [["toContactDestructured", "toContactMembers"]],
  },

  // -------------------------------------------------------------------
  // loop-form — index loops, for-of, forEach, push loops vs map/filter
  // -------------------------------------------------------------------
  {
    id: "XF-P30",
    mode: "functions",
    category: "loop-form",
    description: "index loop with element temp vs for-of vs forEach",
    files: {
      "a.ts": `
declare function primeCache(key: string): void;

export function warmCachesIndexed(keys: string[]): number {
  let warmed = 0;
  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    primeCache(key);
    warmed += 1;
  }
  return warmed;
}

export function warmCachesForOf(keys: string[]): number {
  let warmed = 0;
  for (const key of keys) {
    primeCache(key);
    warmed += 1;
  }
  return warmed;
}

export function warmCachesForEach(keys: string[]): number {
  let warmed = 0;
  keys.forEach((key) => {
    primeCache(key);
    warmed += 1;
  });
  return warmed;
}
`,
    },
    expectPairs: [
      ["warmCachesIndexed", "warmCachesForOf"],
      ["warmCachesIndexed", "warmCachesForEach"],
      ["warmCachesForOf", "warmCachesForEach"],
    ],
  },
  {
    id: "XF-P31",
    mode: "functions",
    category: "loop-form",
    description: "push loop vs .map with arrow",
    files: {
      "a.ts": `
export function centsToDollarsLoop(cents: number[]): string[] {
  const rendered = [];
  for (const amount of cents) {
    rendered.push((amount / 100).toFixed(2));
  }
  return rendered;
}

export function centsToDollarsMap(cents: number[]): string[] {
  return cents.map((amount) => (amount / 100).toFixed(2));
}
`,
    },
    expectPairs: [["centsToDollarsLoop", "centsToDollarsMap"]],
  },
  {
    id: "XF-P32",
    mode: "functions",
    category: "loop-form",
    description: "guarded push loop vs .filter",
    files: {
      "a.ts": `
export function activeIdsLoop(ids: number[]): number[] {
  const kept = [];
  for (const id of ids) {
    if (id > 0) {
      kept.push(id);
    }
  }
  return kept;
}

export function activeIdsFilter(ids: number[]): number[] {
  return ids.filter((id) => id > 0);
}
`,
    },
    expectPairs: [["activeIdsLoop", "activeIdsFilter"]],
  },
  {
    id: "XF-P33",
    mode: "functions",
    category: "loop-form",
    description: "forEach push vs map with renamed locals across files",
    files: {
      "a.ts": `
export function slugifyTitles(titles: string[]): string[] {
  const slugs = [];
  titles.forEach((title) => {
    slugs.push(title.toLowerCase().split(" ").join("-"));
  });
  return slugs;
}
`,
      "b.ts": `
export function makeUrlKeys(headings: string[]): string[] {
  return headings.map((heading) => heading.toLowerCase().split(" ").join("-"));
}
`,
    },
    expectPairs: [["slugifyTitles", "makeUrlKeys"]],
  },
  {
    id: "XF-P34",
    mode: "functions",
    category: "loop-form",
    description: "for(;cond;) vs while",
    files: {
      "a.ts": `
export function drainQueueFor(queue: string[]): number {
  let drained = 0;
  for (; queue.length > 0; ) {
    queue.pop();
    drained += 1;
  }
  return drained;
}

export function drainQueueWhile(queue: string[]): number {
  let drained = 0;
  while (queue.length > 0) {
    queue.pop();
    drained += 1;
  }
  return drained;
}
`,
    },
    expectPairs: [["drainQueueFor", "drainQueueWhile"]],
  },

  // -------------------------------------------------------------------
  // micro-idiom — temp-return, at(-1), callback body form, yoda
  // -------------------------------------------------------------------
  {
    id: "XF-P35",
    mode: "functions",
    category: "micro-idiom",
    description: "temp variable before return vs direct return",
    files: {
      "a.ts": `
export function taxedTotalTemp(net: number, rate: number): number {
  const gross = net * (1 + rate);
  const rounded = Math.round(gross * 100) / 100;
  return rounded;
}

export function taxedTotalDirect(net: number, rate: number): number {
  const gross = net * (1 + rate);
  return Math.round(gross * 100) / 100;
}
`,
    },
    expectPairs: [["taxedTotalTemp", "taxedTotalDirect"]],
  },
  {
    id: "XF-P36",
    mode: "functions",
    category: "micro-idiom",
    description: "length-1 indexing vs .at(-1)",
    files: {
      "a.ts": `
export function lastEventLegacy(events: string[]): string {
  const latest = events[events.length - 1];
  if (latest === undefined) {
    return "none";
  }
  return latest;
}

export function lastEventModern(events: string[]): string {
  const latest = events.at(-1);
  if (latest === undefined) {
    return "none";
  }
  return latest;
}
`,
    },
    expectPairs: [["lastEventLegacy", "lastEventModern"]],
  },
  {
    id: "XF-P37",
    mode: "functions",
    category: "micro-idiom",
    description: "callback expression body vs block body",
    files: {
      "a.ts": `
export function doubledExpressionBody(values: number[]): number[] {
  const doubled = values.map((value) => value * 2);
  const capped = doubled.filter((value) => value < 1000);
  return capped;
}

export function doubledBlockBody(values: number[]): number[] {
  const doubled = values.map((value) => {
    return value * 2;
  });
  const capped = doubled.filter((value) => {
    return value < 1000;
  });
  return capped;
}
`,
    },
    expectPairs: [["doubledExpressionBody", "doubledBlockBody"]],
  },
  {
    id: "XF-P38",
    mode: "functions",
    category: "micro-idiom",
    description: "yoda comparisons vs natural order",
    files: {
      "a.ts": `
export function classifyInputYoda(text: string): string {
  if (0 === text.length) {
    return "empty";
  }
  if ("/" === text[0]) {
    return "command";
  }
  return "message";
}

export function classifyInputNatural(text: string): string {
  if (text.length === 0) {
    return "empty";
  }
  if (text[0] === "/") {
    return "command";
  }
  return "message";
}
`,
    },
    expectPairs: [["classifyInputYoda", "classifyInputNatural"]],
  },

  // -------------------------------------------------------------------
  // combo — several transforms stacked plus full renaming
  // -------------------------------------------------------------------
  {
    id: "XF-P39",
    mode: "functions",
    category: "combo",
    description: "rename + guard style + template vs concat",
    files: {
      "a.ts": `
export function formatUserBadge(user: { name: string; score: number }): string {
  if (user.score <= 0) {
    return user.name;
  }
  const badge = \`\${user.name} (\${user.score}pt)\`;
  return badge;
}
`,
      "b.ts": `
export function renderMemberLabel(member: { name: string; score: number }): string {
  if (member.score <= 0) {
    return member.name;
  }
  return member.name + " (" + member.score + "pt)";
}
`,
    },
    expectPairs: [["formatUserBadge", "renderMemberLabel"]],
  },
  {
    id: "XF-P40",
    mode: "functions",
    category: "combo",
    description: "rename + filter loop + guard + temp return",
    files: {
      "a.ts": `
export function collectOverdueLabels(invoices: { id: string; overdue: boolean }[]): string {
  const labels = [];
  for (const invoice of invoices) {
    if (invoice.overdue) {
      labels.push(invoice);
    }
  }
  if (labels.length === 0) {
    return "clear";
  } else {
    const joined = labels.map((entry) => entry.id).join("; ");
    return joined;
  }
}
`,
      "b.ts": `
export function summarizeLateBills(bills: { id: string; overdue: boolean }[]): string {
  const late = bills.filter((bill) => bill.overdue);
  if (0 === late.length) {
    return "clear";
  }
  return late.map((item) => item.id).join("; ");
}
`,
    },
    expectPairs: [["collectOverdueLabels", "summarizeLateBills"]],
  },
  {
    id: "XF-P41",
    mode: "functions",
    category: "combo",
    description: "rename + destructure + nullish + negation swap",
    files: {
      "a.ts": `
export function resolveTimeoutMs(options: { timeoutMs?: number | null; label: string }): string {
  const { timeoutMs, label } = options;
  const effective = timeoutMs == null ? 3000 : timeoutMs;
  if (!(effective > 0)) {
    return label + ":disabled";
  } else {
    return label + ":" + effective;
  }
}
`,
      "b.ts": `
export function describeDeadline(config: { timeoutMs?: number | null; label: string }): string {
  const wait = config.timeoutMs ?? 3000;
  const tag = config.label;
  if (wait > 0) {
    return tag + ":" + wait;
  } else {
    return tag + ":disabled";
  }
}
`,
    },
    expectPairs: [["resolveTimeoutMs", "describeDeadline"]],
  },
  {
    id: "XF-P42",
    mode: "functions",
    category: "combo",
    description: "rename + index loop + compound assignment spellings + yoda",
    files: {
      "a.ts": `
export function averageLatency(samples: number[]): number {
  if (samples.length === 0) {
    return 0;
  }
  let total = 0;
  for (let i = 0; i < samples.length; i++) {
    const sample = samples[i];
    total = total + sample;
  }
  return total / samples.length;
}
`,
      "b.ts": `
export function meanResponseTime(readings: number[]): number {
  if (0 === readings.length) {
    return 0;
  }
  let sum = 0;
  for (const reading of readings) {
    sum += reading;
  }
  return sum / readings.length;
}
`,
    },
    expectPairs: [["averageLatency", "meanResponseTime"]],
  },
];
