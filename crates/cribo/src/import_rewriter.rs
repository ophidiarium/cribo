/// Import rewriter that moves module-level imports into function scope
/// to resolve circular dependencies
use anyhow::Result;
use log::{debug, trace};
use ruff_python_ast::{
    self as ast, AtomicNodeIndex, Identifier, ModModule, Stmt, StmtFunctionDef, StmtImport,
    StmtImportFrom, visitor::Visitor,
};
use ruff_text_size::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    cribo_graph::{CriboGraph, ModuleId},
    semantic_bundler::SemanticBundler,
    visitors::{DiscoveredImport, ImportDiscoveryVisitor},
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
    pub source_module: String,
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
        module_asts: &[(String, &ModModule)],
    ) -> Result<Vec<MovableImport>> {
        let mut movable_imports = Vec::new();

        // Cache to avoid re-analyzing modules that appear in multiple cycles
        let mut module_import_cache: FxHashMap<ModuleId, Vec<DiscoveredImport>> =
            FxHashMap::default();

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
            for module_name in &cycle.modules {
                if let Some(module_id) = graph.module_names.get(module_name) {
                    // Check if we've already analyzed this module
                    let discovered_imports =
                        if let Some(cached_imports) = module_import_cache.get(module_id) {
                            trace!("Using cached import analysis for module '{module_name}'");
                            cached_imports.clone()
                        } else {
                            // Find the AST for this module
                            if let Some((_, ast)) =
                                module_asts.iter().find(|(name, _)| name == module_name)
                            {
                                // Perform semantic analysis using enhanced ImportDiscoveryVisitor
                                let mut visitor = ImportDiscoveryVisitor::with_semantic_bundler(
                                    semantic_bundler,
                                    *module_id,
                                );
                                for stmt in &ast.body {
                                    visitor.visit_stmt(stmt);
                                }
                                let imports = visitor.into_imports();

                                // Cache the results for future use
                                module_import_cache.insert(*module_id, imports.clone());
                                imports
                            } else {
                                continue;
                            }
                        };

                    // Find movable imports based on semantic analysis
                    let candidates = self.find_movable_imports_from_discovered(
                        &discovered_imports,
                        module_name,
                        &cycle.modules,
                    );
                    movable_imports.extend(candidates);
                }
            }
        }

        debug!(
            "Found {} movable imports using semantic analysis",
            movable_imports.len()
        );
        Ok(movable_imports)
    }

    /// Find movable imports based on discovered imports with semantic analysis
    fn find_movable_imports_from_discovered(
        &self,
        discovered_imports: &[DiscoveredImport],
        module_name: &str,
        cycle_modules: &[String],
    ) -> Vec<MovableImport> {
        let mut movable = Vec::new();

        for import_info in discovered_imports {
            // Check if this import is part of the cycle
            if let Some(imported_module) = &import_info.module_name {
                if !self.is_import_in_cycle(imported_module, cycle_modules) {
                    continue;
                }

                // Skip if not movable based on semantic analysis
                if !import_info.is_movable {
                    trace!(
                        "Import {imported_module} in {module_name} has side effects, cannot move"
                    );
                    continue;
                }

                // Import is movable, now determine target functions
                // For now, we'll move to all functions (could be enhanced later)
                let target_functions = vec!["*".to_string()]; // Move to all functions

                trace!("Import {imported_module} in {module_name} can be moved to functions");

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
                    source_module: module_name.to_string(),
                });
            }
        }

        movable
    }

    /// Check if an import is part of a circular dependency cycle
    fn is_import_in_cycle(&self, imported_module: &str, cycle_modules: &[String]) -> bool {
        // Direct match
        if cycle_modules.contains(&imported_module.to_string()) {
            return true;
        }

        // Check if it's a submodule of any cycle module
        for cycle_module in cycle_modules {
            if imported_module.starts_with(&format!("{cycle_module}.")) {
                return true;
            }
        }

        false
    }

    /// Rewrite a module's AST to move imports into function scope
    pub fn rewrite_module(
        &mut self,
        module_ast: &mut ModModule,
        movable_imports: &[MovableImport],
        module_name: &str,
    ) -> Result<()> {
        debug!(
            "Rewriting module {} with {} movable imports",
            module_name,
            movable_imports.len()
        );

        // Filter imports for this module
        let module_imports: Vec<_> = movable_imports
            .iter()
            .filter(|mi| mi.source_module == module_name)
            .collect();

        if module_imports.is_empty() {
            return Ok(());
        }

        // Step 1: Remove module-level imports that will be moved
        let imports_to_remove = self.identify_imports_to_remove(&module_imports, &module_ast.body);
        self.remove_module_imports(module_ast, &imports_to_remove)?;

        // Step 2: Add imports to function bodies
        self.add_function_imports(module_ast, &module_imports)?;

        Ok(())
    }

    /// Identify which statement indices contain imports to remove
    fn identify_imports_to_remove(
        &self,
        movable_imports: &[&MovableImport],
        body: &[Stmt],
    ) -> FxHashSet<usize> {
        let mut indices_to_remove = FxHashSet::default();

        for (idx, stmt) in body.iter().enumerate() {
            match stmt {
                Stmt::Import(import_stmt) => {
                    // For regular imports, check each alias individually
                    for alias in &import_stmt.names {
                        let import = ImportStatement::Import {
                            module: alias.name.to_string(),
                            alias: alias.asname.as_ref().map(std::string::ToString::to_string),
                        };

                        if movable_imports.iter().any(|mi| mi.import_stmt == import) {
                            indices_to_remove.insert(idx);
                            break; // Once we find a match, mark the whole statement for removal
                        }
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
        indices_to_remove: &FxHashSet<usize>,
    ) -> Result<()> {
        // Remove imports in reverse order to maintain indices
        let mut indices: Vec<_> = indices_to_remove.iter().copied().collect();
        indices.sort_by(|a, b| b.cmp(a));

        for idx in indices {
            module_ast.body.remove(idx);
        }

        Ok(())
    }

    /// Add imports to function bodies
    fn add_function_imports(
        &self,
        module_ast: &mut ModModule,
        module_imports: &[&MovableImport],
    ) -> Result<()> {
        // Group imports by target function
        let mut imports_by_function: FxHashMap<String, Vec<&MovableImport>> = FxHashMap::default();

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

                if let Some(imports) = imports_by_function.get(&func_name) {
                    self.add_imports_to_function_body(func_def, imports.as_slice())?;
                }
            }
        }

        Ok(())
    }

    /// Add import statements to a function body
    fn add_imports_to_function_body(
        &self,
        func_def: &mut StmtFunctionDef,
        imports: &[&MovableImport],
    ) -> Result<()> {
        // Deduplicate imports based on their ImportStatement
        let mut seen_imports = FxHashSet::default();
        let mut import_stmts = Vec::new();

        for movable_import in imports {
            // Only add the import if we haven't seen this exact import statement before
            if seen_imports.insert(movable_import.import_stmt.clone()) {
                let stmt = self.create_import_statement(&movable_import.import_stmt)?;
                import_stmts.push(stmt);
            }
        }

        // Insert imports at the beginning of the function body
        match self.dedup_strategy {
            ImportDeduplicationStrategy::FunctionStart => {
                // Insert all imports at the start
                func_def.body.splice(0..0, import_stmts);
            }
        }

        Ok(())
    }

    /// Create an AST import statement from our normalized representation
    fn create_import_statement(&self, import: &ImportStatement) -> Result<Stmt> {
        match import {
            ImportStatement::Import { module, alias } => {
                let alias_stmt = ast::Alias {
                    node_index: AtomicNodeIndex::dummy(),
                    name: Identifier::new(module.clone(), TextRange::default()),
                    asname: alias
                        .as_ref()
                        .map(|a| Identifier::new(a.clone(), TextRange::default())),
                    range: TextRange::default(),
                };

                Ok(Stmt::Import(StmtImport {
                    node_index: AtomicNodeIndex::dummy(),
                    names: vec![alias_stmt],
                    range: TextRange::default(),
                }))
            }
            ImportStatement::FromImport {
                module,
                names,
                level,
            } => {
                let aliases: Vec<_> = names
                    .iter()
                    .map(|(name, alias)| ast::Alias {
                        node_index: AtomicNodeIndex::dummy(),
                        name: Identifier::new(name.clone(), TextRange::default()),
                        asname: alias
                            .as_ref()
                            .map(|a| Identifier::new(a.clone(), TextRange::default())),
                        range: TextRange::default(),
                    })
                    .collect();

                Ok(Stmt::ImportFrom(StmtImportFrom {
                    node_index: AtomicNodeIndex::dummy(),
                    module: module
                        .as_ref()
                        .map(|m| Identifier::new(m.clone(), TextRange::default())),
                    names: aliases,
                    level: *level,
                    range: TextRange::default(),
                }))
            }
        }
    }
}
