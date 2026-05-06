// A Square (artifact tile) carries enough identity to compose its
// own data URL: an `entity` DID and a `branch` (defaulting to
// "main"). The optional `name` is the user-typed label before
// resolution — once we wire bookmark→DID lookup (task 6), the
// name is what the user types and the entity is what gets
// resolved. The element only owns `repo` + `host`; everything
// else lives per-tile.
export type Square = {
  id: string;
  w: number;
  h: number;
  col: number;
  row: number;
  minimized: boolean;
  entity?: string;
  branch?: string;
  name?: string;
};

export type PixelRect = {
  x: number;
  y: number;
  w: number;
  h: number;
};
