use std::path::PathBuf;

use log::debug;
use ruff_python_ast::{Expr, ModModule, Stmt};

use crate::{
    resolver::ModuleResolver,
    side_effects::module_has_side_effects,
    types::{FxIndexMap, FxIndexSet},
    visitors::ExportCollector,
};

/// Result of module classification
pub struct ClassificationResult {
    pub inlinable_modules: Vec<(String, ModModule, PathBuf, String)>,
    pub wrapper_modules: Vec<(String, ModModule, PathBuf, String)>,
    pub module_exports_map: FxIndexMap<String, Option<Vec<String>>>,
}

/// Analyzes and classifies modules for bundling
pub struct ModuleClassifier<'a> {
    resolver: &'a ModuleResolver,
    entry_module_name: String,
    entry_is_package_init_or_main: bool,
    modules_with_explicit_all: FxIndexSet<String>,
    namespace_imported_modules: FxIndexMap<String, FxIndexSet<String>>,
    circular_modules: FxIndexSet<String>,
}

impl<'a> ModuleClassifier<'a> {
    /// Create a new module classifier
    pub fn new(
        resolver: &'a ModuleResolver,
        entry_module_name: String,
        entry_is_package_init_or_main: bool,
        namespace_imported_modules: FxIndexMap<String, FxIndexSet<String>>,
        circular_modules: FxIndexSet<String>,
    ) -> Self {
        Self {
            resolver,
            entry_module_name,
            entry_is_package_init_or_main,
            modules_with_explicit_all: FxIndexSet::default(),
            namespace_imported_modules,
            circular_modules,
        }
    }

    /// Get the entry package name when entry is a package __init__.py
    /// Returns None if entry is not a package __init__.py
    fn entry_package_name(&self) -> Option<&str> {
        if crate::util::is_init_module(&self.entry_module_name) {
            // Strip the .__init__ suffix if present, otherwise return None
            // Note: if entry is bare "__init__", we don't have the package name
            self.entry_module_name.strip_suffix(".__init__")
        } else {
            None
        }
    }

    /// Check if a module accesses attributes on imported modules at module level
    /// where those imported modules are part of the same circular dependency
    fn module_accesses_imported_attributes(&self, ast: &ModModule, module_name: &str) -> bool {
        use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};

        // First, collect all module-level imports and their names
        let mut imported_module_names = FxIndexSet::default();

        for stmt in &ast.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let imported_module = alias.name.as_str();
                        // In Python, `import a.b` binds `a`; `import a.b as x` binds `x`
                        let imported_as: String = if let Some(asname) = &alias.asname {
                            asname.as_str().to_string()
                        } else {
                            // For "import a.b.c", only "a" is bound in the namespace
                            imported_module
                                .split('.')
                                .next()
                                .unwrap_or(imported_module)
                                .to_string()
                        };
                        // Check if this imported module is in the circular dependency
                        if self.circular_modules.contains(imported_module) {
                            imported_module_names.insert(imported_as);
                        }
                    }
                }
                Stmt::ImportFrom(import_from) => {
                    // Handle relative and absolute imports via the resolver for correctness
                    let resolved_module = if import_from.level > 0 {
                        self.resolver.resolve_relative_import_from_package_name(
                            import_from.level,
                            import_from.module.as_deref(),
                            module_name,
                        )
                    } else if let Some(module) = &import_from.module {
                        module.as_str().to_string()
                    } else {
                        continue; // Invalid import
                    };

                    // Check if we're importing the module itself (from x import y where y is a
                    // module)
                    for alias in &import_from.names {
                        let name = alias.name.as_str();
                        let imported_as = alias.asname.as_ref().unwrap_or(&alias.name);
                        // Check if this could be a module import
                        let potential_module = format!("{resolved_module}.{name}");
                        if self.circular_modules.contains(&potential_module) {
                            imported_module_names.insert(imported_as.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        // If no circular modules are imported, no need to check further
        if imported_module_names.is_empty() {
            return false;
        }

        // Now check if we access attributes on any of these imported circular modules
        struct AttributeAccessChecker<'a> {
            has_circular_attribute_access: bool,
            imported_circular_modules: &'a FxIndexSet<String>,
        }

        impl<'a> Visitor<'a> for AttributeAccessChecker<'a> {
            fn visit_stmt(&mut self, stmt: &'a Stmt) {
                match stmt {
                    // Skip function and class bodies - we only care about module-level code
                    Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                        // Don't recurse into function or class bodies
                    }
                    _ => {
                        // Continue visiting for other statements
                        walk_stmt(self, stmt);
                    }
                }
            }

            fn visit_expr(&mut self, expr: &'a Expr) {
                if self.has_circular_attribute_access {
                    return; // Already found one
                }

                // Check for attribute access on names (e.g., mod_c.C_CONSTANT)
                if let Expr::Attribute(attr) = expr
                    && let Expr::Name(name_expr) = &*attr.value
                {
                    // Check if this name is one of our imported circular modules
                    if self
                        .imported_circular_modules
                        .contains(name_expr.id.as_str())
                    {
                        self.has_circular_attribute_access = true;
                        return;
                    }
                }

                // Continue walking
                walk_expr(self, expr);
            }
        }

        let mut checker = AttributeAccessChecker {
            has_circular_attribute_access: false,
            imported_circular_modules: &imported_module_names,
        };

        checker.visit_body(&ast.body);
        checker.has_circular_attribute_access
    }

    /// Classify modules into inlinable and wrapper modules
    /// Also collects module exports and tracks modules with explicit __all__
    pub fn classify_modules(
        mut self,
        modules: &[(String, ModModule, PathBuf, String)],
        python_version: u8,
    ) -> (ClassificationResult, FxIndexSet<String>) {
        let mut inlinable_modules = Vec::new();
        let mut wrapper_modules = Vec::new();
        let mut module_exports_map = FxIndexMap::default();

        for (module_name, ast, module_path, content_hash) in modules {
            debug!("Processing module: '{module_name}'");
            if module_name == &self.entry_module_name {
                continue;
            }

            // Also skip if this is the entry package itself when entry is a package __init__.py
            // e.g., skip "yaml" when entry is "yaml.__init__"
            if self.entry_is_package_init_or_main {
                if let Some(entry_pkg) = self.entry_package_name() {
                    if module_name == entry_pkg {
                        debug!(
                            "Skipping module '{module_name}' as it's the package name for entry module '__init__.py'"
                        );
                        continue;
                    }
                } else if crate::util::is_init_module(&self.entry_module_name)
                    && self.entry_module_name == "__init__"
                {
                    // Special case: entry is bare "__init__" without package prefix
                    // In this case, we need to check if the module matches the inferred package name
                    // This happens when the entry module is discovered as "__init__" without full path context
                    if !module_name.contains('.') {
                        // This could be the package, but we need more context to be sure
                        // For safety, we should NOT skip it unless we're certain
                        debug!(
                            "Not skipping top-level module '{module_name}' as we cannot confirm it matches entry '__init__'"
                        );
                    }
                }
            }

            // Extract __all__ exports from the module using ExportCollector
            let export_info = ExportCollector::analyze(ast);
            let has_explicit_all = export_info.exported_names.is_some();
            if has_explicit_all {
                self.modules_with_explicit_all.insert(module_name.clone());
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
                                "Module '{module_name}' has wildcard import from '{dots}{from_module_str}'"
                            );

                            // Mark that this module has a wildcard import
                            if expanded_exports.is_none() {
                                expanded_exports = Some(Vec::new());
                            }

                            // We'll resolve this in the second pass - for now, exclude '*' from the exports
                            if let Some(ref mut exports) = expanded_exports {
                                exports.retain(|s| s != "*");
                            }
                        }
                    }
                }
            }

            module_exports_map.insert(module_name.clone(), expanded_exports);

            // Check if module is imported as a namespace
            let is_namespace_imported = self.namespace_imported_modules.contains_key(module_name);

            if is_namespace_imported {
                debug!(
                    "Module '{}' is imported as namespace by: {:?}",
                    module_name,
                    self.namespace_imported_modules.get(module_name)
                );
            }

            // With full static bundling, we only need to wrap modules with side effects
            // All imports are rewritten at bundle time, so namespace imports, direct imports,
            // and circular dependencies can all be handled through static transformation
            let has_side_effects = module_has_side_effects(ast, python_version);

            // Check if this module is in a circular dependency and accesses imported module
            // attributes
            let needs_wrapping_for_circular = self.circular_modules.contains(module_name)
                && self.module_accesses_imported_attributes(ast, module_name);

            // Check if this module has an invalid identifier (can't be imported normally)
            // These modules are likely imported via importlib and need to be wrapped
            // Note: Module names with dots are valid (e.g., "core.utils.helpers"), so we only
            // check if the module name itself (without dots) is invalid
            let module_base_name = module_name.split('.').next_back().unwrap_or(module_name);
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
                        "Module '{module_name}' is in circular dependency and accesses imported \
                         attributes - using wrapper approach"
                    );
                } else {
                    debug!("Module '{module_name}' has side effects - using wrapper approach");
                }

                wrapper_modules.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            } else {
                debug!("Module '{module_name}' has no side effects - can be inlined");
                inlinable_modules.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            }
        }

        // Second pass: resolve wildcard imports now that all modules have been processed
        let mut wildcard_imports: FxIndexMap<String, FxIndexSet<String>> = FxIndexMap::default();

        for (module_name, ast, _, _) in modules {
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
                                module_name,
                            )
                        } else if let Some(module) = &import_from.module {
                            module.to_string()
                        } else {
                            continue;
                        };

                        wildcard_imports
                            .entry(module_name.clone())
                            .or_default()
                            .insert(imported);
                    }
                }
            }
        }

        // Now expand wildcard imports in module_exports_map
        for (module_name, wildcard_sources) in wildcard_imports {
            // Respect explicit __all__: don't auto-expand wildcard imports
            if self.modules_with_explicit_all.contains(&module_name) {
                debug!(
                    "Skipping wildcard expansion for module '{module_name}' due to explicit __all__"
                );
                continue;
            }

            debug!("Module '{module_name}' has wildcard imports from: {wildcard_sources:?}");

            // Collect exports from all source modules first to avoid double borrow
            let mut exports_to_add = Vec::new();
            for source_module in &wildcard_sources {
                if let Some(source_exports) = module_exports_map.get(source_module)
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
                && let Some(exports) = module_exports_map.get_mut(&module_name)
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

        let result = ClassificationResult {
            inlinable_modules,
            wrapper_modules,
            module_exports_map,
        };

        (result, self.modules_with_explicit_all)
    }
}
