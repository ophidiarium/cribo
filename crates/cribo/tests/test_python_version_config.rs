#![allow(clippy::disallowed_methods)]

#[cfg(test)]
mod tests {
    use cribo::config::Config;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_target_version_configuration() {
        println!("Testing target-version configuration...");

        // Test default behavior (should be "py310")
        let default_config = Config::default();
        assert_eq!(default_config.target_version, "py310");
        assert_eq!(default_config.python_version().unwrap(), 10);

        // Test setting target version programmatically
        let mut config = Config::default();
        config.set_target_version("py311".to_string()).unwrap();
        assert_eq!(config.target_version, "py311");
        assert_eq!(config.python_version().unwrap(), 11);

        // Test parsing various valid versions
        assert_eq!(Config::parse_target_version("py38").unwrap(), 8);
        assert_eq!(Config::parse_target_version("py39").unwrap(), 9);
        assert_eq!(Config::parse_target_version("py310").unwrap(), 10);
        assert_eq!(Config::parse_target_version("py311").unwrap(), 11);
        assert_eq!(Config::parse_target_version("py312").unwrap(), 12);
        assert_eq!(Config::parse_target_version("py313").unwrap(), 13);

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

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(toml_content.as_bytes()).unwrap();

        let config = Config::load(Some(temp_file.path())).unwrap();
        assert_eq!(config.target_version, "py312");
        assert_eq!(config.python_version().unwrap(), 12);
        assert!(!config.preserve_comments);
    }

    #[test]
    fn test_invalid_toml_config() {
        // Test invalid target-version in TOML config
        let toml_content = r#"
        target-version = "invalid_version"
        "#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(toml_content.as_bytes()).unwrap();

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
