use std::{cell::RefCell, collections::VecDeque};

use log::{debug, info, trace, warn};

use crate::{
    dependency_graph::{DependencyGraph, ItemData, ItemType},
    resolver::{ModuleId, ModuleResolver},
    types::{FxIndexMap, FxIndexSet},
};

/// Tree shaker that removes unused symbols from modules
#[derive(Debug)]
pub(crate) struct TreeShaker<'a> {
    /// Centralized module resolver for import resolution
    resolver: &'a ModuleResolver,
    /// Borrowed reference to the dependency graph (avoids cloning all items)
    graph: &'a DependencyGraph,
    /// Final set of symbols to keep (`module_id`, `symbol_name`)
    used_symbols: FxIndexSet<(ModuleId, String)>,
    /// Side-effect modules already seeded during this analysis run.
    seeded_side_effect_modules: RefCell<FxIndexSet<ModuleId>>,
    /// Modules already checked for dynamic `__all__` access during this analysis run.
    seeded_dynamic_all_modules: RefCell<FxIndexSet<ModuleId>>,
}

impl<'a> TreeShaker<'a> {
    // Removed resolver_context_module_name: no longer needed

    /// Resolve a relative import using the resolver with filesystem context when available.
    fn resolve_relative_with_context(
        &self,
        current_module_id: ModuleId,
        level: u32,
        name: &str,
    ) -> String {
        let name_opt = if name.is_empty() { None } else { Some(name) };
        if let Some(current_path) = self.resolver.get_module_path(current_module_id)
            && let Some(resolved) = self.resolver.resolve_relative_to_absolute_module_name(
                level,
                name_opt,
                &current_path,
            )
        {
            return resolved;
        }
        // Fallback to name-based resolution
        let current_name = self
            .graph
            .modules
            .get(&current_module_id)
            .map_or("", |m| m.module_name.as_str());
        self.resolver
            .resolve_relative_import_from_package_name(level, name_opt, current_name)
    }

    /// Resolve an import target to its absolute module name.
    pub(crate) fn resolve_import_module_name(
        &self,
        current_module_id: ModuleId,
        module: &str,
        level: u32,
    ) -> String {
        if level > 0 {
            self.resolve_relative_with_context(
                current_module_id,
                level,
                module.trim_start_matches('.'),
            )
        } else {
            module.to_owned()
        }
    }

    /// Create a tree shaker from an existing `DependencyGraph`
    pub(crate) fn from_graph(graph: &'a DependencyGraph, resolver: &'a ModuleResolver) -> Self {
        Self {
            resolver,
            graph,
            used_symbols: FxIndexSet::default(),
            seeded_side_effect_modules: RefCell::new(FxIndexSet::default()),
            seeded_dynamic_all_modules: RefCell::new(FxIndexSet::default()),
        }
    }

    /// Helper to get module display name for logging
    fn get_module_display_name(&self, module_id: ModuleId) -> String {
        self.graph.modules.get(&module_id).map_or_else(
            || format!("<unknown module {module_id}>"),
            |m| m.module_name.clone(),
        )
    }

    /// Analyze which symbols should be kept based on entry point
    pub(crate) fn analyze(&mut self, entry_module: &str) {
        info!("Starting tree-shaking analysis from entry module: {entry_module}");
        self.seeded_side_effect_modules.borrow_mut().clear();
        self.seeded_dynamic_all_modules.borrow_mut().clear();

        // Verify that the entry module is registered with the expected ID
        let entry_id = self.graph.module_names.get(entry_module).copied();
        if entry_id != Some(ModuleId::ENTRY) {
            warn!("Entry module '{entry_module}' not registered as ModuleId::ENTRY");
            if entry_id.is_none() {
                warn!("Entry module '{entry_module}' not found in module registry");
                return;
            }
        }

        // Then, mark symbols used from the entry module
        self.mark_used_symbols(entry_id.unwrap_or(ModuleId::ENTRY));

        info!(
            "Tree-shaking complete. Keeping {} symbols",
            self.used_symbols.len()
        );
    }

    /// Check if a symbol is defined in a specific module
    fn is_defined_in_module(&self, module_id: ModuleId, symbol: &str) -> bool {
        self.graph
            .modules
            .get(&module_id)
            .is_some_and(|module_dep| module_dep.defines_symbol(symbol))
    }

    /// Find which module defines a symbol
    fn find_defining_module(&self, symbol: &str) -> Option<ModuleId> {
        for (&module_id, module_dep) in &self.graph.modules {
            if module_dep.defines_symbol(symbol) {
                return Some(module_id);
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
        if let Some(module_dep) = self.graph.modules.get(&current_module_id) {
            if let Some(bindings) = module_dep.named_import_bindings_for(alias) {
                for binding in bindings {
                    let resolved_module_name = self.resolve_import_module_name(
                        current_module_id,
                        &binding.module,
                        binding.level,
                    );

                    if let Some(&resolved_id) = self.graph.module_names.get(&resolved_module_name) {
                        return Some((resolved_id, binding.original_name.clone()));
                    }
                }
            }

            for wildcard_import in module_dep.wildcard_imports() {
                let resolved_module_name = self.resolve_import_module_name(
                    current_module_id,
                    &wildcard_import.module,
                    wildcard_import.level,
                );
                if let Some(&resolved_id) = self.graph.module_names.get(&resolved_module_name)
                    && let Some(target_dep) = self.graph.modules.get(&resolved_id)
                    && target_dep.is_in_all_export(alias)
                {
                    return Some((resolved_id, alias.to_owned()));
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
        if let Some(module_dep) = self.graph.modules.get(&current_module_id) {
            if let Some(targets) = module_dep.module_import_aliases_for(alias) {
                for target in targets {
                    if let Some(&module_id) = self.graph.module_names.get(target) {
                        return Some(module_id);
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
        if let Some(module_dep) = self.graph.modules.get(&current_module_id) {
            if let Some(bindings) = module_dep.named_import_bindings_for(alias) {
                for binding in bindings {
                    let resolved_module = self.resolve_import_module_name(
                        current_module_id,
                        &binding.module,
                        binding.level,
                    );

                    let potential_full_module =
                        format!("{resolved_module}.{}", binding.original_name);
                    if let Some(&module_id) = self.graph.module_names.get(&potential_full_module) {
                        return Some(module_id);
                    }
                }
            }
        }
        None
    }

    // Note: previous custom resolve_relative_module helper removed in favor of centralized resolver

    /// Seed side effects for a module that has been reached via imports
    fn seed_side_effects_for_module(
        &self,
        module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        {
            let mut seeded_modules = self.seeded_side_effect_modules.borrow_mut();
            if !seeded_modules.insert(module_id) {
                debug!(
                    "Skipping already-seeded side effects for module {}",
                    self.get_module_display_name(module_id)
                );
                return;
            }
        }

        if let Some(module_dep) = self.graph.modules.get(&module_id) {
            debug!(
                "Seeding side effects for reachable module: {}",
                module_dep.module_name
            );
            self.seed_dynamic_all_symbols_for_module(module_id, worklist);
            for item in module_dep.items.values() {
                match &item.item_type {
                    ItemType::Import { module, .. } => {
                        self.handle_direct_import(module, "module", worklist);
                    }
                    ItemType::FromImport { .. } => {
                        self.handle_from_import(item, module_id, "module", worklist);
                    }
                    ItemType::FunctionDef { .. } | ItemType::ClassDef { .. } => {
                        for symbol in &item.defined_symbols {
                            worklist.push_back((module_id, symbol.clone()));
                        }
                    }
                    _ if item.has_side_effects => {
                        self.add_item_dependencies(item, module_id, worklist);
                    }
                    _ => {}
                }
            }
        }
    }

    fn seed_dynamic_all_symbols_for_module(
        &self,
        module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        {
            let mut seeded_modules = self.seeded_dynamic_all_modules.borrow_mut();
            if !seeded_modules.insert(module_id) {
                return;
            }
        }

        if self.module_uses_dynamic_all_access(module_id) {
            self.mark_all_symbols_from_module_all_as_used(module_id, worklist);
        }
    }

    /// Mark all symbols transitively used from entry module
    fn mark_used_symbols(&mut self, entry_id: ModuleId) {
        let mut worklist: VecDeque<(ModuleId, String)> = VecDeque::new();

        // Start with all symbols referenced by the entry module
        if let Some(entry_dep) = self.graph.modules.get(&entry_id) {
            self.seed_dynamic_all_symbols_for_module(entry_id, &mut worklist);
            for item in entry_dep.items.values() {
                match &item.item_type {
                    ItemType::Import { module, .. } => {
                        self.handle_direct_import(module, "entry module", &mut worklist);
                    }
                    ItemType::FromImport { .. } => {
                        self.handle_from_import(item, entry_id, "entry module", &mut worklist);
                    }
                    _ => {}
                }

                // Mark classes and functions defined in the entry module as used
                // This ensures that classes/functions defined in the entry module
                // (even inside try blocks) are kept along with their dependencies
                match &item.item_type {
                    ItemType::ClassDef { name } | ItemType::FunctionDef { name } => {
                        debug!("Marking entry module class/function '{name}' as used");
                        worklist.push_back((entry_id, name.clone()));
                    }
                    _ => {}
                }

                // Add symbols from read_vars
                self.add_vars_to_worklist(&item.read_vars, entry_id, &mut worklist, "entry module");

                // Add symbols from eventual_read_vars
                self.add_vars_to_worklist(
                    &item.eventual_read_vars,
                    entry_id,
                    &mut worklist,
                    "entry module (eventual)",
                );

                // Mark all side-effect items as used
                if item.has_side_effects {
                    for symbol in &item.defined_symbols {
                        worklist.push_back((entry_id, symbol.clone()));
                    }
                }

                // Process attribute accesses - if we access `greetings.message`,
                // we need the `message` symbol from the `greetings` module
                self.add_attribute_accesses_to_worklist(
                    &item.attribute_accesses,
                    entry_id,
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
        let Some(module_dep) = self.graph.modules.get(&module_id) else {
            return;
        };

        let module_display = self.get_module_display_name(module_id);
        debug!("Processing symbol definition: {module_display}::{symbol}");

        self.seed_dynamic_all_symbols_for_module(module_id, worklist);

        // First check if this symbol is actually defined in this module
        // (not just imported/re-exported)
        let symbol_is_defined_here = module_dep.defines_symbol(symbol);

        // Only check for re-exports if the symbol is not defined here
        if !symbol_is_defined_here {
            if let Some(bindings) = module_dep.named_import_bindings_for(symbol) {
                for binding in bindings {
                    let resolved_module_name =
                        self.resolve_import_module_name(module_id, &binding.module, binding.level);

                    if let Some(&resolved_module_id) =
                        self.graph.module_names.get(&resolved_module_name)
                    {
                        debug!(
                            "Symbol {symbol} is re-exported from \
                             {resolved_module_name}::{}",
                            binding.original_name
                        );
                        worklist.push_back((resolved_module_id, binding.original_name.clone()));
                        return;
                    }
                }
            }

            for wildcard_import in module_dep.wildcard_imports() {
                let resolved_module_name = self.resolve_import_module_name(
                    module_id,
                    &wildcard_import.module,
                    wildcard_import.level,
                );

                if let Some(&resolved_module_id) =
                    self.graph.module_names.get(&resolved_module_name)
                    && let Some(target_dep) = self.graph.modules.get(&resolved_module_id)
                    && target_dep.is_in_all_export(symbol)
                {
                    debug!(
                        "Symbol {symbol} resolved via wildcard re-export from \
                         {resolved_module_name}"
                    );
                    worklist.push_back((resolved_module_id, symbol.to_owned()));
                    return;
                }
            }
        }

        for item in module_dep.items.values() {
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
        let Some(module_dep) = self.graph.modules.get(&module_id) else {
            return;
        };

        for item in module_dep.items.values() {
            // Skip items not in the target scope
            let Some(ref containing_scope) = item.containing_scope else {
                continue;
            };
            if containing_scope != scope_name {
                continue;
            }

            // This import is inside the function/class being marked as used
            match &item.item_type {
                ItemType::Import {
                    module: imported_module,
                    ..
                } => {
                    self.handle_direct_import(imported_module, scope_name, worklist);
                    self.mark_import_bindings_as_used(item, module_id, scope_name, worklist);
                }
                ItemType::FromImport { .. } => {
                    self.handle_from_import(item, module_id, scope_name, worklist);
                }
                _ => {}
            }
        }
    }

    /// Handle direct import statements within a scope
    fn handle_direct_import(
        &self,
        imported_module: &str,
        scope_name: &str,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        debug!("Marking import {imported_module} as used (inside scope {scope_name})");

        // If this imported module has side effects, seed them
        let Some(&imported_module_id) = self.graph.module_names.get(imported_module) else {
            return;
        };

        if self.module_has_side_effects(imported_module_id) {
            self.seed_side_effects_for_module(imported_module_id, worklist);
        }
    }

    /// Handle from-import statements within a reachable scope.
    fn handle_from_import(
        &self,
        item: &ItemData,
        module_id: ModuleId,
        scope_name: &str,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let ItemType::FromImport {
            module: from_module,
            names,
            level,
            is_star,
        } = &item.item_type
        else {
            return;
        };

        let resolved_module_name = self.resolve_import_module_name(module_id, from_module, *level);

        if let Some(&from_module_id) = self.graph.module_names.get(&resolved_module_name)
            && self.module_has_side_effects(from_module_id)
        {
            self.seed_side_effects_for_module(from_module_id, worklist);
        }

        if *is_star {
            self.handle_star_import(&resolved_module_name, worklist);
        } else {
            self.handle_named_imports(&resolved_module_name, names, scope_name, worklist);
        }

        if item.containing_scope.is_some() {
            self.mark_import_bindings_as_used(item, module_id, scope_name, worklist);
        }
    }

    fn mark_import_bindings_as_used(
        &self,
        item: &ItemData,
        module_id: ModuleId,
        scope_name: &str,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        for var in &item.var_decls {
            debug!("  Tracking imported binding {var} in scope {scope_name}");
            worklist.push_back((module_id, var.clone()));
        }
    }

    /// Handle star imports
    fn handle_star_import(
        &self,
        resolved_module_name: &str,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let Some(&resolved_module_id) = self.graph.module_names.get(resolved_module_name) else {
            return;
        };

        let Some(target_dep) = self.graph.modules.get(&resolved_module_id) else {
            return;
        };

        if target_dep.has_explicit_all() {
            self.mark_all_defined_symbols_as_used(resolved_module_id, worklist);
        } else {
            self.mark_non_private_symbols_as_used(resolved_module_id, worklist);
        }
    }

    /// Handle named imports
    fn handle_named_imports(
        &self,
        resolved_module_name: &str,
        names: &[(String, Option<String>)],
        scope_name: &str,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        for (name, _alias) in names {
            debug!("Tracking named import {resolved_module_name}::{name} in scope {scope_name}");

            // Check if this is importing a submodule
            let potential_module = format!("{resolved_module_name}.{name}");
            self.check_and_seed_submodule(&potential_module, worklist);
        }
    }

    /// Check if a potential module name is a submodule with side effects and seed them
    fn check_and_seed_submodule(
        &self,
        potential_module: &str,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let Some(&submodule_id) = self.graph.module_names.get(potential_module) else {
            return;
        };

        if self.module_has_side_effects(submodule_id) {
            self.seed_side_effects_for_module(submodule_id, worklist);
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
            if let Some(source_module_id) = self.resolve_from_module_import(current_module_id, var)
            {
                self.mark_module_namespace_as_used(
                    source_module_id,
                    worklist,
                    "item read dependency",
                );
            } else if let Some(source_module_id) =
                self.resolve_module_import_alias(current_module_id, var)
            {
                self.mark_module_namespace_as_used(
                    source_module_id,
                    worklist,
                    "item read dependency",
                );
            } else if let Some((source_module_id, original_name)) =
                self.resolve_import_alias(current_module_id, var)
            {
                worklist.push_back((source_module_id, original_name));
            } else if let Some(module_id) = self.find_defining_module(var) {
                worklist.push_back((module_id, var.clone()));
            }
        }

        // Add eventual reads (from function bodies)
        for var in &item.eventual_read_vars {
            if let Some(source_module_id) = self.resolve_from_module_import(current_module_id, var)
            {
                self.mark_module_namespace_as_used(
                    source_module_id,
                    worklist,
                    "eventual item read dependency",
                );
            } else if let Some(source_module_id) =
                self.resolve_module_import_alias(current_module_id, var)
            {
                self.mark_module_namespace_as_used(
                    source_module_id,
                    worklist,
                    "eventual item read dependency",
                );
            } else if let Some((source_module_id, original_name)) =
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
    pub(crate) fn get_used_symbols_for_module(&self, module_name: &str) -> FxIndexSet<String> {
        // Get the ModuleId for this module name
        if let Some(&module_id) = self.graph.module_names.get(module_name) {
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
    pub(crate) fn is_symbol_used(&self, module_name: &str, symbol_name: &str) -> bool {
        // Get the ModuleId for this module name
        if let Some(&module_id) = self.graph.module_names.get(module_name) {
            self.used_symbols
                .contains(&(module_id, symbol_name.to_owned()))
        } else {
            false
        }
    }

    // Removed get_unused_symbols_for_module: dead code

    /// Check if a module has side effects that prevent tree-shaking
    pub(crate) fn module_has_side_effects(&self, module_id: ModuleId) -> bool {
        self.graph
            .modules
            .get(&module_id)
            .is_some_and(|module_dep| {
                // Check if any top-level item has side effects
                module_dep.items.values().any(|item| {
                    item.has_side_effects
                        && !matches!(
                            item.item_type,
                            ItemType::Import { .. } | ItemType::FromImport { .. }
                        )
                })
            })
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
            if let Some(source_module_id) = self.resolve_from_module_import(module_id, var) {
                let source_display = self.get_module_display_name(source_module_id);
                debug!(
                    "Found imported submodule dependency in {context}: {var} -> module \
                     {source_display}"
                );
                self.mark_module_namespace_as_used(source_module_id, worklist, context);
            } else if let Some((source_module_id, original_name)) =
                self.resolve_import_alias(module_id, var)
            {
                let source_display = self.get_module_display_name(source_module_id);
                debug!(
                    "Found import dependency in {context}: {var} -> \
                     {source_display}::{original_name}"
                );
                worklist.push_back((source_module_id, original_name));
            } else if let Some(source_module_id) = self.resolve_module_import_alias(module_id, var)
            {
                let source_display = self.get_module_display_name(source_module_id);
                debug!(
                    "Found module namespace dependency in {context}: {var} -> module \
                     {source_display}"
                );
                self.mark_module_namespace_as_used(source_module_id, worklist, context);
            } else if let Some(found_module_id) = self.find_defining_module(var) {
                let module_display = self.get_module_display_name(found_module_id);
                debug!("Found symbol dependency in {context}: {var} in module {module_display}");
                worklist.push_back((found_module_id, var.clone()));
            }
        }
    }

    /// Mark a module object as used, preserving only the surface observable through that namespace.
    fn mark_module_namespace_as_used(
        &self,
        module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
        context: &str,
    ) {
        let module_display = self.get_module_display_name(module_id);
        debug!("Preserving namespace surface for module {module_display} from {context}");

        if self.module_has_side_effects(module_id) {
            self.seed_side_effects_for_module(module_id, worklist);
        }

        let Some(module_dep) = self.graph.modules.get(&module_id) else {
            return;
        };

        if module_dep.has_explicit_all() {
            self.mark_all_defined_symbols_as_used(module_id, worklist);
        } else {
            self.mark_non_private_symbols_as_used(module_id, worklist);
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
            .graph
            .modules
            .get(&module_id)
            .map_or("", |m| m.module_name.as_str());
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
            } else if let Some(&base_module_id) = self.graph.module_names.get(base_var) {
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
            .graph
            .module_names
            .keys()
            .any(|name| name.starts_with(&format!("{base_var}.")));

        if !is_namespace {
            debug!("Unknown base variable for attribute access in {context}: {base_var}");
            return;
        }

        debug!("Found namespace package access in {context}: {base_var}");
        for attr in accessed_attrs {
            debug!("Looking for {attr} in submodules of {base_var}");

            // Find which submodule defines this attribute
            if let Some(module_id) = self.find_attribute_in_submodules(base_var, attr) {
                debug!(
                    "Found {attr} defined in {}",
                    self.get_module_display_name(module_id)
                );
                worklist.push_back((module_id, attr.clone()));
            } else {
                warn!("Could not find {attr} in any submodule of {base_var} from {context}");
            }
        }
    }

    /// Find which submodule defines an attribute
    fn find_attribute_in_submodules(&self, base_var: &str, attr: &str) -> Option<ModuleId> {
        let prefix = format!("{base_var}.");
        for (name, &module_id) in &self.graph.module_names {
            if name.starts_with(&prefix) {
                if let Some(module_dep) = self.graph.modules.get(&module_id)
                    && module_dep.defines_symbol(attr)
                {
                    return Some(module_id);
                }
            }
        }
        None
    }

    /// Mark symbols defined in __all__ as used for star imports
    fn mark_all_defined_symbols_as_used(
        &self,
        resolved_from_module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let resolved_from_module = self
            .graph
            .modules
            .get(&resolved_from_module_id)
            .map_or("", |m| m.module_name.as_str());
        if let Some(module_dep) = self.graph.modules.get(&resolved_from_module_id) {
            for symbol in module_dep.explicit_all_names() {
                debug!("Marking {symbol} from star import of {resolved_from_module} as used");
                worklist.push_back((resolved_from_module_id, symbol.clone()));
            }
        }
    }

    /// Mark all non-private symbols as used when no __all__ is defined
    fn mark_non_private_symbols_as_used(
        &self,
        resolved_from_module_id: ModuleId,
        worklist: &mut VecDeque<(ModuleId, String)>,
    ) {
        let resolved_from_module = self
            .graph
            .modules
            .get(&resolved_from_module_id)
            .map_or("", |m| m.module_name.as_str());
        if let Some(module_dep) = self.graph.modules.get(&resolved_from_module_id) {
            for symbol in module_dep.non_private_defined_symbol_names() {
                debug!("Marking {symbol} from star import of {resolved_from_module} as used");
                worklist.push_back((resolved_from_module_id, symbol.clone()));
            }
        }
    }

    /// Check if a module uses the dynamic __all__ access pattern
    /// This pattern involves using `locals()` or `globals()` with a loop over __all__ and setattr
    fn module_uses_dynamic_all_access(&self, module_id: ModuleId) -> bool {
        let Some(module_dep) = self.graph.modules.get(&module_id) else {
            return false;
        };

        if !module_dep.has_explicit_all() {
            return false;
        }

        // Check if the module uses setattr, (locals() or globals()), and reads __all__ in a single
        // pass Note: We don't check for vars() because that's our transformation that
        // happens after tree-shaking
        let mut uses_setattr = false;
        let mut uses_locals_or_globals = false;
        let mut reads_all = false;

        for item in module_dep.items.values() {
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
            .graph
            .modules
            .get(&module_id)
            .map_or("", |m| m.module_name.as_str());
        if let Some(module_dep) = self.graph.modules.get(&module_id) {
            for symbol in module_dep.explicit_all_names() {
                debug!(
                    "Marking {symbol} from module {module_name} as used due to dynamic \
                     __all__ access"
                );
                if let Some((source_module_id, original_name)) =
                    self.resolve_import_alias(module_id, symbol)
                {
                    worklist.push_back((source_module_id, original_name));
                } else {
                    worklist.push_back((module_id, symbol.clone()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn function_item(name: &str) -> ItemData {
        ItemData {
            item_type: ItemType::FunctionDef {
                name: name.to_owned(),
            },
            defined_symbols: std::iter::once(name.to_owned()).collect(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            var_decls: std::iter::once(name.to_owned()).collect(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: false,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: None,
        }
    }

    fn scoped_import_item(module: &str, local_name: &str, scope_name: &str) -> ItemData {
        ItemData {
            item_type: ItemType::Import {
                module: module.to_owned(),
                alias: None,
            },
            defined_symbols: FxIndexSet::default(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            var_decls: std::iter::once(local_name.to_owned()).collect(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: false,
            imported_names: std::iter::once(local_name.to_owned()).collect(),
            reexported_names: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: Some(scope_name.to_owned()),
        }
    }

    fn scoped_from_import_item(module: &str, name: &str, scope_name: &str) -> ItemData {
        ItemData {
            item_type: ItemType::FromImport {
                module: module.to_owned(),
                names: vec![(name.to_owned(), None)],
                level: 0,
                is_star: false,
            },
            defined_symbols: FxIndexSet::default(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            var_decls: std::iter::once(name.to_owned()).collect(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: false,
            imported_names: std::iter::once(name.to_owned()).collect(),
            reexported_names: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: Some(scope_name.to_owned()),
        }
    }

    #[test]
    fn test_basic_tree_shaking() {
        let mut graph = DependencyGraph::new();
        let resolver = ModuleResolver::new(crate::config::Config::default());

        // Create a simple module with used and unused functions
        let module_id = graph.add_module(
            ModuleId::new(1),
            "test_module".to_owned(),
            &std::path::PathBuf::from("test.py"),
        );
        let module = graph
            .modules
            .get_mut(&module_id)
            .expect("module should exist");

        // Add a used function
        module.add_item(ItemData {
            item_type: ItemType::FunctionDef {
                name: "used_func".to_owned(),
            },
            defined_symbols: std::iter::once("used_func".into()).collect(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            var_decls: std::iter::once("used_func".into()).collect(),
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
                name: "unused_func".to_owned(),
            },
            defined_symbols: std::iter::once("unused_func".into()).collect(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            var_decls: std::iter::once("unused_func".into()).collect(),
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
            ModuleId::new(0),
            "__main__".to_owned(),
            &std::path::PathBuf::from("main.py"),
        );
        let entry = graph
            .modules
            .get_mut(&entry_id)
            .expect("entry module should exist");

        entry.add_item(ItemData {
            item_type: ItemType::Expression,
            defined_symbols: FxIndexSet::default(),
            read_vars: std::iter::once("used_func".into()).collect(),
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
        let mut shaker = TreeShaker::from_graph(&graph, &resolver);
        shaker.analyze("__main__");

        // Check results
        assert!(shaker.is_symbol_used("test_module", "used_func"));
        assert!(!shaker.is_symbol_used("test_module", "unused_func"));

        // Verify unused symbol by negative is_symbol_used check
        assert!(!shaker.is_symbol_used("test_module", "unused_func"));
    }

    #[test]
    fn test_mark_all_symbols_from_module_all_as_used_falls_back_to_local_symbol() {
        let mut graph = DependencyGraph::new();
        let resolver = ModuleResolver::new(crate::config::Config::default());

        let module_id = graph.add_module(
            ModuleId::new(1),
            "all_module".to_owned(),
            &std::path::PathBuf::from("all_module.py"),
        );
        let module = graph
            .modules
            .get_mut(&module_id)
            .expect("module should exist");

        module.add_item(ItemData {
            item_type: ItemType::Assignment {
                targets: vec!["__all__".to_owned()],
            },
            defined_symbols: FxIndexSet::default(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: std::iter::once("local_export".to_owned()).collect(),
            var_decls: std::iter::once("__all__".to_owned()).collect(),
            write_vars: std::iter::once("__all__".to_owned()).collect(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: false,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: None,
        });
        module.add_item(function_item("local_export"));

        let shaker = TreeShaker::from_graph(&graph, &resolver);
        let mut worklist = VecDeque::new();
        shaker.mark_all_symbols_from_module_all_as_used(module_id, &mut worklist);

        assert_eq!(
            worklist.pop_front(),
            Some((module_id, "local_export".to_owned()))
        );
        assert!(worklist.is_empty());
    }

    #[test]
    fn test_mark_scoped_imports_marks_local_import_bindings_used() {
        let mut graph = DependencyGraph::new();
        let resolver = ModuleResolver::new(crate::config::Config::default());

        let module_id = graph.add_module(
            ModuleId::new(1),
            "scoped_imports".to_owned(),
            &std::path::PathBuf::from("scoped_imports.py"),
        );
        let module = graph
            .modules
            .get_mut(&module_id)
            .expect("module should exist");

        module.add_item(function_item("load"));
        module.add_item(scoped_import_item("math", "math", "load"));
        module.add_item(scoped_from_import_item("operator", "add", "load"));

        let shaker = TreeShaker::from_graph(&graph, &resolver);
        let mut worklist = VecDeque::new();
        shaker.mark_scoped_imports_as_used(module_id, "load", &mut worklist);

        let queued_symbols: FxIndexSet<(ModuleId, String)> = worklist.into_iter().collect();
        assert!(queued_symbols.contains(&(module_id, "math".to_owned())));
        assert!(queued_symbols.contains(&(module_id, "add".to_owned())));
    }

    #[test]
    fn test_find_attribute_in_submodules_returns_matching_namespace_submodule() {
        let mut graph = DependencyGraph::new();
        let resolver = ModuleResolver::new(crate::config::Config::default());

        let submodule_id = graph.add_module(
            ModuleId::new(1),
            "namespace_pkg.feature".to_owned(),
            &std::path::PathBuf::from("namespace_pkg/feature.py"),
        );
        let submodule = graph
            .modules
            .get_mut(&submodule_id)
            .expect("namespace submodule should exist");

        submodule.add_item(function_item("exported_attr"));

        let shaker = TreeShaker::from_graph(&graph, &resolver);
        let resolved_id = shaker.find_attribute_in_submodules("namespace_pkg", "exported_attr");
        let missing_id = shaker.find_attribute_in_submodules("namespace_pkg", "nonexistent_attr");

        assert_eq!(resolved_id, Some(submodule_id));
        assert_eq!(missing_id, None);
    }
}
