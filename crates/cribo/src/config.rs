use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use crate::{
    combine::Combine,
    dirs::{system_config_file, user_cribo_config_dir},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Source directories to scan for first-party modules
    pub src: Vec<PathBuf>,

    /// Known first-party module names
    pub known_first_party: IndexSet<String>,

    /// Known third-party module names
    pub known_third_party: IndexSet<String>,

    /// Whether to preserve comments in output
    pub preserve_comments: bool,

    /// Whether to preserve type hints in output
    pub preserve_type_hints: bool,

    /// Target Python version for standard library and builtin checks
    /// Supports Ruff-style string values: "py38", "py39", "py310", "py311", "py312", "py313"
    /// Defaults to "py310" (Python 3.10)
    #[serde(rename = "target-version")]
    pub target_version: String,

    /// Whether to enable tree-shaking to remove unused code
    pub tree_shake: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            src: vec![], // Empty by default - entry directory will be added automatically
            known_first_party: IndexSet::new(),
            known_third_party: IndexSet::new(),
            preserve_comments: true,
            preserve_type_hints: true,
            target_version: "py310".to_owned(),
            tree_shake: true, // Tree-shaking enabled by default
        }
    }
}

impl Combine for Config {
    fn combine(self, other: Self) -> Self {
        Self {
            // For collections, higher precedence (self) completely replaces lower precedence
            // (other) if self has non-default values, otherwise use other
            src: if self.src == Config::default().src {
                other.src
            } else {
                self.src
            },
            known_first_party: if self.known_first_party.is_empty() {
                other.known_first_party
            } else {
                self.known_first_party
            },
            known_third_party: if self.known_third_party.is_empty() {
                other.known_third_party
            } else {
                self.known_third_party
            },
            // For scalars, self always takes precedence
            preserve_comments: self.preserve_comments,
            preserve_type_hints: self.preserve_type_hints,
            target_version: self.target_version,
            tree_shake: self.tree_shake,
        }
    }
}

/// Configuration values from environment variables with CRIBO_ prefix
#[derive(Debug, Clone, Default)]
pub struct EnvConfig {
    pub src: Option<Vec<PathBuf>>,
    pub known_first_party: Option<IndexSet<String>>,
    pub known_third_party: Option<IndexSet<String>>,
    pub preserve_comments: Option<bool>,
    pub preserve_type_hints: Option<bool>,
    pub target_version: Option<String>,
    pub tree_shake: Option<bool>,
}

impl EnvConfig {
    /// Load configuration from environment variables with CRIBO_ prefix
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // CRIBO_SRC - comma-separated list of source directories
        if let Ok(src_str) = env::var("CRIBO_SRC") {
            let paths: Vec<PathBuf> = src_str
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect();
            if !paths.is_empty() {
                config.src = Some(paths);
            }
        }

        // CRIBO_KNOWN_FIRST_PARTY - comma-separated list of first-party modules
        if let Ok(first_party_str) = env::var("CRIBO_KNOWN_FIRST_PARTY") {
            let modules: IndexSet<String> = first_party_str
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(std::borrow::ToOwned::to_owned)
                .collect();
            if !modules.is_empty() {
                config.known_first_party = Some(modules);
            }
        }

        // CRIBO_KNOWN_THIRD_PARTY - comma-separated list of third-party modules
        if let Ok(third_party_str) = env::var("CRIBO_KNOWN_THIRD_PARTY") {
            let modules: IndexSet<String> = third_party_str
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(std::borrow::ToOwned::to_owned)
                .collect();
            if !modules.is_empty() {
                config.known_third_party = Some(modules);
            }
        }

        // CRIBO_PRESERVE_COMMENTS - boolean flag
        if let Ok(preserve_comments_str) = env::var("CRIBO_PRESERVE_COMMENTS") {
            config.preserve_comments = parse_bool(&preserve_comments_str);
        }

        // CRIBO_PRESERVE_TYPE_HINTS - boolean flag
        if let Ok(preserve_type_hints_str) = env::var("CRIBO_PRESERVE_TYPE_HINTS") {
            config.preserve_type_hints = parse_bool(&preserve_type_hints_str);
        }

        // CRIBO_TARGET_VERSION - target Python version
        if let Ok(target_version) = env::var("CRIBO_TARGET_VERSION") {
            config.target_version = Some(target_version);
        }

        // CRIBO_TREE_SHAKE - boolean flag
        if let Ok(tree_shake_str) = env::var("CRIBO_TREE_SHAKE") {
            config.tree_shake = parse_bool(&tree_shake_str);
        }

        config
    }

    /// Apply environment config to base config
    pub fn apply_to(self, mut config: Config) -> Config {
        if let Some(src) = self.src {
            config.src = src;
        }
        if let Some(known_first_party) = self.known_first_party {
            config.known_first_party = known_first_party;
        }
        if let Some(known_third_party) = self.known_third_party {
            config.known_third_party = known_third_party;
        }
        if let Some(preserve_comments) = self.preserve_comments {
            config.preserve_comments = preserve_comments;
        }
        if let Some(preserve_type_hints) = self.preserve_type_hints {
            config.preserve_type_hints = preserve_type_hints;
        }
        if let Some(target_version) = self.target_version {
            config.target_version = target_version;
        }
        if let Some(tree_shake) = self.tree_shake {
            config.tree_shake = tree_shake;
        }
        config
    }
}

/// Parse a boolean value from string, supporting various common formats
fn parse_bool(value: &str) -> Option<bool> {
    use cow_utils::CowUtils;
    match value.cow_to_lowercase().as_ref() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl Config {
    /// Parse a Ruff-style target version string to u8 version number
    /// Supports: "py38" -> 8, "py39" -> 9, "py310" -> 10, "py311" -> 11, "py312" -> 12, "py313" ->
    /// 13
    pub fn parse_target_version(version_str: &str) -> Result<u8> {
        match version_str {
            "py38" => Ok(8),
            "py39" => Ok(9),
            "py310" => Ok(10),
            "py311" => Ok(11),
            "py312" => Ok(12),
            "py313" => Ok(13),
            _ => Err(anyhow!(
                "Invalid target version '{}'. Supported versions: py38, py39, py310, py311, \
                 py312, py313",
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

    /// Load a single config file from a path
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Config> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        // Validate the target version
        config.python_version().with_context(|| {
            format!(
                "Invalid target-version in config file: {}",
                config.target_version
            )
        })?;

        Ok(config)
    }

    fn try_load_and_combine<P: AsRef<Path>>(
        config: &mut Config,
        path: P,
        context: &str,
    ) -> Result<()> {
        if path.as_ref().exists() {
            log::debug!("Loading {} from: {}", context, path.as_ref().display());
            let loaded = Self::load_from_file(&path).with_context(|| {
                format!(
                    "Failed to load {} from {}",
                    context,
                    path.as_ref().display()
                )
            })?;
            *config = loaded.combine(config.clone());
        }
        Ok(())
    }

    /// Load configuration with hierarchical precedence:
    /// 1. CLI-provided config path (highest precedence)
    /// 2. Environment variables (CRIBO_*)
    /// 3. Project config (cribo.toml in current directory)
    /// 4. User config (~/.config/cribo/cribo.toml)
    /// 5. System config (/etc/cribo/cribo.toml or equivalent)
    /// 6. Default values (lowest precedence)
    pub fn load(cli_config_path: Option<&Path>) -> Result<Self> {
        let mut config = Config::default();

        // 1. Load system config (lowest precedence)
        if let Some(system_config_path) = system_config_file() {
            Self::try_load_and_combine(&mut config, &system_config_path, "system config")?;
        }

        // 2. Load user config
        if let Some(user_config_dir) = user_cribo_config_dir() {
            let user_config_path = user_config_dir.join("cribo.toml");
            Self::try_load_and_combine(&mut config, &user_config_path, "user config")?;
        }

        // 3. Load project config (cribo.toml in current directory)
        let project_config_path = PathBuf::from("cribo.toml");
        Self::try_load_and_combine(&mut config, &project_config_path, "project config")?;

        // 4. Apply environment variables
        let env_config = EnvConfig::from_env();
        config = env_config.apply_to(config);

        // 5. Load CLI-provided config (highest precedence)
        if let Some(cli_config_path) = cli_config_path {
            Self::try_load_and_combine(&mut config, cli_config_path, "CLI config")?;
        }

        // Final validation
        config.python_version().with_context(|| {
            format!(
                "Invalid target-version in final config: {}",
                config.target_version
            )
        })?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn test_target_version_configuration() {
        // Test default behavior (should be "py310")
        let default_config = Config::default();
        assert_eq!(default_config.target_version, "py310");
        assert_eq!(
            default_config
                .python_version()
                .expect("default config should have valid python version"),
            10
        );

        // Test setting target version programmatically
        let mut config = Config::default();
        config
            .set_target_version("py311".to_string())
            .expect("py311 should be a valid target version");
        assert_eq!(config.target_version, "py311");
        assert_eq!(
            config
                .python_version()
                .expect("config should have valid python version after setting"),
            11
        );

        // Test parsing various valid versions
        assert_eq!(
            Config::parse_target_version("py38").expect("py38 should parse to version 8"),
            8
        );
        assert_eq!(
            Config::parse_target_version("py39").expect("py39 should parse to version 9"),
            9
        );
        assert_eq!(
            Config::parse_target_version("py310").expect("py310 should parse to version 10"),
            10
        );
        assert_eq!(
            Config::parse_target_version("py311").expect("py311 should parse to version 11"),
            11
        );
        assert_eq!(
            Config::parse_target_version("py312").expect("py312 should parse to version 12"),
            12
        );
        assert_eq!(
            Config::parse_target_version("py313").expect("py313 should parse to version 13"),
            13
        );

        // Test invalid version strings
        assert!(Config::parse_target_version("invalid").is_err());
        assert!(Config::parse_target_version("py37").is_err()); // too old
        assert!(Config::parse_target_version("py314").is_err()); // too new
        assert!(Config::parse_target_version("3.10").is_err()); // wrong format
    }

    #[test]
    fn test_toml_config_loading() {
        // Test loading target-version from TOML config
        let toml_content = r#"
target-version = "py312"
preserve_comments = false
src = ["src", "lib"]
        "#;

        let mut temp_file =
            NamedTempFile::new().expect("should be able to create temp file for test");
        temp_file
            .write_all(toml_content.as_bytes())
            .expect("should be able to write test config to temp file");

        let config = Config::load(Some(temp_file.path()))
            .expect("should be able to load valid config from temp file");
        assert_eq!(config.target_version, "py312");
        assert_eq!(
            config
                .python_version()
                .expect("loaded config should have valid python version"),
            12
        );
        assert!(!config.preserve_comments);
    }

    #[test]
    fn test_invalid_toml_config() {
        // Test invalid target-version in TOML config
        let toml_content = r#"
        target-version = "invalid_version"
        "#;

        let mut temp_file = NamedTempFile::new()
            .expect("should be able to create temp file for invalid config test");
        temp_file
            .write_all(toml_content.as_bytes())
            .expect("should be able to write invalid test config to temp file");

        let result = Config::load(Some(temp_file.path()));
        assert!(result.is_err());
        let error_message = result.unwrap_err().to_string();
        // The error should mention either loading config or invalid target version
        assert!(
            error_message.contains("Failed to load CLI config")
                || error_message.contains("Invalid target version")
                || error_message.contains("Invalid target-version")
        );
    }
}
