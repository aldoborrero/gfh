# gfh

> Git FIDO helper, or God Fucking Help me.

gfh is a tool for helping you sign your commits in Git with resident SSH keys
stored on multiple FIDO devices.

<p align="center">
  <a href="#getting-started">Getting Started</a>
  -
  <a href="#usage">Usage</a>
  -
  <a href="#installation">Installation</a>
</p>

**NB:** Currently this project has only been tested with the YubiKey 5C NFC. Any
FIDO2 certified device _should_ be compatible, but please let me know if you
encountered any issues with particular devices, or also if it works fine so that
I can have a running list of all keys that are verified working.

## Getting Started

Before you get started with gfh, you'll need to make sure that you already have
a resident SSH key on your FIDO key(s). The simplest way to do this is via
`ssh-keygen -t ed25519-sk -O resident`, but there are better guides online if
you need some different stuff.

If you don't own multiple FIDO keys/only use one resident SSH key, then you more
than likely do not need to use gfh. This tool has a very niche use case due to
Git not supporting multiple `signingkey`s. If you only use one resident SSH key
for signing your commits, you can just use that config option without gfh.

## Usage

The simplest way to add your keys to gfh is via `gfh add`. This will prompt you
to select the FIDO key to use, as well as the path to the public key (or private
key) to use with it (this must be a resident key that you generated for that
particular FIDO device). If only one device is connected, it will be
auto-selected.

`gfh list` shows the configured mappings and whether each device is currently
connected, and `gfh remove` deletes one.

If you prefer, you can edit the config manually by creating a file at
`~/.config/gfh/keys` with the following format:

```
serial::~/.ssh/id_ed25519_sk
serial::~/.ssh/id_ecdsa_sk
```

(Blank lines & lines starting with `#` are preserved.)

After importing your keys to gfh, run the following commands to set up SSH
signing with Git:

```sh
git config --global commit.gpgsign true
git config --global tag.gpgsign true
git config --global gpg.format "ssh"
git config --global gpg.ssh.defaultKeyCommand "gfh"
```

(You shouldn't set `user.signingkey` because gfh will handle that for you
automatically.)

gfh reads the `.pub` file corresponding to the configured key path and outputs
its content directly. Git then uses ssh-agent (`-U` flag) to find the matching
private key for signing. This means:

- No need for `gpg.ssh.program` — gfh works with the default `ssh-keygen`.
- The signing key must be loaded in your ssh-agent. If it isn't, gfh loads it
  for you from the configured key file, which needs neither a PIN nor a touch —
  only the signature itself does.
- For a resident credential with no local key file, run `ssh-add -K` to pull it
  off the plugged-in FIDO device. gfh warns and lets git report the failure.
- If `SSH_AUTH_SOCK` points at a read-only agent multiplexer, which forwards
  signing requests but refuses new keys, set `GFH_AGENT_SOCK` to the writable
  agent behind it.

If all goes according to plan, you should be able to create a new commit or tag
with your FIDO key plugged in, and Git will correctly prompt you to sign with
it.

## Installation

### Nix

```
nix run github:aldoborrero/gfh
```

Or add to your flake inputs and reference `gfh.packages.${system}.default`.

### Crate

`cargo install gfh`

### From source

Pull this repository and run `cargo build --release`, and look for the `gfh`
binary in `./target/release/`.

When building from source or from Cargo, on Linux you will need the following
packages: `pkg-config libpcsclite-dev libudev-dev`

### Homebrew

`brew install ovyerus/tap/gfh`

### AUR (unofficial)

An unofficial AUR package from [wale](https://github.com/wale) is available at
https://aur.archlinux.org/packages/gfh.

## License

This program is licensed under the MIT license (see [LICENSE](./LICENSE) or
https://opensource.org/licenses/MIT).
