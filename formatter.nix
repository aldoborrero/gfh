{
  flake,
  inputs,
  pkgs,
  ...
}:
let
  # Format with the toolchain the crate is built with, so `nix fmt` and
  # `checks.fmt` cannot disagree about rustfmt's edition-dependent rules.
  inherit
    (flake.lib.mkRust {
      inherit pkgs;
      extensions = [ "rustfmt" ];
    })
    rust
    ;

  formatter = inputs.treefmt-nix.lib.mkWrapper pkgs {
    projectRootFile = "flake.nix";

    programs = {
      deadnix.enable = true;
      nixfmt.enable = true;
      rustfmt = {
        enable = true;
        package = rust;
        # Standalone rustfmt ignores Cargo.toml and treefmt-nix defaults to 2024;
        # import ordering is edition-dependent, so take it from the crate.
        inherit ((pkgs.lib.importTOML ./Cargo.toml).package) edition;
      };
      taplo.enable = true;
    };

    settings = {
      formatter.deadnix.pipeline = "nix";
      formatter.deadnix.priority = 1;
      formatter.nixfmt.pipeline = "nix";
      formatter.nixfmt.priority = 2;
    };
  };

  check =
    pkgs.runCommand "format-check"
      {
        nativeBuildInputs = [
          formatter
          pkgs.git
        ];
        meta.platforms = pkgs.lib.platforms.linux;
      }
      ''
        export HOME=$NIX_BUILD_TOP/home

        # keep timestamps so that treefmt is able to detect mtime changes
        cp --no-preserve=mode --preserve=timestamps -r ${flake} source
        cd source
        git init --quiet
        git add .
        treefmt --no-cache
        if ! git diff --exit-code; then
          echo "-------------------------------"
          echo "aborting due to above changes ^"
          exit 1
        fi
        touch $out
      '';
in
formatter
// {
  meta = formatter.meta // {
    tests = { inherit check; };
  };
}
