//! Initialization Phase
//!
//! This phase handles the initial setup of the bundler, including:
//! - Storing graph and semantic bundler references
//! - Initializing bundler settings
//! - Collecting future imports from all modules

use ruff_python_ast::Stmt;

use crate::code_generator::{
    bundler::Bundler,
    context::{BundleParams, InitializationResult},
};

/// Initialization phase handler (stateless)
#[derive(Default)]
pub(crate) struct InitializationPhase;

impl InitializationPhase {
    /// Create a new initialization phase
    pub(crate) const fn new() -> Self {
        Self
    }

    /// Execute the initialization phase
    ///
    /// This method:
    /// 1. Stores references to the graph and semantic bundler
    /// 2. Initializes bundler settings (tree shaking, __all__ access, entry module info)
    /// 3. Collects future imports from all modules
    ///
    /// Returns an `InitializationResult` containing the collected future imports.
    /// Note: Circular dependencies and namespace imports are discovered later in
    /// `prepare_modules` phase.
    pub(crate) fn execute<'a>(
        &self,
        bundler: &mut Bundler<'a>,
        params: &BundleParams<'a>,
    ) -> InitializationResult {
        // Store the graph reference for use in transformation methods
        bundler.graph = Some(params.graph);

        // Store the semantic bundler reference for use in transformations
        bundler.conflict_resolver = Some(params.conflict_resolver);

        // Initialize bundler settings and collect preliminary data
        bundler.initialize_bundler(params);

        // Collect future imports (already done in initialize_bundler)
        let future_imports = bundler.future_imports.clone();

        // Circular modules are already identified in prepare_modules and stored in bundler
        // Namespace-imported modules are discovered during prepare_modules(); no need to precompute
        // here.

        InitializationResult { future_imports }
    }
}

/// Generate future import statements for the bundle
///
/// This converts the collected future imports into AST statements
/// that should be placed at the beginning of the bundle.
pub(crate) fn generate_future_import_statements(result: &InitializationResult) -> Vec<Stmt> {
    if result.future_imports.is_empty() {
        return Vec::new();
    }

    let mut names: Vec<&str> = result.future_imports.iter().map(String::as_str).collect();
    // Sort for deterministic output
    names.sort_unstable();

    let aliases = names
        .iter()
        .map(|name| crate::ast_builder::other::alias(name, None))
        .collect();

    let future_import_stmt =
        crate::ast_builder::statements::import_from(Some("__future__"), aliases, 0);

    log::debug!("Added future imports to bundle: {names:?}");

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
        };

        let stmts = generate_future_import_statements(&result);

        assert!(stmts.is_empty());
    }

    #[test]
    fn test_generate_future_import_statements_with_imports() {
        let mut future_imports = FxIndexSet::default();
        future_imports.insert("annotations".to_owned());
        future_imports.insert("division".to_owned());

        let result = InitializationResult { future_imports };

        let stmts = generate_future_import_statements(&result);

        assert_eq!(stmts.len(), 1);
        // Verify it's an import statement
        assert!(matches!(stmts[0], Stmt::ImportFrom(_)));
    }

    #[test]
    fn test_future_imports_deterministic_ordering() {
        let mut future_imports = FxIndexSet::default();
        // Insert in non-alphabetical order
        future_imports.insert("with_statement".to_owned());
        future_imports.insert("annotations".to_owned());
        future_imports.insert("division".to_owned());

        let result = InitializationResult { future_imports };

        let stmts1 = generate_future_import_statements(&result);
        let stmts2 = generate_future_import_statements(&result);

        // Should produce identical output (deterministic)
        assert_eq!(format!("{stmts1:?}"), format!("{:?}", stmts2));
    }

    #[test]
    fn test_initialization_result_construction() {
        let mut future_imports = FxIndexSet::default();
        future_imports.insert("annotations".to_owned());

        let result = InitializationResult {
            future_imports: future_imports.clone(),
        };

        assert_eq!(result.future_imports.len(), 1);
        assert!(result.future_imports.contains("annotations"));
    }
}
