import { useCallback, useEffect, useState } from "react";
import { flushSync } from "react-dom";
import { Grid } from "./grid/Grid";
import { HostContext, RepoContext, ViewModeContext, type ViewMode } from "./context";

// View Transitions API isn't typed in @types/react / lib.dom on older
// targets; declare what we need.
type DocumentWithVT = Document & {
  startViewTransition?: (cb: () => void) => unknown;
};

export type AppProps = {
  repo: string;
  host: string;
};

const SYNC_INTERVAL_MS = 3000;
const SYNC_BRANCH = "main";
const MOBILE_MEDIA_QUERY = "(max-width: 768px)";

// Poll `POST /sync` (pull-then-push) on the repo's main branch
// while the portals shell is mounted. One request in flight at a
// time; the timer skips a tick rather than overlapping. Pauses
// while the tab is hidden so a backgrounded portal doesn't spin
// the upstream.
function useBackgroundSync(repo: string) {
  useEffect(() => {
    if (!repo) return;
    let inflight = false;
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | null = null;

    const url = `/api/repository/${encodeURIComponent(repo)}/branch/${encodeURIComponent(
      SYNC_BRANCH,
    )}/sync`;

    const tick = async () => {
      if (cancelled || inflight) return;
      if (typeof document !== "undefined" && document.hidden) return;
      inflight = true;
      try {
        await fetch(url, { method: "POST" });
      } catch (err) {
        console.warn("[tonk-portals] background sync failed:", err);
      } finally {
        inflight = false;
      }
    };

    tick();
    timer = setInterval(tick, SYNC_INTERVAL_MS);

    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, [repo]);
}

function initialViewMode(): ViewMode {
  if (typeof window === "undefined") return "canvas";
  return window.matchMedia(MOBILE_MEDIA_QUERY).matches ? "doc" : "canvas";
}

function ViewModeSwitcher({
  mode,
  onChange,
}: {
  mode: ViewMode;
  onChange: (m: ViewMode) => void;
}) {
  return (
    <div className="tp-header">
      <div className="tp-mode-switch" role="tablist" aria-label="view mode">
        <button
          role="tab"
          aria-selected={mode === "canvas"}
          className={`tp-mode-switch__btn${mode === "canvas" ? " tp-mode-switch__btn--active" : ""}`}
          onClick={() => onChange("canvas")}
        >
          Canvas
        </button>
        <button
          role="tab"
          aria-selected={mode === "doc"}
          className={`tp-mode-switch__btn${mode === "doc" ? " tp-mode-switch__btn--active" : ""}`}
          onClick={() => onChange("doc")}
        >
          Doc
        </button>
      </div>
    </div>
  );
}

export function App({ repo, host }: AppProps) {
  // The element gates rendering on `repo && host` (Leptos side
  // renders a spinner until both are present), so by the time we
  // get here both values are real and stable for the React tree.
  useBackgroundSync(repo);
  const [viewMode, setViewMode] = useState<ViewMode>(initialViewMode);

  // Cross-mode glide: wrap the state change in startViewTransition so
  // each tile (named via view-transition-name) morphs between its
  // canvas position/size and its doc-mode grid slot. flushSync is
  // required — without it React would commit asynchronously, both
  // snapshots would be the same, and we'd get no animation.
  const switchMode = useCallback((next: ViewMode) => {
    const doc = document as DocumentWithVT;
    if (typeof doc.startViewTransition !== "function") {
      setViewMode(next);
      return;
    }
    doc.startViewTransition(() => {
      flushSync(() => setViewMode(next));
    });
  }, []);

  return (
    <RepoContext.Provider value={repo}>
      <HostContext.Provider value={host}>
        <ViewModeContext.Provider value={viewMode}>
          <ViewModeSwitcher mode={viewMode} onChange={switchMode} />
          <Grid />
        </ViewModeContext.Provider>
      </HostContext.Provider>
    </RepoContext.Provider>
  );
}
