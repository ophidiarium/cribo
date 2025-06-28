use std::path::Path;

use cow_utils::CowUtils;
use indexmap::{IndexMap as FxIndexMap, IndexSet as FxIndexSet};
use ruff_python_ast::{
    Arguments, AtomicNodeIndex, ExceptHandler, Expr, ExprAttribute, ExprCall, ExprContext,
    ExprFString, ExprList, ExprName, ExprStringLiteral, FString, FStringFlags, FStringValue,
    Identifier, InterpolatedElement, InterpolatedStringElement, InterpolatedStringElements,
    ModModule, Stmt, StmtAssign, StmtImportFrom, StringLiteral, StringLiteralFlags,
    StringLiteralValue,
};
use ruff_text_size::TextRange;

use crate::code_generator::bundler::HybridStaticBundler;

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

    /// Get whether any importlib.import_module calls were transformed
    pub fn did_transform_importlib(&self) -> bool {
        self.importlib_transformed
    }

    /// Get whether any types.SimpleNamespace objects were created
    pub fn created_namespace_objects(&self) -> bool {
        self.created_namespace_objects
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
                return Some(
                    self.bundler
                        .create_module_access_expr(&resolved_name, self.symbol_renames),
                );
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
                self.bundler.is_hoisted_import(&stmts[i])
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
                                    let base_str = if let Expr::Name(name) = base {
                                        name.id.as_str().to_string()
                                    } else if let Expr::Attribute(attr) = base {
                                        if let Expr::Name(name) = &*attr.value {
                                            format!("{}.{}", name.id.as_str(), attr.attr.as_str())
                                        } else {
                                            String::new()
                                        }
                                    } else {
                                        String::new()
                                    };

                                    // Check if this specific base is a hard dependency
                                    let is_hard_dep_base =
                                        self.bundler.hard_dependencies.iter().any(|dep| {
                                            dep.module_name == self.module_name
                                                && dep.class_name == class_name
                                                && (dep.imported_attr == base_str
                                                    || dep
                                                        .base_class
                                                        .ends_with(&format!(".{base_str}")))
                                        });

                                    if !is_hard_dep_base {
                                        // Not a hard dependency base, transform normally
                                        self.transform_expr(base);
                                    } else {
                                        log::debug!(
                                            "Skipping transformation of hard dependency base \
                                             class {base_str} for class {class_name}"
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

                    self.bundler
                        .rewrite_import_with_renames(import_stmt.clone(), self.symbol_renames)
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
                        resolved,
                        &import_from.names,
                        self.symbol_renames,
                        self.deferred_imports,
                        self.is_entry_module,
                    );
                } else {
                    log::debug!("  Module '{resolved}' is inlined, handling import assignments");
                    // For the entry module, we should not defer these imports
                    // because they need to be available when the entry module's code runs
                    let import_stmts = self.bundler.handle_imports_from_inlined_module(
                        resolved,
                        &import_from.names,
                        self.symbol_renames,
                        self.deferred_imports,
                        self.is_entry_module,
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
        // TODO: Implement proper transformation when the method is available in bundler
        // The original code calls rewrite_import_in_stmt_multiple_with_full_context
        // which handles both Import and ImportFrom statements, but the current
        // bundler only has a method for Import statements.
        // For now, return the original import unchanged
        vec![Stmt::ImportFrom(import_from.clone())]
    }

    /// Transform an expression, rewriting module attribute access to direct references
    fn transform_expr(&mut self, expr: &mut Expr) {
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
