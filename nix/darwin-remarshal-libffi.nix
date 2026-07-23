# Darwin-only workaround for the macOS libffi trampoline break.
#
# Apple's libffi fork allocates closure trampolines with a `vm_remap` that
# recent macOS rejects, so every Python ctypes closure aborts with
# `Assertion failed: (trampoline_handle) ... closures.c`. crane generates
# each crate's `Cargo.toml` by shelling out to `remarshal` (Python ->
# ctypes -> libffi), so every crane build here dies at the generated
# `Cargo.toml` derivation with exit code 134. MIT libffi (`libffiReal`) is
# unaffected.
#
# The fix only needs remarshal to *run* under an interpreter linked against
# `libffiReal`. Rebuilding the interpreter's whole package set from source
# is both unnecessary and unworkable here: overriding the top-level
# `python3` cascades into rebuilding much of the tree (meson, zstd, ...),
# and remarshal's own closure has C extensions whose build backend and test
# suites don't build from source on this macOS.
#
# So rewrap the already-cached `remarshal` onto a `libffiReal` interpreter
# instead of recompiling anything. `pyReal` is the same CPython (only its
# libffi differs, which touches only `_ctypes`), so remarshal's cached,
# ABI-compatible dependency modules load unchanged — they are baked into
# the wrapper as absolute `site.addsitedir` paths, independent of the
# interpreter. Only the interpreter the wrappers exec has to change.
#
# `craneLib = crane.mkLib pkgs`, so overlaying the top-level `remarshal`
# reaches every crane build. Drop this overlay once nixpkgs' darwin libffi
# (or the OS) is fixed.
final: prev:
prev.lib.optionalAttrs prev.stdenv.hostPlatform.isDarwin {
  remarshal =
    let
      pyReal = prev.python3.override { libffi = prev.libffiReal; };
    in
    prev.runCommandLocal "remarshal-${prev.remarshal.version}-libffireal" { } ''
      cp -r ${prev.remarshal} "$out"
      chmod -R u+w "$out"
      for f in $(find "$out/bin" -type f); do
        # Point the wrappers' interpreter at the libffiReal CPython, and
        # redirect their self-references (the outer wrapper execs
        # `.remarshal-wrapped` by absolute path) from the stock store path
        # to this one, so `bin/remarshal` runs the rewrapped script rather
        # than jumping back to the stock, still-broken interpreter.
        substituteInPlace "$f" \
          --replace-quiet '${prev.python3}' '${pyReal}' \
          --replace-quiet '${prev.remarshal}' "$out"
      done
    '';
}
