use anyhow::{anyhow, Context, Result};
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

    let mut cfg =
        config::read_config(&path).or_else(|e| match e.downcast_ref::<std::io::Error>() {
            None => Err(e),
            Some(inner) => match inner.kind() {
                std::io::ErrorKind::NotFound => Ok(config::Config::default()),
                _ => Err(e),
            },
        })?;

    cfg.insert(key.serial(), ssh_key);
    config::write_config(path, cfg)?;
    println!("Success!");

    Ok(())
}
