//! Namespace analysis module
//!
//! This module provides functionality for analyzing namespace requirements,
//! including package hierarchies and namespace object needs.

use std::path::PathBuf;

use log::debug;
use ruff_python_ast::ModModule;

use crate::{
    analyzers::types::NamespaceAnalysis,
    types::{FxIndexMap, FxIndexSet},
};

/// Namespace analyzer for processing namespace requirements
pub struct NamespaceAnalyzer;

impl NamespaceAnalyzer {
    /// Identify required namespaces from module names
    /// This includes both missing parent namespaces and modules with submodules
    pub fn identify_required_namespaces(
        modules: &[(String, ModModule, PathBuf, String)],
    ) -> FxIndexSet<String> {
        debug!(
            "Identifying required namespaces from {} modules",
            modules.len()
        );

        let mut required_namespaces = FxIndexSet::default();

        // First, collect all module names to check if parent modules exist
        // Normalize __init__ to the actual package name if present
        let all_module_names: FxIndexSet<String> = modules
            .iter()
            .map(|(name, _, _, _)| {
                if name == "__init__" {
                    // Find the actual package name from other modules
                    // e.g., if we have "requests.compat", the package is "requests"
                    if let Some((other_name, _, _, _)) =
                        modules.iter().find(|(n, _, _, _)| n.contains('.'))
                        && let Some(package_name) = other_name.split('.').next()
                    {
                        return package_name.to_string();
                    }
                }
                name.clone()
            })
            .collect();

        // Scan all modules to find dotted module names
        for (module_name, _, _, _) in modules {
            // Skip __init__ module as it's already handled above
            if module_name == "__init__" {
                continue;
            }

            if !module_name.contains('.') {
                continue;
            }

            // Split the module name and identify all parent namespaces
            let parts: Vec<&str> = module_name.split('.').collect();

            // Add all parent namespace levels
            for i in 1..parts.len() {
                let namespace = parts[..i].join(".");

                // We need to create a namespace for ALL parent namespaces, regardless of whether
                // they are wrapped modules or not. This is because child modules need to be
                // assigned as attributes on their parent namespaces.
                debug!("Identified required namespace: {namespace}");
                required_namespaces.insert(namespace);
            }
        }

        // IMPORTANT: Also add modules that have submodules as required namespaces
        // This ensures that parent modules like 'models' and 'services' exist as namespaces
        // before we try to assign their submodules
        for module_name in &all_module_names {
            // Check if this module has any submodules
            let has_submodules = all_module_names
                .iter()
                .any(|m| m != module_name && m.starts_with(&format!("{module_name}.")));

            if has_submodules {
                // Any module with submodules needs a namespace, regardless of whether it's
                // a wrapper module or the entry module
                debug!("Identified module with submodules as required namespace: {module_name}");
                required_namespaces.insert(module_name.clone());
            }
        }

        debug!("Total required namespaces: {}", required_namespaces.len());

        required_namespaces
    }

    /// Check if a module requires a namespace object
    pub fn module_needs_namespace(
        module_name: &str,
        directly_imported_modules: &FxIndexSet<String>,
        namespace_imported_modules: &FxIndexSet<String>,
        has_exports: bool,
    ) -> bool {
        // Module needs namespace if:
        // 1. It's imported as a namespace (from pkg import module)
        // 2. It's imported directly and has exports
        namespace_imported_modules.contains(module_name)
            || (directly_imported_modules.contains(module_name) && has_exports)
    }

    /// Analyze namespace requirements for a set of modules
    pub fn analyze_namespace_requirements(
        modules: &[(String, ModModule, PathBuf, String)],
        directly_imported_modules: &FxIndexSet<String>,
        namespace_imported_modules: &FxIndexSet<String>,
        module_exports: &FxIndexSet<String>,
    ) -> NamespaceAnalysis {
        let required_namespaces = Self::identify_required_namespaces(modules);

        let mut modules_needing_namespaces = FxIndexSet::default();

        for (module_name, _, _, _) in modules {
            let has_exports = module_exports.contains(module_name);

            if Self::module_needs_namespace(
                module_name,
                directly_imported_modules,
                namespace_imported_modules,
                has_exports,
            ) {
                modules_needing_namespaces.insert(module_name.clone());
            }
        }

        NamespaceAnalysis {
            required_namespaces,
            namespace_hierarchy: FxIndexMap::default(), /* TODO: Implement namespace hierarchy
                                                         * building */
            modules_needing_namespaces,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identify_required_namespaces() {
        use ruff_python_parser::parse_module;

        // Create simple module ASTs for testing
        let parsed = parse_module("pass").unwrap();
        let module_ast = parsed.into_syntax();

        let modules = vec![
            (
                "pkg.sub.module_a".to_string(),
                module_ast.clone(),
                PathBuf::new(),
                "hash1".to_string(),
            ),
            (
                "pkg.sub.module_b".to_string(),
                module_ast.clone(),
                PathBuf::new(),
                "hash2".to_string(),
            ),
            (
                "pkg.other".to_string(),
                module_ast.clone(),
                PathBuf::new(),
                "hash3".to_string(),
            ),
            (
                "toplevel".to_string(),
                module_ast,
                PathBuf::new(),
                "hash4".to_string(),
            ),
        ];

        let required = NamespaceAnalyzer::identify_required_namespaces(&modules);

        // Should include:
        // - "pkg" (parent namespace for pkg.sub.* and pkg.other)
        // - "pkg.sub" (parent namespace for pkg.sub.module_a and pkg.sub.module_b)
        // - "pkg" again because it has submodules (pkg.sub and pkg.other)
        assert_eq!(required.len(), 2);
        assert!(required.contains("pkg"));
        assert!(required.contains("pkg.sub"));
    }

    #[test]
    fn test_module_needs_namespace() {
        let mut directly_imported = FxIndexSet::default();
        directly_imported.insert("module_a".to_string());

        let mut namespace_imported = FxIndexSet::default();
        namespace_imported.insert("module_b".to_string());

        // Namespace imported module needs namespace
        assert!(NamespaceAnalyzer::module_needs_namespace(
            "module_b",
            &directly_imported,
            &namespace_imported,
            false
        ));

        // Directly imported module with exports needs namespace
        assert!(NamespaceAnalyzer::module_needs_namespace(
            "module_a",
            &directly_imported,
            &namespace_imported,
            true
        ));

        // Directly imported module without exports doesn't need namespace
        assert!(!NamespaceAnalyzer::module_needs_namespace(
            "module_a",
            &directly_imported,
            &namespace_imported,
            false
        ));

        // Module not imported doesn't need namespace
        assert!(!NamespaceAnalyzer::module_needs_namespace(
            "module_c",
            &directly_imported,
            &namespace_imported,
            true
        ));
    }
}
