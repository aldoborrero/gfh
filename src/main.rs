use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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
    #[arg(short, long, global = true, default_value_t = default_config_path())]
    file: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Import a FIDO device and associate it with an SSH key
    Add,
    /// List configured device-to-key mappings and their status
    List,
    /// Remove a device mapping from the config
    Remove,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let path = tilde(&args.file).into_owned();

    match args.command {
        Some(Command::Add) => add_key::run(path),
        Some(Command::List) => cmd_list(&path),
        Some(Command::Remove) => cmd_remove(&path),
        None => cmd_sign(&path),
    }
}

/// Default: output the signing key for git's defaultKeyCommand
fn cmd_sign(path: &str) -> Result<()> {
    let cfg = config::read_config(path)?;

    if cfg.is_empty() {
        anyhow::bail!("config is empty. Use `gfh add` to import a SSH key");
    }

    let devices = util::get_all_devices()?;

    let selected = devices
        .iter()
        .find_map(|y| cfg.get(&y.serial()))
        .with_context(|| format!("no matching FIDO key found in the config at {path}"))?;

    let key_path = tilde(selected).into_owned();
    let (priv_path, pub_path) = match key_path.strip_suffix(".pub") {
        Some(stripped) => (stripped.to_owned(), key_path),
        None => (key_path.clone(), format!("{}.pub", key_path)),
    };
    let key_content = fs::read_to_string(&pub_path)
        .with_context(|| format!("failed to read public key at {}", pub_path))?;
    let key_content = key_content.trim();

    // git signs via `ssh-keygen -Y sign -U`, which needs the key in the agent.
    if !util::is_key_in_agent(key_content, None) {
        if std::path::Path::new(&priv_path).exists() {
            util::load_key_into_agent(&priv_path, key_content)?;
        } else {
            // A resident credential need not have a local private file. Only the
            // token can supply it, so warn and let git report the failure rather
            // than aborting a commit we cannot diagnose.
            eprintln!(
                "warning: signing key is not in the ssh-agent and {priv_path} does not exist.\n\
                 If it is a resident credential, load it with: ssh-add -K"
            );
        }
    }

    println!("key::{}", key_content);
    Ok(())
}

/// List all configured mappings with connection status
fn cmd_list(path: &str) -> Result<()> {
    let cfg = if std::path::Path::new(path).exists() {
        config::read_config(path)?
    } else {
        eprintln!("No config file found at {path}. Use `gfh add` to get started.");
        return Ok(());
    };

    if cfg.is_empty() {
        eprintln!("Config is empty. Use `gfh add` to import a SSH key.");
        return Ok(());
    }

    let devices = util::get_all_devices()?;
    let connected_serials: Vec<String> = devices.iter().map(|d| d.serial()).collect();

    for entry in cfg.entries() {
        if let config::ConfigEntry::Mapping { serial, key } = entry {
            let connected = if connected_serials.contains(serial) {
                "connected"
            } else {
                "not connected"
            };

            let expanded = tilde(key);
            let key_exists = std::path::Path::new(expanded.as_ref()).exists();
            let key_status = if key_exists {
                ""
            } else {
                " (key file missing)"
            };

            println!("{serial} :: {key} [{connected}]{key_status}");
        }
    }

    Ok(())
}

/// Remove a device mapping interactively
fn cmd_remove(path: &str) -> Result<()> {
    let cfg = if std::path::Path::new(path).exists() {
        config::read_config(path)?
    } else {
        anyhow::bail!("no config file found at {path}");
    };

    if cfg.is_empty() {
        anyhow::bail!("config is empty, nothing to remove");
    }

    let mappings: Vec<(String, String)> = cfg
        .entries()
        .iter()
        .filter_map(|e| match e {
            config::ConfigEntry::Mapping { serial, key } => Some((serial.clone(), key.clone())),
            _ => None,
        })
        .collect();

    let labels: Vec<String> = mappings
        .iter()
        .map(|(s, k)| format!("{s} :: {k}"))
        .collect();

    let selected = inquire::Select::new("Select mapping to remove:", labels)
        .prompt()
        .with_context(|| "failed to create selection input")?;

    let idx = mappings
        .iter()
        .position(|(s, k)| format!("{s} :: {k}") == selected)
        .unwrap();
    let (serial, _) = &mappings[idx];

    let new_cfg = cfg.without(serial);
    config::write_config(path, new_cfg)?;
    println!("Removed mapping for serial {serial}.");

    Ok(())
}
