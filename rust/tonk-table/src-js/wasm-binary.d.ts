// esbuild's `binary` loader turns a `.wasm` import into a module whose
// default export is the file's bytes (see scripts/build.mjs). Declared
// here so `engine.ts` typechecks; esbuild does the actual loading.
declare module "*.wasm" {
  const bytes: Uint8Array;
  export default bytes;
}
