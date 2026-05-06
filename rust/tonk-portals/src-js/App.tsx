import { Grid } from "./grid/Grid";
import { HostContext, RepoContext } from "./context";

export type AppProps = {
  repo: string;
  host: string;
};

export function App({ repo, host }: AppProps) {
  // The element gates rendering on `repo && host` (Leptos side
  // renders a spinner until both are present), so by the time we
  // get here both values are real and stable for the React tree.
  return (
    <RepoContext.Provider value={repo}>
      <HostContext.Provider value={host}>
        <Grid />
      </HostContext.Provider>
    </RepoContext.Provider>
  );
}
