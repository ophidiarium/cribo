use ruff_python_ast::{AtomicNodeIndex, ExprContext, Stmt, StmtFor, StmtIf};
use ruff_text_size::TextRange;

use crate::ast_builder::{expressions, statements};

/// Generate a `for` loop that merges public attributes from a source module/object
/// into a target namespace using `setattr`.
///
/// Produces Python equivalent:
///
/// for attr in `dir(source_module)`:
///     if not attr.startswith('_'):
///         setattr(namespace, attr, `getattr(source_module`, attr))
///
/// Returns the `for` statement node.
pub fn generate_merge_module_attributes(namespace_name: &str, source_module_name: &str) -> Stmt {
    let attr_var = "attr";

    // Target of the for loop: `attr`
    let loop_target = expressions::name(attr_var, ExprContext::Store);

    // Iterator of the for loop: `dir(source_module)`
    let dir_call = expressions::call(
        expressions::name("dir", ExprContext::Load),
        vec![expressions::name(source_module_name, ExprContext::Load)],
        vec![],
    );

    // Condition: `not attr.startswith('_')`
    let condition = expressions::unary_op(
        ruff_python_ast::UnaryOp::Not,
        expressions::call(
            expressions::attribute(
                expressions::name(attr_var, ExprContext::Load),
                "startswith",
                ExprContext::Load,
            ),
            vec![expressions::string_literal("_")],
            vec![],
        ),
    );

    // Value to set: `getattr(source_module, attr)`
    let getattr_call = expressions::call(
        expressions::name("getattr", ExprContext::Load),
        vec![
            expressions::name(source_module_name, ExprContext::Load),
            expressions::name(attr_var, ExprContext::Load),
        ],
        vec![],
    );

    // Body action: `setattr(namespace, attr, getattr(...))`
    let setattr_call_stmt = statements::expr(expressions::call(
        expressions::name("setattr", ExprContext::Load),
        vec![
            expressions::name(namespace_name, ExprContext::Load),
            expressions::name(attr_var, ExprContext::Load),
            getattr_call,
        ],
        vec![],
    ));

    // if not attr.startswith('_'): setattr(...)
    let if_stmt = Stmt::If(StmtIf {
        node_index: AtomicNodeIndex::dummy(),
        test: Box::new(condition),
        body: vec![setattr_call_stmt],
        elif_else_clauses: vec![],
        range: TextRange::default(),
    });

    // for attr in dir(...): if ...
    Stmt::For(StmtFor {
        node_index: AtomicNodeIndex::dummy(),
        target: Box::new(loop_target),
        iter: Box::new(dir_call),
        body: vec![if_stmt],
        orelse: vec![],
        is_async: false,
        range: TextRange::default(),
    })
}
