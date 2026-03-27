{ pkgs, inputs, flake, ... }:
let
  inherit (pkgs) lib stdenv pcsclite eudev libiconvReal pkg-config darwin;

  pkgs' = import inputs.nixpkgs {
    inherit (stdenv.hostPlatform) system;
    overlays = [ (import inputs.rust-overlay) ];
  };
  rust = pkgs'.rust-bin.stable.latest.default.override {
    extensions = [ "clippy" ];
  };
  craneLib = (inputs.crane.mkLib pkgs').overrideToolchain rust;

  commonArgs = {
    stdenv = p: if p.stdenv.isLinux then p.stdenv else p.clangStdenv;
    src = craneLib.cleanCargoSource flake.outPath;

    buildInputs =
      [ pcsclite ]
      ++ (lib.optionals stdenv.isLinux [ eudev ])
      ++ (lib.optionals stdenv.isDarwin [ libiconvReal ]);

    nativeBuildInputs =
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
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.cargoClippy (commonArgs // {
  inherit cargoArtifacts;
  cargoClippyExtraArgs = "--all-targets -- --deny warnings";
})
