#![allow(clippy::excessive_nesting)]

use std::path::Path;

use cow_utils::CowUtils;
use ruff_python_ast::{
    AtomicNodeIndex, ExceptHandler, Expr, ExprCall, ExprContext, ExprFString, ExprName, FString,
    FStringValue, Identifier, InterpolatedElement, InterpolatedStringElement,
    InterpolatedStringElements, ModModule, Stmt, StmtImport, StmtImportFrom,
};
use ruff_text_size::TextRange;

use crate::{
    analyzers::symbol_analyzer::SymbolAnalyzer,
    ast_builder::{expressions, statements},
    code_generator::{
        bundler::Bundler, import_deduplicator, module_registry::sanitize_module_name_for_identifier,
    },
    types::{FxIndexMap, FxIndexSet},
};

/// Parameters for creating a `RecursiveImportTransformer`
#[derive(Debug)]
pub struct RecursiveImportTransformerParams<'a> {
    pub bundler: &'a Bundler<'a>,
    pub module_name: &'a str,
    pub module_path: Option<&'a Path>,
    pub symbol_renames: &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    pub deferred_imports: &'a mut Vec<Stmt>,
    pub is_entry_module: bool,
    pub is_wrapper_init: bool,
    pub global_deferred_imports:
        Option<&'a FxIndexMap<(crate::resolver::ModuleId, String), crate::resolver::ModuleId>>,
    pub python_version: u8,
}

/// Transformer that recursively handles import statements and module references
pub struct RecursiveImportTransformer<'a> {
    bundler: &'a Bundler<'a>,
    module_name: &'a str,
    module_path: Option<&'a Path>,
    symbol_renames: &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    /// Maps import aliases to their actual module names
    /// e.g., "`helper_utils`" -> "utils.helpers"
    pub(crate) import_aliases: FxIndexMap<String, String>,
    /// Deferred import assignments for cross-module imports
    deferred_imports: &'a mut Vec<Stmt>,
    /// Flag indicating if this is the entry module
    is_entry_module: bool,
    /// Flag indicating if we're inside a wrapper module's init function
    is_wrapper_init: bool,
    /// Reference to global deferred imports registry
    global_deferred_imports:
        Option<&'a FxIndexMap<(crate::resolver::ModuleId, String), crate::resolver::ModuleId>>,
    /// Track local variable assignments to avoid treating them as module aliases
    local_variables: FxIndexSet<String>,
    /// Track if any `importlib.import_module` calls were transformed
    pub(crate) importlib_transformed: bool,
    /// Track variables that were assigned from `importlib.import_module()` of inlined modules
    /// Maps variable name to the inlined module name
    importlib_inlined_modules: FxIndexMap<String, String>,
    /// Track if we created any types.SimpleNamespace calls
    pub(crate) created_namespace_objects: bool,
    /// Track imports from wrapper modules that need to be rewritten
    /// Maps local name to (`wrapper_module`, `original_name`)
    wrapper_module_imports: FxIndexMap<String, (String, String)>,
    /// Track which modules have already been populated with symbols in this transformation session
    /// This prevents duplicate namespace assignments when multiple imports reference the same
    /// module
    populated_modules: FxIndexSet<String>,
    /// Track which stdlib modules were actually imported in this module
    /// This prevents transforming references to stdlib modules that weren't imported
    imported_stdlib_modules: FxIndexSet<String>,
    /// Python version for compatibility checks
    python_version: u8,
}

impl<'a> RecursiveImportTransformer<'a> {
    /// Check if a condition is a `TYPE_CHECKING` check
    fn is_type_checking_condition(expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
            Expr::Attribute(attr) => {
                attr.attr.as_str() == "TYPE_CHECKING"
                    && match &*attr.value {
                        // Check for both typing.TYPE_CHECKING and _cribo.typing.TYPE_CHECKING
                        Expr::Name(name) => name.id.as_str() == "typing",
                        Expr::Attribute(inner_attr) => {
                            // Handle _cribo.typing.TYPE_CHECKING
                            inner_attr.attr.as_str() == "typing"
                                && matches!(&*inner_attr.value, Expr::Name(name) if name.id.as_str() == crate::ast_builder::CRIBO_PREFIX)
                        }
                        _ => false,
                    }
            }
            _ => false,
        }
    }

    /// Create a new transformer from parameters
    #[allow(clippy::needless_pass_by_value)] // params contains mutable references
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
            populated_modules: FxIndexSet::default(),
            imported_stdlib_modules: FxIndexSet::default(),
            python_version: params.python_version,
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

    /// Check if this is an `importlib.import_module()` call
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

                    // Resolve package to path, then use resolver
                    if let Ok(Some(package_path)) =
                        self.bundler.resolver.resolve_module_path(package)
                    {
                        let level = module_name.chars().take_while(|&c| c == '.').count() as u32;
                        let name_part = module_name.trim_start_matches('.');

                        self.bundler
                            .resolver
                            .resolve_relative_to_absolute_module_name(
                                level,
                                if name_part.is_empty() {
                                    None
                                } else {
                                    Some(name_part)
                                },
                                &package_path,
                            )
                            .unwrap_or_else(|| module_name.to_string())
                    } else {
                        // Use resolver's method for package name resolution when path not found
                        let level = module_name.chars().take_while(|&c| c == '.').count() as u32;
                        let name_part = module_name.trim_start_matches('.');

                        self.bundler
                            .resolver
                            .resolve_relative_import_from_package_name(
                                level,
                                if name_part.is_empty() {
                                    None
                                } else {
                                    Some(name_part)
                                },
                                package,
                            )
                    }
                } else {
                    module_name.to_string()
                }
            } else {
                module_name.to_string()
            };

            // Check if this module was bundled
            if self
                .bundler
                .get_module_id(&resolved_name)
                .is_some_and(|id| self.bundler.bundled_modules.contains(&id))
            {
                log::debug!(
                    "Transforming importlib.import_module('{module_name}') to module access \
                     '{resolved_name}'"
                );

                self.importlib_transformed = true;

                // Check if this creates a namespace object
                if self
                    .bundler
                    .get_module_id(&resolved_name)
                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                {
                    self.created_namespace_objects = true;
                }

                // Use common logic for module access
                return Some(self.create_module_access_expr(&resolved_name));
            }
        }
        None
    }

    /// Check if this is a stdlib import that should be normalized
    fn should_normalize_stdlib_import(&self, module_name: &str) -> bool {
        // Recognize full stdlib module paths and submodules for the current Python version
        crate::resolver::is_stdlib_module(module_name, self.python_version)
    }

    /// Build a mapping of stdlib imports to their rewritten paths
    /// This mapping is used during expression rewriting
    fn build_stdlib_rename_map(
        &self,
        imports: &[(String, Option<String>)],
    ) -> FxIndexMap<String, String> {
        let mut rename_map = FxIndexMap::default();

        for (module_name, alias) in imports {
            let local_name = alias.as_ref().unwrap_or(module_name);
            let rewritten_path = Bundler::get_rewritten_stdlib_path(module_name);
            rename_map.insert(local_name.clone(), rewritten_path);
        }

        rename_map
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

                        // Transform decorators
                        for decorator in &mut func_def.decorator_list {
                            self.transform_expr(&mut decorator.expression);
                        }

                        // Transform parameter annotations and default values
                        for param in &mut func_def.parameters.posonlyargs {
                            if let Some(annotation) = &mut param.parameter.annotation {
                                self.transform_expr(annotation);
                            }
                            if let Some(default) = &mut param.default {
                                self.transform_expr(default);
                            }
                        }
                        for param in &mut func_def.parameters.args {
                            if let Some(annotation) = &mut param.parameter.annotation {
                                self.transform_expr(annotation);
                            }
                            if let Some(default) = &mut param.default {
                                self.transform_expr(default);
                            }
                        }
                        if let Some(vararg) = &mut func_def.parameters.vararg
                            && let Some(annotation) = &mut vararg.annotation
                        {
                            self.transform_expr(annotation);
                        }
                        for param in &mut func_def.parameters.kwonlyargs {
                            if let Some(annotation) = &mut param.parameter.annotation {
                                self.transform_expr(annotation);
                            }
                            if let Some(default) = &mut param.default {
                                self.transform_expr(default);
                            }
                        }
                        if let Some(kwarg) = &mut func_def.parameters.kwarg
                            && let Some(annotation) = &mut kwarg.annotation
                        {
                            self.transform_expr(annotation);
                        }

                        // Transform return type annotation
                        if let Some(returns) = &mut func_def.returns {
                            self.transform_expr(returns);
                        }

                        // Save current local variables and create a new scope for the function
                        let saved_locals = self.local_variables.clone();

                        // Track function parameters as local variables before transforming the body
                        // This prevents incorrect transformation of parameter names that shadow stdlib modules
                        for param in &func_def.parameters.args {
                            self.local_variables
                                .insert(param.parameter.name.as_str().to_string());
                            log::debug!(
                                "Tracking function parameter as local: {}",
                                param.parameter.name.as_str()
                            );
                        }

                        // Transform the function body
                        self.transform_statements(&mut func_def.body);

                        // Restore the previous scope's local variables
                        self.local_variables = saved_locals;
                    }
                    Stmt::ClassDef(class_def) => {
                        // Transform decorators
                        for decorator in &mut class_def.decorator_list {
                            self.transform_expr(&mut decorator.expression);
                        }

                        // Check if this class has hard dependencies that should not be transformed
                        let class_name = class_def.name.as_str();

                        // Pre-filter hard dependencies for this specific class to avoid repeated
                        // scans
                        let class_hard_deps: Vec<_> = self
                            .bundler
                            .hard_dependencies
                            .iter()
                            .filter(|dep| {
                                dep.module_name == self.module_name && dep.class_name == class_name
                            })
                            .collect();

                        let has_hard_deps = !class_hard_deps.is_empty();

                        // Transform base classes only if there are no hard dependencies
                        if let Some(ref mut arguments) = class_def.arguments {
                            for base in &mut arguments.args {
                                if has_hard_deps {
                                    // For classes with hard dependencies, check if this base is a
                                    // hard dep
                                    let base_str =
                                        Self::extract_base_class_name(base).unwrap_or_default();

                                    // Closure to check if a dependency base matches
                                    let base_matches_dep = |dep: &&crate::code_generator::context::HardDependency| -> bool {
                                        dep.base_class == base_str
                                            || base_str.starts_with(&format!("{}.", dep.imported_attr))
                                            || dep.imported_attr == base_str
                                    };

                                    // Check if this specific base is a hard dependency
                                    let is_hard_dep_base = if base_str.is_empty() {
                                        // If we can't extract the base class name, skip
                                        // transformation to be safe
                                        true
                                    } else {
                                        class_hard_deps.iter().any(base_matches_dep)
                                    };

                                    if is_hard_dep_base {
                                        // Check if this specific hard dependency is from a stdlib
                                        // module
                                        // If so, still transform it since stdlib normalization
                                        // handles it
                                        let is_from_stdlib = if base_str.is_empty() {
                                            // For complex/unknown base expressions, don't attempt
                                            // transformation
                                            false
                                        } else {
                                            class_hard_deps.iter().any(|dep| {
                                                base_matches_dep(dep)
                                                    && crate::resolver::is_stdlib_module(
                                                        &dep.source_module,
                                                        self.python_version,
                                                    )
                                            })
                                        };

                                        if is_from_stdlib {
                                            log::debug!(
                                                "Transforming stdlib hard dependency base class \
                                                 {} for class {class_name} - stdlib normalization \
                                                 will handle it",
                                                if base_str.is_empty() {
                                                    "<complex expression>"
                                                } else {
                                                    &base_str
                                                }
                                            );
                                        } else {
                                            // Even if it's not from stdlib, we still need to
                                            // transform it
                                            // in case it's a wrapper module import that needs
                                            // rewriting
                                            log::debug!(
                                                "Transforming hard dependency base class {} for \
                                                 class {class_name} - checking for wrapper module \
                                                 imports",
                                                if base_str.is_empty() {
                                                    "<complex expression>"
                                                } else {
                                                    &base_str
                                                }
                                            );
                                        }
                                        // Transform the base expression (common to both branches)
                                        self.transform_expr(base);
                                    } else {
                                        // Not a hard dependency base, transform normally
                                        self.transform_expr(base);
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

                        // Check if this is a TYPE_CHECKING block and ensure it has a body
                        if if_stmt.body.is_empty()
                            && Self::is_type_checking_condition(&if_stmt.test)
                        {
                            log::debug!(
                                "Adding pass statement to empty TYPE_CHECKING block in import transformer"
                            );
                            if_stmt.body.push(crate::ast_builder::statements::pass());
                        }

                        for clause in &mut if_stmt.elif_else_clauses {
                            if let Some(test_expr) = &mut clause.test {
                                self.transform_expr(test_expr);
                            }
                            self.transform_statements(&mut clause.body);

                            // Ensure non-empty body for elif/else clauses too
                            if clause.body.is_empty() {
                                log::debug!(
                                    "Adding pass statement to empty elif/else clause in import transformer"
                                );
                                clause.body.push(crate::ast_builder::statements::pass());
                            }
                        }
                    }
                    Stmt::While(while_stmt) => {
                        self.transform_expr(&mut while_stmt.test);
                        self.transform_statements(&mut while_stmt.body);
                        self.transform_statements(&mut while_stmt.orelse);
                    }
                    Stmt::For(for_stmt) => {
                        // Track loop variable as local before transforming to prevent incorrect stdlib transformations
                        if let Expr::Name(name) = for_stmt.target.as_ref() {
                            self.local_variables.insert(name.id.as_str().to_string());
                            log::debug!(
                                "Tracking for loop variable as local: {}",
                                name.id.as_str()
                            );
                        }

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

                        // Ensure try body is not empty
                        if try_stmt.body.is_empty() {
                            log::debug!(
                                "Adding pass statement to empty try body in import transformer"
                            );
                            try_stmt.body.push(crate::ast_builder::statements::pass());
                        }

                        for handler in &mut try_stmt.handlers {
                            let ExceptHandler::ExceptHandler(eh) = handler;
                            self.transform_statements(&mut eh.body);

                            // Ensure exception handler body is not empty
                            if eh.body.is_empty() {
                                log::debug!(
                                    "Adding pass statement to empty except handler in import transformer"
                                );
                                eh.body.push(crate::ast_builder::statements::pass());
                            }
                        }
                        self.transform_statements(&mut try_stmt.orelse);
                        self.transform_statements(&mut try_stmt.finalbody);
                    }
                    Stmt::AnnAssign(ann_assign) => {
                        // Transform the annotation
                        self.transform_expr(&mut ann_assign.annotation);

                        // Transform the target
                        self.transform_expr(&mut ann_assign.target);

                        // Transform the value if present
                        if let Some(value) = &mut ann_assign.value {
                            self.transform_expr(value);
                        }
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
                                if self
                                    .bundler
                                    .get_module_id(module_name)
                                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                                {
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
                    // Check if this is a stdlib import that should be normalized
                    let mut stdlib_imports = Vec::new();
                    let mut non_stdlib_imports = Vec::new();

                    for alias in &import_stmt.names {
                        let module_name = alias.name.as_str();

                        // Normalize ALL stdlib imports, including those with aliases
                        if self.should_normalize_stdlib_import(module_name) {
                            // Track that this stdlib module was imported
                            self.imported_stdlib_modules.insert(module_name.to_string());
                            // Also track parent modules for dotted imports (e.g., collections.abc imports collections too)
                            if let Some(dot_pos) = module_name.find('.') {
                                let parent = &module_name[..dot_pos];
                                self.imported_stdlib_modules.insert(parent.to_string());
                            }
                            stdlib_imports.push((
                                module_name.to_string(),
                                alias.asname.as_ref().map(|n| n.as_str().to_string()),
                            ));
                        } else {
                            non_stdlib_imports.push(alias.clone());
                        }
                    }

                    // Handle stdlib imports
                    if !stdlib_imports.is_empty() {
                        // Build rename map for expression rewriting
                        let rename_map = self.build_stdlib_rename_map(&stdlib_imports);

                        // Track these renames for expression rewriting
                        for (local_name, rewritten_path) in rename_map {
                            self.import_aliases.insert(local_name, rewritten_path);
                        }

                        // If we're in a wrapper module, create local assignments for stdlib imports
                        if self.is_wrapper_init {
                            let mut assignments = Vec::new();

                            for (module_name, alias) in &stdlib_imports {
                                // Determine the local name that the import creates
                                let local_name = if let Some(alias_name) = alias {
                                    // Aliased import: "import json as j" creates local "j"
                                    alias_name.clone()
                                } else if module_name.contains('.') {
                                    // Dotted import without alias: "import collections.abc" doesn't create a binding
                                    // Skip these as they don't create local variables
                                    continue;
                                } else {
                                    // Simple import: "import json" creates local "json"
                                    module_name.clone()
                                };

                                let proxy_path =
                                    format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX);
                                let proxy_parts: Vec<&str> = proxy_path.split('.').collect();
                                let value_expr = crate::ast_builder::expressions::dotted_name(
                                    &proxy_parts,
                                    ExprContext::Load,
                                );
                                let target = crate::ast_builder::expressions::name(
                                    local_name.as_str(),
                                    ExprContext::Store,
                                );
                                let assign_stmt = crate::ast_builder::statements::assign(
                                    vec![target],
                                    value_expr,
                                );
                                assignments.push(assign_stmt);

                                // Note: The module_transformer will handle adding these to the
                                // module namespace based on the stdlib_reexports mechanism
                            }

                            // If there are non-stdlib imports, keep them and add assignments
                            if !non_stdlib_imports.is_empty() {
                                let new_import = StmtImport {
                                    names: non_stdlib_imports,
                                    ..import_stmt.clone()
                                };
                                assignments.insert(0, Stmt::Import(new_import));
                            }

                            return assignments;
                        }
                    }

                    // If all imports were stdlib, we need to handle aliased imports
                    if non_stdlib_imports.is_empty() {
                        // Create local assignments for aliased stdlib imports
                        let mut assignments = Vec::new();
                        for (module_name, alias) in &stdlib_imports {
                            if let Some(alias_name) = alias {
                                // Aliased import creates a local binding
                                let proxy_path =
                                    format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX);
                                let proxy_parts: Vec<&str> = proxy_path.split('.').collect();
                                let value_expr = crate::ast_builder::expressions::dotted_name(
                                    &proxy_parts,
                                    ExprContext::Load,
                                );
                                let target = crate::ast_builder::expressions::name(
                                    alias_name.as_str(),
                                    ExprContext::Store,
                                );
                                let assign_stmt = crate::ast_builder::statements::assign(
                                    vec![target],
                                    value_expr,
                                );
                                assignments.push(assign_stmt);

                                // Track the alias for import_module resolution
                                if module_name == "importlib" {
                                    log::debug!(
                                        "Tracking importlib alias: {alias_name} -> importlib"
                                    );
                                    self.import_aliases
                                        .insert(alias_name.clone(), "importlib".to_string());
                                }
                            }
                        }
                        return assignments;
                    }

                    // Otherwise, create a new import with only non-stdlib imports
                    let new_import = StmtImport {
                        names: non_stdlib_imports,
                        ..import_stmt.clone()
                    };

                    // Track import aliases before rewriting
                    for alias in &new_import.names {
                        let module_name = alias.name.as_str();
                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                        // Track if it's an aliased import of an inlined module (but not in entry
                        // module)
                        if !self.is_entry_module
                            && alias.asname.is_some()
                            && self
                                .bundler
                                .get_module_id(module_name)
                                .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
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
                        new_import.clone(),
                        self.symbol_renames,
                        &mut self.populated_modules,
                    );

                    // Track any aliases created by the import to prevent incorrect stdlib transformations
                    for alias in &new_import.names {
                        if let Some(asname) = &alias.asname {
                            let local_name = asname.as_str();
                            self.local_variables.insert(local_name.to_string());
                            log::debug!(
                                "Tracking import alias as local variable: {} (from {})",
                                local_name,
                                alias.name.as_str()
                            );
                        }
                    }

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
                    let resolved_module = if import_from.level > 0 {
                        self.module_path.and_then(|path| {
                            self.bundler
                                .resolver
                                .resolve_relative_to_absolute_module_name(
                                    import_from.level,
                                    import_from
                                        .module
                                        .as_ref()
                                        .map(ruff_python_ast::Identifier::as_str),
                                    path,
                                )
                        })
                    } else {
                        import_from
                            .module
                            .as_ref()
                            .map(std::string::ToString::to_string)
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
                                if let Some(module_id) =
                                    self.bundler.get_module_id(&full_module_path)
                                {
                                    if self.bundler.inlined_modules.contains(&module_id) {
                                        // Check if this is a namespace-imported module
                                        if self
                                            .bundler
                                            .namespace_imported_modules
                                            .contains_key(&module_id)
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
                                    }
                                } else if self
                                    .bundler
                                    .get_module_id(resolved)
                                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                                {
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

    /// Handle `ImportFrom` statements
    fn handle_import_from(&mut self, import_from: &StmtImportFrom) -> Vec<Stmt> {
        log::debug!(
            "RecursiveImportTransformer::handle_import_from: from {:?} import {:?}",
            import_from
                .module
                .as_ref()
                .map(ruff_python_ast::Identifier::as_str),
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
        );

        // Check if this is a stdlib module that should be normalized
        if let Some(module) = &import_from.module {
            let module_str = module.as_str();
            if import_from.level == 0 && self.should_normalize_stdlib_import(module_str) {
                // Track that this stdlib module was imported
                self.imported_stdlib_modules.insert(module_str.to_string());
                // Also track parent modules for dotted imports
                if let Some(dot_pos) = module_str.find('.') {
                    let parent = &module_str[..dot_pos];
                    self.imported_stdlib_modules.insert(parent.to_string());
                }
                // If we're in a wrapper module, create local assignments
                if self.is_wrapper_init {
                    let mut assignments = Vec::new();

                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        if imported_name == "*" {
                            // Preserve wildcard imports from stdlib to avoid incorrect symbol drops
                            return vec![Stmt::ImportFrom(import_from.clone())];
                        }

                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                        let full_path = format!(
                            "{}.{module_str}.{imported_name}",
                            crate::ast_builder::CRIBO_PREFIX
                        );

                        // Track this renaming for expression rewriting
                        // For importlib.import_module, track it without the _cribo prefix for detection
                        if module_str == "importlib" && imported_name == "import_module" {
                            self.import_aliases.insert(
                                local_name.to_string(),
                                format!("{module_str}.{imported_name}"),
                            );
                        } else {
                            self.import_aliases
                                .insert(local_name.to_string(), full_path.clone());
                        }

                        // Create local assignment: local_name = _cribo.module.symbol
                        let proxy_parts: Vec<&str> = full_path.split('.').collect();
                        let value_expr = crate::ast_builder::expressions::dotted_name(
                            &proxy_parts,
                            ExprContext::Load,
                        );
                        let target =
                            crate::ast_builder::expressions::name(local_name, ExprContext::Store);
                        let assign_stmt =
                            crate::ast_builder::statements::assign(vec![target], value_expr);
                        assignments.push(assign_stmt);

                        // Note: The module_transformer will handle adding these to the
                        // module namespace based on the stdlib_reexports mechanism
                    }

                    return assignments;
                } else {
                    // For non-wrapper modules, create local assignments for from-imported stdlib symbols
                    let mut assignments = Vec::new();

                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        if imported_name == "*" {
                            // Preserve wildcard imports from stdlib to avoid incorrect symbol drops
                            return vec![Stmt::ImportFrom(import_from.clone())];
                        }

                        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
                        let full_path = format!(
                            "{}.{module_str}.{imported_name}",
                            crate::ast_builder::CRIBO_PREFIX
                        );

                        // Track this renaming for expression rewriting
                        // For importlib.import_module, track it without the _cribo prefix for detection
                        if module_str == "importlib" && imported_name == "import_module" {
                            self.import_aliases.insert(
                                local_name.to_string(),
                                format!("{module_str}.{imported_name}"),
                            );
                        } else {
                            self.import_aliases
                                .insert(local_name.to_string(), full_path.clone());
                        }

                        // Create local assignment: local_name = _cribo.module.symbol
                        let proxy_parts: Vec<&str> = full_path.split('.').collect();
                        let value_expr = crate::ast_builder::expressions::dotted_name(
                            &proxy_parts,
                            ExprContext::Load,
                        );
                        let target =
                            crate::ast_builder::expressions::name(local_name, ExprContext::Store);
                        let assign_stmt =
                            crate::ast_builder::statements::assign(vec![target], value_expr);
                        assignments.push(assign_stmt);
                    }

                    return assignments;
                }
            }
        }

        // Resolve relative imports
        let resolved_module = if import_from.level > 0 {
            self.module_path.and_then(|path| {
                self.bundler
                    .resolver
                    .resolve_relative_to_absolute_module_name(
                        import_from.level,
                        import_from
                            .module
                            .as_ref()
                            .map(ruff_python_ast::Identifier::as_str),
                        path,
                    )
            })
        } else {
            import_from
                .module
                .as_ref()
                .map(std::string::ToString::to_string)
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
            if self.bundler.get_module_id(resolved).is_some_and(|id| {
                self.bundler
                    .module_info_registry
                    .as_ref()
                    .is_some_and(|reg| reg.contains_module(&id))
            }) {
                // Check if we have access to global deferred imports
                if let Some(global_deferred) = self.global_deferred_imports {
                    // Check each symbol to see if it's already been deferred
                    let mut all_symbols_deferred = true;
                    if let Some(module_id) = self.bundler.resolver.get_module_id_by_name(resolved) {
                        for alias in &import_from.names {
                            let imported_name = alias.name.as_str(); // The actual name being imported
                            if !global_deferred
                                .contains_key(&(module_id, imported_name.to_string()))
                            {
                                all_symbols_deferred = false;
                                break;
                            }
                        }
                    } else {
                        // Module not found, can't be deferred
                        all_symbols_deferred = false;
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
                    self.bundler
                        .get_module_id(&full_module_path)
                        .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                );

                // Check if this is importing a submodule (like from . import config)
                // First check if it's a wrapper submodule, then check if it's inlined
                if crate::code_generator::module_registry::is_wrapper_submodule(
                    &full_module_path,
                    self.bundler.module_info_registry,
                    &self.bundler.inlined_modules,
                    self.bundler.resolver,
                ) {
                    // This is a wrapper submodule
                    log::debug!("  '{full_module_path}' is a wrapper submodule");

                    // For wrapper modules importing wrapper submodules from the same package
                    if self.is_wrapper_init {
                        // Initialize the wrapper submodule if needed
                        result_stmts.extend(
                            self.bundler
                                .create_module_initialization_for_import(&full_module_path),
                        );

                        // Create assignment: local_name = parent.submodule
                        let module_expr =
                            expressions::module_reference(&full_module_path, ExprContext::Load);

                        result_stmts.push(statements::simple_assign(local_name, module_expr));

                        // Track as local to avoid any accidental rewrites later in this transform pass
                        self.local_variables.insert(local_name.to_string());

                        log::debug!(
                            "  Created assignment for wrapper submodule: {local_name} = {full_module_path}"
                        );

                        // Note: The module attribute assignment (_cribo_module.<local_name> = ...)
                        // is handled later in create_assignments_for_inlined_imports to avoid duplication

                        handled_any = true;
                    } else if !self.is_entry_module
                        && self
                            .bundler
                            .get_module_id(self.module_name)
                            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                    {
                        // This is an inlined module importing a wrapper submodule
                        // We need to defer this import because the wrapper module may not be initialized yet
                        log::debug!(
                            "  Inlined module '{}' importing wrapper submodule '{}' - deferring",
                            self.module_name,
                            full_module_path
                        );

                        // Create a deferred assignment to the wrapper module
                        // This will be executed after all modules are initialized
                        self.deferred_imports.push(statements::simple_assign(
                            local_name,
                            expressions::module_reference(&full_module_path, ExprContext::Load),
                        ));

                        // Track as local to avoid any accidental rewrites later in this transform pass
                        self.local_variables.insert(local_name.to_string());

                        handled_any = true;
                    }
                } else if self
                    .bundler
                    .get_module_id(&full_module_path)
                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                {
                    log::debug!("  '{full_module_path}' is an inlined module");

                    // Check if this module was namespace imported
                    if self
                        .bundler
                        .get_module_id(&full_module_path)
                        .is_some_and(|id| self.bundler.namespace_imported_modules.contains_key(&id))
                    {
                        // Create assignment: local_name = full_module_path_with_underscores
                        // But be careful about stdlib conflicts - only create in entry module if
                        // there's a conflict
                        let namespace_var = sanitize_module_name_for_identifier(&full_module_path);

                        // Check if this would shadow a stdlib module
                        let shadows_stdlib =
                            crate::resolver::is_stdlib_module(local_name, self.python_version);

                        // Only create the assignment if:
                        // 1. We're in the entry module (where user expects the shadowing), OR
                        // 2. The name doesn't conflict with stdlib
                        if self.is_entry_module || !shadows_stdlib {
                            log::debug!(
                                "  Creating namespace assignment: {local_name} = {namespace_var}"
                            );
                            result_stmts.push(statements::simple_assign(
                                local_name,
                                expressions::name(&namespace_var, ExprContext::Load),
                            ));

                            // Track this as a local variable to prevent it from being transformed as a stdlib module
                            self.local_variables.insert(local_name.to_string());
                            log::debug!(
                                "  Tracked '{local_name}' as local variable to prevent stdlib transformation"
                            );
                        } else {
                            log::debug!(
                                "  Skipping namespace assignment: {local_name} = {namespace_var} \
                                 - would shadow stdlib in non-entry module"
                            );
                        }
                        handled_any = true;
                    } else {
                        // This is importing an inlined submodule
                        // We need to handle this specially when the current module is being inlined
                        // (i.e., not the entry module and not a wrapper module)
                        let current_module_is_inlined = self
                            .bundler
                            .get_module_id(self.module_name)
                            .is_some_and(|id| self.bundler.inlined_modules.contains(&id));
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
                                // For inlined modules, use the sanitized module name instead of
                                // local_name e.g., pkg_compat
                                // instead of compat
                                let namespace_var =
                                    sanitize_module_name_for_identifier(&full_module_path);

                                // Only create the namespace if it hasn't been created yet
                                // The bundler should have already registered it during pre-scanning
                                if !self.bundler.is_namespace_registered(&namespace_var) {
                                    // Create: namespace_var = types.SimpleNamespace()
                                    let types_simple_namespace_call = expressions::call(
                                        expressions::simple_namespace_ctor(),
                                        vec![],
                                        vec![],
                                    );
                                    self.deferred_imports.push(statements::simple_assign(
                                        &namespace_var,
                                        types_simple_namespace_call,
                                    ));
                                }

                                // If local_name is different from namespace_var, create an alias
                                if local_name != namespace_var {
                                    self.deferred_imports.push(statements::simple_assign(
                                        local_name,
                                        expressions::name(&namespace_var, ExprContext::Load),
                                    ));
                                }
                                self.created_namespace_objects = true;

                                // If this is a submodule being imported (from . import compat),
                                // and the parent module is also being used as a namespace
                                // externally, we need to create the
                                // parent.child assignment
                                if resolved_module.as_deref() == Some(self.module_name) {
                                    // Check if this submodule is in the parent's __all__ exports
                                    let parent_exports = self
                                        .bundler
                                        .get_module_id(self.module_name)
                                        .and_then(|id| self.bundler.module_exports.get(&id))
                                        .and_then(|opt| opt.as_ref())
                                        .is_some_and(|exports| {
                                            exports.contains(&imported_name.to_string())
                                        });

                                    if parent_exports {
                                        // Check if this is a submodule that was inlined or uses an
                                        // init function
                                        let full_submodule_path =
                                            format!("{}.{}", self.module_name, imported_name);
                                        let is_inlined_submodule = self
                                            .bundler
                                            .get_module_id(&full_submodule_path)
                                            .is_some_and(|id| {
                                                self.bundler.inlined_modules.contains(&id)
                                            });
                                        let uses_init_function = self
                                            .bundler
                                            .get_module_id(&full_submodule_path)
                                            .and_then(|id| {
                                                self.bundler.module_init_functions.get(&id)
                                            })
                                            .is_some();

                                        log::debug!(
                                            "  Checking submodule status for \
                                             {full_submodule_path}: \
                                             is_inlined={is_inlined_submodule}, \
                                             uses_init={uses_init_function}"
                                        );

                                        if is_inlined_submodule || uses_init_function {
                                            // This submodule was already assigned to the parent
                                            // namespace
                                            // by the bundler when it created the init function
                                            log::debug!(
                                                "  Skipping parent module assignment for {}.{} - \
                                                 already handled by init function",
                                                self.module_name,
                                                local_name
                                            );
                                        } else {
                                            // For the case where a module uses an init function but
                                            // wasn't detected above,
                                            // we need to double-check if this is really a symbol or
                                            // a module
                                            let is_actually_a_module = self
                                                .bundler
                                                .get_module_id(&full_submodule_path)
                                                .is_some_and(|id| {
                                                    self.bundler.bundled_modules.contains(&id)
                                                        || self
                                                            .bundler
                                                            .module_info_registry
                                                            .as_ref()
                                                            .is_some_and(|reg| {
                                                                reg.contains_module(&id)
                                                            })
                                                        || self
                                                            .bundler
                                                            .inlined_modules
                                                            .contains(&id)
                                                });

                                            if is_actually_a_module {
                                                // This is a module, not a symbol - skip the
                                                // assignment
                                                log::debug!(
                                                    "Skipping assignment for {}.{} - it's a \
                                                     module, not a symbol",
                                                    self.module_name,
                                                    local_name
                                                );
                                            } else {
                                                // This is a symbol, not a submodule, so we need the
                                                // assignment
                                                log::debug!(
                                                    "Creating parent module assignment: {}.{} = \
                                                     {} (symbol exported from parent)",
                                                    self.module_name,
                                                    local_name,
                                                    local_name
                                                );
                                                self.deferred_imports.push(statements::assign(
                                                    vec![expressions::attribute(
                                                        expressions::name(
                                                            self.module_name,
                                                            ExprContext::Load,
                                                        ),
                                                        local_name,
                                                        ExprContext::Store,
                                                    )],
                                                    expressions::name(
                                                        local_name,
                                                        ExprContext::Load,
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                }

                                // Now add the exported symbols from the inlined module to the
                                // namespace
                                if let Some(module_id) =
                                    self.bundler.get_module_id(&full_module_path)
                                    && let Some(exports) = self
                                        .bundler
                                        .module_exports
                                        .get(&module_id)
                                        .cloned()
                                        .flatten()
                                {
                                    // Filter exports to only include symbols that survived
                                    // tree-shaking
                                    let filtered_exports: Vec<String> =
                                        SymbolAnalyzer::filter_exports_by_tree_shaking(
                                            &exports,
                                            &module_id,
                                            self.bundler.tree_shaking_keep_symbols.as_ref(),
                                            false,
                                            self.bundler.resolver,
                                        )
                                        .into_iter()
                                        .cloned()
                                        .collect();

                                    // Add __all__ attribute to the namespace with filtered exports
                                    // BUT ONLY if the original module had an explicit __all__ AND
                                    // the code actually accesses this module's __all__
                                    if !filtered_exports.is_empty()
                                        && self
                                            .bundler
                                            .modules_with_explicit_all
                                            .contains(&module_id)
                                        && self.bundler.get_module_id(self.module_name).is_some_and(
                                            |id| {
                                                self.bundler.modules_with_accessed_all.iter().any(
                                                    |(module, alias)| {
                                                        module == &id && alias == local_name
                                                    },
                                                )
                                            },
                                        )
                                    {
                                        let export_strings: Vec<&str> =
                                            filtered_exports.iter().map(String::as_str).collect();
                                        self.deferred_imports.push(statements::set_list_attribute(
                                            &namespace_var,
                                            "__all__",
                                            &export_strings,
                                        ));
                                    }

                                    // Only populate the namespace if it wasn't already populated
                                    // Check if this namespace was already populated by the bundler
                                    // symbols_populated_after_deferred contains (ModuleId, symbol)
                                    // tuples
                                    let namespace_already_populated =
                                        self.bundler.get_module_id(&full_module_path).is_some_and(
                                            |id| {
                                                self.bundler
                                                    .symbols_populated_after_deferred
                                                    .iter()
                                                    .any(|(ns, _)| ns == &id)
                                            },
                                        ) || self.populated_modules.contains(&full_module_path);

                                    if !namespace_already_populated {
                                        for symbol in filtered_exports {
                                            // Use the sanitized namespace variable for inlined
                                            // modules
                                            // namespace_var.symbol = symbol
                                            let target = expressions::attribute(
                                                expressions::name(
                                                    &namespace_var,
                                                    ExprContext::Load,
                                                ),
                                                &symbol,
                                                ExprContext::Store,
                                            );
                                            let symbol_name = self
                                                .bundler
                                                .get_module_id(&full_module_path)
                                                .and_then(|id| self.symbol_renames.get(&id))
                                                .and_then(|renames| renames.get(&symbol))
                                                .cloned()
                                                .unwrap_or_else(|| symbol.clone());
                                            let value =
                                                expressions::name(&symbol_name, ExprContext::Load);
                                            self.deferred_imports
                                                .push(statements::assign(vec![target], value));
                                        }
                                        // Mark this module as populated to prevent duplicate
                                        // assignments
                                        self.populated_modules.insert(full_module_path.clone());
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
                                if let Some(module_id) =
                                    self.bundler.get_module_id(&full_module_path)
                                    && let Some(exports) = self
                                        .bundler
                                        .module_exports
                                        .get(&module_id)
                                        .cloned()
                                        .flatten()
                                {
                                    // Filter exports to only include symbols that survived
                                    // tree-shaking
                                    let filtered_exports: Vec<String> =
                                        SymbolAnalyzer::filter_exports_by_tree_shaking(
                                            &exports,
                                            &module_id,
                                            self.bundler.tree_shaking_keep_symbols.as_ref(),
                                            false,
                                            self.bundler.resolver,
                                        )
                                        .into_iter()
                                        .cloned()
                                        .collect();

                                    // Add __all__ attribute to the namespace with filtered exports
                                    // BUT ONLY if the original module had an explicit __all__ AND
                                    // the code actually accesses this module's __all__
                                    if !filtered_exports.is_empty()
                                        && self
                                            .bundler
                                            .modules_with_explicit_all
                                            .contains(&module_id)
                                        && self.bundler.get_module_id(self.module_name).is_some_and(
                                            |id| {
                                                self.bundler.modules_with_accessed_all.iter().any(
                                                    |(module, alias)| {
                                                        module == &id && alias == local_name
                                                    },
                                                )
                                            },
                                        )
                                    {
                                        let export_strings: Vec<&str> =
                                            filtered_exports.iter().map(String::as_str).collect();
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
                                            .bundler
                                            .get_module_id(&full_module_path)
                                            .and_then(|id| self.symbol_renames.get(&id))
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
            }
            log::debug!(
                "  Returning {} transformed statements for import",
                result_stmts.len()
            );
            log::debug!("  Statements: {result_stmts:?}");
            // We've already handled the import completely, don't fall through to other handling
            return result_stmts;
        }

        if let Some(ref resolved) = resolved_module {
            // Check if this is an inlined module
            if let Some(resolved_id) = self.bundler.get_module_id(resolved)
                && self.bundler.inlined_modules.contains(&resolved_id)
            {
                // Check if this is a circular module with pre-declarations
                if self.bundler.circular_modules.contains(&resolved_id) {
                    log::debug!("  Module '{resolved}' is a circular module with pre-declarations");
                    let current_module_id = self.bundler.get_module_id(self.module_name);
                    log::debug!(
                        "  Current module '{}' is circular: {}, is inlined: {}",
                        self.module_name,
                        current_module_id
                            .is_some_and(|id| self.bundler.circular_modules.contains(&id)),
                        current_module_id
                            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                    );
                    // Special handling for imports between circular inlined modules
                    // If the current module is also a circular inlined module, we need to defer or
                    // transform differently
                    if current_module_id
                        .is_some_and(|id| self.bundler.circular_modules.contains(&id))
                        && current_module_id
                            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
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

                            // Check if this is actually a submodule import
                            let full_submodule_path = format!("{resolved}.{imported_name}");
                            log::debug!(
                                "  Checking if '{full_submodule_path}' is a submodule (bundled: \
                                 {}, inlined: {})",
                                self.bundler
                                    .get_module_id(&full_submodule_path)
                                    .is_some_and(|id| self.bundler.bundled_modules.contains(&id)),
                                self.bundler
                                    .get_module_id(&full_submodule_path)
                                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                            );
                            if self
                                .bundler
                                .get_module_id(&full_submodule_path)
                                .is_some_and(|id| self.bundler.bundled_modules.contains(&id))
                                || self
                                    .bundler
                                    .get_module_id(&full_submodule_path)
                                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                            {
                                log::debug!(
                                    "  Skipping assignment for '{imported_name}' - it's a \
                                     submodule, not a symbol"
                                );
                                // This is a submodule import, not a symbol import
                                // The submodule will be handled separately, so we don't create an
                                // assignment
                                continue;
                            }

                            // Check if the symbol was renamed during bundling
                            let actual_name = if let Some(resolved_id) =
                                self.bundler.get_module_id(resolved)
                                && let Some(renames) = self.symbol_renames.get(&resolved_id)
                            {
                                renames
                                    .get(imported_name)
                                    .map_or(imported_name, String::as_str)
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
                    }
                    // Original behavior for non-circular modules importing from circular
                    // modules
                    return handle_imports_from_inlined_module_with_context(
                        self.bundler,
                        import_from,
                        resolved,
                        self.symbol_renames,
                        self.is_wrapper_init,
                        Some(self.module_name),
                    );
                } else {
                    log::debug!("  Module '{resolved}' is inlined, handling import assignments");
                    // For the entry module, we should not defer these imports
                    // because they need to be available when the entry module's code runs
                    let import_stmts = handle_imports_from_inlined_module_with_context(
                        self.bundler,
                        import_from,
                        resolved,
                        self.symbol_renames,
                        self.is_wrapper_init,
                        Some(self.module_name),
                    );

                    // Only defer if we're not in the entry module or wrapper init
                    if self.is_entry_module || self.is_wrapper_init {
                        // For entry module and wrapper init functions, return the imports immediately
                        // In wrapper init functions, module attributes need to be set where the import was
                        if !import_stmts.is_empty() {
                            return import_stmts;
                        }
                        // If handle_imports_from_inlined_module returned empty (e.g., for submodule
                        // imports), fall through to check if we need to
                        // handle it differently
                        log::debug!(
                            "  handle_imports_from_inlined_module returned empty for entry \
                             module or wrapper init, checking for submodule imports"
                        );
                    } else {
                        self.deferred_imports.extend(import_stmts);
                        // Return empty - these imports will be added after all modules are inlined
                        return vec![];
                    }
                }
            }

            // Check if this is a wrapper module (in module_registry)
            // This check must be after the inlined module check to avoid double-handling
            if self.bundler.get_module_id(resolved).is_some_and(|id| {
                self.bundler
                    .module_info_registry
                    .as_ref()
                    .is_some_and(|reg| reg.contains_module(&id))
            }) {
                log::debug!("  Module '{resolved}' is a wrapper module");

                // For modules importing from wrapper modules, we may need to defer
                // the imports to ensure proper initialization order
                let current_module_is_inlined = self
                    .bundler
                    .get_module_id(self.module_name)
                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id));

                // When an inlined module imports from a wrapper module, we need to
                // track the imports and rewrite all usages within the module
                if !self.is_entry_module && current_module_is_inlined {
                    log::debug!(
                        "  Tracking wrapper module imports for rewriting in module '{}' (inlined: \
                         {})",
                        self.module_name,
                        current_module_is_inlined
                    );

                    // First, ensure the wrapper module is initialized
                    // This is crucial for lazy imports inside functions
                    let init_stmts = Vec::new();

                    // Check if the parent module needs handling
                    if let Some((parent, child)) = resolved.rsplit_once('.') {
                        // If the parent is also a wrapper module, DO NOT initialize it here
                        // It will be initialized when accessed
                        if self.bundler.get_module_id(parent).is_some_and(|id| {
                            self.bundler
                                .module_info_registry
                                .is_some_and(|reg| reg.contains_module(&id))
                        }) {
                            log::debug!(
                                "  Parent '{parent}' is a wrapper module - skipping immediate initialization"
                            );
                            // Don't initialize parent wrapper module here
                        }

                        // If the parent is an inlined module, the submodule assignment is handled
                        // by its own initialization, so we only need to log
                        if self
                            .bundler
                            .get_module_id(parent)
                            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                        {
                            log::debug!(
                                "Parent '{parent}' is inlined, submodule '{child}' assignment \
                                 already handled"
                            );
                        }
                    }

                    // Check if this is a wildcard import
                    let is_wildcard =
                        import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*";

                    // DO NOT initialize the wrapper module here!
                    // When an inlined module imports from a wrapper module, the wrapper will be
                    // initialized later when it's actually accessed (lazy initialization).
                    // Initializing it here would cause forward reference errors if the wrapper
                    // module depends on other modules that haven't been defined yet.
                    // For wildcard imports, we'll handle this specially to ensure proper ordering
                    if !is_wildcard {
                        log::debug!(
                            "  Skipping immediate initialization of wrapper module '{resolved}' - will be initialized lazily"
                        );
                        // Don't create initialization here - wrapper modules are initialized on-demand
                    }

                    // Handle wildcard import export assignments
                    if is_wildcard {
                        log::debug!("  Handling wildcard import from wrapper module '{resolved}'");

                        // For wildcard imports from wrapper modules in inlined modules,
                        // we need to:
                        // 1. Initialize the wrapper module
                        // 2. Import all exports from that module into the current namespace

                        // Remember insertion point for newly appended assignments
                        let start_idx = self.deferred_imports.len();

                        // Get the exports from the wrapper module
                        if let Some(exports) = self
                            .bundler
                            .get_module_id(resolved)
                            .and_then(|id| self.bundler.module_exports.get(&id))
                        {
                            if let Some(export_list) = exports {
                                log::debug!(
                                    "  Wrapper module '{resolved}' exports: {export_list:?}"
                                );

                                // Create assignment statements for each export
                                // These should be simple references since we're in an inlined module
                                for export in export_list {
                                    if export != "*" {
                                        // In an inlined module doing wildcard import, we just need to
                                        // make the symbols available at the current scope level
                                        // The actual assignment will happen when the wrapper is initialized

                                        // Create: export_name = module.path.export_name
                                        // We reference the module through its dotted path
                                        let module_ref = if resolved.contains('.') {
                                            // Use dotted attribute access for submodules
                                            let parts: Vec<&str> = resolved.split('.').collect();
                                            expressions::dotted_name(&parts, ExprContext::Load)
                                        } else {
                                            // Simple module name
                                            expressions::name(resolved, ExprContext::Load)
                                        };

                                        // Use ast_builder to create the assignment
                                        let target = expressions::name(export, ExprContext::Store);
                                        let value = expressions::attribute(
                                            module_ref,
                                            export,
                                            ExprContext::Load,
                                        );
                                        let assign_stmt = statements::assign(vec![target], value);

                                        // Defer the assignment
                                        self.deferred_imports.push(assign_stmt);
                                    }
                                }
                            } else {
                                // No explicit __all__, import all public symbols
                                log::debug!(
                                    "  Wrapper module '{resolved}' has no explicit exports, importing all public symbols"
                                );

                                // We can't determine all symbols at compile time for wrapper modules
                                // So we'll need to use a different approach - perhaps iterate over the namespace
                                // For now, we'll just initialize the module
                                log::warn!(
                                    "  Warning: Wildcard import from wrapper module without explicit __all__ may not import all symbols correctly"
                                );
                            }
                        } else {
                            log::warn!(
                                "  Warning: Could not find exports for wrapper module '{resolved}'"
                            );
                        }

                        // Add the initialization to deferred imports BEFORE the assignments
                        // We need to prepend it
                        let init_statements = self
                            .bundler
                            .create_module_initialization_for_import(resolved);

                        // Interleave init statements ONLY before the assignments added by this wildcard
                        // without disturbing previously deferred imports.
                        let new_assignments = self.deferred_imports.split_off(start_idx);
                        self.deferred_imports.extend(init_statements);
                        self.deferred_imports.extend(new_assignments);

                        log::debug!(
                            "  Returning {} parent-init statements for wildcard import; wrapper init + assignments were deferred",
                            init_stmts.len()
                        );
                        return init_stmts;
                    }

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

                    // Return the initialization statements
                    // All usages will be rewritten to use the fully qualified name
                    return init_stmts;
                }
                // For wrapper modules importing from other wrapper modules,
                // let it fall through to standard transformation
            }
        }

        // Otherwise, use standard transformation
        rewrite_import_from(RewriteImportFromParams {
            bundler: self.bundler,
            import_from: import_from.clone(),
            current_module: self.module_name,
            module_path: self.module_path,
            symbol_renames: self.symbol_renames,
            inside_wrapper_init: self.is_wrapper_init,
            python_version: self.python_version,
        })
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
                // First check if the base of this attribute is a wrapper module import
                if let Expr::Name(base_name) = &*attr_expr.value {
                    let name = base_name.id.as_str();

                    // Check if this is a stdlib module reference (e.g., collections.abc)
                    if crate::resolver::is_stdlib_module(name, self.python_version) {
                        // Check if this stdlib name is shadowed by local variables or imports
                        // In wrapper modules, we only track local_variables which includes imported names
                        let is_shadowed = self.local_variables.contains(name)
                            || self.import_aliases.contains_key(name);

                        if !is_shadowed {
                            // Transform stdlib module attribute access to use _cribo proxy
                            // e.g., collections.abc -> _cribo.collections.abc
                            log::debug!(
                                "Transforming stdlib attribute access: {}.{} -> _cribo.{}.{}",
                                name,
                                attr_expr.attr.as_str(),
                                name,
                                attr_expr.attr.as_str()
                            );

                            // Create _cribo.module.attr
                            let attr_name = attr_expr.attr.to_string();
                            let attr_ctx = attr_expr.ctx;
                            let attr_range = attr_expr.range;

                            // Create _cribo.name.attr_name
                            let base = expressions::name_attribute(
                                crate::ast_builder::CRIBO_PREFIX,
                                name,
                                ExprContext::Load,
                            );
                            let mut new_expr = expressions::attribute(base, &attr_name, attr_ctx);
                            // Preserve the original range
                            if let Expr::Attribute(attr) = &mut new_expr {
                                attr.range = attr_range;
                            }
                            *expr = new_expr;
                            return;
                        }
                    }

                    if let Some((wrapper_module, imported_name)) =
                        self.wrapper_module_imports.get(name)
                    {
                        // The base is a wrapper module import, rewrite the entire attribute access
                        // e.g., cookielib.CookieJar -> myrequests.compat.cookielib.CookieJar
                        log::debug!(
                            "Rewriting attribute '{}.{}' to '{}.{}.{}'",
                            name,
                            attr_expr.attr.as_str(),
                            wrapper_module,
                            imported_name,
                            attr_expr.attr.as_str()
                        );

                        // Create wrapper_module.imported_name.attr
                        let base = expressions::name_attribute(
                            wrapper_module,
                            imported_name,
                            ExprContext::Load,
                        );
                        let mut new_expr =
                            expressions::attribute(base, attr_expr.attr.as_str(), attr_expr.ctx);
                        // Preserve the original range
                        if let Expr::Attribute(attr) = &mut new_expr {
                            attr.range = attr_expr.range;
                        }
                        *expr = new_expr;
                        return; // Don't process further
                    }
                }

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

                            if self
                                .bundler
                                .get_module_id(&namespace_path)
                                .is_some_and(|id| self.bundler.bundled_modules.contains(&id))
                            {
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
                                let new_expr = if let Some(module_id) =
                                    self.bundler.get_module_id(module_name)
                                    && let Some(module_renames) =
                                        self.symbol_renames.get(&module_id)
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
                        // Check if the base is a stdlib import alias (e.g., j for json)
                        else if let Some(stdlib_path) = self.import_aliases.get(&base) {
                            // This is accessing an attribute on a stdlib module alias
                            // Transform j.dumps to _cribo.json.dumps
                            if attr_path.len() == 1 {
                                let attr_name = &attr_path[0];
                                log::debug!(
                                    "Transforming {base}.{attr_name} to {stdlib_path}.{attr_name} \
                                     (stdlib import alias)"
                                );

                                // Create dotted name expression like _cribo.json.dumps
                                let full_path = format!("{stdlib_path}.{attr_name}");
                                let parts: Vec<&str> = full_path.split('.').collect();
                                let new_expr = crate::ast_builder::expressions::dotted_name(
                                    &parts,
                                    attr_expr.ctx,
                                );
                                *expr = new_expr;
                                return;
                            } else {
                                // For deeper paths like j.decoder.JSONDecoder, build the full path
                                let mut full_path = stdlib_path.clone();
                                for part in &attr_path {
                                    full_path.push('.');
                                    full_path.push_str(part);
                                }
                                log::debug!(
                                    "Transforming {base}.{} to {full_path} (stdlib import alias, deep path)",
                                    attr_path.join(".")
                                );

                                let parts: Vec<&str> = full_path.split('.').collect();
                                let new_expr = crate::ast_builder::expressions::dotted_name(
                                    &parts,
                                    attr_expr.ctx,
                                );
                                *expr = new_expr;
                                return;
                            }
                        }
                        // Check if the base refers to an inlined module
                        else if let Some(actual_module) = self.find_module_for_alias(&base)
                            && self
                                .bundler
                                .get_module_id(&actual_module)
                                .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
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
                                if self
                                    .bundler
                                    .get_module_id(&potential_submodule)
                                    .is_some_and(|id| self.bundler.bundled_modules.contains(&id))
                                    && !self
                                        .bundler
                                        .get_module_id(&potential_submodule)
                                        .is_some_and(|id| {
                                            self.bundler.inlined_modules.contains(&id)
                                        })
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
                                    if self.bundler.get_module_id(&actual_module).is_some_and(
                                        |id| {
                                            self.bundler
                                                .namespace_imported_modules
                                                .contains_key(&id)
                                        },
                                    ) {
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
                                        let new_expr = if let Some(module_id) =
                                            self.bundler.get_module_id(&actual_module)
                                            && let Some(module_renames) =
                                                self.symbol_renames.get(&module_id)
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
                                            if let Some(exports) = self
                                                .bundler
                                                .get_module_id(&actual_module)
                                                .and_then(|id| self.bundler.module_exports.get(&id))
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

                                if self
                                    .bundler
                                    .get_module_id(&potential_module)
                                    .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
                                {
                                    // This is accessing an attribute on a submodule
                                    // Build the remaining attribute path
                                    let remaining_attrs = &attr_path[1..];

                                    if remaining_attrs.len() == 1 {
                                        let final_attr = &remaining_attrs[0];

                                        // Check if this symbol was renamed during inlining
                                        if let Some(module_id) =
                                            self.bundler.get_module_id(&potential_module)
                                            && let Some(module_renames) =
                                                self.symbol_renames.get(&module_id)
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
                // Preserve the original flags from the f-string
                let original_flags =
                    crate::ast_builder::expressions::get_fstring_flags(&fstring_expr.value);
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
                        range: fstring_range,
                        flags: original_flags, // Preserve the original flags including quote style
                    };

                    let new_value = FStringValue::single(new_fstring);

                    *expr = Expr::FString(ExprFString {
                        node_index: AtomicNodeIndex::dummy(),
                        value: new_value,
                        range: fstring_range,
                    });
                }
            }
            // Check if Name expressions need to be rewritten for wrapper module imports or stdlib imports
            Expr::Name(name_expr) => {
                let name = name_expr.id.as_str();

                // Check if this name is a stdlib import alias that needs rewriting
                // Only rewrite if it's not shadowed by a local variable
                if let Some(rewritten_path) = self.import_aliases.get(name) {
                    // Check if this is a stdlib module reference (starts with _cribo.)
                    if rewritten_path.starts_with(crate::ast_builder::CRIBO_PREFIX)
                        && rewritten_path
                            .chars()
                            .nth(crate::ast_builder::CRIBO_PREFIX.len())
                            == Some('.')
                    {
                        // Use semantic analysis to check if this is shadowed by a local variable
                        let is_shadowed =
                            if let Some(_semantic_bundler) = self.bundler.semantic_bundler {
                                // Try to find the module in the semantic bundler
                                // This is a simplified check - in reality we'd need to know the exact scope
                                // For now, we'll skip the semantic check if we don't have proper module info
                                false // TODO: Implement proper semantic check using SemanticModel
                            } else {
                                false
                            };

                        if !is_shadowed {
                            log::debug!(
                                "Rewriting stdlib reference '{name}' to '{rewritten_path}'"
                            );

                            // Parse the rewritten path to create attribute access
                            // e.g., "_cribo.json" becomes _cribo.json
                            let parts: Vec<&str> = rewritten_path.split('.').collect();
                            if parts.len() >= 2 {
                                *expr = expressions::dotted_name(&parts, name_expr.ctx);
                                return;
                            }
                        }
                    }
                }

                // Check if this name was imported from a wrapper module and needs rewriting
                if let Some((wrapper_module, imported_name)) = self.wrapper_module_imports.get(name)
                {
                    log::debug!("Rewriting name '{name}' to '{wrapper_module}.{imported_name}'");

                    // Create wrapper_module.imported_name attribute access
                    // Create wrapper_module.imported_name attribute access
                    let mut new_expr =
                        expressions::name_attribute(wrapper_module, imported_name, name_expr.ctx);
                    // Preserve the original range
                    if let Expr::Attribute(attr) = &mut new_expr {
                        attr.range = name_expr.range;
                    }
                    *expr = new_expr;
                }
            }
            // Constants, etc. don't need transformation
            _ => {}
        }
    }

    /// Collect the full dotted attribute path from a potentially nested attribute expression
    /// Returns (`base_name`, [attr1, attr2, ...])
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
        if !self.is_entry_module
            && self
                .bundler
                .get_module_id(alias)
                .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
        {
            Some(alias.to_string())
        } else {
            None
        }
    }

    /// Create module access expression
    pub fn create_module_access_expr(&self, module_name: &str) -> Expr {
        // Check if this is a wrapper module
        if let Some(synthetic_name) = self
            .bundler
            .get_module_id(module_name)
            .and_then(|id| self.bundler.module_synthetic_names.get(&id))
        {
            // This is a wrapper module - we need to call its init function
            // This handles modules with invalid Python identifiers like "my-module"
            let init_func_name =
                crate::code_generator::module_registry::get_init_function_name(synthetic_name);

            // Create init function call with module as self argument
            let module_var = sanitize_module_name_for_identifier(module_name);
            expressions::call(
                expressions::name(&init_func_name, ExprContext::Load),
                vec![expressions::name(&module_var, ExprContext::Load)],
                vec![],
            )
        } else if self
            .bundler
            .get_module_id(module_name)
            .is_some_and(|id| self.bundler.inlined_modules.contains(&id))
        {
            // This is an inlined module - create namespace object
            let module_renames = self
                .bundler
                .get_module_id(module_name)
                .and_then(|id| self.symbol_renames.get(&id));
            self.create_namespace_call_for_inlined_module(module_name, module_renames)
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
                let module_id = self
                    .bundler
                    .get_module_id(module_name)
                    .expect("Module should exist");
                if !self
                    .bundler
                    .is_symbol_kept_by_tree_shaking(module_id, original_name)
                {
                    log::debug!(
                        "Skipping tree-shaken symbol '{original_name}' from namespace for module \
                         '{module_name}'"
                    );
                    continue;
                }

                seen_args.insert(renamed_name.clone());

                keywords.push(expressions::keyword(
                    Some(original_name),
                    expressions::name(renamed_name, ExprContext::Load),
                ));
            }
        }

        // Also check if module has module-level variables that weren't renamed
        if let Some(module_id) = self.bundler.get_module_id(module_name)
            && let Some(exports) = self.bundler.module_exports.get(&module_id)
            && let Some(export_list) = exports
        {
            for export in export_list {
                // Check if this export was already added as a renamed symbol
                let was_renamed =
                    module_renames.is_some_and(|renames| renames.contains_key(export));
                if !was_renamed && !seen_args.contains(export) {
                    // Check if this symbol survived tree-shaking
                    if !self
                        .bundler
                        .is_symbol_kept_by_tree_shaking(module_id, export)
                    {
                        log::debug!(
                            "Skipping tree-shaken export '{export}' from namespace for module \
                                 '{module_name}'"
                        );
                        continue;
                    }

                    // This export wasn't renamed and wasn't already added, add it directly
                    seen_args.insert(export.clone());
                    keywords.push(expressions::keyword(
                        Some(export),
                        expressions::name(export, ExprContext::Load),
                    ));
                }
            }
        }

        // Create types.SimpleNamespace(**kwargs) call
        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords)
    }
}

/// Rewrite import with renames
fn rewrite_import_with_renames(
    bundler: &Bundler,
    import_stmt: StmtImport,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    populated_modules: &mut FxIndexSet<String>,
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
            if let Some(module_id) = bundler.get_module_id(module_name) {
                if bundler.bundled_modules.contains(&module_id) {
                    if bundler
                        .module_info_registry
                        .is_some_and(|reg| reg.contains_module(&module_id))
                    {
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
                            // For import a.b.c.d, we need:
                            // a.b = <module a.b>
                            // a.b.c = <module a.b.c>
                            // a.b.c.d = <module a.b.c.d>
                            for i in 2..=parts.len() {
                                let parent = parts[..i - 1].join(".");
                                let attr = parts[i - 1];
                                let full_path = parts[..i].join(".");

                                // Determine what the RHS will be for this assignment
                                let sanitized = sanitize_module_name_for_identifier(&full_path);
                                let has_namespace_var =
                                    bundler.created_namespaces.contains(&sanitized);
                                let is_wrapper = bundler
                                    .get_module_id(&full_path)
                                    .is_some_and(|id| bundler.bundled_modules.contains(&id));

                                // Skip only if this would be a true no-op self-assignment
                                // A self-assignment is only redundant if:
                                // 1. There's no namespace variable (so RHS would be the dotted path)
                                // 2. It's not a wrapper module (which needs the assignment for linkage)
                                // 3. The LHS and RHS would be identical dotted paths
                                if !has_namespace_var && !is_wrapper {
                                    // In this case, create_attribute_assignment would generate
                                    // parent.attr = parent.attr (a no-op), so we can skip it
                                    log::debug!(
                                        "Skipping redundant self-assignment: {parent}.{attr} = {full_path}"
                                    );
                                } else {
                                    // Use centralized namespace-aware assignment creation
                                    result_stmts.push(
                                    crate::code_generator::namespace_manager::create_attribute_assignment(
                                        bundler,
                                        &parent,
                                        attr,
                                        &full_path,
                                    )
                                );
                                }
                            }
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
                                // AND we haven't already populated it in this session
                                if bundler
                                    .get_module_id(&partial_module)
                                    .is_some_and(|id| bundler.bundled_modules.contains(&id))
                                    && !populated_modules.contains(&partial_module)
                                {
                                    // Note: This is a limitation - we can't mutate
                                    // namespace_assignments_made
                                    // from here since bundler is immutable. This will be handled during
                                    // the main bundle process where bundler is mutable.
                                    log::debug!(
                                        "Cannot track namespace assignments for '{partial_module}' in \
                                     import transformer due to immutability"
                                    );
                                    // For now, we'll create the statements without tracking duplicates
                                    let mut ctx = create_namespace_population_context(bundler);
                                    if let Some(partial_module_id) =
                                        bundler.get_module_id(&partial_module)
                                    {
                                        let new_stmts = crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                                        &mut ctx,
                                        &partial_module,
                                        partial_module_id,
                                        symbol_renames,
                                    );
                                        result_stmts.extend(new_stmts);
                                    } else {
                                        log::warn!(
                                            "Could not find module ID for partial module '{partial_module}'"
                                        );
                                    }
                                    populated_modules.insert(partial_module.clone());
                                }
                            }
                        } else {
                            // For simple imports or aliased imports, create namespace object with
                            // the module's exports

                            // Check if namespace already exists
                            if bundler.created_namespaces.contains(target_name.as_str()) {
                                log::debug!(
                                    "Skipping namespace creation for '{}' - already created globally",
                                    target_name.as_str()
                                );
                            } else {
                                let namespace_stmt = bundler.create_namespace_object_for_module(
                                    target_name.as_str(),
                                    module_name,
                                );
                                result_stmts.push(namespace_stmt);
                            }

                            // Always populate the namespace with symbols
                            log::debug!(
                                "Cannot track namespace assignments for '{module_name}' in import \
                             transformer due to immutability"
                            );
                            // For now, we'll create the statements without tracking duplicates
                            let mut ctx = create_namespace_population_context(bundler);
                            let new_stmts = crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                            &mut ctx,
                            target_name.as_str(),
                            module_id,
                            symbol_renames,
                        );
                            result_stmts.extend(new_stmts);
                        }
                    }
                }
            } else {
                handled_all = false;
            }
        } else {
            // Non-dotted import - handle as before
            let module_id = if let Some(id) = bundler.get_module_id(module_name) {
                id
            } else {
                handled_all = false;
                continue;
            };

            if !bundler.bundled_modules.contains(&module_id) {
                handled_all = false;
                continue;
            }

            if bundler
                .module_info_registry
                .is_some_and(|reg| reg.contains_module(&module_id))
            {
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
                if bundler.created_namespaces.contains(target_name.as_str()) {
                    log::debug!(
                        "Skipping namespace creation for '{}' - already created globally",
                        target_name.as_str()
                    );
                } else {
                    let namespace_stmt = bundler
                        .create_namespace_object_for_module(target_name.as_str(), module_name);
                    result_stmts.push(namespace_stmt);
                }

                // Populate the namespace with symbols only if not already populated
                if populated_modules.contains(module_name) {
                    log::debug!(
                        "Skipping namespace population for '{module_name}' - already populated in \
                         this transformation session"
                    );
                } else {
                    log::debug!(
                        "Cannot track namespace assignments for '{module_name}' in import \
                         transformer due to immutability"
                    );
                    // For now, we'll create the statements without tracking duplicates
                    let mut ctx = create_namespace_population_context(bundler);
                    let new_stmts = crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                        &mut ctx,
                        target_name.as_str(),
                        module_id,
                        symbol_renames,
                    );
                    result_stmts.extend(new_stmts);
                    populated_modules.insert(module_name.to_string());
                }
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

/// Create a `NamespacePopulationContext` for populating namespace symbols.
///
/// This helper function reduces code duplication when creating the context
/// for namespace population operations in import transformation.
fn create_namespace_population_context<'a>(
    bundler: &'a crate::code_generator::bundler::Bundler,
) -> crate::code_generator::namespace_manager::NamespacePopulationContext<'a> {
    crate::code_generator::namespace_manager::NamespacePopulationContext {
        inlined_modules: &bundler.inlined_modules,
        module_exports: &bundler.module_exports,
        tree_shaking_keep_symbols: &bundler.tree_shaking_keep_symbols,
        bundled_modules: &bundler.bundled_modules,
        modules_with_accessed_all: &bundler.modules_with_accessed_all,
        wrapper_modules: &bundler.wrapper_modules,
        module_asts: &bundler.module_asts,
        symbols_populated_after_deferred: &bundler.symbols_populated_after_deferred,
        global_deferred_imports: &bundler.global_deferred_imports,
        module_init_functions: &bundler.module_init_functions,
        resolver: bundler.resolver,
    }
}

/// Check if an import statement is importing bundled submodules
fn has_bundled_submodules(
    import_from: &StmtImportFrom,
    module_name: &str,
    bundler: &Bundler,
) -> bool {
    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let full_module_path = format!("{module_name}.{imported_name}");
        log::trace!("  Checking if '{full_module_path}' is in bundled_modules");
        if bundler
            .get_module_id(&full_module_path)
            .is_some_and(|id| bundler.bundled_modules.contains(&id))
        {
            log::trace!("    -> YES, it's bundled");
            return true;
        }
        log::trace!("    -> NO, not bundled");
    }
    false
}

/// Parameters for rewriting import from statements
struct RewriteImportFromParams<'a> {
    bundler: &'a Bundler<'a>,
    import_from: StmtImportFrom,
    current_module: &'a str,
    module_path: Option<&'a Path>,
    symbol_renames: &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    inside_wrapper_init: bool,
    python_version: u8,
}

/// Rewrite import from statement with proper handling for bundled modules
fn rewrite_import_from(params: RewriteImportFromParams) -> Vec<Stmt> {
    let RewriteImportFromParams {
        bundler,
        import_from,
        current_module,
        module_path,
        symbol_renames,
        inside_wrapper_init,
        python_version,
    } = params;
    // Resolve relative imports to absolute module names
    log::debug!(
        "rewrite_import_from: Processing import {:?} in module '{}'",
        import_from
            .module
            .as_ref()
            .map(ruff_python_ast::Identifier::as_str),
        current_module
    );
    log::debug!(
        "  Importing names: {:?}",
        import_from
            .names
            .iter()
            .map(|a| (
                a.name.as_str(),
                a.asname.as_ref().map(ruff_python_ast::Identifier::as_str)
            ))
            .collect::<Vec<_>>()
    );
    log::trace!("  bundled_modules size: {}", bundler.bundled_modules.len());
    log::trace!("  inlined_modules size: {}", bundler.inlined_modules.len());
    let resolved_module_name = if import_from.level > 0 {
        module_path.and_then(|path| {
            bundler.resolver.resolve_relative_to_absolute_module_name(
                import_from.level,
                import_from
                    .module
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str),
                path,
            )
        })
    } else {
        import_from
            .module
            .as_ref()
            .map(std::string::ToString::to_string)
    };

    let Some(module_name) = resolved_module_name else {
        // If we can't resolve the module, return the original import
        log::warn!(
            "Could not resolve module name for import {:?}, keeping original import",
            import_from
                .module
                .as_ref()
                .map(ruff_python_ast::Identifier::as_str)
        );
        return vec![Stmt::ImportFrom(import_from)];
    };

    if !bundler
        .get_module_id(&module_name)
        .is_some_and(|id| bundler.bundled_modules.contains(&id))
    {
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
                if bundler
                    .get_module_id(&full_module_path)
                    .is_some_and(|id| bundler.bundled_modules.contains(&id))
                {
                    log::debug!("    - {full_module_path}");
                }
            }
            // Transform each submodule import
            return crate::code_generator::namespace_manager::transform_namespace_package_imports(
                bundler,
                import_from,
                &module_name,
                symbol_renames,
            );
        }

        // Check if this module is inlined
        if bundler
            .get_module_id(&module_name)
            .is_some_and(|id| bundler.inlined_modules.contains(&id))
        {
            log::debug!(
                "Module '{module_name}' is an inlined module, \
                 inside_wrapper_init={inside_wrapper_init}"
            );
            // Handle imports from inlined modules
            return handle_imports_from_inlined_module_with_context(
                bundler,
                &import_from,
                &module_name,
                symbol_renames,
                inside_wrapper_init,
                Some(current_module),
            );
        }

        // Check if this module is in the module_registry (wrapper module)
        // A module is a wrapper if it's bundled but NOT inlined
        if bundler.get_module_id(&module_name).is_some_and(|id| {
            bundler.bundled_modules.contains(&id) && !bundler.inlined_modules.contains(&id)
        }) {
            log::debug!("Module '{module_name}' is a wrapper module in module_registry");
            // This is a wrapper module, we need to transform it
            return bundler.transform_bundled_import_from_multiple_with_current_module(
                &import_from,
                &module_name,
                inside_wrapper_init,
                Some(current_module),
                symbol_renames,
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
        bundler
            .get_module_id(&module_name)
            .is_some_and(|id| bundler.bundled_modules.contains(&id)
                && !bundler.inlined_modules.contains(&id))
    );

    // Check if this module is in the registry (wrapper approach)
    // A module is a wrapper if it's bundled but NOT inlined
    if bundler.get_module_id(&module_name).is_some_and(|id| {
        bundler.bundled_modules.contains(&id) && !bundler.inlined_modules.contains(&id)
    }) {
        // Module uses wrapper approach - transform to sys.modules access
        // For relative imports, we need to create an absolute import
        let mut absolute_import = import_from.clone();
        if import_from.level > 0 {
            // Convert relative import to absolute
            absolute_import.level = 0;
            absolute_import.module = Some(Identifier::new(&module_name, TextRange::default()));
        }
        bundler.transform_bundled_import_from_multiple_with_current_module(
            &absolute_import,
            &module_name,
            inside_wrapper_init,
            Some(current_module),
            symbol_renames,
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
            return crate::code_generator::namespace_manager::transform_namespace_package_imports(
                bundler,
                import_from,
                &module_name,
                symbol_renames,
            );
        }

        // Module was inlined - create assignments for imported symbols
        log::debug!(
            "Module '{module_name}' was inlined, creating assignments for imported symbols"
        );

        let (assignments, namespace_requirements) =
            crate::code_generator::module_registry::create_assignments_for_inlined_imports(
                &import_from,
                &module_name,
                symbol_renames,
                bundler.module_info_registry,
                &bundler.inlined_modules,
                &bundler.bundled_modules,
                bundler.resolver,
                python_version,
            );

        // Check for unregistered namespaces - this indicates a bug in pre-detection
        let unregistered_namespaces: Vec<_> = namespace_requirements
            .iter()
            .filter(|ns_req| !bundler.namespace_registry.contains_key(&ns_req.var_name))
            .collect();

        assert!(
            unregistered_namespaces.is_empty(),
            "Unregistered namespaces detected: {:?}. This indicates a bug in \
             detect_namespace_requirements_from_imports",
            unregistered_namespaces
                .iter()
                .map(|ns| format!("{} (var: {})", ns.path, ns.var_name))
                .collect::<Vec<_>>()
        );

        // The namespaces are now pre-created by detect_namespace_requirements_from_imports
        // and the aliases are handled by create_assignments_for_inlined_imports,
        // so we just return the assignments
        assignments
    }
}

/// Resolve a relative import with context
///
/// This function resolves relative imports (e.g., `from . import foo` or `from ..bar import baz`)
/// to absolute module names based on the current module and its file path.
/// Handle imports from inlined modules
pub(super) fn handle_imports_from_inlined_module_with_context(
    bundler: &Bundler,
    import_from: &StmtImportFrom,
    module_name: &str,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    is_wrapper_init: bool,
    current_module: Option<&str>,
) -> Vec<Stmt> {
    let module_id = bundler
        .get_module_id(module_name)
        .expect("Module ID must exist for inlined module");
    log::debug!(
        "handle_imports_from_inlined_module_with_context: module_name={}, available_renames={:?}",
        module_name,
        symbol_renames.get(&module_id)
    );
    let mut result_stmts = Vec::new();

    // Check if this is a wildcard import
    if import_from.names.len() == 1 && import_from.names[0].name.as_str() == "*" {
        // Handle wildcard import from inlined module
        log::debug!("Handling wildcard import from inlined module '{module_name}'");

        // Get the module's exports (either from __all__ or all non-private symbols)
        let module_id = bundler
            .get_module_id(module_name)
            .expect("Module ID must exist for inlined module");
        let module_exports = if let Some(Some(export_list)) = bundler.module_exports.get(&module_id)
        {
            // Module has __all__ defined, use it
            export_list.clone()
        } else if let Some(semantic_exports) = bundler.semantic_exports.get(&module_id) {
            // Use semantic exports from analysis
            semantic_exports.iter().cloned().collect()
        } else {
            // No export information available
            log::warn!(
                "No export information available for inlined module '{module_name}' with \
                     wildcard import"
            );
            return result_stmts;
        };

        log::debug!(
            "Generating wildcard import assignments for {} symbols from inlined module '{}'",
            module_exports.len(),
            module_name
        );

        // Get symbol renames for this module
        let module_renames = symbol_renames.get(&module_id);

        // Cache explicit __all__ (if any) to avoid repeated lookups
        let explicit_all = bundler
            .module_exports
            .get(&module_id)
            .and_then(|exports| exports.as_ref());

        for symbol_name in &module_exports {
            // Skip private symbols unless explicitly in __all__
            if symbol_name.starts_with('_')
                && !explicit_all.is_some_and(|all| all.contains(symbol_name))
            {
                continue;
            }

            // Check if the source symbol was tree-shaken
            if !bundler.is_symbol_kept_by_tree_shaking(module_id, symbol_name) {
                log::debug!(
                    "Skipping wildcard import for tree-shaken symbol '{symbol_name}' from module \
                     '{module_name}'"
                );
                continue;
            }

            // Get the renamed symbol name if it was renamed
            let renamed_symbol = if let Some(renames) = module_renames {
                renames
                    .get(symbol_name)
                    .cloned()
                    .unwrap_or_else(|| symbol_name.clone())
            } else {
                symbol_name.clone()
            };

            // For wildcard imports, create assignments only when necessary
            if renamed_symbol == *symbol_name {
                // Symbol wasn't renamed - skip creating self-referential assignments
                // When importing from an inlined module, the symbols are already
                // defined in the current scope from the inlining process.
                // Creating assignments like `BaseLoader = BaseLoader` is unnecessary
                // and can cause forward reference errors.
                log::debug!(
                    "Skipping self-referential assignment for non-renamed symbol '{symbol_name}' from inlined module"
                );
            } else {
                // Symbol was renamed, create an alias assignment
                result_stmts.push(statements::simple_assign(
                    symbol_name,
                    expressions::name(&renamed_symbol, ExprContext::Load),
                ));
                log::debug!(
                    "Created wildcard import alias for renamed symbol: {symbol_name} = \
                     {renamed_symbol}"
                );
            }
        }

        return result_stmts;
    }

    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

        // First check if we're importing a submodule (e.g., from package import submodule)
        let full_module_path = format!("{module_name}.{imported_name}");
        if bundler
            .get_module_id(&full_module_path)
            .is_some_and(|id| bundler.bundled_modules.contains(&id))
        {
            // This is importing a submodule, not a symbol
            // This should be handled by transform_namespace_package_imports instead
            log::debug!(
                "Skipping submodule import '{imported_name}' from '{module_name}' - should be \
                 handled elsewhere"
            );
            continue;
        }

        // Prefer precise re-export detection from inlined submodules
        let renamed_symbol = if let Some((source_module, source_symbol)) =
            bundler.is_symbol_from_inlined_submodule(module_name, imported_name)
        {
            // Apply symbol renames from the source module if they exist
            let source_module_id = bundler
                .get_module_id(&source_module)
                .expect("Source module should exist");
            let global_name = symbol_renames
                .get(&source_module_id)
                .and_then(|renames| renames.get(&source_symbol))
                .cloned()
                .unwrap_or(source_symbol);

            log::debug!(
                "Resolved re-exported symbol via inlined submodule: {module_name}.{imported_name} -> {global_name}"
            );
            global_name
        } else {
            // Fallback: package re-export heuristic only if there is no explicit rename
            let is_package_reexport = is_package_init_reexport(bundler, module_name);
            let has_rename = symbol_renames
                .get(&module_id)
                .and_then(|renames| renames.get(imported_name))
                .is_some();

            log::debug!(
                "  is_package_reexport for module '{module_name}': {is_package_reexport}, has_rename: {has_rename}"
            );

            if is_package_reexport && !has_rename {
                log::debug!(
                    "Using original name '{imported_name}' for symbol imported from package \
                     '{module_name}' (no rename found)"
                );
                imported_name.to_string()
            } else {
                symbol_renames
                    .get(&module_id)
                    .and_then(|renames| renames.get(imported_name))
                    .cloned()
                    .unwrap_or_else(|| imported_name.to_string())
            }
        };

        log::debug!(
            "Processing import: module={}, imported_name={}, local_name={}, renamed_symbol={}, available_renames={:?}",
            module_name,
            imported_name,
            local_name,
            renamed_symbol,
            symbol_renames.get(&module_id)
        );

        // Check if the source symbol was tree-shaken
        if !bundler.is_symbol_kept_by_tree_shaking(module_id, imported_name) {
            log::debug!(
                "Skipping import assignment for tree-shaken symbol '{imported_name}' from module \
                 '{module_name}'"
            );
            continue;
        }

        // Handle wrapper init functions specially
        if is_wrapper_init {
            // In wrapper init functions, always set the module attribute to the resolved symbol
            if let Some(current_mod) = current_module {
                let module_var =
                    crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                        current_mod,
                    );
                log::debug!(
                    "Creating module attribute assignment in wrapper init: {module_var}.{local_name} = {renamed_symbol}"
                );
                result_stmts.push(
                    crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                        &module_var,
                        local_name,
                        &renamed_symbol,
                    ),
                );
            } else {
                log::warn!(
                    "is_wrapper_init is true but current_module is None, skipping module attribute assignment"
                );
            }
            // Keep a local alias only when renamed, to preserve intra-init references
            if local_name != renamed_symbol {
                log::debug!("Creating local alias: {local_name} = {renamed_symbol}");
                result_stmts.push(statements::simple_assign(
                    local_name,
                    expressions::name(&renamed_symbol, ExprContext::Load),
                ));
            }
        } else if local_name != renamed_symbol {
            // For non-wrapper contexts, only create assignment if names differ
            log::debug!("Creating assignment: {local_name} = {renamed_symbol}");
            result_stmts.push(statements::simple_assign(
                local_name,
                expressions::name(&renamed_symbol, ExprContext::Load),
            ));
        }
    }

    result_stmts
}

/// Check if a symbol is likely a re-export from a package __init__.py
fn is_package_init_reexport(bundler: &Bundler, module_name: &str) -> bool {
    // Special handling for package __init__.py files
    // If we're importing from "greetings" and there's a "greetings.X" module
    // that could be the source of the symbol

    // For now, check if this looks like a package (no dots) and if there are
    // any inlined submodules
    if !module_name.contains('.') {
        // Check if any inlined module starts with module_name.
        if bundler.inlined_modules.iter().any(|inlined_id| {
            bundler
                .resolver
                .get_module_name(*inlined_id)
                .is_some_and(|name| name.starts_with(&format!("{module_name}.")))
        }) {
            log::debug!("Module '{module_name}' appears to be a package with inlined submodules");
            // For the specific case of greetings/__init__.py importing from
            // greetings.english, we assume the symbol should use its
            // original name
            return true;
        }
    }
    false
}
