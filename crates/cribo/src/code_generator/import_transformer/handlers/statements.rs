use ruff_python_ast::{
    ExceptHandler, StmtAnnAssign, StmtAssert, StmtAugAssign, StmtClassDef, StmtExpr, StmtFor,
    StmtIf, StmtRaise, StmtReturn, StmtTry, StmtWhile, StmtWith,
};

use crate::code_generator::import_transformer::RecursiveImportTransformer;

pub struct StatementsHandler;

impl StatementsHandler {
    pub(in crate::code_generator::import_transformer) fn handle_ann_assign(
        t: &mut RecursiveImportTransformer,
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
        t: &mut RecursiveImportTransformer,
        s: &mut StmtAugAssign,
    ) {
        t.transform_expr(&mut s.target);
        t.transform_expr(&mut s.value);
    }

    pub(in crate::code_generator::import_transformer) fn handle_expr_stmt(
        t: &mut RecursiveImportTransformer,
        s: &mut StmtExpr,
    ) {
        t.transform_expr(&mut s.value);
    }

    pub(in crate::code_generator::import_transformer) fn handle_return(
        t: &mut RecursiveImportTransformer,
        s: &mut StmtReturn,
    ) {
        if let Some(value) = &mut s.value {
            t.transform_expr(value);
        }
    }

    pub(in crate::code_generator::import_transformer) fn handle_raise(
        t: &mut RecursiveImportTransformer,
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
        t: &mut RecursiveImportTransformer,
        s: &mut StmtAssert,
    ) {
        t.transform_expr(&mut s.test);
        if let Some(msg) = &mut s.msg {
            t.transform_expr(msg);
        }
    }

    pub(in crate::code_generator::import_transformer) fn handle_try(
        t: &mut RecursiveImportTransformer,
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
                t.state.local_variables.insert(name.as_str().to_string());
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
        t: &mut RecursiveImportTransformer,
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
        t: &mut RecursiveImportTransformer,
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
        t: &mut RecursiveImportTransformer,
        s: &mut StmtWhile,
    ) {
        t.transform_expr(&mut s.test);
        t.transform_statements(&mut s.body);
        t.transform_statements(&mut s.orelse);
    }

    pub(in crate::code_generator::import_transformer) fn handle_if(
        t: &mut RecursiveImportTransformer,
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
        t: &mut RecursiveImportTransformer,
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
}
