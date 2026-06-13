// A TreeLoader backed by the tonk worker's `tree/*` query formulas.
//
// Each method POSTs a formula query to the branch's `/query` endpoint
// (predicate = a bare string like "tree/node"; terms carry the node
// hash) and maps the returned Conclusion rows to the component's shapes.
// This is the bridge from `tonk-tree` (which knows nothing about
// tonk) to a live branch.

import type { NodeHash, TreeEntry, TreeLoader, TreeNode } from "./types.js";

/** One Conclusion row: an entity URI plus a field map. */
interface Conclusion {
  this: string;
  fields: Record<string, unknown>;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}
function num(v: unknown): number | undefined {
  return typeof v === "number" ? v : undefined;
}

export interface WorkerLoaderOptions {
  /** Base URL of the worker, e.g. "" (same origin) or "http://…". */
  base?: string;
  /** Repository name. */
  repo: string;
  /** Branch name (default "main"). */
  branch?: string;
}

export class WorkerTreeLoader implements TreeLoader {
  #url: string;

  constructor(opts: WorkerLoaderOptions) {
    const base = opts.base ?? "";
    const branch = opts.branch ?? "main";
    this.#url = `${base}/api/repository/${encodeURIComponent(opts.repo)}/branch/${encodeURIComponent(branch)}/query`;
  }

  async #query(formula: string, terms: Record<string, unknown>): Promise<Conclusion[]> {
    const res = await fetch(this.#url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ predicate: formula, terms }),
    });
    if (!res.ok) {
      throw new Error(`${formula} → ${res.status} ${await res.text().catch(() => "")}`);
    }
    return (await res.json()) as Conclusion[];
  }

  #toNode(row: Conclusion): TreeNode {
    const f = row.fields;
    const kind = str(f.kind) === "segment" ? "segment" : "index";
    return {
      hash: str(f.child) ?? row.this,
      kind,
      size: num(f.size) ?? 0,
      count: num(f.count) ?? 0,
      bound: str(f.bound),
      at: num(f.at),
      cached: f.cached === false ? false : true,
    };
  }

  async root(): Promise<TreeNode | null> {
    const rows = await this.#query("tree/node", {});
    return rows.length ? this.#toNode(rows[0]) : null;
  }

  async children(hash: NodeHash): Promise<TreeNode[]> {
    const rows = await this.#query("tree/child", { hash });
    return rows.map((r) => this.#toNode(r));
  }

  async entries(hash: NodeHash): Promise<TreeEntry[]> {
    const rows = await this.#query("tree/entry", { hash });
    return rows.map((r) => {
      const f = r.fields;
      const value = f.value;
      return {
        key: str(f.key) ?? r.this,
        at: num(f.at) ?? 0,
        state: f.retracted === true ? "removed" : "added",
        entity: str(f.entity),
        attribute: str(f.attribute),
        type: str(f.type),
        value: value === undefined || value === null ? undefined : String(value),
      };
    });
  }
}
