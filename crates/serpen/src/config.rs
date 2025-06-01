use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Source directories to scan for first-party modules
    pub src: Vec<PathBuf>,

    /// Known first-party module names
    pub known_first_party: HashSet<String>,

    /// Known third-party module names
    pub known_third_party: HashSet<String>,

    /// Whether to preserve comments in output
    pub preserve_comments: bool,

    /// Whether to preserve type hints in output
    pub preserve_type_hints: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            src: vec![PathBuf::from("src"), PathBuf::from(".")],
            known_first_party: HashSet::new(),
            known_third_party: HashSet::new(),
            preserve_comments: true,
            preserve_type_hints: true,
        }
    }
}

impl Config {
    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let config_file = config_path.map(|p| p.to_path_buf()).or_else(|| {
            // Look for serpen.toml in current directory
            let path = PathBuf::from("serpen.toml");
            if path.exists() { Some(path) } else { None }
        });

        if let Some(config_file) = config_file {
            let content = std::fs::read_to_string(&config_file)
                .with_context(|| format!("Failed to read config file: {:?}", config_file))?;

            let config: Config = toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file: {:?}", config_file))?;

            Ok(config)
        } else {
            Ok(Config::default())
        }
    }
}
