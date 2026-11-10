/**
 * Corpus for the syntax the engine used to drop or flatten: JSX, call and
 * construct signatures, overloads, accessors, members keyed by a number or
 * a symbol, rest and default parameters, function-typed parameters, and
 * classes written as expressions.
 *
 * Each of these was invisible to the comparison in some way, and
 * invisible in both declarations of a pair: two components rendering
 * unrelated markup were the same tree, two interfaces that are all
 * signature were two empty shapes, an overloaded member was its last
 * signature, a class bound to a `const` was no class. The positives pin
 * the other side — the same declarations still pair up now that the
 * syntax counts.
 *
 * Evaluated with the CLI defaults from the README invocation
 * (threshold 0.8, minLines 3) in the mode each case targets.
 */

import type { BenchCase } from "../cases.js";

export const syntaxCoverageCases: BenchCase[] = [
  // -------------------------------------------------------------------
  // functions — JSX is structure, not one opaque leaf
  // -------------------------------------------------------------------
  {
    id: "XF-N47",
    mode: "functions",
    category: "jsx",
    description: "Components of the same outline rendering unrelated markup",
    files: {
      "a.tsx": `
export function UserCard({ user }: { user: User }) {
  return (
    <section className="card">
      <h2>{user.name}</h2>
      <p>{user.email}</p>
    </section>
  );
}

export function OrderTable({ orders }: { orders: Order[] }) {
  return (
    <table className="orders">
      <tbody>
        {orders.map((order) => (
          <tr key={order.id}>
            <td>{order.total}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
`,
    },
    forbidPairs: [["UserCard", "OrderTable"]],
  },
  {
    id: "XF-N48",
    mode: "functions",
    category: "jsx",
    description: "The same form wrapper around different controls",
    files: {
      "a.tsx": `
export function LoginForm(props: FormProps) {
  return (
    <form onSubmit={props.onSubmit}>
      <input name="email" type="email" />
      <input name="password" type="password" />
      <button type="submit">Sign in</button>
    </form>
  );
}

export function SearchBar(props: FormProps) {
  return (
    <form onSubmit={props.onSubmit}>
      <label htmlFor="q">Search</label>
      <select name="scope">
        <option value="all">Everything</option>
      </select>
    </form>
  );
}
`,
    },
    forbidPairs: [["LoginForm", "SearchBar"]],
  },
  {
    id: "XF-N49",
    mode: "functions",
    category: "jsx",
    description: "Identical statements around different components and props",
    files: {
      "a.tsx": `
export function SaveButton(props: ActionProps) {
  const label = props.busy ? "Saving" : "Save";
  return <Button kind="primary" disabled={props.busy} onClick={props.onSave}>{label}</Button>;
}

export function HelpLink(props: ActionProps) {
  const label = props.busy ? "Saving" : "Save";
  return <Tooltip text={label}><Icon name="help" size={16} /><a href="/help">Docs</a></Tooltip>;
}
`,
    },
    forbidPairs: [["SaveButton", "HelpLink"]],
  },
  {
    id: "XF-N50",
    mode: "functions",
    category: "jsx",
    description: "List renderers over different item markup",
    files: {
      "a.tsx": `
export function renderTags(tags: string[]) {
  return (
    <ul className="tags">
      {tags.map((tag) => (
        <li key={tag}>{tag}</li>
      ))}
    </ul>
  );
}

export function renderSteps(steps: Step[]) {
  return (
    <ol className="steps">
      {steps.map((step) => (
        <li key={step.id}><strong>{step.title}</strong><em>{step.hint}</em><Progress value={step.done} /></li>
      ))}
    </ol>
  );
}
`,
    },
    forbidPairs: [["renderTags", "renderSteps"]],
  },
  {
    id: "XF-P55",
    mode: "functions",
    category: "jsx",
    description: "Renamed component twins across files",
    files: {
      "a.tsx": `
export function UserBadge(props: BadgeProps) {
  const initials = props.name.slice(0, 2).toUpperCase();
  return (
    <span className="badge" title={props.name}>
      <img src={props.avatar} alt="" />
      {initials}
    </span>
  );
}
`,
      "b.tsx": `
export function MemberBadge(input: BadgeProps) {
  const letters = input.name.slice(0, 2).toUpperCase();
  return (
    <span className="badge" title={input.name}>
      <img src={input.avatar} alt="" />
      {letters}
    </span>
  );
}
`,
    },
    expectPairs: [["UserBadge", "MemberBadge"]],
  },
  {
    id: "XF-P56",
    mode: "functions",
    category: "jsx",
    description: "Arrow component and function component with the same body",
    files: {
      "a.tsx": `
export const EmptyState = (props: EmptyProps) => {
  if (props.loading) {
    return <Spinner size="large" />;
  }
  return (
    <div className="empty">
      <p>{props.message}</p>
      <button onClick={props.onRetry}>Retry</button>
    </div>
  );
};

export function NothingHere(options: EmptyProps) {
  if (options.loading) {
    return <Spinner size="large" />;
  }
  return (
    <div className="empty">
      <p>{options.message}</p>
      <button onClick={options.onRetry}>Retry</button>
    </div>
  );
}
`,
    },
    expectPairs: [["EmptyState", "NothingHere"]],
  },
  {
    id: "XF-P57",
    mode: "functions",
    category: "jsx",
    description: "The same markup on one line and laid out over several",
    files: {
      "a.tsx": `
export function Notice(props: NoticeProps) {
  return <aside className="notice" role="status"><strong>{props.title}</strong><span>{props.body}</span></aside>;
}

export function Banner(props: NoticeProps) {
  return (
    <aside className="notice" role="status">
      <strong>{props.title}</strong>
      <span>{props.body}</span>
    </aside>
  );
}
`,
    },
    expectPairs: [["Notice", "Banner"]],
  },
  // -------------------------------------------------------------------
  // functions — directives and rest parameters are part of a function
  // -------------------------------------------------------------------
  {
    id: "XF-P58",
    mode: "functions",
    category: "parameters-and-directives",
    description: "Renamed server actions with the same directive",
    files: {
      "a.ts": `
export async function saveDraft(form: FormData) {
  "use server";
  const draft = parseDraft(form);
  await store.put(draft.id, draft);
  return draft.id;
}

export async function storeDraft(input: FormData) {
  "use server";
  const parsed = parseDraft(input);
  await store.put(parsed.id, parsed);
  return parsed.id;
}
`,
    },
    expectPairs: [["saveDraft", "storeDraft"]],
  },
  {
    id: "XF-P59",
    mode: "functions",
    category: "parameters-and-directives",
    description: "Renamed variadic helpers",
    files: {
      "a.ts": `
export function joinPath(base: string, ...segments: string[]): string {
  let result = base;
  for (const segment of segments) {
    result = result.replace(/\\/$/, "") + "/" + segment;
  }
  return result;
}

export function resolveUrl(root: string, ...parts: string[]): string {
  let url = root;
  for (const part of parts) {
    url = url.replace(/\\/$/, "") + "/" + part;
  }
  return url;
}
`,
    },
    expectPairs: [["joinPath", "resolveUrl"]],
  },
  // -------------------------------------------------------------------
  // functions — a class expression bound to a name has methods too
  // -------------------------------------------------------------------
  {
    id: "XF-P60",
    mode: "functions",
    category: "class-expression",
    description: "A method of a class expression vs its renamed twin in a class declaration",
    files: {
      "session.ts": `
export const SessionCache = class {
  private entries = new Map<string, Session>();

  lookup(token: string) {
    const entry = this.entries.get(token);
    if (entry === undefined || entry.expires < Date.now()) {
      this.entries.delete(token);
      return undefined;
    }
    return entry;
  }
};
`,
      "token.ts": `
export class TokenCache {
  private entries = new Map<string, Token>();

  find(key: string) {
    const found = this.entries.get(key);
    if (found === undefined || found.expires < Date.now()) {
      this.entries.delete(key);
      return undefined;
    }
    return found;
  }
}
`,
    },
    expectPairs: [["SessionCache.lookup", "TokenCache.find"]],
  },

  // -------------------------------------------------------------------
  // types — call and construct signatures are members
  // -------------------------------------------------------------------
  {
    id: "XT-N26",
    mode: "types",
    category: "signatures",
    description: "Signature-only interfaces with similar names and different contracts",
    files: {
      "a.ts": `
export interface CssType {
  (strings: TemplateStringsArray, ...values: CssVariable[]): Promise<string>;
}

export interface CxType {
  (...names: (string | boolean | null | undefined)[]): string;
}
`,
    },
    forbidPairs: [["CssType", "CxType"]],
  },
  {
    id: "XT-N27",
    mode: "types",
    category: "signatures",
    description: "The getter and the setter interface of one store",
    files: {
      "a.ts": `
export interface GetVariable {
  (key: string): unknown;
}

export interface SetVariable {
  (key: string, value: unknown): void;
}
`,
    },
    forbidPairs: [["GetVariable", "SetVariable"]],
  },
  {
    id: "XT-N28",
    mode: "types",
    category: "signatures",
    description: "A callable shape is not a constructible one",
    files: {
      "a.ts": `
export interface ErrorMapper {
  (issue: Issue): Message;
}

export interface ErrorMaker {
  new (issue: Issue): Message;
}
`,
    },
    forbidPairs: [["ErrorMapper", "ErrorMaker"]],
  },
  {
    id: "XT-N29",
    mode: "types",
    category: "signatures",
    description: "An empty marker interface and a callable interface of the same name",
    files: {
      "a.ts": `
export interface Renderer {}
`,
      "b.ts": `
export interface Renderer {
  (content: string, head: HeadOptions): Response;
}
`,
    },
    forbidPairs: [["Renderer", "Renderer"]],
  },
  {
    id: "XT-P26",
    mode: "types",
    category: "signatures",
    description: "The same call signature under unrelated names and parameter names",
    files: {
      "a.ts": `
export interface RequestHandler {
  (request: IncomingRequest, context: RouteContext): Promise<OutgoingResponse>;
}
`,
      "b.ts": `
export interface Middleware {
  (req: IncomingRequest, ctx: RouteContext): Promise<OutgoingResponse>;
}
`,
    },
    expectPairs: [["RequestHandler", "Middleware"]],
  },
  // -------------------------------------------------------------------
  // types — members keyed by a number or a symbol
  // -------------------------------------------------------------------
  {
    id: "XT-N30",
    mode: "types",
    category: "member-keys",
    description: "Generated response maps keyed by status code with different payloads",
    files: {
      "a.ts": `
export type UserListResponses = {
  200: { result: { data: Array<User> } };
};

export type UserCreateResponses = {
  200: { result: { data: { name: string; id: string; createdAt: Date } } };
};
`,
    },
    forbidPairs: [["UserListResponses", "UserCreateResponses"]],
  },
  {
    id: "XT-P27",
    mode: "types",
    category: "member-keys",
    description: "Generated response maps keyed by status code with the same payloads",
    files: {
      "a.ts": `
export type OrderGetResponses = {
  200: { result: Order };
  404: { error: string };
  500: { error: string; trace: string };
};
`,
      "b.ts": `
export type OrderFetchResponses = {
  200: { result: Order };
  404: { error: string };
  500: { error: string; trace: string };
};
`,
    },
    expectPairs: [["OrderGetResponses", "OrderFetchResponses"]],
  },
  {
    id: "XT-N31",
    mode: "types",
    category: "member-keys",
    description: "Shapes keyed by different well-known symbols",
    files: {
      "a.ts": `
export interface Sequence {
  [Symbol.iterator]: () => Iterator<number>;
  length: number;
}

export interface Stream {
  [Symbol.asyncIterator]: () => AsyncIterator<number>;
  length: number;
}
`,
    },
    forbidPairs: [["Sequence", "Stream"]],
  },
  // -------------------------------------------------------------------
  // types — overloads, accessors, parameter lists
  // -------------------------------------------------------------------
  {
    id: "XT-N32",
    mode: "types",
    category: "overloads",
    description: "Factories that share only the last of their overloads",
    files: {
      "a.ts": `
export interface ElementFactory {
  create(tag: "canvas"): HTMLCanvasElement;
  create(tag: "video"): HTMLVideoElement;
  create(tag: string): HTMLElement;
}

export interface ContextFactory {
  create(kind: "2d"): CanvasRenderingContext2D;
  create(kind: "webgl"): WebGLRenderingContext;
  create(tag: string): HTMLElement;
}
`,
    },
    forbidPairs: [["ElementFactory", "ContextFactory"]],
  },
  {
    id: "XT-P28",
    mode: "types",
    category: "overloads",
    description: "Event interfaces with the same overloads",
    files: {
      "a.ts": `
export interface DocumentEvents {
  on(event: "open", listener: (document: TextDocument) => void): Disposable;
  on(event: "change", listener: (change: TextChange) => void): Disposable;
  on(event: "close", listener: (uri: string) => void): Disposable;
}
`,
      "b.ts": `
export interface EditorEvents {
  on(event: "open", listener: (document: TextDocument) => void): Disposable;
  on(event: "change", listener: (change: TextChange) => void): Disposable;
  on(event: "close", listener: (uri: string) => void): Disposable;
}
`,
    },
    expectPairs: [["DocumentEvents", "EditorEvents"]],
  },
  {
    id: "XT-P29",
    mode: "types",
    category: "accessors",
    description: "Accessor signatures and readonly properties of one contract",
    files: {
      "a.ts": `
export interface CacheStats {
  get size(): number;
  get hits(): number;
  get misses(): number;
  label: string;
}

export interface StoreStats {
  readonly size: number;
  readonly hits: number;
  readonly misses: number;
  label: string;
}
`,
    },
    expectPairs: [["CacheStats", "StoreStats"]],
  },
  {
    id: "XT-N33",
    mode: "types",
    category: "parameter-lists",
    description: "Variadic callbacks are not callbacks without parameters",
    files: {
      "a.ts": `
export interface Logger {
  log: (...parts: string[]) => void;
  warn: (...parts: string[]) => void;
  flush: (...sinks: Sink[]) => Promise<void>;
}

export interface Reporter {
  log: () => void;
  warn: () => void;
  flush: () => Promise<void>;
}
`,
    },
    forbidPairs: [["Logger", "Reporter"]],
  },
  {
    id: "XT-P30",
    mode: "types",
    category: "parameter-lists",
    description: "Callbacks that take a callback, with every parameter renamed",
    files: {
      "a.ts": `
export interface Scheduler {
  defer: (done: (error: Error | null) => void) => void;
  repeat: (tick: (count: number) => boolean, every: number) => Timer;
}
`,
      "b.ts": `
export interface Planner {
  defer: (callback: (failure: Error | null) => void) => void;
  repeat: (step: (n: number) => boolean, interval: number) => Timer;
}
`,
    },
    expectPairs: [["Scheduler", "Planner"]],
  },

  // -------------------------------------------------------------------
  // classes — method bodies that hold JSX
  // -------------------------------------------------------------------
  {
    id: "XC-N16",
    mode: "classes",
    category: "jsx",
    description: "Class components with the same members and different bodies",
    files: {
      "a.tsx": `
export class ProfilePanel extends Component<PanelProps> {
  state = { open: false };
  toggle() {
    this.setState({ open: !this.state.open });
  }
  render() {
    return (
      <section className="profile">
        <h2>{this.props.title}</h2>
        <Avatar user={this.props.user} />
      </section>
    );
  }
}

export class BillingPanel extends Component<PanelProps> {
  state = { open: false };
  toggle() {
    this.props.onToggle(this.state.open);
    this.props.track("billing");
  }
  render() {
    return (
      <table className="billing">
        <tbody>{this.props.rows.map((row) => <InvoiceRow key={row.id} row={row} />)}</tbody>
      </table>
    );
  }
}
`,
    },
    forbidPairs: [["ProfilePanel", "BillingPanel"]],
  },
  {
    id: "XC-P19",
    mode: "classes",
    category: "jsx",
    description: "Class components that are copies of each other",
    files: {
      "a.tsx": `
export class UserMenu extends Component<MenuProps> {
  state = { open: false };
  toggle() {
    this.setState({ open: !this.state.open });
  }
  render() {
    return (
      <nav className="menu">
        <button onClick={() => this.toggle()}>{this.props.label}</button>
        {this.state.open ? <MenuItems items={this.props.items} /> : null}
      </nav>
    );
  }
}
`,
      "b.tsx": `
export class TeamMenu extends Component<MenuProps> {
  state = { open: false };
  toggle() {
    this.setState({ open: !this.state.open });
  }
  render() {
    return (
      <nav className="menu">
        <button onClick={() => this.toggle()}>{this.props.label}</button>
        {this.state.open ? <MenuItems items={this.props.items} /> : null}
      </nav>
    );
  }
}
`,
    },
    expectPairs: [["UserMenu", "TeamMenu"]],
  },
  // -------------------------------------------------------------------
  // classes — a class expression bound to a name is a class
  // -------------------------------------------------------------------
  {
    id: "XC-P20",
    mode: "classes",
    category: "class-expression",
    description: "A class expression and the class declaration it is a copy of",
    files: {
      "session.ts": `
export const SessionCache = class {
  private entries = new Map<string, Session>();
  constructor(private readonly clock: Clock) {}
  lookup(token: string): Session | undefined {
    const entry = this.entries.get(token);
    return entry !== undefined && entry.expires > this.clock.now() ? entry : undefined;
  }
  store(token: string, session: Session): void {
    this.entries.set(token, session);
  }
};
`,
      "token.ts": `
export class TokenCache {
  private entries = new Map<string, Session>();
  constructor(private readonly clock: Clock) {}
  lookup(token: string): Session | undefined {
    const entry = this.entries.get(token);
    return entry !== undefined && entry.expires > this.clock.now() ? entry : undefined;
  }
  store(token: string, session: Session): void {
    this.entries.set(token, session);
  }
}
`,
    },
    expectPairs: [["SessionCache", "TokenCache"]],
  },
  {
    id: "XC-N17",
    mode: "classes",
    category: "class-expression",
    description: "A class expression next to a class of the same outline and another job",
    files: {
      "a.ts": `
export const RetryPolicy = class {
  attempts = 0;
  constructor(readonly limit: number) {}
  next(): number {
    this.attempts += 1;
    return Math.min(1000 * 2 ** this.attempts, 30000);
  }
  exhausted(): boolean {
    return this.attempts >= this.limit;
  }
};

export class RateWindow {
  hits = 0;
  constructor(readonly limit: number) {}
  record(at: Date): string {
    this.hits += 1;
    return at.toISOString() + "#" + this.hits;
  }
  reset(): void {
    this.hits = 0;
  }
}
`,
    },
    forbidPairs: [["RetryPolicy", "RateWindow"]],
  },
];
