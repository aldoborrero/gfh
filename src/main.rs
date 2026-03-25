use anyhow::{Context, Result};
use clap::Parser;
use shellexpand::tilde;
use std::fs;

mod add_key;
mod config;
mod util;
mod yubikey;

fn default_config_path() -> String {
    dirs::config_dir()
        .map(|p| p.join("gfh").join("keys").to_string_lossy().into_owned())
        .unwrap_or_else(|| "~/.config/gfh/keys".to_owned())
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = default_config_path())]
    file: String,

    #[arg(short, long)]
    add: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let path = tilde(&args.file).into_owned();

    if args.add {
        return add_key::run(path);
    }

    let cfg = config::read_config(&path)?;

    if cfg.is_empty() {
        anyhow::bail!("config is empty. Use `gfh -a` to import a SSH key");
    }

    let devices = util::get_all_devices()?;

    let selected = devices
        .iter()
        .find_map(|y| cfg.get(&y.serial()))
        .with_context(|| format!("no matching FIDO key found in the config at {path}"))?;

    // Read the public key file and output its content
    let key_path = tilde(selected).into_owned();
    let pub_path = if key_path.ends_with(".pub") {
        key_path
    } else {
        format!("{}.pub", key_path)
    };
    let key_content = fs::read_to_string(&pub_path)
        .with_context(|| format!("failed to read public key at {}", pub_path))?;
    let key_content = key_content.trim();

    if !util::is_key_in_agent(key_content) {
        eprintln!("warning: signing key not found in ssh-agent. Run `ssh-add -K` to load keys from your FIDO device.");
    }

    println!("key::{}", key_content);

    Ok(())
}
