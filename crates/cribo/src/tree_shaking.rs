use std::collections::VecDeque;

use anyhow::Result;
use indexmap::{IndexMap, IndexSet};
use log::{debug, trace};
use rustc_hash::FxHashSet;

use crate::cribo_graph::{CriboGraph, ItemData, ItemType, ModuleId};

/// Tree shaker that removes unused symbols from modules
#[derive(Debug)]
pub struct TreeShaker {
    /// Module items from semantic analysis (reused from CriboGraph)
    module_items: IndexMap<String, Vec<ItemData>>,
    /// Track which symbols are used across module boundaries
    cross_module_refs: IndexMap<(String, String), IndexSet<String>>,
    /// Final set of symbols to keep (module_name, symbol_name)
    used_symbols: IndexSet<(String, String)>,
    /// Map from module ID to module name
    _module_names: IndexMap<ModuleId, String>,
}

impl TreeShaker {
    /// Create a tree shaker from an existing CriboGraph
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
                if let ItemType::FromImport { module, names, .. } = &item.item_type {
                    // Check if this import defines the alias
                    for (original_name, alias_opt) in names {
                        let local_name = alias_opt.as_ref().unwrap_or(original_name);
                        if local_name == alias {
                            // Found the import that defines this alias
                            return Some((module.clone(), original_name.clone()));
                        }
                    }
                }
            }
        }
        None
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
                        ..
                    } => {
                        for (name, _alias) in names {
                            // Check if this is importing a submodule directly
                            let potential_module = format!("{from_module}.{name}");
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
                    _ => {}
                }
            }
        }

        // Start with all symbols referenced by the entry module
        if let Some(items) = self.module_items.get(entry_module) {
            for item in items {
                // Add symbols from read_vars
                for var in &item.read_vars {
                    // Check if this var is an imported alias first
                    if let Some((source_module, original_name)) =
                        self.resolve_import_alias(entry_module, var)
                    {
                        worklist.push_back((source_module, original_name));
                    } else if let Some(module) = self.find_defining_module(var) {
                        worklist.push_back((module, var.clone()));
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
            }
        }

        // For directly imported modules, mark all their exported symbols as used
        for module_name in &directly_imported_modules {
            if let Some(module_items) = self.module_items.get(module_name) {
                for item in module_items {
                    // Mark all non-private symbols as used for direct imports
                    for symbol in &item.defined_symbols {
                        if !symbol.starts_with('_') || symbol == "__all__" {
                            debug!(
                                "Marking {symbol} from directly imported module {module_name} as \
                                 used"
                            );
                            worklist.push_back((module_name.clone(), symbol.clone()));
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
    }

    /// Get symbols that survive tree-shaking for a module
    pub fn get_used_symbols_for_module(&self, module_name: &str) -> IndexSet<String> {
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

    /// Check if an import is required by any surviving symbol
    pub fn is_import_required(
        &self,
        module_name: &str,
        import_name: &str,
        _import_source: &str,
    ) -> bool {
        // Check if any surviving symbol in this module uses this import
        for symbol in self.get_used_symbols_for_module(module_name) {
            if let Some(items) = self.module_items.get(module_name) {
                for item in items {
                    if item.defined_symbols.contains(&symbol) {
                        // Check if this item uses the import
                        if item.read_vars.contains(import_name)
                            || item.eventual_read_vars.contains(import_name)
                        {
                            return true;
                        }

                        // Check symbol dependencies
                        if let Some(deps) = item.symbol_dependencies.get(&symbol)
                            && deps.contains(import_name)
                        {
                            return true;
                        }
                    }
                }
            }
        }

        false
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

    /// Get items that should be kept for a module
    pub fn get_items_to_keep(&self, module_name: &str) -> FxHashSet<usize> {
        let mut items_to_keep = FxHashSet::default();

        if let Some(items) = self.module_items.get(module_name) {
            for (idx, item) in items.iter().enumerate() {
                // Keep the item if:
                // 1. It defines a used symbol
                let defines_used_symbol = item
                    .defined_symbols
                    .iter()
                    .any(|symbol| self.is_symbol_used(module_name, symbol));

                // 2. It has side effects
                let has_side_effects = item.has_side_effects
                    && !matches!(
                        item.item_type,
                        ItemType::Import { .. } | ItemType::FromImport { .. }
                    );

                // 3. It's an import that's still needed
                let is_needed_import = match &item.item_type {
                    ItemType::Import { module, .. } | ItemType::FromImport { module, .. } => {
                        // Check if any imported name is still used
                        item.imported_names
                            .iter()
                            .any(|name| self.is_import_required(module_name, name, module))
                    }
                    _ => false,
                };

                if defines_used_symbol || has_side_effects || is_needed_import {
                    items_to_keep.insert(idx);
                }
            }
        }

        items_to_keep
    }
}

#[cfg(test)]
mod tests {
    use rustc_hash::FxHashMap;

    use super::*;

    #[test]
    fn test_basic_tree_shaking() {
        let mut graph = CriboGraph::new();

        // Create a simple module with used and unused functions
        let module_id = graph.add_module(
            "test_module".to_string(),
            std::path::PathBuf::from("test.py"),
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
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            symbol_dependencies: FxHashMap::default(),
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
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            symbol_dependencies: FxHashMap::default(),
        });

        // Add entry module that uses only used_func
        let entry_id =
            graph.add_module("__main__".to_string(), std::path::PathBuf::from("main.py"));
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
            span: None,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            symbol_dependencies: FxHashMap::default(),
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
