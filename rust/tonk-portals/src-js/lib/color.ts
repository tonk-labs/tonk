export type ArtifactColor = { full: string; soft: string };

const PALETTE = ["#c4ddd6", "#d4ddd6", "#e4ddd6", "#e4e3cd", "#ececdd"];

function hashStr(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0;
  }
  return Math.abs(h);
}

export function colorForArtifact(input: {
  entity?: string;
  name?: string;
}): ArtifactColor | undefined {
  const seed = input.entity ?? input.name;
  if (!seed) return undefined;
  const color = PALETTE[hashStr(seed) % PALETTE.length];
  return {
    full: color,
    soft: color + "99",
  };
}
