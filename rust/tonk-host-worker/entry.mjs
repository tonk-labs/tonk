// The Worker's entry: the kernel it was deployed with first, then the Worker
// itself, as worker-build made it.
import "./kernel/setup.mjs";

export * from "./build/index.js";
export { default } from "./build/index.js";
