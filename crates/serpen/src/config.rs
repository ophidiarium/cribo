use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::combine::Combine;
use crate::dirs::{system_config_file, user_serpen_config_dir};

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

impl Combine for Config {
    fn combine(self, other: Self) -> Self {
        Self {
            // For collections, higher precedence (self) completely replaces lower precedence (other)
            // if self has non-default values, otherwise use other
            src: if self.src != Config::default().src {
                self.src
            } else {
                other.src
            },
            known_first_party: if !self.known_first_party.is_empty() {
                self.known_first_party
            } else {
                other.known_first_party
            },
            known_third_party: if !self.known_third_party.is_empty() {
                self.known_third_party
            } else {
                other.known_third_party
            },
            // For scalars, self always takes precedence
            preserve_comments: self.preserve_comments,
            preserve_type_hints: self.preserve_type_hints,
            target_version: self.target_version,
        }
    }
}

/// Configuration values from environment variables with SERPEN_ prefix
#[derive(Debug, Clone, Default)]
pub struct EnvConfig {
    pub src: Option<Vec<PathBuf>>,
    pub known_first_party: Option<HashSet<String>>,
    pub known_third_party: Option<HashSet<String>>,
    pub preserve_comments: Option<bool>,
    pub preserve_type_hints: Option<bool>,
    pub target_version: Option<String>,
}

impl EnvConfig {
    /// Load configuration from environment variables with SERPEN_ prefix
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // SERPEN_SRC - comma-separated list of source directories
        if let Ok(src_str) = env::var("SERPEN_SRC") {
            let paths: Vec<PathBuf> = src_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect();
            if !paths.is_empty() {
                config.src = Some(paths);
            }
        }

        // SERPEN_KNOWN_FIRST_PARTY - comma-separated list of first-party modules
        if let Ok(first_party_str) = env::var("SERPEN_KNOWN_FIRST_PARTY") {
            let modules: HashSet<String> = first_party_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            if !modules.is_empty() {
                config.known_first_party = Some(modules);
            }
        }

        // SERPEN_KNOWN_THIRD_PARTY - comma-separated list of third-party modules
        if let Ok(third_party_str) = env::var("SERPEN_KNOWN_THIRD_PARTY") {
            let modules: HashSet<String> = third_party_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            if !modules.is_empty() {
                config.known_third_party = Some(modules);
            }
        }

        // SERPEN_PRESERVE_COMMENTS - boolean flag
        if let Ok(preserve_comments_str) = env::var("SERPEN_PRESERVE_COMMENTS") {
            config.preserve_comments = parse_bool(&preserve_comments_str);
        }

        // SERPEN_PRESERVE_TYPE_HINTS - boolean flag
        if let Ok(preserve_type_hints_str) = env::var("SERPEN_PRESERVE_TYPE_HINTS") {
            config.preserve_type_hints = parse_bool(&preserve_type_hints_str);
        }

        // SERPEN_TARGET_VERSION - target Python version
        if let Ok(target_version) = env::var("SERPEN_TARGET_VERSION") {
            config.target_version = Some(target_version);
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
        config
    }
}

/// Parse a boolean value from string, supporting various common formats
fn parse_bool(value: &str) -> Option<bool> {
    match value.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
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

    /// Load a single config file from a path
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Config> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {:?}", path))?;

        // Validate the target version
        config.python_version().with_context(|| {
            format!(
                "Invalid target-version in config file: {}",
                config.target_version
            )
        })?;

        Ok(config)
    }

    /// Load configuration with hierarchical precedence:
    /// 1. CLI-provided config path (highest precedence)
    /// 2. Environment variables (SERPEN_*)
    /// 3. Project config (serpen.toml in current directory)
    /// 4. User config (~/.config/serpen/serpen.toml)
    /// 5. System config (/etc/serpen/serpen.toml or equivalent)
    /// 6. Default values (lowest precedence)
    pub fn load(cli_config_path: Option<&Path>) -> Result<Self> {
        // Start with default configuration
        let mut config = Config::default();

        // 1. Load system config (lowest precedence) - combine into defaults
        if let Some(system_config_path) = system_config_file() {
            if system_config_path.exists() {
                log::debug!("Loading system config from: {:?}", system_config_path);
                let system_config =
                    Self::load_from_file(&system_config_path).with_context(|| {
                        format!("Failed to load system config from {:?}", system_config_path)
                    })?;
                config = system_config.combine(config); // system takes precedence over defaults
            }
        }

        // 2. Load user config
        if let Some(user_config_dir) = user_serpen_config_dir() {
            let user_config_path = user_config_dir.join("serpen.toml");
            if user_config_path.exists() {
                log::debug!("Loading user config from: {:?}", user_config_path);
                let user_config = Self::load_from_file(&user_config_path).with_context(|| {
                    format!("Failed to load user config from {:?}", user_config_path)
                })?;
                config = user_config.combine(config); // user takes precedence over system
            }
        }

        // 3. Load project config (serpen.toml in current directory)
        let project_config_path = PathBuf::from("serpen.toml");
        if project_config_path.exists() {
            log::debug!("Loading project config from: {:?}", project_config_path);
            let project_config = Self::load_from_file(&project_config_path).with_context(|| {
                format!(
                    "Failed to load project config from {:?}",
                    project_config_path
                )
            })?;
            config = project_config.combine(config); // project takes precedence over user
        }

        // 4. Apply environment variables
        let env_config = EnvConfig::from_env();
        config = env_config.apply_to(config);

        // 5. Load CLI-provided config (highest precedence)
        if let Some(cli_config_path) = cli_config_path {
            log::debug!("Loading CLI config from: {:?}", cli_config_path);
            let cli_config = Self::load_from_file(cli_config_path)
                .with_context(|| format!("Failed to load CLI config from {:?}", cli_config_path))?;
            config = cli_config.combine(config); // CLI takes precedence over everything
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
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_target_version() {
        assert_eq!(Config::parse_target_version("py38").unwrap(), 8);
        assert_eq!(Config::parse_target_version("py39").unwrap(), 9);
        assert_eq!(Config::parse_target_version("py310").unwrap(), 10);
        assert_eq!(Config::parse_target_version("py311").unwrap(), 11);
        assert_eq!(Config::parse_target_version("py312").unwrap(), 12);
        assert_eq!(Config::parse_target_version("py313").unwrap(), 13);
        assert!(Config::parse_target_version("py37").is_err());
        assert!(Config::parse_target_version("invalid").is_err());
    }

    #[test]
    fn test_parse_bool() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("on"), Some(true));

        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("FALSE"), Some(false));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("off"), Some(false));

        assert_eq!(parse_bool("invalid"), None);
    }

    #[test]
    fn test_config_combine() {
        let config1 = Config {
            src: vec![PathBuf::from("src1")],
            known_first_party: HashSet::from(["module1".to_string()]),
            preserve_comments: true,
            target_version: "py39".to_string(),
            ..Default::default()
        };

        let config2 = Config {
            src: vec![PathBuf::from("src2")],
            known_first_party: HashSet::from(["module2".to_string()]),
            preserve_comments: false,
            target_version: "py310".to_string(),
            ..Default::default()
        };

        let combined = config1.combine(config2);

        // Higher precedence (config1) should win for all values
        assert!(combined.preserve_comments);
        assert_eq!(combined.target_version, "py39");

        // For collections, higher precedence completely replaces
        assert_eq!(combined.src, vec![PathBuf::from("src1")]);
        assert!(combined.known_first_party.contains("module1"));
        assert!(!combined.known_first_party.contains("module2"));
    }

    #[test]
    #[serial_test::serial]
    fn test_env_config_parsing() {
        // Struct to ensure environment cleanup on panic
        struct EnvGuard {
            vars: Vec<&'static str>,
        }

        impl Drop for EnvGuard {
            fn drop(&mut self) {
                for var in &self.vars {
                    unsafe {
                        env::remove_var(var);
                    }
                }
            }
        }

        let _guard = EnvGuard {
            vars: vec![
                "SERPEN_SRC",
                "SERPEN_KNOWN_FIRST_PARTY",
                "SERPEN_PRESERVE_COMMENTS",
                "SERPEN_TARGET_VERSION",
            ],
        };

        // Test with environment variables set
        unsafe {
            env::set_var("SERPEN_SRC", "src1,src2,src3");
            env::set_var("SERPEN_KNOWN_FIRST_PARTY", "mod1,mod2");
            env::set_var("SERPEN_PRESERVE_COMMENTS", "false");
            env::set_var("SERPEN_TARGET_VERSION", "py312");
        }

        let env_config = EnvConfig::from_env();

        assert_eq!(
            env_config.src,
            Some(vec![
                PathBuf::from("src1"),
                PathBuf::from("src2"),
                PathBuf::from("src3"),
            ])
        );
        assert_eq!(
            env_config.known_first_party,
            Some(HashSet::from(["mod1".to_string(), "mod2".to_string(),]))
        );
        assert_eq!(env_config.preserve_comments, Some(false));
        assert_eq!(env_config.target_version, Some("py312".to_string()));

        // Environment variables are cleaned up automatically by the guard
    }

    #[test]
    fn test_load_from_file() -> anyhow::Result<()> {
        let temp_dir = TempDir::new()?;
        let config_path = temp_dir.path().join("serpen.toml");

        let config_content = r#"
src = ["custom_src"]
known_first_party = ["my_module"]
preserve_comments = false
target-version = "py311"
"#;

        fs::write(&config_path, config_content)?;

        let config = Config::load_from_file(&config_path)?;

        assert_eq!(config.src, vec![PathBuf::from("custom_src")]);
        assert!(config.known_first_party.contains("my_module"));
        assert!(!config.preserve_comments);
        assert_eq!(config.target_version, "py311");

        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn test_hierarchical_config_loading() -> anyhow::Result<()> {
        let temp_dir = TempDir::new()?;

        // Create a project config
        let project_config_path = temp_dir.path().join("serpen.toml");
        fs::write(
            &project_config_path,
            r#"
src = ["project_src"]
preserve_comments = true
target-version = "py310"
"#,
        )?;

        // Change to temp directory with guard for restoration
        let original_dir = env::current_dir()?;
        struct DirGuard(PathBuf);
        impl Drop for DirGuard {
            fn drop(&mut self) {
                let _ = env::set_current_dir(&self.0);
            }
        }
        let _dir_guard = DirGuard(original_dir);
        env::set_current_dir(&temp_dir)?;

        // Environment variable guard to ensure cleanup
        struct EnvGuard;
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                unsafe {
                    env::remove_var("SERPEN_TARGET_VERSION");
                }
            }
        }
        let _env_guard = EnvGuard;

        // Set environment variable
        unsafe {
            env::set_var("SERPEN_TARGET_VERSION", "py312");
        }

        let config = Config::load(None)?;

        // Environment should override project config for target version
        assert_eq!(config.target_version, "py312");
        // Project config should provide other values
        assert_eq!(config.src, vec![PathBuf::from("project_src")]);
        assert!(config.preserve_comments);

        // Environment variable is cleaned up automatically by the guard
        Ok(())
    }
}
