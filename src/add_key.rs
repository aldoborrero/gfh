use anyhow::{Context, Result, anyhow};
use inquire::{Select, Text};
use shellexpand::tilde;
use std::{fs, path::Path};

use crate::{config, util};

pub fn run<P: AsRef<Path>>(path: P) -> Result<()> {
    let keys = util::get_all_devices()?;

    if keys.is_empty() {
        return Err(anyhow!("was not able to find any connected FIDO keys"));
    }

    let key = if keys.len() == 1 {
        let device = keys.into_iter().next().unwrap();
        eprintln!("Auto-selected the only connected device: {}", device);
        device
    } else {
        Select::new("Select which key to import:", keys)
            .prompt()
            .with_context(|| "failed to create key selection input")?
    };

    let ssh_key = loop {
        let answer = Text::new("Path to the associated SSH key:")
            .prompt()
            .with_context(|| "failed to create key path text input")?;
        let expanded = tilde(&answer).into_owned();

        match fs::File::open(&expanded) {
            Ok(_) => {
                let tmp = fs::canonicalize(expanded)
                    .with_context(|| "failed to canonicalize SSH key path")?;
                break tmp
                    .to_str()
                    .with_context(|| "SSH key path contains invalid UTF-8")?
                    .to_owned();
            }
            Err(_) => println!("Failed to read file. Try something else."),
        }
    };

    let mut cfg = if path.as_ref().exists() {
        config::read_config(&path)?
    } else {
        config::Config::default()
    };

    // Without a serial there is nothing to key the mapping on, and a placeholder
    // would make this device match every other serial-less one.
    let serial = key
        .serial()
        .with_context(|| format!("{key} reports no serial number, so it cannot be mapped"))?;

    cfg.insert(serial, ssh_key);
    config::write_config(path, &cfg)?;
    println!("Success!");

    Ok(())
}
