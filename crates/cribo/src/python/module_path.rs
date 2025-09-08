use std::path::Path;

use super::constants::{INIT_FILE, INIT_STEM, MAIN_FILE, MAIN_STEM};

/// Classification of a Python path/module
///
/// This enum distinguishes between different types of Python modules and packages,
/// which is crucial for proper import resolution and bundling behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    /// A normal Python module file (e.g., `foo.py`, `bar/baz.py`)
    ///
    /// This represents a regular `.py` file that is imported as a module.
    /// The file's parent directory is not considered a package unless it
    /// contains an `__init__.py` file.
    RegularModule,

    /// A package initializer file (`__init__.py`)
    ///
    /// This file makes its containing directory a regular Python package.
    /// When importing the package, this file's code is executed, and its
    /// namespace becomes the package's namespace. The directory path should
    /// be treated as the package root.
    PackageInit,

    /// A namespace package directory (PEP 420)
    ///
    /// A directory that forms a namespace package - it has no `__init__.py` file
    /// but can still be imported as a package. Unlike `PackageInit`, there's no
    /// initialization code to run. Multiple directories with the same name can
    /// contribute to the same namespace package. The directory path should be
    /// treated as a package root for import resolution.
    NamespacePackageDir,

    /// A package entry-point module (`__main__.py`)
    ///
    /// This special module is executed when a package is run directly
    /// (e.g., `python -m package`). It typically resides inside a package
    /// directory alongside or instead of `__init__.py`.
    Main,
}

/// Returns true if the given file name is exactly "__init__.py".
#[inline]
pub fn is_init_file_name(name: &str) -> bool {
    name == INIT_FILE
}

/// Returns true if the given file stem is exactly "__init__".
#[inline]
pub fn is_init_stem(stem: &str) -> bool {
    stem == INIT_STEM
}

/// Returns true if the given file name is exactly "__main__.py".
#[inline]
pub fn is_main_file_name(name: &str) -> bool {
    name == MAIN_FILE
}

/// Returns true if the given file name is an entry-like special file (init or main).
#[inline]
pub fn is_special_entry_file_name(name: &str) -> bool {
    is_init_file_name(name) || is_main_file_name(name)
}

// (removed) is_init_path: prefer using file-name specific checks or resolver metadata

/// Returns true if `dir/__init__.py` exists.
#[inline]
pub fn is_package_dir_with_init(dir: &Path) -> bool {
    dir.join(INIT_FILE).is_file()
}

/// Returns true if `dir` exists and is a directory without `__init__.py` (PEP 420).
#[inline]
pub fn is_namespace_package_dir(dir: &Path) -> bool {
    dir.is_dir() && !is_package_dir_with_init(dir)
}

/// Convert a relative path to a canonical Python module name.
/// - Strips the `.py` extension
/// - Collapses `__init__.py` and `__main__.py` to the parent package name
/// - Returns `None` when path does not map to a module name (e.g., bare `__init__.py` at root)
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
    if Path::new(last_part)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
        && let Some(stem) = Path::new(last_part).file_stem().and_then(|s| s.to_str())
    {
        // Replace with stem to avoid any off-by-one issues
        *last_part = stem.to_string();
    }

    // Handle __init__.py and __main__.py files
    if is_init_stem(last_part) || last_part == MAIN_STEM {
        parts.pop();
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("."))
}

// (removed) classify_path: not needed currently; rely on resolver

// (removed) parent_package_dir: not used

// (removed) canonical_module_name: call sites use explicit suffix stripping when needed

/// Return true if the module name refers to an `__init__` module.
/// Accepts both bare "__init__" and dotted forms like "pkg.__init__".
#[inline]
pub fn is_init_module_name(module_name: &str) -> bool {
    module_name == INIT_STEM
        || module_name
            .strip_suffix(INIT_STEM)
            .is_some_and(|p| p.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_is_init_file_name_and_path() {
        assert!(is_init_file_name(INIT_FILE));
        assert!(is_init_stem(INIT_STEM));
        assert!(is_main_file_name(MAIN_FILE));
    }

    #[test]
    fn test_package_dir_classification() {
        // These tests do not touch filesystem for existence; just exercise API shape
        let dir = PathBuf::from("pkg");
        // Not guaranteed to exist; just ensure functions are callable
        let _ = is_package_dir_with_init(&dir);
        let _ = is_namespace_package_dir(Path::new("."));
        // The above line ensures function is callable without asserting environment specifics.
    }

    #[test]
    fn test_module_name_from_relative() {
        assert_eq!(
            module_name_from_relative(Path::new("pkg/module.py")),
            Some("pkg.module".to_string())
        );
        assert_eq!(
            module_name_from_relative(Path::new("pkg/__init__.py")),
            Some("pkg".to_string())
        );
        assert_eq!(
            module_name_from_relative(Path::new("pkg/__main__.py")),
            Some("pkg".to_string())
        );
        assert_eq!(
            module_name_from_relative(Path::new("pkg/subpkg/__init__.py")),
            Some("pkg.subpkg".to_string())
        );
        assert_eq!(module_name_from_relative(Path::new("__init__.py")), None);
    }

    #[test]
    fn test_is_init_name_predicate() {
        assert!(is_init_module_name("__init__"));
        assert!(is_init_module_name("pkg.__init__"));
    }
}
