//! Centralized side effect detection for modules and imports
//!
//! This module provides a single source of truth for determining whether
//! Python modules, imports, or AST nodes have side effects that would
//! prevent optimization techniques like hoisting or inlining.

use ruff_python_ast::{ModModule, StmtImport, StmtImportFrom};

use crate::visitors::SideEffectDetector;

/// Check if a module name represents a safe stdlib module that can be hoisted
/// without side effects
pub fn is_safe_stdlib_module(module_name: &str) -> bool {
    crate::stdlib_normalization::is_safe_stdlib_module(module_name)
}

/// Check if an import statement would have side effects
pub fn import_has_side_effects(module_name: &str) -> bool {
    // Safe stdlib modules don't have side effects
    if is_safe_stdlib_module(module_name) {
        return false;
    }

    // All non-stdlib modules are considered to have potential side effects
    // This is conservative but safe - third-party modules can have any behavior
    true
}

/// Check if a from-import statement would have side effects
pub fn from_import_has_side_effects(import_from: &StmtImportFrom) -> bool {
    // Star imports always have potential side effects (except from safe stdlib)
    let is_star = import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*";

    if let Some(module) = &import_from.module {
        let module_name = module.as_str();

        // Safe stdlib modules don't have side effects even with star imports
        if is_safe_stdlib_module(module_name) {
            return false;
        }

        // Star imports from non-stdlib modules have side effects
        if is_star {
            return true;
        }

        // Check if this is a known side-effect import
        import_has_side_effects(module_name)
    } else {
        // Relative imports
        is_star
    }
}

/// Check if a regular import statement would have side effects
pub fn regular_import_has_side_effects(import_stmt: &StmtImport) -> bool {
    import_stmt.names.iter().any(|alias| {
        let module_name = alias.name.as_str();
        import_has_side_effects(module_name)
    })
}

/// Check if a module AST has side effects that prevent optimization
///
/// This checks for executable code at the module level beyond simple
/// definitions and safe imports.
pub fn module_has_side_effects(ast: &ModModule) -> bool {
    // Delegate to the AST visitor
    SideEffectDetector::check_module(ast)
}
