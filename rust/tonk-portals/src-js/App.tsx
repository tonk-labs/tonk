import { useEffect } from "react";
import { Grid } from "./grid/Grid";
import { HostContext, RepoContext } from "./context";

export type AppProps = {
  repo: string;
  host: string;
};

const SYNC_INTERVAL_MS = 3000;
const SYNC_BRANCH = "main";

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

export function App({ repo, host }: AppProps) {
  // The element gates rendering on `repo && host` (Leptos side
  // renders a spinner until both are present), so by the time we
  // get here both values are real and stable for the React tree.
  useBackgroundSync(repo);
  return (
    <RepoContext.Provider value={repo}>
      <HostContext.Provider value={host}>
        <Grid />
      </HostContext.Provider>
    </RepoContext.Provider>
  );
}
