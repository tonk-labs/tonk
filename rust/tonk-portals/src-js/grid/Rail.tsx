import type { Square } from "../types";
import { colorForArtifact } from "../lib/color";

type Props = {
  squares: Square[];
  onRestore: (id: string) => void;
};

function labelFor(sq: Square): string {
  return sq.name ?? sq.entity ?? "Empty";
}

export function Rail({ squares, onRestore }: Props) {
  if (squares.length === 0) return null;
  return (
    <div className="rail">
      {squares.map((sq) => {
        const label = labelFor(sq);
        const color = colorForArtifact(sq);
        return (
          <button
            key={sq.id}
            className="rail__tab"
            onClick={() => onRestore(sq.id)}
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
