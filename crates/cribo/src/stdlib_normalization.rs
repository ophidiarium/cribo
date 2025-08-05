use log::debug;
use ruff_python_ast::{ExprContext, Identifier, ModModule, Stmt, StmtImport};
use ruff_text_size::TextRange;

use crate::{
    ast_builder::{expressions, statements},
    code_generator::expression_handlers,
    side_effects::is_safe_stdlib_module,
    types::{FxIndexMap, FxIndexSet},
};

/// Result of stdlib normalization
pub struct NormalizationResult {
    /// Mapping of aliases to canonical names (e.g., "`PyPath`" -> "pathlib.Path")
    pub alias_to_canonical: FxIndexMap<String, String>,
    /// Set of modules that were created by normalization (e.g., "abc", "collections")
    pub normalized_modules: FxIndexSet<String>,
}

/// Normalizes stdlib import aliases within a module's AST
/// Converts "import json as j" to "import json" and rewrites all "j.dumps" to "json.dumps"
/// Also converts "from pathlib import Path as `PyPath`" to "import pathlib" and rewrites "`PyPath`"
/// to "pathlib.Path"
pub fn normalize_stdlib_imports(ast: &mut ModModule) -> NormalizationResult {
    let normalizer = StdlibNormalizer::new();
    normalizer.normalize(ast)
}

struct StdlibNormalizer {
    // No state needed for now
}

impl StdlibNormalizer {
    fn new() -> Self {
        Self {}
    }

    /// Main normalization entry point
    fn normalize(&self, ast: &mut ModModule) -> NormalizationResult {
        // Step 1: Build alias-to-canonical mapping for this file
        let mut alias_to_canonical = FxIndexMap::default();
        let mut modules_to_convert = FxIndexSet::default();

        for stmt in &ast.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    self.collect_stdlib_aliases(import_stmt, &mut alias_to_canonical);
                }
                Stmt::ImportFrom(import_from) => {
                    // Skip relative imports
                    if import_from.level > 0 {
                        continue;
                    }

                    if let Some(ref module) = import_from.module {
                        let module_name = module.as_str();
                        if is_safe_stdlib_module(module_name) {
                            // Extract the root module for stdlib imports
                            let root_module = module_name.split('.').next().unwrap_or(module_name);

                            // Collect all imports from "from" statements for normalization
                            for alias in &import_from.names {
                                let name = alias.name.as_str();
                                if let Some(ref alias_name) = alias.asname {
                                    // Map alias to module.name (e.g., PyPath -> pathlib.Path)
                                    let canonical = format!("{module_name}.{name}");
                                    alias_to_canonical
                                        .insert(alias_name.as_str().to_string(), canonical);
                                } else {
                                    // Even without alias, we need to convert to module.name form
                                    // e.g., "Any" -> "typing.Any"
                                    let canonical = format!("{module_name}.{name}");
                                    alias_to_canonical.insert(name.to_string(), canonical);
                                }
                            }

                            // Convert ALL stdlib "from" imports to regular imports
                            // This applies to typing, collections.abc, etc.
                            modules_to_convert.insert(root_module.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        if alias_to_canonical.is_empty() && modules_to_convert.is_empty() {
            return NormalizationResult {
                alias_to_canonical,
                normalized_modules: FxIndexSet::default(),
            };
        }

        debug!("Normalizing stdlib aliases: {alias_to_canonical:?}");
        debug!("Modules to convert from 'from' imports: {modules_to_convert:?}");

        // Step 2: Transform all expressions that reference aliases
        for (idx, stmt) in ast.body.iter_mut().enumerate() {
            match stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // We'll handle import statements separately
                }
                _ => {
                    let stmt_type = match stmt {
                        Stmt::FunctionDef(f) => format!("FunctionDef({})", f.name.as_str()),
                        Stmt::ClassDef(c) => format!("ClassDef({})", c.name.as_str()),
                        Stmt::Assign(_) => "Assign".to_string(),
                        Stmt::Expr(_) => "Expr".to_string(),
                        _ => format!("{:?}", std::mem::discriminant(stmt)),
                    };
                    debug!("Rewriting aliases in statement at index {idx}: {stmt_type}");
                    expression_handlers::rewrite_aliases_in_stmt(stmt, &alias_to_canonical);
                }
            }
        }

        // Step 3: Transform import statements
        let mut new_imports = Vec::new();
        let mut indices_to_remove = Vec::new();
        // Track assignments we need to add for implicit exports
        // Maps module name to list of (local_name, full_path) tuples
        let mut implicit_exports: FxIndexMap<String, Vec<(String, String)>> = FxIndexMap::default();

        for (idx, stmt) in ast.body.iter_mut().enumerate() {
            match stmt {
                Stmt::Import(import_stmt) => {
                    debug!(
                        "Processing import statement at index {}: {:?}",
                        idx,
                        import_stmt
                            .names
                            .iter()
                            .map(|a| (
                                a.name.as_str(),
                                a.asname.as_ref().map(ruff_python_ast::Identifier::as_str)
                            ))
                            .collect::<Vec<_>>()
                    );
                    self.normalize_import_aliases(import_stmt);
                }
                Stmt::ImportFrom(import_from) => {
                    // Skip relative imports
                    if import_from.level > 0 {
                        continue;
                    }

                    if let Some(ref module) = import_from.module {
                        let module_name = module.as_str();
                        // Check if this is a safe stdlib module or submodule
                        if is_safe_stdlib_module(module_name) {
                            // Extract the root module name
                            let root_module = module_name.split('.').next().unwrap_or(module_name);

                            if modules_to_convert.contains(root_module) {
                                // Mark this import for removal - we'll convert it to a regular
                                // import
                                indices_to_remove.push(idx);

                                // For submodules like collections.abc, we need to import the full
                                // module path not just the root
                                // module
                                if !new_imports.iter().any(|m: &String| m == module_name) {
                                    new_imports.push(module_name.to_string());
                                }

                                // Collect implicit exports that need assignment statements
                                // e.g., from collections.abc import MutableMapping
                                // becomes: import collections.abc + MutableMapping =
                                // collections.abc.MutableMapping
                                self.process_import_from_names(
                                    import_from,
                                    module_name,
                                    &mut new_imports,
                                    &mut implicit_exports,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Step 4: Remove the from imports and add regular imports
        // Remove in reverse order to maintain indices
        for idx in indices_to_remove.into_iter().rev() {
            ast.body.remove(idx);
        }

        // Add the new regular imports at the beginning (after __future__ imports)
        let future_import_count = ast
            .body
            .iter()
            .take_while(|stmt| {
                if let Stmt::ImportFrom(import_from) = stmt {
                    import_from
                        .module
                        .as_ref()
                        .is_some_and(|m| m.as_str() == "__future__")
                } else {
                    false
                }
            })
            .count();

        let mut insert_position = future_import_count;
        let mut normalized_modules = FxIndexSet::default();

        for module_name in new_imports.into_iter().rev() {
            // Track this module as normalized
            normalized_modules.insert(module_name.clone());

            let import_stmt = Stmt::Import(StmtImport {
                node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                names: vec![ruff_python_ast::Alias {
                    node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                    name: Identifier::new(&module_name, TextRange::default()),
                    asname: None,
                    range: TextRange::default(),
                }],
                range: TextRange::default(),
            });
            ast.body.insert(insert_position, import_stmt);
            insert_position += 1;

            // Add assignment statements for implicit exports from this module
            if let Some(exports) = implicit_exports.get(&module_name) {
                for (local_name, full_path) in exports {
                    let assign_stmt = self.create_assignment_statement(local_name, full_path);
                    ast.body.insert(insert_position, assign_stmt);
                    insert_position += 1;
                }
            }
        }

        NormalizationResult {
            alias_to_canonical,
            normalized_modules,
        }
    }

    /// Check if a path refers to a known stdlib submodule
    fn is_known_stdlib_submodule(&self, module_path: &str) -> bool {
        // Check if this is a stdlib module itself (not just an attribute)
        match module_path {
            // Known stdlib submodules that need separate imports
            "http.cookiejar"
            | "http.cookies"
            | "http.server"
            | "http.client"
            | "urllib.parse"
            | "urllib.request"
            | "urllib.response"
            | "urllib.error"
            | "urllib.robotparser"
            | "xml.etree"
            | "xml.etree.ElementTree"
            | "xml.dom"
            | "xml.sax"
            | "xml.parsers"
            | "email.mime"
            | "email.parser"
            | "email.message"
            | "email.utils"
            | "collections.abc"
            | "concurrent.futures"
            | "importlib.util"
            | "importlib.machinery"
            | "importlib.resources"
            | "multiprocessing.pool"
            | "multiprocessing.managers"
            | "os.path" => true,
            _ => {
                // For other cases, check if it's a known stdlib module
                let root = module_path.split('.').next().unwrap_or(module_path);
                if ruff_python_stdlib::sys::is_known_standard_library(10, root) {
                    // If the root is stdlib and the full path is also recognized as stdlib,
                    // it's likely a submodule
                    ruff_python_stdlib::sys::is_known_standard_library(10, module_path)
                } else {
                    false
                }
            }
        }
    }

    /// Create an assignment statement: `local_name` = `full_path`
    fn create_assignment_statement(&self, local_name: &str, full_path: &str) -> Stmt {
        // Parse the full path to create attribute access
        // e.g., "collections.abc.MutableMapping" becomes collections.abc.MutableMapping
        let value_expr =
            expressions::dotted_name(&full_path.split('.').collect::<Vec<_>>(), ExprContext::Load);

        // Create assignment: local_name = full_path
        statements::simple_assign(local_name, value_expr)
    }

    /// Collect stdlib aliases from import statement
    fn collect_stdlib_aliases(
        &self,
        import_stmt: &StmtImport,
        alias_to_canonical: &mut FxIndexMap<String, String>,
    ) {
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            if !is_safe_stdlib_module(module_name) {
                continue;
            }
            if let Some(ref alias_name) = alias.asname {
                // This is an aliased import: import json as j
                alias_to_canonical.insert(alias_name.as_str().to_string(), module_name.to_string());
            }
        }
    }

    /// Normalize import aliases by removing them for stdlib modules
    fn normalize_import_aliases(&self, import_stmt: &mut StmtImport) {
        for alias in &mut import_stmt.names {
            let module_name = alias.name.as_str();
            if !is_safe_stdlib_module(module_name) {
                debug!("Skipping non-safe stdlib module: {module_name}");
                continue;
            }
            if alias.asname.is_none() {
                continue;
            }
            // Remove the alias, keeping only the canonical name
            alias.asname = None;
            debug!("Normalized import to canonical: import {module_name}");
        }
    }

    // All the rewrite functions have been moved to expression_handlers.rs

    /// Process names from an import-from statement and collect imports and exports
    fn process_import_from_names(
        &self,
        import_from: &ruff_python_ast::StmtImportFrom,
        module_name: &str,
        new_imports: &mut Vec<String>,
        implicit_exports: &mut FxIndexMap<String, Vec<(String, String)>>,
    ) {
        for alias in &import_from.names {
            let name = alias.name.as_str();
            if name == "*" {
                continue; // Skip star imports
            }

            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Check if this is importing a submodule (e.g., from http import cookiejar)
            let submodule_path = format!("{module_name}.{name}");
            if self.is_known_stdlib_submodule(&submodule_path) {
                self.handle_submodule_import(
                    &submodule_path,
                    local_name,
                    module_name,
                    new_imports,
                    implicit_exports,
                );
            } else {
                // Regular attribute import
                let full_path = format!("{module_name}.{name}");
                implicit_exports
                    .entry(module_name.to_string())
                    .or_default()
                    .push((local_name.to_string(), full_path));
            }
        }
    }

    /// Handle submodule imports by adding to `new_imports` and `implicit_exports`
    fn handle_submodule_import(
        &self,
        submodule_path: &str,
        local_name: &str,
        module_name: &str,
        new_imports: &mut Vec<String>,
        implicit_exports: &mut FxIndexMap<String, Vec<(String, String)>>,
    ) {
        // This is a submodule import, we need to import it separately
        if !new_imports.iter().any(|m: &String| m == submodule_path) {
            new_imports.push(submodule_path.to_string());
        }
        // And create assignment: local_name = submodule_path
        implicit_exports
            .entry(module_name.to_string())
            .or_default()
            .push((local_name.to_string(), submodule_path.to_string()));
    }
}
