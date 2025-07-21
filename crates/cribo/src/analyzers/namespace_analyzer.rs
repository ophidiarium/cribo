//! Namespace analysis module
//!
//! This module provides functionality for analyzing namespace requirements,
//! including package hierarchies and namespace object needs.

use std::path::PathBuf;

use log::debug;
use ruff_python_ast::ModModule;

use crate::types::FxIndexSet;

/// Namespace analyzer for processing namespace requirements
pub struct NamespaceAnalyzer;

impl NamespaceAnalyzer {
    /// Identify required namespaces from module names
    pub fn identify_required_namespaces(
        modules: &[(String, ModModule, PathBuf, String)],
    ) -> FxIndexSet<String> {
        debug!(
            "Identifying required namespaces from {} modules",
            modules.len()
        );

        let mut required_namespaces = FxIndexSet::default();
        let mut module_names = FxIndexSet::default();

        // Collect all module names
        for (module_name, _, _, _) in modules {
            module_names.insert(module_name.clone());
        }

        // For each module, check if its parent namespaces need to be created
        for module_name in &module_names {
            let parts: Vec<&str> = module_name.split('.').collect();

            // Check each parent namespace
            for i in 1..parts.len() {
                let namespace = parts[..i].join(".");

                // If this namespace isn't an actual module, we need to create it
                if !module_names.contains(&namespace) {
                    debug!("Identified required namespace: {namespace}");
                    required_namespaces.insert(namespace);
                }
            }
        }

        debug!(
            "Identified {} required namespaces: {:?}",
            required_namespaces.len(),
            required_namespaces
        );

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

        let mut modules_needing_namespace = FxIndexSet::default();

        for (module_name, _, _, _) in modules {
            let has_exports = module_exports.contains(module_name);

            if Self::module_needs_namespace(
                module_name,
                directly_imported_modules,
                namespace_imported_modules,
                has_exports,
            ) {
                modules_needing_namespace.insert(module_name.clone());
            }
        }

        NamespaceAnalysis {
            required_namespaces,
            modules_needing_namespace,
        }
    }
}

/// Results of namespace analysis
#[derive(Debug, Default)]
pub struct NamespaceAnalysis {
    /// Namespaces that need to be created (e.g., "pkg" for "pkg.module")
    pub required_namespaces: FxIndexSet<String>,
    /// Modules that need namespace objects
    pub modules_needing_namespace: FxIndexSet<String>,
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
