{ pkgs, inputs, ... }:
let
  pkgs' = import inputs.nixpkgs {
    inherit (pkgs.stdenv.hostPlatform) system;
    overlays = [ (import inputs.rust-overlay) ];
  };
  rust = pkgs'.rust-bin.stable.latest.default;

  stdenv =
    if pkgs.stdenv.isLinux then pkgs.stdenv else pkgs.clangStdenv;
in
pkgs.mkShell {
  inherit stdenv;

  nativeBuildInputs =
    with pkgs;
    [ pkg-config ]
    ++ (lib.optionals stdenv.isDarwin (
      with darwin.apple_sdk;
      [
        frameworks.AppKit
        frameworks.CoreFoundation
        frameworks.IOKit
        frameworks.PCSC
      ]
    ));

  buildInputs =
    with pkgs;
    [ rust pcsclite ]
    ++ (lib.optionals stdenv.isLinux [ eudev ])
    ++ (lib.optionals stdenv.isDarwin [ libiconvReal ]);
}
