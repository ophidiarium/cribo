//! Import analysis module
//!
//! This module provides functionality for analyzing import patterns,
//! including direct imports, namespace imports, and import relationships.

use std::path::PathBuf;

use log::debug;
use ruff_python_ast::{Expr, ModModule, Stmt, StmtImportFrom};

use crate::{
    analyzers::types::UnusedImportInfo,
    cribo_graph::{ItemData, ItemId},
    types::{FxIndexMap, FxIndexSet},
};

/// Import analyzer for processing import patterns and relationships
pub struct ImportAnalyzer;

/// Context for checking if an import is unused
struct ImportUsageContext<'a> {
    imported_name: &'a str,
    import_id: ItemId,
    is_init_py: bool,
    import_data: &'a ItemData,
    module: &'a crate::cribo_graph::ModuleDepGraph,
}

impl ImportAnalyzer {
    /// Find modules that are imported directly (e.g., `import module`)
    pub fn find_directly_imported_modules(
        modules: &[(String, ModModule, PathBuf, String)],
        _entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut directly_imported = FxIndexSet::default();

        // Pre-compute module names for O(1) lookup performance
        let module_names: FxIndexSet<&str> = modules
            .iter()
            .map(|(name, _, _, _)| name.as_str())
            .collect();

        // Check all modules for direct imports (both module-level and function-scoped)
        for (module_name, ast, module_path, _) in modules {
            debug!("Checking module '{module_name}' for direct imports");

            // Check the module body
            Self::collect_direct_imports_recursive(
                &ast.body,
                module_name,
                module_path,
                &module_names,
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

        // Pre-compute module names for O(1) lookup performance
        let module_names: FxIndexSet<&str> = modules
            .iter()
            .map(|(name, _, _, _)| name.as_str())
            .collect();

        // Check all modules for namespace imports
        for (importing_module, ast, _, _) in modules {
            debug!("Checking module '{importing_module}' for namespace imports");
            for stmt in &ast.body {
                Self::collect_namespace_imports(
                    stmt,
                    modules,
                    &module_names,
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

    /// Find unused imports in a specific module
    pub fn find_unused_imports_in_module(
        module: &crate::cribo_graph::ModuleDepGraph,
        is_init_py: bool,
    ) -> Vec<UnusedImportInfo> {
        let mut unused_imports = Vec::new();

        // First, collect all imported names
        let imported_items: Vec<_> = module.get_all_import_items();

        // For each imported name, check if it's used
        for (import_id, import_data) in imported_items {
            for imported_name in &import_data.imported_names {
                let ctx = ImportUsageContext {
                    imported_name,
                    import_id,
                    is_init_py,
                    import_data,
                    module,
                };

                if Self::is_import_unused(&ctx) {
                    let module_name = match &import_data.item_type {
                        crate::cribo_graph::ItemType::Import { module, .. }
                        | crate::cribo_graph::ItemType::FromImport { module, .. } => module.clone(),
                        _ => continue,
                    };

                    unused_imports.push(UnusedImportInfo {
                        name: imported_name.clone(),
                        module: module_name,
                    });
                }
            }
        }

        unused_imports
    }

    /// Check if a specific imported name is unused
    fn is_import_unused(ctx: &ImportUsageContext<'_>) -> bool {
        // Check for special cases where imports should be preserved
        if ctx.is_init_py {
            // In __init__.py, preserve all imports as they might be part of the public API
            return false;
        }

        // Check if it's a star import
        if let crate::cribo_graph::ItemType::FromImport { is_star: true, .. } =
            &ctx.import_data.item_type
        {
            // Star imports are always preserved
            return false;
        }

        // Check if it's explicitly re-exported
        if ctx.import_data.reexported_names.contains(ctx.imported_name) {
            return false;
        }

        // Check if the import has side effects (includes stdlib imports)
        if ctx.import_data.has_side_effects {
            return false;
        }

        // Fetch the import item's declared variables once to avoid repeated lookups
        let import_vars_to_check = ctx
            .module
            .items
            .get(&ctx.import_id)
            .map(|item| &item.var_decls);

        // Check if the name is used anywhere in the module
        for (item_id, item_data) in &ctx.module.items {
            // Skip the import statement itself
            if *item_id == ctx.import_id {
                continue;
            }

            // Check if the name is read by this item
            if item_data.read_vars.contains(ctx.imported_name)
                || item_data.eventual_read_vars.contains(ctx.imported_name)
            {
                log::trace!(
                    "Import '{}' is used by item {:?} (read_vars: {:?}, eventual_read_vars: {:?})",
                    ctx.imported_name,
                    item_id,
                    item_data.read_vars,
                    item_data.eventual_read_vars
                );
                return false;
            }

            // For dotted imports like `import xml.etree.ElementTree`, also check if any of the
            // declared variables from that import are used
            if let Some(import_vars) = import_vars_to_check {
                let is_var_used = import_vars.iter().any(|var_decl| {
                    item_data.read_vars.contains(var_decl)
                        || item_data.eventual_read_vars.contains(var_decl)
                });

                if is_var_used {
                    log::trace!(
                        "Import '{}' is used via declared variables by item {:?}",
                        ctx.imported_name,
                        item_id
                    );
                    return false;
                }
            }
        }

        // Check if the name is in the module's __all__ export list
        if Self::is_in_module_exports(ctx.module, ctx.imported_name) {
            return false;
        }

        log::trace!("Import '{}' is UNUSED", ctx.imported_name);
        true
    }

    /// Check if a name is in the module's __all__ export list
    /// This is the single source of truth for __all__ exports, using the `reexported_names`
    /// field which is populated by the `ExportCollector` during graph building
    fn is_in_module_exports(module: &crate::cribo_graph::ModuleDepGraph, name: &str) -> bool {
        // Look for __all__ assignment
        for item_data in module.items.values() {
            if let crate::cribo_graph::ItemType::Assignment { targets } = &item_data.item_type
                && targets.contains(&"__all__".to_string())
            {
                // Check if the name is in the reexported_names set
                // which contains the parsed __all__ list values from ExportCollector
                return item_data.reexported_names.contains(name);
            }
        }
        false
    }

    /// Collect direct imports recursively through the AST
    fn collect_direct_imports_recursive(
        body: &[Stmt],
        current_module: &str,
        _module_path: &std::path::Path,
        module_names: &FxIndexSet<&str>,
        directly_imported: &mut FxIndexSet<String>,
    ) {
        for stmt in body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let import_name = alias.name.to_string();
                        debug!("Found direct import '{import_name}' in module '{current_module}'");

                        // Check if this import corresponds to a module we're bundling
                        if module_names.contains(import_name.as_str()) {
                            directly_imported.insert(import_name);
                        }
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    // Recursively check function bodies
                    Self::collect_direct_imports_recursive(
                        &func_def.body,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                }
                Stmt::ClassDef(class_def) => {
                    // Recursively check class bodies
                    Self::collect_direct_imports_recursive(
                        &class_def.body,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                }
                Stmt::If(if_stmt) => {
                    // Check if branches
                    Self::collect_direct_imports_recursive(
                        &if_stmt.body,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                    for clause in &if_stmt.elif_else_clauses {
                        Self::collect_direct_imports_recursive(
                            &clause.body,
                            current_module,
                            _module_path,
                            module_names,
                            directly_imported,
                        );
                    }
                }
                Stmt::While(while_stmt) => {
                    // Check while body
                    Self::collect_direct_imports_recursive(
                        &while_stmt.body,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                    // Check else clause
                    Self::collect_direct_imports_recursive(
                        &while_stmt.orelse,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                }
                Stmt::For(for_stmt) => {
                    // Check for body
                    Self::collect_direct_imports_recursive(
                        &for_stmt.body,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                    // Check else clause
                    Self::collect_direct_imports_recursive(
                        &for_stmt.orelse,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                }
                Stmt::Try(try_stmt) => {
                    // Check try body
                    Self::collect_direct_imports_recursive(
                        &try_stmt.body,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                    // Check except handlers
                    for handler in &try_stmt.handlers {
                        let ruff_python_ast::ExceptHandler::ExceptHandler(except_handler) = handler;
                        Self::collect_direct_imports_recursive(
                            &except_handler.body,
                            current_module,
                            _module_path,
                            module_names,
                            directly_imported,
                        );
                    }
                    // Check else clause
                    Self::collect_direct_imports_recursive(
                        &try_stmt.orelse,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                    // Check finally clause
                    Self::collect_direct_imports_recursive(
                        &try_stmt.finalbody,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                }
                Stmt::With(with_stmt) => {
                    // Check with body
                    Self::collect_direct_imports_recursive(
                        &with_stmt.body,
                        current_module,
                        _module_path,
                        module_names,
                        directly_imported,
                    );
                }
                Stmt::Match(match_stmt) => {
                    // Check match cases
                    for case in &match_stmt.cases {
                        Self::collect_direct_imports_recursive(
                            &case.body,
                            current_module,
                            _module_path,
                            module_names,
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
        module_names: &FxIndexSet<&str>,
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
                        let is_namespace_import = module_names.contains(full_module_path.as_str());

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
                        module_names,
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
                        module_names,
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
                        module_names,
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
                            module_names,
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
                        module_names,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
                // Also check else clause
                for stmt in &while_stmt.orelse {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        module_names,
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
                        module_names,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
                // Also check else clause
                for stmt in &for_stmt.orelse {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        module_names,
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
                        module_names,
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
                            module_names,
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
                        module_names,
                        importing_module,
                        namespace_imported_modules,
                    );
                }
                // Check finally clause
                for stmt in &try_stmt.finalbody {
                    Self::collect_namespace_imports(
                        stmt,
                        modules,
                        module_names,
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
                        module_names,
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
                            module_names,
                            importing_module,
                            namespace_imported_modules,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// Check if a symbol from a module is imported by any other module in the bundle
    ///
    /// This function is used to determine if private symbols (e.g., starting with underscore)
    /// should still be exported because they're imported by other modules.
    pub fn is_symbol_imported_by_other_modules(
        module_asts: &FxIndexMap<crate::resolver::ModuleId, (ModModule, PathBuf, String)>,
        module_id: crate::resolver::ModuleId,
        symbol_name: &str,
        module_exports: Option<&FxIndexMap<crate::resolver::ModuleId, Option<Vec<String>>>>,
        resolver: &crate::resolver::ModuleResolver,
    ) -> bool {
        let module_name = resolver
            .get_module_name(module_id)
            .unwrap_or_else(|| format!("module#{module_id}"));
        debug!(
            "Checking imports for symbol '{}' from module '{}' in {} modules",
            symbol_name,
            module_name,
            module_asts.len()
        );

        // Look through all modules to see if any import this symbol
        for (other_module_id, (ast, _, _)) in module_asts {
            // Skip the module itself
            if other_module_id == &module_id {
                continue;
            }

            let other_module_name = resolver
                .get_module_name(*other_module_id)
                .unwrap_or_else(|| format!("module#{other_module_id}"));

            // Check import statements in the module
            if Self::module_imports_symbol_with_ids(
                ast,
                *other_module_id,
                module_id,
                symbol_name,
                module_exports,
                resolver,
            ) {
                debug!(
                    "Symbol '{symbol_name}' from module '{module_name}' is imported by module \
                     '{other_module_name}'"
                );
                return true;
            }
        }

        debug!(
            "Symbol '{symbol_name}' from module '{module_name}' is not imported by any other \
             modules"
        );
        false
    }

    /// Check if a module imports a specific symbol from another module (using `ModuleIds`)
    fn module_imports_symbol_with_ids(
        ast: &ModModule,
        importing_module_id: crate::resolver::ModuleId,
        target_module_id: crate::resolver::ModuleId,
        symbol_name: &str,
        module_exports: Option<&FxIndexMap<crate::resolver::ModuleId, Option<Vec<String>>>>,
        resolver: &crate::resolver::ModuleResolver,
    ) -> bool {
        // Convert to strings for now - we'll need to update SymbolImportVisitor later
        let importing_module = resolver
            .get_module_name(importing_module_id)
            .unwrap_or_else(|| format!("module#{importing_module_id}"));
        let target_module = resolver
            .get_module_name(target_module_id)
            .unwrap_or_else(|| format!("module#{target_module_id}"));

        // Convert module_exports to String-based for the visitor
        let module_exports_strings: FxIndexMap<String, Option<Vec<String>>> = module_exports
            .map(|exports| {
                exports
                    .iter()
                    .filter_map(|(id, export_list)| {
                        resolver
                            .get_module_name(*id)
                            .map(|name| (name, export_list.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut visitor = SymbolImportVisitor::new(
            &importing_module,
            &target_module,
            symbol_name,
            if module_exports_strings.is_empty() {
                None
            } else {
                Some(&module_exports_strings)
            },
        );
        ruff_python_ast::visitor::walk_body(&mut visitor, &ast.body);
        visitor.found
    }

    /// Check if an import statement imports from the target module
    fn import_matches_module(
        import_from: &StmtImportFrom,
        importing_module: &str,
        target_module: &str,
    ) -> bool {
        if let Some(import_module) = &import_from.module {
            let import_module_str = import_module.as_str();

            // Handle both absolute and relative imports
            if import_from.level == 0 {
                // Absolute import: "from rich.cells import ..."
                import_module_str == target_module
            } else {
                // Relative import: use the centralized helper
                let resolved_module = crate::resolver::resolve_relative_import_from_name(
                    import_from.level,
                    Some(import_module_str),
                    importing_module,
                );
                resolved_module == target_module
            }
        } else {
            false
        }
    }

    /// Collect module-level absolute `from __future__ import ...` names
    ///
    /// Ignores invalid wildcard imports and any non-absolute (level > 0) forms.
    ///
    /// Performance optimization: Returns early when encountering non-import/non-docstring
    /// statements, as `__future__` imports must appear at the top of the file per Python's
    /// specification (after module docstring but before any other code).
    pub fn collect_future_imports(ast: &ModModule) -> FxIndexSet<String> {
        let mut future_imports = FxIndexSet::default();
        let mut seen_docstring = false;

        for stmt in &ast.body {
            match stmt {
                // Module docstring can appear before __future__ imports
                Stmt::Expr(expr_stmt) if !seen_docstring => {
                    // Check if this is a string literal (docstring)
                    if matches!(expr_stmt.value.as_ref(), Expr::StringLiteral(_)) {
                        seen_docstring = true;
                        continue;
                    }
                    // Non-docstring expression - no more __future__ imports allowed
                    break;
                }

                // Process import statements
                Stmt::ImportFrom(import_from) => {
                    // Only process absolute __future__ imports
                    if import_from.level == 0
                        && import_from
                            .module
                            .as_deref()
                            .is_some_and(|m| m == "__future__")
                    {
                        for alias in &import_from.names {
                            let name = alias.name.as_str();
                            // Ignore wildcard imports (invalid for __future__)
                            if name != "*" {
                                future_imports.insert(name.to_string());
                            }
                        }
                    }
                    // Continue checking - there might be more imports
                }

                // Regular imports are allowed before/after __future__ imports
                Stmt::Import(_) => continue,

                // Any other statement type means no more __future__ imports are allowed
                _ => break,
            }
        }

        future_imports
    }
}

/// Visitor that checks if a specific symbol is imported from a target module
struct SymbolImportVisitor<'a> {
    importing_module: &'a str,
    target_module: &'a str,
    symbol_name: &'a str,
    module_exports: Option<&'a FxIndexMap<String, Option<Vec<String>>>>,
    found: bool,
}

impl<'a> SymbolImportVisitor<'a> {
    fn new(
        importing_module: &'a str,
        target_module: &'a str,
        symbol_name: &'a str,
        module_exports: Option<&'a FxIndexMap<String, Option<Vec<String>>>>,
    ) -> Self {
        Self {
            importing_module,
            target_module,
            symbol_name,
            module_exports,
            found: false,
        }
    }

    fn check_import_from(&mut self, import_from: &StmtImportFrom) {
        if ImportAnalyzer::import_matches_module(
            import_from,
            self.importing_module,
            self.target_module,
        ) {
            // Check if the symbol is being imported
            for alias in &import_from.names {
                let alias_name = alias.name.as_str();
                if alias_name == self.symbol_name {
                    self.found = true;
                    return;
                }
                // Handle wildcard imports (from module import *)
                if alias_name == "*" {
                    // If we have module export information, check if the symbol is in __all__
                    if let Some(exports_map) = self.module_exports {
                        if let Some(Some(export_list)) = exports_map.get(self.target_module) {
                            // Module has __all__, check if symbol is in it
                            if export_list.iter().any(|s| s == self.symbol_name) {
                                self.found = true;
                                return;
                            }
                        } else {
                            // No __all__ defined, wildcard imports public names only
                            if !self.symbol_name.starts_with('_') {
                                self.found = true;
                                return;
                            }
                        }
                    } else {
                        // No export information available, conservatively treat wildcard
                        // as importing only public names (non-underscore)
                        if !self.symbol_name.starts_with('_') {
                            self.found = true;
                            return;
                        }
                    }
                }
            }
        }
    }
}

impl<'a> ruff_python_ast::visitor::Visitor<'a> for SymbolImportVisitor<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Early exit if we've already found what we're looking for
        if self.found {
            return;
        }

        match stmt {
            Stmt::ImportFrom(import_from) => {
                self.check_import_from(import_from);
            }
            _ => {
                // Continue traversing for other statement types
                ruff_python_ast::visitor::walk_stmt(self, stmt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_find_directly_imported_modules() {
        let code1 = r"
import module_a
import module_b as mb

def func():
    import module_c
";
        let parsed1 = parse_module(code1).expect("Test code should parse successfully");
        let ast1 = parsed1.into_syntax();

        let code2 = r"
def other_func():
    pass
";
        let parsed2 = parse_module(code2).expect("Test code should parse successfully");
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
    fn test_find_directly_imported_modules_in_compound_statements() {
        let code = r#"
# Test all compound statements
try:
    import module_in_try
except:
    import module_in_except
else:
    import module_in_else
finally:
    import module_in_finally

for i in range(1):
    import module_in_for

while False:
    import module_in_while

with open("test") as f:
    import module_in_with

match x:
    case _:
        import module_in_match
"#;
        let parsed = parse_module(code).expect("Test code should parse successfully");
        let ast = parsed.into_syntax();

        let dummy_ast = parse_module("pass")
            .expect("Dummy module should parse")
            .into_syntax();

        let modules = vec![
            (
                "test_module".to_string(),
                ast,
                PathBuf::from("test.py"),
                "hash1".to_string(),
            ),
            (
                "module_in_try".to_string(),
                dummy_ast.clone(),
                PathBuf::from("module_in_try.py"),
                "hash2".to_string(),
            ),
            (
                "module_in_except".to_string(),
                dummy_ast.clone(),
                PathBuf::from("module_in_except.py"),
                "hash3".to_string(),
            ),
            (
                "module_in_else".to_string(),
                dummy_ast.clone(),
                PathBuf::from("module_in_else.py"),
                "hash4".to_string(),
            ),
            (
                "module_in_finally".to_string(),
                dummy_ast.clone(),
                PathBuf::from("module_in_finally.py"),
                "hash5".to_string(),
            ),
            (
                "module_in_for".to_string(),
                dummy_ast.clone(),
                PathBuf::from("module_in_for.py"),
                "hash6".to_string(),
            ),
            (
                "module_in_while".to_string(),
                dummy_ast.clone(),
                PathBuf::from("module_in_while.py"),
                "hash7".to_string(),
            ),
            (
                "module_in_with".to_string(),
                dummy_ast.clone(),
                PathBuf::from("module_in_with.py"),
                "hash8".to_string(),
            ),
            (
                "module_in_match".to_string(),
                dummy_ast,
                PathBuf::from("module_in_match.py"),
                "hash9".to_string(),
            ),
        ];

        let directly_imported =
            ImportAnalyzer::find_directly_imported_modules(&modules, "test_module");

        // All imports should be found
        assert_eq!(directly_imported.len(), 8);
        assert!(directly_imported.contains("module_in_try"));
        assert!(directly_imported.contains("module_in_except"));
        assert!(directly_imported.contains("module_in_else"));
        assert!(directly_imported.contains("module_in_finally"));
        assert!(directly_imported.contains("module_in_for"));
        assert!(directly_imported.contains("module_in_while"));
        assert!(directly_imported.contains("module_in_with"));
        assert!(directly_imported.contains("module_in_match"));
    }

    #[test]
    fn test_find_namespace_imported_modules() {
        let code1 = r"
from pkg import module_a
from pkg.sub import module_b
";
        let parsed1 = parse_module(code1).expect("Test code should parse successfully");
        let ast1 = parsed1.into_syntax();

        let code2 = r"pass";
        let parsed2 = parse_module(code2).expect("Test code should parse successfully");
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
                .expect("pkg.module_a should be in namespace_imported")
                .contains("test_module")
        );
        assert!(
            namespace_imported
                .get("pkg.sub.module_b")
                .expect("pkg.sub.module_b should be in namespace_imported")
                .contains("test_module")
        );
    }

    #[test]
    fn test_collect_future_imports_basic() {
        let src = r"
from __future__ import annotations
from __future__ import generator_stop, unicode_literals
from something import else_
";
        let ast = parse_module(src).expect("Should parse").into_syntax();

        let got = ImportAnalyzer::collect_future_imports(&ast);
        assert_eq!(got.len(), 3);
        assert!(got.contains("annotations"));
        assert!(got.contains("generator_stop"));
        assert!(got.contains("unicode_literals"));
    }

    #[test]
    fn test_collect_future_imports_ignores_invalid_cases() {
        // Test that wildcard imports and relative imports are ignored
        let src = r"
from __future__ import annotations
from __future__ import *
from .__future__ import division
from ..__future__ import print_function
";
        let ast = parse_module(src).expect("Should parse").into_syntax();

        let got = ImportAnalyzer::collect_future_imports(&ast);
        // Should only contain 'annotations', wildcards and relative imports are ignored
        assert_eq!(got.len(), 1);
        assert!(got.contains("annotations"));
        assert!(!got.contains("*"));
        assert!(!got.contains("division"));
        assert!(!got.contains("print_function"));
    }

    #[test]
    fn test_collect_future_imports_with_docstring() {
        // Test that __future__ imports after module docstring are collected
        let src = r#"
"""Module docstring."""
from __future__ import annotations, print_function
import sys
from __future__ import division
"#;
        let ast = parse_module(src).expect("Should parse").into_syntax();

        let got = ImportAnalyzer::collect_future_imports(&ast);
        // Should collect both annotations and print_function
        assert_eq!(got.len(), 3);
        assert!(got.contains("annotations"));
        assert!(got.contains("print_function"));
        assert!(got.contains("division"));
    }

    #[test]
    fn test_collect_future_imports_early_return() {
        // Test that collection stops at first non-import statement
        let src = r"
from __future__ import annotations
import sys
x = 1  # This stops future import scanning
from __future__ import division  # This won't be collected
";
        let ast = parse_module(src).expect("Should parse").into_syntax();

        let got = ImportAnalyzer::collect_future_imports(&ast);
        // Should only contain 'annotations' - stops at x = 1
        assert_eq!(got.len(), 1);
        assert!(got.contains("annotations"));
        assert!(!got.contains("division"));
    }
}
