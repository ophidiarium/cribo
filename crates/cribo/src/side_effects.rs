//! Centralized side effect detection for modules and imports
//!
//! This module provides a single source of truth for determining whether
//! Python modules, imports, or AST nodes have side effects that would
//! prevent optimization techniques like hoisting or inlining.

use ruff_python_ast::{ModModule, StmtImportFrom};

use crate::visitors::SideEffectDetector;

/// Check if an import statement would have side effects
pub fn import_has_side_effects(module_name: &str, python_version: u8) -> bool {
    // Check if it's a stdlib module
    let root_module = module_name.split('.').next().unwrap_or(module_name);
    if ruff_python_stdlib::sys::is_known_standard_library(python_version, root_module) {
        // Stdlib modules are handled by the proxy, so no side effects for our purposes
        return false;
    }

    // All non-stdlib modules are considered to have potential side effects
    // This is conservative but safe - third-party modules can have any behavior
    true
}

/// Check if a from-import statement would have side effects
pub fn from_import_has_side_effects(import_from: &StmtImportFrom, python_version: u8) -> bool {
    // Star imports always have potential side effects (except from stdlib)
    let is_star = import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*";

    if let Some(module) = &import_from.module {
        let module_name = module.as_str();

        // Check if it's a stdlib module
        let root_module = module_name.split('.').next().unwrap_or(module_name);
        if ruff_python_stdlib::sys::is_known_standard_library(python_version, root_module) {
            // Stdlib modules are handled by the proxy, so no side effects for our purposes
            return false;
        }

        // Star imports from non-stdlib modules have side effects
        if is_star {
            return true;
        }

        // Check if this is a known side-effect import
        import_has_side_effects(module_name, python_version)
    } else {
        // Relative imports
        is_star
    }
}

/// Check if a module AST has side effects that prevent optimization
///
/// This checks for executable code at the module level beyond simple
/// definitions and safe imports.
pub fn module_has_side_effects(ast: &ModModule, python_version: u8) -> bool {
    // Delegate to the AST visitor
    SideEffectDetector::check_module(ast, python_version)
}
