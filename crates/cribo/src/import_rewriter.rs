/// Import rewriter that moves module-level imports into function scope
/// to resolve circular dependencies
use anyhow::Result;
use log::{debug, trace};
use ruff_python_ast::{
    self as ast, Identifier, ModModule, Stmt, StmtFunctionDef, StmtImport, StmtImportFrom,
};
use ruff_text_size::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::cribo_graph::{CriboGraph, ItemType, ModuleDepGraph};

/// Strategy for deduplicating imports within functions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportDeduplicationStrategy {
    /// Place import at the start of the function
    FunctionStart,
    /// Place import just before first use
    BeforeFirstUse,
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
    /// Line number of the original import
    pub line_number: usize,
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

    /// Analyze a module graph to identify imports that can be moved to break cycles
    pub fn analyze_movable_imports(
        &mut self,
        graph: &CriboGraph,
        resolvable_cycles: &[crate::cribo_graph::CircularDependencyGroup],
    ) -> Vec<MovableImport> {
        let mut movable_imports = Vec::new();

        for cycle in resolvable_cycles {
            debug!(
                "Analyzing cycle of type {:?} with {} modules",
                cycle.cycle_type,
                cycle.modules.len()
            );

            // Only handle function-level cycles
            if !matches!(
                cycle.cycle_type,
                crate::cribo_graph::CircularDependencyType::FunctionLevel
            ) {
                continue;
            }

            // For each module in the cycle, find imports that can be moved
            for module_name in &cycle.modules {
                if let Some(module_graph) = graph.get_module_by_name(module_name) {
                    let candidates = self.find_movable_imports_in_module(
                        module_graph,
                        module_name,
                        &cycle.modules,
                    );
                    movable_imports.extend(candidates);
                }
            }
        }

        debug!("Found {} movable imports", movable_imports.len());
        movable_imports
    }

    /// Find imports in a module that can be moved to function scope
    fn find_movable_imports_in_module(
        &self,
        module_graph: &ModuleDepGraph,
        module_name: &str,
        cycle_modules: &[String],
    ) -> Vec<MovableImport> {
        let mut movable = Vec::new();

        // Iterate through all items in the module
        for item_data in module_graph.items.values() {
            // Check if this is an import
            let import_info = match &item_data.item_type {
                ItemType::Import { module, alias } => Some((
                    ImportStatement::Import {
                        module: module.clone(),
                        alias: alias.clone(),
                    },
                    module.clone(),
                )),
                ItemType::FromImport {
                    module,
                    names,
                    level,
                    ..
                } => Some((
                    ImportStatement::FromImport {
                        module: Some(module.clone()),
                        names: names.clone(),
                        level: *level,
                    },
                    module.clone(),
                )),
                _ => None,
            };

            if let Some((import_stmt, imported_module)) = import_info {
                // Check if this import is part of the cycle
                if !self.is_import_in_cycle(&imported_module, cycle_modules) {
                    continue;
                }

                // Check if the import is only used in functions
                let usage = self.analyze_import_usage(module_graph, &item_data.imported_names);

                // Additionally check if the module itself has side effects
                let module_has_side_effects = Self::is_likely_side_effect_import(&imported_module);

                if usage.is_function_only && !usage.has_side_effects && !module_has_side_effects {
                    trace!(
                        "Import {:?} in {} can be moved to functions: {:?}",
                        import_stmt, module_name, usage.used_in_functions
                    );

                    movable.push(MovableImport {
                        import_stmt,
                        target_functions: usage.used_in_functions,
                        source_module: module_name.to_string(),
                        line_number: item_data.span.map(|(start, _)| start).unwrap_or(0),
                    });
                }
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
            if imported_module.starts_with(&format!("{}.", cycle_module)) {
                return true;
            }
        }

        false
    }

    /// Analyze how an import is used in the module
    fn analyze_import_usage(
        &self,
        module_graph: &ModuleDepGraph,
        imported_names: &FxHashSet<String>,
    ) -> ImportUsageAnalysis {
        let mut analysis = ImportUsageAnalysis {
            is_function_only: true,
            used_in_functions: Vec::new(),
            has_side_effects: false,
        };

        // Check if any imported names are from modules with side effects
        // Note: This only checks the imported symbol names, not the source module.
        // The source module is checked separately in find_movable_imports_in_module
        for name in imported_names {
            if Self::is_likely_side_effect_import(name) {
                analysis.has_side_effects = true;
            }
        }

        // Check all items in the module for usage of these imported names
        for item_data in module_graph.items.values() {
            // Check if any imported name is used at module level
            for imported_name in imported_names {
                if item_data.read_vars.contains(imported_name) {
                    // Used at module level - cannot be moved
                    match &item_data.item_type {
                        ItemType::FunctionDef { .. } | ItemType::ClassDef { .. } => {
                            // Decorators and annotations are module-level
                            analysis.is_function_only = false;
                            return analysis;
                        }
                        _ => {
                            analysis.is_function_only = false;
                            return analysis;
                        }
                    }
                }

                // Check if used in function bodies (eventual reads)
                if !item_data.eventual_read_vars.contains(imported_name) {
                    continue;
                }

                if let ItemType::FunctionDef { name } = &item_data.item_type {
                    if !analysis.used_in_functions.contains(name) {
                        analysis.used_in_functions.push(name.clone());
                    }
                }
            }
        }

        analysis
    }

    /// Check if an import is likely to have side effects
    fn is_likely_side_effect_import(name: &str) -> bool {
        // Modules that modify global state or have initialization side effects
        matches!(
            name,
            // From is_safe_stdlib_module's exclusion list
            "antigravity" | "this" | "__hello__" | "__phello__" |
            "site" | "sitecustomize" | "usercustomize" |
            "readline" | "rlcompleter" | 
            "turtle" | "tkinter" |
            "webbrowser" |
            "platform" | "locale" |
            // Additional modules with common side effects
            "os" | "sys" | "logging" | "warnings" | "encodings" |
            "pygame" | "matplotlib"
        ) || name.starts_with("_") // Private modules often have initialization side effects
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
                            alias: alias.asname.as_ref().map(|n| n.to_string()),
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
        let stmt_module = import_from.module.as_ref().map(|m| m.to_string());
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
                    alias.asname.as_ref().map(|n| n.as_str()),
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
            ImportDeduplicationStrategy::BeforeFirstUse => {
                // NOTE: BeforeFirstUse strategy is not yet implemented
                // This would require tracking variable usage locations within the function
                // and inserting imports just before their first use.
                // For now, we fall back to FunctionStart behavior.
                unimplemented!(
                    "BeforeFirstUse import placement strategy is not yet implemented. \
                     Use FunctionStart strategy instead."
                );
            }
        }

        Ok(())
    }

    /// Create an AST import statement from our normalized representation
    fn create_import_statement(&self, import: &ImportStatement) -> Result<Stmt> {
        match import {
            ImportStatement::Import { module, alias } => {
                let alias_stmt = ast::Alias {
                    name: Identifier::new(module.clone(), TextRange::default()),
                    asname: alias
                        .as_ref()
                        .map(|a| Identifier::new(a.clone(), TextRange::default())),
                    range: TextRange::default(),
                };

                Ok(Stmt::Import(StmtImport {
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
                        name: Identifier::new(name.clone(), TextRange::default()),
                        asname: alias
                            .as_ref()
                            .map(|a| Identifier::new(a.clone(), TextRange::default())),
                        range: TextRange::default(),
                    })
                    .collect();

                Ok(Stmt::ImportFrom(StmtImportFrom {
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

/// Analysis result for import usage
#[derive(Debug)]
struct ImportUsageAnalysis {
    /// Whether the import is only used in function bodies
    is_function_only: bool,
    /// Names of functions that use this import
    used_in_functions: Vec<String>,
    /// Whether the import has side effects
    has_side_effects: bool,
}
