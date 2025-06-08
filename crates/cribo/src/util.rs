use cow_utils::CowUtils;
use std::path::{Path, PathBuf};

/// Convert a relative path to a Python module name, handling .py extension and __init__.py
pub fn module_name_from_relative(relative_path: &Path) -> Option<String> {
    let mut parts: Vec<String> = relative_path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    if parts.is_empty() {
        return None;
    }

    let last_part = parts.last_mut()?;
    // Remove .py extension
    if last_part.ends_with(".py") {
        *last_part = last_part[..last_part.len() - 3].to_owned();
    }

    // Handle __init__.py files
    if last_part == "__init__" {
        parts.pop();
    }

    // Skip files that don't map to a module
    if parts.is_empty() {
        return None;
    }

    Some(parts.join("."))
}

/// Convert a file system path to a Python module name, handling .py extension and __init__.py
/// Strips the `src_dir` prefix before processing.
pub fn path_to_module_name(src_dir: &Path, file_path: &Path) -> Option<String> {
    let relative_path = match file_path.strip_prefix(src_dir) {
        Ok(path) => path,
        Err(_) => return None,
    };
    // Handle root __init__.py specially
    if relative_path.components().count() == 1
        && relative_path.file_name().and_then(|n| n.to_str()) == Some("__init__.py")
    {
        return src_dir
            .file_name()
            .and_then(|os| os.to_str())
            .map(|s| s.to_owned());
    }
    module_name_from_relative(relative_path)
}

/// Normalize line endings to LF (\n) for cross-platform consistency
/// This ensures reproducible builds regardless of the platform where bundling occurs
pub fn normalize_line_endings(content: String) -> String {
    // Replace Windows CRLF (\r\n) and Mac CR (\r) with Unix LF (\n)
    content
        .cow_replace("\r\n", "\n")
        .cow_replace('\r', "\n")
        .into_owned()
}

/// Get the Python executable path, with support for virtual environments
///
/// This function checks for the VIRTUAL_ENV environment variable and constructs
/// the appropriate Python executable path for the current platform:
/// - Unix-like systems (Linux, macOS): `VIRTUAL_ENV/bin/python`
/// - Windows: `VIRTUAL_ENV\Scripts\python.exe`
///
/// If VIRTUAL_ENV is not set, falls back to the default Python executable name.
///
/// # Returns
///
/// The path to the Python executable as a String.
pub fn get_python_executable() -> String {
    match std::env::var("VIRTUAL_ENV") {
        Ok(venv_path) => {
            let mut python_path = PathBuf::from(venv_path);

            #[cfg(windows)]
            {
                python_path.push("Scripts");
                python_path.push("python.exe");
            }
            #[cfg(not(windows))]
            {
                python_path.push("bin");
                python_path.push("python");
            }

            python_path.to_string_lossy().to_string()
        }
        Err(_) => {
            // Fallback to default Python executable
            #[cfg(windows)]
            {
                "python.exe".to_string()
            }
            #[cfg(not(windows))]
            {
                "python3".to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::VirtualEnvGuard;

    #[test]
    #[serial_test::serial]
    fn test_get_python_executable_with_virtual_env() {
        // Test with VIRTUAL_ENV set
        let test_venv = "/path/to/venv";
        let _guard = VirtualEnvGuard::new(test_venv);

        let python_path = get_python_executable();

        #[cfg(windows)]
        {
            assert!(python_path.contains("Scripts"));
            assert!(python_path.contains("python.exe"));
        }
        #[cfg(not(windows))]
        {
            assert!(python_path.contains("/bin/python"));
        }

        assert!(python_path.contains(test_venv));
    }

    #[test]
    #[serial_test::serial]
    fn test_get_python_executable_without_virtual_env() {
        // Ensure VIRTUAL_ENV is not set
        let _guard = VirtualEnvGuard::unset();

        // Verify that VIRTUAL_ENV is actually unset - if not, skip this test
        // This handles cases where environment variable cleanup doesn't work
        // properly in CI environments
        if std::env::var("VIRTUAL_ENV").is_ok() {
            eprintln!(
                "Warning: VIRTUAL_ENV could not be unset (value: {:?}). \
                 Skipping test due to environment variable cleanup issues.",
                std::env::var("VIRTUAL_ENV").ok()
            );
            return;
        }

        let python_path = get_python_executable();

        #[cfg(windows)]
        assert_eq!(python_path, "python.exe");
        #[cfg(not(windows))]
        assert_eq!(python_path, "python3");
    }
}
