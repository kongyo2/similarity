/**
 * Negative-only corpus for `functions` mode: near-twin lookalikes.
 *
 * Every case pairs functions that share length, overall shape, and
 * statement sequence yet differ in behavior — a flipped operator, literal,
 * callee, boundary, or evaluation contract. Reporting any `forbidPairs`
 * entry as a duplicate is a false positive that would pollute a
 * refactoring plan, so an analyzer must keep the pair apart even after
 * aggressive style canonicalization (rename-, formatting-, and
 * idiom-insensitive matching).
 *
 * Evaluated with the CLI defaults from the README invocation
 * (threshold 0.8, minLines 3) in `functions` mode.
 */

import type { BenchCase } from "../cases.js";

export const functionNegativeCases: BenchCase[] = [
  // -------------------------------------------------------------------
  // boundary-loose-strict — nullish/truthiness checks that are NOT
  // interchangeable even though they canonicalize to the same shape
  // -------------------------------------------------------------------
  {
    id: "XF-N01",
    mode: "functions",
    category: "boundary-loose-strict",
    description:
      "Loose == null also counts undefined entries; strict === null counts only explicit nulls",
    files: {
      "a.ts": `
export function countMissingPrices(prices: (number | null | undefined)[]): number {
  let missing = 0;
  for (const price of prices) {
    if (price == null) {
      missing += 1;
    }
  }
  return missing;
}

export function countClearedPrices(prices: (number | null | undefined)[]): number {
  let cleared = 0;
  for (const price of prices) {
    if (price === null) {
      cleared += 1;
    }
  }
  return cleared;
}
`,
    },
    forbidPairs: [["countMissingPrices", "countClearedPrices"]],
  },
  {
    id: "XF-N02",
    mode: "functions",
    category: "boundary-loose-strict",
    description:
      "Optional-chain access tolerates a missing section and falls back; plain access is a hard contract that throws on it",
    files: {
      "a.ts": `
export function cityOfOptionalStop(stop: { port?: { city: string } }): string {
  const city = stop.port?.city;
  const label = city ?? "unknown";
  return label.toUpperCase();
}

export function cityOfRequiredStop(stop: { port: { city: string } }): string {
  const city = stop.port.city;
  const label = city ?? "unknown";
  return label.toUpperCase();
}
`,
    },
    forbidPairs: [["cityOfOptionalStop", "cityOfRequiredStop"]],
  },
  {
    id: "XF-N03",
    mode: "functions",
    category: "boundary-loose-strict",
    description:
      "Patch semantics: === undefined treats only absent values as untouched, === null treats explicit clears — the two nullish inputs take opposite branches",
    files: {
      "a.ts": `
export function resolveDraftTitle(stored: string | null | undefined, fallback: string): string {
  if (stored === undefined) {
    return fallback;
  }
  return stored ?? "";
}

export function resolveFinalTitle(stored: string | null | undefined, fallback: string): string {
  if (stored === null) {
    return fallback;
  }
  return stored ?? "";
}
`,
    },
    forbidPairs: [["resolveDraftTitle", "resolveFinalTitle"]],
  },
  {
    id: "XF-N04",
    mode: "functions",
    category: "boundary-loose-strict",
    description:
      "Explicit !== undefined renders a configured 0; bare truthiness silently drops it",
    files: {
      "a.ts": `
export function describeRetryLimit(limit: number | undefined, label: string): string {
  if (limit !== undefined) {
    return label + ": " + limit;
  }
  return label;
}

export function describeRetryBudget(budget: number | undefined, label: string): string {
  if (budget) {
    return label + ": " + budget;
  }
  return label;
}
`,
    },
    forbidPairs: [["describeRetryLimit", "describeRetryBudget"]],
  },
  {
    id: "XF-N05",
    mode: "functions",
    category: "boundary-loose-strict",
    description:
      "?? keeps an intentional 0 (delay disabled); || swaps 0 for the default — different runtime behavior for falsy config values",
    files: {
      "a.ts": `
export function delayWithNullishDefault(configured: number | undefined): number {
  const delay = configured ?? 250;
  if (delay > 60000) {
    return 60000;
  }
  return delay;
}

export function delayWithFalsyDefault(configured: number | undefined): number {
  const delay = configured || 250;
  if (delay > 60000) {
    return 60000;
  }
  return delay;
}
`,
    },
    forbidPairs: [["delayWithNullishDefault", "delayWithFalsyDefault"]],
  },

  // -------------------------------------------------------------------
  // boundary-demorgan — negations that look like De Morgan rewrites but
  // distribute wrongly, so the predicates are not equivalent
  // -------------------------------------------------------------------
  {
    id: "XF-N06",
    mode: "functions",
    category: "boundary-demorgan",
    description:
      "!(a && b) fires when at least one check failed; !a && !b only when both failed — a De Morgan distribution that forgot to flip the operator",
    files: {
      "a.ts": `
export function needsAnyFix(check: { linted: boolean; typed: boolean }): boolean {
  if (!(check.linted && check.typed)) {
    return true;
  }
  return false;
}

export function needsFullFix(check: { linted: boolean; typed: boolean }): boolean {
  if (!check.linted && !check.typed) {
    return true;
  }
  return false;
}
`,
    },
    forbidPairs: [["needsAnyFix", "needsFullFix"]],
  },
  {
    id: "XF-N07",
    mode: "functions",
    category: "boundary-demorgan",
    description:
      "!(a || b) allows only when neither flag is set; !a || b leaves the second operand un-negated and allows whenever it is set",
    files: {
      "a.ts": `
export function canPauseQueue(queue: { draining: boolean; locked: boolean }): boolean {
  if (!(queue.draining || queue.locked)) {
    return true;
  }
  return false;
}

export function canFlushQueue(queue: { draining: boolean; locked: boolean }): boolean {
  if (!queue.draining || queue.locked) {
    return true;
  }
  return false;
}
`,
    },
    forbidPairs: [["canPauseQueue", "canFlushQueue"]],
  },
  {
    id: "XF-N08",
    mode: "functions",
    category: "boundary-demorgan",
    description:
      "Looks like a negate-condition-and-swap-branches rewrite, but !paid && !inStock is not the negation of paid && inStock, so mixed inputs land in different lanes",
    files: {
      "a.ts": `
export function laneForCheckout(order: { paid: boolean; inStock: boolean }): string {
  if (order.paid && order.inStock) {
    return "fulfill";
  } else {
    return "hold";
  }
}

export function laneForRestock(order: { paid: boolean; inStock: boolean }): string {
  if (!order.paid && !order.inStock) {
    return "hold";
  } else {
    return "fulfill";
  }
}
`,
    },
    forbidPairs: [["laneForCheckout", "laneForRestock"]],
  },

  // -------------------------------------------------------------------
  // boundary-loop — iteration idioms whose shapes canonicalize together
  // but whose visit set, order, or lifetime differs
  // -------------------------------------------------------------------
  {
    id: "XF-N09",
    mode: "functions",
    category: "boundary-loop",
    description:
      "Identical callback text, but map transforms every element while filter keeps the untrimmed originals and drops blank ones",
    files: {
      "a.ts": `
export function tidyLabels(labels: string[]): string[] {
  const result = labels.map((label) => label.trim());
  return result;
}

export function keepLabels(labels: string[]): string[] {
  const result = labels.filter((label) => label.trim());
  return result;
}
`,
    },
    forbidPairs: [["tidyLabels", "keepLabels"]],
  },
  {
    id: "XF-N10",
    mode: "functions",
    category: "boundary-loop",
    description:
      "map builds a fresh array per call; the forEach twin pushes into a module-level ledger and returns that shared array, which keeps growing across calls",
    files: {
      "a.ts": `
const skuLedger: string[] = [];

export function stampSkuBatch(skus: string[]): string[] {
  const stamped = skus.map((sku) => {
    return sku.toUpperCase();
  });
  return stamped;
}

export function recordSkuBatch(skus: string[]): string[] {
  skus.forEach((sku) => {
    skuLedger.push(sku.toUpperCase());
  });
  return skuLedger;
}
`,
    },
    forbidPairs: [["stampSkuBatch", "recordSkuBatch"]],
  },
  {
    id: "XF-N11",
    mode: "functions",
    category: "boundary-loop",
    description:
      "Exclusive i < n vs inclusive i <= n: the sums differ by the boundary term",
    files: {
      "a.ts": `
export function sumBelowLimit(limit: number): number {
  let total = 0;
  for (let step = 1; step < limit; step++) {
    total += step;
  }
  return total;
}

export function sumThroughLimit(limit: number): number {
  let total = 0;
  for (let step = 1; step <= limit; step++) {
    total += step;
  }
  return total;
}
`,
    },
    forbidPairs: [["sumBelowLimit", "sumThroughLimit"]],
  },
  {
    id: "XF-N12",
    mode: "functions",
    category: "boundary-loop",
    description:
      "for-of folds the tag values; for-in folds the index strings, producing 0,1,2 instead of the tags",
    files: {
      "a.ts": `
export function joinTagValues(tags: string[]): string {
  let csv = "";
  for (const tag of tags) {
    csv += tag + ",";
  }
  return csv;
}

export function joinTagIndices(tags: string[]): string {
  let csv = "";
  for (const tag in tags) {
    csv += tag + ",";
  }
  return csv;
}
`,
    },
    forbidPairs: [["joinTagValues", "joinTagIndices"]],
  },
  {
    id: "XF-N13",
    mode: "functions",
    category: "boundary-loop",
    description:
      "The break makes one function return the first failing step; without it the loop keeps overwriting and returns the last",
    files: {
      "a.ts": `
export function firstFailedStep(steps: { name: string; ok: boolean }[]): string {
  let picked = "";
  for (const step of steps) {
    if (!step.ok) {
      picked = step.name;
      break;
    }
  }
  return picked;
}

export function lastFailedStep(steps: { name: string; ok: boolean }[]): string {
  let picked = "";
  for (const step of steps) {
    if (!step.ok) {
      picked = step.name;
    }
  }
  return picked;
}
`,
    },
    forbidPairs: [["firstFailedStep", "lastFailedStep"]],
  },
  {
    id: "XF-N14",
    mode: "functions",
    category: "boundary-loop",
    description:
      "slice().reverse() flips the visit order, and order is observable in the folded trace string",
    files: {
      "a.ts": `
export function replayTraceForward(events: string[]): string {
  let trace = "";
  events.forEach((event) => {
    trace += event + ">";
  });
  return trace;
}

export function replayTraceBackward(events: string[]): string {
  let trace = "";
  events.slice().reverse().forEach((event) => {
    trace += event + ">";
  });
  return trace;
}
`,
    },
    forbidPairs: [["replayTraceForward", "replayTraceBackward"]],
  },

  // -------------------------------------------------------------------
  // boundary-index — same access shape, different element selected
  // -------------------------------------------------------------------
  {
    id: "XF-N15",
    mode: "functions",
    category: "boundary-index",
    description:
      "at(-1) picks the newest snapshot, at(0) the oldest — one literal apart",
    files: {
      "a.ts": `
export function newestSnapshot(snapshots: string[]): string {
  const picked = snapshots.at(-1);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}

export function oldestSnapshot(snapshots: string[]): string {
  const picked = snapshots.at(0);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}
`,
    },
    forbidPairs: [["newestSnapshot", "oldestSnapshot"]],
  },
  {
    id: "XF-N16",
    mode: "functions",
    category: "boundary-index",
    description:
      "length - 1 reads the current revision, length - 2 the previous one — adjacent but different elements",
    files: {
      "a.ts": `
export function currentRevision(revisions: number[]): number {
  if (revisions.length < 2) {
    return -1;
  }
  return revisions[revisions.length - 1];
}

export function previousRevision(revisions: number[]): number {
  if (revisions.length < 2) {
    return -1;
  }
  return revisions[revisions.length - 2];
}
`,
    },
    forbidPairs: [["currentRevision", "previousRevision"]],
  },
  {
    id: "XF-N17",
    mode: "functions",
    category: "boundary-index",
    description:
      "slice(0, n) takes the head of the queue, slice(n) drops it — complementary halves from the same call shape",
    files: {
      "a.ts": `
export function takeTopJobs(jobs: string[], cut: number): string[] {
  if (jobs.length === 0) {
    return [];
  }
  return jobs.slice(0, cut);
}

export function dropTopJobs(jobs: string[], cut: number): string[] {
  if (jobs.length === 0) {
    return [];
  }
  return jobs.slice(cut);
}
`,
    },
    forbidPairs: [["takeTopJobs", "dropTopJobs"]],
  },

  // -------------------------------------------------------------------
  // operator-twin — bodies identical except for one operator
  // -------------------------------------------------------------------
  {
    id: "XF-N18",
    mode: "functions",
    category: "operator-twin",
    description:
      "Ledger folds identical except + vs - buried in the loop body: credits grow the balance, debits shrink it",
    files: {
      "a.ts": `
export function applyCredits(entries: { amount: number }[], opening: number): number {
  let balance = opening;
  for (const entry of entries) {
    balance = balance + entry.amount;
  }
  return balance;
}

export function applyDebits(entries: { amount: number }[], opening: number): number {
  let balance = opening;
  for (const entry of entries) {
    balance = balance - entry.amount;
  }
  return balance;
}
`,
    },
    forbidPairs: [["applyCredits", "applyDebits"]],
  },
  {
    id: "XF-N19",
    mode: "functions",
    category: "operator-twin",
    description:
      "Same rounding pipeline, but * converts to minor units and / back to major units — inverse conversions",
    files: {
      "a.ts": `
export function toMinorUnits(prices: number[], factor: number): number[] {
  return prices.map((price) => {
    const scaled = price * factor;
    return Math.round(scaled);
  });
}

export function toMajorUnits(prices: number[], factor: number): number[] {
  return prices.map((price) => {
    const scaled = price / factor;
    return Math.round(scaled);
  });
}
`,
    },
    forbidPairs: [["toMinorUnits", "toMajorUnits"]],
  },
  {
    id: "XF-N20",
    mode: "functions",
    category: "operator-twin",
    description:
      "Money guard: >= lets a customer drain the balance to exactly zero, > blocks spending the final unit — a policy difference on the boundary",
    files: {
      "a.ts": `
export function approveDebitToZero(balance: number, amount: number): boolean {
  if (amount <= 0) {
    return false;
  }
  return balance >= amount;
}

export function approveDebitWithBuffer(balance: number, amount: number): boolean {
  if (amount <= 0) {
    return false;
  }
  return balance > amount;
}
`,
    },
    forbidPairs: [["approveDebitToZero", "approveDebitWithBuffer"]],
  },
  {
    id: "XF-N21",
    mode: "functions",
    category: "operator-twin",
    description:
      "Validity check where && requires both verifications and || accepts either — one token changes the admission policy",
    files: {
      "a.ts": `
export function passesStrictKyc(applicant: { idChecked: boolean; addressChecked: boolean; withdrawn: boolean }): boolean {
  if (applicant.withdrawn) {
    return false;
  }
  return applicant.idChecked && applicant.addressChecked;
}

export function passesBasicKyc(applicant: { idChecked: boolean; addressChecked: boolean; withdrawn: boolean }): boolean {
  if (applicant.withdrawn) {
    return false;
  }
  return applicant.idChecked || applicant.addressChecked;
}
`,
    },
    forbidPairs: [["passesStrictKyc", "passesBasicKyc"]],
  },
  {
    id: "XF-N22",
    mode: "functions",
    category: "operator-twin",
    description:
      "+= accumulates a sum, *= a product; only the compound operator and the seed differ",
    files: {
      "a.ts": `
export function sumGrowthFactors(factors: number[]): number {
  let acc = 0;
  for (const factor of factors) {
    acc += factor;
  }
  return acc;
}

export function compoundGrowthFactors(factors: number[]): number {
  let acc = 1;
  for (const factor of factors) {
    acc *= factor;
  }
  return acc;
}
`,
    },
    forbidPairs: [["sumGrowthFactors", "compoundGrowthFactors"]],
  },
  {
    id: "XF-N23",
    mode: "functions",
    category: "operator-twin",
    description:
      "Math.max tracks the worst ping, Math.min the best — identical reduction skeleton around opposite callees",
    files: {
      "a.ts": `
export function highestPing(samples: number[]): number {
  let bound = samples[0] ?? 0;
  for (const sample of samples) {
    bound = Math.max(bound, sample);
  }
  return bound;
}

export function lowestPing(samples: number[]): number {
  let bound = samples[0] ?? 0;
  for (const sample of samples) {
    bound = Math.min(bound, sample);
  }
  return bound;
}
`,
    },
    forbidPairs: [["highestPing", "lowestPing"]],
  },

  // -------------------------------------------------------------------
  // free-callee — identical wrappers around different free functions;
  // the call target IS the behavior
  // -------------------------------------------------------------------
  {
    id: "XF-N24",
    mode: "functions",
    category: "free-callee",
    description:
      "Fan-out loops identical except for the delivery channel invoked: email, SMS, and push are different side effects",
    files: {
      "a.ts": `
declare function deliverEmail(address: string, message: string): void;
declare function deliverSms(address: string, message: string): void;
declare function deliverPush(address: string, message: string): void;

export function alertByEmail(recipients: string[], message: string): number {
  let sent = 0;
  for (const recipient of recipients) {
    deliverEmail(recipient, message);
    sent += 1;
  }
  return sent;
}

export function alertBySms(recipients: string[], message: string): number {
  let sent = 0;
  for (const recipient of recipients) {
    deliverSms(recipient, message);
    sent += 1;
  }
  return sent;
}

export function alertByPush(recipients: string[], message: string): number {
  let sent = 0;
  for (const recipient of recipients) {
    deliverPush(recipient, message);
    sent += 1;
  }
  return sent;
}
`,
    },
    forbidPairs: [
      ["alertByEmail", "alertBySms"],
      ["alertByEmail", "alertByPush"],
      ["alertBySms", "alertByPush"],
    ],
  },
  {
    id: "XF-N25",
    mode: "functions",
    category: "free-callee",
    description:
      "Chunk pipelines identical except one encrypts and the other decrypts — inverse operations behind the same shape",
    files: {
      "a.ts": `
declare function encryptChunk(chunk: string, key: string): string;
declare function decryptChunk(chunk: string, key: string): string;

export function lockPayload(chunks: string[], key: string): string[] {
  const output: string[] = [];
  for (const chunk of chunks) {
    output.push(encryptChunk(chunk, key));
  }
  return output;
}

export function unlockPayload(chunks: string[], key: string): string[] {
  const output: string[] = [];
  for (const chunk of chunks) {
    output.push(decryptChunk(chunk, key));
  }
  return output;
}
`,
    },
    forbidPairs: [["lockPayload", "unlockPayload"]],
  },
  {
    id: "XF-N26",
    mode: "functions",
    category: "free-callee",
    description:
      "Same map-and-return wrapper around serialize vs deserialize — opposite directions of the same codec",
    files: {
      "a.ts": `
declare function serializeRecord(record: { id: number }): string;
declare function deserializeRecord(raw: string): { id: number };

export function packRecords(records: { id: number }[]): string[] {
  return records.map((record) => {
    const packed = serializeRecord(record);
    return packed;
  });
}

export function unpackRecords(rows: string[]): { id: number }[] {
  return rows.map((row) => {
    const parsed = deserializeRecord(row);
    return parsed;
  });
}
`,
    },
    forbidPairs: [["packRecords", "unpackRecords"]],
  },
  {
    id: "XF-N27",
    mode: "functions",
    category: "free-callee",
    description:
      "Bulk helpers identical except one opens channels and the other closes them — running the wrong one tears down live connections",
    files: {
      "a.ts": `
declare function openChannel(name: string): boolean;
declare function closeChannel(name: string): boolean;

export function openAllChannels(names: string[]): number {
  let touched = 0;
  for (const name of names) {
    if (openChannel(name)) {
      touched += 1;
    }
  }
  return touched;
}

export function closeAllChannels(names: string[]): number {
  let touched = 0;
  for (const name of names) {
    if (closeChannel(name)) {
      touched += 1;
    }
  }
  return touched;
}
`,
    },
    forbidPairs: [["openAllChannels", "closeAllChannels"]],
  },
  {
    id: "XF-N28",
    mode: "functions",
    category: "free-callee",
    description:
      "validate passes fields through (or rejects) while sanitize rewrites them — same collection loop, different contract with the data",
    files: {
      "a.ts": `
declare function validateField(raw: string): string;
declare function sanitizeField(raw: string): string;

export function checkSubmission(fields: string[]): string[] {
  const accepted: string[] = [];
  for (const field of fields) {
    accepted.push(validateField(field));
  }
  return accepted;
}

export function scrubSubmission(fields: string[]): string[] {
  const accepted: string[] = [];
  for (const field of fields) {
    accepted.push(sanitizeField(field));
  }
  return accepted;
}
`,
    },
    forbidPairs: [["checkSubmission", "scrubSubmission"]],
  },

  // -------------------------------------------------------------------
  // literal-twin — same shape, one critical literal apart
  // -------------------------------------------------------------------
  {
    id: "XF-N29",
    mode: "functions",
    category: "literal-twin",
    description:
      "Identical request builders differing only in the method string literal — data-only variation, parameterizable (F-P02 precedent)",
    files: {
      "a.ts": `
type Transport = (url: string, options: { method: string }) => Promise<string>;

export function readUserRecord(send: Transport, id: string): Promise<string> {
  const url = "/api/users/" + id;
  return send(url, { method: "GET" });
}

export function purgeUserRecord(send: Transport, id: string): Promise<string> {
  const url = "/api/users/" + id;
  return send(url, { method: "DELETE" });
}
`,
    },
    expectPairs: [["readUserRecord", "purgeUserRecord"]],
  },
  {
    id: "XF-N30",
    mode: "functions",
    category: "literal-twin",
    description:
      "Status tallies identical except the compared code: 200 successes, 500 crashes, 404 misses",
    files: {
      "a.ts": `
export function countOkResponses(statuses: number[]): number {
  let seen = 0;
  for (const status of statuses) {
    if (status === 200) {
      seen += 1;
    }
  }
  return seen;
}

export function countCrashResponses(statuses: number[]): number {
  let seen = 0;
  for (const status of statuses) {
    if (status === 500) {
      seen += 1;
    }
  }
  return seen;
}

export function countMissingResponses(statuses: number[]): number {
  let seen = 0;
  for (const status of statuses) {
    if (status === 404) {
      seen += 1;
    }
  }
  return seen;
}
`,
    },
    expectPairs: [
      ["countOkResponses", "countCrashResponses"],
      ["countOkResponses", "countMissingResponses"],
      ["countCrashResponses", "countMissingResponses"],
    ],
  },
  {
    id: "XF-N31",
    mode: "functions",
    category: "literal-twin",
    description:
      "Same guard-then-test shape, but the regex accepts digits only vs any word characters — different input languages",
    files: {
      "a.ts": `
export function isNumericHandle(handle: string): boolean {
  if (handle.length === 0) {
    return false;
  }
  return /^\\d+$/.test(handle);
}

export function isWordHandle(handle: string): boolean {
  if (handle.length === 0) {
    return false;
  }
  return /^\\w+$/.test(handle);
}
`,
    },
    forbidPairs: [["isNumericHandle", "isWordHandle"]],
  },
  {
    id: "XF-N32",
    mode: "functions",
    category: "literal-twin",
    description:
      "Role scans identical except the permission string: viewer, editor, and admin gate very different capabilities",
    files: {
      "a.ts": `
export function allowsViewing(roles: string[]): boolean {
  for (const role of roles) {
    if (role === "viewer") {
      return true;
    }
  }
  return false;
}

export function allowsEditing(roles: string[]): boolean {
  for (const role of roles) {
    if (role === "editor") {
      return true;
    }
  }
  return false;
}

export function allowsAdministration(roles: string[]): boolean {
  for (const role of roles) {
    if (role === "admin") {
      return true;
    }
  }
  return false;
}
`,
    },
    expectPairs: [
      ["allowsViewing", "allowsEditing"],
      ["allowsViewing", "allowsAdministration"],
      ["allowsEditing", "allowsAdministration"],
    ],
  },

  // -------------------------------------------------------------------
  // contract — same statements, different caller-visible contract
  // -------------------------------------------------------------------
  {
    id: "XF-N33",
    mode: "functions",
    category: "contract",
    description:
      "Sync fold returns a number immediately; the async twin awaits each charge sequentially and returns a promise — different caller contract and timing",
    files: {
      "a.ts": `
export function settleOrdersNow(amounts: number[], charge: (amount: number) => number): number {
  let settled = 0;
  for (const amount of amounts) {
    settled += charge(amount);
  }
  return settled;
}

export async function settleOrdersQueued(amounts: number[], charge: (amount: number) => Promise<number>): Promise<number> {
  let settled = 0;
  for (const amount of amounts) {
    settled += await charge(amount);
  }
  return settled;
}
`,
    },
    forbidPairs: [["settleOrdersNow", "settleOrdersQueued"]],
  },
  {
    id: "XF-N34",
    mode: "functions",
    category: "contract",
    description:
      "Generator yields lazily into a single-pass iterator; the twin eagerly materializes a reusable array — same loop, different production contract",
    files: {
      "a.ts": `
export function* streamEvenNumbers(limit: number): Generator<number> {
  for (let value = 0; value < limit; value += 2) {
    yield value;
  }
}

export function listEvenNumbers(limit: number): number[] {
  const values: number[] = [];
  for (let value = 0; value < limit; value += 2) {
    values.push(value);
  }
  return values;
}
`,
    },
    forbidPairs: [["streamEvenNumbers", "listEvenNumbers"]],
  },
  {
    id: "XF-N35",
    mode: "functions",
    category: "contract",
    description:
      "sort() reorders the caller's array in place; slice().sort() works on a copy — aliasing callers observe completely different effects",
    files: {
      "a.ts": `
export function rankScoresInPlace(scores: number[]): number[] {
  const ranked = scores.sort((a, b) => b - a);
  return ranked;
}

export function rankScoresCopy(scores: number[]): number[] {
  const ranked = scores.slice().sort((a, b) => b - a);
  return ranked;
}
`,
    },
    forbidPairs: [["rankScoresInPlace", "rankScoresCopy"]],
  },
  {
    id: "XF-N36",
    mode: "functions",
    category: "contract",
    description:
      "Same two statements, swapped: (base + fee) * rate taxes the fee, base * rate + fee does not — order is the semantics",
    files: {
      "a.ts": `
export function quoteFeeThenTax(base: number, fee: number, rate: number): number {
  let total = base;
  total += fee;
  total *= rate;
  return total;
}

export function quoteTaxThenFee(base: number, fee: number, rate: number): number {
  let total = base;
  total *= rate;
  total += fee;
  return total;
}
`,
    },
    forbidPairs: [["quoteFeeThenTax", "quoteTaxThenFee"]],
  },

  // -------------------------------------------------------------------
  // then-shape — promise/object idioms that naive canonicalization maps
  // onto each other despite different semantics
  // -------------------------------------------------------------------
  {
    id: "XF-N37",
    mode: "functions",
    category: "then-shape",
    description:
      "Statement-position .then is fire-and-forget: the callback's return exits only the callback and the caller gets void; the await form suspends the caller and returns the count",
    files: {
      "a.ts": `
declare function cacheBadgeCount(count: number): void;

export function syncBadgeQuiet(pull: () => Promise<number>): void {
  pull().then((count) => {
    if (count < 0) return;
    cacheBadgeCount(count);
  });
}

export async function syncBadgeTracked(pull: () => Promise<number>): Promise<number> {
  const count = await pull();
  if (count < 0) return count;
  cacheBadgeCount(count);
  return count;
}
`,
    },
    forbidPairs: [["syncBadgeQuiet", "syncBadgeTracked"]],
  },
  {
    id: "XF-N38",
    mode: "functions",
    category: "then-shape",
    description:
      ".then(cb).catch(handler) also recovers when JSON.parse throws inside cb; the two-argument then(cb, handler) lets that same throw reject — different failure surface",
    files: {
      "a.ts": `
export function parseTitleChained(fetchRaw: () => Promise<string>, warn: (reason: string) => void): Promise<string> {
  return fetchRaw()
    .then((raw) => JSON.parse(raw).title as string)
    .catch((error) => {
      warn(String(error));
      return "untitled";
    });
}

export function parseTitleForked(fetchRaw: () => Promise<string>, warn: (reason: string) => void): Promise<string> {
  return fetchRaw()
    .then((raw) => JSON.parse(raw).title as string, (error) => {
      warn(String(error));
      return "untitled";
    });
}
`,
    },
    forbidPairs: [["parseTitleChained", "parseTitleForked"]],
  },
  {
    id: "XF-N39",
    mode: "functions",
    category: "then-shape",
    description:
      "Object.assign writes the patch into the caller's live object; the spread twin returns a fresh copy and leaves the original untouched",
    files: {
      "a.ts": `
export function mergeQuotaPatch(current: Record<string, number>, patch: Record<string, number>): Record<string, number> {
  if (Object.keys(patch).length === 0) {
    return current;
  }
  const next = Object.assign(current, patch);
  return next;
}

export function previewQuotaPatch(current: Record<string, number>, patch: Record<string, number>): Record<string, number> {
  if (Object.keys(patch).length === 0) {
    return current;
  }
  const next = { ...current, ...patch };
  return next;
}
`,
    },
    forbidPairs: [["mergeQuotaPatch", "previewQuotaPatch"]],
  },
  {
    id: "XF-N40",
    mode: "functions",
    category: "then-shape",
    description:
      "String folds with swapped concat operands: appending builds a/b/c/ while prepending builds c/b/a/ — string + is not commutative",
    files: {
      "a.ts": `
export function buildTrailForward(segments: string[]): string {
  let trail = "";
  for (const segment of segments) {
    trail = trail + segment + "/";
  }
  return trail;
}

export function buildTrailReversed(segments: string[]): string {
  let trail = "";
  for (const segment of segments) {
    trail = segment + "/" + trail;
  }
  return trail;
}
`,
    },
    forbidPairs: [["buildTrailForward", "buildTrailReversed"]],
  },
];
