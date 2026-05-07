import type { Leaf } from "../lib/layout";
import { colorForArtifact } from "../lib/color";

type Props = {
  leaves: Leaf[];
  onRestore: (id: string) => void;
};

function labelFor(leaf: Leaf): string {
  return leaf.name ?? leaf.entity ?? "Empty";
}

// The bottom tab strip for minimized tiles. Sibling of the grid
// stage, so it sits below the active layout regardless of how the
// tree is split. Clicking a tab pops the tile back into the tree.
export function Rail({ leaves, onRestore }: Props) {
  if (leaves.length === 0) return null;
  return (
    <div className="rail">
      {leaves.map((leaf) => {
        const label = labelFor(leaf);
        const color = colorForArtifact(leaf);
        return (
          <button
            key={leaf.id}
            className="rail__tab"
            onClick={() => onRestore(leaf.id)}
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
