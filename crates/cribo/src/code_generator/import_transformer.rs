#![allow(clippy::excessive_nesting)]

use std::path::Path;

use cow_utils::CowUtils;
use ruff_python_ast::{
    AtomicNodeIndex, ExceptHandler, Expr, ExprAttribute, ExprCall, ExprContext, ExprFString,
    ExprName, FString, FStringFlags, FStringValue, Identifier, InterpolatedElement,
    InterpolatedStringElement, InterpolatedStringElements, Keyword, ModModule, Stmt, StmtImport,
    StmtImportFrom,
};
use ruff_text_size::TextRange;

use crate::{
    ast_builder::{expressions, statements},
    code_generator::{
        bundler::HybridStaticBundler, import_deduplicator,
        module_registry::sanitize_module_name_for_identifier,
    },
    types::{FxIndexMap, FxIndexSet},
};

/// Parameters for creating a RecursiveImportTransformer
#[derive(Debug)]
pub struct RecursiveImportTransformerParams<'a> {
    pub bundler: &'a HybridStaticBundler<'a>,
    pub module_name: &'a str,
    pub module_path: Option<&'a Path>,
    pub symbol_renames: &'a FxIndexMap<String, FxIndexMap<String, String>>,
    pub deferred_imports: &'a mut Vec<Stmt>,
    pub is_entry_module: bool,
    pub is_wrapper_init: bool,
    pub global_deferred_imports: Option<&'a FxIndexMap<(String, String), String>>,
}

/// Transformer that recursively handles import statements and module references
pub struct RecursiveImportTransformer<'a> {
    bundler: &'a HybridStaticBundler<'a>,
    module_name: &'a str,
    module_path: Option<&'a Path>,
    symbol_renames: &'a FxIndexMap<String, FxIndexMap<String, String>>,
    /// Maps import aliases to their actual module names
    /// e.g., "helper_utils" -> "utils.helpers"
    pub(crate) import_aliases: FxIndexMap<String, String>,
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
    /// Track if any importlib.import_module calls were transformed
    pub(crate) importlib_transformed: bool,
    /// Track variables that were assigned from importlib.import_module() of inlined modules
    /// Maps variable name to the inlined module name
    importlib_inlined_modules: FxIndexMap<String, String>,
    /// Track if we created any types.SimpleNamespace calls
    pub(crate) created_namespace_objects: bool,
    /// Track imports from wrapper modules that need to be rewritten
    /// Maps local name to (wrapper_module, original_name)
    wrapper_module_imports: FxIndexMap<String, (String, String)>,
}

impl<'a> RecursiveImportTransformer<'a> {
    /// Create a new transformer from parameters
    pub fn new(params: RecursiveImportTransformerParams<'a>) -> Self {
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
            importlib_transformed: false,
            importlib_inlined_modules: FxIndexMap::default(),
            created_namespace_objects: false,
            wrapper_module_imports: FxIndexMap::default(),
        }
    }

    /// Get whether any types.SimpleNamespace objects were created
    pub fn created_namespace_objects(&self) -> bool {
        self.created_namespace_objects
    }

    /// Extract base class name from an expression
    /// Returns None if the expression type is not supported
    fn extract_base_class_name(base: &Expr) -> Option<String> {
        match base {
            Expr::Name(name) => Some(name.id.as_str().to_string()),
            Expr::Attribute(attr) => {
                if let Expr::Name(name) = &*attr.value {
                    Some(format!("{}.{}", name.id.as_str(), attr.attr.as_str()))
                } else {
                    // Complex attribute chains not supported
                    None
                }
            }
            _ => None, // Other expression types not supported
        }
    }

    /// Check if this is an importlib.import_module() call
    fn is_importlib_import_module_call(&self, call: &ExprCall) -> bool {
        match &call.func.as_ref() {
            // Direct call: importlib.import_module()
            Expr::Attribute(attr) if attr.attr.as_str() == "import_module" => {
                match &attr.value.as_ref() {
                    Expr::Name(name) => {
                        let name_str = name.id.as_str();
                        // Check if it's 'importlib' directly or an alias that maps to 'importlib'
                        name_str == "importlib"
                            || self.import_aliases.get(name_str) == Some(&"importlib".to_string())
                    }
                    _ => false,
                }
            }
            // Function call: im() where im is import_module
            Expr::Name(name) => {
                // Check if this name is an alias for importlib.import_module
                self.import_aliases
                    .get(name.id.as_str())
                    .is_some_and(|module| module == "importlib.import_module")
            }
            _ => false,
        }
    }

    /// Transform importlib.import_module("module-name") to direct module reference
    fn transform_importlib_import_module(&mut self, call: &ExprCall) -> Option<Expr> {
        // Get the first argument which should be the module name
        if let Some(arg) = call.arguments.args.first()
            && let Expr::StringLiteral(lit) = arg
        {
            let module_name = lit.value.to_str();

            // Handle relative imports with package context
            let resolved_name = if module_name.starts_with('.') && call.arguments.args.len() >= 2 {
                // Get the package context from the second argument
                if let Expr::StringLiteral(package_lit) = &call.arguments.args[1] {
                    let package = package_lit.value.to_str();

                    // Resolve relative import
                    let level = module_name.chars().take_while(|&c| c == '.').count();
                    let name_part = module_name.trim_start_matches('.');

                    let mut package_parts: Vec<&str> = package.split('.').collect();

                    // Go up 'level - 1' levels (one dot means current package)
                    if level > 1 && package_parts.len() >= level - 1 {
                        package_parts.truncate(package_parts.len() - (level - 1));
                    }

                    // Append the name part if it's not empty
                    if !name_part.is_empty() {
                        package_parts.push(name_part);
                    }

                    package_parts.join(".")
                } else {
                    module_name.to_string()
                }
            } else {
                module_name.to_string()
            };

            // Check if this module was bundled
            if self.bundler.bundled_modules.contains(&resolved_name) {
                log::debug!(
                    "Transforming importlib.import_module('{module_name}') to module access \
                     '{resolved_name}'"
                );

                self.importlib_transformed = true;

                // Check if this creates a namespace object
                if self.bundler.inlined_modules.contains(&resolved_name) {
                    self.created_namespace_objects = true;
                }

                // Use common logic for module access
                return Some(self.create_module_access_expr(&resolved_name));
            }
        }
        None
    }

    /// Transform a module recursively, handling all imports at any depth
    pub(crate) fn transform_module(&mut self, module: &mut ModModule) {
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
            let is_import = matches!(&stmts[i], Stmt::Import(_) | Stmt::ImportFrom(_));
            let is_hoisted = if is_import {
                import_deduplicator::is_hoisted_import(self.bundler, &stmts[i])
            } else {
                false
            };

            if is_import {
                log::debug!(
                    "transform_statements: Found import in module '{}', is_hoisted={}",
                    self.module_name,
                    is_hoisted
                );
            }

            let needs_transformation = is_import && !is_hoisted;

            if needs_transformation {
                // Transform the import statement
                let transformed = self.transform_statement(&mut stmts[i]);

                log::debug!(
                    "transform_statements: Transforming import in module '{}', got {} statements \
                     back",
                    self.module_name,
                    transformed.len()
                );

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
                        // Check if this class has hard dependencies that should not be transformed
                        let class_name = class_def.name.as_str();
                        let has_hard_deps = self.bundler.hard_dependencies.iter().any(|dep| {
                            dep.module_name == self.module_name && dep.class_name == class_name
                        });

                        // Transform base classes only if there are no hard dependencies
                        if let Some(ref mut arguments) = class_def.arguments {
                            for base in &mut arguments.args {
                                if has_hard_deps {
                                    // For classes with hard dependencies, check if this base is a
                                    // hard dep
                                    let base_str =
                                        Self::extract_base_class_name(base).unwrap_or_default();

                                    // Check if this specific base is a hard dependency
                                    let is_hard_dep_base = if !base_str.is_empty() {
                                        self.bundler.hard_dependencies.iter().any(|dep| {
                                            dep.module_name == self.module_name
                                                && dep.class_name == class_name
                                                && (dep.imported_attr == base_str
                                                    || dep
                                                        .base_class
                                                        .ends_with(&format!(".{base_str}")))
                                        })
                                    } else {
                                        // If we can't extract the base class name, skip
                                        // transformation to be safe
                                        true
                                    };

                                    if !is_hard_dep_base {
                                        // Not a hard dependency base, transform normally
                                        self.transform_expr(base);
                                    } else {
                                        log::debug!(
                                            "Skipping transformation of hard dependency base \
                                             class {} for class {class_name}",
                                            if base_str.is_empty() {
                                                "<complex expression>"
                                            } else {
                                                &base_str
                                            }
                                        );
                                    }
                                } else {
                                    // No hard dependencies, transform normally
                                    self.transform_expr(base);
                                }
                            }
                        }
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
                        // First check if this is an assignment from importlib.import_module()
                        let mut importlib_module = None;
                        if let Expr::Call(call) = &assign.value.as_ref()
                            && self.is_importlib_import_module_call(call)
                        {
                            // Get the module name from the call
                            if let Some(arg) = call.arguments.args.first()
                                && let Expr::StringLiteral(lit) = arg
                            {
                                let module_name = lit.value.to_str();
                                // Only track if it's an inlined module (not a wrapper module)
                                if self.bundler.inlined_modules.contains(module_name) {
                                    importlib_module = Some(module_name.to_string());
                                }
                            }
                        }

                        // Track local variable assignments
                        for target in &assign.targets {
                            if let Expr::Name(name) = target {
                                let var_name = name.id.to_string();
                                self.local_variables.insert(var_name.clone());

                                // If this was assigned from importlib.import_module() of an inlined
                                // module, track it specially
                                if let Some(module) = &importlib_module {
                                    log::debug!(
                                        "Tracking importlib assignment: {var_name} = \
                                         importlib.import_module('{module}') [inlined module]"
                                    );
                                    self.importlib_inlined_modules
                                        .insert(var_name, module.clone());
                                }
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

    /// Transform a statement, potentially returning multiple statements
    fn transform_statement(&mut self, stmt: &mut Stmt) -> Vec<Stmt> {
        // Check if it's a hoisted import before matching
        let is_hoisted = import_deduplicator::is_hoisted_import(self.bundler, stmt);

        match stmt {
            Stmt::Import(import_stmt) => {
                log::debug!(
                    "RecursiveImportTransformer::transform_statement: Found Import statement"
                );
                if is_hoisted {
                    vec![stmt.clone()]
                } else {
                    // Track import aliases before rewriting
                    for alias in &import_stmt.names {
                        let module_name = alias.name.as_str();
                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                        // Track if it's an aliased import of an inlined module (but not in entry
                        // module)
                        if !self.is_entry_module
                            && alias.asname.is_some()
                            && self.bundler.inlined_modules.contains(module_name)
                        {
                            log::debug!("Tracking import alias: {local_name} -> {module_name}");
                            self.import_aliases
                                .insert(local_name.to_string(), module_name.to_string());
                        }
                        // Also track importlib aliases for static import resolution (in any module)
                        else if module_name == "importlib" && alias.asname.is_some() {
                            log::debug!("Tracking importlib alias: {local_name} -> importlib");
                            self.import_aliases
                                .insert(local_name.to_string(), "importlib".to_string());
                        }
                    }

                    let result = rewrite_import_with_renames(
                        self.bundler,
                        import_stmt.clone(),
                        self.symbol_renames,
                    );
                    log::debug!(
                        "rewrite_import_with_renames for module '{}': import {:?} -> {} statements",
                        self.module_name,
                        import_stmt
                            .names
                            .iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>(),
                        result.len()
                    );
                    result
                }
            }
            Stmt::ImportFrom(import_from) => {
                log::debug!(
                    "RecursiveImportTransformer::transform_statement: Found ImportFrom statement \
                     (is_hoisted: {is_hoisted})"
                );
                // Track import aliases before handling the import (even for hoisted imports)
                if let Some(module) = &import_from.module {
                    let module_str = module.as_str();
                    log::debug!(
                        "Processing ImportFrom in RecursiveImportTransformer: from {} import {:?} \
                         (is_entry_module: {})",
                        module_str,
                        import_from
                            .names
                            .iter()
                            .map(|a| format!(
                                "{}{}",
                                a.name.as_str(),
                                a.asname
                                    .as_ref()
                                    .map(|n| format!(" as {n}"))
                                    .unwrap_or_default()
                            ))
                            .collect::<Vec<_>>(),
                        self.is_entry_module
                    );

                    // Special handling for importlib imports
                    if module_str == "importlib" {
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str();
                            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                            if imported_name == "import_module" {
                                log::debug!(
                                    "Tracking importlib.import_module alias: {local_name} -> \
                                     importlib.import_module"
                                );
                                self.import_aliases.insert(
                                    local_name.to_string(),
                                    "importlib.import_module".to_string(),
                                );
                            }
                        }
                    }

                    // Resolve relative imports first
                    let resolved_module = resolve_relative_import_with_context(
                        import_from,
                        self.module_name,
                        self.module_path,
                        self.bundler.entry_path.as_deref(),
                        &self.bundler.bundled_modules,
                    );

                    if let Some(resolved) = &resolved_module {
                        // Track aliases for imported symbols (non-importlib)
                        if resolved != "importlib" {
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
                                    } else if !self.is_entry_module {
                                        // This is importing a submodule as a name (inlined module)
                                        // Don't track in entry module - namespace objects are
                                        // created instead
                                        log::debug!(
                                            "Tracking module import alias: {local_name} -> \
                                             {full_module_path}"
                                        );
                                        self.import_aliases
                                            .insert(local_name.to_string(), full_module_path);
                                    } else {
                                        log::debug!(
                                            "Not tracking module import as alias in entry module: \
                                             {local_name} -> {full_module_path} (namespace object)"
                                        );
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
                }

                // Now handle the import based on whether it's hoisted
                if is_hoisted {
                    vec![stmt.clone()]
                } else {
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
        let resolved_module = resolve_relative_import_with_context(
            import_from,
            self.module_name,
            self.module_path,
            self.bundler.entry_path.as_deref(),
            &self.bundler.bundled_modules,
        );

        log::debug!(
            "handle_import_from: resolved_module={:?}, is_wrapper_init={}, current_module={}",
            resolved_module,
            self.is_wrapper_init,
            self.module_name
        );

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
                        let namespace_var = sanitize_module_name_for_identifier(&full_module_path);
                        log::debug!(
                            "  Creating namespace assignment: {local_name} = {namespace_var}"
                        );
                        result_stmts.push(statements::simple_assign(
                            local_name,
                            expressions::name(&namespace_var, ExprContext::Load),
                        ));
                        handled_any = true;
                    } else {
                        // This is importing an inlined submodule
                        // We need to handle this specially when the current module is being inlined
                        // (i.e., not the entry module and not a wrapper module)
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
                                let types_simple_namespace_call = expressions::call(
                                    expressions::simple_namespace_ctor(),
                                    vec![],
                                    vec![],
                                );
                                self.deferred_imports.push(statements::simple_assign(
                                    local_name,
                                    types_simple_namespace_call,
                                ));
                                self.created_namespace_objects = true;

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
                                            &full_module_path,
                                            &exports,
                                        );

                                    // Add __all__ attribute to the namespace with filtered exports
                                    // BUT ONLY if the original module had an explicit __all__
                                    if !filtered_exports.is_empty()
                                        && self
                                            .bundler
                                            .modules_with_explicit_all
                                            .contains(&full_module_path)
                                    {
                                        let export_strings: Vec<&str> =
                                            filtered_exports.iter().map(|s| s.as_str()).collect();
                                        self.deferred_imports.push(statements::set_list_attribute(
                                            local_name,
                                            "__all__",
                                            &export_strings,
                                        ));
                                    }

                                    for symbol in filtered_exports {
                                        // local_name.symbol = symbol
                                        let target = expressions::attribute(
                                            expressions::name(local_name, ExprContext::Load),
                                            &symbol,
                                            ExprContext::Store,
                                        );
                                        let symbol_name = self
                                            .symbol_renames
                                            .get(&full_module_path)
                                            .and_then(|renames| renames.get(&symbol))
                                            .cloned()
                                            .unwrap_or_else(|| symbol.clone());
                                        let value =
                                            expressions::name(&symbol_name, ExprContext::Load);
                                        self.deferred_imports
                                            .push(statements::assign(vec![target], value));
                                    }
                                }
                            } else {
                                // For wrapper modules importing inlined modules, we need to create
                                // the namespace immediately since it's used in the module body
                                log::debug!(
                                    "  Creating immediate namespace for wrapper module import"
                                );

                                // Create: local_name = types.SimpleNamespace()
                                result_stmts.push(statements::simple_assign(
                                    local_name,
                                    expressions::call(
                                        expressions::simple_namespace_ctor(),
                                        vec![],
                                        vec![],
                                    ),
                                ));
                                self.created_namespace_objects = true;

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
                                            &full_module_path,
                                            &exports,
                                        );

                                    // Add __all__ attribute to the namespace with filtered exports
                                    // BUT ONLY if the original module had an explicit __all__
                                    if !filtered_exports.is_empty()
                                        && self
                                            .bundler
                                            .modules_with_explicit_all
                                            .contains(&full_module_path)
                                    {
                                        let export_strings: Vec<&str> =
                                            filtered_exports.iter().map(|s| s.as_str()).collect();
                                        result_stmts.push(statements::set_list_attribute(
                                            local_name,
                                            "__all__",
                                            &export_strings,
                                        ));
                                    }

                                    for symbol in filtered_exports {
                                        // local_name.symbol = symbol
                                        let target = expressions::attribute(
                                            expressions::name(local_name, ExprContext::Load),
                                            &symbol,
                                            ExprContext::Store,
                                        );
                                        let symbol_name = self
                                            .symbol_renames
                                            .get(&full_module_path)
                                            .and_then(|renames| renames.get(&symbol))
                                            .cloned()
                                            .unwrap_or_else(|| symbol.clone());
                                        let value =
                                            expressions::name(&symbol_name, ExprContext::Load);
                                        result_stmts.push(statements::assign(vec![target], value));
                                    }
                                }
                            }

                            handled_any = true;
                        } else if !self.is_entry_module {
                            // This is a wrapper module importing an inlined module
                            log::debug!(
                                "  Deferring inlined submodule import in wrapper module: \
                                 {local_name} -> {full_module_path}"
                            );
                        } else {
                            // For entry module, create namespace object immediately

                            // Create the namespace object with symbols
                            // This mimics what happens in non-entry modules

                            // First create the empty namespace
                            result_stmts.push(statements::simple_assign(
                                local_name,
                                expressions::call(
                                    expressions::simple_namespace_ctor(),
                                    vec![],
                                    vec![],
                                ),
                            ));

                            // Track this as a local variable, not an import alias
                            self.local_variables.insert(local_name.to_string());

                            handled_any = true;
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
                log::debug!("  Statements: {result_stmts:?}");
                // We've already handled the import completely, don't fall through to other handling
                return result_stmts;
            }
        }

        if let Some(ref resolved) = resolved_module {
            // Check if this is an inlined module
            if self.bundler.inlined_modules.contains(resolved) {
                // Check if this is a circular module with pre-declarations
                if self.bundler.circular_modules.contains(resolved) {
                    log::debug!("  Module '{resolved}' is a circular module with pre-declarations");
                    // Special handling for imports between circular inlined modules
                    // If the current module is also a circular inlined module, we need to defer or
                    // transform differently
                    if self.bundler.circular_modules.contains(self.module_name)
                        && self.bundler.inlined_modules.contains(self.module_name)
                    {
                        log::debug!(
                            "  Both modules are circular and inlined - transforming to direct \
                             assignments"
                        );
                        // Generate direct assignments since both modules will be in the same scope
                        let mut assignments = Vec::new();
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str();
                            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                            // Check if the symbol was renamed during bundling
                            let actual_name =
                                if let Some(renames) = self.symbol_renames.get(resolved) {
                                    renames
                                        .get(imported_name)
                                        .map(|s| s.as_str())
                                        .unwrap_or(imported_name)
                                } else {
                                    imported_name
                                };

                            // Create assignment: local_name = actual_name
                            if local_name != actual_name {
                                assignments.push(statements::simple_assign(
                                    local_name,
                                    expressions::name(actual_name, ExprContext::Load),
                                ));
                            }
                        }
                        return assignments;
                    } else {
                        // Original behavior for non-circular modules importing from circular
                        // modules
                        return self.bundler.handle_imports_from_inlined_module(
                            import_from,
                            resolved,
                            self.symbol_renames,
                        );
                    }
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
                        if !import_stmts.is_empty() {
                            return import_stmts;
                        }
                        // If handle_imports_from_inlined_module returned empty (e.g., for submodule
                        // imports), fall through to check if we need to
                        // handle it differently
                        log::debug!(
                            "  handle_imports_from_inlined_module returned empty for entry \
                             module, checking for submodule imports"
                        );
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

                // When an inlined module imports from a wrapper module, we need to
                // track the imports and rewrite all usages within the module
                if !self.is_entry_module && current_module_is_inlined {
                    log::debug!(
                        "  Tracking wrapper module imports for rewriting in module '{}' (inlined: \
                         {})",
                        self.module_name,
                        current_module_is_inlined
                    );

                    // Track each imported symbol for rewriting
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                        // Store mapping: local_name -> (wrapper_module, imported_name)
                        self.wrapper_module_imports.insert(
                            local_name.to_string(),
                            (resolved.to_string(), imported_name.to_string()),
                        );

                        log::debug!(
                            "    Tracking import: {local_name} -> {resolved}.{imported_name}"
                        );
                    }

                    // Return empty - we'll rewrite all usages instead of creating imports
                    return vec![];
                }
                // For wrapper modules importing from other wrapper modules,
                // let it fall through to standard transformation
            }
        }

        // Otherwise, use standard transformation
        let empty_renames = FxIndexMap::default();
        rewrite_import_from(
            self.bundler,
            import_from.clone(),
            self.module_name,
            &empty_renames,
            self.is_wrapper_init,
        )
    }

    /// Transform an expression, rewriting module attribute access to direct references
    fn transform_expr(&mut self, expr: &mut Expr) {
        // First check if this is an attribute expression and collect the path
        let attribute_info = if matches!(expr, Expr::Attribute(_)) {
            let info = self.collect_attribute_path(expr);
            log::debug!(
                "transform_expr: Found attribute expression - base: {:?}, path: {:?}, \
                 is_entry_module: {}",
                info.0,
                info.1,
                self.is_entry_module
            );

            Some(info)
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

                        // First check if the base is a variable assigned from
                        // importlib.import_module()
                        if let Some(module_name) = self.importlib_inlined_modules.get(&base) {
                            // This is accessing attributes on a variable that was assigned from
                            // importlib.import_module() of an inlined module
                            if attr_path.len() == 1 {
                                let attr_name = &attr_path[0];
                                log::debug!(
                                    "Transforming {base}.{attr_name} - {base} was assigned from \
                                     importlib.import_module('{module_name}') [inlined module]"
                                );

                                // Check if this symbol was renamed during inlining
                                let new_expr = if let Some(module_renames) =
                                    self.symbol_renames.get(module_name)
                                {
                                    if let Some(renamed) = module_renames.get(attr_name) {
                                        // Use the renamed symbol
                                        let renamed_str = renamed.clone();
                                        log::debug!(
                                            "Rewrote {base}.{attr_name} to {renamed_str} (renamed \
                                             symbol from importlib inlined module)"
                                        );
                                        Expr::Name(ExprName {
                                            node_index: AtomicNodeIndex::dummy(),
                                            id: renamed_str.into(),
                                            ctx: attr_expr.ctx,
                                            range: attr_expr.range,
                                        })
                                    } else {
                                        // Use the original name
                                        log::debug!(
                                            "Rewrote {base}.{attr_name} to {attr_name} (symbol \
                                             from importlib inlined module)"
                                        );
                                        Expr::Name(ExprName {
                                            node_index: AtomicNodeIndex::dummy(),
                                            id: attr_name.into(),
                                            ctx: attr_expr.ctx,
                                            range: attr_expr.range,
                                        })
                                    }
                                } else {
                                    // Module wasn't found in renames, use original
                                    log::debug!(
                                        "Rewrote {base}.{attr_name} to {attr_name} (no renames \
                                         for importlib inlined module)"
                                    );
                                    Expr::Name(ExprName {
                                        node_index: AtomicNodeIndex::dummy(),
                                        id: attr_name.into(),
                                        ctx: attr_expr.ctx,
                                        range: attr_expr.range,
                                    })
                                };
                                *expr = new_expr;
                                return;
                            }
                        }
                        // Check if the base refers to an inlined module
                        else if let Some(actual_module) = self.find_module_for_alias(&base)
                            && self.bundler.inlined_modules.contains(&actual_module)
                        {
                            log::debug!(
                                "Found module alias: {base} -> {actual_module} (is_entry_module: \
                                 {})",
                                self.is_entry_module
                            );

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
                // Check if this is importlib.import_module() with a static string literal
                if self.is_importlib_import_module_call(call_expr)
                    && let Some(transformed) = self.transform_importlib_import_module(call_expr)
                {
                    *expr = transformed;
                    return;
                }

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
            // Check if Name expressions need to be rewritten for wrapper module imports
            Expr::Name(name_expr) => {
                let name = name_expr.id.as_str();

                // Check if this name was imported from a wrapper module and needs rewriting
                if let Some((wrapper_module, imported_name)) = self.wrapper_module_imports.get(name)
                {
                    log::debug!("Rewriting name '{name}' to '{wrapper_module}.{imported_name}'");

                    // Create wrapper_module.imported_name attribute access
                    *expr = Expr::Attribute(ExprAttribute {
                        node_index: AtomicNodeIndex::dummy(),
                        value: Box::new(Expr::Name(ExprName {
                            node_index: AtomicNodeIndex::dummy(),
                            id: wrapper_module.into(),
                            ctx: ExprContext::Load,
                            range: TextRange::default(),
                        })),
                        attr: Identifier::new(imported_name, TextRange::default()),
                        ctx: name_expr.ctx,
                        range: name_expr.range,
                    });
                }
            }
            // Constants, etc. don't need transformation
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
        log::debug!(
            "find_module_for_alias: alias={}, is_entry_module={}, local_vars={:?}",
            alias,
            self.is_entry_module,
            self.local_variables.contains(alias)
        );

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
            None
        }
    }

    /// Create module access expression
    pub fn create_module_access_expr(&self, module_name: &str) -> Expr {
        // Check if this is a wrapper module
        if let Some(synthetic_name) = self.bundler.module_registry.get(module_name) {
            // This is a wrapper module - we need to call its init function
            // This handles modules with invalid Python identifiers like "my-module"
            let init_func_name =
                crate::code_generator::module_registry::get_init_function_name(synthetic_name);

            // Create init function call
            expressions::call(
                expressions::name(&init_func_name, ExprContext::Load),
                vec![],
                vec![],
            )
        } else if self.bundler.inlined_modules.contains(module_name) {
            // This is an inlined module - create namespace object
            self.create_namespace_call_for_inlined_module(
                module_name,
                self.symbol_renames.get(module_name),
            )
        } else {
            // This module wasn't bundled - shouldn't happen for static imports
            log::warn!("Module '{module_name}' referenced in static import but not bundled");
            expressions::none_literal()
        }
    }

    /// Create a namespace call expression for an inlined module
    fn create_namespace_call_for_inlined_module(
        &self,
        module_name: &str,
        module_renames: Option<&FxIndexMap<String, String>>,
    ) -> Expr {
        // Create a types.SimpleNamespace with all the module's symbols
        let mut keywords = Vec::new();
        let mut seen_args = FxIndexSet::default();

        // Add all renamed symbols as keyword arguments, avoiding duplicates
        if let Some(renames) = module_renames {
            for (original_name, renamed_name) in renames {
                // Check if the renamed name was already added
                if seen_args.contains(renamed_name) {
                    log::debug!(
                        "Skipping duplicate namespace argument '{renamed_name}' (from \
                         '{original_name}') for module '{module_name}'"
                    );
                    continue;
                }

                // Check if this symbol survived tree-shaking
                if let Some(ref kept_symbols) = self.bundler.tree_shaking_keep_symbols
                    && !kept_symbols.contains(&(module_name.to_string(), original_name.clone()))
                {
                    log::debug!(
                        "Skipping tree-shaken symbol '{original_name}' from namespace for module \
                         '{module_name}'"
                    );
                    continue;
                }

                seen_args.insert(renamed_name.clone());

                keywords.push(Keyword {
                    node_index: AtomicNodeIndex::dummy(),
                    arg: Some(Identifier::new(original_name, TextRange::default())),
                    value: expressions::name(renamed_name, ExprContext::Load),
                    range: TextRange::default(),
                });
            }
        }

        // Also check if module has module-level variables that weren't renamed
        if let Some(exports) = self.bundler.module_exports.get(module_name)
            && let Some(export_list) = exports
        {
            for export in export_list {
                // Check if this export was already added as a renamed symbol
                let was_renamed =
                    module_renames.is_some_and(|renames| renames.contains_key(export));
                if !was_renamed && !seen_args.contains(export) {
                    // Check if this symbol survived tree-shaking
                    if let Some(ref kept_symbols) = self.bundler.tree_shaking_keep_symbols
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
                        value: expressions::name(export, ExprContext::Load),
                        range: TextRange::default(),
                    });
                }
            }
        }

        // Create types.SimpleNamespace(**kwargs) call
        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords)
    }
}

/// Rewrite import with renames
fn rewrite_import_with_renames(
    bundler: &HybridStaticBundler,
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
            if bundler.bundled_modules.contains(module_name) {
                if bundler.module_registry.contains_key(module_name) {
                    // Create all parent namespaces if needed (e.g., for a.b.c.d, create a, a.b,
                    // a.b.c)
                    bundler.create_parent_namespaces(&parts, &mut result_stmts);

                    // Initialize the module at import time
                    result_stmts
                        .extend(bundler.create_module_initialization_for_import(module_name));

                    let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                    // If there's no alias, we need to handle the dotted name specially
                    if alias.asname.is_none() {
                        // Create assignments for each level of nesting
                        bundler.create_dotted_assignments(&parts, &mut result_stmts);
                    } else {
                        // For aliased imports or non-dotted imports, just assign to the target
                        // Skip self-assignments - the module is already initialized
                        if target_name.as_str() != module_name {
                            result_stmts.push(bundler.create_module_reference_assignment(
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
                        bundler.create_all_namespace_objects(&parts, &mut result_stmts);

                        // Populate ALL namespace levels with their symbols, not just the leaf
                        // For "import greetings.greeting", populate both "greetings" and
                        // "greetings.greeting"
                        for i in 1..=parts.len() {
                            let partial_module = parts[..i].join(".");
                            // Only populate if this module was actually bundled and has exports
                            if bundler.bundled_modules.contains(&partial_module) {
                                bundler.populate_namespace_with_module_symbols_with_renames(
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

                        // Check if namespace already exists
                        if !bundler.created_namespaces.contains(target_name.as_str()) {
                            let namespace_stmt = bundler.create_namespace_object_for_module(
                                target_name.as_str(),
                                module_name,
                            );
                            result_stmts.push(namespace_stmt);
                        } else {
                            log::debug!(
                                "Skipping namespace creation for '{}' - already created globally",
                                target_name.as_str()
                            );
                        }

                        // Always populate the namespace with symbols
                        bundler.populate_namespace_with_module_symbols_with_renames(
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
            if !bundler.bundled_modules.contains(module_name) {
                handled_all = false;
                continue;
            }

            if bundler.module_registry.contains_key(module_name) {
                // Module uses wrapper approach - need to initialize it now
                let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                // First, ensure the module is initialized
                result_stmts.extend(bundler.create_module_initialization_for_import(module_name));

                // Then create assignment if needed (skip self-assignments)
                if target_name.as_str() != module_name {
                    result_stmts.push(
                        bundler
                            .create_module_reference_assignment(target_name.as_str(), module_name),
                    );
                }
            } else {
                // Module was inlined - create a namespace object
                let target_name = alias.asname.as_ref().unwrap_or(&alias.name);

                // Create namespace object with the module's exports
                // Check if namespace already exists
                if !bundler.created_namespaces.contains(target_name.as_str()) {
                    let namespace_stmt = bundler
                        .create_namespace_object_for_module(target_name.as_str(), module_name);
                    result_stmts.push(namespace_stmt);
                } else {
                    log::debug!(
                        "Skipping namespace creation for '{}' - already created globally",
                        target_name.as_str()
                    );
                }

                // Always populate the namespace with symbols
                bundler.populate_namespace_with_module_symbols_with_renames(
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

/// Check if an import statement is importing bundled submodules
fn has_bundled_submodules(
    import_from: &StmtImportFrom,
    module_name: &str,
    bundler: &HybridStaticBundler,
) -> bool {
    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let full_module_path = format!("{module_name}.{imported_name}");
        log::trace!("  Checking if '{full_module_path}' is in bundled_modules");
        if bundler.bundled_modules.contains(&full_module_path) {
            log::trace!("    -> YES, it's bundled");
            return true;
        } else {
            log::trace!("    -> NO, not bundled");
        }
    }
    false
}

/// Rewrite import from statement with proper handling for bundled modules
fn rewrite_import_from(
    bundler: &HybridStaticBundler,
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
    log::debug!(
        "  Importing names: {:?}",
        import_from
            .names
            .iter()
            .map(|a| (a.name.as_str(), a.asname.as_ref().map(|n| n.as_str())))
            .collect::<Vec<_>>()
    );
    log::trace!("  bundled_modules size: {}", bundler.bundled_modules.len());
    log::trace!("  inlined_modules size: {}", bundler.inlined_modules.len());
    let resolved_module_name = resolve_relative_import_with_context(
        &import_from,
        current_module,
        None,
        bundler.entry_path.as_deref(),
        &bundler.bundled_modules,
    );

    let Some(module_name) = resolved_module_name else {
        // If we can't resolve the module, return the original import
        log::warn!(
            "Could not resolve module name for import {:?}, keeping original import",
            import_from.module.as_ref().map(|m| m.as_str())
        );
        return vec![Stmt::ImportFrom(import_from)];
    };

    if !bundler.bundled_modules.contains(&module_name) {
        log::trace!(
            "  bundled_modules contains: {:?}",
            bundler.bundled_modules.iter().collect::<Vec<_>>()
        );
        log::debug!(
            "Module '{module_name}' not found in bundled modules, checking if inlined or \
             importing submodules"
        );

        // First check if we're importing bundled submodules from a namespace package
        // This check MUST come before the inlined module check
        // e.g., from greetings import greeting where greeting is actually greetings.greeting
        if has_bundled_submodules(&import_from, &module_name, bundler) {
            // We have bundled submodules, need to transform them
            log::debug!("Module '{module_name}' has bundled submodules, transforming imports");
            log::debug!("  Found bundled submodules:");
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                let full_module_path = format!("{module_name}.{imported_name}");
                if bundler.bundled_modules.contains(&full_module_path) {
                    log::debug!("    - {full_module_path}");
                }
            }
            // Transform each submodule import
            return bundler.transform_namespace_package_imports(
                import_from,
                &module_name,
                symbol_renames,
            );
        }

        // Check if this module is inlined
        if bundler.inlined_modules.contains(&module_name) {
            log::debug!(
                "Module '{module_name}' is an inlined module, \
                 inside_wrapper_init={inside_wrapper_init}"
            );
            // Handle imports from inlined modules
            return bundler.handle_imports_from_inlined_module(
                &import_from,
                &module_name,
                symbol_renames,
            );
        }

        // Check if this module is in the module_registry (wrapper module)
        if bundler.module_registry.contains_key(&module_name) {
            log::debug!("Module '{module_name}' is a wrapper module in module_registry");
            // This is a wrapper module, we need to transform it
            return bundler.transform_bundled_import_from_multiple_with_current_module(
                import_from,
                &module_name,
                inside_wrapper_init,
                Some(current_module),
            );
        }

        // No bundled submodules, keep original import
        // For relative imports from non-bundled modules, convert to absolute import
        if import_from.level > 0 {
            let mut absolute_import = import_from.clone();
            absolute_import.level = 0;
            absolute_import.module = Some(Identifier::new(&module_name, TextRange::default()));
            return vec![Stmt::ImportFrom(absolute_import)];
        }
        return vec![Stmt::ImportFrom(import_from)];
    }

    log::debug!(
        "Transforming bundled import from module: {module_name}, is wrapper: {}",
        bundler.module_registry.contains_key(&module_name)
    );

    // Check if this module is in the registry (wrapper approach)
    // or if it was inlined
    if bundler.module_registry.contains_key(&module_name) {
        // Module uses wrapper approach - transform to sys.modules access
        // For relative imports, we need to create an absolute import
        let mut absolute_import = import_from.clone();
        if import_from.level > 0 {
            // Convert relative import to absolute
            absolute_import.level = 0;
            absolute_import.module = Some(Identifier::new(&module_name, TextRange::default()));
        }
        bundler.transform_bundled_import_from_multiple_with_current_module(
            absolute_import,
            &module_name,
            inside_wrapper_init,
            Some(current_module),
        )
    } else {
        // Module was inlined - but first check if we're importing bundled submodules
        // e.g., from my_package import utils where my_package.utils is a bundled module
        if has_bundled_submodules(&import_from, &module_name, bundler) {
            log::debug!(
                "Inlined module '{module_name}' has bundled submodules, using \
                 transform_namespace_package_imports"
            );
            // Use namespace package imports for bundled submodules
            return bundler.transform_namespace_package_imports(
                import_from,
                &module_name,
                symbol_renames,
            );
        }

        // Module was inlined - create assignments for imported symbols
        log::debug!(
            "Module '{module_name}' was inlined, creating assignments for imported symbols"
        );
        #[allow(clippy::too_many_arguments)]
        crate::code_generator::module_registry::create_assignments_for_inlined_imports(
            import_from,
            &module_name,
            symbol_renames,
            &bundler.module_registry,
            &bundler.inlined_modules,
            &bundler.bundled_modules,
            |local_name, full_module_path| {
                bundler.create_namespace_with_name(local_name, full_module_path)
            },
        )
    }
}

/// Resolve a relative import with context
///
/// This function resolves relative imports (e.g., `from . import foo` or `from ..bar import baz`)
/// to absolute module names based on the current module and its file path.
///
/// # Arguments
/// * `import_from` - The import statement to resolve
/// * `current_module` - The name of the module containing the import
/// * `module_path` - Optional path to the module file (used to detect __init__.py)
/// * `entry_path` - Optional entry point path for the bundle
/// * `bundled_modules` - Set of all bundled module names
///
/// # Returns
/// The resolved absolute module name, or None if the import cannot be resolved
pub fn resolve_relative_import_with_context(
    import_from: &StmtImportFrom,
    current_module: &str,
    module_path: Option<&Path>,
    entry_path: Option<&str>,
    bundled_modules: &FxIndexSet<String>,
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

            // Check if current module is a package __init__.py file
            let is_package_init = if let Some(path) = module_path {
                path.file_name()
                    .and_then(|f| f.to_str())
                    .map(|f| f == "__init__.py")
                    .unwrap_or(false)
            } else {
                false
            };

            // Check if this module is the entry module and is __init__.py
            let is_entry_init = current_module
                == entry_path
                    .map(Path::new)
                    .and_then(Path::file_stem)
                    .and_then(std::ffi::OsStr::to_str)
                    .unwrap_or("")
                && is_package_init;

            if is_entry_init {
                // This is the entry __init__.py - relative imports should resolve within the
                // package but without the package prefix
                log::debug!(
                    "Module '{current_module}' is the entry __init__.py, clearing parts for \
                     relative import"
                );
                parts.clear();
            } else {
                // Check if this module is in the bundled_modules to
                // determine if it's a package
                let is_package = bundled_modules
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
                bundled_modules
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

        // Handle the case where relative import resolves to empty or just the package itself
        // This happens with "from . import something" in a package __init__.py
        if resolved.is_empty() {
            // For "from . import X" in a package, the resolved module is the current package
            // We need to check if we're in a package __init__.py
            if import_from.level == 1 && import_from.module.is_none() {
                // This is "from . import X" - we need to determine the parent package
                // For a module like "requests.utils", the parent is "requests"
                // For a module like "__init__", it's the current directory
                if current_module.contains('.') {
                    // Module has a parent package - extract it
                    let parent_parts: Vec<&str> = current_module.split('.').collect();
                    let parent = parent_parts[..parent_parts.len() - 1].join(".");
                    log::debug!(
                        "Relative import 'from . import' in module '{current_module}' - returning \
                         parent package '{parent}'"
                    );
                    return Some(parent);
                } else if current_module == "__init__" {
                    // This is a package __init__.py doing "from . import X"
                    // The package name should be derived from the directory
                    log::debug!(
                        "Relative import 'from . import' in __init__ module - this case needs \
                         special handling"
                    );
                    // For now, we'll return None and let it be handled elsewhere
                    return None;
                } else {
                    // Single-level module doing "from . import X" - this is importing from the
                    // same directory We need to return empty string to
                    // indicate current directory
                    log::debug!(
                        "Relative import 'from . import' in root-level module '{current_module}' \
                         - returning empty for current directory"
                    );
                    return Some(String::new());
                }
            }
            log::debug!("Invalid relative import - resolved to empty module");
            return None;
        }

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
