{
  pkgs,
  crane,
  nix-filter,
  rustToolchain,
  dialog-db-src,
}:

let
  craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

  src = nix-filter.lib {
    root = dialog-db-src;
    include = [
      ".cargo"
      "Cargo.lock"
      "Cargo.toml"
      "rust-toolchain.toml"
      "rust"
    ];
  };

  commonArgs = {
    pname = "wbg-pool";
    version = "0.1.0";
    inherit src;

    cargoExtraArgs = "--locked -p wbg-pool";
    strictDeps = true;
    doCheck = false;
  };

  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      pname = "wbg-pool-deps";
    }
  );
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    patches = [
      ./patches/wbg-pool-live-daemon.patch
      ./patches/wbg-pool-preserve-report-fetch.patch
    ];
  }
)
