//! Semantic analysis for Python bundling using `ruff_python_semantic`
//!
//! This module leverages ruff's existing semantic analysis infrastructure
//! to detect symbol conflicts across modules during bundling.

use std::path::Path;

use anyhow::Result;
use ruff_python_ast::{ModModule, Stmt};
use ruff_python_semantic::{
    BindingFlags, BindingId, BindingKind, Module, ModuleKind, ModuleSource, SemanticModel,
};
use ruff_python_stdlib::builtins::{MAGIC_GLOBALS, python_builtins};
use ruff_text_size::{Ranged, TextRange};

use crate::{
    import_alias_tracker::{EnhancedFromImport, ImportAliasTracker},
    python::module_path,
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Semantic bundler that analyzes symbol conflicts across modules using full semantic models
#[derive(Debug)]
pub struct SemanticBundler {
    /// Module-specific semantic models
    module_semantics: FxIndexMap<ModuleId, ModuleSemanticInfo>,
    /// Global symbol registry with full semantic information
    global_symbols: SymbolRegistry,
    /// Import alias tracker for resolving import aliases
    import_alias_tracker: ImportAliasTracker,
    /// Track which `ModuleIds` have already been analyzed to prevent duplicate processing
    analyzed_modules: FxIndexSet<ModuleId>,
}

/// Semantic model builder that properly populates bindings using visitor pattern
struct SemanticModelBuilder<'a> {
    semantic: SemanticModel<'a>,
    /// Tracks enhanced from-import information found during traversal
    from_imports: Vec<EnhancedFromImport>,
}

impl<'a> SemanticModelBuilder<'a> {
    /// Create and populate a semantic model for a module
    fn build_semantic_model(
        file_path: &'a Path,
        ast: &'a ModModule,
    ) -> (SemanticModel<'a>, Vec<EnhancedFromImport>) {
        // Step 1: We already have the parsed AST; no need to re-parse the source here.

        // Step 2: Determine module kind
        let kind = if file_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(module_path::is_init_file_name)
        {
            ModuleKind::Package
        } else {
            ModuleKind::Module
        };

        // Step 3: Create module and semantic model
        let module = Module {
            kind,
            source: ModuleSource::File(file_path),
            python_ast: &ast.body,
            name: None,
        };

        let semantic = SemanticModel::new(&[], file_path, module);

        // Step 4: Create builder and populate semantic model
        let mut builder = Self {
            semantic,
            from_imports: Vec::new(),
        };
        builder.bind_builtins();
        builder.traverse_and_bind(&ast.body);

        (builder.semantic, builder.from_imports)
    }

    /// Bind builtin symbols to the semantic model
    fn bind_builtins(&mut self) {
        for builtin in python_builtins(u8::MAX, false).chain(MAGIC_GLOBALS.iter().copied()) {
            let binding_id = self.semantic.push_builtin();
            let scope = self.semantic.global_scope_mut();
            scope.add(builtin, binding_id);
        }
    }

    /// Traverse AST and create bindings for module-level definitions
    fn traverse_and_bind(&mut self, statements: &'a [Stmt]) {
        log::trace!("Traversing {} statements", statements.len());

        for stmt in statements {
            self.visit_stmt(stmt);
        }
    }

    /// Visit a statement and create appropriate bindings
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::ClassDef(class_def) => {
                log::trace!("Processing class definition: {}", class_def.name.id);
                self.add_binding(
                    class_def.name.id.as_str(),
                    class_def.name.range,
                    BindingKind::ClassDefinition(self.semantic.scope_id),
                    BindingFlags::empty(),
                );
            }
            Stmt::FunctionDef(func_def) => {
                log::trace!("Processing function definition: {}", func_def.name.id);
                self.add_binding(
                    func_def.name.id.as_str(),
                    func_def.name.range,
                    BindingKind::FunctionDefinition(self.semantic.scope_id),
                    BindingFlags::empty(),
                );
            }
            Stmt::Assign(assign) => {
                // Handle assignments to create variable bindings
                for target in &assign.targets {
                    if let ruff_python_ast::Expr::Name(name_expr) = target {
                        log::trace!("Processing assignment: {}", name_expr.id);
                        self.add_binding(
                            name_expr.id.as_str(),
                            name_expr.range(),
                            BindingKind::Assignment,
                            BindingFlags::empty(),
                        );
                    }
                }
            }
            // Handle imports to enable qualified name resolution
            Stmt::Import(import) => {
                for alias in &import.names {
                    let module = alias
                        .name
                        .as_str()
                        .split('.')
                        .next()
                        .expect("module name should have at least one part");
                    self.semantic.add_module(module);

                    let name = alias
                        .asname
                        .as_ref()
                        .map_or(alias.name.as_str(), ruff_python_ast::Identifier::as_str);
                    self.add_binding(
                        name,
                        alias.range,
                        BindingKind::Import(ruff_python_semantic::Import {
                            qualified_name: Box::new(
                                ruff_python_ast::name::QualifiedName::user_defined(
                                    alias.name.as_str(),
                                ),
                            ),
                        }),
                        BindingFlags::EXTERNAL,
                    );
                }
            }
            Stmt::ImportFrom(import_from) => {
                // Get the module name
                let module_name = import_from
                    .module
                    .as_ref()
                    .map(std::string::ToString::to_string);

                for alias in &import_from.names {
                    let original_name = alias.name.as_str();
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map_or(original_name, ruff_python_ast::Identifier::as_str);

                    if local_name != "*" {
                        // Track the enhanced import information
                        if let Some(ref module) = module_name {
                            let has_alias = alias.asname.is_some();
                            self.from_imports.push(EnhancedFromImport {
                                module: module.clone(),
                                original_name: original_name.to_string(),
                                local_alias: if has_alias {
                                    Some(local_name.to_string())
                                } else {
                                    None
                                },
                            });
                        }

                        self.add_binding(
                            local_name,
                            alias.range,
                            BindingKind::FromImport(ruff_python_semantic::FromImport {
                                qualified_name: Box::new(
                                    ruff_python_ast::name::QualifiedName::user_defined(
                                        original_name,
                                    ),
                                ),
                            }),
                            BindingFlags::EXTERNAL,
                        );
                    }
                }
            }
            _ => {
                // Skip other statement types for now
            }
        }
    }

    /// Add a binding to the semantic model
    fn add_binding(
        &mut self,
        name: &'a str,
        range: TextRange,
        kind: BindingKind<'a>,
        flags: BindingFlags,
    ) -> BindingId {
        // Mark private declarations
        let mut binding_flags = flags;
        if name.starts_with('_') && !name.starts_with("__") {
            binding_flags |= BindingFlags::PRIVATE_DECLARATION;
        }

        // Create binding and add to current scope
        let binding_id = self.semantic.push_binding(range, kind, binding_flags);
        let scope = self.semantic.current_scope_mut();
        scope.add(name, binding_id);

        log::trace!("Added binding '{name}' with ID {binding_id:?}");
        binding_id
    }

    /// Extract symbols from a populated semantic model
    fn extract_symbols_from_semantic_model(semantic: &SemanticModel) -> FxIndexSet<String> {
        let mut symbols = FxIndexSet::default();

        // Get the global scope (module scope)
        let global_scope = semantic.global_scope();

        log::trace!(
            "Extracting from global scope with {} bindings",
            global_scope.bindings().count()
        );

        // Iterate through all bindings in global scope
        for (name, binding_id) in global_scope.bindings() {
            let binding = &semantic.bindings[binding_id];

            // Only include symbols that are actual definitions (not imports) and not builtins
            // and are not private (unless they are dunder methods)
            log::trace!("Processing binding '{}' with kind {:?}", name, binding.kind);
            match &binding.kind {
                BindingKind::ClassDefinition(_) => {
                    if !name.starts_with('_') || name.starts_with("__") {
                        log::trace!("Adding class symbol: {name}");
                        symbols.insert(name.to_string());
                    }
                }
                BindingKind::FunctionDefinition(_) => {
                    if !name.starts_with('_') || name.starts_with("__") {
                        log::trace!("Adding function symbol: {name}");
                        symbols.insert(name.to_string());
                    }
                }
                BindingKind::Assignment => {
                    // Include module-level assignments (variables)
                    if !name.starts_with('_') {
                        log::trace!("Adding assignment symbol: {name}");
                        symbols.insert(name.to_string());
                    }
                }
                BindingKind::FromImport(_) => {
                    // Include FromImport symbols as exports
                    // This is important for __init__.py files that re-export symbols
                    if !name.starts_with('_') || name.starts_with("__") {
                        log::trace!("Adding from-import symbol: {name}");
                        symbols.insert(name.to_string());
                    }
                }
                // Skip regular imports and builtins
                BindingKind::Builtin | BindingKind::Import(_) => {
                    log::trace!("Skipping import/builtin binding: {name}");
                }
                _ => {
                    log::trace!(
                        "Skipping other binding '{}' of kind {:?}",
                        name,
                        binding.kind
                    );
                }
            }
        }

        log::trace!("Final extracted symbols: {symbols:?}");
        symbols
    }

    /// Extract ALL module-scope symbols that need to be exposed in the module namespace
    /// This includes symbols defined in conditional blocks (if/else, try/except) and imports
    fn extract_all_module_scope_symbols(semantic: &SemanticModel) -> FxIndexSet<String> {
        let mut symbols = FxIndexSet::default();

        // Get the global scope (module scope)
        let global_scope = semantic.global_scope();

        log::trace!(
            "Extracting ALL module-scope symbols from global scope with {} bindings",
            global_scope.bindings().count()
        );

        // Iterate through all bindings in global scope
        for (name, binding_id) in global_scope.bindings() {
            let binding = &semantic.bindings[binding_id];

            // Include ALL symbols except builtins
            if let BindingKind::Builtin = &binding.kind {
                log::trace!("Skipping builtin binding: {name}");
            } else {
                // Include all non-builtin symbols: classes, functions, assignments, imports
                log::trace!(
                    "Adding module-scope symbol '{}' of kind {:?}",
                    name,
                    binding.kind
                );
                symbols.insert(name.to_string());
            }
        }

        log::trace!("All module-scope symbols: {symbols:?}");
        symbols
    }
}

/// Module semantic analyzer that provides static methods for symbol extraction
/// Semantic information for a single module
#[derive(Debug)]
pub struct ModuleSemanticInfo {
    /// Symbols exported by this module (from semantic analysis)
    pub exported_symbols: FxIndexSet<String>,
    /// Symbol conflicts detected in this module
    pub conflicts: Vec<String>,
    /// All module-scope symbols that need to be exposed in the module namespace
    /// This includes symbols defined in conditional blocks (if/else, try/except)
    pub module_scope_symbols: FxIndexSet<String>,
}

/// Global symbol registry across all modules with semantic information
#[derive(Debug)]
pub struct SymbolRegistry {
    /// Symbol name -> list of modules that define it
    pub symbols: FxIndexMap<String, Vec<ModuleId>>,
    /// Renames: (`ModuleId`, `OriginalName`) -> `NewName`
    pub renames: FxIndexMap<(ModuleId, String), String>,
}

impl Default for SymbolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolRegistry {
    /// Create a new symbol registry
    pub fn new() -> Self {
        Self {
            symbols: FxIndexMap::default(),
            renames: FxIndexMap::default(),
        }
    }

    /// Register a symbol from a module (legacy interface)
    pub fn register_symbol(&mut self, symbol: String, module_id: ModuleId) {
        self.symbols.entry(symbol).or_default().push(module_id);
    }

    /// Detect conflicts across all modules
    pub fn detect_conflicts(&self) -> Vec<SymbolConflict> {
        let mut conflicts = Vec::new();

        for (symbol, modules) in &self.symbols {
            if modules.len() > 1 {
                conflicts.push(SymbolConflict {
                    symbol: symbol.clone(),
                    modules: modules.clone(),
                });
            }
        }

        conflicts
    }

    /// Generate rename for conflicting symbol
    pub fn generate_rename(
        &mut self,
        module_id: ModuleId,
        original: &str,
        suffix: usize,
    ) -> String {
        let new_name = format!("{original}_{suffix}");
        self.renames
            .insert((module_id, original.to_string()), new_name.clone());
        new_name
    }

    /// Get rename for a symbol if it exists
    pub fn get_rename(&self, module_id: ModuleId, original: &str) -> Option<&str> {
        self.renames
            .get(&(module_id, original.to_string()))
            .map(std::string::String::as_str)
    }
}

/// Represents a symbol conflict across modules
pub struct SymbolConflict {
    pub symbol: String,
    pub modules: Vec<ModuleId>,
}

/// Information about module-level global usage
#[derive(Debug, Clone, Default)]
pub struct ModuleGlobalInfo {
    /// Variables that exist at module level
    pub module_level_vars: FxIndexSet<String>,

    /// Variables that are eligible for lifting when referenced via `global` statements.
    /// Includes module-level assignments/classes plus names introduced by imports and
    /// function definitions at module scope.
    pub liftable_vars: FxIndexSet<String>,

    /// Variables declared with 'global' keyword in functions
    pub global_declarations: FxIndexMap<String, Vec<TextRange>>,

    /// Functions that use global statements
    pub functions_using_globals: FxIndexSet<String>,

    /// Module name for generating unique prefixes
    pub module_name: String,
}

impl Default for SemanticBundler {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticBundler {
    /// Create a new semantic bundler
    pub fn new() -> Self {
        Self {
            module_semantics: FxIndexMap::default(),
            global_symbols: SymbolRegistry::new(),
            import_alias_tracker: ImportAliasTracker::new(),
            analyzed_modules: FxIndexSet::default(),
        }
    }

    /// Analyze a module using full semantic model approach
    pub fn analyze_module(
        &mut self,
        module_id: ModuleId,
        ast: &ModModule,
        path: &Path,
    ) -> Result<()> {
        // Check if this ModuleId has already been analyzed
        // This prevents duplicate processing when multiple module names map to the same file
        if !self.analyzed_modules.insert(module_id) {
            log::debug!(
                "Module {} already analyzed, skipping duplicate processing",
                module_id.as_u32()
            );
            return Ok(());
        }

        log::debug!(
            "Starting semantic analysis for module {}",
            module_id.as_u32()
        );

        // Build semantic model and extract information
        let (semantic_model, from_imports) = SemanticModelBuilder::build_semantic_model(path, ast);

        // Extract exported symbols (public API)
        let exported_symbols =
            SemanticModelBuilder::extract_symbols_from_semantic_model(&semantic_model);
        log::debug!(
            "Module {} has exported symbols: {:?}",
            module_id.as_u32(),
            exported_symbols
        );

        // Extract ALL module-scope symbols (including conditional imports)
        let module_scope_symbols =
            SemanticModelBuilder::extract_all_module_scope_symbols(&semantic_model);
        log::debug!(
            "Module {} has all module-scope symbols: {:?}",
            module_id.as_u32(),
            module_scope_symbols
        );

        // Register from imports in the alias tracker
        for import in from_imports {
            self.import_alias_tracker.register_from_import(
                module_id,
                import.module,
                import.original_name,
                import.local_alias,
            );
        }

        // Register symbols in global registry, but only those that are defined locally
        // Skip FromImport symbols to avoid incorrect conflict resolution

        // Build a lookup map for O(1) access to binding information
        let binding_lookup: FxIndexMap<&str, BindingId> =
            semantic_model.global_scope().bindings().collect();

        for symbol in &exported_symbols {
            // Check if this symbol is a FromImport by looking at the semantic model
            let is_from_import = binding_lookup.get(symbol.as_str()).is_some_and(|&id| {
                matches!(semantic_model.bindings[id].kind, BindingKind::FromImport(_))
            });

            if is_from_import {
                log::debug!(
                    "Skipping registration of FromImport symbol '{}' from module {} for conflict \
                     resolution",
                    symbol,
                    module_id.as_u32()
                );
            } else {
                self.global_symbols
                    .register_symbol(symbol.clone(), module_id);
            }
        }

        // Store module semantic info
        self.module_semantics.insert(
            module_id,
            ModuleSemanticInfo {
                exported_symbols,
                conflicts: Vec::new(), // Will be populated later
                module_scope_symbols,
            },
        );

        Ok(())
    }

    /// Detect and resolve symbol conflicts across all modules
    pub fn detect_and_resolve_conflicts(&mut self) -> Vec<SymbolConflict> {
        let conflicts = self.global_symbols.detect_conflicts();

        // Generate renames for conflicting symbols
        for conflict in &conflicts {
            for (i, module_id) in conflict.modules.iter().enumerate() {
                // Generate renames for all modules in conflict (including first)
                // Generate and register a rename (return value not needed)
                self.global_symbols.generate_rename(
                    *module_id,
                    &conflict.symbol,
                    i + 1, // Start numbering from 1 instead of 0
                );

                // Update conflicts in module info
                if let Some(module_info) = self.module_semantics.get_mut(module_id) {
                    module_info.conflicts.push(conflict.symbol.clone());
                }
            }
        }

        conflicts
    }

    /// Get module semantic info
    pub fn get_module_info(&self, module_id: ModuleId) -> Option<&ModuleSemanticInfo> {
        self.module_semantics.get(&module_id)
    }

    /// Get symbol registry
    pub fn symbol_registry(&self) -> &SymbolRegistry {
        &self.global_symbols
    }
}
