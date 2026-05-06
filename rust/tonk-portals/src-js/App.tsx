// Top-level React component for the portals UI. The custom
// element passes in `repo` / `host`; we render a stub for now so
// we can verify the bundle, the script injection, and the Leptos
// shell wire-up before porting the grid.
//
// Branch is intentionally NOT a top-level prop. Portals are a UI
// layer over a repo; each artifact tile inside the grid will
// carry its own branch when composing the data URL it loads.

export type AppProps = {
  repo: string;
  host: string;
};

export function App({ repo, host }: AppProps) {
  const ready = repo && host;
  return (
    <div className="tonk-portals-stub">
      <h2>tonk-portals</h2>
      <dl>
        <dt>repo</dt>
        <dd>{repo || <em>unset</em>}</dd>
        <dt>host</dt>
        <dd>
          <code>{host || <em>waiting…</em>}</code>
        </dd>
      </dl>
      <p className="tonk-portals-stub__note">
        {ready
          ? "Bundle loaded. Grid port lands next."
          : "Waiting on host id from the service worker."}
      </p>
    </div>
  );
}
