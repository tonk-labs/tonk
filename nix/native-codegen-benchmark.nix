# Experimental derivations only: no normal CI or shipping profile is changed.
{
  repo,
  units,
  nonce,
  scope ? "worker",
  stage ? "archive",
  optLevel ? 1,
}:
let
  flake = builtins.getFlake repo;
  system = builtins.currentSystem;
  pkgs = flake.inputs.nixpkgs.legacyPackages.${system};
  original = flake.packages.${system}.tests-native-ci;
  workspaceArgs = "--workspace --exclude tonk-ui --exclude tonk-core --features integration-tests";
  args =
    if scope == "workspace" then
      workspaceArgs
    else
      "--package tonk-worker --lib --features integration-tests";
  codegenOverrides = {
    CARGO_PROFILE_CI_TEST_CODEGEN_UNITS = toString units;
  };
  baselineDependencies = original.cargoArtifacts.overrideAttrs (
    old:
    codegenOverrides
    // {
      # Crane expands its command arguments into phases before overrideAttrs.
      buildPhase = builtins.replaceStrings [ workspaceArgs ] [ args ] old.buildPhase;
      checkPhase = builtins.replaceStrings [ workspaceArgs ] [ args ] old.checkPhase;
    }
  );
  overrides =
    codegenOverrides
    // pkgs.lib.optionalAttrs (optLevel != 1) {
      CARGO_PROFILE_CI_TEST_OPT_LEVEL = toString optLevel;
    };
  dependencies =
    if optLevel == 1 then
      baselineDependencies
    else
      baselineDependencies.overrideAttrs (_: {
        # External packages remain at the manifest's explicit opt-level 1.
        # Seed from that cache, then rebuild dummy workspace units at opt-level 0.
        cargoArtifacts = baselineDependencies;
        CARGO_PROFILE_CI_TEST_OPT_LEVEL = toString optLevel;
      });
  # Both configurations see identical source bytes. The nonce makes each trial
  # a real source rebuild, not a download of a previously compiled archive.
  # Dependencies deliberately retain the original dummy source and cache key.
  source = pkgs.runCommand "native-codegen-source" { } ''
    cp -R ${original.src} $out
    chmod -R u+w $out
    printf '\n// Native codegen benchmark: %s\n' ${pkgs.lib.escapeShellArg nonce} >> $out/rust/tonk-worker/src/lib.rs
  '';
  archive = original.overrideAttrs (
    old:
    overrides
    // {
      src = source;
      cargoArtifacts = dependencies;
      buildPhase =
        builtins.replaceStrings
          [ workspaceArgs "cargo nextest archive" ]
          [ args "cargo nextest archive --cargo-verbose" ]
          old.buildPhase;
    }
  );
in
assert pkgs.lib.hasInfix workspaceArgs original.buildPhase;
assert pkgs.lib.hasInfix workspaceArgs original.cargoArtifacts.buildPhase;
assert pkgs.lib.hasInfix workspaceArgs original.cargoArtifacts.checkPhase;
assert builtins.elem optLevel [
  0
  1
];
assert builtins.elem scope [
  "worker"
  "workspace"
];
assert builtins.elem stage [
  "archive"
  "dependencies"
];
assert builtins.elem units [
  16
  256
];
if stage == "dependencies" then dependencies else archive
