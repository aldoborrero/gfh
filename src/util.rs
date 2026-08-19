use anyhow::{Context, Result};
use core::fmt;
use ctap_hid_fido2::HidInfo;
use yubikey_api::YubiKey;

use crate::yubikey;

pub enum FidoDevice {
    YubiKey(YubiKey),
    Generic(HidInfo),
}

impl FidoDevice {
    pub fn name(&self) -> String {
        match self {
            Self::YubiKey(yubi) => yubi.name().to_owned(),
            Self::Generic(device) => device.product_string.clone(),
        }
    }

    pub fn serial(&self) -> String {
        match self {
            Self::YubiKey(yubi) => yubi.serial().0.to_string(),
            Self::Generic(device) => {
                let found = device
                    .info
                    .split(' ')
                    .find(|x| x.starts_with("serial_number="));

                match found {
                    Some(part) => part
                        .split_once('=')
                        .map(|(_, v)| v.to_owned())
                        .unwrap_or_else(|| String::from("unknown")),
                    None => String::from("unknown"),
                }
            }
        }
    }
}

impl fmt::Display for FidoDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} - {}", self.serial(), self.name())
    }
}

pub fn get_generics() -> Vec<FidoDevice> {
    ctap_hid_fido2::get_fidokey_devices()
        .into_iter()
        .map(FidoDevice::Generic)
        .collect()
}

pub fn get_all_devices() -> Result<Vec<FidoDevice>> {
    let fidos = get_generics();
    let mut yubikeys = yubikey::get_yubikeys();
    // A YubiKey's HID interface reports an empty iSerial, while the config keys
    // on the PIV serial read over PC/SC. Do not try to reconcile the two: an
    // HID entry's empty serial matches nothing, and shadows a real mapping if a
    // config line also has an empty serial.
    let mut fidos: Vec<FidoDevice> = fidos
        .into_iter()
        .filter(|x| match x {
            FidoDevice::Generic(h) => !h.product_string.to_lowercase().contains("yubikey"),
            _ => false,
        })
        .collect();

    // Dropping them means losing PC/SC hides YubiKeys completely; say so instead
    // of reporting "no matching FIDO key found" with one plugged in.
    if yubikeys.is_empty() && !fidos.is_empty() {
        eprintln!(
            "warning: no YubiKey found over PC/SC. If one is plugged in, check that \
             pcscd is running and that the CCID interface is enabled."
        );
    }

    fidos.append(&mut yubikeys);
    Ok(fidos)
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
    if !is_key_in_agent(key_content, None) {
        if is_key_in_agent(key_content, Some(&sock)) {
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
pub fn is_key_in_agent(key_content: &str, sock: Option<&str>) -> bool {
    let mut cmd = std::process::Command::new("ssh-add");
    cmd.arg("-L");
    if let Some(sock) = sock {
        cmd.env("SSH_AUTH_SOCK", sock);
    }
    let output = cmd.output();
    match output {
        Ok(out) if out.status.success() => {
            let agent_keys = String::from_utf8_lossy(&out.stdout);
            // Match on key type + base64 blob (first two fields)
            let parts: Vec<&str> = key_content.split_whitespace().collect();
            if parts.len() >= 2 {
                let key_id = format!("{} {}", parts[0], parts[1]);
                agent_keys.lines().any(|line| line.contains(&key_id))
            } else {
                false
            }
        }
        _ => false,
    }
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
