//! Standard library detection utilities
//!
//! This module provides a single source of truth for determining whether a module
//! is part of the Python standard library and whether it's safe to hoist.

use ruff_python_stdlib::sys;

/// The target Python version for stdlib detection (3.8)
const PYTHON_VERSION: u8 = 38;

/// Check if a module name represents a Python standard library module
///
/// This uses ruff's comprehensive stdlib database and handles both direct
/// matches and submodules (e.g., both "os" and "os.path" are recognized).
pub fn is_stdlib_module(module_name: &str) -> bool {
    // Check direct match using ruff_python_stdlib
    if sys::is_known_standard_library(PYTHON_VERSION, module_name) {
        return true;
    }

    // Check if it's a submodule of a stdlib module
    if let Some(top_level) = module_name.split('.').next() {
        sys::is_known_standard_library(PYTHON_VERSION, top_level)
    } else {
        false
    }
}

/// Check if a module is a safe stdlib module that can be hoisted without side effects
///
/// Some stdlib modules have side effects when imported (e.g., antigravity opens
/// a web browser). This function returns true only for stdlib modules that are
/// safe to hoist to the top of the bundle.
pub fn is_stdlib_without_side_effects(module_name: &str) -> bool {
    // First check if it's even a stdlib module
    if !is_stdlib_module(module_name) {
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

        // Otherwise, if it's a stdlib module, it's safe to hoist
        _ => true,
    }
}

/// Check if a module should be hoisted based on its type
///
/// This determines whether an import should be moved to the top of the bundle.
/// Only __future__ imports and safe stdlib imports should be hoisted.
pub fn should_hoist_import(module_name: &str) -> bool {
    module_name == "__future__" || is_stdlib_without_side_effects(module_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_stdlib_module() {
        // Direct stdlib modules
        assert!(is_stdlib_module("os"));
        assert!(is_stdlib_module("sys"));
        assert!(is_stdlib_module("json"));
        assert!(is_stdlib_module("collections"));

        // Submodules
        assert!(is_stdlib_module("os.path"));
        assert!(is_stdlib_module("collections.abc"));
        assert!(is_stdlib_module("urllib.parse"));

        // Not stdlib
        assert!(!is_stdlib_module("numpy"));
        assert!(!is_stdlib_module("requests"));
        assert!(!is_stdlib_module("my_module"));
    }

    #[test]
    fn test_is_stdlib_without_side_effects() {
        // Safe stdlib modules
        assert!(is_stdlib_without_side_effects("os"));
        assert!(is_stdlib_without_side_effects("sys"));
        assert!(is_stdlib_without_side_effects("json"));
        assert!(is_stdlib_without_side_effects("math"));
        assert!(is_stdlib_without_side_effects("collections"));
        assert!(is_stdlib_without_side_effects("typing"));

        // Stdlib modules with side effects
        assert!(!is_stdlib_without_side_effects("antigravity"));
        assert!(!is_stdlib_without_side_effects("this"));
        assert!(!is_stdlib_without_side_effects("turtle"));
        assert!(!is_stdlib_without_side_effects("tkinter"));
        assert!(!is_stdlib_without_side_effects("site"));
        assert!(!is_stdlib_without_side_effects("webbrowser"));

        // Non-stdlib modules
        assert!(!is_stdlib_without_side_effects("numpy"));
        assert!(!is_stdlib_without_side_effects("requests"));
    }

    #[test]
    fn test_should_hoist_import() {
        // __future__ is always hoisted
        assert!(should_hoist_import("__future__"));

        // Safe stdlib modules are hoisted
        assert!(should_hoist_import("os"));
        assert!(should_hoist_import("sys"));
        assert!(should_hoist_import("json"));

        // Unsafe stdlib modules are not hoisted
        assert!(!should_hoist_import("antigravity"));
        assert!(!should_hoist_import("site"));

        // Third-party modules are not hoisted
        assert!(!should_hoist_import("numpy"));
    }
}
