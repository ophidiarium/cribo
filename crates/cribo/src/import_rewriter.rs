/// Import rewriter that moves module-level imports into function scope
/// to resolve circular dependencies
use log::{debug, trace};
use ruff_python_ast::{
    self as ast, ModModule, Stmt, StmtFunctionDef, StmtImportFrom, visitor::Visitor,
};

use crate::{
    cribo_graph::CriboGraph,
    resolver::ModuleId,
    semantic_bundler::SemanticBundler,
    types::{FxIndexMap, FxIndexSet},
    visitors::{DiscoveredImport, ImportDiscoveryVisitor, ImportLocation},
};

/// Strategy for deduplicating imports within functions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportDeduplicationStrategy {
    /// Place import at the start of the function
    FunctionStart,
}

/// Information about an import that can be moved
#[derive(Debug, Clone)]
pub struct MovableImport {
    /// The original import statement
    pub import_stmt: ImportStatement,
    /// Functions that use this import
    pub target_functions: Vec<String>,
    /// The source module containing this import
    pub source_module_id: ModuleId,
}

/// Represents an import statement in a normalized form
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportStatement {
    /// Regular import: `import module` or `import module as alias`
    Import {
        module: String,
        alias: Option<String>,
    },
    /// From import: `from module import name` or `from module import name as alias`
    FromImport {
        module: Option<String>,
        names: Vec<(String, Option<String>)>,
        level: u32,
    },
}

/// Import rewriter that transforms module-level imports to function-level
pub struct ImportRewriter {
    /// Import deduplication strategy
    dedup_strategy: ImportDeduplicationStrategy,
}

impl ImportRewriter {
    /// Create a new import rewriter
    pub fn new(dedup_strategy: ImportDeduplicationStrategy) -> Self {
        Self { dedup_strategy }
    }

    /// Analyze movable imports using semantic analysis for accurate context detection
    pub fn analyze_movable_imports_semantic(
        &mut self,
        graph: &CriboGraph,
        resolvable_cycles: &[crate::analyzers::types::CircularDependencyGroup],
        semantic_bundler: &SemanticBundler,
        module_asts: &FxIndexMap<ModuleId, &ModModule>,
    ) -> Vec<MovableImport> {
        let mut movable_imports = Vec::new();

        // Cache to avoid re-analyzing modules that appear in multiple cycles
        let mut module_import_cache: FxIndexMap<ModuleId, Vec<DiscoveredImport>> =
            FxIndexMap::default();

        for cycle in resolvable_cycles {
            debug!(
                "Analyzing cycle of type {:?} with {} modules using semantic analysis",
                cycle.cycle_type,
                cycle.modules.len()
            );

            // Only handle function-level cycles
            if !matches!(
                cycle.cycle_type,
                crate::analyzers::types::CircularDependencyType::FunctionLevel
            ) {
                continue;
            }

            // For each module in the cycle, find imports that can be moved
            for &module_id in &cycle.modules {
                // Get module name from graph (for logging only)
                let module_name = if let Some(module) = graph.modules.get(&module_id) {
                    &module.module_name
                } else {
                    continue;
                };

                // Check if we've already analyzed this module
                let discovered_imports = if let Some(cached_imports) =
                    module_import_cache.get(&module_id)
                {
                    trace!("Using cached import analysis for module '{module_name}'");
                    cached_imports.clone()
                } else {
                    // Find the AST for this module using ModuleId
                    let Some(ast) = module_asts.get(&module_id) else {
                        continue;
                    };

                    // Perform semantic analysis using enhanced ImportDiscoveryVisitor
                    let mut visitor =
                        ImportDiscoveryVisitor::with_semantic_bundler(semantic_bundler, module_id);
                    for stmt in &ast.body {
                        visitor.visit_stmt(stmt);
                    }
                    let imports = visitor.into_imports();

                    // Cache the results for future use
                    module_import_cache.insert(module_id, imports.clone());
                    imports
                };

                // Find movable imports based on semantic analysis
                let candidates = self.find_movable_imports_from_discovered(
                    &discovered_imports,
                    module_id,
                    &cycle.modules,
                    graph,
                );
                movable_imports.extend(candidates);
            }
        }

        debug!(
            "Found {} movable imports using semantic analysis",
            movable_imports.len()
        );
        movable_imports
    }

    /// Find movable imports based on discovered imports with semantic analysis
    fn find_movable_imports_from_discovered(
        &self,
        discovered_imports: &[DiscoveredImport],
        source_module_id: ModuleId,
        cycle_module_ids: &[ModuleId],
        graph: &CriboGraph,
    ) -> Vec<MovableImport> {
        let mut movable = Vec::new();

        // Get module name for logging
        let module_name = graph
            .modules
            .get(&source_module_id)
            .map_or("<unknown>", |m| m.module_name.as_str());

        for import_info in discovered_imports {
            // Check if this import is part of the cycle
            if let Some(imported_module) = &import_info.module_name {
                if !self.is_import_in_cycle(imported_module, cycle_module_ids, graph) {
                    continue;
                }

                // Skip if not movable based on semantic analysis
                if !import_info.is_movable {
                    trace!(
                        "Import {imported_module} in {module_name} has side effects, cannot move"
                    );
                    continue;
                }

                // Import is movable, now determine target functions based on import location
                let target_functions = match &import_info.location {
                    ImportLocation::Function(func_name) => {
                        // Import is in a specific function, move it only to that function
                        trace!(
                            "Import {imported_module} in {module_name}::{func_name} can be moved \
                             to function scope"
                        );
                        vec![func_name.clone()]
                    }
                    ImportLocation::Method { class, method } => {
                        // Import is in a method, we need to handle this specially
                        // For now, skip methods as they're more complex
                        trace!(
                            "Import {imported_module} in {module_name}::{class}::{method} is in a \
                             method, skipping"
                        );
                        continue;
                    }
                    ImportLocation::Module => {
                        // Module-level import that needs to be moved to all functions that use it
                        // This requires more complex analysis to determine which functions actually
                        // use it For now, move to all functions
                        trace!(
                            "Import {imported_module} in {module_name} is at module level, moving \
                             to all functions"
                        );
                        vec!["*".to_string()]
                    }
                    _ => {
                        // Other locations (Class, Conditional, Nested) are not handled yet
                        trace!(
                            "Import {imported_module} in {module_name} has complex location {:?}, \
                             skipping",
                            import_info.location
                        );
                        continue;
                    }
                };

                // Convert to ImportStatement
                let import_stmt =
                    if import_info.names.len() == 1 && import_info.names[0].0 == *imported_module {
                        // This is a regular import statement (e.g., "import foo" or "import foo as
                        // bar") For regular imports, the module name
                        // matches the first (and only) name in the names vector
                        let alias = import_info.names[0].1.clone();

                        ImportStatement::Import {
                            module: imported_module.clone(),
                            alias,
                        }
                    } else {
                        // This is a from import statement
                        ImportStatement::FromImport {
                            module: import_info.module_name.clone(),
                            names: import_info.names.clone(),
                            level: import_info.level,
                        }
                    };

                movable.push(MovableImport {
                    import_stmt,
                    target_functions,
                    source_module_id,
                });
            }
        }

        movable
    }

    /// Check if an import is part of a circular dependency cycle
    fn is_import_in_cycle(
        &self,
        imported_module_name: &str,
        cycle_module_ids: &[ModuleId],
        graph: &CriboGraph,
    ) -> bool {
        // Check each module ID in the cycle
        for &module_id in cycle_module_ids {
            if let Some(module) = graph.modules.get(&module_id) {
                // Direct match
                if module.module_name == imported_module_name {
                    return true;
                }
                // Check if it's a submodule
                if imported_module_name.starts_with(&format!("{}.", module.module_name)) {
                    return true;
                }
            }
        }
        false
    }

    /// Rewrite a module's AST to move imports into function scope
    pub fn rewrite_module(
        &mut self,
        module_ast: &mut ModModule,
        movable_imports: &[MovableImport],
        module_id: ModuleId,
    ) {
        debug!(
            "Rewriting module {:?} with {} movable imports",
            module_id,
            movable_imports.len()
        );

        // Filter imports for this module
        let module_imports: Vec<_> = movable_imports
            .iter()
            .filter(|mi| mi.source_module_id == module_id)
            .collect();

        if module_imports.is_empty() {
            return;
        }

        // Step 1: Remove module-level imports that will be moved
        let imports_to_remove = self.identify_imports_to_remove(&module_imports, &module_ast.body);
        self.remove_module_imports(module_ast, &imports_to_remove);

        // Step 2: Add imports to function bodies
        self.add_function_imports(module_ast, &module_imports);
    }

    /// Identify which statement indices contain imports to remove
    fn identify_imports_to_remove(
        &self,
        movable_imports: &[&MovableImport],
        body: &[Stmt],
    ) -> FxIndexSet<usize> {
        let mut indices_to_remove = FxIndexSet::default();

        for (idx, stmt) in body.iter().enumerate() {
            match stmt {
                Stmt::Import(import_stmt) => {
                    // Check if all aliases in the import are movable
                    let all_aliases_movable = import_stmt.names.iter().all(|alias| {
                        let import = ImportStatement::Import {
                            module: alias.name.to_string(),
                            alias: alias.asname.as_ref().map(std::string::ToString::to_string),
                        };
                        movable_imports.iter().any(|mi| mi.import_stmt == import)
                    });
                    if all_aliases_movable {
                        indices_to_remove.insert(idx);
                    }
                }
                Stmt::ImportFrom(import_from) => {
                    if self.matches_any_movable_import(import_from, movable_imports) {
                        indices_to_remove.insert(idx);
                    }
                }
                _ => {}
            }
        }

        indices_to_remove
    }

    /// Check if an import statement matches any movable import
    fn matches_any_movable_import(
        &self,
        import_from: &StmtImportFrom,
        movable_imports: &[&MovableImport],
    ) -> bool {
        let stmt_module = import_from
            .module
            .as_ref()
            .map(std::string::ToString::to_string);
        let stmt_level = import_from.level;

        movable_imports.iter().any(|mi| {
            self.import_matches_statement(&mi.import_stmt, &stmt_module, stmt_level, import_from)
        })
    }

    /// Check if a movable import matches an import statement
    fn import_matches_statement(
        &self,
        import: &ImportStatement,
        stmt_module: &Option<String>,
        stmt_level: u32,
        import_from: &StmtImportFrom,
    ) -> bool {
        match import {
            ImportStatement::FromImport {
                module,
                level,
                names,
            } => {
                // Module and level must match
                if module != stmt_module || level != &stmt_level {
                    return false;
                }

                // Check if all names in the movable import are present in the statement
                self.all_names_present_in_statement(names, &import_from.names)
            }
            _ => false,
        }
    }

    /// Check if all names are present in the statement
    fn all_names_present_in_statement(
        &self,
        names: &[(String, Option<String>)],
        stmt_names: &[ast::Alias],
    ) -> bool {
        // For exact matching, both lists must have the same length
        if names.len() != stmt_names.len() {
            return false;
        }

        // Create sorted representations for order-independent comparison
        let mut sorted_names: Vec<_> = names
            .iter()
            .map(|(name, alias)| (name.as_str(), alias.as_deref()))
            .collect();
        sorted_names.sort();

        let mut sorted_stmt_names: Vec<_> = stmt_names
            .iter()
            .map(|alias| {
                (
                    alias.name.as_str(),
                    alias
                        .asname
                        .as_ref()
                        .map(ruff_python_ast::Identifier::as_str),
                )
            })
            .collect();
        sorted_stmt_names.sort();

        // Compare the sorted lists
        sorted_names == sorted_stmt_names
    }

    /// Remove module-level imports
    fn remove_module_imports(
        &self,
        module_ast: &mut ModModule,
        indices_to_remove: &FxIndexSet<usize>,
    ) {
        // Remove imports in reverse order to maintain indices
        let mut indices: Vec<_> = indices_to_remove.iter().copied().collect();
        indices.sort_by(|a, b| b.cmp(a));

        for idx in indices {
            module_ast.body.remove(idx);
        }
    }

    /// Add imports to function bodies
    fn add_function_imports(&self, module_ast: &mut ModModule, module_imports: &[&MovableImport]) {
        // Group imports by target function
        let mut imports_by_function: FxIndexMap<String, Vec<&MovableImport>> =
            FxIndexMap::default();

        for import in module_imports {
            for func_name in &import.target_functions {
                imports_by_function
                    .entry(func_name.clone())
                    .or_default()
                    .push(import);
            }
        }

        // Add imports to each function
        for stmt in &mut module_ast.body {
            if let Stmt::FunctionDef(func_def) = stmt {
                let func_name = func_def.name.to_string();

                // Combine wildcard ("*") imports with function-specific imports
                let mut combined: Vec<&MovableImport> = Vec::new();
                if let Some(all) = imports_by_function.get("*") {
                    combined.extend_from_slice(all);
                }
                if let Some(specific) = imports_by_function.get(&func_name) {
                    combined.extend_from_slice(specific);
                }
                if !combined.is_empty() {
                    self.add_imports_to_function_body(func_def, &combined);
                }
            }
        }
    }

    /// Add import statements to a function body
    fn add_imports_to_function_body(
        &self,
        func_def: &mut StmtFunctionDef,
        imports: &[&MovableImport],
    ) {
        // First, collect existing imports in the function to avoid duplicates
        let mut existing_imports = FxIndexSet::default();
        for stmt in &func_def.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        existing_imports.insert(ImportStatement::Import {
                            module: alias.name.to_string(),
                            alias: alias.asname.as_ref().map(std::string::ToString::to_string),
                        });
                    }
                }
                Stmt::ImportFrom(from_stmt) => {
                    let names: Vec<(String, Option<String>)> = from_stmt
                        .names
                        .iter()
                        .map(|alias| {
                            (
                                alias.name.to_string(),
                                alias.asname.as_ref().map(std::string::ToString::to_string),
                            )
                        })
                        .collect();
                    existing_imports.insert(ImportStatement::FromImport {
                        module: from_stmt
                            .module
                            .as_ref()
                            .map(std::string::ToString::to_string),
                        names,
                        level: from_stmt.level,
                    });
                }
                _ => {}
            }
        }

        // Deduplicate imports based on their ImportStatement
        let mut seen_imports = existing_imports; // Start with existing imports
        let mut import_stmts = Vec::new();

        for movable_import in imports {
            // Only add the import if we haven't seen this exact import statement before
            if seen_imports.insert(movable_import.import_stmt.clone()) {
                let stmt = self.create_import_statement(&movable_import.import_stmt);
                import_stmts.push(stmt);
            }
        }

        // Insert imports at the beginning of the function body, but after any docstring
        match self.dedup_strategy {
            ImportDeduplicationStrategy::FunctionStart => {
                // Insert imports after a leading docstring, if present
                let insert_at = usize::from(
                    func_def
                        .body
                        .first()
                        .is_some_and(ruff_python_ast::helpers::is_docstring_stmt),
                );
                func_def.body.splice(insert_at..insert_at, import_stmts);
            }
        }
    }

    /// Create an AST import statement from our normalized representation
    fn create_import_statement(&self, import: &ImportStatement) -> Stmt {
        use crate::ast_builder::{other, statements};

        match import {
            ImportStatement::Import { module, alias } => {
                let alias_stmt = other::alias(module, alias.as_deref());
                statements::import(vec![alias_stmt])
            }
            ImportStatement::FromImport {
                module,
                names,
                level,
            } => {
                let aliases = names
                    .iter()
                    .map(|(name, alias)| other::alias(name, alias.as_deref()))
                    .collect();

                statements::import_from(module.as_deref(), aliases, *level)
            }
        }
    }
}
