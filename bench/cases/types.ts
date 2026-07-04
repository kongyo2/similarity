/**
 * TYPE-mode benchmark corpus.
 *
 * Positives (XT-P*) are pairs of type declarations that are structurally the
 * same type — a reviewer would merge them. They differ only in declaration
 * order, identifier names, or standard TypeScript spelling alternatives
 * (`Array<T>` vs `T[]`, `x?: T` vs `x: T | undefined`, interface vs type
 * alias, generic parameter names, union member order).
 *
 * Negatives (XT-N*) are pairs that look similar (same property names or
 * similar shapes) but are genuinely different contracts: differing property
 * types, optionality flips, different generic arguments, different union
 * members, or different index-signature value types.
 */

import type { BenchCase } from "../cases.js";

export const typeCases: BenchCase[] = [
  // -------------------------------------------------------------------
  // positives — member-order
  // -------------------------------------------------------------------
  {
    id: "XT-P01",
    mode: "types",
    category: "member-order",
    description: "Identical members declared in fully shuffled order",
    files: {
      "a.ts": `
export interface PaymentReceipt {
  id: string;
  amountCents: number;
  currency: string;
  paidAt: Date;
  method: string;
}
`,
      "b.ts": `
export interface ChargeReceipt {
  currency: string;
  paidAt: Date;
  id: string;
  method: string;
  amountCents: number;
}
`,
    },
    expectPairs: [["PaymentReceipt", "ChargeReceipt"]],
  },
  {
    id: "XT-P02",
    mode: "types",
    category: "member-order",
    description: "Six-member config with every member moved to a new position",
    files: {
      "a.ts": `
export interface GatewayConfig {
  host: string;
  port: number;
  useTls: boolean;
  maxConnections: number;
  logLevel: string;
  tags: string[];
}
`,
      "b.ts": `
export interface EdgeNodeConfig {
  tags: string[];
  maxConnections: number;
  logLevel: string;
  useTls: boolean;
  host: string;
  port: number;
}
`,
    },
    expectPairs: [["GatewayConfig", "EdgeNodeConfig"]],
  },
  {
    id: "XT-P03",
    mode: "types",
    category: "member-order",
    description: "Mixed optional and required members, fully shuffled",
    files: {
      "a.ts": `
export interface ProfileForm {
  username: string;
  email: string;
  bio?: string;
  avatarUrl?: string;
  age: number;
  newsletter: boolean;
}
`,
      "b.ts": `
export interface AccountForm {
  avatarUrl?: string;
  newsletter: boolean;
  username: string;
  bio?: string;
  email: string;
  age: number;
}
`,
    },
    expectPairs: [["ProfileForm", "AccountForm"]],
  },

  // -------------------------------------------------------------------
  // positives — rename-props
  // -------------------------------------------------------------------
  {
    id: "XT-P04",
    mode: "types",
    category: "rename-props",
    description: "Address shape with every property renamed, all strings",
    files: {
      "a.ts": `
export interface ShippingAddress {
  street: string;
  city: string;
  zip: string;
}
`,
      "b.ts": `
export interface MailingLocation {
  line: string;
  town: string;
  postal: string;
}
`,
    },
    expectPairs: [["ShippingAddress", "MailingLocation"]],
  },
  {
    id: "XT-P05",
    mode: "types",
    category: "rename-props",
    description:
      "Distinctive skeleton (string/number/boolean/string[]/number) with all names changed",
    files: {
      "a.ts": `
export interface RepoStats {
  name: string;
  stars: number;
  isArchived: boolean;
  topics: string[];
  forks: number;
}
`,
      "b.ts": `
export interface PackageMetrics {
  title: string;
  downloads: number;
  isDeprecated: boolean;
  keywords: string[];
  dependents: number;
}
`,
    },
    expectPairs: [["RepoStats", "PackageMetrics"]],
  },
  {
    id: "XT-P06",
    mode: "types",
    category: "rename-props",
    description: "Personnel records with renamed properties and type name",
    files: {
      "a.ts": `
export interface EmployeeRecord {
  fullName: string;
  hiredAt: Date;
  salaryCents: number;
  active: boolean;
  teams: string[];
}
`,
      "b.ts": `
export interface ContractorEntry {
  legalName: string;
  startedAt: Date;
  rateCents: number;
  engaged: boolean;
  clients: string[];
}
`,
    },
    expectPairs: [["EmployeeRecord", "ContractorEntry"]],
  },

  // -------------------------------------------------------------------
  // positives — array-spelling
  // -------------------------------------------------------------------
  {
    id: "XT-P07",
    mode: "types",
    category: "array-spelling",
    description: "Array<string> vs string[] for the same member set",
    files: {
      "a.ts": `
export interface TagIndex {
  entries: Array<string>;
  total: number;
  updatedAt: Date;
}
`,
      "b.ts": `
export interface LabelIndex {
  entries: string[];
  total: number;
  updatedAt: Date;
}
`,
    },
    expectPairs: [["TagIndex", "LabelIndex"]],
  },
  {
    id: "XT-P08",
    mode: "types",
    category: "array-spelling",
    description: "ReadonlyArray<number> vs readonly number[]",
    files: {
      "a.ts": `
export interface MetricWindow {
  samples: ReadonlyArray<number>;
  intervalMs: number;
  source: string;
}
`,
      "b.ts": `
export interface GaugeWindow {
  samples: readonly number[];
  intervalMs: number;
  source: string;
}
`,
    },
    expectPairs: [["MetricWindow", "GaugeWindow"]],
  },
  {
    id: "XT-P09",
    mode: "types",
    category: "array-spelling",
    description: "Nested Array<Array<GridPoint>> vs GridPoint[][]",
    files: {
      "a.ts": `
export interface GridPoint {
  x: number;
  y: number;
}

export interface HeatmapModel {
  cells: Array<Array<GridPoint>>;
  width: number;
  height: number;
}

export interface ContourModel {
  cells: GridPoint[][];
  width: number;
  height: number;
}
`,
    },
    expectPairs: [["HeatmapModel", "ContourModel"]],
  },

  // -------------------------------------------------------------------
  // positives — optional-spelling
  // -------------------------------------------------------------------
  {
    id: "XT-P10",
    mode: "types",
    category: "optional-spelling",
    description: "cursor?: string vs cursor: string | undefined",
    files: {
      "a.ts": `
export interface SearchQuery {
  term: string;
  limit: number;
  cursor?: string;
}
`,
      "b.ts": `
export interface LookupQuery {
  term: string;
  limit: number;
  cursor: string | undefined;
}
`,
    },
    expectPairs: [["SearchQuery", "LookupQuery"]],
  },
  {
    id: "XT-P11",
    mode: "types",
    category: "optional-spelling",
    description: "Two optional members spelled both ways among required members",
    files: {
      "a.ts": `
export interface NotificationPrefs {
  email: boolean;
  sms: boolean;
  locale: string;
  digestHour?: number;
  timezone?: string;
}
`,
      "b.ts": `
export interface AlertPrefs {
  email: boolean;
  sms: boolean;
  locale: string;
  digestHour: number | undefined;
  timezone: string | undefined;
}
`,
    },
    expectPairs: [["NotificationPrefs", "AlertPrefs"]],
  },
  {
    id: "XT-P12",
    mode: "types",
    category: "optional-spelling",
    description: "Interface with ?-optionals vs type alias with | undefined",
    files: {
      "a.ts": `
export interface CheckoutDraft {
  items: string[];
  total: number;
  couponCode?: string;
  note?: string;
}

export type BasketDraft = {
  items: string[];
  total: number;
  couponCode: string | undefined;
  note: string | undefined;
};
`,
    },
    expectPairs: [["CheckoutDraft", "BasketDraft"]],
  },

  // -------------------------------------------------------------------
  // positives — union-order
  // -------------------------------------------------------------------
  {
    id: "XT-P13",
    mode: "types",
    category: "union-order",
    description: "String-literal status union with members reordered",
    files: {
      "a.ts": `
export type SubscriptionStatus = "active" | "inactive" | "pending";
export type MembershipStatus = "pending" | "active" | "inactive";
`,
    },
    expectPairs: [["SubscriptionStatus", "MembershipStatus"]],
  },
  {
    id: "XT-P14",
    mode: "types",
    category: "union-order",
    description: "Union of object type references with arms reordered",
    files: {
      "a.ts": `
export interface CardPayment {
  kind: "card";
  last4: string;
  expiryMonth: number;
}

export interface BankPayment {
  kind: "bank";
  iban: string;
  holder: string;
}

export interface WalletPayment {
  kind: "wallet";
  provider: string;
  walletId: string;
}

export type PaymentMethod = CardPayment | BankPayment | WalletPayment;
export type SettlementMethod = WalletPayment | CardPayment | BankPayment;
`,
    },
    expectPairs: [["PaymentMethod", "SettlementMethod"]],
  },
  {
    id: "XT-P15",
    mode: "types",
    category: "union-order",
    description: "string | number | null permuted",
    files: {
      "a.ts": `
export type CellValue = string | number | null;
export type FieldValue = number | null | string;
`,
    },
    expectPairs: [["CellValue", "FieldValue"]],
  },

  // -------------------------------------------------------------------
  // positives — cross-kind
  // -------------------------------------------------------------------
  {
    id: "XT-P16",
    mode: "types",
    category: "cross-kind",
    description: "Interface vs type alias with identical plain bodies",
    files: {
      "a.ts": `
export interface AuditEntry {
  actorId: string;
  action: string;
  at: Date;
  metadata: Record<string, string>;
}
`,
      "b.ts": `
export type LedgerEntry = {
  actorId: string;
  action: string;
  at: Date;
  metadata: Record<string, string>;
};
`,
    },
    expectPairs: [["AuditEntry", "LedgerEntry"]],
  },
  {
    id: "XT-P17",
    mode: "types",
    category: "cross-kind",
    description:
      "Interface method shorthand m(): void vs type alias property m: () => void",
    files: {
      "a.ts": `
export interface Publisher {
  topic: string;
  buffered: boolean;
  publish(message: string): void;
  close(): Promise<void>;
}

export type Broadcaster = {
  topic: string;
  buffered: boolean;
  publish: (message: string) => void;
  close: () => Promise<void>;
};
`,
    },
    expectPairs: [["Publisher", "Broadcaster"]],
  },

  // -------------------------------------------------------------------
  // positives — generics
  // -------------------------------------------------------------------
  {
    id: "XT-P18",
    mode: "types",
    category: "generics",
    description: "Generic container with type parameter renamed T -> U",
    files: {
      "a.ts": `
export interface Box<T> {
  value: T;
  tag: string;
}
`,
      "b.ts": `
export interface Crate<U> {
  value: U;
  tag: string;
}
`,
    },
    expectPairs: [["Box", "Crate"]],
  },
  {
    id: "XT-P19",
    mode: "types",
    category: "generics",
    description: "Two type parameters renamed K,V -> A,B",
    files: {
      "a.ts": `
export interface CachePair<K, V> {
  key: K;
  value: V;
  expiresAt: number;
}
`,
      "b.ts": `
export interface StorePair<A, B> {
  key: A;
  value: B;
  expiresAt: number;
}
`,
    },
    expectPairs: [["CachePair", "StorePair"]],
  },
  {
    id: "XT-P20",
    mode: "types",
    category: "generics",
    description: "Concrete instantiation: CatalogItem[] vs Array<CatalogItem>",
    files: {
      "a.ts": `
export interface CatalogItem {
  sku: string;
  priceCents: number;
}

export interface ItemWrapper {
  data: CatalogItem[];
  count: number;
}

export interface ItemBundle {
  data: Array<CatalogItem>;
  count: number;
}
`,
    },
    expectPairs: [["ItemWrapper", "ItemBundle"]],
  },

  // -------------------------------------------------------------------
  // negatives — same-names-diff-types
  // -------------------------------------------------------------------
  {
    id: "XT-N01",
    mode: "types",
    category: "same-names-diff-types",
    description: "Parsed device state vs raw CSV row with identical keys",
    files: {
      "a.ts": `
export interface DeviceInfo {
  id: string;
  battery: number;
  online: boolean;
  lastSeen: Date;
}

export interface DeviceCsvRow {
  id: number;
  battery: string;
  online: string;
  lastSeen: string;
}
`,
    },
    forbidPairs: [["DeviceInfo", "DeviceCsvRow"]],
  },
  {
    id: "XT-N02",
    mode: "types",
    category: "same-names-diff-types",
    description: "Domain balance vs wire-format balance, three types differ",
    files: {
      "a.ts": `
export interface AccountBalance {
  userId: string;
  amount: number;
  currency: string;
  updatedAt: Date;
}

export interface WireBalance {
  userId: number;
  amount: string;
  currency: string;
  updatedAt: number;
}
`,
    },
    forbidPairs: [["AccountBalance", "WireBalance"]],
  },
  {
    id: "XT-N03",
    mode: "types",
    category: "same-names-diff-types",
    description: "Form values vs per-field error messages, same keys",
    files: {
      "a.ts": `
export interface CheckoutFields {
  email: string;
  quantity: number;
  giftWrap: boolean;
  notes: string;
}

export interface CheckoutFieldErrors {
  email: string[];
  quantity: string[];
  giftWrap: string[];
  notes: string[];
}
`,
    },
    forbidPairs: [["CheckoutFields", "CheckoutFieldErrors"]],
  },

  // -------------------------------------------------------------------
  // negatives — optional-required
  // -------------------------------------------------------------------
  {
    id: "XT-N04",
    mode: "types",
    category: "optional-required",
    description: "Create-input with four optionals vs fully-required stored row",
    files: {
      "a.ts": `
export interface CreateTicketInput {
  title: string;
  body?: string;
  assignee?: string;
  priority?: number;
  labels?: string[];
}

export interface TicketRow {
  title: string;
  body: string;
  assignee: string;
  priority: number;
  labels: string[];
}
`,
    },
    forbidPairs: [["CreateTicketInput", "TicketRow"]],
  },
  {
    id: "XT-N05",
    mode: "types",
    category: "optional-required",
    description: "Caller options with defaults vs resolved settings, three flips",
    files: {
      "a.ts": `
export interface RenderOptions {
  width: number;
  height: number;
  dpi?: number;
  background?: string;
  antialias?: boolean;
}

export interface ResolvedRenderSettings {
  width: number;
  height: number;
  dpi: number;
  background: string;
  antialias: boolean;
}
`,
    },
    forbidPairs: [["RenderOptions", "ResolvedRenderSettings"]],
  },

  // -------------------------------------------------------------------
  // negatives — generic-arg
  // -------------------------------------------------------------------
  {
    id: "XT-N06",
    mode: "types",
    category: "generic-arg",
    description: "Array<string> vs Array<number> element types",
    files: {
      "a.ts": `
export interface RecipientList {
  label: string;
  items: Array<string>;
  total: number;
}

export interface PortAllowList {
  label: string;
  items: Array<number>;
  total: number;
}
`,
    },
    forbidPairs: [["RecipientList", "PortAllowList"]],
  },
  {
    id: "XT-N07",
    mode: "types",
    category: "generic-arg",
    description: "Repositories returning Promise<ShopUser> vs Promise<ShopOrder>",
    files: {
      "a.ts": `
export interface ShopUser {
  id: string;
  email: string;
  displayName: string;
}

export interface UserGateway {
  findById(id: string): Promise<ShopUser>;
  listRecent(limit: number): Promise<ShopUser[]>;
  count(): Promise<number>;
}
`,
      "b.ts": `
export interface ShopOrder {
  id: string;
  totalCents: number;
  placedAt: Date;
}

export interface OrderGateway {
  findById(id: string): Promise<ShopOrder>;
  listRecent(limit: number): Promise<ShopOrder[]>;
  count(): Promise<number>;
}
`,
    },
    forbidPairs: [["UserGateway", "OrderGateway"]],
  },
  {
    id: "XT-N08",
    mode: "types",
    category: "generic-arg",
    description: "Map<string, number> vs Map<number, string> key/value swap",
    files: {
      "a.ts": `
export interface PriceLookup {
  entries: Map<string, number>;
  source: string;
  refreshedAt: Date;
}

export interface SkuLookup {
  entries: Map<number, string>;
  source: string;
  refreshedAt: Date;
}
`,
    },
    forbidPairs: [["PriceLookup", "SkuLookup"]],
  },

  // -------------------------------------------------------------------
  // negatives — union-members
  // -------------------------------------------------------------------
  {
    id: "XT-N09",
    mode: "types",
    category: "union-members",
    description: "HTTP method unions sharing two of three literals",
    files: {
      "a.ts": `
export type FormMethod = "GET" | "POST" | "PUT";
export type RestMethod = "GET" | "POST" | "DELETE";
`,
    },
    forbidPairs: [["FormMethod", "RestMethod"]],
  },
  {
    id: "XT-N10",
    mode: "types",
    category: "union-members",
    description: "Object-ref unions overlapping in two of three arms",
    files: {
      "a.ts": `
export interface ClickEvt {
  kind: "click";
  x: number;
  y: number;
}

export interface KeyEvt {
  kind: "key";
  code: string;
}

export interface ScrollEvt {
  kind: "scroll";
  deltaY: number;
}

export interface FocusEvt {
  kind: "focus";
  targetId: string;
}

export type EditorEvent = ClickEvt | KeyEvt | ScrollEvt;
export type DialogEvent = ClickEvt | KeyEvt | FocusEvt;
`,
    },
    forbidPairs: [["EditorEvent", "DialogEvent"]],
  },

  // -------------------------------------------------------------------
  // negatives — index-signature
  // -------------------------------------------------------------------
  {
    id: "XT-N11",
    mode: "types",
    category: "index-signature",
    description: "String-keyed index signatures with number vs string values",
    files: {
      "a.ts": `
export interface MetricCounters {
  [metric: string]: number;
}

export interface MetricLabels {
  [metric: string]: string;
}
`,
    },
    forbidPairs: [["MetricCounters", "MetricLabels"]],
  },
  {
    id: "XT-N12",
    mode: "types",
    category: "index-signature",
    description: "Open index-signature map vs single concrete boolean member",
    files: {
      "a.ts": `
export interface FeatureFlags {
  [flag: string]: boolean;
}

export interface FeatureToggle {
  enabled: boolean;
}
`,
    },
    forbidPairs: [["FeatureFlags", "FeatureToggle"]],
  },
];
