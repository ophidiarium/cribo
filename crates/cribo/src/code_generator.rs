#![allow(clippy::excessive_nesting)]

use std::{
    hash::BuildHasherDefault,
    path::{Path, PathBuf},
};

use anyhow::Result;
#[allow(unused_imports)] // CowUtils trait is used for the replace method
use cow_utils::CowUtils;
use indexmap::{IndexMap, IndexSet};
use log::debug;
use ruff_python_ast::{
    Arguments, AtomicNodeIndex, ExceptHandler, Expr, ExprAttribute, ExprCall, ExprContext,
    ExprFString, ExprList, ExprName, ExprNoneLiteral, ExprStringLiteral, ExprSubscript, FString,
    FStringFlags, FStringValue, Identifier, InterpolatedElement, InterpolatedStringElement,
    InterpolatedStringElements, Keyword, ModModule, Stmt, StmtAssign, StmtClassDef,
    StmtFunctionDef, StmtImport, StmtImportFrom, StringLiteral, StringLiteralFlags,
    StringLiteralValue,
};
use ruff_text_size::TextRange;
use rustc_hash::FxHasher;

use crate::{
    cribo_graph::CriboGraph as DependencyGraph,
    semantic_bundler::{ModuleGlobalInfo, SemanticBundler, SymbolRegistry},
    transformation_context::TransformationContext,
    visitors::SideEffectDetector,
};

/// Type alias for IndexMap with FxHasher for better performance
type FxIndexMap<K, V> = IndexMap<K, V, BuildHasherDefault<FxHasher>>;
/// Type alias for IndexSet with FxHasher for better performance
type FxIndexSet<T> = IndexSet<T, BuildHasherDefault<FxHasher>>;

/// Context for module transformation operations
struct ModuleTransformContext<'a> {
    module_name: &'a str,
    synthetic_name: &'a str,
    module_path: &'a Path,
    global_info: Option<ModuleGlobalInfo>,
}

/// Context for inlining operations
struct InlineContext<'a> {
    module_exports_map: &'a FxIndexMap<String, Option<Vec<String>>>,
    global_symbols: &'a mut FxIndexSet<String>,
    module_renames: &'a mut FxIndexMap<String, FxIndexMap<String, String>>,
    inlined_stmts: &'a mut Vec<Stmt>,
    /// Import aliases in the current module being inlined (alias -> actual_name)
    import_aliases: FxIndexMap<String, String>,
    /// Deferred import assignments that need to be placed after all modules are inlined
    deferred_imports: &'a mut Vec<Stmt>,
}

/// Context for semantic analysis operations
struct SemanticContext<'a> {
    graph: &'a DependencyGraph,
    symbol_registry: &'a SymbolRegistry,
    semantic_bundler: &'a SemanticBundler,
}

/// Parameters for namespace import operations
#[allow(dead_code)]
struct NamespaceImportParams<'a> {
    local_name: &'a str,
    imported_name: &'a str,
    resolved_module: &'a str,
    full_module_path: &'a str,
}

/// Parameters for processing module globals
#[allow(dead_code)]
struct ProcessGlobalsParams<'a> {
    module_name: &'a str,
    ast: &'a ModModule,
    semantic_ctx: &'a SemanticContext<'a>,
}

/// Parameters for handling inlined module imports
#[allow(dead_code)]
struct InlinedImportParams<'a> {
    import_from: &'a StmtImportFrom,
    resolved_module: &'a str,
    ctx: &'a ModuleTransformContext<'a>,
}

/// Parameters for adding symbols to namespace
#[allow(dead_code)]
struct AddSymbolsParams<'a> {
    local_name: &'a str,
    imported_name: &'a str,
    inlined_module_key: &'a str,
}

/// Context for collecting direct imports
struct DirectImportContext<'a> {
    current_module: &'a str,
    module_path: &'a Path,
    modules: &'a [(String, ModModule, PathBuf, String)],
}

/// Parameters for handling symbol imports from inlined modules
#[allow(dead_code)]
struct SymbolImportParams<'a> {
    imported_name: &'a str,
    local_name: &'a str,
    resolved_module: &'a str,
    ctx: &'a ModuleTransformContext<'a>,
}

/// Parameters for transforming function body for lifted globals
struct TransformFunctionParams<'a> {
    lifted_names: &'a FxIndexMap<String, String>,
    global_info: &'a ModuleGlobalInfo,
    function_globals: &'a FxIndexSet<String>,
}

/// Parameters for bundle_modules function
pub struct BundleParams<'a> {
    pub modules: Vec<(String, ModModule, PathBuf, String)>, // (name, ast, path, content_hash)
    pub sorted_modules: &'a [(String, PathBuf, Vec<String>)], // Module data from CriboGraph
    pub entry_module_name: &'a str,
    pub graph: &'a DependencyGraph, // Dependency graph for unused import detection
    pub semantic_bundler: &'a SemanticBundler, // Semantic analysis results
    pub circular_dep_analysis: Option<&'a crate::cribo_graph::CircularDependencyAnalysis>, /* Circular dependency analysis */
    pub tree_shaker: Option<&'a crate::tree_shaking::TreeShaker>, // Tree shaking analysis
}

/// Transformer that lifts module-level globals to true global scope
struct GlobalsLifter {
    /// Map from original name to lifted name
    lifted_names: FxIndexMap<String, String>,
    /// Statements to add at module top level
    lifted_declarations: Vec<Stmt>,
}

/// Symbol dependency tracking for circular modules
#[derive(Debug, Default)]
struct SymbolDependencyGraph {
    /// Map from (module, symbol) to list of (module, symbol) dependencies
    dependencies: FxIndexMap<(String, String), Vec<(String, String)>>,
    /// Track which symbols are defined in which modules
    symbol_definitions: FxIndexMap<(String, String), SymbolDefinition>,
    /// Module-level dependencies (used at definition time, not inside function bodies)
    module_level_dependencies: FxIndexMap<(String, String), Vec<(String, String)>>,
}

#[derive(Debug, Clone)]
struct SymbolDefinition {
    /// Whether this is a function definition
    #[allow(dead_code)]
    is_function: bool,
    /// Whether this is a class definition
    #[allow(dead_code)]
    is_class: bool,
    /// Whether this is an assignment
    #[allow(dead_code)]
    is_assignment: bool,
    /// Dependencies this symbol has on other symbols
    #[allow(dead_code)]
    depends_on: Vec<(String, String)>,
}

impl SymbolDependencyGraph {
    /// Perform topological sort on symbols within circular modules
    /// Returns symbols in reverse topological order (dependencies first)
    fn topological_sort_symbols(
        &self,
        circular_modules: &FxIndexSet<String>,
    ) -> Result<Vec<(String, String)>> {
        use petgraph::{
            algo::toposort,
            graph::{DiGraph, NodeIndex},
        };
        use rustc_hash::FxHashMap;

        // Build a directed graph of symbol dependencies
        let mut graph = DiGraph::new();
        let mut node_map: FxHashMap<(String, String), NodeIndex> = FxHashMap::default();

        // Add nodes for all symbols in circular modules
        for (module_symbol, _) in &self.symbol_definitions {
            if circular_modules.contains(&module_symbol.0) {
                let node = graph.add_node(module_symbol.clone());
                node_map.insert(module_symbol.clone(), node);
            }
        }

        // Add edges for dependencies
        for (module_symbol, deps) in &self.dependencies {
            if let Some(&from_node) = node_map.get(module_symbol) {
                for dep in deps {
                    if let Some(&to_node) = node_map.get(dep) {
                        // Edge from symbol to its dependency
                        graph.add_edge(from_node, to_node, ());
                    }
                }
            }
        }

        // Perform topological sort
        match toposort(&graph, None) {
            Ok(sorted_nodes) => {
                // Return in reverse order (dependencies first)
                let mut result = Vec::new();
                for node_idx in sorted_nodes.into_iter().rev() {
                    result.push(graph[node_idx].clone());
                }
                Ok(result)
            }
            Err(_) => {
                // If topological sort fails, there's a cycle within symbols
                // Fall back to module order
                let mut result = Vec::new();
                for (module_symbol, _) in &self.symbol_definitions {
                    if circular_modules.contains(&module_symbol.0) {
                        result.push(module_symbol.clone());
                    }
                }
                Ok(result)
            }
        }
    }

    /// Get symbols for a specific module in dependency order
    fn get_module_symbols_ordered(&self, module_name: &str) -> Vec<String> {
        let mut module_symbols = Vec::new();

        // Collect all symbols from this module
        for ((module, symbol), _) in &self.symbol_definitions {
            if module == module_name {
                module_symbols.push(symbol.clone());
            }
        }

        // Sort by dependency order within the module
        // For now, return in definition order
        module_symbols
    }
}

/// Transform globals() calls to module.__dict__ when inside module functions
fn transform_globals_in_expr(expr: &mut Expr) {
    match expr {
        Expr::Call(call_expr) => {
            // Check if this is a globals() call
            if let Expr::Name(name_expr) = &*call_expr.func
                && name_expr.id.as_str() == "globals"
                && call_expr.arguments.args.is_empty()
            {
                // Replace the entire expression with module.__dict__
                *expr = Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "module".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("__dict__", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });
                return;
            }

            // Recursively transform in function and arguments
            transform_globals_in_expr(&mut call_expr.func);
            for arg in &mut call_expr.arguments.args {
                transform_globals_in_expr(arg);
            }
            for keyword in &mut call_expr.arguments.keywords {
                transform_globals_in_expr(&mut keyword.value);
            }
        }
        Expr::Attribute(attr_expr) => {
            transform_globals_in_expr(&mut attr_expr.value);
        }
        Expr::Subscript(subscript_expr) => {
            transform_globals_in_expr(&mut subscript_expr.value);
            transform_globals_in_expr(&mut subscript_expr.slice);
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_globals_in_expr(elem);
            }
        }
        Expr::Dict(dict_expr) => {
            for item in &mut dict_expr.items {
                if let Some(ref mut key) = item.key {
                    transform_globals_in_expr(key);
                }
                transform_globals_in_expr(&mut item.value);
            }
        }
        Expr::If(if_expr) => {
            transform_globals_in_expr(&mut if_expr.test);
            transform_globals_in_expr(&mut if_expr.body);
            transform_globals_in_expr(&mut if_expr.orelse);
        }
        // Add more expression types as needed
        _ => {}
    }
}

/// Transform globals() calls in a statement
fn transform_globals_in_stmt(stmt: &mut Stmt) {
    match stmt {
        Stmt::Expr(expr_stmt) => {
            transform_globals_in_expr(&mut expr_stmt.value);
        }
        Stmt::Assign(assign_stmt) => {
            transform_globals_in_expr(&mut assign_stmt.value);
            for target in &mut assign_stmt.targets {
                transform_globals_in_expr(target);
            }
        }
        Stmt::Return(return_stmt) => {
            if let Some(ref mut value) = return_stmt.value {
                transform_globals_in_expr(value);
            }
        }
        Stmt::If(if_stmt) => {
            transform_globals_in_expr(&mut if_stmt.test);
            for stmt in &mut if_stmt.body {
                transform_globals_in_stmt(stmt);
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(ref mut test_expr) = clause.test {
                    transform_globals_in_expr(test_expr);
                }
                for stmt in &mut clause.body {
                    transform_globals_in_stmt(stmt);
                }
            }
        }
        Stmt::FunctionDef(func_def) => {
            // Transform globals() calls in function body
            for stmt in &mut func_def.body {
                transform_globals_in_stmt(stmt);
            }
        }
        // Add more statement types as needed
        _ => {}
    }
}

impl GlobalsLifter {
    fn new(global_info: &ModuleGlobalInfo) -> Self {
        let mut lifted_names = FxIndexMap::default();
        let mut lifted_declarations = Vec::new();

        debug!("GlobalsLifter::new for module: {}", global_info.module_name);
        debug!("Module level vars: {:?}", global_info.module_level_vars);
        debug!(
            "Global declarations: {:?}",
            global_info.global_declarations.keys().collect::<Vec<_>>()
        );

        // Generate lifted names and declarations for all module-level variables
        // that are referenced with global statements
        for var_name in &global_info.module_level_vars {
            // Only lift variables that are actually used with global statements
            if global_info.global_declarations.contains_key(var_name) {
                let module_name_sanitized = global_info.module_name.cow_replace(".", "_");
                let module_name_sanitized = module_name_sanitized.cow_replace("-", "_");
                let lifted_name = format!("_cribo_{module_name_sanitized}_{var_name}");

                debug!("Creating lifted declaration for {var_name} -> {lifted_name}");

                lifted_names.insert(var_name.clone(), lifted_name.clone());

                // Create assignment: __cribo_module_var = None (will be set by init function)
                lifted_declarations.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: lifted_name.into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::NoneLiteral(ExprNoneLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));
            }
        }

        debug!("Created {} lifted declarations", lifted_declarations.len());

        Self {
            lifted_names,
            lifted_declarations,
        }
    }

    /// Get the lifted global declarations
    fn get_lifted_declarations(&self) -> Vec<Stmt> {
        self.lifted_declarations.clone()
    }

    /// Get the lifted names mapping
    fn get_lifted_names(&self) -> &FxIndexMap<String, String> {
        &self.lifted_names
    }
}

/// Parameters for creating a RecursiveImportTransformer
struct RecursiveImportTransformerParams<'a> {
    bundler: &'a HybridStaticBundler,
    module_name: &'a str,
    module_path: Option<&'a Path>,
    symbol_renames: &'a FxIndexMap<String, FxIndexMap<String, String>>,
    deferred_imports: &'a mut Vec<Stmt>,
    is_entry_module: bool,
    is_wrapper_init: bool,
    global_deferred_imports: Option<&'a FxIndexMap<(String, String), String>>,
}

/// Transformer that recursively transforms all imports in the AST
struct RecursiveImportTransformer<'a> {
    bundler: &'a HybridStaticBundler,
    module_name: &'a str,
    module_path: Option<&'a Path>,
    symbol_renames: &'a FxIndexMap<String, FxIndexMap<String, String>>,
    /// Maps import aliases to their actual module names
    /// e.g., "helper_utils" -> "utils.helpers"
    import_aliases: FxIndexMap<String, String>,
    /// Deferred import assignments for cross-module imports
    deferred_imports: &'a mut Vec<Stmt>,
    /// Flag indicating if this is the entry module
    is_entry_module: bool,
    /// Flag indicating if we're inside a wrapper module's init function
    is_wrapper_init: bool,
    /// Reference to global deferred imports registry
    global_deferred_imports: Option<&'a FxIndexMap<(String, String), String>>,
    /// Track local variable assignments to avoid treating them as module aliases
    local_variables: FxIndexSet<String>,
}

impl<'a> RecursiveImportTransformer<'a> {
    fn new(params: RecursiveImportTransformerParams<'a>) -> Self {
        Self {
            bundler: params.bundler,
            module_name: params.module_name,
            module_path: params.module_path,
            symbol_renames: params.symbol_renames,
            import_aliases: FxIndexMap::default(),
            deferred_imports: params.deferred_imports,
            is_entry_module: params.is_entry_module,
            is_wrapper_init: params.is_wrapper_init,
            global_deferred_imports: params.global_deferred_imports,
            local_variables: FxIndexSet::default(),
        }
    }

    /// Transform a module recursively, handling all imports at any depth
    fn transform_module(&mut self, module: &mut ModModule) {
        log::debug!(
            "RecursiveImportTransformer::transform_module for '{}'",
            self.module_name
        );
        // Transform all statements recursively
        self.transform_statements(&mut module.body);
    }

    /// Transform a list of statements recursively
    fn transform_statements(&mut self, stmts: &mut Vec<Stmt>) {
        log::debug!(
            "RecursiveImportTransformer::transform_statements: Processing {} statements",
            stmts.len()
        );
        let mut i = 0;
        while i < stmts.len() {
            // First check if this is an import statement that needs transformation
            let needs_transformation = matches!(&stmts[i], Stmt::Import(_) | Stmt::ImportFrom(_))
                && !self.bundler.is_hoisted_import(&stmts[i]);

            if needs_transformation {
                // Transform the import statement
                let transformed = self.transform_statement(&mut stmts[i]);

                // Remove the original statement
                stmts.remove(i);

                // Insert all transformed statements
                let num_inserted = transformed.len();
                for (j, new_stmt) in transformed.into_iter().enumerate() {
                    stmts.insert(i + j, new_stmt);
                }

                // Skip past the inserted statements
                i += num_inserted;
            } else {
                // For non-import statements, recurse into nested structures and transform
                // expressions
                match &mut stmts[i] {
                    Stmt::FunctionDef(func_def) => {
                        log::debug!(
                            "RecursiveImportTransformer: Entering function '{}'",
                            func_def.name.as_str()
                        );
                        self.transform_statements(&mut func_def.body);
                    }
                    Stmt::ClassDef(class_def) => {
                        self.transform_statements(&mut class_def.body);
                    }
                    Stmt::If(if_stmt) => {
                        self.transform_expr(&mut if_stmt.test);
                        self.transform_statements(&mut if_stmt.body);
                        for clause in &mut if_stmt.elif_else_clauses {
                            if let Some(test_expr) = &mut clause.test {
                                self.transform_expr(test_expr);
                            }
                            self.transform_statements(&mut clause.body);
                        }
                    }
                    Stmt::While(while_stmt) => {
                        self.transform_expr(&mut while_stmt.test);
                        self.transform_statements(&mut while_stmt.body);
                        self.transform_statements(&mut while_stmt.orelse);
                    }
                    Stmt::For(for_stmt) => {
                        self.transform_expr(&mut for_stmt.target);
                        self.transform_expr(&mut for_stmt.iter);
                        self.transform_statements(&mut for_stmt.body);
                        self.transform_statements(&mut for_stmt.orelse);
                    }
                    Stmt::With(with_stmt) => {
                        for item in &mut with_stmt.items {
                            self.transform_expr(&mut item.context_expr);
                        }
                        self.transform_statements(&mut with_stmt.body);
                    }
                    Stmt::Try(try_stmt) => {
                        self.transform_statements(&mut try_stmt.body);
                        for handler in &mut try_stmt.handlers {
                            let ExceptHandler::ExceptHandler(eh) = handler;
                            self.transform_statements(&mut eh.body);
                        }
                        self.transform_statements(&mut try_stmt.orelse);
                        self.transform_statements(&mut try_stmt.finalbody);
                    }
                    Stmt::Assign(assign) => {
                        // Track local variable assignments
                        for target in &assign.targets {
                            if let Expr::Name(name) = target {
                                self.local_variables.insert(name.id.to_string());
                            }
                        }
                        for target in &mut assign.targets {
                            self.transform_expr(target);
                        }
                        self.transform_expr(&mut assign.value);
                    }
                    Stmt::AugAssign(aug_assign) => {
                        self.transform_expr(&mut aug_assign.target);
                        self.transform_expr(&mut aug_assign.value);
                    }
                    Stmt::Expr(expr_stmt) => {
                        self.transform_expr(&mut expr_stmt.value);
                    }
                    Stmt::Return(ret_stmt) => {
                        if let Some(value) = &mut ret_stmt.value {
                            self.transform_expr(value);
                        }
                    }
                    Stmt::Raise(raise_stmt) => {
                        if let Some(exc) = &mut raise_stmt.exc {
                            self.transform_expr(exc);
                        }
                        if let Some(cause) = &mut raise_stmt.cause {
                            self.transform_expr(cause);
                        }
                    }
                    Stmt::Assert(assert_stmt) => {
                        self.transform_expr(&mut assert_stmt.test);
                        if let Some(msg) = &mut assert_stmt.msg {
                            self.transform_expr(msg);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
    }

    /// Transform a single statement
    fn transform_statement(&mut self, stmt: &mut Stmt) -> Vec<Stmt> {
        // Check if it's a hoisted import before matching
        let is_hoisted = self.bundler.is_hoisted_import(stmt);

        match stmt {
            Stmt::Import(import_stmt) => {
                log::debug!(
                    "RecursiveImportTransformer::transform_statement: Found Import statement"
                );
                if is_hoisted {
                    vec![stmt.clone()]
                } else {
                    // Track import aliases before rewriting
                    // But not in the entry module - in the entry module, imports create namespace
                    // objects
                    if !self.is_entry_module {
                        for alias in &import_stmt.names {
                            let module_name = alias.name.as_str();
                            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                            // Only track if it's an aliased import of an inlined module
                            if alias.asname.is_some()
                                && self.bundler.inlined_modules.contains(module_name)
                            {
                                log::debug!("Tracking import alias: {local_name} -> {module_name}");
                                self.import_aliases
                                    .insert(local_name.to_string(), module_name.to_string());
                            }
                        }
                    }

                    self.bundler
                        .rewrite_import_with_renames(import_stmt.clone(), self.symbol_renames)
                }
            }
            Stmt::ImportFrom(import_from) => {
                log::debug!(
                    "RecursiveImportTransformer::transform_statement: Found ImportFrom statement"
                );
                if is_hoisted {
                    vec![stmt.clone()]
                } else {
                    // Track import aliases before handling the import
                    if let Some(module) = &import_from.module {
                        let _module_str = module.as_str();

                        // Resolve relative imports first
                        let resolved_module = if let Some(module_path) = self.module_path {
                            self.bundler.resolve_relative_import_with_context(
                                import_from,
                                self.module_name,
                                Some(module_path),
                            )
                        } else {
                            self.bundler
                                .resolve_relative_import(import_from, self.module_name)
                        };

                        if let Some(resolved) = &resolved_module {
                            // Track aliases for imported symbols
                            for alias in &import_from.names {
                                let imported_name = alias.name.as_str();
                                let local_name =
                                    alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                                // Check if we're importing a submodule
                                let full_module_path = format!("{resolved}.{imported_name}");
                                if self.bundler.inlined_modules.contains(&full_module_path) {
                                    // Check if this is a namespace-imported module
                                    if self
                                        .bundler
                                        .namespace_imported_modules
                                        .contains_key(&full_module_path)
                                    {
                                        // Don't track namespace imports as aliases in the entry
                                        // module
                                        // They remain as namespace object references
                                        log::debug!(
                                            "Not tracking namespace import as alias: {local_name} \
                                             (namespace module)"
                                        );
                                    } else {
                                        // This is importing a submodule as a name (inlined module)
                                        log::debug!(
                                            "Tracking module import alias: {local_name} -> \
                                             {full_module_path}"
                                        );
                                        self.import_aliases
                                            .insert(local_name.to_string(), full_module_path);
                                    }
                                } else if self.bundler.inlined_modules.contains(resolved) {
                                    // Importing from an inlined module
                                    // Don't track symbol imports as module aliases!
                                    // import_aliases should only contain actual module imports,
                                    // not "from module import symbol" style imports
                                    log::debug!(
                                        "Not tracking symbol import as module alias: {local_name} \
                                         is a symbol from {resolved}, not a module alias"
                                    );
                                }
                            }
                        }
                    }

                    self.handle_import_from(import_from)
                }
            }
            _ => vec![stmt.clone()],
        }
    }

    /// Handle ImportFrom statements
    fn handle_import_from(&mut self, import_from: &StmtImportFrom) -> Vec<Stmt> {
        log::debug!(
            "RecursiveImportTransformer::handle_import_from: from {:?} import {:?}",
            import_from.module.as_ref().map(|m| m.as_str()),
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
        );

        // Resolve relative imports
        let resolved_module = if let Some(module_path) = self.module_path {
            self.bundler.resolve_relative_import_with_context(
                import_from,
                self.module_name,
                Some(module_path),
            )
        } else {
            self.bundler
                .resolve_relative_import(import_from, self.module_name)
        };

        // For entry module, check if this import would duplicate deferred imports
        if self.is_entry_module
            && let Some(ref resolved) = resolved_module
        {
            // Check if this is a wrapper module
            if self.bundler.module_registry.contains_key(resolved) {
                // Check if we have access to global deferred imports
                if let Some(global_deferred) = self.global_deferred_imports {
                    // Check each symbol to see if it's already been deferred
                    let mut all_symbols_deferred = true;
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str(); // The actual name being imported
                        if !global_deferred
                            .contains_key(&(resolved.to_string(), imported_name.to_string()))
                        {
                            all_symbols_deferred = false;
                            break;
                        }
                    }

                    if all_symbols_deferred {
                        log::debug!(
                            "  Skipping import from '{resolved}' in entry module - all symbols \
                             already deferred by inlined modules"
                        );
                        return vec![];
                    }
                }
            }
        }

        // Check if we're importing submodules that have been inlined
        // e.g., from utils import calculator where calculator is utils.calculator
        // This must be checked BEFORE checking if the parent module is inlined
        let mut result_stmts = Vec::new();
        let mut handled_any = false;

        // Handle both regular module imports and relative imports
        if let Some(ref resolved_base) = resolved_module {
            log::debug!(
                "RecursiveImportTransformer: Checking import from '{}' in module '{}'",
                resolved_base,
                self.module_name
            );
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                let full_module_path = format!("{resolved_base}.{imported_name}");

                log::debug!("  Checking if '{full_module_path}' is an inlined module");
                log::debug!(
                    "  inlined_modules contains '{}': {}",
                    full_module_path,
                    self.bundler.inlined_modules.contains(&full_module_path)
                );

                // Check if this is importing a submodule (like from . import config)
                if self.bundler.inlined_modules.contains(&full_module_path) {
                    log::debug!("  '{full_module_path}' is an inlined module");

                    // Check if this module was namespace imported
                    if self
                        .bundler
                        .namespace_imported_modules
                        .contains_key(&full_module_path)
                    {
                        // Create assignment: local_name = full_module_path_with_underscores
                        let namespace_var = full_module_path.cow_replace('.', "_").into_owned();
                        log::debug!(
                            "  Creating namespace assignment: {local_name} = {namespace_var}"
                        );
                        result_stmts.push(Stmt::Assign(StmtAssign {
                            node_index: AtomicNodeIndex::dummy(),
                            targets: vec![Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: local_name.into(),
                                ctx: ExprContext::Store,
                                range: TextRange::default(),
                            })],
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: namespace_var.into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
                        handled_any = true;
                    } else {
                        // This is importing an inlined submodule
                        // We need to handle this specially when the current module is being inlined
                        // (i.e., not the entry module and not a wrapper module that will be in
                        // sys.modules)
                        let current_module_is_inlined =
                            self.bundler.inlined_modules.contains(self.module_name);
                        let current_module_is_wrapper =
                            !current_module_is_inlined && !self.is_entry_module;

                        if !self.is_entry_module
                            && (current_module_is_inlined || current_module_is_wrapper)
                        {
                            log::debug!(
                                "  Creating namespace for inlined submodule: {local_name} -> \
                                 {full_module_path}"
                            );

                            if current_module_is_inlined {
                                // For inlined modules importing other inlined modules, we need to
                                // defer the namespace creation
                                // until after all modules are inlined
                                log::debug!(
                                    "  Deferring namespace creation for inlined module import"
                                );

                                // Create the namespace and populate it as deferred imports
                                // Create: local_name = types.SimpleNamespace()
                                self.deferred_imports.push(Stmt::Assign(StmtAssign {
                                    node_index: AtomicNodeIndex::dummy(),
                                    targets: vec![Expr::Name(ExprName {
                                        node_index: AtomicNodeIndex::dummy(),
                                        id: local_name.into(),
                                        ctx: ExprContext::Store,
                                        range: TextRange::default(),
                                    })],
                                    value: Box::new(Expr::Call(ruff_python_ast::ExprCall {
                                        node_index: AtomicNodeIndex::dummy(),
                                        func: Box::new(Expr::Attribute(ExprAttribute {
                                            node_index: AtomicNodeIndex::dummy(),
                                            value: Box::new(Expr::Name(ExprName {
                                                node_index: AtomicNodeIndex::dummy(),
                                                id: "types".into(),
                                                ctx: ExprContext::Load,
                                                range: TextRange::default(),
                                            })),
                                            attr: Identifier::new(
                                                "SimpleNamespace",
                                                TextRange::default(),
                                            ),
                                            ctx: ExprContext::Load,
                                            range: TextRange::default(),
                                        })),
                                        arguments: Arguments {
                                            node_index: AtomicNodeIndex::dummy(),
                                            args: Box::from([]),
                                            keywords: Box::from([]),
                                            range: TextRange::default(),
                                        },
                                        range: TextRange::default(),
                                    })),
                                    range: TextRange::default(),
                                }));

                                // Now add the exported symbols from the inlined module to the
                                // namespace
                                if let Some(exports) = self
                                    .bundler
                                    .module_exports
                                    .get(&full_module_path)
                                    .cloned()
                                    .flatten()
                                {
                                    // Filter exports to only include symbols that survived
                                    // tree-shaking
                                    let filtered_exports =
                                        self.bundler.filter_exports_by_tree_shaking(
                                            &exports,
                                            &full_module_path,
                                            self.bundler.tree_shaking_keep_symbols.as_ref(),
                                        );

                                    // Add __all__ attribute to the namespace with filtered exports
                                    // BUT ONLY if the original module had an explicit __all__
                                    if !filtered_exports.is_empty()
                                        && self
                                            .bundler
                                            .modules_with_explicit_all
                                            .contains(&full_module_path)
                                    {
                                        self.deferred_imports.push(Stmt::Assign(StmtAssign {
                                            node_index: AtomicNodeIndex::dummy(),
                                            targets: vec![Expr::Attribute(ExprAttribute {
                                                node_index: AtomicNodeIndex::dummy(),
                                                value: Box::new(Expr::Name(ExprName {
                                                    node_index: AtomicNodeIndex::dummy(),
                                                    id: local_name.into(),
                                                    ctx: ExprContext::Load,
                                                    range: TextRange::default(),
                                                })),
                                                attr: Identifier::new(
                                                    "__all__",
                                                    TextRange::default(),
                                                ),
                                                ctx: ExprContext::Store,
                                                range: TextRange::default(),
                                            })],
                                            value: Box::new(Expr::List(ExprList {
                                                node_index: AtomicNodeIndex::dummy(),
                                                elts: filtered_exports
                                                    .iter()
                                                    .map(|name| {
                                                        Expr::StringLiteral(ExprStringLiteral {
                                                            node_index: AtomicNodeIndex::dummy(),
                                                            value: StringLiteralValue::single(
                                                                StringLiteral {
                                                                    node_index:
                                                                        AtomicNodeIndex::dummy(),
                                                                    value: name.as_str().into(),
                                                                    flags:
                                                                        StringLiteralFlags::empty(),
                                                                    range: TextRange::default(),
                                                                },
                                                            ),
                                                            range: TextRange::default(),
                                                        })
                                                    })
                                                    .collect(),
                                                ctx: ExprContext::Load,
                                                range: TextRange::default(),
                                            })),
                                            range: TextRange::default(),
                                        }));
                                    }

                                    for symbol in filtered_exports {
                                        // local_name.symbol = symbol
                                        self.deferred_imports.push(Stmt::Assign(StmtAssign {
                                            node_index: AtomicNodeIndex::dummy(),
                                            targets: vec![Expr::Attribute(ExprAttribute {
                                                node_index: AtomicNodeIndex::dummy(),
                                                value: Box::new(Expr::Name(ExprName {
                                                    node_index: AtomicNodeIndex::dummy(),
                                                    id: local_name.into(),
                                                    ctx: ExprContext::Load,
                                                    range: TextRange::default(),
                                                })),
                                                attr: Identifier::new(
                                                    &symbol,
                                                    TextRange::default(),
                                                ),
                                                ctx: ExprContext::Store,
                                                range: TextRange::default(),
                                            })],
                                            value: Box::new(Expr::Name(ExprName {
                                                node_index: AtomicNodeIndex::dummy(),
                                                // The symbol should use the renamed version if it
                                                // exists
                                                id: if let Some(renames) =
                                                    self.symbol_renames.get(&full_module_path)
                                                {
                                                    if let Some(renamed) = renames.get(&symbol) {
                                                        renamed.into()
                                                    } else {
                                                        symbol.into()
                                                    }
                                                } else {
                                                    symbol.clone().into()
                                                },
                                                ctx: ExprContext::Load,
                                                range: TextRange::default(),
                                            })),
                                            range: TextRange::default(),
                                        }));
                                    }
                                }
                            } else {
                                // For wrapper modules importing inlined modules, we need to create
                                // the namespace immediately since it's used in the module body
                                log::debug!(
                                    "  Creating immediate namespace for wrapper module import"
                                );

                                // Create: local_name = types.SimpleNamespace()
                                result_stmts.push(Stmt::Assign(StmtAssign {
                                    node_index: AtomicNodeIndex::dummy(),
                                    targets: vec![Expr::Name(ExprName {
                                        node_index: AtomicNodeIndex::dummy(),
                                        id: local_name.into(),
                                        ctx: ExprContext::Store,
                                        range: TextRange::default(),
                                    })],
                                    value: Box::new(Expr::Call(ruff_python_ast::ExprCall {
                                        node_index: AtomicNodeIndex::dummy(),
                                        func: Box::new(Expr::Attribute(ExprAttribute {
                                            node_index: AtomicNodeIndex::dummy(),
                                            value: Box::new(Expr::Name(ExprName {
                                                node_index: AtomicNodeIndex::dummy(),
                                                id: "types".into(),
                                                ctx: ExprContext::Load,
                                                range: TextRange::default(),
                                            })),
                                            attr: Identifier::new(
                                                "SimpleNamespace",
                                                TextRange::default(),
                                            ),
                                            ctx: ExprContext::Load,
                                            range: TextRange::default(),
                                        })),
                                        arguments: Arguments {
                                            node_index: AtomicNodeIndex::dummy(),
                                            args: Box::from([]),
                                            keywords: Box::from([]),
                                            range: TextRange::default(),
                                        },
                                        range: TextRange::default(),
                                    })),
                                    range: TextRange::default(),
                                }));

                                // Now add the exported symbols from the inlined module to the
                                // namespace
                                if let Some(exports) = self
                                    .bundler
                                    .module_exports
                                    .get(&full_module_path)
                                    .cloned()
                                    .flatten()
                                {
                                    // Filter exports to only include symbols that survived
                                    // tree-shaking
                                    let filtered_exports =
                                        self.bundler.filter_exports_by_tree_shaking(
                                            &exports,
                                            &full_module_path,
                                            self.bundler.tree_shaking_keep_symbols.as_ref(),
                                        );

                                    // Add __all__ attribute to the namespace with filtered exports
                                    // BUT ONLY if the original module had an explicit __all__
                                    if !filtered_exports.is_empty()
                                        && self
                                            .bundler
                                            .modules_with_explicit_all
                                            .contains(&full_module_path)
                                    {
                                        result_stmts.push(Stmt::Assign(StmtAssign {
                                            node_index: AtomicNodeIndex::dummy(),
                                            targets: vec![Expr::Attribute(ExprAttribute {
                                                node_index: AtomicNodeIndex::dummy(),
                                                value: Box::new(Expr::Name(ExprName {
                                                    node_index: AtomicNodeIndex::dummy(),
                                                    id: local_name.into(),
                                                    ctx: ExprContext::Load,
                                                    range: TextRange::default(),
                                                })),
                                                attr: Identifier::new(
                                                    "__all__",
                                                    TextRange::default(),
                                                ),
                                                ctx: ExprContext::Store,
                                                range: TextRange::default(),
                                            })],
                                            value: Box::new(Expr::List(ExprList {
                                                node_index: AtomicNodeIndex::dummy(),
                                                elts: filtered_exports
                                                    .iter()
                                                    .map(|name| {
                                                        Expr::StringLiteral(ExprStringLiteral {
                                                            node_index: AtomicNodeIndex::dummy(),
                                                            value: StringLiteralValue::single(
                                                                StringLiteral {
                                                                    node_index:
                                                                        AtomicNodeIndex::dummy(),
                                                                    value: name.as_str().into(),
                                                                    flags:
                                                                        StringLiteralFlags::empty(),
                                                                    range: TextRange::default(),
                                                                },
                                                            ),
                                                            range: TextRange::default(),
                                                        })
                                                    })
                                                    .collect(),
                                                ctx: ExprContext::Load,
                                                range: TextRange::default(),
                                            })),
                                            range: TextRange::default(),
                                        }));
                                    }

                                    for symbol in filtered_exports {
                                        // local_name.symbol = symbol
                                        result_stmts.push(Stmt::Assign(StmtAssign {
                                            node_index: AtomicNodeIndex::dummy(),
                                            targets: vec![Expr::Attribute(ExprAttribute {
                                                node_index: AtomicNodeIndex::dummy(),
                                                value: Box::new(Expr::Name(ExprName {
                                                    node_index: AtomicNodeIndex::dummy(),
                                                    id: local_name.into(),
                                                    ctx: ExprContext::Load,
                                                    range: TextRange::default(),
                                                })),
                                                attr: Identifier::new(
                                                    &symbol,
                                                    TextRange::default(),
                                                ),
                                                ctx: ExprContext::Store,
                                                range: TextRange::default(),
                                            })],
                                            value: Box::new(Expr::Name(ExprName {
                                                node_index: AtomicNodeIndex::dummy(),
                                                // The symbol should use the renamed version if it
                                                // exists
                                                id: if let Some(renames) =
                                                    self.symbol_renames.get(&full_module_path)
                                                {
                                                    if let Some(renamed) = renames.get(&symbol) {
                                                        renamed.into()
                                                    } else {
                                                        symbol.into()
                                                    }
                                                } else {
                                                    symbol.into()
                                                },
                                                ctx: ExprContext::Load,
                                                range: TextRange::default(),
                                            })),
                                            range: TextRange::default(),
                                        }));
                                    }
                                }
                            }

                            handled_any = true;
                        } else if !self.is_entry_module {
                            // This is a wrapper module importing an inlined module
                            // The wrapper will exist in sys.modules, so we can defer the import
                            log::debug!(
                                "  Deferring inlined submodule import in wrapper module: \
                                 {local_name} -> {full_module_path}"
                            );
                        } else {
                            // For entry module, handle differently
                            log::debug!(
                                "  Inlined submodule import in entry module: {local_name} -> \
                                 {full_module_path}"
                            );
                        }
                    }
                }
            }
        }

        if handled_any {
            // For deferred imports, we return empty to remove the original import
            if result_stmts.is_empty() {
                log::debug!("  Import handling deferred, returning empty");
                return vec![];
            } else {
                log::debug!(
                    "  Returning {} transformed statements for import",
                    result_stmts.len()
                );
                return result_stmts;
            }
        }

        if let Some(ref resolved) = resolved_module {
            // Check if this is an inlined module
            if self.bundler.inlined_modules.contains(resolved) {
                // Check if this is a circular module with pre-declarations
                if self.bundler.circular_modules.contains(resolved) {
                    log::debug!("  Module '{resolved}' is a circular module with pre-declarations");
                    // Return import assignments immediately - symbols are pre-declared
                    return self.bundler.handle_imports_from_inlined_module(
                        import_from,
                        resolved,
                        self.symbol_renames,
                    );
                } else {
                    log::debug!("  Module '{resolved}' is inlined, handling import assignments");
                    // For the entry module, we should not defer these imports
                    // because they need to be available when the entry module's code runs
                    let import_stmts = self.bundler.handle_imports_from_inlined_module(
                        import_from,
                        resolved,
                        self.symbol_renames,
                    );

                    // Only defer if we're not in the entry module
                    if !self.is_entry_module {
                        self.deferred_imports.extend(import_stmts);
                        // Return empty - these imports will be added after all modules are inlined
                        return vec![];
                    } else {
                        // For entry module, return the imports immediately
                        return import_stmts;
                    }
                }
            }

            // Check if this is a wrapper module (in module_registry)
            // This check must be after the inlined module check to avoid double-handling
            if self.bundler.module_registry.contains_key(resolved) {
                log::debug!("  Module '{resolved}' is a wrapper module");

                // For modules importing from wrapper modules, we may need to defer
                // the imports to ensure proper initialization order
                let current_module_is_inlined =
                    self.bundler.inlined_modules.contains(self.module_name);

                // Defer imports from wrapper modules when:
                // 1. Current module is inlined (to ensure wrapper is initialized first)
                // 2. Current module is entry module AND the check above determined all symbols are
                //    already deferred (handled by early return above)
                if !self.is_entry_module && current_module_is_inlined {
                    log::debug!(
                        "  Deferring wrapper module imports for module '{}' (inlined: {})",
                        self.module_name,
                        current_module_is_inlined
                    );

                    // Generate the standard transformation which includes init calls
                    let empty_renames = FxIndexMap::default();
                    let import_stmts = self
                        .bundler
                        .rewrite_import_in_stmt_multiple_with_full_context(
                            Stmt::ImportFrom(import_from.clone()),
                            self.module_name,
                            &empty_renames,
                            self.is_wrapper_init,
                        );

                    // Defer these imports until after all modules are inlined
                    self.deferred_imports.extend(import_stmts);
                    return vec![];
                }
                // For wrapper modules importing from other wrapper modules,
                // let it fall through to standard transformation
            }
        }

        // Otherwise, use standard transformation
        let empty_renames = FxIndexMap::default();
        self.bundler
            .rewrite_import_in_stmt_multiple_with_full_context(
                Stmt::ImportFrom(import_from.clone()),
                self.module_name,
                &empty_renames,
                self.is_wrapper_init,
            )
    }

    /// Transform an expression, rewriting module attribute access to direct references
    fn transform_expr(&self, expr: &mut Expr) {
        // First check if this is an attribute expression and collect the path
        let attribute_info = if matches!(expr, Expr::Attribute(_)) {
            Some(self.collect_attribute_path(expr))
        } else {
            None
        };

        match expr {
            Expr::Attribute(attr_expr) => {
                // Handle nested attribute access using the pre-collected path
                if let Some((base_name, attr_path)) = attribute_info {
                    if let Some(base) = base_name {
                        // In the entry module, check if this is accessing a namespace object
                        // created by a dotted import
                        if self.is_entry_module && attr_path.len() >= 2 {
                            // For "greetings.greeting.get_greeting()", we have:
                            // base: "greetings", attr_path: ["greeting", "get_greeting"]
                            // Check if "greetings.greeting" is a bundled module (created by "import
                            // greetings.greeting")
                            let namespace_path = format!("{}.{}", base, attr_path[0]);

                            if self.bundler.bundled_modules.contains(&namespace_path) {
                                // This is accessing a method/attribute on a namespace object
                                // created by a dotted import
                                // Don't transform it - let the namespace object handle it
                                log::debug!(
                                    "Not transforming {base}.{} - accessing namespace object \
                                     created by dotted import",
                                    attr_path.join(".")
                                );
                                // Don't recursively transform - the whole expression should remain
                                // as-is
                                return;
                            }
                        }

                        // Check if the base refers to an inlined module
                        if let Some(actual_module) = self.find_module_for_alias(&base)
                            && self.bundler.inlined_modules.contains(&actual_module)
                        {
                            // For a single attribute access (e.g., greetings.message or
                            // config.DEFAULT_NAME)
                            if attr_path.len() == 1 {
                                let attr_name = &attr_path[0];

                                // Check if we're accessing a submodule that's bundled as a wrapper
                                let potential_submodule = format!("{actual_module}.{attr_name}");
                                if self.bundler.bundled_modules.contains(&potential_submodule)
                                    && !self.bundler.inlined_modules.contains(&potential_submodule)
                                {
                                    // This is accessing a wrapper module through its parent
                                    // namespace Don't transform
                                    // it - let it remain as namespace access
                                    log::debug!(
                                        "Not transforming {base}.{attr_name} - it's a wrapper \
                                         module access"
                                    );
                                    // Fall through to recursive transformation
                                } else {
                                    // Check if this is accessing a namespace object (e.g.,
                                    // simple_module)
                                    // that was created by a namespace import
                                    if self
                                        .bundler
                                        .namespace_imported_modules
                                        .contains_key(&actual_module)
                                    {
                                        // This is accessing attributes on a namespace object
                                        // Don't transform - let it remain as namespace.attribute
                                        log::debug!(
                                            "Not transforming {base}.{attr_name} - accessing \
                                             namespace object attribute"
                                        );
                                        // Fall through to recursive transformation
                                    } else {
                                        // This is accessing a symbol from an inlined module
                                        // The symbol should be directly available in the bundled
                                        // scope
                                        log::debug!(
                                            "Transforming {base}.{attr_name} - {base} is alias \
                                             for inlined module {actual_module}"
                                        );

                                        // Check if this symbol was renamed during inlining
                                        let new_expr = if let Some(module_renames) =
                                            self.symbol_renames.get(&actual_module)
                                        {
                                            if let Some(renamed) = module_renames.get(attr_name) {
                                                // Use the renamed symbol
                                                let renamed_str = renamed.clone();
                                                log::debug!(
                                                    "Rewrote {base}.{attr_name} to {renamed_str} \
                                                     (renamed)"
                                                );
                                                Some(Expr::Name(ExprName {
                                                    node_index: AtomicNodeIndex::dummy(),
                                                    id: renamed_str.into(),
                                                    ctx: attr_expr.ctx,
                                                    range: attr_expr.range,
                                                }))
                                            } else {
                                                // Symbol exists but wasn't renamed, use the direct
                                                // name
                                                log::debug!(
                                                    "Rewrote {base}.{attr_name} to {attr_name} \
                                                     (not renamed)"
                                                );
                                                Some(Expr::Name(ExprName {
                                                    node_index: AtomicNodeIndex::dummy(),
                                                    id: attr_name.clone().into(),
                                                    ctx: attr_expr.ctx,
                                                    range: attr_expr.range,
                                                }))
                                            }
                                        } else {
                                            // No rename information available
                                            // Only transform if we're certain this symbol exists in
                                            // the inlined module
                                            // Otherwise, leave the attribute access unchanged
                                            if let Some(exports) =
                                                self.bundler.module_exports.get(&actual_module)
                                                && let Some(export_list) = exports
                                                && export_list.contains(&attr_name.to_string())
                                            {
                                                // This symbol is exported by the module, use direct
                                                // name
                                                log::debug!(
                                                    "Rewrote {base}.{attr_name} to {attr_name} \
                                                     (exported symbol)"
                                                );
                                                Some(Expr::Name(ExprName {
                                                    node_index: AtomicNodeIndex::dummy(),
                                                    id: attr_name.clone().into(),
                                                    ctx: attr_expr.ctx,
                                                    range: attr_expr.range,
                                                }))
                                            } else {
                                                // Not an exported symbol - don't transform
                                                log::debug!(
                                                    "Not transforming {base}.{attr_name} - not an \
                                                     exported symbol"
                                                );
                                                None
                                            }
                                        };

                                        if let Some(new_expr) = new_expr {
                                            *expr = new_expr;
                                            return;
                                        }
                                    }
                                }
                            }
                            // For nested attribute access (e.g., greetings.greeting.message)
                            // We need to handle the case where greetings.greeting is a submodule
                            else if attr_path.len() > 1 {
                                // Check if base.attr_path[0] forms a complete module name
                                let potential_module =
                                    format!("{}.{}", actual_module, attr_path[0]);

                                if self.bundler.inlined_modules.contains(&potential_module) {
                                    // This is accessing an attribute on a submodule
                                    // Build the remaining attribute path
                                    let remaining_attrs = &attr_path[1..];

                                    if remaining_attrs.len() == 1 {
                                        let final_attr = &remaining_attrs[0];

                                        // Check if this symbol was renamed during inlining
                                        if let Some(module_renames) =
                                            self.symbol_renames.get(&potential_module)
                                            && let Some(renamed) = module_renames.get(final_attr)
                                        {
                                            log::debug!(
                                                "Rewrote {base}.{}.{final_attr} to {renamed}",
                                                attr_path[0]
                                            );
                                            *expr = Expr::Name(ExprName {
                                                node_index: AtomicNodeIndex::dummy(),
                                                id: renamed.clone().into(),
                                                ctx: attr_expr.ctx,
                                                range: attr_expr.range,
                                            });
                                            return;
                                        }

                                        // No rename, use the original name with module prefix
                                        let direct_name = format!(
                                            "{final_attr}_{}",
                                            potential_module.cow_replace('.', "_").as_ref()
                                        );
                                        log::debug!(
                                            "Rewrote {base}.{}.{final_attr} to {direct_name}",
                                            attr_path[0]
                                        );
                                        *expr = Expr::Name(ExprName {
                                            node_index: AtomicNodeIndex::dummy(),
                                            id: direct_name.into(),
                                            ctx: attr_expr.ctx,
                                            range: attr_expr.range,
                                        });
                                        return;
                                    }
                                }
                            }
                        }
                    }

                    // If we didn't handle it above, recursively transform the value
                    self.transform_expr(&mut attr_expr.value);
                } // Close the if let Some((base_name, attr_path)) = attribute_info
            }
            Expr::Call(call_expr) => {
                self.transform_expr(&mut call_expr.func);
                for arg in &mut call_expr.arguments.args {
                    self.transform_expr(arg);
                }
                for keyword in &mut call_expr.arguments.keywords {
                    self.transform_expr(&mut keyword.value);
                }
            }
            Expr::BinOp(binop_expr) => {
                self.transform_expr(&mut binop_expr.left);
                self.transform_expr(&mut binop_expr.right);
            }
            Expr::UnaryOp(unaryop_expr) => {
                self.transform_expr(&mut unaryop_expr.operand);
            }
            Expr::BoolOp(boolop_expr) => {
                for value in &mut boolop_expr.values {
                    self.transform_expr(value);
                }
            }
            Expr::Compare(compare_expr) => {
                self.transform_expr(&mut compare_expr.left);
                for comparator in &mut compare_expr.comparators {
                    self.transform_expr(comparator);
                }
            }
            Expr::If(if_expr) => {
                self.transform_expr(&mut if_expr.test);
                self.transform_expr(&mut if_expr.body);
                self.transform_expr(&mut if_expr.orelse);
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    self.transform_expr(elem);
                }
            }
            Expr::Tuple(tuple_expr) => {
                for elem in &mut tuple_expr.elts {
                    self.transform_expr(elem);
                }
            }
            Expr::Dict(dict_expr) => {
                for item in &mut dict_expr.items {
                    if let Some(key) = &mut item.key {
                        self.transform_expr(key);
                    }
                    self.transform_expr(&mut item.value);
                }
            }
            Expr::Set(set_expr) => {
                for elem in &mut set_expr.elts {
                    self.transform_expr(elem);
                }
            }
            Expr::ListComp(listcomp_expr) => {
                self.transform_expr(&mut listcomp_expr.elt);
                for generator in &mut listcomp_expr.generators {
                    self.transform_expr(&mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        self.transform_expr(if_clause);
                    }
                }
            }
            Expr::DictComp(dictcomp_expr) => {
                self.transform_expr(&mut dictcomp_expr.key);
                self.transform_expr(&mut dictcomp_expr.value);
                for generator in &mut dictcomp_expr.generators {
                    self.transform_expr(&mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        self.transform_expr(if_clause);
                    }
                }
            }
            Expr::SetComp(setcomp_expr) => {
                self.transform_expr(&mut setcomp_expr.elt);
                for generator in &mut setcomp_expr.generators {
                    self.transform_expr(&mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        self.transform_expr(if_clause);
                    }
                }
            }
            Expr::Generator(genexp_expr) => {
                self.transform_expr(&mut genexp_expr.elt);
                for generator in &mut genexp_expr.generators {
                    self.transform_expr(&mut generator.iter);
                    for if_clause in &mut generator.ifs {
                        self.transform_expr(if_clause);
                    }
                }
            }
            Expr::Subscript(subscript_expr) => {
                self.transform_expr(&mut subscript_expr.value);
                self.transform_expr(&mut subscript_expr.slice);
            }
            Expr::Slice(slice_expr) => {
                if let Some(lower) = &mut slice_expr.lower {
                    self.transform_expr(lower);
                }
                if let Some(upper) = &mut slice_expr.upper {
                    self.transform_expr(upper);
                }
                if let Some(step) = &mut slice_expr.step {
                    self.transform_expr(step);
                }
            }
            Expr::Lambda(lambda_expr) => {
                self.transform_expr(&mut lambda_expr.body);
            }
            Expr::Yield(yield_expr) => {
                if let Some(value) = &mut yield_expr.value {
                    self.transform_expr(value);
                }
            }
            Expr::YieldFrom(yieldfrom_expr) => {
                self.transform_expr(&mut yieldfrom_expr.value);
            }
            Expr::Await(await_expr) => {
                self.transform_expr(&mut await_expr.value);
            }
            Expr::Starred(starred_expr) => {
                self.transform_expr(&mut starred_expr.value);
            }
            Expr::FString(fstring_expr) => {
                // Transform expressions within the f-string
                let fstring_range = fstring_expr.range;
                let mut transformed_elements = Vec::new();
                let mut any_transformed = false;

                for element in fstring_expr.value.elements() {
                    match element {
                        InterpolatedStringElement::Literal(lit_elem) => {
                            transformed_elements
                                .push(InterpolatedStringElement::Literal(lit_elem.clone()));
                        }
                        InterpolatedStringElement::Interpolation(expr_elem) => {
                            let mut new_expr = expr_elem.expression.clone();
                            self.transform_expr(&mut new_expr);

                            if !matches!(&new_expr, other if other == &expr_elem.expression) {
                                any_transformed = true;
                            }

                            let new_element = InterpolatedElement {
                                node_index: AtomicNodeIndex::dummy(),
                                expression: new_expr,
                                debug_text: expr_elem.debug_text.clone(),
                                conversion: expr_elem.conversion,
                                format_spec: expr_elem.format_spec.clone(),
                                range: expr_elem.range,
                            };
                            transformed_elements
                                .push(InterpolatedStringElement::Interpolation(new_element));
                        }
                    }
                }

                if any_transformed {
                    let new_fstring = FString {
                        node_index: AtomicNodeIndex::dummy(),
                        elements: InterpolatedStringElements::from(transformed_elements),
                        range: TextRange::default(),
                        flags: FStringFlags::empty(),
                    };

                    let new_value = FStringValue::single(new_fstring);

                    *expr = Expr::FString(ExprFString {
                        node_index: AtomicNodeIndex::dummy(),
                        value: new_value,
                        range: fstring_range,
                    });
                }
            }
            // Name, Constants, etc. don't need transformation
            _ => {}
        }
    }

    /// Collect the full dotted attribute path from a potentially nested attribute expression
    /// Returns (base_name, [attr1, attr2, ...])
    /// For example: greetings.greeting.message returns (Some("greetings"), ["greeting", "message"])
    fn collect_attribute_path(&self, expr: &Expr) -> (Option<String>, Vec<String>) {
        let mut attrs = Vec::new();
        let mut current = expr;

        loop {
            match current {
                Expr::Attribute(attr) => {
                    attrs.push(attr.attr.as_str().to_string());
                    current = &attr.value;
                }
                Expr::Name(name) => {
                    attrs.reverse();
                    return (Some(name.id.as_str().to_string()), attrs);
                }
                _ => {
                    attrs.reverse();
                    return (None, attrs);
                }
            }
        }
    }

    /// Find the actual module name for a given alias
    fn find_module_for_alias(&self, alias: &str) -> Option<String> {
        // Don't treat local variables as module aliases
        if self.local_variables.contains(alias) {
            return None;
        }

        // First check our tracked import aliases
        if let Some(module_name) = self.import_aliases.get(alias) {
            return Some(module_name.clone());
        }

        // Then check if the alias directly matches a module name
        // But not in the entry module - in the entry module, direct module names
        // are namespace objects, not aliases
        if !self.is_entry_module && self.bundler.inlined_modules.contains(alias) {
            Some(alias.to_string())
        } else {
            // Check common patterns like "import utils.helpers as helper_utils"
            // where alias is "helper_utils" and module is "utils.helpers"
            for module in &self.bundler.inlined_modules {
                if let Some(last_part) = module.split('.').next_back()
                    && (alias == format!("{last_part}_utils") || alias == format!("{last_part}s"))
                {
                    return Some(module.clone());
                }
            }
            None
        }
    }
}

/// Hybrid static bundler that uses sys.modules and hash-based naming
/// This approach avoids forward reference issues while maintaining Python module semantics
pub struct HybridStaticBundler {
    /// Map from original module name to synthetic module name
    module_registry: FxIndexMap<String, String>,
    /// Map from synthetic module name to init function name
    init_functions: FxIndexMap<String, String>,
    /// Collected future imports
    future_imports: FxIndexSet<String>,
    /// Collected stdlib imports that are safe to hoist
    /// Maps module name to set of imported names for deduplication
    stdlib_import_from_map: FxIndexMap<String, FxIndexSet<String>>,
    /// Regular import statements (import module)
    stdlib_import_statements: Vec<Stmt>,
    /// Track which modules have been bundled
    bundled_modules: FxIndexSet<String>,
    /// Modules that were inlined (not wrapper modules)
    inlined_modules: FxIndexSet<String>,
    /// Entry point path for calculating relative paths
    entry_path: Option<String>,
    /// Module export information (for __all__ handling)
    module_exports: FxIndexMap<String, Option<Vec<String>>>,
    /// Lifted global declarations to add at module top level
    lifted_global_declarations: Vec<Stmt>,
    /// Modules that are imported as namespaces (e.g., from package import module)
    /// Maps module name to set of importing modules
    namespace_imported_modules: FxIndexMap<String, FxIndexSet<String>>,
    /// Modules that are part of circular dependencies
    circular_modules: FxIndexSet<String>,
    /// Pre-declared symbols for circular modules (module -> symbol -> renamed)
    circular_predeclarations: FxIndexMap<String, FxIndexMap<String, String>>,
    /// Symbol dependency graph for circular modules
    symbol_dep_graph: SymbolDependencyGraph,
    /// Module ASTs for resolving re-exports
    module_asts: Option<Vec<(String, ModModule, PathBuf, String)>>,
    /// Global registry of deferred imports to prevent duplication
    /// Maps (module_name, symbol_name) to the source module that deferred it
    global_deferred_imports: FxIndexMap<(String, String), String>,
    /// Track all namespaces that need to be created before module initialization
    /// This ensures parent namespaces exist before any submodule assignments
    required_namespaces: FxIndexSet<String>,
    /// Runtime tracking of all created namespaces to prevent duplicates
    /// This includes both pre-identified and dynamically created namespaces
    created_namespaces: FxIndexSet<String>,
    /// Modules that have explicit __all__ defined
    modules_with_explicit_all: FxIndexSet<String>,
    /// Transformation context for tracking node mappings
    transformation_context: TransformationContext,
    /// Module/symbol pairs that should be kept after tree shaking
    tree_shaking_keep_symbols: Option<indexmap::IndexSet<(String, String)>>,
}

impl HybridStaticBundler {
    /// Check if a renamed symbol exists after tree-shaking
    fn renamed_symbol_exists(
        &self,
        renamed_symbol: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> bool {
        // If not using tree-shaking, all symbols exist
        let Some(ref kept_symbols) = self.tree_shaking_keep_symbols else {
            return true;
        };

        // Check all modules to see if any have this renamed symbol
        for (module, renames) in symbol_renames {
            for (orig_name, renamed) in renames {
                if renamed == renamed_symbol
                    && kept_symbols.contains(&(module.clone(), orig_name.clone()))
                {
                    return true;
                }
            }
        }

        false
    }
}

impl Default for HybridStaticBundler {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl HybridStaticBundler {
    pub fn new() -> Self {
        Self {
            module_registry: FxIndexMap::default(),
            init_functions: FxIndexMap::default(),
            future_imports: FxIndexSet::default(),
            stdlib_import_from_map: FxIndexMap::default(),
            stdlib_import_statements: Vec::new(),
            bundled_modules: FxIndexSet::default(),
            inlined_modules: FxIndexSet::default(),
            entry_path: None,
            module_exports: FxIndexMap::default(),
            lifted_global_declarations: Vec::new(),
            namespace_imported_modules: FxIndexMap::default(),
            circular_modules: FxIndexSet::default(),
            circular_predeclarations: FxIndexMap::default(),
            symbol_dep_graph: SymbolDependencyGraph::default(),
            module_asts: None,
            global_deferred_imports: FxIndexMap::default(),
            required_namespaces: FxIndexSet::default(),
            created_namespaces: FxIndexSet::default(),
            modules_with_explicit_all: FxIndexSet::default(),
            transformation_context: TransformationContext::new(),
            tree_shaking_keep_symbols: None,
        }
    }

    /// Create a new node with a proper index from the transformation context
    fn create_node_index(&mut self) -> AtomicNodeIndex {
        self.transformation_context.create_node_index()
    }

    /// Create a new node and record it as a transformation
    fn create_transformed_node(&mut self, reason: String) -> AtomicNodeIndex {
        self.transformation_context.create_new_node(reason)
    }

    /// Post-process AST to assign proper node indices to any nodes created with dummy indices
    fn assign_node_indices_to_ast(&mut self, module: &mut ModModule) {
        use ruff_python_ast::visitor::source_order::SourceOrderVisitor;

        struct NodeIndexAssigner<'a> {
            bundler: &'a mut HybridStaticBundler,
        }

        impl<'a> SourceOrderVisitor<'_> for NodeIndexAssigner<'a> {
            fn visit_stmt(&mut self, stmt: &Stmt) {
                // Check if this node has a dummy index (value 0)
                let node_index = match stmt {
                    Stmt::FunctionDef(s) => &s.node_index,
                    Stmt::ClassDef(s) => &s.node_index,
                    Stmt::Import(s) => &s.node_index,
                    Stmt::ImportFrom(s) => &s.node_index,
                    Stmt::Assign(s) => &s.node_index,
                    Stmt::Return(s) => &s.node_index,
                    Stmt::Delete(s) => &s.node_index,
                    Stmt::AugAssign(s) => &s.node_index,
                    Stmt::AnnAssign(s) => &s.node_index,
                    Stmt::TypeAlias(s) => &s.node_index,
                    Stmt::For(s) => &s.node_index,
                    Stmt::While(s) => &s.node_index,
                    Stmt::If(s) => &s.node_index,
                    Stmt::With(s) => &s.node_index,
                    Stmt::Match(s) => &s.node_index,
                    Stmt::Raise(s) => &s.node_index,
                    Stmt::Try(s) => &s.node_index,
                    Stmt::Assert(s) => &s.node_index,
                    Stmt::Global(s) => &s.node_index,
                    Stmt::Nonlocal(s) => &s.node_index,
                    Stmt::Expr(s) => &s.node_index,
                    Stmt::Pass(s) => &s.node_index,
                    Stmt::Break(s) => &s.node_index,
                    Stmt::Continue(s) => &s.node_index,
                    Stmt::IpyEscapeCommand(s) => &s.node_index,
                };

                // If it's a dummy index (0), assign a new one
                if node_index.load().as_usize() == 0 {
                    let new_index = self.bundler.create_node_index();
                    node_index.set(new_index.load().as_usize() as u32);
                }

                // Continue walking
                ruff_python_ast::visitor::source_order::walk_stmt(self, stmt);
            }

            fn visit_expr(&mut self, expr: &Expr) {
                // Similar logic for expressions
                let node_index = match expr {
                    Expr::BoolOp(e) => &e.node_index,
                    Expr::BinOp(e) => &e.node_index,
                    Expr::UnaryOp(e) => &e.node_index,
                    Expr::Lambda(e) => &e.node_index,
                    Expr::If(e) => &e.node_index,
                    Expr::Dict(e) => &e.node_index,
                    Expr::Set(e) => &e.node_index,
                    Expr::ListComp(e) => &e.node_index,
                    Expr::SetComp(e) => &e.node_index,
                    Expr::DictComp(e) => &e.node_index,
                    Expr::Generator(e) => &e.node_index,
                    Expr::Await(e) => &e.node_index,
                    Expr::Yield(e) => &e.node_index,
                    Expr::YieldFrom(e) => &e.node_index,
                    Expr::Compare(e) => &e.node_index,
                    Expr::Call(e) => &e.node_index,
                    Expr::NumberLiteral(e) => &e.node_index,
                    Expr::StringLiteral(e) => &e.node_index,
                    Expr::FString(e) => &e.node_index,
                    Expr::BytesLiteral(e) => &e.node_index,
                    Expr::BooleanLiteral(e) => &e.node_index,
                    Expr::NoneLiteral(e) => &e.node_index,
                    Expr::EllipsisLiteral(e) => &e.node_index,
                    Expr::Attribute(e) => &e.node_index,
                    Expr::Subscript(e) => &e.node_index,
                    Expr::Starred(e) => &e.node_index,
                    Expr::Name(e) => &e.node_index,
                    Expr::List(e) => &e.node_index,
                    Expr::Tuple(e) => &e.node_index,
                    Expr::Slice(e) => &e.node_index,
                    Expr::IpyEscapeCommand(e) => &e.node_index,
                    Expr::Named(e) => &e.node_index,
                    Expr::TString(e) => &e.node_index,
                };

                if node_index.load().as_usize() == 0 {
                    let new_index = self.bundler.create_node_index();
                    node_index.set(new_index.load().as_usize() as u32);
                }

                ruff_python_ast::visitor::source_order::walk_expr(self, expr);
            }
        }

        let mut assigner = NodeIndexAssigner { bundler: self };
        assigner.visit_mod(&ruff_python_ast::Mod::Module(module.clone()));
    }

    /// Check if an expression is accessing a module namespace
    fn is_module_namespace_access(&self, expr: &Expr) -> bool {
        match expr {
            // Direct module name (e.g., schemas)
            Expr::Name(name) => {
                let module_name = name.id.as_str();
                // Check if it's a known module or namespace
                self.bundled_modules.contains(module_name)
                    || self.namespace_imported_modules.contains_key(module_name)
                    || self.created_namespaces.contains(module_name)
            }
            // Nested module access (e.g., schemas.user)
            Expr::Attribute(_attr) => {
                // Check if this represents a module path
                let module_path = Self::expr_to_module_path(expr);
                if let Some(path) = module_path {
                    self.bundled_modules.contains(&path)
                        || self.namespace_imported_modules.contains_key(&path)
                        || self.created_namespaces.contains(&path)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Convert an expression to a module path if it represents one
    fn expr_to_module_path(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(name) => Some(name.id.to_string()),
            Expr::Attribute(attr) => Self::expr_to_module_path(&attr.value)
                .map(|base_path| format!("{}.{}", base_path, attr.attr.as_str())),
            _ => None,
        }
    }

    /// Compare two expressions for equality
    fn expr_equals(expr1: &Expr, expr2: &Expr) -> bool {
        match (expr1, expr2) {
            (Expr::Name(n1), Expr::Name(n2)) => n1.id == n2.id,
            (Expr::Attribute(a1), Expr::Attribute(a2)) => {
                a1.attr == a2.attr && Self::expr_equals(&a1.value, &a2.value)
            }
            _ => false,
        }
    }

    /// Check if an ImportFrom statement is a duplicate of any existing import in the body
    fn is_duplicate_import_from(
        &self,
        import_from: &StmtImportFrom,
        existing_body: &[Stmt],
    ) -> bool {
        if let Some(ref module) = import_from.module {
            let module_name = module.as_str();
            // For third-party imports, check if they're already in the body
            if !self.is_safe_stdlib_module(module_name)
                && !self.is_bundled_module_or_package(module_name)
            {
                return existing_body.iter().any(|existing| {
                    if let Stmt::ImportFrom(existing_import) = existing {
                        existing_import.module.as_ref().map(|m| m.as_str()) == Some(module_name)
                            && Self::import_names_match(&import_from.names, &existing_import.names)
                    } else {
                        false
                    }
                });
            }
        }
        false
    }

    /// Check if an Import statement is a duplicate of any existing import in the body
    fn is_duplicate_import(&self, import_stmt: &StmtImport, existing_body: &[Stmt]) -> bool {
        import_stmt.names.iter().any(|alias| {
            let module_name = alias.name.as_str();
            // For third-party imports, check if they're already in the body
            if !self.is_safe_stdlib_module(module_name)
                && !self.is_bundled_module_or_package(module_name)
            {
                existing_body.iter().any(|existing| {
                    if let Stmt::Import(existing_import) = existing {
                        existing_import.names.iter().any(|existing_alias| {
                            existing_alias.name == alias.name
                                && existing_alias.asname == alias.asname
                        })
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        })
    }

    /// Filter module exports based on tree-shaking results
    /// Returns only the symbols that survived tree-shaking, or all exports if tree-shaking is
    /// disabled
    fn filter_exports_by_tree_shaking(
        &self,
        exports: &[String],
        module_path: &str,
        kept_symbols: Option<&indexmap::IndexSet<(String, String)>>,
    ) -> Vec<String> {
        if let Some(kept_symbols) = kept_symbols {
            exports
                .iter()
                .filter(|symbol| {
                    // Check if this symbol is kept in this module
                    kept_symbols.contains(&(module_path.to_string(), (*symbol).clone()))
                })
                .cloned()
                .collect()
        } else {
            // No tree-shaking, include all exports
            exports.to_vec()
        }
    }

    /// Filter module exports based on tree-shaking results with debug logging
    /// Returns references to the symbols that survived tree-shaking
    fn filter_exports_by_tree_shaking_with_logging<'a>(
        &self,
        exports: &'a [String],
        module_name: &str,
        kept_symbols: Option<&indexmap::IndexSet<(String, String)>>,
    ) -> Vec<&'a String> {
        if let Some(kept_symbols) = kept_symbols {
            let result: Vec<&String> = exports
                .iter()
                .filter(|symbol| {
                    // Check if this symbol is kept in this module
                    let is_kept =
                        kept_symbols.contains(&(module_name.to_string(), (*symbol).clone()));
                    if !is_kept {
                        log::debug!(
                            "Filtering out symbol '{symbol}' from __all__ of module \
                             '{module_name}' - removed by tree-shaking"
                        );
                    } else {
                        log::debug!(
                            "Keeping symbol '{symbol}' in __all__ of module '{module_name}' - \
                             survived tree-shaking"
                        );
                    }
                    is_kept
                })
                .collect();
            log::debug!(
                "Module '{}' __all__ filtering: {} symbols -> {} symbols",
                module_name,
                exports.len(),
                result.len()
            );
            result
        } else {
            // No tree-shaking, include all exports
            exports.iter().collect()
        }
    }

    /// Pre-scan all modules to identify required namespaces
    /// This ensures parent namespaces are created before any module initialization
    fn identify_required_namespaces(&mut self, modules: &[(String, ModModule, PathBuf, String)]) {
        debug!(
            "Identifying required namespaces from {} modules",
            modules.len()
        );

        // Clear any existing namespaces
        self.required_namespaces.clear();

        // First, collect all module names to check if parent modules exist
        let all_module_names: FxIndexSet<String> =
            modules.iter().map(|(name, _, _, _)| name.clone()).collect();

        // Scan all modules to find dotted module names
        for (module_name, _, _, _) in modules {
            if !module_name.contains('.') {
                continue;
            }

            // Split the module name and identify all parent namespaces
            let parts: Vec<&str> = module_name.split('.').collect();

            // Add all parent namespace levels
            for i in 1..parts.len() {
                let namespace = parts[..i].join(".");

                // We need to create a namespace for ALL parent namespaces, regardless of whether
                // they are wrapped modules or not. This is because child modules need to be
                // assigned as attributes on their parent namespaces.
                debug!("Identified required namespace: {namespace}");
                self.required_namespaces.insert(namespace);
            }
        }

        // IMPORTANT: Also add wrapper modules that have submodules as required namespaces
        // This ensures that parent modules like 'models' and 'services' exist as namespaces
        // before we try to assign their submodules
        for module_name in &all_module_names {
            // Check if this module has any submodules
            let has_submodules = all_module_names
                .iter()
                .any(|m| m != module_name && m.starts_with(&format!("{module_name}.")));

            if has_submodules && self.module_registry.contains_key(module_name) {
                // This is a wrapper module with submodules - it needs a namespace
                debug!(
                    "Identified wrapper module with submodules as required namespace: \
                     {module_name}"
                );
                self.required_namespaces.insert(module_name.clone());
            }
        }

        debug!(
            "Total required namespaces: {}",
            self.required_namespaces.len()
        );
    }

    /// Create all required namespace statements before module initialization
    fn create_namespace_statements(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();

        // Sort namespaces for deterministic output
        let mut sorted_namespaces: Vec<String> = self.required_namespaces.iter().cloned().collect();
        sorted_namespaces.sort();

        for namespace in sorted_namespaces {
            debug!("Creating namespace statement for: {namespace}");

            // Use ensure_namespace_exists to handle both simple and dotted namespaces
            let namespace_stmts = self.ensure_namespace_exists(&namespace);
            statements.extend(namespace_stmts);
        }

        statements
    }

    /// Ensure a namespace exists, creating it and any parent namespaces if needed
    /// Returns statements to create any missing namespaces
    fn ensure_namespace_exists(&mut self, namespace_path: &str) -> Vec<Stmt> {
        let mut statements = Vec::new();

        // For dotted names like "models.user", we need to ensure "models" exists first
        if namespace_path.contains('.') {
            let parts: Vec<&str> = namespace_path.split('.').collect();

            // Create all parent namespaces
            for i in 1..=parts.len() {
                let namespace = parts[..i].join(".");

                if !self.created_namespaces.contains(&namespace) {
                    debug!("Creating namespace dynamically: {namespace}");

                    if i == 1 {
                        // Top-level namespace
                        statements.extend(self.create_namespace_module(&namespace));
                    } else {
                        // Nested namespace - create as attribute
                        let parent = parts[..i - 1].join(".");
                        let attr = parts[i - 1];
                        statements.push(self.create_namespace_attribute(&parent, attr));
                    }

                    self.created_namespaces.insert(namespace);
                }
            }
        } else {
            // Simple namespace without dots
            if !self.created_namespaces.contains(namespace_path) {
                debug!("Creating simple namespace dynamically: {namespace_path}");
                statements.extend(self.create_namespace_module(namespace_path));
                self.created_namespaces.insert(namespace_path.to_string());
            }
        }

        statements
    }

    /// Create a namespace as an attribute of a parent namespace
    fn create_namespace_attribute(&self, parent: &str, attr: &str) -> Stmt {
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: parent.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(attr, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Find matching module name from the modules list (for namespace imports)
    fn find_matching_module_name_namespace(
        modules: &[(String, ModModule, PathBuf, String)],
        full_module_path: &str,
    ) -> String {
        modules
            .iter()
            .find(|(name, _, _, _)| name == full_module_path || name.ends_with(full_module_path))
            .map(|(name, _, _, _)| name.clone())
            .unwrap_or_else(|| full_module_path.to_string())
    }

    /// Check if a module AST has side effects (executable code at top level)
    /// Returns true if the module has side effects beyond simple definitions
    pub fn has_side_effects(ast: &ModModule) -> bool {
        // Use static method to avoid allocation in this hot path
        SideEffectDetector::check_module(ast)
    }

    /// Check if a module is part of any circular dependency
    fn is_in_circular_dependency(
        module_name: &str,
        circular_dep_analysis: Option<&crate::cribo_graph::CircularDependencyAnalysis>,
    ) -> bool {
        if let Some(analysis) = circular_dep_analysis {
            // Check if module is in any resolvable cycle
            for cycle in &analysis.resolvable_cycles {
                if cycle.modules.contains(&module_name.to_string()) {
                    return true;
                }
            }
            // Check if module is in any unresolvable cycle
            for cycle in &analysis.unresolvable_cycles {
                if cycle.modules.contains(&module_name.to_string()) {
                    return true;
                }
            }
        }
        false
    }

    /// Build symbol-level dependency graph for circular modules
    fn build_symbol_dependency_graph(
        &mut self,
        modules: &[(String, ModModule, PathBuf, String)],
        graph: &crate::cribo_graph::CriboGraph,
        _semantic_ctx: &SemanticContext,
    ) {
        // For each module in a circular dependency, analyze its symbols
        for (module_name, _ast, _path, _source) in modules {
            if !self.circular_modules.contains(module_name) {
                continue;
            }

            log::debug!("Building symbol dependency graph for circular module: {module_name}");

            // Get the module from the graph
            if let Some(module_dep_graph) = graph.get_module_by_name(module_name) {
                // For each item in the module
                for item_data in module_dep_graph.items.values() {
                    match &item_data.item_type {
                        crate::cribo_graph::ItemType::FunctionDef { name } => {
                            self.analyze_function_dependencies(
                                module_name,
                                name,
                                item_data,
                                module_dep_graph,
                                graph,
                            );
                        }
                        crate::cribo_graph::ItemType::ClassDef { name } => {
                            self.analyze_class_dependencies(
                                module_name,
                                name,
                                item_data,
                                module_dep_graph,
                                graph,
                            );
                        }
                        crate::cribo_graph::ItemType::Assignment { targets } => {
                            for target in targets {
                                self.analyze_assignment_dependencies(
                                    module_name,
                                    target,
                                    item_data,
                                    module_dep_graph,
                                    graph,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Analyze dependencies for a function definition
    fn analyze_function_dependencies(
        &mut self,
        module_name: &str,
        function_name: &str,
        item_data: &crate::cribo_graph::ItemData,
        _module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
        graph: &crate::cribo_graph::CriboGraph,
    ) {
        let key = (module_name.to_string(), function_name.to_string());
        let mut all_dependencies = Vec::new();
        let mut module_level_deps = Vec::new();

        // Track what this function reads at module level (e.g., decorators, default args)
        for var in &item_data.read_vars {
            // Check if this variable is from another circular module
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && self.circular_modules.contains(&dep_module)
                && dep_module != module_name
            {
                let dep = (dep_module, var.clone());
                all_dependencies.push(dep.clone());
                module_level_deps.push(dep); // Module-level reads need pre-declaration
            }
        }

        // Also check eventual reads (inside the function body) - these don't need pre-declaration
        for var in &item_data.eventual_read_vars {
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && self.circular_modules.contains(&dep_module)
                && dep_module != module_name
            {
                all_dependencies.push((dep_module, var.clone()));
                // Note: NOT added to module_level_deps since these are lazy
            }
        }

        self.symbol_dep_graph
            .dependencies
            .insert(key.clone(), all_dependencies);
        self.symbol_dep_graph
            .module_level_dependencies
            .insert(key.clone(), module_level_deps);
        self.symbol_dep_graph.symbol_definitions.insert(
            key,
            SymbolDefinition {
                is_function: true,
                is_class: false,
                is_assignment: false,
                depends_on: vec![],
            },
        );
    }

    /// Analyze dependencies for a class definition
    fn analyze_class_dependencies(
        &mut self,
        module_name: &str,
        class_name: &str,
        item_data: &crate::cribo_graph::ItemData,
        _module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
        graph: &crate::cribo_graph::CriboGraph,
    ) {
        let key = (module_name.to_string(), class_name.to_string());
        let mut all_dependencies = Vec::new();
        let mut module_level_deps = Vec::new();

        // For classes, check both immediate reads (base classes) and eventual reads (methods)
        for var in &item_data.read_vars {
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && self.circular_modules.contains(&dep_module)
                && dep_module != module_name
            {
                let dep = (dep_module, var.clone());
                all_dependencies.push(dep.clone());
                module_level_deps.push(dep); // Base classes need to exist at definition time
            }
        }

        self.symbol_dep_graph
            .dependencies
            .insert(key.clone(), all_dependencies);
        self.symbol_dep_graph
            .module_level_dependencies
            .insert(key.clone(), module_level_deps);
        self.symbol_dep_graph.symbol_definitions.insert(
            key,
            SymbolDefinition {
                is_function: false,
                is_class: true,
                is_assignment: false,
                depends_on: vec![],
            },
        );
    }

    /// Analyze dependencies for an assignment
    fn analyze_assignment_dependencies(
        &mut self,
        module_name: &str,
        var_name: &str,
        item_data: &crate::cribo_graph::ItemData,
        _module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
        graph: &crate::cribo_graph::CriboGraph,
    ) {
        let key = (module_name.to_string(), var_name.to_string());
        let mut dependencies = Vec::new();

        // Assignments are evaluated immediately - all dependencies are module-level
        for var in &item_data.read_vars {
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && self.circular_modules.contains(&dep_module)
                && dep_module != module_name
            {
                dependencies.push((dep_module, var.clone()));
            }
        }

        self.symbol_dep_graph
            .dependencies
            .insert(key.clone(), dependencies.clone());
        self.symbol_dep_graph
            .module_level_dependencies
            .insert(key.clone(), dependencies); // All assignment deps are module-level
        self.symbol_dep_graph.symbol_definitions.insert(
            key,
            SymbolDefinition {
                is_function: false,
                is_class: false,
                is_assignment: true,
                depends_on: vec![],
            },
        );
    }

    /// Find which module defines a symbol
    fn find_symbol_module(
        &self,
        symbol: &str,
        current_module: &str,
        graph: &crate::cribo_graph::CriboGraph,
    ) -> Option<String> {
        // First check if it's defined in the current module
        if let Some(module_dep_graph) = graph.get_module_by_name(current_module) {
            for item_data in module_dep_graph.items.values() {
                match &item_data.item_type {
                    crate::cribo_graph::ItemType::FunctionDef { name } if name == symbol => {
                        return Some(current_module.to_string());
                    }
                    crate::cribo_graph::ItemType::ClassDef { name } if name == symbol => {
                        return Some(current_module.to_string());
                    }
                    crate::cribo_graph::ItemType::Assignment { targets } => {
                        if targets.contains(&symbol.to_string()) {
                            return Some(current_module.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        // Check other circular modules
        for module_name in &self.circular_modules {
            if module_name == current_module {
                continue;
            }

            if let Some(module_dep_graph) = graph.get_module_by_name(module_name) {
                for item_data in module_dep_graph.items.values() {
                    match &item_data.item_type {
                        crate::cribo_graph::ItemType::FunctionDef { name } if name == symbol => {
                            return Some(module_name.clone());
                        }
                        crate::cribo_graph::ItemType::ClassDef { name } if name == symbol => {
                            return Some(module_name.clone());
                        }
                        crate::cribo_graph::ItemType::Assignment { targets } => {
                            if targets.contains(&symbol.to_string()) {
                                return Some(module_name.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        None
    }

    /// Sort wrapped modules by their dependencies to ensure correct initialization order
    fn sort_wrapped_modules_by_dependencies(
        &self,
        wrapped_modules: &[String],
        all_modules: &[(String, PathBuf, Vec<String>)],
    ) -> Vec<String> {
        // Build a dependency map for wrapped modules only
        let mut deps_map: FxIndexMap<String, Vec<String>> = FxIndexMap::default();

        for module_name in wrapped_modules {
            deps_map.insert(module_name.clone(), Vec::new());

            // Add child modules as dependencies to ensure they're initialized first
            // For example, "models" depends on "models.base" and "models.user"
            // because models/__init__.py might import from them
            for other_module in wrapped_modules {
                if other_module != module_name
                    && other_module.starts_with(module_name)
                    && other_module[module_name.len()..].starts_with('.')
                {
                    // other_module is a child of module_name
                    debug!("  - {module_name} depends on child module {other_module}");
                    if let Some(module_deps) = deps_map.get_mut(module_name)
                        && !module_deps.contains(other_module)
                    {
                        module_deps.push(other_module.clone());
                    }
                }
            }

            // Find this module's dependencies from all_modules
            if let Some((_, _, deps)) = all_modules.iter().find(|(name, _, _)| name == module_name)
            {
                debug!("Module {module_name} has dependencies: {deps:?}");
                for dep in deps {
                    // Check if this dependency or any of its submodules are wrapped
                    for wrapped in wrapped_modules {
                        // Check exact match or if wrapped module is a submodule of dep
                        if wrapped == dep
                            || (wrapped.starts_with(dep) && wrapped[dep.len()..].starts_with('.'))
                        {
                            debug!("  - {module_name} depends on wrapped module {wrapped}");
                            if let Some(module_deps) = deps_map.get_mut(module_name)
                                && !module_deps.contains(wrapped)
                            {
                                module_deps.push(wrapped.clone());
                            }
                        }
                    }
                }
            }
        }

        debug!("Dependency map for wrapped modules: {deps_map:?}");

        // Perform a simple topological sort on wrapped modules
        let mut sorted = Vec::new();
        let mut visited = FxIndexSet::default();
        let mut visiting = FxIndexSet::default();

        fn visit(
            module: &str,
            deps_map: &FxIndexMap<String, Vec<String>>,
            visited: &mut FxIndexSet<String>,
            visiting: &mut FxIndexSet<String>,
            sorted: &mut Vec<String>,
        ) -> bool {
            if visited.contains(module) {
                return true;
            }
            if visiting.contains(module) {
                // Circular dependency among wrapped modules
                return false;
            }

            visiting.insert(module.to_string());

            if let Some(deps) = deps_map.get(module) {
                for dep in deps {
                    if !visit(dep, deps_map, visited, visiting, sorted) {
                        return false;
                    }
                }
            }

            visiting.shift_remove(module);
            visited.insert(module.to_string());
            sorted.push(module.to_string());
            true
        }

        for module in wrapped_modules {
            visit(module, &deps_map, &mut visited, &mut visiting, &mut sorted);
        }

        sorted
    }

    /// Generate synthetic module name using content hash
    fn get_synthetic_module_name(&self, module_name: &str, content_hash: &str) -> String {
        let module_name_escaped = module_name
            .chars()
            .map(|c| if c == '.' { '_' } else { c })
            .collect::<String>();
        // Use first 6 characters of content hash for readability
        let short_hash = &content_hash[..6];
        format!("__cribo_{short_hash}_{module_name_escaped}")
    }

    /// Trim unused imports from all modules before bundling using graph information
    fn trim_unused_imports_from_modules(
        &mut self,
        modules: Vec<(String, ModModule, PathBuf, String)>,
        graph: &DependencyGraph,
        tree_shaker: Option<&crate::tree_shaking::TreeShaker>,
    ) -> Result<Vec<(String, ModModule, PathBuf, String)>> {
        let mut trimmed_modules = Vec::new();

        for (module_name, mut ast, module_path, content_hash) in modules {
            log::debug!("Trimming unused imports from module: {module_name}");

            // Check if this is an __init__.py file
            let is_init_py =
                module_path.file_name().and_then(|name| name.to_str()) == Some("__init__.py");

            // Get unused imports from the graph
            if let Some(module_dep_graph) = graph.get_module_by_name(&module_name) {
                let mut unused_imports = module_dep_graph.find_unused_imports(is_init_py);

                // If tree shaking is enabled, also check if imported symbols were removed
                // Note: We only apply tree-shaking logic to "from module import symbol" style
                // imports, not to "import module" style imports, since module
                // imports set up namespace objects
                if let Some(shaker) = tree_shaker {
                    // Only apply tree-shaking-aware import removal if tree shaking is actually
                    // enabled Get the symbols that survive tree-shaking for
                    // this module
                    let used_symbols = shaker.get_used_symbols_for_module(&module_name);

                    // Check each import to see if it's only used by tree-shaken code
                    let import_items = module_dep_graph.get_all_import_items();
                    for (item_id, import_item) in import_items {
                        match &import_item.item_type {
                            crate::cribo_graph::ItemType::FromImport {
                                module: from_module,
                                names,
                                ..
                            } => {
                                // For from imports, check each imported name
                                for (imported_name, alias_opt) in names {
                                    let local_name = alias_opt.as_ref().unwrap_or(imported_name);

                                    // Skip if already marked as unused
                                    if unused_imports.iter().any(|u| u.name == *local_name) {
                                        continue;
                                    }

                                    // Skip if this is a re-export (in __all__ or explicit
                                    // re-export)
                                    if import_item.reexported_names.contains(local_name)
                                        || module_dep_graph.is_in_all_export(local_name)
                                    {
                                        log::debug!(
                                            "Skipping tree-shaking for re-exported import \
                                             '{local_name}' from '{from_module}'"
                                        );
                                        continue;
                                    }

                                    // Check if this import is only used by symbols that were
                                    // tree-shaken
                                    let mut used_by_surviving_code = false;

                                    // First check if any surviving symbol uses this import
                                    for symbol in &used_symbols {
                                        if module_dep_graph
                                            .does_symbol_use_import(symbol, local_name)
                                        {
                                            used_by_surviving_code = true;
                                            break;
                                        }
                                    }

                                    // Also check if the module has side effects and uses this
                                    // import at module level
                                    if !used_by_surviving_code
                                        && shaker.module_has_side_effects(&module_name)
                                    {
                                        // Check if any module-level code uses this import
                                        for item in module_dep_graph.items.values() {
                                            if matches!(
                                                item.item_type,
                                                crate::cribo_graph::ItemType::Expression
                                                    | crate::cribo_graph::ItemType::Assignment { .. }
                                            ) && item.read_vars.contains(local_name)
                                            {
                                                used_by_surviving_code = true;
                                                log::debug!(
                                                    "Import '{local_name}' is used by \
                                                     module-level code in module with side effects"
                                                );
                                                break;
                                            }
                                        }
                                    }

                                    if !used_by_surviving_code {
                                        // This import is not used by any surviving symbol or
                                        // module-level code
                                        log::debug!(
                                            "Import '{local_name}' from '{from_module}' is not \
                                             used by surviving code after tree-shaking"
                                        );
                                        unused_imports.push(crate::cribo_graph::UnusedImportInfo {
                                            item_id,
                                            name: local_name.clone(),
                                            module: from_module.clone(),
                                            is_reexport: import_item
                                                .reexported_names
                                                .contains(local_name),
                                        });
                                    }
                                }
                            }
                            crate::cribo_graph::ItemType::Import { module, .. } => {
                                // For regular imports (import module), check if they're only used
                                // by tree-shaken code
                                let import_name = module.split('.').next_back().unwrap_or(module);

                                // Skip if already marked as unused
                                if unused_imports.iter().any(|u| u.name == *import_name) {
                                    continue;
                                }

                                // Skip if this is a re-export
                                if import_item.reexported_names.contains(import_name)
                                    || module_dep_graph.is_in_all_export(import_name)
                                {
                                    log::debug!(
                                        "Skipping tree-shaking for re-exported import \
                                         '{import_name}'"
                                    );
                                    continue;
                                }

                                // Check if this import is only used by symbols that were
                                // tree-shaken
                                let mut used_by_surviving_code = false;

                                // Check if any surviving symbol uses this import
                                for symbol in &used_symbols {
                                    if module_dep_graph.does_symbol_use_import(symbol, import_name)
                                    {
                                        used_by_surviving_code = true;
                                        break;
                                    }
                                }

                                // Also check if any module-level code that has side effects uses it
                                for item in module_dep_graph.items.values() {
                                    if item.has_side_effects
                                        && !matches!(
                                            item.item_type,
                                            crate::cribo_graph::ItemType::Import { .. }
                                                | crate::cribo_graph::ItemType::FromImport { .. }
                                        )
                                        && (item.read_vars.contains(import_name)
                                            || item.eventual_read_vars.contains(import_name))
                                    {
                                        used_by_surviving_code = true;
                                        break;
                                    }
                                }

                                if !used_by_surviving_code {
                                    log::debug!(
                                        "Import '{import_name}' is not used by surviving code \
                                         after tree-shaking"
                                    );
                                    unused_imports.push(crate::cribo_graph::UnusedImportInfo {
                                        item_id,
                                        name: import_name.to_string(),
                                        module: module.clone(),
                                        is_reexport: false,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if !unused_imports.is_empty() {
                    log::debug!(
                        "Found {} unused imports in {}",
                        unused_imports.len(),
                        module_name
                    );
                    // Log unused imports details
                    Self::log_unused_imports_details(&unused_imports);

                    // Filter out unused imports from the AST
                    ast.body
                        .retain(|stmt| !self.should_remove_import_stmt(stmt, &unused_imports));
                }
            }

            trimmed_modules.push((module_name, ast, module_path, content_hash));
        }

        log::debug!(
            "Successfully trimmed unused imports from {} modules",
            trimmed_modules.len()
        );
        Ok(trimmed_modules)
    }

    /// Check if an import statement should be removed based on unused imports
    fn should_remove_import_stmt(
        &self,
        stmt: &Stmt,
        unused_imports: &[crate::cribo_graph::UnusedImportInfo],
    ) -> bool {
        match stmt {
            Stmt::Import(import_stmt) => {
                // Check if all names in this import are unused
                let should_remove = import_stmt.names.iter().all(|alias| {
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(alias.name.as_str());

                    unused_imports.iter().any(|unused| {
                        log::trace!(
                            "Checking if import '{}' matches unused '{}' from '{}'",
                            local_name,
                            unused.name,
                            unused.module
                        );
                        unused.name == local_name
                    })
                });

                if should_remove {
                    log::debug!(
                        "Removing import statement: {:?}",
                        import_stmt
                            .names
                            .iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>()
                    );
                }
                should_remove
            }
            Stmt::ImportFrom(import_from) => {
                // Skip __future__ imports - they're handled separately
                if import_from.module.as_ref().map(|m| m.as_str()) == Some("__future__") {
                    return false;
                }

                // Check if all names in this from-import are unused
                import_from.names.iter().all(|alias| {
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(alias.name.as_str());

                    unused_imports
                        .iter()
                        .any(|unused| unused.name == local_name)
                })
            }
            _ => false,
        }
    }

    /// Collect future imports from an AST
    fn collect_future_imports_from_ast(&mut self, ast: &ModModule) {
        for stmt in &ast.body {
            let Stmt::ImportFrom(import_from) = stmt else {
                continue;
            };

            let Some(ref module) = import_from.module else {
                continue;
            };

            if module.as_str() == "__future__" {
                for alias in &import_from.names {
                    self.future_imports.insert(alias.name.to_string());
                }
            }
        }
    }

    /// Bundle multiple modules using the hybrid approach
    pub fn bundle_modules(&mut self, params: BundleParams<'_>) -> Result<ModModule> {
        let mut final_body = Vec::new();

        // Store tree shaking decisions if provided
        if let Some(shaker) = params.tree_shaker {
            // Extract all kept symbols from the tree shaker
            let mut kept_symbols = indexmap::IndexSet::new();
            for (module_name, _, _, _) in &params.modules {
                for symbol in shaker.get_used_symbols_for_module(module_name) {
                    kept_symbols.insert((module_name.clone(), symbol));
                }
            }
            self.tree_shaking_keep_symbols = Some(kept_symbols);
            log::debug!(
                "Tree shaking enabled, keeping {} symbols",
                self.tree_shaking_keep_symbols
                    .as_ref()
                    .map(|s| s.len())
                    .unwrap_or(0)
            );
        }

        log::debug!("Entry module name: {}", params.entry_module_name);
        log::debug!(
            "Module names in modules vector: {:?}",
            params
                .modules
                .iter()
                .map(|(name, _, _, _)| name)
                .collect::<Vec<_>>()
        );

        // First pass: collect future imports from ALL modules before trimming
        // This ensures future imports are hoisted even if they appear late in the file
        for (_module_name, ast, _, _) in &params.modules {
            self.collect_future_imports_from_ast(ast);
        }

        // Trim unused imports from all modules
        let mut modules = self.trim_unused_imports_from_modules(
            params.modules,
            params.graph,
            params.tree_shaker,
        )?;

        // Index all module ASTs to assign node indices and initialize transformation context
        log::debug!("Indexing {} modules", modules.len());
        let mut module_indices = Vec::new();
        let mut total_nodes = 0u32;
        let mut module_id = 0u32;

        // Create a mapping from module name to module ID for debugging
        let mut module_id_map = FxIndexMap::default();

        for (module_name, ast, path, _content_hash) in &mut modules {
            let indexed = crate::ast_indexer::index_module_with_id(ast, module_id);
            let node_count = indexed.node_count;
            log::debug!(
                "Module {} (ID: {}) indexed with {} nodes (indices {}-{})",
                module_name,
                module_id,
                node_count,
                module_id * crate::ast_indexer::MODULE_INDEX_RANGE,
                module_id * crate::ast_indexer::MODULE_INDEX_RANGE + node_count - 1
            );
            module_id_map.insert(module_name.clone(), module_id);
            module_indices.push((module_name.clone(), path.clone(), indexed));
            total_nodes += node_count;
            module_id += 1;
        }

        // Initialize transformation context
        // Start new node indices after all module ranges
        self.transformation_context = TransformationContext::new();
        let starting_index = module_id * crate::ast_indexer::MODULE_INDEX_RANGE;
        for _ in 0..starting_index {
            self.transformation_context.next_node_index();
        }
        log::debug!(
            "Transformation context initialized. Module count: {module_id}, Total nodes: \
             {total_nodes}, New nodes start at: {starting_index}"
        );

        // Store module ASTs for re-export resolution
        self.module_asts = Some(modules.clone());

        // Store entry path for relative path calculation
        if let Some((_, entry_path, _)) = params.sorted_modules.last() {
            self.entry_path = Some(entry_path.to_string_lossy().to_string());
        }

        // Track bundled modules
        for (module_name, _, _, _) in &modules {
            self.bundled_modules.insert(module_name.clone());
        }

        // Check which modules are imported directly (e.g., import module_name)
        let directly_imported_modules =
            self.find_directly_imported_modules(&modules, params.entry_module_name);
        log::debug!("Directly imported modules: {directly_imported_modules:?}");

        // Find modules that are imported as namespaces (e.g., from models import base)
        // We need to include the entry module in this analysis since it might contain namespace
        // imports
        let mut all_modules_for_namespace_check = modules.clone();

        // Find the entry module from the topologically sorted modules
        for (module_name, ast, module_path, content_hash) in &modules {
            if module_name == params.entry_module_name {
                all_modules_for_namespace_check.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
                break;
            }
        }

        self.find_namespace_imported_modules(&all_modules_for_namespace_check);

        // Identify all modules that are part of circular dependencies
        if let Some(analysis) = params.circular_dep_analysis {
            log::debug!("CircularDependencyAnalysis received:");
            log::debug!("  Resolvable cycles: {:?}", analysis.resolvable_cycles);
            log::debug!("  Unresolvable cycles: {:?}", analysis.unresolvable_cycles);
            for group in &analysis.resolvable_cycles {
                for module in &group.modules {
                    self.circular_modules.insert(module.clone());
                }
            }
            for group in &analysis.unresolvable_cycles {
                for module in &group.modules {
                    self.circular_modules.insert(module.clone());
                }
            }
            log::debug!("Circular modules: {:?}", self.circular_modules);
        } else {
            log::debug!("No circular dependency analysis provided");
        }

        // Separate modules into inlinable and wrapper modules
        let mut inlinable_modules = Vec::new();
        let mut wrapper_modules = Vec::new();
        let mut module_exports_map = FxIndexMap::default();

        for (module_name, ast, module_path, content_hash) in &modules {
            log::debug!("Processing module: '{module_name}'");
            if module_name == params.entry_module_name {
                continue;
            }

            // Extract __all__ exports from the module
            let (has_explicit_all, module_exports) = self.extract_all_exports(ast);
            if has_explicit_all {
                self.modules_with_explicit_all.insert(module_name.clone());
            }
            module_exports_map.insert(module_name.clone(), module_exports.clone());

            // Check if module is imported as a namespace
            let is_namespace_imported = self.namespace_imported_modules.contains_key(module_name);

            if is_namespace_imported {
                log::debug!(
                    "Module '{}' is imported as namespace by: {:?}",
                    module_name,
                    self.namespace_imported_modules.get(module_name)
                );
            }

            // With full static bundling, we only need to wrap modules with side effects
            // All imports are rewritten at bundle time, so namespace imports, direct imports,
            // and circular dependencies can all be handled through static transformation
            let has_side_effects = Self::has_side_effects(ast);

            if has_side_effects {
                log::debug!("Module '{module_name}' has side effects - using wrapper approach");
                wrapper_modules.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            } else {
                log::debug!("Module '{module_name}' has no side effects - can be inlined");
                inlinable_modules.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            }
        }

        // Track which modules will be inlined (before wrapper module generation)
        for (module_name, _, _, _) in &inlinable_modules {
            self.inlined_modules.insert(module_name.clone());
            // Also store module exports for inlined modules
            self.module_exports.insert(
                module_name.clone(),
                module_exports_map.get(module_name).cloned().flatten(),
            );
        }

        // First pass: normalize stdlib import aliases in ALL modules before collecting imports
        let mut modules_normalized = modules;
        for (_module_name, ast, _, _) in &mut modules_normalized {
            self.normalize_stdlib_import_aliases(ast);
        }

        // Second pass: collect imports from ALL modules (for hoisting)
        for (module_name, ast, module_path, _) in &modules_normalized {
            self.collect_imports_from_module(ast, module_name, module_path);
        }

        // If we have wrapper modules, inject types as stdlib dependency
        if !wrapper_modules.is_empty() {
            log::debug!("Adding types import for wrapper modules");
            self.add_stdlib_import("types");
        }

        // If we have namespace imports, inject types as stdlib dependency
        if !self.namespace_imported_modules.is_empty() {
            log::debug!("Adding types import for namespace imports");
            self.add_stdlib_import("types");
        }

        // Check if entry module has direct imports or dotted imports that might create namespace
        // objects - but only for first-party modules that we're actually bundling
        let needs_types_for_entry_imports = if let Some((_, entry_path, _)) = params
            .sorted_modules
            .iter()
            .find(|(name, _, _)| name == params.entry_module_name)
        {
            // Load and parse the entry module
            if let Ok(content) = std::fs::read_to_string(entry_path) {
                let normalized_content = crate::util::normalize_line_endings(content);
                if let Ok(parsed) = ruff_python_parser::parse_module(&normalized_content) {
                    let ast = parsed.into_syntax();
                    ast.body.iter().any(|stmt| {
                        if let Stmt::Import(import_stmt) = stmt {
                            import_stmt.names.iter().any(|alias| {
                                let module_name = alias.name.as_str();
                                // Check for dotted imports - but only first-party ones
                                if module_name.contains('.') {
                                    // Check if this dotted import refers to a first-party module
                                    // by checking if any bundled module matches this dotted path
                                    let is_first_party_dotted =
                                        modules_normalized.iter().any(|(name, _, _, _)| {
                                            name == module_name
                                                || module_name.starts_with(&format!("{name}."))
                                        });
                                    if is_first_party_dotted {
                                        log::debug!(
                                            "Found first-party dotted import '{module_name}' that \
                                             requires namespace"
                                        );
                                        return true;
                                    }
                                }
                                // Check for direct imports of inlined modules that have exports
                                if self.inlined_modules.contains(module_name) {
                                    // Check if the module has exports
                                    if let Some(Some(exports)) =
                                        self.module_exports.get(module_name)
                                    {
                                        let has_exports = !exports.is_empty();
                                        if has_exports {
                                            log::debug!(
                                                "Direct import of inlined module '{module_name}' \
                                                 with exports: {exports:?}"
                                            );
                                        }
                                        return has_exports;
                                    }
                                }
                                false
                            })
                        } else {
                            false
                        }
                    })
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if needs_types_for_entry_imports {
            log::debug!("Adding types import for namespace objects in entry module");
            self.add_stdlib_import("types");
        }

        // Register wrapper modules
        for (module_name, _ast, _module_path, content_hash) in &wrapper_modules {
            self.module_exports.insert(
                module_name.clone(),
                module_exports_map.get(module_name).cloned().flatten(),
            );

            // Register module with synthetic name using content hash
            let synthetic_name = self.get_synthetic_module_name(module_name, content_hash);
            self.module_registry
                .insert(module_name.clone(), synthetic_name.clone());

            // Register init function
            let init_func_name = format!("__cribo_init_{synthetic_name}");
            self.init_functions.insert(synthetic_name, init_func_name);
        }

        // Add imports first
        self.add_hoisted_imports(&mut final_body);

        // Check if we have wrapper modules
        let has_wrapper_modules = !wrapper_modules.is_empty();

        // Check if we need types import (for namespace imports)
        let _need_types_import = !self.namespace_imported_modules.is_empty();

        // Get symbol renames from semantic analysis
        let symbol_registry = params.semantic_bundler.symbol_registry();
        let mut symbol_renames = FxIndexMap::default();

        // Create semantic context
        let semantic_ctx = SemanticContext {
            graph: params.graph,
            symbol_registry,
            semantic_bundler: params.semantic_bundler,
        };

        // Convert ModuleId-based renames to module name-based renames
        for (module_name, _, _, _) in &modules_normalized {
            self.collect_module_renames(module_name, &semantic_ctx, &mut symbol_renames);
        }

        // Collect global symbols from the entry module first (for compatibility)
        let mut global_symbols =
            self.collect_global_symbols(&modules_normalized, params.entry_module_name);

        // Save wrapper modules for later processing
        let wrapper_modules_saved = wrapper_modules;

        // Build symbol-level dependency graph for circular modules if needed
        if !self.circular_modules.is_empty() {
            log::debug!("Building symbol dependency graph for circular modules");

            // Convert modules to the format expected by build_symbol_dependency_graph
            let modules_for_graph: Vec<(String, ModModule, PathBuf, String)> = modules_normalized
                .iter()
                .map(|(name, ast, path, hash)| {
                    (name.clone(), ast.clone(), path.clone(), hash.clone())
                })
                .collect();

            self.build_symbol_dependency_graph(&modules_for_graph, params.graph, &semantic_ctx);

            // Get ordered symbols for circular modules
            match self
                .symbol_dep_graph
                .topological_sort_symbols(&self.circular_modules)
            {
                Ok(ordered_symbols) => {
                    log::debug!("Symbol ordering for circular modules: {ordered_symbols:?}");
                }
                Err(e) => {
                    log::warn!("Failed to order symbols in circular modules: {e}");
                    // Continue with default ordering
                }
            }
        }

        // Generate pre-declarations only for symbols that actually need them
        let mut circular_predeclarations = Vec::new();
        if !self.circular_modules.is_empty() {
            log::debug!("Analyzing circular modules for necessary pre-declarations");

            // Collect all symbols that need pre-declaration based on actual forward references
            let mut symbols_needing_predeclaration = FxIndexSet::default();

            // First pass: Build a map of where each symbol will be defined in the final output
            let mut symbol_definition_order = FxIndexMap::default();
            let mut order_index = 0;

            for (module_name, _, _, _) in &inlinable_modules {
                if let Some(module_renames) = symbol_renames.get(module_name) {
                    for (original_name, _) in module_renames {
                        symbol_definition_order
                            .insert((module_name.clone(), original_name.clone()), order_index);
                        order_index += 1;
                    }
                }
            }

            // Second pass: Find actual forward references using module-level dependencies
            for ((module, symbol), module_level_deps) in
                &self.symbol_dep_graph.module_level_dependencies
            {
                if self.circular_modules.contains(module) && !module_level_deps.is_empty() {
                    // Check each module-level dependency
                    for (dep_module, dep_symbol) in module_level_deps {
                        if self.circular_modules.contains(dep_module) {
                            // Get the order indices
                            let symbol_order =
                                symbol_definition_order.get(&(module.clone(), symbol.clone()));
                            let dep_order = symbol_definition_order
                                .get(&(dep_module.clone(), dep_symbol.clone()));

                            if let (Some(&sym_idx), Some(&dep_idx)) = (symbol_order, dep_order) {
                                // Check if this creates a forward reference
                                if dep_idx > sym_idx {
                                    log::debug!(
                                        "Found forward reference: {module}.{symbol} (order \
                                         {sym_idx}) uses {dep_module}.{dep_symbol} (order \
                                         {dep_idx}) at module level"
                                    );
                                    symbols_needing_predeclaration
                                        .insert((dep_module.clone(), dep_symbol.clone()));
                                }
                            }
                        }
                    }
                }
            }

            // Now generate pre-declarations only for symbols that actually need them
            log::debug!("Symbols needing pre-declaration: {symbols_needing_predeclaration:?}");
            for (module_name, symbol_name) in symbols_needing_predeclaration {
                if let Some(module_renames) = symbol_renames.get(&module_name)
                    && let Some(renamed_name) = module_renames.get(&symbol_name)
                {
                    log::debug!(
                        "Pre-declaring {renamed_name} (from {module_name}.{symbol_name}) due to \
                         forward reference"
                    );
                    circular_predeclarations.push(Stmt::Assign(StmtAssign {
                        node_index: self.create_transformed_node(format!(
                            "Pre-declaration for circular dependency: {renamed_name}"
                        )),
                        targets: vec![Expr::Name(ExprName {
                            node_index: self.create_node_index(),
                            id: renamed_name.clone().into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::NoneLiteral(ExprNoneLiteral {
                            node_index: self.create_node_index(),
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    }));

                    // Track the pre-declaration
                    self.circular_predeclarations
                        .entry(module_name.clone())
                        .or_default()
                        .insert(symbol_name.clone(), renamed_name.clone());
                }
            }
        }

        // Add pre-declarations at the very beginning
        final_body.extend(circular_predeclarations);

        // Inline the inlinable modules FIRST to populate symbol_renames
        // This ensures we know what symbols have been renamed before processing wrapper modules and
        // namespace hybrids
        let mut all_deferred_imports = Vec::new();
        for (module_name, ast, _module_path, _content_hash) in &inlinable_modules {
            log::debug!("Inlining module '{module_name}'");
            let mut inlined_stmts = Vec::new();
            let mut deferred_imports = Vec::new();
            let mut inline_ctx = InlineContext {
                module_exports_map: &module_exports_map,
                global_symbols: &mut global_symbols,
                module_renames: &mut symbol_renames,
                inlined_stmts: &mut inlined_stmts,
                import_aliases: FxIndexMap::default(),
                deferred_imports: &mut deferred_imports,
            };
            self.inline_module(module_name, ast.clone(), _module_path, &mut inline_ctx)?;
            log::debug!(
                "Inlined {} statements from module '{}'",
                inlined_stmts.len(),
                module_name
            );
            final_body.extend(inlined_stmts);
            // Track deferred imports globally
            for stmt in &deferred_imports {
                if let Stmt::Assign(assign) = stmt {
                    // Check for pattern: symbol = sys.modules['module'].symbol
                    if let Expr::Attribute(attr) = &assign.value.as_ref()
                        && let Expr::Subscript(subscript) = &attr.value.as_ref()
                        && let Expr::Attribute(sys_attr) = &subscript.value.as_ref()
                        && let Expr::Name(sys_name) = &sys_attr.value.as_ref()
                        && sys_name.id.as_str() == "sys"
                        && sys_attr.attr.as_str() == "modules"
                        && let Expr::StringLiteral(lit) = &subscript.slice.as_ref()
                    {
                        let import_module = lit.value.to_str();
                        let attr_name = attr.attr.as_str();
                        log::debug!(
                            "Registering deferred import: {attr_name} from {import_module} \
                             (deferred by {module_name})"
                        );
                        self.global_deferred_imports.insert(
                            (import_module.to_string(), attr_name.to_string()),
                            module_name.to_string(),
                        );
                    }
                }
            }

            // Filter deferred imports to avoid conflicts
            // If an inlined module imports a symbol but doesn't export it,
            // and that symbol would conflict with other imports, skip it
            for stmt in deferred_imports {
                let should_include = if let Stmt::Assign(assign) = &stmt {
                    if let [Expr::Name(target)] = assign.targets.as_slice()
                        && let Expr::Name(_value) = &*assign.value
                    {
                        let symbol_name = target.id.as_str();

                        // Check if this module exports the symbol
                        let exports_symbol =
                            if let Some(Some(exports)) = module_exports_map.get(module_name) {
                                exports.contains(&symbol_name.to_string())
                            } else {
                                // No explicit __all__, check if it's a module-level definition
                                // For now, assume it's not exported if there's no __all__
                                false
                            };

                        if !exports_symbol {
                            // Check if this would conflict with existing deferred imports
                            let has_conflict = all_deferred_imports.iter().any(|existing| {
                                if let Stmt::Assign(existing_assign) = existing
                                    && let [Expr::Name(existing_target)] =
                                        existing_assign.targets.as_slice()
                                {
                                    existing_target.id.as_str() == symbol_name
                                } else {
                                    false
                                }
                            });

                            if has_conflict {
                                log::debug!(
                                    "Skipping deferred import '{symbol_name}' from module \
                                     '{module_name}' due to conflict"
                                );
                                false
                            } else {
                                true
                            }
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                } else {
                    true
                };

                if should_include {
                    // Check if this deferred import already exists in all_deferred_imports
                    let is_duplicate = if let Stmt::Assign(assign) = &stmt {
                        if let Expr::Name(target) = &assign.targets[0] {
                            let target_name = target.id.as_str();

                            // Check against existing deferred imports
                            all_deferred_imports.iter().any(|existing| {
                                if let Stmt::Assign(existing_assign) = existing
                                    && let [Expr::Name(existing_target)] =
                                        existing_assign.targets.as_slice()
                                    && existing_target.id.as_str() == target_name
                                {
                                    // Check if the values are the same
                                    return Self::expr_equals(
                                        &existing_assign.value,
                                        &assign.value,
                                    );
                                }
                                false
                            })
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !is_duplicate {
                        // Log what we're adding to deferred imports
                        if let Stmt::Assign(assign) = &stmt
                            && let Expr::Name(target) = &assign.targets[0]
                        {
                            if let Expr::Attribute(attr) = &assign.value.as_ref() {
                                let attr_path = self.extract_attribute_path(attr);
                                log::debug!(
                                    "Adding to all_deferred_imports: {} = {} (from inlined module \
                                     '{}')",
                                    target.id.as_str(),
                                    attr_path,
                                    module_name
                                );
                            } else if let Expr::Name(value) = &assign.value.as_ref() {
                                log::debug!(
                                    "Adding to all_deferred_imports: {} = {} (from inlined module \
                                     '{}')",
                                    target.id.as_str(),
                                    value.id.as_str(),
                                    module_name
                                );
                            }
                        }
                        all_deferred_imports.push(stmt);
                    } else {
                        log::debug!(
                            "Skipping duplicate deferred import from module '{module_name}': \
                             {stmt:?}"
                        );
                    }
                }
            }
        }

        // Create namespace objects for inlined modules that are imported as namespaces
        for (module_name, _, _, _) in &inlinable_modules {
            if self.namespace_imported_modules.contains_key(module_name) {
                log::debug!("Creating namespace for inlined module '{module_name}'");

                // Get the symbols that were inlined from this module
                if let Some(module_rename_map) = symbol_renames.get(module_name) {
                    // Create a SimpleNamespace for this module
                    let namespace_stmt = self
                        .create_namespace_for_inlined_module_static(module_name, module_rename_map);
                    final_body.push(namespace_stmt);
                }
            }
        }

        // Now transform wrapper modules into init functions AFTER inlining
        // This way we have access to symbol_renames for proper import resolution
        if has_wrapper_modules {
            // First pass: analyze globals in all wrapper modules
            let mut module_globals = FxIndexMap::default();
            let mut all_lifted_declarations = Vec::new();

            for (module_name, ast, _, _) in &wrapper_modules_saved {
                let params = ProcessGlobalsParams {
                    module_name,
                    ast,
                    semantic_ctx: &semantic_ctx,
                };
                self.process_wrapper_module_globals(
                    &params,
                    &mut module_globals,
                    &mut all_lifted_declarations,
                );
            }

            // Store all lifted declarations
            debug!(
                "Collected {} total lifted declarations",
                all_lifted_declarations.len()
            );
            self.lifted_global_declarations = all_lifted_declarations.clone();

            // Add lifted global declarations to final body before init functions
            if !all_lifted_declarations.is_empty() {
                debug!(
                    "Adding {} lifted global declarations to final body",
                    all_lifted_declarations.len()
                );
                final_body.extend(all_lifted_declarations);
            }

            // Second pass: transform modules with global info
            for (module_name, ast, module_path, _content_hash) in &wrapper_modules_saved {
                let synthetic_name = self.module_registry[module_name].clone();
                let global_info = module_globals.get(module_name).cloned();
                let ctx = ModuleTransformContext {
                    module_name,
                    synthetic_name: &synthetic_name,
                    module_path,
                    global_info,
                };
                let init_function =
                    self.transform_module_to_init_function(ctx, ast.clone(), &symbol_renames)?;
                final_body.push(init_function);
            }

            // Now add the registries after init functions are defined
            final_body.extend(self.generate_registries_and_hook());
        }

        // Initialize wrapper modules in dependency order AFTER inlined modules are defined
        if has_wrapper_modules {
            debug!("Creating parent namespaces before module initialization");

            // Pre-scan to identify all required namespaces
            self.identify_required_namespaces(&modules_normalized);

            // Create namespace statements BEFORE module initialization
            let namespace_statements = self.create_namespace_statements();
            debug!(
                "Created {} namespace statements",
                namespace_statements.len()
            );

            // Add namespace creation statements to the final body
            final_body.extend(namespace_statements);

            debug!("Initializing modules in order:");

            // First, collect all wrapped modules that need initialization
            let mut wrapped_modules_to_init = Vec::new();
            for (module_name, _, _) in params.sorted_modules {
                if module_name == params.entry_module_name {
                    continue;
                }
                if self.module_registry.contains_key(module_name) {
                    wrapped_modules_to_init.push(module_name.clone());
                }
            }

            // Sort wrapped modules by their dependencies to ensure correct initialization order
            // This is critical for namespace imports in circular dependencies
            debug!("Wrapped modules before sorting: {wrapped_modules_to_init:?}");
            let sorted_wrapped = self.sort_wrapped_modules_by_dependencies(
                &wrapped_modules_to_init,
                params.sorted_modules,
            );
            debug!("Wrapped modules after sorting: {sorted_wrapped:?}");

            // Initialize all modules in dependency order
            for module_name in &sorted_wrapped {
                debug!("  - Initializing module: {module_name}");
                if let Some(synthetic_name) = self.module_registry.get(module_name).cloned() {
                    let init_stmts = self.generate_module_init_call(&synthetic_name);
                    final_body.extend(init_stmts);
                }
            }

            // After all modules are initialized, assign temporary variables to their namespace
            // locations For parent modules that are also wrapper modules, we need to
            // merge their attributes
            for module_name in &sorted_wrapped {
                let temp_var_name = format!("_cribo_temp_{}", module_name.cow_replace(".", "_"));

                // Check if this module has submodules (is a parent module)
                let is_parent_module = sorted_wrapped
                    .iter()
                    .any(|m| m != module_name && m.starts_with(&format!("{module_name}.")));

                if module_name.contains('.') {
                    // For dotted modules, check if they have their own submodules
                    // If they do, we need to merge attributes instead of overwriting
                    if is_parent_module {
                        debug!(
                            "Dotted module '{module_name}' is both a wrapper module and parent \
                             namespace"
                        );
                        // We need to merge the wrapper module's attributes into the existing
                        // namespace Get the parts to construct the
                        // namespace path
                        let parts: Vec<&str> = module_name.split('.').collect();
                        let mut namespace_path = String::new();
                        for (i, part) in parts.iter().enumerate() {
                            if i > 0 {
                                namespace_path.push('.');
                            }
                            namespace_path.push_str(part);
                        }

                        // Generate merge code for dotted parent modules
                        self.generate_merge_module_attributes(
                            &mut final_body,
                            &namespace_path,
                            &temp_var_name,
                        );
                    } else {
                        // Normal dotted module assignment
                        let parts: Vec<&str> = module_name.split('.').collect();

                        // Create the namespace assignment: models.base = _cribo_temp_models_base
                        let mut target_expr = Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: parts[0].into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        });

                        for (i, part) in parts[1..].iter().enumerate() {
                            let ctx = if i == parts.len() - 2 {
                                ExprContext::Store // Last part is Store context
                            } else {
                                ExprContext::Load
                            };
                            target_expr = Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(target_expr),
                                attr: Identifier::new(*part, TextRange::default()),
                                ctx,
                                range: TextRange::default(),
                            });
                        }

                        // models.base = _cribo_temp_models_base
                        final_body.push(Stmt::Assign(StmtAssign {
                            node_index: AtomicNodeIndex::dummy(),
                            targets: vec![target_expr],
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: temp_var_name.into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
                    }
                } else {
                    // For simple module names that are parent modules, we need to merge attributes
                    if is_parent_module {
                        debug!(
                            "Module '{module_name}' is both a wrapper module and parent namespace"
                        );
                        // Instead of overwriting, merge the wrapper module's attributes into the
                        // namespace This is done by copying all attributes
                        // from the temporary module to the namespace
                        self.generate_merge_module_attributes(
                            &mut final_body,
                            module_name,
                            &temp_var_name,
                        );
                    } else {
                        // Simple assignment for non-parent modules
                        debug!("Assigning simple module '{module_name}' = '{temp_var_name}'");
                        final_body.push(Stmt::Assign(StmtAssign {
                            node_index: AtomicNodeIndex::dummy(),
                            targets: vec![Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: module_name.into(),
                                ctx: ExprContext::Store,
                                range: TextRange::default(),
                            })],
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: temp_var_name.into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
                    }
                }
            }

            // After all modules are initialized, ensure sub-modules are attached to parent modules
            // This is necessary for relative imports like "from . import messages" to work
            // correctly
            // Check what modules are imported in the entry module to avoid duplicates
            let entry_imported_modules =
                self.get_entry_module_imports(&modules_normalized, params.entry_module_name);

            debug!(
                "About to generate submodule attributes, current body length: {}",
                final_body.len()
            );
            self.generate_submodule_attributes_with_exclusions(
                params.sorted_modules,
                &mut final_body,
                &entry_imported_modules,
            );
            debug!(
                "After generate_submodule_attributes, body length: {}",
                final_body.len()
            );
        }

        // Add deferred imports from inlined modules before entry module code
        // This ensures they're available when the entry module code runs
        if !all_deferred_imports.is_empty() {
            log::debug!(
                "Adding {} deferred imports from inlined modules before entry module",
                all_deferred_imports.len()
            );

            // Log what deferred imports we have
            for (i, stmt) in all_deferred_imports.iter().enumerate() {
                if let Stmt::Assign(assign) = stmt
                    && let Expr::Name(target) = &assign.targets[0]
                {
                    log::debug!("  Deferred import {}: {} = ...", i, target.id.as_str());
                }
            }

            // Filter out init calls - they should already be added when wrapper modules were
            // initialized
            let imports_without_init_calls: Vec<Stmt> = all_deferred_imports
                .iter()
                .filter(|stmt| {
                    // Skip init calls
                    if let Stmt::Expr(expr_stmt) = stmt
                        && let Expr::Call(call) = &expr_stmt.value.as_ref()
                        && let Expr::Name(name) = &call.func.as_ref()
                    {
                        return !name.id.as_str().starts_with("__cribo_init_");
                    }
                    true
                })
                .cloned()
                .collect();

            // Then add the deferred imports (without init calls)
            // Pass the current final_body so we can check for existing assignments
            let num_imports_before = imports_without_init_calls.len();
            log::debug!(
                "About to deduplicate {} deferred imports against {} existing statements",
                num_imports_before,
                final_body.len()
            );
            let deduped_imports = self.deduplicate_deferred_imports_with_existing(
                imports_without_init_calls,
                &final_body,
            );
            log::debug!(
                "After deduplication: {} imports remain from {} original",
                deduped_imports.len(),
                num_imports_before
            );
            final_body.extend(deduped_imports);

            // Clear the collection so we don't add them again later
            all_deferred_imports.clear();
        }

        // Finally, add entry module code (it's always last in topological order)
        for (module_name, mut ast, module_path, _) in modules_normalized {
            if module_name != params.entry_module_name {
                continue;
            }

            log::debug!("Processing entry module: '{module_name}'");
            log::debug!("Entry module has {} statements", ast.body.len());

            // Entry module - add its code directly at the end
            // The entry module needs special handling for symbol conflicts
            let entry_module_renames = symbol_renames
                .get(&module_name)
                .cloned()
                .unwrap_or_default();

            log::debug!("Entry module '{module_name}' renames: {entry_module_renames:?}");

            // First pass: collect locally defined symbols in the entry module
            let mut locally_defined_symbols = FxIndexSet::default();
            for stmt in &ast.body {
                match stmt {
                    Stmt::FunctionDef(func_def) => {
                        locally_defined_symbols.insert(func_def.name.to_string());
                    }
                    Stmt::ClassDef(class_def) => {
                        locally_defined_symbols.insert(class_def.name.to_string());
                    }
                    _ => {}
                }
            }
            log::debug!("Entry module locally defined symbols: {locally_defined_symbols:?}");

            // Apply recursive import transformation to the entry module
            log::debug!("Creating RecursiveImportTransformer for entry module '{module_name}'");
            let mut entry_deferred_imports = Vec::new();
            let mut transformer = RecursiveImportTransformer::new(
                RecursiveImportTransformerParams {
                    bundler: self,
                    module_name: &module_name,
                    module_path: Some(&module_path),
                    symbol_renames: &symbol_renames,
                    deferred_imports: &mut entry_deferred_imports,
                    is_entry_module: true,  // This is the entry module
                    is_wrapper_init: false, // Not a wrapper init
                    global_deferred_imports: Some(&self.global_deferred_imports), /* Pass global deferred imports for checking */
                },
            );
            log::debug!(
                "Transforming entry module '{module_name}' with RecursiveImportTransformer"
            );
            transformer.transform_module(&mut ast);
            log::debug!("Finished transforming entry module '{module_name}'");

            // Process statements in order
            for stmt in &ast.body {
                let is_hoisted = self.is_hoisted_import(stmt);
                if is_hoisted {
                    continue;
                }

                match stmt {
                    Stmt::ImportFrom(import_from) => {
                        let is_duplicate = self.is_duplicate_import_from(import_from, &final_body);

                        if !is_duplicate {
                            // Imports have already been transformed by RecursiveImportTransformer
                            final_body.push(stmt.clone());
                        } else {
                            log::debug!(
                                "Skipping duplicate import in entry module: {:?}",
                                import_from.module
                            );
                        }
                    }
                    Stmt::Import(import_stmt) => {
                        let is_duplicate = self.is_duplicate_import(import_stmt, &final_body);

                        if !is_duplicate {
                            // Imports have already been transformed by RecursiveImportTransformer
                            final_body.push(stmt.clone());
                        } else {
                            log::debug!("Skipping duplicate import in entry module");
                        }
                    }
                    Stmt::Assign(assign) => {
                        // Check if this is an import assignment for a locally defined symbol
                        let is_import_for_local_symbol = if assign.targets.len() == 1 {
                            if let Expr::Name(target) = &assign.targets[0] {
                                locally_defined_symbols.contains(target.id.as_str())
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if is_import_for_local_symbol {
                            log::debug!(
                                "Skipping import assignment for locally defined symbol in entry \
                                 module"
                            );
                            continue;
                        }

                        // Check if this assignment already exists in final_body to avoid duplicates
                        let is_duplicate = if assign.targets.len() == 1 {
                            // Check the exact assignment pattern
                            if let (Expr::Name(target), _value) =
                                (&assign.targets[0], &assign.value.as_ref())
                            {
                                // Look for exact duplicate in final_body
                                final_body.iter().any(|stmt| {
                                    if let Stmt::Assign(existing) = stmt {
                                        if existing.targets.len() == 1 {
                                            if let Expr::Name(existing_target) =
                                                &existing.targets[0]
                                            {
                                                // Check if it's the same assignment
                                                existing_target.id == target.id
                                                    && Self::expr_equals(
                                                        &existing.value,
                                                        &assign.value,
                                                    )
                                            } else {
                                                false
                                            }
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    }
                                })
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if is_duplicate {
                            log::debug!("Skipping duplicate assignment in entry module");
                        } else {
                            let mut stmt_clone = stmt.clone();
                            self.process_entry_module_statement(
                                &mut stmt_clone,
                                &entry_module_renames,
                                &mut final_body,
                            );
                        }
                    }
                    _ => {
                        let mut stmt_clone = stmt.clone();
                        self.process_entry_module_statement(
                            &mut stmt_clone,
                            &entry_module_renames,
                            &mut final_body,
                        );
                    }
                }
            }

            // Add deferred imports from the entry module after all its statements
            // But first update the global registry to prevent future duplicates
            for stmt in &entry_deferred_imports {
                if let Stmt::Assign(assign) = stmt
                    && let Expr::Attribute(attr) = &assign.value.as_ref()
                    && let Expr::Subscript(subscript) = &attr.value.as_ref()
                    && let Expr::Attribute(sys_attr) = &subscript.value.as_ref()
                    && let Expr::Name(sys_name) = &sys_attr.value.as_ref()
                    && sys_name.id.as_str() == "sys"
                    && sys_attr.attr.as_str() == "modules"
                    && let Expr::StringLiteral(lit) = &subscript.slice.as_ref()
                {
                    let import_module = lit.value.to_str();
                    let attr_name = attr.attr.as_str();
                    if let Expr::Name(target) = &assign.targets[0] {
                        let _symbol_name = target.id.as_str();
                        self.global_deferred_imports.insert(
                            (import_module.to_string(), attr_name.to_string()),
                            module_name.to_string(),
                        );
                    }
                }
            }
            // Add entry module's deferred imports to the collection
            log::debug!(
                "Adding {} deferred imports from entry module",
                entry_deferred_imports.len()
            );
            for stmt in &entry_deferred_imports {
                if let Stmt::Assign(assign) = stmt
                    && let Expr::Name(target) = &assign.targets[0]
                    && let Expr::Attribute(attr) = &assign.value.as_ref()
                {
                    let attr_path = self.extract_attribute_path(attr);
                    log::debug!(
                        "Entry module deferred import: {} = {}",
                        target.id.as_str(),
                        attr_path
                    );
                }
            }
            // Add entry module's deferred imports with deduplication
            for stmt in entry_deferred_imports {
                let is_duplicate = if let Stmt::Assign(assign) = &stmt {
                    if let Expr::Name(target) = &assign.targets[0] {
                        let target_name = target.id.as_str();

                        // Check against existing deferred imports
                        all_deferred_imports.iter().any(|existing| {
                            if let Stmt::Assign(existing_assign) = existing
                                && let [Expr::Name(existing_target)] =
                                    existing_assign.targets.as_slice()
                                && existing_target.id.as_str() == target_name
                            {
                                // Check if the values are the same
                                return Self::expr_equals(&existing_assign.value, &assign.value);
                            }
                            false
                        })
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !is_duplicate {
                    all_deferred_imports.push(stmt);
                } else {
                    log::debug!("Skipping duplicate deferred import from entry module");
                }
            }
        }

        // Add any remaining deferred imports from the entry module
        // (The inlined module imports were already added before the entry module code)
        if !all_deferred_imports.is_empty() {
            log::debug!(
                "Adding {} remaining deferred imports from entry module",
                all_deferred_imports.len()
            );

            // First, ensure we have init calls for all wrapper modules that need them
            let mut needed_init_calls = FxIndexSet::default();
            for stmt in &all_deferred_imports {
                if let Stmt::Assign(assign) = stmt
                    && let Expr::Attribute(attr) = &assign.value.as_ref()
                    && let Expr::Subscript(subscript) = &attr.value.as_ref()
                    && let Expr::Attribute(sys_attr) = &subscript.value.as_ref()
                    && let Expr::Name(sys_name) = &sys_attr.value.as_ref()
                    && sys_name.id.as_str() == "sys"
                    && sys_attr.attr.as_str() == "modules"
                    && let Expr::StringLiteral(lit) = &subscript.slice.as_ref()
                {
                    let module_name = lit.value.to_str();
                    if let Some(synthetic_name) = self.module_registry.get(module_name) {
                        needed_init_calls.insert(synthetic_name.clone());
                    }
                }
            }

            // Add init calls first
            for synthetic_name in needed_init_calls {
                // Note: This is in a context where we can't mutate self, so we'll rely on
                // the namespaces being pre-created by identify_required_namespaces
                let init_stmts = self.generate_module_init_call(&synthetic_name);
                final_body.extend(init_stmts);
            }

            // Then deduplicate and add the actual imports (without init calls)
            let imports_without_init_calls: Vec<Stmt> = all_deferred_imports
                .into_iter()
                .filter(|stmt| {
                    // Skip init calls - we've already added them above
                    if let Stmt::Expr(expr_stmt) = stmt
                        && let Expr::Call(call) = &expr_stmt.value.as_ref()
                        && let Expr::Name(name) = &call.func.as_ref()
                    {
                        return !name.id.as_str().starts_with("__cribo_init_");
                    }
                    true
                })
                .collect();

            let deduped_imports = self.deduplicate_deferred_imports_with_existing(
                imports_without_init_calls,
                &final_body,
            );
            log::debug!(
                "Total deferred imports after deduplication: {}",
                deduped_imports.len()
            );
            final_body.extend(deduped_imports);
        }

        let mut result = ModModule {
            node_index: self.create_transformed_node("Bundled module root".to_string()),
            range: TextRange::default(),
            body: final_body,
        };

        // Assign proper node indices to all nodes in the final AST
        self.assign_node_indices_to_ast(&mut result);

        // Log transformation statistics
        let stats = self.transformation_context.get_stats();
        log::info!("Transformation statistics:");
        log::info!("  Total transformations: {}", stats.total_transformations);
        log::info!("  Direct copies: {}", stats.direct_copies);
        log::info!("  Imports rewritten: {}", stats.imports_rewritten);
        log::info!("  Globals replaced: {}", stats.globals_replaced);
        log::info!("  Modules wrapped: {}", stats.modules_wrapped);
        log::info!("  Dead code eliminated: {}", stats.dead_code_eliminated);
        log::info!("  New nodes created: {}", stats.new_nodes);
        log::info!("  Nodes merged: {}", stats.nodes_merged);

        Ok(result)
    }

    /// Process a statement in the entry module, handling renames and reassignments
    fn process_entry_module_statement(
        &mut self,
        stmt: &mut Stmt,
        entry_module_renames: &FxIndexMap<String, String>,
        final_body: &mut Vec<Stmt>,
    ) {
        // For non-import statements in the entry module, apply symbol renames
        let mut pending_reassignment: Option<(String, String)> = None;

        if !entry_module_renames.is_empty() {
            // We need special handling for different statement types
            match stmt {
                Stmt::FunctionDef(func_def) => {
                    pending_reassignment =
                        self.process_entry_module_function(func_def, entry_module_renames);
                }
                Stmt::ClassDef(class_def) => {
                    pending_reassignment =
                        self.process_entry_module_class(class_def, entry_module_renames);
                }
                _ => {
                    // For other statements, use the existing rewrite method
                    self.rewrite_aliases_in_stmt(stmt, entry_module_renames);

                    // Check if this is an assignment that was renamed
                    if let Stmt::Assign(assign) = &stmt {
                        pending_reassignment =
                            self.check_renamed_assignment(assign, entry_module_renames);
                    }
                }
            }
        }

        final_body.push(stmt.clone());

        // Add reassignment if needed, but skip if original and renamed are the same
        // or if the reassignment already exists
        if let Some((original, renamed)) = pending_reassignment
            && original != renamed
        {
            // Check if this reassignment already exists in final_body
            let assignment_exists = final_body.iter().any(|stmt| {
                if let Stmt::Assign(assign) = stmt {
                    if assign.targets.len() == 1 {
                        if let (Expr::Name(target), Expr::Name(value)) =
                            (&assign.targets[0], assign.value.as_ref())
                        {
                            target.id.as_str() == original && value.id.as_str() == renamed
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            });

            if !assignment_exists {
                let reassign = self.create_reassignment(&original, &renamed);
                final_body.push(reassign);
            }
        }
    }

    /// Process a function definition in the entry module
    fn process_entry_module_function(
        &self,
        func_def: &mut StmtFunctionDef,
        entry_module_renames: &FxIndexMap<String, String>,
    ) -> Option<(String, String)> {
        let func_name = func_def.name.to_string();
        let needs_reassignment = if let Some(new_name) = entry_module_renames.get(&func_name) {
            log::debug!("Renaming function '{func_name}' to '{new_name}' in entry module");
            func_def.name = Identifier::new(new_name, TextRange::default());
            true
        } else {
            false
        };

        // For function bodies, we need special handling:
        // - Global statements must be renamed to match module-level renames
        // - But other references should NOT be renamed (Python resolves at runtime)
        self.rewrite_global_statements_in_function(func_def, entry_module_renames);

        if needs_reassignment {
            Some((func_name.clone(), entry_module_renames[&func_name].clone()))
        } else {
            None
        }
    }

    /// Process a class definition in the entry module
    fn process_entry_module_class(
        &self,
        class_def: &mut StmtClassDef,
        entry_module_renames: &FxIndexMap<String, String>,
    ) -> Option<(String, String)> {
        let class_name = class_def.name.to_string();
        let needs_reassignment = if let Some(new_name) = entry_module_renames.get(&class_name) {
            log::debug!("Renaming class '{class_name}' to '{new_name}' in entry module");
            class_def.name = Identifier::new(new_name, TextRange::default());
            true
        } else {
            false
        };

        // Apply renames to class body - classes don't create new scopes for globals
        // We need to create a temporary Stmt to pass to rewrite_aliases_in_stmt
        let mut temp_stmt = Stmt::ClassDef(class_def.clone());
        self.rewrite_aliases_in_stmt(&mut temp_stmt, entry_module_renames);
        if let Stmt::ClassDef(updated_class) = temp_stmt {
            *class_def = updated_class;
        }

        if needs_reassignment {
            Some((
                class_name.clone(),
                entry_module_renames[&class_name].clone(),
            ))
        } else {
            None
        }
    }

    /// Check if an assignment statement has been renamed
    fn check_renamed_assignment(
        &self,
        assign: &StmtAssign,
        entry_module_renames: &FxIndexMap<String, String>,
    ) -> Option<(String, String)> {
        if assign.targets.len() != 1 {
            return None;
        }

        let Expr::Name(name_expr) = &assign.targets[0] else {
            return None;
        };

        let assigned_name = name_expr.id.as_str();
        // Check if this is a renamed variable (e.g., Logger_1)
        for (original, renamed) in entry_module_renames {
            if assigned_name == renamed {
                // This is a renamed assignment, mark for reassignment
                return Some((original.clone(), renamed.clone()));
            }
        }
        None
    }

    /// Transform a module into an initialization function
    fn transform_module_to_init_function(
        &self,
        ctx: ModuleTransformContext,
        mut ast: ModModule,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Result<Stmt> {
        let init_func_name = &self.init_functions[ctx.synthetic_name];
        let mut body = Vec::new();

        // Create module object (returns multiple statements)
        body.extend(self.create_module_object_stmt(ctx.synthetic_name, ctx.module_path));

        // Apply globals lifting if needed
        let lifted_names = if let Some(ref global_info) = ctx.global_info {
            if !global_info.global_declarations.is_empty() {
                let globals_lifter = GlobalsLifter::new(global_info);
                let lifted_names = globals_lifter.get_lifted_names().clone();

                // Transform the AST to use lifted globals
                self.transform_ast_with_lifted_globals(&mut ast, &lifted_names, global_info);

                Some(lifted_names)
            } else {
                None
            }
        } else {
            None
        };

        // First, recursively transform all imports in the AST
        // For wrapper modules, we don't need to defer imports since they run in their own scope
        let mut wrapper_deferred_imports = Vec::new();
        let mut transformer = RecursiveImportTransformer::new(RecursiveImportTransformerParams {
            bundler: self,
            module_name: ctx.module_name,
            module_path: Some(ctx.module_path),
            symbol_renames,
            deferred_imports: &mut wrapper_deferred_imports,
            is_entry_module: false,        // This is not the entry module
            is_wrapper_init: true,         // This IS a wrapper init function
            global_deferred_imports: None, // No need for global deferred imports in wrapper modules
        });

        // Track imports from inlined modules before transformation
        let mut imports_from_inlined = Vec::new();
        for stmt in &ast.body {
            if let Stmt::ImportFrom(import_from) = stmt {
                // Resolve the module to check if it's inlined
                let resolved_module = self.resolve_relative_import_with_context(
                    import_from,
                    ctx.module_name,
                    Some(ctx.module_path),
                );

                if let Some(ref module) = resolved_module {
                    // Check if the module is bundled (either inlined or wrapper)
                    let is_bundled = self.inlined_modules.contains(module)
                        || self.module_registry.contains_key(module);

                    debug!(
                        "Checking if resolved module '{}' is bundled (inlined: {}, wrapper: {})",
                        module,
                        self.inlined_modules.contains(module),
                        self.module_registry.contains_key(module)
                    );

                    if is_bundled {
                        // Track all imported names from this bundled module
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str();
                            debug!(
                                "Tracking imported name '{imported_name}' from bundled module \
                                 '{module}'"
                            );
                            imports_from_inlined.push(imported_name.to_string());
                        }
                    }
                }
            }
        }

        transformer.transform_module(&mut ast);

        // Store deferred imports to add after module body
        let deferred_imports_to_add = wrapper_deferred_imports.clone();

        // IMPORTANT: Add import alias assignments FIRST, before processing the module body
        // This ensures that aliases like 'helper_validate = validate' are available when
        // the module body code tries to use them (e.g., helper_validate.__name__)
        for stmt in &deferred_imports_to_add {
            if let Stmt::Assign(assign) = stmt {
                // Check if this is a simple name-to-name assignment (import alias)
                if let [Expr::Name(_target)] = assign.targets.as_slice()
                    && let Expr::Name(_value) = &*assign.value
                {
                    // This is an import alias assignment, add it immediately
                    body.push(stmt.clone());
                }
            }
        }

        // Collect all variables that are referenced by exported functions
        // This is needed because some private variables (like _VERSION) might be used by exported
        // functions
        let mut vars_used_by_exported_functions = FxIndexSet::default();
        for stmt in &ast.body {
            if let Stmt::FunctionDef(func_def) = stmt
                && self.should_export_symbol(func_def.name.as_ref(), ctx.module_name)
            {
                // This function will be exported, collect variables it references
                self.collect_referenced_vars(&func_def.body, &mut vars_used_by_exported_functions);
            }
        }

        // Now process the transformed module
        for stmt in ast.body {
            match &stmt {
                Stmt::Import(import_stmt) => {
                    // Skip stdlib imports that have been hoisted
                    let mut skip_stmt = false;
                    for alias in &import_stmt.names {
                        if self.is_safe_stdlib_module(alias.name.as_str()) {
                            // This stdlib import has been hoisted, skip it
                            skip_stmt = true;
                            break;
                        }
                    }

                    if !skip_stmt {
                        // Non-stdlib imports have already been transformed by
                        // RecursiveImportTransformer
                        body.push(stmt.clone());
                    }
                }
                Stmt::ImportFrom(import_from) => {
                    // Skip __future__ imports - they cannot appear inside functions
                    if import_from.module.as_ref().map(|m| m.as_str()) == Some("__future__") {
                        continue;
                    }

                    // Skip stdlib imports that have been hoisted
                    if let Some(module_name) = import_from.module.as_ref()
                        && self.is_safe_stdlib_module(module_name.as_str())
                    {
                        // This stdlib import has been hoisted, skip it
                        continue;
                    }

                    // Other imports have already been transformed by RecursiveImportTransformer
                    body.push(stmt.clone());
                }
                Stmt::ClassDef(class_def) => {
                    // Add class definition
                    body.push(stmt.clone());
                    // Set as module attribute only if it should be exported
                    let symbol_name = class_def.name.to_string();
                    if self.should_export_symbol(&symbol_name, ctx.module_name) {
                        body.push(self.create_module_attr_assignment("module", &symbol_name));
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    // Clone the function for transformation
                    let mut func_def_clone = func_def.clone();

                    // Transform nested functions to use module attributes for module-level vars
                    if let Some(ref global_info) = ctx.global_info {
                        self.transform_nested_function_for_module_vars(
                            &mut func_def_clone,
                            &global_info.module_level_vars,
                        );
                    }

                    // Add transformed function definition
                    body.push(Stmt::FunctionDef(func_def_clone));

                    // Set as module attribute only if it should be exported
                    let symbol_name = func_def.name.to_string();
                    if self.should_export_symbol(&symbol_name, ctx.module_name) {
                        body.push(self.create_module_attr_assignment("module", &symbol_name));
                    }
                }
                Stmt::Assign(assign) => {
                    // Skip self-referential assignments like `process = process`
                    // These are meaningless in the init function context and cause errors
                    if !self.is_self_referential_assignment(assign) {
                        // For simple assignments, also set as module attribute if it should be
                        // exported
                        body.push(stmt.clone());

                        // Check if this assignment came from a transformed import
                        if let Some(name) = self.extract_simple_assign_target(assign) {
                            debug!(
                                "Checking assignment '{}' in module '{}' (imports_from_inlined: \
                                 {:?})",
                                name, ctx.module_name, imports_from_inlined
                            );
                            if imports_from_inlined.contains(&name) {
                                // This was imported from an inlined module, export it
                                debug!("Exporting imported symbol '{name}' as module attribute");
                                body.push(self.create_module_attr_assignment("module", &name));
                            } else if let Some(name) = self.extract_simple_assign_target(assign) {
                                // Check if this variable is used by exported functions
                                if vars_used_by_exported_functions.contains(&name) {
                                    debug!("Exporting '{name}' as it's used by exported functions");
                                    body.push(self.create_module_attr_assignment("module", &name));
                                } else {
                                    // Regular assignment, use the normal export logic
                                    self.add_module_attr_if_exported(
                                        assign,
                                        ctx.module_name,
                                        &mut body,
                                    );
                                }
                            } else {
                                // Not a simple assignment
                                self.add_module_attr_if_exported(
                                    assign,
                                    ctx.module_name,
                                    &mut body,
                                );
                            }
                        }
                    } else {
                        log::debug!(
                            "Skipping self-referential assignment in module '{}': {:?}",
                            ctx.module_name,
                            assign.targets.first().and_then(|t| match t {
                                Expr::Name(name) => Some(name.id.as_str()),
                                _ => None,
                            })
                        );
                    }
                }
                Stmt::Try(try_stmt) => {
                    // Clone the try statement
                    let try_clone = try_stmt.clone();

                    // Process assignments in try body
                    let mut additional_exports = Vec::new();
                    for stmt in &try_stmt.body {
                        if let Stmt::Assign(assign) = stmt
                            && let Some(name) = self.extract_simple_assign_target(assign)
                            && self.should_export_symbol(&name, ctx.module_name)
                        {
                            additional_exports
                                .push(self.create_module_attr_assignment("module", &name));
                        }
                    }

                    // Process assignments in except handlers
                    for handler in &try_stmt.handlers {
                        let ExceptHandler::ExceptHandler(eh) = handler;
                        for stmt in &eh.body {
                            if let Stmt::Assign(assign) = stmt
                                && let Some(name) = self.extract_simple_assign_target(assign)
                                && self.should_export_symbol(&name, ctx.module_name)
                            {
                                additional_exports
                                    .push(self.create_module_attr_assignment("module", &name));
                            }
                        }
                    }

                    // Add the try statement
                    body.push(Stmt::Try(try_clone));

                    // Add any module attribute assignments after the try block
                    body.extend(additional_exports);
                }
                _ => {
                    // Other statements execute normally
                    body.push(stmt.clone());
                }
            }
        }

        // Initialize lifted globals if any
        if let Some(ref lifted_names) = lifted_names {
            for (original_name, lifted_name) in lifted_names {
                // global __cribo_module_var
                body.push(Stmt::Global(ruff_python_ast::StmtGlobal {
                    node_index: AtomicNodeIndex::dummy(),
                    names: vec![Identifier::new(lifted_name, TextRange::default())],
                    range: TextRange::default(),
                }));

                // __cribo_module_var = original_var
                body.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: lifted_name.clone().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: original_name.clone().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));
            }
        }

        // Set submodules as attributes on this module BEFORE processing deferred imports
        // This is needed because deferred imports may reference these submodules
        let current_module_prefix = format!("{}.", ctx.module_name);
        let mut submodules_to_add = Vec::new();

        // Collect all direct submodules
        for (module_name, _) in &self.module_registry {
            if module_name.starts_with(&current_module_prefix) {
                let relative_name = &module_name[current_module_prefix.len()..];
                // Only handle direct children, not nested submodules
                if !relative_name.contains('.') {
                    submodules_to_add.push((module_name.clone(), relative_name.to_string()));
                }
            }
        }

        // Also check inlined modules
        for module_name in &self.inlined_modules {
            if module_name.starts_with(&current_module_prefix) {
                let relative_name = &module_name[current_module_prefix.len()..];
                // Only handle direct children, not nested submodules
                if !relative_name.contains('.') {
                    submodules_to_add.push((module_name.clone(), relative_name.to_string()));
                }
            }
        }

        // Now add the submodules as attributes
        for (full_name, relative_name) in submodules_to_add {
            debug!(
                "Setting submodule {} as attribute {} on {}",
                full_name, relative_name, ctx.module_name
            );

            if self.inlined_modules.contains(&full_name) {
                // For inlined submodules, we create a types.SimpleNamespace with the exported
                // symbols
                let create_namespace_stmts = self.create_namespace_for_inlined_submodule(
                    &full_name,
                    &relative_name,
                    symbol_renames,
                );
                body.extend(create_namespace_stmts);
            } else {
                // For wrapped submodules, we'll set them up later when they're initialized
                // For now, just skip - the parent module will get the submodule reference
                // when the submodule's init function is called
            }
        }

        // Add remaining deferred imports after submodule namespaces are created
        // Skip import alias assignments since they were already added at the beginning
        for stmt in &deferred_imports_to_add {
            // Skip simple name-to-name assignments (import aliases) as they were already added
            let is_import_alias = if let Stmt::Assign(assign) = stmt {
                matches!(
                    (assign.targets.as_slice(), &*assign.value),
                    ([Expr::Name(_)], Expr::Name(_))
                )
            } else {
                false
            };

            if is_import_alias {
                continue; // Already added at the beginning
            }

            if let Stmt::Assign(assign) = stmt
                && !self.is_self_referential_assignment(assign)
            {
                // For deferred imports that are assignments, also set as module attribute if
                // exported
                body.push(stmt.clone());
                self.add_module_attr_if_exported(assign, ctx.module_name, &mut body);
            } else {
                body.push(stmt.clone());
            }
        }

        // Generate __all__ for the bundled module only if the original module had explicit __all__
        if self.modules_with_explicit_all.contains(ctx.module_name) {
            body.push(self.create_all_assignment_for_module(ctx.module_name));
        }

        // For imports from inlined modules that don't create assignments,
        // we still need to set them as module attributes if they're exported
        for imported_name in imports_from_inlined {
            if self.should_export_symbol(&imported_name, ctx.module_name) {
                // Check if we already have a module attribute assignment for this
                let already_assigned = body.iter().any(|stmt| {
                    if let Stmt::Assign(assign) = stmt
                        && let [Expr::Attribute(attr)] = assign.targets.as_slice()
                        && let Expr::Name(name) = &*attr.value
                    {
                        return name.id == "module" && attr.attr == imported_name;
                    }
                    false
                });

                if !already_assigned {
                    body.push(self.create_module_attr_assignment("module", &imported_name));
                }
            }
        }

        // Return the module object
        body.push(Stmt::Return(ruff_python_ast::StmtReturn {
            node_index: AtomicNodeIndex::dummy(),
            value: Some(Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: "module".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            }))),
            range: TextRange::default(),
        }));

        // Transform globals() calls to module.__dict__ in the entire body
        for stmt in &mut body {
            transform_globals_in_stmt(stmt);
        }

        // Create the init function
        Ok(Stmt::FunctionDef(StmtFunctionDef {
            node_index: AtomicNodeIndex::dummy(),
            name: Identifier::new(init_func_name, TextRange::default()),
            type_params: None,
            parameters: Box::new(ruff_python_ast::Parameters {
                node_index: AtomicNodeIndex::dummy(),
                posonlyargs: vec![],
                args: vec![],
                vararg: None,
                kwonlyargs: vec![],
                kwarg: None,
                range: TextRange::default(),
            }),
            returns: None,
            body,
            decorator_list: vec![],
            is_async: false,
            range: TextRange::default(),
        }))
    }

    /// Generate registries and import hook after init functions are defined
    fn generate_registries_and_hook(&self) -> Vec<Stmt> {
        // No longer needed - we don't use sys.modules or import hooks
        Vec::new()
    }

    /// Create the import hook class and install it
    fn create_import_hook(&self) -> Vec<Stmt> {
        Vec::new()
    }

    /// Create a namespace for an inlined submodule
    fn create_namespace_for_inlined_submodule(
        &self,
        full_module_name: &str,
        attr_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        let mut stmts = Vec::new();

        // Create a types.SimpleNamespace() for the inlined module
        stmts.push(Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: attr_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ruff_python_ast::ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        }));

        // Get the module exports for this inlined module
        let exported_symbols = self.module_exports.get(full_module_name).cloned().flatten();

        // Add all exported symbols from the inlined module to the namespace
        if let Some(exports) = exported_symbols {
            for symbol in exports {
                // For re-exported symbols, check if the original symbol is kept by tree-shaking
                let should_include = if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                {
                    // First check if this symbol is directly defined in this module
                    if kept_symbols.contains(&(full_module_name.to_string(), symbol.clone())) {
                        true
                    } else {
                        // If not, check if this is a re-exported symbol from another module
                        // For modules with __all__, we always include symbols that are re-exported
                        // even if they're not directly defined in the module
                        let module_has_all_export = self
                            .module_exports
                            .get(full_module_name)
                            .and_then(|exports| exports.as_ref())
                            .map(|exports| exports.contains(&symbol))
                            .unwrap_or(false);

                        if module_has_all_export {
                            log::debug!(
                                "Including re-exported symbol {symbol} from module \
                                 {full_module_name} (in __all__)"
                            );
                            true
                        } else {
                            false
                        }
                    }
                } else {
                    // No tree-shaking, include everything
                    true
                };

                if !should_include {
                    log::debug!(
                        "Skipping namespace assignment for {full_module_name}.{symbol} - removed \
                         by tree-shaking"
                    );
                    continue;
                }

                // Get the renamed version of this symbol
                let renamed_symbol =
                    if let Some(module_renames) = symbol_renames.get(full_module_name) {
                        module_renames
                            .get(&symbol)
                            .cloned()
                            .unwrap_or_else(|| symbol.clone())
                    } else {
                        symbol.clone()
                    };

                // Before creating the assignment, check if the renamed symbol exists after
                // tree-shaking
                if !self.renamed_symbol_exists(&renamed_symbol, symbol_renames) {
                    log::warn!(
                        "Skipping namespace assignment {attr_name}.{symbol} = {renamed_symbol} - \
                         renamed symbol doesn't exist after tree-shaking"
                    );
                    continue;
                }

                // attr_name.symbol = renamed_symbol
                log::debug!(
                    "Creating namespace assignment: {attr_name}.{symbol} = {renamed_symbol}"
                );
                stmts.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: attr_name.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new(&symbol, TextRange::default()),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: renamed_symbol.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));
            }
        } else {
            // If no explicit exports, we still need to check if this module defines symbols
            // This is a fallback for modules that don't have __all__ defined
            // For now, log a warning since we can't determine exports without module analysis
            log::warn!(
                "Inlined module '{full_module_name}' has no explicit exports (__all__). Namespace \
                 will be empty unless symbols are added elsewhere."
            );
        }

        stmts
    }

    /// Create an assignment to a namespace attribute
    fn create_namespace_attr_assignment(
        &self,
        namespace_var: &str,
        attr: &str,
        value: &str,
    ) -> Stmt {
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: namespace_var.to_string().into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(attr, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: value.to_string().into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create a string literal expression
    fn create_string_literal(&self, value: &str) -> Expr {
        Expr::StringLiteral(ExprStringLiteral {
            node_index: AtomicNodeIndex::dummy(),
            value: StringLiteralValue::single(ruff_python_ast::StringLiteral {
                node_index: AtomicNodeIndex::dummy(),
                value: value.to_string().into(),
                flags: ruff_python_ast::StringLiteralFlags::empty(),
                range: TextRange::default(),
            }),
            range: TextRange::default(),
        })
    }

    /// Create a number literal expression
    fn create_number_literal(&self, value: i32) -> Expr {
        Expr::NumberLiteral(ruff_python_ast::ExprNumberLiteral {
            node_index: AtomicNodeIndex::dummy(),
            value: ruff_python_ast::Number::Int(ruff_python_ast::Int::from(value as u32)),
            range: TextRange::default(),
        })
    }

    /// Create module object
    fn create_module_object_stmt(&self, _synthetic_name: &str, _module_path: &Path) -> Vec<Stmt> {
        let module_call = Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::dummy(),
            func: Box::new(Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "types".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("SimpleNamespace", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: ruff_python_ast::Arguments {
                node_index: AtomicNodeIndex::dummy(),
                args: Box::from([]),
                keywords: Box::from([]),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        vec![
            // module = types.SimpleNamespace()
            Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "module".into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(module_call),
                range: TextRange::default(),
            }),
        ]
    }

    /// Create module attribute assignment
    fn create_module_attr_assignment(&self, module_var: &str, attr_name: &str) -> Stmt {
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: module_var.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(attr_name, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: attr_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Transform nested functions to use module attributes for module-level variables
    fn transform_nested_function_for_module_vars(
        &self,
        func_def: &mut StmtFunctionDef,
        module_level_vars: &rustc_hash::FxHashSet<String>,
    ) {
        // Transform the function body
        for stmt in &mut func_def.body {
            self.transform_stmt_for_module_vars(stmt, module_level_vars);
        }
    }

    /// Transform a statement to use module attributes for module-level variables
    fn transform_stmt_for_module_vars(
        &self,
        stmt: &mut Stmt,
        module_level_vars: &rustc_hash::FxHashSet<String>,
    ) {
        match stmt {
            Stmt::FunctionDef(nested_func) => {
                // Recursively transform nested functions
                self.transform_nested_function_for_module_vars(nested_func, module_level_vars);
            }
            Stmt::Assign(assign) => {
                // Transform assignment targets and values
                for target in &mut assign.targets {
                    self.transform_expr_for_module_vars(target, module_level_vars);
                }
                self.transform_expr_for_module_vars(&mut assign.value, module_level_vars);
            }
            Stmt::Expr(expr_stmt) => {
                self.transform_expr_for_module_vars(&mut expr_stmt.value, module_level_vars);
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    self.transform_expr_for_module_vars(value, module_level_vars);
                }
            }
            Stmt::If(if_stmt) => {
                self.transform_expr_for_module_vars(&mut if_stmt.test, module_level_vars);
                for stmt in &mut if_stmt.body {
                    self.transform_stmt_for_module_vars(stmt, module_level_vars);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(condition) = &mut clause.test {
                        self.transform_expr_for_module_vars(condition, module_level_vars);
                    }
                    for stmt in &mut clause.body {
                        self.transform_stmt_for_module_vars(stmt, module_level_vars);
                    }
                }
            }
            Stmt::For(for_stmt) => {
                self.transform_expr_for_module_vars(&mut for_stmt.target, module_level_vars);
                self.transform_expr_for_module_vars(&mut for_stmt.iter, module_level_vars);
                for stmt in &mut for_stmt.body {
                    self.transform_stmt_for_module_vars(stmt, module_level_vars);
                }
            }
            _ => {
                // Handle other statement types as needed
            }
        }
    }

    /// Transform an expression to use module attributes for module-level variables
    #[allow(clippy::only_used_in_recursion)]
    fn transform_expr_for_module_vars(
        &self,
        expr: &mut Expr,
        module_level_vars: &rustc_hash::FxHashSet<String>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                // If this is a module-level variable being read, transform to module.var
                if module_level_vars.contains(name_expr.id.as_str())
                    && matches!(name_expr.ctx, ExprContext::Load)
                {
                    // Transform foo -> module.foo
                    *expr = Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: "module".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new(name_expr.id.as_str(), TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
                }
            }
            Expr::Call(call) => {
                self.transform_expr_for_module_vars(&mut call.func, module_level_vars);
                for arg in &mut call.arguments.args {
                    self.transform_expr_for_module_vars(arg, module_level_vars);
                }
                for keyword in &mut call.arguments.keywords {
                    self.transform_expr_for_module_vars(&mut keyword.value, module_level_vars);
                }
            }
            Expr::BinOp(binop) => {
                self.transform_expr_for_module_vars(&mut binop.left, module_level_vars);
                self.transform_expr_for_module_vars(&mut binop.right, module_level_vars);
            }
            Expr::Dict(dict) => {
                for item in &mut dict.items {
                    if let Some(key) = &mut item.key {
                        self.transform_expr_for_module_vars(key, module_level_vars);
                    }
                    self.transform_expr_for_module_vars(&mut item.value, module_level_vars);
                }
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    self.transform_expr_for_module_vars(elem, module_level_vars);
                }
            }
            Expr::Attribute(attr) => {
                self.transform_expr_for_module_vars(&mut attr.value, module_level_vars);
            }
            Expr::Subscript(subscript) => {
                self.transform_expr_for_module_vars(&mut subscript.value, module_level_vars);
                self.transform_expr_for_module_vars(&mut subscript.slice, module_level_vars);
            }
            _ => {
                // Handle other expression types as needed
            }
        }
    }

    /// Extract the full attribute path from an ExprAttribute
    /// e.g., services.auth.manager.User -> "services.auth.manager.User"
    fn extract_attribute_path(&self, attr: &ExprAttribute) -> String {
        let mut parts = vec![attr.attr.as_str()];
        let mut current = &attr.value;

        loop {
            match current.as_ref() {
                Expr::Attribute(inner_attr) => {
                    parts.push(inner_attr.attr.as_str());
                    current = &inner_attr.value;
                }
                Expr::Name(name) => {
                    parts.push(name.id.as_str());
                    break;
                }
                _ => break,
            }
        }

        parts.reverse();
        parts.join(".")
    }

    /// Deduplicate deferred import statements
    /// This prevents duplicate init calls and symbol assignments
    fn deduplicate_deferred_imports(&self, imports: Vec<Stmt>) -> Vec<Stmt> {
        let mut seen_init_calls = FxIndexSet::default();
        let mut seen_assignments = FxIndexSet::default();
        let mut result = Vec::new();

        log::debug!("Deduplicating {} deferred imports", imports.len());

        for (idx, stmt) in imports.into_iter().enumerate() {
            log::debug!("Processing deferred import {idx}: {stmt:?}");
            match &stmt {
                // Check for init function calls
                Stmt::Expr(expr_stmt) => {
                    if let Expr::Call(call) = &expr_stmt.value.as_ref() {
                        if let Expr::Name(name) = &call.func.as_ref() {
                            let func_name = name.id.as_str();
                            if func_name.starts_with("__cribo_init_") {
                                if seen_init_calls.insert(func_name.to_string()) {
                                    result.push(stmt);
                                } else {
                                    log::debug!("Skipping duplicate init call: {func_name}");
                                }
                            } else {
                                result.push(stmt);
                            }
                        } else {
                            result.push(stmt);
                        }
                    } else {
                        result.push(stmt);
                    }
                }
                // Check for symbol assignments like: symbol = sys.modules['module'].symbol
                Stmt::Assign(assign) => {
                    // Check if this is an assignment like: UserSchema =
                    // sys.modules['schemas.user'].UserSchema
                    if let Expr::Attribute(attr) = &assign.value.as_ref() {
                        if let Expr::Subscript(subscript) = &attr.value.as_ref() {
                            if let Expr::Attribute(sys_attr) = &subscript.value.as_ref() {
                                if let Expr::Name(sys_name) = &sys_attr.value.as_ref() {
                                    if sys_name.id.as_str() == "sys"
                                        && sys_attr.attr.as_str() == "modules"
                                    {
                                        // This is a sys.modules access
                                        if let Expr::StringLiteral(lit) = &subscript.slice.as_ref()
                                        {
                                            let module_name = lit.value.to_str();
                                            let attr_name = attr.attr.as_str();
                                            if let Expr::Name(target) = &assign.targets[0] {
                                                let symbol_name = target.id.as_str();
                                                // Include the target variable name in the key to
                                                // properly deduplicate
                                                // assignments like User =
                                                // services.auth.manager.User
                                                let key = format!(
                                                    "{symbol_name} = {module_name}.{attr_name}"
                                                );
                                                log::debug!("Checking assignment key: {key}");
                                                if seen_assignments.insert(key.clone()) {
                                                    log::debug!(
                                                        "First occurrence of {key}, including"
                                                    );
                                                    result.push(stmt);
                                                } else {
                                                    log::debug!(
                                                        "Skipping duplicate assignment: \
                                                         {symbol_name} = \
                                                         sys.modules['{module_name}'].{attr_name}"
                                                    );
                                                }
                                            } else {
                                                result.push(stmt);
                                            }
                                        } else {
                                            result.push(stmt);
                                        }
                                    } else {
                                        result.push(stmt);
                                    }
                                } else {
                                    result.push(stmt);
                                }
                            } else {
                                result.push(stmt);
                            }
                        } else {
                            result.push(stmt);
                        }
                    } else {
                        // Check for simple assignments like: Logger = Logger_4
                        if assign.targets.len() == 1 {
                            if let Expr::Name(target) = &assign.targets[0] {
                                if let Expr::Name(value) = &assign.value.as_ref() {
                                    // This is a simple name assignment
                                    let target_str = target.id.as_str();
                                    let value_str = value.id.as_str();
                                    let key = format!("{target_str} = {value_str}");

                                    // Check for self-assignment
                                    if target_str == value_str {
                                        log::warn!(
                                            "Found self-assignment in deferred imports: {key}"
                                        );
                                        // Skip self-assignments entirely
                                        log::debug!("Skipping self-assignment: {key}");
                                    } else if seen_assignments.insert(key.clone()) {
                                        log::debug!("First occurrence of simple assignment: {key}");
                                        result.push(stmt);
                                    } else {
                                        log::debug!("Skipping duplicate simple assignment: {key}");
                                    }
                                } else {
                                    // Not a simple name assignment, check for duplicates
                                    // Handle attribute assignments like User =
                                    // services.auth.manager.User
                                    let target_str = target.id.as_str();

                                    // For attribute assignments, extract the actual attribute path
                                    let key = if let Expr::Attribute(attr) = &assign.value.as_ref()
                                    {
                                        // Extract the full attribute path (e.g.,
                                        // services.auth.manager.User)
                                        let attr_path = self.extract_attribute_path(attr);
                                        format!("{target_str} = {attr_path}")
                                    } else {
                                        // Fallback to debug format for other types
                                        let value_str = format!("{:?}", assign.value);
                                        format!("{target_str} = {value_str}")
                                    };

                                    if seen_assignments.insert(key.clone()) {
                                        log::debug!(
                                            "First occurrence of attribute assignment: {key}"
                                        );
                                        result.push(stmt);
                                    } else {
                                        log::debug!(
                                            "Skipping duplicate attribute assignment: {key}"
                                        );
                                    }
                                }
                            } else {
                                // Target is not a simple name, include it
                                result.push(stmt);
                            }
                        } else {
                            // Multiple targets, include it
                            result.push(stmt);
                        }
                    }
                }
                _ => result.push(stmt),
            }
        }

        result
    }

    /// Deduplicate deferred imports, checking against existing statements in the body
    fn deduplicate_deferred_imports_with_existing(
        &self,
        imports: Vec<Stmt>,
        existing_body: &[Stmt],
    ) -> Vec<Stmt> {
        let mut seen_init_calls = FxIndexSet::default();
        let mut seen_assignments = FxIndexSet::default();
        let mut result = Vec::new();

        // First, collect all existing assignments from the body
        for stmt in existing_body {
            if let Stmt::Assign(assign) = stmt
                && assign.targets.len() == 1
                && let Expr::Name(target) = &assign.targets[0]
            {
                let target_str = target.id.as_str();

                // Handle simple name assignments
                if let Expr::Name(value) = &assign.value.as_ref() {
                    let key = format!("{} = {}", target_str, value.id.as_str());
                    seen_assignments.insert(key);
                }
                // Handle attribute assignments like User = services.auth.manager.User
                else if let Expr::Attribute(attr) = &assign.value.as_ref() {
                    let attr_path = self.extract_attribute_path(attr);
                    let key = format!("{target_str} = {attr_path}");
                    seen_assignments.insert(key);
                }
            }
        }

        log::debug!(
            "Found {} existing assignments in body",
            seen_assignments.len()
        );
        log::debug!("Deduplicating {} deferred imports", imports.len());

        // Now process the deferred imports
        for (idx, stmt) in imports.into_iter().enumerate() {
            log::debug!("Processing deferred import {idx}: {stmt:?}");
            match &stmt {
                // Check for init function calls
                Stmt::Expr(expr_stmt) => {
                    if let Expr::Call(call) = &expr_stmt.value.as_ref() {
                        if let Expr::Name(name) = &call.func.as_ref() {
                            let func_name = name.id.as_str();
                            if func_name.starts_with("__cribo_init_") {
                                if seen_init_calls.insert(func_name.to_string()) {
                                    result.push(stmt);
                                } else {
                                    log::debug!("Skipping duplicate init call: {func_name}");
                                }
                            } else {
                                result.push(stmt);
                            }
                        } else {
                            result.push(stmt);
                        }
                    } else {
                        result.push(stmt);
                    }
                }
                // Check for symbol assignments
                Stmt::Assign(assign) => {
                    // Check if this is an assignment like: UserSchema =
                    // sys.modules['schemas.user'].UserSchema
                    if let Expr::Attribute(attr) = &assign.value.as_ref() {
                        if let Expr::Subscript(subscript) = &attr.value.as_ref() {
                            if let Expr::Attribute(sys_attr) = &subscript.value.as_ref() {
                                if let Expr::Name(sys_name) = &sys_attr.value.as_ref() {
                                    if sys_name.id.as_str() == "sys"
                                        && sys_attr.attr.as_str() == "modules"
                                    {
                                        // This is a sys.modules access
                                        if let Expr::StringLiteral(lit) = &subscript.slice.as_ref()
                                        {
                                            let module_name = lit.value.to_str();
                                            let attr_name = attr.attr.as_str();
                                            if let Expr::Name(target) = &assign.targets[0] {
                                                let symbol_name = target.id.as_str();
                                                // Include the target variable name in the key to
                                                // properly deduplicate
                                                // assignments like User =
                                                // services.auth.manager.User
                                                let key = format!(
                                                    "{symbol_name} = {module_name}.{attr_name}"
                                                );
                                                log::debug!("Checking assignment key: {key}");
                                                if seen_assignments.insert(key.clone()) {
                                                    log::debug!(
                                                        "First occurrence of {key}, including"
                                                    );
                                                    result.push(stmt);
                                                } else {
                                                    log::debug!(
                                                        "Skipping duplicate assignment: \
                                                         {symbol_name} = \
                                                         sys.modules['{module_name}'].{attr_name}"
                                                    );
                                                }
                                            } else {
                                                result.push(stmt);
                                            }
                                        } else {
                                            result.push(stmt);
                                        }
                                    } else {
                                        result.push(stmt);
                                    }
                                } else {
                                    result.push(stmt);
                                }
                            } else {
                                result.push(stmt);
                            }
                        } else {
                            result.push(stmt);
                        }
                    } else {
                        // Check for simple assignments like: Logger = Logger_4
                        if assign.targets.len() == 1 {
                            if let Expr::Name(target) = &assign.targets[0] {
                                if let Expr::Name(value) = &assign.value.as_ref() {
                                    // This is a simple name assignment
                                    let target_str = target.id.as_str();
                                    let value_str = value.id.as_str();
                                    let key = format!("{target_str} = {value_str}");

                                    // Check for self-assignment
                                    if target_str == value_str {
                                        log::warn!(
                                            "Found self-assignment in deferred imports: {key}"
                                        );
                                        // Skip self-assignments entirely
                                        log::debug!("Skipping self-assignment: {key}");
                                    } else if seen_assignments.insert(key.clone()) {
                                        log::debug!("First occurrence of simple assignment: {key}");
                                        result.push(stmt);
                                    } else {
                                        log::debug!("Skipping duplicate simple assignment: {key}");
                                    }
                                } else {
                                    // Not a simple name assignment, check for duplicates
                                    // Handle attribute assignments like User =
                                    // services.auth.manager.User
                                    let target_str = target.id.as_str();

                                    // For attribute assignments, extract the actual attribute path
                                    let key = if let Expr::Attribute(attr) = &assign.value.as_ref()
                                    {
                                        // Extract the full attribute path (e.g.,
                                        // services.auth.manager.User)
                                        let attr_path = self.extract_attribute_path(attr);
                                        format!("{target_str} = {attr_path}")
                                    } else {
                                        // Fallback to debug format for other types
                                        let value_str = format!("{:?}", assign.value);
                                        format!("{target_str} = {value_str}")
                                    };

                                    if seen_assignments.insert(key.clone()) {
                                        log::debug!(
                                            "First occurrence of attribute assignment: {key}"
                                        );
                                        result.push(stmt);
                                    } else {
                                        log::debug!(
                                            "Skipping duplicate attribute assignment: {key}"
                                        );
                                    }
                                }
                            } else {
                                // Target is not a simple name, include it
                                result.push(stmt);
                            }
                        } else {
                            // Multiple targets, include it
                            result.push(stmt);
                        }
                    }
                }
                _ => result.push(stmt),
            }
        }

        result
    }

    /// Extract simple assignment target
    fn extract_simple_assign_target(&self, assign: &StmtAssign) -> Option<String> {
        if assign.targets.len() == 1
            && let Expr::Name(name) = &assign.targets[0]
        {
            return Some(name.id.to_string());
        }
        None
    }

    /// Add module attribute assignment if the symbol should be exported
    fn add_module_attr_if_exported(
        &self,
        assign: &StmtAssign,
        module_name: &str,
        body: &mut Vec<Stmt>,
    ) {
        if let Some(name) = self.extract_simple_assign_target(assign)
            && self.should_export_symbol(&name, module_name)
        {
            body.push(self.create_module_attr_assignment("module", &name));
        }
    }

    /// Generate a call to initialize a module
    fn generate_module_init_call(&mut self, synthetic_name: &str) -> Vec<Stmt> {
        let mut statements = Vec::new();

        if let Some(init_func_name) = self.init_functions.get(synthetic_name) {
            // Get the original module name for this synthetic name
            let module_name = self
                .module_registry
                .iter()
                .find(|(_, syn_name)| syn_name == &synthetic_name)
                .map(|(orig_name, _)| orig_name.as_str())
                .unwrap_or(synthetic_name);

            // Always use temporary variables for wrapper modules to avoid overwriting namespaces
            // Create a temporary variable name for the module
            let temp_var_name = format!("_cribo_temp_{}", module_name.cow_replace(".", "_"));

            // _cribo_temp_module_name = __cribo_init_synthetic_name()
            statements.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: temp_var_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Call(ExprCall {
                    node_index: AtomicNodeIndex::dummy(),
                    func: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: init_func_name.as_str().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    arguments: ruff_python_ast::Arguments {
                        node_index: AtomicNodeIndex::dummy(),
                        args: Box::from([]),
                        keywords: Box::from([]),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }));
        } else {
            statements.push(Stmt::Pass(ruff_python_ast::StmtPass {
                node_index: AtomicNodeIndex::dummy(),
                range: TextRange::default(),
            }));
        }

        statements
    }

    /// Get modules imported directly in the entry module
    fn get_entry_module_imports(
        &self,
        modules: &[(String, ModModule, PathBuf, String)],
        entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut imported_modules = FxIndexSet::default();

        // Find the entry module
        for (module_name, ast, _, _) in modules {
            if module_name == entry_module_name {
                // Check all import statements
                for stmt in &ast.body {
                    if let Stmt::Import(import_stmt) = stmt {
                        for alias in &import_stmt.names {
                            let module_name = alias.name.as_str();
                            // Track both dotted and non-dotted wrapper modules
                            if self.module_registry.contains_key(module_name) {
                                debug!("Entry module imports wrapper module: {module_name}");
                                imported_modules.insert(module_name.to_string());
                            }
                        }
                    }
                }
                break;
            }
        }

        debug!("Entry module imported modules: {imported_modules:?}");
        imported_modules
    }

    /// Generate statements to attach sub-modules to their parent modules
    fn generate_submodule_attributes(
        &self,
        sorted_modules: &[(String, PathBuf, Vec<String>)],
        final_body: &mut Vec<Stmt>,
    ) {
        let empty_exclusions = FxIndexSet::default();
        self.generate_submodule_attributes_with_exclusions(
            sorted_modules,
            final_body,
            &empty_exclusions,
        );
    }

    /// Generate statements to attach sub-modules to their parent modules with exclusions
    fn generate_submodule_attributes_with_exclusions(
        &self,
        sorted_modules: &[(String, PathBuf, Vec<String>)],
        final_body: &mut Vec<Stmt>,
        exclusions: &FxIndexSet<String>,
    ) {
        debug!(
            "generate_submodule_attributes: Starting with {} modules",
            sorted_modules.len()
        );

        // Step 1: Identify all namespaces and modules that need to be created/assigned
        let mut namespace_modules = FxIndexSet::default(); // Simple namespace modules to create
        let mut module_assignments = Vec::new(); // (depth, parent, attr, module_name)

        // First, collect ALL modules that have been initialized (both wrapper and namespace)
        let mut all_initialized_modules = FxIndexSet::default();

        // Add all wrapper modules
        for (module_name, _, _) in sorted_modules {
            if self.module_registry.contains_key(module_name) {
                all_initialized_modules.insert(module_name.clone());
            }
        }

        // Now analyze what namespaces are needed based on all initialized modules
        for module_name in &all_initialized_modules {
            if !module_name.contains('.') {
                continue;
            }

            // This is a dotted module - ensure all parent namespaces exist
            let parts: Vec<&str> = module_name.split('.').collect();

            // Collect all parent levels that need to exist
            for i in 1..parts.len() {
                let parent_path = parts[..i].join(".");

                // If this parent is not already an initialized module, it's a namespace that needs
                // to be created
                if !all_initialized_modules.contains(&parent_path) {
                    if i == 1 {
                        // Top-level namespace (e.g., 'core', 'models', 'services')
                        namespace_modules.insert(parent_path);
                    } else {
                        // Intermediate namespace (e.g., 'core.database')
                        // These will be created as attributes after their parent exists
                        let parent = parts[..i - 1].join(".");
                        let attr = parts[i - 1];
                        module_assignments.push((i, parent, attr.to_string(), parent_path));
                    }
                }
            }
        }

        // Add wrapper module assignments
        for module_name in &all_initialized_modules {
            if !module_name.contains('.') {
                continue;
            }

            let parts: Vec<&str> = module_name.split('.').collect();
            let parent = parts[..parts.len() - 1].join(".");
            let attr = parts[parts.len() - 1];

            // Only add if this is actually a wrapper module
            if self.module_registry.contains_key(module_name) {
                module_assignments.push((
                    parts.len(),
                    parent,
                    attr.to_string(),
                    module_name.clone(),
                ));
            }
        }

        // Step 2: Create top-level namespace modules and wrapper module references
        let mut created_namespaces = FxIndexSet::default();

        // Add all namespaces that were already created via the namespace tracking index
        for namespace in &self.required_namespaces {
            created_namespaces.insert(namespace.clone());
        }

        // First, create references to top-level wrapper modules
        let mut top_level_wrappers = Vec::new();
        for module_name in &all_initialized_modules {
            if !module_name.contains('.') && self.module_registry.contains_key(module_name) {
                // This is a top-level wrapper module
                top_level_wrappers.push(module_name.clone());
            }
        }
        top_level_wrappers.sort(); // Deterministic order

        for wrapper in top_level_wrappers {
            // Skip if this module is imported in the entry module
            if exclusions.contains(&wrapper) {
                debug!("Skipping top-level wrapper '{wrapper}' - imported in entry module");
                created_namespaces.insert(wrapper);
                continue;
            }

            debug!("Top-level wrapper '{wrapper}' already initialized, skipping assignment");
            // Top-level wrapper modules are already initialized via their init functions
            // No need to create any assignment - the module already exists
            created_namespaces.insert(wrapper);
        }

        // Then, create namespace modules
        let mut sorted_namespaces: Vec<String> = namespace_modules.into_iter().collect();
        sorted_namespaces.sort(); // Deterministic order

        for namespace in sorted_namespaces {
            // Skip if this namespace was already created via the namespace tracking index
            if self.required_namespaces.contains(&namespace) {
                debug!(
                    "Skipping top-level namespace '{namespace}' - already created via namespace \
                     index"
                );
                created_namespaces.insert(namespace);
                continue;
            }

            debug!("Creating top-level namespace: {namespace}");
            final_body.extend(self.create_namespace_module(&namespace));
            created_namespaces.insert(namespace);
        }

        // Step 3: Sort module assignments by depth to ensure parents exist before children
        module_assignments.sort_by_key(|(depth, parent, attr, name)| {
            (*depth, parent.clone(), attr.clone(), name.clone())
        });

        // Step 4: Process all assignments in order
        for (depth, parent, attr, module_name) in module_assignments {
            debug!("Processing assignment: {parent}.{attr} = {module_name} (depth={depth})");

            // Check if parent exists or will exist
            let parent_exists = created_namespaces.contains(&parent)
                || self.module_registry.contains_key(&parent)
                || parent.is_empty(); // Empty parent means top-level

            if !parent_exists {
                debug!("Warning: Parent '{parent}' doesn't exist for assignment {parent}.{attr}");
                continue;
            }

            if self.module_registry.contains_key(&module_name) {
                // Check if parent module has this attribute in __all__ (indicating a re-export)
                // OR if the parent is a wrapper module and the attribute is already defined there
                let skip_assignment = if let Some(Some(parent_exports)) =
                    self.module_exports.get(&parent)
                {
                    if parent_exports.contains(&attr) {
                        // Check if this is a symbol re-exported from within the parent module
                        // rather than the submodule itself
                        // For example, in mypackage/__init__.py:
                        // from .config import config  # imports the 'config' instance, not the
                        // module __all__ = ['config']        # exports the
                        // instance

                        // In this case, 'config' in parent_exports refers to an imported symbol,
                        // not the submodule 'mypackage.config'
                        debug!(
                            "Skipping submodule assignment for {parent}.{attr} - it's a \
                             re-exported attribute (not the module itself)"
                        );
                        true
                    } else {
                        false
                    }
                } else if self.module_registry.contains_key(&parent) {
                    // Parent is a wrapper module - check if it already has this attribute defined
                    // This handles cases where the wrapper module imports a symbol with the same
                    // name as a submodule (e.g., from .config import config)
                    debug!(
                        "Parent {parent} is a wrapper module, checking if {attr} is already \
                         defined there"
                    );
                    // For now, we'll check if the attribute is in parent_exports
                    // This may need refinement based on more complex cases
                    false
                } else {
                    false
                };

                if !skip_assignment {
                    // Check if this module was imported in the entry module
                    if exclusions.contains(&module_name) {
                        debug!(
                            "Skipping wrapper module assignment '{parent}.{attr} = {module_name}' \
                             - imported in entry module"
                        );
                    } else {
                        // Check if this would be a redundant self-assignment
                        let full_target = format!("{parent}.{attr}");
                        if full_target == module_name {
                            debug!(
                                "Skipping redundant self-assignment: {parent}.{attr} = \
                                 {module_name}"
                            );
                        } else {
                            // This is a wrapper module - assign direct reference
                            debug!("Assigning wrapper module: {parent}.{attr} = {module_name}");
                            final_body.push(self.create_dotted_attribute_assignment(
                                &parent,
                                &attr,
                                &module_name,
                            ));
                        }
                    }
                }
            } else {
                // This is an intermediate namespace - skip if already created via namespace index
                if self.required_namespaces.contains(&module_name) {
                    debug!(
                        "Skipping intermediate namespace '{module_name}' - already created via \
                         namespace index"
                    );
                    created_namespaces.insert(module_name);
                    continue;
                }

                debug!(
                    "Creating intermediate namespace: {parent}.{attr} = types.SimpleNamespace()"
                );
                final_body.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: parent.clone().into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new(&attr, TextRange::default()),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Call(ExprCall {
                        node_index: AtomicNodeIndex::dummy(),
                        func: Box::new(Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: "types".into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            attr: Identifier::new("SimpleNamespace", TextRange::default()),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        arguments: ruff_python_ast::Arguments {
                            node_index: AtomicNodeIndex::dummy(),
                            args: Box::from([]),
                            keywords: Box::from([]),
                            range: TextRange::default(),
                        },
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));

                // Set the __name__ attribute
                final_body.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: parent.clone().into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            attr: Identifier::new(&attr, TextRange::default()),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("__name__", TextRange::default()),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        value: StringLiteralValue::single(StringLiteral {
                            node_index: AtomicNodeIndex::dummy(),
                            value: module_name.to_string().into(),
                            range: TextRange::default(),
                            flags: StringLiteralFlags::empty(),
                        }),
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));

                created_namespaces.insert(module_name);
            }
        }
    }

    /// Add hoisted imports to the final body
    fn add_hoisted_imports(&self, final_body: &mut Vec<Stmt>) {
        // Future imports first - combine all into a single import statement
        if !self.future_imports.is_empty() {
            // Sort future imports for deterministic output
            let mut sorted_imports: Vec<String> = self.future_imports.iter().cloned().collect();
            sorted_imports.sort();

            let aliases: Vec<ruff_python_ast::Alias> = sorted_imports
                .into_iter()
                .map(|import| ruff_python_ast::Alias {
                    node_index: AtomicNodeIndex::dummy(),
                    name: Identifier::new(&import, TextRange::default()),
                    asname: None,
                    range: TextRange::default(),
                })
                .collect();

            final_body.push(Stmt::ImportFrom(StmtImportFrom {
                node_index: AtomicNodeIndex::dummy(),
                module: Some(Identifier::new("__future__", TextRange::default())),
                names: aliases,
                level: 0,
                range: TextRange::default(),
            }));
        }

        // Then stdlib from imports - deduplicated and sorted by module name
        let mut sorted_modules: Vec<_> = self.stdlib_import_from_map.iter().collect();
        sorted_modules.sort_by_key(|(module_name, _)| *module_name);

        for (module_name, imported_names) in sorted_modules {
            // Sort the imported names for deterministic output
            let mut sorted_names: Vec<String> = imported_names.iter().cloned().collect();
            sorted_names.sort();

            let aliases: Vec<ruff_python_ast::Alias> = sorted_names
                .into_iter()
                .map(|name| ruff_python_ast::Alias {
                    node_index: AtomicNodeIndex::dummy(),
                    name: Identifier::new(&name, TextRange::default()),
                    asname: None,
                    range: TextRange::default(),
                })
                .collect();

            final_body.push(Stmt::ImportFrom(StmtImportFrom {
                node_index: AtomicNodeIndex::dummy(),
                module: Some(Identifier::new(module_name, TextRange::default())),
                names: aliases,
                level: 0,
                range: TextRange::default(),
            }));
        }

        // NOTE: We do NOT hoist third-party imports because they may have side effects.
        // Only stdlib imports that are known to be side-effect-free are hoisted.
        // Third-party imports remain in their original location (inside wrapper functions
        // or at module level for inlined modules) to preserve execution order and
        // potential side effects.

        // Regular stdlib import statements - deduplicated and sorted by module name
        let mut seen_modules = FxIndexSet::default();
        let mut unique_imports = Vec::new();

        for stmt in &self.stdlib_import_statements {
            if let Stmt::Import(import_stmt) = stmt {
                self.collect_unique_imports(import_stmt, &mut seen_modules, &mut unique_imports);
            }
        }

        // Sort by module name for deterministic output
        unique_imports.sort_by_key(|(module_name, _)| module_name.clone());

        for (_, import_stmt) in unique_imports {
            final_body.push(import_stmt);
        }

        // NOTE: We do NOT hoist third-party regular import statements for the same reason
        // as above - they may have side effects and should remain in their original context.
    }

    /// Collect imports from a module for hoisting
    fn collect_imports_from_module(
        &mut self,
        ast: &ModModule,
        current_module: &str,
        module_path: &Path,
    ) {
        for stmt in &ast.body {
            match stmt {
                Stmt::ImportFrom(import_from) => {
                    self.collect_import_from(import_from, stmt, current_module, module_path);
                }
                Stmt::Import(import_stmt) => {
                    self.collect_import(import_stmt, stmt);
                }
                _ => {}
            }
        }
    }

    /// Collect ImportFrom statements
    fn collect_import_from(
        &mut self,
        import_from: &StmtImportFrom,
        _stmt: &Stmt,
        current_module: &str,
        module_path: &Path,
    ) {
        // Skip relative imports from bundled modules - they will be handled during transformation
        if import_from.level > 0 {
            // This is a relative import - we need to check if it resolves to a bundled module
            if let Some(resolved) = self.resolve_relative_import_with_context(
                import_from,
                current_module,
                Some(module_path),
            ) && self.bundled_modules.contains(&resolved)
            {
                // This is a relative import that resolves to a bundled module
                // It will be handled during module transformation, not hoisted
                log::debug!(
                    "Skipping relative import that resolves to bundled module: {} -> {}",
                    import_from
                        .module
                        .as_ref()
                        .map(|m| m.as_str())
                        .unwrap_or(""),
                    resolved
                );
                return;
            }
        }

        // Resolve relative imports to absolute module names
        let resolved_module_name = self.resolve_relative_import_with_context(
            import_from,
            current_module,
            Some(module_path),
        );

        let module_name = if let Some(ref resolved) = resolved_module_name {
            resolved.as_str()
        } else if let Some(ref module) = import_from.module {
            module.as_str()
        } else {
            return;
        };

        if module_name == "__future__" {
            for alias in &import_from.names {
                self.future_imports.insert(alias.name.to_string());
            }
        } else if self.is_safe_stdlib_module(module_name) {
            // Get or create the set of imported names for this module
            let imported_names = self
                .stdlib_import_from_map
                .entry(module_name.to_string())
                .or_default();

            // Add all imported names to the set (this automatically deduplicates)
            for alias in &import_from.names {
                imported_names.insert(alias.name.to_string());
            }
        } else if !self.is_bundled_module_or_package(module_name) {
            // This is a third-party import (not stdlib, not bundled)
            // We do NOT collect third-party imports for hoisting because they may have
            // side effects. They will remain in their original location within the module.
            log::debug!(
                "Skipping third-party import from module '{module_name}' - will not be hoisted"
            );
        }
    }

    /// Check if two import name lists match (same names with same aliases)
    fn import_names_match(
        names1: &[ruff_python_ast::Alias],
        names2: &[ruff_python_ast::Alias],
    ) -> bool {
        if names1.len() != names2.len() {
            return false;
        }

        // Check if all names match (order doesn't matter)
        names1.iter().all(|n1| {
            names2
                .iter()
                .any(|n2| n1.name == n2.name && n1.asname == n2.asname)
        })
    }

    /// Check if a module is bundled directly or is a package containing bundled modules
    fn is_bundled_module_or_package(&self, module_name: &str) -> bool {
        // Direct check
        if self.bundled_modules.contains(module_name) {
            return true;
        }

        // Check if it's a package containing bundled modules
        // e.g., if "greetings.greeting" is bundled, then "greetings" is a package
        let package_prefix = format!("{module_name}.");
        let has_submodules = self
            .bundled_modules
            .iter()
            .any(|bundled| bundled.starts_with(&package_prefix));

        if module_name.starts_with("schemas") || module_name.starts_with("utils") {
            log::info!(
                "is_bundled_module_or_package('{}') -> {} (direct: {}, has_submodules: {})",
                module_name,
                self.bundled_modules.contains(module_name) || has_submodules,
                self.bundled_modules.contains(module_name),
                has_submodules
            );
        }

        has_submodules
    }

    /// Collect unique imports from an import statement
    fn collect_unique_imports(
        &self,
        import_stmt: &StmtImport,
        seen_modules: &mut FxIndexSet<String>,
        unique_imports: &mut Vec<(String, Stmt)>,
    ) {
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            if seen_modules.contains(module_name) {
                continue;
            }
            seen_modules.insert(module_name.to_string());
            // Create import statement preserving the original alias
            unique_imports.push((
                module_name.to_string(),
                Stmt::Import(StmtImport {
                    node_index: AtomicNodeIndex::dummy(),
                    names: vec![ruff_python_ast::Alias {
                        node_index: AtomicNodeIndex::dummy(),
                        name: Identifier::new(module_name, TextRange::default()),
                        asname: alias.asname.clone(),
                        range: TextRange::default(),
                    }],
                    range: TextRange::default(),
                }),
            ));
        }
    }

    /// Normalize import aliases by removing them for stdlib modules
    fn normalize_import_aliases(&self, import_stmt: &mut StmtImport) {
        for alias in &mut import_stmt.names {
            let module_name = alias.name.as_str();
            if !self.is_safe_stdlib_module(module_name) || alias.asname.is_none() {
                continue;
            }
            // Remove the alias, keeping only the canonical name
            alias.asname = None;
            log::debug!("Normalized import to canonical: import {module_name}");
        }
    }

    /// Collect stdlib aliases from import statement
    fn collect_stdlib_aliases(
        &self,
        import_stmt: &StmtImport,
        alias_to_canonical: &mut FxIndexMap<String, String>,
    ) {
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            if !self.is_safe_stdlib_module(module_name) {
                continue;
            }
            if let Some(ref alias_name) = alias.asname {
                // This is an aliased import: import json as j
                alias_to_canonical.insert(alias_name.as_str().to_string(), module_name.to_string());
            }
        }
    }

    /// Normalize stdlib import aliases within a single file
    /// Converts "import json as j" to "import json" and rewrites all "j.dumps" to "json.dumps"
    fn normalize_stdlib_import_aliases(&self, ast: &mut ModModule) {
        // Step 1: Build alias-to-canonical mapping for this file
        let mut alias_to_canonical = FxIndexMap::default();

        for stmt in &ast.body {
            if let Stmt::Import(import_stmt) = stmt {
                self.collect_stdlib_aliases(import_stmt, &mut alias_to_canonical);
            }
        }

        if alias_to_canonical.is_empty() {
            return; // No aliases to normalize
        }

        log::debug!("Normalizing stdlib aliases: {alias_to_canonical:?}");

        // Step 2: Transform all expressions that reference aliases
        for stmt in &mut ast.body {
            match stmt {
                Stmt::Import(_) => {
                    // We'll handle import statements separately
                }
                _ => {
                    self.rewrite_aliases_in_stmt(stmt, &alias_to_canonical);
                }
            }
        }

        // Step 3: Transform import statements to canonical form
        for stmt in &mut ast.body {
            if let Stmt::Import(import_stmt) = stmt {
                self.normalize_import_aliases(import_stmt);
            }
        }
    }

    /// Recursively rewrite aliases in a statement
    /// Rewrite only global statements within a function, leaving other references untouched
    fn rewrite_global_statements_in_function(
        &self,
        func_def: &mut StmtFunctionDef,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        for stmt in &mut func_def.body {
            self.rewrite_global_statements_only(stmt, alias_to_canonical);
        }
    }

    /// Recursively rewrite only global statements, not other name references
    fn rewrite_global_statements_only(
        &self,
        stmt: &mut Stmt,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::Global(global_stmt) => {
                // Apply renames to global variable names
                for name in &mut global_stmt.names {
                    let name_str = name.as_str();
                    if let Some(new_name) = alias_to_canonical.get(name_str) {
                        log::debug!(
                            "Rewriting global statement variable '{name_str}' to '{new_name}'"
                        );
                        *name = Identifier::new(new_name, TextRange::default());
                    }
                }
            }
            // For control flow statements, recurse into their bodies
            Stmt::If(if_stmt) => {
                for stmt in &mut if_stmt.body {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    for stmt in &mut clause.body {
                        self.rewrite_global_statements_only(stmt, alias_to_canonical);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                for stmt in &mut while_stmt.body {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
                for stmt in &mut while_stmt.orelse {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
            }
            Stmt::For(for_stmt) => {
                for stmt in &mut for_stmt.body {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
                for stmt in &mut for_stmt.orelse {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
            }
            Stmt::With(with_stmt) => {
                for stmt in &mut with_stmt.body {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
            }
            Stmt::Try(try_stmt) => {
                for stmt in &mut try_stmt.body {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
                self.process_exception_handlers(&mut try_stmt.handlers, alias_to_canonical);
                for stmt in &mut try_stmt.orelse {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
                for stmt in &mut try_stmt.finalbody {
                    self.rewrite_global_statements_only(stmt, alias_to_canonical);
                }
            }
            // Nested functions need the same treatment
            Stmt::FunctionDef(nested_func) => {
                self.rewrite_global_statements_in_function(nested_func, alias_to_canonical);
            }
            // For other statements, do nothing - we don't want to rewrite name references
            _ => {}
        }
    }

    /// Process exception handlers to rewrite global statements
    fn process_exception_handlers(
        &self,
        handlers: &mut [ExceptHandler],
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        for handler in handlers {
            match handler {
                ExceptHandler::ExceptHandler(except_handler) => {
                    for stmt in &mut except_handler.body {
                        self.rewrite_global_statements_only(stmt, alias_to_canonical);
                    }
                }
            }
        }
    }

    /// Create a reassignment statement: original_name = renamed_name
    fn create_reassignment(&self, original_name: &str, renamed_name: &str) -> Stmt {
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: original_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: renamed_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    fn rewrite_aliases_in_stmt(
        &self,
        stmt: &mut Stmt,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Rewrite in default arguments
                let params = &mut func_def.parameters;
                for param in &mut params.args {
                    if let Some(ref mut default) = param.default {
                        self.rewrite_aliases_in_expr(default, alias_to_canonical);
                    }
                }
                // Rewrite in function body
                for stmt in &mut func_def.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::ClassDef(class_def) => {
                // Rewrite in base classes
                if let Some(ref mut arguments) = class_def.arguments {
                    for arg in &mut arguments.args {
                        self.rewrite_aliases_in_expr(arg, alias_to_canonical);
                    }
                }
                // Rewrite in class body
                for stmt in &mut class_def.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::If(if_stmt) => {
                self.rewrite_aliases_in_expr(&mut if_stmt.test, alias_to_canonical);
                for stmt in &mut if_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(ref mut condition) = clause.test {
                        self.rewrite_aliases_in_expr(condition, alias_to_canonical);
                    }
                    for stmt in &mut clause.body {
                        self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                self.rewrite_aliases_in_expr(&mut while_stmt.test, alias_to_canonical);
                for stmt in &mut while_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut while_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::For(for_stmt) => {
                self.rewrite_aliases_in_expr(&mut for_stmt.iter, alias_to_canonical);
                for stmt in &mut for_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut for_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::With(with_stmt) => {
                for item in &mut with_stmt.items {
                    self.rewrite_aliases_in_expr(&mut item.context_expr, alias_to_canonical);
                }
                for stmt in &mut with_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::Try(try_stmt) => {
                for stmt in &mut try_stmt.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for handler in &mut try_stmt.handlers {
                    self.rewrite_aliases_in_except_handler(handler, alias_to_canonical);
                }
                for stmt in &mut try_stmt.orelse {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
                for stmt in &mut try_stmt.finalbody {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
            Stmt::Assign(assign) => {
                // Rewrite in targets
                for target in &mut assign.targets {
                    self.rewrite_aliases_in_expr(target, alias_to_canonical);
                }
                // Rewrite in value
                self.rewrite_aliases_in_expr(&mut assign.value, alias_to_canonical);
            }
            Stmt::AugAssign(aug_assign) => {
                self.rewrite_aliases_in_expr(&mut aug_assign.target, alias_to_canonical);
                self.rewrite_aliases_in_expr(&mut aug_assign.value, alias_to_canonical);
            }
            Stmt::AnnAssign(ann_assign) => {
                self.rewrite_aliases_in_expr(&mut ann_assign.target, alias_to_canonical);
                if let Some(ref mut value) = ann_assign.value {
                    self.rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Stmt::Expr(expr_stmt) => {
                self.rewrite_aliases_in_expr(&mut expr_stmt.value, alias_to_canonical);
            }
            Stmt::Return(return_stmt) => {
                if let Some(ref mut value) = return_stmt.value {
                    self.rewrite_aliases_in_expr(value, alias_to_canonical);
                }
            }
            Stmt::Raise(raise_stmt) => {
                if let Some(ref mut exc) = raise_stmt.exc {
                    self.rewrite_aliases_in_expr(exc, alias_to_canonical);
                }
                if let Some(ref mut cause) = raise_stmt.cause {
                    self.rewrite_aliases_in_expr(cause, alias_to_canonical);
                }
            }
            Stmt::Assert(assert_stmt) => {
                self.rewrite_aliases_in_expr(&mut assert_stmt.test, alias_to_canonical);
                if let Some(ref mut msg) = assert_stmt.msg {
                    self.rewrite_aliases_in_expr(msg, alias_to_canonical);
                }
            }
            Stmt::Delete(delete_stmt) => {
                for target in &mut delete_stmt.targets {
                    self.rewrite_aliases_in_expr(target, alias_to_canonical);
                }
            }
            Stmt::Global(global_stmt) => {
                // Apply renames to global variable names
                for name in &mut global_stmt.names {
                    let name_str = name.as_str();
                    if let Some(new_name) = alias_to_canonical.get(name_str) {
                        log::debug!("Rewriting global variable '{name_str}' to '{new_name}'");
                        *name = Identifier::new(new_name, TextRange::default());
                    }
                }
            }
            Stmt::Nonlocal(_) => {
                // Nonlocal statements don't need rewriting in our use case
            }
            Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => {
                // These don't contain expressions
            }
            Stmt::Import(_) | Stmt::ImportFrom(_) => {
                // Import statements are handled separately and shouldn't be rewritten here
            }
            Stmt::TypeAlias(type_alias) => {
                self.rewrite_aliases_in_expr(&mut type_alias.value, alias_to_canonical);
            }
            Stmt::Match(match_stmt) => {
                self.rewrite_aliases_in_expr(&mut match_stmt.subject, alias_to_canonical);
                for case in &mut match_stmt.cases {
                    // Note: Pattern rewriting would be complex and is skipped for now
                    if let Some(ref mut guard) = case.guard {
                        self.rewrite_aliases_in_expr(guard, alias_to_canonical);
                    }
                    for stmt in &mut case.body {
                        self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                    }
                }
            }
            // Catch-all for any future statement types
            _ => {
                log::debug!("Unhandled statement type in alias rewriting: {stmt:?}");
            }
        }
    }

    /// Recursively rewrite aliases in an expression
    fn rewrite_aliases_in_expr(
        &self,
        expr: &mut Expr,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        rewrite_aliases_in_expr_impl(expr, alias_to_canonical);
    }

    /// Helper to rewrite aliases in except handlers to reduce nesting
    fn rewrite_aliases_in_except_handler(
        &self,
        handler: &mut ruff_python_ast::ExceptHandler,
        alias_to_canonical: &FxIndexMap<String, String>,
    ) {
        match handler {
            ruff_python_ast::ExceptHandler::ExceptHandler(except_handler) => {
                for stmt in &mut except_handler.body {
                    self.rewrite_aliases_in_stmt(stmt, alias_to_canonical);
                }
            }
        }
    }
}

/// Helper function to recursively rewrite aliases in an expression
fn rewrite_aliases_in_expr_impl(expr: &mut Expr, alias_to_canonical: &FxIndexMap<String, String>) {
    match expr {
        Expr::Name(name_expr) => {
            let name_str = name_expr.id.as_str();
            if let Some(canonical) = alias_to_canonical.get(name_str) {
                log::debug!("Rewriting alias '{name_str}' to canonical '{canonical}'");
                name_expr.id = canonical.clone().into();
            }
        }
        Expr::Attribute(attr_expr) => {
            // Handle cases like j.dumps -> json.dumps
            rewrite_aliases_in_expr_impl(&mut attr_expr.value, alias_to_canonical);
        }
        Expr::Call(call_expr) => {
            rewrite_aliases_in_expr_impl(&mut call_expr.func, alias_to_canonical);
            for arg in &mut call_expr.arguments.args {
                rewrite_aliases_in_expr_impl(arg, alias_to_canonical);
            }
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                rewrite_aliases_in_expr_impl(elem, alias_to_canonical);
            }
        }
        Expr::Dict(dict_expr) => {
            for item in &mut dict_expr.items {
                if let Some(ref mut key) = item.key {
                    rewrite_aliases_in_expr_impl(key, alias_to_canonical);
                }
                rewrite_aliases_in_expr_impl(&mut item.value, alias_to_canonical);
            }
        }
        Expr::Tuple(tuple_expr) => {
            for elem in &mut tuple_expr.elts {
                rewrite_aliases_in_expr_impl(elem, alias_to_canonical);
            }
        }
        Expr::Set(set_expr) => {
            for elem in &mut set_expr.elts {
                rewrite_aliases_in_expr_impl(elem, alias_to_canonical);
            }
        }
        Expr::BinOp(binop_expr) => {
            rewrite_aliases_in_expr_impl(&mut binop_expr.left, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut binop_expr.right, alias_to_canonical);
        }
        Expr::UnaryOp(unaryop_expr) => {
            rewrite_aliases_in_expr_impl(&mut unaryop_expr.operand, alias_to_canonical);
        }
        Expr::Compare(compare_expr) => {
            rewrite_aliases_in_expr_impl(&mut compare_expr.left, alias_to_canonical);
            for comparator in &mut compare_expr.comparators {
                rewrite_aliases_in_expr_impl(comparator, alias_to_canonical);
            }
        }
        Expr::BoolOp(boolop_expr) => {
            for value in &mut boolop_expr.values {
                rewrite_aliases_in_expr_impl(value, alias_to_canonical);
            }
        }
        Expr::If(if_expr) => {
            rewrite_aliases_in_expr_impl(&mut if_expr.test, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut if_expr.body, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut if_expr.orelse, alias_to_canonical);
        }
        Expr::ListComp(listcomp_expr) => {
            rewrite_aliases_in_expr_impl(&mut listcomp_expr.elt, alias_to_canonical);
            for generator in &mut listcomp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::SetComp(setcomp_expr) => {
            rewrite_aliases_in_expr_impl(&mut setcomp_expr.elt, alias_to_canonical);
            for generator in &mut setcomp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::DictComp(dictcomp_expr) => {
            rewrite_aliases_in_expr_impl(&mut dictcomp_expr.key, alias_to_canonical);
            rewrite_aliases_in_expr_impl(&mut dictcomp_expr.value, alias_to_canonical);
            for generator in &mut dictcomp_expr.generators {
                rewrite_aliases_in_expr_impl(&mut generator.iter, alias_to_canonical);
                for if_clause in &mut generator.ifs {
                    rewrite_aliases_in_expr_impl(if_clause, alias_to_canonical);
                }
            }
        }
        Expr::Subscript(subscript_expr) => {
            // Rewrite the value expression (e.g., the `obj` in `obj[key]`)
            rewrite_aliases_in_expr_impl(&mut subscript_expr.value, alias_to_canonical);
            // DO NOT rewrite string literals in slice position - they are dictionary keys,
            // not variable references. Only rewrite if the slice is a Name expression.
            if matches!(subscript_expr.slice.as_ref(), Expr::Name(_)) {
                rewrite_aliases_in_expr_impl(&mut subscript_expr.slice, alias_to_canonical);
            }
        }
        Expr::Slice(slice_expr) => {
            if let Some(ref mut lower) = slice_expr.lower {
                rewrite_aliases_in_expr_impl(lower, alias_to_canonical);
            }
            if let Some(ref mut upper) = slice_expr.upper {
                rewrite_aliases_in_expr_impl(upper, alias_to_canonical);
            }
            if let Some(ref mut step) = slice_expr.step {
                rewrite_aliases_in_expr_impl(step, alias_to_canonical);
            }
        }
        Expr::Lambda(lambda_expr) => {
            rewrite_aliases_in_expr_impl(&mut lambda_expr.body, alias_to_canonical);
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                rewrite_aliases_in_expr_impl(value, alias_to_canonical);
            }
        }
        Expr::YieldFrom(yieldfrom_expr) => {
            rewrite_aliases_in_expr_impl(&mut yieldfrom_expr.value, alias_to_canonical);
        }
        Expr::Await(await_expr) => {
            rewrite_aliases_in_expr_impl(&mut await_expr.value, alias_to_canonical);
        }
        Expr::Starred(starred_expr) => {
            rewrite_aliases_in_expr_impl(&mut starred_expr.value, alias_to_canonical);
        }
        Expr::FString(_fstring_expr) => {
            // FString handling is complex due to its structure
            // For now, skip FString rewriting as it's rarely used with module aliases
            log::debug!("FString expression rewriting not yet implemented");
        }
        // Constant values and other literals don't need rewriting
        Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_) => {
            // These don't contain references to aliases
        }
        _ => {
            // Log unhandled expression types for future reference
            log::trace!("Unhandled expression type in alias rewriting");
        }
    }
}

impl HybridStaticBundler {
    /// Transform AST to use lifted global variables
    fn transform_ast_with_lifted_globals(
        &self,
        ast: &mut ModModule,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &ModuleGlobalInfo,
    ) {
        // Transform all statements that use global declarations
        for stmt in &mut ast.body {
            self.transform_stmt_for_lifted_globals(stmt, lifted_names, global_info, None);
        }
    }

    /// Transform a statement to use lifted globals
    fn transform_stmt_for_lifted_globals(
        &self,
        stmt: &mut Stmt,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &ModuleGlobalInfo,
        current_function_globals: Option<&FxIndexSet<String>>,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Check if this function uses globals
                if global_info
                    .functions_using_globals
                    .contains(&func_def.name.to_string())
                {
                    // Collect globals declared in this function
                    let function_globals = self.collect_function_globals(&func_def.body);

                    // Create initialization statements for lifted globals
                    let init_stmts =
                        self.create_global_init_statements(&function_globals, lifted_names);

                    // Transform the function body
                    let params = TransformFunctionParams {
                        lifted_names,
                        global_info,
                        function_globals: &function_globals,
                    };
                    self.transform_function_body_for_lifted_globals(func_def, &params, init_stmts);
                }
            }
            Stmt::Assign(assign) => {
                // Transform assignments to use lifted names if they're in a function with global
                // declarations
                for target in &mut assign.targets {
                    self.transform_expr_for_lifted_globals(
                        target,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
                self.transform_expr_for_lifted_globals(
                    &mut assign.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::Expr(expr_stmt) => {
                self.transform_expr_for_lifted_globals(
                    &mut expr_stmt.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::If(if_stmt) => {
                self.transform_expr_for_lifted_globals(
                    &mut if_stmt.test,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                for stmt in &mut if_stmt.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(test_expr) = &mut clause.test {
                        self.transform_expr_for_lifted_globals(
                            test_expr,
                            lifted_names,
                            global_info,
                            current_function_globals,
                        );
                    }
                    for stmt in &mut clause.body {
                        self.transform_stmt_for_lifted_globals(
                            stmt,
                            lifted_names,
                            global_info,
                            current_function_globals,
                        );
                    }
                }
            }
            Stmt::While(while_stmt) => {
                self.transform_expr_for_lifted_globals(
                    &mut while_stmt.test,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                for stmt in &mut while_stmt.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
            }
            Stmt::For(for_stmt) => {
                self.transform_expr_for_lifted_globals(
                    &mut for_stmt.target,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                self.transform_expr_for_lifted_globals(
                    &mut for_stmt.iter,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                for stmt in &mut for_stmt.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    self.transform_expr_for_lifted_globals(
                        value,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
            }
            Stmt::ClassDef(class_def) => {
                // Transform methods in the class that use globals
                for stmt in &mut class_def.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Transform augmented assignments to use lifted names
                self.transform_expr_for_lifted_globals(
                    &mut aug_assign.target,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                self.transform_expr_for_lifted_globals(
                    &mut aug_assign.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            _ => {
                // Other statement types handled as needed
            }
        }
    }

    /// Transform an expression to use lifted globals
    fn transform_expr_for_lifted_globals(
        &self,
        expr: &mut Expr,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &ModuleGlobalInfo,
        in_function_with_globals: Option<&FxIndexSet<String>>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                // Transform if this is a lifted global and we're in a function that declares it
                // global
                if let Some(function_globals) = in_function_with_globals
                    && function_globals.contains(name_expr.id.as_str())
                    && let Some(lifted_name) = lifted_names.get(name_expr.id.as_str())
                {
                    name_expr.id = lifted_name.clone().into();
                }
            }
            Expr::Call(call) => {
                self.transform_expr_for_lifted_globals(
                    &mut call.func,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
                for arg in &mut call.arguments.args {
                    self.transform_expr_for_lifted_globals(
                        arg,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            Expr::Attribute(attr) => {
                self.transform_expr_for_lifted_globals(
                    &mut attr.value,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::FString(_) => {
                self.transform_fstring_for_lifted_globals(
                    expr,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::BinOp(binop) => {
                self.transform_expr_for_lifted_globals(
                    &mut binop.left,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
                self.transform_expr_for_lifted_globals(
                    &mut binop.right,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::UnaryOp(unaryop) => {
                self.transform_expr_for_lifted_globals(
                    &mut unaryop.operand,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::Compare(compare) => {
                self.transform_expr_for_lifted_globals(
                    &mut compare.left,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
                for comparator in &mut compare.comparators {
                    self.transform_expr_for_lifted_globals(
                        comparator,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            Expr::Subscript(subscript) => {
                self.transform_expr_for_lifted_globals(
                    &mut subscript.value,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
                self.transform_expr_for_lifted_globals(
                    &mut subscript.slice,
                    lifted_names,
                    global_info,
                    in_function_with_globals,
                );
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    self.transform_expr_for_lifted_globals(
                        elem,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            Expr::Tuple(tuple_expr) => {
                for elem in &mut tuple_expr.elts {
                    self.transform_expr_for_lifted_globals(
                        elem,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            Expr::Dict(dict_expr) => {
                for item in &mut dict_expr.items {
                    if let Some(key) = &mut item.key {
                        self.transform_expr_for_lifted_globals(
                            key,
                            lifted_names,
                            global_info,
                            in_function_with_globals,
                        );
                    }
                    self.transform_expr_for_lifted_globals(
                        &mut item.value,
                        lifted_names,
                        global_info,
                        in_function_with_globals,
                    );
                }
            }
            _ => {
                // Other expressions handled as needed
            }
        }
    }
}

impl HybridStaticBundler {
    /// Collect module renames from semantic analysis
    fn collect_module_renames(
        &self,
        module_name: &str,
        semantic_ctx: &SemanticContext,
        symbol_renames: &mut FxIndexMap<String, FxIndexMap<String, String>>,
    ) {
        log::debug!("collect_module_renames: Processing module '{module_name}'");

        // Find the module ID for this module name
        let module_id = match semantic_ctx.graph.get_module_by_name(module_name) {
            Some(module) => module.module_id,
            None => {
                log::warn!("Module '{module_name}' not found in graph");
                return;
            }
        };

        log::debug!("Module '{module_name}' has ID: {module_id:?}");

        // Get all renames for this module from semantic analysis
        let mut module_renames = FxIndexMap::default();

        // Use ModuleSemanticInfo to get ALL exported symbols from the module
        if let Some(module_info) = semantic_ctx.semantic_bundler.get_module_info(&module_id) {
            log::debug!(
                "Module '{}' exports {} symbols: {:?}",
                module_name,
                module_info.exported_symbols.len(),
                module_info.exported_symbols.iter().collect::<Vec<_>>()
            );

            // Process all exported symbols from the module
            for symbol in &module_info.exported_symbols {
                if let Some(new_name) = semantic_ctx.symbol_registry.get_rename(&module_id, symbol)
                {
                    module_renames.insert(symbol.to_string(), new_name.to_string());
                    log::debug!(
                        "Module '{module_name}': symbol '{symbol}' renamed to '{new_name}'"
                    );
                } else {
                    // Include non-renamed symbols too - they still need to be in the namespace
                    module_renames.insert(symbol.to_string(), symbol.to_string());
                    log::debug!(
                        "Module '{module_name}': symbol '{symbol}' has no rename, using original \
                         name"
                    );
                }
            }
        } else {
            log::warn!("No semantic info found for module '{module_name}' with ID {module_id:?}");
        }

        // For inlined modules with __all__, we need to also include symbols from __all__
        // even if they're not defined in this module (they might be re-exports)
        if self.inlined_modules.contains(module_name) {
            log::debug!("Module '{module_name}' is inlined, checking for __all__ exports");
            if let Some(export_info) = self.module_exports.get(module_name) {
                log::debug!("Module '{module_name}' export info: {export_info:?}");
                if let Some(all_exports) = export_info {
                    log::debug!(
                        "Module '{}' has __all__ with {} exports: {:?}",
                        module_name,
                        all_exports.len(),
                        all_exports
                    );

                    // Add any symbols from __all__ that aren't already in module_renames
                    for export in all_exports {
                        if !module_renames.contains_key(export) {
                            // This is a re-exported symbol - use the original name
                            module_renames.insert(export.clone(), export.clone());
                            log::debug!(
                                "Module '{module_name}': adding re-exported symbol '{export}' \
                                 from __all__"
                            );
                        }
                    }
                }
            }
        }

        if !module_renames.is_empty() {
            log::debug!(
                "Inserting {} renames for module '{}' with key '{}': {:?}",
                module_renames.len(),
                module_name,
                module_name,
                module_renames.keys().collect::<Vec<_>>()
            );
            symbol_renames.insert(module_name.to_string(), module_renames);
        } else {
            log::debug!("No renames to insert for module '{module_name}'");
        }
    }

    /// Process wrapper module for global analysis and lifting
    fn process_wrapper_module_globals(
        &self,
        params: &ProcessGlobalsParams,
        module_globals: &mut FxIndexMap<String, ModuleGlobalInfo>,
        all_lifted_declarations: &mut Vec<Stmt>,
    ) {
        // Get module ID from graph
        let module = match params
            .semantic_ctx
            .graph
            .get_module_by_name(params.module_name)
        {
            Some(m) => m,
            None => return,
        };

        let module_id = module.module_id;
        let global_info = params.semantic_ctx.semantic_bundler.analyze_module_globals(
            module_id,
            params.ast,
            params.module_name,
        );

        // Create GlobalsLifter and collect declarations
        if !global_info.global_declarations.is_empty() {
            let globals_lifter = GlobalsLifter::new(&global_info);
            all_lifted_declarations.extend(globals_lifter.get_lifted_declarations());
        }

        module_globals.insert(params.module_name.to_string(), global_info);
    }
}

#[allow(dead_code)]
impl HybridStaticBundler {
    /// Collect Import statements
    fn collect_import(&mut self, import_stmt: &StmtImport, stmt: &Stmt) {
        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();
            if self.is_safe_stdlib_module(module_name) {
                self.stdlib_import_statements.push(stmt.clone());
                break;
            } else if !self.is_bundled_module_or_package(module_name) {
                // This is a third-party import (not stdlib, not bundled)
                // We do NOT collect third-party imports for hoisting because they may have
                // side effects. They will remain in their original location within the module.
                log::debug!("Skipping third-party import '{module_name}' - will not be hoisted");
                break;
            }
        }
    }

    /// Add a regular stdlib import (e.g., "sys", "types")
    /// This creates an import statement and adds it to the tracked imports
    fn add_stdlib_import(&mut self, module_name: &str) {
        // Check if we already have this import to avoid duplicates
        let already_imported = self.stdlib_import_statements.iter().any(|stmt| {
            if let Stmt::Import(import_stmt) = stmt {
                import_stmt
                    .names
                    .iter()
                    .any(|alias| alias.name.as_str() == module_name)
            } else {
                false
            }
        });

        if already_imported {
            log::debug!("Stdlib import '{module_name}' already exists, skipping");
            return;
        }

        let import_stmt = Stmt::Import(StmtImport {
            node_index: AtomicNodeIndex::dummy(),
            names: vec![ruff_python_ast::Alias {
                node_index: AtomicNodeIndex::dummy(),
                name: Identifier::new(module_name, TextRange::default()),
                asname: None,
                range: TextRange::default(),
            }],
            range: TextRange::default(),
        });
        self.stdlib_import_statements.push(import_stmt);
    }

    /// Collect a symbol from a module statement
    fn collect_module_symbol(&self, stmt: &Stmt, symbols: &mut Vec<String>) {
        match stmt {
            Stmt::FunctionDef(func) => {
                symbols.push(func.name.to_string());
            }
            Stmt::ClassDef(class) => {
                symbols.push(class.name.to_string());
            }
            Stmt::Assign(assign) if assign.targets.len() == 1 => {
                if let Expr::Name(name) = &assign.targets[0] {
                    symbols.push(name.id.to_string());
                }
            }
            _ => {}
        }
    }

    /// Extract __all__ exports from a module
    /// Returns (has_explicit_all, exports) where:
    /// - has_explicit_all: true if __all__ is explicitly defined
    /// - exports: Some(vec) if there are exports, None if no exports
    fn extract_all_exports(&self, ast: &ModModule) -> (bool, Option<Vec<String>>) {
        // First, look for explicit __all__
        for stmt in &ast.body {
            let Stmt::Assign(assign) = stmt else {
                continue;
            };

            // Look for __all__ = [...]
            if assign.targets.len() != 1 {
                continue;
            }

            let Expr::Name(name) = &assign.targets[0] else {
                continue;
            };

            if name.id.as_str() == "__all__" {
                return (true, self.extract_string_list_from_expr(&assign.value));
            }
        }

        // If no __all__, collect all top-level symbols (including private ones for module state)
        let mut symbols = Vec::new();
        for stmt in &ast.body {
            match stmt {
                Stmt::FunctionDef(func) => {
                    symbols.push(func.name.to_string());
                }
                Stmt::ClassDef(class) => {
                    symbols.push(class.name.to_string());
                }
                Stmt::Assign(assign) => {
                    // Include ALL variable assignments (including private ones starting with _)
                    // This ensures module state variables like _config, _logger are available
                    for target in &assign.targets {
                        if let Expr::Name(name) = target
                            && name.id.as_str() != "__all__"
                        {
                            symbols.push(name.id.to_string());
                        }
                    }
                }
                Stmt::AnnAssign(ann_assign) => {
                    // Include ALL annotated assignments (including private ones)
                    if let Expr::Name(name) = ann_assign.target.as_ref() {
                        symbols.push(name.id.to_string());
                    }
                }
                _ => {}
            }
        }

        if symbols.is_empty() {
            (false, None)
        } else {
            // Sort symbols for deterministic output
            symbols.sort();
            (false, Some(symbols))
        }
    }

    /// Extract a list of strings from an expression (for __all__ parsing)
    fn extract_string_list_from_expr(&self, expr: &Expr) -> Option<Vec<String>> {
        match expr {
            Expr::List(list_expr) => {
                let mut exports = Vec::new();
                for element in &list_expr.elts {
                    if let Expr::StringLiteral(string_lit) = element {
                        let string_value = string_lit.value.to_str();
                        exports.push(string_value.to_string());
                    }
                }
                Some(exports)
            }
            Expr::Tuple(tuple_expr) => {
                let mut exports = Vec::new();
                for element in &tuple_expr.elts {
                    if let Expr::StringLiteral(string_lit) = element {
                        let string_value = string_lit.value.to_str();
                        exports.push(string_value.to_string());
                    }
                }
                Some(exports)
            }
            _ => None, // Other expressions like computed lists are not supported
        }
    }

    /// Collect all variable names referenced in a list of statements
    fn collect_referenced_vars(&self, stmts: &[Stmt], vars: &mut FxIndexSet<String>) {
        for stmt in stmts {
            self.collect_vars_in_stmt(stmt, vars);
        }
    }

    /// Collect variable names referenced in a statement
    fn collect_vars_in_stmt(&self, stmt: &Stmt, vars: &mut FxIndexSet<String>) {
        match stmt {
            Stmt::Expr(expr_stmt) => self.collect_vars_in_expr(&expr_stmt.value, vars),
            Stmt::Return(ret) => {
                if let Some(value) = &ret.value {
                    self.collect_vars_in_expr(value, vars);
                }
            }
            Stmt::Assign(assign) => {
                self.collect_vars_in_expr(&assign.value, vars);
            }
            Stmt::If(if_stmt) => {
                self.collect_vars_in_expr(&if_stmt.test, vars);
                self.collect_referenced_vars(&if_stmt.body, vars);
                for clause in &if_stmt.elif_else_clauses {
                    if let Some(condition) = &clause.test {
                        self.collect_vars_in_expr(condition, vars);
                    }
                    self.collect_referenced_vars(&clause.body, vars);
                }
            }
            Stmt::For(for_stmt) => {
                self.collect_vars_in_expr(&for_stmt.iter, vars);
                self.collect_referenced_vars(&for_stmt.body, vars);
                self.collect_referenced_vars(&for_stmt.orelse, vars);
            }
            Stmt::While(while_stmt) => {
                self.collect_vars_in_expr(&while_stmt.test, vars);
                self.collect_referenced_vars(&while_stmt.body, vars);
                self.collect_referenced_vars(&while_stmt.orelse, vars);
            }
            Stmt::Try(try_stmt) => {
                self.collect_referenced_vars(&try_stmt.body, vars);
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    self.collect_referenced_vars(&eh.body, vars);
                }
                self.collect_referenced_vars(&try_stmt.orelse, vars);
                self.collect_referenced_vars(&try_stmt.finalbody, vars);
            }
            Stmt::With(with_stmt) => {
                for item in &with_stmt.items {
                    self.collect_vars_in_expr(&item.context_expr, vars);
                }
                self.collect_referenced_vars(&with_stmt.body, vars);
            }
            _ => {}
        }
    }

    /// Collect variable names referenced in an expression
    #[allow(clippy::only_used_in_recursion)]
    fn collect_vars_in_expr(&self, expr: &Expr, vars: &mut FxIndexSet<String>) {
        match expr {
            Expr::Name(name) => {
                vars.insert(name.id.to_string());
            }
            Expr::Call(call) => {
                self.collect_vars_in_expr(&call.func, vars);
                for arg in call.arguments.args.iter() {
                    self.collect_vars_in_expr(arg, vars);
                }
                for keyword in call.arguments.keywords.iter() {
                    self.collect_vars_in_expr(&keyword.value, vars);
                }
            }
            Expr::Attribute(attr) => {
                self.collect_vars_in_expr(&attr.value, vars);
            }
            Expr::BinOp(binop) => {
                self.collect_vars_in_expr(&binop.left, vars);
                self.collect_vars_in_expr(&binop.right, vars);
            }
            Expr::UnaryOp(unaryop) => {
                self.collect_vars_in_expr(&unaryop.operand, vars);
            }
            Expr::BoolOp(boolop) => {
                for value in boolop.values.iter() {
                    self.collect_vars_in_expr(value, vars);
                }
            }
            Expr::Compare(compare) => {
                self.collect_vars_in_expr(&compare.left, vars);
                for comparator in compare.comparators.iter() {
                    self.collect_vars_in_expr(comparator, vars);
                }
            }
            Expr::List(list) => {
                for elt in list.elts.iter() {
                    self.collect_vars_in_expr(elt, vars);
                }
            }
            Expr::Tuple(tuple) => {
                for elt in tuple.elts.iter() {
                    self.collect_vars_in_expr(elt, vars);
                }
            }
            Expr::Dict(dict) => {
                for item in dict.items.iter() {
                    if let Some(key) = &item.key {
                        self.collect_vars_in_expr(key, vars);
                    }
                    self.collect_vars_in_expr(&item.value, vars);
                }
            }
            Expr::Subscript(subscript) => {
                self.collect_vars_in_expr(&subscript.value, vars);
                self.collect_vars_in_expr(&subscript.slice, vars);
            }
            Expr::If(if_expr) => {
                self.collect_vars_in_expr(&if_expr.test, vars);
                self.collect_vars_in_expr(&if_expr.body, vars);
                self.collect_vars_in_expr(&if_expr.orelse, vars);
            }
            _ => {}
        }
    }

    /// Check if an assignment is self-referential (e.g., `x = x`)
    fn is_self_referential_assignment(&self, assign: &StmtAssign) -> bool {
        // Check if this is a simple assignment with a single target and value
        if assign.targets.len() == 1
            && let (Expr::Name(target), Expr::Name(value)) =
                (&assign.targets[0], assign.value.as_ref())
        {
            // It's self-referential if target and value have the same name
            let is_self_ref = target.id == value.id;
            if is_self_ref {
                log::debug!(
                    "Found self-referential assignment: {} = {}",
                    target.id,
                    value.id
                );
            }
            return is_self_ref;
        }
        false
    }

    /// Determine if a symbol should be exported based on __all__ or default visibility rules
    fn should_export_symbol(&self, symbol_name: &str, module_name: &str) -> bool {
        // Don't export __all__ itself as a module attribute
        if symbol_name == "__all__" {
            return false;
        }

        // Check if the module has explicit __all__ exports
        if let Some(Some(exports)) = self.module_exports.get(module_name) {
            // Module defines __all__, only export symbols listed there
            exports.contains(&symbol_name.to_string())
        } else {
            // No __all__ defined, use default Python visibility rules
            // Export all symbols that don't start with underscore
            !symbol_name.starts_with('_')
        }
    }

    /// Add module attribute assignments for imported symbols that should be re-exported
    fn add_imported_symbol_attributes(
        &self,
        stmt: &Stmt,
        module_name: &str,
        module_path: Option<&Path>,
        body: &mut Vec<Stmt>,
    ) {
        match stmt {
            Stmt::ImportFrom(import_from) => {
                // First check if this is an import from an inlined module
                let resolved_module_name = self.resolve_relative_import_with_context(
                    import_from,
                    module_name,
                    module_path,
                );
                if let Some(ref imported_module) = resolved_module_name {
                    // If this is an inlined module, skip module attribute assignment
                    // The symbols will be referenced directly in the transformed import
                    if self.bundled_modules.contains(imported_module)
                        && !self.module_registry.contains_key(imported_module)
                    {
                        // This is an inlined module - skip adding module attributes
                        return;
                    }
                }

                // For "from module import symbol1, symbol2 as alias"
                for alias in &import_from.names {
                    let _imported_name = alias.name.as_str();
                    let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                    // Check if this imported symbol should be exported
                    if self.should_export_symbol(local_name, module_name) {
                        body.push(self.create_module_attr_assignment("module", local_name));
                    }
                }
            }
            Stmt::Import(import_stmt) => {
                // For "import module" or "import module as alias"
                for alias in &import_stmt.names {
                    let imported_module = alias.name.as_str();

                    // Skip if this is an inlined module
                    if self.bundled_modules.contains(imported_module)
                        && !self.module_registry.contains_key(imported_module)
                    {
                        continue;
                    }

                    let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                    // Check if this imported module should be exported
                    if self.should_export_symbol(local_name, module_name) {
                        body.push(self.create_module_attr_assignment("module", local_name));
                    }
                }
            }
            _ => {}
        }
    }

    /// Create an __all__ assignment for a bundled module to make exports explicit
    /// This should only be called for modules that originally defined __all__
    fn create_all_assignment_for_module(&self, module_name: &str) -> Stmt {
        let exported_symbols = self
            .module_exports
            .get(module_name)
            .and_then(|opt| opt.as_ref())
            .cloned()
            .unwrap_or_default();

        // Create string literals for each exported symbol
        let elements: Vec<Expr> = exported_symbols
            .iter()
            .map(|symbol| self.create_string_literal(symbol))
            .collect();

        // Create: module.__all__ = [exported_symbols...]
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "module".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("__all__", TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::List(ExprList {
                node_index: AtomicNodeIndex::dummy(),
                elts: elements,
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Check if a specific module is in our hoisted stdlib imports
    fn is_import_in_hoisted_stdlib(&self, module_name: &str) -> bool {
        // Check if module is in our from imports map
        if self.stdlib_import_from_map.contains_key(module_name) {
            return true;
        }

        // Check if module is in our regular import statements
        self.stdlib_import_statements.iter().any(|hoisted| {
            matches!(hoisted, Stmt::Import(hoisted_import)
                if hoisted_import.names.iter().any(|alias| alias.name.as_str() == module_name))
        })
    }

    /// Check if an import has been hoisted
    fn is_hoisted_import(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::ImportFrom(import_from) => {
                if let Some(ref module) = import_from.module {
                    let module_name = module.as_str();
                    // Check if this is a __future__ import (always hoisted)
                    if module_name == "__future__" {
                        return true;
                    }
                    // Check if this is a stdlib import that we've hoisted
                    if self.is_safe_stdlib_module(module_name) {
                        // Check if this exact import is in our hoisted stdlib imports
                        return self.is_import_in_hoisted_stdlib(module_name);
                    }
                    // We no longer hoist third-party imports, so they should never be considered
                    // hoisted Only stdlib and __future__ imports are hoisted
                }
                false
            }
            Stmt::Import(import_stmt) => {
                // Check if any of the imported modules are hoisted (stdlib or third-party)
                import_stmt.names.iter().any(|alias| {
                    let module_name = alias.name.as_str();
                    // Check stdlib imports
                    if self.is_safe_stdlib_module(module_name) {
                        self.stdlib_import_statements.iter().any(|hoisted| {
                            matches!(hoisted, Stmt::Import(hoisted_import)
                                if hoisted_import.names.iter().any(|h| h.name == alias.name))
                        })
                    }
                    // We no longer hoist third-party imports
                    else {
                        false
                    }
                })
            }
            _ => false,
        }
    }

    /// Transform a bundled "from module import ..." statement into multiple assignments
    fn transform_bundled_import_from_multiple(
        &self,
        import_from: StmtImportFrom,
        module_name: &str,
    ) -> Vec<Stmt> {
        self.transform_bundled_import_from_multiple_with_context(import_from, module_name, false)
    }

    /// Transform a bundled "from module import ..." statement into multiple assignments with
    /// context
    fn transform_bundled_import_from_multiple_with_context(
        &self,
        import_from: StmtImportFrom,
        module_name: &str,
        inside_wrapper_init: bool,
    ) -> Vec<Stmt> {
        log::debug!(
            "transform_bundled_import_from_multiple: module_name={}, imports={:?}, \
             inside_wrapper_init={}",
            module_name,
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            inside_wrapper_init
        );
        let mut assignments = Vec::new();
        let mut initialized_modules = FxIndexSet::default();

        // Pre-check if this is a wrapper module that needs initialization
        let module_needs_init = if self.module_registry.contains_key(module_name) {
            if inside_wrapper_init {
                // Inside a wrapper module's init function, we can't reference ANY global namespace
                // because it hasn't been set up yet. We need to initialize all wrapper modules
                // locally.
                true
            } else if module_name.contains('.') {
                // In other contexts, nested modules like schemas.user are initialized during
                // global namespace creation
                false
            } else {
                // Top-level wrapper modules may need initialization
                true
            }
        } else {
            false
        };

        // If we need to initialize the module, do it once before processing any imports
        let module_var_name = if module_needs_init && !initialized_modules.contains(module_name) {
            let local_module_var = format!(
                "_cribo_module_{}",
                module_name.cow_replace('.', "_").as_ref()
            );

            // We need to determine the correct source variable to reference
            let source_var =
                if inside_wrapper_init || self.module_registry.contains_key(module_name) {
                    // Inside wrapper init OR when importing from a wrapper module,
                    // use the temporary variable which will contain the initialized module
                    format!("_cribo_temp_{}", module_name.cow_replace(".", "_"))
                } else if module_name.contains('.') {
                    // For dotted modules outside wrapper init, reference the temporary variable if
                    // it exists
                    format!("_cribo_temp_{}", module_name.cow_replace(".", "_"))
                } else {
                    // For top-level modules outside wrapper init that are not wrapper modules,
                    // reference the module directly
                    module_name.to_string()
                };

            // Add: _cribo_module_xxx = source_var (either the module or temp variable)
            assignments.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: local_module_var.clone().into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: source_var.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }));
            initialized_modules.insert(module_name.to_string());
            Some(local_module_var)
        } else {
            None
        };

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

            // Check if we're importing a submodule (e.g., from greetings import greeting)
            let full_module_path = format!("{module_name}.{imported_name}");

            // First check if the parent module has an __init__.py (is a wrapper module)
            // and might re-export this name
            let parent_is_wrapper = self.module_registry.contains_key(module_name);
            let submodule_exists = self.bundled_modules.contains(&full_module_path)
                && self.module_registry.contains_key(&full_module_path);

            // If both the parent is a wrapper and a submodule exists, we need to decide
            // In Python, attributes from __init__.py take precedence over submodules
            // So we should prefer the attribute unless we have evidence it's not re-exported
            let importing_submodule = if parent_is_wrapper && submodule_exists {
                // Check if the parent module explicitly exports this name
                if let Some(Some(export_list)) = self.module_exports.get(module_name) {
                    // If __all__ is defined and doesn't include this name, it's the submodule
                    !export_list.contains(&imported_name.to_string())
                } else {
                    // No __all__ defined or no export info - assume it's an attribute (safer
                    // default)
                    false
                }
            } else {
                // Simple case: just check if it's a submodule
                submodule_exists
            };

            if importing_submodule {
                // We're importing a submodule, not an attribute
                // First, ensure the submodule is initialized if it's a wrapper module
                if let Some(synthetic_name) = self.module_registry.get(&full_module_path) {
                    // Only add initialization call if we haven't already initialized this module
                    if !initialized_modules.contains(&full_module_path) {
                        // Add initialization call: __cribo_init_<synthetic>()
                        let init_func_name = format!("__cribo_init_{synthetic_name}");
                        assignments.push(Stmt::Expr(ruff_python_ast::StmtExpr {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(Expr::Call(ExprCall {
                                node_index: AtomicNodeIndex::dummy(),
                                func: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: init_func_name.into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                arguments: Arguments {
                                    node_index: AtomicNodeIndex::dummy(),
                                    args: Box::from([]),
                                    keywords: Box::from([]),
                                    range: TextRange::default(),
                                },
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
                        initialized_modules.insert(full_module_path.clone());
                    }
                }

                // Create: target = module.submodule (direct namespace reference)
                log::debug!(
                    "Importing submodule '{imported_name}' from '{module_name}' via from import"
                );

                // Build the direct namespace reference
                let namespace_expr = if full_module_path.contains('.') {
                    // For nested modules like models.user, create models.user expression
                    let parts: Vec<&str> = full_module_path.split('.').collect();
                    let mut expr = Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
                    for part in &parts[1..] {
                        expr = Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(expr),
                            attr: Identifier::new(*part, TextRange::default()),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        });
                    }
                    expr
                } else {
                    // Top-level module
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: full_module_path.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
                };

                assignments.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: target_name.as_str().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(namespace_expr),
                    range: TextRange::default(),
                }));
            } else {
                // Regular attribute import
                // Create: target = module.imported_name
                let module_expr = if let Some(ref var_name) = module_var_name {
                    // We initialized the module locally, use the local variable
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: var_name.clone().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
                } else if module_name.contains('.') {
                    // For nested modules like models.user, create models.user expression
                    let parts: Vec<&str> = module_name.split('.').collect();
                    let mut expr = Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
                    for part in &parts[1..] {
                        expr = Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(expr),
                            attr: Identifier::new(*part, TextRange::default()),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        });
                    }
                    expr
                } else {
                    // Top-level module
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: module_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
                };

                let assignment = Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: target_name.as_str().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(module_expr),
                        attr: Identifier::new(imported_name, TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                });

                log::debug!(
                    "Generating attribute assignment: {} = {}.{} (inside_wrapper_init: {})",
                    target_name.as_str(),
                    module_name,
                    imported_name,
                    inside_wrapper_init
                );

                assignments.push(assignment);
            }
        }

        assignments
    }

    /// Rewrite imports in a statement with module context for relative import resolution
    fn rewrite_import_in_stmt_multiple_with_context(
        &self,
        stmt: Stmt,
        current_module: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        self.rewrite_import_in_stmt_multiple_with_full_context(
            stmt,
            current_module,
            symbol_renames,
            false,
        )
    }

    /// Rewrite imports in a statement with full context including wrapper init flag
    fn rewrite_import_in_stmt_multiple_with_full_context(
        &self,
        stmt: Stmt,
        current_module: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        inside_wrapper_init: bool,
    ) -> Vec<Stmt> {
        match stmt {
            Stmt::ImportFrom(import_from) => self.rewrite_import_from(
                import_from,
                current_module,
                symbol_renames,
                inside_wrapper_init,
            ),
            Stmt::Import(import_stmt) => {
                self.rewrite_import_with_renames(import_stmt, symbol_renames)
            }
            _ => vec![stmt],
        }
    }

    /// Check if a module is safe to hoist
    fn is_safe_stdlib_module(&self, module_name: &str) -> bool {
        match module_name {
            // Modules that modify global state - DO NOT HOIST
            "antigravity" | "this" | "__hello__" | "__phello__" => false,
            "site" | "sitecustomize" | "usercustomize" => false,
            "readline" | "rlcompleter" => false,
            "turtle" | "tkinter" => false,
            "webbrowser" => false,
            "platform" | "locale" => false,

            _ => {
                let root_module = module_name.split('.').next().unwrap_or(module_name);
                ruff_python_stdlib::sys::is_known_standard_library(10, root_module)
            }
        }
    }

    /// Handle imports from inlined modules in wrapper functions
    fn handle_inlined_module_import(
        &self,
        params: &InlinedImportParams,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        body: &mut Vec<Stmt>,
    ) -> bool {
        // Check if this module is inlined
        let is_inlined = if self.inlined_modules.contains(params.resolved_module) {
            true
        } else {
            // Try removing the first component if it exists
            if let Some(dot_pos) = params.resolved_module.find('.') {
                let without_prefix = &params.resolved_module[dot_pos + 1..];
                self.inlined_modules.contains(without_prefix)
            } else {
                false
            }
        };

        log::debug!(
            "Is {} in inlined_modules? {}",
            params.resolved_module,
            is_inlined
        );
        if !is_inlined {
            return false;
        }

        // Handle each imported name from the inlined module
        for alias in &params.import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Check if we're importing a module itself (not a symbol from it)
            let full_module_path = format!("{}.{}", params.resolved_module, imported_name);
            let importing_module = self.check_if_importing_module(
                params.resolved_module,
                imported_name,
                &full_module_path,
            );

            log::debug!(
                "Checking if '{imported_name}' is a module import: \
                 full_path='{full_module_path}', importing_module={importing_module}"
            );

            if importing_module {
                // Check if this module is actually a wrapper module (not inlined)
                if self.module_registry.contains_key(&full_module_path) {
                    // It's a wrapper module, we need to use the synthetic name
                    let synthetic_name = &self.module_registry[&full_module_path];
                    log::debug!(
                        "Module '{full_module_path}' is a wrapper module with synthetic name \
                         '{synthetic_name}'"
                    );

                    // First ensure the module is initialized by calling its init function
                    let init_func_name = &self.init_functions[synthetic_name];
                    body.push(Stmt::Expr(ruff_python_ast::StmtExpr {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Call(ruff_python_ast::ExprCall {
                            node_index: AtomicNodeIndex::dummy(),
                            func: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: init_func_name.into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            arguments: Arguments {
                                node_index: AtomicNodeIndex::dummy(),
                                args: Box::new([]),
                                keywords: Box::new([]),
                                range: TextRange::default(),
                            },
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    }));

                    // Then access it via sys.modules using the real module name
                    body.push(Stmt::Assign(StmtAssign {
                        node_index: AtomicNodeIndex::dummy(),
                        targets: vec![Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: local_name.into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: "sys".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new("modules", TextRange::default()),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            slice: Box::new(self.create_string_literal(&full_module_path)),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    }));
                } else {
                    // It's truly an inlined module, create a namespace
                    let namespace_params = NamespaceImportParams {
                        local_name,
                        imported_name,
                        resolved_module: params.resolved_module,
                        full_module_path: &full_module_path,
                    };
                    self.create_namespace_for_inlined_module(
                        &namespace_params,
                        symbol_renames,
                        body,
                    );
                }
                continue;
            }

            // Handle regular symbol import from inlined module
            let symbol_params = SymbolImportParams {
                imported_name,
                local_name,
                resolved_module: params.resolved_module,
                ctx: params.ctx,
            };
            self.handle_symbol_import_from_inlined_module(&symbol_params, symbol_renames, body);
        }

        true
    }

    /// Check if an imported name refers to a module
    fn check_if_importing_module(
        &self,
        resolved_module: &str,
        imported_name: &str,
        full_module_path: &str,
    ) -> bool {
        if self.inlined_modules.contains(full_module_path)
            || self.bundled_modules.contains(full_module_path)
        {
            return true;
        }

        // Try without the first component if it exists
        if let Some(dot_pos) = resolved_module.find('.') {
            let without_prefix = &resolved_module[dot_pos + 1..];
            let alt_path = format!("{without_prefix}.{imported_name}");
            self.inlined_modules.contains(&alt_path) || self.bundled_modules.contains(&alt_path)
        } else {
            false
        }
    }

    /// Create a namespace object for an inlined module
    fn create_namespace_for_inlined_module(
        &self,
        params: &NamespaceImportParams,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        body: &mut Vec<Stmt>,
    ) {
        log::debug!(
            "Creating namespace object for module '{}' imported from '{}' - module was inlined",
            params.imported_name,
            params.resolved_module
        );

        // Find the actual module path that was inlined
        let inlined_module_key = if self.inlined_modules.contains(params.full_module_path) {
            params.full_module_path.to_string()
        } else if let Some(dot_pos) = params.resolved_module.find('.') {
            let without_prefix = &params.resolved_module[dot_pos + 1..];
            format!("{}.{}", without_prefix, params.imported_name)
        } else {
            params.full_module_path.to_string()
        };

        // Create a SimpleNamespace-like object
        body.push(Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: params.local_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        }));

        // Add symbols to the namespace
        let add_params = AddSymbolsParams {
            local_name: params.local_name,
            imported_name: params.imported_name,
            inlined_module_key: &inlined_module_key,
        };
        self.add_symbols_to_namespace(&add_params, symbol_renames, body);
    }

    /// Add symbols from an inlined module to a namespace object
    fn add_symbols_to_namespace(
        &self,
        params: &AddSymbolsParams,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        body: &mut Vec<Stmt>,
    ) {
        log::debug!(
            "add_symbols_to_namespace: local_name='{}', imported_name='{}', \
             inlined_module_key='{}'",
            params.local_name,
            params.imported_name,
            params.inlined_module_key
        );
        log::debug!(
            "Available keys in symbol_renames: {:?}",
            symbol_renames.keys().collect::<Vec<_>>()
        );

        // Get the renames from the symbol registry
        if let Some(module_renames) = symbol_renames.get(params.inlined_module_key).or_else(|| {
            // Try without prefix
            if let Some(dot_pos) = params.inlined_module_key.find('.') {
                let without_prefix = &params.inlined_module_key[dot_pos + 1..];
                log::debug!("Trying without prefix: '{without_prefix}'");
                symbol_renames.get(without_prefix)
            } else {
                None
            }
        }) {
            // Add each symbol from the module to the namespace
            for (original_name, renamed_name) in module_renames {
                // The renamed_name here is what was actually used when inlining the module
                // We should use it as-is since conflict checking was already done during inlining
                self.add_symbol_to_namespace(params.local_name, original_name, renamed_name, body);
            }
        } else {
            log::warn!(
                "No renames found for module '{}' when creating namespace '{}'",
                params.inlined_module_key,
                params.local_name
            );
        }
    }

    /// Add a single symbol to a namespace object
    fn add_symbol_to_namespace(
        &self,
        namespace_name: &str,
        original_name: &str,
        target_name: &str,
        body: &mut Vec<Stmt>,
    ) {
        body.push(Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: namespace_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(original_name, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: target_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        }));
    }

    /// Handle symbol import from an inlined module
    fn handle_symbol_import_from_inlined_module(
        &self,
        params: &SymbolImportParams,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        body: &mut Vec<Stmt>,
    ) {
        // Look up the renamed symbol in symbol_renames
        let module_key = if symbol_renames.contains_key(params.resolved_module) {
            params.resolved_module.to_string()
        } else if let Some(dot_pos) = params.resolved_module.find('.') {
            let without_prefix = &params.resolved_module[dot_pos + 1..];
            if symbol_renames.contains_key(without_prefix) {
                without_prefix.to_string()
            } else {
                params.resolved_module.to_string()
            }
        } else {
            params.resolved_module.to_string()
        };

        // Get the renamed symbol name
        let renamed_symbol = symbol_renames
            .get(&module_key)
            .and_then(|renames| renames.get(params.imported_name))
            .cloned()
            .unwrap_or_else(|| {
                log::warn!(
                    "Symbol '{}' from module '{}' not found in renames, using original name",
                    params.imported_name,
                    module_key
                );
                params.imported_name.to_string()
            });

        // Only create assignment if local name differs from the symbol
        if params.local_name != renamed_symbol {
            body.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: params.local_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: renamed_symbol.clone().into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }));
        }

        // Always set as module attribute
        body.push(self.create_module_attr_assignment("module", params.local_name));

        log::debug!(
            "Import '{}' as '{}' from inlined module '{}' resolved to '{}' in wrapper '{}'",
            params.imported_name,
            params.local_name,
            params.resolved_module,
            renamed_symbol,
            params.ctx.module_name
        );
    }

    /// Resolve a relative import to an absolute module name
    fn resolve_relative_import(
        &self,
        import_from: &StmtImportFrom,
        current_module: &str,
    ) -> Option<String> {
        self.resolve_relative_import_with_context(import_from, current_module, None)
    }

    /// Resolve a relative import to an absolute module name with optional module path context
    fn resolve_relative_import_with_context(
        &self,
        import_from: &StmtImportFrom,
        current_module: &str,
        module_path: Option<&Path>,
    ) -> Option<String> {
        log::debug!(
            "Resolving relative import: level={}, module={:?}, current_module={}",
            import_from.level,
            import_from.module,
            current_module
        );

        if import_from.level > 0 {
            // This is a relative import
            let mut parts: Vec<&str> = current_module.split('.').collect();

            // Special handling for different module types
            if parts.len() == 1 && import_from.level == 1 {
                // For single-component modules with level 1 imports, we need to determine
                // if this is a root-level module or a package __init__ file
                // Check if this module is in the inlined_modules or module_registry to determine if
                // it's a package
                let is_package = self
                    .bundled_modules
                    .iter()
                    .any(|m| m.starts_with(&format!("{current_module}.")));

                if is_package {
                    // This is a package __init__ file - level 1 imports stay in the package
                    log::debug!(
                        "Module '{current_module}' is a package, keeping parts for relative import"
                    );
                    // Keep parts as is
                } else {
                    // This is a root-level module - level 1 imports are siblings
                    log::debug!(
                        "Module '{current_module}' is root-level, clearing parts for relative \
                         import"
                    );
                    parts.clear();
                }
            } else {
                // For modules with multiple components (e.g., "greetings.greeting")
                // Special handling: if this module represents a package __init__.py file,
                // the first level doesn't remove anything (stays in the package)
                // Subsequent levels go up the hierarchy

                // Check if current module is a package __init__.py file
                let is_package_init = if let Some(path) = module_path {
                    path.file_name()
                        .and_then(|f| f.to_str())
                        .map(|f| f == "__init__.py")
                        .unwrap_or(false)
                } else {
                    // Fallback: check if module has submodules
                    self.bundled_modules
                        .iter()
                        .any(|m| m.starts_with(&format!("{current_module}.")))
                };

                let levels_to_remove = if is_package_init {
                    // For package __init__.py files, the first dot stays in the package
                    // So we remove (level - 1) parts
                    import_from.level.saturating_sub(1)
                } else {
                    // For regular modules, remove 'level' parts
                    import_from.level
                };

                log::debug!(
                    "Relative import resolution: current_module={}, is_package_init={}, level={}, \
                     levels_to_remove={}, parts={:?}",
                    current_module,
                    is_package_init,
                    import_from.level,
                    levels_to_remove,
                    parts
                );

                for _ in 0..levels_to_remove {
                    if parts.is_empty() {
                        log::debug!("Invalid relative import - ran out of parent levels");
                        return None; // Invalid relative import
                    }
                    parts.pop();
                }
            }

            // Add the module name if specified
            if let Some(ref module) = import_from.module {
                parts.push(module.as_str());
            }

            let resolved = parts.join(".");

            // Check for potential circular import
            if resolved == current_module {
                log::warn!("Potential circular import detected: {current_module} importing itself");
            }

            log::debug!("Resolved relative import to: {resolved}");
            Some(resolved)
        } else {
            // Not a relative import
            let resolved = import_from.module.as_ref().map(|m| m.as_str().to_string());
            log::debug!("Not a relative import, resolved to: {resolved:?}");
            resolved
        }
    }

    /// Find modules that have function-scoped imports (from import rewriting)
    fn find_modules_with_function_imports(
        &self,
        modules: &[(String, ModModule, PathBuf, String)],
    ) -> FxIndexSet<String> {
        let mut modules_with_function_imports = FxIndexSet::default();

        // Check each module for imports inside function bodies
        for (module_name, ast, _, _) in modules {
            if self.module_has_function_scoped_imports(ast) {
                log::debug!("Module '{module_name}' has function-scoped imports");
                modules_with_function_imports.insert(module_name.clone());
            }
        }

        modules_with_function_imports
    }

    /// Check if a module has imports inside function bodies or class methods
    fn module_has_function_scoped_imports(&self, ast: &ModModule) -> bool {
        for stmt in &ast.body {
            match stmt {
                // FunctionDef covers both sync and async functions (is_async field)
                Stmt::FunctionDef(func_def) => {
                    if Self::function_has_imports(&func_def.body) {
                        return true;
                    }
                }
                Stmt::ClassDef(class_def) => {
                    // Check methods inside the class (including async methods)
                    if self.class_has_method_imports(class_def) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Check if a class has methods with imports
    fn class_has_method_imports(&self, class_def: &StmtClassDef) -> bool {
        class_def.body.iter().any(|class_stmt| {
            matches!(class_stmt, Stmt::FunctionDef(method_def) if Self::function_has_imports(&method_def.body))
        })
    }

    /// Check if a function body contains import statements
    fn function_has_imports(body: &[Stmt]) -> bool {
        for stmt in body {
            match stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => return true,
                Stmt::If(if_stmt) => {
                    if Self::function_has_imports(&if_stmt.body) {
                        return true;
                    }
                    for clause in &if_stmt.elif_else_clauses {
                        if Self::function_has_imports(&clause.body) {
                            return true;
                        }
                    }
                }
                Stmt::While(while_stmt) => {
                    if Self::function_has_imports(&while_stmt.body) {
                        return true;
                    }
                }
                Stmt::For(for_stmt) => {
                    if Self::function_has_imports(&for_stmt.body) {
                        return true;
                    }
                }
                Stmt::Try(try_stmt) => {
                    if Self::function_has_imports(&try_stmt.body) {
                        return true;
                    }
                    for handler in &try_stmt.handlers {
                        let ExceptHandler::ExceptHandler(except_handler) = handler;
                        if Self::function_has_imports(&except_handler.body) {
                            return true;
                        }
                    }
                    if Self::function_has_imports(&try_stmt.orelse) {
                        return true;
                    }
                    if Self::function_has_imports(&try_stmt.finalbody) {
                        return true;
                    }
                }
                Stmt::With(with_stmt) => {
                    if Self::function_has_imports(&with_stmt.body) {
                        return true;
                    }
                }
                Stmt::FunctionDef(nested_func) => {
                    if Self::function_has_imports(&nested_func.body) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Find which modules are imported directly in all modules
    fn find_directly_imported_modules(
        &self,
        modules: &[(String, ModModule, PathBuf, String)],
        _entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut directly_imported = FxIndexSet::default();

        // Check all modules for direct imports (both module-level and function-scoped)
        for (module_name, ast, module_path, _) in modules {
            log::debug!("Checking module '{module_name}' for direct imports");
            let ctx = DirectImportContext {
                current_module: module_name,
                module_path,
                modules,
            };
            for stmt in &ast.body {
                self.collect_direct_imports(stmt, &ctx, &mut directly_imported);
                // Also check for imports inside functions, classes, etc.
                self.collect_direct_imports_recursive(stmt, &ctx, &mut directly_imported);
            }
        }

        log::debug!(
            "Found {} directly imported modules: {:?}",
            directly_imported.len(),
            directly_imported
        );
        directly_imported
    }

    /// Helper to collect direct imports from a statement
    fn collect_direct_imports(
        &self,
        stmt: &Stmt,
        ctx: &DirectImportContext<'_>,
        directly_imported: &mut FxIndexSet<String>,
    ) {
        match stmt {
            Stmt::Import(import_stmt) => {
                for alias in &import_stmt.names {
                    let imported_module = alias.name.as_str();
                    // Check if this is a bundled module
                    if ctx
                        .modules
                        .iter()
                        .any(|(name, _, _, _)| name == imported_module)
                    {
                        directly_imported.insert(imported_module.to_string());

                        // When importing a submodule, Python implicitly imports parent packages
                        // For example, importing 'greetings.irrelevant' also imports 'greetings'
                        if imported_module.contains('.') {
                            self.mark_parent_packages_as_imported(
                                imported_module,
                                ctx.modules,
                                directly_imported,
                            );
                        }
                    }
                }
            }
            Stmt::ImportFrom(import_from) => {
                if let Some(module) = &import_from.module {
                    let module_name = module.as_str();
                    // Check if any imported name is actually a submodule
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        let full_module_path = format!("{module_name}.{imported_name}");

                        log::debug!(
                            "Checking if '{full_module_path}' (from {module_name} import \
                             {imported_name}) is a bundled module"
                        );

                        // Check if this full path matches a bundled module
                        if ctx
                            .modules
                            .iter()
                            .any(|(name, _, _, _)| name == &full_module_path)
                        {
                            // This is importing a submodule directly
                            log::debug!(
                                "Found submodule import: from {module_name} import \
                                 {imported_name} -> {full_module_path}"
                            );
                            // Note: We don't mark this as directly imported anymore
                            // `from models import base` should allow `models.base` to be inlined
                            // if it has no side effects. Only `import models.base` should
                            // force wrapping.
                        }
                    }
                } else if import_from.level > 0 {
                    // Handle relative imports (e.g., from . import greeting)
                    self.collect_direct_relative_imports(import_from, ctx, directly_imported);
                }
            }
            _ => {}
        }
    }

    /// Collect direct imports from relative import statements
    fn collect_direct_relative_imports(
        &self,
        import_from: &StmtImportFrom,
        ctx: &DirectImportContext<'_>,
        directly_imported: &mut FxIndexSet<String>,
    ) {
        let resolved_module = self.resolve_relative_import_with_context(
            import_from,
            ctx.current_module,
            Some(ctx.module_path),
        );

        let Some(base_module) = resolved_module else {
            return;
        };

        // Check if any imported name is actually a submodule
        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let full_module_path = format!("{base_module}.{imported_name}");

            log::debug!(
                "Checking if '{full_module_path}' (from . import {imported_name}) is a bundled \
                 module"
            );

            // Check if this full path matches a bundled module
            let is_bundled_module = ctx
                .modules
                .iter()
                .any(|(name, _, _, _)| name == &full_module_path);

            if is_bundled_module {
                // This is importing a submodule directly
                log::debug!(
                    "Found direct submodule import via relative import: from . import \
                     {imported_name} -> {full_module_path}"
                );
                directly_imported.insert(full_module_path);
            }
        }
    }

    /// Mark parent packages as directly imported when a submodule is imported
    fn mark_parent_packages_as_imported(
        &self,
        imported_module: &str,
        modules: &[(String, ModModule, PathBuf, String)],
        directly_imported: &mut FxIndexSet<String>,
    ) {
        let parts: Vec<&str> = imported_module.split('.').collect();
        let mut parent = String::new();

        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                parent.push('.');
            }
            parent.push_str(part);

            // Only add if it's a bundled module (skip the last part as it's already added)
            if i < parts.len() - 1 && modules.iter().any(|(name, _, _, _)| name == &parent) {
                log::debug!(
                    "Marking parent package '{parent}' as directly imported (implicit import via \
                     '{imported_module}')"
                );
                directly_imported.insert(parent.clone());
            }
        }
    }

    /// Recursively collect direct imports from inside functions, classes, etc.
    fn collect_direct_imports_recursive(
        &self,
        stmt: &Stmt,
        ctx: &DirectImportContext<'_>,
        directly_imported: &mut FxIndexSet<String>,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Check imports inside function body
                for stmt in &func_def.body {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
            }
            Stmt::ClassDef(class_def) => {
                // Check imports inside class body
                for stmt in &class_def.body {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
            }
            Stmt::If(if_stmt) => {
                // Check imports inside if/elif/else blocks
                for stmt in &if_stmt.body {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
                for clause in &if_stmt.elif_else_clauses {
                    for stmt in &clause.body {
                        self.collect_direct_imports(stmt, ctx, directly_imported);
                        self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                // Check imports inside while body
                for stmt in &while_stmt.body {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
            }
            Stmt::For(for_stmt) => {
                // Check imports inside for body
                for stmt in &for_stmt.body {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
            }
            Stmt::Try(try_stmt) => {
                // Check imports inside try/except/else/finally blocks
                for stmt in &try_stmt.body {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(except_handler) = handler;
                    for stmt in &except_handler.body {
                        self.collect_direct_imports(stmt, ctx, directly_imported);
                        self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                    }
                }
                for stmt in &try_stmt.orelse {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
                for stmt in &try_stmt.finalbody {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
            }
            Stmt::With(with_stmt) => {
                // Check imports inside with body
                for stmt in &with_stmt.body {
                    self.collect_direct_imports(stmt, ctx, directly_imported);
                    self.collect_direct_imports_recursive(stmt, ctx, directly_imported);
                }
            }
            _ => {}
        }
    }

    /// Find modules that are imported at function scope
    fn find_function_imported_modules(
        &self,
        modules: &[(String, ModModule, PathBuf, String)],
    ) -> FxIndexSet<String> {
        let mut function_imported = FxIndexSet::default();

        // Use the ImportDiscoveryVisitor to find function-scoped imports
        for (_module_name, ast, _, _) in modules {
            use crate::visitors::{ImportDiscoveryVisitor, ImportLocation};

            let mut visitor = ImportDiscoveryVisitor::new();
            // Visit all statements in the module
            for stmt in &ast.body {
                use ruff_python_ast::visitor::Visitor;
                visitor.visit_stmt(stmt);
            }

            for import in visitor.into_imports() {
                // Check if this import is inside a function
                if matches!(
                    import.location,
                    ImportLocation::Function(_) | ImportLocation::Method { .. }
                ) {
                    // Get the module name from the import
                    if let Some(module_name) = &import.module_name {
                        // Check if this is a bundled module
                        if modules.iter().any(|(name, _, _, _)| name == module_name) {
                            log::debug!(
                                "Found function-scoped import of module '{}' at {:?}",
                                module_name,
                                import.location
                            );
                            function_imported.insert(module_name.clone());
                        }
                    }
                }
            }
        }

        function_imported
    }

    /// Find modules that are imported as namespaces (e.g., from models import base)
    /// Returns a map from module name to set of importing modules
    fn find_namespace_imported_modules(
        &mut self,
        modules: &[(String, ModModule, PathBuf, String)],
    ) {
        log::debug!(
            "find_namespace_imported_modules: Checking {} modules",
            modules.len()
        );

        // Check all modules for namespace imports
        for (importing_module, ast, _, _) in modules {
            log::debug!("Checking module '{importing_module}' for namespace imports");
            for stmt in &ast.body {
                self.collect_namespace_imports(stmt, modules, importing_module);
            }
        }

        log::debug!(
            "Found {} namespace imported modules: {:?}",
            self.namespace_imported_modules.len(),
            self.namespace_imported_modules
        );
    }

    /// Helper to collect namespace imports from a statement
    fn collect_namespace_imports(
        &mut self,
        stmt: &Stmt,
        modules: &[(String, ModModule, PathBuf, String)],
        importing_module: &str,
    ) {
        match stmt {
            Stmt::ImportFrom(import_from) if import_from.module.is_some() => {
                let module = import_from
                    .module
                    .as_ref()
                    .expect("module was checked to be Some");
                let module_name = module.as_str();
                log::debug!(
                    "  Found import from '{}' with names: {:?} in '{}'",
                    module_name,
                    import_from
                        .names
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>(),
                    importing_module
                );

                // Check if any imported name is actually a submodule
                for alias in &import_from.names {
                    let imported_name = alias.name.as_str();
                    let full_module_path = format!("{module_name}.{imported_name}");

                    log::debug!(
                        "  Checking if '{full_module_path}' (from {module_name} import \
                         {imported_name}) is a bundled module in namespace import check"
                    );

                    // Check if this full path matches a bundled module
                    let is_namespace_import = modules.iter().any(|(name, _, _, _)| {
                        name == &full_module_path || name.ends_with(&full_module_path)
                    });

                    if is_namespace_import {
                        // Find the actual module name that matched
                        let actual_module_name =
                            Self::find_matching_module_name_namespace(modules, &full_module_path);

                        // This is importing a submodule as a namespace
                        log::debug!(
                            "  Found namespace import: from {module_name} import {imported_name} \
                             -> {full_module_path} (actual: {actual_module_name}) in module \
                             {importing_module}"
                        );
                        self.namespace_imported_modules
                            .entry(actual_module_name)
                            .or_default()
                            .insert(importing_module.to_string());
                    }
                }
            }
            // Recursively check function bodies
            Stmt::FunctionDef(func_def) => {
                for body_stmt in &func_def.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            // Recursively check class bodies and methods
            Stmt::ClassDef(class_def) => {
                for body_stmt in &class_def.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            // Recursively check other compound statements
            Stmt::If(if_stmt) => {
                for body_stmt in &if_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for clause in &if_stmt.elif_else_clauses {
                    for body_stmt in &clause.body {
                        self.collect_namespace_imports(body_stmt, modules, importing_module);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                for body_stmt in &while_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for body_stmt in &while_stmt.orelse {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            Stmt::For(for_stmt) => {
                for body_stmt in &for_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for body_stmt in &for_stmt.orelse {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            Stmt::With(with_stmt) => {
                for body_stmt in &with_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            Stmt::Try(try_stmt) => {
                for body_stmt in &try_stmt.body {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    for body_stmt in &eh.body {
                        self.collect_namespace_imports(body_stmt, modules, importing_module);
                    }
                }
                for body_stmt in &try_stmt.orelse {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
                for body_stmt in &try_stmt.finalbody {
                    self.collect_namespace_imports(body_stmt, modules, importing_module);
                }
            }
            _ => {}
        }
    }

    /// Collect all defined symbols in the global scope
    fn collect_global_symbols(
        &self,
        modules: &[(String, ModModule, PathBuf, String)],
        entry_module_name: &str,
    ) -> FxIndexSet<String> {
        let mut global_symbols = FxIndexSet::default();

        // Collect symbols from all modules that will be in the bundle
        for (module_name, ast, _, _) in modules {
            if module_name == entry_module_name {
                // For entry module, collect all top-level symbols
                for stmt in &ast.body {
                    self.collect_symbol_from_statement(stmt, &mut global_symbols);
                }
            }
        }

        global_symbols
    }

    /// Helper to collect symbols from a statement
    fn collect_symbol_from_statement(&self, stmt: &Stmt, global_symbols: &mut FxIndexSet<String>) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                global_symbols.insert(func_def.name.to_string());
            }
            Stmt::ClassDef(class_def) => {
                global_symbols.insert(class_def.name.to_string());
            }
            Stmt::Assign(assign) => {
                if let Some(name) = self.extract_simple_assign_target(assign) {
                    global_symbols.insert(name);
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                if let Expr::Name(name) = ann_assign.target.as_ref() {
                    global_symbols.insert(name.id.to_string());
                }
            }
            _ => {}
        }
    }

    /// Generate a unique symbol name to avoid conflicts
    fn generate_unique_name(
        &self,
        base_name: &str,
        existing_symbols: &FxIndexSet<String>,
    ) -> String {
        if !existing_symbols.contains(base_name) {
            return base_name.to_string();
        }

        // Try adding numeric suffixes
        for i in 1..1000 {
            let candidate = format!("{base_name}_{i}");
            if !existing_symbols.contains(&candidate) {
                return candidate;
            }
        }

        // Fallback with module prefix
        format!("__cribo_renamed_{base_name}")
    }

    /// Get a unique name for a symbol by appending module suffix
    fn get_unique_name_with_module_suffix(&self, base_name: &str, module_name: &str) -> String {
        let module_suffix = module_name.cow_replace('.', "_").into_owned();
        format!("{base_name}_{module_suffix}")
    }

    /// Get a unique name for a symbol, using the same pattern as generate_unique_name
    fn get_unique_name(&self, base_name: &str, existing_symbols: &FxIndexSet<String>) -> String {
        self.generate_unique_name(base_name, existing_symbols)
    }

    /// Reorder statements in a module based on symbol dependencies for circular modules
    fn reorder_statements_for_circular_module(
        &self,
        module_name: &str,
        statements: Vec<Stmt>,
    ) -> Vec<Stmt> {
        // Get the ordered symbols for this module from the dependency graph
        let ordered_symbols = self
            .symbol_dep_graph
            .get_module_symbols_ordered(module_name);

        if ordered_symbols.is_empty() {
            // No ordering information, return statements as-is
            return statements;
        }

        log::debug!(
            "Reordering statements for circular module '{module_name}' based on symbol order: \
             {ordered_symbols:?}"
        );

        // Create a map from symbol name to statement
        let mut symbol_to_stmt: FxIndexMap<String, Stmt> = FxIndexMap::default();
        let mut other_stmts = Vec::new();
        let mut imports = Vec::new();

        for stmt in statements {
            match &stmt {
                Stmt::FunctionDef(func_def) => {
                    symbol_to_stmt.insert(func_def.name.to_string(), stmt);
                }
                Stmt::ClassDef(class_def) => {
                    symbol_to_stmt.insert(class_def.name.to_string(), stmt);
                }
                Stmt::Assign(assign) => {
                    if let Some(name) = self.extract_simple_assign_target(assign) {
                        // Skip self-referential assignments - they'll be handled later
                        if self.is_self_referential_assignment(assign) {
                            log::debug!(
                                "Skipping self-referential assignment '{name}' in circular module \
                                 reordering"
                            );
                            other_stmts.push(stmt);
                        } else if symbol_to_stmt.contains_key(&name) {
                            // If we already have a function/class with this name, keep the
                            // function/class and treat the assignment
                            // as a regular statement
                            log::debug!(
                                "Assignment '{name}' conflicts with existing function/class, \
                                 keeping function/class"
                            );
                            other_stmts.push(stmt);
                        } else {
                            symbol_to_stmt.insert(name, stmt);
                        }
                    } else {
                        other_stmts.push(stmt);
                    }
                }
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // Keep imports at the beginning
                    imports.push(stmt);
                }
                _ => {
                    // Other statements maintain their relative order
                    other_stmts.push(stmt);
                }
            }
        }

        // Build the reordered statement list
        let mut reordered = Vec::new();

        // First, add all imports
        reordered.extend(imports);

        // Then add symbols in the specified order
        for symbol in &ordered_symbols {
            if let Some(stmt) = symbol_to_stmt.shift_remove(symbol) {
                reordered.push(stmt);
            }
        }

        // Add any remaining symbols that weren't in the ordered list
        reordered.extend(symbol_to_stmt.into_values());

        // Finally, add other statements
        reordered.extend(other_stmts);

        reordered
    }

    /// Reorder statements to ensure module-level variables are declared before use
    fn reorder_statements_for_proper_declaration_order(&self, statements: Vec<Stmt>) -> Vec<Stmt> {
        let mut imports = Vec::new();
        let mut assignments = Vec::new();
        let mut self_assignments = Vec::new();
        let mut functions_and_classes = Vec::new();
        let mut other_stmts = Vec::new();

        // Categorize statements
        for stmt in statements {
            match &stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    imports.push(stmt);
                }
                Stmt::Assign(assign) => {
                    // Check if this is a self-assignment (e.g., validate = validate)
                    let is_self_assignment = if assign.targets.len() == 1 {
                        if let (Expr::Name(target), Expr::Name(value)) =
                            (&assign.targets[0], assign.value.as_ref())
                        {
                            target.id == value.id
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if is_self_assignment {
                        // Self-assignments should come after function definitions
                        self_assignments.push(stmt);
                    } else {
                        // Regular module-level variable assignments
                        assignments.push(stmt);
                    }
                }
                Stmt::AnnAssign(_) => {
                    // Annotated assignments are regular variable declarations
                    assignments.push(stmt);
                }
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                    // Functions and classes that might reference module-level variables
                    functions_and_classes.push(stmt);
                }
                _ => {
                    other_stmts.push(stmt);
                }
            }
        }

        // Build the reordered list:
        // 1. Imports first
        // 2. Module-level assignments (variables) - but not self-assignments
        // 3. Functions and classes
        // 4. Self-assignments (after functions are defined)
        // 5. Other statements
        let mut reordered = Vec::new();
        reordered.extend(imports);
        reordered.extend(assignments);
        reordered.extend(functions_and_classes);
        reordered.extend(self_assignments);
        reordered.extend(other_stmts);

        reordered
    }

    /// Inline a module without side effects directly into the bundle
    fn inline_module(
        &mut self,
        module_name: &str,
        mut ast: ModModule,
        module_path: &Path,
        ctx: &mut InlineContext,
    ) -> Result<Vec<Stmt>> {
        let mut module_renames = FxIndexMap::default();

        // First, apply recursive import transformation to the module
        let mut transformer = RecursiveImportTransformer::new(RecursiveImportTransformerParams {
            bundler: self,
            module_name,
            module_path: Some(module_path),
            symbol_renames: ctx.module_renames,
            deferred_imports: ctx.deferred_imports,
            is_entry_module: false, // This is not the entry module
            is_wrapper_init: false, // Not a wrapper init
            global_deferred_imports: Some(&self.global_deferred_imports), // Pass global registry
        });
        transformer.transform_module(&mut ast);

        // Reorder statements to ensure proper declaration order
        let statements = if self.circular_modules.contains(module_name) {
            self.reorder_statements_for_circular_module(module_name, ast.body)
        } else {
            // Even for non-circular modules, ensure module-level variables are declared
            // before functions that might use them
            self.reorder_statements_for_proper_declaration_order(ast.body)
        };

        // Process each statement in the module
        for stmt in statements {
            match &stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // Imports have already been transformed by RecursiveImportTransformer
                    // Include them in the inlined output
                    if !self.is_hoisted_import(&stmt) {
                        ctx.inlined_stmts.push(stmt.clone());
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    let func_name = func_def.name.to_string();
                    if !self.should_inline_symbol(&func_name, module_name, ctx.module_exports_map) {
                        continue;
                    }

                    // Check if this symbol was renamed by semantic analysis
                    let renamed_name = if let Some(module_rename_map) =
                        ctx.module_renames.get(module_name)
                    {
                        if let Some(new_name) = module_rename_map.get(&func_name) {
                            // Only use semantic rename if it's actually different
                            if new_name != &func_name {
                                log::debug!(
                                    "Using semantic rename for '{func_name}' to '{new_name}' in \
                                     module '{module_name}'"
                                );
                                new_name.clone()
                            } else {
                                // Semantic rename is same as original, check if there's a conflict
                                if ctx.global_symbols.contains(&func_name) {
                                    // There's a conflict, apply module suffix pattern
                                    let base_name = self.get_unique_name_with_module_suffix(
                                        &func_name,
                                        module_name,
                                    );
                                    self.get_unique_name(&base_name, ctx.global_symbols)
                                } else {
                                    // No conflict, use original name
                                    func_name.clone()
                                }
                            }
                        } else {
                            // No semantic rename, check if there's a conflict
                            if ctx.global_symbols.contains(&func_name) {
                                // There's a conflict, apply module suffix pattern
                                let base_name = self
                                    .get_unique_name_with_module_suffix(&func_name, module_name);
                                self.get_unique_name(&base_name, ctx.global_symbols)
                            } else {
                                // No conflict, use original name
                                func_name.clone()
                            }
                        }
                    } else {
                        // No semantic rename, check if there's a conflict
                        if ctx.global_symbols.contains(&func_name) {
                            // There's a conflict, apply module suffix pattern
                            let base_name =
                                self.get_unique_name_with_module_suffix(&func_name, module_name);
                            self.get_unique_name(&base_name, ctx.global_symbols)
                        } else {
                            // No conflict, use original name
                            func_name.clone()
                        }
                    };

                    // Always track the symbol mapping, even if not renamed
                    module_renames.insert(func_name.clone(), renamed_name.clone());
                    ctx.global_symbols.insert(renamed_name.clone());

                    // Clone and rename the function
                    let mut func_def_clone = func_def.clone();
                    func_def_clone.name = Identifier::new(renamed_name, TextRange::default());

                    // Apply renames to function annotations (parameters and return type)
                    // Apply renames to function annotations (parameters and return type)
                    if let Some(ref mut returns) = func_def_clone.returns {
                        self.resolve_import_aliases_in_expr(returns, &ctx.import_aliases);
                        self.rewrite_aliases_in_expr(returns, &module_renames);
                    }

                    // Apply renames to parameter annotations
                    for param in &mut func_def_clone.parameters.args {
                        if let Some(ref mut annotation) = param.parameter.annotation {
                            self.resolve_import_aliases_in_expr(annotation, &ctx.import_aliases);
                            self.rewrite_aliases_in_expr(annotation, &module_renames);
                        }
                    }

                    // Apply renames and resolve import aliases in function body
                    for body_stmt in &mut func_def_clone.body {
                        self.resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
                        self.rewrite_aliases_in_stmt(body_stmt, &module_renames);
                        // Also apply semantic renames from context
                        if let Some(semantic_renames) = ctx.module_renames.get(module_name) {
                            self.rewrite_aliases_in_stmt(body_stmt, semantic_renames);
                        }
                    }

                    ctx.inlined_stmts.push(Stmt::FunctionDef(func_def_clone));
                }
                Stmt::ClassDef(class_def) => {
                    self.inline_class(class_def, module_name, &mut module_renames, ctx);
                }
                Stmt::Assign(assign) => {
                    self.inline_assignment(assign, module_name, &mut module_renames, ctx);
                }
                Stmt::AnnAssign(ann_assign) => {
                    self.inline_ann_assignment(ann_assign, module_name, &mut module_renames, ctx);
                }
                // TypeAlias statements are safe metadata definitions
                Stmt::TypeAlias(_) => {
                    // Type aliases don't need renaming in Python, they're just metadata
                    ctx.inlined_stmts.push(stmt);
                }
                // Pass statements are no-ops and safe
                Stmt::Pass(_) => {
                    // Pass statements can be included as-is
                    ctx.inlined_stmts.push(stmt);
                }
                // Expression statements that are string literals are docstrings
                Stmt::Expr(expr_stmt) => {
                    if matches!(expr_stmt.value.as_ref(), Expr::StringLiteral(_)) {
                        // This is a docstring - safe to include
                        ctx.inlined_stmts.push(stmt);
                    } else {
                        // Other expression statements shouldn't exist in side-effect-free modules
                        log::warn!(
                            "Unexpected expression statement in side-effect-free module \
                             '{module_name}': {stmt:?}"
                        );
                    }
                }
                _ => {
                    // Any other statement type that we haven't explicitly handled
                    log::warn!(
                        "Unexpected statement type in side-effect-free module '{module_name}': \
                         {stmt:?}"
                    );
                }
            }
        }

        // Store the renames for this module
        if !module_renames.is_empty() {
            ctx.module_renames
                .insert(module_name.to_string(), module_renames);
        }

        Ok(Vec::new()) // Statements are accumulated in ctx.inlined_stmts
    }

    /// Transform a function body to use renamed symbols
    fn transform_function_body_for_renames(
        &self,
        func_def: &mut StmtFunctionDef,
        module_renames: &FxIndexMap<String, String>,
    ) {
        for stmt in &mut func_def.body {
            self.transform_stmt_for_renames(stmt, module_renames);
        }
    }

    /// Transform a statement to use renamed symbols
    fn transform_stmt_for_renames(
        &self,
        stmt: &mut Stmt,
        module_renames: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::Expr(expr_stmt) => {
                Self::rename_references_in_expr(&mut expr_stmt.value, module_renames);
            }
            Stmt::Assign(assign) => {
                // Rename assignment targets if they're renamed globals
                for target in &mut assign.targets {
                    if let Expr::Name(name_expr) = target
                        && let Some(renamed) = module_renames.get(name_expr.id.as_str())
                    {
                        name_expr.id = renamed.clone().into();
                    }
                }
                // Also rename values (RHS)
                Self::rename_references_in_expr(&mut assign.value, module_renames);
            }
            Stmt::Return(ret_stmt) => {
                if let Some(value) = &mut ret_stmt.value {
                    Self::rename_references_in_expr(value, module_renames);
                }
            }
            Stmt::If(if_stmt) => {
                Self::rename_references_in_expr(&mut if_stmt.test, module_renames);
                for body_stmt in &mut if_stmt.body {
                    self.transform_stmt_for_renames(body_stmt, module_renames);
                }
                for elif_else_clause in &mut if_stmt.elif_else_clauses {
                    if let Some(test_expr) = &mut elif_else_clause.test {
                        Self::rename_references_in_expr(test_expr, module_renames);
                    }
                    for body_stmt in &mut elif_else_clause.body {
                        self.transform_stmt_for_renames(body_stmt, module_renames);
                    }
                }
            }
            Stmt::For(for_stmt) => {
                Self::rename_references_in_expr(&mut for_stmt.iter, module_renames);
                for body_stmt in &mut for_stmt.body {
                    self.transform_stmt_for_renames(body_stmt, module_renames);
                }
                for orelse_stmt in &mut for_stmt.orelse {
                    self.transform_stmt_for_renames(orelse_stmt, module_renames);
                }
            }
            Stmt::While(while_stmt) => {
                Self::rename_references_in_expr(&mut while_stmt.test, module_renames);
                for body_stmt in &mut while_stmt.body {
                    self.transform_stmt_for_renames(body_stmt, module_renames);
                }
                for orelse_stmt in &mut while_stmt.orelse {
                    self.transform_stmt_for_renames(orelse_stmt, module_renames);
                }
            }
            Stmt::FunctionDef(inner_func) => {
                // Recursively transform nested functions
                self.transform_function_body_for_renames(inner_func, module_renames);
            }
            Stmt::ClassDef(class_def) => {
                // Transform methods in the class
                for body_stmt in &mut class_def.body {
                    self.transform_stmt_for_renames(body_stmt, module_renames);
                }
            }
            Stmt::Global(global_stmt) => {
                // Rename global variable names
                for name in &mut global_stmt.names {
                    if let Some(renamed) = module_renames.get(name.as_str()) {
                        *name = Identifier::new(renamed, TextRange::default());
                    }
                }
            }
            // Add more cases as needed
            _ => {}
        }
    }

    /// Rename references in an expression based on module renames
    fn rename_references_in_expr(expr: &mut Expr, module_renames: &FxIndexMap<String, String>) {
        match expr {
            Expr::Name(name_expr) => {
                if let Some(renamed) = module_renames.get(name_expr.id.as_str()) {
                    name_expr.id = renamed.clone().into();
                }
            }
            Expr::Attribute(attr_expr) => {
                Self::rename_references_in_expr(&mut attr_expr.value, module_renames);
            }
            Expr::Call(call_expr) => {
                Self::rename_references_in_expr(&mut call_expr.func, module_renames);
                for arg in &mut call_expr.arguments.args {
                    Self::rename_references_in_expr(arg, module_renames);
                }
                for keyword in &mut call_expr.arguments.keywords {
                    Self::rename_references_in_expr(&mut keyword.value, module_renames);
                }
            }
            Expr::FString(fstring) => {
                // Handle FString transformation
                let fstring_range = fstring.range;
                let mut transformed_elements = Vec::new();
                let mut any_transformed = false;

                for element in fstring.value.elements() {
                    match element {
                        InterpolatedStringElement::Literal(lit_elem) => {
                            transformed_elements
                                .push(InterpolatedStringElement::Literal(lit_elem.clone()));
                        }
                        InterpolatedStringElement::Interpolation(expr_elem) => {
                            let mut new_expr = expr_elem.expression.clone();
                            Self::rename_references_in_expr(&mut new_expr, module_renames);

                            let new_element = InterpolatedElement {
                                node_index: AtomicNodeIndex::dummy(),
                                expression: new_expr,
                                debug_text: expr_elem.debug_text.clone(),
                                conversion: expr_elem.conversion,
                                format_spec: expr_elem.format_spec.clone(),
                                range: expr_elem.range,
                            };
                            transformed_elements
                                .push(InterpolatedStringElement::Interpolation(new_element));
                            any_transformed = true;
                        }
                    }
                }

                if any_transformed {
                    let new_fstring = FString {
                        node_index: AtomicNodeIndex::dummy(),
                        elements: InterpolatedStringElements::from(transformed_elements),
                        range: TextRange::default(),
                        flags: FStringFlags::empty(),
                    };

                    let new_value = FStringValue::single(new_fstring);

                    *expr = Expr::FString(ExprFString {
                        node_index: AtomicNodeIndex::dummy(),
                        value: new_value,
                        range: fstring_range,
                    });
                }
            }
            Expr::BinOp(binop) => {
                Self::rename_references_in_expr(&mut binop.left, module_renames);
                Self::rename_references_in_expr(&mut binop.right, module_renames);
            }
            Expr::Compare(compare) => {
                Self::rename_references_in_expr(&mut compare.left, module_renames);
                for comparator in &mut compare.comparators {
                    Self::rename_references_in_expr(comparator, module_renames);
                }
            }
            Expr::BoolOp(boolop) => {
                for value in &mut boolop.values {
                    Self::rename_references_in_expr(value, module_renames);
                }
            }
            Expr::UnaryOp(unaryop) => {
                Self::rename_references_in_expr(&mut unaryop.operand, module_renames);
            }
            Expr::If(ifexp) => {
                Self::rename_references_in_expr(&mut ifexp.test, module_renames);
                Self::rename_references_in_expr(&mut ifexp.body, module_renames);
                Self::rename_references_in_expr(&mut ifexp.orelse, module_renames);
            }
            Expr::Dict(dict_expr) => {
                for item in &mut dict_expr.items {
                    if let Some(key) = &mut item.key {
                        Self::rename_references_in_expr(key, module_renames);
                    }
                    Self::rename_references_in_expr(&mut item.value, module_renames);
                }
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    Self::rename_references_in_expr(elem, module_renames);
                }
            }
            Expr::Tuple(tuple_expr) => {
                for elem in &mut tuple_expr.elts {
                    Self::rename_references_in_expr(elem, module_renames);
                }
            }
            Expr::Set(set_expr) => {
                for elem in &mut set_expr.elts {
                    Self::rename_references_in_expr(elem, module_renames);
                }
            }
            Expr::Subscript(subscript) => {
                // Special handling for globals()["symbol"] pattern
                if let Expr::Call(call) = subscript.value.as_ref()
                    && let Expr::Name(name) = &*call.func
                    && name.id.as_str() == "globals"
                    && call.arguments.args.is_empty()
                {
                    // This is a globals()["..."] access
                    if let Expr::StringLiteral(string_lit) = subscript.slice.as_ref()
                        && let Some(first_lit) = string_lit.value.iter().next()
                    {
                        let symbol_name = &*first_lit.value;
                        if let Some(renamed) = module_renames.get(symbol_name) {
                            // Replace the string literal with the renamed symbol
                            log::debug!(
                                "Rewriting globals()[{symbol_name:?}] to globals()[{renamed:?}]"
                            );
                            *expr = Expr::Subscript(ExprSubscript {
                                node_index: AtomicNodeIndex::dummy(),
                                value: subscript.value.clone(),
                                slice: Box::new(Expr::StringLiteral(ExprStringLiteral {
                                    node_index: AtomicNodeIndex::dummy(),
                                    value: StringLiteralValue::single(StringLiteral {
                                        node_index: AtomicNodeIndex::dummy(),
                                        value: renamed.clone().into_boxed_str(),
                                        range: TextRange::default(),
                                        flags: StringLiteralFlags::empty(),
                                    }),
                                    range: TextRange::default(),
                                })),
                                ctx: subscript.ctx,
                                range: subscript.range,
                            });
                            return;
                        }
                    }
                }

                // Default handling
                Self::rename_references_in_expr(&mut subscript.value, module_renames);
                Self::rename_references_in_expr(&mut subscript.slice, module_renames);
            }
            Expr::Lambda(lambda) => {
                // Don't rename parameters, but do rename body
                Self::rename_references_in_expr(&mut lambda.body, module_renames);
            }
            Expr::ListComp(comp) => {
                Self::rename_references_in_expr(&mut comp.elt, module_renames);
                for generator in &mut comp.generators {
                    Self::rename_references_in_expr(&mut generator.iter, module_renames);
                    for if_clause in &mut generator.ifs {
                        Self::rename_references_in_expr(if_clause, module_renames);
                    }
                }
            }
            Expr::SetComp(comp) => {
                Self::rename_references_in_expr(&mut comp.elt, module_renames);
                for generator in &mut comp.generators {
                    Self::rename_references_in_expr(&mut generator.iter, module_renames);
                    for if_clause in &mut generator.ifs {
                        Self::rename_references_in_expr(if_clause, module_renames);
                    }
                }
            }
            Expr::Generator(comp) => {
                Self::rename_references_in_expr(&mut comp.elt, module_renames);
                for generator in &mut comp.generators {
                    Self::rename_references_in_expr(&mut generator.iter, module_renames);
                    for if_clause in &mut generator.ifs {
                        Self::rename_references_in_expr(if_clause, module_renames);
                    }
                }
            }
            Expr::DictComp(comp) => {
                Self::rename_references_in_expr(&mut comp.key, module_renames);
                Self::rename_references_in_expr(&mut comp.value, module_renames);
                for generator in &mut comp.generators {
                    Self::rename_references_in_expr(&mut generator.iter, module_renames);
                    for if_clause in &mut generator.ifs {
                        Self::rename_references_in_expr(if_clause, module_renames);
                    }
                }
            }
            // Add more cases as needed
            _ => {}
        }
    }

    /// Process import statements in wrapper modules
    fn process_wrapper_module_import(
        &self,
        stmt: Stmt,
        ctx: &ModuleTransformContext,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        body: &mut Vec<Stmt>,
    ) {
        if self.is_hoisted_import(&stmt) {
            return;
        }

        let mut handled_inlined_import = false;

        // For wrapper modules, we need special handling for imports from inlined modules
        if let Stmt::ImportFrom(import_from) = &stmt {
            // Check if this is importing from an inlined module
            let resolved_module = self.resolve_relative_import_with_context(
                import_from,
                ctx.module_name,
                Some(ctx.module_path),
            );
            log::debug!(
                "Checking import from {:?} in wrapper module {}: resolved to {:?}",
                import_from.module.as_ref().map(|m| m.as_str()),
                ctx.module_name,
                resolved_module
            );
            if let Some(ref resolved) = resolved_module {
                let params = InlinedImportParams {
                    import_from,
                    resolved_module: resolved,
                    ctx,
                };
                handled_inlined_import =
                    self.handle_inlined_module_import(&params, symbol_renames, body);
            }
        }

        // Only do standard transformation if we didn't handle it as an inlined import
        if !handled_inlined_import {
            // For other imports, use the standard transformation
            log::debug!(
                "Standard import transformation for {:?} in wrapper module '{}'",
                match &stmt {
                    Stmt::ImportFrom(imp) =>
                        format!("from {:?}", imp.module.as_ref().map(|m| m.as_str())),
                    _ => "non-import".to_string(),
                },
                ctx.module_name
            );
            let empty_renames = FxIndexMap::default();
            let transformed_stmts = self.rewrite_import_in_stmt_multiple_with_full_context(
                stmt.clone(),
                ctx.module_name,
                &empty_renames,
                true, // inside_wrapper_init = true since we're creating a wrapper module
            );
            body.extend(transformed_stmts);

            // Check if any imported symbols should be re-exported as module attributes
            self.add_imported_symbol_attributes(
                &stmt,
                ctx.module_name,
                Some(ctx.module_path),
                body,
            );
        }
    }

    /// Collect global declarations from a function body
    fn collect_function_globals(&self, body: &[Stmt]) -> FxIndexSet<String> {
        let mut function_globals = FxIndexSet::default();
        for stmt in body {
            if let Stmt::Global(global_stmt) = stmt {
                for name in &global_stmt.names {
                    function_globals.insert(name.to_string());
                }
            }
        }
        function_globals
    }

    /// Create initialization statements for lifted globals
    fn create_global_init_statements(
        &self,
        _function_globals: &FxIndexSet<String>,
        _lifted_names: &FxIndexMap<String, String>,
    ) -> Vec<Stmt> {
        // No initialization statements needed - global declarations mean
        // we use the lifted names directly, not through local variables
        Vec::new()
    }

    /// Transform function body for lifted globals
    fn transform_function_body_for_lifted_globals(
        &self,
        func_def: &mut StmtFunctionDef,
        params: &TransformFunctionParams,
        init_stmts: Vec<Stmt>,
    ) {
        let mut new_body = Vec::new();
        let mut added_init = false;

        for body_stmt in &mut func_def.body {
            match body_stmt {
                Stmt::Global(global_stmt) => {
                    // Rewrite global statement to use lifted names
                    for name in &mut global_stmt.names {
                        if let Some(lifted_name) = params.lifted_names.get(name.as_str()) {
                            *name = Identifier::new(lifted_name, TextRange::default());
                        }
                    }
                    new_body.push(body_stmt.clone());

                    // Add initialization statements after global declarations
                    if !added_init && !init_stmts.is_empty() {
                        new_body.extend(init_stmts.clone());
                        added_init = true;
                    }
                }
                _ => {
                    // Transform other statements recursively with function context
                    self.transform_stmt_for_lifted_globals(
                        body_stmt,
                        params.lifted_names,
                        params.global_info,
                        Some(params.function_globals),
                    );
                    new_body.push(body_stmt.clone());

                    // After transforming, check if we need to add synchronization
                    self.add_global_sync_if_needed(
                        body_stmt,
                        params.function_globals,
                        params.lifted_names,
                        &mut new_body,
                    );
                }
            }
        }

        // Replace function body with new body
        func_def.body = new_body;
    }

    /// Add synchronization statements for global variable modifications
    fn add_global_sync_if_needed(
        &self,
        stmt: &Stmt,
        function_globals: &FxIndexSet<String>,
        lifted_names: &FxIndexMap<String, String>,
        new_body: &mut Vec<Stmt>,
    ) {
        match stmt {
            Stmt::Assign(assign) => {
                // Check if this is an assignment to a global variable
                if let [Expr::Name(name)] = &assign.targets[..] {
                    let var_name = name.id.as_str();

                    // The variable name might already be transformed to the lifted name,
                    // so we need to check if it's a lifted variable
                    if let Some(original_name) = lifted_names
                        .iter()
                        .find(|(orig, lifted)| {
                            lifted.as_str() == var_name && function_globals.contains(orig.as_str())
                        })
                        .map(|(orig, _)| orig)
                    {
                        log::debug!(
                            "Adding sync for assignment to global {var_name}: {var_name} -> \
                             module.{original_name}"
                        );
                        // Add: module.<original_name> = <lifted_name>
                        new_body.push(Stmt::Assign(StmtAssign {
                            node_index: AtomicNodeIndex::dummy(),
                            targets: vec![Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: "module".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new(original_name, TextRange::default()),
                                ctx: ExprContext::Store,
                                range: TextRange::default(),
                            })],
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: var_name.into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
                    }
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Check if this is an augmented assignment to a global variable
                if let Expr::Name(name) = aug_assign.target.as_ref() {
                    let var_name = name.id.as_str();
                    log::debug!(
                        "Checking augmented assignment to {var_name}, function_globals: \
                         {function_globals:?}, lifted_names: {lifted_names:?}"
                    );

                    // The variable name might already be transformed to the lifted name,
                    // so we need to check if it's a lifted variable
                    if let Some(original_name) = lifted_names
                        .iter()
                        .find(|(orig, lifted)| {
                            lifted.as_str() == var_name && function_globals.contains(orig.as_str())
                        })
                        .map(|(orig, _)| orig)
                    {
                        log::debug!(
                            "Adding sync for augmented assignment to global {var_name}: \
                             {var_name} -> module.{original_name}"
                        );
                        // Add: module.<original_name> = <lifted_name>
                        new_body.push(Stmt::Assign(StmtAssign {
                            node_index: AtomicNodeIndex::dummy(),
                            targets: vec![Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: "module".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new(original_name, TextRange::default()),
                                ctx: ExprContext::Store,
                                range: TextRange::default(),
                            })],
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: var_name.into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
                    }
                }
            }
            _ => {
                // Other statement types don't modify globals directly
            }
        }
    }

    /// Transform f-string expressions for lifted globals
    fn transform_fstring_for_lifted_globals(
        &self,
        expr: &mut Expr,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &ModuleGlobalInfo,
        in_function_with_globals: Option<&FxIndexSet<String>>,
    ) {
        if let Expr::FString(fstring) = expr {
            let fstring_range = fstring.range;
            let mut transformed_elements = Vec::new();
            let mut any_transformed = false;

            for element in fstring.value.elements() {
                match element {
                    InterpolatedStringElement::Literal(lit_elem) => {
                        // Literal elements stay the same
                        transformed_elements
                            .push(InterpolatedStringElement::Literal(lit_elem.clone()));
                    }
                    InterpolatedStringElement::Interpolation(expr_elem) => {
                        let (new_element, was_transformed) = self.transform_fstring_expression(
                            expr_elem,
                            lifted_names,
                            global_info,
                            in_function_with_globals,
                        );
                        transformed_elements
                            .push(InterpolatedStringElement::Interpolation(new_element));
                        if was_transformed {
                            any_transformed = true;
                        }
                    }
                }
            }

            // If any expressions were transformed, we need to rebuild the f-string
            if any_transformed {
                // Create a new FString with our transformed elements
                let new_fstring = FString {
                    node_index: AtomicNodeIndex::dummy(),
                    elements: InterpolatedStringElements::from(transformed_elements),
                    range: TextRange::default(),
                    flags: FStringFlags::empty(),
                };

                // Create a new FStringValue containing our FString
                let new_value = FStringValue::single(new_fstring);

                // Replace the entire expression with the new f-string
                *expr = Expr::FString(ExprFString {
                    node_index: AtomicNodeIndex::dummy(),
                    value: new_value,
                    range: fstring_range,
                });

                log::debug!("Transformed f-string expressions for lifted globals");
            }
        }
    }

    /// Transform a single f-string expression element
    fn transform_fstring_expression(
        &self,
        expr_elem: &InterpolatedElement,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &ModuleGlobalInfo,
        in_function_with_globals: Option<&FxIndexSet<String>>,
    ) -> (InterpolatedElement, bool) {
        // Clone and transform the expression
        let mut new_expr = (*expr_elem.expression).clone();
        let old_expr_str = format!("{new_expr:?}");

        self.transform_expr_for_lifted_globals(
            &mut new_expr,
            lifted_names,
            global_info,
            in_function_with_globals,
        );

        let new_expr_str = format!("{new_expr:?}");
        let was_transformed = old_expr_str != new_expr_str;

        // Create a new expression element with the transformed expression
        let new_element = InterpolatedElement {
            node_index: AtomicNodeIndex::dummy(),
            expression: Box::new(new_expr),
            debug_text: expr_elem.debug_text.clone(),
            conversion: expr_elem.conversion,
            format_spec: expr_elem.format_spec.clone(),
            range: expr_elem.range,
        };

        (new_element, was_transformed)
    }

    /// Track import aliases from a statement
    fn track_import_aliases(
        &mut self,
        import_from: &StmtImportFrom,
        module_name: &str,
        module_path: &Path,
        ctx: &mut InlineContext,
    ) {
        let resolved_module =
            self.resolve_relative_import_with_context(import_from, module_name, Some(module_path));
        if let Some(resolved) = resolved_module {
            // Handle imports from wrapper modules in namespace hybrid context
            // These need to be stored for later generation
            if self.module_registry.contains_key(&resolved) {
                log::debug!(
                    "Found import from wrapper module '{resolved}' in namespace hybrid module \
                     '{module_name}'"
                );

                // Store wrapper imports for later generation
                for alias in &import_from.names {
                    let imported_name = alias.name.as_str();
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map(|n| n.as_str())
                        .unwrap_or(imported_name);

                    log::debug!(
                        "Skipping wrapper import: {local_name} = \
                         sys.modules['{resolved}'].{imported_name} (not using namespace approach)"
                    );
                }
                return;
            }

            // Track aliases for imports from inlined modules
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let local_name = alias
                    .asname
                    .as_ref()
                    .map(|n| n.as_str())
                    .unwrap_or(imported_name);

                // For imports from inlined modules, check if the symbol was renamed
                let actual_name = self.get_actual_import_name(&resolved, imported_name, ctx);

                if local_name != imported_name || self.inlined_modules.contains(&resolved) {
                    ctx.import_aliases
                        .insert(local_name.to_string(), actual_name);
                }
            }
        }
    }

    /// Get the actual name for an imported symbol, handling renames
    fn get_actual_import_name(
        &self,
        resolved_module: &str,
        imported_name: &str,
        ctx: &InlineContext,
    ) -> String {
        if self.inlined_modules.contains(resolved_module) {
            // First check if we already have the rename in our context
            if let Some(source_renames) = ctx.module_renames.get(resolved_module) {
                source_renames
                    .get(imported_name)
                    .cloned()
                    .unwrap_or_else(|| imported_name.to_string())
            } else {
                // The module will be inlined later, we don't know the rename yet
                // Store as "module:symbol" format that we'll resolve later
                format!("{resolved_module}:{imported_name}")
            }
        } else {
            // For non-inlined imports, just track the mapping
            imported_name.to_string()
        }
    }

    /// Check if a symbol should be inlined based on export rules
    fn should_inline_symbol(
        &self,
        symbol_name: &str,
        module_name: &str,
        module_exports_map: &FxIndexMap<String, Option<Vec<String>>>,
    ) -> bool {
        // First check tree-shaking decisions if available
        if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols {
            let symbol_key = (module_name.to_string(), symbol_name.to_string());
            if !kept_symbols.contains(&symbol_key) {
                log::trace!(
                    "Tree shaking: removing unused symbol '{symbol_name}' from module \
                     '{module_name}'"
                );
                return false;
            }
        }

        let exports = module_exports_map.get(module_name).and_then(|e| e.as_ref());

        if let Some(export_list) = exports {
            // Module has exports (either explicit __all__ or extracted symbols)
            // Only inline if the symbol is in the export list
            export_list.contains(&symbol_name.to_string())
        } else {
            // No exports at all, don't inline anything
            false
        }
    }

    /// Create assignment statements for symbols imported from an inlined module
    fn create_assignments_for_inlined_imports(
        &self,
        import_from: StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        let mut assignments = Vec::new();

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name);

            // Check if we're importing a module itself (not a symbol from it)
            // This happens when the imported name refers to a submodule
            let full_module_path = format!("{module_name}.{imported_name}");

            // Check if this is a module import
            // First check if it's a wrapped module
            if self.module_registry.contains_key(&full_module_path) {
                // For pure static approach, we don't use sys.modules
                // Instead, we'll handle this as a deferred import
                log::debug!("Module '{full_module_path}' is a wrapped module, deferring import");
                // Skip this - it will be handled differently
                continue;
            } else if self.inlined_modules.contains(&full_module_path)
                || self.bundled_modules.contains(&full_module_path)
            {
                // Create a namespace object for the inlined module
                log::debug!(
                    "Creating namespace object for module '{imported_name}' imported from \
                     '{module_name}' - module was inlined"
                );

                // Create a SimpleNamespace-like object with __name__ set
                let namespace_stmts =
                    self.create_namespace_with_name(local_name, &full_module_path);
                assignments.extend(namespace_stmts);

                // Now add all symbols from the inlined module to the namespace
                // This should come from semantic analysis of what symbols the module exports
                if let Some(module_renames) = symbol_renames.get(&full_module_path) {
                    // Add each symbol from the module to the namespace
                    for (original_name, renamed_name) in module_renames {
                        // base.original_name = renamed_name
                        assignments.push(Stmt::Assign(StmtAssign {
                            node_index: AtomicNodeIndex::dummy(),
                            targets: vec![Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: local_name.as_str().into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new(original_name, TextRange::default()),
                                ctx: ExprContext::Store,
                                range: TextRange::default(),
                            })],
                            value: Box::new(Expr::Name(ExprName {
                                node_index: AtomicNodeIndex::dummy(),
                                id: renamed_name.clone().into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            range: TextRange::default(),
                        }));
                    }
                }
            } else {
                // Regular symbol import
                // Check if this symbol was renamed during inlining
                let actual_name = if let Some(module_renames) = symbol_renames.get(module_name) {
                    module_renames
                        .get(imported_name)
                        .map(|s| s.as_str())
                        .unwrap_or(imported_name)
                } else {
                    imported_name
                };

                // Only create assignment if the names are different
                if local_name.as_str() != actual_name {
                    log::debug!(
                        "Creating assignment: {local_name} = {actual_name} (from inlined module \
                         '{module_name}')"
                    );

                    let assignment = StmtAssign {
                        node_index: AtomicNodeIndex::dummy(),
                        targets: vec![Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: local_name.as_str().into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: actual_name.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    };

                    assignments.push(Stmt::Assign(assignment));
                }
            }
        }

        assignments
    }

    /// Rewrite ImportFrom statements
    fn rewrite_import_from(
        &self,
        import_from: StmtImportFrom,
        current_module: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        inside_wrapper_init: bool,
    ) -> Vec<Stmt> {
        // Resolve relative imports to absolute module names
        log::debug!(
            "rewrite_import_from: Processing import {:?} in module '{}'",
            import_from.module.as_ref().map(|m| m.as_str()),
            current_module
        );
        let resolved_module_name = self.resolve_relative_import(&import_from, current_module);

        let Some(module_name) = resolved_module_name else {
            // If we can't resolve the module, return the original import
            log::warn!(
                "Could not resolve module name for import {:?}, keeping original import",
                import_from.module.as_ref().map(|m| m.as_str())
            );
            return vec![Stmt::ImportFrom(import_from)];
        };

        if !self.bundled_modules.contains(&module_name) {
            log::debug!(
                "Module '{module_name}' not found in bundled modules, checking if inlined or \
                 importing submodules"
            );

            // Check if this module is inlined
            if self.inlined_modules.contains(&module_name) {
                log::debug!("Module '{module_name}' is an inlined module");
                // Handle imports from inlined modules
                return self.handle_imports_from_inlined_module(
                    &import_from,
                    &module_name,
                    symbol_renames,
                );
            }

            // Check if we're importing submodules from a namespace package
            // e.g., from greetings import greeting where greeting is actually greetings.greeting
            let mut has_bundled_submodules = false;
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let full_module_path = format!("{module_name}.{imported_name}");
                if self.bundled_modules.contains(&full_module_path) {
                    has_bundled_submodules = true;
                    break;
                }
            }

            if !has_bundled_submodules {
                // No bundled submodules, keep original import
                // For relative imports from non-bundled modules, convert to absolute import
                if import_from.level > 0 {
                    let mut absolute_import = import_from.clone();
                    absolute_import.level = 0;
                    absolute_import.module =
                        Some(Identifier::new(&module_name, TextRange::default()));
                    return vec![Stmt::ImportFrom(absolute_import)];
                }
                return vec![Stmt::ImportFrom(import_from)];
            }

            // We have bundled submodules, need to transform them
            log::debug!("Module '{module_name}' has bundled submodules, transforming imports");
            // Transform each submodule import
            return self.transform_namespace_package_imports(
                import_from,
                &module_name,
                symbol_renames,
            );
        }

        log::debug!(
            "Transforming bundled import from module: {module_name}, is wrapper: {}",
            self.module_registry.contains_key(&module_name)
        );

        // Check if this module is in the registry (wrapper approach)
        // or if it was inlined
        if self.module_registry.contains_key(&module_name) {
            // Module uses wrapper approach - transform to sys.modules access
            // For relative imports, we need to create an absolute import
            let mut absolute_import = import_from.clone();
            if import_from.level > 0 {
                // Convert relative import to absolute
                absolute_import.level = 0;
                absolute_import.module = Some(Identifier::new(&module_name, TextRange::default()));
            }
            self.transform_bundled_import_from_multiple_with_context(
                absolute_import,
                &module_name,
                inside_wrapper_init,
            )
        } else {
            // Module was inlined - create assignments for imported symbols
            log::debug!(
                "Module '{module_name}' was inlined, creating assignments for imported symbols"
            );
            self.create_assignments_for_inlined_imports(import_from, &module_name, symbol_renames)
        }
    }

    /// Transform imports from a namespace package (package without __init__.py)
    fn transform_namespace_package_imports(
        &self,
        import_from: StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        let mut result_stmts = Vec::new();

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            let full_module_path = format!("{module_name}.{imported_name}");

            if self.bundled_modules.contains(&full_module_path) {
                if self.module_registry.contains_key(&full_module_path) {
                    // Wrapper module - create sys.modules access
                    result_stmts.push(Stmt::Assign(StmtAssign {
                        node_index: AtomicNodeIndex::dummy(),
                        targets: vec![Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: local_name.into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: "sys".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new("modules", TextRange::default()),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            slice: Box::new(self.create_string_literal(&full_module_path)),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    }));
                } else {
                    // Inlined module - create a namespace object for it
                    log::debug!(
                        "Submodule '{imported_name}' from namespace package '{module_name}' was \
                         inlined, creating namespace"
                    );

                    // For namespace hybrid modules, we need to create the namespace object
                    // The inlined module's symbols are already renamed with module prefix
                    // e.g., message -> message_greetings_greeting
                    let _inlined_key = full_module_path.cow_replace('.', "_").into_owned();

                    // Create a SimpleNamespace object manually with all the inlined symbols
                    // Since the module was inlined, we need to map the original names to the
                    // renamed ones
                    result_stmts.push(Stmt::Assign(StmtAssign {
                        node_index: AtomicNodeIndex::dummy(),
                        targets: vec![Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: local_name.into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Call(ExprCall {
                            node_index: AtomicNodeIndex::dummy(),
                            func: Box::new(Expr::Attribute(ExprAttribute {
                                node_index: AtomicNodeIndex::dummy(),
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: "types".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new("SimpleNamespace", TextRange::default()),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            arguments: Arguments {
                                node_index: AtomicNodeIndex::dummy(),
                                args: Box::new([]),
                                keywords: Box::new([]),
                                range: TextRange::default(),
                            },
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    }));

                    // Add all the renamed symbols as attributes to the namespace
                    // Get the symbol renames for this module if available
                    if let Some(module_renames) = symbol_renames.get(&full_module_path) {
                        let module_suffix = full_module_path.cow_replace('.', "_");
                        for (original_name, renamed_name) in module_renames {
                            // Check if this is an identity mapping (no semantic rename)
                            let actual_renamed_name = if renamed_name == original_name {
                                // No semantic rename, apply module suffix pattern

                                self.get_unique_name_with_module_suffix(
                                    original_name,
                                    &module_suffix,
                                )
                            } else {
                                // Use the semantic rename
                                renamed_name.clone()
                            };

                            // base.original_name = actual_renamed_name
                            result_stmts.push(Stmt::Assign(StmtAssign {
                                node_index: AtomicNodeIndex::dummy(),
                                targets: vec![Expr::Attribute(ExprAttribute {
                                    node_index: AtomicNodeIndex::dummy(),
                                    value: Box::new(Expr::Name(ExprName {
                                        node_index: AtomicNodeIndex::dummy(),
                                        id: local_name.into(),
                                        ctx: ExprContext::Load,
                                        range: TextRange::default(),
                                    })),
                                    attr: Identifier::new(original_name, TextRange::default()),
                                    ctx: ExprContext::Store,
                                    range: TextRange::default(),
                                })],
                                value: Box::new(Expr::Name(ExprName {
                                    node_index: AtomicNodeIndex::dummy(),
                                    id: actual_renamed_name.into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                range: TextRange::default(),
                            }));
                        }
                    } else {
                        // Fallback: try to guess the renamed symbols based on module suffix
                        log::warn!(
                            "No symbol renames found for inlined module '{full_module_path}', \
                             namespace will be empty"
                        );
                    }
                }
            } else {
                // Not a bundled submodule, keep as attribute access
                // This might be importing a symbol from the namespace package's __init__.py
                // But since we're here, the namespace package has no __init__.py
                log::warn!(
                    "Import '{imported_name}' from namespace package '{module_name}' is not a \
                     bundled module"
                );
            }
        }

        if result_stmts.is_empty() {
            // If we didn't transform anything, return the original
            vec![Stmt::ImportFrom(import_from)]
        } else {
            result_stmts
        }
    }

    /// Rewrite Import statements without symbol renames
    fn rewrite_import(&self, import_stmt: StmtImport) -> Vec<Stmt> {
        self.rewrite_import_with_renames(import_stmt, &FxIndexMap::default())
    }

    /// Rewrite Import statements with symbol renames
    fn rewrite_import_with_renames(
        &self,
        import_stmt: StmtImport,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        // Check each import individually
        let mut result_stmts = Vec::new();
        let mut handled_all = true;

        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();

            // Check if this is a dotted import (e.g., greetings.greeting)
            if module_name.contains('.') {
                // Handle dotted imports specially
                let parts: Vec<&str> = module_name.split('.').collect();

                // Check if the full module is bundled
                if self.bundled_modules.contains(module_name) {
                    if self.module_registry.contains_key(module_name) {
                        // Create all parent namespaces if needed (e.g., for a.b.c.d, create a, a.b,
                        // a.b.c)
                        self.create_parent_namespaces(&parts, &mut result_stmts);

                        let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                        // If there's no alias, we need to handle the dotted name specially
                        if alias.asname.is_none() && module_name.contains('.') {
                            // Create assignments for each level of nesting
                            self.create_dotted_assignments(&parts, &mut result_stmts);
                        } else {
                            // For aliased imports or non-dotted imports, just assign to the target
                            // Skip self-assignments - the module is already initialized
                            if target_name.as_str() != module_name {
                                result_stmts.push(self.create_module_reference_assignment(
                                    target_name.as_str(),
                                    module_name,
                                ));
                            }
                        }
                    } else {
                        // Module was inlined - create a namespace object
                        let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                        // For dotted imports, we need to create the parent namespaces
                        if alias.asname.is_none() && module_name.contains('.') {
                            // For non-aliased dotted imports like "import a.b.c"
                            // Create all parent namespace objects AND the leaf namespace
                            self.create_all_namespace_objects(&parts, &mut result_stmts);

                            // Populate ALL namespace levels with their symbols, not just the leaf
                            // For "import greetings.greeting", populate both "greetings" and
                            // "greetings.greeting"
                            for i in 1..=parts.len() {
                                let partial_module = parts[..i].join(".");
                                // Only populate if this module was actually bundled and has exports
                                if self.bundled_modules.contains(&partial_module) {
                                    self.populate_namespace_with_module_symbols_with_renames(
                                        &partial_module,
                                        &partial_module,
                                        &mut result_stmts,
                                        symbol_renames,
                                    );
                                }
                            }
                        } else {
                            // For simple imports or aliased imports, create namespace object with
                            // the module's exports
                            let namespace_stmt = self.create_namespace_object_for_module(
                                target_name.as_str(),
                                module_name,
                            );
                            result_stmts.push(namespace_stmt);

                            // Also populate the namespace with symbols
                            self.populate_namespace_with_module_symbols_with_renames(
                                target_name.as_str(),
                                module_name,
                                &mut result_stmts,
                                symbol_renames,
                            );
                        }
                    }
                } else {
                    handled_all = false;
                    continue;
                }
            } else {
                // Non-dotted import - handle as before
                if !self.bundled_modules.contains(module_name) {
                    handled_all = false;
                    continue;
                }

                if self.module_registry.contains_key(module_name) {
                    // Module uses wrapper approach - transform to sys.modules access
                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);
                    // Skip self-assignments - the module is already initialized
                    if target_name.as_str() != module_name {
                        result_stmts.push(
                            self.create_module_reference_assignment(
                                target_name.as_str(),
                                module_name,
                            ),
                        );
                    }
                } else {
                    // Module was inlined - create a namespace object
                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                    // Create namespace object with the module's exports
                    let namespace_stmt =
                        self.create_namespace_object_for_module(target_name.as_str(), module_name);
                    result_stmts.push(namespace_stmt);

                    // Also populate the namespace with symbols
                    self.populate_namespace_with_module_symbols_with_renames(
                        target_name.as_str(),
                        module_name,
                        &mut result_stmts,
                        symbol_renames,
                    );
                }
            }
        }

        if handled_all {
            result_stmts
        } else {
            // Keep original import for non-bundled modules
            vec![Stmt::Import(import_stmt)]
        }
    }

    /// Rewrite Import statements in entry module with namespace tracking
    fn rewrite_import_entry_module(&mut self, import_stmt: StmtImport) -> Vec<Stmt> {
        // We need to handle namespace tracking differently to avoid borrow issues
        let mut result_stmts = Vec::new();
        let mut handled_all = true;

        for alias in &import_stmt.names {
            let module_name = alias.name.as_str();

            // Check if this is a dotted import (e.g., greetings.greeting)
            if module_name.contains('.') {
                // Handle dotted imports specially
                let parts: Vec<&str> = module_name.split('.').collect();

                // Check if the full module is bundled
                if self.bundled_modules.contains(module_name)
                    && self.module_registry.contains_key(module_name)
                {
                    // Create all parent namespaces if needed (e.g., for a.b.c.d, create a, a.b,
                    // a.b.c)
                    self.create_parent_namespaces_entry_module(&parts, &mut result_stmts);

                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                    // If there's no alias, we need to handle the dotted name specially
                    if alias.asname.is_none() && module_name.contains('.') {
                        // For dotted imports without alias, we need to ensure the parent has the
                        // child as an attribute e.g., for "import
                        // greetings.greeting", we need "greetings.greeting =
                        // sys.modules['greetings.greeting']"
                        self.handle_dotted_import_attribute(&parts, module_name, &mut result_stmts);
                    } else {
                        // For aliased imports or non-dotted imports, just assign to the target
                        result_stmts.push(
                            self.create_module_reference_assignment(
                                target_name.as_str(),
                                module_name,
                            ),
                        );
                    }
                } else {
                    handled_all = false;
                    continue;
                }
            } else {
                // Non-dotted import - handle as before
                if !self.bundled_modules.contains(module_name) {
                    handled_all = false;
                    continue;
                }

                if self.module_registry.contains_key(module_name) {
                    // Module uses wrapper approach - transform to sys.modules access
                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);
                    // Skip self-assignments - the module is already initialized
                    if target_name.as_str() != module_name {
                        result_stmts.push(
                            self.create_module_reference_assignment(
                                target_name.as_str(),
                                module_name,
                            ),
                        );
                    }
                } else {
                    // Module was inlined - this is problematic for direct imports
                    // We need to create a mock module object
                    log::debug!(
                        "Direct import of inlined module '{module_name}' detected - will create \
                         namespace object"
                    );

                    // Create a namespace object for the inlined module
                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);
                    let namespace_stmt = self.create_namespace_object_for_direct_import(
                        module_name,
                        target_name.as_str(),
                    );
                    result_stmts.push(namespace_stmt);
                }
            }
        }

        if handled_all {
            result_stmts
        } else {
            // Keep original import for non-bundled modules
            vec![Stmt::Import(import_stmt)]
        }
    }

    /// Check if importing from a package __init__.py that might re-export symbols
    fn is_package_init_reexport(&self, module_name: &str, _symbol_name: &str) -> bool {
        // Special handling for package __init__.py files
        // If we're importing from "greetings" and there's a "greetings.X" module
        // that could be the source of the symbol

        // For now, check if this looks like a package (no dots) and if there are
        // any inlined submodules
        if !module_name.contains('.') {
            // Check if any inlined module starts with module_name.
            for inlined in &self.inlined_modules {
                if inlined.starts_with(&format!("{module_name}.")) {
                    log::debug!(
                        "Module '{module_name}' appears to be a package with inlined submodule \
                         '{inlined}'"
                    );
                    // For the specific case of greetings/__init__.py importing from
                    // greetings.english, we assume the symbol should use its
                    // original name
                    return true;
                }
            }
        }
        false
    }

    /// Handle imports from inlined modules
    fn handle_imports_from_inlined_module(
        &self,
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) -> Vec<Stmt> {
        self.handle_imports_from_inlined_module_with_context(
            import_from,
            module_name,
            symbol_renames,
            None,
        )
    }

    /// Handle imports from inlined modules with optional module context
    fn handle_imports_from_inlined_module_with_context(
        &self,
        import_from: &StmtImportFrom,
        module_name: &str,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
        _modules: Option<&[(String, ModModule, PathBuf, String)]>,
    ) -> Vec<Stmt> {
        let mut result_stmts = Vec::new();

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Check if this is likely a re-export from a package __init__.py
            let is_package_reexport = self.is_package_init_reexport(module_name, imported_name);

            let renamed_symbol = if is_package_reexport {
                // For package re-exports, use the original symbol name
                // This handles cases like greetings/__init__.py re-exporting from greetings.english
                log::debug!(
                    "Using original name '{imported_name}' for symbol imported from package \
                     '{module_name}'"
                );
                imported_name.to_string()
            } else {
                // Not a re-export, check normal renames
                if let Some(module_renames) = symbol_renames.get(module_name) {
                    module_renames
                        .get(imported_name)
                        .cloned()
                        .unwrap_or_else(|| {
                            // If no rename found, use the default pattern
                            let module_suffix = module_name.cow_replace('.', "_").into_owned();
                            format!("{imported_name}_{module_suffix}")
                        })
                } else {
                    // If no rename map, use the default pattern
                    let module_suffix = module_name.cow_replace('.', "_").into_owned();
                    format!("{imported_name}_{module_suffix}")
                }
            };

            // Only create assignment if the names are different
            if local_name != renamed_symbol {
                result_stmts.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: local_name.into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: renamed_symbol.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));
            }
        }

        result_stmts
    }

    /// Handle dotted import attribute assignment
    fn handle_dotted_import_attribute(
        &self,
        parts: &[&str],
        module_name: &str,
        result_stmts: &mut Vec<Stmt>,
    ) {
        if parts.len() > 1 {
            let parent = parts[..parts.len() - 1].join(".");
            let attr = parts[parts.len() - 1];

            // Only add the attribute assignment if the parent is a namespace (not a real module)
            if !parent.is_empty() && !self.module_registry.contains_key(&parent) {
                result_stmts.push(self.create_dotted_attribute_assignment(
                    &parent,
                    attr,
                    module_name,
                ));
            }
        }
    }

    /// Create parent namespaces for dotted imports in entry module
    fn create_parent_namespaces_entry_module(
        &mut self,
        parts: &[&str],
        result_stmts: &mut Vec<Stmt>,
    ) {
        for i in 1..parts.len() {
            let parent_path = parts[..i].join(".");

            if self.module_registry.contains_key(&parent_path) {
                // Parent is a wrapper module - don't create assignment here
                // generate_submodule_attributes will handle all necessary parent assignments
            } else if !self.bundled_modules.contains(&parent_path) {
                // Check if we haven't already created this namespace globally
                if !self.created_namespaces.contains(&parent_path) {
                    // Parent is not a wrapper module and not an inlined module, create a simple
                    // namespace
                    result_stmts.extend(self.create_namespace_module(&parent_path));
                    // Track that we created this namespace
                    self.created_namespaces.insert(parent_path);
                }
            }
        }
    }

    /// Create a module reference assignment
    fn create_module_reference_assignment(&self, target_name: &str, module_name: &str) -> Stmt {
        // Simply assign the module reference: target_name = module_name
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: target_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: module_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create parent namespaces for dotted imports
    fn create_parent_namespaces(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
        for i in 1..parts.len() {
            let parent_path = parts[..i].join(".");

            if self.module_registry.contains_key(&parent_path) {
                // Parent is a wrapper module, create reference to it
                result_stmts
                    .push(self.create_module_reference_assignment(&parent_path, &parent_path));
            } else if !self.bundled_modules.contains(&parent_path) {
                // Check if we haven't already created this namespace in result_stmts
                let already_created = self.is_namespace_already_created(&parent_path, result_stmts);

                if !already_created {
                    // Parent is not a wrapper module and not an inlined module, create a simple
                    // namespace
                    result_stmts.extend(self.create_namespace_module(&parent_path));
                }
            }
        }
    }

    /// Check if a namespace module was already created
    fn is_namespace_already_created(&self, parent_path: &str, result_stmts: &[Stmt]) -> bool {
        result_stmts.iter().any(|stmt| {
            if let Stmt::Assign(assign) = stmt
                && let Some(Expr::Name(name)) = assign.targets.first()
            {
                return name.id.as_str() == parent_path;
            }
            false
        })
    }

    /// Create dotted attribute assignments for imports
    fn create_dotted_assignments(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
        // For import a.b.c.d, we need:
        // a.b = sys.modules['a.b']
        // a.b.c = sys.modules['a.b.c']
        // a.b.c.d = sys.modules['a.b.c.d']
        for i in 2..=parts.len() {
            let parent = parts[..i - 1].join(".");
            let attr = parts[i - 1];
            let full_path = parts[..i].join(".");

            // Check if this would be a redundant self-assignment
            let full_target = format!("{parent}.{attr}");
            if full_target == full_path {
                debug!(
                    "Skipping redundant self-assignment in create_dotted_assignments: \
                     {parent}.{attr} = {full_path}"
                );
            } else {
                result_stmts
                    .push(self.create_dotted_attribute_assignment(&parent, attr, &full_path));
            }
        }
    }

    /// Create namespace keywords for a module
    fn create_namespace_keywords(&self, full_module_path: &str, inlined_key: &str) -> Vec<Keyword> {
        let mut keywords = Vec::new();
        if let Some(Some(exports)) = self.module_exports.get(full_module_path) {
            for symbol in exports {
                keywords.push(Keyword {
                    node_index: AtomicNodeIndex::dummy(),
                    arg: Some(Identifier::new(symbol.as_str(), TextRange::default())),
                    value: Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: format!("{symbol}_{inlined_key}").into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    }),
                    range: TextRange::default(),
                });
            }
        }
        keywords
    }

    /// Create a simple namespace module object
    /// Generate statements to merge attributes from a wrapper module into its namespace
    /// This is used when a module is both a wrapper module and a parent namespace
    fn generate_merge_module_attributes(
        &self,
        statements: &mut Vec<Stmt>,
        namespace_name: &str,
        temp_module_name: &str,
    ) {
        // Generate code like:
        // for attr in dir(_cribo_temp_mypackage):
        //     if not attr.startswith('_'):
        //         setattr(mypackage, attr, getattr(_cribo_temp_mypackage, attr))

        let attr_var = "attr";
        let loop_target = Expr::Name(ExprName {
            node_index: AtomicNodeIndex::dummy(),
            id: attr_var.into(),
            ctx: ExprContext::Store,
            range: TextRange::default(),
        });

        // dir(_cribo_temp_mypackage)
        let dir_call = Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::dummy(),
            func: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: "dir".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                node_index: AtomicNodeIndex::dummy(),
                args: Box::from([Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: temp_module_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })]),
                keywords: Box::from([]),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        // not attr.startswith('_')
        let condition = Expr::UnaryOp(ruff_python_ast::ExprUnaryOp {
            node_index: AtomicNodeIndex::dummy(),
            op: ruff_python_ast::UnaryOp::Not,
            operand: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: attr_var.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("startswith", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([Expr::StringLiteral(ExprStringLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        value: StringLiteralValue::single(StringLiteral {
                            node_index: AtomicNodeIndex::dummy(),
                            value: "_".into(),
                            range: TextRange::default(),
                            flags: StringLiteralFlags::empty(),
                        }),
                        range: TextRange::default(),
                    })]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        });

        // getattr(_cribo_temp_mypackage, attr)
        let getattr_call = Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::dummy(),
            func: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: "getattr".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                node_index: AtomicNodeIndex::dummy(),
                args: Box::from([
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: temp_module_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    }),
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: attr_var.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    }),
                ]),
                keywords: Box::from([]),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        // setattr(mypackage, attr, getattr(...))
        let setattr_call = Stmt::Expr(ruff_python_ast::StmtExpr {
            node_index: AtomicNodeIndex::dummy(),
            value: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "setattr".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([
                        Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: namespace_name.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        }),
                        Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: attr_var.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        }),
                        getattr_call,
                    ]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        });

        // if not attr.startswith('_'): setattr(...)
        let if_stmt = Stmt::If(ruff_python_ast::StmtIf {
            node_index: AtomicNodeIndex::dummy(),
            test: Box::new(condition),
            body: vec![setattr_call],
            elif_else_clauses: vec![],
            range: TextRange::default(),
        });

        // for attr in dir(...): if ...
        let for_loop = Stmt::For(ruff_python_ast::StmtFor {
            node_index: AtomicNodeIndex::dummy(),
            target: Box::new(loop_target),
            iter: Box::new(dir_call),
            body: vec![if_stmt],
            orelse: vec![],
            is_async: false,
            range: TextRange::default(),
        });

        statements.push(for_loop);
    }

    fn create_namespace_module(&self, module_name: &str) -> Vec<Stmt> {
        // Create: module_name = types.SimpleNamespace()
        // Note: This should only be called with simple (non-dotted) module names
        debug_assert!(
            !module_name.contains('.'),
            "create_namespace_module called with dotted name: {module_name}"
        );

        // This method is called by create_namespace_statements which already
        // filters based on required_namespaces, so we don't need to check again

        // Create the namespace
        let mut statements = vec![Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: module_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })];

        // Set the __name__ attribute to match real module behavior
        statements.push(Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: module_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("__name__", TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                node_index: AtomicNodeIndex::dummy(),
                value: StringLiteralValue::single(StringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: module_name.to_string().into(),
                    range: TextRange::default(),
                    flags: StringLiteralFlags::empty(),
                }),
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        }));

        statements
    }

    /// Create a namespace with a specific variable name and module path for __name__
    fn create_namespace_with_name(&self, var_name: &str, module_path: &str) -> Vec<Stmt> {
        // Create: var_name = types.SimpleNamespace()
        let mut statements = vec![Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: var_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })];

        // Set the __name__ attribute
        statements.push(Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: var_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("__name__", TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                node_index: AtomicNodeIndex::dummy(),
                value: StringLiteralValue::single(StringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: module_path.to_string().into(),
                    range: TextRange::default(),
                    flags: StringLiteralFlags::empty(),
                }),
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        }));

        statements
    }

    /// Create dotted attribute assignment (e.g., greetings.greeting = greeting)
    fn create_dotted_attribute_assignment(
        &self,
        parent_module: &str,
        attr_name: &str,
        full_module_name: &str,
    ) -> Stmt {
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: parent_module.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(attr_name, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: full_module_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create a module assignment, handling dotted names properly
    fn create_module_assignment(&self, final_body: &mut Vec<Stmt>, module: &str) {
        if module.contains('.') {
            // For dotted module names, we need to create proper attribute assignments
            // e.g., for "a.b.c", create: a.b.c = sys.modules['a.b.c']
            // But this needs to be done as: parent.attr = sys.modules['a.b.c']
            let parts: Vec<&str> = module.split('.').collect();
            if parts.len() > 1 {
                let parent = parts[..parts.len() - 1].join(".");
                let attr = parts[parts.len() - 1];

                // Check if this would be a redundant self-assignment
                let full_target = format!("{parent}.{attr}");
                if full_target == module {
                    debug!(
                        "Skipping redundant self-assignment in create_module_assignment: \
                         {parent}.{attr} = {module}"
                    );
                } else {
                    final_body.push(self.create_dotted_attribute_assignment(&parent, attr, module));
                }
            }
        } else {
            // Simple module name without dots
            final_body.push(self.create_module_reference_assignment(module, module));
        }
    }

    /// Check if an assignment statement has a dotted target matching parent.attr
    fn assignment_has_dotted_target(&self, assign: &StmtAssign, parent: &str, attr: &str) -> bool {
        assign.targets.iter().any(|target| {
            if let Expr::Attribute(attr_expr) = target {
                attr_expr.attr.as_str() == attr && self.is_name_chain(&attr_expr.value, parent)
            } else {
                false
            }
        })
    }

    /// Check if a module has already been assigned in the final body
    fn is_module_already_assigned(&self, final_body: &[Stmt], module: &str) -> bool {
        if module.contains('.') {
            // For dotted names, check if there's already an attribute assignment
            let parts: Vec<&str> = module.split('.').collect();
            if parts.len() > 1 {
                let parent = parts[..parts.len() - 1].join(".");
                let attr = parts[parts.len() - 1];
                final_body.iter().any(|stmt| {
                    matches!(stmt, Stmt::Assign(assign) if
                        self.assignment_has_dotted_target(assign, &parent, attr)
                    )
                })
            } else {
                false
            }
        } else {
            // For simple names, check for name assignment
            final_body.iter().any(|stmt| {
                matches!(stmt, Stmt::Assign(assign) if
                    assign.targets.iter().any(|target|
                        matches!(target, Expr::Name(name) if name.id.as_str() == module)
                    )
                )
            })
        }
    }

    /// Check if an expression represents a dotted name chain (e.g., "a.b.c")
    fn is_name_chain(&self, expr: &Expr, expected_chain: &str) -> bool {
        let parts: Vec<&str> = expected_chain.split('.').collect();
        if parts.is_empty() {
            return false;
        }

        let mut current_expr = expr;
        let mut index = parts.len() - 1;

        loop {
            match current_expr {
                Expr::Attribute(attr) => {
                    // Check if this part matches
                    if index < parts.len() && attr.attr.as_str() != parts[index] {
                        return false;
                    }
                    if index == 0 {
                        return false; // Still have attribute but no more parts
                    }
                    index -= 1;
                    current_expr = &attr.value;
                }
                Expr::Name(name) => {
                    // This should be the base name
                    return index == 0 && name.id.as_str() == parts[0];
                }
                _ => return false,
            }
        }
    }

    /// Create all namespace objects including the leaf for a dotted import
    fn create_all_namespace_objects(&self, parts: &[&str], result_stmts: &mut Vec<Stmt>) {
        // For "import a.b.c", we need to create namespace objects for "a", "a.b", and "a.b.c"
        for i in 1..=parts.len() {
            let partial_module = parts[..i].join(".");

            // Skip if this module is already a wrapper module
            if self.module_registry.contains_key(&partial_module) {
                continue;
            }

            // Create empty namespace object
            let namespace_expr = Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: vec![].into(),
                    keywords: vec![].into(),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            });

            // Assign to the first part of the name
            if i == 1 {
                result_stmts.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(namespace_expr),
                    range: TextRange::default(),
                }));
            } else {
                // For deeper levels, create attribute assignments
                let mut target = Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: parts[0].into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });

                // Build up the chain up to but not including the last part
                for part in &parts[1..(i - 1)] {
                    target = Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(target),
                        attr: Identifier::new(&**part, TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });
                }

                result_stmts.push(Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(target),
                        attr: Identifier::new(parts[i - 1], TextRange::default()),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(namespace_expr),
                    range: TextRange::default(),
                }));
            }
        }
    }

    /// Populate a namespace with symbols from an inlined module
    fn populate_namespace_with_module_symbols(
        &self,
        module_name: &str,
        result_stmts: &mut Vec<Stmt>,
    ) {
        self.populate_namespace_with_module_symbols_using_target(
            module_name,
            module_name,
            result_stmts,
        );
    }

    /// Populate a namespace with symbols from an inlined module using a specific target name (no
    /// renames)
    fn populate_namespace_with_module_symbols_using_target(
        &self,
        target_name: &str,
        module_name: &str,
        result_stmts: &mut Vec<Stmt>,
    ) {
        self.populate_namespace_with_module_symbols_with_renames(
            target_name,
            module_name,
            result_stmts,
            &FxIndexMap::default(),
        );
    }

    /// Populate a namespace with symbols from an inlined module using a specific target name
    fn populate_namespace_with_module_symbols_with_renames(
        &self,
        target_name: &str,
        module_name: &str,
        result_stmts: &mut Vec<Stmt>,
        symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    ) {
        // Get the module's exports
        if let Some(exports) = self
            .module_exports
            .get(module_name)
            .and_then(|e| e.as_ref())
        {
            // Build the namespace access expression for the target
            let parts: Vec<&str> = target_name.split('.').collect();

            // First, add __all__ attribute to the namespace
            // Create the target expression for __all__
            let mut all_target = Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: parts[0].into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            });

            for part in &parts[1..] {
                all_target = Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(all_target),
                    attr: Identifier::new(&**part, TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                });
            }

            // Filter exports to only include symbols that survived tree-shaking
            let filtered_exports = self.filter_exports_by_tree_shaking_with_logging(
                exports,
                module_name,
                self.tree_shaking_keep_symbols.as_ref(),
            );

            // Create __all__ = [...] assignment with filtered exports
            let all_list = Expr::List(ExprList {
                node_index: AtomicNodeIndex::dummy(),
                elts: filtered_exports
                    .iter()
                    .map(|name| {
                        Expr::StringLiteral(ExprStringLiteral {
                            node_index: AtomicNodeIndex::dummy(),
                            value: StringLiteralValue::single(StringLiteral {
                                node_index: AtomicNodeIndex::dummy(),
                                value: name.as_str().into(),
                                flags: StringLiteralFlags::empty(),
                                range: TextRange::default(),
                            }),
                            range: TextRange::default(),
                        })
                    })
                    .collect(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            });

            result_stmts.push(Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(all_target),
                    attr: Identifier::new("__all__", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(all_list),
                range: TextRange::default(),
            }));

            // For each exported symbol that survived tree-shaking, add it to the namespace
            for symbol in &filtered_exports {
                // For re-exported symbols, check if the original symbol is kept by tree-shaking
                let should_include = if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                {
                    // First check if this symbol is directly defined in this module
                    if kept_symbols.contains(&(module_name.to_string(), (*symbol).clone())) {
                        true
                    } else {
                        // If not, check if this is a re-exported symbol from another module
                        // For modules with __all__, we always include symbols that are re-exported
                        // even if they're not directly defined in the module
                        let module_has_all_export = self
                            .module_exports
                            .get(module_name)
                            .and_then(|exports| exports.as_ref())
                            .map(|exports| exports.contains(symbol))
                            .unwrap_or(false);

                        if module_has_all_export {
                            log::debug!(
                                "Including re-exported symbol {symbol} from module {module_name} \
                                 (in __all__)"
                            );
                            true
                        } else {
                            false
                        }
                    }
                } else {
                    // No tree-shaking, include everything
                    true
                };

                if !should_include {
                    log::debug!(
                        "Skipping namespace assignment for {module_name}.{symbol} - removed by \
                         tree-shaking"
                    );
                    continue;
                }

                // Get the renamed symbol if it exists
                let actual_symbol_name =
                    if let Some(module_renames) = symbol_renames.get(module_name) {
                        module_renames
                            .get(*symbol)
                            .cloned()
                            .unwrap_or_else(|| (*symbol).clone())
                    } else {
                        (*symbol).clone()
                    };

                // Create the target expression
                // For simple modules, this will be the module name directly
                // For dotted modules (e.g., greetings.greeting), build the chain
                let target = if parts.len() == 1 {
                    // Simple module - use the name directly
                    Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })
                } else {
                    // Dotted module - build the attribute chain
                    let mut base = Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: parts[0].into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    });

                    for part in &parts[1..] {
                        base = Expr::Attribute(ExprAttribute {
                            node_index: AtomicNodeIndex::dummy(),
                            value: Box::new(base),
                            attr: Identifier::new(&**part, TextRange::default()),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        });
                    }
                    base
                };

                // Now add the symbol as an attribute (e.g., greetings.greeting.get_greeting =
                // get_greeting)
                let attr_assignment = Stmt::Assign(StmtAssign {
                    node_index: AtomicNodeIndex::dummy(),
                    targets: vec![Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(target),
                        attr: Identifier::new(*symbol, TextRange::default()),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: actual_symbol_name.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                });

                result_stmts.push(attr_assignment);
            }
        }
    }

    /// Create a namespace object for an inlined module
    fn create_namespace_object_for_module(&self, target_name: &str, _module_name: &str) -> Stmt {
        // For inlined modules, we need to return a vector of statements:
        // 1. Create the namespace object
        // 2. Add all the module's symbols to it

        // We'll create a compound statement that does both
        let _stmts: Vec<Stmt> = Vec::new();

        // First, create the empty namespace
        let namespace_expr = Expr::Call(ExprCall {
            node_index: AtomicNodeIndex::dummy(),
            func: Box::new(Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: "types".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("SimpleNamespace", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: Arguments {
                node_index: AtomicNodeIndex::dummy(),
                args: vec![].into(),
                keywords: vec![].into(),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        // Create assignment for the namespace

        // For now, return just the namespace creation
        // The actual symbol population needs to happen after all symbols are available
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: target_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(namespace_expr),
            range: TextRange::default(),
        })
    }

    /// Inline a class definition
    #[allow(clippy::too_many_arguments)]
    fn inline_class(
        &self,
        class_def: &ruff_python_ast::StmtClassDef,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let class_name = class_def.name.to_string();
        if !self.should_inline_symbol(&class_name, module_name, ctx.module_exports_map) {
            return;
        }

        // Check if this symbol was renamed by semantic analysis
        let renamed_name = if let Some(module_rename_map) = ctx.module_renames.get(module_name) {
            if let Some(new_name) = module_rename_map.get(&class_name) {
                // Only use semantic rename if it's actually different
                if new_name != &class_name {
                    log::debug!(
                        "Using semantic rename for class '{class_name}' to '{new_name}' in module \
                         '{module_name}'"
                    );
                    new_name.clone()
                } else {
                    // Semantic rename is same as original, check if there's a conflict
                    if ctx.global_symbols.contains(&class_name) {
                        // There's a conflict, apply module suffix pattern
                        let base_name =
                            self.get_unique_name_with_module_suffix(&class_name, module_name);
                        self.get_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        // No conflict, use original name
                        class_name.clone()
                    }
                }
            } else {
                // No semantic rename, check if there's a conflict
                if ctx.global_symbols.contains(&class_name) {
                    // There's a conflict, apply module suffix pattern
                    let base_name =
                        self.get_unique_name_with_module_suffix(&class_name, module_name);
                    self.get_unique_name(&base_name, ctx.global_symbols)
                } else {
                    // No conflict, use original name
                    class_name.clone()
                }
            }
        } else {
            // No semantic rename, check if there's a conflict
            if ctx.global_symbols.contains(&class_name) {
                // There's a conflict, apply module suffix pattern
                let base_name = self.get_unique_name_with_module_suffix(&class_name, module_name);
                self.get_unique_name(&base_name, ctx.global_symbols)
            } else {
                // No conflict, use original name
                class_name.clone()
            }
        };

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(class_name.clone(), renamed_name.clone());
        ctx.global_symbols.insert(renamed_name.clone());

        // Clone and rename the class
        let mut class_def_clone = class_def.clone();
        class_def_clone.name = Identifier::new(renamed_name.clone(), TextRange::default());

        // Apply renames to base classes
        // Apply renames and resolve import aliases in class body
        for body_stmt in &mut class_def_clone.body {
            self.resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);

            // Build a combined rename map that includes renames from all modules
            // This is needed because global variables from other modules might be renamed
            let mut combined_renames = module_renames.clone();

            // Add renames from all modules to handle cross-module global variable renames
            for (_other_module, other_renames) in ctx.module_renames.iter() {
                for (original_name, renamed_name) in other_renames {
                    // Only add if not already present (local module renames take precedence)
                    if !combined_renames.contains_key(original_name) {
                        combined_renames.insert(original_name.clone(), renamed_name.clone());
                    }
                }
            }

            self.rewrite_aliases_in_stmt(body_stmt, &combined_renames);
        }

        ctx.inlined_stmts.push(Stmt::ClassDef(class_def_clone));

        // Set the __module__ attribute to preserve the original module name
        let module_attr_stmt = Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Attribute(ExprAttribute {
                node_index: AtomicNodeIndex::dummy(),
                value: Box::new(Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: renamed_name.clone().into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("__module__", TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                node_index: AtomicNodeIndex::dummy(),
                value: StringLiteralValue::single(StringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: module_name.to_string().into(),
                    range: TextRange::default(),
                    flags: StringLiteralFlags::empty(),
                }),
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        });
        ctx.inlined_stmts.push(module_attr_stmt);

        // If the class was renamed, also set __name__ to preserve the original class name
        if renamed_name != class_name {
            let name_attr_stmt = Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: renamed_name.clone().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("__name__", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::StringLiteral(ExprStringLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    value: StringLiteralValue::single(StringLiteral {
                        node_index: AtomicNodeIndex::dummy(),
                        value: class_name.to_string().into(),
                        range: TextRange::default(),
                        flags: StringLiteralFlags::empty(),
                    }),
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            });
            ctx.inlined_stmts.push(name_attr_stmt);
        }
    }

    /// Inline an assignment statement
    #[allow(clippy::too_many_arguments)]
    fn inline_assignment(
        &self,
        assign: &StmtAssign,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let Some(name) = self.extract_simple_assign_target(assign) else {
            return;
        };
        if !self.should_inline_symbol(&name, module_name, ctx.module_exports_map) {
            return;
        }

        // Clone the assignment first
        let mut assign_clone = assign.clone();

        // Check if this is a self-referential assignment
        let is_self_referential = self.is_self_referential_assignment(assign);

        // Skip self-referential assignments entirely - they're meaningless
        if is_self_referential {
            log::debug!("Skipping self-referential assignment '{name}' in module '{module_name}'");
            // Still need to track the rename for the symbol so namespace creation works
            // But we should check if there's already a rename for this symbol
            // (e.g., from a function or class definition)
            if !module_renames.contains_key(&name) {
                // Only create a rename if we haven't seen this symbol yet
                let renamed_name = if let Some(module_rename_map) =
                    ctx.module_renames.get(module_name)
                {
                    if let Some(new_name) = module_rename_map.get(&name) {
                        new_name.clone()
                    } else if ctx.global_symbols.contains(&name) {
                        let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                        self.get_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        name.clone()
                    }
                } else if ctx.global_symbols.contains(&name) {
                    let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                    self.get_unique_name(&base_name, ctx.global_symbols)
                } else {
                    name.clone()
                };
                module_renames.insert(name.clone(), renamed_name.clone());
                ctx.global_symbols.insert(renamed_name);
            }
            return;
        }

        // Apply existing renames to the RHS value BEFORE creating new rename for LHS
        self.resolve_import_aliases_in_expr(&mut assign_clone.value, &ctx.import_aliases);
        self.rewrite_aliases_in_expr(&mut assign_clone.value, module_renames);

        // Now create a new rename for the LHS
        // Check if this symbol was renamed by semantic analysis
        let renamed_name = if let Some(module_rename_map) = ctx.module_renames.get(module_name) {
            if let Some(new_name) = module_rename_map.get(&name) {
                // Only use semantic rename if it's actually different
                if new_name != &name {
                    log::debug!(
                        "Using semantic rename for variable '{name}' to '{new_name}' in module \
                         '{module_name}'"
                    );
                    new_name.clone()
                } else {
                    // Semantic rename is same as original, check if there's a conflict
                    if ctx.global_symbols.contains(&name) {
                        // There's a conflict, apply module suffix pattern
                        let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                        self.get_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        // No conflict, use original name
                        name.clone()
                    }
                }
            } else {
                // No semantic rename, check if there's a conflict
                if ctx.global_symbols.contains(&name) {
                    // There's a conflict, apply module suffix pattern
                    let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                    self.get_unique_name(&base_name, ctx.global_symbols)
                } else {
                    // No conflict, use original name
                    name.clone()
                }
            }
        } else {
            // No semantic rename, check if there's a conflict
            if ctx.global_symbols.contains(&name) {
                // There's a conflict, apply module suffix pattern
                let base_name = self.get_unique_name_with_module_suffix(&name, module_name);
                self.get_unique_name(&base_name, ctx.global_symbols)
            } else {
                // No conflict, use original name
                name.clone()
            }
        };

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(name.clone(), renamed_name.clone());
        ctx.global_symbols.insert(renamed_name.clone());

        // Apply the rename to the LHS
        if let Expr::Name(name_expr) = &mut assign_clone.targets[0] {
            name_expr.id = renamed_name.clone().into();
        }

        ctx.inlined_stmts.push(Stmt::Assign(assign_clone));
    }

    /// Inline an annotated assignment statement
    #[allow(clippy::too_many_arguments)]
    fn inline_ann_assignment(
        &self,
        ann_assign: &ruff_python_ast::StmtAnnAssign,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let Expr::Name(name) = ann_assign.target.as_ref() else {
            return;
        };

        let var_name = name.id.to_string();
        if !self.should_inline_symbol(&var_name, module_name, ctx.module_exports_map) {
            return;
        }

        // Check if this symbol was renamed by semantic analysis
        let renamed_name = if let Some(module_rename_map) = ctx.module_renames.get(module_name) {
            if let Some(new_name) = module_rename_map.get(&var_name) {
                // Only use semantic rename if it's actually different
                if new_name != &var_name {
                    log::debug!(
                        "Using semantic rename for annotated variable '{var_name}' to \
                         '{new_name}' in module '{module_name}'"
                    );
                    new_name.clone()
                } else {
                    // Semantic rename is same as original, check if there's a conflict
                    if ctx.global_symbols.contains(&var_name) {
                        // There's a conflict, apply module suffix pattern
                        let base_name =
                            self.get_unique_name_with_module_suffix(&var_name, module_name);
                        self.get_unique_name(&base_name, ctx.global_symbols)
                    } else {
                        // No conflict, use original name
                        var_name.clone()
                    }
                }
            } else {
                // No semantic rename, check if there's a conflict
                if ctx.global_symbols.contains(&var_name) {
                    // There's a conflict, apply module suffix pattern
                    let base_name = self.get_unique_name_with_module_suffix(&var_name, module_name);
                    self.get_unique_name(&base_name, ctx.global_symbols)
                } else {
                    // No conflict, use original name
                    var_name.clone()
                }
            }
        } else {
            // No semantic rename, check if there's a conflict
            if ctx.global_symbols.contains(&var_name) {
                // There's a conflict, apply module suffix pattern
                let base_name = self.get_unique_name_with_module_suffix(&var_name, module_name);
                self.get_unique_name(&base_name, ctx.global_symbols)
            } else {
                // No conflict, use original name
                var_name.clone()
            }
        };

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(var_name.clone(), renamed_name.clone());
        if renamed_name != var_name {
            log::debug!(
                "Renaming annotated variable '{var_name}' to '{renamed_name}' in module \
                 '{module_name}'"
            );
        }
        ctx.global_symbols.insert(renamed_name.clone());

        // Clone and rename the annotated assignment
        let mut ann_assign_clone = ann_assign.clone();
        if let Expr::Name(name_expr) = ann_assign_clone.target.as_mut() {
            name_expr.id = renamed_name.into();
        }
        ctx.inlined_stmts.push(Stmt::AnnAssign(ann_assign_clone));
    }

    /// Log unused imports details if debug logging is enabled
    fn log_unused_imports_details(unused_imports: &[crate::cribo_graph::UnusedImportInfo]) {
        if log::log_enabled!(log::Level::Debug) {
            for unused in unused_imports {
                log::debug!("  - {} from {}", unused.name, unused.module);
            }
        }
    }

    /// Resolve import aliases in an expression
    #[allow(clippy::only_used_in_recursion)]
    fn resolve_import_aliases_in_expr(
        &self,
        expr: &mut Expr,
        import_aliases: &FxIndexMap<String, String>,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                // Check if this name is an import alias
                if let Some(resolved) = import_aliases.get(name_expr.id.as_str()) {
                    // Check if this is a module:symbol format
                    if let Some(colon_pos) = resolved.find(':') {
                        let module = &resolved[..colon_pos];
                        let symbol = &resolved[colon_pos + 1..];

                        // For now, just use the symbol name as-is
                        // TODO: We need access to module_renames to resolve this properly
                        let actual_name = symbol;

                        log::debug!(
                            "Resolving import alias: {} -> {} (renamed from {}:{})",
                            name_expr.id,
                            actual_name,
                            module,
                            symbol
                        );
                        name_expr.id = actual_name.to_string().into();
                    } else {
                        log::debug!("Resolving import alias: {} -> {}", name_expr.id, resolved);
                        name_expr.id = resolved.clone().into();
                    }
                }
            }
            Expr::Attribute(attr_expr) => {
                self.resolve_import_aliases_in_expr(&mut attr_expr.value, import_aliases);
            }
            Expr::Call(call_expr) => {
                self.resolve_import_aliases_in_expr(&mut call_expr.func, import_aliases);
                for arg in &mut call_expr.arguments.args {
                    self.resolve_import_aliases_in_expr(arg, import_aliases);
                }
                for keyword in &mut call_expr.arguments.keywords {
                    self.resolve_import_aliases_in_expr(&mut keyword.value, import_aliases);
                }
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    self.resolve_import_aliases_in_expr(elem, import_aliases);
                }
            }
            Expr::Dict(dict_expr) => {
                for item in &mut dict_expr.items {
                    if let Some(ref mut key) = item.key {
                        self.resolve_import_aliases_in_expr(key, import_aliases);
                    }
                    self.resolve_import_aliases_in_expr(&mut item.value, import_aliases);
                }
            }
            Expr::Tuple(tuple_expr) => {
                for elem in &mut tuple_expr.elts {
                    self.resolve_import_aliases_in_expr(elem, import_aliases);
                }
            }
            Expr::BinOp(binop_expr) => {
                self.resolve_import_aliases_in_expr(&mut binop_expr.left, import_aliases);
                self.resolve_import_aliases_in_expr(&mut binop_expr.right, import_aliases);
            }
            Expr::UnaryOp(unaryop_expr) => {
                self.resolve_import_aliases_in_expr(&mut unaryop_expr.operand, import_aliases);
            }
            Expr::Compare(compare_expr) => {
                self.resolve_import_aliases_in_expr(&mut compare_expr.left, import_aliases);
                for comparator in &mut compare_expr.comparators {
                    self.resolve_import_aliases_in_expr(comparator, import_aliases);
                }
            }
            Expr::BoolOp(boolop_expr) => {
                for value in &mut boolop_expr.values {
                    self.resolve_import_aliases_in_expr(value, import_aliases);
                }
            }
            Expr::If(if_expr) => {
                self.resolve_import_aliases_in_expr(&mut if_expr.test, import_aliases);
                self.resolve_import_aliases_in_expr(&mut if_expr.body, import_aliases);
                self.resolve_import_aliases_in_expr(&mut if_expr.orelse, import_aliases);
            }
            Expr::Subscript(subscript_expr) => {
                self.resolve_import_aliases_in_expr(&mut subscript_expr.value, import_aliases);
                self.resolve_import_aliases_in_expr(&mut subscript_expr.slice, import_aliases);
            }
            _ => {} // Other expressions don't contain identifiers to resolve
        }
    }

    /// Resolve import aliases in a statement
    fn resolve_import_aliases_in_stmt(
        &self,
        stmt: &mut Stmt,
        import_aliases: &FxIndexMap<String, String>,
    ) {
        match stmt {
            Stmt::Assign(assign) => {
                self.resolve_import_aliases_in_expr(&mut assign.value, import_aliases);
            }
            Stmt::AnnAssign(ann_assign) => {
                if let Some(ref mut value) = ann_assign.value {
                    self.resolve_import_aliases_in_expr(value, import_aliases);
                }
                self.resolve_import_aliases_in_expr(&mut ann_assign.annotation, import_aliases);
            }
            Stmt::Return(return_stmt) => {
                if let Some(ref mut value) = return_stmt.value {
                    self.resolve_import_aliases_in_expr(value, import_aliases);
                }
            }
            Stmt::Expr(expr_stmt) => {
                self.resolve_import_aliases_in_expr(&mut expr_stmt.value, import_aliases);
            }
            Stmt::If(if_stmt) => {
                self.resolve_import_aliases_in_expr(&mut if_stmt.test, import_aliases);
                for body_stmt in &mut if_stmt.body {
                    self.resolve_import_aliases_in_stmt(body_stmt, import_aliases);
                }
                for elif_else in &mut if_stmt.elif_else_clauses {
                    if let Some(ref mut condition) = elif_else.test {
                        self.resolve_import_aliases_in_expr(condition, import_aliases);
                    }
                    for body_stmt in &mut elif_else.body {
                        self.resolve_import_aliases_in_stmt(body_stmt, import_aliases);
                    }
                }
            }
            Stmt::While(while_stmt) => {
                self.resolve_import_aliases_in_expr(&mut while_stmt.test, import_aliases);
                for body_stmt in &mut while_stmt.body {
                    self.resolve_import_aliases_in_stmt(body_stmt, import_aliases);
                }
                for else_stmt in &mut while_stmt.orelse {
                    self.resolve_import_aliases_in_stmt(else_stmt, import_aliases);
                }
            }
            Stmt::For(for_stmt) => {
                self.resolve_import_aliases_in_expr(&mut for_stmt.iter, import_aliases);
                for body_stmt in &mut for_stmt.body {
                    self.resolve_import_aliases_in_stmt(body_stmt, import_aliases);
                }
                for else_stmt in &mut for_stmt.orelse {
                    self.resolve_import_aliases_in_stmt(else_stmt, import_aliases);
                }
            }
            Stmt::FunctionDef(func_def) => {
                // Resolve in parameter defaults and annotations
                for param in &mut func_def.parameters.args {
                    if let Some(ref mut default) = param.default {
                        self.resolve_import_aliases_in_expr(default, import_aliases);
                    }
                    if let Some(ref mut annotation) = param.parameter.annotation {
                        self.resolve_import_aliases_in_expr(annotation, import_aliases);
                    }
                }
                // Resolve in return type annotation
                if let Some(ref mut returns) = func_def.returns {
                    self.resolve_import_aliases_in_expr(returns, import_aliases);
                }
                // Resolve in function body
                for stmt in &mut func_def.body {
                    self.resolve_import_aliases_in_stmt(stmt, import_aliases);
                }
            }
            Stmt::ClassDef(class_def) => {
                // Resolve in base classes
                if let Some(ref mut arguments) = class_def.arguments {
                    for arg in &mut arguments.args {
                        self.resolve_import_aliases_in_expr(arg, import_aliases);
                    }
                }
                // Resolve in class body
                for stmt in &mut class_def.body {
                    self.resolve_import_aliases_in_stmt(stmt, import_aliases);
                }
            }
            // Add more statement types as needed
            _ => {}
        }
    }

    /// Create a namespace object for a direct import of an inlined module
    fn create_namespace_object_for_direct_import(
        &mut self,
        module_name: &str,
        target_name: &str,
    ) -> Stmt {
        // Get module exports if available
        let exports = self
            .module_exports
            .get(module_name)
            .and_then(|e| e.as_ref())
            .cloned()
            .unwrap_or_default();

        // Check if the module has any symbols that were inlined
        let module_has_inlined_symbols =
            self.inlined_modules.contains(module_name) && !exports.is_empty();

        // Only use types.SimpleNamespace if we have actual symbols
        if module_has_inlined_symbols {
            // Create a types.SimpleNamespace with the actual exported symbols
            let mut keywords = Vec::new();

            // For each exported symbol, add it as a keyword argument to the namespace
            for export in exports {
                // For now, just use the export name directly
                // The symbol should already be available in the global scope after inlining
                keywords.push(Keyword {
                    node_index: AtomicNodeIndex::dummy(),
                    arg: Some(Identifier::new(export.clone(), TextRange::default())),
                    value: Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: export.into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    }),
                    range: TextRange::default(),
                });
            }

            // Create: target_name = types.SimpleNamespace(**kwargs)
            Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: target_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Call(ExprCall {
                    node_index: AtomicNodeIndex::dummy(),
                    func: Box::new(Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: "types".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("SimpleNamespace", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    arguments: Arguments {
                        node_index: AtomicNodeIndex::dummy(),
                        args: Box::from([]),
                        keywords: keywords.into_boxed_slice(),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            })
        } else {
            // No symbols to export, just assign None
            Stmt::Assign(StmtAssign {
                node_index: AtomicNodeIndex::dummy(),
                targets: vec![Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: target_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::NoneLiteral(ExprNoneLiteral {
                    node_index: AtomicNodeIndex::dummy(),
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            })
        }
    }

    /// Create a namespace object for an inlined module that is imported as a namespace
    fn create_namespace_for_inlined_module_static(
        &self,
        module_name: &str,
        module_renames: &FxIndexMap<String, String>,
    ) -> Stmt {
        // Create a types.SimpleNamespace with all the module's symbols
        let mut keywords = Vec::new();
        let mut seen_args = FxIndexSet::default();

        // Add all renamed symbols as keyword arguments, avoiding duplicates
        for (original_name, renamed_name) in module_renames {
            // Skip if we've already added this argument name
            if seen_args.contains(original_name) {
                log::debug!(
                    "Skipping duplicate namespace argument '{original_name}' for module \
                     '{module_name}'"
                );
                continue;
            }

            // Check if this symbol survived tree-shaking
            if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                && !kept_symbols.contains(&(module_name.to_string(), original_name.clone()))
            {
                log::debug!(
                    "Skipping tree-shaken symbol '{original_name}' from namespace for module \
                     '{module_name}'"
                );
                continue;
            }

            seen_args.insert(original_name.clone());

            keywords.push(Keyword {
                node_index: AtomicNodeIndex::dummy(),
                arg: Some(Identifier::new(original_name, TextRange::default())),
                value: Expr::Name(ExprName {
                    node_index: AtomicNodeIndex::dummy(),
                    id: renamed_name.clone().into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                }),
                range: TextRange::default(),
            });
        }

        // Also check if module has module-level variables that weren't renamed
        if let Some(exports) = self.module_exports.get(module_name)
            && let Some(export_list) = exports
        {
            for export in export_list {
                if !module_renames.contains_key(export) && !seen_args.contains(export) {
                    // Check if this symbol survived tree-shaking
                    if let Some(ref kept_symbols) = self.tree_shaking_keep_symbols
                        && !kept_symbols.contains(&(module_name.to_string(), export.clone()))
                    {
                        log::debug!(
                            "Skipping tree-shaken export '{export}' from namespace for module \
                             '{module_name}'"
                        );
                        continue;
                    }

                    // This export wasn't renamed and wasn't already added, add it directly
                    seen_args.insert(export.clone());
                    keywords.push(Keyword {
                        node_index: AtomicNodeIndex::dummy(),
                        arg: Some(Identifier::new(export, TextRange::default())),
                        value: Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: export.clone().into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        }),
                        range: TextRange::default(),
                    });
                }
            }
        }

        // Create the namespace variable name
        let namespace_var = module_name.cow_replace('.', "_").into_owned();

        // Create namespace = types.SimpleNamespace(**kwargs) assignment
        Stmt::Assign(StmtAssign {
            node_index: AtomicNodeIndex::dummy(),
            targets: vec![Expr::Name(ExprName {
                node_index: AtomicNodeIndex::dummy(),
                id: namespace_var.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                node_index: AtomicNodeIndex::dummy(),
                func: Box::new(Expr::Attribute(ExprAttribute {
                    node_index: AtomicNodeIndex::dummy(),
                    value: Box::new(Expr::Name(ExprName {
                        node_index: AtomicNodeIndex::dummy(),
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: Arguments {
                    node_index: AtomicNodeIndex::dummy(),
                    args: Box::from([]),
                    keywords: keywords.into_boxed_slice(),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }
}
