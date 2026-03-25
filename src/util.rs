use anyhow::Result;
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
    let devices = ctap_hid_fido2::get_fidokey_devices();
    let devices = devices
        .iter()
        .map(|x| FidoDevice::Generic(x.to_owned()))
        .collect::<Vec<FidoDevice>>();
    devices
}

pub fn get_all_devices() -> Result<Vec<FidoDevice>> {
    let fidos = get_generics();
    let mut yubikeys = yubikey::get_yubikeys();
    let mut fidos: Vec<FidoDevice> = fidos
        .into_iter()
        .filter(|x| match x {
            FidoDevice::Generic(h) => !h.product_string.to_lowercase().contains("yubikey"),
            _ => false,
        })
        .collect();

    fidos.append(&mut yubikeys);
    Ok(fidos)
}

pub fn is_key_in_agent(key_content: &str) -> bool {
    let output = std::process::Command::new("ssh-add")
        .arg("-L")
        .output();
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
