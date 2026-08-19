{ pkgs, flake }:
let
  inherit (flake.lib.mkRust { inherit pkgs; }) craneLib;

  commonArgs = flake.lib.mkNativeDeps pkgs // {
    src = craneLib.cleanCargoSource (craneLib.path ../../.);
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (commonArgs // { inherit cargoArtifacts; })
