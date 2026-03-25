{
  lib,
  stdenv,
  pcsclite,
  eudev,
  libiconvReal,
  pkg-config,
  darwin,
  inputs,
}:
let
  pkgs' = import inputs.nixpkgs { inherit (stdenv.hostPlatform) system; overlays = [ (import inputs.rust-overlay) ]; };
  rust = pkgs'.rust-bin.stable.latest.default;
  craneLib = (inputs.crane.mkLib pkgs').overrideToolchain rust;

  commonArgs = {
    stdenv = if stdenv.isLinux then stdenv else pkgs'.clangStdenv;
    src = craneLib.cleanCargoSource (craneLib.path ../../.);

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
craneLib.buildPackage (commonArgs // { inherit cargoArtifacts; })
