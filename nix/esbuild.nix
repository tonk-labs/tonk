# Nix overlay so that we can control the version of esbuild we're using
# worker-build needs this minimum esbuild version, otherwise it fails
# due to emerging ESM + Wasm module syntax.
# SEE: https://github.com/cloudflare/workers-rs/pull/813
final: prev: {
  esbuild = prev.buildGoModule rec {
    pname = "esbuild";
    version = "0.25.10";

    src = prev.fetchFromGitHub {
      owner = "evanw";
      repo = "esbuild";
      rev = "v${version}";
      hash = "sha256-EkQOIHqVrULig7s3w4nI8/yVIz2NZA5DCrMof0HHvHM=";
    };

    vendorHash = "sha256-+BfxCyg0KkDQpHt/wycy/8CTG6YBA/VJvJFhhzUnSiQ=";

    subPackages = [ "cmd/esbuild" ];

    ldflags = [
      "-s"
      "-w"
    ];

    meta = {
      description = "Extremely fast JavaScript bundler";
      homepage = "https://esbuild.github.io";
      changelog = "https://github.com/evanw/esbuild/blob/v${version}/CHANGELOG.md";
      license = prev.lib.licenses.mit;
      maintainers = with prev.lib.maintainers; [
        lucus16
        undefined-moe
        ivan
      ];
      mainProgram = "esbuild";
    };
  };
}

