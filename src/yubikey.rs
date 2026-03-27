use yubikey_api::Context as YKContext;

use crate::util::FidoDevice;

pub fn get_yubikeys() -> Vec<FidoDevice> {
    let mut readers = match YKContext::open() {
        Ok(ctx) => ctx,
        Err(_) => return Vec::new(),
    };

    let iter = match readers.iter() {
        Ok(iter) => iter,
        Err(_) => return Vec::new(),
    };

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

    output
}
