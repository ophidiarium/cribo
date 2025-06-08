use std::{
    env,
    path::{Path, PathBuf},
};

use etcetera::BaseStrategy;

/// Configuration directory name
const CONFIG_DIR: &str = "cribo";

/// Configuration file name
const CONFIG_FILE: &str = "cribo.toml";

/// Returns the path to the user configuration directory.
///
/// On Windows, use, e.g., C:\Users\Alice\AppData\Roaming
/// On Linux and macOS, use `XDG_CONFIG_HOME` or $HOME/.config, e.g., /home/alice/.config.
pub fn user_config_dir() -> Option<PathBuf> {
    match etcetera::choose_base_strategy() {
        Ok(dirs) => Some(dirs.config_dir()),
        Err(_) => None,
    }
}

pub fn user_cribo_config_dir() -> Option<PathBuf> {
    user_config_dir().map(|mut path| {
        path.push(CONFIG_DIR);
        path
    })
}

#[cfg(not(windows))]
fn locate_system_config_xdg(value: Option<&str>) -> Option<PathBuf> {
    // On Linux and macOS, read the `XDG_CONFIG_DIRS` environment variable.
    let default = "/etc/xdg";
    let config_dirs = value.filter(|s| !s.is_empty()).unwrap_or(default);

    for dir in config_dirs.split(':').take_while(|s| !s.is_empty()) {
        let cribo_toml_path = Path::new(dir).join(CONFIG_DIR).join(CONFIG_FILE);
        if cribo_toml_path.is_file() {
            return Some(cribo_toml_path);
        }
    }
    None
}

#[cfg(windows)]
fn locate_system_config_windows(system_drive: impl AsRef<Path>) -> Option<PathBuf> {
    // On Windows, use `%SYSTEMDRIVE%\ProgramData\cribo\cribo.toml` (e.g., `C:\ProgramData`).
    let candidate = system_drive
        .as_ref()
        .join("ProgramData")
        .join(CONFIG_DIR)
        .join(CONFIG_FILE);
    candidate.as_path().is_file().then_some(candidate)
}

/// Returns the path to the system configuration file.
///
/// On Unix-like systems, uses the `XDG_CONFIG_DIRS` environment variable (falling back to
/// `/etc/xdg/cribo/cribo.toml` if unset or empty) and then `/etc/cribo/cribo.toml`
///
/// On Windows, uses `%SYSTEMDRIVE%\ProgramData\cribo\cribo.toml`.
pub fn system_config_file() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var("SYSTEMDRIVE")
            .ok()
            .and_then(|system_drive| locate_system_config_windows(PathBuf::from(system_drive)))
    }

    #[cfg(not(windows))]
    {
        // Convert Result to Option - we want to continue if env var is not set
        let xdg_config_dirs = env::var("XDG_CONFIG_DIRS").ok();
        if let Some(path) = locate_system_config_xdg(xdg_config_dirs.as_deref()) {
            return Some(path);
        }

        // Fallback to `/etc/cribo/cribo.toml` if `XDG_CONFIG_DIRS` is not set or no valid
        // path is found.
        let candidate = Path::new("/etc").join(CONFIG_DIR).join(CONFIG_FILE);
        match candidate.try_exists() {
            Ok(true) => Some(candidate.to_path_buf()),
            Ok(false) => None,
            Err(err) => {
                log::warn!("Failed to query system configuration file: {err}");
                None
            }
        }
    }
}

#[cfg(test)]
mod test {
    #[cfg(windows)]
    use crate::dirs::locate_system_config_windows;
    #[cfg(not(windows))]
    use crate::dirs::locate_system_config_xdg;
    use crate::dirs::{CONFIG_DIR, CONFIG_FILE};

    use std::fs;
    use tempfile::TempDir;

    #[test]
    #[cfg(not(windows))]
    fn test_locate_system_config_xdg() -> anyhow::Result<()> {
        // Write a `cribo.toml` to a temporary directory.
        let context = TempDir::new()?;
        let config_dir = context.path().join(CONFIG_DIR);
        fs::create_dir_all(&config_dir)?;
        fs::write(config_dir.join(CONFIG_FILE), "[bundler]\nsrc = [\"src\"]")?;

        // None
        assert_eq!(locate_system_config_xdg(None), None);

        // Empty string
        assert_eq!(locate_system_config_xdg(Some("")), None);

        // Single colon
        assert_eq!(locate_system_config_xdg(Some(":")), None);

        // Assert that the function returns the correct path.
        assert_eq!(
            locate_system_config_xdg(Some(
                context.path().to_str().expect("path should be valid UTF-8")
            ))
            .expect("config should be found"),
            config_dir.join(CONFIG_FILE)
        );

        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn test_windows_config() -> anyhow::Result<()> {
        // Write a `cribo.toml` to a temporary directory.
        let context = TempDir::new()?;
        let program_data = context.path().join("ProgramData").join(CONFIG_DIR);
        fs::create_dir_all(&program_data)?;
        fs::write(program_data.join(CONFIG_FILE), "[bundler]\nsrc = [\"src\"]")?;

        assert_eq!(
            locate_system_config_windows(context.path()).unwrap(),
            program_data.join(CONFIG_FILE)
        );

        // This does not have a `ProgramData` child, so contains no config.
        let context = TempDir::new()?;
        assert_eq!(locate_system_config_windows(context.path()), None);

        Ok(())
    }
}
