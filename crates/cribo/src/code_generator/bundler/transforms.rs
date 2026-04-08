//! AST statement/expression transformation and global variable lifting.

use ruff_python_ast::{ExceptHandler, Expr, ExprContext, ModModule, Stmt, StmtFunctionDef};
use ruff_text_size::TextRange;

use super::Bundler;
use crate::{
    ast_builder::{expressions, other, statements},
    code_generator::{expression_handlers, module_registry::sanitize_module_name_for_identifier},
    types::{FxIndexMap, FxIndexSet},
    visitors::LocalVarCollector,
};

/// Parameters for transforming functions with lifted globals
struct TransformFunctionParams<'a> {
    lifted_names: &'a FxIndexMap<String, String>,
    global_info: &'a crate::symbol_conflict_resolver::ModuleGlobalInfo,
    function_globals: &'a FxIndexSet<String>,
    module_name: Option<&'a str>,
}

impl Bundler<'_> {
    /// Process module body recursively to handle conditional imports
    pub(crate) fn process_body_recursive(
        &self,
        body: Vec<Stmt>,
        module_name: &str,
        module_scope_symbols: Option<&FxIndexSet<String>>,
    ) -> Vec<Stmt> {
        self.process_body_recursive_impl(body, module_name, module_scope_symbols, false)
    }

    /// Implementation of `process_body_recursive` with conditional context tracking
    fn process_body_recursive_impl(
        &self,
        body: Vec<Stmt>,
        module_name: &str,
        module_scope_symbols: Option<&FxIndexSet<String>>,
        in_conditional_context: bool,
    ) -> Vec<Stmt> {
        let mut result = Vec::new();

        for stmt in body {
            match &stmt {
                Stmt::If(if_stmt) => {
                    // Process if body recursively (inside conditional context)
                    let mut processed_body = self.process_body_recursive_impl(
                        if_stmt.body.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Check if this is a TYPE_CHECKING block and ensure it has a body
                    if processed_body.is_empty() && Self::is_type_checking_condition(&if_stmt.test)
                    {
                        log::debug!("Adding pass statement to empty TYPE_CHECKING block");
                        // Add a pass statement to avoid IndentationError
                        processed_body.push(statements::pass());
                    }

                    // Process elif/else clauses
                    let processed_elif_else = if_stmt
                        .elif_else_clauses
                        .iter()
                        .map(|clause| {
                            let mut processed_clause_body = self.process_body_recursive_impl(
                                clause.body.clone(),
                                module_name,
                                module_scope_symbols,
                                true,
                            );

                            // Ensure non-empty body for elif/else clauses too
                            if processed_clause_body.is_empty() {
                                log::debug!("Adding pass statement to empty elif/else clause");
                                processed_clause_body.push(statements::pass());
                            }

                            ruff_python_ast::ElifElseClause {
                                node_index: clause.node_index.clone(),
                                test: clause.test.clone(),
                                body: processed_clause_body,
                                range: clause.range,
                            }
                        })
                        .collect();

                    // Create new if statement with processed bodies
                    let new_if = ruff_python_ast::StmtIf {
                        node_index: if_stmt.node_index.clone(),
                        test: if_stmt.test.clone(),
                        body: processed_body,
                        elif_else_clauses: processed_elif_else,
                        range: if_stmt.range,
                    };

                    result.push(Stmt::If(new_if));
                }
                Stmt::Try(try_stmt) => {
                    // Process try body recursively (inside conditional context)
                    let processed_body = self.process_body_recursive_impl(
                        try_stmt.body.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Process handlers
                    let processed_handlers = try_stmt
                        .handlers
                        .iter()
                        .map(|handler| {
                            let ExceptHandler::ExceptHandler(handler) = handler;
                            let processed_handler_body = self.process_body_recursive_impl(
                                handler.body.clone(),
                                module_name,
                                module_scope_symbols,
                                true,
                            );
                            ExceptHandler::ExceptHandler(
                                ruff_python_ast::ExceptHandlerExceptHandler {
                                    node_index: handler.node_index.clone(),
                                    type_: handler.type_.clone(),
                                    name: handler.name.clone(),
                                    body: processed_handler_body,
                                    range: handler.range,
                                },
                            )
                        })
                        .collect();

                    // Process orelse (inside conditional context)
                    let processed_orelse = self.process_body_recursive_impl(
                        try_stmt.orelse.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Process finalbody (inside conditional context)
                    let processed_finalbody = self.process_body_recursive_impl(
                        try_stmt.finalbody.clone(),
                        module_name,
                        module_scope_symbols,
                        true,
                    );

                    // Create new try statement
                    let new_try = ruff_python_ast::StmtTry {
                        node_index: try_stmt.node_index.clone(),
                        body: processed_body,
                        handlers: processed_handlers,
                        orelse: processed_orelse,
                        finalbody: processed_finalbody,
                        is_star: try_stmt.is_star,
                        range: try_stmt.range,
                    };

                    result.push(Stmt::Try(new_try));
                }
                Stmt::ImportFrom(import_from) => {
                    // Skip __future__ imports
                    if import_from
                        .module
                        .as_ref()
                        .map(ruff_python_ast::Identifier::as_str)
                        != Some("__future__")
                    {
                        // Check if this is a relative import that needs special handling
                        // Skip wildcard cases to preserve semantics
                        let has_wildcard = import_from.names.iter().any(|a| a.name.as_str() == "*");
                        let handled = if import_from.level > 0 && !has_wildcard {
                            // For relative imports, transform same-module case to explicit
                            // assignments
                            let from_mod = import_from
                                .module
                                .as_ref()
                                .map_or("", ruff_python_ast::Identifier::as_str);
                            let resolved = self.resolve_from_import_target(
                                module_name,
                                from_mod,
                                import_from.level,
                            );
                            if resolved == module_name {
                                let parent_pkg = self.derive_parent_package_for_relative_import(
                                    module_name,
                                    import_from.level,
                                );
                                crate::code_generator::import_transformer::handlers::relative::transform_relative_import_aliases(
                                    self,
                                    import_from,
                                    &parent_pkg, // correct parent package
                                    module_name, // current module
                                    &mut result,
                                    true,        // add module attributes
                                );
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if handled {
                            // Helper emitted the local bindings and module attrs; skip fall-through
                            // to avoid duplicates
                            continue;
                        }
                        result.push(stmt.clone());

                        // Add module attribute assignments for imported symbols when in conditional
                        // context
                        if in_conditional_context {
                            for alias in &import_from.names {
                                // Skip wildcard imports — can't create module.* = *
                                if alias.name.as_str() == "*" {
                                    continue;
                                }
                                let local_name =
                                    alias.asname.as_ref().unwrap_or(&alias.name).as_str();

                                log::debug!(
                                    "Checking conditional ImportFrom symbol '{local_name}' in \
                                     module '{module_name}' for export"
                                );

                                // For conditional imports, always add module attributes for
                                // non-private symbols regardless of
                                // __all__ restrictions, since they can be defined at runtime
                                if local_name.starts_with('_') {
                                    log::debug!(
                                        "NOT exporting conditional ImportFrom symbol \
                                         '{local_name}' in module '{module_name}' (private symbol)"
                                    );
                                } else {
                                    log::debug!(
                                        "Adding module.{local_name} = {local_name} after \
                                         conditional import (bypassing __all__ restrictions)"
                                    );
                                    let module_var =
                                        sanitize_module_name_for_identifier(module_name);
                                    result.push(
                                        crate::code_generator::module_registry::create_module_attr_assignment(
                                            &module_var,
                                            local_name,
                                        ),
                                    );
                                }
                            }
                        } else {
                            // Non-conditional imports
                            self.handle_nonconditional_from_import_exports(
                                import_from,
                                module_scope_symbols,
                                module_name,
                                &mut result,
                            );
                        }
                    }
                }
                Stmt::Import(import_stmt) => {
                    // Add the import statement itself
                    result.push(stmt.clone());

                    // Add module attribute assignments for imported modules when in conditional
                    // context
                    if in_conditional_context {
                        for alias in &import_stmt.names {
                            let imported_name = alias.name.as_str();
                            let local_name = alias
                                .asname
                                .as_ref()
                                .map_or(imported_name, ruff_python_ast::Identifier::as_str);

                            // For conditional imports, always add module attributes for non-private
                            // symbols regardless of __all__
                            // restrictions, since they can be defined at runtime
                            // Only handle simple (non-dotted) names that can be valid attribute
                            // names
                            if !local_name.starts_with('_')
                                && !local_name.contains('.')
                                && !local_name.is_empty()
                                && !local_name.as_bytes()[0].is_ascii_digit()
                                && local_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                            {
                                log::debug!(
                                    "Adding module.{local_name} = {local_name} after conditional \
                                     import (bypassing __all__ restrictions)"
                                );
                                let module_var = sanitize_module_name_for_identifier(module_name);
                                result.push(
                                    crate::code_generator::module_registry::create_module_attr_assignment(
                                        &module_var,
                                        local_name
                                    ),
                                );
                            } else {
                                log::debug!(
                                    "NOT exporting conditional Import symbol '{local_name}' in \
                                     module '{module_name}' (complex or invalid attribute name)"
                                );
                            }
                        }
                    }
                }
                Stmt::Assign(assign) => {
                    // Add the assignment itself
                    result.push(stmt.clone());

                    // Check if this assignment should create a module attribute when in conditional
                    // context
                    if in_conditional_context
                        && let Some(name) =
                            expression_handlers::extract_simple_assign_target(assign)
                    {
                        // For conditional assignments, always add module attributes for non-private
                        // symbols regardless of __all__ restrictions, since
                        // they can be defined at runtime
                        if !name.starts_with('_') {
                            log::debug!(
                                "Adding module.{name} = {name} after conditional assignment \
                                 (bypassing __all__ restrictions)"
                            );
                            let module_var = sanitize_module_name_for_identifier(module_name);
                            result.push(
                                crate::code_generator::module_registry::create_module_attr_assignment(
                                    &module_var,
                                    &name
                                ),
                            );
                        }
                    }
                }
                _ => {
                    // For other statements, just add them as-is
                    result.push(stmt.clone());
                }
            }
        }

        result
    }

    /// Transform nested functions to use module attributes for module-level variables,
    /// including lifted variables (they access through module attrs unless they declare global)
    pub(crate) fn transform_nested_function_for_module_vars_with_global_info(
        &self,
        func_def: &mut StmtFunctionDef,
        module_level_vars: &FxIndexSet<String>,
        global_declarations: &FxIndexMap<String, Vec<TextRange>>,
        lifted_names: Option<&FxIndexMap<String, String>>,
        module_var_name: &str,
    ) {
        // First, collect all names in this function scope that must NOT be rewritten
        // (globals declared here or nonlocals captured from an outer function)
        let mut global_vars = FxIndexSet::default();

        // Build a reverse map for lifted names to avoid O(n) scans per name
        let lifted_to_original: Option<FxIndexMap<String, String>> = lifted_names.map(|m| {
            m.iter()
                .map(|(orig, lift)| (lift.clone(), orig.clone()))
                .collect()
        });

        for stmt in &func_def.body {
            if let Stmt::Global(global_stmt) = stmt {
                for name in &global_stmt.names {
                    let var_name = name.to_string();

                    // The global statement might have already been rewritten to use lifted names
                    // (e.g., "_cribo_httpx__transports_default_HTTPCORE_EXC_MAP")
                    // We need to check both the lifted name AND the original name

                    // First check if this is directly a global declaration
                    if global_declarations.contains_key(&var_name) {
                        global_vars.insert(var_name.clone());
                    }

                    // Also check if this is a lifted name via reverse lookup
                    if let Some(rev) = &lifted_to_original
                        && let Some(original_name) = rev.get(var_name.as_str())
                    {
                        // Exclude both original and lifted names from transformation
                        global_vars.insert(original_name.clone());
                        global_vars.insert(var_name.clone());
                    }
                }
            } else if let Stmt::Nonlocal(nonlocal_stmt) = stmt {
                // Nonlocals are not module-level; exclude them from module attribute rewrites
                for name in &nonlocal_stmt.names {
                    global_vars.insert(name.to_string());
                }
            }
        }

        // Now transform the function, but skip variables that are declared as global
        // Create a modified set of module_level_vars that excludes the global vars
        let mut filtered_module_vars = module_level_vars.clone();
        for global_var in &global_vars {
            filtered_module_vars.swap_remove(global_var);
        }

        // Transform using the filtered set
        self.transform_nested_function_for_module_vars(
            func_def,
            &filtered_module_vars,
            module_var_name,
        );
    }

    /// Transform nested functions to use module attributes for module-level variables
    pub(crate) fn transform_nested_function_for_module_vars(
        &self,
        func_def: &mut StmtFunctionDef,
        module_level_vars: &FxIndexSet<String>,
        module_var_name: &str,
    ) {
        // First, collect all global declarations in this function
        let mut global_vars = FxIndexSet::default();
        for stmt in &func_def.body {
            if let Stmt::Global(global_stmt) = stmt {
                for name in &global_stmt.names {
                    global_vars.insert(name.to_string());
                }
            }
        }

        // Collect local variables defined in this function
        let mut local_vars = FxIndexSet::default();

        // Add function parameters to local variables
        for param in &func_def.parameters.args {
            local_vars.insert(param.parameter.name.to_string());
        }
        for param in &func_def.parameters.posonlyargs {
            local_vars.insert(param.parameter.name.to_string());
        }
        for param in &func_def.parameters.kwonlyargs {
            local_vars.insert(param.parameter.name.to_string());
        }
        if let Some(ref vararg) = func_def.parameters.vararg {
            local_vars.insert(vararg.name.to_string());
        }
        if let Some(ref kwarg) = func_def.parameters.kwarg {
            local_vars.insert(kwarg.name.to_string());
        }

        // Collect all local variables assigned in the function body
        // Pass global_vars to exclude them from local_vars
        let mut collector = LocalVarCollector::new(&mut local_vars, &global_vars);
        collector.collect_from_stmts(&func_def.body);

        // Transform the function body, excluding local variables
        for stmt in &mut func_def.body {
            self.transform_stmt_for_module_vars_with_locals(
                stmt,
                module_level_vars,
                &local_vars,
                module_var_name,
            );
        }
    }

    /// Transform a statement with awareness of local variables
    fn transform_stmt_for_module_vars_with_locals(
        &self,
        stmt: &mut Stmt,
        module_level_vars: &FxIndexSet<String>,
        local_vars: &FxIndexSet<String>,
        module_var_name: &str,
    ) {
        match stmt {
            Stmt::FunctionDef(nested_func) => {
                // Recursively transform nested functions
                self.transform_nested_function_for_module_vars(
                    nested_func,
                    module_level_vars,
                    module_var_name,
                );
            }
            Stmt::Assign(assign) => {
                // Transform assignment targets and values
                for target in &mut assign.targets {
                    Self::transform_expr_for_module_vars_with_locals(
                        target,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                Self::transform_expr_for_module_vars_with_locals(
                    &mut assign.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Stmt::Expr(expr_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut expr_stmt.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    Self::transform_expr_for_module_vars_with_locals(
                        value,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::If(if_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut if_stmt.test,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for stmt in &mut if_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(condition) = &mut clause.test {
                        Self::transform_expr_for_module_vars_with_locals(
                            condition,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                    for stmt in &mut clause.body {
                        self.transform_stmt_for_module_vars_with_locals(
                            stmt,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                }
            }
            Stmt::For(for_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut for_stmt.target,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut for_stmt.iter,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for stmt in &mut for_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for stmt in &mut for_stmt.orelse {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::While(while_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut while_stmt.test,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for stmt in &mut while_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for stmt in &mut while_stmt.orelse {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::Try(try_stmt) => {
                for stmt in &mut try_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for handler in &mut try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    for stmt in &mut eh.body {
                        self.transform_stmt_for_module_vars_with_locals(
                            stmt,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                }
                for stmt in &mut try_stmt.orelse {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for stmt in &mut try_stmt.finalbody {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::AugAssign(aug_assign) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut aug_assign.target,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut aug_assign.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Stmt::AnnAssign(ann_assign) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut ann_assign.target,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut ann_assign.annotation,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                if let Some(value) = &mut ann_assign.value {
                    Self::transform_expr_for_module_vars_with_locals(
                        value,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::ClassDef(class_def) => {
                // Transform decorators and base classes
                for dec in &mut class_def.decorator_list {
                    Self::transform_expr_for_module_vars_with_locals(
                        &mut dec.expression,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                if let Some(args) = &mut class_def.arguments {
                    for base in &mut args.args {
                        Self::transform_expr_for_module_vars_with_locals(
                            base,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                    for kw in &mut args.keywords {
                        Self::transform_expr_for_module_vars_with_locals(
                            &mut kw.value,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                }
                // Recurse into class body (class scope executes at definition time)
                for stmt in &mut class_def.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::With(with_stmt) => {
                for item in &mut with_stmt.items {
                    Self::transform_expr_for_module_vars_with_locals(
                        &mut item.context_expr,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                    if let Some(var) = &mut item.optional_vars {
                        Self::transform_expr_for_module_vars_with_locals(
                            var,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                }
                for stmt in &mut with_stmt.body {
                    self.transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::Match(match_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut match_stmt.subject,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for case in &mut match_stmt.cases {
                    if let Some(guard) = &mut case.guard {
                        Self::transform_expr_for_module_vars_with_locals(
                            guard,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                    for stmt in &mut case.body {
                        self.transform_stmt_for_module_vars_with_locals(
                            stmt,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                }
            }
            Stmt::Assert(assert_stmt) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut assert_stmt.test,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                if let Some(msg) = &mut assert_stmt.msg {
                    Self::transform_expr_for_module_vars_with_locals(
                        msg,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::Delete(delete_stmt) => {
                for target in &mut delete_stmt.targets {
                    Self::transform_expr_for_module_vars_with_locals(
                        target,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Stmt::Raise(raise_stmt) => {
                if let Some(exc) = &mut raise_stmt.exc {
                    Self::transform_expr_for_module_vars_with_locals(
                        exc,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                if let Some(cause) = &mut raise_stmt.cause {
                    Self::transform_expr_for_module_vars_with_locals(
                        cause,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            _ => {
                // Remaining: Import, ImportFrom, Pass, Break, Continue, Global, Nonlocal,
                // TypeAlias, IpyEscapeCommand — none contain name expressions to rewrite
            }
        }
    }

    /// Transform an expression with awareness of local variables
    fn transform_expr_for_module_vars_with_locals(
        expr: &mut Expr,
        module_level_vars: &FxIndexSet<String>,
        local_vars: &FxIndexSet<String>,
        module_var_name: &str,
    ) {
        match expr {
            Expr::Name(name_expr) => {
                let name_str = name_expr.id.as_str();

                // Special case: transform __name__ to module.__name__
                if name_str == "__name__" && matches!(name_expr.ctx, ExprContext::Load) {
                    // Transform __name__ -> module.__name__
                    *expr = expressions::attribute(
                        expressions::name(module_var_name, ExprContext::Load),
                        "__name__",
                        ExprContext::Load,
                    );
                }
                // If this is a module-level variable being read AND NOT a local variable AND NOT a
                // builtin, transform to module.var
                else if module_level_vars.contains(name_str)
                    && !local_vars.contains(name_str)
                    && !ruff_python_stdlib::builtins::python_builtins(u8::MAX, false)
                        .any(|b| b == name_str)
                    && matches!(name_expr.ctx, ExprContext::Load)
                {
                    // Transform foo -> module.foo
                    *expr = expressions::attribute(
                        expressions::name(module_var_name, ExprContext::Load),
                        name_str,
                        ExprContext::Load,
                    );
                }
            }
            Expr::Call(call) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut call.func,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for arg in &mut call.arguments.args {
                    Self::transform_expr_for_module_vars_with_locals(
                        arg,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
                for keyword in &mut call.arguments.keywords {
                    Self::transform_expr_for_module_vars_with_locals(
                        &mut keyword.value,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::BinOp(binop) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut binop.left,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut binop.right,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Dict(dict) => {
                for item in &mut dict.items {
                    if let Some(key) = &mut item.key {
                        Self::transform_expr_for_module_vars_with_locals(
                            key,
                            module_level_vars,
                            local_vars,
                            module_var_name,
                        );
                    }
                    Self::transform_expr_for_module_vars_with_locals(
                        &mut item.value,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::List(list_expr) => {
                for elem in &mut list_expr.elts {
                    Self::transform_expr_for_module_vars_with_locals(
                        elem,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::Attribute(attr) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut attr.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Subscript(subscript) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut subscript.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut subscript.slice,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::BoolOp(bool_op) => {
                for val in &mut bool_op.values {
                    Self::transform_expr_for_module_vars_with_locals(
                        val,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::Compare(compare) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut compare.left,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                for comp in &mut compare.comparators {
                    Self::transform_expr_for_module_vars_with_locals(
                        comp,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::UnaryOp(unary_op) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut unary_op.operand,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::If(if_expr) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut if_expr.test,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut if_expr.body,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut if_expr.orelse,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Tuple(tuple) => {
                for elt in &mut tuple.elts {
                    Self::transform_expr_for_module_vars_with_locals(
                        elt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::Set(set) => {
                for elt in &mut set.elts {
                    Self::transform_expr_for_module_vars_with_locals(
                        elt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::Starred(starred) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut starred.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Await(await_expr) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut await_expr.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Yield(yield_expr) => {
                if let Some(value) = &mut yield_expr.value {
                    Self::transform_expr_for_module_vars_with_locals(
                        value,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                    );
                }
            }
            Expr::YieldFrom(yield_from) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut yield_from.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Lambda(lambda) => {
                // Lambda defaults are evaluated at definition time
                if let Some(params) = &mut lambda.parameters {
                    for param in params
                        .args
                        .iter_mut()
                        .chain(params.posonlyargs.iter_mut())
                        .chain(params.kwonlyargs.iter_mut())
                    {
                        if let Some(default) = &mut param.default {
                            Self::transform_expr_for_module_vars_with_locals(
                                default,
                                module_level_vars,
                                local_vars,
                                module_var_name,
                            );
                        }
                    }
                }
                // Lambda body is deferred — skip (like function bodies)
            }
            Expr::ListComp(comp) => {
                let comp_locals = Self::comprehension_local_vars(&comp.generators, local_vars);
                Self::transform_expr_for_module_vars_with_locals(
                    &mut comp.elt,
                    module_level_vars,
                    &comp_locals,
                    module_var_name,
                );
                Self::transform_comprehension_generators(
                    &mut comp.generators,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::SetComp(comp) => {
                let comp_locals = Self::comprehension_local_vars(&comp.generators, local_vars);
                Self::transform_expr_for_module_vars_with_locals(
                    &mut comp.elt,
                    module_level_vars,
                    &comp_locals,
                    module_var_name,
                );
                Self::transform_comprehension_generators(
                    &mut comp.generators,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::DictComp(comp) => {
                let comp_locals = Self::comprehension_local_vars(&comp.generators, local_vars);
                Self::transform_expr_for_module_vars_with_locals(
                    &mut comp.key,
                    module_level_vars,
                    &comp_locals,
                    module_var_name,
                );
                Self::transform_expr_for_module_vars_with_locals(
                    &mut comp.value,
                    module_level_vars,
                    &comp_locals,
                    module_var_name,
                );
                Self::transform_comprehension_generators(
                    &mut comp.generators,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::Generator(generator_expr) => {
                let comp_locals =
                    Self::comprehension_local_vars(&generator_expr.generators, local_vars);
                Self::transform_expr_for_module_vars_with_locals(
                    &mut generator_expr.elt,
                    module_level_vars,
                    &comp_locals,
                    module_var_name,
                );
                Self::transform_comprehension_generators(
                    &mut generator_expr.generators,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            Expr::FString(fstring) => {
                for part in &mut fstring.value {
                    if let ruff_python_ast::FStringPart::FString(f) = part {
                        for elem in &mut *f.elements {
                            if let Some(interp) = elem.as_interpolation_mut() {
                                Self::transform_expr_for_module_vars_with_locals(
                                    &mut interp.expression,
                                    module_level_vars,
                                    local_vars,
                                    module_var_name,
                                );
                            }
                        }
                    }
                }
            }
            Expr::Named(named) => {
                Self::transform_expr_for_module_vars_with_locals(
                    &mut named.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                );
            }
            _ => {
                // Remaining: literals (NumberLiteral, StringLiteral, BytesLiteral,
                // BooleanLiteral, NoneLiteral, EllipsisLiteral), Slice, IpyEscapeCommand
                // — none contain rewritable name references
            }
        }
    }

    /// Collect names bound by comprehension generator targets and merge with existing locals.
    /// In Python, comprehension targets shadow outer scope names (PEP 289/572).
    fn comprehension_local_vars(
        generators: &[ruff_python_ast::Comprehension],
        existing_locals: &FxIndexSet<String>,
    ) -> FxIndexSet<String> {
        let mut locals = existing_locals.clone();
        for g in generators {
            for name in crate::visitors::utils::collect_names_from_assignment_target(&g.target) {
                locals.insert(name.to_owned());
            }
        }
        locals
    }

    /// Transform comprehension generators with incremental scoping per Python semantics.
    ///
    /// In Python, each generator's `iter` is evaluated before that generator's target is
    /// bound. The first generator's `iter` uses the enclosing scope (no comp targets),
    /// subsequent generators' `iter` see only preceding targets. Each generator's `ifs`
    /// see the current target plus all preceding ones.
    fn transform_comprehension_generators(
        generators: &mut [ruff_python_ast::Comprehension],
        module_level_vars: &FxIndexSet<String>,
        local_vars: &FxIndexSet<String>,
        module_var_name: &str,
    ) {
        let mut accumulated_locals = local_vars.clone();
        for generator in generators.iter_mut() {
            // Transform iter with only preceding targets (not current)
            Self::transform_expr_for_module_vars_with_locals(
                &mut generator.iter,
                module_level_vars,
                &accumulated_locals,
                module_var_name,
            );
            // Add current generator's target names before processing ifs
            for name in
                crate::visitors::utils::collect_names_from_assignment_target(&generator.target)
            {
                accumulated_locals.insert(name.to_owned());
            }
            // Transform ifs with current + preceding targets
            for if_clause in &mut generator.ifs {
                Self::transform_expr_for_module_vars_with_locals(
                    if_clause,
                    module_level_vars,
                    &accumulated_locals,
                    module_var_name,
                );
            }
        }
    }

    /// Transform AST to use lifted globals
    pub(crate) fn transform_ast_with_lifted_globals(
        &self,
        ast: &mut ModModule,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::symbol_conflict_resolver::ModuleGlobalInfo,
        module_name: Option<&str>,
    ) {
        // Transform all statements that use global declarations
        for stmt in &mut ast.body {
            self.transform_stmt_for_lifted_globals(
                stmt,
                lifted_names,
                global_info,
                None,
                module_name,
            );
        }
    }

    /// Transform a statement to use lifted globals
    fn transform_stmt_for_lifted_globals(
        &self,
        stmt: &mut Stmt,
        lifted_names: &FxIndexMap<String, String>,
        global_info: &crate::symbol_conflict_resolver::ModuleGlobalInfo,
        current_function_globals: Option<&FxIndexSet<String>>,
        module_name: Option<&str>,
    ) {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                if global_info
                    .functions_using_globals
                    .contains(&func_def.name.to_string())
                {
                    // This function directly uses globals — rewrite its body
                    let function_globals =
                        crate::visitors::VariableCollector::collect_function_globals(
                            &func_def.body,
                        );
                    let params = TransformFunctionParams {
                        lifted_names,
                        global_info,
                        function_globals: &function_globals,
                        module_name,
                    };
                    self.transform_function_body_for_lifted_globals(func_def, &params);
                } else {
                    // Still descend into the body to find nested functions that use globals
                    for stmt in &mut func_def.body {
                        self.transform_stmt_for_lifted_globals(
                            stmt,
                            lifted_names,
                            global_info,
                            current_function_globals,
                            module_name,
                        );
                    }
                }
            }
            Stmt::Assign(assign) => {
                // Transform assignments to use lifted names if they're in a function with global
                // declarations
                for target in &mut assign.targets {
                    expression_handlers::transform_expr_for_lifted_globals(
                        self,
                        target,
                        lifted_names,
                        global_info,
                        current_function_globals,
                    );
                }
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut assign.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::Expr(expr_stmt) => {
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut expr_stmt.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::If(if_stmt) => {
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
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
                        module_name,
                    );
                }
                for clause in &mut if_stmt.elif_else_clauses {
                    if let Some(test_expr) = &mut clause.test {
                        expression_handlers::transform_expr_for_lifted_globals(
                            self,
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
                            module_name,
                        );
                    }
                }
            }
            Stmt::While(while_stmt) => {
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
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
                        module_name,
                    );
                }
                for stmt in &mut while_stmt.orelse {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }
            }
            Stmt::For(for_stmt) => {
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut for_stmt.target,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
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
                        module_name,
                    );
                }
                for stmt in &mut for_stmt.orelse {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }
            }
            Stmt::Return(return_stmt) => {
                if let Some(value) = &mut return_stmt.value {
                    expression_handlers::transform_expr_for_lifted_globals(
                        self,
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
                        module_name,
                    );
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Transform augmented assignments to use lifted names
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut aug_assign.target,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
                expression_handlers::transform_expr_for_lifted_globals(
                    self,
                    &mut aug_assign.value,
                    lifted_names,
                    global_info,
                    current_function_globals,
                );
            }
            Stmt::Try(try_stmt) => {
                // Transform try block body
                for stmt in &mut try_stmt.body {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }

                // Transform exception handlers
                for handler in &mut try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;

                    // Transform the exception type expression if present
                    if let Some(ref mut type_expr) = eh.type_ {
                        expression_handlers::transform_expr_for_lifted_globals(
                            self,
                            type_expr,
                            lifted_names,
                            global_info,
                            current_function_globals,
                        );
                    }

                    // Transform the handler body
                    for stmt in &mut eh.body {
                        self.transform_stmt_for_lifted_globals(
                            stmt,
                            lifted_names,
                            global_info,
                            current_function_globals,
                            module_name,
                        );
                    }
                }

                // Transform orelse block
                for stmt in &mut try_stmt.orelse {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }

                // Transform finally block
                for stmt in &mut try_stmt.finalbody {
                    self.transform_stmt_for_lifted_globals(
                        stmt,
                        lifted_names,
                        global_info,
                        current_function_globals,
                        module_name,
                    );
                }
            }
            _ => {
                // Other statement types handled as needed
            }
        }
    }
}

impl Bundler<'_> {
    /// Transform function body for lifted globals
    fn transform_function_body_for_lifted_globals(
        &self,
        func_def: &mut StmtFunctionDef,
        params: &TransformFunctionParams<'_>,
    ) {
        let mut new_body = Vec::new();
        let old_body = std::mem::take(&mut func_def.body);

        for mut body_stmt in old_body {
            if let Stmt::Global(ref mut global_stmt) = body_stmt {
                // Rewrite global statement to use lifted names
                for name in &mut global_stmt.names {
                    if let Some(lifted_name) = params.lifted_names.get(name.as_str()) {
                        *name = other::identifier(lifted_name);
                    }
                }
                new_body.push(body_stmt);
            } else {
                // Transform other statements recursively with function context
                self.transform_stmt_for_lifted_globals(
                    &mut body_stmt,
                    params.lifted_names,
                    params.global_info,
                    Some(params.function_globals),
                    params.module_name,
                );

                // Collect sync stmts separately to avoid borrow conflict
                // (add_global_sync_if_needed reads &body_stmt while appending)
                let mut sync_stmts = Vec::new();
                self.add_global_sync_if_needed(
                    &body_stmt,
                    params.function_globals,
                    params.lifted_names,
                    &mut sync_stmts,
                    params.module_name,
                );
                new_body.push(body_stmt);
                new_body.extend(sync_stmts);
            }
        }

        func_def.body = new_body;
    }

    /// Add synchronization statements for global variable modifications
    fn add_global_sync_if_needed(
        &self,
        stmt: &Stmt,
        function_globals: &FxIndexSet<String>,
        lifted_names: &FxIndexMap<String, String>,
        new_body: &mut Vec<Stmt>,
        module_name: Option<&str>,
    ) {
        match stmt {
            Stmt::Assign(assign) => {
                // Collect all names from all targets (handles simple and unpacking assignments)
                let mut all_names = Vec::new();
                for target in &assign.targets {
                    all_names.extend(
                        crate::visitors::utils::collect_names_from_assignment_target(target),
                    );
                }

                // Process each collected name
                for var_name in all_names {
                    // The variable name might already be transformed to the lifted name,
                    // so we need to check if it's a lifted variable
                    if let Some(original_name) = lifted_names
                        .iter()
                        .find(|(orig, lifted)| {
                            lifted.as_str() == var_name && function_globals.contains(orig.as_str())
                        })
                        .map(|(orig, _)| orig.as_str())
                    {
                        log::debug!(
                            "Adding sync for assignment to global {var_name}: {var_name} -> \
                             module.{original_name}"
                        );
                        // Add: module.<original_name> = <lifted_name>
                        // Use the provided module name if available, otherwise we can't sync
                        if let Some(mod_name) = module_name {
                            let module_var = sanitize_module_name_for_identifier(mod_name);
                            new_body.push(statements::assign(
                                vec![expressions::attribute(
                                    expressions::name(&module_var, ExprContext::Load),
                                    original_name,
                                    ExprContext::Store,
                                )],
                                expressions::name(var_name, ExprContext::Load),
                            ));
                        }
                    }
                }
            }
            Stmt::AugAssign(aug_assign) => {
                // Collect names from the target (though augmented assignment typically doesn't use
                // unpacking)
                let target_names = crate::visitors::utils::collect_names_from_assignment_target(
                    &aug_assign.target,
                );

                for var_name in target_names {
                    // Similar check for augmented assignments
                    if let Some(original_name) = lifted_names
                        .iter()
                        .find(|(orig, lifted)| {
                            lifted.as_str() == var_name && function_globals.contains(orig.as_str())
                        })
                        .map(|(orig, _)| orig.as_str())
                    {
                        log::debug!(
                            "Adding sync for augmented assignment to global {var_name}: \
                             {var_name} -> module.{original_name}"
                        );
                        // Add: module.<original_name> = <lifted_name>
                        // Use the provided module name if available, otherwise we can't sync
                        if let Some(mod_name) = module_name {
                            let module_var = sanitize_module_name_for_identifier(mod_name);
                            new_body.push(statements::assign(
                                vec![expressions::attribute(
                                    expressions::name(&module_var, ExprContext::Load),
                                    original_name,
                                    ExprContext::Store,
                                )],
                                expressions::name(var_name, ExprContext::Load),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
