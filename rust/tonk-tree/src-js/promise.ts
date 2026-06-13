// Loading state for async data, mirroring dialog-diagnose's `Promise`
// enum: a value is either Resolved or still Pending. Lets a view render
// "loading…" without juggling nulls.

export type Loadable<T> =
  | { readonly status: "pending" }
  | { readonly status: "resolved"; readonly value: T };

export const Pending: Loadable<never> = { status: "pending" };

export function Resolved<T>(value: T): Loadable<T> {
  return { status: "resolved", value };
}

export function isResolved<T>(p: Loadable<T>): p is { status: "resolved"; value: T } {
  return p.status === "resolved";
}
