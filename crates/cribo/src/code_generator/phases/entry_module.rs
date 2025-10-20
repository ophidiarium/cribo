//! Entry Module Phase
//!
//! This phase handles special processing for the entry module, including:
//! - Circular dependency statement reordering
//! - Import transformation and deduplication
//! - Child module exposure at module level
//! - Namespace attachment for package __init__.py

use ruff_python_ast::{ModModule, Stmt};

use crate::{
    code_generator::{bundler::Bundler, context::BundleParams},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Entry module phase handler (stateless)
#[derive(Default)]
pub(crate) struct EntryModulePhase;

/// Result from processing the entry module
#[derive(Debug, Clone)]
pub(crate) struct EntryModuleProcessingResult {
    /// Transformed and processed entry module statements
    pub statements: Vec<Stmt>,
    /// Locally defined symbols in the entry module
    pub entry_symbols: FxIndexSet<String>,
    /// Symbol renames for the entry module
    pub entry_renames: FxIndexMap<String, String>,
}

impl EntryModulePhase {
    /// Create a new entry module phase
    pub(crate) const fn new() -> Self {
        Self
    }

    /// Execute the entry module processing phase
    ///
    /// This method:
    /// 1. Extracts the entry module from the modules map
    /// 2. Reorders statements if entry is part of circular dependencies
    /// 3. Collects locally defined symbols
    /// 4. Transforms imports using `RecursiveImportTransformer`
    /// 5. Processes and deduplicates statements
    /// 6. Exposes child modules at module level
    ///
    /// Returns processed statements, symbols, and renames for the entry module.
    pub(crate) fn execute(
        &self,
        bundler: &Bundler<'_>,
        params: &BundleParams<'_>,
        modules: &mut FxIndexMap<ModuleId, (ModModule, std::path::PathBuf, String)>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        final_body: &[Stmt],
    ) -> Option<EntryModuleProcessingResult> {
        // Extract entry module
        let (mut ast, _module_path, _) = modules.shift_remove(&ModuleId::ENTRY)?;

        let module_name = bundler
            .resolver
            .get_module_name(ModuleId::ENTRY)
            .expect("Entry module must have a name");

        log::debug!("Processing entry module: '{module_name}'");
        log::debug!("Entry module has {} statements", ast.body.len());

        // Reorder statements if entry is in circular dependencies
        if bundler.is_module_in_circular_deps(ModuleId::ENTRY) {
            ast.body = Self::reorder_entry_module_statements(
                bundler,
                &module_name,
                ast.body,
                params.python_version,
            );
        }

        // Get entry module renames
        let entry_module_renames = symbol_renames
            .get(&ModuleId::ENTRY)
            .cloned()
            .unwrap_or_default();

        log::debug!("Entry module '{module_name}' renames: {entry_module_renames:?}");

        // Collect locally defined symbols
        let entry_module_symbols = Self::collect_entry_symbols(&ast);
        log::debug!("Entry module locally defined symbols: {entry_module_symbols:?}");

        // Transform imports
        Self::transform_entry_imports(bundler, &mut ast, symbol_renames, params.python_version);

        // Process statements with deduplication
        let mut entry_statements = Vec::new();
        Self::process_entry_statements(
            bundler,
            &ast,
            &entry_module_symbols,
            &entry_module_renames,
            final_body,
            params.python_version,
            &mut entry_statements,
        );

        // Add child module exposure
        Self::expose_child_modules(
            bundler,
            &module_name,
            &entry_module_symbols,
            &mut entry_statements,
        );

        Some(EntryModuleProcessingResult {
            statements: entry_statements,
            entry_symbols: entry_module_symbols,
            entry_renames: entry_module_renames,
        })
    }

    /// Reorder entry module statements for circular dependencies
    fn reorder_entry_module_statements(
        bundler: &Bundler<'_>,
        module_name: &str,
        body: Vec<Stmt>,
        python_version: u8,
    ) -> Vec<Stmt> {
        let lookup_name = if crate::util::is_init_module(module_name) {
            bundler
                .circular_modules
                .iter()
                .filter_map(|id| bundler.resolver.get_module_name(*id))
                .find(|name| !name.contains('.') && !crate::util::is_init_module(name))
                .unwrap_or_else(|| module_name.to_owned())
        } else {
            module_name.to_owned()
        };

        log::debug!(
            "Entry module '{module_name}' is part of circular dependencies, reordering statements \
             (lookup: '{lookup_name}')"
        );

        bundler.reorder_statements_for_circular_module(&lookup_name, body, python_version)
    }

    /// Collect locally defined symbols in the entry module
    fn collect_entry_symbols(ast: &ModModule) -> FxIndexSet<String> {
        let mut locally_defined_symbols = FxIndexSet::default();
        for stmt in &ast.body {
            match stmt {
                Stmt::FunctionDef(func_def) => {
                    locally_defined_symbols.insert(func_def.name.to_string());
                }
                Stmt::ClassDef(class_def) => {
                    locally_defined_symbols.insert(class_def.name.to_string());
                }
                _ => {}
            }
        }
        locally_defined_symbols
    }

    /// Transform imports in the entry module
    fn transform_entry_imports(
        bundler: &Bundler<'_>,
        ast: &mut ModModule,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        python_version: u8,
    ) {
        use crate::code_generator::import_transformer::{
            RecursiveImportTransformer, RecursiveImportTransformerParams,
        };

        log::debug!("Transforming imports for entry module");

        let params = RecursiveImportTransformerParams {
            bundler,
            module_id: ModuleId::ENTRY,
            symbol_renames,
            is_wrapper_init: false,
            python_version,
        };

        let mut transformer = RecursiveImportTransformer::new(&params);

        // Pre-populate stdlib aliases from ENTRY module
        let mut entry_stdlib_aliases: FxIndexMap<String, String> = FxIndexMap::default();
        for stmt in &ast.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let imported = alias.name.as_str();
                        let root = imported.split('.').next().unwrap_or(imported);
                        if ruff_python_stdlib::sys::is_known_standard_library(python_version, root)
                        {
                            let local = alias
                                .asname
                                .as_ref()
                                .map_or(imported, ruff_python_ast::Identifier::as_str);
                            entry_stdlib_aliases.insert(local.to_owned(), imported.to_owned());
                        }
                    }
                }
                Stmt::ImportFrom(import_from) => {
                    bundler.collect_aliases_from_stdlib_from_import(
                        import_from,
                        python_version,
                        &mut entry_stdlib_aliases,
                    );
                }
                _ => {}
            }
        }

        for (alias_name, module_name) in entry_stdlib_aliases {
            let rewritten_path = Bundler::get_rewritten_stdlib_path(&module_name);
            log::debug!("Entry stdlib alias: {alias_name} -> {rewritten_path}");
            transformer
                .import_aliases_mut()
                .insert(alias_name, rewritten_path);
        }

        transformer.transform_module(ast);
    }

    /// Process entry module statements with deduplication
    #[expect(clippy::too_many_arguments)]
    fn process_entry_statements(
        bundler: &Bundler<'_>,
        ast: &ModModule,
        locally_defined_symbols: &FxIndexSet<String>,
        entry_module_renames: &FxIndexMap<String, String>,
        final_body: &[Stmt],
        python_version: u8,
        entry_statements: &mut Vec<Stmt>,
    ) {
        use crate::code_generator::import_deduplicator;

        for stmt in &ast.body {
            // Check if this import was hoisted
            let is_hoisted = import_deduplicator::is_hoisted_import(bundler, stmt);
            if is_hoisted {
                continue;
            }

            match stmt {
                Stmt::ImportFrom(import_from) => {
                    let duplicate = import_deduplicator::is_duplicate_import_from(
                        bundler,
                        import_from,
                        final_body,
                        python_version,
                    );

                    if duplicate {
                        log::debug!(
                            "Skipping duplicate import in entry module: {:?}",
                            import_from.module
                        );
                    } else {
                        entry_statements.push(stmt.clone());
                    }
                }
                Stmt::Import(import_stmt) => {
                    let duplicate =
                        import_deduplicator::is_duplicate_import(bundler, import_stmt, final_body);

                    if !duplicate {
                        entry_statements.push(stmt.clone());
                    }
                }
                Stmt::Assign(assign) => {
                    if Bundler::is_import_for_local_symbol(assign, locally_defined_symbols) {
                        log::debug!("Skipping import assignment for locally defined symbol");
                        continue;
                    }

                    let is_duplicate = Self::check_duplicate_assignment(assign, final_body);

                    if !is_duplicate {
                        let mut stmt_clone = stmt.clone();
                        bundler.process_entry_module_statement(
                            &mut stmt_clone,
                            entry_module_renames,
                            entry_statements,
                        );
                    }
                }
                _ => {
                    let mut stmt_clone = stmt.clone();
                    bundler.process_entry_module_statement(
                        &mut stmt_clone,
                        entry_module_renames,
                        entry_statements,
                    );
                }
            }
        }
    }

    /// Check if an assignment is a duplicate
    fn check_duplicate_assignment(
        assign: &ruff_python_ast::StmtAssign,
        final_body: &[Stmt],
    ) -> bool {
        use ruff_python_ast::Expr;

        if assign.targets.len() == 1 {
            match &assign.targets[0] {
                Expr::Name(_) => Bundler::is_duplicate_name_assignment(assign, final_body),
                Expr::Attribute(_) => {
                    if Bundler::is_duplicate_module_init_attr_assignment(assign, final_body) {
                        log::debug!("Found duplicate module init in final_body");
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }

    /// Expose child modules at module level for the entry module
    fn expose_child_modules(
        bundler: &Bundler<'_>,
        module_name: &str,
        entry_module_symbols: &FxIndexSet<String>,
        entry_statements: &mut Vec<Stmt>,
    ) {
        use ruff_python_ast::ExprContext;

        use crate::ast_builder::{expressions, statements};

        if module_name != bundler.entry_module_name {
            return;
        }

        log::debug!("Adding module-level exposure for child modules of entry module {module_name}");

        let package_name = if crate::util::is_init_module(module_name) {
            module_name
                .strip_suffix(&format!(".{}", crate::python::constants::INIT_STEM))
                .map(ToString::to_string)
                .or_else(|| bundler.infer_entry_root_package())
                .unwrap_or_else(|| module_name.to_owned())
        } else {
            module_name.to_owned()
        };

        log::debug!("Package name for exposure: {package_name}");

        let entry_child_modules: Vec<String> = bundler
            .bundled_modules
            .iter()
            .filter_map(|id| {
                bundler.resolver.get_module_name(*id).filter(|name| {
                    name.starts_with(&format!("{package_name}.")) && name.contains('.')
                })
            })
            .collect();

        // Collect existing variables to avoid conflicts
        let existing_variables: FxIndexSet<String> = entry_statements
            .iter()
            .filter_map(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && let [ruff_python_ast::Expr::Name(name)] = assign.targets.as_slice()
                {
                    Some(name.id.to_string())
                } else {
                    None
                }
            })
            .collect();

        for child_module in entry_child_modules {
            if let Some(local_name) = child_module.strip_prefix(&format!("{package_name}."))
                && !local_name.contains('.')
                && !existing_variables.contains(local_name)
                && !entry_module_symbols.contains(local_name)
            {
                log::debug!("Exposing child module {child_module} as {local_name}");

                let expose_stmt = statements::simple_assign(
                    local_name,
                    expressions::attribute(
                        expressions::name(&package_name, ExprContext::Load),
                        local_name,
                        ExprContext::Load,
                    ),
                );
                entry_statements.push(expose_stmt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_module_processing_result_construction() {
        let mut entry_symbols = FxIndexSet::default();
        entry_symbols.insert("foo".to_owned());

        let mut entry_renames = FxIndexMap::default();
        entry_renames.insert("bar".to_owned(), "bar_renamed".to_owned());

        let result = EntryModuleProcessingResult {
            statements: vec![],
            entry_symbols: entry_symbols.clone(),
            entry_renames: entry_renames.clone(),
        };

        assert!(result.statements.is_empty());
        assert_eq!(result.entry_symbols.len(), 1);
        assert_eq!(result.entry_renames.len(), 1);
    }

    #[test]
    fn test_entry_module_result_with_statements() {
        use ruff_python_ast::{AtomicNodeIndex, StmtExpr};
        use ruff_text_size::TextRange;

        let stmt = Stmt::Expr(StmtExpr {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            value: Box::new(ruff_python_ast::Expr::NumberLiteral(
                ruff_python_ast::ExprNumberLiteral {
                    node_index: AtomicNodeIndex::NONE,
                    range: TextRange::default(),
                    value: ruff_python_ast::Number::Int(ruff_python_ast::Int::ZERO),
                },
            )),
        });

        let result = EntryModuleProcessingResult {
            statements: vec![stmt],
            entry_symbols: FxIndexSet::default(),
            entry_renames: FxIndexMap::default(),
        };

        assert_eq!(result.statements.len(), 1);
    }

    #[test]
    fn test_entry_module_symbols_tracking() {
        let mut entry_symbols = FxIndexSet::default();
        entry_symbols.insert("main".to_owned());
        entry_symbols.insert("helper".to_owned());
        entry_symbols.insert("Config".to_owned());

        let result = EntryModuleProcessingResult {
            statements: vec![],
            entry_symbols: entry_symbols.clone(),
            entry_renames: FxIndexMap::default(),
        };

        assert_eq!(result.entry_symbols.len(), 3);
        assert!(result.entry_symbols.contains("main"));
        assert!(result.entry_symbols.contains("helper"));
        assert!(result.entry_symbols.contains("Config"));
    }

    #[test]
    fn test_entry_module_renames_tracking() {
        let mut entry_renames = FxIndexMap::default();
        entry_renames.insert("main".to_owned(), "main_requests".to_owned());
        entry_renames.insert("session".to_owned(), "session_requests".to_owned());

        let result = EntryModuleProcessingResult {
            statements: vec![],
            entry_symbols: FxIndexSet::default(),
            entry_renames: entry_renames.clone(),
        };

        assert_eq!(result.entry_renames.len(), 2);
        assert_eq!(
            result.entry_renames.get("main"),
            Some(&"main_requests".to_owned())
        );
        assert_eq!(
            result.entry_renames.get("session"),
            Some(&"session_requests".to_owned())
        );
    }
}
