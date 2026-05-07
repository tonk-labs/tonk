import type { Zone } from "../lib/zones";

type Props = {
  zones: Zone[];
  hoveredId: string | null;
  onEnter: (zone: Zone) => void;
  onLeave: () => void;
  onCommit: (zone: Zone) => void;
};

// Renders the four edge rails as a flat list of positioned hit
// zones. Visual state is driven by `hoveredId` so the same zone
// the parent uses to compute the preview layout is the one we
// highlight.
export function EdgeRails({ zones, hoveredId, onEnter, onLeave, onCommit }: Props) {
  return (
    <div className="edge-rails">
      {zones.map((z) => (
        <button
          key={z.id}
          type="button"
          className={`edge-zone edge-zone--${z.edge}${z.id === hoveredId ? " edge-zone--hot" : ""}`}
          style={{
            transform: `translate(${z.rect.x}px, ${z.rect.y}px)`,
            width: z.rect.w,
            height: z.rect.h,
          }}
          onMouseEnter={() => onEnter(z)}
          onMouseLeave={onLeave}
          onClick={(e) => {
            e.stopPropagation();
            onCommit(z);
          }}
          aria-label={`insert ${z.edge}`}
        />
      ))}
    </div>
  );
}
