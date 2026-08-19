{ pkgs, flake, ... }:
let
  inherit
    (flake.lib.mkRust {
      inherit pkgs;
      extensions = [ "rustfmt" ];
    })
    craneLib
    ;
in
craneLib.cargoFmt {
  src = craneLib.cleanCargoSource flake.outPath;
}
