{
  pkgs,
  flake,
  inputs,
  ...
}:
let
  # Rewritten to a scratch directory at build time; the module bakes the home
  # directory into the activation script.
  home = "/gfh-test-home";

  hm = inputs.home-manager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      flake.homeModules.gfh
      {
        home = {
          username = "tester";
          homeDirectory = home;
          stateVersion = "25.11";
        };
        programs.git.enable = true;
        programs.gfh = {
          enable = true;
          package = flake.packages.${pkgs.stdenv.hostPlatform.system}.gfh;
          devices = {
            "111" = {
              name = "Primary";
              signingKey = "~/.ssh/primary_sk";
            };
            "222" = {
              name = "Backup";
              signingKey = "${home}/.ssh/backup_sk.pub";
            };
            "333" = {
              name = "Missing";
              signingKey = "~/.ssh/missing_sk";
            };
          };
          git = {
            userName = "Tester";
            userEmail = "tester@example.com";
          };
        };
      }
    ];
  };

  inherit (hm.config.programs.git) iniContent;

  activation = pkgs.writeText "generate-allowed-signers" hm.config.home.activation.generateAllowedSigners.data;
in
assert iniContent.gpg.ssh.allowedSignersFile == "${home}/.ssh/allowed_signers";
assert iniContent.user.name == "Tester";
assert iniContent.user.email == "tester@example.com";
pkgs.runCommand "gfh-home-module" { } ''
  set -eu
  root="$TMPDIR/home"
  mkdir -p "$root/.ssh"
  echo "sk-ssh-ed25519@openssh.com AAAAprimary" > "$root/.ssh/primary_sk.pub"
  echo "sk-ssh-ed25519@openssh.com AAAAbackup" > "$root/.ssh/backup_sk.pub"

  warnEcho() { echo "$*" >> "$TMPDIR/warnings"; }
  script="$TMPDIR/activation.sh"
  sed "s|${home}|$root|g" ${activation} > "$script"
  . "$script"

  cat > "$TMPDIR/expected" <<EOF
  tester@example.com sk-ssh-ed25519@openssh.com AAAAprimary
  tester@example.com sk-ssh-ed25519@openssh.com AAAAbackup
  EOF
  sort "$root/.ssh/allowed_signers" > "$TMPDIR/got"
  sort "$TMPDIR/expected" > "$TMPDIR/want"
  diff -u "$TMPDIR/want" "$TMPDIR/got"
  grep -q missing_sk.pub "$TMPDIR/warnings"

  touch $out
''
