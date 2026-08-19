use anyhow::{Context, Result};
use yubikey_api::Context as YKContext;

use crate::util::FidoDevice;

/// Enumerate YubiKeys over PC/SC.
///
/// An error means the PC/SC layer itself is unavailable — pcscd stopped, the
/// CCID interface disabled — which is a different thing from an empty result,
/// meaning no YubiKey is plugged in. Callers need to tell the two apart.
pub fn get_yubikeys() -> Result<Vec<FidoDevice>> {
    let mut readers = YKContext::open().context("could not open a PC/SC context")?;
    let iter = readers.iter().context("could not list PC/SC readers")?;

    let mut output = Vec::new();
    for reader in iter {
        if reader
            .name()
            .as_ref()
            .to_ascii_lowercase()
            .contains("yubikey")
        {
            match reader.open() {
                Ok(yubikey) => output.push(FidoDevice::YubiKey(yubikey)),
                Err(e) => eprintln!("warning: failed to open yubikey {}: {}", reader.name(), e),
            }
        }
    }

    Ok(output)
}
