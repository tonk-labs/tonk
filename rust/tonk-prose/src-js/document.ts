// Document mode: the element's half of the automerge document protocol.
//
// The element holds NO automerge. The host owns the document; the
// element owns a piece of content and the HEADS it last saw, and
// exchanges both:
//
//   read  -> { heads, content }                 the branch's version
//   write -> send { heads: known, content }     "my content, made on
//                                               top of these heads"
//         <- { local, heads, content }          `local` = the heads my
//                                               content now IS;
//                                               `heads`/`content` = the
//                                               branch after merging
//
// The host applies a write ON TOP OF the heads it names, so a write
// made from a stale view merges instead of overwriting. Two rules keep
// the editor and the branch from fighting:
//
//   1. Remote content is only ever applied when the editor's content is
//      exactly what the host last accounted for. If the user typed
//      during a round trip, the reply's content is dropped and only
//      `local` is kept: the next write then starts from the heads that
//      the previously sent content corresponds to, so nothing is applied
//      twice and nothing is lost.
//   2. One write is in flight at a time. Edits made meanwhile ride the
//      next one.
//
// Pure logic: the transport and the editor are injected, so the node
// tests cover every interleaving. The same file is copied into
// tonk-table, the way `hlc.ts` and `content.ts` are.

/** What the host answers. `T` is the content: markdown for a text
 *  document, a workbook for a table. */
export interface Reply<T> {
  /** The heads the branch is at. */
  heads: string[];
  /** After a write: the heads the content I sent corresponds to. */
  local?: string[];
  /** The content at `heads`. */
  content: T;
  /** The format rule: the document is in a format newer than this app
   *  knows. It can be shown and must not be edited. */
  readonly?: boolean;
  /** After a write: the stored document passed the size at which the
   *  host asks for a warning. Past the hard limit a write is refused. */
  large?: boolean;
}

/** How the session reaches the host. */
export interface Transport<T, E> {
  read(): Promise<Reply<T>>;
  write(heads: string[], edits: E[], request?: EditRequest): Promise<Reply<T>>;
}

export interface EditRequest { id: string; time: number }
function requestIdentity(): EditRequest {
  return { id: crypto.randomUUID(), time: Math.floor(Date.now() / 1000) };
}

/** What the session needs from its editor. */
export interface Editor<T, E> {
  /** The editor's content right now. */
  current(): T;
  /** Replace the editor's content with remote content. */
  apply(content: T): void;
  /** Whether two contents are the same. */
  same(a: T, b: T): boolean;
  /** The edits that turn `from` into `to`. */
  edits(from: T, to: T): E[];
}

export class DocumentSession<T, E> {
  readonly #transport: Transport<T, E>;
  readonly #editor: Editor<T, E>;

  /** The heads the content in `#accounted` corresponds to. */
  #known: string[] = [];
  /** The content the host last accounted for: what a write diffs
   *  against, and the only state remote content may replace. */
  #accounted: T | null = null;
  #inFlight = false;
  #flight: Promise<void> | null = null;
  #pending: { heads: string[]; edits: E[]; content: T; request: EditRequest } | null = null;
  #final: { content: T } | null = null;
  #again = false;
  #closed = false;
  /** Pinned to a past version: opened once, then neither written nor
   *  polled. The transport reads that version; the session only makes
   *  sure nothing the editor holds can leave. */
  readonly #pinned: boolean;
  readonly #onOpen: (() => void) | undefined;
  readonly #onLarge: (() => void) | undefined;
  #large = false;
  /** The host said the document is in a newer format. */
  #newer = false;

  constructor(
    transport: Transport<T, E>,
    editor: Editor<T, E>,
    options: { pinned?: boolean; onOpen?: () => void; onLarge?: () => void } = {},
  ) {
    this.#transport = transport;
    this.#editor = editor;
    this.#pinned = options.pinned === true;
    this.#onOpen = options.onOpen;
    this.#onLarge = options.onLarge;
  }

  get pinned(): boolean {
    return this.#pinned;
  }

  /** Whether the document takes no edits from this element: a past
   *  version, or a format newer than this app knows. */
  get readonly(): boolean {
    return this.#pinned || this.#newer;
  }

  /** The heads this element is at. */
  get heads(): readonly string[] {
    return this.#known;
  }

  get opened(): boolean {
    return this.#accounted !== null;
  }

  /** Load the branch's version into the editor. */
  async open(): Promise<void> {
    const reply = await this.#transport.read();
    if (this.#closed || this.opened) return;
    this.#known = reply.heads;
    this.#accounted = reply.content;
    this.#newer = reply.readonly === true;
    this.#editor.apply(reply.content);
    this.#onOpen?.();
  }

  /** Whether the editor holds edits the host has not accounted for. */
  get dirty(): boolean {
    return (
      this.#pending !== null || (this.#accounted !== null &&
      !this.#editor.same(this.#current(), this.#accounted))
    );
  }

  /** Send the editor's unsent edits. Safe to call at any time and any
   *  number of times; concurrent calls coalesce. */
  flush(): Promise<void> {
    if (this.#flight) { this.#again = true; return this.#flight; }
    this.#flight = this.#flush().finally(() => { this.#flight = null; });
    return this.#flight;
  }

  #current(): T { return this.#final ? this.#final.content : this.#editor.current(); }

  async #flush(): Promise<void> {
    if (this.#closed || this.readonly || this.#accounted === null) return;
    if (this.#inFlight) {
      this.#again = true;
      return;
    }
    const current = this.#current();
    if (!this.#pending && this.#editor.same(current, this.#accounted)) return;
    const pending = this.#pending ??= { heads: [...this.#known], content: current,
      edits: this.#editor.edits(this.#accounted, current), request: requestIdentity() };
    const sent = pending.content;

    this.#inFlight = true;
    try {
      const reply = await this.#transport.write(pending.heads, pending.edits, pending.request);
      if (this.#closed) return;
      this.#pending = null;
      if (reply.large === true && !this.#large) {
        // Said once: automerge keeps every edit, so it only gets larger.
        this.#large = true;
        this.#onLarge?.();
      }
      if (this.#editor.same(this.#current(), sent)) {
        // Nothing typed meanwhile: take the merged branch state.
        this.#known = reply.heads;
        this.#accounted = reply.content;
        if (this.#final) this.#final.content = reply.content;
        if (!this.#final && !this.#editor.same(reply.content, sent)) {
          this.#editor.apply(reply.content);
        }
      } else {
        // Typed during the round trip: keep the editor as it is. The
        // content we sent now IS `local`, so the next write diffs from
        // it and starts at those heads.
        this.#known = reply.local ?? reply.heads;
        this.#accounted = sent;
        this.#again = true;
      }
    } finally {
      this.#inFlight = false;
    }
    if (this.#again) {
      this.#again = false;
      await this.#flush();
    }
  }

  /** Look for changes made elsewhere. Skipped while the editor holds
   *  unsent edits — those go first, and their reply brings the rest. */
  async poll(): Promise<void> {
    if (this.#closed) return;
    // The first read failed — the heads can arrive before their bytes,
    // and the host may not be up yet. Try again; until it works the
    // element shows nothing and takes no edits.
    if (this.#accounted === null) return this.open();
    if (this.#pinned) return;
    if (this.#inFlight || this.dirty) return;
    const known = this.#known;
    const reply = await this.#transport.read();
    if (this.#closed || this.#inFlight || this.dirty || this.#known !== known) return;
    if (sameHeads(reply.heads, this.#known)) return;
    this.#known = reply.heads;
    this.#accounted = reply.content;
    this.#editor.apply(reply.content);
  }

  close(): void {
    this.#closed = true;
  }

  /** Snapshot final text before the editor is destroyed; drain a write
   * already in flight AND the keystrokes made since that write. */
  async finish(): Promise<void> {
    this.#final = { content: this.#editor.current() };
    await this.flush();
    this.close();
  }
}

/** The heads an `at` attribute names: change hashes separated by spaces. */
export function parseHeads(text: string | null): string[] {
  return (text ?? "").split(/\s+/).filter((head) => head !== "");
}

export function sameHeads(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  const left = [...a].sort();
  const right = [...b].sort();
  return left.every((head, index) => head === right[index]);
}

/** A wire edit of a text document. */
export type TextEdit = { edit: "set-text"; text: string };

/** The editor half for a text document. */
export function textEditor(
  current: () => string,
  apply: (text: string) => void,
): Editor<string, TextEdit> {
  return {
    current,
    apply,
    same: (a, b) => a === b,
    // The whole text: the host diffs it against the text at the heads
    // the write names, which is what the element last accounted for.
    edits: (_from, to) => [{ edit: "set-text", text: to }],
  };
}

/** Reach the host through a bubbling `tonk-document` event, the way
 *  every tonk element reaches it: the host (or, in a sealed guest, its
 *  relay) resolves the repository and branch from the element's routing
 *  context and answers on `detail.result`. */
export function eventTransport<T>(
  element: HTMLElement,
  entity: string,
  format: string,
  content: (body: Record<string, unknown>) => T,
  at: string[] = [],
): Transport<T, unknown> {
  let route: Record<string, unknown> = {};
  const call = async (write?: { heads: string[]; edits: unknown[]; request?: EditRequest }): Promise<Reply<T>> => {
    const detail: Record<string, unknown> = { ...(element.isConnected ? {} : route), entity, format };
    if (write) detail.write = { ...write, format };
    // A past version: the host reads at these heads instead of the branch's.
    else if (at.length > 0) detail.heads = at;
    const event = new CustomEvent("tonk-document", {
      detail,
      bubbles: true,
      composed: true,
      cancelable: true,
    });
    (element.isConnected ? element : element.ownerDocument).dispatchEvent(event);
    if (!event.defaultPrevented || !(detail.result instanceof Promise)) {
      throw new Error("tonk-document: no host answered");
    }
    route = { space: detail.space, branch: detail.branch, profile: detail.profile };
    const body = (await detail.result) as Record<string, unknown>;
    return {
      heads: (body.heads as string[]) ?? [],
      local: body.local as string[] | undefined,
      content: content(body),
      readonly: body.readonly === true,
      large: body.large === true,
    };
  };
  return {
    read: () => call(),
    write: (heads, edits, request) => call({ heads, edits, request }),
  };
}
