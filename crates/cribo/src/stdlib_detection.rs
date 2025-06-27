//! Standard library detection utilities
//!
//! This module provides a single source of truth for determining whether a module
//! is part of the Python standard library and whether it's safe to hoist.

use ruff_python_stdlib::sys;

/// Check if a module name represents a Python standard library module
///
/// This uses ruff's comprehensive stdlib database and handles both direct
/// matches and submodules (e.g., both "os" and "os.path" are recognized).
///
/// # Arguments
/// * `module_name` - The module name to check
/// * `python_version` - The Python version as a u8 (e.g., 38 for Python 3.8, 10 for Python 3.10)
pub fn is_stdlib_module(module_name: &str, python_version: u8) -> bool {
    // Special case for __future__ which is always a stdlib module
    // but not included in ruff's is_known_standard_library
    if module_name == "__future__" {
        return true;
    }

    // Check direct match using ruff_python_stdlib
    if sys::is_known_standard_library(python_version, module_name) {
        return true;
    }

    // Check if it's a submodule of a stdlib module
    if let Some(top_level) = module_name.split('.').next() {
        sys::is_known_standard_library(python_version, top_level)
    } else {
        false
    }
}

/// Check if a module is a safe stdlib module that can be hoisted without side effects
///
/// Some stdlib modules have side effects when imported (e.g., antigravity opens
/// a web browser). This function returns true only for stdlib modules that are
/// safe to hoist to the top of the bundle.
///
/// # Arguments
/// * `module_name` - The module name to check
/// * `python_version` - The Python version as a u8 (e.g., 38 for Python 3.8, 10 for Python 3.10)
pub fn is_stdlib_without_side_effects(module_name: &str, python_version: u8) -> bool {
    // First check if it's even a stdlib module
    if !is_stdlib_module(module_name, python_version) {
        return false;
    }

    // Check against known modules with side effects
    match module_name {
        // Modules that modify global state or have observable side effects
        "antigravity" => false, // Opens web browser
        "this" => false,        // Prints "The Zen of Python"
        "__hello__" => false,   // Prints "Hello world!"
        "__phello__" => false,  // Frozen hello module

        // Site-specific modules that modify sys.path and other globals
        "site" => false,
        "sitecustomize" => false,
        "usercustomize" => false,

        // Terminal/readline modules that modify terminal state
        "readline" => false,
        "rlcompleter" => false,

        // GUI modules that may initialize display systems
        "turtle" => false,
        "tkinter" => false,

        // Opens web browser
        "webbrowser" => false,

        // May modify locale settings
        "locale" => false,

        // Platform module can have initialization side effects
        "platform" => false,

        // Logging configuration modules
        "logging" => false,  // Can configure global logging state
        "warnings" => false, // Can modify global warning filters

        // Encoding modules that load codecs on import
        "encodings" => false, // Loads encoding modules

        // Otherwise, if it's a stdlib module, it's safe to hoist
        _ => true,
    }
}

/// Check if a module should be hoisted based on its type
///
/// This determines whether an import should be moved to the top of the bundle.
/// Only __future__ imports and safe stdlib imports should be hoisted.
///
/// # Arguments
/// * `module_name` - The module name to check
/// * `python_version` - The Python version as a u8 (e.g., 38 for Python 3.8, 10 for Python 3.10)
pub fn should_hoist_import(module_name: &str, python_version: u8) -> bool {
    module_name == "__future__" || is_stdlib_without_side_effects(module_name, python_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_stdlib_module() {
        // Use Python 3.10 as test version
        let py_version = 10;

        // Test __future__ specifically
        assert!(
            is_stdlib_module("__future__", py_version),
            "__future__ should be recognized as stdlib"
        );

        // Direct stdlib modules
        assert!(is_stdlib_module("os", py_version));
        assert!(is_stdlib_module("sys", py_version));
        assert!(is_stdlib_module("json", py_version));
        assert!(is_stdlib_module("collections", py_version));

        // Submodules
        assert!(is_stdlib_module("os.path", py_version));
        assert!(is_stdlib_module("collections.abc", py_version));
        assert!(is_stdlib_module("urllib.parse", py_version));

        // Not stdlib
        assert!(!is_stdlib_module("numpy", py_version));
        assert!(!is_stdlib_module("requests", py_version));
        assert!(!is_stdlib_module("my_module", py_version));
    }

    #[test]
    fn test_is_stdlib_without_side_effects() {
        // Use Python 3.10 as test version
        let py_version = 10;

        // Safe stdlib modules
        assert!(is_stdlib_without_side_effects("os", py_version));
        assert!(is_stdlib_without_side_effects("sys", py_version));
        assert!(is_stdlib_without_side_effects("json", py_version));
        assert!(is_stdlib_without_side_effects("math", py_version));
        assert!(is_stdlib_without_side_effects("collections", py_version));
        assert!(is_stdlib_without_side_effects("typing", py_version));

        // Stdlib modules with side effects
        assert!(!is_stdlib_without_side_effects("antigravity", py_version));
        assert!(!is_stdlib_without_side_effects("this", py_version));
        assert!(!is_stdlib_without_side_effects("turtle", py_version));
        assert!(!is_stdlib_without_side_effects("tkinter", py_version));
        assert!(!is_stdlib_without_side_effects("site", py_version));
        assert!(!is_stdlib_without_side_effects("webbrowser", py_version));
        assert!(!is_stdlib_without_side_effects("logging", py_version));
        assert!(!is_stdlib_without_side_effects("warnings", py_version));
        assert!(!is_stdlib_without_side_effects("encodings", py_version));

        // Non-stdlib modules
        assert!(!is_stdlib_without_side_effects("numpy", py_version));
        assert!(!is_stdlib_without_side_effects("requests", py_version));
    }

    #[test]
    fn test_should_hoist_import() {
        // Use Python 3.10 as test version
        let py_version = 10;

        // __future__ is always hoisted
        assert!(should_hoist_import("__future__", py_version));

        // Safe stdlib modules are hoisted
        assert!(should_hoist_import("os", py_version));
        assert!(should_hoist_import("sys", py_version));
        assert!(should_hoist_import("json", py_version));

        // Unsafe stdlib modules are not hoisted
        assert!(!should_hoist_import("antigravity", py_version));
        assert!(!should_hoist_import("site", py_version));

        // Third-party modules are not hoisted
        assert!(!should_hoist_import("numpy", py_version));
    }
}
