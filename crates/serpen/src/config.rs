use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

    /// Target Python version for standard library and builtin checks
    /// Supports Ruff-style string values: "py38", "py39", "py310", "py311", "py312", "py313"
    /// Defaults to "py310" (Python 3.10)
    #[serde(rename = "target-version")]
    pub target_version: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            src: vec![PathBuf::from("src"), PathBuf::from(".")],
            known_first_party: HashSet::new(),
            known_third_party: HashSet::new(),
            preserve_comments: true,
            preserve_type_hints: true,
            target_version: "py310".to_string(),
        }
    }
}

impl Config {
    /// Parse a Ruff-style target version string to u8 version number
    /// Supports: "py38" -> 8, "py39" -> 9, "py310" -> 10, "py311" -> 11, "py312" -> 12, "py313" -> 13
    pub fn parse_target_version(version_str: &str) -> Result<u8> {
        match version_str {
            "py38" => Ok(8),
            "py39" => Ok(9),
            "py310" => Ok(10),
            "py311" => Ok(11),
            "py312" => Ok(12),
            "py313" => Ok(13),
            _ => Err(anyhow!(
                "Invalid target version '{}'. Supported versions: py38, py39, py310, py311, py312, py313",
                version_str
            )),
        }
    }

    /// Get the Python version as u8 for compatibility with existing code
    pub fn python_version(&self) -> Result<u8> {
        Self::parse_target_version(&self.target_version)
    }

    /// Set the target version from a string value
    pub fn set_target_version(&mut self, version: String) -> Result<()> {
        // Validate the version string
        Self::parse_target_version(&version)?;
        self.target_version = version;
        Ok(())
    }

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

            // Validate the target version
            config.python_version().with_context(|| {
                format!(
                    "Invalid target-version in config file: {}",
                    config.target_version
                )
            })?;

            Ok(config)
        } else {
            Ok(Config::default())
        }
    }
}
