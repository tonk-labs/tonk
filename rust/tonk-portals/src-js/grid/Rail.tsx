import type { Tile } from "../lib/grid";
import { colorForArtifact } from "../lib/color";

type Props = {
  tiles: Tile[];
  onRestore: (id: string) => void;
};

function labelFor(tile: Tile): string {
  return tile.name ?? tile.entity ?? "Empty";
}

export function Rail({ tiles, onRestore }: Props) {
  if (tiles.length === 0) return null;
  return (
    <div className="rail">
      {tiles.map((tile) => {
        const label = labelFor(tile);
        const color = colorForArtifact(tile);
        return (
          <button
            key={tile.id}
            className="rail__tab"
            onClick={() => onRestore(tile.id)}
            title={label}
            style={color ? { background: color.full } : undefined}
          >
            <span className="rail__tab-label">{label}</span>
          </button>
        );
      })}
    </div>
  );
}
