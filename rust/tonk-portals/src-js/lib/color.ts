import type { Square } from "../types";

export type ArtifactColor = { full: string; soft: string };

function hashStr(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0;
  }
  return Math.abs(h);
}

// Seed off whichever artifact identity the tile carries: a typed
// entity DID once chosen, otherwise the user-typed name (so a tile
// keyed by a not-yet-resolved name still gets a stable hue).
function seedFor(sq: Square): string | undefined {
  return sq.entity ?? sq.name;
}

// Return artifact colors as hsl/hsla so they layer on top of
// whatever the host theme paints — important on dark themes,
// where a literal pastel `#fbe3ff` would whiteout the bar and
// hide the chrome icons (the original prototype assumed a light
// host). `full` is the saturated mid-tone for rail-tab fills;
// `soft` is a translucent tint for the bar that lets the square
// background show through, so button colors keep their contrast.
export function colorForArtifact(sq: Square): ArtifactColor | undefined {
  const seed = seedFor(sq);
  if (!seed) return undefined;
  const hue = hashStr(seed) % 360;
  return {
    full: `hsl(${hue} 55% 60%)`,
    soft: `hsla(${hue}, 70%, 55%, 0.18)`,
  };
}
