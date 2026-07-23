// The IronCalc engine wasm as bytes, isolated in its own entry chunk
// (`tonk-table-engine.js`). This file exists so the multi-megabyte
// binary lives in a pure data LEAF that only changes on an IronCalc
// version bump — grid-UI iteration never rewrites it — and so the
// engine always instantiates FROM BYTES (`init({ module_or_path })`),
// never from a URL fetch: the property that lets the graph blob-mint
// into sealed, opaque-origin portal guests where fetch is dead.
//
// esbuild's `binary` loader (see scripts/build.mjs) turns the `.wasm`
// import into an embedded base64 string decoded to a Uint8Array at
// module evaluation.

import bytes from "@ironcalc/wasm/wasm_bg.wasm";

export default bytes;
