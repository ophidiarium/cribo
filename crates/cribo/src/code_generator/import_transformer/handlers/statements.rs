use ruff_python_ast::{
    ExceptHandler, StmtAnnAssign, StmtAssert, StmtAugAssign, StmtClassDef, StmtExpr, StmtFor,
    StmtIf, StmtRaise, StmtReturn, StmtTry, StmtWhile, StmtWith,
};

use crate::code_generator::import_transformer::RecursiveImportTransformer;

pub(crate) struct StatementsHandler;

impl StatementsHandler {
    pub(in crate::code_generator::import_transformer) fn handle_ann_assign(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtAnnAssign,
    ) {
        // Transform the annotation
        t.transform_expr(&mut s.annotation);

        // Transform the target
        t.transform_expr(&mut s.target);

        // Transform the value if present
        if let Some(value) = &mut s.value {
            t.transform_expr(value);
        }
    }

    pub(in crate::code_generator::import_transformer) fn handle_aug_assign(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtAugAssign,
    ) {
        t.transform_expr(&mut s.target);
        t.transform_expr(&mut s.value);
    }

    pub(in crate::code_generator::import_transformer) fn handle_expr_stmt(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtExpr,
    ) {
        t.transform_expr(&mut s.value);
    }

    pub(in crate::code_generator::import_transformer) fn handle_return(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtReturn,
    ) {
        if let Some(value) = &mut s.value {
            t.transform_expr(value);
        }
    }

    pub(in crate::code_generator::import_transformer) fn handle_raise(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtRaise,
    ) {
        if let Some(exc) = &mut s.exc {
            t.transform_expr(exc);
        }
        if let Some(cause) = &mut s.cause {
            t.transform_expr(cause);
        }
    }

    pub(in crate::code_generator::import_transformer) fn handle_assert(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtAssert,
    ) {
        t.transform_expr(&mut s.test);
        if let Some(msg) = &mut s.msg {
            t.transform_expr(msg);
        }
    }

    pub(in crate::code_generator::import_transformer) fn handle_try(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtTry,
    ) {
        t.transform_statements(&mut s.body);

        // Ensure try body is not empty
        if s.body.is_empty() {
            log::debug!("Adding pass statement to empty try body in import transformer");
            s.body.push(crate::ast_builder::statements::pass());
        }

        for handler in &mut s.handlers {
            let ExceptHandler::ExceptHandler(eh) = handler;
            if let Some(exc_type) = &mut eh.type_ {
                t.transform_expr(exc_type);
            }
            if let Some(name) = &eh.name {
                t.state.local_variables.insert(name.as_str().to_owned());
                log::debug!("Tracking except alias as local: {}", name.as_str());
            }
            t.transform_statements(&mut eh.body);

            // Ensure exception handler body is not empty
            if eh.body.is_empty() {
                log::debug!("Adding pass statement to empty except handler in import transformer");
                eh.body.push(crate::ast_builder::statements::pass());
            }
        }
        t.transform_statements(&mut s.orelse);
        t.transform_statements(&mut s.finalbody);
    }

    pub(in crate::code_generator::import_transformer) fn handle_with(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtWith,
    ) {
        for item in &mut s.items {
            t.transform_expr(&mut item.context_expr);
            if let Some(vars) = &mut item.optional_vars {
                // Track assigned names as locals before transforming
                let mut with_names = crate::types::FxIndexSet::default();
                crate::code_generator::import_transformer::statement::StatementProcessor::collect_assigned_names(
                    vars,
                    &mut with_names,
                );
                for n in with_names {
                    t.state.local_variables.insert(n.clone());
                    log::debug!("Tracking with-as variable as local: {n}");
                }
                t.transform_expr(vars);
            }
        }
        t.transform_statements(&mut s.body);
    }

    pub(in crate::code_generator::import_transformer) fn handle_for(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtFor,
    ) {
        // Track loop variable as local before transforming
        {
            let mut loop_names = crate::types::FxIndexSet::default();
            crate::code_generator::import_transformer::statement::StatementProcessor::collect_assigned_names(
                &s.target,
                &mut loop_names,
            );
            for n in loop_names {
                t.state.local_variables.insert(n.clone());
                log::debug!("Tracking for loop variable as local: {n}");
            }
        }

        t.transform_expr(&mut s.target);
        t.transform_expr(&mut s.iter);
        t.transform_statements(&mut s.body);
        t.transform_statements(&mut s.orelse);
    }

    pub(in crate::code_generator::import_transformer) fn handle_while(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtWhile,
    ) {
        t.transform_expr(&mut s.test);
        t.transform_statements(&mut s.body);
        t.transform_statements(&mut s.orelse);
    }

    pub(in crate::code_generator::import_transformer) fn handle_if(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtIf,
    ) {
        t.transform_expr(&mut s.test);
        t.transform_statements(&mut s.body);

        // Check if this is a TYPE_CHECKING block and ensure it has a body
        if s.body.is_empty()
            && crate::code_generator::import_transformer::statement::StatementProcessor::is_type_checking_condition(
                &s.test,
            )
        {
            log::debug!(
                "Adding pass statement to empty TYPE_CHECKING block in import transformer"
            );
            s.body.push(crate::ast_builder::statements::pass());
        }

        for clause in &mut s.elif_else_clauses {
            if let Some(test_expr) = &mut clause.test {
                t.transform_expr(test_expr);
            }
            t.transform_statements(&mut clause.body);

            // Ensure non-empty body for elif/else clauses too
            if clause.body.is_empty() {
                log::debug!(
                    "Adding pass statement to empty elif/else clause in import transformer"
                );
                clause.body.push(crate::ast_builder::statements::pass());
            }
        }
    }

    pub(in crate::code_generator::import_transformer) fn handle_class_def(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut StmtClassDef,
    ) {
        // Transform decorators
        for decorator in &mut s.decorator_list {
            t.transform_expr(&mut decorator.expression);
        }

        // Transform base classes
        t.transform_class_bases(s);

        // Transform class body
        t.transform_statements(&mut s.body);
    }

    pub(in crate::code_generator::import_transformer) fn handle_function_def(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut ruff_python_ast::StmtFunctionDef,
    ) {
        log::debug!(
            "RecursiveImportTransformer: Entering function '{}'",
            s.name.as_str()
        );

        // Transform decorators
        for decorator in &mut s.decorator_list {
            t.transform_expr(&mut decorator.expression);
        }

        // Transform parameter annotations and default values
        for param in &mut s.parameters.posonlyargs {
            if let Some(annotation) = &mut param.parameter.annotation {
                t.transform_expr(annotation);
            }
            if let Some(default) = &mut param.default {
                t.transform_expr(default);
            }
        }
        for param in &mut s.parameters.args {
            if let Some(annotation) = &mut param.parameter.annotation {
                t.transform_expr(annotation);
            }
            if let Some(default) = &mut param.default {
                t.transform_expr(default);
            }
        }
        if let Some(vararg) = &mut s.parameters.vararg
            && let Some(annotation) = &mut vararg.annotation
        {
            t.transform_expr(annotation);
        }
        for param in &mut s.parameters.kwonlyargs {
            if let Some(annotation) = &mut param.parameter.annotation {
                t.transform_expr(annotation);
            }
            if let Some(default) = &mut param.default {
                t.transform_expr(default);
            }
        }
        if let Some(kwarg) = &mut s.parameters.kwarg
            && let Some(annotation) = &mut kwarg.annotation
        {
            t.transform_expr(annotation);
        }

        // Transform return type annotation
        if let Some(returns) = &mut s.returns {
            t.transform_expr(returns);
        }

        // Save current local variables and create a new scope for the function
        let saved_locals = t.state.local_variables.clone();

        // Save the wrapper module imports - these should be scoped to each function
        // to prevent imports from one function affecting another
        let saved_wrapper_imports = t.state.wrapper_module_imports.clone();

        // Track function parameters as local variables before transforming the body
        // This prevents incorrect transformation of parameter names that shadow
        // stdlib modules

        // Track positional-only parameters
        for param in &s.parameters.posonlyargs {
            t.state
                .local_variables
                .insert(param.parameter.name.as_str().to_owned());
            log::debug!(
                "Tracking function parameter as local (posonly): {}",
                param.parameter.name.as_str()
            );
        }

        // Track regular parameters
        for param in &s.parameters.args {
            t.state
                .local_variables
                .insert(param.parameter.name.as_str().to_owned());
            log::debug!(
                "Tracking function parameter as local: {}",
                param.parameter.name.as_str()
            );
        }

        // Track *args if present
        if let Some(vararg) = &s.parameters.vararg {
            t.state
                .local_variables
                .insert(vararg.name.as_str().to_owned());
            log::debug!(
                "Tracking function parameter as local (vararg): {}",
                vararg.name.as_str()
            );
        }

        // Track keyword-only parameters
        for param in &s.parameters.kwonlyargs {
            t.state
                .local_variables
                .insert(param.parameter.name.as_str().to_owned());
            log::debug!(
                "Tracking function parameter as local (kwonly): {}",
                param.parameter.name.as_str()
            );
        }

        // Track **kwargs if present
        if let Some(kwarg) = &s.parameters.kwarg {
            t.state
                .local_variables
                .insert(kwarg.name.as_str().to_owned());
            log::debug!(
                "Tracking function parameter as local (kwarg): {}",
                kwarg.name.as_str()
            );
        }

        // Save the current scope level and mark that we're entering a local scope
        let saved_at_module_level = t.state.at_module_level;
        t.state.at_module_level = false;

        // Save current function context and compute symbol analysis once
        let saved_function_body = t.state.current_function_body.take();
        let saved_used_symbols = t.state.current_function_used_symbols.take();

        // Compute used symbols once from the original body (before transformation)
        t.state.current_function_used_symbols = Some(
            crate::visitors::SymbolUsageVisitor::collect_used_symbols(&s.body),
        );

        // Set function body for compatibility with existing APIs
        t.state.current_function_body = Some(s.body.clone());

        // Transform the function body
        t.transform_statements(&mut s.body);

        // After all transformations, hoist and deduplicate any inserted
        // `global` statements to the start of the function body (after a
        // docstring if present) to ensure correct Python semantics.
        crate::code_generator::import_transformer::statement::StatementProcessor::hoist_function_globals(
            s,
        );

        // Restore the previous scope level
        t.state.at_module_level = saved_at_module_level;

        // Restore the previous function context
        t.state.current_function_body = saved_function_body;
        t.state.current_function_used_symbols = saved_used_symbols;

        // Restore the wrapper module imports to prevent function-level imports from
        // affecting other functions
        t.state.wrapper_module_imports = saved_wrapper_imports;

        // Restore the previous scope's local variables
        t.state.local_variables = saved_locals;
    }

    /// Handle assignment statement. Returns whether the caller should advance `i` normally
    /// (true) or perform `i += 1; continue;` (false). Mirrors current control flow which
    /// advances and continues within the arm.
    pub(in crate::code_generator::import_transformer) fn handle_assign(
        t: &mut RecursiveImportTransformer<'_>,
        s: &mut ruff_python_ast::StmtAssign,
    ) -> bool {
        // Track assignment LHS names to prevent collapsing RHS to self
        let mut lhs_names = crate::types::FxIndexSet::<String>::default();
        for target in &s.targets {
            crate::code_generator::import_transformer::statement::StatementProcessor::collect_assigned_names(
                target,
                &mut lhs_names,
            );
        }

        let saved_targets = t.state.current_assignment_targets.clone();
        t.state.current_assignment_targets = if lhs_names.is_empty() {
            None
        } else {
            Some(lhs_names)
        };

        // Handle importlib.import_module() assignment tracking
        if let ruff_python_ast::Expr::Call(call) = &s.value.as_ref()
            && crate::code_generator::import_transformer::handlers::dynamic::DynamicHandler::is_importlib_import_module_call(
                call,
                &t.state.import_aliases,
            )
        {
            // Get assigned names to pass to the handler
            let mut assigned_names = crate::types::FxIndexSet::default();
            for target in &s.targets {
                crate::code_generator::import_transformer::statement::StatementProcessor::collect_assigned_names(
                    target,
                    &mut assigned_names,
                );
            }

            // Track all assigned names (including tuple/list destructuring) as locals
            for n in &assigned_names {
                t.state.local_variables.insert(n.clone());
            }

            crate::code_generator::import_transformer::handlers::dynamic::DynamicHandler::handle_importlib_assignment(
                &assigned_names,
                call,
                t.state.bundler,
                &mut t.state.importlib_inlined_modules,
            );
        } else {
            // For non-importlib assignments, still track all assigned names as locals
            let mut assigned_names = crate::types::FxIndexSet::default();
            for target in &s.targets {
                crate::code_generator::import_transformer::statement::StatementProcessor::collect_assigned_names(
                    target,
                    &mut assigned_names,
                );
            }

            for n in &assigned_names {
                t.state.local_variables.insert(n.clone());
            }
        }

        // Transform the targets
        for target in &mut s.targets {
            t.transform_expr(target);
        }

        // Transform the RHS
        t.transform_expr(&mut s.value);

        // Restore previous context
        t.state.current_assignment_targets = saved_targets;

        // Original code performs `i += 1; continue;` in the caller.
        false
    }
}
