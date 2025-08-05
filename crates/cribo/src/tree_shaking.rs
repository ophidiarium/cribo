#![allow(clippy::excessive_nesting)]

use std::collections::VecDeque;

use anyhow::Result;
use indexmap::{IndexMap, IndexSet};
use log::{debug, trace};
use rustc_hash::FxHashSet;

use crate::cribo_graph::{CriboGraph, ItemData, ItemType, ModuleId};

/// Tree shaker that removes unused symbols from modules
#[derive(Debug)]
pub struct TreeShaker {
    /// Module items from semantic analysis (reused from `CriboGraph`)
    module_items: IndexMap<String, Vec<ItemData>>,
    /// Track which symbols are used across module boundaries
    cross_module_refs: IndexMap<(String, String), IndexSet<String>>,
    /// Final set of symbols to keep (`module_name`, `symbol_name`)
    used_symbols: IndexSet<(String, String)>,
    /// Map from module ID to module name
    _module_names: IndexMap<ModuleId, String>,
}

impl TreeShaker {
    /// Create a tree shaker from an existing `CriboGraph`
    pub fn from_graph(graph: &CriboGraph) -> Self {
        let mut module_items = IndexMap::new();
        let mut module_names = IndexMap::new();

        // Extract module items from the graph
        for (module_id, module_dep_graph) in &graph.modules {
            let module_name = module_dep_graph.module_name.clone();
            module_names.insert(*module_id, module_name.clone());

            // Collect all items for this module
            let items: Vec<ItemData> = module_dep_graph.items.values().cloned().collect();

            module_items.insert(module_name, items);
        }

        Self {
            module_items,
            cross_module_refs: IndexMap::new(),
            used_symbols: IndexSet::new(),
            _module_names: module_names,
        }
    }

    /// Analyze which symbols should be kept based on entry point
    pub fn analyze(&mut self, entry_module: &str) -> Result<()> {
        debug!("Starting tree-shaking analysis from entry module: {entry_module}");

        // First, build cross-module reference information
        self.build_cross_module_refs();

        // Then, mark symbols used from the entry module
        self.mark_used_symbols(entry_module)?;

        debug!(
            "Tree-shaking complete. Keeping {} symbols",
            self.used_symbols.len()
        );
        Ok(())
    }

    /// Build cross-module reference information
    fn build_cross_module_refs(&mut self) {
        trace!("Building cross-module reference information");

        for (module_name, items) in &self.module_items {
            for item in items {
                // Track which external symbols this item references
                for read_var in &item.read_vars {
                    // Check if this is a reference to another module's symbol
                    if self.is_external_symbol(module_name, read_var) {
                        // Find which module defines this symbol
                        if let Some(defining_module) = self.find_defining_module(read_var) {
                            self.cross_module_refs
                                .entry((defining_module.clone(), read_var.clone()))
                                .or_default()
                                .insert(module_name.clone());
                        }
                    }
                }

                // Also check eventual_read_vars for function-level imports
                for read_var in &item.eventual_read_vars {
                    if self.is_external_symbol(module_name, read_var)
                        && let Some(defining_module) = self.find_defining_module(read_var)
                    {
                        self.cross_module_refs
                            .entry((defining_module.clone(), read_var.clone()))
                            .or_default()
                            .insert(module_name.clone());
                    }
                }
            }
        }
    }

    /// Check if a symbol is external to the current module
    fn is_external_symbol(&self, module_name: &str, symbol: &str) -> bool {
        !self.is_defined_in_module(module_name, symbol)
    }

    /// Check if a symbol is defined in a specific module
    fn is_defined_in_module(&self, module_name: &str, symbol: &str) -> bool {
        if let Some(items) = self.module_items.get(module_name) {
            for item in items {
                if item.defined_symbols.contains(symbol) {
                    return true;
                }
            }
        }
        false
    }

    /// Find which module defines a symbol
    fn find_defining_module(&self, symbol: &str) -> Option<String> {
        for (module_name, items) in &self.module_items {
            for item in items {
                if item.defined_symbols.contains(symbol) {
                    return Some(module_name.clone());
                }
            }
        }
        None
    }

    /// Resolve an import alias to its original module and name
    fn resolve_import_alias(&self, current_module: &str, alias: &str) -> Option<(String, String)> {
        if let Some(items) = self.module_items.get(current_module) {
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
                            let resolved_module = if *level > 0 {
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

                            return Some((resolved_module, original_name.clone()));
                        }
                    }
                }
            }
        }
        None
    }

    /// Resolve a module import alias (from regular imports like `import x.y as z`)
    fn resolve_module_import_alias(&self, current_module: &str, alias: &str) -> Option<String> {
        if let Some(items) = self.module_items.get(current_module) {
            for item in items {
                if let ItemType::Import {
                    module,
                    alias: Some(alias_name),
                } = &item.item_type
                {
                    // Check if this import has an alias that matches
                    if alias_name == alias {
                        // Found the import with matching alias
                        return Some(module.clone());
                    }
                }
            }
        }
        None
    }

    /// Resolve a from import that imports a module (e.g., from utils import calculator)
    fn resolve_from_module_import(&self, current_module: &str, alias: &str) -> Option<String> {
        if let Some(items) = self.module_items.get(current_module) {
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
                            if self.module_items.contains_key(&potential_full_module) {
                                // This is importing a module
                                return Some(potential_full_module);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Resolve a relative module import to an absolute module name
    fn resolve_relative_module(
        &self,
        current_module: &str,
        relative_module: &str,
        level: u32,
    ) -> String {
        // Split current module into parts
        let parts: Vec<&str> = current_module.split('.').collect();

        // Check if current module is a package (has sub-modules or is known to be a package)
        // Note: A module is a package if:
        // 1. It has sub-modules in module_items, OR
        // 2. The module name suggests it's a package (e.g., "greetings.greeting" where both parts
        //    could be packages)
        // For relative imports with level > 1, the importing module must be in a package
        let has_submodules = self
            .module_items
            .keys()
            .any(|key| key != current_module && key.starts_with(&format!("{current_module}.")));

        // If we're doing a level 2+ import, the current module must be a package
        // because you can only go up multiple levels from within a package structure
        let is_package = has_submodules || (level > 1 && parts.len() > 1);

        debug!(
            "resolve_relative_module: current_module='{current_module}', \
             relative_module='{relative_module}', level={level}, parts={parts:?}, \
             is_package={is_package}"
        );

        // Calculate how many levels to actually remove
        let levels_to_remove = if is_package {
            // For packages, level 1 means current package, not parent
            if level > 0 { level - 1 } else { 0 }
        } else {
            // For regular modules, remove 'level' parts
            level
        } as usize;

        // If we need to go up more levels than we have, something is wrong
        if levels_to_remove > parts.len() {
            debug!(
                "Warning: relative import level {} exceeds module depth {} for module {}",
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

        debug!(
            "levels_to_remove={levels_to_remove}, parent_parts={parent_parts:?}, \
             relative_part='{relative_part}'"
        );

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

        debug!("resolve_relative_module result: '{result}'");
        result
    }

    /// Mark all symbols transitively used from entry module
    pub fn mark_used_symbols(&mut self, entry_module: &str) -> Result<()> {
        let mut worklist = VecDeque::new();
        let mut directly_imported_modules = IndexSet::new();

        // First pass: find all direct module imports across all modules
        for (module_name, items) in &self.module_items {
            for item in items {
                match &item.item_type {
                    // Check for direct module imports (import module_name)
                    ItemType::Import { module, .. } => {
                        directly_imported_modules.insert(module.clone());
                        debug!("Found direct import of module {module} in {module_name}");
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

                        // Handle star imports - from module import *
                        if *is_star {
                            // For star imports, we need to mark all symbols from __all__ (if
                            // defined) or all non-private symbols as
                            // potentially used
                            if let Some(target_items) = self.module_items.get(&resolved_from_module)
                            {
                                // Check if the module has __all__ defined
                                let has_all = target_items
                                    .iter()
                                    .any(|item| item.defined_symbols.contains("__all__"));

                                if has_all {
                                    // Mark only symbols in __all__ for star imports
                                    self.mark_all_defined_symbols_as_used(
                                        target_items,
                                        &resolved_from_module,
                                        &mut worklist,
                                    );
                                } else {
                                    // No __all__ defined, mark all non-private symbols
                                    self.mark_non_private_symbols_as_used(
                                        target_items,
                                        &resolved_from_module,
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
                                if self.module_items.contains_key(&potential_module) {
                                    directly_imported_modules.insert(potential_module.clone());
                                    debug!(
                                        "Found from import of module {potential_module} in \
                                         {module_name}"
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Start with all symbols referenced by the entry module
        if let Some(items) = self.module_items.get(entry_module) {
            for item in items {
                // Mark classes and functions defined in the entry module as used
                // This ensures that classes/functions defined in the entry module
                // (even inside try blocks) are kept along with their dependencies
                match &item.item_type {
                    ItemType::ClassDef { name } | ItemType::FunctionDef { name } => {
                        debug!("Marking entry module class/function '{name}' as used");
                        worklist.push_back((entry_module.to_string(), name.clone()));
                    }
                    _ => {}
                }

                // Add symbols from read_vars
                for var in &item.read_vars {
                    // Check if this var is an imported alias first
                    if let Some((source_module, original_name)) =
                        self.resolve_import_alias(entry_module, var)
                    {
                        debug!("Found import alias: {var} -> {source_module}::{original_name}");
                        worklist.push_back((source_module, original_name));
                    } else if let Some(module) = self.find_defining_module(var) {
                        debug!("Found direct symbol usage: {var} in module {module}");
                        worklist.push_back((module, var.clone()));
                    } else {
                        debug!("Symbol {var} not found in any module");
                    }
                }

                // Add symbols from eventual_read_vars
                for var in &item.eventual_read_vars {
                    // Check if this var is an imported alias first
                    if let Some((source_module, original_name)) =
                        self.resolve_import_alias(entry_module, var)
                    {
                        worklist.push_back((source_module, original_name));
                    } else if let Some(module) = self.find_defining_module(var) {
                        worklist.push_back((module, var.clone()));
                    }
                }

                // Mark all side-effect items as used
                if item.has_side_effects {
                    for symbol in &item.defined_symbols {
                        worklist.push_back((entry_module.to_string(), symbol.clone()));
                    }
                }

                // Process attribute accesses - if we access `greetings.message`,
                // we need the `message` symbol from the `greetings` module
                for (base_var, accessed_attrs) in &item.attribute_accesses {
                    debug!(
                        "Processing attribute access: base_var={base_var}, \
                         attrs={accessed_attrs:?}"
                    );

                    // First, check if base_var is an imported module alias (e.g., import x.y as z)
                    if let Some(source_module) =
                        self.resolve_module_import_alias(entry_module, base_var)
                    {
                        // This is a module import alias with attribute access
                        for attr in accessed_attrs {
                            debug!(
                                "Found attribute access on module alias: {base_var}.{attr} -> \
                                 marking {source_module}::{attr} as used"
                            );
                            worklist.push_back((source_module.clone(), attr.clone()));
                        }
                    } else if let Some(source_module) =
                        self.resolve_from_module_import(entry_module, base_var)
                    {
                        // This is a from import of a module (e.g., from utils import calculator)
                        for attr in accessed_attrs {
                            debug!(
                                "Found attribute access on from-imported module: \
                                 {base_var}.{attr} -> marking {source_module}::{attr} as used"
                            );
                            worklist.push_back((source_module.clone(), attr.clone()));
                        }
                    } else if let Some((source_module, _)) =
                        self.resolve_import_alias(entry_module, base_var)
                    {
                        // This is an imported symbol with attribute access
                        for attr in accessed_attrs {
                            debug!(
                                "Found attribute access: {base_var}.{attr} -> marking \
                                 {source_module}::{attr} as used"
                            );
                            worklist.push_back((source_module.clone(), attr.clone()));
                        }
                    } else if self.module_items.contains_key(base_var) {
                        // Direct module import like `import greetings`
                        for attr in accessed_attrs {
                            debug!("Found direct module attribute access: {base_var}.{attr}");
                            worklist.push_back((base_var.clone(), attr.clone()));
                        }
                    } else {
                        self.find_attribute_in_namespace(
                            base_var,
                            accessed_attrs,
                            &mut worklist,
                            entry_module,
                        );
                    }
                }
            }
        }

        // Process all modules with side effects - their module-level code will run
        for (module_name, items) in &self.module_items {
            if self.module_has_side_effects(module_name) {
                debug!("Processing side-effect module: {module_name}");
                for item in items {
                    // Add all dependencies from module-level code
                    if matches!(
                        item.item_type,
                        ItemType::Expression | ItemType::Assignment { .. }
                    ) {
                        debug!(
                            "Processing module-level item in {}: read_vars={:?}",
                            module_name, item.read_vars
                        );
                        for var in &item.read_vars {
                            // Check if this var is an imported alias first
                            if let Some((source_module, original_name)) =
                                self.resolve_import_alias(module_name, var)
                            {
                                debug!(
                                    "Found import alias in side-effect module: {var} -> \
                                     {source_module}::{original_name}"
                                );
                                worklist.push_back((source_module, original_name));
                            } else if let Some(module) = self.find_defining_module(var) {
                                debug!(
                                    "Found direct symbol usage in side-effect module: {var} in \
                                     module {module}"
                                );
                                worklist.push_back((module, var.clone()));
                            }
                        }
                    }
                }
            }
        }

        // Process worklist using existing dependency info
        while let Some((module, symbol)) = worklist.pop_front() {
            let key = (module.clone(), symbol.clone());
            if self.used_symbols.contains(&key) {
                continue;
            }

            trace!("Marking symbol as used: {module}::{symbol}");
            self.used_symbols.insert(key);

            // Process the item that defines this symbol
            self.process_symbol_definition(&module, &symbol, &mut worklist);

            // Check if other modules reference this symbol
            if let Some(referencing_modules) = self
                .cross_module_refs
                .get(&(module.clone(), symbol.clone()))
            {
                trace!(
                    "Symbol {}::{} is referenced by {} modules",
                    module,
                    symbol,
                    referencing_modules.len()
                );
            }
        }

        Ok(())
    }

    /// Process a symbol definition and add its dependencies to the worklist
    fn process_symbol_definition(
        &self,
        module: &str,
        symbol: &str,
        worklist: &mut VecDeque<(String, String)>,
    ) {
        let Some(items) = self.module_items.get(module) else {
            return;
        };

        debug!("Processing symbol definition: {module}::{symbol}");

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
                        let resolved_module = if *level > 0 {
                            self.resolve_relative_module(module, from_module, *level)
                        } else {
                            from_module.clone()
                        };
                        debug!(
                            "Symbol {symbol} is re-exported from \
                             {resolved_module}::{original_name}"
                        );
                        worklist.push_back((resolved_module, original_name.clone()));
                        // Also mark the import itself as used
                        self.add_item_dependencies(item, module, worklist);
                        return;
                    }
                }
            }
        }

        for item in items {
            if !item.defined_symbols.contains(symbol) {
                continue;
            }

            // Add all symbols this item depends on
            self.add_item_dependencies(item, module, worklist);

            // Add symbol-specific dependencies if tracked
            if let Some(deps) = item.symbol_dependencies.get(symbol) {
                for dep in deps {
                    if let Some(dep_module) = self.find_defining_module(dep) {
                        worklist.push_back((dep_module, dep.clone()));
                    }
                }
            }
        }
    }

    /// Add dependencies of an item to the worklist
    fn add_item_dependencies(
        &self,
        item: &ItemData,
        current_module: &str,
        worklist: &mut VecDeque<(String, String)>,
    ) {
        // Add all variables read by this item
        for var in &item.read_vars {
            // Check if this var is an imported alias first
            if let Some((source_module, original_name)) =
                self.resolve_import_alias(current_module, var)
            {
                worklist.push_back((source_module, original_name));
            } else if let Some(module) = self.find_defining_module(var) {
                worklist.push_back((module, var.clone()));
            }
        }

        // Add eventual reads (from function bodies)
        for var in &item.eventual_read_vars {
            // Check if this var is an imported alias first
            if let Some((source_module, original_name)) =
                self.resolve_import_alias(current_module, var)
            {
                worklist.push_back((source_module, original_name));
            } else {
                // For reads without global statement, prioritize current module
                let defining_module = if self.is_defined_in_module(current_module, var) {
                    Some(current_module.to_string())
                } else {
                    self.find_defining_module(var)
                };

                if let Some(module) = defining_module {
                    debug!(
                        "Adding eventual read dependency: {} reads {} (defined in {})",
                        item.item_type.name().unwrap_or("<unknown>"),
                        var,
                        module
                    );
                    worklist.push_back((module, var.clone()));
                }
            }
        }

        // Add all variables written by this item (for global statements)
        for var in &item.write_vars {
            // For global statements, first check if the variable is defined in the current module
            let defining_module = if self.is_defined_in_module(current_module, var) {
                Some(current_module.to_string())
            } else {
                self.find_defining_module(var)
            };

            if let Some(module) = defining_module {
                debug!(
                    "Adding write dependency: {} writes to {} (defined in {})",
                    item.item_type.name().unwrap_or("<unknown>"),
                    var,
                    module
                );
                worklist.push_back((module, var.clone()));
            } else {
                debug!(
                    "Warning: {} writes to {} but cannot find defining module",
                    item.item_type.name().unwrap_or("<unknown>"),
                    var
                );
            }
        }

        // Add eventual writes (from function bodies with global statements)
        for var in &item.eventual_write_vars {
            // For global statements, first check if the variable is defined in the current module
            let defining_module = if self.is_defined_in_module(current_module, var) {
                Some(current_module.to_string())
            } else {
                self.find_defining_module(var)
            };

            if let Some(module) = defining_module {
                debug!(
                    "Adding eventual write dependency: {} eventually writes to {} (defined in {})",
                    item.item_type.name().unwrap_or("<unknown>"),
                    var,
                    module
                );
                worklist.push_back((module, var.clone()));
            }
        }

        // For classes, we need to include base classes
        if let ItemType::ClassDef { .. } = &item.item_type {
            // Base classes are in read_vars
            for base_class in &item.read_vars {
                if let Some(module) = self.find_defining_module(base_class) {
                    worklist.push_back((module, base_class.clone()));
                }
            }
        }

        // Process attribute accesses
        for (base_var, accessed_attrs) in &item.attribute_accesses {
            // First, check if base_var is an imported module alias (e.g., import x.y as z)
            if let Some(source_module) = self.resolve_module_import_alias(current_module, base_var)
            {
                // This is a module import alias with attribute access
                for attr in accessed_attrs {
                    debug!(
                        "Found attribute access on module alias in {current_module}: \
                         {base_var}.{attr} -> marking {source_module}::{attr} as used"
                    );
                    worklist.push_back((source_module.clone(), attr.clone()));
                }
            } else if let Some(source_module) =
                self.resolve_from_module_import(current_module, base_var)
            {
                // This is a from import of a module (e.g., from utils import calculator)
                for attr in accessed_attrs {
                    debug!(
                        "Found attribute access on from-imported module in {current_module}: \
                         {base_var}.{attr} -> marking {source_module}::{attr} as used"
                    );
                    worklist.push_back((source_module.clone(), attr.clone()));
                }
            } else if let Some((source_module, _)) =
                self.resolve_import_alias(current_module, base_var)
            {
                // This is an imported symbol with attribute access
                for attr in accessed_attrs {
                    debug!(
                        "Found attribute access in {current_module}: {base_var}.{attr} -> marking \
                         {source_module}::{attr} as used"
                    );
                    worklist.push_back((source_module.clone(), attr.clone()));
                }
            } else if self.module_items.contains_key(base_var) {
                // Direct module reference
                for attr in accessed_attrs {
                    debug!(
                        "Found direct module attribute access in {current_module}: \
                         {base_var}.{attr}"
                    );
                    worklist.push_back((base_var.clone(), attr.clone()));
                }
            } else {
                self.find_attribute_in_namespace(
                    base_var,
                    accessed_attrs,
                    worklist,
                    current_module,
                );
            }
        }
    }

    /// Get symbols that survive tree-shaking for a module
    pub fn get_used_symbols_for_module(
        &self,
        module_name: &str,
    ) -> crate::types::FxIndexSet<String> {
        self.used_symbols
            .iter()
            .filter(|(module, _)| module == module_name)
            .map(|(_, symbol)| symbol.clone())
            .collect()
    }

    /// Check if a symbol is used after tree-shaking
    pub fn is_symbol_used(&self, module_name: &str, symbol_name: &str) -> bool {
        self.used_symbols
            .contains(&(module_name.to_string(), symbol_name.to_string()))
    }

    /// Get all unused symbols for a module
    pub fn get_unused_symbols_for_module(&self, module_name: &str) -> Vec<String> {
        let mut unused = Vec::new();

        if let Some(items) = self.module_items.get(module_name) {
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
    pub fn module_has_side_effects(&self, module_name: &str) -> bool {
        if let Some(items) = self.module_items.get(module_name) {
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

    /// Find attribute in namespace package submodules
    fn find_attribute_in_namespace(
        &self,
        base_var: &str,
        accessed_attrs: &FxHashSet<String>,
        worklist: &mut VecDeque<(String, String)>,
        context: &str,
    ) {
        let is_namespace = self
            .module_items
            .keys()
            .any(|key| key.starts_with(&format!("{base_var}.")));

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
                debug!(
                    "Warning: Could not find {attr} in any submodule of {base_var} from {context}"
                );
            }
        }
    }

    /// Find which submodule defines an attribute
    fn find_attribute_in_submodules(&self, base_var: &str, attr: &str) -> Option<String> {
        for (module_name, items) in &self.module_items {
            if module_name.starts_with(&format!("{base_var}.")) {
                for item in items {
                    if item.defined_symbols.contains(attr) {
                        return Some(module_name.clone());
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
        resolved_from_module: &str,
        worklist: &mut VecDeque<(String, String)>,
    ) {
        for item in target_items {
            if item.defined_symbols.contains("__all__")
                && let ItemType::Assignment { targets, .. } = &item.item_type
            {
                for target in targets {
                    if target == "__all__" {
                        // Mark all symbols listed in __all__
                        for symbol in &item.read_vars {
                            if !symbol.starts_with('_') {
                                debug!(
                                    "Marking {symbol} from star import of {resolved_from_module} \
                                     as used"
                                );
                                worklist
                                    .push_back((resolved_from_module.to_string(), symbol.clone()));
                            }
                        }
                    }
                }
            }
        }
    }

    /// Mark all non-private symbols as used when no __all__ is defined
    fn mark_non_private_symbols_as_used(
        &self,
        target_items: &[ItemData],
        resolved_from_module: &str,
        worklist: &mut VecDeque<(String, String)>,
    ) {
        for item in target_items {
            for symbol in &item.defined_symbols {
                if !symbol.starts_with('_') {
                    debug!("Marking {symbol} from star import of {resolved_from_module} as used");
                    worklist.push_back((resolved_from_module.to_string(), symbol.clone()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::{FxHashMap, FxHashSet};

    use super::*;

    #[test]
    fn test_basic_tree_shaking() {
        let mut graph = CriboGraph::new();

        // Create a simple module with used and unused functions
        let module_id = graph.add_module(
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
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            var_decls: ["used_func".into()].into_iter().collect(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            symbol_dependencies: FxHashMap::default(),
            attribute_accesses: FxHashMap::default(),
            is_normalized_import: false,
        });

        // Add an unused function
        module.add_item(ItemData {
            item_type: ItemType::FunctionDef {
                name: "unused_func".to_string(),
            },
            defined_symbols: ["unused_func".into()].into_iter().collect(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            var_decls: ["unused_func".into()].into_iter().collect(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            symbol_dependencies: FxHashMap::default(),
            attribute_accesses: FxHashMap::default(),
            is_normalized_import: false,
        });

        // Add entry module that uses only used_func
        let entry_id =
            graph.add_module("__main__".to_string(), &std::path::PathBuf::from("main.py"));
        let entry = graph
            .modules
            .get_mut(&entry_id)
            .expect("entry module should exist");

        entry.add_item(ItemData {
            item_type: ItemType::Expression,
            defined_symbols: FxHashSet::default(),
            read_vars: ["used_func".into()].into_iter().collect(),
            eventual_read_vars: FxHashSet::default(),
            var_decls: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: true,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            symbol_dependencies: FxHashMap::default(),
            attribute_accesses: FxHashMap::default(),
            is_normalized_import: false,
        });

        // Run tree shaking
        let mut shaker = TreeShaker::from_graph(&graph);
        shaker.analyze("__main__").expect("analyze should succeed");

        // Check results
        assert!(shaker.is_symbol_used("test_module", "used_func"));
        assert!(!shaker.is_symbol_used("test_module", "unused_func"));

        let unused = shaker.get_unused_symbols_for_module("test_module");
        assert_eq!(unused, vec!["unused_func"]);
    }
}
