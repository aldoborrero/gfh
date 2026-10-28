{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.gfh;

  # Generate gfh config file content from devices attrset
  gfhConfigContent =
    lib.concatStringsSep "\n" (
      lib.mapAttrsToList (serial: device: "# ${device.name}\n${serial}::${device.signingKey}") cfg.devices
    )
    + "\n";

  # Expand a leading `~/` here: the activation script quotes every path, and
  # the shell does not expand a tilde inside quotes.
  expandHome =
    path:
    if lib.hasPrefix "~/" path then
      "${config.home.homeDirectory}/${lib.removePrefix "~/" path}"
    else
      path;

  # Collect all signing key .pub paths for allowed_signers
  allSigningKeyPubs = lib.mapAttrsToList (
    _serial: device:
    let
      key = expandHome device.signingKey;
    in
    if lib.hasSuffix ".pub" key then key else "${key}.pub"
  ) cfg.devices;

  allowedSignersFile = "${config.home.homeDirectory}/.ssh/allowed_signers";

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
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Git user name for commit attribution (sets `user.name`). Null leaves it unset.";
        example = "Aldo Borrero";
      };

      userEmail = lib.mkOption {
        type = lib.types.str;
        description = "Git user email (sets `user.email`). Also used as the principal in allowed_signers.";
        example = "82811+aldoborrero@users.noreply.github.com";
      };

      signCommits = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Whether to sign commits and tags by default.

          Without this, gfh is wired up as git's signing backend but nothing asks
          git to sign, so commits are silently created unsigned.
        '';
      };
    };

    agentSocket = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "\${XDG_RUNTIME_DIR}/ssh-agent";
      description = ''
        Agent socket gfh uses to load a signing key (sets `GFH_AGENT_SOCK`).

        Only needed when `SSH_AUTH_SOCK` points at a read-only agent multiplexer,
        which forwards signing requests but refuses new keys. Null uses
        `SSH_AUTH_SOCK`.
      '';
    };

    hidTimeout = lib.mkOption {
      type = lib.types.nullOr (lib.types.ints.between 1 60000);
      default = null;
      example = 500;
      description = ''
        How long gfh waits for a YubiKey to answer over FIDO HID, in
        milliseconds (sets `GFH_HID_TIMEOUT_MS`). A healthy key answers in a few
        milliseconds; this only bounds how long a key that never answers can
        delay a commit. Null keeps gfh's default of 2000.
      '';
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

    home.sessionVariables =
      lib.optionalAttrs (cfg.agentSocket != null) { GFH_AGENT_SOCK = cfg.agentSocket; }
      // lib.optionalAttrs (cfg.hidTimeout != null) {
        GFH_HID_TIMEOUT_MS = toString cfg.hidTimeout;
      };

    # Configure git for SSH signing via gfh
    programs.git = lib.mkIf cfg.git.enable {
      signing.format = "ssh";
      settings = {
        gpg.ssh.defaultKeyCommand = "${cfg.package}/bin/gfh";
        # Lets `git log --show-signature` and `git verify-commit` check them.
        gpg.ssh.allowedSignersFile = allowedSignersFile;
        user = {
          email = lib.mkDefault cfg.git.userEmail;
        }
        // lib.optionalAttrs (cfg.git.userName != null) {
          name = lib.mkDefault cfg.git.userName;
        };
      }
      # Spelled as home-manager's git module spells it, so the two definitions
      # merge and mkDefault can lose to `signing.signByDefault`. A lowercase
      # `gpgsign` is a distinct attribute that would silently win instead: git
      # treats the keys as one and takes the last.
      // lib.optionalAttrs cfg.git.signCommits {
        commit.gpgSign = lib.mkDefault true;
        tag.gpgSign = lib.mkDefault true;
      };
    };

    # Generate allowed_signers at activation time (needs to read .pub files)
    home.activation.generateAllowedSigners = lib.mkIf cfg.git.enable (
      lib.hm.dag.entryAfter [ "writeBoundary" ] ''
        email=${lib.escapeShellArg cfg.git.userEmail}
        signers=${lib.escapeShellArg allowedSignersFile}
        mkdir -p "$(dirname "$signers")"
        : > "$signers"
        ${lib.concatMapStringsSep "\n" (pubPath: ''
          if [ -f ${lib.escapeShellArg pubPath} ]; then
            echo "$email $(cat ${lib.escapeShellArg pubPath})" >> "$signers"
          else
            warnEcho "gfh: signing key "${lib.escapeShellArg pubPath}" not found, left out of $signers"
          fi
        '') allSigningKeyPubs}
      ''
    );
  };
}
