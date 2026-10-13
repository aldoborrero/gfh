use anyhow::{Context, Result};
use core::fmt;
use ctap_hid_fido2::{HidInfo, HidParam};
use hidapi::HidApi;
use std::time::Duration;
use yubikey_api::YubiKey;

use crate::{hid_serial, yubikey};

const YUBICO_VID: u16 = 0x1050;

pub enum FidoDevice {
    /// A YubiKey read over PC/SC.
    YubiKey(YubiKey),
    /// A YubiKey whose serial was read over its FIDO HID interface.
    YubiKeyHid {
        name: String,
        serial: u32,
    },
    Generic(HidInfo),
}

impl FidoDevice {
    pub fn name(&self) -> String {
        match self {
            Self::YubiKey(yubi) => yubi.name().to_owned(),
            Self::YubiKeyHid { name, .. } => name.clone(),
            Self::Generic(device) => device.product_string.clone(),
        }
    }

    /// The device serial, if it reports one.
    ///
    /// `None` rather than a sentinel: the serial is a config lookup key, and
    /// `"unknown"` or `""` are lines a config can legitimately contain, so a
    /// device without a serial would match another device's mapping.
    pub fn serial(&self) -> Option<String> {
        match self {
            Self::YubiKey(yubi) => Some(yubi.serial().0.to_string()),
            Self::YubiKeyHid { serial, .. } => Some(serial.to_string()),
            Self::Generic(device) => device
                .info
                .split(' ')
                .find_map(|x| x.strip_prefix("serial_number="))
                .filter(|serial| !serial.is_empty())
                .map(ToOwned::to_owned),
        }
    }
}

impl fmt::Display for FidoDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.serial() {
            Some(serial) => write!(f, "{serial} - {}", self.name()),
            None => write!(f, "<no serial> - {}", self.name()),
        }
    }
}

/// Every connected FIDO device. `timeout` bounds each YubiKey's answer over
/// FIDO HID.
pub fn get_all_devices(timeout: Duration) -> Result<Vec<FidoDevice>> {
    let (yubikeys, others): (Vec<HidInfo>, Vec<HidInfo>) = ctap_hid_fido2::get_fidokey_devices()
        .into_iter()
        .partition(|h| h.vid == YUBICO_VID);
    let mut devices: Vec<FidoDevice> = others.into_iter().map(FidoDevice::Generic).collect();

    // A YubiKey's HID interface reports an empty iSerial, while the config keys
    // on the device serial. Read that over FIDO HID first, so that signing
    // never touches PC/SC: on a key shared over USB/IP, pcscd polling CCID
    // corrupts the FIDO signature that follows (see hid_serial).
    let mut unread = 0;
    match HidApi::new() {
        Ok(api) => {
            for h in &yubikeys {
                let serial = match &h.param {
                    HidParam::Path(path) => hid_serial::read_serial(&api, path, timeout),
                    HidParam::VidPid { .. } => Ok(None),
                };
                match serial {
                    Ok(Some(serial)) => devices.push(FidoDevice::YubiKeyHid {
                        name: h.product_string.clone(),
                        serial,
                    }),
                    Ok(None) => unread += 1,
                    Err(err) => {
                        eprintln!("warning: {}: {err:#}", h.product_string);
                        unread += 1;
                    }
                }
            }
        }
        Err(err) => {
            eprintln!("warning: could not open the HID API: {err}");
            unread = yubikeys.len();
        }
    }

    // Older YubiKeys do not answer READ_CONFIG over FIDO; only those need PC/SC.
    if unread > 0 {
        // Degrade rather than fail: other FIDO devices still work without PC/SC.
        // But say so, because a YubiKey is then invisible and the only other
        // signal the user gets is "no matching FIDO key found", which blames
        // the config.
        let pcsc = yubikey::get_yubikeys().unwrap_or_else(|err| {
            eprintln!(
                "warning: {err:#}. If a YubiKey is plugged in, check that pcscd is \
                 running and that the CCID interface is enabled."
            );
            Vec::new()
        });
        let known: Vec<String> = devices.iter().filter_map(FidoDevice::serial).collect();
        devices.extend(
            pcsc.into_iter()
                .filter(|d| d.serial().is_none_or(|s| !known.contains(&s))),
        );
    }

    Ok(devices)
}

/// Socket to use when *adding* a key.
///
/// `SSH_AUTH_SOCK` may point at a read-only multiplexer that forwards signing
/// requests but refuses `add_identity`; `GFH_AGENT_SOCK` names the writable agent
/// behind it.
fn writable_agent_sock() -> Option<String> {
    std::env::var("GFH_AGENT_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("SSH_AUTH_SOCK")
                .ok()
                .filter(|s| !s.is_empty())
        })
}

/// Reject key paths `ssh-add` would read as options.
///
/// `ssh-add` has no `--` terminator, so a config path like `-D` becomes a flag
/// that wipes the agent.
pub fn check_key_path(priv_path: &str) -> Result<()> {
    if priv_path.starts_with('-') {
        anyhow::bail!("refusing key path {priv_path:?}: ssh-add would parse it as an option");
    }
    Ok(())
}

/// Register a FIDO key handle with the ssh-agent.
///
/// Only the credential handle is handed over, so no touch is needed here, and no
/// PIN unless the key file itself is passphrase-protected. Touch is still
/// required for every signature the agent later performs, unless the credential
/// was created with `no-touch-required`; a PIN is required per signature only
/// for a `verify-required` credential.
pub fn load_key_into_agent(priv_path: &str, key_content: &str) -> Result<()> {
    check_key_path(priv_path)?;

    let sock = writable_agent_sock()
        .context("no ssh-agent socket: neither GFH_AGENT_SOCK nor SSH_AUTH_SOCK is set")?;

    // git blocks on this command, so ssh-add must never be able to wait on a
    // prompt: a passphrase-protected key would otherwise hang the commit on an
    // askpass dialog with no output.
    let output = std::process::Command::new("ssh-add")
        .arg(priv_path)
        .env("SSH_AUTH_SOCK", &sock)
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env_remove("DISPLAY")
        .stdin(std::process::Stdio::null())
        .output()
        .context("failed to run ssh-add")?;

    if !output.status.success() {
        anyhow::bail!(
            "could not load the signing key into the ssh-agent at {sock}\n  \
             tried: ssh-add {priv_path}\n  \
             ssh-add said: {}\n\
             If that socket is a read-only agent multiplexer, set GFH_AGENT_SOCK to \
             the writable agent behind it (e.g. $XDG_RUNTIME_DIR/ssh-agent).",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    // The add went to `sock`, but git will look at SSH_AUTH_SOCK. Distinguish
    // the two so a mismatch is not reported as a failed load.
    if !is_key_in_agent(key_content, None)? {
        if is_key_in_agent(key_content, Some(&sock))? {
            anyhow::bail!(
                "loaded the signing key into the agent at {sock}, but the agent git \
                 uses (SSH_AUTH_SOCK) does not expose it.\n\
                 Point GFH_AGENT_SOCK at an agent that SSH_AUTH_SOCK reaches."
            );
        }
        anyhow::bail!("ssh-add reported success but the key is not in the agent at {sock}");
    }

    Ok(())
}

/// Whether the agent holds this key. `sock` overrides `SSH_AUTH_SOCK`; `None`
/// queries the agent git itself will use.
///
/// `Err` means the agent could not be asked, which is a third outcome distinct
/// from holding the key or not. Collapsing it into `false` makes callers blame
/// the key for what is really a missing agent.
pub fn is_key_in_agent(key_content: &str, sock: Option<&str>) -> Result<bool> {
    let mut cmd = std::process::Command::new("ssh-add");
    cmd.arg("-L");
    if let Some(sock) = sock {
        cmd.env("SSH_AUTH_SOCK", sock);
    }
    let out = cmd.output().context("failed to run ssh-add -L")?;

    // ssh-add(1): 1 is "the agent has no identities", 2 is "could not contact
    // the agent". Only the latter is a failure to answer the question.
    match out.status.code() {
        Some(0 | 1) => {}
        _ => anyhow::bail!(
            "could not contact an ssh-agent at {}: {}",
            sock.unwrap_or("$SSH_AUTH_SOCK"),
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }

    // Compare key type + base64 blob, the two fields that identify a key; the
    // trailing comment is free-form and differs between agent and file.
    let mut wanted = key_content.split_whitespace();
    let Some(key_id) = wanted.next().zip(wanted.next()) else {
        anyhow::bail!("malformed public key: expected `<type> <base64>`");
    };

    Ok(String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        let mut have = line.split_whitespace();
        have.next().zip(have.next()) == Some(key_id)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_option_like_key_path() {
        // `ssh-add -D` would wipe every identity from the agent.
        assert!(check_key_path("-D").is_err());
        assert!(check_key_path("-s/tmp/evil.so").is_err());
    }

    #[test]
    fn accepts_ordinary_key_path() {
        assert!(check_key_path("/home/u/.ssh/id_ed25519_sk").is_ok());
        assert!(check_key_path("./relative-key").is_ok());
    }
}
