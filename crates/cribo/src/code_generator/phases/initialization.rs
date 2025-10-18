//! Initialization Phase
//!
//! This phase handles the initial setup of the bundler, including:
//! - Collecting future imports from all modules
//! - Identifying circular dependencies
//! - Storing bundler configuration and references

use indexmap::IndexMap as FxIndexMap;
use ruff_python_ast::Stmt;

use crate::code_generator::{
    bundler::Bundler,
    context::{BundleParams, InitializationResult},
};

/// Initialization phase handler (stateless)
pub struct InitializationPhase;

impl InitializationPhase {
    /// Create a new initialization phase
    pub fn new() -> Self {
        Self
    }

    /// Execute the initialization phase
    ///
    /// This method:
    /// 1. Stores references to the graph and semantic bundler
    /// 2. Initializes bundler settings (tree shaking, __all__ access, entry module info)
    /// 3. Collects future imports from all modules
    /// 4. Identifies circular dependencies
    /// 5. Finds namespace-imported modules
    ///
    /// Returns an `InitializationResult` containing the future imports, circular modules,
    /// and namespace import information.
    pub fn execute<'a>(
        &self,
        bundler: &mut Bundler<'a>,
        params: &BundleParams<'a>,
    ) -> InitializationResult {
        // Store the graph reference for use in transformation methods
        bundler.graph = Some(params.graph);

        // Store the semantic bundler reference for use in transformations
        bundler.semantic_bundler = Some(params.semantic_bundler);

        // Initialize bundler settings and collect preliminary data
        bundler.initialize_bundler(params);

        // Collect future imports (already done in initialize_bundler)
        let future_imports = bundler.future_imports.clone();

        // Collect circular modules (already identified in prepare_modules, but we'll capture them
        // here)
        let circular_modules = bundler.circular_modules.clone();

        // Find namespace-imported modules
        // Convert modules to the format expected by find_namespace_imported_modules
        let mut modules_map = FxIndexMap::default();
        for (module_id, ast, hash) in params.modules {
            let path = params
                .resolver
                .get_module_path(*module_id)
                .unwrap_or_else(|| {
                    let name = params
                        .resolver
                        .get_module_name(*module_id)
                        .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
                    std::path::PathBuf::from(&name)
                });
            modules_map.insert(*module_id, (ast.clone(), path, hash.clone()));
        }

        bundler.find_namespace_imported_modules(&modules_map);
        let namespace_imported_modules = bundler.namespace_imported_modules.clone();

        InitializationResult {
            future_imports,
            circular_modules,
            namespace_imported_modules,
        }
    }
}

/// Generate future import statements for the bundle
///
/// This converts the collected future imports into AST statements
/// that should be placed at the beginning of the bundle.
pub fn generate_future_import_statements(result: &InitializationResult) -> Vec<Stmt> {
    if result.future_imports.is_empty() {
        return Vec::new();
    }

    let mut future_import_names: Vec<String> = result.future_imports.iter().cloned().collect();
    // Sort for deterministic output
    future_import_names.sort();

    let aliases = future_import_names
        .iter()
        .map(|name| crate::ast_builder::other::alias(name, None))
        .collect();

    let future_import_stmt =
        crate::ast_builder::statements::import_from(Some("__future__"), aliases, 0);

    log::debug!("Added future imports to bundle: {future_import_names:?}");

    vec![future_import_stmt]
}

#[cfg(test)]
mod tests {
    use indexmap::IndexSet as FxIndexSet;

    use super::*;

    #[test]
    fn test_generate_future_import_statements_empty() {
        let result = InitializationResult {
            future_imports: FxIndexSet::default(),
            circular_modules: FxIndexSet::default(),
            namespace_imported_modules: FxIndexMap::default(),
        };

        let stmts = generate_future_import_statements(&result);

        assert!(stmts.is_empty());
    }

    #[test]
    fn test_generate_future_import_statements_with_imports() {
        let mut future_imports = FxIndexSet::default();
        future_imports.insert("annotations".to_string());
        future_imports.insert("division".to_string());

        let result = InitializationResult {
            future_imports,
            circular_modules: FxIndexSet::default(),
            namespace_imported_modules: FxIndexMap::default(),
        };

        let stmts = generate_future_import_statements(&result);

        assert_eq!(stmts.len(), 1);
        // Verify it's an import statement
        assert!(matches!(stmts[0], Stmt::ImportFrom(_)));
    }

    #[test]
    fn test_future_imports_deterministic_ordering() {
        let mut future_imports = FxIndexSet::default();
        // Insert in non-alphabetical order
        future_imports.insert("with_statement".to_string());
        future_imports.insert("annotations".to_string());
        future_imports.insert("division".to_string());

        let result = InitializationResult {
            future_imports,
            circular_modules: FxIndexSet::default(),
            namespace_imported_modules: FxIndexMap::default(),
        };

        let stmts1 = generate_future_import_statements(&result);
        let stmts2 = generate_future_import_statements(&result);

        // Should produce identical output (deterministic)
        assert_eq!(format!("{stmts1:?}"), format!("{:?}", stmts2));
    }

    #[test]
    fn test_initialization_result_construction() {
        let mut future_imports = FxIndexSet::default();
        future_imports.insert("annotations".to_string());

        let result = InitializationResult {
            future_imports: future_imports.clone(),
            circular_modules: FxIndexSet::default(),
            namespace_imported_modules: FxIndexMap::default(),
        };

        assert_eq!(result.future_imports.len(), 1);
        assert!(result.future_imports.contains("annotations"));
        assert!(result.circular_modules.is_empty());
        assert!(result.namespace_imported_modules.is_empty());
    }
}
