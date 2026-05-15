export type ArtifactColor = { full: string; soft: string };

// tldraw's solid light-mode palette (minus black + white).
// Same set the Leptos shell exposes as `--tl-*` tokens.
const PALETTE = [
  "#4465e9", // blue
  "#099268", // green
  "#e16919", // orange
  "#ae3ec9", // violet
  "#f1ac4b", // yellow
  "#4ba1f1", // light-blue
  "#4cb05e", // light-green
  "#e03131", // red
  "#e085f4", // light-violet
  "#f87777", // light-red
  "#9fa8b2", // grey
];

// First-come-first-served, *per-tile* assignment: each tile id
// gets its own colour, so the first PALETTE.length tiles cycle
// through the entire palette before any colour repeats. Keying
// off `entity`/`name` (the previous approach) collapsed multiple
// tiles of the same artifact to one colour, which broke the
// "no duplicates until we wrap" guarantee the moment the user
// duplicated an artifact.
const assigned = new Map<string, string>();

function pickColor(seed: string): string {
  const cached = assigned.get(seed);
  if (cached) return cached;
  const next = PALETTE[assigned.size % PALETTE.length]!;
  assigned.set(seed, next);
  return next;
}

export function colorForArtifact(input: {
  id?: string;
  entity?: string;
  name?: string;
}): ArtifactColor | undefined {
  const seed = input.id ?? input.entity ?? input.name;
  if (!seed) return undefined;
  const color = pickColor(seed);
  return {
    full: color,
    soft: color + "99",
  };
}
