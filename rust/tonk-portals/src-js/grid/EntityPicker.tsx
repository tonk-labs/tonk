import { useContext, useEffect, useMemo, useRef, useState } from "react";
import { RepoContext } from "../context";
import { resolveName } from "../lib/resolveName";
import { listArtifacts, type Artifact } from "../lib/artifacts";

export type PickPayload = {
  // The resolved entity DID. Always a `did:key:…` once committed
  // — the picker either receives it directly (the user pasted a
  // DID), or resolves it from a bookmark name before calling
  // onPick. Square never has to think about resolution state.
  entity: string;
  // The user-typed (or selected) label, kept on the square so
  // the chrome can show "hello-page" instead of the DID.
  name?: string;
};

type Props = {
  initialEntity?: string;
  onPick: (payload: PickPayload) => void;
  onClose: () => void;
};

// Hard-coded for now. The picker doesn't expose branch as a
// concept — the empty-tile flow always lands on `main`, and
// cross-branch navigation happens via the iframe's postMessage
// bridge (see Grid's `tonk:navigate` handler).
const DEFAULT_BRANCH = "main";

// Cap the suggestion list. Past this the list reads as a wall of
// rows and arrow-keying through it is annoying. The user can
// always narrow the list by typing more characters.
const MAX_SUGGESTIONS = 8;

export function EntityPicker({ initialEntity, onPick, onClose }: Props) {
  const repo = useContext(RepoContext);
  const [query, setQuery] = useState(initialEntity ?? "");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [artifacts, setArtifacts] = useState<Artifact[]>([]);
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  useEffect(() => {
    if (!repo) return;
    let cancelled = false;
    listArtifacts(repo, DEFAULT_BRANCH)
      .then((as) => {
        if (!cancelled) setArtifacts(as);
      })
      .catch(() => {
        // A failed list shouldn't break the picker — the user can
        // still paste a DID or type a name and fall back to the
        // resolve route. Silently leave `artifacts` empty.
      });
    return () => {
      cancelled = true;
    };
  }, [repo]);

  const trimmed = query.trim();
  const looksLikeDid = trimmed.startsWith("did:");

  const suggestions = useMemo<Artifact[]>(() => {
    if (looksLikeDid) return [];
    const q = trimmed.toLowerCase();
    const filtered = q
      ? artifacts.filter(
          (a) =>
            a.name.toLowerCase().includes(q) ||
            a.entity.toLowerCase().includes(q),
        )
      : artifacts;
    return filtered.slice(0, MAX_SUGGESTIONS);
  }, [trimmed, looksLikeDid, artifacts]);

  // Keep the highlight valid as the suggestion list changes.
  // Without this clamp, typing a character that shrinks the list
  // can leave `highlight` past the end and Enter does nothing.
  useEffect(() => {
    if (highlight >= suggestions.length) setHighlight(0);
  }, [highlight, suggestions.length]);

  const submit = async () => {
    if (busy) return;
    // If a suggestion is visible and highlighted, picking it wins
    // over re-resolving the typed text — the user has already
    // chosen.
    const picked = suggestions[highlight];
    if (picked && !looksLikeDid) {
      onPick({ entity: picked.entity, name: picked.name });
      return;
    }

    if (!trimmed) return;
    setError(null);

    if (looksLikeDid) {
      onPick({ entity: trimmed });
      return;
    }

    // Typed a name that wasn't in the suggestion list — fall back
    // to the worker's `/resolve/{name}` route. Covers the case
    // where the bookmark exists on the branch but our cache hasn't
    // caught up, or the list fetch failed silently.
    setBusy(true);
    try {
      const resolved = await resolveName(repo, DEFAULT_BRANCH, trimmed);
      onPick({ entity: resolved, name: trimmed });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const pickSuggestion = (a: Artifact) => {
    onPick({ entity: a.entity, name: a.name });
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        void submit();
        return;
      }
      if (e.key === "ArrowDown" && suggestions.length) {
        e.preventDefault();
        setHighlight((h) => (h + 1) % suggestions.length);
        return;
      }
      if (e.key === "ArrowUp" && suggestions.length) {
        e.preventDefault();
        setHighlight((h) => (h - 1 + suggestions.length) % suggestions.length);
        return;
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // submit closes over query/highlight/suggestions/busy/repo;
    // rebind on each change so the keydown handler always sees
    // current state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, highlight, suggestions, busy, repo, onPick, onClose]);

  useEffect(() => {
    const onDocDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) onClose();
    };
    document.addEventListener("mousedown", onDocDown);
    return () => document.removeEventListener("mousedown", onDocDown);
  }, [onClose]);

  const showLoadButton = looksLikeDid || suggestions.length === 0;

  return (
    <div
      ref={rootRef}
      className="picker"
      onMouseDown={(e) => e.stopPropagation()}
    >
      <input
        ref={inputRef}
        className="picker__input"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          if (error) setError(null);
        }}
        placeholder="search artifacts or paste did:key:…"
        disabled={busy}
      />
      {error && <div className="picker__error">{error}</div>}
      {!looksLikeDid && suggestions.length > 0 && (
        <ul className="picker__list" role="listbox">
          {suggestions.map((a, i) => (
            <li
              key={a.entity}
              role="option"
              aria-selected={i === highlight}
              className={`picker__item${i === highlight ? " picker__item--highlighted" : ""}`}
              onMouseEnter={() => setHighlight(i)}
              onMouseDown={(e) => {
                // mousedown so we beat the document-level
                // mousedown that would otherwise close the picker
                // before the click resolves.
                e.preventDefault();
                pickSuggestion(a);
              }}
            >
              <span className="picker__item-name">{a.name}</span>
              <span className="picker__item-entity">{a.entity}</span>
            </li>
          ))}
        </ul>
      )}
      {!looksLikeDid && trimmed && suggestions.length === 0 && artifacts.length > 0 && (
        <div className="picker__empty">No artifact matches "{trimmed}"</div>
      )}
      {showLoadButton && (
        <div className="picker__actions">
          <button
            className="picker__action picker__action--primary"
            onClick={() => void submit()}
            type="button"
            disabled={!trimmed || busy}
          >
            {busy ? "Resolving…" : "Load"}
          </button>
        </div>
      )}
    </div>
  );
}
