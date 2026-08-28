{ inputs, ... }:
{
  # Rust toolchain and the crane lib bound to it. Returned together because crane
  # must be given the same toolchain that builds the crate.
  mkRust =
    {
      pkgs,
      extensions ? [ ],
    }:
    let
      pkgs' = import inputs.nixpkgs {
        inherit (pkgs.stdenv.hostPlatform) system;
        overlays = [ (import inputs.rust-overlay) ];
      };
      rust = pkgs'.rust-bin.stable.latest.default.override { inherit extensions; };
    in
    {
      inherit rust;
      # The stdenv must be selected here rather than passed per-derivation:
      # crane deprecated the `stdenv` argument to `mkCargoDerivation`.
      craneLib = ((inputs.crane.mkLib pkgs').overrideToolchain rust).overrideScope (
        _final: _prev: {
          stdenvSelector = p: if p.stdenv.hostPlatform.isLinux then p.stdenv else p.clangStdenv;
        }
      );
    };

  # Native dependencies for talking to a smart card, shared by every derivation
  # that compiles the crate.
  mkNativeDeps =
    pkgs:
    let
      inherit (pkgs) lib stdenv;
    in
    {
      buildInputs = [
        pkgs.pcsclite
      ]
      ++ lib.optionals stdenv.hostPlatform.isLinux [ pkgs.eudev ]
      ++ lib.optionals stdenv.hostPlatform.isDarwin [ pkgs.libiconvReal ];

      nativeBuildInputs = [
        pkgs.pkg-config
      ]
      ++ lib.optionals stdenv.hostPlatform.isDarwin (
        with pkgs.darwin.apple_sdk;
        [
          frameworks.AppKit
          frameworks.CoreFoundation
          frameworks.IOKit
          frameworks.PCSC
        ]
      );
    };
}
