// The space's kernel the Worker was deployed with, put on the global object
// before the Rust module loads (see src/kernel.rs).
import component from "./kernel.component";
import { modules } from "./modules/index.mjs";

globalThis.TONK_KERNEL = { component, modules };
