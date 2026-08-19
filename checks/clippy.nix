{ pkgs, flake, ... }:
let
  inherit
    (flake.lib.mkRust {
      inherit pkgs;
      extensions = [ "clippy" ];
    })
    craneLib
    ;

  commonArgs = flake.lib.mkNativeDeps pkgs // {
    src = craneLib.cleanCargoSource flake.outPath;
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.cargoClippy (
  commonArgs
  // {
    inherit cargoArtifacts;
    cargoClippyExtraArgs = "--all-targets -- --deny warnings";
  }
)
