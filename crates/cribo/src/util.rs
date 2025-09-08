use std::path::Path;

use cow_utils::CowUtils;

use crate::python::module_path;

/// Convert a relative path to a Python module name, handling .py extension and __init__.py
pub fn module_name_from_relative(relative_path: &Path) -> Option<String> {
    module_path::module_name_from_relative(relative_path)
}

/// Normalize line endings to LF (\n) for cross-platform consistency
/// This ensures reproducible builds regardless of the platform where bundling occurs
pub fn normalize_line_endings(content: &str) -> String {
    // Replace Windows CRLF (\r\n) and Mac CR (\r) with Unix LF (\n)
    content
        .cow_replace("\r\n", "\n")
        .cow_replace('\r', "\n")
        .into_owned()
}

/// Check if a module name represents an __init__ module
/// Returns true for both bare "__init__" and dotted forms like "pkg.__init__"
pub fn is_init_module(module_name: &str) -> bool {
    module_path::is_init_module_name(module_name)
}

// (removed) is_special_module_file: call sites should use python::module_path predicates

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_init_module() {
        assert!(is_init_module(crate::python::constants::INIT_STEM));
        assert!(is_init_module(&format!(
            "pkg.{}",
            crate::python::constants::INIT_STEM
        )));
        assert!(is_init_module(&format!(
            "my.package.{}",
            crate::python::constants::INIT_STEM
        )));
        assert!(!is_init_module(crate::python::constants::INIT_FILE));
        assert!(!is_init_module("module"));
        assert!(!is_init_module("pkg.module"));
    }

    // removed: is_special_module_file tests (function removed)

    #[test]
    fn test_module_name_from_relative() {
        use std::path::PathBuf;

        // Test regular module
        assert_eq!(
            module_name_from_relative(&PathBuf::from("pkg/module.py")),
            Some("pkg.module".to_string())
        );

        // Test __init__.py files - should return package name
        assert_eq!(
            module_name_from_relative(&PathBuf::from(format!(
                "pkg/{}",
                crate::python::constants::INIT_FILE
            ))),
            Some("pkg".to_string())
        );

        // Test __main__.py files - should return package name
        assert_eq!(
            module_name_from_relative(&PathBuf::from(format!(
                "pkg/{}",
                crate::python::constants::MAIN_FILE
            ))),
            Some("pkg".to_string())
        );

        // Test nested packages
        assert_eq!(
            module_name_from_relative(&PathBuf::from(format!(
                "pkg/subpkg/{}",
                crate::python::constants::INIT_FILE
            ))),
            Some("pkg.subpkg".to_string())
        );

        // Test bare __init__.py at root
        assert_eq!(
            module_name_from_relative(&PathBuf::from(crate::python::constants::INIT_FILE)),
            None
        );
    }
}
