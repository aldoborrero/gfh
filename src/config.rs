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
        for entry in &mut self.entries {
            if let ConfigEntry::Mapping { serial: s, key: k } = entry {
                if *s == serial {
                    *k = key;
                    return;
                }
            }
        }
        self.entries.push(ConfigEntry::Mapping { serial, key });
    }

    pub fn push_comment(&mut self, text: String) {
        self.entries.push(ConfigEntry::Comment(text));
    }

    pub fn push_blank(&mut self) {
        self.entries.push(ConfigEntry::Blank);
    }

    pub fn entries(&self) -> &[ConfigEntry] {
        &self.entries
    }

    pub fn without(self, serial: &str) -> Self {
        Config {
            entries: self
                .entries
                .into_iter()
                .filter(|e| !matches!(e, ConfigEntry::Mapping { serial: s, .. } if s == serial))
                .collect(),
        }
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
    let basepath = path.as_ref().parent().with_context(|| {
        format!(
            "config path has no parent directory: {}",
            path.as_ref().to_string_lossy()
        )
    })?;
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
            output.push_blank();
            continue;
        }

        if line.starts_with('#') {
            output.push_comment(line.to_owned());
            continue;
        }

        let (serial, key) = line.split_once("::").with_context(|| {
            format!("malformed line {i} in config. expected `<serial>::<file path>`")
        })?;

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

    Ok(output)
}

fn serialise_config(cfg: &Config) -> String {
    let mut output = String::new();

    for entry in cfg.entries() {
        match entry {
            ConfigEntry::Comment(text) => {
                output.push_str(text);
                output.push('\n');
            }
            ConfigEntry::Blank => {
                output.push('\n');
            }
            ConfigEntry::Mapping { serial, key } => {
                output.push_str(&format!("{}::{}\n", serial, key));
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_config() {
        let input = "12345678::~/.ssh/id_ed25519_sk\n87654321::~/.ssh/id_ecdsa_sk\n";
        let cfg = parse_config(input).unwrap();
        assert_eq!(cfg.get("12345678"), Some("~/.ssh/id_ed25519_sk"));
        assert_eq!(cfg.get("87654321"), Some("~/.ssh/id_ecdsa_sk"));
        assert!(!cfg.is_empty());
    }

    #[test]
    fn parse_empty_input_returns_empty_config() {
        let cfg = parse_config("").unwrap();
        assert!(cfg.is_empty());
    }

    #[test]
    fn parse_only_comments_returns_empty_config() {
        let input = "# comment one\n# comment two\n";
        let cfg = parse_config(input).unwrap();
        assert!(cfg.is_empty());
    }

    #[test]
    fn parse_malformed_line_errors() {
        let input = "no-separator-here\n";
        assert!(parse_config(input).is_err());
    }

    #[test]
    fn parse_preserves_comments_and_blanks() {
        let input = "# my keys\n\n12345678::~/.ssh/key\n";
        let cfg = parse_config(input).unwrap();

        assert!(matches!(&cfg.entries()[0], ConfigEntry::Comment(s) if s == "# my keys"));
        assert!(matches!(&cfg.entries()[1], ConfigEntry::Blank));
        assert!(
            matches!(&cfg.entries()[2], ConfigEntry::Mapping { serial, .. } if serial == "12345678")
        );
    }

    #[test]
    fn roundtrip_preserves_content() {
        let input = "# Primary YubiKey\n12345678::~/.ssh/id_ed25519_sk\n\n# Backup\n87654321::~/.ssh/id_ecdsa_sk\n";
        let cfg = parse_config(input).unwrap();
        let output = serialise_config(&cfg);
        assert_eq!(input, output);
    }

    #[test]
    fn insert_updates_existing_key() {
        let input = "12345678::~/.ssh/old_key\n";
        let mut cfg = parse_config(input).unwrap();
        cfg.insert("12345678".to_owned(), "~/.ssh/new_key".to_owned());
        assert_eq!(cfg.get("12345678"), Some("~/.ssh/new_key"));
    }

    #[test]
    fn insert_appends_new_key() {
        let mut cfg = Config::new();
        cfg.insert("12345678".to_owned(), "~/.ssh/key".to_owned());
        assert_eq!(cfg.get("12345678"), Some("~/.ssh/key"));
        assert!(!cfg.is_empty());
    }

    #[test]
    fn get_unknown_serial_returns_none() {
        let input = "12345678::~/.ssh/key\n";
        let cfg = parse_config(input).unwrap();
        assert_eq!(cfg.get("99999999"), None);
    }

    #[test]
    fn default_config_is_empty() {
        let cfg = Config::default();
        assert!(cfg.is_empty());
        assert_eq!(cfg.get("anything"), None);
    }

    #[test]
    fn without_removes_mapping() {
        let input = "12345678::~/.ssh/key_a\n87654321::~/.ssh/key_b\n";
        let cfg = parse_config(input).unwrap();
        let cfg = cfg.without("12345678");
        assert_eq!(cfg.get("12345678"), None);
        assert_eq!(cfg.get("87654321"), Some("~/.ssh/key_b"));
    }

    #[test]
    fn without_preserves_comments() {
        let input = "# keep me\n12345678::~/.ssh/key\n";
        let cfg = parse_config(input).unwrap();
        let cfg = cfg.without("12345678");
        assert!(cfg.is_empty());
        assert!(matches!(&cfg.entries()[0], ConfigEntry::Comment(s) if s == "# keep me"));
    }
}
