use std::path::Path;

use cow_utils::CowUtils;

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
    if Path::new(last_part)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
    {
        *last_part = last_part[..last_part.len() - 3].to_owned();
    }

    // Handle __init__.py and __main__.py files
    if is_special_module_file(last_part) {
        parts.pop();
    }

    // Skip files that don't map to a module
    if parts.is_empty() {
        return None;
    }

    Some(parts.join("."))
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
    module_name == "__init__" || module_name.ends_with(".__init__")
}

/// Extract the package name from an __init__ module name
/// For "__init__" returns None (bare __init__ without package)
/// For "pkg.__init__" returns Some("pkg")
/// For regular modules returns None
pub fn extract_package_from_init(module_name: &str) -> Option<&str> {
    if module_name == "__init__" {
        None
    } else if let Some(package) = module_name.strip_suffix(".__init__") {
        Some(package)
    } else {
        None
    }
}

/// Check if a string is an __init__ or __main__ file name (used for path processing)
pub fn is_special_module_file(name: &str) -> bool {
    name == "__init__" || name == "__main__"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_init_module() {
        assert!(is_init_module("__init__"));
        assert!(is_init_module("pkg.__init__"));
        assert!(is_init_module("my.package.__init__"));
        assert!(!is_init_module("__init__.py"));
        assert!(!is_init_module("module"));
        assert!(!is_init_module("pkg.module"));
    }

    #[test]
    fn test_extract_package_from_init() {
        assert_eq!(extract_package_from_init("__init__"), None);
        assert_eq!(extract_package_from_init("pkg.__init__"), Some("pkg"));
        assert_eq!(
            extract_package_from_init("my.package.__init__"),
            Some("my.package")
        );
        assert_eq!(extract_package_from_init("module"), None);
        assert_eq!(extract_package_from_init("pkg.module"), None);
    }

    #[test]
    fn test_is_special_module_file() {
        assert!(is_special_module_file("__init__"));
        assert!(is_special_module_file("__main__"));
        assert!(!is_special_module_file("__init__.py"));
        assert!(!is_special_module_file("module"));
    }

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
            module_name_from_relative(&PathBuf::from("pkg/__init__.py")),
            Some("pkg".to_string())
        );

        // Test __main__.py files - should return package name
        assert_eq!(
            module_name_from_relative(&PathBuf::from("pkg/__main__.py")),
            Some("pkg".to_string())
        );

        // Test nested packages
        assert_eq!(
            module_name_from_relative(&PathBuf::from("pkg/subpkg/__init__.py")),
            Some("pkg.subpkg".to_string())
        );

        // Test bare __init__.py at root
        assert_eq!(
            module_name_from_relative(&PathBuf::from("__init__.py")),
            None
        );
    }
}
