use anyhow::{Context, Result};
use shellexpand::tilde;
use std::{
    fs::{create_dir_all, read_to_string, write},
    path::Path,
};

pub enum ConfigEntry {
    Comment(String),
    Blank,
    Mapping { serial: String, key: String },
}

pub struct Config {
    entries: Vec<ConfigEntry>,
}

impl Config {
    pub fn new() -> Self {
        Config {
            entries: Vec::new(),
        }
    }

    pub fn get(&self, serial: &str) -> Option<&str> {
        self.entries.iter().find_map(|e| match e {
            ConfigEntry::Mapping { serial: s, key } if s == serial => Some(key.as_str()),
            _ => None,
        })
    }

    pub fn insert(&mut self, serial: String, key: String) {
        // Update existing or append
        for entry in &mut self.entries {
            if let ConfigEntry::Mapping {
                serial: s,
                key: k,
            } = entry
            {
                if *s == serial {
                    *k = key;
                    return;
                }
            }
        }
        self.entries.push(ConfigEntry::Mapping { serial, key });
    }

    pub fn is_empty(&self) -> bool {
        !self
            .entries
            .iter()
            .any(|e| matches!(e, ConfigEntry::Mapping { .. }))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

pub fn read_config<P: AsRef<Path>>(path: P) -> Result<Config> {
    let content = read_to_string(&path).with_context(|| {
        format!(
            "failed to read config at {}",
            path.as_ref().to_string_lossy()
        )
    })?;
    let cfg = parse_config(&content)?;
    Ok(cfg)
}

pub fn write_config<P: AsRef<Path>>(path: P, cfg: Config) -> Result<()> {
    let serialised = serialise_config(&cfg);
    let basepath = path
        .as_ref()
        .parent()
        .with_context(|| format!("config path has no parent directory: {}", path.as_ref().to_string_lossy()))?;
    create_dir_all(basepath).with_context(|| {
        format!(
            "failed to create directory tree `{}` for config",
            basepath.to_string_lossy()
        )
    })?;
    write(&path, serialised).with_context(|| {
        format!(
            "failed to write config at {}",
            path.as_ref().to_string_lossy()
        )
    })?;

    Ok(())
}

fn parse_config(content: &str) -> Result<Config> {
    let lines = content.lines().enumerate();
    let mut output = Config::new();

    for (i, line) in lines {
        if line.is_empty() {
            output.entries.push(ConfigEntry::Blank);
            continue;
        }

        if line.starts_with('#') {
            output.entries.push(ConfigEntry::Comment(line.to_owned()));
            continue;
        }

        let (serial, key) = line
            .split_once("::")
            .with_context(|| format!("malformed line {i} in config. expected `<serial>::<file path>`"))?;

        // Validate that the referenced key file exists
        let expanded = tilde(key);
        let key_path = Path::new(expanded.as_ref());
        if !key_path.exists() {
            eprintln!(
                "warning: key file '{}' referenced on line {} does not exist",
                key,
                i + 1
            );
        }

        output.insert(serial.to_owned(), key.to_owned());
    }

    if output.is_empty() {
        anyhow::bail!("config is empty. Use `gfh -a` to import a SSH key");
    }

    Ok(output)
}

fn serialise_config(cfg: &Config) -> String {
    let mut output = String::new();

    for entry in &cfg.entries {
        match entry {
            ConfigEntry::Comment(text) => {
                output.push_str(text);
                output.push('\n');
            }
            ConfigEntry::Blank => {
                output.push('\n');
            }
            ConfigEntry::Mapping { serial, key } => {
                output.push_str(&format!("{}::{}\n", serial, key.replace('\\', "\\\\")));
            }
        }
    }

    output
}
