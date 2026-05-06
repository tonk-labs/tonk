// Wire contract for messages an artifact iframe can send to the
// portals shell.
//
// Artifacts post messages with `parent.postMessage(msg, '*')`. The
// shell validates `event.origin` against its own origin (sandboxed
// iframes carry `allow-same-origin`, so they post under the shell's
// real origin) and dispatches by `type`.
//
// The contract is intentionally explicit and `tonk:`-prefixed so it
// can coexist with messages from other framework code on the same
// page without false positives.

/// Ask the shell to load a different artifact into the tile that
/// sent the message. Either `entity` (a `did:key:…` DID) or `name`
/// (a bookmark) must be set; if both are present `entity` wins.
/// `branch` is optional and defaults to the tile's current branch.
export type NavigateMessage = {
  type: "tonk:navigate";
  entity?: string;
  name?: string;
  branch?: string;
};

/// Ask the shell to close the tile that sent the message. Same
/// behaviour as the user clicking the tile's `×` chrome button.
export type CloseMessage = {
  type: "tonk:close";
};

export type TileMessage = NavigateMessage | CloseMessage;

export function isTileMessage(value: unknown): value is TileMessage {
  if (!value || typeof value !== "object") return false;
  const t = (value as { type?: unknown }).type;
  return typeof t === "string" && t.startsWith("tonk:");
}

/// Marker attribute the shell stamps on each tile wrapper so the
/// message listener can map a posting iframe back to its owning
/// square. Kept in this module so the shell side and any test code
/// reach for the same constant.
export const SQUARE_ID_ATTR = "data-square-id";
