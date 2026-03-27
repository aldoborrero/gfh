{ pkgs, inputs, flake, ... }:
let
  pkgs' = import inputs.nixpkgs {
    inherit (pkgs.stdenv.hostPlatform) system;
    overlays = [ (import inputs.rust-overlay) ];
  };
  rust = pkgs'.rust-bin.stable.latest.default.override {
    extensions = [ "rustfmt" ];
  };
  craneLib = (inputs.crane.mkLib pkgs').overrideToolchain rust;
in
craneLib.cargoFmt {
  src = craneLib.cleanCargoSource flake.outPath;
}
