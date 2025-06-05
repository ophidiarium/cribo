use std::{
    env,
    path::{Path, PathBuf},
};

use etcetera::BaseStrategy;

/// Returns the path to the user configuration directory.
///
/// On Windows, use, e.g., C:\Users\Alice\AppData\Roaming
/// On Linux and macOS, use `XDG_CONFIG_HOME` or $HOME/.config, e.g., /home/alice/.config.
pub fn user_config_dir() -> Option<PathBuf> {
    etcetera::choose_base_strategy()
        .map(|dirs| dirs.config_dir())
        .ok()
}

pub fn user_serpen_config_dir() -> Option<PathBuf> {
    user_config_dir().map(|mut path| {
        path.push("serpen");
        path
    })
}

#[cfg(not(windows))]
fn locate_system_config_xdg(value: Option<&str>) -> Option<PathBuf> {
    // On Linux and macOS, read the `XDG_CONFIG_DIRS` environment variable.
    let default = "/etc/xdg";
    let config_dirs = value.filter(|s| !s.is_empty()).unwrap_or(default);

    for dir in config_dirs.split(':').take_while(|s| !s.is_empty()) {
        let serpen_toml_path = Path::new(dir).join("serpen").join("serpen.toml");
        if serpen_toml_path.is_file() {
            return Some(serpen_toml_path);
        }
    }
    None
}

#[cfg(windows)]
fn locate_system_config_windows(system_drive: impl AsRef<Path>) -> Option<PathBuf> {
    // On Windows, use `%SYSTEMDRIVE%\ProgramData\serpen\serpen.toml` (e.g., `C:\ProgramData`).
    let candidate = system_drive
        .as_ref()
        .join("ProgramData")
        .join("serpen")
        .join("serpen.toml");
    candidate.as_path().is_file().then_some(candidate)
}

/// Returns the path to the system configuration file.
///
/// On Unix-like systems, uses the `XDG_CONFIG_DIRS` environment variable (falling back to
/// `/etc/xdg/serpen/serpen.toml` if unset or empty) and then `/etc/serpen/serpen.toml`
///
/// On Windows, uses `%SYSTEMDRIVE%\ProgramData\serpen\serpen.toml`.
pub fn system_config_file() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var("SYSTEMDRIVE")
            .ok()
            .and_then(|system_drive| locate_system_config_windows(PathBuf::from(system_drive)))
    }

    #[cfg(not(windows))]
    {
        if let Some(path) = locate_system_config_xdg(env::var("XDG_CONFIG_DIRS").ok().as_deref()) {
            return Some(path);
        }

        // Fallback to `/etc/serpen/serpen.toml` if `XDG_CONFIG_DIRS` is not set or no valid
        // path is found.
        let candidate = Path::new("/etc/serpen/serpen.toml");
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

    use std::fs;
    use tempfile::TempDir;

    #[test]
    #[cfg(not(windows))]
    fn test_locate_system_config_xdg() -> anyhow::Result<()> {
        // Write a `serpen.toml` to a temporary directory.
        let context = TempDir::new()?;
        let config_dir = context.path().join("serpen");
        fs::create_dir_all(&config_dir)?;
        fs::write(config_dir.join("serpen.toml"), "[bundler]\nsrc = [\"src\"]")?;

        // None
        assert_eq!(locate_system_config_xdg(None), None);

        // Empty string
        assert_eq!(locate_system_config_xdg(Some("")), None);

        // Single colon
        assert_eq!(locate_system_config_xdg(Some(":")), None);

        // Assert that the function returns the correct path.
        assert_eq!(
            locate_system_config_xdg(Some(context.path().to_str().unwrap())).unwrap(),
            config_dir.join("serpen.toml")
        );

        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn test_windows_config() -> anyhow::Result<()> {
        // Write a `serpen.toml` to a temporary directory.
        let context = TempDir::new()?;
        let program_data = context.path().join("ProgramData").join("serpen");
        fs::create_dir_all(&program_data)?;
        fs::write(
            program_data.join("serpen.toml"),
            "[bundler]\nsrc = [\"src\"]",
        )?;

        assert_eq!(
            locate_system_config_windows(context.path()).unwrap(),
            program_data.join("serpen.toml")
        );

        // This does not have a `ProgramData` child, so contains no config.
        let context = TempDir::new()?;
        assert_eq!(locate_system_config_windows(context.path()), None);

        Ok(())
    }
}
