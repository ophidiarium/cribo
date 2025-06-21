//! AST transformer that removes no-op patterns from Python code.
//!
//! This transformer removes operations that have no side effects and don't
//! change program behavior, such as self-reference assignments (x = x),
//! unnecessary pass statements, and empty expression statements.

use ruff_python_ast::{
    self as ast, Expr, ExprContext, Operator, Stmt, StmtAssign, StmtAugAssign, StmtExpr,
    visitor::transformer::{Transformer, walk_body, walk_stmt},
};
use ruff_text_size::TextRange;

/// Removes no-op patterns from Python AST
pub struct NoOpsRemovalTransformer;

impl NoOpsRemovalTransformer {
    /// Create a new no-ops removal transformer
    pub fn new() -> Self {
        Self
    }

    /// Transform a module by removing no-op patterns
    pub fn transform_module(&self, module: &mut ast::ModModule) {
        // Process the module body to remove no-ops
        self.process_body(&mut module.body);
    }

    /// Check if a statement should be removed
    fn is_removable_statement(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Assign(assign) => self.is_self_reference_assignment(assign),
            Stmt::AugAssign(aug_assign) => self.is_identity_aug_assign(aug_assign),
            Stmt::Pass(_) => true, // We'll handle pass statements specially in context
            Stmt::Expr(expr_stmt) => self.is_removable_expr_stmt(expr_stmt),
            _ => false,
        }
    }

    /// Check if an assignment is a self-reference (e.g., x = x)
    fn is_self_reference_assignment(&self, assign: &StmtAssign) -> bool {
        // Only check simple assignments with one target and one value
        if assign.targets.len() != 1 {
            return false;
        }

        // Check if both target and value are simple names
        if let (Expr::Name(target), Expr::Name(value)) = (&assign.targets[0], assign.value.as_ref())
        {
            // Must be same identifier, target in Store context, value in Load context
            let is_self_ref = target.id == value.id
                && matches!(target.ctx, ExprContext::Store)
                && matches!(value.ctx, ExprContext::Load);

            if is_self_ref {
                log::debug!(
                    "Removing self-reference assignment: {} = {}",
                    target.id,
                    value.id
                );
            }

            is_self_ref
        } else {
            false
        }
    }

    /// Check if an augmented assignment is an identity operation
    fn is_identity_aug_assign(&self, aug_assign: &StmtAugAssign) -> bool {
        // Only check simple name targets (not attributes or subscripts)
        if !matches!(aug_assign.target.as_ref(), Expr::Name(_)) {
            return false;
        }

        match (&aug_assign.op, aug_assign.value.as_ref()) {
            // Numeric identity operations
            (Operator::Add, Expr::NumberLiteral(n)) => n.value.as_int().is_some_and(|i| *i == 0),
            (Operator::Sub, Expr::NumberLiteral(n)) => n.value.as_int().is_some_and(|i| *i == 0),
            (Operator::Mult, Expr::NumberLiteral(n)) => n.value.as_int().is_some_and(|i| *i == 1),
            (Operator::Div | Operator::FloorDiv, Expr::NumberLiteral(n)) => {
                n.value.as_int().is_some_and(|i| *i == 1)
            }
            (Operator::Pow, Expr::NumberLiteral(n)) => n.value.as_int().is_some_and(|i| *i == 1),

            // String/list concatenation with empty
            (Operator::Add, Expr::StringLiteral(s)) => s.value.is_empty(),
            (Operator::Add, Expr::List(l)) => l.elts.is_empty(),

            // Set operations
            (Operator::BitOr, Expr::Call(call)) => {
                // Check for set() constructor
                if let Expr::Name(name) = call.func.as_ref() {
                    name.id.as_str() == "set" && call.arguments.args.is_empty()
                } else {
                    false
                }
            }
            (Operator::BitOr, Expr::Set(s)) => s.elts.is_empty(),

            // Boolean operations
            (Operator::BitAnd, Expr::BooleanLiteral(b)) => b.value,
            (Operator::BitOr, Expr::BooleanLiteral(b)) => !b.value,
            (Operator::BitOr, Expr::NumberLiteral(n)) => n.value.as_int().is_some_and(|i| *i == 0),
            (Operator::BitAnd, Expr::NumberLiteral(n)) => {
                n.value.as_int().is_some_and(|i| *i == -1)
            }

            // Self-intersection (x &= x)
            (Operator::BitAnd, Expr::Name(name)) => {
                if let Expr::Name(target_name) = aug_assign.target.as_ref() {
                    target_name.id == name.id
                } else {
                    false
                }
            }

            _ => false,
        }
    }

    /// Check if an expression statement is removable
    fn is_removable_expr_stmt(&self, expr_stmt: &StmtExpr) -> bool {
        match expr_stmt.value.as_ref() {
            // Literal expressions (except potential docstrings)
            Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_) => true,

            Expr::StringLiteral(_) => {
                // Don't remove potential docstrings
                // String literals as expression statements could be docstrings
                false
            }

            // Other literals
            Expr::BytesLiteral(_)
            | Expr::List(_)
            | Expr::Tuple(_)
            | Expr::Set(_)
            | Expr::Dict(_) => true,

            // Simple name reference (not a call or operation)
            Expr::Name(_) => true,

            _ => false,
        }
    }

    /// Process a body of statements and filter out no-ops
    fn process_body(&self, body: &mut Vec<Stmt>) {
        // First transform all nested statements
        for stmt in body.iter_mut() {
            self.visit_stmt(stmt);
        }

        // Then filter out removable statements
        let original_len = body.len();
        body.retain(|stmt| {
            if matches!(stmt, Stmt::Pass(_)) {
                // Keep pass statements only if they're necessary
                false // We'll add them back if needed
            } else {
                !self.is_removable_statement(stmt)
            }
        });

        // If the body is now empty but wasn't before, add a pass statement
        if body.is_empty() && original_len > 0 {
            body.push(Stmt::Pass(ast::StmtPass {
                range: TextRange::default(),
            }));
        }
    }
}

impl Transformer for NoOpsRemovalTransformer {
    fn visit_body(&self, body: &mut [Stmt]) {
        // We can't directly modify a slice, so we handle this at a higher level
        // The actual work is done in process_body for Vec<Stmt> and transform_module
        walk_body(self, body);
    }

    fn visit_stmt(&self, stmt: &mut Stmt) {
        // Apply transformations based on statement type
        match stmt {
            Stmt::FunctionDef(func_def) => {
                // Process function body
                self.process_body(&mut func_def.body);
            }
            Stmt::ClassDef(class_def) => {
                // Process class body
                self.process_body(&mut class_def.body);
            }
            Stmt::If(if_stmt) => {
                // Process if body
                self.process_body(&mut if_stmt.body);
                // Process elif/else bodies
                for elif_else in &mut if_stmt.elif_else_clauses {
                    self.process_body(&mut elif_else.body);
                }
            }
            Stmt::While(while_stmt) => {
                // Process while body
                self.process_body(&mut while_stmt.body);
                // Process else body if present
                self.process_body(&mut while_stmt.orelse);
            }
            Stmt::For(for_stmt) => {
                // Process for body
                self.process_body(&mut for_stmt.body);
                // Process else body if present
                self.process_body(&mut for_stmt.orelse);
            }
            Stmt::With(with_stmt) => {
                // Process with body
                self.process_body(&mut with_stmt.body);
            }
            Stmt::Try(try_stmt) => {
                // Process try body
                self.process_body(&mut try_stmt.body);
                // Process except handlers
                for handler in &mut try_stmt.handlers {
                    match handler {
                        ast::ExceptHandler::ExceptHandler(handler) => {
                            self.process_body(&mut handler.body);
                        }
                    }
                }
                // Process else body
                self.process_body(&mut try_stmt.orelse);
                // Process finally body
                self.process_body(&mut try_stmt.finalbody);
            }
            _ => {
                // For other statement types, apply default transformation
                walk_stmt(self, stmt);
            }
        }
    }
}

impl Default for NoOpsRemovalTransformer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::{ParseError, parse_module};

    use super::*;

    fn parse_and_transform(code: &str) -> Result<ast::ModModule, ParseError> {
        let parsed = parse_module(code)?;
        let mut module = parsed.into_syntax();
        let transformer = NoOpsRemovalTransformer::new();
        transformer.transform_module(&mut module);
        Ok(module)
    }

    #[test]
    fn test_self_reference_removal() {
        let code = r#"
x = 42
x = x
y = "hello"
y = y
"#;
        let module = parse_and_transform(code).expect("failed to parse and transform test code");

        // Should have only 2 statements left (the initial assignments)
        assert_eq!(module.body.len(), 2);
    }

    #[test]
    fn test_identity_aug_assign_removal() {
        let code = r#"
x = 10
x += 0
x -= 0
x *= 1
x //= 1
"#;
        let module = parse_and_transform(code).expect("failed to parse and transform test code");

        // Should have only 1 statement left (the initial assignment)
        assert_eq!(module.body.len(), 1);
    }

    #[test]
    fn test_empty_expression_removal() {
        let code = r#"
42
None
True
[1, 2, 3]
x = 10
"#;
        let module = parse_and_transform(code).expect("failed to parse and transform test code");

        // Should have only 1 statement left (the assignment)
        assert_eq!(module.body.len(), 1);
    }

    #[test]
    fn test_pass_in_empty_function() {
        let code = r#"
def empty():
    pass
"#;
        let module = parse_and_transform(code).expect("failed to parse and transform test code");

        // Should keep the function with pass
        assert_eq!(module.body.len(), 1);
        if let Stmt::FunctionDef(f) = &module.body[0] {
            assert_eq!(f.body.len(), 1);
            assert!(matches!(f.body[0], Stmt::Pass(_)));
        }
    }
}
