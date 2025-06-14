use anyhow::Result;
#[allow(unused_imports)] // CowUtils trait is used for the replace method
use cow_utils::CowUtils;
use indexmap::{IndexMap, IndexSet};
use log::debug;
use ruff_python_ast::{
    Arguments, CmpOp, Comprehension, ExceptHandler, Expr, ExprAttribute, ExprCall, ExprCompare,
    ExprContext, ExprFString, ExprIf, ExprList, ExprName, ExprNoneLiteral, ExprStringLiteral,
    FString, FStringFlags, FStringValue, Identifier, InterpolatedElement,
    InterpolatedStringElement, InterpolatedStringElements, Keyword, ModModule, Stmt, StmtAssign,
    StmtClassDef, StmtFunctionDef, StmtIf, StmtImport, StmtImportFrom, StringLiteral,
    StringLiteralFlags, StringLiteralValue,
};
use ruff_text_size::TextRange;
use rustc_hash::FxHasher;
use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};

use crate::cribo_graph::CriboGraph as DependencyGraph;
use crate::semantic_bundler::{ModuleGlobalInfo, SemanticBundler, SymbolRegistry};

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
}

/// Context for semantic analysis operations
struct SemanticContext<'a> {
    graph: &'a DependencyGraph,
    symbol_registry: &'a SymbolRegistry,
    semantic_bundler: &'a SemanticBundler,
}

/// Parameters for namespace import operations
struct NamespaceImportParams<'a> {
    local_name: &'a str,
    imported_name: &'a str,
    resolved_module: &'a str,
    full_module_path: &'a str,
}

/// Parameters for processing module globals
struct ProcessGlobalsParams<'a> {
    module_name: &'a str,
    ast: &'a ModModule,
    semantic_ctx: &'a SemanticContext<'a>,
}

/// Parameters for handling inlined module imports
struct InlinedImportParams<'a> {
    import_from: &'a StmtImportFrom,
    resolved_module: &'a str,
    ctx: &'a ModuleTransformContext<'a>,
}

/// Parameters for adding symbols to namespace
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
}

/// Transformer that lifts module-level globals to true global scope
struct GlobalsLifter {
    /// Map from original name to lifted name
    lifted_names: FxIndexMap<String, String>,
    /// Statements to add at module top level
    lifted_declarations: Vec<Stmt>,
}

/// Transform globals() calls to module.__dict__ when inside module functions
fn transform_globals_in_expr(expr: &mut Expr) {
    match expr {
        Expr::Call(call_expr) => {
            // Check if this is a globals() call
            if let Expr::Name(name_expr) = &*call_expr.func {
                if name_expr.id.as_str() == "globals" && call_expr.arguments.args.is_empty() {
                    // Replace the entire expression with module.__dict__
                    *expr = Expr::Attribute(ExprAttribute {
                        value: Box::new(Expr::Name(ExprName {
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
                let lifted_name = format!("__cribo_{}_{}", module_name_sanitized, var_name);

                debug!(
                    "Creating lifted declaration for {} -> {}",
                    var_name, lifted_name
                );

                lifted_names.insert(var_name.clone(), lifted_name.clone());

                // Create assignment: __cribo_module_var = None (will be set by init function)
                lifted_declarations.push(Stmt::Assign(StmtAssign {
                    targets: vec![Expr::Name(ExprName {
                        id: lifted_name.into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::NoneLiteral(ExprNoneLiteral {
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
    /// Tracks namespace modules already created in the entry module to avoid duplicates
    /// Used when processing multiple dotted imports
    created_namespace_modules: FxIndexSet<String>,
}

impl Default for HybridStaticBundler {
    fn default() -> Self {
        Self::new()
    }
}

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
            created_namespace_modules: FxIndexSet::default(),
        }
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
        // First, collect all imported names
        let mut imported_names = FxIndexSet::default();
        for stmt in &ast.body {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                        imported_names.insert(name.to_string());
                    }
                }
                Stmt::ImportFrom(import_from) => {
                    for alias in &import_from.names {
                        let name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                        imported_names.insert(name.to_string());
                    }
                }
                _ => {}
            }
        }

        for stmt in &ast.body {
            match stmt {
                // These statements are pure definitions, no side effects
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) | Stmt::AnnAssign(_) => continue,

                // Simple variable assignments are generally safe
                Stmt::Assign(assign) => {
                    // Special case: __all__ assignments are metadata, not side effects
                    if Self::is_all_assignment(assign) {
                        continue;
                    }
                    // Check if the assignment has function calls or other complex expressions
                    if Self::expression_has_side_effects(&assign.value) {
                        return true;
                    }
                    // Check if the assignment uses imported names
                    if Self::expression_uses_imported_names(&assign.value, &imported_names) {
                        return true;
                    }
                }

                // Import statements are handled separately by the bundler
                Stmt::Import(_) | Stmt::ImportFrom(_) => continue,

                // Type alias statements are safe
                Stmt::TypeAlias(_) => continue,

                // Pass statements are no-ops and safe
                Stmt::Pass(_) => continue,

                // Expression statements - check if they're docstrings
                Stmt::Expr(expr_stmt) => {
                    if matches!(expr_stmt.value.as_ref(), Expr::StringLiteral(_)) {
                        // Docstring - safe
                        continue;
                    } else {
                        // Other expression statements have side effects
                        return true;
                    }
                }

                // These are definitely side effects
                Stmt::If(_)
                | Stmt::While(_)
                | Stmt::For(_)
                | Stmt::With(_)
                | Stmt::Match(_)
                | Stmt::Raise(_)
                | Stmt::Try(_)
                | Stmt::Assert(_)
                | Stmt::Global(_)
                | Stmt::Nonlocal(_)
                | Stmt::Delete(_) => return true,

                // Any other statement type is considered a side effect
                _ => return true,
            }
        }
        false
    }

    /// Check if an expression uses any imported names
    fn expression_uses_imported_names(expr: &Expr, imported_names: &FxIndexSet<String>) -> bool {
        match expr {
            Expr::Name(name) => imported_names.contains(name.id.as_str()),

            // Recursively check compound expressions
            Expr::List(list) => list
                .elts
                .iter()
                .any(|e| Self::expression_uses_imported_names(e, imported_names)),
            Expr::Tuple(tuple) => tuple
                .elts
                .iter()
                .any(|e| Self::expression_uses_imported_names(e, imported_names)),
            Expr::Dict(dict) => dict.items.iter().any(|item| {
                item.key
                    .as_ref()
                    .is_some_and(|k| Self::expression_uses_imported_names(k, imported_names))
                    || Self::expression_uses_imported_names(&item.value, imported_names)
            }),
            Expr::Set(set) => set
                .elts
                .iter()
                .any(|e| Self::expression_uses_imported_names(e, imported_names)),

            Expr::BinOp(binop) => {
                Self::expression_uses_imported_names(&binop.left, imported_names)
                    || Self::expression_uses_imported_names(&binop.right, imported_names)
            }

            Expr::UnaryOp(unaryop) => {
                Self::expression_uses_imported_names(&unaryop.operand, imported_names)
            }

            Expr::Call(call) => {
                Self::expression_uses_imported_names(&call.func, imported_names)
                    || call
                        .arguments
                        .args
                        .iter()
                        .any(|arg| Self::expression_uses_imported_names(arg, imported_names))
                    || call
                        .arguments
                        .keywords
                        .iter()
                        .any(|kw| Self::expression_uses_imported_names(&kw.value, imported_names))
            }

            Expr::Attribute(attr) => {
                Self::expression_uses_imported_names(&attr.value, imported_names)
            }

            Expr::Subscript(sub) => {
                Self::expression_uses_imported_names(&sub.value, imported_names)
                    || Self::expression_uses_imported_names(&sub.slice, imported_names)
            }

            // Literals don't use imported names
            _ => false,
        }
    }

    /// Check if an expression has side effects
    fn expression_has_side_effects(expr: &Expr) -> bool {
        match expr {
            // Literals and simple names are safe
            Expr::NumberLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::Name(_) => false,

            // List/tuple/dict/set literals are safe if their elements are
            Expr::List(list) => list.elts.iter().any(Self::expression_has_side_effects),
            Expr::Tuple(tuple) => tuple.elts.iter().any(Self::expression_has_side_effects),
            Expr::Dict(dict) => dict.items.iter().any(|item| {
                item.key
                    .as_ref()
                    .is_some_and(Self::expression_has_side_effects)
                    || Self::expression_has_side_effects(&item.value)
            }),
            Expr::Set(set) => set.elts.iter().any(Self::expression_has_side_effects),

            // Binary operations on literals are safe
            Expr::BinOp(binop) => {
                Self::expression_has_side_effects(&binop.left)
                    || Self::expression_has_side_effects(&binop.right)
            }

            // Unary operations are safe if the operand is
            Expr::UnaryOp(unaryop) => Self::expression_has_side_effects(&unaryop.operand),

            // Function calls always have potential side effects
            Expr::Call(_) => true,

            // Attribute access might trigger __getattr__, so it's a side effect
            Expr::Attribute(_) => true,

            // Subscripts might trigger __getitem__, so it's a side effect
            Expr::Subscript(_) => true,

            // Comprehensions need recursive checking of their parts
            Expr::ListComp(comp) => {
                Self::expression_has_side_effects(&comp.elt)
                    || Self::generators_have_side_effects(&comp.generators)
            }
            Expr::SetComp(comp) => {
                Self::expression_has_side_effects(&comp.elt)
                    || Self::generators_have_side_effects(&comp.generators)
            }
            Expr::DictComp(comp) => {
                Self::expression_has_side_effects(&comp.key)
                    || Self::expression_has_side_effects(&comp.value)
                    || Self::generators_have_side_effects(&comp.generators)
            }
            Expr::Generator(comp) => {
                Self::expression_has_side_effects(&comp.elt)
                    || Self::generators_have_side_effects(&comp.generators)
            }

            // Any other expression type is considered to have side effects
            _ => true,
        }
    }

    /// Check if an assignment is to __all__
    fn is_all_assignment(assign: &StmtAssign) -> bool {
        if assign.targets.len() != 1 {
            return false;
        }
        matches!(&assign.targets[0], Expr::Name(name) if name.id.as_str() == "__all__")
    }

    /// Check if comprehension generators have side effects
    fn generators_have_side_effects(generators: &[Comprehension]) -> bool {
        for generator in generators {
            // Check the iterator expression
            if Self::expression_has_side_effects(&generator.iter) {
                return true;
            }
            // Check all condition expressions
            for condition in &generator.ifs {
                if Self::expression_has_side_effects(condition) {
                    return true;
                }
            }
        }
        false
    }

    /// Generate synthetic module name using content hash
    fn get_synthetic_module_name(&self, module_name: &str, content_hash: &str) -> String {
        let module_name_escaped = module_name
            .chars()
            .map(|c| if c == '.' { '_' } else { c })
            .collect::<String>();
        // Use first 6 characters of content hash for readability
        let short_hash = &content_hash[..6];
        format!("__cribo_{}_{}", short_hash, module_name_escaped)
    }

    /// Trim unused imports from all modules before bundling using graph information
    fn trim_unused_imports_from_modules(
        &mut self,
        modules: Vec<(String, ModModule, PathBuf, String)>,
        graph: &DependencyGraph,
    ) -> Result<Vec<(String, ModModule, PathBuf, String)>> {
        let mut trimmed_modules = Vec::new();

        for (module_name, mut ast, module_path, content_hash) in modules {
            log::debug!("Trimming unused imports from module: {}", module_name);

            // Check if this is an __init__.py file
            let is_init_py =
                module_path.file_name().and_then(|name| name.to_str()) == Some("__init__.py");

            // Get unused imports from the graph
            if let Some(module_dep_graph) = graph.get_module_by_name(&module_name) {
                let unused_imports = module_dep_graph.find_unused_imports(is_init_py);

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

        // Second pass: trim unused imports from all modules
        let modules = self.trim_unused_imports_from_modules(params.modules, params.graph)?;

        // Check which modules have function-scoped imports (from import rewriting)
        let modules_with_function_imports = self.find_modules_with_function_imports(&modules);

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
        log::debug!("Directly imported modules: {:?}", directly_imported_modules);

        // Find modules that are imported as namespaces (e.g., from models import base)
        self.find_namespace_imported_modules(&modules);

        // Separate modules into inlinable, wrapper, and namespace-hybrid modules
        let mut inlinable_modules = Vec::new();
        let mut wrapper_modules = Vec::new();
        let mut namespace_hybrid_modules = Vec::new();
        let mut module_exports_map = FxIndexMap::default();

        for (module_name, ast, module_path, content_hash) in &modules {
            log::info!("Processing module: '{}'", module_name);
            if module_name == params.entry_module_name {
                continue;
            }

            // Extract __all__ exports from the module
            let module_exports = self.extract_all_exports(ast);
            module_exports_map.insert(module_name.clone(), module_exports.clone());

            // Check if module is imported as a namespace
            let is_namespace_imported = self.namespace_imported_modules.contains_key(module_name);

            if is_namespace_imported {
                log::info!(
                    "Module '{}' is imported as namespace by: {:?}",
                    module_name,
                    self.namespace_imported_modules.get(module_name)
                );
            }

            // Check if module can be inlined
            // A module can only be inlined if:
            // 1. It has no side effects
            // 2. It's never imported directly (only from X import Y style)
            // 3. It's not imported as a namespace
            // 4. It doesn't have function-scoped imports (from import rewriting)
            let has_side_effects = Self::has_side_effects(ast);
            let is_directly_imported = directly_imported_modules.contains(module_name);
            let has_function_imports = modules_with_function_imports.contains(module_name);

            if is_namespace_imported {
                // Module is imported as namespace - use hybrid approach
                log::debug!(
                    "Module '{}' is imported as namespace - using hybrid inlining approach",
                    module_name
                );
                namespace_hybrid_modules.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            } else if has_side_effects || is_directly_imported || has_function_imports {
                let reason = if has_side_effects {
                    "has side effects"
                } else if is_directly_imported {
                    "is imported directly"
                } else {
                    "has function-scoped imports"
                };
                log::debug!(
                    "Module '{}' {} - using wrapper approach",
                    module_name,
                    reason
                );
                wrapper_modules.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash.clone(),
                ));
            } else {
                log::debug!(
                    "Module '{}' has no side effects and is not imported directly - can be inlined",
                    module_name
                );
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
        }

        // Track namespace hybrid modules as both bundled and inlined
        // They need to be in inlined_modules for namespace creation to work
        for (module_name, _, _, _) in &namespace_hybrid_modules {
            self.bundled_modules.insert(module_name.clone());
            self.inlined_modules.insert(module_name.clone());
        }

        // First pass: normalize stdlib import aliases in ALL modules before collecting imports
        let mut modules_normalized = modules;
        for (_module_name, ast, _, _) in &mut modules_normalized {
            self.normalize_stdlib_import_aliases(ast);
        }

        // Second pass: collect imports from ALL modules (for hoisting)
        for (_module_name, ast, _, _) in &modules_normalized {
            self.collect_imports_from_module(ast);
        }

        // If we have wrapper modules, inject sys and types as stdlib dependencies
        if !wrapper_modules.is_empty() {
            self.add_stdlib_import("sys");
            self.add_stdlib_import("types");
        }

        // If we have namespace imports, inject types as stdlib dependency
        if !self.namespace_imported_modules.is_empty() {
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
            let init_func_name = format!("__cribo_init_{}", synthetic_name);
            self.init_functions.insert(synthetic_name, init_func_name);
        }

        // Also register namespace hybrid modules' exports
        // We'll use either explicit __all__ exports or fall back to all module-level symbols
        for (module_name, ast, _, _) in &namespace_hybrid_modules {
            let explicit_exports = module_exports_map.get(module_name).cloned().flatten();

            // If no explicit __all__, extract all module-level symbols as exports
            let exports = if explicit_exports.is_none() {
                // Extract all module-level assignments, functions, and classes
                let mut symbols = Vec::new();
                for stmt in &ast.body {
                    self.collect_module_symbol(stmt, &mut symbols);
                }
                if symbols.is_empty() {
                    None
                } else {
                    Some(symbols)
                }
            } else {
                explicit_exports
            };

            self.module_exports.insert(module_name.clone(), exports);
        }

        // Add imports first
        self.add_hoisted_imports(&mut final_body);

        // Check if we need sys import (for wrapper modules)
        let need_sys_import = !wrapper_modules.is_empty();

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

        // Process namespace hybrid modules - inline their symbols with module-qualified names
        for (module_name, ast, _module_path, _content_hash) in &namespace_hybrid_modules {
            log::debug!("Processing namespace hybrid module '{}'", module_name);
            let mut inlined_stmts = Vec::new();
            let mut inline_ctx = InlineContext {
                module_exports_map: &module_exports_map,
                global_symbols: &mut global_symbols,
                module_renames: &mut symbol_renames,
                inlined_stmts: &mut inlined_stmts,
                import_aliases: FxIndexMap::default(),
            };
            self.inline_module_for_namespace(
                module_name,
                ast.clone(),
                _module_path,
                &mut inline_ctx,
            )?;
            log::debug!(
                "Inlined {} statements from namespace hybrid module '{}'",
                inlined_stmts.len(),
                module_name
            );
            final_body.extend(inlined_stmts);
        }

        // Inline the inlinable modules FIRST to populate symbol_renames
        // This ensures we know what symbols have been renamed before processing wrapper modules
        for (module_name, ast, _module_path, _content_hash) in &inlinable_modules {
            log::debug!("Inlining module '{}'", module_name);
            let mut inlined_stmts = Vec::new();
            let mut inline_ctx = InlineContext {
                module_exports_map: &module_exports_map,
                global_symbols: &mut global_symbols,
                module_renames: &mut symbol_renames,
                inlined_stmts: &mut inlined_stmts,
                import_aliases: FxIndexMap::default(),
            };
            self.inline_module(module_name, ast.clone(), _module_path, &mut inline_ctx)?;
            log::debug!(
                "Inlined {} statements from module '{}'",
                inlined_stmts.len(),
                module_name
            );
            final_body.extend(inlined_stmts);
        }

        // Now transform wrapper modules into init functions AFTER inlining
        // This way we have access to symbol_renames for proper import resolution
        if need_sys_import {
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
        if need_sys_import {
            for (module_name, _, _) in params.sorted_modules {
                if module_name == params.entry_module_name {
                    continue;
                }

                if let Some(synthetic_name) = self.module_registry.get(module_name) {
                    let init_call = self.generate_module_init_call(synthetic_name);
                    final_body.push(init_call);
                }
            }

            // Note: Post-init attribute generation is disabled because wrapper modules
            // already handle their own imports correctly in their init functions
        }

        // Finally, add entry module code (it's always last in topological order)
        for (module_name, ast, _, _) in &modules_normalized {
            if module_name != params.entry_module_name {
                continue;
            }

            // Entry module - add its code directly at the end
            // The entry module needs special handling for symbol conflicts
            let entry_module_renames = symbol_renames.get(module_name).cloned().unwrap_or_default();

            log::debug!(
                "Entry module '{}' renames: {:?}",
                module_name,
                entry_module_renames
            );

            for mut stmt in ast.body.clone() {
                if self.is_hoisted_import(&stmt) {
                    continue;
                }

                // For the entry module, we need to handle both imports and symbol references
                match &mut stmt {
                    Stmt::ImportFrom(_) => {
                        // Handle from imports with renaming
                        let rewritten_stmts = self.rewrite_import_in_stmt_multiple_with_context(
                            stmt,
                            module_name,
                            &symbol_renames,
                        );
                        final_body.extend(rewritten_stmts);
                    }
                    Stmt::Import(import_stmt) => {
                        // Handle regular imports with namespace tracking for entry module
                        let rewritten_stmts = self.rewrite_import_entry_module(import_stmt.clone());
                        final_body.extend(rewritten_stmts);
                    }
                    _ => {
                        self.process_entry_module_statement(
                            &mut stmt,
                            &entry_module_renames,
                            &mut final_body,
                        );
                    }
                }
            }
        }

        Ok(ModModule {
            range: TextRange::default(),
            body: final_body,
        })
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
        if let Some((original, renamed)) = pending_reassignment {
            if original != renamed {
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
            log::debug!(
                "Renaming function '{}' to '{}' in entry module",
                func_name,
                new_name
            );
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
            log::debug!(
                "Renaming class '{}' to '{}' in entry module",
                class_name,
                new_name
            );
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

        // Check if module already exists in sys.modules
        body.push(self.create_module_exists_check(ctx.synthetic_name));

        // Create module object (returns multiple statements)
        body.extend(self.create_module_object_stmt(ctx.synthetic_name, ctx.module_path));

        // Register in sys.modules with both synthetic and original names
        body.push(self.create_sys_modules_registration(ctx.synthetic_name));
        body.push(self.create_sys_modules_registration_alias(ctx.synthetic_name, ctx.module_name));

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

        // Transform module contents
        for stmt in ast.body {
            match &stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // Transform any import statements in non-entry modules
                    self.process_wrapper_module_import(
                        stmt.clone(),
                        &ctx,
                        symbol_renames,
                        &mut body,
                    );
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
                    // Add function definition
                    body.push(stmt.clone());
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
                        // For simple assignments, also set as module attribute if it should be exported
                        body.push(stmt.clone());
                        self.add_module_attr_if_exported(assign, ctx.module_name, &mut body);
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
                    names: vec![Identifier::new(lifted_name, TextRange::default())],
                    range: TextRange::default(),
                }));

                // __cribo_module_var = original_var
                body.push(Stmt::Assign(StmtAssign {
                    targets: vec![Expr::Name(ExprName {
                        id: lifted_name.clone().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Name(ExprName {
                        id: original_name.clone().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));
            }
        }

        // Generate __all__ for the bundled module only if the original module had __all__
        if let Some(Some(_)) = self.module_exports.get(ctx.module_name) {
            body.push(self.create_all_assignment_for_module(ctx.module_name));
        }

        // Return the module object
        body.push(Stmt::Return(ruff_python_ast::StmtReturn {
            value: Some(Box::new(Expr::Name(ExprName {
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
            name: Identifier::new(init_func_name, TextRange::default()),
            type_params: None,
            parameters: Box::new(ruff_python_ast::Parameters {
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
        let mut stmts = Vec::new();

        // Create module registry
        stmts.push(self.create_module_registry());

        // Create init functions registry
        stmts.push(self.create_init_functions_registry());

        // Create and install import hook
        stmts.extend(self.create_import_hook());

        stmts
    }

    /// Create the __cribo_modules registry
    fn create_module_registry(&self) -> Stmt {
        let mut items = Vec::new();

        for (original_name, synthetic_name) in &self.module_registry {
            items.push(ruff_python_ast::DictItem {
                key: Some(self.create_string_literal(original_name)),
                value: self.create_string_literal(synthetic_name),
            });
        }

        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Name(ExprName {
                id: "__cribo_modules".into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Dict(ruff_python_ast::ExprDict {
                items,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create the __cribo_init_functions registry
    fn create_init_functions_registry(&self) -> Stmt {
        let mut items = Vec::new();

        for (synthetic_name, init_func_name) in &self.init_functions {
            items.push(ruff_python_ast::DictItem {
                key: Some(self.create_string_literal(synthetic_name)),
                value: Expr::Name(ExprName {
                    id: init_func_name.as_str().into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                }),
            });
        }

        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Name(ExprName {
                id: "__cribo_init_functions".into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Dict(ruff_python_ast::ExprDict {
                items,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create the import hook class and install it
    fn create_import_hook(&self) -> Vec<Stmt> {
        let mut stmts = Vec::new();

        // Define CriboBundledFinder class
        let finder_class = self.create_finder_class();
        stmts.push(finder_class);

        // Install the hook: sys.meta_path.insert(0, CriboBundledFinder(__cribo_modules, __cribo_init_functions))
        let install_stmt = Stmt::Expr(ruff_python_ast::StmtExpr {
            value: Box::new(Expr::Call(ExprCall {
                func: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Attribute(ExprAttribute {
                        value: Box::new(Expr::Name(ExprName {
                            id: "sys".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("meta_path", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("insert", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    args: Box::from([
                        self.create_number_literal(0),
                        Expr::Call(ExprCall {
                            func: Box::new(Expr::Name(ExprName {
                                id: "CriboBundledFinder".into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            arguments: ruff_python_ast::Arguments {
                                args: Box::from([
                                    Expr::Name(ExprName {
                                        id: "__cribo_modules".into(),
                                        ctx: ExprContext::Load,
                                        range: TextRange::default(),
                                    }),
                                    Expr::Name(ExprName {
                                        id: "__cribo_init_functions".into(),
                                        ctx: ExprContext::Load,
                                        range: TextRange::default(),
                                    }),
                                ]),
                                keywords: Box::from([]),
                                range: TextRange::default(),
                            },
                            range: TextRange::default(),
                        }),
                    ]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        });

        stmts.push(install_stmt);
        stmts
    }

    /// Create the CriboBundledFinder class
    fn create_finder_class(&self) -> Stmt {
        use ruff_python_ast::{Parameter, ParameterWithDefault, StmtClassDef, StmtReturn};

        let mut class_body = Vec::new();

        // __init__ method
        let init_params = vec![
            ParameterWithDefault {
                parameter: Parameter {
                    name: Identifier::new("self", TextRange::default()),
                    annotation: None,
                    range: TextRange::default(),
                },
                default: None,
                range: TextRange::default(),
            },
            ParameterWithDefault {
                parameter: Parameter {
                    name: Identifier::new("module_registry", TextRange::default()),
                    annotation: None,
                    range: TextRange::default(),
                },
                default: None,
                range: TextRange::default(),
            },
            ParameterWithDefault {
                parameter: Parameter {
                    name: Identifier::new("init_functions", TextRange::default()),
                    annotation: None,
                    range: TextRange::default(),
                },
                default: None,
                range: TextRange::default(),
            },
        ];

        let init_body = vec![
            // self.module_registry = module_registry
            Stmt::Assign(StmtAssign {
                targets: vec![Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "self".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("module_registry", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Name(ExprName {
                    id: "module_registry".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }),
            // self.init_functions = init_functions
            Stmt::Assign(StmtAssign {
                targets: vec![Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "self".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("init_functions", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Name(ExprName {
                    id: "init_functions".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }),
        ];

        let init_method = Stmt::FunctionDef(StmtFunctionDef {
            name: Identifier::new("__init__", TextRange::default()),
            type_params: None,
            parameters: Box::new(ruff_python_ast::Parameters {
                posonlyargs: vec![],
                args: init_params,
                vararg: None,
                kwonlyargs: vec![],
                kwarg: None,
                range: TextRange::default(),
            }),
            returns: None,
            body: init_body,
            decorator_list: vec![],
            is_async: false,
            range: TextRange::default(),
        });

        class_body.push(init_method);

        // find_spec method
        let find_spec_params = vec![
            ParameterWithDefault {
                parameter: Parameter {
                    name: Identifier::new("self", TextRange::default()),
                    annotation: None,
                    range: TextRange::default(),
                },
                default: None,
                range: TextRange::default(),
            },
            ParameterWithDefault {
                parameter: Parameter {
                    name: Identifier::new("fullname", TextRange::default()),
                    annotation: None,
                    range: TextRange::default(),
                },
                default: None,
                range: TextRange::default(),
            },
            ParameterWithDefault {
                parameter: Parameter {
                    name: Identifier::new("path", TextRange::default()),
                    annotation: None,
                    range: TextRange::default(),
                },
                default: None,
                range: TextRange::default(),
            },
            ParameterWithDefault {
                parameter: Parameter {
                    name: Identifier::new("target", TextRange::default()),
                    annotation: None,
                    range: TextRange::default(),
                },
                default: Some(Box::new(Expr::NoneLiteral(
                    ruff_python_ast::ExprNoneLiteral {
                        range: TextRange::default(),
                    },
                ))),
                range: TextRange::default(),
            },
        ];

        let find_spec_kwonlyargs = vec![];

        let mut find_spec_body = Vec::new();

        // if fullname in self.module_registry:
        let condition = Expr::Compare(ruff_python_ast::ExprCompare {
            left: Box::new(Expr::Name(ExprName {
                id: "fullname".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            ops: Box::from([ruff_python_ast::CmpOp::In]),
            comparators: Box::from([Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: "self".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("module_registry", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })]),
            range: TextRange::default(),
        });

        let mut if_body = Vec::new();

        // synthetic_name = self.module_registry[fullname]
        if_body.push(Stmt::Assign(StmtAssign {
            targets: vec![Expr::Name(ExprName {
                id: "synthetic_name".into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                value: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "self".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("module_registry", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                slice: Box::new(Expr::Name(ExprName {
                    id: "fullname".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        }));

        // if synthetic_name not in sys.modules:
        let inner_condition = Expr::Compare(ruff_python_ast::ExprCompare {
            left: Box::new(Expr::Name(ExprName {
                id: "synthetic_name".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            ops: Box::from([ruff_python_ast::CmpOp::NotIn]),
            comparators: Box::from([Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: "sys".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("modules", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })]),
            range: TextRange::default(),
        });

        let mut inner_if_body = Vec::new();

        // init_func = self.init_functions.get(synthetic_name)
        inner_if_body.push(Stmt::Assign(StmtAssign {
            targets: vec![Expr::Name(ExprName {
                id: "init_func".into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                func: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Attribute(ExprAttribute {
                        value: Box::new(Expr::Name(ExprName {
                            id: "self".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("init_functions", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("get", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    args: Box::from([Expr::Name(ExprName {
                        id: "synthetic_name".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        }));

        // if init_func:
        let init_func_condition = Expr::Name(ExprName {
            id: "init_func".into(),
            ctx: ExprContext::Load,
            range: TextRange::default(),
        });

        let init_func_if_body = vec![
            // init_func()
            Stmt::Expr(ruff_python_ast::StmtExpr {
                value: Box::new(Expr::Call(ExprCall {
                    func: Box::new(Expr::Name(ExprName {
                        id: "init_func".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    arguments: ruff_python_ast::Arguments {
                        args: Box::from([]),
                        keywords: Box::from([]),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }),
        ];

        inner_if_body.push(Stmt::If(StmtIf {
            test: Box::new(init_func_condition),
            body: init_func_if_body,
            elif_else_clauses: vec![],
            range: TextRange::default(),
        }));

        if_body.push(Stmt::If(StmtIf {
            test: Box::new(inner_condition),
            body: inner_if_body,
            elif_else_clauses: vec![],
            range: TextRange::default(),
        }));

        // import importlib.util
        if_body.push(Stmt::Import(StmtImport {
            names: vec![ruff_python_ast::Alias {
                name: Identifier::new("importlib.util", TextRange::default()),
                asname: None,
                range: TextRange::default(),
            }],
            range: TextRange::default(),
        }));

        // return importlib.util.find_spec(synthetic_name)
        if_body.push(Stmt::Return(StmtReturn {
            value: Some(Box::new(Expr::Call(ExprCall {
                func: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Attribute(ExprAttribute {
                        value: Box::new(Expr::Name(ExprName {
                            id: "importlib".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("util", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("find_spec", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    args: Box::from([Expr::Name(ExprName {
                        id: "synthetic_name".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            }))),
            range: TextRange::default(),
        }));

        find_spec_body.push(Stmt::If(StmtIf {
            test: Box::new(condition),
            body: if_body,
            elif_else_clauses: vec![],
            range: TextRange::default(),
        }));

        // return None
        find_spec_body.push(Stmt::Return(StmtReturn {
            value: Some(Box::new(Expr::NoneLiteral(
                ruff_python_ast::ExprNoneLiteral {
                    range: TextRange::default(),
                },
            ))),
            range: TextRange::default(),
        }));

        let find_spec_method = Stmt::FunctionDef(StmtFunctionDef {
            name: Identifier::new("find_spec", TextRange::default()),
            type_params: None,
            parameters: Box::new(ruff_python_ast::Parameters {
                posonlyargs: vec![],
                args: find_spec_params,
                vararg: None,
                kwonlyargs: find_spec_kwonlyargs,
                kwarg: None,
                range: TextRange::default(),
            }),
            returns: None,
            body: find_spec_body,
            decorator_list: vec![],
            is_async: false,
            range: TextRange::default(),
        });

        class_body.push(find_spec_method);

        // Create the class definition
        Stmt::ClassDef(StmtClassDef {
            name: Identifier::new("CriboBundledFinder", TextRange::default()),
            type_params: None,
            arguments: None,
            body: class_body,
            decorator_list: vec![],
            range: TextRange::default(),
        })
    }

    /// Create a string literal expression
    fn create_string_literal(&self, value: &str) -> Expr {
        Expr::StringLiteral(ExprStringLiteral {
            value: StringLiteralValue::single(ruff_python_ast::StringLiteral {
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
            value: ruff_python_ast::Number::Int(ruff_python_ast::Int::from(value as u32)),
            range: TextRange::default(),
        })
    }

    /// Check if module exists in sys.modules
    fn create_module_exists_check(&self, synthetic_name: &str) -> Stmt {
        let condition = Expr::Compare(ruff_python_ast::ExprCompare {
            left: Box::new(self.create_string_literal(synthetic_name)),
            ops: Box::from([ruff_python_ast::CmpOp::In]),
            comparators: Box::from([Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: "sys".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("modules", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })]),
            range: TextRange::default(),
        });

        Stmt::If(StmtIf {
            test: Box::new(condition),
            body: vec![Stmt::Return(ruff_python_ast::StmtReturn {
                value: Some(Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                    value: Box::new(Expr::Attribute(ExprAttribute {
                        value: Box::new(Expr::Name(ExprName {
                            id: "sys".into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new("modules", TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    slice: Box::new(self.create_string_literal(synthetic_name)),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                }))),
                range: TextRange::default(),
            })],
            elif_else_clauses: vec![],
            range: TextRange::default(),
        })
    }

    /// Create module object
    fn create_module_object_stmt(&self, synthetic_name: &str, _module_path: &Path) -> Vec<Stmt> {
        let module_call = Expr::Call(ExprCall {
            func: Box::new(Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: "types".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("ModuleType", TextRange::default()),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            arguments: ruff_python_ast::Arguments {
                args: Box::from([self.create_string_literal(synthetic_name)]),
                keywords: Box::from([]),
                range: TextRange::default(),
            },
            range: TextRange::default(),
        });

        vec![
            // module = types.ModuleType(synthetic_name)
            Stmt::Assign(StmtAssign {
                targets: vec![Expr::Name(ExprName {
                    id: "module".into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(module_call),
                range: TextRange::default(),
            }),
            // module.__file__ = __file__ if '__file__' in globals() else None
            Stmt::Assign(StmtAssign {
                targets: vec![Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "module".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("__file__", TextRange::default()),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::If(ExprIf {
                    test: Box::new(Expr::Compare(ExprCompare {
                        left: Box::new(Expr::StringLiteral(ExprStringLiteral {
                            value: StringLiteralValue::single(StringLiteral {
                                value: Box::from("__file__"),
                                range: TextRange::default(),
                                flags: StringLiteralFlags::empty(),
                            }),
                            range: TextRange::default(),
                        })),
                        ops: Box::from([CmpOp::In]),
                        comparators: Box::from([Expr::Call(ExprCall {
                            func: Box::new(Expr::Name(ExprName {
                                id: "globals".into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            arguments: ruff_python_ast::Arguments {
                                args: Box::from([]),
                                keywords: Box::from([]),
                                range: TextRange::default(),
                            },
                            range: TextRange::default(),
                        })]),
                        range: TextRange::default(),
                    })),
                    body: Box::new(Expr::Name(ExprName {
                        id: "__file__".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    orelse: Box::new(Expr::NoneLiteral(ExprNoneLiteral {
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            }),
        ]
    }

    /// Register module in sys.modules
    fn create_sys_modules_registration(&self, synthetic_name: &str) -> Stmt {
        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Subscript(ruff_python_ast::ExprSubscript {
                value: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "sys".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("modules", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                slice: Box::new(self.create_string_literal(synthetic_name)),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                id: "module".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Register module in sys.modules with original name as alias
    fn create_sys_modules_registration_alias(
        &self,
        _synthetic_name: &str,
        original_name: &str,
    ) -> Stmt {
        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Subscript(ruff_python_ast::ExprSubscript {
                value: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "sys".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("modules", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                slice: Box::new(self.create_string_literal(original_name)),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                id: "module".into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create module attribute assignment
    fn create_module_attr_assignment(&self, module_var: &str, attr_name: &str) -> Stmt {
        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: module_var.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(attr_name, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
                id: attr_name.into(),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Extract simple assignment target
    fn extract_simple_assign_target(&self, assign: &StmtAssign) -> Option<String> {
        if assign.targets.len() == 1 {
            if let Expr::Name(name) = &assign.targets[0] {
                return Some(name.id.to_string());
            }
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
        if let Some(name) = self.extract_simple_assign_target(assign) {
            if self.should_export_symbol(&name, module_name) {
                body.push(self.create_module_attr_assignment("module", &name));
            }
        }
    }

    /// Generate a call to initialize a module
    fn generate_module_init_call(&self, synthetic_name: &str) -> Stmt {
        if let Some(init_func_name) = self.init_functions.get(synthetic_name) {
            Stmt::Expr(ruff_python_ast::StmtExpr {
                value: Box::new(Expr::Call(ExprCall {
                    func: Box::new(Expr::Name(ExprName {
                        id: init_func_name.as_str().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    arguments: ruff_python_ast::Arguments {
                        args: Box::from([]),
                        keywords: Box::from([]),
                        range: TextRange::default(),
                    },
                    range: TextRange::default(),
                })),
                range: TextRange::default(),
            })
        } else {
            Stmt::Pass(ruff_python_ast::StmtPass {
                range: TextRange::default(),
            })
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
                    name: Identifier::new(&import, TextRange::default()),
                    asname: None,
                    range: TextRange::default(),
                })
                .collect();

            final_body.push(Stmt::ImportFrom(StmtImportFrom {
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
                    name: Identifier::new(&name, TextRange::default()),
                    asname: None,
                    range: TextRange::default(),
                })
                .collect();

            final_body.push(Stmt::ImportFrom(StmtImportFrom {
                module: Some(Identifier::new(module_name, TextRange::default())),
                names: aliases,
                level: 0,
                range: TextRange::default(),
            }));
        }

        // Finally, regular import statements - deduplicated and sorted by module name
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
    }

    /// Collect imports from a module for hoisting
    fn collect_imports_from_module(&mut self, ast: &ModModule) {
        for stmt in &ast.body {
            match stmt {
                Stmt::ImportFrom(import_from) => {
                    self.collect_import_from(import_from, stmt);
                }
                Stmt::Import(import_stmt) => {
                    self.collect_import(import_stmt, stmt);
                }
                _ => {}
            }
        }
    }

    /// Collect ImportFrom statements
    fn collect_import_from(&mut self, import_from: &StmtImportFrom, _stmt: &Stmt) {
        let Some(ref module) = import_from.module else {
            return;
        };

        let module_name = module.as_str();
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
        }
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
            // Create canonical import statement
            unique_imports.push((
                module_name.to_string(),
                Stmt::Import(StmtImport {
                    names: vec![ruff_python_ast::Alias {
                        name: Identifier::new(module_name, TextRange::default()),
                        asname: None,
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
            log::debug!("Normalized import to canonical: import {}", module_name);
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

        log::debug!("Normalizing stdlib aliases: {:?}", alias_to_canonical);

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
                            "Rewriting global statement variable '{}' to '{}'",
                            name_str,
                            new_name
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
            targets: vec![Expr::Name(ExprName {
                id: original_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
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
                        log::debug!("Rewriting global variable '{}' to '{}'", name_str, new_name);
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
                log::debug!("Unhandled statement type in alias rewriting: {:?}", stmt);
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
                log::debug!(
                    "Rewriting alias '{}' to canonical '{}'",
                    name_str,
                    canonical
                );
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
                // Transform assignments to use lifted names if they're in a function with global declarations
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
                // Transform if this is a lifted global and we're in a function that declares it global
                if let Some(function_globals) = in_function_with_globals {
                    if function_globals.contains(name_expr.id.as_str()) {
                        if let Some(lifted_name) = lifted_names.get(name_expr.id.as_str()) {
                            name_expr.id = lifted_name.clone().into();
                        }
                    }
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
        log::info!(
            "collect_module_renames: Processing module '{}'",
            module_name
        );

        // Find the module ID for this module name
        let module_id = match semantic_ctx.graph.get_module_by_name(module_name) {
            Some(module) => module.module_id,
            None => {
                log::warn!("Module '{}' not found in graph", module_name);
                return;
            }
        };

        log::debug!("Module '{}' has ID: {:?}", module_name, module_id);

        // Get all renames for this module from semantic analysis
        let mut module_renames = FxIndexMap::default();

        // Use ModuleSemanticInfo to get ALL exported symbols from the module
        if let Some(module_info) = semantic_ctx.semantic_bundler.get_module_info(&module_id) {
            log::info!(
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
                        "Module '{}': symbol '{}' renamed to '{}'",
                        module_name,
                        symbol,
                        new_name
                    );
                } else {
                    // Include non-renamed symbols too - they still need to be in the namespace
                    module_renames.insert(symbol.to_string(), symbol.to_string());
                    log::debug!(
                        "Module '{}': symbol '{}' has no rename, using original name",
                        module_name,
                        symbol
                    );
                }
            }
        } else {
            log::warn!(
                "No semantic info found for module '{}' with ID {:?}",
                module_name,
                module_id
            );
        }

        if !module_renames.is_empty() {
            log::info!(
                "Inserting {} renames for module '{}' with key '{}': {:?}",
                module_renames.len(),
                module_name,
                module_name,
                module_renames.keys().collect::<Vec<_>>()
            );
            symbol_renames.insert(module_name.to_string(), module_renames);
        } else {
            log::info!("No renames to insert for module '{}'", module_name);
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

impl HybridStaticBundler {
    /// Collect Import statements
    fn collect_import(&mut self, import_stmt: &StmtImport, stmt: &Stmt) {
        for alias in &import_stmt.names {
            if self.is_safe_stdlib_module(alias.name.as_str()) {
                self.stdlib_import_statements.push(stmt.clone());
                break;
            }
        }
    }

    /// Add a regular stdlib import (e.g., "sys", "types")
    /// This creates an import statement and adds it to the tracked imports
    fn add_stdlib_import(&mut self, module_name: &str) {
        let import_stmt = Stmt::Import(StmtImport {
            names: vec![ruff_python_ast::Alias {
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
    /// Returns Some(vec) if __all__ is defined, None if not defined
    fn extract_all_exports(&self, ast: &ModModule) -> Option<Vec<String>> {
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
                return self.extract_string_list_from_expr(&assign.value);
            }
        }
        None
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

    /// Check if an assignment is self-referential (e.g., `x = x`)
    fn is_self_referential_assignment(&self, assign: &StmtAssign) -> bool {
        // Check if this is a simple assignment with a single target and value
        if assign.targets.len() == 1 {
            if let (Expr::Name(target), Expr::Name(value)) =
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
            targets: vec![Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: "module".into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new("__all__", TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::List(ExprList {
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
                }
                false
            }
            Stmt::Import(import_stmt) => {
                // Check if any of the imported modules are stdlib modules we've hoisted
                import_stmt.names.iter().any(|alias| {
                    self.is_safe_stdlib_module(alias.name.as_str())
                        && self.stdlib_import_statements.iter().any(|hoisted| {
                            matches!(hoisted, Stmt::Import(hoisted_import)
                                if hoisted_import.names.iter().any(|h| h.name == alias.name))
                        })
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
        let mut assignments = Vec::new();

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

            // Check if we're importing a submodule (e.g., from greetings import greeting)
            let full_module_path = format!("{}.{}", module_name, imported_name);
            let importing_submodule = self.bundled_modules.contains(&full_module_path)
                && self.module_registry.contains_key(&full_module_path);

            if importing_submodule {
                // We're importing a submodule, not an attribute
                // Create: target = sys.modules['module.submodule']
                log::debug!(
                    "Importing submodule '{}' from '{}' via from import",
                    imported_name,
                    module_name
                );
                assignments.push(Stmt::Assign(StmtAssign {
                    targets: vec![Expr::Name(ExprName {
                        id: target_name.as_str().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                        value: Box::new(Expr::Attribute(ExprAttribute {
                            value: Box::new(Expr::Name(ExprName {
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
                // Regular attribute import
                // Create: target = sys.modules['module'].imported_name
                assignments.push(Stmt::Assign(StmtAssign {
                    targets: vec![Expr::Name(ExprName {
                        id: target_name.as_str().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Attribute(ExprAttribute {
                        value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                            value: Box::new(Expr::Attribute(ExprAttribute {
                                value: Box::new(Expr::Name(ExprName {
                                    id: "sys".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new("modules", TextRange::default()),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            slice: Box::new(self.create_string_literal(module_name)),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new(imported_name, TextRange::default()),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));
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
        match stmt {
            Stmt::ImportFrom(import_from) => {
                self.rewrite_import_from(import_from, current_module, symbol_renames)
            }
            Stmt::Import(import_stmt) => self.rewrite_import(import_stmt),
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
                "Checking if '{}' is a module import: full_path='{}', importing_module={}",
                imported_name,
                full_module_path,
                importing_module
            );

            if importing_module {
                // Check if this module is actually a wrapper module (not inlined)
                if self.module_registry.contains_key(&full_module_path) {
                    // It's a wrapper module, create a sys.modules access instead
                    log::debug!(
                        "Module '{}' is a wrapper module, creating sys.modules access",
                        full_module_path
                    );
                    body.push(Stmt::Assign(StmtAssign {
                        targets: vec![Expr::Name(ExprName {
                            id: local_name.into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                            value: Box::new(Expr::Attribute(ExprAttribute {
                                value: Box::new(Expr::Name(ExprName {
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
            let alt_path = format!("{}.{}", without_prefix, imported_name);
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
            targets: vec![Expr::Name(ExprName {
                id: params.local_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                func: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("SimpleNamespace", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
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
            "add_symbols_to_namespace: local_name='{}', imported_name='{}', inlined_module_key='{}'",
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
                log::debug!("Trying without prefix: '{}'", without_prefix);
                symbol_renames.get(without_prefix)
            } else {
                None
            }
        }) {
            // Add each symbol from the module to the namespace
            for (original_name, renamed_name) in module_renames {
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
            targets: vec![Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: namespace_name.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(original_name, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Name(ExprName {
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
                targets: vec![Expr::Name(ExprName {
                    id: params.local_name.into(),
                    ctx: ExprContext::Store,
                    range: TextRange::default(),
                })],
                value: Box::new(Expr::Name(ExprName {
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
                // Check if this module is in the inlined_modules or module_registry to determine if it's a package
                let is_package = self
                    .bundled_modules
                    .iter()
                    .any(|m| m.starts_with(&format!("{}.", current_module)));

                if is_package {
                    // This is a package __init__ file - level 1 imports stay in the package
                    log::debug!(
                        "Module '{}' is a package, keeping parts for relative import",
                        current_module
                    );
                    // Keep parts as is
                } else {
                    // This is a root-level module - level 1 imports are siblings
                    log::debug!(
                        "Module '{}' is root-level, clearing parts for relative import",
                        current_module
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
                        .any(|m| m.starts_with(&format!("{}.", current_module)))
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
                    "Relative import resolution: current_module={}, is_package_init={}, level={}, levels_to_remove={}, parts={:?}",
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
                log::warn!(
                    "Potential circular import detected: {} importing itself",
                    current_module
                );
            }

            log::debug!("Resolved relative import to: {}", resolved);
            Some(resolved)
        } else {
            // Not a relative import
            let resolved = import_from.module.as_ref().map(|m| m.as_str().to_string());
            log::debug!("Not a relative import, resolved to: {:?}", resolved);
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
                log::info!("Module '{}' has function-scoped imports", module_name);
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

        // Check all modules for direct imports
        for (module_name, ast, module_path, _) in modules {
            log::debug!("Checking module '{}' for direct imports", module_name);
            let ctx = DirectImportContext {
                current_module: module_name,
                module_path,
                modules,
            };
            for stmt in &ast.body {
                self.collect_direct_imports(stmt, &ctx, &mut directly_imported);
            }
        }

        log::info!(
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
                        let full_module_path = format!("{}.{}", module_name, imported_name);

                        log::debug!(
                            "Checking if '{}' (from {} import {}) is a bundled module",
                            full_module_path,
                            module_name,
                            imported_name
                        );

                        // Check if this full path matches a bundled module
                        if ctx
                            .modules
                            .iter()
                            .any(|(name, _, _, _)| name == &full_module_path)
                        {
                            // This is importing a submodule directly
                            log::info!(
                                "Found direct submodule import: from {} import {} -> {}",
                                module_name,
                                imported_name,
                                full_module_path
                            );
                            directly_imported.insert(full_module_path);

                            // Note: We don't mark the parent module as directly imported here
                            // because `from models import base` is importing the submodule `base`,
                            // not the parent package `models` itself
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
            let full_module_path = format!("{}.{}", base_module, imported_name);

            log::debug!(
                "Checking if '{}' (from . import {}) is a bundled module",
                full_module_path,
                imported_name
            );

            // Check if this full path matches a bundled module
            let is_bundled_module = ctx
                .modules
                .iter()
                .any(|(name, _, _, _)| name == &full_module_path);

            if is_bundled_module {
                // This is importing a submodule directly
                log::info!(
                    "Found direct submodule import via relative import: from . import {} -> {}",
                    imported_name,
                    full_module_path
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
                log::info!(
                    "Marking parent package '{}' as directly imported (implicit import via '{}')",
                    parent,
                    imported_module
                );
                directly_imported.insert(parent.clone());
            }
        }
    }

    /// Find modules that are imported as namespaces (e.g., from models import base)
    /// Returns a map from module name to set of importing modules
    fn find_namespace_imported_modules(
        &mut self,
        modules: &[(String, ModModule, PathBuf, String)],
    ) {
        // Check all modules for namespace imports
        for (importing_module, ast, _, _) in modules {
            log::debug!(
                "Checking module '{}' for namespace imports",
                importing_module
            );
            for stmt in &ast.body {
                self.collect_namespace_imports(stmt, modules, importing_module);
            }
        }

        log::info!(
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
        if let Stmt::ImportFrom(import_from) = stmt {
            if let Some(module) = &import_from.module {
                let module_name = module.as_str();
                // Check if any imported name is actually a submodule
                for alias in &import_from.names {
                    let imported_name = alias.name.as_str();
                    let full_module_path = format!("{}.{}", module_name, imported_name);

                    log::debug!(
                        "Checking if '{}' (from {} import {}) is a bundled module in namespace import check",
                        full_module_path,
                        module_name,
                        imported_name
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
                        log::info!(
                            "Found namespace import: from {} import {} -> {} (actual: {}) in module {}",
                            module_name,
                            imported_name,
                            full_module_path,
                            actual_module_name,
                            importing_module
                        );
                        self.namespace_imported_modules
                            .entry(actual_module_name)
                            .or_default()
                            .insert(importing_module.to_string());
                    }
                }
            }
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
            let candidate = format!("{}_{}", base_name, i);
            if !existing_symbols.contains(&candidate) {
                return candidate;
            }
        }

        // Fallback with module prefix
        format!("__cribo_renamed_{}", base_name)
    }

    /// Get a unique name for a symbol, using the same pattern as generate_unique_name
    fn get_unique_name(&self, base_name: &str, existing_symbols: &FxIndexSet<String>) -> String {
        self.generate_unique_name(base_name, existing_symbols)
    }

    /// Inline a module without side effects directly into the bundle
    fn inline_module(
        &mut self,
        module_name: &str,
        ast: ModModule,
        module_path: &Path,
        ctx: &mut InlineContext,
    ) -> Result<Vec<Stmt>> {
        let mut module_renames = FxIndexMap::default();

        // Process each statement in the module
        for stmt in ast.body {
            match &stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // Track import aliases for resolution in assignments
                    if let Stmt::ImportFrom(import_from) = &stmt {
                        self.track_import_aliases(import_from, module_name, module_path, ctx);
                    }

                    // Skip imports - they should be handled separately
                    if !self.is_hoisted_import(&stmt) {
                        log::debug!(
                            "Skipping import in inlined module '{}': {:?}",
                            module_name,
                            stmt
                        );
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    let func_name = func_def.name.to_string();
                    if !self.should_inline_symbol(&func_name, module_name, ctx.module_exports_map) {
                        continue;
                    }

                    // Check if this symbol was renamed by semantic analysis
                    let renamed_name =
                        if let Some(module_rename_map) = ctx.module_renames.get(module_name) {
                            if let Some(new_name) = module_rename_map.get(&func_name) {
                                log::debug!(
                                    "Using semantic rename for '{}' to '{}' in module '{}'",
                                    func_name,
                                    new_name,
                                    module_name
                                );
                                new_name.clone()
                            } else {
                                func_name.clone()
                            }
                        } else {
                            func_name.clone()
                        };

                    if renamed_name != func_name {
                        module_renames.insert(func_name.clone(), renamed_name.clone());
                    }
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
                            "Unexpected expression statement in side-effect-free module '{}': {:?}",
                            module_name,
                            stmt
                        );
                    }
                }
                _ => {
                    // Any other statement type that we haven't explicitly handled
                    log::warn!(
                        "Unexpected statement type in side-effect-free module '{}': {:?}",
                        module_name,
                        stmt
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

    /// Inline module symbols with module-qualified names for namespace imports
    fn inline_module_for_namespace(
        &mut self,
        module_name: &str,
        ast: ModModule,
        module_path: &Path,
        ctx: &mut InlineContext,
    ) -> Result<Vec<Stmt>> {
        log::debug!("Inlining module '{}' for namespace import", module_name);
        let mut module_renames = FxIndexMap::default();

        // First pass: collect all symbol renames and track imports
        for stmt in &ast.body {
            match &stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // Track import aliases for resolution in the module
                    if let Stmt::ImportFrom(import_from) = &stmt {
                        self.track_import_aliases(import_from, module_name, module_path, ctx);
                    }
                    log::debug!(
                        "Tracked import in namespace hybrid module '{}': {:?}",
                        module_name,
                        stmt
                    );
                }
                Stmt::FunctionDef(func_def) => {
                    let func_name = func_def.name.to_string();
                    if !self.should_inline_symbol(&func_name, module_name, ctx.module_exports_map) {
                        continue;
                    }

                    let module_suffix = module_name.cow_replace('.', "_").into_owned();
                    let base_name = format!("{}_{}", func_name, module_suffix);
                    let renamed = self.get_unique_name(&base_name, ctx.global_symbols);

                    ctx.global_symbols.insert(renamed.clone());
                    module_renames.insert(func_name.clone(), renamed);
                }
                Stmt::ClassDef(class_def) => {
                    let class_name = class_def.name.to_string();
                    if !self.should_inline_symbol(&class_name, module_name, ctx.module_exports_map)
                    {
                        continue;
                    }

                    let module_suffix = module_name.cow_replace('.', "_").into_owned();
                    let base_name = format!("{}_{}", class_name, module_suffix);
                    let renamed = self.get_unique_name(&base_name, ctx.global_symbols);

                    ctx.global_symbols.insert(renamed.clone());
                    module_renames.insert(class_name.clone(), renamed);
                }
                Stmt::Assign(assign) => {
                    // Handle module-level assignments
                    if assign.targets.len() != 1 {
                        continue;
                    }

                    let Expr::Name(name_expr) = &assign.targets[0] else {
                        continue;
                    };

                    let var_name = name_expr.id.to_string();
                    if !self.should_inline_symbol(&var_name, module_name, ctx.module_exports_map) {
                        continue;
                    }

                    // Check for self-referential assignment and existing rename
                    let is_self_ref = matches!(&*assign.value, Expr::Name(value_name) if value_name.id.as_str() == var_name);
                    if is_self_ref && module_renames.contains_key(&var_name) {
                        let existing_renamed = module_renames.get(&var_name).expect(
                            "module_renames should contain var_name after contains_key check",
                        );
                        log::debug!(
                            "Handling self-referential assignment '{}' in namespace module '{}' -> '{}' = '{}'",
                            var_name,
                            module_name,
                            existing_renamed,
                            existing_renamed
                        );
                        // Skip this - it's redundant after renaming
                        continue;
                    }

                    // Generate module-qualified name
                    let module_suffix = module_name.cow_replace('.', "_").into_owned();
                    let base_name = format!("{}_{}", var_name, module_suffix);
                    let renamed = self.get_unique_name(&base_name, ctx.global_symbols);

                    log::debug!(
                        "Collecting rename for variable '{}' from namespace module '{}' as '{}'",
                        var_name,
                        module_name,
                        renamed
                    );

                    ctx.global_symbols.insert(renamed.clone());
                    module_renames.insert(var_name.clone(), renamed);
                }
                _ => {}
            }
        }

        // Second pass: process statements with all renames available
        for stmt in ast.body {
            match &stmt {
                Stmt::Import(_) | Stmt::ImportFrom(_) => {
                    // Skip imports
                }
                Stmt::FunctionDef(func_def) => {
                    let func_name = func_def.name.to_string();
                    if let Some(renamed) = module_renames.get(&func_name) {
                        log::debug!(
                            "Inlining function '{}' from namespace module '{}' as '{}'",
                            func_name,
                            module_name,
                            renamed
                        );

                        let mut renamed_func = func_def.clone();
                        renamed_func.name = Identifier::new(renamed, TextRange::default());

                        // Transform the function body to use renamed symbols and resolve imports
                        for body_stmt in &mut renamed_func.body {
                            self.resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
                        }
                        self.transform_function_body_for_renames(
                            &mut renamed_func,
                            &module_renames,
                        );

                        ctx.inlined_stmts.push(Stmt::FunctionDef(renamed_func));
                    }
                }
                Stmt::ClassDef(class_def) => {
                    let class_name = class_def.name.to_string();
                    if let Some(renamed) = module_renames.get(&class_name) {
                        log::debug!(
                            "Inlining class '{}' from namespace module '{}' as '{}'",
                            class_name,
                            module_name,
                            renamed
                        );

                        let mut renamed_class = class_def.clone();
                        renamed_class.name = Identifier::new(renamed, TextRange::default());

                        // Transform the class body to use renamed symbols and resolve imports
                        for body_stmt in &mut renamed_class.body {
                            self.resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
                            self.transform_stmt_for_renames(body_stmt, &module_renames);
                        }

                        ctx.inlined_stmts.push(Stmt::ClassDef(renamed_class));
                    }
                }
                Stmt::Assign(assign) => {
                    if assign.targets.len() != 1 {
                        continue;
                    }

                    let Expr::Name(name_expr) = &assign.targets[0] else {
                        continue;
                    };

                    let var_name = name_expr.id.to_string();
                    if !self.should_inline_symbol(&var_name, module_name, ctx.module_exports_map) {
                        continue;
                    }

                    // Skip if not in renames
                    let Some(renamed) = module_renames.get(&var_name) else {
                        continue;
                    };

                    // Skip self-referential assignments
                    let is_self_ref = matches!(&*assign.value, Expr::Name(value_name) if value_name.id.as_str() == var_name);
                    if is_self_ref {
                        continue;
                    }

                    log::debug!(
                        "Inlining variable '{}' from namespace module '{}' as '{}'",
                        var_name,
                        module_name,
                        renamed
                    );

                    let mut renamed_assign = assign.clone();
                    renamed_assign.targets = vec![Expr::Name(ExprName {
                        id: renamed.clone().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })];

                    // Transform the value expression to use renamed symbols and resolve imports
                    let mut value = (*assign.value).clone();
                    self.resolve_import_aliases_in_expr(&mut value, &ctx.import_aliases);
                    Self::rename_references_in_expr(&mut value, &module_renames);
                    renamed_assign.value = Box::new(value);

                    ctx.inlined_stmts.push(Stmt::Assign(renamed_assign));
                }
                _ => {
                    log::debug!(
                        "Skipping statement type in namespace module '{}' during second pass",
                        module_name
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
                    if let Expr::Name(name_expr) = target {
                        if let Some(renamed) = module_renames.get(name_expr.id.as_str()) {
                            name_expr.id = renamed.clone().into();
                        }
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
                        elements: InterpolatedStringElements::from(transformed_elements),
                        range: TextRange::default(),
                        flags: FStringFlags::empty(),
                    };

                    let new_value = FStringValue::single(new_fstring);

                    *expr = Expr::FString(ExprFString {
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
            let transformed_stmts = self.rewrite_import_in_stmt_multiple_with_context(
                stmt.clone(),
                ctx.module_name,
                &empty_renames,
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
        function_globals: &FxIndexSet<String>,
        lifted_names: &FxIndexMap<String, String>,
    ) -> Vec<Stmt> {
        let mut init_stmts = Vec::new();
        for global_name in function_globals {
            if let Some(lifted_name) = lifted_names.get(global_name) {
                // Add: local_var = __cribo_module_var at the beginning
                init_stmts.push(Stmt::Assign(StmtAssign {
                    targets: vec![Expr::Name(ExprName {
                        id: global_name.clone().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Name(ExprName {
                        id: lifted_name.clone().into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));
            }
        }
        init_stmts
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
                }
            }
        }

        // Replace function body with new body
        func_def.body = new_body;
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
                    elements: InterpolatedStringElements::from(transformed_elements),
                    range: TextRange::default(),
                    flags: FStringFlags::empty(),
                };

                // Create a new FStringValue containing our FString
                let new_value = FStringValue::single(new_fstring);

                // Replace the entire expression with the new f-string
                *expr = Expr::FString(ExprFString {
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
        let old_expr_str = format!("{:?}", new_expr);

        self.transform_expr_for_lifted_globals(
            &mut new_expr,
            lifted_names,
            global_info,
            in_function_with_globals,
        );

        let new_expr_str = format!("{:?}", new_expr);
        let was_transformed = old_expr_str != new_expr_str;

        // Create a new expression element with the transformed expression
        let new_element = InterpolatedElement {
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
        &self,
        import_from: &StmtImportFrom,
        module_name: &str,
        module_path: &Path,
        ctx: &mut InlineContext,
    ) {
        let resolved_module =
            self.resolve_relative_import_with_context(import_from, module_name, Some(module_path));
        if let Some(resolved) = resolved_module {
            // Track aliases for ALL imports, not just from inlined modules
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
                format!("{}:{}", resolved_module, imported_name)
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
        let exports = module_exports_map.get(module_name).and_then(|e| e.as_ref());

        if let Some(export_list) = exports {
            // Module has explicit __all__, only inline if exported
            export_list.contains(&symbol_name.to_string())
        } else {
            // No __all__, export non-private symbols
            !symbol_name.starts_with('_')
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
            let full_module_path = format!("{}.{}", module_name, imported_name);

            // Check if this is a module import
            let importing_module = self.inlined_modules.contains(&full_module_path)
                || self.bundled_modules.contains(&full_module_path);

            if importing_module {
                // Create a namespace object for the inlined module
                log::debug!(
                    "Creating namespace object for module '{}' imported from '{}' - module was inlined",
                    imported_name,
                    module_name
                );

                // Create a SimpleNamespace-like object
                // First, create the namespace: base = types.SimpleNamespace()
                assignments.push(Stmt::Assign(StmtAssign {
                    targets: vec![Expr::Name(ExprName {
                        id: local_name.as_str().into(),
                        ctx: ExprContext::Store,
                        range: TextRange::default(),
                    })],
                    value: Box::new(Expr::Call(ExprCall {
                        func: Box::new(Expr::Attribute(ExprAttribute {
                            value: Box::new(Expr::Name(ExprName {
                                id: "types".into(),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            attr: Identifier::new("SimpleNamespace", TextRange::default()),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        arguments: ruff_python_ast::Arguments {
                            args: Box::from([]),
                            keywords: Box::from([]),
                            range: TextRange::default(),
                        },
                        range: TextRange::default(),
                    })),
                    range: TextRange::default(),
                }));

                // Now add all symbols from the inlined module to the namespace
                // This should come from semantic analysis of what symbols the module exports
                if let Some(module_renames) = symbol_renames.get(&full_module_path) {
                    // Add each symbol from the module to the namespace
                    for (original_name, renamed_name) in module_renames {
                        // base.original_name = renamed_name
                        assignments.push(Stmt::Assign(StmtAssign {
                            targets: vec![Expr::Attribute(ExprAttribute {
                                value: Box::new(Expr::Name(ExprName {
                                    id: local_name.as_str().into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new(original_name, TextRange::default()),
                                ctx: ExprContext::Store,
                                range: TextRange::default(),
                            })],
                            value: Box::new(Expr::Name(ExprName {
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
                        "Creating assignment: {} = {} (from inlined module '{}')",
                        local_name,
                        actual_name,
                        module_name
                    );

                    let assignment = StmtAssign {
                        targets: vec![Expr::Name(ExprName {
                            id: local_name.as_str().into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Name(ExprName {
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
            return vec![Stmt::ImportFrom(import_from)];
        };

        log::debug!(
            "Checking if resolved module '{}' is in bundled modules: {:?}",
            module_name,
            self.bundled_modules.contains(&module_name)
        );

        if !self.bundled_modules.contains(&module_name) {
            log::debug!(
                "Module '{}' not found in bundled modules, checking if importing submodules",
                module_name
            );

            // Check if we're importing submodules from a namespace package
            // e.g., from greetings import greeting where greeting is actually greetings.greeting
            let mut has_bundled_submodules = false;
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let full_module_path = format!("{}.{}", module_name, imported_name);
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
            log::debug!(
                "Module '{}' has bundled submodules, transforming imports",
                module_name
            );
            // Transform each submodule import
            return self.transform_namespace_package_imports(import_from, &module_name);
        }

        log::debug!("Transforming bundled import from module: {}", module_name);

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
            self.transform_bundled_import_from_multiple(absolute_import, &module_name)
        } else {
            // Module was inlined - create assignments for imported symbols
            log::debug!(
                "Module '{}' was inlined, creating assignments for imported symbols",
                module_name
            );
            self.create_assignments_for_inlined_imports(import_from, &module_name, symbol_renames)
        }
    }

    /// Transform imports from a namespace package (package without __init__.py)
    fn transform_namespace_package_imports(
        &self,
        import_from: StmtImportFrom,
        module_name: &str,
    ) -> Vec<Stmt> {
        let mut result_stmts = Vec::new();

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            let full_module_path = format!("{}.{}", module_name, imported_name);

            if self.bundled_modules.contains(&full_module_path) {
                if self.module_registry.contains_key(&full_module_path) {
                    // Wrapper module - create sys.modules access
                    result_stmts.push(Stmt::Assign(StmtAssign {
                        targets: vec![Expr::Name(ExprName {
                            id: local_name.into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                            value: Box::new(Expr::Attribute(ExprAttribute {
                                value: Box::new(Expr::Name(ExprName {
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
                        "Submodule '{}' from namespace package '{}' was inlined, creating namespace",
                        imported_name,
                        module_name
                    );

                    // For namespace hybrid modules, we need to create the namespace object
                    // The inlined module's symbols are already renamed with module prefix
                    // e.g., message -> message_greetings_greeting
                    let inlined_key = full_module_path.cow_replace('.', "_").into_owned();

                    // Get the exported symbols from this module
                    let keywords = self.create_namespace_keywords(&full_module_path, &inlined_key);

                    // Create a types.SimpleNamespace with the renamed attributes
                    result_stmts.push(Stmt::Assign(StmtAssign {
                        targets: vec![Expr::Name(ExprName {
                            id: local_name.into(),
                            ctx: ExprContext::Store,
                            range: TextRange::default(),
                        })],
                        value: Box::new(Expr::Call(ExprCall {
                            func: Box::new(Expr::Attribute(ExprAttribute {
                                value: Box::new(Expr::Name(ExprName {
                                    id: "types".into(),
                                    ctx: ExprContext::Load,
                                    range: TextRange::default(),
                                })),
                                attr: Identifier::new("SimpleNamespace", TextRange::default()),
                                ctx: ExprContext::Load,
                                range: TextRange::default(),
                            })),
                            arguments: Arguments {
                                args: Box::new([]),
                                keywords: keywords.into_boxed_slice(),
                                range: TextRange::default(),
                            },
                            range: TextRange::default(),
                        })),
                        range: TextRange::default(),
                    }));
                }
            } else {
                // Not a bundled submodule, keep as attribute access
                // This might be importing a symbol from the namespace package's __init__.py
                // But since we're here, the namespace package has no __init__.py
                log::warn!(
                    "Import '{}' from namespace package '{}' is not a bundled module",
                    imported_name,
                    module_name
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

    /// Rewrite Import statements
    fn rewrite_import(&self, import_stmt: StmtImport) -> Vec<Stmt> {
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
                if self.bundled_modules.contains(module_name)
                    && self.module_registry.contains_key(module_name)
                {
                    // Create all parent namespaces if needed (e.g., for a.b.c.d, create a, a.b, a.b.c)
                    self.create_parent_namespaces(&parts, &mut result_stmts);

                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                    // If there's no alias, we need to handle the dotted name specially
                    if alias.asname.is_none() && module_name.contains('.') {
                        // Create assignments for each level of nesting
                        self.create_dotted_assignments(&parts, &mut result_stmts);
                    } else {
                        // For aliased imports or non-dotted imports, just assign to the target
                        result_stmts.push(
                            self.create_sys_modules_assignment(target_name.as_str(), module_name),
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
                    result_stmts.push(
                        self.create_sys_modules_assignment(target_name.as_str(), module_name),
                    );
                } else {
                    // Module was inlined - this is problematic for direct imports
                    // We need to create a mock module object
                    log::warn!(
                        "Direct import of inlined module '{}' detected - this pattern is not fully supported",
                        module_name
                    );
                    // For now, skip it
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
                    // Create all parent namespaces if needed (e.g., for a.b.c.d, create a, a.b, a.b.c)
                    self.create_parent_namespaces_entry_module(&parts, &mut result_stmts);

                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                    // If there's no alias, we need to handle the dotted name specially
                    if alias.asname.is_none() && module_name.contains('.') {
                        // Create assignments for each level of nesting
                        self.create_dotted_assignments(&parts, &mut result_stmts);
                    } else {
                        // For aliased imports or non-dotted imports, just assign to the target
                        result_stmts.push(
                            self.create_sys_modules_assignment(target_name.as_str(), module_name),
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
                    result_stmts.push(
                        self.create_sys_modules_assignment(target_name.as_str(), module_name),
                    );
                } else {
                    // Module was inlined - this is problematic for direct imports
                    // We need to create a mock module object
                    log::warn!(
                        "Direct import of inlined module '{}' detected - this pattern is not fully supported",
                        module_name
                    );
                    // For now, skip it
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

    /// Create parent namespaces for dotted imports in entry module
    fn create_parent_namespaces_entry_module(
        &mut self,
        parts: &[&str],
        result_stmts: &mut Vec<Stmt>,
    ) {
        for i in 1..parts.len() {
            let parent_path = parts[..i].join(".");

            if self.module_registry.contains_key(&parent_path) {
                // Parent is a wrapper module, get it from sys.modules
                result_stmts.push(self.create_sys_modules_assignment(&parent_path, &parent_path));
            } else if !self.bundled_modules.contains(&parent_path) {
                // Check if we haven't already created this namespace globally or in result_stmts
                let already_created = self.created_namespace_modules.contains(&parent_path)
                    || self.is_namespace_already_created(&parent_path, result_stmts);

                if !already_created {
                    // Parent is not a wrapper module and not an inlined module, create a simple namespace
                    result_stmts.push(self.create_namespace_module(&parent_path));
                    // Track that we created this namespace module
                    self.created_namespace_modules.insert(parent_path);
                }
            }
        }
    }

    /// Create a sys.modules assignment for an import
    fn create_sys_modules_assignment(&self, target_name: &str, module_name: &str) -> Stmt {
        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Name(ExprName {
                id: target_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                value: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "sys".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("modules", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                slice: Box::new(self.create_string_literal(module_name)),
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
                // Parent is a wrapper module, get it from sys.modules
                result_stmts.push(self.create_sys_modules_assignment(&parent_path, &parent_path));
            } else if !self.bundled_modules.contains(&parent_path) {
                // Check if we haven't already created this namespace in result_stmts
                let already_created = self.is_namespace_already_created(&parent_path, result_stmts);

                if !already_created {
                    // Parent is not a wrapper module and not an inlined module, create a simple namespace
                    result_stmts.push(self.create_namespace_module(&parent_path));
                }
            }
        }
    }

    /// Check if a namespace module was already created
    fn is_namespace_already_created(&self, parent_path: &str, result_stmts: &[Stmt]) -> bool {
        result_stmts.iter().any(|stmt| {
            if let Stmt::Assign(assign) = stmt {
                if let Some(Expr::Name(name)) = assign.targets.first() {
                    return name.id.as_str() == parent_path;
                }
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
            result_stmts.push(self.create_dotted_attribute_assignment(&parent, attr, &full_path));
        }
    }

    /// Create namespace keywords for a module
    fn create_namespace_keywords(&self, full_module_path: &str, inlined_key: &str) -> Vec<Keyword> {
        let mut keywords = Vec::new();
        if let Some(Some(exports)) = self.module_exports.get(full_module_path) {
            for symbol in exports {
                keywords.push(Keyword {
                    arg: Some(Identifier::new(symbol.as_str(), TextRange::default())),
                    value: Expr::Name(ExprName {
                        id: format!("{}_{}", symbol, inlined_key).into(),
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
    fn create_namespace_module(&self, module_name: &str) -> Stmt {
        // Create: module_name = types.ModuleType('module_name')
        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Name(ExprName {
                id: module_name.into(),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Call(ExprCall {
                func: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "types".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("ModuleType", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                arguments: ruff_python_ast::Arguments {
                    args: Box::from([self.create_string_literal(module_name)]),
                    keywords: Box::from([]),
                    range: TextRange::default(),
                },
                range: TextRange::default(),
            })),
            range: TextRange::default(),
        })
    }

    /// Create dotted attribute assignment (e.g., greetings.greeting = sys.modules['greetings.greeting'])
    fn create_dotted_attribute_assignment(
        &self,
        parent_module: &str,
        attr_name: &str,
        full_module_name: &str,
    ) -> Stmt {
        Stmt::Assign(StmtAssign {
            targets: vec![Expr::Attribute(ExprAttribute {
                value: Box::new(Expr::Name(ExprName {
                    id: parent_module.into(),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                attr: Identifier::new(attr_name, TextRange::default()),
                ctx: ExprContext::Store,
                range: TextRange::default(),
            })],
            value: Box::new(Expr::Subscript(ruff_python_ast::ExprSubscript {
                value: Box::new(Expr::Attribute(ExprAttribute {
                    value: Box::new(Expr::Name(ExprName {
                        id: "sys".into(),
                        ctx: ExprContext::Load,
                        range: TextRange::default(),
                    })),
                    attr: Identifier::new("modules", TextRange::default()),
                    ctx: ExprContext::Load,
                    range: TextRange::default(),
                })),
                slice: Box::new(self.create_string_literal(full_module_name)),
                ctx: ExprContext::Load,
                range: TextRange::default(),
            })),
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
                log::debug!(
                    "Using semantic rename for class '{}' to '{}' in module '{}'",
                    class_name,
                    new_name,
                    module_name
                );
                new_name.clone()
            } else {
                class_name.clone()
            }
        } else {
            class_name.clone()
        };

        if renamed_name != class_name {
            module_renames.insert(class_name.clone(), renamed_name.clone());
        }
        ctx.global_symbols.insert(renamed_name.clone());

        // Clone and rename the class
        let mut class_def_clone = class_def.clone();
        class_def_clone.name = Identifier::new(renamed_name, TextRange::default());

        // Apply renames to base classes
        // Apply renames and resolve import aliases in class body
        for body_stmt in &mut class_def_clone.body {
            self.resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
            self.rewrite_aliases_in_stmt(body_stmt, module_renames);
        }

        ctx.inlined_stmts.push(Stmt::ClassDef(class_def_clone));
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

        // Apply existing renames to the RHS value BEFORE creating new rename for LHS
        self.resolve_import_aliases_in_expr(&mut assign_clone.value, &ctx.import_aliases);
        self.rewrite_aliases_in_expr(&mut assign_clone.value, module_renames);

        // Now create a new rename for the LHS
        // Check if this symbol was renamed by semantic analysis
        let renamed_name = if let Some(module_rename_map) = ctx.module_renames.get(module_name) {
            if let Some(new_name) = module_rename_map.get(&name) {
                log::debug!(
                    "Using semantic rename for variable '{}' to '{}' in module '{}'",
                    name,
                    new_name,
                    module_name
                );
                new_name.clone()
            } else {
                name.clone()
            }
        } else {
            name.clone()
        };

        if renamed_name != name {
            module_renames.insert(name.clone(), renamed_name.clone());
        }
        ctx.global_symbols.insert(renamed_name.clone());

        // Apply the rename to the LHS
        if let Expr::Name(name_expr) = &mut assign_clone.targets[0] {
            name_expr.id = renamed_name.into();
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

        let renamed_name = self.get_unique_name(&var_name, ctx.global_symbols);
        if renamed_name != var_name {
            module_renames.insert(var_name.clone(), renamed_name.clone());
            log::debug!(
                "Renaming annotated variable '{}' to '{}' in module '{}'",
                var_name,
                renamed_name,
                module_name
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
}
