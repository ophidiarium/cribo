#![allow(clippy::excessive_nesting)]

use std::collections::VecDeque;

use log::{debug, info, trace, warn};

use crate::{
    cribo_graph::{CriboGraph, ItemData, ItemType},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Tree shaker that removes unused symbols from modules
#[derive(Debug)]
pub struct TreeShaker {
    /// Module items from semantic analysis (reused from `CriboGraph`)
    module_items: FxIndexMap<ModuleId, Vec<ItemData>>,
    /// Final set of symbols to keep (`module_id`, `symbol_name`)
    used_symbols: FxIndexSet<(ModuleId, String)>,
    /// Map from module ID to module name (for display/logging)
    module_names: FxIndexMap<ModuleId, String>,
    /// Map from module name to module ID (for handling entry point and imports)
    module_name_to_id: FxIndexMap<String, ModuleId>,
}

impl TreeShaker {
    /// Create a tree shaker from an existing `CriboGraph`
    pub fn from_graph(graph: &CriboGraph) -> Self {
        let mut module_items = FxIndexMap::default();
        let mut module_names = FxIndexMap::default();

        // Extract module items from the graph
        for (module_id, module_dep_graph) in &graph.modules {
            let module_name = module_dep_graph.module_name.clone();
            module_names.insert(*module_id, module_name.clone());

            // Collect all items for this module
            let items: Vec<ItemData> = module_dep_graph.items.values().cloned().collect();

            module_items.insert(*module_id, items);
        }

        // Clone the module_name -> ModuleId mapping from the graph
        // This includes all aliases (e.g., symlinks) that map to the same ModuleId
        let module_name_to_id = graph.module_names.clone();

        Self {
            module_items,
            used_symbols: FxIndexSet::default(),
            module_names,
            module_name_to_id,
        }
    }

    /// Helper to get module display name for logging
    fn get_module_display_name(&self, module_id: ModuleId) -> String {
        self.module_names
            .get(&module_id)
            .cloned()
            .unwrap_or_else(|| format!("<unknown module {module_id}>"))
    }

    /// Analyze which symbols should be kept based on entry point
    pub fn analyze(&mut self, entry_module: &str) {
        info!("Starting tree-shaking analysis from entry module: {entry_module}");

        // Verify that the entry module is registered with the expected ID
        let entry_id = self.module_name_to_id.get(entry_module).copied();
        if entry_id != Some(ModuleId::ENTRY) {
            warn!("Entry module '{entry_module}' not registered as ModuleId::ENTRY");
            if entry_id.is_none() {
                warn!("Entry module '{entry_module}' not found in module registry");
                return;
            }
        }

        // Then, mark symbols used from the entry module
        self.mark_used_symbols();

        info!(
            "Tree-shaking complete. Keeping {} symbols",
            self.used_symbols.len()
        );
    }

    /// Check if a symbol is defined in a specific module
    fn is_defined_in_module(&self, module_id: ModuleId, symbol: &str) -> bool {
        if let Some(items) = self.module_items.get(&module_id) {
            for item in items {
                if item.defined_symbols.contains(symbol) {
                    return true;
                }
            }
        }
        false
    }

    /// Find which module defines a symbol
    fn find_defining_module(&self, symbol: &str) -> Option<ModuleId> {
        for (&module_id, items) in &self.module_items {
            for item in items {
                if item.defined_symbols.contains(symbol) {
                    return Some(module_id);
                }
            }
        }
        None
    }

    /// Find which module defines a symbol, preferring the current module if it defines it
    fn find_defining_module_preferring_local(
        &self,
        current_module_id: ModuleId,
        symbol: &str,
    ) -> Option<ModuleId> {
        if self.is_defined_in_module(current_module_id, symbol) {
            Some(current_module_id)
        } else {
            self.find_defining_module(symbol)
        }
    }

    /// Resolve an import alias to its original module and name
    fn resolve_import_alias(
        &self,
        current_module_id: ModuleId,
        alias: &str,
    ) -> Option<(ModuleId, String)> {
        if let Some(items) = self.module_items.get(&current_module_id) {
            let current_module = self
                .module_names
                .get(&current_module_id)
                .map_or("", std::string::String::as_str);
            for item in items {
                if let ItemType::FromImport {
                    module,
                    names,
                    level,
                    ..
                } = &item.item_type
                {
                    // Check if this import defines the alias
                    for (original_name, alias_opt) in names {
                        let local_name = alias_opt.as_ref().unwrap_or(original_name);
                        if local_name == alias {
                            // Found the import that defines this alias
                            // Resolve relative imports to absolute module names
                            let resolved_module_name = if *level > 0 {
                                debug!(
                                    "Resolving relative import: module='{module}', level={level}, \
                                     current_module='{current_module}'"
                                );
                                let result =
                                    self.resolve_relative_module(current_module, module, *level);
                                debug!("Resolved to: '{result}'");
                                result
                            } else {
                                module.clone()
                            };

                            // Convert module name to ModuleId
                            if let Some(&resolved_id) =
                                self.module_name_to_id.get(&resolved_module_name)
                            {
                                return Some((resolved_id, original_name.clone()));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Resolve a module import alias (from regular imports like `import x.y as z`)
    fn resolve_module_import_alias(
        &self,
        current_module_id: ModuleId,
        alias: &str,
    ) -> Option<ModuleId> {
        if let Some(items) = self.module_items.get(&current_module_id) {
            for item in items {
                if let ItemType::Import {
                    module,
                    alias: Some(alias_name),
                } = &item.item_type
                {
                    // Check if this import has an alias that matches
                    if alias_name == alias {
                        // Found the import with matching alias
                        // Convert module name to ModuleId
                        if let Some(&module_id) = self.module_name_to_id.get(module) {
                            return Some(module_id);
                        }
                    }
                }
            }
        }
        None
    }

    /// Resolve a from import that imports a module (e.g., from utils import calculator)
    fn resolve_from_module_import(
        &self,
        current_module_id: ModuleId,
        alias: &str,
    ) -> Option<ModuleId> {
        if let Some(items) = self.module_items.get(&current_module_id) {
            let current_module = self
                .module_names
                .get(&current_module_id)
                .map_or("", std::string::String::as_str);
            for item in items {
                if let ItemType::FromImport {
                    module,
                    names,
                    level,
                    ..
                } = &item.item_type
                {
                    // Check if this import defines the alias
                    for (original_name, alias_opt) in names {
                        let local_name = alias_opt.as_ref().unwrap_or(original_name);
                        if local_name == alias {
                            // Resolve relative imports to absolute module names
                            let resolved_module = if *level > 0 {
                                self.resolve_relative_module(current_module, module, *level)
                            } else {
                                module.clone()
                            };

                            // Check if we're importing a submodule
                            let potential_full_module =
                                format!("{resolved_module}.{original_name}");
                            if let Some(&module_id) =
                                self.module_name_to_id.get(&potential_full_module)
                            {
                                // This is importing a module
                                return Some(module_id);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Resolve a relative module import to an absolute module name
    ///
    /// Note: We cannot use `resolver::resolve_relative_import_from_name` directly because:
    /// 1. The resolver expects module names ending with "__init__" for packages, but we have
    ///    module names like "parent.subpkg" that are packages
    /// 2. We need to determine if a module is a package based on whether it has submodules,
    ///    which requires access to `self.module_names`
    /// 3. The resolver's heuristics don't match our tree-shaking context where we're working
    ///    with already-resolved module names rather than file paths
    fn resolve_relative_module(
        &self,
        current_module: &str,
        relative_module: &str,
        level: u32,
    ) -> String {
        // Split current module into parts
        let parts: Vec<&str> = current_module.split('.').collect();

        // Check if current module is a package (has sub-modules)
        // A module is a package if it has sub-modules in our module registry
        let has_submodules = self
            .module_names
            .values()
            .any(|name| name != current_module && name.starts_with(&format!("{current_module}.")));

        // For relative imports with level > 1, the importing module must be in a package
        let is_package = has_submodules || (level > 1 && parts.len() > 1);

        debug!(
            "resolve_relative_module: current_module='{current_module}', relative_module='{relative_module}', \
             level={level}, is_package={is_package}"
        );

        // Calculate how many levels to actually remove
        // For packages, level 1 means current package, not parent
        // For regular modules, we remove 'level' parts
        let levels_to_remove = if is_package {
            if level > 0 { level - 1 } else { 0 }
        } else {
            level
        } as usize;

        // If we need to go up more levels than we have, something is wrong
        if levels_to_remove > parts.len() {
            warn!(
                "Relative import level {} exceeds module depth {} for module {}",
                level,
                parts.len(),
                current_module
            );
            return relative_module.to_string();
        }

        // Get the parent module parts
        let parent_parts = &parts[..parts.len().saturating_sub(levels_to_remove)];

        // Remove the dots from the relative module name
        let relative_part = relative_module.trim_start_matches('.');

        // Combine parent parts with relative module
        let result = if relative_part.is_empty() {
            // Import from parent package itself
            parent_parts.join(".")
        } else if parent_parts.is_empty() {
            // At top level
            relative_part.to_string()
        } else {
            // Normal case: parent.relative
            format!("{}.{}", parent_parts.join("."), relative_part)
        };

        debug!("Resolved relative import to: '{result}'");
        result
    }

    /// Seed side effects for a module that has been reached via imports
    fn seed_side_effects_for_module(
        &self,
        module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        if let Some(items) = self.module_items.get(&module_id) {
            let module_name = self.module_names.get(&module_id).map_or("", String::as_str);
            debug!("Seeding side effects for reachable module: {module_name}");
            for item in items {
                match item.item_type {
                    ItemType::Expression | ItemType::Assignment { .. } => {
                        self.add_vars_to_worklist(
                            &item.read_vars,
                            module_id,
                            worklist,
                            "reachable side-effect module",
                        );
                        self.add_attribute_accesses_to_worklist(
                            &item.attribute_accesses,
                            module_id,
                            worklist,
                        );
                    }
                    ItemType::FunctionDef { .. } | ItemType::ClassDef { .. } => {
                        for symbol in &item.defined_symbols {
                            worklist.push_back((module_id, symbol.clone()));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Mark all symbols transitively used from entry module
    pub fn mark_used_symbols(&mut self) {
        let mut worklist: VecDeque<(ModuleId, String)> = VecDeque::new();

        // First pass: find all direct module imports across all modules
        // Also detect dynamic access patterns that require keeping all __all__ symbols
        for (&module_id, items) in &self.module_items {
            let module_name = self
                .module_names
                .get(&module_id)
                .map_or("", std::string::String::as_str);
            // Check if this module uses dynamic access pattern (locals()/vars() with __all__)
            let uses_dynamic_access = self.module_uses_dynamic_all_access(items);

            if uses_dynamic_access {
                debug!(
                    "Module {module_name} uses dynamic __all__ access pattern (locals/globals with setattr loop)"
                );
                // Mark all symbols in __all__ as used for this module
                self.mark_all_symbols_from_module_all_as_used(module_id, &mut worklist);
            }

            for item in items {
                match &item.item_type {
                    // Check for direct module imports (import module_name)
                    ItemType::Import { module, .. } => {
                        let module_display = self.get_module_display_name(module_id);
                        debug!("Found direct import of module {module} in {module_display}");
                        // If this imported module has side effects, seed them
                        if let Some(&imported_module_id) = self.module_name_to_id.get(module)
                            && self.module_has_side_effects(imported_module_id)
                        {
                            self.seed_side_effects_for_module(imported_module_id, &mut worklist);
                        }
                    }
                    // Check for from imports that import the module itself (from x import module)
                    ItemType::FromImport {
                        module: from_module,
                        names,
                        level,
                        is_star,
                        ..
                    } => {
                        // First resolve relative imports
                        let resolved_from_module = if *level > 0 {
                            self.resolve_relative_module(module_name, from_module, *level)
                        } else {
                            from_module.clone()
                        };

                        // When importing from a module, if that module has side effects, seed them
                        // This handles cases like: from .utils.config import some_function
                        // where .utils.config has side effects that need to run
                        if let Some(&from_module_id) =
                            self.module_name_to_id.get(&resolved_from_module)
                            && self.module_has_side_effects(from_module_id)
                        {
                            self.seed_side_effects_for_module(from_module_id, &mut worklist);
                        }

                        // Handle star imports - from module import *
                        if *is_star {
                            // For star imports, we need to mark all symbols from __all__ (if
                            // defined) or all non-private symbols as
                            // potentially used
                            if let Some(&target_module_id) =
                                self.module_name_to_id.get(&resolved_from_module)
                                && let Some(target_items) = self.module_items.get(&target_module_id)
                            {
                                // Check if the module has __all__ defined
                                let has_explicit_all =
                                    target_items.iter().any(Self::is_all_assignment);

                                if has_explicit_all {
                                    // Mark only symbols in __all__ for star imports
                                    self.mark_all_defined_symbols_as_used(
                                        target_items,
                                        target_module_id,
                                        &mut worklist,
                                    );
                                } else {
                                    // No __all__ defined, mark all non-private symbols
                                    self.mark_non_private_symbols_as_used(
                                        target_items,
                                        target_module_id,
                                        &mut worklist,
                                    );
                                }
                            }
                        } else {
                            // Regular from imports
                            for (name, _alias) in names {
                                // Check if this is importing a submodule directly
                                let potential_module = format!("{resolved_from_module}.{name}");
                                // Check if this module exists
                                if let Some(&submodule_id) =
                                    self.module_name_to_id.get(&potential_module)
                                    && self.module_has_side_effects(submodule_id)
                                {
                                    let module_display = self.get_module_display_name(module_id);
                                    debug!(
                                        "Found from import of module {potential_module} in \
                                         {module_display}"
                                    );
                                    // If this submodule has side effects, seed them
                                    self.seed_side_effects_for_module(submodule_id, &mut worklist);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Start with all symbols referenced by the entry module
        if let Some(items) = self.module_items.get(&ModuleId::ENTRY) {
            for item in items {
                // Mark classes and functions defined in the entry module as used
                // This ensures that classes/functions defined in the entry module
                // (even inside try blocks) are kept along with their dependencies
                match &item.item_type {
                    ItemType::ClassDef { name } | ItemType::FunctionDef { name } => {
                        debug!("Marking entry module class/function '{name}' as used");
                        worklist.push_back((ModuleId::ENTRY, name.clone()));
                    }
                    _ => {}
                }

                // Add symbols from read_vars
                self.add_vars_to_worklist(
                    &item.read_vars,
                    ModuleId::ENTRY,
                    &mut worklist,
                    "entry module",
                );

                // Add symbols from eventual_read_vars
                self.add_vars_to_worklist(
                    &item.eventual_read_vars,
                    ModuleId::ENTRY,
                    &mut worklist,
                    "entry module (eventual)",
                );

                // Mark all side-effect items as used
                if item.has_side_effects {
                    for symbol in &item.defined_symbols {
                        worklist.push_back((ModuleId::ENTRY, symbol.clone()));
                    }
                }

                // Process attribute accesses - if we access `greetings.message`,
                // we need the `message` symbol from the `greetings` module
                self.add_attribute_accesses_to_worklist(
                    &item.attribute_accesses,
                    ModuleId::ENTRY,
                    &mut worklist,
                );
            }
        }

        // Process worklist using existing dependency info
        while let Some((module_id, symbol)) = worklist.pop_front() {
            let key = (module_id, symbol.clone());
            if self.used_symbols.contains(&key) {
                continue;
            }

            let module_display = self.get_module_display_name(module_id);
            trace!("Marking symbol as used: {module_display}::{symbol}");
            self.used_symbols.insert(key);

            // Process the item that defines this symbol
            self.process_symbol_definition(module_id, &symbol, &mut worklist);
        }
    }

    /// Process a symbol definition and add its dependencies to the worklist
    fn process_symbol_definition(
        &self,
        module_id: ModuleId,
        symbol: &str,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let Some(items) = self.module_items.get(&module_id) else {
            return;
        };

        let module_display = self.get_module_display_name(module_id);
        debug!("Processing symbol definition: {module_display}::{symbol}");

        // First check if this symbol is actually defined in this module
        // (not just imported/re-exported)
        let symbol_is_defined_here = items
            .iter()
            .any(|item| item.defined_symbols.contains(symbol));

        // Only check for re-exports if the symbol is not defined here
        if !symbol_is_defined_here {
            // Check if this symbol is imported from another module (re-export)
            for item in items {
                if let ItemType::FromImport {
                    module: from_module,
                    names,
                    level,
                    ..
                } = &item.item_type
                {
                    for (original_name, alias_opt) in names {
                        let local_name = alias_opt.as_ref().unwrap_or(original_name);
                        if local_name == symbol {
                            // This symbol is re-exported from another module
                            let module_name = self
                                .module_names
                                .get(&module_id)
                                .map_or("", std::string::String::as_str);
                            let resolved_module_name = if *level > 0 {
                                self.resolve_relative_module(module_name, from_module, *level)
                            } else {
                                from_module.clone()
                            };

                            if let Some(&resolved_module_id) =
                                self.module_name_to_id.get(&resolved_module_name)
                            {
                                debug!(
                                    "Symbol {symbol} is re-exported from \
                                 {resolved_module_name}::{original_name}"
                                );
                                worklist.push_back((resolved_module_id, original_name.clone()));
                                // Also mark the import itself as used
                                self.add_item_dependencies(item, module_id, worklist);
                                return;
                            }
                        }
                    }
                }
            }
        }

        for item in items {
            if !item.defined_symbols.contains(symbol) {
                continue;
            }

            // Add all symbols this item depends on
            self.add_item_dependencies(item, module_id, worklist);

            // If this is a function or class, also mark all imports within its scope as used
            if matches!(
                item.item_type,
                ItemType::FunctionDef { .. } | ItemType::ClassDef { .. }
            ) {
                debug!("Symbol {symbol} is a function/class, checking for scoped imports");
                self.mark_scoped_imports_as_used(module_id, symbol, worklist);
            }

            // Add symbol-specific dependencies if tracked
            if let Some(deps) = item.symbol_dependencies.get(symbol) {
                for dep in deps {
                    // First check if the dependency is defined in the current module
                    // (for local references like metaclass=MyMetaclass in the same module)
                    let dep_module = self.find_defining_module_preferring_local(module_id, dep);

                    if let Some(dep_module_id) = dep_module {
                        worklist.push_back((dep_module_id, dep.clone()));
                    }
                }
            }
        }
    }

    /// Mark all imports within a function or class scope as used
    fn mark_scoped_imports_as_used(
        &self,
        module_id: ModuleId,
        scope_name: &str,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let Some(items) = self.module_items.get(&module_id) else {
            return;
        };

        for item in items {
            // Check if this item is an import within the given scope
            if let Some(ref containing_scope) = item.containing_scope
                && containing_scope == scope_name
            {
                // This import is inside the function/class being marked as used
                match &item.item_type {
                    ItemType::Import {
                        module: imported_module,
                        ..
                    } => {
                        debug!(
                            "Marking import {imported_module} as used (inside scope {scope_name})"
                        );
                        // For direct imports, we need to mark the variables they declare as used
                        for var in &item.var_decls {
                            debug!("  Adding imported variable {var} to worklist");
                            worklist.push_back((module_id, var.clone()));
                        }
                        // If this imported module has side effects, seed them
                        if let Some(&imported_module_id) =
                            self.module_name_to_id.get(imported_module)
                            && self.module_has_side_effects(imported_module_id)
                        {
                            self.seed_side_effects_for_module(imported_module_id, worklist);
                        }
                    }
                    ItemType::FromImport {
                        module: from_module,
                        names,
                        level,
                        is_star,
                        ..
                    } => {
                        // Resolve relative imports
                        let module_name = self
                            .module_names
                            .get(&module_id)
                            .map_or("", std::string::String::as_str);
                        let resolved_module_name = if *level > 0 {
                            self.resolve_relative_module(module_name, from_module, *level)
                        } else {
                            from_module.clone()
                        };

                        // Note: Side effects for the source module are already seeded
                        // in the first pass when we encounter the FromImport

                        if *is_star {
                            // Handle star imports
                            if let Some(&resolved_module_id) =
                                self.module_name_to_id.get(&resolved_module_name)
                                && let Some(target_items) =
                                    self.module_items.get(&resolved_module_id)
                            {
                                let has_explicit_all =
                                    target_items.iter().any(Self::is_all_assignment);
                                if has_explicit_all {
                                    self.mark_all_defined_symbols_as_used(
                                        target_items,
                                        resolved_module_id,
                                        worklist,
                                    );
                                } else {
                                    self.mark_non_private_symbols_as_used(
                                        target_items,
                                        resolved_module_id,
                                        worklist,
                                    );
                                }
                            }
                        } else {
                            // Mark upstream symbols
                            if let Some(&resolved_module_id) =
                                self.module_name_to_id.get(&resolved_module_name)
                            {
                                for (name, _alias) in names {
                                    debug!(
                                        "Marking {resolved_module_name}::{name} as used (imported in scope {scope_name})"
                                    );
                                    worklist.push_back((resolved_module_id, name.clone()));

                                    // Check if this is importing a submodule
                                    let potential_module = format!("{resolved_module_name}.{name}");
                                    if let Some(&submodule_id) =
                                        self.module_name_to_id.get(&potential_module)
                                        && self.module_has_side_effects(submodule_id)
                                    {
                                        self.seed_side_effects_for_module(submodule_id, worklist);
                                    }
                                }
                            }
                        }
                        // Always mark the local bindings declared by this import as used,
                        // so the in-scope import statement is preserved.
                        for var in &item.var_decls {
                            debug!("  Adding local imported binding {var} to worklist");
                            worklist.push_back((module_id, var.clone()));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Add dependencies of an item to the worklist
    fn add_item_dependencies(
        &self,
        item: &ItemData,
        current_module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        // Add all variables read by this item
        for var in &item.read_vars {
            // Check if this var is an imported alias first
            if let Some((source_module_id, original_name)) =
                self.resolve_import_alias(current_module_id, var)
            {
                worklist.push_back((source_module_id, original_name));
            } else if let Some(module_id) = self.find_defining_module(var) {
                worklist.push_back((module_id, var.clone()));
            }
        }

        // Add eventual reads (from function bodies)
        for var in &item.eventual_read_vars {
            // Check if this var is an imported alias first
            if let Some((source_module_id, original_name)) =
                self.resolve_import_alias(current_module_id, var)
            {
                worklist.push_back((source_module_id, original_name));
            } else {
                // For reads without global statement, prioritize current module
                let defining_module_id =
                    self.find_defining_module_preferring_local(current_module_id, var);

                if let Some(module_id) = defining_module_id {
                    let module_display = self.get_module_display_name(module_id);
                    debug!(
                        "Adding eventual read dependency: {} reads {} (defined in {})",
                        item.item_type.name().unwrap_or("<unknown>"),
                        var,
                        module_display
                    );
                    worklist.push_back((module_id, var.clone()));
                }
            }
        }

        // Add all variables written by this item (for global statements)
        for var in &item.write_vars {
            // For global statements, first check if the variable is defined in the current module
            let defining_module_id =
                self.find_defining_module_preferring_local(current_module_id, var);

            if let Some(module_id) = defining_module_id {
                let module_display = self.get_module_display_name(module_id);
                debug!(
                    "Adding write dependency: {} writes to {} (defined in {})",
                    item.item_type.name().unwrap_or("<unknown>"),
                    var,
                    module_display
                );
                worklist.push_back((module_id, var.clone()));
            } else {
                warn!(
                    "{} writes to {} but cannot find defining module",
                    item.item_type.name().unwrap_or("<unknown>"),
                    var
                );
            }
        }

        // Add eventual writes (from function bodies with global statements)
        for var in &item.eventual_write_vars {
            // For global statements, first check if the variable is defined in the current module
            let defining_module_id =
                self.find_defining_module_preferring_local(current_module_id, var);

            if let Some(module_id) = defining_module_id {
                let module_display = self.get_module_display_name(module_id);
                debug!(
                    "Adding eventual write dependency: {} eventually writes to {} (defined in {})",
                    item.item_type.name().unwrap_or("<unknown>"),
                    var,
                    module_display
                );
                worklist.push_back((module_id, var.clone()));
            }
        }

        // For classes, we need to include base classes
        if let ItemType::ClassDef { .. } = &item.item_type {
            // Base classes are in read_vars
            for base_class in &item.read_vars {
                if let Some(module_id) = self.find_defining_module(base_class) {
                    worklist.push_back((module_id, base_class.clone()));
                }
            }
        }

        // Process attribute accesses
        self.add_attribute_accesses_to_worklist(
            &item.attribute_accesses,
            current_module_id,
            worklist,
        );
    }

    /// Get symbols that survive tree-shaking for a module
    pub fn get_used_symbols_for_module(
        &self,
        module_name: &str,
    ) -> crate::types::FxIndexSet<String> {
        // Get the ModuleId for this module name
        if let Some(&module_id) = self.module_name_to_id.get(module_name) {
            self.used_symbols
                .iter()
                .filter(|(id, _)| *id == module_id)
                .map(|(_, symbol)| symbol.clone())
                .collect()
        } else {
            FxIndexSet::default()
        }
    }

    /// Check if a symbol is used after tree-shaking
    pub fn is_symbol_used(&self, module_name: &str, symbol_name: &str) -> bool {
        // Get the ModuleId for this module name
        if let Some(&module_id) = self.module_name_to_id.get(module_name) {
            self.used_symbols
                .contains(&(module_id, symbol_name.to_string()))
        } else {
            false
        }
    }

    /// Get all unused symbols for a module
    pub fn get_unused_symbols_for_module(&self, module_name: &str) -> Vec<String> {
        let mut unused = Vec::new();

        if let Some(&module_id) = self.module_name_to_id.get(module_name)
            && let Some(items) = self.module_items.get(&module_id)
        {
            for item in items {
                for symbol in &item.defined_symbols {
                    if !self.is_symbol_used(module_name, symbol) {
                        unused.push(symbol.clone());
                    }
                }
            }
        }

        unused
    }

    /// Check if a module has side effects that prevent tree-shaking
    pub fn module_has_side_effects(&self, module_id: ModuleId) -> bool {
        if let Some(items) = self.module_items.get(&module_id) {
            // Check if any top-level item has side effects
            items.iter().any(|item| {
                item.has_side_effects
                    && !matches!(
                        item.item_type,
                        ItemType::Import { .. } | ItemType::FromImport { .. }
                    )
            })
        } else {
            false
        }
    }

    /// Check if a module has side effects that prevent tree-shaking (by name)
    /// This is a convenience method for external callers that have module names
    pub fn module_has_side_effects_by_name(&self, module_name: &str) -> bool {
        if let Some(&module_id) = self.module_name_to_id.get(module_name) {
            self.module_has_side_effects(module_id)
        } else {
            false
        }
    }

    /// Helper method to add variables to the worklist, resolving imports and finding definitions
    fn add_vars_to_worklist(
        &self,
        vars: &FxIndexSet<String>,
        module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
        context: &str,
    ) {
        for var in vars {
            if let Some((source_module_id, original_name)) =
                self.resolve_import_alias(module_id, var)
            {
                let source_display = self.get_module_display_name(source_module_id);
                debug!(
                    "Found import dependency in {context}: {var} -> \
                     {source_display}::{original_name}"
                );
                worklist.push_back((source_module_id, original_name));
            } else if let Some(found_module_id) = self.find_defining_module(var) {
                let module_display = self.get_module_display_name(found_module_id);
                debug!("Found symbol dependency in {context}: {var} in module {module_display}");
                worklist.push_back((found_module_id, var.clone()));
            }
        }
    }

    /// Helper method to process attribute accesses and add them to the worklist
    fn add_attribute_accesses_to_worklist(
        &self,
        attribute_accesses: &FxIndexMap<String, FxIndexSet<String>>,
        module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let module_name = self
            .module_names
            .get(&module_id)
            .map_or("", std::string::String::as_str);
        for (base_var, accessed_attrs) in attribute_accesses {
            // 1) Module alias via `import x.y as z`
            if let Some(source_module_id) = self.resolve_module_import_alias(module_id, base_var) {
                for attr in accessed_attrs {
                    let source_display = self.get_module_display_name(source_module_id);
                    debug!(
                        "Found attribute access on module alias in {module_name}: \
                         {base_var}.{attr} -> marking {source_display}::{attr} as used"
                    );
                    worklist.push_back((source_module_id, attr.clone()));
                }
            // 2) From-imported module via `from utils import calculator`
            } else if let Some(source_module_id) =
                self.resolve_from_module_import(module_id, base_var)
            {
                for attr in accessed_attrs {
                    let source_display = self.get_module_display_name(source_module_id);
                    debug!(
                        "Found attribute access on from-imported module in {module_name}: \
                         {base_var}.{attr} -> marking {source_display}::{attr} as used"
                    );
                    worklist.push_back((source_module_id, attr.clone()));
                }
            // 3) Imported symbol with attribute access
            } else if let Some((source_module_id, _)) =
                self.resolve_import_alias(module_id, base_var)
            {
                for attr in accessed_attrs {
                    let source_display = self.get_module_display_name(source_module_id);
                    debug!(
                        "Found attribute access in {module_name}: {base_var}.{attr} -> marking \
                         {source_display}::{attr} as used"
                    );
                    worklist.push_back((source_module_id, attr.clone()));
                }
            // 4) Direct module reference like `import greetings`
            } else if let Some(&base_module_id) = self.module_name_to_id.get(base_var) {
                for attr in accessed_attrs {
                    debug!(
                        "Found direct module attribute access in {module_name}: {base_var}.{attr}"
                    );
                    worklist.push_back((base_module_id, attr.clone()));
                }
            // 5) Namespace package lookup
            } else {
                self.find_attribute_in_namespace(base_var, accessed_attrs, worklist, module_name);
            }
        }
    }

    /// Find attribute in namespace package submodules
    fn find_attribute_in_namespace(
        &self,
        base_var: &str,
        accessed_attrs: &FxIndexSet<String>,
        worklist: &mut VecDeque<(ModuleId, String)>,
        context: &str,
    ) {
        let is_namespace = self
            .module_names
            .values()
            .any(|name| name.starts_with(&format!("{base_var}.")));

        if !is_namespace {
            debug!("Unknown base variable for attribute access in {context}: {base_var}");
            return;
        }

        debug!("Found namespace package access in {context}: {base_var}");
        for attr in accessed_attrs {
            debug!("Looking for {attr} in submodules of {base_var}");

            // Find which submodule defines this attribute
            if let Some(module_name) = self.find_attribute_in_submodules(base_var, attr) {
                debug!("Found {attr} defined in {module_name}");
                worklist.push_back((module_name, attr.clone()));
            } else {
                warn!("Could not find {attr} in any submodule of {base_var} from {context}");
            }
        }
    }

    /// Find which submodule defines an attribute
    fn find_attribute_in_submodules(&self, base_var: &str, attr: &str) -> Option<ModuleId> {
        for (&module_id, module_name) in &self.module_names {
            if module_name.starts_with(&format!("{base_var}."))
                && let Some(items) = self.module_items.get(&module_id)
            {
                for item in items {
                    if item.defined_symbols.contains(attr) {
                        return Some(module_id);
                    }
                }
            }
        }
        None
    }

    /// Mark symbols defined in __all__ as used for star imports
    fn mark_all_defined_symbols_as_used(
        &self,
        target_items: &[ItemData],
        resolved_from_module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let resolved_from_module = self
            .module_names
            .get(&resolved_from_module_id)
            .map_or("", std::string::String::as_str);
        for item in target_items {
            if Self::is_all_assignment(item) {
                // Mark all symbols listed in __all__
                for symbol in &item.eventual_read_vars {
                    if !symbol.starts_with('_') {
                        debug!(
                            "Marking {symbol} from star import of {resolved_from_module} \
                             as used"
                        );
                        worklist.push_back((resolved_from_module_id, symbol.clone()));
                    }
                }
            }
        }
    }

    /// Mark all non-private symbols as used when no __all__ is defined
    fn mark_non_private_symbols_as_used(
        &self,
        target_items: &[ItemData],
        resolved_from_module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let resolved_from_module = self
            .module_names
            .get(&resolved_from_module_id)
            .map_or("", std::string::String::as_str);
        for item in target_items {
            for symbol in &item.defined_symbols {
                if !symbol.starts_with('_') {
                    debug!("Marking {symbol} from star import of {resolved_from_module} as used");
                    worklist.push_back((resolved_from_module_id, symbol.clone()));
                }
            }
        }
    }

    /// Helper method to check if an item is an __all__ assignment
    fn is_all_assignment(item: &ItemData) -> bool {
        matches!(&item.item_type, ItemType::Assignment { targets, .. } if targets.contains(&"__all__".to_string()))
    }

    /// Check if a module uses the dynamic __all__ access pattern
    /// This pattern involves using `locals()` or `globals()` with a loop over __all__ and setattr
    fn module_uses_dynamic_all_access(&self, items: &[ItemData]) -> bool {
        // Check if the module has __all__ defined
        let has_all = items.iter().any(Self::is_all_assignment);

        if !has_all {
            return false;
        }

        // Check if the module uses setattr, (locals() or globals()), and reads __all__ in a single pass
        // Note: We don't check for vars() because that's our transformation that happens after tree-shaking
        let mut uses_setattr = false;
        let mut uses_locals_or_globals = false;
        let mut reads_all = false;

        for item in items {
            if !uses_setattr {
                uses_setattr = item.read_vars.contains("setattr")
                    || item.eventual_read_vars.contains("setattr");
            }
            if !uses_locals_or_globals {
                uses_locals_or_globals = item.read_vars.contains("locals")
                    || item.eventual_read_vars.contains("locals")
                    || item.read_vars.contains("globals")
                    || item.eventual_read_vars.contains("globals");
            }
            if !reads_all {
                // Check if __all__ is actually accessed (not just defined)
                reads_all = item.read_vars.contains("__all__")
                    || item.eventual_read_vars.contains("__all__");
            }
            // Early return if all conditions are met
            if uses_setattr && uses_locals_or_globals && reads_all {
                return true;
            }
        }

        // All conditions must be met for this to be the dynamic __all__ access pattern
        uses_setattr && uses_locals_or_globals && reads_all
    }

    /// Mark all symbols from a module's __all__ as used
    fn mark_all_symbols_from_module_all_as_used(
        &self,
        module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let module_name = self
            .module_names
            .get(&module_id)
            .map_or("", std::string::String::as_str);
        if let Some(items) = self.module_items.get(&module_id) {
            for item in items {
                if Self::is_all_assignment(item) {
                    // Mark all symbols listed in __all__ (stored in eventual_read_vars)
                    for symbol in &item.eventual_read_vars {
                        debug!(
                            "Marking {symbol} from module {module_name} as used due to dynamic __all__ access"
                        );
                        // Resolve the symbol's source module or use the current module
                        if let Some((source_module_id, original_name)) =
                            self.resolve_import_alias(module_id, symbol)
                        {
                            worklist.push_back((source_module_id, original_name));
                        } else {
                            // Symbol is defined in the current module
                            worklist.push_back((module_id, symbol.clone()));
                        }
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_basic_tree_shaking() {
        let mut graph = CriboGraph::new();

        // Create a simple module with used and unused functions
        let module_id = graph.add_module(
            crate::resolver::ModuleId::new(1),
            "test_module".to_string(),
            &std::path::PathBuf::from("test.py"),
        );
        let module = graph
            .modules
            .get_mut(&module_id)
            .expect("module should exist");

        // Add a used function
        module.add_item(ItemData {
            item_type: ItemType::FunctionDef {
                name: "used_func".to_string(),
            },
            defined_symbols: ["used_func".into()].into_iter().collect(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            var_decls: ["used_func".into()].into_iter().collect(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: false,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: None,
        });

        // Add an unused function
        module.add_item(ItemData {
            item_type: ItemType::FunctionDef {
                name: "unused_func".to_string(),
            },
            defined_symbols: ["unused_func".into()].into_iter().collect(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            var_decls: ["unused_func".into()].into_iter().collect(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: false,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: None,
        });

        // Add entry module that uses only used_func
        let entry_id = graph.add_module(
            crate::resolver::ModuleId::new(0),
            "__main__".to_string(),
            &std::path::PathBuf::from("main.py"),
        );
        let entry = graph
            .modules
            .get_mut(&entry_id)
            .expect("entry module should exist");

        entry.add_item(ItemData {
            item_type: ItemType::Expression,
            defined_symbols: FxIndexSet::default(),
            read_vars: ["used_func".into()].into_iter().collect(),
            eventual_read_vars: FxIndexSet::default(),
            var_decls: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: true,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: None,
        });

        // Run tree shaking
        let mut shaker = TreeShaker::from_graph(&graph);
        shaker.analyze("__main__");

        // Check results
        assert!(shaker.is_symbol_used("test_module", "used_func"));
        assert!(!shaker.is_symbol_used("test_module", "unused_func"));

        let unused = shaker.get_unused_symbols_for_module("test_module");
        assert_eq!(unused, vec!["unused_func"]);
    }
}
