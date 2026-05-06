import { useContext, useEffect, useRef, useState } from "react";
import { RepoContext } from "../context";
import { resolveName } from "../lib/resolveName";

export type PickPayload = {
  // The resolved entity DID. Always a `did:key:…` once committed
  // (the picker resolves bookmark names to DIDs server-side
  // before calling onPick), so Square never has to think about
  // resolution state.
  entity: string;
  // The user-typed label, kept on the square so the chrome can
  // show "hello-page" instead of the DID.
  name?: string;
  branch: string;
};

type Props = {
  initialEntity?: string;
  initialBranch?: string;
  onPick: (payload: PickPayload) => void;
  onClose: () => void;
};

export function EntityPicker({ initialEntity, initialBranch, onPick, onClose }: Props) {
  const repo = useContext(RepoContext);
  const [entity, setEntity] = useState(initialEntity ?? "");
  const [branch, setBranch] = useState(initialBranch ?? "main");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const submit = async () => {
    const trimmed = entity.trim();
    if (!trimmed || busy) return;
    const branchTrimmed = branch.trim() || "main";
    setError(null);

    if (trimmed.startsWith("did:")) {
      onPick({ entity: trimmed, branch: branchTrimmed });
      return;
    }

    setBusy(true);
    try {
      const resolved = await resolveName(repo, branchTrimmed, trimmed);
      onPick({ entity: resolved, name: trimmed, branch: branchTrimmed });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const paste = async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (text) setEntity(text.trim());
    } catch {
      // clipboard read can be denied; user can still type.
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      } else if (e.key === "Enter") {
        e.preventDefault();
        void submit();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // submit closes over entity/branch/repo/busy; rebind on change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entity, branch, repo, busy, onPick, onClose]);

  useEffect(() => {
    const onDocDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) onClose();
    };
    document.addEventListener("mousedown", onDocDown);
    return () => document.removeEventListener("mousedown", onDocDown);
  }, [onClose]);

  return (
    <div
      ref={rootRef}
      className="picker"
      onMouseDown={(e) => e.stopPropagation()}
    >
      <input
        ref={inputRef}
        className="picker__input"
        value={entity}
        onChange={(e) => {
          setEntity(e.target.value);
          if (error) setError(null);
        }}
        placeholder="did:key:… or bookmark name"
        disabled={busy}
      />
      <input
        className="picker__input picker__input--secondary"
        value={branch}
        onChange={(e) => setBranch(e.target.value)}
        placeholder="branch"
        disabled={busy}
      />
      {error && <div className="picker__error">{error}</div>}
      <div className="picker__actions">
        <button
          className="picker__action"
          onClick={paste}
          type="button"
          disabled={busy}
        >
          Paste
        </button>
        <button
          className="picker__action picker__action--primary"
          onClick={() => void submit()}
          type="button"
          disabled={!entity.trim() || busy}
        >
          {busy ? "Resolving…" : "Load"}
        </button>
      </div>
      <div className="picker__hint">
        <kbd>Enter</kbd> to load · <kbd>Esc</kbd> to cancel
      </div>
    </div>
  );
}
