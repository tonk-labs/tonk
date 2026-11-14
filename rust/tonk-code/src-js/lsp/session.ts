// One language-server session per endpoint, shared by every library
// `<code-editor>` in this realm.
//
// The library's `<code-editor>` is branch data with a CodeMirror of its
// own, so it cannot take a client built against another
// `@codemirror/state`: such an extension is rejected ("Unrecognized
// extension value"). This module runs the session lifecycle (lazy
// connect once the service worker controls the page, rebuild on a
// closed stream, hold for `controllerchange` while a worker retires),
// so the core can hand an editor a client of its own.

import type { LSPClient } from "@codemirror/lsp-client";
import { connectLsp } from "./client";
import { httpTransport } from "./transport";

/** An editor a session hands its client to. */
export type Attachable = {
  connect(client: LSPClient): void;
  disconnect(): void;
};

/** The language server at one endpoint, and the editors on it. */
export type Session = {
  /** Hand `editor` the client for `source`, now or once it is up. */
  attach(source: string, editor: Attachable): void;
  /** Forget `source`. The editor detaches itself. */
  detach(source: string): void;
};

const RECONNECT_MS = 5_000;
const RECONNECT_MAX_MS = 30_000;
/** Fallback for a `controllerchange` that never arrives while an update
 *  is pending. Long on purpose: a short one would be a timed redial in
 *  disguise, pinning the retiring worker. */
const UPDATE_HOLD_MS = 30_000;

const sessions = new Map<string, Session>();

/** The session for `url`, opened on first use. */
export function languageServer(url: string): Session {
  let session = sessions.get(url);
  if (!session) {
    session = open(url);
    sessions.set(url, session);
  }
  return session;
}

function open(url: string): Session {
  const attached = new Map<string, Attachable>();
  let client: LSPClient | null = null;
  let generation = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectDelay = RECONNECT_MS;
  let updatePending = false;
  let cancelHold: (() => void) | null = null;

  const tearDown = () => {
    generation++;
    cancelHold?.();
    cancelHold = null;
    if (reconnectTimer !== null) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    for (const editor of attached.values()) {
      try {
        editor.disconnect();
      } catch {
        // The editor may have torn itself down already.
      }
    }
    client?.disconnect();
    client = null;
  };

  const ensureClient = () => {
    if (client || attached.size === 0) return;
    const owner = ++generation;
    void whenWorkerReady().then(() => {
      if (owner !== generation || client || attached.size === 0) return;
      const transport = httpTransport({
        url,
        onClose: (reason) => {
          if (owner !== generation) return;
          updatePending = reason.updatePending;
          tearDown();
          scheduleReconnect();
        },
      });
      client = connectLsp(transport);
      reconnectDelay = RECONNECT_MS;
      for (const editor of attached.values()) {
        try {
          editor.connect(client);
        } catch {
          // One bad editor must not keep the rest off the server.
        }
      }
    });
  };

  const scheduleReconnect = () => {
    if (reconnectTimer !== null || attached.size === 0) return;
    const container = serviceWorker();
    if (updatePending && container) {
      updatePending = false;
      const owner = generation;
      const resume = () => {
        cancelHold?.();
        cancelHold = null;
        if (reconnectTimer !== null) {
          clearTimeout(reconnectTimer);
          reconnectTimer = null;
        }
        if (owner !== generation) return;
        reconnectDelay = RECONNECT_MS;
        ensureClient();
      };
      container.addEventListener("controllerchange", resume);
      cancelHold = () => container.removeEventListener("controllerchange", resume);
      reconnectTimer = setTimeout(resume, UPDATE_HOLD_MS);
      return;
    }
    updatePending = false;
    const delay = reconnectDelay;
    reconnectDelay = Math.min(delay * 2, RECONNECT_MAX_MS);
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      ensureClient();
    }, delay);
  };

  return {
    attach(source, editor) {
      attached.set(source, editor);
      if (client) editor.connect(client);
      else ensureClient();
    },
    detach(source) {
      attached.delete(source);
    },
  };
}

/** `navigator.serviceWorker`, or null where reading it throws: a sealed
 *  (opaque-origin) frame has none of its own, and its `fetch` reaches the
 *  host's worker through the portal bridge anyway. */
function serviceWorker(): ServiceWorkerContainer | null {
  try {
    return navigator.serviceWorker ?? null;
  } catch {
    return null;
  }
}

/** Resolve once requests reach the worker's router rather than the static
 *  fallback, so `initialize` never lands in the activation window. Bounded
 *  at every step: where no worker will ever control the page, the
 *  transport still gets its chance. */
async function whenWorkerReady(): Promise<void> {
  const container = serviceWorker();
  if (!container) return;
  try {
    const ready = await Promise.race([
      container.ready.then(() => true),
      new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 2_000)),
    ]);
    if (!ready || container.controller) return;
    await new Promise<void>((resolve) => {
      const done = () => {
        container.removeEventListener("controllerchange", done);
        clearTimeout(timer);
        resolve();
      };
      const timer = setTimeout(done, 5_000);
      container.addEventListener("controllerchange", done);
    });
  } catch {
    // Worker machinery unavailable: let the transport try.
  }
}
