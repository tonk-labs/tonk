import { createContext } from "react";

// Per-portal context. The element passes `repo` and `host` in;
// every Square reads them when composing its iframe URL. No
// branch context — that's per-tile (see types.ts).
export const RepoContext = createContext<string>("");
export const HostContext = createContext<string>("");
