{
  flake,
  pkgs,
  ...
}:
let
  inherit
    (flake.lib.mkRust {
      inherit pkgs;
      extensions = [
        "clippy"
        "rust-analyzer"
        "rust-src"
        "rustfmt"
      ];
    })
    rust
    ;

  stdenv = if pkgs.stdenv.hostPlatform.isLinux then pkgs.stdenv else pkgs.clangStdenv;
in
pkgs.mkShell {
  inherit stdenv;

  nativeBuildInputs =
    with pkgs;
    [ pkg-config ]
    ++ (lib.optionals stdenv.hostPlatform.isDarwin (
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
    [
      rust
      pcsclite
    ]
    ++ (lib.optionals stdenv.hostPlatform.isLinux [ eudev ])
    ++ (lib.optionals stdenv.hostPlatform.isDarwin [ libiconvReal ]);
}
