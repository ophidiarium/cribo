//! Semantic analysis for Python bundling using ruff_python_semantic
//!
//! This module leverages ruff's existing semantic analysis infrastructure
//! to detect symbol conflicts across modules during bundling.

use anyhow::Result;
use ruff_linter::source_kind::SourceKind;
use ruff_python_ast::{Expr, ModModule, PySourceType, Stmt};
use ruff_python_parser::parse_unchecked_source;
use ruff_python_semantic::{
    BindingFlags, BindingId, BindingKind, Module, ModuleKind, ModuleSource, SemanticModel,
};
use ruff_python_stdlib::builtins::{MAGIC_GLOBALS, python_builtins};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashMap as FxIndexMap;
use rustc_hash::FxHashSet as FxIndexSet;
use std::path::Path;

use crate::cribo_graph::ModuleId;

/// Semantic bundler that analyzes symbol conflicts across modules using full semantic models
pub struct SemanticBundler {
    /// Module-specific semantic models
    module_semantics: FxIndexMap<ModuleId, ModuleSemanticInfo>,
    /// Global symbol registry with full semantic information
    global_symbols: SymbolRegistry,
}

/// Semantic model builder that properly populates bindings using visitor pattern
struct SemanticModelBuilder<'a> {
    semantic: SemanticModel<'a>,
}

impl<'a> SemanticModelBuilder<'a> {
    /// Create and populate a semantic model for a module
    fn build_semantic_model(
        source: &'a str,
        file_path: &'a Path,
        ast: &'a ModModule,
    ) -> Result<SemanticModel<'a>> {
        // Step 1: Parse source and create infrastructure
        let source_kind = SourceKind::Python(source.to_string());
        let source_type = PySourceType::from(file_path);
        let _parsed = parse_unchecked_source(source_kind.source_code(), source_type);

        // Step 2: Determine module kind
        let kind = if file_path.file_name().and_then(|name| name.to_str()) == Some("__init__.py") {
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
        let mut builder = Self { semantic };
        builder.bind_builtins();
        builder.traverse_and_bind(&ast.body)?;

        Ok(builder.semantic)
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
    fn traverse_and_bind(&mut self, statements: &'a [Stmt]) -> Result<()> {
        eprintln!("DEBUG: Traversing {} statements", statements.len());

        for stmt in statements {
            self.visit_stmt(stmt)?;
        }

        Ok(())
    }

    /// Visit a statement and create appropriate bindings
    fn visit_stmt(&mut self, stmt: &'a Stmt) -> Result<()> {
        match stmt {
            Stmt::ClassDef(class_def) => {
                eprintln!("DEBUG: Processing class definition: {}", class_def.name.id);
                self.add_binding(
                    class_def.name.id.as_str(),
                    class_def.name.range,
                    BindingKind::ClassDefinition(self.semantic.scope_id),
                    BindingFlags::empty(),
                )?;
            }
            Stmt::FunctionDef(func_def) => {
                eprintln!(
                    "DEBUG: Processing function definition: {}",
                    func_def.name.id
                );
                self.add_binding(
                    func_def.name.id.as_str(),
                    func_def.name.range,
                    BindingKind::FunctionDefinition(self.semantic.scope_id),
                    BindingFlags::empty(),
                )?;
            }
            Stmt::Assign(assign) => {
                // Handle assignments to create variable bindings
                for target in &assign.targets {
                    if let ruff_python_ast::Expr::Name(name_expr) = target {
                        eprintln!("DEBUG: Processing assignment: {}", name_expr.id);
                        self.add_binding(
                            name_expr.id.as_str(),
                            name_expr.range(),
                            BindingKind::Assignment,
                            BindingFlags::empty(),
                        )?;
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
                        .map(|n| n.as_str())
                        .unwrap_or(alias.name.as_str());
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
                    )?;
                }
            }
            Stmt::ImportFrom(import_from) => {
                for alias in &import_from.names {
                    let name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(alias.name.as_str());
                    if name != "*" {
                        self.add_binding(
                            name,
                            alias.range,
                            BindingKind::FromImport(ruff_python_semantic::FromImport {
                                qualified_name: Box::new(
                                    ruff_python_ast::name::QualifiedName::user_defined(
                                        alias.name.as_str(),
                                    ),
                                ),
                            }),
                            BindingFlags::EXTERNAL,
                        )?;
                    }
                }
            }
            _ => {
                // Skip other statement types for now
            }
        }

        Ok(())
    }

    /// Add a binding to the semantic model
    fn add_binding(
        &mut self,
        name: &'a str,
        range: TextRange,
        kind: BindingKind<'a>,
        flags: BindingFlags,
    ) -> Result<BindingId> {
        // Mark private declarations
        let mut binding_flags = flags;
        if name.starts_with('_') && !name.starts_with("__") {
            binding_flags |= BindingFlags::PRIVATE_DECLARATION;
        }

        // Create binding and add to current scope
        let binding_id = self.semantic.push_binding(range, kind, binding_flags);
        let scope = self.semantic.current_scope_mut();
        scope.add(name, binding_id);

        eprintln!("DEBUG: Added binding '{}' with ID {:?}", name, binding_id);
        Ok(binding_id)
    }

    /// Extract symbols from a populated semantic model
    fn extract_symbols_from_semantic_model(semantic: &SemanticModel) -> Result<FxIndexSet<String>> {
        let mut symbols = FxIndexSet::default();

        // Get the global scope (module scope)
        let global_scope = semantic.global_scope();

        eprintln!(
            "DEBUG: Extracting from global scope with {} bindings",
            global_scope.bindings().count()
        );

        // Iterate through all bindings in global scope
        for (name, binding_id) in global_scope.bindings() {
            let binding = &semantic.bindings[binding_id];

            // Only include symbols that are actual definitions (not imports) and not builtins
            // and are not private (unless they are dunder methods)
            match &binding.kind {
                BindingKind::ClassDefinition(_) => {
                    if !name.starts_with('_') || name.starts_with("__") {
                        eprintln!("DEBUG: Adding class symbol: {}", name);
                        symbols.insert(name.to_string());
                    }
                }
                BindingKind::FunctionDefinition(_) => {
                    if !name.starts_with('_') || name.starts_with("__") {
                        eprintln!("DEBUG: Adding function symbol: {}", name);
                        symbols.insert(name.to_string());
                    }
                }
                BindingKind::Assignment => {
                    // Include module-level assignments (variables)
                    if !name.starts_with('_') {
                        eprintln!("DEBUG: Adding assignment symbol: {}", name);
                        symbols.insert(name.to_string());
                    }
                }
                // Skip imports, builtins, and other binding types for symbol extraction
                BindingKind::Builtin | BindingKind::Import(_) | BindingKind::FromImport(_) => {
                    eprintln!("DEBUG: Skipping import/builtin binding: {}", name);
                }
                _ => {
                    eprintln!(
                        "DEBUG: Skipping other binding '{}' of kind {:?}",
                        name, binding.kind
                    );
                }
            }
        }

        eprintln!("DEBUG: Final extracted symbols: {:?}", symbols);
        Ok(symbols)
    }
}

/// Module semantic analyzer that provides static methods for symbol extraction
pub struct ModuleSemanticAnalyzer;

impl ModuleSemanticAnalyzer {
    /// Extract symbols from a module using semantic analysis
    pub fn extract_symbols_from_module(
        source: &str,
        path: &Path,
        ast: &ModModule,
    ) -> Result<FxIndexSet<String>> {
        let semantic = SemanticModelBuilder::build_semantic_model(source, path, ast)?;
        SemanticModelBuilder::extract_symbols_from_semantic_model(&semantic)
    }
}

/// Semantic information for a single module
pub struct ModuleSemanticInfo {
    /// Symbols exported by this module (from semantic analysis)
    pub exported_symbols: FxIndexSet<String>,
    /// Symbol conflicts detected in this module
    pub conflicts: Vec<String>,
    /// Source code for re-analysis if needed
    pub source: String,
    /// File path for this module
    pub file_path: std::path::PathBuf,
}

/// Global symbol registry across all modules with semantic information
pub struct SymbolRegistry {
    /// Symbol name -> list of modules that define it
    pub symbols: FxIndexMap<String, Vec<ModuleId>>,
    /// Renames: (ModuleId, OriginalName) -> NewName
    pub renames: FxIndexMap<(ModuleId, String), String>,
    /// Symbol binding information for scope analysis
    pub symbol_bindings: FxIndexMap<(ModuleId, String), SymbolBindingInfo>,
}

/// Information about a symbol binding from semantic analysis
#[derive(Debug, Clone)]
pub struct SymbolBindingInfo {
    /// The binding kind (class, function, assignment, etc.)
    pub binding_kind: String, // Simplified from BindingKind for storage
    /// Whether this symbol is at module level
    pub is_module_level: bool,
    /// Whether this symbol is private
    pub is_private: bool,
    /// Scope level where this symbol is defined
    pub scope_level: usize,
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
            symbol_bindings: FxIndexMap::default(),
        }
    }

    /// Register a symbol from a module with semantic information
    pub fn register_symbol_with_binding(
        &mut self,
        symbol: String,
        module_id: ModuleId,
        binding_info: SymbolBindingInfo,
    ) {
        self.symbols
            .entry(symbol.clone())
            .or_default()
            .push(module_id);

        self.symbol_bindings
            .insert((module_id, symbol), binding_info);
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
        let new_name = format!("{}_{}", original, suffix);
        self.renames
            .insert((module_id, original.to_string()), new_name.clone());
        new_name
    }

    /// Get rename for a symbol if it exists
    pub fn get_rename(&self, module_id: &ModuleId, original: &str) -> Option<&str> {
        self.renames
            .get(&(*module_id, original.to_string()))
            .map(|s| s.as_str())
    }

    /// Check if a symbol has conflicts
    pub fn has_conflict(&self, symbol: &str) -> bool {
        self.symbols
            .get(symbol)
            .is_some_and(|modules| modules.len() > 1)
    }

    /// Get binding information for a symbol in a module
    pub fn get_symbol_binding(
        &self,
        module_id: &ModuleId,
        symbol: &str,
    ) -> Option<&SymbolBindingInfo> {
        self.symbol_bindings.get(&(*module_id, symbol.to_string()))
    }

    /// Check if a symbol is module-level in a specific module
    pub fn is_module_level_symbol(&self, module_id: &ModuleId, symbol: &str) -> bool {
        self.get_symbol_binding(module_id, symbol)
            .is_some_and(|info| info.is_module_level)
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

    /// Variables declared with 'global' keyword in functions
    pub global_declarations: FxIndexMap<String, Vec<TextRange>>,

    /// Locations where globals are read
    pub global_reads: FxIndexMap<String, Vec<TextRange>>,

    /// Locations where globals are written  
    pub global_writes: FxIndexMap<String, Vec<TextRange>>,

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
        }
    }

    /// Analyze a module using full semantic model approach
    pub fn analyze_module(
        &mut self,
        module_id: ModuleId,
        ast: &ModModule,
        source: &str,
        path: &Path,
    ) -> Result<()> {
        eprintln!(
            "SEMANTIC: Starting semantic analysis for module {}",
            module_id.as_u32()
        );

        // Extract module-level symbols using semantic analysis
        let exported_symbols =
            ModuleSemanticAnalyzer::extract_symbols_from_module(source, path, ast)?;
        eprintln!(
            "SEMANTIC: Module {} has symbols: {:?}",
            module_id.as_u32(),
            exported_symbols
        );

        // Register symbols in global registry (simplified for now)
        for symbol in &exported_symbols {
            self.global_symbols
                .register_symbol(symbol.clone(), module_id);
        }

        // Store module semantic info
        self.module_semantics.insert(
            module_id,
            ModuleSemanticInfo {
                exported_symbols,
                conflicts: Vec::new(), // Will be populated later
                source: source.to_string(),
                file_path: path.to_path_buf(),
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
                let _new_name = self.global_symbols.generate_rename(
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
    pub fn get_module_info(&self, module_id: &ModuleId) -> Option<&ModuleSemanticInfo> {
        self.module_semantics.get(module_id)
    }

    /// Get symbol registry
    pub fn symbol_registry(&self) -> &SymbolRegistry {
        &self.global_symbols
    }

    /// Get semantic information for a module (for scope analysis during code generation)
    pub fn get_module_semantic_info(&self, module_id: &ModuleId) -> Option<&ModuleSemanticInfo> {
        self.module_semantics.get(module_id)
    }

    /// Analyze global variable usage in a module
    pub fn analyze_module_globals(
        &self,
        _module_id: ModuleId,
        ast: &ModModule,
        module_name: &str,
    ) -> ModuleGlobalInfo {
        let mut info = ModuleGlobalInfo {
            module_name: module_name.to_string(),
            ..Default::default()
        };

        // First pass: collect module-level variables
        for stmt in &ast.body {
            match stmt {
                Stmt::Assign(assign) => {
                    for target in &assign.targets {
                        if let Expr::Name(name) = target {
                            info.module_level_vars.insert(name.id.to_string());
                        }
                    }
                }
                Stmt::AnnAssign(ann_assign) => {
                    if let Expr::Name(name) = ann_assign.target.as_ref() {
                        info.module_level_vars.insert(name.id.to_string());
                    }
                }
                _ => {}
            }
        }

        // Second pass: analyze global usage in functions
        GlobalUsageVisitor::new(&mut info).visit_module(ast);

        info
    }
}

/// Visitor that tracks global variable usage in a module
pub struct GlobalUsageVisitor<'a> {
    info: &'a mut ModuleGlobalInfo,
    current_function: Option<String>,
}

impl<'a> GlobalUsageVisitor<'a> {
    pub fn new(info: &'a mut ModuleGlobalInfo) -> Self {
        Self {
            info,
            current_function: None,
        }
    }

    pub fn visit_module(&mut self, module: &ModModule) {
        for stmt in &module.body {
            self.visit_stmt(stmt);
        }
    }

    fn track_global_assignments(&mut self, targets: &[Expr]) {
        for target in targets {
            if let Expr::Name(name) = target {
                let name_str = name.id.to_string();
                if self.info.global_declarations.contains_key(&name_str) {
                    self.info
                        .global_writes
                        .entry(name_str)
                        .or_default()
                        .push(target.range());
                }
            }
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::FunctionDef(func) => {
                let old_function = self.current_function.clone();
                self.current_function = Some(func.name.to_string());

                // Visit function body
                for stmt in &func.body {
                    self.visit_stmt(stmt);
                }

                self.current_function = old_function;
            }
            Stmt::Global(global_stmt) => {
                if let Some(ref func_name) = self.current_function {
                    self.info.functions_using_globals.insert(func_name.clone());

                    for name in &global_stmt.names {
                        let name_str = name.to_string();
                        self.info
                            .global_declarations
                            .entry(name_str)
                            .or_default()
                            .push(global_stmt.range());
                    }
                }
            }
            Stmt::ClassDef(class) => {
                // Visit methods within the class
                for stmt in &class.body {
                    self.visit_stmt(stmt);
                }
            }
            Stmt::Assign(assign) => {
                // Check if we're assigning to a global
                if self.current_function.is_some() {
                    self.track_global_assignments(&assign.targets);
                }
                // Statement processed
            }
            _ => {
                // Statement processed
            }
        }
    }
}
