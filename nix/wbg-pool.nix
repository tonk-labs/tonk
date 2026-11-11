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

  # The one change to wbg-pool that the pinned dialog does not have yet.
  # dialog carries it from dialog-db/dialog-db#774 on; when `dialog-db-src`
  # moves to a release with that, this patch and the directory go.
  patches = [
    ./patches/wbg-pool-focus-tabs.patch
  ];

  packageArgs = commonArgs // {
    inherit cargoArtifacts patches;
  };

  unitTests = craneLib.cargoTest (
    packageArgs
    // {
      pname = "wbg-pool-unit-tests";
      doCheck = true;
    }
  );
in
(craneLib.buildPackage packageArgs).overrideAttrs (old: {
  passthru = (old.passthru or { }) // {
    tests.unit = unitTests;
  };
})
