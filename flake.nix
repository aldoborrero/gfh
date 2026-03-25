{
  description = "Git FIDO helper - use multiple FIDO keys to sign Git commits";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      crane,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rust = pkgs.rust-bin.stable.latest.default;
        craneLib = (crane.mkLib pkgs).overrideToolchain rust;

        stdenv =
          if pkgs.stdenv.isLinux then pkgs.stdenv else pkgs.clangStdenv;

        commonArgs = {
          inherit stdenv;
          src = craneLib.cleanCargoSource (craneLib.path ./.);

          buildInputs =
            with pkgs;
            [ pcsclite ]
            ++ (lib.optionals stdenv.isLinux [ eudev ])
            ++ (lib.optionals stdenv.isDarwin [ libiconvReal ]);

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
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        gfh = craneLib.buildPackage (commonArgs // { inherit cargoArtifacts; });
      in
      {
        packages.default = gfh;

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = commonArgs.nativeBuildInputs;
          buildInputs = [ rust ] ++ commonArgs.buildInputs;
        };
      }
    );
}
