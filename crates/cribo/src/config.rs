use std::{
    env, fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use indexmap::{IndexMap, IndexSet};
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

    /// Whether to bundle third-party (site-packages) dependencies into the output.
    /// Packages that contain native extensions (.so/.pyd) are automatically kept
    /// external and emitted into requirements.txt instead.
    /// Kept as `Option` so layered configs only override when the key is present;
    /// use [`Config::bundle_third_party`] to read the effective value.
    #[serde(rename = "bundle-third-party", alias = "bundle_third_party")]
    pub bundle_third_party: Option<bool>,

    /// Source map delivery mode. `None` disables source map generation (default).
    /// See `docs/source-maps.md`.
    pub sourcemap: Option<SourceMapMode>,

    /// Whether to embed original source text in the map as `sourcesContent`.
    /// `None` selects the mode-dependent default (omitted for `inline`,
    /// included for `linked`/`external`).
    #[serde(rename = "sources-content", alias = "sources_content")]
    pub sources_content: Option<bool>,

    /// Configuration for mapping imports to installable requirements
    pub requirements: RequirementsConfig,
}

/// Source map delivery mode, mirroring esbuild's `--sourcemap` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum SourceMapMode {
    /// Write `<output>.map` next to the bundle and append a
    /// `# sourceMappingURL=<basename>.map` comment (the default for a bare flag).
    Linked,
    /// Embed the map as a base64 data-URL comment at the end of the bundle.
    Inline,
    /// Write `<output>.map` with no comment in the bundle.
    External,
}

impl Config {
    /// Effective `sourcesContent` policy for source map generation.
    ///
    /// An explicit `sources-content` setting wins; otherwise the mode default
    /// applies — omitted for `inline` (keeps the bundle small), included for
    /// `linked`/`external` (self-contained maps).
    pub fn include_sources_content(&self) -> bool {
        self.sources_content.unwrap_or(match self.sourcemap {
            Some(SourceMapMode::Linked | SourceMapMode::External) => true,
            Some(SourceMapMode::Inline) | None => false,
        })
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RequirementsConfig {
    /// Python interpreter whose environment supplies distribution metadata
    pub python: Option<PathBuf>,

    /// Explicit import-prefix to PEP 508 requirement mappings
    #[serde(rename = "module-map")]
    pub module_map: IndexMap<String, String>,
}

struct RedactedModuleMap(usize);

impl fmt::Debug for RedactedModuleMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "<redacted: {} entries>", self.0)
    }
}

impl fmt::Debug for RequirementsConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequirementsConfig")
            .field("python", &self.python)
            .field("module_map", &RedactedModuleMap(self.module_map.len()))
            .finish()
    }
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
            tree_shake: true,         // Tree-shaking enabled by default
            bundle_third_party: None, // Opt-in: third-party deps stay external by default
            sourcemap: None,          // Opt-in: no source map by default
            sources_content: None,    // Mode-dependent default; see docs/source-maps.md
            requirements: RequirementsConfig::default(),
        }
    }
}

impl Combine for Config {
    fn combine(self, other: Self) -> Self {
        Self {
            // For collections, higher precedence (self) completely replaces lower precedence
            // (other) if self has non-default values, otherwise use other
            src: if self.src == Self::default().src {
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
            // Option scalar: absent keys in higher-precedence layers preserve lower layers
            bundle_third_party: self.bundle_third_party.or(other.bundle_third_party),
            sourcemap: self.sourcemap.or(other.sourcemap),
            sources_content: self.sources_content.or(other.sources_content),
            requirements: RequirementsConfig {
                python: self.requirements.python.or(other.requirements.python),
                module_map: if self.requirements.module_map.is_empty() {
                    other.requirements.module_map
                } else {
                    self.requirements.module_map
                },
            },
        }
    }
}

/// Configuration values from environment variables with CRIBO_ prefix
#[derive(Debug, Clone, Default)]
pub(crate) struct EnvConfig {
    pub src: Option<Vec<PathBuf>>,
    pub known_first_party: Option<IndexSet<String>>,
    pub known_third_party: Option<IndexSet<String>>,
    pub preserve_comments: Option<bool>,
    pub preserve_type_hints: Option<bool>,
    pub target_version: Option<String>,
    pub tree_shake: Option<bool>,
    pub bundle_third_party: Option<bool>,
    pub sourcemap: Option<SourceMapMode>,
    pub sources_content: Option<bool>,
    pub python: Option<PathBuf>,
}

impl EnvConfig {
    /// Load configuration from environment variables with CRIBO_ prefix
    pub(crate) fn from_env() -> Self {
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
                .map(ToOwned::to_owned)
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
                .map(ToOwned::to_owned)
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

        // CRIBO_BUNDLE_THIRD_PARTY - boolean flag
        if let Ok(bundle_third_party_str) = env::var("CRIBO_BUNDLE_THIRD_PARTY") {
            config.bundle_third_party = parse_bool(&bundle_third_party_str);
        }

        // CRIBO_SOURCEMAP - source map delivery mode (linked|inline|external)
        if let Ok(sourcemap_str) = env::var("CRIBO_SOURCEMAP") {
            config.sourcemap = parse_sourcemap_mode(&sourcemap_str);
            if config.sourcemap.is_none() {
                log::warn!(
                    "Ignoring CRIBO_SOURCEMAP='{sourcemap_str}': expected linked, inline, or \
                     external"
                );
            }
        }

        // CRIBO_SOURCES_CONTENT - boolean flag overriding the sourcesContent default
        if let Ok(sources_content_str) = env::var("CRIBO_SOURCES_CONTENT") {
            config.sources_content = parse_bool(&sources_content_str);
            if config.sources_content.is_none() {
                log::warn!(
                    "Ignoring CRIBO_SOURCES_CONTENT='{sources_content_str}': expected a boolean"
                );
            }
        }

        if let Ok(python) = env::var("CRIBO_PYTHON") {
            config.python = parse_env_path(&python);
        }

        config
    }

    /// Apply environment config to base config
    pub(crate) fn apply_to(self, mut config: Config) -> Config {
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
        if let Some(bundle_third_party) = self.bundle_third_party {
            config.bundle_third_party = Some(bundle_third_party);
        }
        if let Some(sourcemap) = self.sourcemap {
            config.sourcemap = Some(sourcemap);
        }
        if let Some(sources_content) = self.sources_content {
            config.sources_content = Some(sources_content);
        }
        if let Some(python) = self.python {
            config.requirements.python = Some(python);
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

/// Parse a source map delivery mode from an environment value.
fn parse_sourcemap_mode(value: &str) -> Option<SourceMapMode> {
    use cow_utils::CowUtils;
    match value.cow_to_lowercase().as_ref() {
        "linked" => Some(SourceMapMode::Linked),
        "inline" => Some(SourceMapMode::Inline),
        "external" => Some(SourceMapMode::External),
        _ => None,
    }
}

/// Parse a non-empty environment value as a filesystem path.
fn parse_env_path(value: &str) -> Option<PathBuf> {
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

impl Config {
    /// Effective third-party bundling policy (defaults to disabled when unset).
    pub fn bundle_third_party(&self) -> bool {
        self.bundle_third_party.unwrap_or(false)
    }

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
                "Invalid target version '{version_str}'. Supported versions: py38, py39, py310, \
                 py311, py312, py313"
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
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Self = toml::from_str(&content)
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
        config: &mut Self,
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
        let mut config = Self::default();

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
    fn test_empty_environment_path_is_ignored() {
        assert_eq!(parse_env_path(""), None);
        assert_eq!(
            parse_env_path(".venv/bin/python"),
            Some(PathBuf::from(".venv/bin/python"))
        );
    }

    #[test]
    fn test_requirements_debug_redacts_module_map() {
        let config = Config {
            requirements: RequirementsConfig {
                python: Some(PathBuf::from(".venv/bin/python")),
                module_map: IndexMap::from([(
                    "private_module".to_owned(),
                    "private-package @ https://user:secret-token@example.com/package.whl"
                        .to_owned(),
                )]),
            },
            ..Config::default()
        };

        let debug_output = format!("{config:?}");
        assert!(debug_output.contains(".venv/bin/python"));
        assert!(debug_output.contains("<redacted: 1 entries>"));
        assert!(!debug_output.contains("private_module"));
        assert!(!debug_output.contains("secret-token"));
    }

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
            .set_target_version("py311".to_owned())
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

    /// Verify `bundle-third-party` defaults to disabled, loads from TOML, and preserves
    /// lower-precedence layers when a higher-precedence config omits the key.
    #[test]
    fn test_bundle_third_party_config() {
        // Disabled by default (opt-in)
        assert!(!Config::default().bundle_third_party());

        // Loadable from TOML
        let toml_content = r#"
bundle-third-party = true
        "#;
        let mut temp_file =
            NamedTempFile::new().expect("should be able to create temp file for test");
        temp_file
            .write_all(toml_content.as_bytes())
            .expect("should be able to write test config to temp file");
        let config = Config::load(Some(temp_file.path()))
            .expect("should be able to load valid config from temp file");
        assert!(config.bundle_third_party());

        // A higher-precedence layer that omits the key preserves the lower layer
        let lower = Config {
            bundle_third_party: Some(true),
            ..Config::default()
        };
        let combined = Config::default().combine(lower);
        assert!(
            combined.bundle_third_party(),
            "absent bundle-third-party key must not override lower-precedence layers"
        );
    }

    #[test]
    fn test_toml_config_loading() {
        // Test loading target-version from TOML config
        let toml_content = r#"
target-version = "py312"
preserve_comments = false
src = ["src", "lib"]

[requirements]
python = ".venv/bin/python"
module-map = { sklearn = "scikit-learn", "google.cloud.storage" = "google-cloud-storage>=2" }
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
        assert_eq!(
            config.requirements.python,
            Some(PathBuf::from(".venv/bin/python"))
        );
        assert_eq!(
            config.requirements.module_map.get("sklearn"),
            Some(&"scikit-learn".to_owned())
        );
        assert_eq!(
            config.requirements.module_map.get("google.cloud.storage"),
            Some(&"google-cloud-storage>=2".to_owned())
        );
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
