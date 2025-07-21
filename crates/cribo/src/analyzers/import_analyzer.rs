//! Import analysis module
//!
//! This module provides functionality for analyzing import patterns,
//! including direct imports, namespace imports, and import relationships.

use std::path::PathBuf;

use log::debug;
use ruff_python_ast::{ModModule, Stmt};

use crate::{
    analyzers::types::UnusedImportInfo,
    cribo_graph::CriboGraph as DependencyGraph,
    types::{FxIndexMap, FxIndexSet},
};

/// Import analyzer for processing import patterns and relationships
pub struct ImportAnalyzer;

impl ImportAnalyzer {
    /// Find modules that are imported directly (e.g., `import module`)
    pub fn find_directly_imported_modules(
        modules: &[(String, ModModule, PathBuf, String)],
        _entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut directly_imported = FxIndexSet::default();

        // Check all modules for direct imports (both module-level and function-scoped)
        for (module_name, ast, module_path, _) in modules {
            debug!("Checking module '{module_name}' for direct imports");

            // Check the module body
            Self::collect_direct_imports_recursive(
                &ast.body,
                module_name,
                module_path,
                modules,
                &mut directly_imported,
            );
        }

        debug!(
            "Found {} directly imported modules",
            directly_imported.len()
        );
        directly_imported
    }

    /// Find modules that are imported as namespaces (e.g., `from pkg import module`)
    pub fn find_namespace_imported_modules(
        modules: &[(String, ModModule, PathBuf, String)],
    ) -> FxIndexMap<String, FxIndexSet<String>> {
        let mut namespace_imported_modules: FxIndexMap<String, FxIndexSet<String>> =
            FxIndexMap::default();

        debug!(
            "find_namespace_imported_modules: Checking {} modules",
            modules.len()
        );

        // Check all modules for namespace imports
        for (importing_module, ast, _, _) in modules {
            debug!("Checking module '{importing_module}' for namespace imports");
            for stmt in &ast.body {
                Self::collect_namespace_imports(
                    stmt,
                    modules,
                    importing_module,
                    &mut namespace_imported_modules,
                );
            }
        }

        debug!(
            "Found {} namespace imported modules: {:?}",
            namespace_imported_modules.len(),
            namespace_imported_modules
        );

        namespace_imported_modules
    }

    /// Find matching module name for namespace imports
    pub fn find_matching_module_name_namespace(
        modules: &[(String, ModModule, PathBuf, String)],
        full_module_path: &str,
    ) -> String {
        // Find the actual module name that matched
        modules
            .iter()
            .find_map(|(name, _, _, _)| {
                if name == full_module_path || name.ends_with(&format!(".{full_module_path}")) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| full_module_path.to_string())
    }

    /// Find unused imports in a dependency graph
    pub fn find_unused_imports(
        graph: &DependencyGraph,
        is_init_py: bool,
    ) -> Vec<(String, Vec<UnusedImportInfo>)> {
        let mut results = Vec::new();

        // Iterate through all modules in the graph
        for module in graph.modules.values() {
            let unused_imports = module.find_unused_imports(is_init_py);

            if !unused_imports.is_empty() {
                results.push((module.module_name.clone(), unused_imports));
            }
        }

        results
    }

    /// Collect direct imports recursively through the AST
    fn collect_direct_imports_recursive(
        body: &[Stmt],
        current_module: &str,
        module_path: &std::path::Path,
        modules: &[(String, ModModule, PathBuf, String)],
        directly_imported: &mut FxIndexSet<String>,
    ) {
        for stmt in body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let import_name = alias.name.to_string();
                        debug!("Found direct import '{import_name}' in module '{current_module}'");

                        // Check if this import corresponds to a module we're bundling
                        if modules.iter().any(|(name, _, _, _)| name == &import_name) {
                            directly_imported.insert(import_name);
                        }
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    // Recursively check function bodies
                    Self::collect_direct_imports_recursive(
                        &func_def.body,
                        current_module,
                        module_path,
                        modules,
                        directly_imported,
                    );
                }
                Stmt::ClassDef(class_def) => {
                    // Recursively check class bodies
                    Self::collect_direct_imports_recursive(
                        &class_def.body,
                        current_module,
                        module_path,
                        modules,
                        directly_imported,
                    );
                }
                Stmt::If(if_stmt) => {
                    // Check if branches
                    Self::collect_direct_imports_recursive(
                        &if_stmt.body,
                        current_module,
                        module_path,
                        modules,
                        directly_imported,
                    );
                    for clause in &if_stmt.elif_else_clauses {
                        Self::collect_direct_imports_recursive(
                            &clause.body,
                            current_module,
                            module_path,
                            modules,
                            directly_imported,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// Collect namespace imports from a statement
    fn collect_namespace_imports(
        stmt: &Stmt,
        modules: &[(String, ModModule, PathBuf, String)],
        importing_module: &str,
        namespace_imported_modules: &mut FxIndexMap<String, FxIndexSet<String>>,
    ) {
        match stmt {
            Stmt::ImportFrom(import_from) => {
                if let Some(module_name) = &import_from.module {
                    let module_str = module_name.to_string();
                    debug!(
                        "Checking ImportFrom: from {module_str} import ... in module \
                         {importing_module}"
                    );

                    for alias in &import_from.names {
                        let imported_name = alias.name.to_string();

                        // Check if this imports a module (namespace import)
                        let full_module_path = format!("{module_str}.{imported_name}");

                        // Check if this is importing a module we're bundling
                        let is_namespace_import = modules
                            .iter()
                            .any(|(name, _, _, _)| name == &full_module_path);

                        if is_namespace_import {
                            // Find the actual module name that matched
                            let actual_module_name = Self::find_matching_module_name_namespace(
                                modules,
                                &full_module_path,
                            );

                            debug!(
                                "  Found namespace import: from {module_name} import \
                                 {imported_name} -> {full_module_path} (actual: \
                                 {actual_module_name}) in module {importing_module}"
                            );
                            namespace_imported_modules
                                .entry(actual_module_name)
                                .or_default()
                                .insert(importing_module.to_string());
                        }
                    }
                }
            }
            // Recursively check function and class bodies
            Stmt::FunctionDef(func_def) => {
                for stmt in &func_def.body {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
            }
            Stmt::ClassDef(class_def) => {
                for stmt in &class_def.body {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
            }
            // Handle other compound statements
            Stmt::If(if_stmt) => {
                // Check body
                for stmt in &if_stmt.body {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
                // Check elif/else clauses
                for clause in &if_stmt.elif_else_clauses {
                    for stmt in &clause.body {
                        Self::collect_namespace_imports(
                            stmt,
                            modules,
                            importing_module,
                            namespace_imported_modules,
                        );
                    }
                }
            }
            Stmt::While(while_stmt) => {
                for stmt in &while_stmt.body {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
                // Also check else clause
                for stmt in &while_stmt.orelse {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
            }
            Stmt::For(for_stmt) => {
                for stmt in &for_stmt.body {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
                // Also check else clause
                for stmt in &for_stmt.orelse {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
            }
            Stmt::Try(try_stmt) => {
                // Check try body
                for stmt in &try_stmt.body {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
                // Check except handlers
                for handler in &try_stmt.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(except_handler) = handler;
                    for stmt in &except_handler.body {
                        Self::collect_namespace_imports(
                            stmt,
                            modules,
                            importing_module,
                            namespace_imported_modules,
                        );
                    }
                }
                // Check else clause
                for stmt in &try_stmt.orelse {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
                // Check finally clause
                for stmt in &try_stmt.finalbody {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
            }
            Stmt::With(with_stmt) => {
                for stmt in &with_stmt.body {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
            }
            Stmt::Match(match_stmt) => {
                for case in &match_stmt.cases {
                    for stmt in &case.body {
                        Self::collect_namespace_imports(
                            stmt,
                            modules,
                            importing_module,
                            namespace_imported_modules,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_find_directly_imported_modules() {
        let code1 = r#"
import module_a
import module_b as mb

def func():
    import module_c
"#;
        let parsed1 = parse_module(code1).unwrap();
        let ast1 = parsed1.into_syntax();

        let code2 = r#"
def other_func():
    pass
"#;
        let parsed2 = parse_module(code2).unwrap();
        let ast2 = parsed2.into_syntax();

        let modules = vec![
            (
                "test_module".to_string(),
                ast1,
                PathBuf::from("test.py"),
                "hash1".to_string(),
            ),
            (
                "module_a".to_string(),
                ast2.clone(),
                PathBuf::from("module_a.py"),
                "hash2".to_string(),
            ),
            (
                "module_b".to_string(),
                ast2.clone(),
                PathBuf::from("module_b.py"),
                "hash3".to_string(),
            ),
            (
                "module_c".to_string(),
                ast2,
                PathBuf::from("module_c.py"),
                "hash4".to_string(),
            ),
        ];

        let directly_imported =
            ImportAnalyzer::find_directly_imported_modules(&modules, "test_module");

        assert_eq!(directly_imported.len(), 3);
        assert!(directly_imported.contains("module_a"));
        assert!(directly_imported.contains("module_b"));
        assert!(directly_imported.contains("module_c"));
    }

    #[test]
    fn test_find_namespace_imported_modules() {
        let code1 = r#"
from pkg import module_a
from pkg.sub import module_b
"#;
        let parsed1 = parse_module(code1).unwrap();
        let ast1 = parsed1.into_syntax();

        let code2 = r#"pass"#;
        let parsed2 = parse_module(code2).unwrap();
        let ast2 = parsed2.into_syntax();

        let modules = vec![
            (
                "test_module".to_string(),
                ast1,
                PathBuf::from("test.py"),
                "hash1".to_string(),
            ),
            (
                "pkg.module_a".to_string(),
                ast2.clone(),
                PathBuf::from("pkg/module_a.py"),
                "hash2".to_string(),
            ),
            (
                "pkg.sub.module_b".to_string(),
                ast2,
                PathBuf::from("pkg/sub/module_b.py"),
                "hash3".to_string(),
            ),
        ];

        let namespace_imported = ImportAnalyzer::find_namespace_imported_modules(&modules);

        assert_eq!(namespace_imported.len(), 2);
        assert!(
            namespace_imported
                .get("pkg.module_a")
                .unwrap()
                .contains("test_module")
        );
        assert!(
            namespace_imported
                .get("pkg.sub.module_b")
                .unwrap()
                .contains("test_module")
        );
    }
}
