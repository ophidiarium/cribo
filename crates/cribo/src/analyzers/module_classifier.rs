use std::path::PathBuf;

use log::debug;
use ruff_python_ast::{ModModule, Stmt};

use crate::{
    resolver::{ModuleId, ModuleResolver},
    side_effects::module_has_side_effects,
    types::{FxIndexMap, FxIndexSet},
    visitors::ExportCollector,
};

/// Result of module classification
pub struct ClassificationResult {
    pub inlinable_modules: Vec<(ModuleId, ModModule, PathBuf, String)>,
    pub wrapper_modules: Vec<(ModuleId, ModModule, PathBuf, String)>,
    pub module_exports_map: FxIndexMap<ModuleId, Option<Vec<String>>>,
    pub modules_with_explicit_all: FxIndexSet<ModuleId>,
}

/// Analyzes and classifies modules for bundling
pub struct ModuleClassifier<'a> {
    resolver: &'a ModuleResolver,
    entry_is_package_init_or_main: bool,
    modules_with_explicit_all: FxIndexSet<ModuleId>,
    namespace_imported_modules: FxIndexMap<ModuleId, FxIndexSet<ModuleId>>,
    circular_modules: FxIndexSet<ModuleId>,
}

impl<'a> ModuleClassifier<'a> {
    /// Create a new module classifier
    pub fn new(
        resolver: &'a ModuleResolver,
        entry_is_package_init_or_main: bool,
        namespace_imported_modules: FxIndexMap<ModuleId, FxIndexSet<ModuleId>>,
        circular_modules: FxIndexSet<ModuleId>,
    ) -> Self {
        Self {
            resolver,
            entry_is_package_init_or_main,
            modules_with_explicit_all: FxIndexSet::default(),
            namespace_imported_modules,
            circular_modules,
        }
    }

    /// Get the entry package name when entry is a package __init__.py or __main__.py
    /// Returns None if entry is not a package __init__.py or __main__.py
    fn entry_package_name(&self) -> Option<String> {
        let entry_module_name = self.resolver.get_module_name(ModuleId::ENTRY)?;
        // Strip ".<init>" or ".<main>" suffix if present; bare "__init__"/"__main__" have no pkg.
        entry_module_name
            .strip_suffix(&format!(".{}", crate::python::constants::INIT_STEM))
            .or_else(|| {
                entry_module_name.strip_suffix(&format!(".{}", crate::python::constants::MAIN_STEM))
            })
            .map(std::string::ToString::to_string)
    }

    /// Classify modules into inlinable and wrapper modules
    /// Also collects module exports and tracks modules with explicit __all__
    pub fn classify_modules(
        mut self,
        modules: &FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
        python_version: u8,
    ) -> ClassificationResult {
        let mut inlinable_modules = Vec::new();
        let mut wrapper_modules = Vec::new();
        let mut module_exports_map = FxIndexMap::default();

        let entry_module_name = self
            .resolver
            .get_module_name(ModuleId::ENTRY)
            .unwrap_or_else(|| "entry".to_string());

        for (module_id, (ast, module_path, content_hash)) in modules {
            let module_name = self
                .resolver
                .get_module_name(*module_id)
                .expect("Module name must exist for ModuleId");
            debug!("Processing module: '{module_name}'");

            // Skip the entry module itself
            if *module_id == ModuleId::ENTRY {
                continue;
            }

            // Also skip if this is the entry package itself when entry is a package __init__.py
            // e.g., skip "yaml" when entry is "yaml.__init__"
            if self.entry_is_package_init_or_main {
                if let Some(entry_pkg) = self.entry_package_name() {
                    if module_name == entry_pkg {
                        debug!(
                            "Skipping module '{module_name}' as it's the package name for entry \
                             module '__init__.py'"
                        );
                        continue;
                    }
                } else if crate::util::is_init_module(&entry_module_name)
                    && entry_module_name == crate::python::constants::INIT_STEM
                {
                    // Special case: entry is bare "__init__" without package prefix
                    // In this case, we need to check if the module matches the inferred package
                    // name This happens when the entry module is discovered as
                    // "__init__" without full path context
                    if !module_name.contains('.') {
                        // This could be the package, but we need more context to be sure
                        // For safety, we should NOT skip it unless we're certain
                        debug!(
                            "Not skipping top-level module '{module_name}' as we cannot confirm \
                             it matches entry '__init__'"
                        );
                    }
                }
            }

            // We already have the ModuleId

            // Extract __all__ exports from the module using ExportCollector
            let export_info = ExportCollector::analyze(ast);
            let has_explicit_all = export_info.exported_names.is_some();
            if has_explicit_all {
                self.modules_with_explicit_all.insert(*module_id);
            }

            // Convert export info to the format expected by the bundler
            let module_exports = if let Some(exported_names) = export_info.exported_names {
                Some(exported_names)
            } else {
                // If no __all__, collect all top-level symbols using SymbolCollector
                let collected = crate::visitors::symbol_collector::SymbolCollector::analyze(ast);
                let mut symbols: Vec<_> = collected
                    .global_symbols
                    .values()
                    .filter(|s| {
                        // Include all public symbols (not starting with underscore)
                        // except __all__ itself
                        // Dunder names (e.g., __version__, __author__, __doc__) are conventionally
                        // public
                        s.name != "__all__"
                            && (!s.name.starts_with('_')
                                || (s.name.starts_with("__") && s.name.ends_with("__")))
                    })
                    .map(|s| s.name.clone())
                    .collect();

                if symbols.is_empty() {
                    None
                } else {
                    // Sort symbols for deterministic output
                    symbols.sort();
                    Some(symbols)
                }
            };

            // Handle wildcard imports - if the module has wildcard imports and no explicit __all__,
            // we need to expand those to include the actual exports from the imported modules
            let mut expanded_exports = module_exports.clone();
            if !has_explicit_all {
                // Check for wildcard imports in the module
                for stmt in &ast.body {
                    if let Stmt::ImportFrom(import_from) = stmt {
                        // Check if this is a wildcard import
                        if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*"
                        {
                            // Simple debug message - actual resolution happens in second pass
                            let from_module_str = import_from.module.as_deref().unwrap_or_default();
                            let dots = ".".repeat(import_from.level as usize);
                            debug!(
                                "Module '{module_name}' has wildcard import from \
                                 '{dots}{from_module_str}'"
                            );

                            // Mark that this module has a wildcard import
                            if expanded_exports.is_none() {
                                expanded_exports = Some(Vec::new());
                            }

                            // We'll resolve this in the second pass
                        }
                    }
                }
            }

            module_exports_map.insert(*module_id, expanded_exports);

            // Check if module is imported as a namespace
            let is_namespace_imported = self.namespace_imported_modules.contains_key(module_id);

            if is_namespace_imported {
                debug!(
                    "Module '{}' is imported as namespace by: {:?}",
                    module_name,
                    self.namespace_imported_modules.get(module_id)
                );
            }

            // With full static bundling, we only need to wrap modules with side effects
            // All imports are rewritten at bundle time, so namespace imports, direct imports,
            // and circular dependencies can all be handled through static transformation
            let has_side_effects = module_has_side_effects(ast, python_version);

            // Check if this module is in a circular dependency
            // ALL modules in a circular dependency MUST be wrapper modules to handle init ordering
            let needs_wrapping_for_circular = self.circular_modules.contains(module_id);

            // Check if this module has an invalid identifier (can't be imported normally)
            // These modules are likely imported via importlib and need to be wrapped
            // Note: Module names with dots are valid (e.g., "core.utils.helpers"), so we only
            // check if the module name itself (without dots) is invalid
            let module_base_name = module_name.split('.').next_back().unwrap_or(&module_name);
            let has_invalid_identifier =
                !ruff_python_stdlib::identifiers::is_identifier(module_base_name);

            if has_side_effects || has_invalid_identifier || needs_wrapping_for_circular {
                if has_invalid_identifier {
                    debug!(
                        "Module '{module_name}' has invalid Python identifier - using wrapper \
                         approach"
                    );
                } else if needs_wrapping_for_circular {
                    debug!(
                        "Module '{module_name}' is in circular dependency - using wrapper approach"
                    );
                } else {
                    debug!("Module '{module_name}' has side effects - using wrapper approach");
                }

                wrapper_modules.push((
                    *module_id,
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            } else {
                debug!("Module '{module_name}' has no side effects - can be inlined");
                inlinable_modules.push((
                    *module_id,
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            }
        }

        // Second pass: resolve wildcard imports now that all modules have been processed
        let mut wildcard_imports: FxIndexMap<ModuleId, FxIndexSet<String>> = FxIndexMap::default();

        for (module_id, (ast, _, _)) in modules {
            let module_name = self
                .resolver
                .get_module_name(*module_id)
                .expect("Module name must exist for ModuleId");

            // Look for wildcard imports in this module
            for stmt in &ast.body {
                if let Stmt::ImportFrom(import_from) = stmt {
                    // Check if this is a wildcard import
                    if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
                        // Resolve the imported module name using the resolver
                        let imported = if import_from.level > 0 {
                            // Relative import - use the resolver to resolve it properly
                            self.resolver.resolve_relative_import_from_package_name(
                                import_from.level,
                                import_from.module.as_deref(),
                                &module_name,
                            )
                        } else if let Some(module) = &import_from.module {
                            module.to_string()
                        } else {
                            continue;
                        };

                        wildcard_imports
                            .entry(*module_id)
                            .or_default()
                            .insert(imported);
                    }
                }
            }
        }

        // Now expand wildcard imports in module_exports_map
        for (module_id, wildcard_sources) in wildcard_imports {
            // module_id is already a ModuleId from the wildcard_imports map

            // Respect explicit __all__: don't auto-expand wildcard imports
            if self.modules_with_explicit_all.contains(&module_id) {
                let module_name = self
                    .resolver
                    .get_module_name(module_id)
                    .expect("Module name must exist for ModuleId");
                debug!(
                    "Skipping wildcard expansion for module '{module_name}' due to explicit \
                     __all__"
                );
                continue;
            }

            let module_name = self
                .resolver
                .get_module_name(module_id)
                .expect("Module name must exist for ModuleId");
            debug!("Module '{module_name}' has wildcard imports from: {wildcard_sources:?}");

            // Collect exports from all source modules first to avoid double borrow
            let mut exports_to_add = Vec::new();
            for source_module in &wildcard_sources {
                let source_id = match self.resolver.get_module_id_by_name(source_module) {
                    Some(id) => id,
                    None => continue,
                };
                if let Some(source_exports) = module_exports_map.get(&source_id)
                    && let Some(source_exports) = source_exports
                {
                    debug!(
                        "  Expanding wildcard import from '{}' with {} exports",
                        source_module,
                        source_exports.len()
                    );
                    for export in source_exports {
                        if export != "*" {
                            exports_to_add.push(export.clone());
                        }
                    }
                }
            }

            // Now add the collected exports to the module
            if !exports_to_add.is_empty()
                && let Some(exports) = module_exports_map.get_mut(&module_id)
            {
                if let Some(export_list) = exports {
                    // Merge, then sort + dedup for deterministic output
                    export_list.extend(exports_to_add);
                    export_list.sort();
                    export_list.dedup();
                } else {
                    // Module has no exports yet, create sorted, deduped list
                    let mut list = exports_to_add;
                    list.sort();
                    list.dedup();
                    *exports = Some(list);
                }
            }
        }

        ClassificationResult {
            inlinable_modules,
            wrapper_modules,
            module_exports_map,
            modules_with_explicit_all: self.modules_with_explicit_all,
        }
    }
}
