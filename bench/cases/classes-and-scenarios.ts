/**
 * Extended labeled corpus: class-mode equivalence cases plus realistic
 * copy-paste "scenario" cases for functions mode.
 *
 * Part 1 (XC-*) targets `--mode classes`: accepted equivalences a class
 * comparator must see through (member reordering, member/local renames,
 * constructor parameter-property shorthand, method style spellings) and
 * contract differences it must not collapse (static vs instance, accessor
 * vs method, heritage / super calls, empty marker classes).
 *
 * Part 2 (XS-*) targets `--mode functions` and models what an AI assistant
 * meets when it scans a real repo: copy-paste-then-rename duplicates spread
 * across two files (repositories, validators, mappers) that a refactoring
 * plan must catch, and the classic false-positive traps (twins that differ
 * only in a SQL table / endpoint / locale literal, create vs update
 * semantics, paginated vs unpaginated fetchers).
 *
 * Positives are behavior-identical: every string/number literal is the same
 * in both pair members; only identifiers and accepted style spellings vary.
 * Negatives are genuinely different in behavior or contract.
 *
 * Evaluated with the CLI defaults from the README invocation
 * (threshold 0.8, minLines 3) in the mode each case targets.
 */

import type { BenchCase } from "../cases.js";

export const classAndScenarioCases: BenchCase[] = [
  // -------------------------------------------------------------------
  // Part 1: classes — positives
  // -------------------------------------------------------------------
  {
    id: "XC-P01",
    mode: "classes",
    category: "member-order",
    description: "Identical ledger classes with methods and properties reordered",
    files: {
      "a.ts": `
export class ShipmentLedger {
  private entries: string[] = [];
  private sealed = false;
  append(entry: string): void {
    if (this.sealed) throw new Error("ledger sealed");
    this.entries.push(entry);
  }
  seal(): void {
    this.sealed = true;
  }
  snapshot(): string[] {
    return [...this.entries];
  }
}
`,
      "b.ts": `
export class FreightLedger {
  snapshot(): string[] {
    return [...this.entries];
  }
  private sealed = false;
  seal(): void {
    this.sealed = true;
  }
  append(entry: string): void {
    if (this.sealed) throw new Error("ledger sealed");
    this.entries.push(entry);
  }
  private entries: string[] = [];
}
`,
    },
    expectPairs: [["ShipmentLedger", "FreightLedger"]],
  },
  {
    id: "XC-P02",
    mode: "classes",
    category: "member-order",
    description: "Identical policy stores with fields shuffled in between methods",
    files: {
      "a.ts": `
export class RetryPolicyStore {
  private limits = new Map<string, number>();
  private fallback = 3;
  setLimit(op: string, limit: number): void {
    this.limits.set(op, limit);
  }
  limitFor(op: string): number {
    return this.limits.get(op) ?? this.fallback;
  }
  clearAll(): void {
    this.limits.clear();
  }
}
`,
      "b.ts": `
export class BackoffPolicyStore {
  clearAll(): void {
    this.limits.clear();
  }
  limitFor(op: string): number {
    return this.limits.get(op) ?? this.fallback;
  }
  private fallback = 3;
  private limits = new Map<string, number>();
  setLimit(op: string, limit: number): void {
    this.limits.set(op, limit);
  }
}
`,
    },
    expectPairs: [["RetryPolicyStore", "BackoffPolicyStore"]],
  },
  {
    id: "XC-P03",
    mode: "classes",
    category: "rename-members",
    description: "Private fields and method-local variables renamed, bodies structurally identical",
    files: {
      "a.ts": `
export class PayoutAggregator {
  private ledger = new Map<string, number>();
  record(merchant: string, amount: number): void {
    const current = this.ledger.get(merchant) ?? 0;
    this.ledger.set(merchant, current + amount);
  }
  totalFor(merchant: string): number {
    const stored = this.ledger.get(merchant);
    return stored ?? 0;
  }
}
`,
      "b.ts": `
export class RefundAggregator {
  private balances = new Map<string, number>();
  record(vendor: string, sum: number): void {
    const existing = this.balances.get(vendor) ?? 0;
    this.balances.set(vendor, existing + sum);
  }
  totalFor(vendor: string): number {
    const found = this.balances.get(vendor);
    return found ?? 0;
  }
}
`,
    },
    expectPairs: [["PayoutAggregator", "RefundAggregator"]],
  },
  {
    id: "XC-P04",
    mode: "classes",
    category: "rename-members",
    description: "Method names renamed too, but signatures and bodies match",
    files: {
      "a.ts": `
export class SampleWindow {
  private values: number[] = [];
  addSample(value: number): void {
    this.values.push(value);
    if (this.values.length > 50) this.values.shift();
  }
  averageOf(): number {
    if (this.values.length === 0) return 0;
    return this.values.reduce((acc, v) => acc + v, 0) / this.values.length;
  }
}
`,
      "b.ts": `
export class ReadingWindow {
  private points: number[] = [];
  recordPoint(point: number): void {
    this.points.push(point);
    if (this.points.length > 50) this.points.shift();
  }
  meanOf(): number {
    if (this.points.length === 0) return 0;
    return this.points.reduce((acc, p) => acc + p, 0) / this.points.length;
  }
}
`,
    },
    expectPairs: [["SampleWindow", "ReadingWindow"]],
  },
  {
    id: "XC-P05",
    mode: "classes",
    category: "rename-members",
    description: "Only the class name renamed; every member is byte-identical",
    files: {
      "a.ts": `
export class CursorPager {
  private cursor: string | null = null;
  advance(next: string | null): void {
    this.cursor = next;
  }
  hasMore(): boolean {
    return this.cursor !== null;
  }
  current(): string | null {
    return this.cursor;
  }
}
`,
      "b.ts": `
export class TokenPager {
  private cursor: string | null = null;
  advance(next: string | null): void {
    this.cursor = next;
  }
  hasMore(): boolean {
    return this.cursor !== null;
  }
  current(): string | null {
    return this.cursor;
  }
}
`,
    },
    expectPairs: [["CursorPager", "TokenPager"]],
  },
  {
    id: "XC-P06",
    mode: "classes",
    category: "ctor-style",
    description: "Constructor parameter-property shorthand vs explicit field plus assignment",
    files: {
      "a.ts": `
interface InvoiceRepo {
  fetchInvoice(id: string): Promise<{ id: string; total: number } | null>;
}

export class InvoiceLookup {
  constructor(private readonly repo: InvoiceRepo) {}
  async totalFor(id: string): Promise<number> {
    const found = await this.repo.fetchInvoice(id);
    return found ? found.total : 0;
  }
}
`,
      "b.ts": `
interface ReceiptRepo {
  fetchInvoice(id: string): Promise<{ id: string; total: number } | null>;
}

export class ReceiptLookup {
  private readonly repo: ReceiptRepo;
  constructor(repo: ReceiptRepo) {
    this.repo = repo;
  }
  async totalFor(id: string): Promise<number> {
    const found = await this.repo.fetchInvoice(id);
    return found ? found.total : 0;
  }
}
`,
    },
    expectPairs: [["InvoiceLookup", "ReceiptLookup"]],
  },
  {
    id: "XC-P07",
    mode: "classes",
    category: "ctor-style",
    description: "Two parameter-properties vs two explicit fields assigned in the constructor",
    files: {
      "a.ts": `
interface AlertChannel {
  post(topic: string, body: string): Promise<void>;
}
interface AlertAudit {
  note(entry: string): void;
}

export class AlertDispatcher {
  constructor(
    private readonly channel: AlertChannel,
    private readonly audit: AlertAudit,
  ) {}
  async raise(topic: string, body: string): Promise<void> {
    await this.channel.post(topic, body);
    this.audit.note(topic + ":" + body);
  }
}
`,
      "b.ts": `
interface NoticeChannel {
  post(topic: string, body: string): Promise<void>;
}
interface NoticeAudit {
  note(entry: string): void;
}

export class NoticeDispatcher {
  private readonly channel: NoticeChannel;
  private readonly audit: NoticeAudit;
  constructor(channel: NoticeChannel, audit: NoticeAudit) {
    this.channel = channel;
    this.audit = audit;
  }
  async raise(topic: string, body: string): Promise<void> {
    await this.channel.post(topic, body);
    this.audit.note(topic + ":" + body);
  }
}
`,
    },
    expectPairs: [["AlertDispatcher", "NoticeDispatcher"]],
  },
  {
    id: "XC-P08",
    mode: "classes",
    category: "method-style",
    description: "Template-literal methods vs string-concatenation methods, same output",
    files: {
      "a.ts": `
export class CouponBanner {
  constructor(private code: string, private percent: number) {}
  headline(): string {
    return \`Use \${this.code} for \${this.percent}% off\`;
  }
  footnote(): string {
    return \`Code \${this.code} expires soon\`;
  }
}
`,
      "b.ts": `
export class PromoBanner {
  constructor(private code: string, private percent: number) {}
  headline(): string {
    return "Use " + this.code + " for " + this.percent + "% off";
  }
  footnote(): string {
    return "Code " + this.code + " expires soon";
  }
}
`,
    },
    expectPairs: [["CouponBanner", "PromoBanner"]],
  },
  {
    id: "XC-P09",
    mode: "classes",
    category: "method-style",
    description: "for-of plus if/else vs forEach plus ternary-return, same behavior",
    files: {
      "a.ts": `
export class GradeBook {
  private marks: number[] = [];
  addAll(marks: number[]): void {
    for (const mark of marks) {
      this.marks.push(mark);
    }
  }
  passed(cutoff: number): boolean {
    if (this.marks.length === 0) {
      return false;
    } else {
      return this.marks.every((m) => m >= cutoff);
    }
  }
}
`,
      "b.ts": `
export class ScoreBook {
  private marks: number[] = [];
  addAll(marks: number[]): void {
    marks.forEach((mark) => {
      this.marks.push(mark);
    });
  }
  passed(cutoff: number): boolean {
    return this.marks.length === 0 ? false : this.marks.every((m) => m >= cutoff);
  }
}
`,
    },
    expectPairs: [["GradeBook", "ScoreBook"]],
  },
  {
    id: "XC-P10",
    mode: "classes",
    category: "method-style",
    description: "Arrow-function class fields vs regular methods with identical bodies",
    files: {
      "a.ts": `
export class ClickRelay {
  constructor(private sink: (event: string) => void) {}
  handle = (event: string): void => {
    if (!event) return;
    this.sink(event.trim());
  };
  reset = (): void => {
    this.sink("reset");
  };
}
`,
      "b.ts": `
export class TapRelay {
  constructor(private sink: (event: string) => void) {}
  handle(event: string): void {
    if (!event) return;
    this.sink(event.trim());
  }
  reset(): void {
    this.sink("reset");
  }
}
`,
    },
    expectPairs: [["ClickRelay", "TapRelay"]],
  },

  // -------------------------------------------------------------------
  // Part 1: classes — negatives
  // -------------------------------------------------------------------
  {
    id: "XC-N01",
    mode: "classes",
    category: "divergent-bodies",
    description: "Same method names, different operators and fields (discount vs surcharge)",
    files: {
      "a.ts": `
export class DiscountEngine {
  constructor(private rate: number) {}
  apply(total: number): number {
    return total - total * this.rate;
  }
  describe(): string {
    return "discount at " + this.rate;
  }
}
`,
      "b.ts": `
export class SurchargeEngine {
  constructor(private flat: number) {}
  apply(total: number): number {
    return total + this.flat;
  }
  describe(): string {
    return "surcharge of " + this.flat;
  }
}
`,
    },
    forbidPairs: [["DiscountEngine", "SurchargeEngine"]],
  },
  {
    id: "XC-N02",
    mode: "classes",
    category: "divergent-bodies",
    description: "Same method names (start/stop), genuinely different calls and state",
    files: {
      "a.ts": `
export class IntervalBeacon {
  private handle: ReturnType<typeof setInterval> | null = null;
  start(tick: () => void): void {
    this.handle = setInterval(tick, 1000);
  }
  stop(): void {
    if (this.handle) clearInterval(this.handle);
    this.handle = null;
  }
}
`,
      "b.ts": `
export class StopwatchBeacon {
  private startedAt = 0;
  start(tick: () => void): void {
    this.startedAt = Date.now();
    tick();
  }
  stop(): number {
    return Date.now() - this.startedAt;
  }
}
`,
    },
    forbidPairs: [["IntervalBeacon", "StopwatchBeacon"]],
  },
  {
    id: "XC-N03",
    mode: "classes",
    category: "static-contract",
    description: "Static methods vs instance methods with the same names and bodies",
    files: {
      "a.ts": `
export class PlanarGeometry {
  static distance(x1: number, y1: number, x2: number, y2: number): number {
    return Math.sqrt((x2 - x1) ** 2 + (y2 - y1) ** 2);
  }
  static midpointX(x1: number, x2: number): number {
    return (x1 + x2) / 2;
  }
}
`,
      "b.ts": `
export class SegmentGeometry {
  distance(x1: number, y1: number, x2: number, y2: number): number {
    return Math.sqrt((x2 - x1) ** 2 + (y2 - y1) ** 2);
  }
  midpointX(x1: number, x2: number): number {
    return (x1 + x2) / 2;
  }
}
`,
    },
    forbidPairs: [["PlanarGeometry", "SegmentGeometry"]],
  },
  {
    id: "XC-N04",
    mode: "classes",
    category: "accessor-contract",
    description: "Getter/setter accessors vs plain getX/setX methods: different access contract",
    files: {
      "a.ts": `
export class DimmerControl {
  private stored = 0;
  get level(): number {
    return this.stored;
  }
  set level(next: number) {
    this.stored = Math.min(100, Math.max(0, next));
  }
}
`,
      "b.ts": `
export class BrightnessControl {
  private stored = 0;
  getLevel(): number {
    return this.stored;
  }
  setLevel(next: number): void {
    this.stored = Math.min(100, Math.max(0, next));
  }
}
`,
    },
    forbidPairs: [["DimmerControl", "BrightnessControl"]],
  },
  {
    id: "XC-N05",
    mode: "classes",
    category: "heritage",
    description: "Same shape, but one class extends a base and delegates via super.flushAll()",
    files: {
      "a.ts": `
export class AuditSinkBase {
  protected records: string[] = [];
  flushAll(): void {
    this.records = [];
  }
}

export class DatabaseAuditSink extends AuditSinkBase {
  capture(entry: string): void {
    this.records.push(entry);
  }
  flushAll(): void {
    super.flushAll();
  }
}
`,
      "b.ts": `
export class ConsoleAuditSink {
  protected records: string[] = [];
  capture(entry: string): void {
    this.records.push(entry);
  }
  flushAll(): void {
    this.records = [];
  }
}
`,
    },
    forbidPairs: [["DatabaseAuditSink", "ConsoleAuditSink"]],
  },
  {
    id: "XC-N06",
    mode: "classes",
    category: "marker-heritage",
    description: "Empty marker error classes with different heritage chains",
    files: {
      "a.ts": `
export class GatewayHttpError extends Error {
  constructor(public readonly status: number, message: string) {
    super(message);
    this.name = "GatewayHttpError";
  }
}

export class MissingResourceError extends GatewayHttpError {}
`,
      "b.ts": `
export class BrokerTransportFault extends Error {
  constructor(public readonly retryAfterMs: number, message: string) {
    super(message);
    this.name = "BrokerTransportFault";
  }
}

export class ThrottleExceededError extends BrokerTransportFault {}
`,
    },
    forbidPairs: [["MissingResourceError", "ThrottleExceededError"]],
  },

  // -------------------------------------------------------------------
  // Part 2: functions scenarios — positives
  // (copy-paste-then-rename duplicates across two files)
  // -------------------------------------------------------------------
  {
    id: "XS-P01",
    mode: "functions",
    category: "crud-rename",
    description: "INSERT helper copy-pasted for a second entity; table is a parameter, only identifiers renamed",
    files: {
      "user-repository.ts": `
interface UserDbClient {
  query(sql: string, params: unknown[]): Promise<{ rows: Record<string, unknown>[] }>;
}

export async function insertUserRow(
  db: UserDbClient,
  table: string,
  data: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const columns = Object.keys(data);
  const placeholders = columns.map((_, index) => "$" + (index + 1));
  const sql =
    "INSERT INTO " + table + " (" + columns.join(", ") + ") VALUES (" + placeholders.join(", ") + ") RETURNING *";
  const result = await db.query(sql, Object.values(data));
  return result.rows[0];
}
`,
      "product-repository.ts": `
interface ProductDbClient {
  query(sql: string, params: unknown[]): Promise<{ rows: Record<string, unknown>[] }>;
}

export async function insertProductRow(
  conn: ProductDbClient,
  relation: string,
  values: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const fields = Object.keys(values);
  const slots = fields.map((_, position) => "$" + (position + 1));
  const statement =
    "INSERT INTO " + relation + " (" + fields.join(", ") + ") VALUES (" + slots.join(", ") + ") RETURNING *";
  const outcome = await conn.query(statement, Object.values(values));
  return outcome.rows[0];
}
`,
    },
    expectPairs: [["insertUserRow", "insertProductRow"]],
  },
  {
    id: "XS-P02",
    mode: "functions",
    category: "crud-rename",
    description: "findById helper duplicated; collection is a parameter, identical literals, renamed identifiers",
    files: {
      "user-store.ts": `
interface UserCollectionDriver {
  findOne(collection: string, filter: Record<string, unknown>): Promise<Record<string, unknown> | null>;
}

export async function findUserById(
  driver: UserCollectionDriver,
  collection: string,
  id: string,
): Promise<Record<string, unknown>> {
  if (!id) {
    throw new Error("id is required");
  }
  const found = await driver.findOne(collection, { _id: id });
  if (found === null) {
    throw new Error("record not found: " + id);
  }
  return found;
}
`,
      "product-store.ts": `
interface ProductCollectionDriver {
  findOne(collection: string, filter: Record<string, unknown>): Promise<Record<string, unknown> | null>;
}

export async function findProductById(
  adapter: ProductCollectionDriver,
  bucket: string,
  key: string,
): Promise<Record<string, unknown>> {
  if (!key) {
    throw new Error("id is required");
  }
  const match = await adapter.findOne(bucket, { _id: key });
  if (match === null) {
    throw new Error("record not found: " + key);
  }
  return match;
}
`,
    },
    expectPairs: [["findUserById", "findProductById"]],
  },
  {
    id: "XS-P03",
    mode: "functions",
    category: "crud-rename",
    description: "Dynamic UPDATE builder duplicated across entities with renamed locals only",
    files: {
      "user-mutations.ts": `
interface UserSqlRunner {
  execute(sql: string, params: unknown[]): Promise<number>;
}

export async function updateUserFields(
  runner: UserSqlRunner,
  table: string,
  id: string,
  patch: Record<string, unknown>,
): Promise<number> {
  const keys = Object.keys(patch);
  if (keys.length === 0) return 0;
  const assignments = keys.map((key, index) => key + " = $" + (index + 2));
  const sql = "UPDATE " + table + " SET " + assignments.join(", ") + " WHERE id = $1";
  return runner.execute(sql, [id, ...Object.values(patch)]);
}
`,
      "product-mutations.ts": `
interface ProductSqlRunner {
  execute(sql: string, params: unknown[]): Promise<number>;
}

export async function updateProductFields(
  executor: ProductSqlRunner,
  relation: string,
  recordId: string,
  changes: Record<string, unknown>,
): Promise<number> {
  const fields = Object.keys(changes);
  if (fields.length === 0) return 0;
  const setters = fields.map((field, position) => field + " = $" + (position + 2));
  const statement = "UPDATE " + relation + " SET " + setters.join(", ") + " WHERE id = $1";
  return executor.execute(statement, [recordId, ...Object.values(changes)]);
}
`,
    },
    expectPairs: [["updateUserFields", "updateProductFields"]],
  },
  {
    id: "XS-P04",
    mode: "functions",
    category: "crud-rename",
    description: "Soft-delete helper duplicated; identical SQL fragments, renamed identifiers",
    files: {
      "user-removal.ts": `
interface UserDeleteRunner {
  execute(sql: string, params: unknown[]): Promise<number>;
}

export async function softDeleteUser(
  runner: UserDeleteRunner,
  table: string,
  id: string,
): Promise<boolean> {
  const sql = "UPDATE " + table + " SET deleted_at = NOW() WHERE id = $1 AND deleted_at IS NULL";
  const affected = await runner.execute(sql, [id]);
  if (affected > 1) {
    throw new Error("soft delete touched multiple rows");
  }
  return affected === 1;
}
`,
      "product-removal.ts": `
interface ProductDeleteRunner {
  execute(sql: string, params: unknown[]): Promise<number>;
}

export async function softDeleteProduct(
  gateway: ProductDeleteRunner,
  relation: string,
  recordId: string,
): Promise<boolean> {
  const statement = "UPDATE " + relation + " SET deleted_at = NOW() WHERE id = $1 AND deleted_at IS NULL";
  const touched = await gateway.execute(statement, [recordId]);
  if (touched > 1) {
    throw new Error("soft delete touched multiple rows");
  }
  return touched === 1;
}
`,
    },
    expectPairs: [["softDeleteUser", "softDeleteProduct"]],
  },
  {
    id: "XS-P05",
    mode: "functions",
    category: "crud-rename",
    description: "List helper duplicated; identical ORDER BY / LIMIT literals, renamed identifiers",
    files: {
      "user-listing.ts": `
interface UserListRunner {
  query(sql: string, params: unknown[]): Promise<{ rows: Record<string, unknown>[] }>;
}

export async function listUserRows(
  runner: UserListRunner,
  table: string,
  limit: number,
): Promise<Record<string, unknown>[]> {
  const capped = Math.min(Math.max(limit, 1), 100);
  const sql = "SELECT * FROM " + table + " ORDER BY created_at DESC LIMIT $1";
  const result = await runner.query(sql, [capped]);
  return result.rows;
}
`,
      "product-listing.ts": `
interface ProductListRunner {
  query(sql: string, params: unknown[]): Promise<{ rows: Record<string, unknown>[] }>;
}

export async function listProductRows(
  executor: ProductListRunner,
  relation: string,
  pageSize: number,
): Promise<Record<string, unknown>[]> {
  const bounded = Math.min(Math.max(pageSize, 1), 100);
  const statement = "SELECT * FROM " + relation + " ORDER BY created_at DESC LIMIT $1";
  const outcome = await executor.query(statement, [bounded]);
  return outcome.rows;
}
`,
    },
    expectPairs: [["listUserRows", "listProductRows"]],
  },
  {
    id: "XS-P06",
    mode: "functions",
    category: "validation-style",
    description: "Form validator duplicated; template literals vs concatenation, identical messages",
    files: {
      "signup-validation.ts": `
export function validateSignupForm(form: { email: string; password: string; age: number }): string[] {
  const problems: string[] = [];
  if (form.email.indexOf("@") === -1) {
    problems.push(\`email is invalid: \${form.email}\`);
  }
  if (form.password.length < 10) {
    problems.push(\`password too short: \${form.password.length}\`);
  }
  if (form.age < 18) {
    problems.push(\`age below minimum: \${form.age}\`);
  }
  return problems;
}
`,
      "invite-validation.ts": `
export function validateInviteForm(input: { email: string; password: string; age: number }): string[] {
  const issues: string[] = [];
  if (input.email.indexOf("@") === -1) {
    issues.push("email is invalid: " + input.email);
  }
  if (input.password.length < 10) {
    issues.push("password too short: " + input.password.length);
  }
  if (input.age < 18) {
    issues.push("age below minimum: " + input.age);
  }
  return issues;
}
`,
    },
    expectPairs: [["validateSignupForm", "validateInviteForm"]],
  },
  {
    id: "XS-P07",
    mode: "functions",
    category: "validation-style",
    description: "Time-window check duplicated; guard-return chain vs if/else-if chain",
    files: {
      "coupon-checks.ts": `
export function checkCouponWindow(coupon: { startsAt: number; endsAt: number }, now: number): string | null {
  if (coupon.startsAt > coupon.endsAt) {
    return "window is inverted";
  }
  if (now < coupon.startsAt) {
    return "not yet active";
  }
  if (now > coupon.endsAt) {
    return "already expired";
  }
  return null;
}
`,
      "voucher-checks.ts": `
export function checkVoucherWindow(voucher: { startsAt: number; endsAt: number }, at: number): string | null {
  if (voucher.startsAt > voucher.endsAt) {
    return "window is inverted";
  } else if (at < voucher.startsAt) {
    return "not yet active";
  } else if (at > voucher.endsAt) {
    return "already expired";
  } else {
    return null;
  }
}
`,
    },
    expectPairs: [["checkCouponWindow", "checkVoucherWindow"]],
  },
  {
    id: "XS-P08",
    mode: "functions",
    category: "validation-style",
    description: "Strength rater duplicated; renamed locals plus if/else vs ternary-return",
    files: {
      "passcode-strength.ts": `
export function ratePasscodeStrength(passcode: string): "weak" | "strong" {
  const hasDigit = /[0-9]/.test(passcode);
  const hasUpper = /[A-Z]/.test(passcode);
  const longEnough = passcode.length >= 12;
  if (hasDigit && hasUpper && longEnough) {
    return "strong";
  } else {
    return "weak";
  }
}
`,
      "secret-strength.ts": `
export function rateSecretStrength(secret: string): "weak" | "strong" {
  const containsDigit = /[0-9]/.test(secret);
  const containsUpper = /[A-Z]/.test(secret);
  const meetsLength = secret.length >= 12;
  return containsDigit && containsUpper && meetsLength ? "strong" : "weak";
}
`,
    },
    expectPairs: [["ratePasscodeStrength", "rateSecretStrength"]],
  },
  {
    id: "XS-P09",
    mode: "functions",
    category: "validation-style",
    description: "Line-item validator duplicated; for-of vs forEach, identical messages",
    files: {
      "order-line-validation.ts": `
export function validateOrderLines(lines: { sku: string; quantity: number; unitPrice: number }[]): string[] {
  const errors: string[] = [];
  for (const line of lines) {
    if (line.quantity <= 0) {
      errors.push(line.sku + ": quantity must be positive");
    }
    if (line.unitPrice < 0) {
      errors.push(line.sku + ": unit price cannot be negative");
    }
  }
  return errors;
}
`,
      "quote-line-validation.ts": `
export function validateQuoteLines(entries: { sku: string; quantity: number; unitPrice: number }[]): string[] {
  const faults: string[] = [];
  entries.forEach((entry) => {
    if (entry.quantity <= 0) {
      faults.push(entry.sku + ": quantity must be positive");
    }
    if (entry.unitPrice < 0) {
      faults.push(entry.sku + ": unit price cannot be negative");
    }
  });
  return faults;
}
`,
    },
    expectPairs: [["validateOrderLines", "validateQuoteLines"]],
  },
  {
    id: "XS-P10",
    mode: "functions",
    category: "validation-style",
    description: "URL validator duplicated; renamed locals, template vs concat, identical rules",
    files: {
      "webhook-url-validation.ts": `
export function validateWebhookUrl(url: string): string[] {
  const violations: string[] = [];
  if (!url.startsWith("https://")) {
    violations.push("webhook url must use https");
  }
  if (url.length > 2048) {
    violations.push(\`webhook url too long: \${url.length}\`);
  }
  if (url.includes(" ")) {
    violations.push("webhook url must not contain spaces");
  }
  return violations;
}
`,
      "callback-url-validation.ts": `
export function validateCallbackUrl(target: string): string[] {
  const findings: string[] = [];
  if (!target.startsWith("https://")) {
    findings.push("webhook url must use https");
  }
  if (target.length > 2048) {
    findings.push("webhook url too long: " + target.length);
  }
  if (target.includes(" ")) {
    findings.push("webhook url must not contain spaces");
  }
  return findings;
}
`,
    },
    expectPairs: [["validateWebhookUrl", "validateCallbackUrl"]],
  },
  {
    id: "XS-P11",
    mode: "functions",
    category: "mapper-style",
    description: "DTO mapper: explicit property access vs destructuring with shorthand",
    files: {
      "member-mapper.ts": `
interface RawMemberDto {
  member_id: string;
  display_name: string;
  joined_at: string;
  karma: number;
}

export function mapMemberResponse(raw: RawMemberDto): { id: string; name: string; joinedAt: Date; karma: number } {
  const view = {
    id: raw.member_id,
    name: raw.display_name,
    joinedAt: new Date(raw.joined_at),
    karma: raw.karma,
  };
  return view;
}
`,
      "account-mapper.ts": `
interface RawAccountDto {
  member_id: string;
  display_name: string;
  joined_at: string;
  karma: number;
}

export function mapAccountResponse(payload: RawAccountDto): { id: string; name: string; joinedAt: Date; karma: number } {
  const { member_id, display_name, joined_at, karma } = payload;
  const summary = {
    id: member_id,
    name: display_name,
    joinedAt: new Date(joined_at),
    karma,
  };
  return summary;
}
`,
    },
    expectPairs: [["mapMemberResponse", "mapAccountResponse"]],
  },
  {
    id: "XS-P12",
    mode: "functions",
    category: "mapper-style",
    description: "Wire-to-view mapper with Number/Date conversion; direct access vs destructuring",
    files: {
      "invoice-view-mapper.ts": `
interface WireInvoice {
  invoice_no: string;
  total_cents: string;
  issued_on: string;
  paid: boolean;
}

export function toInvoiceView(wire: WireInvoice): { number: string; totalCents: number; issuedOn: Date; paid: boolean } {
  return {
    number: wire.invoice_no,
    totalCents: Number(wire.total_cents),
    issuedOn: new Date(wire.issued_on),
    paid: wire.paid,
  };
}
`,
      "receipt-view-mapper.ts": `
interface WireReceipt {
  invoice_no: string;
  total_cents: string;
  issued_on: string;
  paid: boolean;
}

export function toReceiptView(payload: WireReceipt): { number: string; totalCents: number; issuedOn: Date; paid: boolean } {
  const { invoice_no, total_cents, issued_on, paid } = payload;
  return {
    number: invoice_no,
    totalCents: Number(total_cents),
    issuedOn: new Date(issued_on),
    paid,
  };
}
`,
    },
    expectPairs: [["toInvoiceView", "toReceiptView"]],
  },
  {
    id: "XS-P13",
    mode: "functions",
    category: "mapper-style",
    description: "Label builder duplicated; property access vs destructured locals, identical joins",
    files: {
      "shipping-label-mapper.ts": `
interface RawShippingDto {
  recipient: string;
  street: string;
  city: string;
  postal_code: string;
  country_code: string;
}

export function buildShippingLabel(raw: RawShippingDto): string[] {
  const lines = [
    raw.recipient,
    raw.street,
    raw.postal_code + " " + raw.city,
    raw.country_code.toUpperCase(),
  ];
  return lines;
}
`,
      "return-label-mapper.ts": `
interface RawReturnDto {
  recipient: string;
  street: string;
  city: string;
  postal_code: string;
  country_code: string;
}

export function buildReturnLabel(dto: RawReturnDto): string[] {
  const { recipient, street, city, postal_code, country_code } = dto;
  const rows = [
    recipient,
    street,
    postal_code + " " + city,
    country_code.toUpperCase(),
  ];
  return rows;
}
`,
    },
    expectPairs: [["buildShippingLabel", "buildReturnLabel"]],
  },
  {
    id: "XS-P14",
    mode: "functions",
    category: "mapper-style",
    description: "List mapper duplicated; object literal in callback vs destructuring in callback",
    files: {
      "session-list-mapper.ts": `
interface RawSessionDto {
  session_id: string;
  user_agent: string;
  started_at: string;
}

export function mapSessionList(items: RawSessionDto[]): { id: string; agent: string; startedAt: Date }[] {
  return items.map((item) => {
    const mapped = {
      id: item.session_id,
      agent: item.user_agent,
      startedAt: new Date(item.started_at),
    };
    return mapped;
  });
}
`,
      "device-list-mapper.ts": `
interface RawDeviceSessionDto {
  session_id: string;
  user_agent: string;
  started_at: string;
}

export function mapDeviceSessionList(records: RawDeviceSessionDto[]): { id: string; agent: string; startedAt: Date }[] {
  return records.map((record) => {
    const { session_id, user_agent, started_at } = record;
    return {
      id: session_id,
      agent: user_agent,
      startedAt: new Date(started_at),
    };
  });
}
`,
    },
    expectPairs: [["mapSessionList", "mapDeviceSessionList"]],
  },

  // -------------------------------------------------------------------
  // Part 2: functions scenarios — negatives
  // (the classic false-positive traps in real code)
  // -------------------------------------------------------------------
  {
    id: "XS-N01",
    mode: "functions",
    category: "sql-literal-trap",
    description: "COUNT twins identical except the SQL table/column literal (users vs orders)",
    files: {
      "user-count-repo.ts": `
interface UserCountClient {
  query(sql: string, params: unknown[]): Promise<{ rows: { total: number }[] }>;
}

export async function countActiveUsers(client: UserCountClient, since: Date): Promise<number> {
  const sql = "SELECT COUNT(*) AS total FROM users WHERE last_seen_at >= $1";
  const result = await client.query(sql, [since.toISOString()]);
  if (result.rows.length === 0) {
    return 0;
  }
  return result.rows[0].total;
}
`,
      "order-count-repo.ts": `
interface OrderCountClient {
  query(sql: string, params: unknown[]): Promise<{ rows: { total: number }[] }>;
}

export async function countRecentOrders(client: OrderCountClient, since: Date): Promise<number> {
  const sql = "SELECT COUNT(*) AS total FROM orders WHERE placed_at >= $1";
  const result = await client.query(sql, [since.toISOString()]);
  if (result.rows.length === 0) {
    return 0;
  }
  return result.rows[0].total;
}
`,
    },
    expectPairs: [["countActiveUsers", "countRecentOrders"]],
  },
  {
    id: "XS-N02",
    mode: "functions",
    category: "sql-literal-trap",
    description: "DELETE twins identical except the target table literal (sessions vs refresh_tokens)",
    files: {
      "session-cleanup.ts": `
interface SessionCleanupRunner {
  execute(sql: string, params: unknown[]): Promise<number>;
}

export async function purgeExpiredSessions(runner: SessionCleanupRunner, cutoff: Date): Promise<number> {
  const sql = "DELETE FROM sessions WHERE expires_at < $1";
  const removed = await runner.execute(sql, [cutoff.toISOString()]);
  if (removed > 0) {
    console.info("purged " + removed + " rows");
  }
  return removed;
}
`,
      "token-cleanup.ts": `
interface TokenCleanupRunner {
  execute(sql: string, params: unknown[]): Promise<number>;
}

export async function purgeExpiredRefreshTokens(runner: TokenCleanupRunner, cutoff: Date): Promise<number> {
  const sql = "DELETE FROM refresh_tokens WHERE expires_at < $1";
  const removed = await runner.execute(sql, [cutoff.toISOString()]);
  if (removed > 0) {
    console.info("purged " + removed + " rows");
  }
  return removed;
}
`,
    },
    expectPairs: [["purgeExpiredSessions", "purgeExpiredRefreshTokens"]],
  },
  {
    id: "XS-N03",
    mode: "functions",
    category: "endpoint-literal-trap",
    description: "HTTP fetchers identical except the endpoint path literal (customers vs suppliers)",
    files: {
      "customer-api-client.ts": `
export async function fetchCustomerProfile(baseUrl: string, id: string): Promise<unknown> {
  const response = await fetch(baseUrl + "/api/customers/" + id, {
    headers: { accept: "application/json" },
  });
  if (!response.ok) {
    throw new Error("request failed with " + response.status);
  }
  return response.json();
}
`,
      "supplier-api-client.ts": `
export async function fetchSupplierProfile(baseUrl: string, id: string): Promise<unknown> {
  const response = await fetch(baseUrl + "/api/suppliers/" + id, {
    headers: { accept: "application/json" },
  });
  if (!response.ok) {
    throw new Error("request failed with " + response.status);
  }
  return response.json();
}
`,
    },
    expectPairs: [["fetchCustomerProfile", "fetchSupplierProfile"]],
  },
  {
    id: "XS-N04",
    mode: "functions",
    category: "collection-literal-trap",
    description: "Event appenders identical except the collection name literal",
    files: {
      "activity-feed-store.ts": `
interface FeedDatabase {
  collection(name: string): { insertOne(doc: Record<string, unknown>): Promise<void> };
}

export async function appendActivityEvent(db: FeedDatabase, event: Record<string, unknown>): Promise<void> {
  if (!event.type) {
    throw new Error("event type is required");
  }
  const doc = { ...event, recordedAt: Date.now() };
  await db.collection("activity_events").insertOne(doc);
}
`,
      "billing-feed-store.ts": `
interface BillingDatabase {
  collection(name: string): { insertOne(doc: Record<string, unknown>): Promise<void> };
}

export async function appendBillingEvent(db: BillingDatabase, event: Record<string, unknown>): Promise<void> {
  if (!event.type) {
    throw new Error("event type is required");
  }
  const doc = { ...event, recordedAt: Date.now() };
  await db.collection("billing_events").insertOne(doc);
}
`,
    },
    expectPairs: [["appendActivityEvent", "appendBillingEvent"]],
  },
  {
    id: "XS-N05",
    mode: "functions",
    category: "create-vs-update",
    description: "Create inserts blindly (201); update checks existence first, then merges (404/204)",
    files: {
      "article-create-handler.ts": `
interface ArticleWriteStore {
  insert(record: Record<string, unknown>): Promise<string>;
}

export async function handleArticleCreate(
  store: ArticleWriteStore,
  body: { slug: string; title: string },
): Promise<{ status: number; id: string }> {
  if (!body.slug || !body.title) {
    return { status: 422, id: "" };
  }
  const id = await store.insert({ slug: body.slug, title: body.title, createdAt: Date.now() });
  console.info("article created: " + id);
  return { status: 201, id };
}
`,
      "article-update-handler.ts": `
interface ArticleEditStore {
  findBySlug(slug: string): Promise<Record<string, unknown> | null>;
  merge(slug: string, patch: Record<string, unknown>): Promise<void>;
}

export async function handleArticleUpdate(
  store: ArticleEditStore,
  body: { slug: string; title: string },
): Promise<{ status: number }> {
  if (!body.slug || !body.title) {
    return { status: 422 };
  }
  const existing = await store.findBySlug(body.slug);
  if (existing === null) {
    return { status: 404 };
  }
  await store.merge(body.slug, { title: body.title, updatedAt: Date.now() });
  return { status: 204 };
}
`,
    },
    forbidPairs: [["handleArticleCreate", "handleArticleUpdate"]],
  },
  {
    id: "XS-N06",
    mode: "functions",
    category: "create-vs-update",
    description: "Webhook register (insert) vs reconfigure (lookup then patch): different semantics",
    files: {
      "webhook-register-handler.ts": `
interface HookRegistryStore {
  saveSubscription(sub: { url: string; events: string[] }): Promise<string>;
}

export async function registerWebhookEndpoint(
  registry: HookRegistryStore,
  payload: { url: string; events: string[] },
): Promise<{ code: number; token: string }> {
  if (payload.events.length === 0) {
    return { code: 422, token: "" };
  }
  const token = await registry.saveSubscription({ url: payload.url, events: payload.events });
  return { code: 201, token };
}
`,
      "webhook-reconfigure-handler.ts": `
interface HookConfigStore {
  findSubscription(token: string): Promise<{ url: string; events: string[] } | null>;
  patchSubscription(token: string, changes: { events: string[] }): Promise<void>;
}

export async function reconfigureWebhookEndpoint(
  config: HookConfigStore,
  token: string,
  events: string[],
): Promise<{ code: number }> {
  if (events.length === 0) {
    return { code: 422 };
  }
  const existing = await config.findSubscription(token);
  if (existing === null) {
    return { code: 404 };
  }
  await config.patchSubscription(token, { events });
  return { code: 200 };
}
`,
    },
    forbidPairs: [["registerWebhookEndpoint", "reconfigureWebhookEndpoint"]],
  },
  {
    id: "XS-N07",
    mode: "functions",
    category: "pagination-trap",
    description: "Full fetcher vs paginated fetcher that slices with page/perPage",
    files: {
      "tag-catalog-fetcher.ts": `
interface TagApiClient {
  get(path: string): Promise<{ name: string; count: number }[]>;
}

export async function fetchAllTags(api: TagApiClient): Promise<{ name: string; count: number }[]> {
  const tags = await api.get("/tags");
  const visible = tags.filter((tag) => tag.count > 0);
  visible.sort((a, b) => b.count - a.count);
  return visible;
}
`,
      "tag-page-fetcher.ts": `
interface TagPageApiClient {
  get(path: string): Promise<{ name: string; count: number }[]>;
}

export async function fetchTagPage(
  api: TagPageApiClient,
  page: number,
  perPage: number,
): Promise<{ name: string; count: number }[]> {
  const tags = await api.get("/tags");
  const visible = tags.filter((tag) => tag.count > 0);
  visible.sort((a, b) => b.count - a.count);
  const start = (page - 1) * perPage;
  return visible.slice(start, start + perPage);
}
`,
    },
    forbidPairs: [["fetchAllTags", "fetchTagPage"]],
  },
  {
    id: "XS-N08",
    mode: "functions",
    category: "pagination-trap",
    description: "Full audit reader vs windowed reader applying offset/limit slice",
    files: {
      "audit-trail-reader.ts": `
interface TrailLogSource {
  load(stream: string): Promise<{ actor: string; action: string; at: number }[]>;
}

export async function readAuditTrail(
  source: TrailLogSource,
  stream: string,
): Promise<{ actor: string; action: string; at: number }[]> {
  const entries = await source.load(stream);
  const sorted = [...entries];
  sorted.sort((a, b) => a.at - b.at);
  return sorted.filter((entry) => entry.action !== "heartbeat");
}
`,
      "audit-window-reader.ts": `
interface WindowLogSource {
  load(stream: string): Promise<{ actor: string; action: string; at: number }[]>;
}

export async function readAuditWindow(
  source: WindowLogSource,
  stream: string,
  offset: number,
  limit: number,
): Promise<{ actor: string; action: string; at: number }[]> {
  const entries = await source.load(stream);
  const sorted = [...entries];
  sorted.sort((a, b) => a.at - b.at);
  const kept = sorted.filter((entry) => entry.action !== "heartbeat");
  return kept.slice(offset, offset + limit);
}
`,
    },
    forbidPairs: [["readAuditTrail", "readAuditWindow"]],
  },
  {
    id: "XS-N09",
    mode: "functions",
    category: "format-literal-trap",
    description: "Price formatters identical except locale and currency literals",
    files: {
      "usd-price-formatter.ts": `
export function formatUsdPrice(cents: number): string {
  const amount = cents / 100;
  const formatter = new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
  });
  return formatter.format(amount);
}
`,
      "eur-price-formatter.ts": `
export function formatEuroPrice(cents: number): string {
  const amount = cents / 100;
  const formatter = new Intl.NumberFormat("de-DE", {
    style: "currency",
    currency: "EUR",
  });
  return formatter.format(amount);
}
`,
    },
    expectPairs: [["formatUsdPrice", "formatEuroPrice"]],
  },
  {
    id: "XS-N10",
    mode: "functions",
    category: "format-literal-trap",
    description: "Date stampers with the same parts but different output format (ISO vs EU)",
    files: {
      "iso-date-stamp.ts": `
export function stampReportDateIso(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return year + "-" + month + "-" + day;
}
`,
      "eu-date-stamp.ts": `
export function stampReportDateEu(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return day + "/" + month + "/" + year;
}
`,
    },
    expectPairs: [["stampReportDateIso", "stampReportDateEu"]],
  },
];
