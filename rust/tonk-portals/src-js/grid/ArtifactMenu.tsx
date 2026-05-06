import { useEffect, useRef, useState } from "react";

// Right-side bar dropdown. Same shell as the prototype; the
// actions are stubs for now ("link copied" flashes) and will get
// wired to copy share URLs once portals have a stable address.
export function ArtifactMenu() {
  const [open, setOpen] = useState(false);
  const [flash, setFlash] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDocDown = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDocDown);
    return () => document.removeEventListener("mousedown", onDocDown);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open]);

  useEffect(() => {
    if (!flash) return;
    const t = setTimeout(() => setFlash(null), 1400);
    return () => clearTimeout(t);
  }, [flash]);

  const action = (label: string) => {
    setOpen(false);
    setFlash(label);
  };

  return (
    <div
      className="bar-menu"
      ref={wrapRef}
      onMouseDown={(e) => e.stopPropagation()}
      onPointerDown={(e) => e.stopPropagation()}
    >
      <button
        className="bar-menu__btn"
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
        aria-label="menu"
      >
        <span className="bar-menu__line" />
        <span className="bar-menu__line" />
        <span className="bar-menu__line" />
      </button>
      {open && (
        <div className="bar-menu__dropdown">
          <button className="bar-menu__item" onClick={() => action("edit link copied")}>
            Edit artifact
          </button>
          <button className="bar-menu__item" onClick={() => action("share link copied")}>
            Share artifact
          </button>
        </div>
      )}
      {flash && <div className="bar-menu__flash">{flash}</div>}
    </div>
  );
}
