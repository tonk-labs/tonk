// Hybrid Logical Clock — a monotonic, cross-node-comparable timestamp
// for ordering content writes.
//
// An HLC value packs a physical component (wall-clock millis) and a
// logical counter into one 64-bit integer (48 bits physical + 16 bits
// logical), so it total-orders by plain integer compare and doubles as
// an opaque version. 64 bits exceeds JS's safe integer range, so the
// value is a `bigint`.
//
// Why HLC rather than a bare timestamp or counter: it is monotonic by
// construction (each event's physical component is `max(last, now)`,
// never backwards), so a system-clock adjustment can't reorder writes,
// while still tracking real time closely enough to compare across
// nodes and break same-millisecond ties via the logical counter. That
// is exactly what lets the editor decide "is this incoming write newer
// than what I have?" — adopt when its HLC is greater, ignore when not.
//
// See https://sookocheff.com/post/time/hybrid-logical-clocks/ and the
// 48/16 packed encoding used by CockroachDB and others.

/** Bits reserved for the logical counter (low bits). The physical
 *  component occupies the remaining high bits. 16 bits lets a single
 *  node stamp 65 536 events within one physical millisecond before the
 *  counter would overflow into the next millisecond. */
const LOGICAL_BITS = 16n;
const LOGICAL_MASK = (1n << LOGICAL_BITS) - 1n;

/** Pack a physical-millis + logical pair into a single HLC integer. */
export function pack(physical: number, logical: number): bigint {
  return (BigInt(physical) << LOGICAL_BITS) | (BigInt(logical) & LOGICAL_MASK);
}

/** The physical (wall-clock millis) component of an HLC value. */
export function physicalOf(hlc: bigint): number {
  return Number(hlc >> LOGICAL_BITS);
}

/** The logical-counter component of an HLC value. */
export function logicalOf(hlc: bigint): number {
  return Number(hlc & LOGICAL_MASK);
}

/** A mutable clock. `last` holds the most recent HLC this node has
 *  issued or observed; `tick`/`receive` advance it monotonically. The
 *  physical source is injected (default `Date.now`) so the clock is
 *  testable and can run wherever a millisecond source is available. */
export class Clock {
  #last = 0n;
  readonly #now: () => number;

  constructor(now: () => number = () => Date.now()) {
    this.#now = now;
  }

  /** The most recent HLC value (0 before the first tick). */
  get last(): bigint {
    return this.#last;
  }

  /** Advance for a *local* event and return the new HLC. Physical is
   *  `max(lastPhysical, now)`; the logical counter increments when the
   *  physical component didn't move (same millisecond or a stalled /
   *  backwards clock) and resets to 0 when it did. */
  tick(): bigint {
    const now = this.#now();
    const lastPhysical = physicalOf(this.#last);
    const physical = Math.max(lastPhysical, now);
    const logical = physical === lastPhysical ? logicalOf(this.#last) + 1 : 0;
    this.#last = pack(physical, logical);
    return this.#last;
  }

  /** Merge a *remote* HLC observed on an incoming write, advancing the
   *  local clock so a later local tick is ordered after it. Physical
   *  becomes `max(local, remote, now)`; the logical counter follows
   *  the standard HLC receive rule (bump the matching side, or reset
   *  when physical advanced past both). No-op-safe: passing an older
   *  remote HLC leaves `last` at least where it was. */
  receive(remote: bigint): bigint {
    const now = this.#now();
    const lastP = physicalOf(this.#last);
    const remoteP = physicalOf(remote);
    const physical = Math.max(lastP, remoteP, now);
    let logical: number;
    if (physical === lastP && physical === remoteP) {
      logical = Math.max(logicalOf(this.#last), logicalOf(remote)) + 1;
    } else if (physical === lastP) {
      logical = logicalOf(this.#last) + 1;
    } else if (physical === remoteP) {
      logical = logicalOf(remote) + 1;
    } else {
      logical = 0;
    }
    this.#last = pack(physical, logical);
    return this.#last;
  }
}

/** Parse an HLC decimal string into a bigint, or null when it isn't a
 *  non-negative integer. */
export function parseHlc(text: string): bigint | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  try {
    return BigInt(trimmed);
  } catch {
    return null;
  }
}

/** Format an HLC value as its decimal string. */
export function formatHlc(hlc: bigint): string {
  return hlc.toString();
}
