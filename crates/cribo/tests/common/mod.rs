// Common test utilities shared across integration tests

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

/// Get the Python executable to use for testing
///
/// This function searches for Python in the following order:
/// 1. Project-local virtual environment (.venv in repo root)
/// 2. Active virtual environment (`VIRTUAL_ENV`)
/// 3. System Python (python3 or python)
///
/// This ensures tests use the correct Python with all required dependencies.
pub(crate) fn get_python_executable() -> PathBuf {
    // Prefer a project-local virtual environment (repo root .venv) if present.
    // We walk up from the crate's manifest directory to find the first '.venv'.
    // This allows tests to consistently use the pinned dependencies
    // without relying on an externally activated environment.
    {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        for ancestor in manifest_dir.ancestors() {
            let venv_dir = ancestor.join(".venv");
            if venv_dir.is_dir() {
                let candidate = if cfg!(windows) {
                    venv_dir.join("Scripts").join("python.exe")
                } else {
                    venv_dir.join("bin").join("python")
                };
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }

    // Check if we're in a virtual environment (CI or local development)
    if let Ok(virtual_env) = env::var("VIRTUAL_ENV") {
        // Try the virtual environment's Python first
        let venv_python = if cfg!(windows) {
            Path::new(&virtual_env).join("Scripts").join("python.exe")
        } else {
            Path::new(&virtual_env).join("bin").join("python")
        };

        if venv_python.exists() {
            return venv_python;
        }
    }

    // Fall back to trying common Python executable names
    for cmd in &["python3", "python"] {
        if Command::new(cmd)
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
        {
            return PathBuf::from(*cmd);
        }
    }
    panic!("Could not find Python executable");
}
