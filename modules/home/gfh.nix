{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.gfh;

  # Generate gfh config file content from devices attrset
  gfhConfigContent = lib.concatStringsSep "\n" (
    lib.mapAttrsToList (
      serial: device:
      "# ${device.name}\n${serial}::${device.signingKey}"
    ) cfg.devices
  ) + "\n";

  # Collect all signing key .pub paths for allowed_signers
  allSigningKeyPubs = lib.mapAttrsToList (
    _serial: device:
    let
      key = device.signingKey;
    in
    if lib.hasSuffix ".pub" key then key else "${key}.pub"
  ) cfg.devices;

  deviceOpts = {
    options = {
      name = lib.mkOption {
        type = lib.types.str;
        description = "Human-readable name for this device.";
        example = "Primary YubiKey";
      };

      signingKey = lib.mkOption {
        type = lib.types.str;
        description = "Path to the SSH key file (private or public) to use for signing with this device.";
        example = "~/.ssh/id_ed25519_sk_rk_aldo@github.com";
      };
    };
  };
in
{
  options.programs.gfh = {
    enable = lib.mkEnableOption "gfh - Git FIDO Helper for multi-key signing";

    package = lib.mkPackageOption pkgs "gfh" {
      default = null;
      extraDescription = "The gfh package to use. Must be set explicitly since gfh is not in nixpkgs.";
    };

    devices = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule deviceOpts);
      default = { };
      description = "FIDO device serial numbers mapped to their SSH signing keys.";
      example = lib.literalExpression ''
        {
          "18042805" = {
            name = "Primary YubiKey";
            signingKey = "~/.ssh/id_ed25519_sk_rk_aldo@github.com";
          };
          "20646006" = {
            name = "Backup YubiKey";
            signingKey = "~/.ssh/id_ed25519_sk_rk_backup@github.com";
          };
        }
      '';
    };

    git = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether to configure git to use gfh for SSH signing.";
      };

      userName = lib.mkOption {
        type = lib.types.str;
        description = "Git user name for commit attribution.";
        example = "Aldo Borrero";
      };

      userEmail = lib.mkOption {
        type = lib.types.str;
        description = "Git user email. Also used as the principal in allowed_signers.";
        example = "82811+aldoborrero@users.noreply.github.com";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.package != null;
        message = "programs.gfh.package must be set to the gfh package.";
      }
      {
        assertion = cfg.devices != { };
        message = "programs.gfh.devices must contain at least one device mapping.";
      }
    ];

    home.packages = [ cfg.package ];

    # Generate ~/.config/gfh/keys
    xdg.configFile."gfh/keys".text = gfhConfigContent;

    # Configure git for SSH signing via gfh
    programs.git = lib.mkIf cfg.git.enable {
      signing.format = "ssh";
      settings.gpg.ssh.defaultKeyCommand = "${cfg.package}/bin/gfh";
    };

    # Generate allowed_signers at activation time (needs to read .pub files)
    home.activation.generateAllowedSigners = lib.mkIf cfg.git.enable (
      lib.hm.dag.entryAfter [ "writeBoundary" ] ''
        email="${cfg.git.userEmail}"
        sshDir="/home/${config.home.username}/.ssh"
        out="$sshDir/allowed_signers"
        : > "$out"
        ${lib.concatMapStringsSep "\n" (pubPath: ''
          if [ -f "${pubPath}" ]; then
            echo "$email $(cat "${pubPath}")" >> "$out"
          fi
        '') allSigningKeyPubs}
      ''
    );
  };
}
