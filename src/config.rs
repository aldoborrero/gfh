use anyhow::{Context, Result};
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
            if let ConfigEntry::Mapping { serial: s, key: k } = entry
                && *s == serial
            {
                *k = key;
                return;
            }
        }
        self.entries.push(ConfigEntry::Mapping { serial, key });
    }

    /// Append a mapping verbatim, keeping duplicates.
    ///
    /// Parsing must not mutate: `insert` upserts, which is right for `gfh add`
    /// and silently drops a line the user wrote when reading a file back.
    fn push_mapping(&mut self, serial: String, key: String) {
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

    /// Remove the nth mapping, leaving comments, blanks and other mappings.
    ///
    /// Indexed rather than keyed by serial: a config may hold duplicate serials,
    /// and removing by serial would delete every one of them.
    pub fn remove_nth_mapping(&mut self, n: usize) -> bool {
        let Some(pos) = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, ConfigEntry::Mapping { .. }))
            .map(|(i, _)| i)
            .nth(n)
        else {
            return false;
        };
        self.entries.remove(pos);
        true
    }

    /// Whether the config declares no device at all. Comments and blank lines
    /// may still be present, so this is not `entries().is_empty()`.
    pub fn has_no_mappings(&self) -> bool {
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

pub fn write_config<P: AsRef<Path>>(path: P, cfg: &Config) -> Result<()> {
    // A declaratively managed config is a symlink into a read-only store, and
    // the write would surface as a bare EROFS on a path the user never typed.
    if let Ok(target) = std::fs::canonicalize(&path)
        && std::fs::metadata(&target).is_ok_and(|m| m.permissions().readonly())
    {
        anyhow::bail!(
            "config at {} is not writable: it resolves to {}.\n\
             If it is generated (home-manager, chezmoi, a dotfiles repo), edit the \
             source that produces it instead of using `gfh add` or `gfh remove`.",
            path.as_ref().to_string_lossy(),
            target.to_string_lossy()
        );
    }

    let serialised = serialise_config(cfg);
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
        let trimmed = line.trim();

        if trimmed.is_empty() {
            output.push_blank();
            continue;
        }

        if trimmed.starts_with('#') {
            output.push_comment(line.to_owned());
            continue;
        }

        let (serial, key) = trimmed.split_once("::").with_context(|| {
            format!(
                "malformed line {} in config. expected `<serial>::<file path>`",
                i + 1
            )
        })?;

        // `gfh list` renders mappings as `<serial> :: <path>`; without trimming,
        // pasting that back stores a serial that can never match a device.
        let serial = serial.trim();
        let key = key.trim();

        if output.get(serial).is_some() {
            eprintln!(
                "warning: duplicate serial {serial} on line {}; the first mapping is used",
                i + 1
            );
        }

        output.push_mapping(serial.to_owned(), key.to_owned());
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
        assert!(!cfg.has_no_mappings());
    }

    #[test]
    fn parse_empty_input_returns_empty_config() {
        let cfg = parse_config("").unwrap();
        assert!(cfg.has_no_mappings());
    }

    #[test]
    fn parse_only_comments_returns_empty_config() {
        let input = "# comment one\n# comment two\n";
        let cfg = parse_config(input).unwrap();
        assert!(cfg.has_no_mappings());
    }

    #[test]
    fn parse_malformed_line_errors() {
        let input = "no-separator-here\n";
        assert!(parse_config(input).is_err());
    }

    #[test]
    fn parse_trims_around_delimiter() {
        // The format `gfh list` prints, pasted back into the config.
        let cfg = parse_config("12345678 :: ~/.ssh/key\n").unwrap();
        assert_eq!(cfg.get("12345678"), Some("~/.ssh/key"));
    }

    #[test]
    fn parse_treats_whitespace_only_line_as_blank() {
        let cfg = parse_config("12345678::~/.ssh/key\n   \n").unwrap();
        assert_eq!(cfg.get("12345678"), Some("~/.ssh/key"));
        assert!(matches!(&cfg.entries()[1], ConfigEntry::Blank));
    }

    #[test]
    fn parse_keeps_duplicate_serials() {
        // insert() upserts, which silently dropped the first line on read and
        // let `gfh remove` write the truncated file back.
        let input = "11111111::~/.ssh/key_a\n11111111::~/.ssh/key_b\n22222222::~/.ssh/key_c\n";
        let cfg = parse_config(input).unwrap();

        assert_eq!(cfg.entries().len(), 3);
        assert_eq!(serialise_config(&cfg), input);
        // Lookup takes the first mapping, matching the warning the parser prints.
        assert_eq!(cfg.get("11111111"), Some("~/.ssh/key_a"));
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
        assert!(!cfg.has_no_mappings());
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
        assert!(cfg.has_no_mappings());
        assert_eq!(cfg.get("anything"), None);
    }

    #[test]
    fn remove_nth_mapping_removes_only_that_one() {
        let input = "12345678::~/.ssh/key_a\n87654321::~/.ssh/key_b\n";
        let mut cfg = parse_config(input).unwrap();
        assert!(cfg.remove_nth_mapping(0));
        assert_eq!(cfg.get("12345678"), None);
        assert_eq!(cfg.get("87654321"), Some("~/.ssh/key_b"));
    }

    #[test]
    fn remove_nth_mapping_keeps_duplicate_siblings() {
        // Removing by serial would delete both lines; the user picked one.
        let input = "11111111::~/.ssh/key_a\n11111111::~/.ssh/key_b\n";
        let mut cfg = parse_config(input).unwrap();
        assert!(cfg.remove_nth_mapping(0));
        assert_eq!(cfg.entries().len(), 1);
        assert_eq!(cfg.get("11111111"), Some("~/.ssh/key_b"));
    }

    #[test]
    fn remove_nth_mapping_skips_comments_and_blanks() {
        let input = "# keep me\n\n12345678::~/.ssh/key\n";
        let mut cfg = parse_config(input).unwrap();
        // Index 0 is the first *mapping*, not the first entry.
        assert!(cfg.remove_nth_mapping(0));
        assert!(cfg.has_no_mappings());
        assert!(matches!(&cfg.entries()[0], ConfigEntry::Comment(s) if s == "# keep me"));
        assert!(matches!(&cfg.entries()[1], ConfigEntry::Blank));
    }

    #[test]
    fn remove_nth_mapping_out_of_range_is_a_no_op() {
        let mut cfg = parse_config("12345678::~/.ssh/key\n").unwrap();
        assert!(!cfg.remove_nth_mapping(7));
        assert_eq!(cfg.entries().len(), 1);
    }
}
